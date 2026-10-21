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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::rc::Rc;

use git2::{Oid, Repository};
use serde::Serialize;

use super::journals::{JournalKind, JournalReader};
use super::token_blame::TreeHistory;
use crate::file_format::history::rev_summaries::{RevSummaryRecord, rev_summary_path};
use crate::file_format::history::timeline_annotated::{
    HyperLineData, HyperTokenRef, PATH_UNCHANGED, annotated_line, find_token,
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

/// How many annotated files a `Follower` keeps (each can be large, ex: ~110 MB
/// for the HTML Standard's `source`): enough for `follow_through`'s three
/// revisions.
const ANNOTATED_CACHE_SIZE: usize = 3;

/// Follows tokens of a history into the future (see `follow`), sharing what
/// it reads between them: the future journals (expanding the head's for a
/// file reads most of the file's history, ex: ~60s for the HTML Standard's
/// `source`, so following its tokens one at a time would take a minute each)
/// and the last annotated files.
pub struct Follower<'a> {
    tree_history: &'a TreeHistory,
    timeline: &'a Repository,
    history_path: &'a str,
    head: Oid,
    reader: RefCell<JournalReader<'a>>,
    /// The detail records of the head's future journal, by path, newest
    /// first.
    head_records: RefCell<HashMap<String, Rc<Vec<FutureRecord>>>>,
    /// The source revisions of the detail records of a version of a future
    /// journal, by its path and timeline revision.
    since_revs: RefCell<HashMap<(String, Oid), Rc<HashSet<String>>>>,
    /// The annotated files read last, by timeline revision and path, newest
    /// last.
    annotated: RefCell<Vec<((Oid, String), Rc<String>)>>,
}

impl<'a> Follower<'a> {
    pub fn new(tree_history: &'a TreeHistory) -> Result<Follower<'a>, String> {
        let timeline = &*tree_history.timeline;
        let head = tree_history
            .head_timeline_commit()
            .ok_or("The history is empty")?
            .id();
        Ok(Follower {
            tree_history,
            timeline,
            history_path: &tree_history.path,
            head,
            reader: RefCell::new(JournalReader::new(timeline)),
            head_records: RefCell::default(),
            since_revs: RefCell::default(),
            annotated: RefCell::default(),
        })
    }

    fn timeline_rev(&self, source_rev: &str) -> Option<Oid> {
        let oid = Oid::from_str(source_rev).ok()?;
        Some(self.tree_history.timeline_commit(oid)?.id())
    }

    fn annotated(&self, timeline_rev: Oid, path: &str) -> Option<Rc<String>> {
        let key = (timeline_rev, path.to_string());
        let mut cache = self.annotated.borrow_mut();
        if let Some(at) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(at);
            let text = entry.1.clone();
            cache.push(entry);
            return Some(text);
        }
        let commit = self.timeline.find_commit(timeline_rev).ok()?;
        let tree = commit.tree().ok()?;
        let entry = tree
            .get_path(std::path::Path::new(&format!("annotated/{}", path)))
            .ok()?;
        let blob = self.timeline.find_blob(entry.id()).ok()?;
        let text = Rc::new(String::from_utf8_lossy(blob.content()).into_owned());
        if cache.len() >= ANNOTATED_CACHE_SIZE {
            cache.remove(0);
        }
        cache.push((key, text.clone()));
        Some(text)
    }

    fn version(path: &str, timeline_rev: Oid) -> JournalVersionRef {
        JournalVersionRef {
            timeline_rev: timeline_rev.to_string(),
            path: JournalKind::Future.journal_path(path),
        }
    }

    /// The detail records of the future journal for `path` (newest first),
    /// and the source revisions of those of its version as of `since` (a
    /// timeline revision): the records after `since` are the others.
    #[allow(clippy::type_complexity)]
    fn records_since(
        &self,
        path: &str,
        since: Oid,
    ) -> Result<(Rc<Vec<FutureRecord>>, Rc<HashSet<String>>), String> {
        let head_records = match self.head_records.borrow().get(path) {
            Some(records) => Some(records.clone()),
            None => None,
        };
        let head_records = match head_records {
            Some(records) => records,
            None => {
                let mut reader = self.reader.borrow_mut();
                let records = reader.records::<FutureRecord>(&Self::version(path, self.head))?;
                let records = Rc::new(reader.expand(records)?);
                self.head_records
                    .borrow_mut()
                    .insert(path.to_string(), records.clone());
                records
            }
        };
        let key = (path.to_string(), since);
        let since_revs = match self.since_revs.borrow().get(&key) {
            Some(revs) => Some(revs.clone()),
            None => None,
        };
        let since_revs = match since_revs {
            Some(revs) => revs,
            None => {
                let mut reader = self.reader.borrow_mut();
                let records = reader.records::<FutureRecord>(&Self::version(path, since))?;
                let revs: HashSet<String> = reader
                    .expand(records)?
                    .iter()
                    .filter_map(detail_rev)
                    .collect();
                let revs = Rc::new(revs);
                self.since_revs.borrow_mut().insert(key, revs.clone());
                revs
            }
        };
        Ok((head_records, since_revs))
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
/// revision `rev` into the future.  (To follow several tokens, use one
/// `Follower`.)
pub fn follow(
    tree_history: &TreeHistory,
    rev: Oid,
    path: &str,
    token: u32,
) -> Result<Future, String> {
    Follower::new(tree_history)?.follow(rev, path, token)
}

impl Follower<'_> {
    /// Follow the token with the (1-based) index `token` in the file at `path`
    /// in revision `rev` into the future, as `follow` does, given a later
    /// revision, `changed`, before which no revision changed it (ex: for a
    /// definition's tokens in its last revision, the revision which removed
    /// it): by what `changed`'s own future-journal record says happened to the
    /// token, and then by its identity in the head's annotated file, without
    /// reading the file's whole future journal (which `follow` does), unless
    /// the token changed again since, or the record isn't there.
    pub fn follow_through(
        &self,
        rev: Oid,
        path: &str,
        token: u32,
        changed: Oid,
    ) -> Result<Future, String> {
        let lost = |changes| Future {
            outcome: "lost",
            path: None,
            token: None,
            rev: None,
            changes,
        };
        let since = self
            .timeline_rev(&rev.to_string())
            .ok_or("The history doesn't have the revision")?;
        let annotated = self
            .annotated(since, path)
            .ok_or("The history doesn't have the file")?;
        let line = annotated_line(&annotated, token as usize).ok_or("No such token")?;
        let mut data =
            HyperLineData::try_parse(&line).map_err(|e| format!("bad annotated line: {}", e))?;
        data.introduced.resolve_path(path);
        let mut identity = data.introduced.into_owned();

        let changed_rev = changed.to_string();
        let Some(changed_timeline_rev) = self.timeline_rev(&changed_rev) else {
            return self.follow(rev, path, token);
        };
        let records = self
            .reader
            .borrow_mut()
            .records::<FutureRecord>(&Self::version(path, changed_timeline_rev))?;
        let Some(detail) = records.iter().find_map(|record| match record {
            FutureRecord::Detail(detail) if detail.desc.source_rev == changed_rev => Some(detail),
            _ => None,
        }) else {
            return self.follow(rev, path, token);
        };
        let mut path = path.to_string();
        let mut changes = vec![];
        // (Its index as of `changed`, if we know it.)
        let mut index = None;
        if in_set(&detail.extinguished_tokens, &path, &identity) {
            return Ok(Future {
                outcome: "removed",
                path: Some(path),
                token: None,
                rev: Some(changed_rev),
                changes,
            });
        } else if in_set(&detail.evolved_tokens, &path, &identity) {
            let found = self
                .annotated(changed_timeline_rev, &path)
                .and_then(|annotated| find_token(&annotated, &path, &identity, true));
            let Some(at) = found else {
                return Ok(lost(changes));
            };
            identity = HyperTokenRef {
                source_rev: changed_rev.clone().into(),
                path: path.clone().into(),
                lineno: at as u32 + 1,
            };
            changes.push(Change {
                rev: changed_rev.clone(),
                kind: "evolved",
                path: path.clone(),
                token: at as u32 + 1,
            });
            index = Some(at);
        } else if in_set(&detail.moved_out_tokens, &path, &identity) {
            let found = self
                .changed_paths(&changed_rev)
                .into_iter()
                .find_map(|candidate| {
                    if candidate == path {
                        return None;
                    }
                    let annotated = self.annotated(changed_timeline_rev, &candidate)?;
                    let at = find_token(&annotated, &candidate, &identity, false)?;
                    Some((candidate, at))
                });
            let Some((candidate, at)) = found else {
                return Ok(lost(changes));
            };
            changes.push(Change {
                rev: changed_rev.clone(),
                kind: "moved",
                path: candidate.clone(),
                token: at as u32 + 1,
            });
            path = candidate;
            index = Some(at);
        } else if let Some(moved_to) = &detail.file_changes.file_moved_to {
            let found = self
                .annotated(changed_timeline_rev, moved_to)
                .and_then(|annotated| find_token(&annotated, moved_to, &identity, false));
            let Some(at) = found else {
                return Ok(lost(changes));
            };
            changes.push(Change {
                rev: changed_rev.clone(),
                kind: "renamed",
                path: moved_to.clone(),
                token: at as u32 + 1,
            });
            path = moved_to.clone();
            index = Some(at);
        } else if detail.file_changes.file_deleted {
            return Ok(Future {
                outcome: "deleted",
                path: Some(path),
                token: None,
                rev: Some(changed_rev),
                changes,
            });
        }

        // Then it's in the head revision with that identity, unless it
        // changed again since, when we follow it from `changed`.
        let found = self
            .annotated(self.head, &path)
            .and_then(|annotated| find_token(&annotated, &path, &identity, false));
        if let Some(at) = found {
            return Ok(Future {
                outcome: "now",
                path: Some(path),
                token: Some(at as u32 + 1),
                rev: None,
                changes,
            });
        }
        let index = index.or_else(|| {
            let annotated = self.annotated(changed_timeline_rev, &path)?;
            find_token(&annotated, &path, &identity, false)
        });
        let Some(index) = index else {
            return Ok(lost(changes));
        };
        let mut future = self.follow(changed, &path, index as u32 + 1)?;
        changes.append(&mut future.changes);
        future.changes = changes;
        Ok(future)
    }

    /// Follow a token into the future, as `follow` does.
    pub fn follow(&self, rev: Oid, path: &str, token: u32) -> Result<Future, String> {
        let lost = |changes| Future {
            outcome: "lost",
            path: None,
            token: None,
            rev: None,
            changes,
        };

        let mut since = self
            .timeline_rev(&rev.to_string())
            .ok_or("The history doesn't have the revision")?;
        let annotated = self
            .annotated(since, path)
            .ok_or("The history doesn't have the file")?;
        let line = annotated_line(&annotated, token as usize).ok_or("No such token")?;
        let mut data =
            HyperLineData::try_parse(&line).map_err(|e| format!("bad annotated line: {}", e))?;
        data.introduced.resolve_path(path);
        let mut identity = data.introduced.into_owned();
        let mut path = path.to_string();
        let mut changes = vec![];

        'follow: while changes.len() < MAX_CHANGES {
            let (records, since_revs) = self.records_since(&path, since)?;
            for record in records.iter().rev() {
                let FutureRecord::Detail(detail) = record else {
                    continue;
                };
                if since_revs.contains(&detail.desc.source_rev) {
                    continue;
                }
                let change_rev = detail.desc.source_rev.clone();
                let Some(change_timeline_rev) = self.timeline_rev(&change_rev) else {
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
                    let Some(annotated) = self.annotated(change_timeline_rev, &path) else {
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
                    for candidate in self.changed_paths(&change_rev) {
                        if candidate == path {
                            continue;
                        }
                        let Some(annotated) = self.annotated(change_timeline_rev, &candidate)
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
                    let Some(annotated) = self.annotated(change_timeline_rev, moved_to) else {
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
            let found = self
                .annotated(self.head, &path)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_annotated::encode_annotated;
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
        // (The table form, and older histories' JSON lines.)
        let json: Vec<String> = lines
            .iter()
            .map(|line| serde_json::to_string(&HyperLineData::parse(line)).unwrap())
            .collect();
        for annotated in [encode_annotated(&lines), json.join("\n")] {
            check_find_token(&annotated);
        }
    }

    fn check_find_token(annotated: &str) {
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
