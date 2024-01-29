//! This file defines the ND-JSON records we write into files under
//! `history/timeline/tokens/ab/cd/` where "ab" and "cd" are pairs of characters
//! from the (lowercased) prefix of the token to help keep the file-system, or
//! at least directory listings, sane.
//!
//! The files are intended to support UX functionality along the lines of:
//! - `git log -S` by helping make it clear when there are net changes in the
//!   presence of certain tokens which indicates that logic isn't just being
//!   reformatted or moved around.
//! - Letting the user know if what they searched for is no longer in the tree,
//!   but when it was last in the tree and potentially identifying the likely
//!   multiple patches involved in the term being removed.
//! - General interest graphs of net changes in use of the token over time,
//!   aggregated by week.
//!
//! These files are intended to primarily serve as the basis for histograms and
//! serve as a light-weight cross-reference to commits which include the tokens,
//! so we store relatively little information about changes here.  Instead, the
//! assumption is that any queries will use the commit references from this
//! file to look up the rev-summaries for the commit which has an aggregation
//! of the changes.  This should also allow queries that involve multiple tokens
//! to efficiently perform filtering by intersecting commit sets before moving
//! on to look up the commits.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenDeltaDetailRecord {
    #[serde(flatten)]
    pub desc: DetailRecordRef,

    #[serde(flatten)]
    pub delta: TokenDeltaDetails,
}

/// Aggregated statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct TokenDeltaSummaryRecord {
    #[serde(flatten)]
    pub desc: SummaryRecordRef,

    #[serde(flatten)]
    pub delta: TokenDeltaDetails,
}

/// Internally tagged enum for our detail and summary types.  This ends up
/// serializing as `{"type": "Detail" , ...}` or `{"type": "Summary", ...}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TokenDeltaRecord {
    Detail(TokenDeltaDetailRecord),
    Summary(TokenDeltaSummaryRecord),
}
