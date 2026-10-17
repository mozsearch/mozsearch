//! The canvas of a try push (`/{tree}/rev-canvas/{head}`; see
//! `format::format_try_canvas`): the functions the push adds or changes, as
//! cards of their code, and the calls among them, and between them and a few of
//! the functions calling them or called by them which it doesn't change, for
//! laying out by their control flow on an infinite canvas (see
//! static/js/rev-canvas.js and graph.js).  It's from the push's semantic delta
//! (see `try_delta`), whose analysis is the compiler's, so that the calls are
//! the ones the code makes, not a parse's guesses.
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
use std::path::Path;

use git2::Repository;
use serde::Serialize;
use ustr::{Ustr, UstrMap};

use crate::file_format::analysis::{read_analysis, read_source};
use crate::file_format::bisectable_mmap::BisectableMmap;
use crate::file_format::config::Config;
use crate::file_format::crossref::CrossrefData;
use crate::file_format::jumpref::{JumprefData, JumprefTraversals, TreeJumprefs};
use crate::format::{add_rows_sym_info, excerpt_row_html, file_rows};
use crate::rev_lines::changed_lines;
use crate::try_delta::{FileChange, TryDelta};
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

/// The function definitions of an analysis file (of any revision): the
/// definitions whose syntax is a function's or a method's, from their names to
/// the ends of their bodies (their nesting ranges), by their names' lines.
fn functions(file: &Path) -> Vec<Function> {
    if !file.exists() {
        return vec![];
    }
    let mut seen = BTreeSet::new();
    let mut functions = vec![];
    for record in read_analysis(&file.to_string_lossy(), &mut read_source) {
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

/// A file's contents at a commit.
fn blob_at(repo: &Repository, commit: &git2::Commit, path: &str) -> Option<Vec<u8>> {
    let entry = commit.tree().ok()?.get_path(Path::new(path)).ok()?;
    Some(repo.find_blob(entry.id()).ok()?.content().to_vec())
}

/// The lines of the `(line, line, ...)` of a crossref callee's jump
/// ("path#line,line").
fn jump_lines(jump: &str) -> Vec<u32> {
    jump.rsplit_once('#')
        .map(|(_, lines)| lines.split(',').filter_map(|l| l.parse().ok()).collect())
        .unwrap_or_default()
}

/// The canvas of the try push of the tree whose rebased stack's head is
/// `head`.
pub fn build_canvas(
    cfg: &Config,
    tree_name: &str,
    head: &str,
) -> Result<CanvasModel, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let repo = &git.repo;
    let paths = &tree_config.paths;
    let delta =
        TryDelta::open(&paths.index_path, head).ok_or("No semantic analysis of the revision")?;
    let commit = |rev: &str| {
        repo.revparse_single(rev)
            .and_then(|object| object.peel_to_commit())
            .map_err(|_| "Bad revision")
    };
    let head_commit = commit(&delta.meta.head)?;
    let base_commit = commit(&delta.meta.base)?;
    let mut jumprefs = TreeJumprefs::open(paths, &cfg.tree_index_paths);
    jumprefs.delta = delta.jumprefs();
    let crossref = BisectableMmap::<CrossrefData>::new(
        &format!("{}/crossref", paths.index_path),
        &format!("{}/crossref-extra", paths.index_path),
    );
    // (The push's, with its patches.)
    let lookup = |sym: &str| -> Option<CrossrefData> {
        let data = crossref.as_ref().and_then(|c| c.lookup(sym).ok().flatten());
        match delta.crossref_patch(sym).ok().flatten() {
            Some(patch) => patch.apply(data),
            None => data,
        }
    };
    let link = |path: &str| {
        format!(
            "/{}/rev/{}/{}",
            tree_name,
            delta.meta.head,
            url_encode_path(path)
        )
    };

    // ## The functions the push adds or changes.
    let mut nodes: Vec<CanvasNode> = vec![];
    let mut other_files = vec![];
    for (path, change) in &delta.meta.changed {
        let other = |note| CanvasFile {
            path: path.clone(),
            change: *change,
            note,
            link: link(path),
        };
        let Some(analysis) = delta.analysis_file(path).filter(|file| file.exists()) else {
            other_files.push(other(match change {
                FileChange::Removed => "removed",
                _ => "not analyzed",
            }));
            continue;
        };
        let (head_source, base_source) = match delta.generated_source(path) {
            Some(file) => (
                std::fs::read(file).ok(),
                path.strip_prefix("__GENERATED__/")
                    .and_then(|rel| std::fs::read(Path::new(&paths.objdir_path).join(rel)).ok()),
            ),
            None => (
                blob_at(repo, &head_commit, path),
                blob_at(repo, &base_commit, path),
            ),
        };
        let Some(head_source) = head_source else {
            other_files.push(other("not found"));
            continue;
        };
        let (added, touched) = changed_lines(
            match change {
                FileChange::Added => None,
                _ => base_source.as_deref(),
            },
            &head_source,
        );
        let base_functions: BTreeSet<String> = match change {
            FileChange::Added => BTreeSet::new(),
            _ => functions(
                &Path::new(&paths.index_path)
                    .join("analysis")
                    .join(format!("{}.gz", path)),
            )
            .into_iter()
            .map(|f| f.pretty)
            .collect(),
        };
        let before = nodes.len();
        for function in functions(&analysis) {
            let (first, last) = function.lines;
            if touched.range(first..=last).next().is_none() {
                continue;
            }
            nodes.push(CanvasNode {
                status: if base_functions.contains(&function.pretty) {
                    "changed"
                } else {
                    "added"
                },
                id: function.sym,
                pretty: function.pretty,
                path: path.clone(),
                lines: function.lines,
                changed_lines: added.range(first..=last).copied().collect(),
                rows: String::new(),
                complete: false,
                link: link(path),
            });
        }
        if nodes.len() == before {
            other_files.push(other("no definitions changed"));
        }
    }

    // ## Their calls among them, and with a few of the functions the push
    // doesn't change.
    let mut ids: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.clone(), i))
        .collect();
    let mut calls: BTreeMap<(String, String), BTreeSet<u32>> = BTreeMap::new();
    let context_node = |sym: &str| -> Option<CanvasNode> {
        let data = lookup(sym)?;
        // (Only functions; uses' contexts can be other definitions, ex: a
        // variable's, for its initializer.)
        if !data
            .meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.kind.as_str(), "function" | "method"))
        {
            return None;
        }
        let definition = data.definitions.as_ref()?.first()?;
        let line = definition.lines.first()?.lineno;
        let pretty = data
            .meta
            .as_ref()
            .map(|meta| meta.pretty.to_string())
            .or_else(|| Some(jumprefs.lookup(sym).ok()??.pretty.to_string()))?;
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
        })
    };
    for i in 0..nodes.len() {
        let sym = nodes[i].id.clone();
        let Some(data) = lookup(&sym) else {
            continue;
        };
        let mut context = 0;
        for callee in data.callees.iter().flatten() {
            if callee.implements || !matches!(callee.kind.as_str(), "function" | "method") {
                continue;
            }
            let to = callee.sym.to_string();
            if !ids.contains_key(&to) {
                if context == MAX_CONTEXT_PER_NODE {
                    continue;
                }
                let Some(node) = context_node(&to) else {
                    continue;
                };
                context += 1;
                ids.insert(to.clone(), nodes.len());
                nodes.push(node);
            }
            calls
                .entry((sym.clone(), to))
                .or_default()
                .extend(jump_lines(&callee.jump));
        }
        let mut context = 0;
        for hits in data.uses.iter().flatten() {
            for hit in &hits.lines {
                let from = hit.contextsym.to_string();
                if from.is_empty() || from == sym {
                    continue;
                }
                if !ids.contains_key(&from) {
                    if context == MAX_CONTEXT_PER_NODE {
                        continue;
                    }
                    let Some(node) = context_node(&from) else {
                        continue;
                    };
                    context += 1;
                    ids.insert(from.clone(), nodes.len());
                    nodes.push(node);
                }
                calls
                    .entry((from, sym.clone()))
                    .or_default()
                    .insert(hit.lineno);
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
    for (path, lines) in file_lines {
        let Some(rows) = file_rows(cfg, tree_name, Some(&delta), &jumprefs, &path, &lines) else {
            continue;
        };
        let source = match delta.generated_source(&path) {
            Some(file) => std::fs::read(file).ok(),
            None => blob_at(repo, &head_commit, &path),
        }
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
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
            add_rows_sym_info(&jumprefs, &html, &mut sym_info, &mut traversed);
            node.complete = last == node.lines.1;
            node.rows = html;
        }
    }
    // (The history digests are for `/query/`'s results.)
    for jumpref in sym_info.values_mut().flatten() {
        jumpref.recency = None;
        jumpref.recency_from = None;
    }

    let summary = head_commit
        .summary()
        .ok()
        .flatten()
        .unwrap_or_default()
        .to_string();
    Ok(CanvasModel {
        version: CANVAS_VERSION,
        tree: tree_name.to_string(),
        push: delta.meta.push.clone(),
        base: delta.meta.base.clone(),
        head: delta.meta.head.clone(),
        summary,
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
