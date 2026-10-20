use async_trait::async_trait;
use clap::{Args, ValueEnum};
use serde_json::json;
use ustr::{Ustr, ustr};

use super::interface::{PipelineCommand, PipelineValues};
use super::symbol_graph::{
    EdgeDetail, NamedSymbolGraph, SymbolBadge, SymbolGraphCollection, SymbolGraphEdgeSet,
    SymbolGraphNodeSet,
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
/// - No `focus`: the whole tree's subsystems, left to right (so that its few
///   ranks, as most calls go to a few subsystems, are columns flowing down
///   the page, rather than rows across it), but not the calls into the
///   omnipresent subsystems (see `omnipresent_subsystems`), whose boxes say
///   how many subsystems use them; and in big trees (`--groups-at`), as a
///   diagram per group of the subsystems most tied by their calls (see
///   `louvain`): each its strongest internal edges and a few to other
///   groups, then the omnipresent subsystems.
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

    /// For the whole tree, in trees with at least this many subsystems: a
    /// diagram per group of the subsystems most closely tied by their calls
    /// (see `louvain`), one after another down the page, rather than one
    /// diagram.
    #[clap(long, default_value_t = 20)]
    groups_at: usize,
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

/// Add `edges` to `graph`, with their ends, labeled with their counts (of
/// `unit`), with their biggest contributors as tooltips.
async fn add_edges(
    server: &(dyn AbstractServer + Send + Sync),
    edges: &[&BlockEdge],
    unit: &str,
    node_set: &mut SymbolGraphNodeSet,
    edge_set: &mut SymbolGraphEdgeSet,
    graph: &mut NamedSymbolGraph,
) -> Result<()> {
    // (The nodes' and the vias' entries, at once.)
    let syms: Vec<Ustr> = edges
        .iter()
        .flat_map(|edge| [edge.from, edge.to])
        .chain(edges.iter().filter_map(|edge| edge.via))
        .collect();
    server.prefetch_crossref(&syms).await;
    for edge in edges {
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
        edge_set.ensure_edge_in_graph(from_id, to_id, kind, data, graph);
    }
    Ok(())
}

/// Add the omnipresent subsystems (see `omnipresent_subsystems`) to `graph`,
/// saying how many subsystems use them.
async fn add_omnipresent(
    server: &(dyn AbstractServer + Send + Sync),
    omnipresent: &[(Ustr, usize)],
    node_set: &mut SymbolGraphNodeSet,
    graph: &mut NamedSymbolGraph,
) -> Result<()> {
    for (sym, callers) in omnipresent {
        let (id, info) = node_set.ensure_symbol(sym, server, 1).await?;
        let label = ustr(&format!("used by {}", callers));
        if !info.badges.iter().any(|badge| badge.label == label) {
            info.badges.push(SymbolBadge {
                pri: 0,
                label,
                source_jump: None,
            });
        }
        graph.ensure_node(id);
    }
    Ok(())
}

/// Groups of a weighted undirected graph's `n` nodes ("communities"), by
/// Louvain's modularity optimization (Blondel et al., 2008): moving each node
/// to the neighboring group it adds the most modularity to, until none moves,
/// then the same over the groups, as nodes, until nothing moves.  It's
/// deterministic (nodes in order, ties to the lowest group), for diagrams
/// that don't change between requests.  Returns each node's group.
fn louvain(n: usize, edges: &[(usize, usize, u64)]) -> Vec<usize> {
    use std::collections::BTreeMap;
    let mut node_group: Vec<usize> = (0..n).collect();
    let mut adj: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); n];
    for &(a, b, w) in edges {
        let w = w as f64;
        if a == b {
            *adj[a].entry(a).or_default() += 2.0 * w;
        } else {
            *adj[a].entry(b).or_default() += w;
            *adj[b].entry(a).or_default() += w;
        }
    }
    loop {
        let size = adj.len();
        let degree: Vec<f64> = adj.iter().map(|links| links.values().sum()).collect();
        let total: f64 = degree.iter().sum();
        if total == 0.0 {
            break;
        }
        let mut group: Vec<usize> = (0..size).collect();
        let mut group_degree = degree.clone();
        let mut moved = false;
        loop {
            let mut improved = false;
            for i in 0..size {
                let current = group[i];
                let mut links: BTreeMap<usize, f64> = BTreeMap::new();
                for (&j, &w) in &adj[i] {
                    if j != i {
                        *links.entry(group[j]).or_default() += w;
                    }
                }
                group_degree[current] -= degree[i];
                let gain = |g: usize, group_degree: &[f64]| {
                    links.get(&g).copied().unwrap_or(0.0) - group_degree[g] * degree[i] / total
                };
                let (mut best, mut best_gain) = (current, gain(current, &group_degree));
                for &g in links.keys() {
                    let g_gain = gain(g, &group_degree);
                    if g_gain > best_gain + 1e-12 {
                        (best, best_gain) = (g, g_gain);
                    }
                }
                group_degree[best] += degree[i];
                if best != current {
                    group[i] = best;
                    improved = true;
                    moved = true;
                }
            }
            if !improved {
                break;
            }
        }
        if !moved {
            break;
        }
        // (The groups as the next level's nodes.)
        let labels: BTreeMap<usize, usize> = group
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(label, g)| (g, label))
            .collect();
        node_group = node_group.iter().map(|&g| labels[&group[g]]).collect();
        let mut next: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); labels.len()];
        for (i, links) in adj.iter().enumerate() {
            for (&j, &w) in links {
                *next[labels[&group[i]]]
                    .entry(labels[&group[j]])
                    .or_default() += w;
            }
        }
        adj = next;
    }
    node_group
}

/// How many groups the whole tree's diagram shows at most, and how many of
/// each group's edges to other groups (see `BlockDiagram::groups_at`).
const MAX_GROUPS: usize = 12;
const GROUP_EXTERNAL_EDGES: usize = 6;

/// The whole tree's omnipresent subsystems, by how many subsystems call them:
/// those called by at least half of them (ex: firefox-disco's MFBT, XPCOM,
/// String, Memory Allocator, Cairo, and DOM/Core HTML), in trees with at
/// least `OMNIPRESENT_MIN_SUBSYSTEMS` (the "omnipresent modules" software
/// clustering leaves out): the calls into them took most of the whole tree's
/// diagram's edges (26 of firefox-disco's 48 went to 4 of them), rather than
/// showing how the other subsystems interact.
fn omnipresent_subsystems(rollup: &Rollup) -> std::collections::HashMap<Ustr, usize> {
    let subsystems = rollup.members.len();
    let mut callers: std::collections::HashMap<Ustr, usize> = Default::default();
    if subsystems < OMNIPRESENT_MIN_SUBSYSTEMS {
        return callers;
    }
    for pair in &rollup.edges {
        *callers.entry(pair.to).or_default() += 1;
    }
    callers.retain(|_, n| *n * 2 >= subsystems);
    callers
}

/// See `omnipresent_subsystems`.
const OMNIPRESENT_MIN_SUBSYSTEMS: usize = 20;

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
        // (The whole tree's omnipresent subsystems, by how many use them.)
        let mut omnipresent: std::collections::HashMap<Ustr, usize> = Default::default();
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
            // The whole tree's subsystems, but not the calls into omnipresent
            // ones.
            omnipresent = omnipresent_subsystems(&rollup);
            for pair in &rollup.edges {
                if omnipresent.contains_key(&pair.to) {
                    continue;
                }
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

        let mut node_set = SymbolGraphNodeSet::new();
        let mut edge_set = SymbolGraphEdgeSet::new();
        let is_tree = focus_sym == root_subsystem_sym() && level == BlockLevel::Subsystem;
        let mut omnipresent: Vec<(Ustr, usize)> = omnipresent.into_iter().collect();
        omnipresent.sort();
        let mut graphs: Vec<NamedSymbolGraph> = vec![];
        let mut shown_edges = 0;
        // (The whole tree's, in a big tree: a diagram per group.)
        if is_tree && rollup.members.len() >= self.args.groups_at {
            let omnipresent_syms: std::collections::HashSet<Ustr> =
                omnipresent.iter().map(|(sym, _)| *sym).collect();
            let subsystems: Vec<(Ustr, Ustr)> = rollup
                .members
                .iter()
                .filter(|m| !omnipresent_syms.contains(&m.sym))
                .map(|m| (m.sym, m.pretty))
                .collect();
            let index: std::collections::HashMap<Ustr, usize> = subsystems
                .iter()
                .enumerate()
                .map(|(i, (sym, _))| (*sym, i))
                .collect();
            let weighted: Vec<(usize, usize, u64)> = edges
                .iter()
                .filter_map(|e| Some((*index.get(&e.from)?, *index.get(&e.to)?, e.count as u64)))
                .collect();
            let group_of_index = louvain(subsystems.len(), &weighted);
            let group_of: std::collections::HashMap<Ustr, usize> = subsystems
                .iter()
                .zip(&group_of_index)
                .map(|((sym, _), group)| (*sym, *group))
                .collect();
            // (Each group's subsystems, most calls first, and calls within.)
            let mut members: std::collections::BTreeMap<usize, Vec<Ustr>> = Default::default();
            for ((_, pretty), group) in subsystems.iter().zip(&group_of_index) {
                members.entry(*group).or_default().push(*pretty);
            }
            let mut weight: std::collections::HashMap<usize, u64> = Default::default();
            for e in &edges {
                if let (Some(a), Some(b)) = (group_of.get(&e.from), group_of.get(&e.to))
                    && a == b
                {
                    *weight.entry(*a).or_default() += e.count as u64;
                }
            }
            let mut shown: Vec<usize> = members
                .iter()
                .filter(|(group, names)| names.len() >= 2 && weight.contains_key(group))
                .map(|(group, _)| *group)
                .collect();
            shown.sort_by_key(|group| (std::cmp::Reverse(weight[group]), *group));
            shown.truncate(MAX_GROUPS);
            if shown.len() >= 2 {
                let internal_limit = (self.args.max_edges / 2).max(8);
                for (k, group) in shown.iter().enumerate() {
                    let in_group = |sym: &Ustr| group_of.get(sym) == Some(group);
                    let internal: Vec<&BlockEdge> = edges
                        .iter()
                        .filter(|e| in_group(&e.from) && in_group(&e.to))
                        .take(internal_limit)
                        .collect();
                    // (Its strongest edges to others' subsystems.)
                    let external: Vec<&BlockEdge> = edges
                        .iter()
                        .filter(|e| {
                            in_group(&e.from) != in_group(&e.to)
                                && group_of.contains_key(&e.from)
                                && group_of.contains_key(&e.to)
                        })
                        .take(GROUP_EXTERNAL_EDGES)
                        .collect();
                    let names = &members[group];
                    let mut graph = NamedSymbolGraph::new(format!(
                        "block-diagram {} group {}",
                        focus_sym,
                        k + 1
                    ));
                    graph.graph_attrs.push(("rankdir", "LR".to_string()));
                    graph.graph_attrs.push((
                        "label",
                        format!(
                            "Group {}: {}{} ({} subsystems)",
                            k + 1,
                            names
                                .iter()
                                .take(5)
                                .map(|name| name.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                            if names.len() > 5 { ", …" } else { "" },
                            names.len()
                        ),
                    ));
                    graph.graph_attrs.push(("labelloc", "t".to_string()));
                    add_edges(
                        server,
                        &internal,
                        unit,
                        &mut node_set,
                        &mut edge_set,
                        &mut graph,
                    )
                    .await?;
                    add_edges(
                        server,
                        &external,
                        unit,
                        &mut node_set,
                        &mut edge_set,
                        &mut graph,
                    )
                    .await?;
                    shown_edges += internal.len() + external.len();
                    graphs.push(graph);
                }
                // (Then the omnipresent subsystems, and how many others aren't
                // in the groups shown.)
                let others =
                    subsystems.len() - shown.iter().map(|g| members[g].len()).sum::<usize>();
                let mut graph =
                    NamedSymbolGraph::new(format!("block-diagram {} others", focus_sym));
                graph.graph_attrs.push(("rankdir", "LR".to_string()));
                graph.graph_attrs.push((
                    "label",
                    format!(
                        "{}{}",
                        match omnipresent.is_empty() {
                            true => String::new(),
                            false => "Used by most subsystems (the calls into them aren't drawn)"
                                .to_string(),
                        },
                        match others {
                            0 => String::new(),
                            n => format!(
                                "{}{} other subsystems are in smaller groups, or have none of the strongest calls between subsystems",
                                if omnipresent.is_empty() { "" } else { "; " },
                                n
                            ),
                        }
                    ),
                ));
                graph.graph_attrs.push(("labelloc", "t".to_string()));
                add_omnipresent(server, &omnipresent, &mut node_set, &mut graph).await?;
                graphs.push(graph);
            }
        }
        if graphs.is_empty() {
            edges.truncate(self.args.max_edges);
            shown_edges = edges.len();
            let mut graph = NamedSymbolGraph::new(format!("block-diagram {}", focus_sym));
            // (A class, or a subsystem among its neighbors, even if it has no
            // edges left.)
            if !is_subsystem
                || (level == BlockLevel::Subsystem && focus_sym != root_subsystem_sym())
            {
                server.prefetch_crossref(&[focus_sym]).await;
                let (focus_id, _) = node_set.ensure_symbol(&focus_sym, server, 0).await?;
                graph.ensure_node(focus_id);
            }
            let edges: Vec<&BlockEdge> = edges.iter().collect();
            add_edges(
                server,
                &edges,
                unit,
                &mut node_set,
                &mut edge_set,
                &mut graph,
            )
            .await?;
            // (The whole tree's: left to right, and its omnipresent
            // subsystems, without the calls into them, saying how many use
            // them.)
            if is_tree {
                graph.graph_attrs.push(("rankdir", "LR".to_string()));
                if !omnipresent.is_empty() {
                    graph.graph_attrs.push((
                        "label",
                        "The calls into the subsystems most of the others use aren't drawn; their boxes say how many use them."
                            .to_string(),
                    ));
                    graph.graph_attrs.push(("labelloc", "b".to_string()));
                }
                add_omnipresent(server, &omnipresent, &mut node_set, &mut graph).await?;
            }
            graphs.push(graph);
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
                        "edges": shown_edges,
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
                        "name": "block-groups-at",
                        "label": "Groups at",
                        "value": self.args.groups_at,
                        "default": 20,
                        "range": [1, 100000],
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
                graphs,
                overloads_hit: vec![],
                options,
                hierarchical_graphs: vec![],
            },
        ))
    }
}

#[cfg(test)]
mod louvain_tests {
    use super::louvain;

    /// Two triangles of strong edges joined by a weak one are two groups;
    /// isolated nodes are groups of their own.
    #[test]
    fn test_louvain() {
        let edges = [
            (0, 1, 10),
            (1, 2, 10),
            (2, 0, 10),
            (3, 4, 10),
            (4, 5, 10),
            (5, 3, 10),
            (2, 3, 1),
        ];
        let groups = louvain(7, &edges);
        assert_eq!(groups[0], groups[1]);
        assert_eq!(groups[1], groups[2]);
        assert_eq!(groups[3], groups[4]);
        assert_eq!(groups[4], groups[5]);
        assert_ne!(groups[0], groups[3]);
        assert!(groups[6] != groups[0] && groups[6] != groups[3]);
        assert_eq!(louvain(7, &edges), groups, "deterministic");
    }
}
