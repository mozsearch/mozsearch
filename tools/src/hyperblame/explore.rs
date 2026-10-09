//! The data of the `/explore/` pages, which show a group of commits (ex: the
//! commits of a bug) and the files, classes, and methods they touched as a
//! dense "horizontal Southern blot": a sparkline per file and symbol with a
//! narrow slot per commit, filled if the commit changed it.  Symbols nest (ex:
//! a class's methods under the class), and nested symbols are shown as thin
//! unlabeled rows under their parent's sparkline until expanded.  See "Patch
//! stack strip" and "History views" in the hyperblame notes.
//!
//! What each commit changed comes from its rev-summary (file deltas with
//! per-symbol token totals), or for commits the history doesn't have (ex: from
//! before a history window), just the files, from git.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::Path;

use git2::{Oid, Repository};
use serde::Serialize;

use super::token_blame::TreeHistory;
use crate::commit_index::CommitRef;
use crate::file_format::history::rev_summaries::{RevSummaryRecord, rev_summary_path};

/// The most commits a page shows.
pub const MAX_COMMITS: usize = 200;

#[derive(Debug, Serialize)]
pub struct ExploreCommit {
    /// The commit's (1-based) number on the page.
    pub number: usize,
    pub rev: String,
    #[serde(rename = "isoDate")]
    pub iso_date: String,
    pub backout: bool,
    /// For backouts, the interdiff of the commits they backed out and their
    /// reland, if any (see `relands`).
    pub interdiff: Option<String>,
    /// The commit's header (see `blame::commit_info_json`), as HTML.
    pub header: String,
    /// The first line of the header: the commit message's summary line.
    pub summary: String,
}

/// Order commits oldest first.  Commits landed together (ex: a stack of
/// patches) share their commit time, so those are ordered by ancestry.
pub fn order_commits(repo: &Repository, refs: &mut Vec<CommitRef>) {
    refs.sort_by(|a, b| a.iso_date.cmp(&b.iso_date).then(a.rev.cmp(&b.rev)));
    refs.dedup_by(|a, b| a.rev == b.rev);
    let is_descendant =
        |a: &CommitRef, b: &CommitRef| match (Oid::from_str(&a.rev), Oid::from_str(&b.rev)) {
            (Ok(a), Ok(b)) => repo.graph_descendant_of(a, b).unwrap_or(false),
            _ => false,
        };
    let mut start = 0;
    while start < refs.len() {
        let end = start
            + refs[start..]
                .iter()
                .take_while(|r| r.iso_date == refs[start].iso_date)
                .count();
        // Repeatedly take a commit which isn't a descendant of any of the
        // others left.
        let mut group: Vec<CommitRef> = refs[start..end].to_vec();
        let mut ordered = Vec::with_capacity(group.len());
        while !group.is_empty() {
            let next = (0..group.len())
                .find(|&i| {
                    !group
                        .iter()
                        .enumerate()
                        .any(|(j, other)| j != i && is_descendant(&group[i], other))
                })
                .unwrap_or(0);
            ordered.push(group.remove(next));
        }
        refs.splice(start..end, ordered);
        start = end;
    }
}

/// What a commit changed: the paths of its files, and for each, how many
/// tokens it changed in each symbol (if we know).
pub type CommitChanges = BTreeMap<String, BTreeMap<String, u32>>;

/// What the commit `rev` changed.
pub fn commit_changes(
    history: Option<&TreeHistory>,
    repo: &Repository,
    rev: &str,
) -> CommitChanges {
    if let Some(history) = history {
        let path = Path::new(&history.path)
            .join("rev-summaries")
            .join(rev_summary_path(rev));
        let summary = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<RevSummaryRecord>(&text).ok());
        if let Some(summary) = summary
            && !summary.file_deltas.is_empty()
        {
            return summary
                .file_deltas
                .into_iter()
                .map(|(path, file)| {
                    let symbols = file
                        .delta
                        .symbol_group
                        .symbol_deltas
                        .into_iter()
                        .map(|(pretty, delta)| {
                            let totals = &delta.token_totals;
                            (pretty, totals.added + totals.removed)
                        })
                        .collect();
                    (path, symbols)
                })
                .collect();
        }
    }

    // Just the files, from git.
    let mut changes = CommitChanges::new();
    let Some(commit) = Oid::from_str(rev)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
    else {
        return changes;
    };
    let tree = commit.tree().ok();
    let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());
    if let Ok(diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), tree.as_ref(), None) {
        for delta in diff.deltas() {
            let file = delta.new_file().path().or_else(|| delta.old_file().path());
            if let Some(path) = file.and_then(|path| path.to_str()) {
                changes.insert(path.to_string(), BTreeMap::new());
            }
        }
    }
    changes
}

/// Also mark the commits as backouts which the history recognized as backouts
/// (their rev-summaries say what they backed out), which the commit index's
/// summary-based flags can miss (ex: "Bug 123 - Backed out changeset X", or
/// backouts whose targets came from their bugs; see
/// `hyperblame::backouts`).  `history_path` is the history's directory.
pub fn mark_history_backouts(history_path: Option<&Path>, refs: &mut [CommitRef]) {
    #[derive(serde::Deserialize)]
    struct Backouts {
        #[serde(default)]
        backs_out: Vec<String>,
    }
    let Some(history_path) = history_path else {
        return;
    };
    for commit_ref in refs.iter_mut().filter(|r| !r.backout) {
        let path = history_path
            .join("rev-summaries")
            .join(rev_summary_path(&commit_ref.rev));
        let backs_out = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Backouts>(&text).ok())
            .is_some_and(|summary| !summary.backs_out.is_empty());
        commit_ref.backout = backs_out;
    }
}

/// The relands on a page (see `format::format_interdiff`): for each backout
/// on it (by index in `refs`), the commits on the page which it backed out,
/// and the next commit after it which isn't a backout and changed some of the
/// same files, with the commits after that which landed with it (within a
/// minute of the commit before, since stacks land a second or so apart), if
/// there is one.  The backed out commits come from the backout's rev-summary.
pub fn relands(
    history: Option<&TreeHistory>,
    refs: &[CommitRef],
    changes: &[CommitChanges],
) -> Vec<(usize, Vec<usize>, Vec<usize>)> {
    let Some(history) = history else {
        return vec![];
    };
    let time = |r: &CommitRef| {
        chrono::DateTime::parse_from_rfc3339(&r.iso_date).map_or(0, |date| date.timestamp())
    };
    let mut relands = vec![];
    for (k, backout) in refs.iter().enumerate().filter(|(_, r)| r.backout) {
        let path = Path::new(&history.path)
            .join("rev-summaries")
            .join(rev_summary_path(&backout.rev));
        let Some(summary) = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<RevSummaryRecord>(&text).ok())
        else {
            continue;
        };
        let backed_out: Vec<usize> = (0..k)
            .filter(|&i| summary.backs_out.contains(&refs[i].rev))
            .collect();
        let files: BTreeSet<&String> = backed_out.iter().flat_map(|&i| changes[i].keys()).collect();
        let Some(first) = (k + 1..refs.len())
            .find(|&j| !refs[j].backout && changes[j].keys().any(|file| files.contains(file)))
        else {
            continue;
        };
        let mut reland = vec![first];
        for j in first + 1..refs.len() {
            if refs[j].backout || time(&refs[j]) - time(&refs[j - 1]) > 60 {
                break;
            }
            reland.push(j);
        }
        if !backed_out.is_empty() {
            relands.push((k, backed_out, reland));
        }
    }
    relands
}

/// The blot's rows for commits (in page order) which changed `changes`: a row
/// per file (sorted by path) followed by a row per symbol in it (sorted by
/// pretty).  `link` makes a file's link from its path and the last commit
/// which changed it.
/// How much a commit changed a file or symbol: 0 for not at all, and 1-3 for
/// up to 5 tokens (or an unknown amount), up to 50, and more.
fn level(cell: Option<u32>) -> u8 {
    match cell {
        None => 0,
        Some(0..=5) => 1,
        Some(6..=50) => 2,
        Some(_) => 3,
    }
}

/// The width of each commit's slot in sparklines, in pixels: 1 for many
/// commits, and wider for a few so they're still visible.
pub fn slot_width(commits: usize) -> usize {
    (96 / commits.max(1)).clamp(1, 6)
}

/// An inline SVG of rows of slots, one per commit, where each row is given as
/// its levels, height (in pixels), and whether to fade it (see `blot_files`),
/// with a pixel between rows.  Runs of slots with the same level are single
/// rects.  The SVG also has the cursor, which explore.js moves to the commit
/// under the mouse.
fn blot_svg(rows: &[(&[u8], usize, bool)], slot: usize, class: &str) -> String {
    let width = rows.first().map_or(0, |(levels, _, _)| levels.len()) * slot;
    let height = rows.iter().map(|(_, h, _)| h).sum::<usize>() + rows.len().saturating_sub(1);
    let mut svg = format!(
        r#"<svg class="explore-sparkline {}" width="{}" height="{}"><rect class="explore-track" width="{}" height="{}"/>"#,
        class, width, height, width, height
    );
    let mut y = 0;
    for (levels, row_height, same) in rows {
        let mut i = 0;
        while i < levels.len() {
            let run = levels[i..].iter().take_while(|&&l| l == levels[i]).count();
            if levels[i] > 0 {
                write!(
                    svg,
                    r#"<rect class="explore-l{}{}" x="{}" y="{}" width="{}" height="{}"/>"#,
                    levels[i],
                    if *same { " explore-same-row" } else { "" },
                    i * slot,
                    y,
                    run * slot,
                    row_height
                )
                .unwrap();
            }
            i += run;
        }
        y += row_height + 1;
    }
    write!(
        svg,
        r#"<rect class="explore-cursor" width="{}" height="{}"/></svg>"#,
        slot, height
    )
    .unwrap();
    svg
}

#[derive(Debug, Serialize)]
pub struct ExploreSymbol {
    /// The name, without its parent's prefix.
    pub name: String,
    /// The full pretty identifier.
    pub pretty: String,
    pub sparkline: String,
    /// For symbols with nested symbols, the collapsed view: the symbol's
    /// sparkline (including the nested symbols' changes) with a thin row for
    /// each nested symbol under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blot: Option<String>,
    /// The nested symbols (all of them, including more deeply nested ones,
    /// named relative to this symbol).
    pub children: Vec<ExploreSymbol>,
    /// For interdiffs (see `format::format_interdiff_files`), whether the
    /// patches' changes to the symbol are the same, which fades it.
    pub same: bool,
}

#[derive(Debug, Serialize)]
pub struct ExploreFile {
    pub path: String,
    /// The file as of the last commit which changed it.
    pub link: String,
    pub sparkline: String,
    pub symbols: Vec<ExploreSymbol>,
    /// For interdiffs (see `format::format_interdiff_files`), whether the
    /// patches' changes to the file are the same, and HTML notes about them
    /// and excerpts of where they differ.
    pub same: bool,
    pub note: String,
    pub excerpts: String,
    /// The file's facets (see `format::explore_facets`), as JSON for
    /// facets.js: the values of each facet it's in, and its group for each way
    /// of grouping files.  And the title of its link (ex: its subsystem).
    pub facets: String,
    pub groups: String,
    pub title: String,
}

/// The symbols' parents: the longest proper prefix (by "::" segments) which is
/// itself a symbol or is shared by several symbols, which groups a class's
/// methods even if the class itself didn't change.
fn symbol_parents(symbols: &BTreeSet<&str>) -> BTreeMap<String, String> {
    let mut prefix_counts: BTreeMap<String, usize> = BTreeMap::new();
    for symbol in symbols {
        let segments: Vec<&str> = symbol.split("::").collect();
        for n in 1..segments.len() {
            *prefix_counts.entry(segments[..n].join("::")).or_default() += 1;
        }
    }
    let is_parent = |prefix: &str| {
        symbols.contains(prefix) || prefix_counts.get(prefix).is_some_and(|&c| c > 1)
    };
    let mut parents = BTreeMap::new();
    for symbol in symbols {
        let segments: Vec<&str> = symbol.split("::").collect();
        if let Some(n) = (1..segments.len())
            .rev()
            .find(|&n| is_parent(&segments[..n].join("::")))
        {
            parents.insert(symbol.to_string(), segments[..n].join("::"));
        }
    }
    parents
}

/// The name of the changes outside of any symbol.
pub const TOP_LEVEL: &str = "(top level)";

/// The files which the commits (in page order) changed, sorted by path, with
/// their symbols.  `link` makes a file's link from its path and the last commit
/// which changed it.  For interdiffs, `same` says whether the patches' changes
/// to a file's symbol (by its pretty identifier, or `TOP_LEVEL`) are the same,
/// which fades the symbol, or its row of its parent's collapsed view.
pub fn blot_files(
    commits: &[CommitRef],
    changes: &[CommitChanges],
    link: impl Fn(&str, &str) -> String,
    same: impl Fn(&str, &str) -> bool,
) -> Vec<ExploreFile> {
    let slot = slot_width(commits.len());
    let paths: BTreeSet<&String> = changes.iter().flat_map(|c| c.keys()).collect();
    let mut files = vec![];
    for path in paths {
        let file_levels: Vec<u8> = changes
            .iter()
            .map(|c| level(c.get(path).map(|symbols| symbols.values().sum())))
            .collect();
        let last = file_levels.iter().rposition(|&l| l > 0).unwrap();

        // Each symbol's cells, with "%" (outside of any symbol) renamed.
        let mut cells: BTreeMap<String, Vec<Option<u32>>> = BTreeMap::new();
        for (i, commit_changes) in changes.iter().enumerate() {
            for (symbol, &tokens) in commit_changes.get(path).into_iter().flatten() {
                let name = if symbol == "%" { TOP_LEVEL } else { symbol };
                cells
                    .entry(name.to_string())
                    .or_insert_with(|| vec![None; changes.len()])[i] = Some(tokens);
            }
        }
        let names: BTreeSet<&str> = cells.keys().map(String::as_str).collect();
        let parents = symbol_parents(&names);
        let top_of = |name: &str| {
            let mut top = name.to_string();
            while let Some(parent) = parents.get(&top) {
                top = parent.clone();
            }
            top
        };
        // Top-level symbols (including synthesized parents) and their
        // descendants.
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for name in &names {
            let top = top_of(name);
            let descendants = groups.entry(top.clone()).or_default();
            if top != *name {
                descendants.push(name.to_string());
            }
        }

        let mut symbols: Vec<ExploreSymbol> = groups
            .into_iter()
            .map(|(top, descendants)| {
                let own = cells.get(&top);
                let top_same = same(path, &top);
                // The symbol's sparkline includes its descendants' changes.
                let top_levels: Vec<u8> = (0..changes.len())
                    .map(|i| {
                        let mut tokens = own.and_then(|c| c[i]);
                        for descendant in &descendants {
                            if let Some(t) = cells[descendant][i] {
                                tokens = Some(tokens.unwrap_or(0) + t);
                            }
                        }
                        level(tokens)
                    })
                    .collect();
                let child_levels: Vec<Vec<u8>> = descendants
                    .iter()
                    .map(|d| cells[d].iter().map(|&c| level(c)).collect())
                    .collect();
                let children: Vec<ExploreSymbol> = descendants
                    .iter()
                    .zip(&child_levels)
                    .map(|(descendant, levels)| ExploreSymbol {
                        name: descendant
                            .strip_prefix(&format!("{}::", top))
                            .unwrap_or(descendant)
                            .to_string(),
                        pretty: descendant.clone(),
                        sparkline: blot_svg(&[(levels, 4, false)], slot, "explore-child-sparkline"),
                        blot: None,
                        children: vec![],
                        same: same(path, descendant),
                    })
                    .collect();
                // (A faded symbol's view is faded as a whole, and otherwise the
                // collapsed view fades the rows of its faded nested symbols.)
                let blot =
                    (!children.is_empty()).then(|| {
                        let mut rows: Vec<(&[u8], usize, bool)> = vec![(&top_levels, 6, false)];
                        rows.extend(child_levels.iter().zip(&children).map(|(levels, child)| {
                            (levels.as_slice(), 2, child.same && !top_same)
                        }));
                        blot_svg(&rows, slot, "explore-blot-sparkline")
                    });
                ExploreSymbol {
                    name: top.clone(),
                    pretty: top,
                    sparkline: blot_svg(&[(&top_levels, 6, false)], slot, ""),
                    blot,
                    children,
                    same: top_same,
                }
            })
            .collect();
        // Changes only outside of any symbol are the file's sparkline again.
        if symbols.len() == 1 && symbols[0].pretty == TOP_LEVEL {
            symbols.clear();
        }

        files.push(ExploreFile {
            path: path.clone(),
            link: link(path, &commits[last].rev),
            sparkline: blot_svg(&[(&file_levels, 8, false)], slot, "explore-file-sparkline"),
            symbols,
            same: false,
            note: String::new(),
            excerpts: String::new(),
            facets: String::new(),
            groups: String::new(),
            title: String::new(),
        });
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_parents() {
        let symbols: BTreeSet<&str> = [
            "Foo",
            "Foo::bar",
            "Foo::Inner::baz",
            "Popup::render",
            "Popup::position",
            "lonely::fn",
        ]
        .into_iter()
        .collect();
        let parents = symbol_parents(&symbols);
        assert_eq!(parents.get("Foo::bar").map(String::as_str), Some("Foo"));
        // "Foo::Inner" isn't a symbol and isn't shared, so "Foo" is the parent.
        assert_eq!(
            parents.get("Foo::Inner::baz").map(String::as_str),
            Some("Foo")
        );
        // "Popup" isn't a symbol, but it's shared.
        assert_eq!(
            parents.get("Popup::render").map(String::as_str),
            Some("Popup")
        );
        assert_eq!(parents.get("lonely::fn"), None);
        assert_eq!(parents.get("Foo"), None);
    }

    #[test]
    fn test_mark_history_backouts() {
        let dir = std::env::temp_dir().join(format!("hb-explore-backouts-{}", std::process::id()));
        let commit = |rev: &str, backout: bool| CommitRef {
            rev: rev.to_string(),
            iso_date: String::new(),
            backout,
        };
        let (landing, backout, flagged) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
        let write = |rev: &str, backs_out: &[&str]| {
            let path = dir.join("rev-summaries").join(rev_summary_path(rev));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let summary = serde_json::json!({ "source_rev": rev, "backs_out": backs_out });
            fs::write(path, summary.to_string()).unwrap();
        };
        write(&landing, &[]);
        write(&backout, &[&landing]);
        let mut refs = vec![
            commit(&landing, false),
            commit(&backout, false),
            commit(&flagged, true),
        ];
        mark_history_backouts(Some(&dir), &mut refs);
        let flags: Vec<bool> = refs.iter().map(|r| r.backout).collect();
        // (The index's flag stands without a rev-summary.)
        assert_eq!(flags, vec![false, true, true]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_blot_files() {
        let commit = |rev: &str| CommitRef {
            rev: rev.to_string(),
            iso_date: String::new(),
            backout: false,
        };
        let commits = vec![commit("a"), commit("b")];
        let changes: Vec<CommitChanges> = vec![
            BTreeMap::from([(
                "f.rs".to_string(),
                BTreeMap::from([("Foo::bar".to_string(), 3), ("%".to_string(), 1)]),
            )]),
            BTreeMap::from([
                (
                    "f.rs".to_string(),
                    BTreeMap::from([("Foo::baz".to_string(), 60)]),
                ),
                ("g.rs".to_string(), BTreeMap::new()),
                ("h.rs".to_string(), BTreeMap::from([("%".to_string(), 2)])),
            ]),
        ];
        let link = |path: &str, rev: &str| format!("{}@{}", path, rev);
        let files = blot_files(&commits, &changes, link, |_, _| false);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "f.rs");
        assert_eq!(files[0].link, "f.rs@b");
        let symbols: Vec<(&str, Vec<&str>)> = files[0]
            .symbols
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.children.iter().map(|c| c.name.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            symbols,
            vec![("(top level)", vec![]), ("Foo", vec!["bar", "baz"])]
        );
        let foo = &files[0].symbols[1];
        assert!(foo.blot.is_some());
        // 2 commits get 6px slots.  Foo changed a little in the first commit
        // and a lot in the second.
        assert!(
            foo.sparkline
                .contains(r#"<rect class="explore-l1" x="0" y="0" width="6" height="6"/>"#)
        );
        assert!(
            foo.sparkline
                .contains(r#"<rect class="explore-l3" x="6" y="0" width="6" height="6"/>"#)
        );
        assert!(!foo.blot.as_ref().unwrap().contains("explore-same-row"));
        assert_eq!(files[1].path, "g.rs");
        assert!(files[1].symbols.is_empty());
        // Changes only outside of any symbol don't get their own sparkline.
        assert_eq!(files[2].path, "h.rs");
        assert!(files[2].symbols.is_empty());

        // In an interdiff where only `Foo::baz` differs, `Foo::bar` is faded,
        // and so is its row of `Foo`'s collapsed view, which is under `Foo`'s
        // (6px and a pixel) and is the first of `Foo`'s nested symbols' rows.
        let files = blot_files(&commits, &changes, link, |_, pretty| {
            pretty != "Foo" && pretty != "Foo::baz"
        });
        let foo = &files[0].symbols[1];
        assert!(!foo.same);
        let same: Vec<bool> = foo.children.iter().map(|c| c.same).collect();
        assert_eq!(same, vec![true, false]);
        let blot = foo.blot.as_ref().unwrap();
        assert!(blot.contains(
            r#"<rect class="explore-l1 explore-same-row" x="0" y="7" width="6" height="2"/>"#
        ));
        assert!(blot.contains(r#"<rect class="explore-l3" x="6" y="10" width="6" height="2"/>"#));
        assert!(files[0].symbols[0].same);
    }

    #[test]
    fn test_blot_svg_runs() {
        let svg = blot_svg(&[(&[0, 2, 2, 0, 1], 4, false)], 1, "x");
        assert!(svg.contains(r#"<rect class="explore-l2" x="1" y="0" width="2" height="4"/>"#));
        assert!(svg.contains(r#"<rect class="explore-l1" x="4" y="0" width="1" height="4"/>"#));
    }
}
