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
//! useful if the files change.  Note that we do expect these files to be
//! immutable except potentially in the face of backouts when we would probably
//! update the files.
//!
//! ## File Contents and Relation to File Deltas
//!
//! The revision summary is primarily an aggregation of the individual file
//! deltas.  We only write out a single JSON blob so we only need a record and
//! there's no need for a header.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::timeline_common::FileSyntaxDelta;

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
    /// for merge commits.
    pub file_deltas: BTreeMap<String, RevFileSummaryRecord>,
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
