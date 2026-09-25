//! This file defines the ND-JSON records we write into files under
//! `history/timeline/future`.  Files and records are organized under this root
//! on a "physical" rather than "logical" basis.  This means that these files
//! are never moved to follow a copied/renamed/moved file and they are never
//! deleted when a file is deleted.
//!
//! This enables us to enable functionality like "take me to where this token is
//! now or tell me when it was deleted / moved".  It also enables us to address
//! people following links to old/deleted files.
//!
//! The first line of each file is a `FutureHeader` and the remaining lines are
//! `FutureRecord`s ordered from newest to oldest.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::timeline_common::{
    DetailRecordRef, SummaryRecordRef, TimelineRecord, TokenLinenoSet, TokenRefSet,
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FutureHeader {}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FutureFileChanges {
    /// Was the file deleted in this ref?
    #[serde(default, skip_serializing_if = "is_false")]
    pub file_deleted: bool,

    /// If the file was moved in this ref, what path was it moved to?
    ///
    /// We currently don't try and do anything with copies.  In the summary
    /// record, the most recent move wins because it seems like a weird edge
    /// case for a file to move a bunch in a week/whenever, although backouts
    /// definitely seem like there's a good chance of creating an interesting /
    /// weird situation here.  (In particular, we'd expect any backed out file
    /// to end up with both sides of the move pointing at each other, which is
    /// not really helpful, but that's backouts for you.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_moved_to: Option<String>,

    /// If the file was created in this ref as the result of a move/rename or a
    /// copy, what path did it come from?  This is primarily for symmetry with
    /// `file_moved_to` so that the physical history can be followed backwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_moved_from: Option<String>,

    /// True if `file_moved_from` was a copy rather than a move.
    #[serde(default, skip_serializing_if = "is_false")]
    pub file_copied: bool,
}

/// Details changes from a specific revision for the source file containing this
/// record.
///
/// These records have 2 primary use-cases:
///
/// 1. Efficiently following a token into the future without having to compute
///    any new diffs.  Git stores snapshots of files, so any time we want a diff
///    we need to check out both snapshots and diff them.  This has a cost that
///    is nice to avoid, but more significantly, if we make sure to log the
///    exact decisions we make about token identity, we don't have to worry
///    about correctly and consistently re-inferring our previous decisions.
///    This is nice for my sanity and because inherently a lot of what we're
///    doing or hope to do relies on potentially arbitrary heuristics to map
///    moves and only having to encode those once and not needing to be as
///    concerned about their realtime efficiency is a win.
/// 2. Processing back-outs.  When we process a back-out we want to try and
///    restore the previous state to be identical to its state prior to the
///    landing of the backout, but compensating for any manual corrections that
///    might have happened during the backout.  (They should be rare, but they
///    can happen.)
///
/// All of the token sets use the `TokenRefSet` representation which maps from
/// the source revision of the token's canonical "introduced" `HyperTokenRef` to
/// its path (with "%" meaning the path of this file) to the set of token line
/// numbers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FutureDetailRecord {
    #[serde(flatten)]
    pub desc: DetailRecordRef,

    #[serde(flatten)]
    pub file_changes: FutureFileChanges,

    /// Tracks tokens which were removed without moving anywhere else or having
    /// them evolve into another token.
    ///
    /// This enables us to efficiently find the commit that removed a token
    /// because we can scan the future file.
    #[serde(default, skip_serializing_if = "TokenRefSet::is_empty")]
    pub extinguished_tokens: TokenRefSet,

    /// Same rep as extinguished_tokens, but for tokens moved to other files.
    ///
    /// The current assumption is that we will consult the other data for the
    /// ref'ed revision to figure out where they went, but this could be
    /// enhanced to indicate where the tokens went if helpful
    #[serde(default, skip_serializing_if = "TokenRefSet::is_empty")]
    pub moved_out_tokens: TokenRefSet,

    /// Same rep as extinguished_tokens, but for tokens moved into this file.
    #[serde(default, skip_serializing_if = "TokenRefSet::is_empty")]
    pub moved_in_tokens: TokenRefSet,

    /// Same rep as extinguished_tokens for token refs that have moved from
    /// "introduced" to "predecessor".  That is, the tokens (as they existed in
    /// this file) that evolved into other tokens.  For tokens that have both
    /// moved and evolved, there will be an entry here in the file the token
    /// was originally in.  (For processing back-outs, this lets us know the
    /// line to re-add in the source file, and the newly introduced token will
    /// show up in `added_tokens` in the file it evolved into.)
    #[serde(default, skip_serializing_if = "TokenRefSet::is_empty")]
    pub evolved_tokens: TokenRefSet,

    /// Token indices for newly introduced tokens in this source revision,
    /// including tokens which are the result of an evolution.  Because all such
    /// tokens will have a canonical ref of this source revision and this path,
    /// we only need the line numbers.
    #[serde(default, skip_serializing_if = "TokenLinenoSet::is_empty")]
    pub added_tokens: TokenLinenoSet,
}

impl FutureDetailRecord {
    pub fn new(desc: DetailRecordRef) -> Self {
        FutureDetailRecord {
            desc,
            file_changes: FutureFileChanges::default(),
            extinguished_tokens: TokenRefSet::new(),
            moved_out_tokens: TokenRefSet::new(),
            moved_in_tokens: TokenRefSet::new(),
            evolved_tokens: TokenRefSet::new(),
            added_tokens: TokenLinenoSet::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.file_changes.file_deleted
            && self.file_changes.file_moved_to.is_none()
            && self.file_changes.file_moved_from.is_none()
            && self.extinguished_tokens.is_empty()
            && self.moved_out_tokens.is_empty()
            && self.moved_in_tokens.is_empty()
            && self.evolved_tokens.is_empty()
            && self.added_tokens.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FutureSummaryRecord {
    #[serde(flatten)]
    pub desc: SummaryRecordRef,

    #[serde(flatten)]
    pub file_changes: FutureFileChanges,

    /// The union of all of the extinguished_tokens' revision keys over all the
    /// detail records digested into this summary.
    pub removed_token_revs: BTreeSet<String>,
    /// The union of all of the moved_tokens' revision keys over all the detail
    /// records digested into this summary.
    pub moved_token_revs: BTreeSet<String>,
}

/// Internally tagged enum for our detail and summary types.  This ends up
/// serializing as `{"type": "Detail" , ...}` or `{"type": "Summary", ...}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FutureRecord {
    Detail(FutureDetailRecord),
    Summary(FutureSummaryRecord),
}

impl TimelineRecord for FutureRecord {
    fn dedupe_key(&self) -> String {
        match self {
            FutureRecord::Detail(d) => format!("D{}", d.desc.source_rev),
            FutureRecord::Summary(s) => format!("S{:?}", s.desc.iso_week_range),
        }
    }

    fn iso_date(&self) -> Option<&str> {
        match self {
            FutureRecord::Detail(d) => Some(&d.desc.iso_date),
            FutureRecord::Summary(_) => None,
        }
    }
}
