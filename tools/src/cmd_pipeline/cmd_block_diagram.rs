use async_trait::async_trait;
use clap::{Args, ValueEnum};
use serde_json::json;
use ustr::{Ustr, ustr};

use super::interface::{PipelineCommand, PipelineValues};
use super::symbol_graph::{
    EdgeDetail, NamedSymbolGraph, SymbolGraphCollection, SymbolGraphEdgeSet, SymbolGraphNodeSet,
};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::dataflow::summary::is_field_sym;
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
    /// A class's fields (and methods), and the other classes' fields and
    /// functions data flows between them and (rendered as classes' tables
    /// of fields with `--hier=pretty`); always data edges.
    Field,
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

    /// Only the edges between the focus (its classes, or its fields and
    /// methods) and this class (symbol) or subsystem (its fields, methods
    /// and classes): ex: for an edge between two classes, the first's fields
    /// with `--level field --with <the second>`, or for one between two
    /// subsystems, the first's classes with `--with <the second>`.
    #[clap(long, value_parser)]
    with: Option<String>,
}

/// Whether a symbol is a class, for block diagrams: a C++ (or other
/// language's) class, or an IPDL struct or message, whose fields or
/// parameters data flows through (see `dataflow::summary::IpdlSlots`).
fn is_block_class(sym: &str) -> bool {
    sym.starts_with("T_") || sym.starts_with("IPDL_") || sym.starts_with("IPC_")
}

/// A class or subsystem's symbol, by its symbol or a subsystem's name.
fn block_sym(name: &str) -> Ustr {
    if is_block_class(name) || name.starts_with(SUBSYSTEM_PREFIX) {
        ustr(name)
    } else {
        subsystem_sym(name)
    }
}

/// Whether a diagram's end (a class, a subsystem, a field, or a function) is
/// in `with` (a class or a subsystem): the class itself or its fields and
/// methods; the subsystem itself or its classes and their members (by their
/// crossref entries' subsystems), or its free functions.
async fn is_in(server: &(dyn AbstractServer + Send + Sync), sym: Ustr, with: Ustr) -> Result<bool> {
    if sym == with {
        return Ok(true);
    }
    // (Its class: a field's, by its symbol, or a method's (or an IPDL
    // struct's field's, or a message's parameter's), by its parent; an IPDL
    // struct or message, which has none, is one.)
    let class = match crate::dataflow::summary::field_class(&sym) {
        Some(class) => Some(class),
        None if sym.starts_with("T_") || sym.starts_with(SUBSYSTEM_PREFIX) => Some(sym),
        None => {
            let parent = server
                .crossref_lookup(&sym)
                .await?
                .and_then(|data| data.meta)
                .and_then(|meta| meta.parent_sym)
                .filter(|parent| is_block_class(parent));
            parent.or_else(|| is_block_class(&sym).then_some(sym))
        }
    };
    if is_block_class(&with) {
        return Ok(class == Some(with));
    }
    let subject = class.unwrap_or(sym);
    if subject.starts_with(SUBSYSTEM_PREFIX) {
        return Ok(subject == with);
    }
    let subsystem = server
        .crossref_lookup(&subject)
        .await?
        .and_then(|data| data.meta)
        .and_then(|meta| meta.subsystem);
    Ok(subsystem.is_some_and(|subsystem| subsystem_sym(&subsystem) == with))
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
    /// For data edges between fields: a function the data flows in.
    via: Option<Ustr>,
    /// How many of its calls are IPDL messages.
    ipc: u32,
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
    /// Its roll-ups' crossref data.  (Rough; see `memory_budget`.)
    fn estimate(&self, _server: &(dyn AbstractServer + Send + Sync)) -> u64 {
        crate::memory_budget::mib(128)
    }

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
        } else {
            block_sym(focus)
        };
        let is_subsystem = focus_sym.starts_with(SUBSYSTEM_PREFIX);
        // (The whole tree's subsystems, else classes.)
        let level = self.args.level.unwrap_or(if focus.is_empty() {
            BlockLevel::Subsystem
        } else {
            BlockLevel::Class
        });
        let rollup = if level == BlockLevel::Field {
            Rollup::default()
        } else {
            let Some(rollup) = rollup_of(server, &focus_sym, self.args.edges).await? else {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::BadInput,
                    message: format!("No roll-up for {}", focus_sym),
                }));
            };
            rollup
        };

        let mut edges: Vec<BlockEdge> = vec![];
        // (The focus's side: it, and the members whose edges are shown.)
        let mut own: std::collections::HashSet<Ustr> = [focus_sym].into_iter().collect();
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
        if level == BlockLevel::Field {
            // A class's fields' and methods' data edges.
            let meta = server
                .crossref_lookup(&focus_sym)
                .await?
                .and_then(|data| data.meta);
            let mut members: Vec<Ustr> = vec![];
            if let Some(meta) = &meta {
                members.extend(meta.fields.iter().map(|f| f.sym));
                members.extend(meta.methods.iter().map(|m| m.sym));
            }
            server.prefetch_crossref(&members).await;
            own.extend(members.iter().copied());
            for member in &members {
                let Some(member_rollup) = rollup_of(server, member, BlockEdges::Data).await? else {
                    continue;
                };
                for e in &member_rollup.out {
                    edges.push(BlockEdge {
                        from: *member,
                        to: e.sym,
                        count: e.count,
                        example: None,
                        via: e.via.first().copied(),
                        ipc: 0,
                    });
                }
                for e in &member_rollup.into {
                    edges.push(BlockEdge {
                        from: e.sym,
                        to: *member,
                        count: e.count,
                        example: None,
                        via: e.via.first().copied(),
                        ipc: 0,
                    });
                }
            }
            edges.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
            edges.dedup_by(|a, b| a.from == b.from && a.to == b.to);
        } else if focus_sym == root_subsystem_sym() && level == BlockLevel::Subsystem {
            // The whole tree's subsystems.
            for pair in &rollup.edges {
                edges.push(BlockEdge {
                    from: pair.from,
                    to: pair.to,
                    count: pair.count,
                    example: None,
                    via: None,
                    ipc: 0,
                });
            }
        } else if is_subsystem && level == BlockLevel::Class {
            // A subsystem's classes, and their neighbors outside it.
            let members: Vec<Ustr> = rollup
                .members
                .iter()
                .filter(|m| is_block_class(&m.sym))
                .take(self.args.max_members)
                .map(|m| m.sym)
                .collect();
            server.prefetch_crossref(&members).await;
            own.extend(members.iter().copied());
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
                        via: None,
                        ipc: e.ipc,
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
                            via: None,
                            ipc: e.ipc,
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
                    via: None,
                    ipc: e.ipc,
                });
            }
            for e in &rollup.into {
                edges.push(BlockEdge {
                    from: e.sym,
                    to: focus_sym,
                    count: edge_count(e),
                    example: first_example(e),
                    via: None,
                    ipc: e.ipc,
                });
            }
        }

        edges.retain(|e| e.count >= self.args.min_count && e.from != e.to);
        // (Only those with the other side; see `with`.)
        let with_sym = self
            .args
            .with
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(block_sym);
        if let Some(with) = with_sym {
            let other = |e: &BlockEdge| match (own.contains(&e.from), own.contains(&e.to)) {
                (true, false) => Some(e.to),
                (false, true) => Some(e.from),
                _ => None,
            };
            let mut others: Vec<Ustr> = edges.iter().filter_map(other).collect();
            others.sort();
            others.dedup();
            server.prefetch_crossref(&others).await;
            let mut inside: std::collections::HashSet<Ustr> = Default::default();
            for sym in others {
                if is_in(server, sym, with).await? {
                    inside.insert(sym);
                }
            }
            edges.retain(|e| other(e).is_some_and(|o| inside.contains(&o)));
        }
        let unit = match (self.args.edges, level) {
            (BlockEdges::Calls, l) if l != BlockLevel::Field => "calls",
            _ => "flows",
        };
        // (Most first; and, for fields, edges between fields before those
        // between functions and fields.)
        let between_fields = |e: &BlockEdge| is_field_sym(&e.from) && is_field_sym(&e.to);
        edges.sort_by(|a, b| {
            between_fields(b)
                .cmp(&between_fields(a))
                .then(b.count.cmp(&a.count))
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
        // (The nodes' and the vias' entries, at once.)
        let syms: Vec<Ustr> = std::iter::once(focus_sym)
            .chain(edges.iter().flat_map(|edge| [edge.from, edge.to]))
            .chain(edges.iter().filter_map(|edge| edge.via))
            .collect();
        server.prefetch_crossref(&syms).await;
        for edge in &edges {
            let (from_id, _) = node_set.ensure_symbol(&edge.from, server, 1).await?;
            let (to_id, _) = node_set.ensure_symbol(&edge.to, server, 1).await?;
            let mut data = vec![EdgeDetail::Weight(edge.count)];
            let via = match edge.via {
                Some(via) => server
                    .crossref_lookup(&via)
                    .await?
                    .and_then(|d| d.meta)
                    .map(|m| m.pretty)
                    .or(Some(via)),
                None => None,
            };
            data.push(EdgeDetail::Label(match (&edge.example, via) {
                (_, Some(via)) => format!("{} {}, ex: in {}", edge.count, unit, via),
                (Some((caller, callee)), None) if edge.ipc > 0 => format!(
                    "{} {} ({} IPC messages), ex: {} → {}",
                    edge.count, unit, edge.ipc, caller, callee
                ),
                (Some((caller, callee)), None) => {
                    format!("{} {}, ex: {} → {}", edge.count, unit, caller, callee)
                }
                (None, None) => format!("{} {}", edge.count, unit),
            }));
            // (All IPC: dotted, as calls diagrams' IPC edges are.)
            let kind = if edge.ipc > 0 && edge.ipc >= edge.count {
                EdgeKind::IPC
            } else {
                EdgeKind::Default
            };
            edge_set.ensure_edge_in_graph(from_id, to_id, kind, data, &mut graph);
        }

        // (The diagram panel's controls, as query terms; see `cmd_traverse`.)
        let level_name = |level: BlockLevel| format!("{:?}", level).to_lowercase();
        let default_level = if focus.is_empty() {
            BlockLevel::Subsystem
        } else {
            BlockLevel::Class
        };
        let options = json!([
            {
                "section": "Block diagram",
                "items": [
                    {
                        // (No "label", so the panel ignores it.)
                        "name": "*block*",
                        "focus": focus_sym,
                        "edges": edges.len(),
                        "totalEdges": total_edges,
                    },
                    {
                        "name": "block-level",
                        "label": "Level",
                        "value": level_name(level),
                        "default": level_name(default_level),
                        "type": "string",
                        "placeholder": "subsystem, class or field",
                    },
                    {
                        "name": "block-edges",
                        "label": "Edges",
                        "value": format!("{:?}", self.args.edges).to_lowercase(),
                        "default": "calls",
                        "type": "string",
                        "placeholder": "calls or data",
                    },
                    {
                        "name": "block-min-count",
                        "label": "Minimum count",
                        "value": self.args.min_count,
                        "default": 1,
                        "range": [1, 1000],
                    },
                    {
                        "name": "block-max-edges",
                        "label": "Edge limit",
                        "value": self.args.max_edges,
                        "default": 48,
                        "range": [1, 512],
                    },
                    {
                        "name": "block-with",
                        "label": "With",
                        "value": self.args.with.as_deref().unwrap_or(""),
                        "default": "",
                        "type": "string",
                        "placeholder": "a class's symbol or a subsystem",
                    },
                ],
            }
        ]);
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
