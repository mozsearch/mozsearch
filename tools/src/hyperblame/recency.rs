//! Symbols' history digests (`file_format::recency::Recency`): how recently,
//! and how much, each symbol's code changed, which crossref computes as it
//! reads each file's analysis, for `/query/`'s recency facets.
//!
//! The history's files-delta journals record each revision's (or, for older
//! weeks, each week's) changes to a file by lexical context (the syntax token
//! files' contexts, ex: `mozilla::dom::Foo::Bar`), which aren't crossref's
//! semantic symbols, so crossref links them by location: a definition's symbol
//! gets the changes of the context its definition is in (a container's own
//! tokens, like its name, are in it; see `cst_tokenizer`), including those of
//! the contexts nested in it.  The syntax token files don't say where their
//! tokens are, so we find them in the source, in order (they're its text
//! without the whitespace), which also means that the contexts are the
//! history's own, whichever version of the tokenizer made them.

use std::collections::{BTreeMap, HashSet};
use std::ops::Bound;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Weekday};
use git2::Oid;

use crate::file_format::config::{GitData, ThreadLocalRepository, timeline_commit_to_meta};
use crate::file_format::history::syntax_files::{split_token_line, token_file_lines};
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::file_format::history::timeline_common::JournalVersionRef;
use crate::file_format::history::timeline_files_delta::FileDeltaRecord;
use crate::file_format::recency::{FileRecency, Recency};
use crate::hyperblame::journals::{JournalKind, JournalReader};

/// The history contexts of a file's source, as the byte offsets where runs of
/// tokens with the same context start.
pub struct FileContexts {
    runs: Vec<(u32, String)>,
}

impl FileContexts {
    /// Line up a file's syntax token file (`files/PATH` in the syntax repo)
    /// with its source, or None if they don't match (ex: the history is of
    /// another revision of the file).  Text can be between tokens, since not
    /// all of it is tokens (ex: the words of tree-sitter-rust's `//` comments,
    /// whose nodes only have a child for the `//`).
    pub fn align(source: &str, token_file: &str) -> Option<FileContexts> {
        let mut runs: Vec<(u32, String)> = vec![];
        let mut pos = 0;
        for line in token_file_lines(token_file) {
            let token = split_token_line(line);
            let start = pos + source[pos..].find(token.token)?;
            pos = start + token.token.len();
            if runs.last().map(|(_, context)| context.as_str()) != Some(token.context) {
                runs.push((u32::try_from(start).ok()?, token.context.to_string()));
            }
        }
        Some(FileContexts { runs })
    }

    /// The context of the token at (or before) byte `offset`.
    pub fn context_at(&self, offset: u32) -> Option<&str> {
        let run = self.runs.partition_point(|(start, _)| *start <= offset);
        Some(self.runs.get(run.checked_sub(1)?)?.1.as_str())
    }
}

/// Each context's changes (not counting moves) by age, from a file's
/// files-delta records (newest first), as of `indexed` (the indexed revision's
/// date), without backouts or the revisions they back out (which detail
/// records say; weekly summaries can't).
pub fn context_changes(
    records: &[FileDeltaRecord],
    indexed: NaiveDate,
) -> BTreeMap<String, Recency> {
    let mut changes: BTreeMap<String, Recency> = BTreeMap::new();
    let mut backed_out: HashSet<&str> = HashSet::new();
    for record in records {
        let (date, group) = match record {
            FileDeltaRecord::Detail(detail) => {
                if !detail.desc.backs_out.is_empty() {
                    backed_out.extend(detail.desc.backs_out.iter().map(String::as_str));
                    continue;
                }
                if backed_out.contains(detail.desc.source_rev.as_str()) {
                    continue;
                }
                let date = detail
                    .desc
                    .iso_date
                    .get(..10)
                    .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
                (date, &detail.delta.symbol_group)
            }
            FileDeltaRecord::Summary(summary) => {
                let (year, newest_week, _) = summary.desc.iso_week_range;
                let date =
                    NaiveDate::from_isoywd_opt(year as i32, newest_week as u32, Weekday::Thu);
                (date, &summary.symbol_group)
            }
        };
        let Some(date) = date else {
            continue;
        };
        let bin = Recency::bin_for_age_days((indexed - date).num_days());
        for (context, delta) in &group.symbol_deltas {
            let totals = &delta.token_totals;
            let tokens = totals.added + totals.removed + totals.evolved_from + totals.evolved_into;
            if tokens > 0 {
                changes.entry(context.clone()).or_default().add(bin, tokens);
            }
        }
    }
    changes
}

/// A context's changes, including those of the contexts nested in it.
pub fn nested_changes(changes: &BTreeMap<String, Recency>, context: &str) -> Recency {
    let mut recency = Recency::default();
    for (key, key_changes) in changes.range::<str, _>((Bound::Included(context), Bound::Unbounded))
    {
        let Some(rest) = key.strip_prefix(context) else {
            break;
        };
        if rest.is_empty() || rest.starts_with("::") {
            recency.accumulate(key_changes);
        }
    }
    recency
}

/// A tree's history as of its indexed (checked out) revision.
pub struct HistoryDigests<'a> {
    syntax: &'a ThreadLocalRepository,
    timeline: &'a ThreadLocalRepository,
    syntax_tree: Oid,
    timeline_rev: Oid,
    indexed: NaiveDate,
}

impl<'a> HistoryDigests<'a> {
    /// The history of the tree's indexed revision, if it has one.
    pub fn open(git: &'a GitData) -> Option<HistoryDigests<'a>> {
        let history = git.history.as_ref()?;
        let head = git.repo.head().ok()?.peel_to_commit().ok()?;
        let timeline_commit = history.timeline_commit(head.id())?;
        let meta = timeline_commit_to_meta(&timeline_commit);
        let syntax_tree = history.syntax.find_commit(meta.syntax_rev).ok()?.tree_id();
        Some(HistoryDigests {
            syntax: &history.syntax,
            timeline: &history.timeline,
            syntax_tree,
            timeline_rev: timeline_commit.id(),
            indexed: DateTime::from_timestamp(head.time().seconds(), 0)?.date_naive(),
        })
    }

    /// Whether the history has (the tokens of) a file.
    pub fn has_file(&self, path: &str) -> bool {
        self.syntax
            .find_tree(self.syntax_tree)
            .is_ok_and(|tree| tree.get_path(Path::new(&format!("files/{}", path))).is_ok())
    }

    /// The digests of a file's contexts, if the history has the file and it
    /// lines up with `source`.
    pub fn file(&self, path: &str, source: &str) -> Option<FileDigests> {
        let syntax = &**self.syntax;
        let tree = syntax.find_tree(self.syntax_tree).ok()?;
        let read = |path: &str| -> Option<String> {
            let entry = tree.get_path(Path::new(path)).ok()?;
            let blob = syntax.find_blob(entry.id()).ok()?;
            String::from_utf8(blob.content().to_vec()).ok()
        };
        let contexts = FileContexts::align(source, &read(&format!("files/{}", path))?)?;
        let namespaces = read(&format!("files-struct/{}", path))
            .unwrap_or_default()
            .lines()
            .skip(1)
            .filter_map(|line| serde_json::from_str::<FileStructureRow>(line).ok())
            .filter(|row| row.kind == "namespace")
            .map(|row| row.pretty)
            .collect();
        Some(FileDigests {
            contexts,
            namespaces,
            changes: context_changes(&self.records(path)?, self.indexed),
        })
    }

    /// The whole-file digest of a file without analysis (ex: docs), whose
    /// lines can't have digests of their own (see `FileRecency`), if the
    /// history has changes to it.
    pub fn unanalyzed_file(&self, path: &str) -> Option<FileRecency> {
        let mut file = Recency::default();
        for changes in context_changes(&self.records(path)?, self.indexed).values() {
            file.accumulate(changes);
        }
        (!file.is_empty()).then(|| FileRecency {
            file,
            scopes: BTreeMap::new(),
        })
    }

    /// The records of a file's files-delta journal (none if it doesn't have
    /// one).
    fn records(&self, path: &str) -> Option<Vec<FileDeltaRecord>> {
        JournalReader::new(self.timeline)
            .records::<FileDeltaRecord>(&JournalVersionRef {
                timeline_rev: self.timeline_rev.to_string(),
                path: JournalKind::FilesDelta.journal_path(path),
            })
            .ok()
    }
}

/// A file's history contexts, with their changes.
pub struct FileDigests {
    contexts: FileContexts,
    namespaces: HashSet<String>,
    changes: BTreeMap<String, Recency>,
}

impl FileDigests {
    /// The file's digests for what symbols' don't cover (see `FileRecency`).
    pub fn file_recency(&self) -> FileRecency {
        let mut recency = FileRecency::default();
        for (context, changes) in &self.changes {
            recency.file.accumulate(changes);
            if context == "%" || self.namespaces.contains(context) {
                recency.scopes.insert(context.clone(), *changes);
            }
        }
        recency
    }

    /// The changes of the context at byte `offset` of the source, with those of
    /// the contexts nested in it, unless it's a namespace or the file's top
    /// level (which are too broad), or it had none.
    pub fn recency_at(&self, offset: u32) -> Option<Recency> {
        let context = self.contexts.context_at(offset)?;
        if context == "%" || self.namespaces.contains(context) {
            return None;
        }
        let recency = nested_changes(&self.changes, context);
        (!recency.is_empty()).then_some(recency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align() {
        let source = "namespace a {\nvoid f() {\n  g(\"x y\");\n}\n}  // a\n";
        let token_file = "a k namespace\na i a\na o {\na::f i void\na::f i f\na::f o (\na::f o )\na::f o {\na::f i g\na::f o (\na::f s \"x\na::f s y\"\na::f o )\na::f o ;\na::f o }\na o }\na c //\na c a\n";
        let contexts = FileContexts::align(source, token_file).unwrap();
        let at = |needle: &str| contexts.context_at(source.find(needle).unwrap() as u32);
        assert_eq!(at("namespace"), Some("a"));
        assert_eq!(at("void"), Some("a::f"));
        assert_eq!(at("y\""), Some("a::f"));
        assert_eq!(at("}\n}"), Some("a::f"));
        assert_eq!(at("// a"), Some("a"));
        // Another revision's tokens don't line up.
        assert!(FileContexts::align("int main() {}\n", token_file).is_none());
        // Untokenized text (ex: tree-sitter-rust's `//` comments' words) is
        // skipped.
        let contexts = FileContexts::align(
            "// one two\nfn f() {}\n",
            "% c //\nf k fn\nf i f\nf o (\nf o )\nf o {\nf o }\n",
        )
        .unwrap();
        assert_eq!(contexts.context_at(12), Some("f"));
    }

    #[test]
    fn test_changes() {
        let record = |line: &str| serde_json::from_str::<FileDeltaRecord>(line).unwrap();
        let detail = |rev: &str, date: &str, backs_out: &str, deltas: &str| {
            record(&format!(
                r#"{{"type":"Detail","source_rev":"{}","syntax_rev":"s{}","iso_date":"{}","backs_out":[{}],"change":"changed","symbol_deltas":{}}}"#,
                rev, rev, date, backs_out, deltas
            ))
        };
        let changed = |tokens: u32| {
            format!(
                r#"{{"change":"changed","token_totals":{{"added":{},"moved":7}},"token_changes":{{}}}}"#,
                tokens
            )
        };
        let records = vec![
            // A backout of r2, newest first.
            detail(
                "r3",
                "2026-10-01T00:00:00Z",
                r#""r2""#,
                &format!(r#"{{"a::f":{}}}"#, changed(100)),
            ),
            detail(
                "r2",
                "2026-09-30T00:00:00Z",
                "",
                &format!(r#"{{"a::f":{}}}"#, changed(100)),
            ),
            detail(
                "r1",
                "2026-09-20T00:00:00Z",
                "",
                &format!(
                    r#"{{"a::f":{},"a::f::g":{},"a::fg":{}}}"#,
                    changed(3),
                    changed(4),
                    changed(5)
                ),
            ),
            record(&format!(
                r#"{{"type":"Summary","source_revs":["r0"],"preds":[],"iso_week_range":[2024,10,10],"symbol_deltas":{{"a::f":{}}}}}"#,
                changed(2)
            )),
        ];
        let changes = context_changes(&records, NaiveDate::from_ymd_opt(2026, 10, 2).unwrap());
        // r1 is 12 days old (1-2 weeks), and the summary's week ~2.5 years.
        assert_eq!(changes["a::f"], Recency([0, 3, 0, 0, 0, 0, 0, 0, 2, 0]));
        // With the nested `a::f::g` (but not `a::fg`).
        assert_eq!(
            nested_changes(&changes, "a::f"),
            Recency([0, 7, 0, 0, 0, 0, 0, 0, 2, 0])
        );
        assert_eq!(nested_changes(&changes, "b"), Recency::default());

        // The file's digests: all of it, and its top level's and namespaces'.
        let digests = FileDigests {
            contexts: FileContexts { runs: vec![] },
            namespaces: ["a".to_string()].into_iter().collect(),
            changes: [
                ("%".to_string(), Recency([1, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
                ("a".to_string(), Recency([0, 2, 0, 0, 0, 0, 0, 0, 0, 0])),
                ("a::f".to_string(), Recency([4, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
            ]
            .into_iter()
            .collect(),
        };
        let file = digests.file_recency();
        assert_eq!(file.file, Recency([5, 2, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(file.scopes.keys().collect::<Vec<_>>(), vec!["%", "a"]);
    }
}
