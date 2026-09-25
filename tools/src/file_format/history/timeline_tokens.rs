//! This file defines the ND-JSON records we write into files under
//! `history/timeline/tokens/ab/cd/` where "ab" and "cd" are pairs of characters
//! from the (lowercased) prefix of the token to help keep the file-system, or
//! at least directory listings, sane.  See `token_timeline_path`.
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
//!
//! Only identifier-like tokens (see `is_trackable_token`) get files, and we only
//! emit a record for a revision if the token was added/removed/evolved; pure
//! moves are not recorded here because they are noise for these use-cases, but
//! they can be found in the rev-summaries and files-delta records.
//!
//! The first line of each file is a `TokenHeader` and the remaining lines are
//! `TokenDeltaRecord`s ordered from newest to oldest.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::timeline_common::{
    DetailRecordRef, SummaryRecordRef, TimelineRecord, TokenDeltaDetails,
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TokenHeader {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenDeltaDetailRecord {
    #[serde(flatten)]
    pub desc: DetailRecordRef,

    #[serde(flatten)]
    pub delta: TokenDeltaDetails,
}

/// Aggregated statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenDeltaSummaryRecord {
    #[serde(flatten)]
    pub desc: SummaryRecordRef,

    #[serde(flatten)]
    pub delta: TokenDeltaDetails,
}

/// Internally tagged enum for our detail and summary types.  This ends up
/// serializing as `{"type": "Detail" , ...}` or `{"type": "Summary", ...}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TokenDeltaRecord {
    Detail(TokenDeltaDetailRecord),
    Summary(TokenDeltaSummaryRecord),
}

impl TimelineRecord for TokenDeltaRecord {
    fn dedupe_key(&self) -> String {
        match self {
            TokenDeltaRecord::Detail(d) => format!("D{}", d.desc.source_rev),
            TokenDeltaRecord::Summary(s) => format!("S{:?}", s.desc.iso_week_range),
        }
    }

    fn iso_date(&self) -> Option<&str> {
        match self {
            TokenDeltaRecord::Detail(d) => Some(&d.desc.iso_date),
            TokenDeltaRecord::Summary(_) => None,
        }
    }
}

/// Is this token interesting enough to be tracked in the per-token timeline
/// and itemized in per-symbol `token_changes`?  Currently we require the token
/// look like an identifier: at least 2 characters long, made up of alphanumeric
/// characters, "_", and "$", not starting with a digit, and with at least one
/// alphabetic character.
///
/// Note that we will end up including words from comments since the tokenizer
/// does not currently distinguish them, but this is arguably a feature.
pub fn is_trackable_token(token: &str) -> bool {
    token.len() >= 2
        && token.len() <= 128
        && !token.starts_with(|c: char| c.is_ascii_digit())
        && token
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && token.chars().any(|c| c.is_alphabetic())
}

/// Derive the path of the token's timeline file relative to the timeline root,
/// ex: "nsresult" => "tokens/ns/re/nsresult.ndjson".  Tokens shorter than 4
/// characters are padded with "_" for directory naming purposes.
pub fn token_timeline_path(token: &str) -> PathBuf {
    let mut prefix: Vec<char> = token.to_lowercase().chars().take(4).collect();
    while prefix.len() < 4 {
        prefix.push('_');
    }
    let mut path = PathBuf::from("tokens");
    path.push(prefix[0..2].iter().collect::<String>());
    path.push(prefix[2..4].iter().collect::<String>());
    path.push(format!("{}.ndjson", token));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trackable() {
        assert!(is_trackable_token("nsresult"));
        assert!(is_trackable_token("rv"));
        assert!(is_trackable_token("$foo"));
        assert!(is_trackable_token("mFoo2"));
        assert!(!is_trackable_token("x"));
        assert!(!is_trackable_token("42"));
        assert!(!is_trackable_token("0x10"));
        assert!(!is_trackable_token("::"));
        assert!(!is_trackable_token("\"hello\""));
    }

    #[test]
    fn test_paths() {
        assert_eq!(
            token_timeline_path("nsresult"),
            PathBuf::from("tokens/ns/re/nsresult.ndjson")
        );
        assert_eq!(
            token_timeline_path("rv"),
            PathBuf::from("tokens/rv/__/rv.ndjson")
        );
        assert_eq!(
            token_timeline_path("RefPtr"),
            PathBuf::from("tokens/re/fp/RefPtr.ndjson")
        );
    }
}
