//! Following a token of a past revision of a file into the future, for the blame
//! popup's "follow this token into the future": where the token is now, or the
//! commit which removed it.  See "Follow this line forward until the present or
//! it is removed" in the hyperblame notes.
//!
//! We follow the token's identity (its canonical "introduced" ref) through the
//! records of the file's future journal after the revision:
//! - If a commit extinguished it, it was removed.
//! - If a commit evolved it, we continue with the token which evolved from it,
//!   the one whose predecessor is the identity in the commit's annotated file.
//! - If a commit moved it out of the file, we continue in whichever of the
//!   commit's changed files (from its rev-summary) has it.
//! - If a commit moved (renamed) or deleted the file, we continue with the new
//!   path, or the file was deleted.
//!
//! If no record changes the token, it's in the head revision with the same
//! identity.  This reads the future journals and a few annotated files rather
//! than walking revisions, but a token can change many times, so we limit how
//! many changes we follow.

use std::collections::HashSet;
use std::fs;

use git2::{Oid, Repository};
use serde::Serialize;

use super::journals::{JournalKind, JournalReader};
use super::token_blame::TreeHistory;
use crate::file_format::history::rev_summaries::{RevSummaryRecord, rev_summary_path};
use crate::file_format::history::syntax_files::token_file_lines;
use crate::file_format::history::timeline_annotated::{
    HyperLineData, HyperTokenRef, PATH_UNCHANGED,
};
use crate::file_format::history::timeline_common::{JournalVersionRef, TokenRefSet};
use crate::file_format::history::timeline_future::FutureRecord;

/// The most changes to a token we follow.
const MAX_CHANGES: usize = 64;

/// A change to a token on its way to the future.
#[derive(Debug, Serialize)]
pub struct Change {
    pub rev: String,
    /// "evolved" (the token changed into another token), "moved" (it moved to
    /// another file), or "renamed" (its file was renamed).
    pub kind: &'static str,
    /// The token's path and (1-based) index after the change.
    pub path: String,
    pub token: u32,
}

#[derive(Debug, Serialize)]
pub struct Future {
    /// "now" (the token is in the head revision at `path` and `token`),
    /// "removed" (`rev` removed it), "deleted" (`rev` deleted its file), or
    /// "lost" (we couldn't follow it further, or it changed too many times).
    pub outcome: &'static str,
    pub path: Option<String>,
    pub token: Option<u32>,
    pub rev: Option<String>,
    /// The changes to the token along the way, oldest first.
    pub changes: Vec<Change>,
}

/// The token whose canonical ref is `identity`, as its (0-based) index in
/// `annotated` (the annotated file for `path`), by searching for the start of
/// its annotated line, or if `as_predecessor`, the token whose predecessor it
/// is.
fn find_token(
    annotated: &str,
    path: &str,
    identity: &HyperTokenRef,
    as_predecessor: bool,
) -> Option<usize> {
    let mut relative = identity.clone();
    relative.relativize_path(path);
    let json = serde_json::to_string(&relative).unwrap();
    let needle = if as_predecessor {
        format!(",\"p\":{}", json)
    } else {
        format!("{{\"i\":{}", json)
    };
    let mut from = 0;
    while let Some(found) = annotated[from..].find(&needle) {
        let at = from + found;
        let line_start = annotated[..at].rfind('\n').map_or(0, |i| i + 1);
        // The introduced ref is at the start of the line, and the predecessor
        // comes right after it.
        let matches = if as_predecessor {
            annotated[line_start..at].starts_with("{\"i\":")
                && !annotated[line_start..at].contains("},")
        } else {
            line_start == at
        };
        if matches {
            let line = annotated.as_bytes()[..line_start]
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            // Line 0 is the file sentinel, so line N is token N.
            return line.checked_sub(1);
        }
        from = at + 1;
    }
    None
}

fn in_set(set: &TokenRefSet, path: &str, identity: &HyperTokenRef) -> bool {
    let identity_path = if identity.path == path {
        PATH_UNCHANGED
    } else {
        &identity.path
    };
    set.get(identity.source_rev.as_ref())
        .and_then(|paths| paths.get(identity_path))
        .is_some_and(|linenos| linenos.0.contains(&identity.lineno))
}

struct Follower<'a> {
    tree_history: &'a TreeHistory,
    timeline: &'a Repository,
    history_path: &'a str,
    head: git2::Oid,
}

impl Follower<'_> {
    fn timeline_rev(&self, source_rev: &str) -> Option<Oid> {
        let oid = Oid::from_str(source_rev).ok()?;
        Some(self.tree_history.timeline_commit(oid)?.id())
    }

    fn annotated(&self, timeline_rev: Oid, path: &str) -> Option<String> {
        let commit = self.timeline.find_commit(timeline_rev).ok()?;
        let tree = commit.tree().ok()?;
        let entry = tree
            .get_path(std::path::Path::new(&format!("annotated/{}", path)))
            .ok()?;
        let blob = self.timeline.find_blob(entry.id()).ok()?;
        Some(String::from_utf8_lossy(blob.content()).into_owned())
    }

    /// The detail records of the future journal for `path` for the revisions
    /// after `since` (a timeline revision), oldest first.
    fn records_after(&self, path: &str, since: Oid) -> Result<Vec<FutureRecord>, String> {
        let mut reader = JournalReader::new(self.timeline);
        let version = |timeline_rev: Oid| JournalVersionRef {
            timeline_rev: timeline_rev.to_string(),
            path: JournalKind::Future.journal_path(path),
        };
        let head_records = reader.records::<FutureRecord>(&version(self.head))?;
        let head_records = reader.expand(head_records)?;
        let since_records = reader.records::<FutureRecord>(&version(since))?;
        let since_revs: HashSet<String> = reader
            .expand(since_records)?
            .iter()
            .filter_map(detail_rev)
            .collect();
        let mut after: Vec<FutureRecord> = head_records
            .into_iter()
            .filter(|record| detail_rev(record).is_some_and(|rev| !since_revs.contains(&rev)))
            .collect();
        after.reverse();
        Ok(after)
    }

    fn changed_paths(&self, source_rev: &str) -> Vec<String> {
        let path = std::path::Path::new(self.history_path)
            .join("rev-summaries")
            .join(rev_summary_path(source_rev));
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<RevSummaryRecord>(&text).ok())
            .map(|summary| summary.file_deltas.into_keys().collect())
            .unwrap_or_default()
    }
}

fn detail_rev(record: &FutureRecord) -> Option<String> {
    match record {
        FutureRecord::Detail(detail) => Some(detail.desc.source_rev.clone()),
        _ => None,
    }
}

/// Follow the token with the (1-based) index `token` in the file at `path` in
/// revision `rev` into the future.
pub fn follow(
    tree_history: &TreeHistory,
    rev: Oid,
    path: &str,
    token: u32,
) -> Result<Future, String> {
    let timeline = &*tree_history.timeline;
    let history_path = &tree_history.path;
    let head = tree_history
        .head_timeline_commit()
        .ok_or("The history is empty")?
        .id();
    let follower = Follower {
        tree_history,
        timeline,
        history_path,
        head,
    };
    let lost = |changes| Future {
        outcome: "lost",
        path: None,
        token: None,
        rev: None,
        changes,
    };

    let mut since = follower
        .timeline_rev(&rev.to_string())
        .ok_or("The history doesn't have the revision")?;
    let annotated = follower
        .annotated(since, path)
        .ok_or("The history doesn't have the file")?;
    let line = token_file_lines(&annotated)
        .get(token as usize)
        .copied()
        .ok_or("No such token")?;
    let mut data: HyperLineData =
        serde_json::from_str(line).map_err(|e| format!("bad annotated line: {}", e))?;
    data.introduced.resolve_path(path);
    let mut identity = data.introduced.into_owned();
    let mut path = path.to_string();
    let mut changes = vec![];

    'follow: while changes.len() < MAX_CHANGES {
        for record in follower.records_after(&path, since)? {
            let FutureRecord::Detail(detail) = record else {
                continue;
            };
            let change_rev = detail.desc.source_rev.clone();
            let Some(change_timeline_rev) = follower.timeline_rev(&change_rev) else {
                return Ok(lost(changes));
            };
            if in_set(&detail.extinguished_tokens, &path, &identity) {
                return Ok(Future {
                    outcome: "removed",
                    path: Some(path),
                    token: None,
                    rev: Some(change_rev),
                    changes,
                });
            }
            if in_set(&detail.evolved_tokens, &path, &identity) {
                // The token which evolved from it may be in another file too,
                // but it's usually this one.
                let Some(annotated) = follower.annotated(change_timeline_rev, &path) else {
                    return Ok(lost(changes));
                };
                let Some(index) = find_token(&annotated, &path, &identity, true) else {
                    return Ok(lost(changes));
                };
                identity = HyperTokenRef {
                    source_rev: change_rev.clone().into(),
                    path: path.clone().into(),
                    lineno: index as u32 + 1,
                };
                changes.push(Change {
                    rev: change_rev,
                    kind: "evolved",
                    path: path.clone(),
                    token: index as u32 + 1,
                });
                since = change_timeline_rev;
                continue 'follow;
            }
            if in_set(&detail.moved_out_tokens, &path, &identity) {
                for candidate in follower.changed_paths(&change_rev) {
                    if candidate == path {
                        continue;
                    }
                    let Some(annotated) = follower.annotated(change_timeline_rev, &candidate)
                    else {
                        continue;
                    };
                    if let Some(index) = find_token(&annotated, &candidate, &identity, false) {
                        changes.push(Change {
                            rev: change_rev,
                            kind: "moved",
                            path: candidate.clone(),
                            token: index as u32 + 1,
                        });
                        path = candidate;
                        since = change_timeline_rev;
                        continue 'follow;
                    }
                }
                return Ok(lost(changes));
            }
            if let Some(moved_to) = &detail.file_changes.file_moved_to {
                let Some(annotated) = follower.annotated(change_timeline_rev, moved_to) else {
                    return Ok(lost(changes));
                };
                let Some(index) = find_token(&annotated, moved_to, &identity, false) else {
                    return Ok(lost(changes));
                };
                changes.push(Change {
                    rev: change_rev,
                    kind: "renamed",
                    path: moved_to.clone(),
                    token: index as u32 + 1,
                });
                path = moved_to.clone();
                since = change_timeline_rev;
                continue 'follow;
            }
            if detail.file_changes.file_deleted {
                return Ok(Future {
                    outcome: "deleted",
                    path: Some(path),
                    token: None,
                    rev: Some(change_rev),
                    changes,
                });
            }
        }

        // Nothing changed the token since, so it's in the head revision.
        let found = follower
            .annotated(head, &path)
            .and_then(|annotated| find_token(&annotated, &path, &identity, false));
        return Ok(match found {
            Some(index) => Future {
                outcome: "now",
                path: Some(path),
                token: Some(index as u32 + 1),
                rev: None,
                changes,
            },
            None => lost(changes),
        });
    }
    Ok(lost(changes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_common::token_ref_set_insert;

    fn token_ref(rev: &str, path: &str, lineno: u32) -> HyperTokenRef<'static> {
        HyperTokenRef {
            source_rev: rev.to_string().into(),
            path: path.to_string().into(),
            lineno,
        }
    }

    #[test]
    fn test_find_token() {
        let mut lines = vec![HyperLineData::new_introduced("a", 0).serialize()];
        lines.push(HyperLineData::new_introduced("a", 1).serialize());
        let mut evolved = HyperLineData::new_introduced("b", 2);
        evolved.predecessor = Some(HyperTokenRef::new_unchanged_path("a", 2));
        lines.push(evolved.serialize());
        let mut moved = HyperLineData::new_introduced("a", 12);
        moved.introduced.path = "other.rs".into();
        lines.push(moved.serialize());
        let annotated = lines.join("\n");

        assert_eq!(
            find_token(&annotated, "f.rs", &token_ref("a", "f.rs", 1), false),
            Some(0)
        );
        assert_eq!(
            find_token(&annotated, "f.rs", &token_ref("b", "f.rs", 2), false),
            Some(1)
        );
        assert_eq!(
            find_token(&annotated, "f.rs", &token_ref("a", "other.rs", 12), false),
            Some(2)
        );
        // "a:1" is a prefix of "a:12", but not a match.
        assert_eq!(
            find_token(&annotated, "f.rs", &token_ref("a", "other.rs", 1), false),
            None
        );
        // The token which evolved from "a:2" is token 2.
        assert_eq!(
            find_token(&annotated, "f.rs", &token_ref("a", "f.rs", 2), true),
            Some(1)
        );
        assert_eq!(
            find_token(&annotated, "f.rs", &token_ref("a", "f.rs", 2), false),
            None
        );
    }

    #[test]
    fn test_in_set() {
        let mut set = TokenRefSet::new();
        token_ref_set_insert(&mut set, "a", "%", 3);
        token_ref_set_insert(&mut set, "a", "other.rs", 4);
        assert!(in_set(&set, "f.rs", &token_ref("a", "f.rs", 3)));
        assert!(!in_set(&set, "f.rs", &token_ref("a", "f.rs", 4)));
        assert!(in_set(&set, "f.rs", &token_ref("a", "other.rs", 4)));
        assert!(!in_set(&set, "f.rs", &token_ref("b", "f.rs", 3)));
    }
}
