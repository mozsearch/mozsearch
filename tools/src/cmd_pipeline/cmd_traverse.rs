use std::collections::{HashMap, HashSet, VecDeque};

use async_trait::async_trait;
use bitflags::bitflags;
use clap::Args;
use serde_json::json;
use tracing::trace;
use ustr::{Ustr, ustr};

use super::{
    interface::{OverloadInfo, OverloadKind, PipelineCommand, PipelineValues, SymbolMetaFlags},
    symbol_graph::{
        DerivedSymbolInfo, NamedSymbolGraph, SymbolBadge, SymbolGraphCollection,
        SymbolGraphEdgeSet, SymbolGraphNodeId, SymbolGraphNodeSet,
    },
};

use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    cmd_pipeline::symbol_graph::{EdgeDetail, EdgeKind},
    file_format::{
        analysis::{BindingOwnerLang, BindingSlotKind, OntologySlotKind, PathSearchResult},
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

    /// When traversing a symbol's uses, include at most this many of its
    /// callers (the distinct callable contexts of its uses); see
    /// `OverloadKind::UsesCallers`.
    ///
    /// Note: we use a default of 0 because we differentiate the default based on
    /// whether paths-between is in use, but will use any value specified here.
    /// We don't impose a limit because our other node limits apply sufficiently well.
    #[clap(long, value_parser, default_value = "0")]
    pub caller_limit: u32,

    /// Traverse field member uses when the depth has this value or less.
    #[clap(long, value_parser = clap::value_parser!(i32).range(-1..=16), default_value = "-1")]
    pub traverse_field_member_uses: i32,

    /// If we see "field-member-uses" with this many hits, do not process any
    /// of the uses.
    #[clap(long, value_parser, default_value = "24")]
    pub skip_field_member_uses_at_count: u32,

    /// Pretty names of nodes to leave out, separated by commas (see
    /// `split_ignore_nodes`).
    #[clap(long, value_parser)]
    pub ignore_nodes: Option<String>,

    /// For `--paths-between` over uses: only follow callers that are
    /// methods of classes on a path between the sources' and targets'
    /// classes in crossref's call roll-ups (see `file_format::rollup`), or
    /// that aren't methods, so the traversal needn't look up callers that
    /// can't lead to a source.  (Roll-ups keep only classes' biggest edges,
    /// so a path through a class's weak edges can be missed.)
    #[clap(long)]
    pub rollup_guide: bool,
}

/// The classes on paths from `sources`' to `targets'` (by their call
/// roll-ups' edges, up to `depth` long), and the sources and targets.
async fn rollup_guided_classes(
    server: &(dyn AbstractServer + Send + Sync),
    sources: &[Ustr],
    targets: &[Ustr],
    depth: u32,
) -> Result<HashSet<Ustr>> {
    // (Breadth-first from each side, along callers' or callees' classes.)
    let mut reached: [HashSet<Ustr>; 2] = [HashSet::new(), HashSet::new()];
    for (side, starts) in [sources, targets].into_iter().enumerate() {
        let mut frontier: Vec<Ustr> = starts.to_vec();
        reached[side].extend(starts.iter().copied());
        for _ in 0..depth {
            let mut next = vec![];
            for class in &frontier {
                let Some(data) = server.crossref_lookup(class).await? else {
                    continue;
                };
                let mut neighbors: Vec<Ustr> = vec![];
                if let Some(rollup) = &data.call_rollup {
                    let edges = if side == 0 { &rollup.out } else { &rollup.into };
                    neighbors.extend(edges.iter().map(|edge| edge.sym));
                }
                // (Calls of a class's methods can be of its supers' and
                // subclasses' (virtual) methods, which traversals follow.)
                if let Some(meta) = &data.meta {
                    neighbors.extend(meta.supers.iter().map(|s| s.sym));
                    neighbors.extend(meta.subclass_syms.iter().copied());
                }
                for neighbor in neighbors {
                    if reached[side].insert(neighbor) {
                        next.push(neighbor);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
    }
    let mut allowed: HashSet<Ustr> = reached[0].intersection(&reached[1]).copied().collect();
    allowed.extend(sources.iter().copied());
    allowed.extend(targets.iter().copied());
    Ok(allowed)
}

/// The class a symbol is (for classes) or is a method of, by its structured
/// record.
fn class_of_info(info: &DerivedSymbolInfo) -> Option<Ustr> {
    let meta = info.get_structured()?;
    if meta.kind == "method" {
        meta.parent_sym
    } else if is_class_kind(&meta.kind) {
        Some(meta.sym)
    } else {
        None
    }
}

fn is_class_kind(kind: &str) -> bool {
    matches!(kind, "class" | "struct" | "union" | "interface")
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
        const DEFAULT_CALLER_LIMIT_FROM_TO: u32 = 16;
        const DEFAULT_CALLER_LIMIT_BETWEEN: u32 = 96;

        let default_max_depth = if self.args.paths_between {
            DEFAULT_MAX_DEPTH_BETWEEN
        } else {
            DEFAULT_MAX_DEPTH_FROM_TO
        };

        let default_caller_limit = if self.args.paths_between {
            DEFAULT_CALLER_LIMIT_BETWEEN
        } else {
            DEFAULT_CALLER_LIMIT_FROM_TO
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

        let mut source_syms = vec![];
        let mut target_syms = vec![];

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

            let (sym_node_id, _info) = sym_node_set.add_symbol(DerivedSymbolInfo::new(
                info.symbol,
                Some(info.crossref_info),
                0,
            ));
            // Explicitly put the node in the graph so if we don't find any
            // edges, we still display the node.  This is important for things
            // like "class-diagram" where showing nothing is very confusing.
            graph.ensure_node(sym_node_id.clone());

            // TODO: do something to limit the size of the root-set.  The
            // combinatorial explosion for something like nsGlobalWindowInner is
            // just too silly.  This can added as an overload.
            if is_source {
                source_set.push(sym_node_id.clone());
                source_syms.push(info.symbol.clone());
            }
            if is_target {
                target_set.push(sym_node_id);
                target_syms.push(info.symbol.clone());
            }
        }

        let node_limit = if self.args.paths_between {
            self.args.paths_between_node_limit
        } else {
            self.args.node_limit
        };

        let caller_limit = match self.args.caller_limit {
            0 => default_caller_limit,
            x => x,
        };

        // (The classes worth following callers in, for `--rollup-guide`.)
        let guided_classes = if self.args.rollup_guide && self.args.paths_between {
            let classes = |set: &Vec<SymbolGraphNodeId>| -> Vec<Ustr> {
                let mut classes: Vec<Ustr> = set
                    .iter()
                    .filter_map(|id| class_of_info(sym_node_set.get(id)))
                    .collect();
                classes.sort();
                classes.dedup();
                classes
            };
            let sources = classes(&source_set);
            let targets = classes(&target_set);
            if sources.is_empty() || targets.is_empty() {
                None
            } else {
                Some(rollup_guided_classes(server, &sources, &targets, max_depth).await?)
            }
        } else {
            None
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

        let ignore_node_set: HashSet<String> = match &self.args.ignore_nodes {
            Some(s) => split_ignore_nodes(s).into_iter().collect(),
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
        // (How many of the queue's first items' symbols were prefetched: once
        // they're processed, the items they queued, the next depth's, are, at
        // once; see `AbstractServer::prefetch_crossref`.)
        let mut prefetched = 0;
        loop {
            if prefetched == 0 {
                let room =
                    (node_limit as usize).saturating_sub(sym_node_set.symbol_crossref_infos.len());
                let syms: Vec<Ustr> = to_traverse
                    .iter()
                    .take(room)
                    .map(|(sym, ..)| *sym)
                    .collect();
                server.prefetch_crossref(&syms).await;
                prefetched = to_traverse.len();
            }
            let Some((sym, pretty, depth, cur_traversals)) = to_traverse.pop_front() else {
                break;
            };
            prefetched -= 1;
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

            if let Some(stop_at_label) = &stop_at_class_label
                && let Some(labels) = sym_info.get_structured().map(|meta| &meta.labels).clone()
            {
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

            // ## Clone the slotOwner now before engaging in additional borrows.
            let slot_owner = sym_info
                .get_structured()
                .and_then(|meta| meta.slot_owner.as_ref())
                .cloned();

            if traverse_fields {
                // Traverse the fields out of this class
                // Note that depth won't stop us from showing a class's fields,
                // just whether we process the target symbol!
                if let Some(fields) = sym_info.get_structured().map(|meta| &meta.fields).cloned() {
                    for field in fields {
                        let mut show_field = !field.labels.is_empty();
                        // Attempt to mark the fields with the subsystem of the field's target class
                        // so that we can group fields by which subsystem they're related to.
                        // Because we propagate subsystems from IDL definitions through to their
                        // bindings, this should also end up working for generated bindings.
                        let mut effective_subsystem = None;

                        let mut targets = vec![];
                        for ptr_info in &field.pointer_info {
                            show_field = true;
                            let (target_id, target_info) = sym_node_set
                                .ensure_symbol(&ptr_info.sym, server, next_depth)
                                .await?;

                            let target_pretty =
                                match target_info.get_structured().map(|meta| meta.pretty) {
                                    Some(pretty) => pretty,
                                    _ => ustr(""),
                                };
                            if ignore_node_set.contains(&*target_pretty) {
                                continue;
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

                            if !considered.insert(ptr_info.sym) {
                                continue;
                            }
                            if next_depth >= max_depth {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnFieldPointer,
                                    sym: Some(ptr_info.sym.to_string()),
                                    pretty: Some(target_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                                continue;
                            }
                            trace!(sym = ptr_info.sym.as_str(), "scheduling pointee sym");
                            to_traverse.push_back((
                                ptr_info.sym,
                                target_pretty,
                                next_depth,
                                all_traversals_valid,
                            ));
                        }

                        if !show_field {
                            continue;
                        }
                        let (field_id, field_info) = sym_node_set
                            .ensure_symbol(&field.sym, server, next_depth)
                            .await?;
                        field_info.effective_subsystem = effective_subsystem;
                        for label in &field.labels {
                            // XXX like above, consider moving the emoji label mapping here to
                            // the ontology file or elsewhere.
                            let Some((pri, shorter_label)) = label_to_badge_info(&label) else {
                                continue;
                            };
                            field_info.badges.push(SymbolBadge {
                                pri,
                                label: ustr(shorter_label),
                                source_jump: None,
                            });
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

            if depth as i32 <= traverse_field_member_uses {
                // Find the places where this type is used as a field member.
                //
                // A hack/simplification we do here is just add the class and leave
                // it up to the traversal of the class to generate the field edge
                // for us.  We don't need to worry about weirdness with the depth
                // threshold here because our logic above will always process the
                // class's fields; the field traversal is not a separate step with
                // its own depth addition.
                let sym_info = sym_node_set.get(&sym_id);
                let member_uses = sym_info
                    .crossref_info
                    .as_ref()
                    .and_then(|ci| ci.field_member_uses.clone())
                    .unwrap_or_default();

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
                        let (_target_id, target_info) = sym_node_set
                            .ensure_symbol(&target.sym, server, next_depth)
                            .await?;

                        let target_pretty =
                            match target_info.get_structured().map(|meta| meta.pretty) {
                                Some(pretty) => pretty,
                                _ => ustr(""),
                            };
                        if ignore_node_set.contains(&*target_pretty) {
                            continue;
                        }

                        // we already considered depth in the outer condition
                        if !considered.insert(target_info.symbol) {
                            continue;
                        }
                        trace!(sym = target.sym.as_str(), "scheduling field-member-use");
                        to_traverse.push_back((
                            target_info.symbol,
                            target_pretty,
                            next_depth,
                            all_traversals_valid,
                        ));
                    }
                }
            }

            // Check whether to traverse a parent binding slot relationship.
            if let Some(slot_owner) = slot_owner {
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
                            Some(BindingSlotKind::Recv),
                            true,
                            EdgeKind::IPC,
                        ),
                        // For uses, draw an inbound IPC edge from the "send" slot via the IDL symbol
                        (_, BindingSlotKind::Recv) => (
                            self.args.edge == "uses",
                            true,
                            Some(BindingSlotKind::Send),
                            false,
                            EdgeKind::IPC,
                        ),
                        // For IDL bindings, we want an upward (inbound) edge from the IDL symbol
                        (BindingOwnerLang::Idl, _) => {
                            (true, false, None, false, EdgeKind::Implementation)
                        }
                        (BindingOwnerLang::Glean, _) => {
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

                    let owner_pretty = match owner_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
                        _ => ustr(""),
                    };

                    // Handle the case where we need to traverse a slot: each of
                    // its symbols (ex: a message's `Send` methods' overloads,
                    // and their symbols on each platform).
                    if let Some(other_slot) = traverse_slot {
                        for other_sym in owner_info.get_binding_slot_syms(other_slot) {
                            let (other_id, other_info) = sym_node_set
                                .ensure_symbol(&other_sym, server, next_depth)
                                .await?;

                            let other_pretty =
                                match other_info.get_structured().map(|meta| meta.pretty) {
                                    Some(pretty) => pretty,
                                    _ => ustr(""),
                                };
                            if ignore_node_set.contains(&*other_pretty) {
                                continue;
                            }

                            if outbound_edge {
                                sym_edge_set.ensure_edge_in_graph(
                                    sym_id.clone(),
                                    other_id,
                                    edge_kind.clone(),
                                    vec![],
                                    &mut graph,
                                );
                            } else {
                                sym_edge_set.ensure_edge_in_graph(
                                    other_id,
                                    sym_id.clone(),
                                    edge_kind.clone(),
                                    vec![],
                                    &mut graph,
                                );
                            }
                            if !considered.insert(other_info.symbol) {
                                continue;
                            }
                            if next_depth >= max_depth {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnBindingSlot,
                                    sym: Some(other_info.symbol.to_string()),
                                    pretty: Some(other_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                                continue;
                            }
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
                        continue;
                    }
                    if !ignore_node_set.contains(&*owner_pretty) {
                        'block: {
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
                            if !considered.insert(owner_info.symbol) {
                                break 'block;
                            }
                            if next_depth >= max_depth {
                                overloads_hit.push(OverloadInfo {
                                    kind: OverloadKind::DepthLimitOnBindingSlot,
                                    sym: Some(owner_info.symbol.to_string()),
                                    pretty: Some(owner_pretty.to_string()),
                                    exist: next_depth,
                                    included: depth + 1,
                                    local_limit: 0,
                                    global_limit: max_depth,
                                });
                                break 'block;
                            }
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
            if let Some(slots) = sym_info
                .get_structured()
                .map(|meta| &meta.binding_slots)
                .cloned()
            {
                let mut skip_after_slots = false;
                for slot in &slots {
                    let (should_traverse, skip_other_edges, outbound_edge, edge_kind) =
                        match (slot.props.owner_lang, slot.props.slot_kind) {
                            // Don't bother with IDL bindings; all the relevant traversals involve a
                            // slotOwner at this time.
                            (BindingOwnerLang::Idl, _) => (false, false, false, EdgeKind::Default),
                            (BindingOwnerLang::Glean, _) => {
                                (false, false, false, EdgeKind::Default)
                            }
                            // An event's (ex: an observer topic's) callees are its receivers,
                            // and its uses are its senders, rather than its uses' contexts,
                            // so that a diagram goes from the functions sending it, through
                            // it, to the methods receiving it.
                            (BindingOwnerLang::Event, BindingSlotKind::Recv) => {
                                (self.args.edge == "callees", true, true, EdgeKind::IPC)
                            }
                            (BindingOwnerLang::Event, BindingSlotKind::Send) => {
                                (self.args.edge == "uses", true, false, EdgeKind::IPC)
                            }
                            (BindingOwnerLang::Event, _) => {
                                (false, false, false, EdgeKind::Default)
                            }
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
                    if !should_traverse {
                        continue;
                    }
                    // Skipping is conditional on the decision to traverse.
                    if skip_other_edges {
                        skip_after_slots = true;
                    }
                    let (rel_id, rel_info) = sym_node_set
                        .ensure_symbol(&slot.sym, server, next_depth)
                        .await?;

                    let rel_pretty = match rel_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
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
                    if !considered.insert(slot.sym) {
                        continue;
                    }
                    if next_depth >= max_depth {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnBindingSlot,
                            sym: Some(slot.sym.to_string()),
                            pretty: Some(rel_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                        continue;
                    }
                    trace!(sym = slot.sym.as_str(), "scheduling bind slot sym");
                    to_traverse.push_back((slot.sym, rel_pretty, next_depth, all_traversals_valid));
                }
                if skip_after_slots {
                    // XXX we should potentially be using reduce_memory_usage_by_dropping_non_jumpref_info
                    continue;
                }
            }

            // Check whether we have any ontology shortcuts to handle.
            let sym_info = sym_node_set.get(&sym_id);
            if let Some(slots) = sym_info
                .get_structured()
                .map(|meta| &meta.ontology_slots)
                .cloned()
            {
                let mut keep_going = true;
                for slot in &slots {
                    let (should_traverse, upwards) = match slot.slot_kind {
                        OntologySlotKind::RunnableConstructor => (self.args.edge == "uses", true),
                        OntologySlotKind::RunnableMethod => (self.args.edge == "callees", false),
                    };
                    if !should_traverse {
                        continue;
                    }
                    for rel_sym in &slot.syms {
                        let (rel_id, rel_info) = sym_node_set
                            .ensure_symbol(&rel_sym, server, next_depth)
                            .await?;

                        let rel_pretty = match rel_info.get_structured().map(|meta| meta.pretty) {
                            Some(pretty) => pretty,
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
                        if !considered.insert(*rel_sym) {
                            continue;
                        }
                        if next_depth >= max_depth {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::DepthLimitOnOntologySlot,
                                sym: Some(rel_sym.to_string()),
                                pretty: Some(rel_pretty.to_string()),
                                exist: next_depth,
                                included: depth + 1,
                                local_limit: 0,
                                global_limit: max_depth,
                            });
                            continue;
                        }
                        trace!(sym = rel_sym.as_str(), "scheduling ontology sym");
                        to_traverse.push_back((
                            *rel_sym,
                            rel_pretty,
                            next_depth,
                            all_traversals_valid,
                        ));
                    }
                    // For the case of runnables the override hierarchy is arguably a
                    // distraction from the fundamental control flow going on.
                    //
                    // TODO: Evaluate whether avoiding walking up the override edges is helpful
                    // as implemented here.
                    keep_going = false;
                }
                if !keep_going {
                    continue;
                }
            }

            if traverse_subclasses && cur_traversals.contains(Traversals::Subclass) {
                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .get_structured()
                    .map(|meta| meta.subclass_syms.clone())
                    .unwrap_or_default();

                for target in overrides {
                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
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
                    if !considered.insert(target_info.symbol) {
                        continue;
                    }
                    if next_depth >= max_depth {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnSubclass,
                            sym: Some(target_info.symbol.to_string()),
                            pretty: Some(target_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                        continue;
                    }
                    trace!(sym = target.as_str(), "scheduling subclass");
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

            if traverse_superclasses && cur_traversals.contains(Traversals::Super) {
                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .get_structured()
                    .map(|meta| meta.supers.clone())
                    .unwrap_or_default();

                'target: for target in overrides {
                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target.sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    if let Some(labels) = target_info.get_structured().map(|meta| &meta.labels) {
                        for &label in labels {
                            let Some(badge_label) =
                                label.strip_prefix("class-diagram:elide-and-badge:")
                            else {
                                continue;
                            };
                            let sym_info = sym_node_set.get_mut(&sym_id);
                            sym_info.badges.push(SymbolBadge {
                                pri: 50,
                                label: ustr(badge_label),
                                source_jump: None,
                            });
                            continue 'target;
                        }
                    }

                    sym_edge_set.ensure_edge_in_graph(
                        target_id,
                        sym_id.clone(),
                        EdgeKind::Inheritance,
                        vec![],
                        &mut graph,
                    );

                    if !considered.insert(target_info.symbol) {
                        continue;
                    }
                    if next_depth >= max_depth {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnSuper,
                            sym: Some(target_info.symbol.to_string()),
                            pretty: Some(target_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                        continue;
                    }
                    trace!(sym = target.sym.as_str(), "scheduling super");
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

            if traverse_overrides {
                let sym_info = sym_node_set.get(&sym_id);
                let overrides = sym_info
                    .get_structured()
                    .map(|meta| meta.overrides.clone())
                    .unwrap_or_default();

                for target in overrides {
                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target.sym, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }
                    // (Not up to IPDL's generated virtual `Recv` methods,
                    // whose uses are its generated dispatch: a `Recv`
                    // method's IPC edges from its `Send` methods are its
                    // control flow.  But up to those with the IPC slots, of
                    // the protocols whose actors IPDL still calls virtually;
                    // see ipdl-analyze.)
                    if self.args.edge == "uses"
                        && target_info.is_ipdl_generated()
                        && target_info
                            .get_structured()
                            .is_some_and(|meta| meta.slot_owner.is_none())
                    {
                        continue;
                    }

                    if !considered.insert(target_info.symbol) {
                        continue;
                    }
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
                        continue;
                    }
                    trace!(sym = target.sym.as_str(), "scheduling overrides");
                    to_traverse.push_back((
                        target_info.symbol,
                        target_pretty,
                        next_depth,
                        all_traversals_valid,
                    ));
                }
            }

            if traverse_overridden_by {
                let sym_info = sym_node_set.get(&sym_id);
                let overridden_by = sym_info
                    .get_structured()
                    .map(|meta| meta.overridden_by_syms.clone())
                    .unwrap_or_default();

                for target in overridden_by {
                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol(&target, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }

                    if !considered.insert(target_info.symbol) {
                        continue;
                    }
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
                        continue;
                    }
                    trace!(sym = target.as_str(), "scheduling overridenBy");
                    to_traverse.push_back((
                        target_info.symbol,
                        target_pretty,
                        next_depth,
                        all_traversals_valid,
                    ));
                }
            }

            if traverse_callees {
                let sym_info = sym_node_set.get_mut(&sym_id);
                let callees = match (
                    self.args.retain_all_symbol_data,
                    sym_info
                        .crossref_info
                        .as_mut()
                        .and_then(|ci| ci.callees.as_mut()),
                ) {
                    (true, Some(v)) => v.clone(),
                    (false, Some(v)) => core::mem::take(v),
                    _ => vec![],
                };

                // Callees are synthetically derived from crossref and is a
                // flat list of { kind, pretty, sym }.  This differs from
                // most other edges which are path hit-lists.
                for target in callees {
                    let mut edge_info = vec![];
                    // The jump is precomputed by the crossref process when
                    // deriving the "callees" kindmap entry.
                    edge_info.push(EdgeDetail::Jump(target.jump.clone()));
                    // (A linked tree's callee's jump is to its files.)
                    if let Some(tree) = target.tree {
                        edge_info.push(EdgeDetail::Tree(tree.to_string()));
                    }

                    // (A linked tree's callee is its symbol.)
                    let (target_id, target_info) = sym_node_set
                        .ensure_symbol_from(&target.sym, target.tree, server, next_depth)
                        .await?;

                    let target_pretty = match target_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*target_pretty) {
                        continue;
                    }
                    // (Plumbing labeled to be left out of calls diagrams.)
                    if target_info.has_label("calls-diagram:skip") {
                        continue;
                    }

                    if !target_info.is_callable() {
                        continue;
                    }
                    // (A citation of a callable symbol, ex: a Gecko function's
                    // citation of the web spec algorithm it implements, is an
                    // implementation of it; see `Callee::implements`.)
                    let edge_kind = if target.implements {
                        EdgeKind::Implementation
                    } else {
                        EdgeKind::Default
                    };
                    sym_edge_set.ensure_edge_in_graph(
                        sym_id.clone(),
                        target_id,
                        edge_kind,
                        edge_info,
                        &mut graph,
                    );
                    if !considered.insert(target_info.symbol) {
                        continue;
                    }
                    if next_depth >= max_depth {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnCallees,
                            sym: Some(target_info.symbol.to_string()),
                            pretty: Some(target_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                        continue;
                    }
                    trace!(sym = target.sym.as_str(), "scheduling callees");
                    to_traverse.push_back((
                        target_info.symbol,
                        target_pretty,
                        next_depth,
                        all_traversals_valid,
                    ));
                }
            }

            if traverse_uses {
                let sym_info = sym_node_set.get_mut(&sym_id);
                let uses = match (
                    self.args.retain_all_symbol_data,
                    sym_info
                        .crossref_info
                        .as_mut()
                        .and_then(|ci| ci.uses.as_mut()),
                ) {
                    (true, Some(v)) => v.clone(),
                    (false, Some(v)) => core::mem::take(v),
                    _ => vec![],
                };
                // A callable symbol's citations (ex: a web spec's algorithm's,
                // by the Gecko functions implementing it, in a linked tree)
                // are implementations of it (see `Callee::implements`).
                let implementations: Vec<PathSearchResult> = if sym_info.is_implementable() {
                    sym_info
                        .crossref_info
                        .as_ref()
                        .and_then(|ci| ci.doc_uses.clone())
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                // we just took the uses, but drop the callees too.
                if !self.args.retain_all_symbol_data {
                    sym_info.reduce_memory_usage_by_dropping_non_jumpref_info();
                }

                // The symbol's callers: the distinct contexts of its uses (by
                // their contextsyms, and their trees for linked trees' hits),
                // in the order of their first uses, with where they use it.
                // (Uses without contexts, ex: at a file's top level, have
                // none.)  Many uses by few callers (ex: a web spec algorithm's
                // in the one file of its spec) are fine; many callers aren't.
                type CallerKey = (Ustr, Option<Ustr>, bool);
                let mut callers: Vec<(CallerKey, Vec<EdgeDetail>)> = vec![];
                let mut caller_indices: HashMap<CallerKey, usize> = HashMap::new();
                let uses = uses
                    .into_iter()
                    .map(|hits| (hits, false))
                    .chain(implementations.into_iter().map(|hits| (hits, true)));
                for (path_hits, implements) in uses {
                    for hit in &path_hits.lines {
                        if hit.contextsym.is_empty() {
                            continue;
                        }
                        let key = (hit.contextsym, path_hits.tree, implements);
                        let index = *caller_indices.entry(key).or_insert_with(|| {
                            // (A linked tree's hits' contexts are its symbols.)
                            let details = match path_hits.tree {
                                Some(tree) => vec![EdgeDetail::Tree(tree.to_string())],
                                None => vec![],
                            };
                            callers.push((key, details));
                            callers.len() - 1
                        });
                        callers[index].1.push(EdgeDetail::Jump(format!(
                            "{}#{}",
                            path_hits.path, hit.lineno
                        )));
                    }
                }

                // Include the callable callers, up to the caller limit, looking
                // them up in order (up to the lookup limit, for symbols used in
                // many contexts which mostly aren't callable, ex: a web spec
                // algorithm's in sections of many specs); the rest are left
                // out (see `OverloadKind::UsesCallers`).
                let lookup_limit = caller_limit.saturating_mul(8).max(128);
                // (The first callers' entries, at once: as many as can be
                // included.)
                let first: Vec<Ustr> = callers
                    .iter()
                    .filter(|((_, tree, _), _)| tree.is_none())
                    .take(caller_limit as usize)
                    .map(|((sym, _, _), _)| *sym)
                    .collect();
                server.prefetch_crossref(&first).await;
                let mut included = 0;
                let mut left_out = false;
                for (lookups, ((caller_sym, caller_tree, implements), details)) in
                    callers.iter().enumerate()
                {
                    if lookups as u32 >= lookup_limit {
                        left_out = true;
                        break;
                    }
                    let (source_id, source_info) = sym_node_set
                        .ensure_symbol_from(caller_sym, *caller_tree, server, next_depth)
                        .await?;

                    let source_pretty = match source_info.get_structured().map(|meta| meta.pretty) {
                        Some(pretty) => pretty,
                        _ => ustr(""),
                    };
                    if ignore_node_set.contains(&*source_pretty) || !source_info.is_callable() {
                        continue;
                    }
                    // (Plumbing: callers labeled to be left out of uses
                    // diagrams (ex: mozilla::detail::RunnableFunction::Run,
                    // whose callee, a runnable's lambda, has a direct edge
                    // from where it's made), and IPDL's generated dispatch,
                    // as `Recv` methods' IPC edges from their `Send` methods
                    // are their control flow.)
                    if source_info.has_label("uses-diagram:skip") || source_info.is_ipdl_dispatch()
                    {
                        continue;
                    }
                    if let Some(allowed) = &guided_classes
                        && source_info
                            .get_structured()
                            .is_some_and(|meta| meta.kind == "method")
                        && class_of_info(source_info).is_some_and(|class| !allowed.contains(&class))
                    {
                        continue;
                    }
                    if included >= caller_limit {
                        left_out = true;
                        break;
                    }
                    included += 1;
                    sym_edge_set.ensure_edge_in_graph(
                        source_id,
                        sym_id.clone(),
                        if *implements {
                            EdgeKind::Implementation
                        } else {
                            EdgeKind::Default
                        },
                        details.clone(),
                        &mut graph,
                    );
                    // Only traverse the edge once.  (But not with `considered`
                    // before adding the edge, which would hide cycles.)
                    if !considered.insert(source_info.symbol) {
                        continue;
                    }
                    if next_depth >= max_depth {
                        overloads_hit.push(OverloadInfo {
                            kind: OverloadKind::DepthLimitOnUses,
                            sym: Some(source_info.symbol.to_string()),
                            pretty: Some(source_pretty.to_string()),
                            exist: next_depth,
                            included: depth + 1,
                            local_limit: 0,
                            global_limit: max_depth,
                        });
                        continue;
                    }
                    trace!(sym = caller_sym.as_str(), "scheduling uses");
                    to_traverse.push_back((
                        source_info.symbol,
                        source_pretty,
                        next_depth,
                        all_traversals_valid,
                    ));
                }
                if left_out {
                    overloads_hit.push(OverloadInfo {
                        kind: OverloadKind::UsesCallers,
                        sym: Some(sym.to_string()),
                        pretty: Some(pretty.to_string()),
                        exist: callers.len() as u32,
                        included,
                        local_limit: caller_limit,
                        global_limit: 0,
                    });
                }
            } else if !self.args.retain_all_symbol_data {
                let sym_info = sym_node_set.get_mut(&sym_id);
                sym_info.reduce_memory_usage_by_dropping_non_jumpref_info();
            }
        }

        let mut traverse_options = vec![];
        traverse_options.push(json!({
            "name": "*syms*",
            // Do not put "label", so that the frontend will ignore this,
            // when showing the control panel.
            "sources": json!(source_syms),
            "targets": json!(target_syms),
        }));
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
            // NOTE:
            //   * The name is mapped by query_core.toml
            //   * Range is not enforced on the server side.
            //     The following range is only for the UI purpose
            "name": "caller-limit",
            "label": "Caller limit",
            "value": caller_limit,
            "default": default_caller_limit,
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
            "placeholder": "Comma-separated pretty names (\\, for a comma in one)",
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

/// The pretty names of the `ignore-nodes` option, which separates them with
/// commas, where `\,` is a comma in a name (ex: a C++ template's,
/// `Pair<A\, B>`, or a web spec's term for several interfaces) and `\\` a
/// backslash.
fn split_ignore_nodes(list: &str) -> Vec<String> {
    let mut names = vec![];
    let mut name = String::new();
    let mut chars = list.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => name.extend(chars.next()),
            ',' => names.push(std::mem::take(&mut name)),
            c => name.push(c),
        }
    }
    names.push(name);
    names.retain(|name| !name.is_empty());
    names
}

#[test]
fn test_split_ignore_nodes() {
    assert_eq!(split_ignore_nodes("a::B,c"), vec!["a::B", "c"]);
    assert_eq!(
        split_ignore_nodes("html navigate,Pair<A\\, B>::First,a\\\\b,"),
        vec!["html navigate", "Pair<A, B>::First", "a\\b"]
    );
}
