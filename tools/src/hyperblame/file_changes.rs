//! What each of a page's commits (its token-centric blame's; see
//! `page_blame::BlameInfo::commits`) changed in its file, for the file
//! timeline (see static/js/file-timeline.js): how many tokens it added and
//! removed in the file, for the height of its tick, and its most changed
//! definitions, for the timeline's list of commits.
//!
//! They're the file's files-delta journal's detail records, read once (as of
//! the page's timeline commit), and for the commits which the journal's
//! weekly summaries consolidated (see `consolidation`), whose own changes the
//! summaries don't keep, their rev-summaries (which have the same deltas;
//! cached, since a big commit's is read for each of its files).  Commits not
//! in the journal (ex: those of tokens moved from other files) didn't change
//! the file, and summarized commits without the file in their rev-summaries
//! (merges, and commits of over `MAX_REV_SUMMARY_FILES` files) are unknown.
//! (Both are None.)

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use git2::Commit;
use serde::{Deserialize, Serialize};

use super::journals::{JournalKind, JournalReader};
use super::token_blame::TreeHistory;
use crate::file_format::history::rev_summaries::rev_summary_path;
use crate::file_format::history::timeline_common::{
    ChangeKind, JournalVersionRef, TokenDeltaDetails,
};

/// How many of a commit's changed definitions we keep (see `ScopeChange`).
pub const TOP_SCOPE_CHANGES: usize = 2;

/// How many commits' rev-summaries we keep.
const REV_SUMMARY_CACHE_ENTRIES: usize = 128;

/// A definition's change in a commit (or the top level's, outside any
/// definition, whose `pretty` is "%"): `kind` is "added", "removed",
/// "changed", "renamed" (from `other`), "moved-in" (from the file `other`)
/// or "moved-out" (to the file `other`), with the tokens it added and
/// removed (and renamed or moved, as added or removed).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScopeChange {
    pub kind: &'static str,
    pub pretty: String,
    pub added: u32,
    pub removed: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
}

/// A commit's change to the file.
#[derive(Clone, Debug, PartialEq)]
pub struct FileChange {
    /// The tokens it added and removed in the file.
    pub size: u32,
    /// Its most changed definitions (see `top_scope_changes`).
    pub scopes: Vec<ScopeChange>,
}

/// A journal record's or rev-summary's delta of a definition, without its
/// tokens' changes (`token_changes`, which can be most of a journal: ex: a
/// history's first commit's).
#[derive(Clone, Debug, Deserialize)]
struct SymbolDelta {
    change: ChangeKind,
    #[serde(default)]
    evolved_from: Option<String>,
    #[serde(default)]
    evolved_into: Option<String>,
    #[serde(default)]
    evolved_from_path: Option<String>,
    #[serde(default)]
    evolved_into_path: Option<String>,
    #[serde(default)]
    token_totals: TokenDeltaDetails,
}

type SymbolDeltas = BTreeMap<String, SymbolDelta>;

/// A files-delta journal's record: a detail record (`source_rev`, with the
/// file's `moved_from`) or a summary (`source_revs`).
#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    source_rev: Option<String>,
    #[serde(default)]
    source_revs: Vec<String>,
    #[serde(default)]
    moved_from: Option<String>,
    #[serde(default)]
    symbol_deltas: SymbolDeltas,
}

#[derive(Deserialize)]
struct RevFile {
    #[serde(default)]
    symbol_deltas: SymbolDeltas,
}

#[derive(Deserialize)]
struct RevSummary {
    #[serde(default)]
    file_deltas: HashMap<String, RevFile>,
}

static REV_SUMMARIES: LazyLock<Mutex<(HashMap<String, Arc<RevSummary>>, VecDeque<String>)>> =
    LazyLock::new(Default::default);

/// The rev-summary of the commit `rev`, if the history has it.
fn rev_summary(history: &TreeHistory, rev: &str) -> Option<Arc<RevSummary>> {
    if let Some(summary) = REV_SUMMARIES.lock().unwrap().0.get(rev) {
        return Some(summary.clone());
    }
    let path = Path::new(&history.path)
        .join("rev-summaries")
        .join(rev_summary_path(rev));
    let summary: Arc<RevSummary> =
        Arc::new(serde_json::from_slice(&std::fs::read(path).ok()?).ok()?);
    let (summaries, order) = &mut *REV_SUMMARIES.lock().unwrap();
    if summaries.insert(rev.to_string(), summary.clone()).is_none() {
        order.push_back(rev.to_string());
        while order.len() > REV_SUMMARY_CACHE_ENTRIES {
            if let Some(oldest) = order.pop_front() {
                summaries.remove(&oldest);
            }
        }
    }
    Some(summary)
}

/// The changes of the commits `revs` to the file at `path` (see the module
/// docs), as of the timeline commit `timeline_commit`, in `revs`' order.
pub fn file_changes(
    history: &TreeHistory,
    timeline_commit: &Commit,
    path: &str,
    revs: &[&str],
) -> Vec<Option<FileChange>> {
    let records: Vec<Record> = JournalReader::new(&history.timeline)
        .records(&JournalVersionRef {
            timeline_rev: timeline_commit.id().to_string(),
            path: JournalKind::FilesDelta.journal_path(path),
        })
        .unwrap_or_default();
    // (The file's paths, which its rev-summaries name it by.)
    let mut paths: HashSet<&str> = HashSet::from([path]);
    let mut details: HashMap<&str, &SymbolDeltas> = HashMap::new();
    let mut summarized: HashSet<&str> = HashSet::new();
    for record in &records {
        if let Some(rev) = &record.source_rev {
            details.insert(rev, &record.symbol_deltas);
        }
        summarized.extend(record.source_revs.iter().map(String::as_str));
        paths.extend(record.moved_from.as_deref());
    }
    revs.iter()
        .map(|rev| {
            if let Some(deltas) = details.get(rev) {
                return Some(file_change(deltas));
            }
            if !summarized.contains(rev) {
                return None;
            }
            let summary = rev_summary(history, rev)?;
            let file = paths
                .iter()
                .find_map(|path| summary.file_deltas.get(*path))?;
            Some(file_change(&file.symbol_deltas))
        })
        .collect()
}

/// The tokens a commit added, removed or edited (ex: renamed, or `>` that
/// became `>=`; once, by their new side), not those it moved.
fn changed_tokens(totals: &TokenDeltaDetails) -> u32 {
    totals.added + totals.removed + totals.evolved_from
}

fn file_change(deltas: &SymbolDeltas) -> FileChange {
    FileChange {
        size: deltas
            .values()
            .map(|delta| changed_tokens(&delta.token_totals))
            .sum(),
        scopes: top_scope_changes(deltas),
    }
}

/// A commit's most changed definitions (by the tokens it changed in them, or
/// for those it renamed or moved, those it moved too), without the old names
/// of those it renamed (their new names' changes say so).  Their tokens are
/// as the outline counts them (see `outline_diff::OutlineTokens`): edits are
/// added and removed tokens.
fn top_scope_changes(deltas: &SymbolDeltas) -> Vec<ScopeChange> {
    let mut changes: Vec<(u32, ScopeChange)> = deltas
        .iter()
        .filter_map(|(pretty, delta)| {
            let totals = &delta.token_totals;
            let (kind, other) = match delta.change {
                _ if pretty == "%" => ("changed", None),
                ChangeKind::Added => ("added", None),
                ChangeKind::Changed => ("changed", None),
                ChangeKind::Evolved => match (&delta.evolved_from, &delta.evolved_from_path) {
                    (_, Some(from_path)) => ("moved-in", Some(from_path.clone())),
                    (Some(from), None) => ("renamed", Some(from.clone())),
                    (None, None) => ("changed", None),
                },
                ChangeKind::Removed => match (&delta.evolved_into, &delta.evolved_into_path) {
                    (_, Some(into_path)) => ("moved-out", Some(into_path.clone())),
                    (Some(_), None) => return None,
                    (None, None) => ("removed", None),
                },
            };
            let weight = match kind {
                "renamed" | "moved-in" | "moved-out" => changed_tokens(totals) + totals.moved,
                _ => changed_tokens(totals),
            };
            Some((
                weight,
                ScopeChange {
                    kind,
                    pretty: pretty.clone(),
                    added: totals.added + totals.evolved_from,
                    removed: totals.removed + totals.evolved_into,
                    other,
                },
            ))
        })
        .filter(|(weight, change)| *weight > 0 || change.kind != "changed")
        .collect();
    changes.sort_by(|(a_weight, a), (b_weight, b)| {
        b_weight.cmp(a_weight).then_with(|| a.pretty.cmp(&b.pretty))
    });
    changes
        .into_iter()
        .take(TOP_SCOPE_CHANGES)
        .map(|(_, change)| change)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(change: ChangeKind, added: u32, removed: u32) -> SymbolDelta {
        SymbolDelta {
            change,
            evolved_from: None,
            evolved_into: None,
            evolved_from_path: None,
            evolved_into_path: None,
            token_totals: TokenDeltaDetails {
                added,
                removed,
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_top_scope_changes() {
        let mut deltas = SymbolDeltas::new();
        deltas.insert("%".to_string(), delta(ChangeKind::Changed, 2, 0));
        deltas.insert("Foo::a".to_string(), delta(ChangeKind::Added, 40, 0));
        deltas.insert("Foo::b".to_string(), delta(ChangeKind::Changed, 3, 9));
        // (A rename: the old name's removal, and the new name's evolution.)
        let mut old = delta(ChangeKind::Removed, 0, 0);
        old.evolved_into = Some("Foo::d".to_string());
        old.token_totals.evolved_into = 1;
        deltas.insert("Foo::c".to_string(), old);
        let mut new = delta(ChangeKind::Evolved, 1, 0);
        new.evolved_from = Some("Foo::c".to_string());
        new.token_totals.moved = 50;
        new.token_totals.evolved_from = 1;
        deltas.insert("Foo::d".to_string(), new);
        let change = file_change(&deltas);
        assert_eq!(change.size, 2 + 40 + 12 + 2);
        assert_eq!(
            change.scopes,
            vec![
                ScopeChange {
                    kind: "renamed",
                    pretty: "Foo::d".to_string(),
                    added: 2,
                    removed: 0,
                    other: Some("Foo::c".to_string()),
                },
                ScopeChange {
                    kind: "added",
                    pretty: "Foo::a".to_string(),
                    added: 40,
                    removed: 0,
                    other: None,
                },
            ]
        );
    }
}
