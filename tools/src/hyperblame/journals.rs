//! Reading the timeline repo's journals (see `file_format::history`), including
//! expanding summary records back into the detail records they summarize.
//!
//! ## Expansion
//!
//! A summary record aggregates the detail records of its `source_revs` and
//! lists the versions of the journal (`preds`) which contain those detail
//! records, or summaries of them whose own `preds` do, and so on.  So the
//! details behind any summary can be recovered by loading its preds from the
//! timeline repo.  This is how a UI can show full detail for an old time range,
//! and how we verify consolidation: expanding a consolidated journal must
//! produce the same detail records as the journal would have without
//! consolidation (see the `compare-timeline-journals` tool and the
//! `history-journal` searchfox-tool command).
//!
//! Expanded journals keep the journal's order, with the details behind a
//! summary taking its place in the order they're found in its preds.  A source
//! revision is only listed once even if it's both summarized and detailed (ex:
//! when the parents of a merge disagree about what's summarized).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use git2::{Oid, Repository};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::file_format::history::timeline_common::{JournalVersionRef, TimelineRecord};
use crate::file_format::history::timeline_files_delta::FileDeltaRecord;
use crate::file_format::history::timeline_future::FutureRecord;
use crate::file_format::history::timeline_tokens::{TokenDeltaRecord, token_timeline_path};

/// A limit on how deeply summaries can refer to other summaries, which is far
/// beyond anything consolidation should produce, so that a cycle is an error
/// rather than a hang.
const MAX_EXPANSION_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalKind {
    Future,
    FilesDelta,
    Tokens,
}

impl JournalKind {
    pub const ALL: [JournalKind; 3] = [
        JournalKind::Future,
        JournalKind::FilesDelta,
        JournalKind::Tokens,
    ];

    /// The top-level directory of this kind of journal in the timeline repo.
    pub fn dir(self) -> &'static str {
        match self {
            JournalKind::Future => "future",
            JournalKind::FilesDelta => "files-delta",
            JournalKind::Tokens => "tokens",
        }
    }

    /// The path in the timeline repo of the journal for `target`, which is a
    /// source path for future and files-delta journals, and a token for token
    /// journals.
    pub fn journal_path(self, target: &str) -> String {
        match self {
            JournalKind::Future | JournalKind::FilesDelta => {
                format!("{}/{}.ndjson", self.dir(), target)
            }
            JournalKind::Tokens => token_timeline_path(target).to_string_lossy().into_owned(),
        }
    }

    /// The kind of the journal at a path in the timeline repo, if it's a
    /// journal.
    pub fn for_path(path: &str) -> Option<JournalKind> {
        let (dir, _) = path.split_once('/')?;
        JournalKind::ALL.into_iter().find(|kind| kind.dir() == dir)
    }
}

/// Reads journals from a timeline repo, caching the journal versions it loads
/// (which expansion can load repeatedly).
pub struct JournalReader<'a> {
    repo: &'a Repository,
    cache: HashMap<JournalVersionRef, Option<Rc<String>>>,
}

impl<'a> JournalReader<'a> {
    pub fn new(repo: &'a Repository) -> JournalReader<'a> {
        JournalReader {
            repo,
            cache: HashMap::new(),
        }
    }

    /// The contents of a journal version, or None if that commit doesn't have
    /// the journal.
    fn contents(&mut self, version: &JournalVersionRef) -> Result<Option<Rc<String>>, String> {
        if let Some(contents) = self.cache.get(version) {
            return Ok(contents.clone());
        }
        let describe = |e: git2::Error| format!("{}:{}: {}", version.timeline_rev, version.path, e);
        let oid = Oid::from_str(&version.timeline_rev).map_err(describe)?;
        let tree = self
            .repo
            .find_commit(oid)
            .and_then(|commit| commit.tree())
            .map_err(describe)?;
        let contents = match tree.get_path(Path::new(&version.path)) {
            Ok(entry) => {
                let blob = self.repo.find_blob(entry.id()).map_err(describe)?;
                let text = String::from_utf8(blob.content().to_vec())
                    .map_err(|e| format!("{}:{}: {}", version.timeline_rev, version.path, e))?;
                Some(Rc::new(text))
            }
            Err(_) => None,
        };
        self.cache.insert(version.clone(), contents.clone());
        Ok(contents)
    }

    /// The records of a journal version (without its header line), newest to
    /// oldest, which is empty if the journal doesn't exist.
    pub fn records<R: DeserializeOwned>(
        &mut self,
        version: &JournalVersionRef,
    ) -> Result<Vec<R>, String> {
        let Some(contents) = self.contents(version)? else {
            return Ok(vec![]);
        };
        contents
            .lines()
            .skip(1)
            .map(|line| {
                serde_json::from_str(line).map_err(|e| {
                    format!(
                        "{}:{}: bad record {:?}: {}",
                        version.timeline_rev, version.path, line, e
                    )
                })
            })
            .collect()
    }

    /// Replace the summary records of a journal's records with the detail
    /// records they summarize; see the module docs.
    pub fn expand<R: TimelineRecord + DeserializeOwned>(
        &mut self,
        records: Vec<R>,
    ) -> Result<Vec<R>, String> {
        expand_records(records, &mut |version| self.records(version))
    }

    fn records_json_as<R: TimelineRecord + DeserializeOwned + Serialize>(
        &mut self,
        version: &JournalVersionRef,
        expand: bool,
    ) -> Result<Vec<Value>, String> {
        let mut records = self.records::<R>(version)?;
        if expand {
            records = self.expand(records)?;
        }
        records
            .iter()
            .map(|record| serde_json::to_value(record).map_err(|e| e.to_string()))
            .collect()
    }

    /// The records of a journal version as JSON values, expanded if requested.
    pub fn records_json(
        &mut self,
        kind: JournalKind,
        version: &JournalVersionRef,
        expand: bool,
    ) -> Result<Vec<Value>, String> {
        match kind {
            JournalKind::Future => self.records_json_as::<FutureRecord>(version, expand),
            JournalKind::FilesDelta => self.records_json_as::<FileDeltaRecord>(version, expand),
            JournalKind::Tokens => self.records_json_as::<TokenDeltaRecord>(version, expand),
        }
    }
}

/// Replace the summary records in `records` with the detail records they
/// summarize (see the module docs), using `load` to load the records of journal
/// versions.
pub fn expand_records<R: TimelineRecord>(
    records: Vec<R>,
    load: &mut impl FnMut(&JournalVersionRef) -> Result<Vec<R>, String>,
) -> Result<Vec<R>, String> {
    let mut seen = HashSet::new();
    expand_wanted(records, None, &mut seen, load, 0)
}

/// Expand `records`, only keeping the detail records for the `wanted` source
/// revisions if provided, and skipping ones already `seen`.
fn expand_wanted<R: TimelineRecord>(
    records: Vec<R>,
    wanted: Option<&HashSet<String>>,
    seen: &mut HashSet<String>,
    load: &mut impl FnMut(&JournalVersionRef) -> Result<Vec<R>, String>,
    depth: usize,
) -> Result<Vec<R>, String> {
    if depth > MAX_EXPANSION_DEPTH {
        return Err("summary records nested too deeply (a cycle?)".to_string());
    }
    let mut expanded = vec![];
    for record in records {
        if let Some(summary) = record.summary_ref() {
            let needed: HashSet<String> = summary
                .source_revs
                .iter()
                .filter(|rev| wanted.is_none_or(|wanted| wanted.contains(*rev)))
                .filter(|rev| !seen.contains(*rev))
                .cloned()
                .collect();
            if needed.is_empty() {
                continue;
            }
            let mut found = HashSet::new();
            for pred in &summary.preds {
                let pred_records = load(pred)?;
                let mut pred_seen = seen.clone();
                for detail in
                    expand_wanted(pred_records, Some(&needed), &mut pred_seen, load, depth + 1)?
                {
                    let rev = detail.detail_source_rev().unwrap().to_string();
                    if !found.contains(&rev) && seen.insert(rev.clone()) {
                        found.insert(rev);
                        expanded.push(detail);
                    }
                }
            }
            let mut missing: Vec<&String> = needed.difference(&found).collect();
            if !missing.is_empty() {
                missing.sort();
                return Err(format!(
                    "summary for week {:?} lacks detail records for {:?} in its preds {:?}",
                    summary.iso_week_range, missing, summary.preds
                ));
            }
        } else if let Some(rev) = record.detail_source_rev()
            && wanted.is_none_or(|wanted| wanted.contains(rev))
            && seen.insert(rev.to_string())
        {
            expanded.push(record);
        }
    }
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_common::{
        DetailRecordRef, SummaryRecordRef, TokenDeltaDetails,
    };
    use crate::file_format::history::timeline_tokens::{
        TokenDeltaDetailRecord, TokenDeltaSummaryRecord,
    };

    fn detail(rev: &str, added: u32) -> TokenDeltaRecord {
        TokenDeltaRecord::Detail(TokenDeltaDetailRecord {
            desc: DetailRecordRef {
                source_rev: rev.to_string(),
                syntax_rev: format!("syntax-{}", rev),
                iso_date: format!("2020-01-01T00:00:0{}Z", rev.len()),
                backs_out: vec![],
            },
            delta: TokenDeltaDetails {
                added,
                ..Default::default()
            },
        })
    }

    fn summary(revs: &[&str], preds: &[(Oid, &str)], week: u8) -> TokenDeltaRecord {
        TokenDeltaRecord::Summary(TokenDeltaSummaryRecord {
            desc: SummaryRecordRef {
                source_revs: revs.iter().map(|r| r.to_string()).collect(),
                preds: preds
                    .iter()
                    .map(|(rev, path)| JournalVersionRef {
                        timeline_rev: rev.to_string(),
                        path: path.to_string(),
                    })
                    .collect(),
                iso_week_range: (2020, week, week),
            },
            delta: TokenDeltaDetails::default(),
        })
    }

    /// Commit a journal with the given records at `path` on top of `parent`.
    fn commit_journal(
        repo: &Repository,
        parent: Option<Oid>,
        path: &str,
        records: &[TokenDeltaRecord],
    ) -> Oid {
        let mut contents = "{}".to_string();
        for record in records {
            contents.push('\n');
            contents.push_str(&serde_json::to_string(record).unwrap());
        }
        let blob = repo.blob(contents.as_bytes()).unwrap();
        let mut index = git2::Index::new().unwrap();
        index
            .add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size: contents.len() as u32,
                id: blob,
                flags: path.len() as u16,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            })
            .unwrap();
        let tree = repo.find_tree(index.write_tree_to(repo).unwrap()).unwrap();
        let sig = git2::Signature::new("test", "test@example.com", &git2::Time::new(0, 0)).unwrap();
        let parents: Vec<git2::Commit> = parent
            .into_iter()
            .map(|p| repo.find_commit(p).unwrap())
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(None, &sig, &sig, "journal", &tree, &parent_refs)
            .unwrap()
    }

    fn revs(records: &[TokenDeltaRecord]) -> Vec<String> {
        records
            .iter()
            .map(|r| r.detail_source_rev().unwrap().to_string())
            .collect()
    }

    #[test]
    fn test_expand() {
        let dir = std::env::temp_dir().join(format!("journal-expand-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let path = "tokens/fo/o_/foo.ndjson";
        let old_path = "tokens/ba/r_/bar.ndjson";

        // Week 1 (a, b) summarized in c1, whose parent c0 has the details.
        let c0 = commit_journal(&repo, None, path, &[detail("b", 2), detail("a", 1)]);
        let c1 = commit_journal(
            &repo,
            Some(c0),
            path,
            &[detail("c", 3), summary(&["b", "a"], &[(c0, path)], 1)],
        );
        // Another history of the journal (ex: the other side of a merge, and
        // under another path) with week 1's b and a late d, summarized in c3.
        let c2 = commit_journal(&repo, None, old_path, &[detail("dd", 4), detail("b", 2)]);
        let c3 = commit_journal(
            &repo,
            Some(c2),
            old_path,
            &[summary(&["dd", "b"], &[(c2, old_path)], 1)],
        );
        // A merged summary of week 1 referring to both summaries, plus a newer
        // detail and a detail which is also summarized.
        let reader_records = vec![
            detail("eee", 5),
            summary(&["dd", "b", "a"], &[(c1, path), (c3, old_path)], 1),
            detail("b", 2),
        ];

        let mut reader = JournalReader::new(&repo);
        let expanded = reader.expand(reader_records).unwrap();
        assert_eq!(revs(&expanded), vec!["eee", "b", "a", "dd"]);

        // Expanding a journal without summaries is the identity.
        let plain = reader
            .records::<TokenDeltaRecord>(&JournalVersionRef {
                timeline_rev: c0.to_string(),
                path: path.to_string(),
            })
            .unwrap();
        assert_eq!(revs(&reader.expand(plain).unwrap()), vec!["b", "a"]);

        // A summary whose preds lack some of its revisions is an error.
        let bad = vec![summary(&["b", "zz"], &[(c0, path)], 1)];
        let err = reader.expand(bad).unwrap_err();
        assert!(err.contains("zz"), "{}", err);

        // Missing journals are empty.
        let missing = reader
            .records::<TokenDeltaRecord>(&JournalVersionRef {
                timeline_rev: c0.to_string(),
                path: "tokens/no/pe/nope.ndjson".to_string(),
            })
            .unwrap();
        assert!(missing.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_kind_paths() {
        assert_eq!(
            JournalKind::Future.journal_path("dom/Foo.cpp"),
            "future/dom/Foo.cpp.ndjson"
        );
        assert_eq!(
            JournalKind::Tokens.journal_path("nsresult"),
            "tokens/84/da/nsresult.ndjson"
        );
        assert_eq!(
            JournalKind::for_path("files-delta/dom/Foo.cpp.ndjson"),
            Some(JournalKind::FilesDelta)
        );
        assert_eq!(JournalKind::for_path("annotated/dom/Foo.cpp"), None);
    }
}
