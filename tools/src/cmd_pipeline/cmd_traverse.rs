use std::collections::HashSet;

use async_trait::async_trait;
use structopt::StructOpt;

use super::{
    interface::{PipelineCommand, PipelineValues},
    symbol_graph::{
        DerivedSymbolInfo, NamedSymbolGraph, SymbolGraphCollection, SymbolGraphNodeSet,
    },
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Processes piped-in crossref symbol data, recursively traversing the given
/// edges, building up a graph that also holds onto the crossref data for all
/// traversed symbols.
#[derive(Debug, StructOpt)]
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
    #[structopt(long, short, default_value = "callees")]
    edge: String,
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
        let cil = match input {
            PipelineValues::SymbolCrossrefInfoList(cil) => cil,
            // TODO: Figure out a better way to handle a nonsensical pipeline
            // configuration / usage.
            _ => {
                return Ok(PipelineValues::Void);
            }
        };

        let mut sym_node_set = SymbolGraphNodeSet::new();
        let mut graph = NamedSymbolGraph::new("only".to_string());

        // A to-do list of nodes we have not yet traversed.
        let mut to_traverse = Vec::new();
        // Nodes that have been scheduled to be traversed or ruled out.  A node
        // in this set should not be added to `to_traverse`.
        let mut considered = HashSet::new();

        // Propagate the starting symbols into the graph and queue them up for
        // traversal.
        for info in cil.symbol_crossref_infos {
            to_traverse.push(info.symbol.clone());
            considered.insert(info.symbol.clone());
            sym_node_set.add_symbol(DerivedSymbolInfo::new(
                &info.symbol,
                info.crossref_info.clone(),
            ));
        }

        // General operation:
        // - We pull a node to be traversed off the queue.  This ends up depth
        //   first.
        // - We check if we already have the crossref info for the symbol and
        //   look it up if not.  There's an asymmetry here between the initial
        //   set of symbols we're traversing from which we already have cached
        //   values for and the new edges we discover, but it's not a concern.
        // - We traverse the list of edges.
        while let Some(sym) = to_traverse.pop() {
            let (sym_id, sym_info) =
                sym_node_set.ensure_symbol(&sym, server).await?;

            let edges = sym_info.crossref_info[&self.args.edge].clone();

            if let Some(sym_edges) = edges.as_array() {
                let bad_data = || {
                    let edge = self.args.edge.clone();
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::DataLayer,
                        message: format!("Bad edge info in sym {sym} on edge {edge}"),
                    })
                };
                match self.args.edge.as_str() {
                    "callees" => {
                        for target in sym_edges {
                            let target_sym = target["sym"].as_str().ok_or_else(bad_data)?;
                            //let target_kind = target["kind"].as_str().ok_or_else(bad_data)?;

                            let (target_id, target_info) =
                                sym_node_set.ensure_symbol(&target_sym, server).await?;

                            if target_info.is_callable() {
                                graph.add_edge(sym_id.clone(), target_id);
                                if considered.insert(target_info.symbol.clone()) {
                                    to_traverse.push(target_info.symbol.clone());
                                }
                            }
                        }
                    }
                    "uses" => {}
                    _ => {}
                }
            }
        }

        let graph_coll = SymbolGraphCollection {
            node_set: sym_node_set,
            graphs: vec![graph],
        };

        Ok(PipelineValues::SymbolGraphCollection(graph_coll))
    }
}
