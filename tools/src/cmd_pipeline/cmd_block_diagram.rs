use async_trait::async_trait;
use clap::{Args, ValueEnum};
use serde_json::json;
use ustr::{Ustr, ustr};

use super::diagram_mapping::DiagramMapping;
use super::interface::{PipelineCommand, PipelineValues, TextFile};
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

/// How the whole tree's subsystems are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BlockView {
    /// Block diagrams (in big trees, a diagram per group; see `--groups-at`).
    Diagram,
    /// All of its subsystems in a column, by their groups, with their
    /// strongest calls as arcs beside it (see `arc_diagram_svg`).
    Arcs,
}

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

    /// For the whole tree: block diagrams, or a column of arcs.
    #[clap(long, value_enum, default_value_t = BlockView::Diagram)]
    view: BlockView,

    /// Include tests: tests' subsystems (ex: "Testing/GTest") and test code's
    /// classes (see diagram-mapping.toml's `[tests]`), which block diagrams
    /// leave out, as tests obscure how the code proper works.  (Roll-ups
    /// leave out test code's calls, and calls of it; see crossref's
    /// `rollup_left_out`.)
    #[clap(long)]
    include_tests: bool,

    /// A set of classes (symbols, comma-separated) to diagram rather than
    /// `focus` (ex: those a try push's patches change; see `try_review`):
    /// the edges among them, and each one's strongest (`--max-neighbors`)
    /// with classes outside them.
    #[clap(long, value_parser)]
    classes: Option<String>,

    /// For `--classes`: how many of each class's edges with classes outside
    /// them to show (its strongest).
    #[clap(long, default_value_t = 4)]
    max_neighbors: usize,

    /// The groups of diagram-mapping.toml's left-out calls (ex: "strings",
    /// comma-separated, or "all") whose classes to show, which class block
    /// diagrams leave out as calls diagrams leave out their calls (see
    /// `traverse --show-calls`), but the focus's own.
    #[clap(long, value_parser)]
    show_calls: Option<String>,
}

/// Whether a symbol is a class, for block diagrams: a C++ (or other
/// language's) class, or an IPDL struct or message, whose fields or
/// parameters data flows through (see `dataflow::summary::IpdlSlots`).
fn is_block_class(sym: &str) -> bool {
    sym.starts_with("T_") || sym.starts_with("IPDL_") || sym.starts_with("IPC_")
}

/// A class's or subsystem's name, by its symbol, for people (ex:
/// "Core/Storage/Cache API" for `SUBSYS_Core/Storage/Cache@20API`, whose
/// characters `mangle_file` escaped, or "mozilla::Foo" for `T_mozilla::Foo`).
fn block_display_name(sym: &str) -> String {
    let name = sym
        .strip_prefix(SUBSYSTEM_PREFIX)
        .or_else(|| sym.strip_prefix("T_"))
        .unwrap_or(sym);
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        match rest
            .get(at + 1..at + 3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            Some(byte) => {
                out.push(byte as char);
                rest = &rest[at + 3..];
            }
            None => {
                out.push('@');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Why a diagram of the classes between two sides (`--with`, ex: a subsystem
/// edge's "Classes between...") has none, for its label: the roll-ups between
/// classes have their members' calls (or data), so the subsystems' calls of
/// functions outside classes, of class templates (which have no records, ex:
/// a smart pointer's), or of left-out classes, aren't in them.
fn no_classes_between(focus: &str, with: &str, edges: BlockEdges) -> String {
    let (focus, with) = (block_display_name(focus), block_display_name(with));
    match edges {
        BlockEdges::Calls => format!(
            "None of {}'s classes call {}'s, nor the reverse:\\nthe calls between them are of functions outside classes, or of class templates (ex: smart pointers),\\nwhich classes diagrams don't have, or of the left-out classes the panel lists.",
            focus, with
        ),
        BlockEdges::Data => format!(
            "No data flows between {}'s classes and {}'s:\\nwhat flows between them goes through functions outside classes, or through class templates,\\nwhich classes diagrams don't have, or through the left-out classes the panel lists.",
            focus, with
        ),
    }
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

/// A group of the whole tree's subsystems (see `subsystem_groups`).
struct SubsystemGroup {
    /// Its subsystems (symbols and pretty names), most calls first.
    members: Vec<(Ustr, Ustr)>,
    /// The calls within it.
    weight: u64,
}

/// The whole tree's subsystems but the omnipresent ones (see
/// `omnipresent_subsystems`), grouped by `edges`' calls between them (see
/// `louvain`), the groups with the most calls within first (then the
/// biggest), and each subsystem's group (its index).
fn subsystem_groups(
    rollup: &Rollup,
    omnipresent: &[(Ustr, usize)],
    edges: &[BlockEdge],
) -> (Vec<SubsystemGroup>, std::collections::HashMap<Ustr, usize>) {
    use std::collections::{BTreeMap, HashMap, HashSet};
    let omnipresent: HashSet<Ustr> = omnipresent.iter().map(|(sym, _)| *sym).collect();
    let subsystems: Vec<(Ustr, Ustr)> = rollup
        .members
        .iter()
        .filter(|m| !omnipresent.contains(&m.sym))
        .map(|m| (m.sym, m.pretty))
        .collect();
    let index: HashMap<Ustr, usize> = subsystems
        .iter()
        .enumerate()
        .map(|(i, (sym, _))| (*sym, i))
        .collect();
    let weighted: Vec<(usize, usize, u64)> = edges
        .iter()
        .filter_map(|e| Some((*index.get(&e.from)?, *index.get(&e.to)?, e.count as u64)))
        .collect();
    let group_of_index = louvain(subsystems.len(), &weighted);
    let mut members: BTreeMap<usize, Vec<(Ustr, Ustr)>> = BTreeMap::new();
    for (subsystem, group) in subsystems.iter().zip(&group_of_index) {
        members.entry(*group).or_default().push(*subsystem);
    }
    let mut weight: HashMap<usize, u64> = HashMap::new();
    for (a, b, w) in &weighted {
        if group_of_index[*a] == group_of_index[*b] {
            *weight.entry(group_of_index[*a]).or_default() += w;
        }
    }
    let mut groups: Vec<(usize, SubsystemGroup)> = members
        .into_iter()
        .map(|(group, members)| {
            let weight = weight.get(&group).copied().unwrap_or(0);
            (group, SubsystemGroup { members, weight })
        })
        .collect();
    groups.sort_by_key(|(group, g)| {
        (
            std::cmp::Reverse(g.weight),
            std::cmp::Reverse(g.members.len()),
            *group,
        )
    });
    let renumber: HashMap<usize, usize> = groups
        .iter()
        .enumerate()
        .map(|(i, (group, _))| (*group, i))
        .collect();
    let group_of = subsystems
        .iter()
        .zip(&group_of_index)
        .map(|((sym, _), group)| (*sym, renumber[group]))
        .collect();
    (
        groups.into_iter().map(|(_, group)| group).collect(),
        group_of,
    )
}

/// How many arcs the whole tree's column of arcs shows at most (see
/// `arc_diagram_svg`).
const ARC_EDGES: usize = 400;

/// Text for SVG (its text, and its attributes' values).
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The whole tree's subsystems in a column, by their groups (see
/// `subsystem_groups`), each linking to its block diagram, with `arcs` (its
/// strongest calls between them, of `unit`) beside it: each from its caller's
/// row to its callee's, bulging right as far as the rows are apart (up to
/// `ARC_BULGE`), blue going down and orange going up, as thick as its count
/// among the arcs' (by logarithms); then the omnipresent subsystems (see
/// `omnipresent_subsystems`), without the calls into them.  Hovering over a
/// subsystem highlights its calls, and dims the others.  It flows down the
/// page, at any number of subsystems (ex: firefox-disco's 253, but those
/// without any of the arcs), as a block diagram of them can't.
fn arc_diagram_svg(
    tree: &str,
    groups: &[SubsystemGroup],
    group_of: &std::collections::HashMap<Ustr, usize>,
    omnipresent: &[(Ustr, usize)],
    pretty_of: &std::collections::HashMap<Ustr, Ustr>,
    arcs: &[&BlockEdge],
    unit: &str,
) -> String {
    use std::collections::{HashMap, HashSet};
    use std::fmt::Write;
    const ROW: f64 = 16.0;
    const HEADER: f64 = 34.0;
    const LABEL_RIGHT: f64 = 300.0;
    const ARC_LEFT: f64 = 308.0;
    const ARC_BULGE: f64 = 380.0;
    let query = |q: String| {
        let mut url = url::form_urlencoded::Serializer::new(String::new());
        url.append_pair("q", &q);
        format!("/{}/query/default?{}", tree, url.finish())
    };
    let quote = |name: &str| match name.contains('\'') {
        true => format!("\"{}\"", name),
        false => format!("'{}'", name),
    };
    // (The rows: the subsystems with arcs, by their groups.)
    let with_arcs: HashSet<Ustr> = arcs.iter().flat_map(|e| [e.from, e.to]).collect();
    let mut y = 44.0;
    let mut rows: Vec<(Ustr, Ustr, f64)> = vec![];
    let mut headers: Vec<(String, f64, f64)> = vec![];
    for (g, group) in groups.iter().enumerate() {
        let members: Vec<&(Ustr, Ustr)> = group
            .members
            .iter()
            .filter(|(sym, _)| with_arcs.contains(sym))
            .collect();
        if members.is_empty() {
            continue;
        }
        let top = y;
        y += HEADER;
        for (sym, pretty) in members {
            rows.push((*sym, *pretty, y));
            y += ROW;
        }
        let names = group
            .members
            .iter()
            .take(3)
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>();
        headers.push((
            format!(
                "Group {}: {}{} ({} subsystems)",
                g + 1,
                names.join(", "),
                if group.members.len() > 3 { ", …" } else { "" },
                group.members.len()
            ),
            top,
            y,
        ));
        y += ROW / 2.0;
    }
    let omnipresent_top = y;
    if !omnipresent.is_empty() {
        y += HEADER + ROW * omnipresent.len() as f64;
    }
    let height = y + ROW;
    let width = ARC_LEFT + ARC_BULGE + 24.0;
    let row_y: HashMap<Ustr, f64> = rows.iter().map(|(sym, _, y)| (*sym, *y)).collect();
    let log = |n: u32| (n.max(1) as f64).ln();
    let (least, most) = arcs.iter().fold((f64::MAX, 0.0f64), |(l, m), e| {
        (l.min(log(e.count)), m.max(log(e.count)))
    });

    let mut svg = String::new();
    let _ = write!(
        svg,
        r##"<svg class="arcs" xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="sans-serif" font-size="11">
<defs>
<marker id="arcs-head-down" viewBox="0 0 6 6" refX="5" refY="3" markerWidth="5" markerHeight="5" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#3b78d8"/></marker>
<marker id="arcs-head-up" viewBox="0 0 6 6" refX="5" refY="3" markerWidth="5" markerHeight="5" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#d8783b"/></marker>
</defs>
<text x="4" y="14" font-size="12">The tree's subsystems by their groups, with their {n} strongest {unit} between them as arcs:</text>
<text x="4" y="30" font-size="12">blue going down, orange going up, thicker for more; hover over a subsystem for its own. <a href="{diagrams}"><tspan fill="#0095dd">Block diagrams</tspan></a></text>
"##,
        w = width,
        h = height,
        n = arcs.len(),
        unit = xml_escape(unit),
        diagrams = xml_escape(&query("block-diagram:*".to_string())),
    );
    for (title, top, bottom) in &headers {
        let _ = writeln!(
            svg,
            r#"<rect class="arcs-band" x="0" y="{}" width="{}" height="{}"/><text x="4" y="{}" font-weight="bold">{}</text>"#,
            top,
            width,
            bottom - top,
            top + 15.0,
            xml_escape(title)
        );
    }
    // (Each row with the arcs of its calls, for its hovering.)
    for (sym, pretty, y) in &rows {
        let _ = writeln!(
            svg,
            r#"<g class="arcs-row" data-group="{}">"#,
            group_of[sym] + 1
        );
        for e in arcs.iter().filter(|e| e.from == *sym) {
            let Some(&to_y) = row_y.get(&e.to) else {
                continue;
            };
            let down = to_y > *y;
            let ry = (to_y - y).abs() / 2.0;
            let width = match most > least {
                true => 0.75 + 3.25 * (log(e.count) - least) / (most - least),
                false => 2.0,
            };
            let _ = writeln!(
                svg,
                r#"<path class="arcs-arc {}" d="M{} {} A{} {} 0 0 {} {} {}" stroke-width="{:.2}" marker-end="url(#arcs-head-{})"><title>{} → {}: {} {}</title></path>"#,
                if down { "arcs-down" } else { "arcs-up" },
                ARC_LEFT,
                y - 3.0,
                ry.min(ARC_BULGE).max(1.0),
                ry.max(1.0),
                if down { 1 } else { 0 },
                ARC_LEFT,
                to_y - 3.0,
                width,
                if down { "down" } else { "up" },
                xml_escape(pretty),
                xml_escape(
                    rows.iter()
                        .find(|r| r.0 == e.to)
                        .map_or("", |r| r.1.as_str())
                ),
                e.count,
                xml_escape(unit)
            );
        }
        let _ = writeln!(
            svg,
            r#"<a href="{}"><text x="{}" y="{}" text-anchor="end">{}</text></a></g>"#,
            xml_escape(&query(format!("block-diagram:{}", quote(pretty)))),
            LABEL_RIGHT,
            y,
            xml_escape(pretty)
        );
    }
    if !omnipresent.is_empty() {
        let _ = writeln!(
            svg,
            r#"<text x="4" y="{}" font-weight="bold">Used by most subsystems (the calls into them aren't drawn)</text>"#,
            omnipresent_top + 15.0
        );
        for (i, (sym, callers)) in omnipresent.iter().enumerate() {
            let name = pretty_of.get(sym).map_or(sym.as_str(), |p| p.as_str());
            let _ = writeln!(
                svg,
                r#"<a href="{}"><text x="{}" y="{}" text-anchor="end">{} (used by {})</text></a>"#,
                xml_escape(&query(format!("block-diagram:{}", quote(&name)))),
                LABEL_RIGHT,
                omnipresent_top + HEADER + ROW * i as f64,
                xml_escape(&name),
                callers
            );
        }
    }
    svg.push_str("</svg>\n");
    svg
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

/// The whole tree's infrastructure subsystems (by diagram-mapping.toml's
/// `[overview]`, ex: IPC, WebIDL bindings), by how many subsystems call
/// them, which are drawn as the omnipresent ones are (see
/// `omnipresent_subsystems`): most code uses them as plumbing (IPC's
/// serialization, bindings' error results), or through generated code, and
/// what they connect (ex: a message's sender and receivers, whose roll-ups
/// count its `Send` as a call of its `Recv`s) is drawn without them.
fn infrastructure_subsystems(
    rollup: &Rollup,
    mapping: &DiagramMapping,
) -> std::collections::HashMap<Ustr, usize> {
    let mut callers: std::collections::HashMap<Ustr, usize> = Default::default();
    for member in &rollup.members {
        if mapping.is_infrastructure_subsystem(&member.pretty) {
            callers.insert(
                member.sym,
                rollup.edges.iter().filter(|e| e.to == member.sym).count(),
            );
        }
    }
    callers
}

/// The classes a class block diagram leaves out (but its focus's own, ex: a
/// subsystem's members): generated classes; test code's (see
/// `DiagramMapping::is_test_xref`) and tests' subsystems', unless
/// `include_tests`; and those of
/// diagram-mapping.toml's groups of left-out calls (ex: strings, containers,
/// IPC's and bindings' plumbing; a class is in a group if its functions are,
/// by its pretty name, or its definitions' paths), unless their groups are
/// shown, which the diagram's legend lists (see `to_json`).
struct LeftOutClasses {
    mapping: std::sync::Arc<DiagramMapping>,
    shown: Vec<bool>,
    include_tests: bool,
    /// For each group, its classes left out (their pretty names), and the
    /// diagram's classes calling (or called by) them.
    groups: Vec<(
        std::collections::BTreeSet<Ustr>,
        std::collections::BTreeSet<Ustr>,
    )>,
}

impl LeftOutClasses {
    fn new(
        mapping: std::sync::Arc<DiagramMapping>,
        show_calls: Option<&str>,
        include_tests: bool,
    ) -> Self {
        let shown = mapping.shown_groups(show_calls);
        let groups = vec![Default::default(); mapping.groups.len()];
        LeftOutClasses {
            mapping,
            shown,
            include_tests,
            groups,
        }
    }

    /// Whether to leave out a class (by its crossref entry), recording its
    /// group's.  (Generated classes are left out too, ex: IPDL actors' base
    /// classes, between their subclasses, which roll-ups count IPC messages
    /// between, or WebIDL unions; and roll-ups leave out generated code; see
    /// crossref's `rollup_left_out`.)
    fn leaves_out(&mut self, xref: &crate::file_format::crossref::CrossrefData) -> Option<usize> {
        let meta = xref.meta.as_ref()?;
        if xref
            .definitions
            .as_ref()
            .is_some_and(|defs| !defs.is_empty() && defs.iter().all(|d| d.path_kind == "generated"))
        {
            return Some(usize::MAX);
        }
        if !self.include_tests
            && (self.mapping.is_test_xref(xref)
                || meta
                    .subsystem
                    .as_ref()
                    .is_some_and(|subsystem| self.mapping.is_test_subsystem(subsystem)))
        {
            return Some(usize::MAX);
        }
        let paths: Vec<&str> = xref
            .definitions
            .iter()
            .flatten()
            .map(|def| def.path.as_str())
            .collect();
        let group = self
            .mapping
            .group_of_symbol(&format!("{}::", meta.pretty), &paths, false)?;
        (!self.shown[group]).then_some(group)
    }

    /// The groups whose classes it left out, for the diagram's legend, as
    /// calls diagrams' (see `LeftOutCalls::to_json` in cmd_traverse.rs, and
    /// static/js/search.js's `addLeftOutCalls`).
    fn to_json(&self) -> serde_json::Value {
        const MAX_PRETTIES: usize = 16;
        let groups: Vec<serde_json::Value> = self
            .groups
            .iter()
            .enumerate()
            .filter(|(_, (classes, _))| !classes.is_empty())
            .map(|(i, (classes, callers))| {
                let group = &self.mapping.groups[i];
                json!({
                    "group": group.group,
                    "name": group.name,
                    "why": group.why,
                    "shown": false,
                    "count": classes.len(),
                    "callees": classes.iter().take(MAX_PRETTIES).collect::<Vec<_>>(),
                    "callers": callers,
                })
            })
            .collect();
        json!({
            // (No "label", so that the control panel leaves it out.)
            "name": "*left-out*",
            "what": "classes",
            "groups": groups,
        })
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
        // (Or a set of classes; see `classes`.)
        let classes: Vec<Ustr> = self
            .args
            .classes
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|class| !class.is_empty())
            .map(ustr)
            .collect();
        let focus_sym = match classes.first() {
            Some(class) => *class,
            None if focus.is_empty() => root_subsystem_sym(),
            None => block_sym(focus),
        };
        let is_subsystem = focus_sym.starts_with(SUBSYSTEM_PREFIX);
        // (The whole tree's subsystems, else classes.)
        let level = self
            .args
            .level
            .unwrap_or(if focus.is_empty() && classes.is_empty() {
                BlockLevel::Subsystem
            } else {
                BlockLevel::Class
            });
        let mapping = server.diagram_mapping();
        let mut rollup = if level == BlockLevel::Field || !classes.is_empty() {
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
        if !classes.is_empty() {
            // A set of classes: the edges among them, and with classes
            // outside them (cut to each one's strongest below, once the
            // left-out classes are left out).
            server.prefetch_crossref(&classes).await;
            own.extend(classes.iter().copied());
            for class in &classes {
                let Some(class_rollup) = rollup_of(server, class, self.args.edges).await? else {
                    continue;
                };
                for e in &class_rollup.out {
                    edges.push(BlockEdge {
                        from: *class,
                        to: e.sym,
                        count: edge_count(e),
                        example: first_example(e),
                        via: None,
                        ipc: e.ipc,
                    });
                }
                // (Those from the others are their `out`s.)
                for e in class_rollup
                    .into
                    .iter()
                    .filter(|e| !classes.contains(&e.sym))
                {
                    edges.push(BlockEdge {
                        from: e.sym,
                        to: *class,
                        count: edge_count(e),
                        example: first_example(e),
                        via: None,
                        ipc: e.ipc,
                    });
                }
            }
        } else if level == BlockLevel::Field {
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
            // The whole tree's subsystems, but tests' (unless asked for), and
            // not the calls into omnipresent ones, and infrastructure.
            if let Some(mapping) = mapping.as_deref().filter(|_| !self.args.include_tests) {
                let tests: std::collections::HashSet<Ustr> = rollup
                    .members
                    .iter()
                    .filter(|m| mapping.is_test_subsystem(&m.pretty))
                    .map(|m| m.sym)
                    .collect();
                rollup.members.retain(|m| !tests.contains(&m.sym));
                rollup
                    .edges
                    .retain(|e| !tests.contains(&e.from) && !tests.contains(&e.to));
            }
            omnipresent = omnipresent_subsystems(&rollup);
            if let Some(mapping) = mapping.as_deref() {
                for (sym, callers) in infrastructure_subsystems(&rollup, mapping) {
                    omnipresent.entry(sym).or_insert(callers);
                }
            }
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
            // A subsystem's classes, and their neighbors outside it (but
            // test code's classes, unless asked for, which can have the most
            // calls, ex: a subsystem's gtests').
            let candidates: Vec<Ustr> = rollup
                .members
                .iter()
                .filter(|m| is_block_class(&m.sym))
                .take(self.args.max_members * 2)
                .map(|m| m.sym)
                .collect();
            server.prefetch_crossref(&candidates).await;
            let mut members: Vec<Ustr> = vec![];
            for sym in candidates {
                if members.len() >= self.args.max_members {
                    break;
                }
                let is_test = match (mapping.as_deref(), self.args.include_tests) {
                    (Some(mapping), false) => server
                        .crossref_lookup(&sym)
                        .await?
                        .is_some_and(|xref| mapping.is_test_xref(&xref)),
                    _ => false,
                };
                if !is_test {
                    members.push(sym);
                }
            }
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
            // A class's, or a subsystem's, neighbors (but tests' subsystems,
            // unless asked for).
            let test_subsystem = |e: &RollupEdge| {
                !self.args.include_tests
                    && e.sym.starts_with(SUBSYSTEM_PREFIX)
                    && mapping
                        .as_deref()
                        .is_some_and(|mapping| mapping.is_test_subsystem(&e.pretty))
            };
            rollup.out.retain(|e| !test_subsystem(e));
            rollup.into.retain(|e| !test_subsystem(e));
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
        // (Class diagrams leave out tests' and infrastructure's classes; see
        // `LeftOutClasses`.)
        let mut left_out_classes = match (level, &mapping) {
            (BlockLevel::Class, Some(mapping)) => Some(LeftOutClasses::new(
                mapping.clone(),
                self.args.show_calls.as_deref(),
                self.args.include_tests,
            )),
            _ => None,
        };
        if let Some(left_out) = left_out_classes.as_mut() {
            let mut others: Vec<Ustr> = edges
                .iter()
                .flat_map(|e| [e.from, e.to])
                .filter(|sym| !own.contains(sym) && is_block_class(sym))
                .collect();
            others.sort();
            others.dedup();
            server.prefetch_crossref(&others).await;
            let mut dropped: std::collections::HashMap<Ustr, usize> = Default::default();
            for sym in others {
                if let Some(xref) = server.crossref_lookup(&sym).await?
                    && let Some(group) = left_out.leaves_out(&xref)
                {
                    dropped.insert(sym, group);
                }
            }
            for e in &edges {
                for (end, other) in [(e.from, e.to), (e.to, e.from)] {
                    if let Some(&group) = dropped.get(&end)
                        && group != usize::MAX
                        && !dropped.contains_key(&other)
                    {
                        let pretty = server
                            .crossref_lookup(&end)
                            .await?
                            .and_then(|data| data.meta)
                            .map_or(end, |meta| meta.pretty);
                        left_out.groups[group].0.insert(pretty);
                        left_out.groups[group].1.insert(other);
                    }
                }
            }
            edges.retain(|e| !dropped.contains_key(&e.from) && !dropped.contains_key(&e.to));
        }
        // (A set of classes' edges with others: each one's strongest.)
        if !classes.is_empty() {
            edges.sort_by(|a, b| b.count.cmp(&a.count));
            let mut outside: std::collections::HashMap<Ustr, usize> = Default::default();
            edges.retain(|e| {
                let (Some(class), false) = (
                    [e.from, e.to].into_iter().find(|end| own.contains(end)),
                    own.contains(&e.from) && own.contains(&e.to),
                ) else {
                    return true;
                };
                let shown = outside.entry(class).or_default();
                *shown += 1;
                *shown <= self.args.max_neighbors
            });
        }
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
        // (The whole tree's as a column of arcs.)
        if is_tree && self.args.view == BlockView::Arcs {
            let (groups, group_of) = subsystem_groups(&rollup, &omnipresent, &edges);
            let arcs: Vec<&BlockEdge> = edges
                .iter()
                .filter(|e| group_of.contains_key(&e.from) && group_of.contains_key(&e.to))
                .take(ARC_EDGES)
                .collect();
            let tree = server.tree_info()?.name;
            let pretty_of: std::collections::HashMap<Ustr, Ustr> =
                rollup.members.iter().map(|m| (m.sym, m.pretty)).collect();
            return Ok(PipelineValues::TextFile(TextFile {
                mime_type: "image/svg+xml".to_string(),
                contents: arc_diagram_svg(
                    &tree,
                    &groups,
                    &group_of,
                    &omnipresent,
                    &pretty_of,
                    &arcs,
                    unit,
                ),
                transient: false,
                facets: None,
            }));
        }
        let mut graphs: Vec<NamedSymbolGraph> = vec![];
        let mut shown_edges = 0;
        // (The whole tree's, in a big tree: a diagram per group.)
        if is_tree && rollup.members.len() >= self.args.groups_at {
            let (groups, group_of) = subsystem_groups(&rollup, &omnipresent, &edges);
            let shown: Vec<usize> = (0..groups.len())
                .filter(|&g| groups[g].members.len() >= 2 && groups[g].weight > 0)
                .take(MAX_GROUPS)
                .collect();
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
                    let names: Vec<Ustr> = groups[*group]
                        .members
                        .iter()
                        .map(|(_, pretty)| *pretty)
                        .collect();
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
                let others = group_of.len()
                    - shown
                        .iter()
                        .map(|g| groups[*g].members.len())
                        .sum::<usize>();
                let mut graph =
                    NamedSymbolGraph::new(format!("block-diagram {} others", focus_sym));
                graph.graph_attrs.push(("rankdir", "LR".to_string()));
                graph.graph_attrs.push((
                    "label",
                    format!(
                        "{}{}",
                        match omnipresent.is_empty() {
                            true => String::new(),
                            false => "Infrastructure, and used by most subsystems (the calls into them aren't drawn)"
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
            // (None between the focus's classes and the other side's, which
            // its label says why.)
            if let Some(with) = with_sym
                && level != BlockLevel::Subsystem
                && edges.is_empty()
            {
                graph.graph_attrs.push((
                    "label",
                    no_classes_between(&focus_sym, &with, self.args.edges),
                ));
                graph.graph_attrs.push(("labelloc", "t".to_string()));
            }
            // (A class, or a subsystem among its neighbors, even if it has no
            // edges left.)
            if !is_subsystem
                || (level == BlockLevel::Subsystem && focus_sym != root_subsystem_sym())
            {
                server.prefetch_crossref(&[focus_sym]).await;
                let (focus_id, _) = node_set.ensure_symbol(&focus_sym, server, 0).await?;
                graph.ensure_node(focus_id);
            }
            // (And each of a set of classes.)
            for class in classes.iter().skip(1) {
                let (id, _) = node_set.ensure_symbol(class, server, 0).await?;
                graph.ensure_node(id);
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
                        "The calls into infrastructure and into the subsystems most of the others use aren't drawn; their boxes say how many use them."
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
                        "name": "block-view",
                        "label": "View",
                        "value": "diagram",
                        "default": "diagram",
                        "choices": [
                            { "value": "diagram", "label": "Block diagrams" },
                            { "value": "arcs", "label": "All subsystems, with arcs (the whole tree)" },
                        ],
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
                    {
                        "name": "include-tests",
                        "label": "Tests",
                        "value": self.args.include_tests,
                        "default": false,
                        "type": "bool",
                    },
                    {
                        "name": "show-calls",
                        "label": "Left-out classes shown",
                        "value": self.args.show_calls.clone().unwrap_or_default(),
                        "default": "",
                        "type": "string",
                        "placeholder": "Groups (ex: strings), comma-separated, or all",
                    },
                ],
            }
        ]);
        let mut options = options;
        if let Some(left_out) = &left_out_classes {
            options[0]["items"]
                .as_array_mut()
                .unwrap()
                .push(left_out.to_json());
        }
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
mod infrastructure_tests {
    use super::*;
    use crate::file_format::rollup::{RollupMember, RollupPair};

    /// The whole tree's infrastructure subsystems (by diagram-mapping.toml's
    /// `[overview]`), by how many subsystems call them.
    /// Classes' and subsystems' names for people, and the label of a diagram
    /// of the classes between two sides which has none.
    #[test]
    fn test_no_classes_between() {
        assert_eq!(
            block_display_name("SUBSYS_Core/Storage/Cache@20API"),
            "Core/Storage/Cache API"
        );
        assert_eq!(block_display_name("T_mozilla::Foo"), "mozilla::Foo");
        assert_eq!(block_display_name("SUBSYS_A@ZZ"), "A@ZZ");
        let label = no_classes_between(
            "SUBSYS_Core/Storage/Cache@20API",
            "SUBSYS_Core/Storage/IndexedDB",
            BlockEdges::Calls,
        );
        assert!(
            label.starts_with(
                "None of Core/Storage/Cache API's classes call Core/Storage/IndexedDB's, nor the reverse:\\n"
            ),
            "{}",
            label
        );
        assert!(
            no_classes_between("SUBSYS_A", "T_B", BlockEdges::Data)
                .starts_with("No data flows between A's classes and B's:")
        );
    }

    #[test]
    fn test_infrastructure_subsystems() {
        let mapping = DiagramMapping::parse(
            "[overview]\ninfrastructure = [\"Core/IPC\", \"Core/DOM/Bindings (WebIDL)\"]\n",
        )
        .unwrap();
        let member = |pretty: &str| RollupMember {
            sym: subsystem_sym(pretty),
            pretty: ustr(pretty),
            count: 1,
        };
        let pair = |from: &str, to: &str| RollupPair {
            from: subsystem_sym(from),
            to: subsystem_sym(to),
            count: 10,
            pairs: 1,
        };
        let rollup = Rollup {
            members: vec![
                member("Core/IPC"),
                member("Core/Networking"),
                member("Core/DOM/Workers"),
            ],
            edges: vec![
                pair("Core/Networking", "Core/IPC"),
                pair("Core/DOM/Workers", "Core/IPC"),
                pair("Core/DOM/Workers", "Core/Networking"),
            ],
            ..Default::default()
        };
        let infrastructure = infrastructure_subsystems(&rollup, &mapping);
        assert_eq!(infrastructure.len(), 1);
        assert_eq!(infrastructure.get(&subsystem_sym("Core/IPC")), Some(&2));
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
