use async_trait::async_trait;
use clap::{Args, ValueEnum};
use serde_json::json;
use ustr::{Ustr, ustr};

use super::interface::{PipelineCommand, PipelineValues};
use super::symbol_graph::{
    EdgeDetail, NamedSymbolGraph, SymbolGraphCollection, SymbolGraphEdgeSet, SymbolGraphNodeSet,
};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::file_format::rollup::{
    Rollup, RollupEdge, SUBSYSTEM_PREFIX, root_subsystem_sym, subsystem_sym,
};
use crate::symbol_graph_edge_kind::EdgeKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BlockEdges {
    /// The calls between functions.
    Calls,
    /// The data flowing between fields, and between functions and other
    /// objects' fields (from dataflow summaries; see
    /// `dataflow::summary::FieldEdge`).
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BlockLevel {
    /// Subsystems' blocks.
    Subsystem,
    /// Classes' blocks (clustered by subsystem when rendered with
    /// `--hier=subsystem`).
    Class,
}

/// A block diagram: how subsystems, or classes, interact, from crossref's
/// roll-ups of the calls between their functions (see
/// `tools::file_format::rollup`), rather than by walking methods, so it's
/// quick for any area.  Its output is a graph for `graph` to render, whose
/// edges are labeled (and weighted) by how many calls they stand for.
///
/// - No `focus`: the whole tree's subsystems.
/// - A subsystem (ex: "Core/DOM/Workers"): with `--level class`, its classes
///   with the most calls, and the classes outside it they call (or are
///   called by) the most; with `--level subsystem`, it and the subsystems it
///   calls or is called by.
/// - A class (by symbol, ex: "T_mozilla::dom::WorkerPrivate"): it and the
///   classes it calls or is called by the most.
#[derive(Debug, Args)]
pub struct BlockDiagram {
    /// The subsystem or class (symbol) to diagram, else the whole tree.
    #[clap(value_parser)]
    focus: Option<String>,

    #[clap(long, value_enum)]
    level: Option<BlockLevel>,

    /// What the edges are.
    #[clap(long, value_enum, default_value_t = BlockEdges::Calls)]
    edges: BlockEdges,

    /// Don't count calls of accessors (getters, setters and trivial methods,
    /// by their dataflow summaries).
    #[clap(long)]
    no_accessors: bool,

    /// Leave out edges standing for fewer calls.
    #[clap(long, default_value_t = 1)]
    min_count: u32,

    /// Keep at most this many edges (the biggest).
    #[clap(long, default_value_t = 48)]
    max_edges: usize,

    /// For a subsystem's classes: how many of its classes (with the most
    /// calls) to show.
    #[clap(long, default_value_t = 24)]
    max_members: usize,
}

#[derive(Debug)]
pub struct BlockDiagramCommand {
    pub args: BlockDiagram,
}

/// An edge of the diagram: its ends, how many calls, and its biggest
/// contributor's (caller's and callee's) pretty names.
struct BlockEdge {
    from: Ustr,
    to: Ustr,
    count: u32,
    example: Option<(Ustr, Ustr)>,
}

async fn rollup_of(
    server: &(dyn AbstractServer + Send + Sync),
    sym: &Ustr,
    edges: BlockEdges,
) -> Result<Option<Rollup>> {
    Ok(server
        .crossref_lookup(sym)
        .await?
        .and_then(|data| match edges {
            BlockEdges::Calls => data.call_rollup,
            BlockEdges::Data => data.data_rollup,
        }))
}

#[async_trait]
impl PipelineCommand for BlockDiagramCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let focus = self.args.focus.as_deref().unwrap_or("").trim();
        // ("*" is the whole tree, for query terms.)
        let focus = if focus == "*" { "" } else { focus };
        let focus_sym = if focus.is_empty() {
            root_subsystem_sym()
        } else if focus.starts_with("T_") || focus.starts_with(SUBSYSTEM_PREFIX) {
            ustr(focus)
        } else {
            subsystem_sym(focus)
        };
        let is_subsystem = focus_sym.starts_with(SUBSYSTEM_PREFIX);
        // (The whole tree's subsystems, else classes.)
        let level = self.args.level.unwrap_or(if focus.is_empty() {
            BlockLevel::Subsystem
        } else {
            BlockLevel::Class
        });
        let Some(rollup) = rollup_of(server, &focus_sym, self.args.edges).await? else {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!("No roll-up for {}", focus_sym),
            }));
        };

        let mut edges: Vec<BlockEdge> = vec![];
        let first_example =
            |e: &RollupEdge| e.examples.first().map(|x| (x.from_pretty, x.to_pretty));
        let no_accessors = self.args.no_accessors;
        let edge_count = |e: &RollupEdge| {
            if no_accessors {
                e.count - e.accessors.min(e.count)
            } else {
                e.count
            }
        };
        if focus_sym == root_subsystem_sym() && level == BlockLevel::Subsystem {
            // The whole tree's subsystems.
            for pair in &rollup.edges {
                edges.push(BlockEdge {
                    from: pair.from,
                    to: pair.to,
                    count: pair.count,
                    example: None,
                });
            }
        } else if is_subsystem && level == BlockLevel::Class {
            // A subsystem's classes, and their neighbors outside it.
            let members: Vec<Ustr> = rollup
                .members
                .iter()
                .filter(|m| m.sym.starts_with("T_"))
                .take(self.args.max_members)
                .map(|m| m.sym)
                .collect();
            for member in &members {
                let Some(member_rollup) = rollup_of(server, member, self.args.edges).await? else {
                    continue;
                };
                for e in &member_rollup.out {
                    edges.push(BlockEdge {
                        from: *member,
                        to: e.sym,
                        count: edge_count(e),
                        example: first_example(e),
                    });
                }
                // (Calls in from outside the members; the members' own are
                // their `out`s.)
                for e in &member_rollup.into {
                    if !members.contains(&e.sym) {
                        edges.push(BlockEdge {
                            from: e.sym,
                            to: *member,
                            count: edge_count(e),
                            example: first_example(e),
                        });
                    }
                }
            }
        } else {
            // A class's, or a subsystem's, neighbors.
            for e in &rollup.out {
                edges.push(BlockEdge {
                    from: focus_sym,
                    to: e.sym,
                    count: edge_count(e),
                    example: first_example(e),
                });
            }
            for e in &rollup.into {
                edges.push(BlockEdge {
                    from: e.sym,
                    to: focus_sym,
                    count: edge_count(e),
                    example: first_example(e),
                });
            }
        }

        edges.retain(|e| e.count >= self.args.min_count && e.from != e.to);
        let unit = match self.args.edges {
            BlockEdges::Calls => "calls",
            BlockEdges::Data => "flows",
        };
        edges.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });
        let total_edges = edges.len();
        edges.truncate(self.args.max_edges);

        let mut node_set = SymbolGraphNodeSet::new();
        let mut edge_set = SymbolGraphEdgeSet::new();
        let mut graph = NamedSymbolGraph::new(format!("block-diagram {}", focus_sym));
        // (A class, or a subsystem among its neighbors, even if it has no
        // edges left.)
        if !is_subsystem || (level == BlockLevel::Subsystem && focus_sym != root_subsystem_sym()) {
            let (focus_id, _) = node_set.ensure_symbol(&focus_sym, server, 0).await?;
            graph.ensure_node(focus_id);
        }
        for edge in &edges {
            let (from_id, _) = node_set.ensure_symbol(&edge.from, server, 1).await?;
            let (to_id, _) = node_set.ensure_symbol(&edge.to, server, 1).await?;
            let mut data = vec![EdgeDetail::Weight(edge.count)];
            data.push(EdgeDetail::Label(match &edge.example {
                Some((caller, callee)) => {
                    format!("{} {}, ex: {} → {}", edge.count, unit, caller, callee)
                }
                None => format!("{} {}", edge.count, unit),
            }));
            edge_set.ensure_edge_in_graph(from_id, to_id, EdgeKind::Default, data, &mut graph);
        }

        let options = json!({
            "blockDiagram": {
                "focus": focus_sym,
                "level": format!("{:?}", level).to_lowercase(),
                "edges": edges.len(),
                "totalEdges": total_edges,
                "minCount": self.args.min_count,
                "edgeKind": format!("{:?}", self.args.edges).to_lowercase(),
            }
        });
        Ok(PipelineValues::SymbolGraphCollection(
            SymbolGraphCollection {
                node_set,
                edge_set,
                graphs: vec![graph],
                overloads_hit: vec![],
                options,
                hierarchical_graphs: vec![],
            },
        ))
    }
}
