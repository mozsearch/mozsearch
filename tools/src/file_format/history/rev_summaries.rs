//! This file defines the JSON records we write into the (non-git)
//! `history/rev-summaries/by-source-rev/aa/bb/REV.json` path structure where AA
//! and BB are the first 2 pairs of the lowercased git source revision hash that
//! we are summarizing.  See `rev_summary_path`.
//!
//! ## Storage: Not Git!
//!
//! This data is intentionally not stored in git because this
//! makes it easier to go directly from a user-provided revision to all of the
//! metadata we have about the revision without having to have a large in-memory
//! map or add a git on-disk map like git-cinnabar does for hg2git.  This also
//! saves us from having to use git to get a checkout of the revision, etc.  We
//! can also easily compress the files, but git can handle that, it just isn't
//! useful if the files change.  These files are immutable except that
//! `backed_out_by` is updated when a later revision backs this one out.
//!
//! ## File Contents and Relation to File Deltas
//!
//! The revision summary is primarily an aggregation of the individual file
//! deltas.  We only write out a single JSON blob so we only need a record and
//! there's no need for a header.
//!
//! Revisions which change more than `MAX_REV_SUMMARY_FILES` files (ex: the
//! start of a history window, which adds every file, or a tree-wide reformat)
//! only get totals, since listing every file would make the summary huge (ex:
//! 3 GB for the start of an unscoped firefox-main window) while the per-file
//! details are in the files-delta journals anyway.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::timeline_common::{ChangeKind, FileSyntaxDelta, TokenDeltaDetails};

/// The most files a rev-summary lists in `file_deltas`, which is a few MB for
/// big firefox-main revisions.
pub const MAX_REV_SUMMARY_FILES: usize = 1000;

/// The same payload as the `FileDeltaDetailRecord` for the given file.
#[derive(Debug, Serialize, Deserialize)]
pub struct RevFileSummaryRecord {
    #[serde(flatten)]
    pub delta: FileSyntaxDelta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevSummaryRecord {
    /// The source git repo revision we're describing; this should also be our
    /// filename.
    pub source_rev: String,

    /// The corresponding hg revision, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hg_rev: Option<String>,

    /// The corresponding old revisions (ex: gecko-dev revisions for the
    /// firefox-* trees), if any; see `tools::cinnabar`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub old_revs: Vec<String>,

    /// The "syntax" history git repo revision corresponding to this revision.
    pub syntax_rev: String,

    // The "timeline" history git repo corresponding to this revision.  This
    // file is expected to be written immediately after committing the given
    // revision so we can have it available.
    pub timeline_rev: String,

    /// The commit message.
    pub message: String,

    /// The commit/push date (versus the potentially misleading authorship date,
    /// if we have that too).
    pub iso_date: String,

    /// The author of the commit, not yet mail-mapped; this must ALWAYS be
    /// passed through a mail-mapping process before being passed to a display
    /// layer in order to avoid dead-naming people.
    pub unmapped_author: String,

    /// Basically the contents of all the `FileDeltaDetailRecords` for all the
    /// files changed in this revision, keyed by the path of the file in this
    /// revision (or its previous path if it was deleted).  This will be empty
    /// for merge commits, and for revisions which changed more than
    /// `MAX_REV_SUMMARY_FILES` files, which get `file_totals` instead.
    pub file_deltas: BTreeMap<String, RevFileSummaryRecord>,

    /// The totals over the files for revisions which changed too many files to
    /// list in `file_deltas`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_totals: Option<RevFileTotals>,

    /// If this revision is a backout (see `hyperblame::backouts`), the source
    /// revisions it backs out, earliest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backs_out: Vec<String>,

    /// The source revisions which back out this revision, added when they're
    /// processed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backed_out_by: Vec<String>,
}

/// Totals over the files changed by a revision; see `MAX_REV_SUMMARY_FILES`.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RevFileTotals {
    /// The number of files with each kind of change.
    pub files: BTreeMap<ChangeKind, u32>,

    /// The changes to all of the tokens in the files.
    #[serde(default, skip_serializing_if = "TokenDeltaDetails::is_empty")]
    pub token_totals: TokenDeltaDetails,
}

/// The `file_deltas` and `file_totals` of a rev-summary for the given file
/// deltas: the file deltas themselves unless there are too many.
pub fn file_deltas_or_totals(
    file_deltas: BTreeMap<String, RevFileSummaryRecord>,
) -> (
    BTreeMap<String, RevFileSummaryRecord>,
    Option<RevFileTotals>,
) {
    if file_deltas.len() <= MAX_REV_SUMMARY_FILES {
        return (file_deltas, None);
    }
    let mut totals = RevFileTotals::default();
    for file in file_deltas.values() {
        *totals.files.entry(file.delta.change).or_default() += 1;
        for symbol in file.delta.symbol_group.symbol_deltas.values() {
            totals.token_totals.accumulate(&symbol.token_totals);
        }
    }
    (BTreeMap::new(), Some(totals))
}

/// The path of the summary for the given source revision relative to the
/// rev-summaries root.
pub fn rev_summary_path(source_rev: &str) -> std::path::PathBuf {
    let rev = source_rev.to_lowercase();
    let mut path = std::path::PathBuf::from("by-source-rev");
    path.push(&rev[0..2]);
    path.push(&rev[2..4]);
    path.push(format!("{}.json", rev));
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_common::{SymbolSyntaxDelta, SymbolSyntaxDeltaGroup};

    fn file(change: ChangeKind, added: u32) -> RevFileSummaryRecord {
        let mut symbol = SymbolSyntaxDelta::new(ChangeKind::Changed);
        symbol.token_totals.added = added;
        RevFileSummaryRecord {
            delta: FileSyntaxDelta {
                change,
                moved_from: None,
                copied: false,
                symbol_group: SymbolSyntaxDeltaGroup {
                    symbol_deltas: BTreeMap::from([("%".to_string(), symbol)]),
                },
            },
        }
    }

    fn files(n: usize) -> BTreeMap<String, RevFileSummaryRecord> {
        (0..n)
            .map(|i| (format!("f{}", i), file(ChangeKind::Added, 2)))
            .collect()
    }

    #[test]
    fn test_file_totals() {
        let (deltas, totals) = file_deltas_or_totals(files(MAX_REV_SUMMARY_FILES));
        assert_eq!(deltas.len(), MAX_REV_SUMMARY_FILES);
        assert!(totals.is_none());

        let mut many = files(MAX_REV_SUMMARY_FILES);
        many.insert("changed".to_string(), file(ChangeKind::Changed, 3));
        let (deltas, totals) = file_deltas_or_totals(many);
        assert!(deltas.is_empty());
        assert_eq!(
            serde_json::to_string(&totals.unwrap()).unwrap(),
            r#"{"files":{"added":1000,"changed":1},"token_totals":{"added":2003}}"#
        );
    }
}
