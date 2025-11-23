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

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenDetailRecord {}

/// Aggregated statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct TokenSummaryRecord {}
