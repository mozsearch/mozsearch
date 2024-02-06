//! This file defines the ND-JSON records we write into files under
//! `history/timeline/future`.  Files and records are organized under this root
//! on a "physical" rather than "logical" basis.  This means that these files
//! are never moved to follow a copied/renamed/moved file and they are never
//! deleted when a file is deleted.
//!
//! This enables us to enable functionality like "take me to where this token is
//! now or tell me when it was deleted / moved".  It also enables us to address
//! people following links to old/deleted files.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::timeline_common::{SummaryRecordRef, DetailRecordRef};

#[derive(Debug, Serialize, Deserialize)]
pub struct FutureHeader {

}

#[derive(Debug, Serialize, Deserialize)]
pub struct FutureFileChanges {
    /// Was the file deleted in this ref?
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
    pub file_moved_to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FutureTokenChanges {

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
///
#[derive(Debug, Serialize, Deserialize)]
pub struct FutureDetailRecord {
    #[serde(flatten)]
    pub desc: DetailRecordRef,

    #[serde(flatten)]
    pub file_changes: FutureFileChanges,

    /// Map tracking extinguished tokens where the keys are the source revision
    /// corresponding to the "introduced" ref for the token and the values are
    /// all of the token line indices ("lineno") for those tokens.
    ///
    /// This enables us to efficiently find the commit that removed a token
    /// without moving it anywhere else or having it evolve into another token
    /// because we can scan the future file
    ///
    /// TODO: Try and encode the token indices in a more compressed rep, like
    /// how IMAP UIDs work.  But for now let's stick with this.
    pub extinguished_tokens: BTreeMap<String, BTreeSet<u32>>,

    /// Same rep as extinguished_tokens, but for tokens moved to other files.
    ///
    /// The current assumption is that we will consult the other data for the
    /// ref'ed revision to figure out where they went, but this could be
    /// enhanced to indicate where the tokens went if helpful
    pub moved_out_tokens: BTreeMap<String, BTreeSet<u32>>,

    /// Same rep as extinguished_tokens, but for tokens moved into this file.
    pub moved_in_tokens: BTreeMap<String, BTreeSet<u32>>,

    /// Same rep as extinguished_tokens for token refs that have moved from
    /// "introduced" to "predecessor".  For tokens that have both moved and
    /// evolved, there will be an entry here in the file where the token is
    /// treated as "moved-in".  (For processing back-outs, the moved-out
    /// tracking is sufficient for us to know the line to re-add in the source
    /// file, and it's only in the moved-in file that we need to know about
    /// the evolution for the corresponding removal.)
    pub evolved_tokens: BTreeSet<u32>,

    /// Token indices for newly added tokens in this source revision.
    pub added_tokens: BTreeSet<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FutureSummaryRecord {
    #[serde(flatten)]
    pub desc: SummaryRecordRef,

    #[serde(flatten)]
    pub file_changes: FutureFileChanges,

    /// The union of all of the removed_tokens' revision keys over all the detail
    /// records digested into this summary.
    pub removed_token_revs: BTreeSet<String>,
    /// The union of all of the moved_tokens' revision keys over all the detail
    /// records digested into this summary.
    pub moved_token_revs: BTreeSet<String>,
}

/// Internally tagged enum for our detail and summary types.  This ends up
/// serializing as `{"type": "Detail" , ...}` or `{"type": "Summary", ...}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FutureRecord {
    Detail(FutureDetailRecord),
    Summary(FutureSummaryRecord),
}
