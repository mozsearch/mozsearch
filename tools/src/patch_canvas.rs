//! The canvas of a try push (`/{tree}/rev-canvas/{head}`; see
//! `format::format_try_canvas`): the functions the push adds or changes, as
//! cards of their code, and the calls among them, and between them and a few of
//! the functions calling them or called by them which it doesn't change, for
//! laying out by their control flow on an infinite canvas (see
//! static/js/rev-canvas.js and graph.js).  It's from the push's semantic delta
//! (see `try_delta`), whose analysis is the compiler's, so that the calls are
//! the ones the code makes, not a parse's guesses, through the tree's server
//! as the push changes it (see `AbstractServer::try_overlay`), so that it can
//! be built over a remote tree's too (see `RemoteServer`).
//!
//! - A function the push changes is a definition (in its analysis of a file it
//!   changes) whose lines, from its name to the end of its body, include lines
//!   the push adds or changes, or lines it removes from between them.  It's
//!   "added" if its file's analysis at the indexed revision has no definition
//!   of that name (so a changed signature, a new symbol, is "changed"), and
//!   otherwise "changed".
//! - The calls are the functions' callees in the push's crossref data, and their
//!   uses' contexts (their callers), with the lines making the calls.
//! - The cards' code is the head's source listing's rows of the functions'
//!   lines (up to `MAX_NODE_LINES`), with the lines the push adds or changes
//!   marked, and for the context's functions, their definitions' lines.
//!
//! The model is a function of the delta, so that it can be cached and shared:
//! ex: by a service keeping people's notes on a push's canvas, which refer to
//! its nodes by their ids (their symbols), and its users' own arrangements of
//! it, which the page keeps (see rev-canvas.js).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;
use serde_json::Value;
use tokio_stream::StreamExt;
use ustr::{Ustr, UstrMap, ustr};

use crate::abstract_server::AbstractServer;
use crate::abstract_server::sym_info::add_html_sym_info;
use crate::file_format::analysis::{analysis_from_values, read_source};
use crate::file_format::jumpref::{JumprefData, JumprefTraversals};
use crate::format::excerpt_row_html;
use crate::rev_lines::changed_lines;
use crate::try_delta::FileChange;
use crate::url_encode_path::url_encode_path;

/// The version of the model's format.
pub const CANVAS_VERSION: u32 = 1;

/// How many lines of a function a card has at most.
pub const MAX_NODE_LINES: u32 = 40;

/// How many of a function's callers, and of its callees, which the push
/// doesn't change, its card is connected to at most.
pub const MAX_CONTEXT_PER_NODE: usize = 3;

/// A try push's canvas.
#[derive(Debug, Serialize)]
pub struct CanvasModel {
    pub version: u32,
    pub tree: String,
    /// The push (its name in the history's `try-pushes/`).
    pub push: String,
    /// The indexed revision its stack is rebased onto.
    pub base: String,
    /// The head of its rebased stack.
    pub head: String,
    /// The head's commit message's first line.
    pub summary: String,
    pub nodes: Vec<CanvasNode>,
    pub edges: Vec<CanvasEdge>,
    /// The files the push changes which have no functions it changes, ex:
    /// headers of declarations, or files its analysis doesn't cover.
    pub other_files: Vec<CanvasFile>,
    /// The SYM_INFO (see `format::format_code`) of the cards' symbols.
    pub sym_info: BTreeMap<Ustr, Option<JumprefData>>,
}

/// A function's card.
#[derive(Debug, Serialize)]
pub struct CanvasNode {
    /// Its symbol, which is its id.
    pub id: String,
    pub pretty: String,
    /// "added", "changed", or "context" (a caller or callee of the push's
    /// functions which the push doesn't change).
    pub status: &'static str,
    pub path: String,
    /// Its lines at the head (1-based, inclusive): from its name to the end
    /// of its body, or for the context's functions, its name's.
    pub lines: (u32, u32),
    /// The lines the push adds or changes in it.
    pub changed_lines: Vec<u32>,
    /// Its lines' rows (see `format::excerpt_row_html`).
    pub rows: String,
    /// Whether `rows` has all of its lines.
    pub complete: bool,
    /// Its file's listing at the head.
    pub link: String,
    /// The pretty names of the other functions whose definitions are its
    /// lines (see `build_canvas`), ex: another instantiation of a template.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<String>,
    /// Its symbols, its id's first, if it has others' (ex: for the slicing's
    /// clusters' functions; see rev-canvas.js).
    #[serde(skip_serializing_if = "has_one")]
    pub syms: Vec<String>,
}

fn has_one(syms: &[String]) -> bool {
    syms.len() <= 1
}

/// A call between two of the canvas's functions.
#[derive(Debug, Serialize)]
pub struct CanvasEdge {
    pub from: String,
    pub to: String,
    /// The lines of the caller's file making the calls.
    pub lines: Vec<u32>,
}

/// A file the push changes which has no functions it changes.
#[derive(Debug, Serialize)]
pub struct CanvasFile {
    pub path: String,
    pub change: FileChange,
    /// Why it has none, ex: "no definitions changed".
    pub note: &'static str,
    pub link: String,
}

/// A function definition in an analysis file.
struct Function {
    sym: String,
    pretty: String,
    lines: (u32, u32),
}

/// The function definitions of a file's analysis records (of any revision;
/// see `AbstractServer::fetch_raw_analysis`): the definitions whose syntax is
/// a function's or a method's, from their names to the ends of their bodies
/// (their nesting ranges), by their names' lines.
fn functions(records: Vec<Value>) -> Vec<Function> {
    let mut seen = BTreeSet::new();
    let mut functions = vec![];
    for record in analysis_from_values(records, &mut read_source) {
        for source in &record.data {
            let syntax: Vec<&str> = source.syntax.iter().map(|s| s.as_str()).collect();
            if !syntax.contains(&"def")
                || !(syntax.contains(&"function") || syntax.contains(&"method"))
            {
                continue;
            }
            let Some(sym) = source.sym.first() else {
                continue;
            };
            if !seen.insert(sym.to_string()) {
                continue;
            }
            // (The pretty identifier without its kind, ex: "function ".)
            let pretty = source
                .pretty
                .split_once(' ')
                .map_or(source.pretty.as_str(), |(_, pretty)| pretty);
            let first = record.loc.lineno;
            functions.push(Function {
                sym: sym.to_string(),
                pretty: pretty.to_string(),
                lines: (first, source.nesting_range.end_lineno.max(first)),
            });
        }
    }
    functions
}

/// The function definitions of the file at `path` in `server`'s analysis
/// (none if it has none).
async fn file_functions(server: &(dyn AbstractServer + Send + Sync), path: &str) -> Vec<Function> {
    match server.fetch_raw_analysis(path).await {
        Ok(records) => functions(records.collect().await),
        Err(_) => vec![],
    }
}

/// A function the push doesn't change, as a card of the context (only
/// functions: uses' contexts can be other definitions, ex: a variable's, for
/// its initializer).
async fn context_node(
    server: &(dyn AbstractServer + Send + Sync),
    sym: &str,
    link: &(dyn Fn(&str) -> String + Sync),
) -> Option<CanvasNode> {
    let data = server.crossref_lookup(sym).await.ok()??;
    if !data
        .meta
        .as_ref()
        .is_some_and(|meta| matches!(meta.kind.as_str(), "function" | "method"))
    {
        return None;
    }
    let definition = data.definitions.as_ref()?.first()?;
    let line = definition.lines.first()?.lineno;
    let pretty = match data.meta.as_ref() {
        Some(meta) => meta.pretty.to_string(),
        None => server.jumpref_lookup(sym).await.ok()??.pretty.to_string(),
    };
    Some(CanvasNode {
        id: sym.to_string(),
        pretty,
        status: "context",
        path: definition.path.to_string(),
        lines: (line, line),
        changed_lines: vec![],
        rows: String::new(),
        complete: true,
        link: link(&definition.path),
        others: vec![],
        syms: vec![sym.to_string()],
    })
}

/// The lines of the `(line, line, ...)` of a crossref callee's jump
/// ("path#line,line").
fn jump_lines(jump: &str) -> Vec<u32> {
    jump.rsplit_once('#')
        .map(|(_, lines)| lines.split(',').filter_map(|l| l.parse().ok()).collect())
        .unwrap_or_default()
}

/// The first of a card's symbols, its id.
fn syms_first(syms: &[String]) -> String {
    syms.first().cloned().unwrap_or_default()
}

/// The canvas of the try push of `server`'s tree whose rebased stack's head
/// is `head` (see `AbstractServer::try_overlay`).
pub async fn build_canvas(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    head: &str,
) -> Result<CanvasModel, &'static str> {
    const NO_ANALYSIS: &str = "No semantic analysis of the revision";
    // (The tree as the push changes it, and as indexed, which is the push's
    // base.)
    let overlay = server.try_overlay(head).ok_or(NO_ANALYSIS)?;
    let overlay = overlay.as_ref();
    let meta = overlay.try_delta_meta().ok_or(NO_ANALYSIS)?;
    let head_commit = overlay.commit_info().ok().flatten().ok_or("Bad revision")?;
    let base = overlay.old_side(&meta.base).ok_or("Bad revision")?;
    let base = base.as_ref();
    if base.commit_info().ok().flatten().map(|commit| commit.rev) != Some(meta.base.clone()) {
        return Err("The push's base isn't the indexed revision");
    }
    let link = |path: &str| format!("/{}/rev/{}/{}", tree_name, meta.head, url_encode_path(path));

    // ## The functions the push adds or changes.
    // (The analyzed files' sources and analysis, on both sides, at once; see
    // `AbstractServer::prefetch_files`.)
    let analyzed: Vec<String> = meta
        .changed
        .iter()
        .filter(|(path, change)| {
            **change != FileChange::Removed && meta.analyzed.contains_key(*path)
        })
        .map(|(path, _)| path.clone())
        .collect();
    let based: Vec<String> = analyzed
        .iter()
        .filter(|path| meta.changed.get(*path) != Some(&FileChange::Added))
        .cloned()
        .collect();
    futures_util::future::join(
        overlay.prefetch_files(&analyzed),
        base.prefetch_files(&based),
    )
    .await;
    let mut nodes: Vec<CanvasNode> = vec![];
    let mut other_files = vec![];
    for (path, change) in &meta.changed {
        let other = |note| CanvasFile {
            path: path.clone(),
            change: *change,
            note,
            link: link(path),
        };
        if *change == FileChange::Removed || !meta.analyzed.contains_key(path) {
            other_files.push(other(match change {
                FileChange::Removed => "removed",
                _ => "not analyzed",
            }));
            continue;
        }
        let Ok(head_source) = overlay.fetch_raw_source(path).await else {
            other_files.push(other("not found"));
            continue;
        };
        let base_source = match change {
            FileChange::Added => None,
            _ => base.fetch_raw_source(path).await.ok(),
        };
        let (added, touched) = changed_lines(
            base_source.as_deref().map(str::as_bytes),
            head_source.as_bytes(),
        );
        let base_functions: BTreeSet<String> = match change {
            FileChange::Added => BTreeSet::new(),
            _ => file_functions(base, path)
                .await
                .into_iter()
                .map(|f| f.pretty)
                .collect(),
        };
        let before = nodes.len();
        // (The definitions of other functions with the same lines, ex: a
        // template's instantiations, or the file's compilations for other
        // versions of a library (ex: FFmpeg's), are the same code: one card,
        // whose id is the first of their symbols.)
        let mut by_lines: BTreeMap<(u32, u32), Vec<Function>> = BTreeMap::new();
        for function in file_functions(overlay, path).await {
            let (first, last) = function.lines;
            if touched.range(first..=last).next().is_none() {
                continue;
            }
            by_lines.entry(function.lines).or_default().push(function);
        }
        for ((first, last), mut functions) in by_lines {
            functions.sort_by(|a, b| a.sym.cmp(&b.sym));
            let changed = functions.iter().any(|f| base_functions.contains(&f.pretty));
            let syms: Vec<String> = functions.iter().map(|f| f.sym.clone()).collect();
            let mut pretties = functions.into_iter().map(|f| f.pretty);
            let pretty = pretties.next().unwrap_or_default();
            let mut others: Vec<String> = pretties.filter(|other| *other != pretty).collect();
            others.sort();
            others.dedup();
            nodes.push(CanvasNode {
                status: if changed { "changed" } else { "added" },
                id: syms_first(&syms),
                pretty,
                path: path.clone(),
                lines: (first, last),
                changed_lines: added.range(first..=last).copied().collect(),
                rows: String::new(),
                complete: false,
                link: link(path),
                others,
                syms,
            });
        }
        if nodes.len() == before {
            other_files.push(other("no definitions changed"));
        }
    }

    // ## Their calls among them, and with a few of the functions the push
    // doesn't change.
    // (By any of their symbols, and by their definitions' places, so that a
    // function the push doesn't change is one card too, and another
    // instantiation of a function it changes is that function's card.)
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut places: HashMap<(String, u32), usize> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        for sym in &node.syms {
            ids.insert(sym.clone(), i);
        }
        places.insert((node.path.clone(), node.lines.0), i);
    }
    let mut calls: BTreeMap<(String, String), BTreeSet<u32>> = BTreeMap::new();
    // (Their entries, at once, and then those of the first of their callees
    // and callers that can be context cards; see `context_node`.)
    let syms: Vec<Ustr> = nodes
        .iter()
        .flat_map(|node| node.syms.iter().map(|sym| ustr(sym)))
        .collect();
    overlay.prefetch_crossref(&syms).await;
    let mut node_data = vec![];
    for node in &nodes {
        let mut data = vec![];
        for sym in &node.syms {
            data.extend(overlay.crossref_lookup(sym).await.ok().flatten());
        }
        node_data.push(data);
    }
    let mut candidates: Vec<Ustr> = vec![];
    for (i, datas) in node_data.iter().enumerate() {
        let mut callees: Vec<Ustr> = vec![];
        let mut callers: Vec<Ustr> = vec![];
        for data in datas {
            for callee in data.callees.iter().flatten() {
                if callees.len() < MAX_CONTEXT_PER_NODE
                    && !callee.implements
                    && matches!(callee.kind.as_str(), "function" | "method")
                    && !ids.contains_key(callee.sym.as_str())
                    && !callees.contains(&callee.sym)
                {
                    callees.push(callee.sym);
                }
            }
            for hit in data.uses.iter().flatten().flat_map(|hits| &hits.lines) {
                let from = hit.contextsym;
                if callers.len() < MAX_CONTEXT_PER_NODE
                    && !from.is_empty()
                    && ids.get(from.as_str()) != Some(&i)
                    && !ids.contains_key(from.as_str())
                    && !callers.contains(&from)
                {
                    callers.push(from);
                }
            }
        }
        candidates.extend(callees);
        candidates.extend(callers);
    }
    overlay.prefetch_crossref(&candidates).await;
    // The card of a function the push doesn't change, `sym`, if it can be one:
    // an existing card if its definition is one's place, else a new card (but
    // not more than `MAX_CONTEXT_PER_NODE` new cards per card, by `context`).
    async fn context_card(
        overlay: &(dyn AbstractServer + Send + Sync),
        sym: &str,
        link: &(dyn Fn(&str) -> String + Sync),
        nodes: &mut Vec<CanvasNode>,
        ids: &mut HashMap<String, usize>,
        places: &mut HashMap<(String, u32), usize>,
        context: &mut usize,
    ) -> Option<usize> {
        if let Some(&i) = ids.get(sym) {
            return Some(i);
        }
        if *context == MAX_CONTEXT_PER_NODE {
            return None;
        }
        let node = context_node(overlay, sym, link).await?;
        let place = (node.path.clone(), node.lines.0);
        let i = match places.get(&place) {
            Some(&i) => {
                let existing = &mut nodes[i];
                existing.syms.push(sym.to_string());
                if node.pretty != existing.pretty && !existing.others.contains(&node.pretty) {
                    existing.others.push(node.pretty);
                    existing.others.sort();
                }
                i
            }
            None => {
                *context += 1;
                places.insert(place, nodes.len());
                nodes.push(node);
                nodes.len() - 1
            }
        };
        ids.insert(sym.to_string(), i);
        Some(i)
    }
    for (i, datas) in node_data.into_iter().enumerate() {
        let mut context = 0;
        for data in &datas {
            for callee in data.callees.iter().flatten() {
                if callee.implements || !matches!(callee.kind.as_str(), "function" | "method") {
                    continue;
                }
                let Some(to) = context_card(
                    overlay,
                    &callee.sym,
                    &link,
                    &mut nodes,
                    &mut ids,
                    &mut places,
                    &mut context,
                )
                .await
                else {
                    continue;
                };
                calls
                    .entry((nodes[i].id.clone(), nodes[to].id.clone()))
                    .or_default()
                    .extend(jump_lines(&callee.jump));
            }
        }
        let mut context = 0;
        for data in &datas {
            for hits in data.uses.iter().flatten() {
                for hit in &hits.lines {
                    let from = hit.contextsym.to_string();
                    if from.is_empty() || ids.get(&from) == Some(&i) {
                        continue;
                    }
                    let Some(from) = context_card(
                        overlay,
                        &from,
                        &link,
                        &mut nodes,
                        &mut ids,
                        &mut places,
                        &mut context,
                    )
                    .await
                    else {
                        continue;
                    };
                    if from == i {
                        continue;
                    }
                    calls
                        .entry((nodes[from].id.clone(), nodes[i].id.clone()))
                        .or_default()
                        .insert(hit.lineno);
                }
            }
        }
    }
    let edges = calls
        .into_iter()
        .map(|((from, to), lines)| CanvasEdge {
            from,
            to,
            lines: lines.into_iter().collect(),
        })
        .collect();

    // ## Their cards' code, and its symbols' SYM_INFO.
    let mut file_lines: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for node in &nodes {
        let last = node.lines.1.min(node.lines.0 + MAX_NODE_LINES - 1);
        file_lines
            .entry(node.path.clone())
            .or_default()
            .extend(node.lines.0..=last);
    }
    let mut sym_info: BTreeMap<Ustr, Option<JumprefData>> = BTreeMap::new();
    let mut traversed: UstrMap<JumprefTraversals> = UstrMap::default();
    // (A file at a time, so that one without rows doesn't lose the others',
    // at once.)
    let mut file_rows: HashMap<Ustr, HashMap<u32, crate::file_format::chunked_gzip::Row>> = {
        use futures_util::stream::{self, StreamExt as FuturesStreamExt};
        let fetches = FuturesStreamExt::map(stream::iter(file_lines.clone()), |(path, lines)| {
            let key = ustr(&path);
            async move {
                let rows = overlay
                    .fetch_html_lines(vec![(key, lines)])
                    .await
                    .ok()
                    .and_then(|mut rows| rows.remove(&key));
                (key, rows)
            }
        });
        let fetched: Vec<_> = FuturesStreamExt::collect(fetches.buffer_unordered(16)).await;
        fetched
            .into_iter()
            .filter_map(|(key, rows)| Some((key, rows?)))
            .collect()
    };
    for (path, _) in file_lines {
        let key = ustr(&path);
        let Some(rows) = file_rows.remove(&key) else {
            continue;
        };
        let source = overlay.fetch_raw_source(&path).await.unwrap_or_default();
        let text: Vec<&str> = source.lines().collect();
        for node in nodes.iter_mut().filter(|node| node.path == path) {
            let last = node.lines.1.min(node.lines.0 + MAX_NODE_LINES - 1);
            // (Without their common indentation.)
            let indent = (node.lines.0..=last)
                .filter_map(|line| text.get(line as usize - 1))
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.len() - line.trim_start().len())
                .min()
                .unwrap_or(0);
            let changed: BTreeSet<u32> = node.changed_lines.iter().copied().collect();
            let mut html = String::new();
            for line in node.lines.0..=last {
                let Some(row) = rows.get(&line) else {
                    continue;
                };
                html.push_str(&excerpt_row_html(
                    &row.html,
                    line,
                    &node.link,
                    "rc-lineno",
                    indent,
                    changed.contains(&line).then_some("rc-changed-line"),
                ));
            }
            add_html_sym_info(overlay, &html, &mut sym_info, &mut traversed).await;
            node.complete = last == node.lines.1;
            node.rows = html;
        }
    }
    // (The history digests are for `/query/`'s results.)
    for jumpref in sym_info.values_mut().flatten() {
        jumpref.recency = None;
        jumpref.recency_from = None;
    }

    Ok(CanvasModel {
        version: CANVAS_VERSION,
        tree: tree_name.to_string(),
        push: meta.push.clone(),
        base: meta.base.clone(),
        head: meta.head.clone(),
        summary: head_commit.summary,
        nodes,
        edges,
        other_files,
        sym_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jump_lines() {
        assert_eq!(jump_lines("a/b.cpp#3,10"), vec![3, 10]);
        assert!(jump_lines("a/b.cpp").is_empty());
    }
}
