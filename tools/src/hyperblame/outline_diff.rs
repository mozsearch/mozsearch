//! Outline diffs: how a file's outline (its containers, ex: classes,
//! functions, sections, nested by their pretty identifiers' "::"; see
//! `FileStructureRow`) changed between two revisions, from the history's
//! outlines of the file at the two revisions and the file's files-delta
//! records of the commits between them (see `outline_diff`).
//!
//! Definitions are matched across the revisions by their pretty identifiers,
//! and by the records' renames (`ChangeKind::Evolved` symbols' `evolved_from`)
//! between them.  Overloads and preprocessor branches' definitions share a
//! pretty identifier, so they're one entry.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::DateTime;
use git2::{Commit, Repository};
use serde::Serialize;

use super::journals::{JournalKind, JournalReader};
use super::token_blame::TreeHistory;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::file_format::history::timeline_common::{
    ChangeKind, JournalVersionRef, SymbolSyntaxDeltaGroup, TokenDeltaDetails,
};
use crate::file_format::history::timeline_files_delta::FileDeltaRecord;

/// How far past the asked time `first_parent_before` looks for a commit the
/// history has before settling for the oldest one it saw.
const MAX_MISSING_STEPS: usize = 100;

/// A revision of an outline diff.
#[derive(Debug, Serialize)]
pub struct OutlineRev {
    pub rev: String,
    pub date: String,
}

/// A commit in the range of an outline diff which changed the file, or for
/// weeks the history summarizes, the week's newest commit standing for the
/// week's `count` commits.
#[derive(Debug, Serialize)]
pub struct OutlineCommit {
    pub rev: String,
    pub date: String,
    pub subject: String,
    #[serde(skip_serializing_if = "is_one")]
    pub count: usize,
}

fn is_one(n: &usize) -> bool {
    *n == 1
}

/// Token changes, counted as `TokenDeltaDetails` count them: the tokens added
/// (including evolved ones' new sides), removed (including evolved ones' old
/// sides), and moved (within or between files, not counting as changes).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OutlineTokens {
    pub added: u32,
    pub removed: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub moved: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl OutlineTokens {
    fn of(totals: &TokenDeltaDetails) -> OutlineTokens {
        OutlineTokens {
            added: totals.added + totals.evolved_from,
            removed: totals.removed + totals.evolved_into,
            moved: totals.moved,
        }
    }

    fn accumulate(&mut self, other: &OutlineTokens) {
        self.added += other.added;
        self.removed += other.removed;
        self.moved += other.moved;
    }

    fn changed(&self) -> bool {
        self.added + self.removed > 0
    }

    fn is_empty(&self) -> bool {
        *self == OutlineTokens::default()
    }
}

/// What happened to a definition between the two revisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutlineStatus {
    Unchanged,
    /// Tokens in it (not counting its nested definitions) were added or
    /// removed.
    Changed,
    Added,
    Removed,
    /// Renamed, from `evolved_from` in the same file.
    Renamed,
    /// Moved from `evolved_from` in another file.
    Moved,
}

/// The other end of a definition's rename or move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutlineEvolution {
    pub pretty: String,
    /// The other file, for moves between files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A definition of the union of the two revisions' outlines.
#[derive(Debug, Serialize)]
pub struct OutlineEntry {
    pub pretty: String,
    /// `pretty` past its nearest ancestor in the union's "::" (ex: "f" for
    /// "C::f" if there's a "C", or "C::f" for a method defined outside of its
    /// class).
    pub name: String,
    pub kind: String,
    /// It's only declared (ex: a method in its class in a header).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub decl: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pp: Vec<String>,
    /// How many of its "::" ancestors the union has.
    pub depth: usize,
    pub status: OutlineStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evolved_from: Option<OutlineEvolution>,
    /// For removed definitions, what they became in another file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evolved_into: Option<OutlineEvolution>,
    #[serde(skip_serializing_if = "OutlineTokens::is_empty")]
    pub tokens: OutlineTokens,
    /// The commits which changed it, as [index in `commits`, tokens added,
    /// tokens removed].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<(usize, u32, u32)>,
    /// Its first and last lines (see `TreeHistory::container_lines`) in the
    /// "to" file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<(u32, u32)>,
    /// Its (or for renamed definitions, its earlier name's) lines in the
    /// "from" file, if it's removed or `outline_diff` was asked for them all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_lines: Option<(u32, u32)>,
}

#[derive(Debug, Serialize)]
pub struct OutlineDiff {
    pub path: String,
    /// The file's path at `from`, if it was renamed (or copied) since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
    pub from: OutlineRev,
    pub to: OutlineRev,
    /// False if some of the range's records couldn't be read, or the history
    /// starts after the asked baseline (see `first_parent_before`).
    pub complete: bool,
    /// The range's commits which changed the file, newest first.
    pub commits: Vec<OutlineCommit>,
    /// The changes outside of any definition.
    pub toplevel: OutlineTokens,
    /// The union of the two outlines, in the "to" outline's order, with
    /// removed definitions after the definitions preceding them in the "from"
    /// outline.
    pub entries: Vec<OutlineEntry>,
}

/// A files-delta record of the range: a commit's, or a week's (see
/// `OutlineCommit`).
pub struct RangeRecord {
    /// The index of its commit in `OutlineDiff::commits`.
    pub commit: usize,
    pub group: SymbolSyntaxDeltaGroup,
}

/// The newest commit on `to`'s first-parent chain committed before `time`
/// (seconds since the epoch) which the history has, and whether it's that; if
/// the history starts after `time`, the oldest commit of the chain the history
/// has.
pub fn first_parent_before<'r>(
    history: &TreeHistory,
    to: &Commit<'r>,
    time: i64,
) -> Option<(Commit<'r>, bool)> {
    let mut commit = to.clone();
    let mut oldest = None;
    let mut missing = 0;
    loop {
        let known = history.timeline_commit(commit.id()).is_some();
        if commit.time().seconds() < time {
            if known {
                return Some((commit, true));
            }
            missing += 1;
            if missing > MAX_MISSING_STEPS {
                return oldest.map(|commit| (commit, false));
            }
        } else if known {
            oldest = Some(commit.clone());
        }
        commit = match commit.parent(0) {
            Ok(parent) => parent,
            Err(_) => return oldest.map(|commit| (commit, false)),
        };
    }
}

fn commit_date(commit: &Commit) -> String {
    DateTime::from_timestamp(commit.time().seconds(), 0)
        .map(|date| date.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

/// The subject and date of the commit `rev`.
fn subject(repo: &Repository, rev: &str) -> (String, String) {
    git2::Oid::from_str(rev)
        .and_then(|oid| repo.find_commit(oid))
        .map(|commit| {
            (
                commit
                    .summary()
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string(),
                commit_date(&commit),
            )
        })
        .unwrap_or_default()
}

/// The outline diff of `path` (as of `to`) from `from` to `to`, which must be
/// an ancestor of `to`, with the lines in the "from" file of all of the
/// definitions it had if `all_from_lines`, or else of the removed ones.
pub fn outline_diff(
    history: &TreeHistory,
    repo: &Repository,
    from: &Commit,
    to: &Commit,
    path: &str,
    all_from_lines: bool,
) -> Result<OutlineDiff, &'static str> {
    let to_timeline = history
        .timeline_commit(to.id())
        .ok_or("The history doesn't have the revision")?;
    let from_timeline = history
        .timeline_commit(from.id())
        .ok_or("The history doesn't have the baseline revision")?;
    let (_, to_rows) = history
        .files_struct(&to_timeline, path)
        .map_err(|_| "The history's outline of the file is broken")?
        .ok_or("The history doesn't have the file")?;

    // The range's revisions.
    let mut walk = repo.revwalk().map_err(|_| "Bad revisions")?;
    walk.push(to.id()).map_err(|_| "Bad revision")?;
    walk.hide(from.id()).map_err(|_| "Bad baseline revision")?;
    let range: HashSet<String> = walk
        .filter_map(Result::ok)
        .map(|oid| oid.to_string())
        .collect();

    let mut reader = JournalReader::new(&history.timeline);
    let records: Vec<FileDeltaRecord> = reader
        .records(&JournalVersionRef {
            timeline_rev: to_timeline.id().to_string(),
            path: JournalKind::FilesDelta.journal_path(path),
        })
        .map_err(|_| "The history's journal of the file is broken")?;

    // The range's records, expanding the summaries of weeks only partly in
    // it, and all of them if the file was renamed by a summarized commit, to
    // find its earlier path (which only detail records say).
    let mut collected = collect_range(repo, &mut reader, &records, &range, false);
    let mut from_rows = match &collected.from_path {
        Some(from_path) => history.files_struct(&from_timeline, from_path),
        None => history.files_struct(&from_timeline, path),
    }
    .map_err(|_| "The history's outline of the file is broken")?;
    if from_rows.is_none() && collected.whole_summaries {
        collected = collect_range(repo, &mut reader, &records, &range, true);
        if let Some(from_path) = &collected.from_path {
            from_rows = history
                .files_struct(&from_timeline, from_path)
                .map_err(|_| "The history's outline of the file is broken")?;
        }
    }
    let from_rows = from_rows.map(|(_, rows)| rows).unwrap_or_default();

    let (toplevel, mut entries) = classify(&from_rows, &to_rows, &collected.records);

    // Their lines, for going to them (without them if the file's tokens differ
    // from the history's, ex: if the tokenizer changed since).
    let lines_at = |timeline: &Commit, commit: &Commit, path: &str| {
        let source = commit
            .tree()
            .and_then(|tree| tree.get_path(Path::new(path)))
            .and_then(|entry| repo.find_blob(entry.id()))
            .ok()?;
        let source = std::str::from_utf8(source.content()).ok()?;
        history
            .container_lines(timeline, path, source)
            .ok()
            .flatten()
    };
    let to_lines = lines_at(&to_timeline, to, path);
    let removed = |entry: &OutlineEntry| entry.status == OutlineStatus::Removed;
    let from_lines = match all_from_lines || entries.iter().any(removed) {
        true => lines_at(
            &from_timeline,
            from,
            collected.from_path.as_deref().unwrap_or(path),
        ),
        false => None,
    };
    for entry in &mut entries {
        // (Its name in the "from" file, if it had it.)
        let from_name = match entry.status {
            OutlineStatus::Removed => Some(&entry.pretty),
            OutlineStatus::Unchanged | OutlineStatus::Changed if all_from_lines => {
                Some(&entry.pretty)
            }
            OutlineStatus::Renamed if all_from_lines => {
                entry.evolved_from.as_ref().map(|from| &from.pretty)
            }
            _ => None,
        };
        entry.from_lines = from_name
            .and_then(|name| from_lines.as_ref()?.get(name))
            .copied();
        if !removed(entry) {
            entry.lines = to_lines
                .as_ref()
                .and_then(|lines| lines.get(&entry.pretty).copied());
        }
    }
    Ok(OutlineDiff {
        path: path.to_string(),
        from_path: collected.from_path,
        from: OutlineRev {
            rev: from.id().to_string(),
            date: commit_date(from),
        },
        to: OutlineRev {
            rev: to.id().to_string(),
            date: commit_date(to),
        },
        complete: collected.complete,
        commits: collected.commits,
        toplevel,
        entries,
    })
}

struct Collected {
    commits: Vec<OutlineCommit>,
    records: Vec<RangeRecord>,
    from_path: Option<String>,
    /// Some summaries were wholly in the range, so their commits' renames
    /// are unknown.
    whole_summaries: bool,
    complete: bool,
}

/// The records of `records` (newest first) of the commits in `range`, without
/// the commits backed out in the range and their backouts.
fn collect_range(
    repo: &Repository,
    reader: &mut JournalReader,
    records: &[FileDeltaRecord],
    range: &HashSet<String>,
    expand_all: bool,
) -> Collected {
    let mut complete = true;
    let mut whole_summaries = false;
    let mut kept: Vec<FileDeltaRecord> = vec![];
    for record in records {
        match record {
            FileDeltaRecord::Detail(detail) => {
                if range.contains(&detail.desc.source_rev) {
                    kept.push(record.clone());
                }
            }
            FileDeltaRecord::Summary(summary) => {
                let in_range = summary
                    .desc
                    .source_revs
                    .iter()
                    .filter(|rev| range.contains(*rev))
                    .count();
                if in_range == 0 {
                    continue;
                }
                if in_range == summary.desc.source_revs.len() && !expand_all {
                    whole_summaries = true;
                    kept.push(record.clone());
                    continue;
                }
                match reader.expand(vec![record.clone()]) {
                    Ok(expanded) => kept.extend(expanded.into_iter().filter(|record| {
                        matches!(record, FileDeltaRecord::Detail(detail)
                            if range.contains(&detail.desc.source_rev))
                    })),
                    Err(_) => complete = false,
                }
            }
        }
    }

    // (Backouts of commits in the range cancel them out.)
    let mut backed_out: HashSet<&str> = HashSet::new();
    let detail_revs: HashSet<&str> = kept
        .iter()
        .filter_map(|record| match record {
            FileDeltaRecord::Detail(detail) => Some(detail.desc.source_rev.as_str()),
            _ => None,
        })
        .collect();
    for record in &kept {
        if let FileDeltaRecord::Detail(detail) = record
            && !detail.desc.backs_out.is_empty()
            && detail
                .desc
                .backs_out
                .iter()
                .all(|rev| detail_revs.contains(rev.as_str()))
        {
            backed_out.insert(&detail.desc.source_rev);
            backed_out.extend(detail.desc.backs_out.iter().map(String::as_str));
        }
    }

    let mut collected = Collected {
        commits: vec![],
        records: vec![],
        from_path: None,
        whole_summaries,
        complete,
    };
    for record in &kept {
        let (commit, group) = match record {
            FileDeltaRecord::Detail(detail) => {
                if detail.delta.change == ChangeKind::Evolved
                    && let Some(moved_from) = &detail.delta.moved_from
                {
                    collected.from_path = Some(moved_from.clone());
                }
                if backed_out.contains(detail.desc.source_rev.as_str()) {
                    continue;
                }
                (
                    OutlineCommit {
                        rev: detail.desc.source_rev.clone(),
                        date: detail.desc.iso_date.clone(),
                        subject: subject(repo, &detail.desc.source_rev).0,
                        count: 1,
                    },
                    &detail.delta.symbol_group,
                )
            }
            FileDeltaRecord::Summary(summary) => {
                let Some(newest) = summary.desc.source_revs.first() else {
                    continue;
                };
                let (subject, date) = subject(repo, newest);
                (
                    OutlineCommit {
                        rev: newest.clone(),
                        date,
                        subject,
                        count: summary.desc.source_revs.len(),
                    },
                    &summary.symbol_group,
                )
            }
        };
        collected.records.push(RangeRecord {
            commit: collected.commits.len(),
            group: group.clone(),
        });
        collected.commits.push(commit);
    }
    collected
}

/// A definition's changes in the range, by its name as of the "to" revision
/// (or, for removed ones, as of their removal).
#[derive(Default)]
struct Changes {
    tokens: OutlineTokens,
    changes: Vec<(usize, u32, u32)>,
    moved_from: Option<OutlineEvolution>,
    moved_into: Option<OutlineEvolution>,
}

impl Changes {
    fn add(&mut self, commit: usize, tokens: &OutlineTokens) {
        self.tokens.accumulate(tokens);
        if !tokens.changed() {
            return;
        }
        match self.changes.last_mut() {
            Some(last) if last.0 == commit => {
                last.1 += tokens.added;
                last.2 += tokens.removed;
            }
            _ => self.changes.push((commit, tokens.added, tokens.removed)),
        }
    }
}

/// The parent of `pretty` in `known`: its longest proper "::" prefix there.
fn nearest_ancestor<'a>(pretty: &'a str, known: &HashSet<&str>) -> Option<&'a str> {
    pretty
        .rmatch_indices("::")
        .map(|(index, _)| &pretty[..index])
        .find(|prefix| known.contains(prefix))
}

fn dedup(rows: &[FileStructureRow]) -> Vec<&FileStructureRow> {
    let mut seen = HashSet::new();
    rows.iter()
        .filter(|row| seen.insert(row.pretty.as_str()))
        .collect()
}

/// The changes outside of any definition and the entries of the union of the
/// outlines `from_rows` and `to_rows`, given the range's `records` (newest
/// first).
pub fn classify(
    from_rows: &[FileStructureRow],
    to_rows: &[FileStructureRow],
    records: &[RangeRecord],
) -> (OutlineTokens, Vec<OutlineEntry>) {
    // The definitions' changes, by their newest names, following renames
    // back through the records: `newest` maps older names to newer ones.
    let mut toplevel = OutlineTokens::default();
    let mut newest: HashMap<String, String> = HashMap::new();
    let mut changes: HashMap<String, Changes> = HashMap::new();
    for record in records {
        let resolve = |name: &str| {
            newest
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_string())
        };
        let mut renames = vec![];
        for (pretty, delta) in &record.group.symbol_deltas {
            let tokens = OutlineTokens::of(&delta.token_totals);
            if pretty == "%" {
                toplevel.accumulate(&tokens);
                continue;
            }
            let target = match (&delta.change, &delta.evolved_into, &delta.evolved_into_path) {
                (ChangeKind::Removed, Some(into), None) => resolve(into),
                (ChangeKind::Removed, _, _) => pretty.clone(),
                _ => resolve(pretty),
            };
            let entry = changes.entry(target.clone()).or_default();
            entry.add(record.commit, &tokens);
            match (&delta.change, &delta.evolved_from, &delta.evolved_from_path) {
                (ChangeKind::Evolved, Some(from), None) if *from != target => {
                    renames.push((from.clone(), target.clone()))
                }
                (ChangeKind::Evolved, Some(from), Some(from_path)) => {
                    entry.moved_from = Some(OutlineEvolution {
                        pretty: from.clone(),
                        path: Some(from_path.clone()),
                    })
                }
                _ => {}
            }
            if let (ChangeKind::Removed, Some(into_path)) =
                (&delta.change, &delta.evolved_into_path)
                && entry.moved_into.is_none()
            {
                entry.moved_into = Some(OutlineEvolution {
                    pretty: delta.evolved_into.clone().unwrap_or_else(|| pretty.clone()),
                    path: Some(into_path.clone()),
                });
            }
        }
        newest.extend(renames);
    }

    let to_rows = dedup(to_rows);
    let from_rows = dedup(from_rows);
    let to_names: HashSet<&str> = to_rows.iter().map(|row| row.pretty.as_str()).collect();
    let from_names: HashSet<&str> = from_rows.iter().map(|row| row.pretty.as_str()).collect();
    let resolve = |name: &str| {
        newest
            .get(name)
            .map(String::as_str)
            .unwrap_or(name)
            .to_string()
    };

    // Which "from" definition each "to" one was renamed from.
    let mut renamed_from: HashMap<String, &str> = HashMap::new();
    for row in &from_rows {
        let to_name = resolve(&row.pretty);
        if to_name != row.pretty
            && to_names.contains(to_name.as_str())
            && !from_names.contains(to_name.as_str())
        {
            renamed_from.entry(to_name).or_insert(&row.pretty);
        }
    }

    let entry =
        |row: &FileStructureRow, status, evolved_from, evolved_into, changes: Option<&Changes>| {
            OutlineEntry {
                pretty: row.pretty.clone(),
                name: String::new(),
                kind: row.kind.clone(),
                decl: !row.is_def,
                pp: row.pp.clone(),
                depth: 0,
                status,
                evolved_from,
                evolved_into,
                tokens: changes.map(|c| c.tokens.clone()).unwrap_or_default(),
                changes: changes.map(|c| c.changes.clone()).unwrap_or_default(),
                lines: None,
                from_lines: None,
            }
        };

    let mut entries: Vec<OutlineEntry> = to_rows
        .iter()
        .map(|row| {
            let found = changes.get(&row.pretty);
            if from_names.contains(row.pretty.as_str()) {
                let changed = found.is_some_and(|c| c.tokens.changed());
                let status = match changed {
                    true => OutlineStatus::Changed,
                    false => OutlineStatus::Unchanged,
                };
                entry(row, status, None, None, found)
            } else if let Some(from) = renamed_from.get(&row.pretty) {
                let evolution = OutlineEvolution {
                    pretty: from.to_string(),
                    path: None,
                };
                entry(row, OutlineStatus::Renamed, Some(evolution), None, found)
            } else if let Some(moved_from) = found.and_then(|c| c.moved_from.clone()) {
                entry(row, OutlineStatus::Moved, Some(moved_from), None, found)
            } else {
                entry(row, OutlineStatus::Added, None, None, found)
            }
        })
        .collect();

    // The removed definitions, each after the entry of the definition before
    // it in the "from" outline (or what it became), or for ones nested in
    // that one's ancestors' siblings, after that sibling's nested entries.
    let mut placed: Vec<Option<String>> = vec![];
    for row in &from_rows {
        let to_name = resolve(&row.pretty);
        if to_names.contains(to_name.as_str()) {
            placed.push(Some(to_name));
            continue;
        }
        let found = changes.get(&to_name);
        let removed = entry(
            row,
            OutlineStatus::Removed,
            None,
            found.and_then(|c| c.moved_into.clone()),
            found,
        );
        let anchor = placed.iter().rev().flatten().next();
        let index = match anchor {
            None => 0,
            Some(anchor) => {
                let at = entries
                    .iter()
                    .position(|entry| entry.pretty == *anchor)
                    .unwrap_or(entries.len() - 1);
                // The anchor's subtree to skip: its ancestor-or-self which is
                // a sibling of the removed one's, if any.
                let anchor_segments: Vec<&str> = anchor.split("::").collect();
                let removed_segments: Vec<&str> = row.pretty.split("::").collect();
                let common = anchor_segments
                    .iter()
                    .zip(&removed_segments[..removed_segments.len() - 1])
                    .take_while(|(a, b)| a == b)
                    .count();
                if common == anchor_segments.len() {
                    at + 1
                } else {
                    let subtree = format!("{}::", anchor_segments[..=common].join("::"));
                    at + 1
                        + entries[at + 1..]
                            .iter()
                            .take_while(|entry| entry.pretty.starts_with(&subtree))
                            .count()
                }
            }
        };
        placed.push(Some(row.pretty.clone()));
        entries.insert(index, removed);
    }

    // Their nesting.
    let known: HashSet<String> = entries.iter().map(|entry| entry.pretty.clone()).collect();
    let known: HashSet<&str> = known.iter().map(String::as_str).collect();
    for entry in &mut entries {
        let mut depth = 0;
        let mut name = entry.pretty.as_str();
        let mut ancestor = nearest_ancestor(&entry.pretty, &known);
        if let Some(parent) = ancestor {
            name = &entry.pretty[parent.len() + 2..];
        }
        while let Some(parent) = ancestor {
            depth += 1;
            ancestor = nearest_ancestor(parent, &known);
        }
        entry.name = name.to_string();
        entry.depth = depth;
    }

    (toplevel, entries)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::file_format::history::timeline_common::SymbolSyntaxDelta;

    fn row(pretty: &str) -> FileStructureRow {
        FileStructureRow {
            pretty: pretty.to_string(),
            is_def: true,
            kind: "function".to_string(),
            pp: vec![],
        }
    }

    fn delta(change: ChangeKind, added: u32, removed: u32) -> SymbolSyntaxDelta {
        let mut delta = SymbolSyntaxDelta::new(change);
        delta.token_totals.added = added;
        delta.token_totals.removed = removed;
        delta
    }

    fn record(commit: usize, deltas: Vec<(&str, SymbolSyntaxDelta)>) -> RangeRecord {
        RangeRecord {
            commit,
            group: SymbolSyntaxDeltaGroup {
                symbol_deltas: deltas
                    .into_iter()
                    .map(|(pretty, delta)| (pretty.to_string(), delta))
                    .collect::<BTreeMap<_, _>>(),
            },
        }
    }

    fn summary(entries: &[OutlineEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| {
                format!(
                    "{}{} {:?}{}",
                    "  ".repeat(entry.depth),
                    entry.name,
                    entry.status,
                    match &entry.evolved_from {
                        Some(from) => format!(" from {}", from.pretty),
                        None => String::new(),
                    }
                )
            })
            .collect()
    }

    #[test]
    fn statuses_and_order() {
        let from = vec![
            row("C"),
            row("C::a"),
            row("C::b"),
            row("C::old"),
            row("D"),
            row("f"),
        ];
        let to = vec![
            row("C"),
            row("C::a"),
            row("C::renamed"),
            row("D"),
            row("D::n"),
            row("f"),
        ];
        let mut renamed = delta(ChangeKind::Evolved, 1, 0);
        renamed.evolved_from = Some("C::b".to_string());
        let mut removed_b = delta(ChangeKind::Removed, 0, 1);
        removed_b.evolved_into = Some("C::renamed".to_string());
        let records = vec![
            record(
                0,
                vec![
                    ("C::a", delta(ChangeKind::Changed, 3, 2)),
                    ("%", delta(ChangeKind::Changed, 1, 0)),
                ],
            ),
            record(1, vec![("C::renamed", renamed), ("C::b", removed_b)]),
            record(
                2,
                vec![
                    ("C::old", delta(ChangeKind::Removed, 0, 9)),
                    ("D::n", delta(ChangeKind::Added, 5, 0)),
                ],
            ),
            record(3, vec![("C::a", delta(ChangeKind::Changed, 1, 0))]),
        ];
        let (toplevel, entries) = classify(&from, &to, &records);
        assert_eq!(toplevel.added, 1);
        assert_eq!(
            summary(&entries),
            vec![
                "C Unchanged",
                "  a Changed",
                "  renamed Renamed from C::b",
                "  old Removed",
                "D Unchanged",
                "  n Added",
                "f Unchanged",
            ]
        );
        let a = &entries[1];
        assert_eq!(
            a.tokens,
            OutlineTokens {
                added: 4,
                removed: 2,
                moved: 0
            }
        );
        assert_eq!(a.changes, vec![(0, 3, 2), (3, 1, 0)]);
        // The rename's two deltas are one change.
        assert_eq!(entries[2].changes, vec![(1, 1, 1)]);
    }

    #[test]
    fn removed_after_subtree_and_moves() {
        // "g" was after "C" and its nested definitions, so it goes after them,
        // and "h" moved to another file.
        let from = vec![row("C"), row("C::a"), row("g"), row("h")];
        let to = vec![row("C"), row("C::a"), row("C::b"), row("k")];
        let mut moved_out = delta(ChangeKind::Removed, 0, 0);
        moved_out.evolved_into = Some("h".to_string());
        moved_out.evolved_into_path = Some("other.rs".to_string());
        let mut moved_in = delta(ChangeKind::Evolved, 0, 0);
        moved_in.token_totals.moved = 7;
        moved_in.evolved_from = Some("k".to_string());
        moved_in.evolved_from_path = Some("old.rs".to_string());
        let records = vec![record(0, vec![("h", moved_out), ("k", moved_in)])];
        let (_, entries) = classify(&from, &to, &records);
        assert_eq!(
            summary(&entries),
            vec![
                "C Unchanged",
                "  a Unchanged",
                "  b Added",
                "g Removed",
                "h Removed",
                "k Moved from k",
            ]
        );
        assert_eq!(
            entries[4].evolved_into,
            Some(OutlineEvolution {
                pretty: "h".to_string(),
                path: Some("other.rs".to_string()),
            })
        );
        assert_eq!(entries[5].tokens.moved, 7);
        assert!(entries[5].changes.is_empty());
    }

    #[test]
    fn renames_chain_and_outside_definitions() {
        // "a" became "b" and then "c", and "C::m" is defined outside of "C".
        let from = vec![row("a"), row("C::m")];
        let to = vec![row("c"), row("C::m")];
        let mut to_c = delta(ChangeKind::Evolved, 0, 0);
        to_c.evolved_from = Some("b".to_string());
        let mut to_b = delta(ChangeKind::Evolved, 2, 0);
        to_b.evolved_from = Some("a".to_string());
        let records = vec![record(0, vec![("c", to_c)]), record(1, vec![("b", to_b)])];
        let (_, entries) = classify(&from, &to, &records);
        assert_eq!(
            summary(&entries),
            vec!["c Renamed from a", "C::m Unchanged"]
        );
        assert_eq!(entries[0].changes, vec![(1, 2, 0)]);
    }
}
