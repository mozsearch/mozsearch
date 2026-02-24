use std::collections::{HashSet, VecDeque};
use std::iter::FromIterator;

use async_trait::async_trait;
use bitflags::bitflags;
use clap::Args;
use serde_json::{from_value, json, Value};
use tracing::trace;
use ustr::{ustr, Ustr};

use super::{
    interface::{OverloadInfo, OverloadKind, PipelineCommand, PipelineValues, SymbolMetaFlags},
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
            BindingOwnerLang, BindingSlotKind, OntologySlotInfo, OntologySlotKind,
            StructuredBindingSlotInfo, StructuredFieldInfo,
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

    /// Maximum traversal depth.  This used to have no limit because traversal
    /// will also be constrained by the applicable node-limit and our breadth
    /// first processing, but depth is now also used to limit the paths-between
    /// maximum path length, so we need to cap it to something that avoids worst
    /// case scenarios.  Honestly, 16 might be too high but should only result
    /// in pathological runtime rather than pathological memory usage.
    ///
    /// The default depth is set to 0 so it can vary if paths-between is enabled
    /// although we use a default depth of 8 for both right now, but it might
    /// make sense to crank paths-between back up to 10.
    #[clap(long, short, value_parser = clap::value_parser!(u32).range(0..=16), default_value = "0")]
    max_depth: u32,

    /// When enabled, the traversal will be performed with the higher
    /// paths-between-node-limit in effect, then the roots of the initial
    /// traversal will be used as pair-wise inputs to the all_simple_paths
    /// petgraph algorithm to derive a new graph which will be constrained to
    /// the normal "node-limit".
    #[clap(long, value_parser)]
    paths_between: bool,

    /// If specified, we will not drop symbol data beyond what is required for
    /// jumpref processing once a symbol has been processed.
    #[clap(long, value_parser)]
    retain_all_symbol_data: bool,

    /// Maximum number of nodes in a resulting graph.  When paths are involved,
    /// we may opt to add the entirety of the path that puts the graph over the
    /// node limit rather than omitting it.
    #[clap(long, value_parser = clap::value_parser!(u32).range(16..=1024), default_value = "384")]
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
    ///
    /// Note: we use a default of 0 because we differentiate the default based on
    /// whether paths-between is in use, but will use any value specified here.
    /// We don't impose a limit because our other node limits apply sufficiently well.
    #[clap(long, value_parser, default_value = "0")]
    pub skip_uses_at_path_count: u32,

    /// Traverse field member uses when the depth has this value or less.
    #[clap(long, value_parser = clap::value_parser!(i32).range(-1..=16), default_value = "-1")]
    pub traverse_field_member_uses: i32,

    /// If we see "field-member-uses" with this many hits, do not process any
    /// of the uses.
    #[clap(long, value_parser, default_value = "24")]
    pub skip_field_member_uses_at_count: u32,

    #[clap(long, value_parser)]
    pub ignore_nodes: Option<String>,
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
    #[allow(clippy::match_like_matches_macro)]
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        const DEFAULT_MAX_DEPTH_FROM_TO: u32 = 8;
        const DEFAULT_MAX_DEPTH_BETWEEN: u32 = 8;
        const DEFAULT_SKIP_USES_AT_PATH_COUNT_FROM_TO: u32 = 16;
        const DEFAULT_SKIP_USES_AT_PATH_COUNT_BETWEEN: u32 = 96;

        let default_max_depth = if self.args.paths_between {
            DEFAULT_MAX_DEPTH_BETWEEN
        } else {
            DEFAULT_MAX_DEPTH_FROM_TO
        };

        let default_skip_uses_at_path_count = if self.args.paths_between {
            DEFAULT_SKIP_USES_AT_PATH_COUNT_BETWEEN
        } else {
            DEFAULT_SKIP_USES_AT_PATH_COUNT_FROM_TO
        };

        let max_depth = match self.args.max_depth {
            0 => default_max_depth,
            x => x,
        };
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
        let mut to_traverse = VecDeque::new();
        // Nodes that have been scheduled to be traversed or ruled out.  A node
        // in this set should not be added to `to_traverse`.
        let mut considered = HashSet::new();
        // Root set for paths-between use.
        let mut source_set = vec![];
        let mut target_set = vec![];

        let mut overloads_hit = vec![];

        let all_traversals_valid = Traversals::Super | Traversals::Subclass;

        // Propagate the starting symbols into the graph and queue them up for
        // traversal.
        for info in cil.symbol_crossref_infos {
            let is_source = info.flags.is_empty() || info.flags.contains(SymbolMetaFlags::Source);
            let is_target = info.flags.is_empty() || info.flags.contains(SymbolMetaFlags::Target);

            if is_target {
                to_traverse.push_back((info.symbol, info.get_pretty(), 0, all_traversals_valid));
            }
            considered.insert(info.symbol);

            let (sym_node_id, _info) =
                sym_node_set.add_symbol(DerivedSymbolInfo::new(info.symbol, info.crossref_info, 0));
            // Explicitly put the node in the graph so if we don't find any
            // edges, we still display the node.  This is important for things
            // like "class-diagram" where showing nothing is very confusing.
            graph.ensure_node(sym_node_id.clone());

            // TODO: do something to limit the size of the root-set.  The
            // combinatorial explosion for something like nsGlobalWindowInner is
            // just too silly.  This can added as an overload.
            if is_source {
                source_set.push(sym_node_id.clone());
            }
            if is_target {
                target_set.push(sym_node_id);
            }
        }

        let node_limit = if self.args.paths_between {
            self.args.paths_between_node_limit
        } else {
            self.args.node_limit
        };

        let skip_uses_at_path_count = match self.args.skip_uses_at_path_count {
            0 => default_skip_uses_at_path_count,
            x => x,
        };

        let stop_at_class_label = match self.args.edge.as_str() {
            "callees" => Some("calls-diagram:stop"),
            "class" => Some("class-diagram:stop"),
            "uses" => Some("uses-diagram:stop"),
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
        // Being able to see the field-member-uses is potentially invaluable,
        // but this really needs additional hueristics to be usable, so only
        // turn this on if explicitly specified for now.
        //
        // TODO: Improve heuristics to allow use of field-member-uses.
        //
        // The general issues:
        // - The fan-out is potentially graph-ruining.  Using an example of
        //   `BlobImpl`, we really do want to show `Blob`
        let traverse_field_member_uses = match self.args.edge.as_str() {
            "class" => self.args.traverse_field_member_uses,
            _ => -1,
        };
        let traverse_overridden_by = match self.args.edge.as_str() {
            "inheritance" => true,
            // For callees, if we have traversed to a specific method, we do
            // care about any further overrides.
            "callees" => true,
            _ => false,
        };
        let traverse_overrides = match self.args.edge.as_str() {
            "inheritance" => true,
            "uses" => true,
            // We intentionally do not traverse upwards here for the callees
            // case; only downwards in the "overridden_by" case above.
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

        let ignore_node_set = match &self.args.ignore_nodes {
            Some(s) => HashSet::from_iter(s.split(",")),
            _ => HashSet::new(),
        };

        // General operation:
        // - We pull a node to be traversed off the queue.  This ends up breadth
        //   first.
        // - We check if we already have the crossref info for the symbol and
        //   look it up if not.  There's an asymmetry here between the initial
        //   set of symbols we're traversing from which we already have cached
        //   values for and the new edges we discover, but it's not a concern.
        // - We traverse the list of edges.
        while let Some((sym, pretty, depth, cur_traversals)) = to_traverse.pop_front() {
            if sym_node_set.symbol_crossref_infos.len() as u32 >= node_limit {
                trace!(sym = %sym, depth, "stopping because of node limit");
                overloads_hit.push(OverloadInfo {
                    kind: OverloadKind::NodeLimit,
                    sym: Some(sym.to_string()),
                    pretty: Some(pretty.to_string()),
                    exist: 0,
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
                        if !self.args.retain_all_symbol_data {
                            sym_info.reduce_memory_usage_by_dropping_non_jumpref_info();
                        }
                        continue;
                    }
                }
            }

            // ## Clone the slotOwner now before engaging in additional borrows.
            let slot_owner = sym_info.crossref_info.pointer("/meta/slotOwner").cloned();

            if traverse_fields {
                // Traverse the fields out of this class
                // Note that depth won't stop us from showing a class's fields,
                // just whether we process the target symbol!
                if let Some(fields_json) = sym_info.crossref_info.pointer("/meta/fields").cloned() {
                    let fields: Vec<StructuredFieldInfo> = from_value(fields_json).unwrap();
                    for field in fields {
                        let mut show_field = !field.labels.is_empty();
                        // Attempt to mark the fields with the subsystem of the field's target class
                        // so that we can group fields by which subsystem they're related to.
                        // Because we propagate subsystems from IDL definitions through to their
                        // bindings, this should also end up working for generated bindings.
                        let mut effective_subsystem = None;

                        let mut targets = vec![];
                        for ptr_info in field.pointer_info {
                            show_field = true;
                            let (target_id, target_info) = sym_node_set
                                .ensure_symbol(&ptr_info.sym, server, next_depth)
                                .await?;

                            let target_pretty =
                                match target_info.crossref_info.pointer("/meta/pretty") {
                                    Some(Value::String(pretty)) => ustr(pretty),
                                    _ => ustr(""),
                                };
                            if ignore_node_set.contains(&*target_pretty) {
                                continue;
                            }

                            if next_depth >= max_depth && !considered.contains(&ptr_info.sym) {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnFieldPointer,
                                    sym: Some(ptr_info.sym.to_string()),
                                    pretty: Some(target_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                            } else if next_depth < max_depth && considered.insert(ptr_info.sym) {
                                trace!(sym = ptr_info.sym.as_str(), "scheduling pointee sym");
                                to_traverse.push_back((
                                    ptr_info.sym,
                                    target_pretty,
                                    next_depth,
                                    all_traversals_valid,
                                ));
                            }

                            // In cases where we have multiple pointer_infos for a field, we
                            // arbitrarily picking the first one for now.
                            // XXX For maps, we probably should be favoring the value over the key,
                            // which would suggest we should pick the last va
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
                            let (field_id, field_info) = sym_node_set
                                .ensure_symbol(&field.sym, server, next_depth)
                                .await?;
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

            if depth as i32 <= traverse_field_member_uses {
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
                let member_uses_storage = sym_info
                    .crossref_info
                    .pointer("/field-member-uses")
                    .unwrap_or(&Value::Array(vec![]))
                    .clone();
                let member_uses = member_uses_storage.as_array().unwrap();

                if member_uses.len() as u32 >= self.args.skip_field_member_uses_at_count {
                    overloads_hit.push(OverloadInfo {
                        kind: OverloadKind::FieldMemberUses,
                        sym: Some(sym.to_string()),
                        pretty: Some(pretty.to_string()),
                        exist: member_uses.len() as u32,
                        included: 0,
                        local_limit: self.args.skip_field_member_uses_at_count,
                        global_limit: 0,
                    });
                } else if next_depth >= max_depth {
                    overloads_hit.push(OverloadInfo {
                        kind: OverloadKind::DepthLimitOnFieldMemberUses,
                        sym: Some(sym.to_string()),
                        pretty: Some(pretty.to_string()),
                        exist: next_depth,
                        included: depth + 1,
                        local_limit: 0,
                        global_limit: max_depth,
                    });
                } else {
                    for target in member_uses {
                        // fmu is { sym, pretty, fields }
                        // The sym is the class referencing our type.
                        let target_sym_str = target["sym"].as_str().ok_or_else(bad_data)?;
                        let target_sym = ustr(target_sym_str);

                        let (_target_id, target_info) = sym_node_set
                            .ensure_symbol(&target_sym, server, next_depth)
                            .await?;

                        let target_pretty = match target_info.crossref_info.pointer("/meta/pretty")
                        {
                            Some(Value::String(pretty)) => ustr(pretty),
                            _ => ustr(""),
                        };
                        if ignore_node_set.contains(&*target_pretty) {
                            continue;
                        }

                        // we already considered depth in the outer condition
                        if considered.insert(target_info.symbol) {
                            trace!(sym = target_sym_str, "scheduling field-member-use");
                            to_traverse.push_back((
                                target_info.symbol,
                                target_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
                        }
                    }
                }
            }

            // Check whether to traverse a parent binding slot relationship.
            if let Some(val) = slot_owner {
                let slot_owner: StructuredBindingSlotInfo = from_value(val).unwrap();

                // There are a few possibilities with a binding slot.  It can be
                // a binding type that is:
                //
                // IDL-Based:
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
                //
                // Cross-language bindings like JNI-wrappers:
                //
                // There's a major divergence here between IDL bindings and cross-language
                // bindings like JNI wrappers.  For IDL, the owner slot is the IDL symbol
                // symbol, but for cross-language wrappers, the owner slot is the
                // implementing language.  (That way, if there are multiple language
                // bindings, they all have a parent of the implementing method, and
                // the implementing method has a list of all of the other-language bindings
                // that reference it.)
                let (should_traverse, skip_remainder, traverse_slot, outbound_edge, edge_kind) =
                    match (slot_owner.props.owner_lang, slot_owner.props.slot_kind) {
                        // Enabling funcs and constants don't count as interesting
                        // uses in either direction; they are support.
                        (
                            _,
                            BindingSlotKind::EnablingPref
                            | BindingSlotKind::EnablingFunc
                            | BindingSlotKind::Const,
                        ) => (false, false, None, false, EdgeKind::Default),
                        // For callees, draw an outbound IPC edge to the "recv" slot via the IDL symbol
                        (_, BindingSlotKind::Send) => (
                            self.args.edge == "callees",
                            true,
                            Some("recv"),
                            true,
                            EdgeKind::IPC,
                        ),
                        // For uses, draw an inbound IPC edge from the "send" slot via the IDL symbol
                        (_, BindingSlotKind::Recv) => (
                            self.args.edge == "uses",
                            true,
                            Some("send"),
                            false,
                            EdgeKind::IPC,
                        ),
                        // For IDL bindings, we want an upward (inbound) edge from the IDL symbol
                        (BindingOwnerLang::Idl, _) => {
                            (true, false, None, false, EdgeKind::Implementation)
                        }
                        // Cross-language binding class relationships are weird because
                        // the bindings inside are bidirectional, so let's ignore them.
                        (_, BindingSlotKind::Class) => {
                            (false, false, None, false, EdgeKind::Default)
                        }
                        // This leaves us with cross-language bindings where the slot owner is always the
                        // implementation symbol so slotOwner is always aligned with "callees"; the "uses"
                        // edges hammen when processing bindingSlots.
                        (_, _) => (
                            self.args.edge == "callees",
                            true,
                            None,
                            true,
                            EdgeKind::CrossLanguage,
                        ),
                    };
                if should_traverse {
                    let (owner_id, owner_info) = sym_node_set
                        .ensure_symbol(&slot_owner.sym, server, next_depth)
                        .await?;

                    let owner_pretty = match owner_info.crossref_info.pointer("/meta/pretty") {
                        Some(Value::String(pretty)) => ustr(pretty),
                        _ => ustr(""),
                    };

                    // Handle the case where we need to traverse a slot
                    if let Some(other_slot) = traverse_slot {
                        if let Some(other_sym) = owner_info.get_binding_slot_sym(other_slot) {
                            let (other_id, other_info) = sym_node_set
                                .ensure_symbol(&other_sym, server, next_depth)
                                .await?;

                            let other_pretty =
                                match other_info.crossref_info.pointer("/meta/pretty") {
                                    Some(Value::String(pretty)) => ustr(pretty),
                                    _ => ustr(""),
                                };
                            if ignore_node_set.contains(&*other_pretty) {
                                continue;
                            }

                            if outbound_edge {
                                sym_edge_set.ensure_edge_in_graph(
                                    sym_id.clone(),
                                    other_id,
                                    edge_kind,
                                    vec![],
                                    &mut graph,
                                );
                            } else {
                                sym_edge_set.ensure_edge_in_graph(
                                    other_id,
                                    sym_id.clone(),
                                    edge_kind,
                                    vec![],
                                    &mut graph,
                                );
                            }
                            if next_depth >= max_depth && !considered.contains(&other_info.symbol) {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnBindingSlot,
                                    sym: Some(other_info.symbol.to_string()),
                                    pretty: Some(other_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                            } else if next_depth < max_depth && considered.insert(other_info.symbol)
                            {
                                trace!(
                                    sym = other_info.symbol.as_str(),
                                    "scheduling traversed binding slot sym"
                                );
                                to_traverse.push_back((
                                    other_info.symbol,
                                    other_pretty,
                                    next_depth,
                                    all_traversals_valid,
                                ));
                            }
                        }
                        continue;
                    } else if !ignore_node_set.contains(&*owner_pretty) {
                        if outbound_edge {
                            sym_edge_set.ensure_edge_in_graph(
                                sym_id.clone(),
                                owner_id,
                                edge_kind,
                                vec![],
                                &mut graph,
                            );
                        } else {
                            sym_edge_set.ensure_edge_in_graph(
                                owner_id,
                                sym_id.clone(),
                                edge_kind,
                                vec![],
                                &mut graph,
                            );
                        }
                        if next_depth >= max_depth && !considered.contains(&owner_info.symbol) {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::DepthLimitOnBindingSlot,
                                sym: Some(owner_info.symbol.to_string()),
                                pretty: Some(owner_pretty.to_string()),
                                exist: next_depth,
                                included: depth + 1,
                                local_limit: 0,
                                global_limit: max_depth,
                            });
                        } else if next_depth < max_depth && considered.insert(owner_info.symbol) {
                            trace!(
                                sym = owner_info.symbol.as_str(),
                                "scheduling owner binding slot sym"
                            );
                            to_traverse.push_back((
                                owner_info.symbol,
                                owner_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
                        }
                    }

                    if skip_remainder {
                        // XXX we should potentially be using reduce_memory_usage_by_dropping_non_jumpref_info
                        continue;
                    }
                }
            }

            // Process this symbol's binding slots.  As noted at the `slot_owner` traversal, there's
            // a major different between IDL bindings and cross-language bindings like Java/Kotlin
            // JNI.  There are also some traversals that are pointless for us to consider right now
            // like for XPIDL where in theory XPConnect allows calls between C++ and JS but we don't
            // have the necessary analysis data to handle that.
            let sym_info = sym_node_set.get(&sym_id);
            if let Some(Value::Array(slots)) = sym_info
                .crossref_info
                .pointer("/meta/bindingSlots")
                .cloned()
            {
                let mut skip_after_slots = false;
                for slot_val in slots {
                    let slot: StructuredBindingSlotInfo = from_value(slot_val).unwrap();
                    let (should_traverse, skip_other_edges, outbound_edge, edge_kind) =
                        match (slot.props.owner_lang, slot.props.slot_kind) {
                            // Don't bother with IDL bindings; all the relevant traversals involve a
                            // slotOwner at this time.
                            (BindingOwnerLang::Idl, _) => (false, false, false, EdgeKind::Default),
                            // Cross-language binding class relationships are weird because
                            // the bindings inside are bidirectional, so let's ignore them.
                            (_, BindingSlotKind::Class) => (false, false, false, EdgeKind::Default),
                            // For cross-language wrappers the implementing language is the slotOwner
                            // so the binding slots are edges to the binding.  That is, the slotOwner
                            // constitutes a "uses" edge and the slots constitute a "callees" edge.
                            (_, _) => (
                                self.args.edge == "uses",
                                true,
                                false,
                                EdgeKind::CrossLanguage,
                            ),
                        };
                    if should_traverse {
                        // Skipping is conditional on the decision to traverse.
                        if skip_other_edges {
                            skip_after_slots = true;
                        }
                        let (rel_id, rel_info) = sym_node_set
                            .ensure_symbol(&slot.sym, server, next_depth)
                            .await?;

                        let rel_pretty = match rel_info.crossref_info.pointer("/meta/pretty") {
                            Some(Value::String(pretty)) => ustr(pretty),
                            _ => ustr(""),
                        };
                        if ignore_node_set.contains(&*rel_pretty) {
                            continue;
                        }

                        if outbound_edge {
                            sym_edge_set.ensure_edge_in_graph(
                                sym_id.clone(),
                                rel_id,
                                edge_kind,
                                vec![],
                                &mut graph,
                            );
                        } else {
                            sym_edge_set.ensure_edge_in_graph(
                                rel_id,
                                sym_id.clone(),
                                edge_kind,
                                vec![],
                                &mut graph,
                            );
                        }
                        if next_depth >= max_depth && !considered.contains(&slot.sym) {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::DepthLimitOnBindingSlot,
                                sym: Some(slot.sym.to_string()),
                                pretty: Some(rel_pretty.to_string()),
                                exist: next_depth,
                                included: depth + 1,
                                local_limit: 0,
                                global_limit: max_depth,
                            });
                        } else if next_depth < max_depth && considered.insert(slot.sym) {
                            trace!(sym = slot.sym.as_str(), "scheduling bind slot sym");
                            to_traverse.push_back((
                                slot.sym,
                                rel_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
                        }
                    }
                }
                if skip_after_slots {
                    // XXX we should potentially be using reduce_memory_usage_by_dropping_non_jumpref_info
                    continue;
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
                            let (rel_id, rel_info) = sym_node_set
                                .ensure_symbol(&rel_sym, server, next_depth)
                                .await?;

                            let rel_pretty = match rel_info.crossref_info.pointer("/meta/pretty") {
                                Some(Value::String(pretty)) => ustr(pretty),
                                _ => ustr(""),
                            };
                            if ignore_node_set.contains(&*rel_pretty) {
                                continue;
                            }

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
                            if next_depth >= max_depth && !considered.contains(&rel_sym) {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnOntologySlot,
                                    sym: Some(rel_sym.to_string()),
                                    pretty: Some(rel_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                            } else if next_depth < max_depth && considered.insert(rel_sym) {
                                trace!(sym = rel_sym.as_str(), "scheduling ontology sym");
                                to_traverse.push_back((
                                    rel_sym,
                                    rel_pretty,
                                    next_depth,
                                    all_traversals_valid,
                                ));
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

                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target_sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.crossref_info.pointer("/meta/pretty") {
                        Some(Value::String(pretty)) => ustr(pretty),
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    sym_edge_set.ensure_edge_in_graph(
                        sym_id.clone(),
                        target_id,
                        EdgeKind::Inheritance,
                        vec![],
                        &mut graph,
                    );

                    if next_depth >= max_depth && !considered.contains(&target_info.symbol) {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnSubclass,
                            sym: Some(target_info.symbol.to_string()),
                            pretty: Some(target_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                    } else if next_depth < max_depth && considered.insert(target_info.symbol) {
                        trace!(sym = target_sym_str, "scheduling subclass");
                        // If we're going in the subclass direction continue only going in the subclass
                        // direction; don't change direction and perform superclass traversals.
                        // XXX we should potentially be tying this into "considered" somehow.
                        to_traverse.push_back((
                            target_info.symbol,
                            target_pretty,
                            next_depth,
                            Traversals::Subclass,
                        ));
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

                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target_sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.crossref_info.pointer("/meta/pretty") {
                        Some(Value::String(pretty)) => ustr(pretty),
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    if let Some(Value::Array(labels_json)) =
                        target_info.crossref_info.pointer("/meta/labels").cloned()
                    {
                        for label in labels_json {
                            if let Value::String(label) = label {
                                if let Some(badge_label) =
                                    label.strip_prefix("class-diagram:elide-and-badge:")
                                {
                                    let sym_info = sym_node_set.get_mut(&sym_id);
                                    sym_info.badges.push(SymbolBadge {
                                        pri: 50,
                                        label: ustr(badge_label),
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

                    if next_depth >= max_depth && !considered.contains(&target_info.symbol) {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnSuper,
                            sym: Some(target_info.symbol.to_string()),
                            pretty: Some(target_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                    } else if next_depth < max_depth && considered.insert(target_info.symbol) {
                        trace!(sym = target_sym_str, "scheduling super");
                        // If we're going in the superclass direction, continue only going in the
                        // superclass direction; don't allow going back down subclasses!
                        // XXX we should potentially be tying this into "considered" somehow
                        to_traverse.push_back((
                            target_info.symbol,
                            target_pretty,
                            next_depth,
                            Traversals::Super,
                        ));
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

                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target_sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.crossref_info.pointer("/meta/pretty") {
                        Some(Value::String(pretty)) => ustr(pretty),
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    if considered.insert(target_info.symbol) {
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
                        if next_depth >= max_depth {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::DepthLimitOnOverrides,
                                sym: Some(target_info.symbol.to_string()),
                                pretty: Some(target_pretty.to_string()),
                                exist: next_depth,
                                included: depth + 1,
                                local_limit: 0,
                                global_limit: max_depth,
                            });
                        } else {
                            trace!(sym = target_sym_str, "scheduling overrides");
                            to_traverse.push_back((
                                target_info.symbol,
                                target_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
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

                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target_sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.crossref_info.pointer("/meta/pretty") {
                        Some(Value::String(pretty)) => ustr(pretty),
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    if considered.insert(target_info.symbol) {
                        // Same rationale on avoiding a duplicate edge.
                        sym_edge_set.ensure_edge_in_graph(
                            sym_id.clone(),
                            target_id,
                            EdgeKind::Inheritance,
                            vec![],
                            &mut graph,
                        );
                        if next_depth >= max_depth {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::DepthLimitOnOverriddenBy,
                                sym: Some(target_info.symbol.to_string()),
                                pretty: Some(target_pretty.to_string()),
                                exist: next_depth,
                                included: depth + 1,
                                local_limit: 0,
                                global_limit: max_depth,
                            });
                        } else {
                            trace!(sym = target_sym_str, "scheduling overridenBy");
                            to_traverse.push_back((
                                target_info.symbol,
                                target_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
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

                let sym_info = sym_node_set.get_mut(&sym_id);
                let callees = match (
                    self.args.retain_all_symbol_data,
                    sym_info.crossref_info.get_mut("callees"),
                ) {
                    (true, Some(v)) => match v.clone() {
                        Value::Array(arr) => arr,
                        _ => vec![],
                    },
                    (false, Some(v)) => match v.take() {
                        Value::Array(arr) => arr,
                        _ => vec![],
                    },
                    _ => vec![],
                };

                // Callees are synthetically derived from crossref and is a
                // flat list of { kind, pretty, sym }.  This differs from
                // most other edges which are path hit-lists.
                for target in callees {
                    let target_sym_str = target["sym"].as_str().ok_or_else(bad_data)?;
                    let target_sym = ustr(target_sym_str);
                    //let target_kind = target["kind"].as_str().ok_or_else(bad_data)?;

                    let mut edge_info = vec![];
                    // The jump is precomputed by the crossref process when
                    // deriving the "callees" kindmap entry.
                    if let Some(Value::String(jump)) = target.get("jump") {
                        edge_info.push(EdgeDetail::Jump(jump.clone()));
                    }

                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target_sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.crossref_info.pointer("/meta/pretty") {
                        Some(Value::String(pretty)) => ustr(pretty),
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    if target_info.is_callable() {
                        sym_edge_set.ensure_edge_in_graph(
                            sym_id.clone(),
                            target_id,
                            EdgeKind::Default,
                            edge_info,
                            &mut graph,
                        );
                        if next_depth >= max_depth && !considered.contains(&target_info.symbol) {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::DepthLimitOnCallees,
                                sym: Some(target_info.symbol.to_string()),
                                pretty: Some(target_pretty.to_string()),
                                exist: next_depth,
                                included: depth + 1,
                                local_limit: 0,
                                global_limit: max_depth,
                            });
                        } else if next_depth < max_depth && considered.insert(target_info.symbol) {
                            trace!(sym = target_sym_str, "scheduling callees");
                            to_traverse.push_back((
                                target_info.symbol,
                                target_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
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

                let sym_info = sym_node_set.get_mut(&sym_id);
                let uses = match (
                    self.args.retain_all_symbol_data,
                    sym_info.crossref_info.get_mut("uses"),
                ) {
                    (true, Some(v)) => match v.clone() {
                        Value::Array(arr) => arr,
                        _ => vec![],
                    },
                    (false, Some(v)) => match v.take() {
                        Value::Array(arr) => arr,
                        _ => vec![],
                    },
                    _ => vec![],
                };
                // we just took the uses, but drop the callees too.
                if !self.args.retain_all_symbol_data {
                    sym_info.reduce_memory_usage_by_dropping_non_jumpref_info();
                }

                // Do not process the uses if there are more paths than our skip limit.
                if uses.len() as u32 >= skip_uses_at_path_count {
                    overloads_hit.push(OverloadInfo {
                        kind: OverloadKind::UsesPaths,
                        sym: Some(sym.to_string()),
                        pretty: Some(pretty.to_string()),
                        exist: uses.len() as u32,
                        included: 0,
                        local_limit: skip_uses_at_path_count,
                        global_limit: 0,
                    });
                    continue;
                }

                // We may see a use edge multiple times so we want to suppress it,
                // but we don't want to use `considered` for this because that would
                // hide cycles in the graph!
                let mut use_considered = HashSet::new();

                let mut line_hits: u32 = 0;

                // Uses are path-hitlists and each array item has the form
                // { path, lines: [ { context, contextsym }] } eliding some
                // of the hit fields.  We really just care about the
                // contextsym.
                for path_hits in uses {
                    let path = path_hits["path"].as_str().ok_or_else(bad_data)?;
                    let hits = path_hits["lines"].as_array().ok_or_else(bad_data)?;
                    // For now we're just going to use the path limit for this too.
                    //
                    // The specific scenario driving this is the "abort" method
                    // which ends up called an immense number of times inside of
                    // mfbt/Assertions.h because of assertion macros where each
                    // hit technically has a different contextsym
                    //
                    // First, handle this specific path breaking things for us as
                    // a local limit.  Then add the line count and check if the
                    // global limit has been hit.
                    if hits.len() as u32 >= skip_uses_at_path_count {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::UsesLines,
                            sym: Some(sym.to_string()),
                            pretty: Some(pretty.to_string()),
                            exist: hits.len() as u32,
                            included: 0,
                            local_limit: skip_uses_at_path_count,
                            global_limit: 0,
                        });
                        break;
                    }
                    line_hits += hits.len() as u32;
                    if line_hits >= skip_uses_at_path_count {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::UsesLines,
                            sym: Some(sym.to_string()),
                            pretty: Some(pretty.to_string()),
                            exist: line_hits,
                            included: line_hits - (hits.len() as u32),
                            local_limit: 0,
                            // Note we're reporting this as a global limit to
                            // differentiate from the above case.
                            global_limit: skip_uses_at_path_count,
                        });
                        break;
                    }
                    for source in hits {
                        let source_sym_str = source["contextsym"].as_str().unwrap_or("");
                        //let source_kind = source["kind"].as_str().ok_or_else(bad_data)?;

                        if source_sym_str.is_empty() {
                            continue;
                        }
                        let source_sym = ustr(source_sym_str);

                        let (source_id, source_info) = sym_node_set
                            .ensure_symbol(&source_sym, server, next_depth)
                            .await?;

                        let source_pretty = match source_info.crossref_info.pointer("/meta/pretty")
                        {
                            Some(Value::String(pretty)) => ustr(pretty),
                            _ => ustr(""),
                        };
                        if ignore_node_set.contains(&*source_pretty) {
                            continue;
                        }

                        if source_info.is_callable() {
                            // We call this even if our check below determines we've already created
                            // and traversed this edge because we want to merge in edge detail
                            // information.
                            let jump = format!("{}#{}", path, source["lno"].as_u64().unwrap_or(0));
                            sym_edge_set.ensure_edge_in_graph(
                                source_id,
                                sym_id.clone(),
                                EdgeKind::Default,
                                vec![EdgeDetail::Jump(jump)],
                                &mut graph,
                            );
                            // Only traverse the edge once.
                            if next_depth >= max_depth
                                && !use_considered.contains(&source_info.symbol)
                                && !considered.contains(&source_info.symbol)
                            {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnUses,
                                    sym: Some(source_info.symbol.to_string()),
                                    pretty: Some(source_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                            } else if use_considered.insert(source_info.symbol)
                                && next_depth < max_depth
                                && considered.insert(source_info.symbol)
                            {
                                trace!(sym = source_sym_str, "scheduling uses");
                                to_traverse.push_back((
                                    source_info.symbol,
                                    source_pretty,
                                    next_depth,
                                    all_traversals_valid,
                                ));
                            }
                        }
                    }
                }
            } else if !self.args.retain_all_symbol_data {
                let sym_info = sym_node_set.get_mut(&sym_id);
                sym_info.reduce_memory_usage_by_dropping_non_jumpref_info();
            }
        }

        let mut traverse_options = vec![];
        traverse_options.push(json!({
            "name": "depth",
            "label": "Depth",
            "value": max_depth,
            "default": default_max_depth,
            "range": [0, 16],
        }));
        if self.args.paths_between {
            traverse_options.push(json!({
                "name": "paths-between-node-limit",
                "label": "Node limit",
                "value": self.args.paths_between_node_limit,
                "default": 8192,
                "range": [16, 16384],
            }));
        } else {
            traverse_options.push(json!({
                "name": "node-limit",
                "label": "Node limit",
                "value": self.args.node_limit,
                "default": 384,
                "range": [16, 1024],
            }));
        }
        traverse_options.push(json!({
            "name": "path-limit",
            "label": "Path limit",
            "value": skip_uses_at_path_count,
            "default": default_skip_uses_at_path_count,
            "range": [0, 16384],
        }));
        if self.args.edge.as_str() == "class" {
            traverse_options.push(json!({
                "name": "fmus-through-depth",
                "label": "Field member uses",
                "value": self.args.traverse_field_member_uses,
                "default": -1,
                "range": [-1, 16],
            }));
        }
        traverse_options.push(json!({
            "name": "ignore-nodes",
            "label": "Ignore nodes",
            "value": match &self.args.ignore_nodes {
                Some(s) => s.clone(),
                _ => "".to_string(),
            },
            "default": "",
            "type": "string",
            "placeholder": "Comma-separated pretty names",
        }));
        let options = json!([
            {
                "section": "Traverse",
                "items": traverse_options,
            }
        ]);

        // ## Paths Between
        let graph_coll = if self.args.paths_between {
            // In this case, we don't want our original node set because we
            // expect it to have an order of magnitude more data than we want
            // in the result set.  So we build a new node set and graph.
            let mut paths_node_set = SymbolGraphNodeSet::new();
            let mut paths_edge_set = SymbolGraphEdgeSet::new();
            let mut paths_graph = NamedSymbolGraph::new("paths".to_string());

            trace!("performing path propagation");
            sym_node_set.propagate_paths(
                &mut graph,
                &source_set,
                &target_set,
                &sym_edge_set,
                // We've relaxed our paths-between node limit and would like to keep it that way,
                // but we definitely need to limit the resulting size of the graph, so we still need
                // to have a node limit, so we use the non-paths-between node limit (which can be
                // raised) for that.
                self.args.node_limit,
                // There's no point considering paths longer than the max depth.
                max_depth,
                &mut paths_graph,
                &mut paths_node_set,
                &mut paths_edge_set,
            );
            if paths_node_set.symbol_crossref_infos.len() as u32 >= self.args.node_limit {
                overloads_hit.push(OverloadInfo {
                    kind: OverloadKind::NodeLimit,
                    sym: None,
                    pretty: None,
                    // We don't know how many there might have been as we did a soft limit
                    // stop while propagating, so say 0 for exist but how many
                    // we included.
                    exist: 0,
                    included: paths_node_set.symbol_crossref_infos.len() as u32,
                    local_limit: 0,
                    global_limit: self.args.node_limit,
                });
            }

            SymbolGraphCollection {
                node_set: paths_node_set,
                edge_set: paths_edge_set,
                graphs: vec![paths_graph],
                overloads_hit,
                options,
                hierarchical_graphs: vec![],
            }
        } else {
            SymbolGraphCollection {
                node_set: sym_node_set,
                edge_set: sym_edge_set,
                graphs: vec![graph],
                overloads_hit,
                options,
                hierarchical_graphs: vec![],
            }
        };

        Ok(PipelineValues::SymbolGraphCollection(graph_coll))
    }
}
