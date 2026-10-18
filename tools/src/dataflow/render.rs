//! Showing traces: as text (for terminals and tests), and as graphviz DOT,
//! with a cluster for each function, so the diagram reads as which functions
//! a value passes through and how.

use std::collections::HashMap;
use std::fmt::Write;

use super::trace::{EdgeKind, NodeKind, TraceGraph, short_pretty};

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn render_text(graph: &TraceGraph) -> String {
    let mut out = String::new();
    // (Functions in the order the trace reached them.)
    let mut order: Vec<(String, String)> = vec![];
    for node in &graph.nodes {
        let key = (node.path.clone(), node.func.clone());
        if !order.contains(&key) {
            order.push(key);
        }
    }
    let mut outgoing: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, edge) in graph.edges.iter().enumerate() {
        outgoing.entry(edge.from).or_default().push(i);
    }
    for (path, func) in &order {
        let _ = writeln!(out, "{} ({})", func, path);
        let mut nodes: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.path == *path && n.func == *func)
            .collect();
        nodes.sort_by_key(|n| (n.line, n.id));
        for node in nodes {
            let _ = writeln!(
                out,
                "  #{:<3} {:>5} {:<10} {:<34} {}",
                node.id,
                node.line,
                format!("{:?}", node.kind).to_lowercase(),
                node.label,
                node.text
            );
            for &ei in outgoing.get(&node.id).into_iter().flatten() {
                let edge = &graph.edges[ei];
                let to = &graph.nodes[edge.to];
                let place = if to.path == node.path && to.func == node.func {
                    String::new()
                } else {
                    format!(" in {}", short_pretty(&to.func))
                };
                let _ = writeln!(
                    out,
                    "        -> #{} {}{} [{}: {}]",
                    to.id,
                    to.label,
                    place,
                    format!("{:?}", edge.kind).to_lowercase(),
                    edge.label
                );
            }
        }
    }
    for note in &graph.notes {
        let _ = writeln!(out, "note: {}", note);
    }
    out
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Default)]
pub struct DotOptions {
    /// Link nodes to their source lines in this tree.
    pub link_tree: Option<String>,
    /// At this revision (a try push's head, for a trace in its context; see
    /// `try_delta`), else the indexed one.
    pub link_rev: Option<String>,
    /// Title the graph with what was followed, and note where the trace
    /// stopped or guessed.
    pub title: bool,
}

/// The trace's seed's description (ex: "Where aRetVal (Preferences.cpp:2903)
/// goes").
fn title(graph: &TraceGraph) -> String {
    let seed = graph.nodes.iter().find(|n| n.kind == NodeKind::Seed);
    let what = match seed {
        Some(seed) => format!("{} ({}:{})", seed.label, file_name(&seed.path), seed.line),
        None => "the value".to_string(),
    };
    match graph.dir {
        Some(super::trace::Dir::Backward) => format!("Where {} comes from", what),
        _ => format!("Where {} goes", what),
    }
}

pub fn render_dot(graph: &TraceGraph, options: &DotOptions) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "digraph dataflow {{");
    let _ = writeln!(out, "  rankdir=TB; compound=true;");
    if options.title {
        let mut label = escape(&title(graph));
        for note in &graph.notes {
            label.push_str("\\n");
            label.push_str(&escape(note));
        }
        let _ = writeln!(
            out,
            "  label=\"{}\"; labelloc=t; fontname=\"sans-serif\"; fontsize=12;",
            label
        );
    }
    let _ = writeln!(
        out,
        "  node [shape=box, fontname=\"monospace\", fontsize=10, style=rounded];"
    );
    let _ = writeln!(out, "  edge [fontname=\"monospace\", fontsize=9];");
    let mut order: Vec<(String, String)> = vec![];
    for node in &graph.nodes {
        let key = (node.path.clone(), node.func.clone());
        if !order.contains(&key) {
            order.push(key);
        }
    }
    for (ci, (path, func)) in order.iter().enumerate() {
        let _ = writeln!(out, "  subgraph cluster_{} {{", ci);
        let _ = writeln!(
            out,
            "    label=\"{}\\n{}\"; style=rounded; color=gray;",
            escape(&short_pretty(func)),
            escape(file_name(path))
        );
        for node in graph
            .nodes
            .iter()
            .filter(|n| n.path == *path && n.func == *func)
        {
            let text = if !node.detail.is_empty() {
                format!("{}: {}", node.line, node.detail)
            } else if node.text.is_empty() {
                node.label.clone()
            } else {
                format!("{}: {}", node.line, node.text)
            };
            let (shape, color) = match node.kind {
                NodeKind::Seed => ("box", "black\", penwidth=\"2"),
                NodeKind::Param => ("cds", "gray30"),
                NodeKind::Const => ("plaintext", "gray30"),
                NodeKind::Opaque => ("box", "orange"),
                NodeKind::Summary => ("box", "steelblue"),
                NodeKind::Test => ("diamond", "gray30"),
                NodeKind::Return => ("box", "gray30"),
                _ => ("box", "black"),
            };
            let label = match node.kind {
                NodeKind::Param
                | NodeKind::Seed
                | NodeKind::Const
                | NodeKind::Opaque
                | NodeKind::Summary => node.label.clone(),
                _ => text,
            };
            let link = match &options.link_tree {
                Some(tree) if node.line > 0 => format!(
                    ", URL=\"/{}/{}/{}#{}\", tooltip=\"{}:{}\"",
                    escape(tree),
                    match &options.link_rev {
                        Some(rev) => format!("rev/{}", escape(rev)),
                        None => "source".to_string(),
                    },
                    escape(&node.path),
                    node.line,
                    escape(file_name(&node.path)),
                    node.line
                ),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "    n{} [label=\"{}\", shape={}, color=\"{}\"{}];",
                node.id,
                escape(&label),
                shape,
                color,
                link
            );
        }
        let _ = writeln!(out, "  }}");
    }
    for edge in &graph.edges {
        let (color, style) = match edge.kind {
            EdgeKind::Local => ("black", "solid"),
            EdgeKind::Call => ("blue", "solid"),
            EdgeKind::Return => ("darkgreen", "solid"),
            EdgeKind::Field => ("purple", "dashed"),
            EdgeKind::Binding => ("red", "bold"),
            EdgeKind::Assumed => ("orange", "dashed"),
            EdgeKind::Oracle => ("darkorange3", "bold"),
        };
        let _ = writeln!(
            out,
            "  n{} -> n{} [label=\"{}\", color=\"{}\", style={}];",
            edge.from,
            edge.to,
            escape(&edge.label),
            color,
            style
        );
    }
    let _ = writeln!(out, "}}");
    out
}
