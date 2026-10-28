//! What became of a path which the tree doesn't have (anymore), for the 404s
//! of `/{tree}/source/{path}` (see `format::format_missing_source`): from its
//! future journal (see `file_format::history::timeline_future`), which, unlike
//! the file, is never moved or deleted, so the history knows about every file
//! it ever had, without tombstones.  Its newest record says where it moved
//! (followed to a path the tree has, or to where it was deleted), or that it
//! was deleted, and then where its content went: the files the deleting
//! commit moved its tokens into (their records' `moved_in_tokens`, which are
//! the same token references as its `moved_out_tokens`).  A directory has no
//! journal, but its files' journals are in the timeline's tree, whose fates
//! say where it went.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use git2::{Oid, Repository};
use serde::Serialize;

use crate::file_format::config::GitData;
use crate::file_format::history::timeline_common::{JournalVersionRef, TokenRefSet};
use crate::file_format::history::timeline_future::{FutureFileChanges, FutureRecord};
use crate::hyperblame::journals::{JournalKind, JournalReader};

/// The most moves followed.
const MAX_MOVES: usize = 32;
/// The most files a deleting commit changed which are checked for its tokens.
const MAX_DESTINATION_CANDIDATES: usize = 500;
/// The most files of a directory whose fates are found.
const MAX_DIRECTORY_FILES: usize = 2000;

/// A commit which moved or deleted a file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FateEvent {
    /// The file's path then.
    pub path: String,
    pub rev: String,
    pub date: String,
    /// Where it moved, if it moved (rather than being deleted).
    pub moved_to: Option<String>,
}

/// What became of a path.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Fate {
    /// It moved, maybe more than once, to `path`, which the tree has.
    Moved { path: String, moves: Vec<FateEvent> },
    /// It was deleted, by `deleted` (after its `moves`, if any): its last
    /// version is the deleting commit's parent's (`parent`), and its content
    /// moved to `content` (paths, by how many of its tokens each got, most
    /// first).
    Deleted {
        moves: Vec<FateEvent>,
        deleted: FateEvent,
        parent: Option<String>,
        content: Vec<(String, usize)>,
    },
    /// A directory whose files the tree doesn't have anymore: how many
    /// (`files`, of which `deleted` were deleted), and the directories they
    /// moved to, by how many files each got, most first.
    Directory {
        files: usize,
        deleted: usize,
        moved_to: Vec<(String, usize)>,
    },
}

/// The fate of `path`, which the tree's revision `tip` doesn't have, if its
/// history knows of it.
pub fn fate(git: &GitData, tip: Oid, path: &str) -> Option<Fate> {
    let history = git.history.as_ref()?;
    let timeline: &Repository = &history.timeline;
    let timeline_rev = history.timeline_commit(tip)?.id();
    let mut reader = JournalReader::new(timeline);
    let tip_tree = git.repo.find_commit(tip).ok()?.tree().ok()?;
    let exists = |path: &str| tip_tree.get_path(Path::new(path)).is_ok();

    let Some(first) = newest_change(&mut reader, timeline_rev, path) else {
        return directory_fate(&mut reader, timeline, timeline_rev, path, &exists);
    };
    let mut moves = vec![];
    let mut seen = HashSet::new();
    let mut event = first;
    loop {
        match event.moved_to.clone() {
            Some(to) => {
                moves.push(event);
                if exists(&to) {
                    return Some(Fate::Moved { path: to, moves });
                }
                if moves.len() >= MAX_MOVES || !seen.insert(to.clone()) {
                    return None;
                }
                event = newest_change(&mut reader, timeline_rev, &to)?;
            }
            None => {
                let parent = Oid::from_str(&event.rev)
                    .ok()
                    .and_then(|rev| git.repo.find_commit(rev).ok())
                    .and_then(|commit| commit.parent_id(0).ok());
                let content = parent
                    .map(|parent| moved_content(&mut reader, git, &event, parent))
                    .unwrap_or_default();
                return Some(Fate::Deleted {
                    moves,
                    deleted: event,
                    parent: parent.map(|parent| parent.to_string()),
                    content,
                });
            }
        }
    }
}

/// The newest record of `path`'s future journal (at the timeline's
/// `timeline_rev`; a summary's newest detail), if it moved or deleted the
/// file; with its file changes and moved out tokens.
fn newest_detail(
    reader: &mut JournalReader,
    timeline_rev: Oid,
    path: &str,
) -> Option<(FateEvent, FutureFileChanges, TokenRefSet)> {
    let version = JournalVersionRef {
        timeline_rev: timeline_rev.to_string(),
        path: JournalKind::Future.journal_path(path),
    };
    let mut records: Vec<FutureRecord> = reader.records(&version).ok()?;
    if matches!(records.first(), Some(FutureRecord::Summary(_))) {
        records.truncate(1);
        records = reader.expand(records).ok()?;
    }
    // (A summary's details are in its journal's order, newest first.)
    let detail = records.into_iter().find_map(|record| match record {
        FutureRecord::Detail(detail) => Some(detail),
        FutureRecord::Summary(_) => None,
    })?;
    let changes = &detail.file_changes;
    (changes.file_deleted || changes.file_moved_to.is_some()).then(|| {
        (
            FateEvent {
                path: path.to_string(),
                rev: detail.desc.source_rev.clone(),
                date: detail.desc.iso_date.clone(),
                moved_to: changes.file_moved_to.clone(),
            },
            detail.file_changes.clone(),
            detail.moved_out_tokens.clone(),
        )
    })
}

/// `newest_detail`'s event: the revision which moved or deleted `path`, if
/// its newest record (as of the timeline's `timeline_rev`) is one.
pub(crate) fn newest_change(
    reader: &mut JournalReader,
    timeline_rev: Oid,
    path: &str,
) -> Option<FateEvent> {
    newest_detail(reader, timeline_rev, path).map(|(event, _, _)| event)
}

/// A token reference set's references, with `%` (the journal's own path)
/// resolved to `path`.
fn token_refs(set: &TokenRefSet, path: &str) -> BTreeSet<(String, String, u32)> {
    let mut refs = BTreeSet::new();
    for (rev, paths) in set {
        for (token_path, lines) in paths {
            let token_path = if token_path == "%" { path } else { token_path };
            for line in &lines.0 {
                refs.insert((rev.clone(), token_path.to_string(), *line));
            }
        }
    }
    refs
}

/// Where the content of the file `deleted` deleted went: the files its commit
/// changed (but it) whose records for it moved its tokens in, by how many.
fn moved_content(
    reader: &mut JournalReader,
    git: &GitData,
    deleted: &FateEvent,
    parent: Oid,
) -> Vec<(String, usize)> {
    let Some(history) = git.history.as_ref() else {
        return vec![];
    };
    let Ok(rev) = Oid::from_str(&deleted.rev) else {
        return vec![];
    };
    let Some(timeline_rev) = history.timeline_commit(rev).map(|commit| commit.id()) else {
        return vec![];
    };
    // (Its moved out tokens, from its own record at the timeline's commit of
    // the deleting revision, whose newest record it is.)
    let Some((_, _, moved_out)) = newest_detail(reader, timeline_rev, &deleted.path) else {
        return vec![];
    };
    let moved_out = token_refs(&moved_out, &deleted.path);
    if moved_out.is_empty() {
        return vec![];
    }
    let changed = changed_files(&git.repo, parent, rev);
    let mut content: Vec<(String, usize)> = changed
        .into_iter()
        .filter(|path| *path != deleted.path)
        .take(MAX_DESTINATION_CANDIDATES)
        .filter_map(|path| {
            let version = JournalVersionRef {
                timeline_rev: timeline_rev.to_string(),
                path: JournalKind::Future.journal_path(&path),
            };
            let records: Vec<FutureRecord> = reader.records(&version).ok()?;
            let FutureRecord::Detail(detail) = records.into_iter().next()? else {
                return None;
            };
            if detail.desc.source_rev != deleted.rev {
                return None;
            }
            let count = token_refs(&detail.moved_in_tokens, &path)
                .intersection(&moved_out)
                .count();
            (count > 0).then_some((path, count))
        })
        .collect();
    content.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    content
}

/// The files `rev` added or modified, from its `parent`.
fn changed_files(repo: &Repository, parent: Oid, rev: Oid) -> Vec<String> {
    let tree = |oid: Oid| repo.find_commit(oid).and_then(|commit| commit.tree()).ok();
    let (Some(old), Some(new)) = (tree(parent), tree(rev)) else {
        return vec![];
    };
    let Ok(diff) = repo.diff_tree_to_tree(Some(&old), Some(&new), None) else {
        return vec![];
    };
    diff.deltas()
        .filter(|delta| delta.status() != git2::Delta::Deleted)
        .filter_map(|delta| Some(delta.new_file().path()?.to_str()?.to_string()))
        .collect()
}

/// The fate of a directory (see `Fate::Directory`), from its files' journals
/// in the timeline's tree.
fn directory_fate(
    reader: &mut JournalReader,
    timeline: &Repository,
    timeline_rev: Oid,
    path: &str,
    exists: &dyn Fn(&str) -> bool,
) -> Option<Fate> {
    let dir = path.trim_end_matches('/');
    let tree = timeline
        .find_commit(timeline_rev)
        .ok()?
        .tree()
        .ok()?
        .get_path(Path::new(&format!("{}/{}", JournalKind::Future.dir(), dir)))
        .ok()?
        .to_object(timeline)
        .ok()?
        .peel_to_tree()
        .ok()?;
    let mut files: Vec<String> = vec![];
    let _ = tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if files.len() >= MAX_DIRECTORY_FILES {
            return git2::TreeWalkResult::Abort;
        }
        if entry.kind() == Some(git2::ObjectType::Blob)
            && let Some(name) = entry
                .name()
                .ok()
                .and_then(|name| name.strip_suffix(".ndjson"))
        {
            files.push(format!("{}{}", root, name));
        }
        git2::TreeWalkResult::Ok
    });
    if files.is_empty() {
        return None;
    }
    let mut deleted = 0;
    let mut moved_to: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        let Some(event) = newest_change(reader, timeline_rev, &format!("{}/{}", dir, file)) else {
            continue;
        };
        match &event.moved_to {
            // (Where the directory went: the destination without the file's
            // path in the directory, if it kept it, else its directory.)
            Some(to) => {
                let to_dir = to
                    .strip_suffix(&format!("/{}", file))
                    .or_else(|| to.rsplit_once('/').map(|(dir, _)| dir))
                    .unwrap_or("");
                if exists(to_dir) || to_dir.is_empty() {
                    *moved_to.entry(to_dir.to_string()).or_default() += 1;
                }
            }
            None => deleted += 1,
        }
    }
    let mut moved_to: Vec<(String, usize)> = moved_to.into_iter().collect();
    moved_to.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Some(Fate::Directory {
        files: files.len(),
        deleted,
        moved_to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_common::TokenLinenoSet;

    #[test]
    fn test_token_refs() {
        let mut out = TokenRefSet::new();
        out.entry("r1".to_string())
            .or_default()
            .insert("%".to_string(), TokenLinenoSet([1, 2].into()));
        let mut moved_in = TokenRefSet::new();
        moved_in
            .entry("r1".to_string())
            .or_default()
            .insert("old.js".to_string(), TokenLinenoSet([2, 3].into()));
        // (The deleted file's "%" is its path; the destination's tokens
        // introduced in the deleted file name it.)
        let shared: Vec<_> = token_refs(&out, "old.js")
            .intersection(&token_refs(&moved_in, "new.js"))
            .cloned()
            .collect();
        assert_eq!(shared, [("r1".to_string(), "old.js".to_string(), 2)]);
    }
}
