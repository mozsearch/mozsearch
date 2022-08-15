use std::collections::HashSet;

use async_trait::async_trait;
use itertools::Itertools;
use serde_json::Value;
use clap::Args;
use tracing::trace;

use super::{
    interface::{OverloadInfo, OverloadKind, PipelineCommand, PipelineValues},
    symbol_graph::{
        DerivedSymbolInfo, NamedSymbolGraph, SymbolGraphCollection, SymbolGraphNodeSet,
    },
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Processes piped-in crossref symbol data, recursively traversing the given
/// edges, building up a graph that also holds onto the crossref data for all
/// traversed symbols.
#[derive(Debug, Args)]
pub struct Traverse {
    /// The edge to traverse, currently "uses" or "callees".
    ///
    /// ### SPECULATIVELY WRITTEN BUT LET'S SEE HOW WE HANDLE THE ABOVE FIRST ###
    ///
    /// The edge to traverse, this is either a 'kind' ("uses", "defs", "assignments",
    /// "decls", "forwards", "idl", "ipc") or one of the synthetic edges ("calls-from",
    /// "calls-to").
    ///
    /// The "calls-from" and "calls-to" synthetic edges have special behaviors:
    /// - Ignores edges to nodes that are not 'callable' as indicated by their
    ///   structured analysis "kind" being "function" or "method".
    /// - Ignores edges to nodes that don't seem to be inside the same
    ///
    /// The fancy prototype previously did but we don't do yet:
    /// - Ignores edges to nodes that are 'boring' as determined by hardcoded
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
    #[clap(long, value_parser, default_value = "256")]
    pub node_limit: u32,
    /// Maximum number of nodes in a graph being built to be processed by
    /// paths-between.
    #[clap(long, value_parser, default_value = "8192")]
    pub paths_between_node_limit: u32,

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
        let mut graph = NamedSymbolGraph::new("only".to_string());

        // A to-do list of nodes we have not yet traversed.
        let mut to_traverse = Vec::new();
        // Nodes that have been scheduled to be traversed or ruled out.  A node
        // in this set should not be added to `to_traverse`.
        let mut considered = HashSet::new();
        // Root set for paths-between use.
        let mut root_set = vec![];

        let mut overloads_hit = vec![];

        // Propagate the starting symbols into the graph and queue them up for
        // traversal.
        for info in cil.symbol_crossref_infos {
            to_traverse.push((info.symbol.clone(), 0));
            considered.insert(info.symbol.clone());

            let (sym_node_id, _info) =
                sym_node_set.add_symbol(DerivedSymbolInfo::new(info.symbol, info.crossref_info));
            root_set.push(sym_node_id);
        }

        let node_limit = if self.args.paths_between {
            self.args.node_limit
        } else {
            self.args.paths_between_node_limit
        };

        // General operation:
        // - We pull a node to be traversed off the queue.  This ends up depth
        //   first.
        // - We check if we already have the crossref info for the symbol and
        //   look it up if not.  There's an asymmetry here between the initial
        //   set of symbols we're traversing from which we already have cached
        //   values for and the new edges we discover, but it's not a concern.
        // - We traverse the list of edges.
        while let Some((sym, depth)) = to_traverse.pop() {
            if sym_node_set.symbol_crossref_infos.len() as u32 >= node_limit {
                trace!(sym = %sym, depth, "stopping because of node limit");
                overloads_hit.push(OverloadInfo {
                    kind: OverloadKind::NodeLimit,
                    exist: to_traverse.len() as u32,
                    included: node_limit,
                    local_limit: 0,
                    global_limit: node_limit,
                });
                to_traverse.clear();
                break;
            };

            trace!(sym = %sym, depth, "processing");
            let (sym_id, sym_info) = sym_node_set.ensure_symbol(&sym, server).await?;

            // Consider "srcsym" as superseding the actual "uses" edge.
            if self.args.edge.as_str() == "uses" {
                if let Some(source_sym) = sym_info
                    .crossref_info
                    .pointer("/meta/srcsym")
                    .unwrap_or(&Value::Null)
                    .clone()
                    .as_str()
                {
                    let (source_id, source_info) =
                        sym_node_set.ensure_symbol(&source_sym, server).await?;

                    if source_info.is_callable() {
                        graph.add_edge(source_id, sym_id.clone());
                        if depth < max_depth && considered.insert(source_info.symbol.clone()) {
                            trace!(sym = source_sym, "scheduling srcsym");
                            to_traverse.push((source_info.symbol.clone(), depth + 1));
                        }
                    }
                    // We explicitly want to bypass the normal "uses" processing
                    // because that will just get us IPC internals or something
                    // similar.
                    continue;
                }
            }
            // TODO: we also would want to handle "targetsym" in the "callees" direction eventually.

            // Clone the edges now before engaging in additional borrows.
            let edges = sym_info.crossref_info[&self.args.edge].clone();

            // ## Handle "overrides" and "overriddenBy"
            //
            // Currently both of these edges are directed to work with "uses"
            // such that call trees will be deeper rather than wider, but this
            // should be revisited and thought out more, especially as to
            // whether this should result in clusters, etc.
            //
            // Note that the logic below is highly duplicative
            let overrides = sym_info
                .crossref_info
                .pointer("/meta/overrides")
                .unwrap_or(&Value::Null)
                .clone();
            //let overridden_by = sym_info.crossref_info.pointer("/meta/overriddenBy").unwrap_or(&Value::Null).clone();

            if let Some(sym_edges) = overrides.as_array() {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on meta overrides"),
                    })
                };

                for target in sym_edges {
                    // overrides is { sym, pretty }
                    let target_sym = target["sym"].as_str().ok_or_else(bad_data)?;

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server).await?;

                    if target_info.is_callable() {
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
                            graph.add_edge(target_id, sym_id.clone());
                            if depth < max_depth {
                                trace!(sym = target_sym, "scheduling overrides");
                                to_traverse.push((target_info.symbol.clone(), depth + 1));
                            }
                        }
                    }
                }
            }

            // commented out because for "uses" we only want to walk up overrides for now.
            /*
            if let Some(sym_edges) = overridden_by.as_array() {
                let bad_data = || {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on meta overriddenBy"),
                    })
                };


                for target in sym_edges {
                    // overriddenBy is just a bare symbol name currently
                    let target_sym = target.as_str().ok_or_else(bad_data)?;

                    let (target_id, target_info) =
                        sym_node_set.ensure_symbol(&target_sym, server).await?;

                    if target_info.is_callable() {
                        if considered.insert(target_info.symbol.clone()) {
                            // Same rationale on avoiding a duplicate edge.
                            graph.add_edge(target_id, sym_id.clone());
                            if depth < max_depth {
                                trace!(sym = target_sym, "scheduling overridenBy");
                                to_traverse.push((target_info.symbol.clone(), depth + 1));
                            }
                        }
                    }
                }
            }
            */

            // ## Handle the explicit edges
            if let Some(sym_edges) = edges.as_array() {
                let bad_data = || {
                    let edge = self.args.edge.clone();
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on edge {edge}"),
                    })
                };
                match self.args.edge.as_str() {
                    // Callees are synthetically derived from crossref and is a
                    // flat list of { kind, pretty, sym }.  This differs from
                    // most other edges which are path hit-lists.
                    "callees" => {
                        for target in sym_edges {
                            let target_sym = target["sym"].as_str().ok_or_else(bad_data)?;
                            //let target_kind = target["kind"].as_str().ok_or_else(bad_data)?;

                            let (target_id, target_info) =
                                sym_node_set.ensure_symbol(&target_sym, server).await?;

                            if target_info.is_callable() {
                                graph.add_edge(sym_id.clone(), target_id);
                                if depth < max_depth
                                    && considered.insert(target_info.symbol.clone())
                                {
                                    trace!(sym = target_sym, "scheduling callees");
                                    to_traverse.push((target_info.symbol.clone(), depth + 1));
                                }
                            }
                        }
                    }
                    // Uses are path-hitlists and each array item has the form
                    // { path, lines: [ { context, contextsym }] } eliding some
                    // of the hit fields.  We really just care about the
                    // contextsym.
                    "uses" => {
                        // Do not process the uses if there are more paths than our skip limit.
                        if sym_edges.len() as u32 >= self.args.skip_uses_at_path_count {
                            overloads_hit.push(OverloadInfo {
                                kind: OverloadKind::UsesPaths,
                                exist: sym_edges.len() as u32,
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
                        for path_hits in sym_edges {
                            let hits = path_hits["lines"].as_array().ok_or_else(bad_data)?;
                            for source in hits {
                                let source_sym = source["contextsym"].as_str().unwrap_or("");
                                //let source_kind = source["kind"].as_str().ok_or_else(bad_data)?;

                                if source_sym.is_empty() {
                                    continue;
                                }

                                let (source_id, source_info) =
                                    sym_node_set.ensure_symbol(&source_sym, server).await?;

                                if source_info.is_callable() {
                                    // Only process this given use edge once.
                                    if use_considered.insert(source_info.symbol.clone()) {
                                        graph.add_edge(source_id, sym_id.clone());
                                        if depth < max_depth
                                            && considered.insert(source_info.symbol.clone())
                                        {
                                            trace!(sym = source_sym, "scheduling uses");
                                            to_traverse
                                                .push((source_info.symbol.clone(), depth + 1));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // ## Paths Between
        let graph_coll = if self.args.paths_between {
            // In this case, we don't want our original node set because we
            // expect it to have an order of magnitude more data than we want
            // in the result set.  So we build a new node set and graph.
            let mut paths_node_set = SymbolGraphNodeSet::new();
            let mut paths_graph = NamedSymbolGraph::new("paths".to_string());
            let mut suppression = HashSet::new();
            for (source_node, target_node) in root_set.iter().tuple_combinations() {
                let node_paths = graph.all_simple_paths(source_node.clone(), target_node.clone());
                trace!(path_count = node_paths.len(), "forward paths found");
                sym_node_set.propagate_paths(
                    node_paths,
                    &mut paths_graph,
                    &mut paths_node_set,
                    &mut suppression,
                );

                let node_paths = graph.all_simple_paths(target_node.clone(), source_node.clone());
                trace!(path_count = node_paths.len(), "reverse paths found");
                sym_node_set.propagate_paths(
                    node_paths,
                    &mut paths_graph,
                    &mut paths_node_set,
                    &mut suppression,
                );
            }
            SymbolGraphCollection {
                node_set: paths_node_set,
                graphs: vec![paths_graph],
                overloads_hit,
            }
        } else {
            SymbolGraphCollection {
                node_set: sym_node_set,
                graphs: vec![graph],
                overloads_hit,
            }
        };

        Ok(PipelineValues::SymbolGraphCollection(graph_coll))
    }
}
