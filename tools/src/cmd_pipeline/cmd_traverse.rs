use std::collections::HashSet;

use async_trait::async_trait;
use bitflags::bitflags;
use clap::Args;
use itertools::Itertools;
use serde_json::{from_value, Value};
use tracing::trace;
use ustr::{ustr, Ustr};

use super::{
    interface::{OverloadInfo, OverloadKind, PipelineCommand, PipelineValues},
    symbol_graph::{
        DerivedSymbolInfo, NamedSymbolGraph, SymbolBadge, SymbolGraphCollection,
        SymbolGraphEdgeSet, SymbolGraphNodeSet,
    },
};

use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    cmd_pipeline::symbol_graph::{EdgeDetail, EdgeKind},
    file_format::{
        analysis::{
            BindingSlotKind, OntologySlotInfo, OntologySlotKind, StructuredBindingSlotInfo,
            StructuredFieldInfo,
        },
        ontology_mapping::{label_to_badge_info, pointer_kind_to_badge_info},
    },
};

/// Processes piped-in crossref symbol data, recursively traversing the given
/// edges, building up a graph that also holds onto the crossref data for all
/// traversed symbols.
#[derive(Debug, Args)]
pub struct Traverse {
    /// The edge to traverse, currently one of: "uses", "callees", "class",
    /// "inheritance".
    #[clap(long, short, value_parser, default_value = "callees")]
    edge: String,

    /// Maximum traversal depth.  Traversal will also be constrained by the
    /// applicable node-limit, but is effectively breadth-first.
    #[clap(long, short, value_parser, default_value = "8")]
    max_depth: u32,

    /// When enabled, the traversal will be performed with the higher
    /// paths-between-node-limit in effect, then the roots of the initial
    /// traversal will be used as pair-wise inputs to the all_simple_paths
    /// petgraph algorithm to derive a new graph which will be constrained to
    /// the normal "node-limit".
    #[clap(long, value_parser)]
    paths_between: bool,

    /// Maximum number of nodes in a resulting graph.  When paths are involved,
    /// we may opt to add the entirety of the path that puts the graph over the
    /// node limit rather than omitting it.
    #[clap(long, value_parser = clap::value_parser!(u32).range(16..=1024), default_value = "256")]
    pub node_limit: u32,
    /// Maximum number of nodes in a graph being built to be processed by
    /// paths-between.
    #[clap(long, value_parser = clap::value_parser!(u32).range(16..=16384), default_value = "8192")]
    pub paths_between_node_limit: u32,
    /// Maximum number of paths to consider for paths-between.  This value has
    /// not been tested but is based on figuring 16 roots is a reasonable number
    /// of roots, and then we could allow growing that by 4.  No clue how this
    /// relates to actual performance.
    #[clap(long, value_parser = clap::value_parser!(u32).range(16..=1024), default_value = "256")]
    pub paths_limit: u32,

    /// If we see "uses" with this many paths with hits, do not process any of
    /// the uses.  This is path-centric because uses are hierarchically
    /// clustered by path right now.
    ///
    /// TODO: Probably have the meta capture the total number of uses so we can
    /// just perform a look-up without this hack.  But this hack works for
    /// experimenting.
    #[clap(long, value_parser, default_value = "16")]
    pub skip_uses_at_path_count: u32,
}

#[derive(Debug)]
pub struct TraverseCommand {
    pub args: Traverse,
}

bitflags! {
    #[derive(Clone, Copy)]
    pub struct Traversals: u32 {
        const Super    = 0b00000001;
        const Subclass = 0b00000010;
    }
}

/// ### Theory of Operation
///
/// The crossref database can be thought of as a massive graph.  Each entry in
/// the crossref database is a symbol and also a node.  The crossref entry
/// contains references to other symbol nodes (particularly via the "meta"
/// structured information) as well as code location nodes which also provide
/// symbol nodes by way of their "contextsym".  (In the future we will likely
/// also infer additional graph relationships by looking at function call
/// arguments.)  There are other systems (ex: Kythe) which explicitly
/// represent their data in a graph database/triple-store, but a fundamental
/// searchfox design decision is to use a de-normalized representation and this
/// seems to be holding up for both performance and human comprehension
/// purposes.
///
/// This command is focused on efficiently deriving an interesting, useful, and
/// comprehensible sub-graph of that massive graph.  Although the current state
/// of implementation operates by starting from a set of nodes and enumerating
/// and considering graph edges dynamically, we could imagine that in the future
/// we might use some combination of pre-computation which could involve bulk /
/// batch processing.
///
/// ### Specific traversals
///
/// We potentially traverse all of the following crossref paths:
/// - "calls"
/// - "meta/fields":
/// - "meta/ontologySlots":
/// - "meta/overrides":
/// - "meta/overriddenBy":
/// - "meta/slotOwner":
/// - "uses":
///
#[async_trait]
impl PipelineCommand for TraverseCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let max_depth = self.args.max_depth;
        let cil = match input {
            PipelineValues::SymbolCrossrefInfoList(cil) => cil,
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "traverse needs a CrossrefInfoList".to_string(),
                }));
            }
        };

        let mut sym_node_set = SymbolGraphNodeSet::new();
        let mut sym_edge_set = SymbolGraphEdgeSet::new();
        let mut graph = NamedSymbolGraph::new("only".to_string());

        // A to-do list of nodes we have not yet traversed.
        let mut to_traverse = Vec::new();
        // Nodes that have been scheduled to be traversed or ruled out.  A node
        // in this set should not be added to `to_traverse`.
        let mut considered = HashSet::new();
        // Root set for paths-between use.
        let mut root_set = vec![];

        let mut overloads_hit = vec![];

        let all_traversals_valid = Traversals::Super | Traversals::Subclass;

        // Propagate the starting symbols into the graph and queue them up for
        // traversal.
        for info in cil.symbol_crossref_infos {
            to_traverse.push((info.symbol.clone(), 0, all_traversals_valid));
            considered.insert(info.symbol.clone());

            let (sym_node_id, _info) =
                sym_node_set.add_symbol(DerivedSymbolInfo::new(info.symbol, info.crossref_info, 0));
            // Explicitly put the node in the graph so if we don't find any
            // edges, we still display the node.  This is important for things
            // like "class-diagram" where showing nothing is very confusing.
            graph.ensure_node(sym_node_id.clone());
            // TODO: do something to limit the size of the root-set.  The
            // combinatorial explosion for something like nsGlobalWindowInner is
            // just too silly.  This can added as an overload.
            root_set.push(sym_node_id);
        }

        let node_limit = if self.args.paths_between {
            self.args.paths_between_node_limit
        } else {
            self.args.node_limit
        };

        let stop_at_class_label = match self.args.edge.as_str() {
            "class" => Some("class-diagram:stop"),
            _ => None,
        };

        let traverse_callees = match self.args.edge.as_str() {
            "callees" => true,
            _ => false,
        };
        let traverse_fields = match self.args.edge.as_str() {
            "class" => true,
            _ => false,
        };
        let traverse_overridden_by = match self.args.edge.as_str() {
            "inheritance" => true,
            _ => false,
        };
        let traverse_overrides = match self.args.edge.as_str() {
            "inheritance" => true,
            "uses" => true,
            _ => false,
        };
        let traverse_subclasses = match self.args.edge.as_str() {
            "class" => true,
            "inheritance" => true,
            _ => false,
        };
        let traverse_superclasses = match self.args.edge.as_str() {
            "class" => true,
            "inheritance" => true,
            _ => false,
        };
        let traverse_uses = match self.args.edge.as_str() {
            "uses" => true,
            _ => false,
        };

        // General operation:
        // - We pull a node to be traversed off the queue.  This ends up depth
        //   first.
        // - We check if we already have the crossref info for the symbol and
        //   look it up if not.  There's an asymmetry here between the initial
        //   set of symbols we're traversing from which we already have cached
        //   values for and the new edges we discover, but it's not a concern.
        // - We traverse the list of edges.
        while let Some((sym, depth, cur_traversals)) = to_traverse.pop() {
            if sym_node_set.symbol_crossref_infos.len() as u32 >= node_limit {
                trace!(sym = %sym, depth, "stopping because of node limit");
                overloads_hit.push(OverloadInfo {
                    kind: OverloadKind::NodeLimit,
                    sym: Some(sym.to_string()),
                    exist: to_traverse.len() as u32,
                    included: node_limit,
                    local_limit: 0,
                    global_limit: node_limit,
                });
                to_traverse.clear();
                break;
            };

            trace!(sym = %sym, depth, "processing");
            let next_depth = depth + 1;
            // Note that we will regularly end up dropping our `sym_info` borrow
            // as we issue other calls to `ensure_symbol`.  In those cases, we
            // can efficiently re-acquire a reference to our sym_info through
            // `sym_node_set.get` or `sym_node_set.get_mut`.  Because we
            // regularly go async, using `Rc` isn't a great alternative because
            // it's not Send so we would need to step up to `Arc` and we don't
            // really need that.
            let (sym_id, sym_info) = sym_node_set.ensure_symbol(&sym, server, depth).await?;

            if let Some(stop_at_label) = &stop_at_class_label {
                // XXX remove these after the next crossref rebuild with the new markers.
                match sym_info.symbol.as_str() {
                    "T_nsWrapperCache" | "T_nsISupports" | "XPIDL_nsISupports" | "T_mozilla::SupportsWeakPtr" | "T_JSObject" |
                    "T_mozilla::Runnable" => {
                        continue;
                    }
                    _ => {}
                };
                if let Some(labels_json) = sym_info.crossref_info.pointer("/meta/labels").cloned() {
                    let labels: Vec<Ustr> = from_value(labels_json).unwrap();
                    let mut skip_symbol = false;
                    for label in labels {
                        if label.as_str() == *stop_at_label {
                            // Don't process the fields if we see a stop.  This is something
                            // manually specified in ontology-mapping.toml currently.
                            skip_symbol = true;
                        }
                    }
                    // only skip if this isn't the requested symbol (depth == 0)
                    if depth > 0 && skip_symbol {
                        continue;
                    }
                }
            }

            // ## Clone the edges now before engaging in additional borrows.
            let slot_owner = sym_info.crossref_info.pointer("/meta/slotOwner").cloned();

            if traverse_fields {
                // Traverse the fields out of this class
                // Note that depth won't stop us from showing a class's fields,
                // just whether we process the target symbol!
                if let Some(fields_json) = sym_info.crossref_info.pointer("/meta/fields").cloned() {
                    let fields: Vec<StructuredFieldInfo> = from_value(fields_json).unwrap();
                    for field in fields {
                        let mut show_field = field.labels.len() > 0;
                        let mut effective_subsystem = None;

                        let mut targets = vec![];
                        for ptr_info in field.pointer_info {
                            show_field = true;
                            let (target_id, target_info) =
                                sym_node_set.ensure_symbol(&ptr_info.sym, server, next_depth).await?;
                            if next_depth < max_depth && considered.insert(ptr_info.sym.clone()) {
                                trace!(sym = ptr_info.sym.as_str(), "scheduling pointee sym");
                                to_traverse.push((ptr_info.sym.clone(), next_depth, all_traversals_valid));
                            }

                            if effective_subsystem.is_none() {
                                effective_subsystem = target_info.get_subsystem();
                            }

                            // XXX consider moving the mapping here to the ontology file
                            // or something that's not source code.
                            let (pri, edge_kind, use_badge, use_class) =
                                pointer_kind_to_badge_info(&ptr_info.kind);
                            targets.push((
                                target_id,
                                pri,
                                edge_kind,
                                use_badge,
                                vec![EdgeDetail::HoverClass(use_class.to_string())],
                            ));
                        }

                        if show_field {
                            let (field_id, field_info) =
                                sym_node_set.ensure_symbol(&field.sym, server, next_depth).await?;
                            field_info.effective_subsystem = effective_subsystem;
                            for label in field.labels {
                                // XXX like above, consider moving the emoji label mapping here to
                                // the ontology file or elsewhere.
                                if let Some((pri, shorter_label)) = label_to_badge_info(&label) {
                                    field_info.badges.push(SymbolBadge {
                                        pri,
                                        label: ustr(shorter_label),
                                        source_jump: None,
                                    });
                                }
                            }
                            for (tgt_id, pri, edge_kind, ptr_badge, edge_details) in targets {
                                field_info.badges.push(SymbolBadge {
                                    pri,
                                    label: ustr(ptr_badge),
                                    source_jump: None,
                                });
                                sym_edge_set.ensure_edge_in_graph(
                                    field_id.clone(),
                                    tgt_id,
                                    edge_kind,
                                    edge_details,
                                    &mut graph,
                                );
                            }
                        }
                    }
                }
            }

            if traverse_fields && next_depth < depth {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on field-member-uses"),
                    })
                };

                // Find the places where this type is used as a field member.
                //
                // A hack/simplification we do here is just add the class and leave
                // it up to the traversal of the class to generate the field edge
                // for us.  We don't need to worry about weirdness with the depth
                // threshold here because our logic above will always process the
                // class's fields; the field traversal is not a separate step with
                // its own depth addition.
                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .crossref_info
                    .pointer("/field-member-uses")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();

                for target in overrides.as_array().unwrap() {
                    // fmu is { sym, pretty, fields }
                    // The sym is the class referencing our type.
                    let target_sym_str = target["sym"].as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);

                    let (_target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server, next_depth).await?;

                    // we already considered depth in the outer condition
                    if considered.insert(target_info.symbol.clone()) {
                        trace!(sym = target_sym_str, "scheduling field-member-use");
                        to_traverse.push((target_info.symbol.clone(), next_depth, all_traversals_valid));
                    }
                }
            }

            // Check whether to traverse a parent binding slot relationship.
            if let Some(val) = slot_owner {
                let slot_owner: StructuredBindingSlotInfo = from_value(val).unwrap();

                // There are a few possibilities with a binding slot.  It can be
                // a binding type that is:
                //
                // 1. An IPC `Recv` where the "uses" of this method will only be
                //    plumbing that is distracting and should be elided in favor
                //    of showing all `Send` calls instead.
                // 2. An XPIDL-like method implementation that can be called
                //    through either a cross-language glue layer like XPConnect
                //    which requires processing the slots or directly as the
                //    implementation does not have to go through a glue layer
                //    but can be called directly.  In this case, we do want to
                //    process uses directly.
                // 3. Support logic like an `EnablingPref` or `EnablingFunc` and
                //    any use of the symbol is terminal and should not be
                //    (erroneously) treated as somehow triggering the WebIDL
                //    functions which it is enabling for.
                let should_traverse = match slot_owner.props.slot_kind {
                    // Enabling funcs and constants don't count as interesting
                    // uses in either direction; they are support.
                    BindingSlotKind::EnablingPref
                    | BindingSlotKind::EnablingFunc
                    | BindingSlotKind::Const
                    | BindingSlotKind::Send => false,
                    _ => true,
                };
                if should_traverse {
                    let (idl_id, idl_info) =
                        sym_node_set.ensure_symbol(&slot_owner.sym, server, next_depth).await?;

                    // So if this was the recv, let's look through to the send
                    // and add an edge to that instead and then continue the
                    // loop so we ignore the other uses.
                    if slot_owner.props.slot_kind == BindingSlotKind::Recv {
                        if let Some(send_sym) = idl_info.get_binding_slot_sym("send") {
                            let (send_id, send_info) =
                                sym_node_set.ensure_symbol(&send_sym, server, next_depth).await?;
                            sym_edge_set.ensure_edge_in_graph(
                                send_id,
                                sym_id.clone(),
                                EdgeKind::IPC,
                                vec![],
                                &mut graph,
                            );
                            if next_depth < max_depth && considered.insert(send_info.symbol.clone())
                            {
                                trace!(sym = send_info.symbol.as_str(), "scheduling send slot sym");
                                to_traverse.push((send_info.symbol.clone(), next_depth, all_traversals_valid));
                            }
                        }
                        continue;
                    } else {
                        // And so here we're, uh, just going to name-check the
                        // parent.
                        // TODO: further implement binding slot magic.
                        sym_edge_set.ensure_edge_in_graph(
                            idl_id,
                            sym_id.clone(),
                            EdgeKind::Implementation,
                            vec![],
                            &mut graph,
                        );
                        if next_depth < max_depth && considered.insert(idl_info.symbol.clone()) {
                            trace!(sym = idl_info.symbol.as_str(), "scheduling owner slot sym");
                            to_traverse.push((idl_info.symbol.clone(), next_depth, all_traversals_valid));
                        }
                    }
                }
            }

            // Check whether we have any ontology shortcuts to handle.
            let sym_info = sym_node_set.get(&sym_id);
            if let Some(Value::Array(slots)) = sym_info
                .crossref_info
                .pointer("/meta/ontologySlots")
                .cloned()
            {
                let mut keep_going = true;
                for slot_val in slots {
                    let slot: OntologySlotInfo = from_value(slot_val).unwrap();
                    let (should_traverse, upwards) = match slot.slot_kind {
                        OntologySlotKind::RunnableConstructor => (self.args.edge == "uses", true),
                        OntologySlotKind::RunnableMethod => (self.args.edge == "callees", false),
                    };
                    if should_traverse {
                        for rel_sym in slot.syms {
                            let (rel_id, _) = sym_node_set.ensure_symbol(&rel_sym, server, next_depth).await?;
                            if upwards {
                                sym_edge_set.ensure_edge_in_graph(
                                    rel_id,
                                    sym_id.clone(),
                                    EdgeKind::Default,
                                    vec![],
                                    &mut graph,
                                );
                            } else {
                                sym_edge_set.ensure_edge_in_graph(
                                    sym_id.clone(),
                                    rel_id,
                                    EdgeKind::Default,
                                    vec![],
                                    &mut graph,
                                );
                            }
                            if next_depth < max_depth && considered.insert(rel_sym.clone()) {
                                trace!(sym = rel_sym.as_str(), "scheduling ontology sym");
                                to_traverse.push((rel_sym.clone(), next_depth, all_traversals_valid));
                            }
                        }
                        // For the case of runnables the override hierarchy is arguably a
                        // distraction from the fundamental control flow going on.
                        //
                        // TODO: Evaluate whether avoiding walking up the override edges is helpful
                        // as implemented here.
                        keep_going = false;
                    }
                }
                if !keep_going {
                    continue;
                }
            }

            if traverse_subclasses && cur_traversals.contains(Traversals::Subclass) {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on meta subclasses"),
                    })
                };

                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .crossref_info
                    .pointer("/meta/subclasses")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();

                for target in overrides.as_array().unwrap() {
                    // subclasses are just the raw symbol, not an object dict.
                    let target_sym_str = target.as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server, next_depth).await?;

                    sym_edge_set.ensure_edge_in_graph(
                        target_id,
                        sym_id.clone(),
                        EdgeKind::Inheritance,
                        vec![],
                        &mut graph,
                    );

                    if next_depth < max_depth && considered.insert(target_info.symbol.clone()) {
                        trace!(sym = target_sym_str, "scheduling subclass");
                        // If we're going in the subclass direction continue only going in the subclass
                        // direction; don't change direction and perform superclass traversals.
                        // XXX we should potentially be tying this into "considered" somehow.
                        to_traverse.push((target_info.symbol.clone(), next_depth, Traversals::Subclass));
                    }
                }
            }

            if traverse_superclasses && cur_traversals.contains(Traversals::Super) {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on meta superclasses"),
                    })
                };

                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .crossref_info
                    .pointer("/meta/supers")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();

                'target: for target in overrides.as_array().unwrap() {
                    // overrides is { sym, pretty, props }
                    let target_sym_str = target["sym"].as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server, next_depth).await?;

                    if let Some(Value::Array(labels_json)) = target_info.crossref_info.pointer("/meta/labels").cloned() {
                        for label in labels_json {
                            if let Value::String(label) = label {
                                if let Some(badge_label) = label.strip_prefix("class-diagram:elide-and-badge:") {
                                    let sym_info = sym_node_set.get_mut(&sym_id);
                                    sym_info.badges.push(SymbolBadge {
                                        pri: 50,
                                        label: ustr(&badge_label),
                                        source_jump: None,
                                    });
                                    continue 'target;
                                }
                            }
                        }
                    }

                    sym_edge_set.ensure_edge_in_graph(
                        target_id,
                        sym_id.clone(),
                        EdgeKind::Inheritance,
                        vec![],
                        &mut graph,
                    );

                    if next_depth < max_depth && considered.insert(target_info.symbol.clone()) {
                        trace!(sym = target_sym_str, "scheduling super");
                        // If we're going in the superclass direction, continue only going in the
                        // superclass direction; don't allow going back down subclasses!
                        // XXX we should potentially be tying this into "considered" somehow
                        to_traverse.push((target_info.symbol.clone(), next_depth, Traversals::Super));
                    }
                }
            }

            if traverse_overrides {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on meta overrides"),
                    })
                };

                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .crossref_info
                    .pointer("/meta/overrides")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();

                for target in overrides.as_array().unwrap() {
                    // overrides is { sym, pretty }
                    let target_sym_str = target["sym"].as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server, next_depth).await?;

                    if considered.insert(target_info.symbol.clone()) {
                        // As a quasi-hack, only add this edge if we didn't
                        // already queue the class for consideration to avoid
                        // getting this edge twice thanks to the reciprocal
                        // relationship we will see when considering it.
                        //
                        // This is only necessary because this is a case
                        // where we are doing bi-directional traversal
                        // because overrides are an equivalence class from
                        // our perspective (right now, before actually
                        // checking the definition of equivalence class. ;)
                        sym_edge_set.ensure_edge_in_graph(
                            target_id,
                            sym_id.clone(),
                            EdgeKind::Inheritance,
                            vec![],
                            &mut graph,
                        );
                        if next_depth < max_depth {
                            trace!(sym = target_sym_str, "scheduling overrides");
                            to_traverse.push((target_info.symbol.clone(), next_depth, all_traversals_valid));
                        }
                    }
                }
            }

            if traverse_overridden_by {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on meta overriddenBy"),
                    })
                };

                let sym_info = sym_node_set.get(&sym_id);
                let overridden_by = sym_info
                    .crossref_info
                    .pointer("/meta/overriddenBy")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();

                for target in overridden_by.as_array().unwrap() {
                    // overriddenBy is just a bare symbol name currently
                    let target_sym_str = target.as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server, next_depth).await?;

                    if considered.insert(target_info.symbol.clone()) {
                        // Same rationale on avoiding a duplicate edge.
                        sym_edge_set.ensure_edge_in_graph(
                            target_id,
                            sym_id.clone(),
                            EdgeKind::Inheritance,
                            vec![],
                            &mut graph,
                        );
                        if next_depth < max_depth {
                            trace!(sym = target_sym_str, "scheduling overridenBy");
                            to_traverse.push((target_info.symbol.clone(), next_depth, all_traversals_valid));
                        }
                    }
                }
            }

            if traverse_callees {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on callees"),
                    })
                };

                let sym_info = sym_node_set.get(&sym_id);
                let callees = sym_info
                    .crossref_info
                    .pointer("/callees")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();

                // Callees are synthetically derived from crossref and is a
                // flat list of { kind, pretty, sym }.  This differs from
                // most other edges which are path hit-lists.
                for target in callees.as_array().unwrap() {
                    let target_sym_str = target["sym"].as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);
                    //let target_kind = target["kind"].as_str().ok_or_else(bad_data)?;

                    let mut edge_info = vec![];
                    if let Some(Value::String(jump)) = target.get("jump") {
                        edge_info.push(EdgeDetail::Jump(jump.clone()));
                    }

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server, next_depth).await?;

                    if target_info.is_callable() {
                        sym_edge_set.ensure_edge_in_graph(
                            sym_id.clone(),
                            target_id,
                            EdgeKind::Default,
                            edge_info,
                            &mut graph,
                        );
                        if next_depth < max_depth && considered.insert(target_info.symbol.clone()) {
                            trace!(sym = target_sym_str, "scheduling callees");
                            to_traverse.push((target_info.symbol.clone(), next_depth, all_traversals_valid));
                        }
                    }
                }
            }

            if traverse_uses {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on callees"),
                    })
                };

                let sym_info = sym_node_set.get(&sym_id);
                let uses_val = sym_info
                    .crossref_info
                    .pointer("/uses")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();
                let uses = uses_val.as_array().unwrap();

                // Do not process the uses if there are more paths than our skip limit.
                if uses.len() as u32 >= self.args.skip_uses_at_path_count {
                    overloads_hit.push(OverloadInfo {
                        kind: OverloadKind::UsesPaths,
                        sym: Some(sym.to_string()),
                        exist: uses.len() as u32,
                        included: 0,
                        local_limit: self.args.skip_uses_at_path_count,
                        global_limit: 0,
                    });
                    continue;
                }

                // We may see a use edge multiple times so we want to suppress it,
                // but we don't want to use `considered` for this because that would
                // hide cycles in the graph!
                let mut use_considered = HashSet::new();

                // Uses are path-hitlists and each array item has the form
                // { path, lines: [ { context, contextsym }] } eliding some
                // of the hit fields.  We really just care about the
                // contextsym.
                for path_hits in uses {
                    let hits = path_hits["lines"].as_array().ok_or_else(bad_data)?;
                    for source in hits {
                        let source_sym_str = source["contextsym"].as_str().unwrap_or("");
                        //let source_kind = source["kind"].as_str().ok_or_else(bad_data)?;

                        if source_sym_str.is_empty() {
                            continue;
                        }
                        let source_sym = ustr(source_sym_str);

                        let (source_id, source_info) =
                            sym_node_set.ensure_symbol(&source_sym, server, next_depth).await?;

                        if source_info.is_callable() {
                            // Only process this given use edge once.
                            if use_considered.insert(source_info.symbol.clone()) {
                                sym_edge_set.ensure_edge_in_graph(
                                    source_id,
                                    sym_id.clone(),
                                    EdgeKind::Default,
                                    vec![],
                                    &mut graph,
                                );
                                if next_depth < max_depth
                                    && considered.insert(source_info.symbol.clone())
                                {
                                    trace!(sym = source_sym_str, "scheduling uses");
                                    to_traverse.push((source_info.symbol.clone(), next_depth, all_traversals_valid));
                                }
                            }
                        }
                    }
                }
            }
        }

        // ## Paths Between
        let graph_coll = if self.args.paths_between {
            // In this case, we don't want our original node set because we
            // expect it to have an order of magnitude more data than we want
            // in the result set.  So we build a new node set and graph.
            let mut paths_node_set = SymbolGraphNodeSet::new();
            let mut paths_edge_set = SymbolGraphEdgeSet::new();
            let mut paths_graph = NamedSymbolGraph::new("paths".to_string());
            let mut suppression = HashSet::new();
            for (source_node, target_node) in root_set
                .iter()
                .tuple_combinations()
                .take(self.args.paths_limit as usize)
            {
                let node_paths = graph.all_simple_paths(source_node.clone(), target_node.clone());
                trace!(path_count = node_paths.len(), "forward paths found");
                sym_node_set.propagate_paths(
                    node_paths,
                    &sym_edge_set,
                    &mut paths_graph,
                    &mut paths_node_set,
                    &mut paths_edge_set,
                    &mut suppression,
                );

                let node_paths = graph.all_simple_paths(target_node.clone(), source_node.clone());
                trace!(path_count = node_paths.len(), "reverse paths found");
                sym_node_set.propagate_paths(
                    node_paths,
                    &sym_edge_set,
                    &mut paths_graph,
                    &mut paths_node_set,
                    &mut paths_edge_set,
                    &mut suppression,
                );
            }
            SymbolGraphCollection {
                node_set: paths_node_set,
                edge_set: paths_edge_set,
                graphs: vec![paths_graph],
                overloads_hit,
                hierarchical_graphs: vec![],
            }
        } else {
            SymbolGraphCollection {
                node_set: sym_node_set,
                edge_set: sym_edge_set,
                graphs: vec![graph],
                overloads_hit,
                hierarchical_graphs: vec![],
            }
        };

        Ok(PipelineValues::SymbolGraphCollection(graph_coll))
    }
}
