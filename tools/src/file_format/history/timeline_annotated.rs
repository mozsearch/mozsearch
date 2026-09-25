//! This file defines the ND-JSON records we write into files under
//! `history/timeline/annotated`.  These are the token-centric analog to the
//! classic line-centric blame files.
//!
//! ## File Layout
//!
//! Every line in an annotated file is a `HyperLineData` JSON record.
//! - Line index 0 (the first line) is a synthetic "sentinel" token which
//!   represents the file itself.  Its `introduced` ref has a `lineno` of 0 and
//!   identifies the revision in which the file was created, or most recently
//!   renamed/copied, in which case its `predecessor` identifies the sentinel of
//!   the file it was renamed/copied from.  The sentinel also hosts the
//!   `removal_marker` for tokens removed from the very start of the file.
//! - Line index N for N >= 1 corresponds to the token on (1-based) line N of the
//!   corresponding `history/syntax/files` token-per-line file.
//!
//! This means that the (0-based) line index of an annotated file is exactly the
//! (1-based) token "lineno" used in `HyperTokenRef`s.
//!
//! ## Paths
//!
//! As with classic blame, a path of "%" means the path of the file containing
//! the record.  This saves space and means that files which are renamed without
//! modification can have their annotated files propagated unchanged.  When
//! records are copied into a file with a different path (due to a rename or
//! tokens being moved between files), any "%" paths get replaced with the path
//! the record is being copied from.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_with::{BorrowCow, serde_as};

/// Path value which means "the path of the file containing this record".
pub const PATH_UNCHANGED: &str = "%";

/// Identifies a specific token in space and time.  For any given token in blame
/// we potentially have a few tokens that we're referencing, so they each get
/// their own reference.
///
/// These refs can be thought of in 2 ways:
/// 1. Canonical.  A ref that when resolved will identify a payload equivalent
///    to itself.  That is, if a token is introduced in source revision A, then
///    the canonical hyper token ref would involve source revision A.
/// 2. Unresolved / non-canonical.  If source revision A has a child revision B
///    we could conceptually create a new hyper token ref that has the source
///    revision A replaced by B.  This still tells us how to find a token, but
///    it's not a useful token ref because all of our data representations are
///    based on canonical hyper token refs.  We would need to look up the line
///    in the "history/annotated" in the corresponding history revision for B
///    in order to load the canonical hyper token ref that we can use.
///
/// Note that our blame UI potentially surfaces the following metadata that we
/// do not include here because it can be looked up from the source revision:
/// - Author
/// - Timestamp of the commit
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct HyperTokenRef<'a> {
    /// The source (not syntax or timeline) revision.
    #[serde(rename = "sr", borrow)]
    #[serde_as(as = "BorrowCow")]
    pub source_rev: Cow<'a, str>,

    /// The path of the file in the source revision, or "%" if this is the same
    /// path as the file containing this record.
    #[serde(rename = "p", borrow)]
    #[serde_as(as = "BorrowCow")]
    pub path: Cow<'a, str>,

    /// The 1-based line number of the token in the `history/syntax/files`
    /// representation of the file at `path` in `source_rev`, or 0 for the
    /// synthetic file sentinel token.
    #[serde(rename = "l")]
    pub lineno: u32,
}

impl<'a> HyperTokenRef<'a> {
    pub fn new_unchanged_path(source_rev: &'a str, lineno: u32) -> Self {
        HyperTokenRef {
            source_rev: Cow::Borrowed(source_rev),
            path: Cow::Borrowed(PATH_UNCHANGED),
            lineno,
        }
    }

    pub fn is_path_unchanged(&self) -> bool {
        self.path == PATH_UNCHANGED
    }

    /// Replace a "%" path with the provided path, which should be the path of
    /// the file this record was read from.
    pub fn resolve_path(&mut self, from_path: &'a str) {
        if self.is_path_unchanged() {
            self.path = Cow::Borrowed(from_path);
        }
    }

    /// Re-express the path relative to a file at `into_path`, which means
    /// replacing an explicit path equal to `into_path` with "%".  The
    /// counterpart to `resolve_path`.
    pub fn relativize_path(&mut self, into_path: &str) {
        if self.path == into_path {
            self.path = Cow::Borrowed(PATH_UNCHANGED);
        }
    }

    pub fn into_owned(self) -> HyperTokenRef<'static> {
        HyperTokenRef {
            source_rev: Cow::Owned(self.source_rev.into_owned()),
            path: Cow::Owned(self.path.into_owned()),
            lineno: self.lineno,
        }
    }
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Right now the use case is just for the blame sidebar where just saying "100
/// tokens were removed here in rev FOO" with our current blame idiom of "go to
/// the removal rev at the point of removal" and "go to the start of the run of
/// removed tokens in the removal rev's parent".
///
/// Identifies the revision in which a run of tokens were deleted without any
/// corresponding additions, the first of the run of tokens that was removed
/// (by ref), and the number of tokens that were removed.
///
/// This means that we can load the parent revision of the annotated file for
/// the path, locate the line with the `introduced` for the given ref, and then
/// count that many tokens and we will have located the exact run of tokens that
/// were removed without having to generate a diff.
///
/// Markers are always hosted by the token which precedes the removed run in the
/// new revision (or the file sentinel if the removal was at the start of the
/// file).  If the hosting token already had a marker, the new marker replaces
/// it.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RemovalMarker<'a> {
    /// The source revision in which the removal happened.
    #[serde(rename = "sr", borrow)]
    #[serde_as(as = "BorrowCow")]
    pub source_rev: Cow<'a, str>,

    /// The path of the file in the parent of `source_rev`, or "%" if it's the
    /// same as the file containing this record.
    // XXX timestamp of removal commit?
    #[serde(rename = "p", borrow)]
    #[serde_as(as = "BorrowCow")]
    pub path: Cow<'a, str>,

    /// The (1-based) line number of the first removed token in the syntax file
    /// in the parent of `source_rev`.
    #[serde(rename = "l")]
    pub lineno: u32,

    /// The canonical ref of the first removed token, which we could also find
    /// by looking at `lineno` in the parent rev's annotated file.
    #[serde(rename = "fr", borrow)]
    pub first_removed: HyperTokenRef<'a>,

    /// The number of tokens removed in the run.
    #[serde(rename = "nr")]
    pub num_removed: u32,

    /// How many of the `num_removed` tokens we believe were moved elsewhere
    /// (in this file or another file) rather than being extinguished.  The
    /// "history/future" file for this path can be consulted for specifics.
    #[serde(rename = "nm", default, skip_serializing_if = "is_zero")]
    pub num_moved: u32,
}

impl<'a> RemovalMarker<'a> {
    pub fn resolve_path(&mut self, from_path: &'a str) {
        if self.path == PATH_UNCHANGED {
            self.path = Cow::Borrowed(from_path);
        }
        self.first_removed.resolve_path(from_path);
    }

    pub fn relativize_path(&mut self, into_path: &str) {
        if self.path == into_path {
            self.path = Cow::Borrowed(PATH_UNCHANGED);
        }
        self.first_removed.relativize_path(into_path);
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct HyperLineData<'a> {
    /// When was this token with its current string value introduced?
    #[serde(rename = "i", borrow)]
    pub introduced: HyperTokenRef<'a>,

    /// If this token evolved from another token, the predecessor token ref.  So
    /// if we have a token like "OkayType" which became "BetterType", the
    /// `introduced` ref above is when the "BetterType" token was introduced,
    /// and this ref is to when "OkayType" was introduced.
    ///
    /// It's possible for evolutions to involve multiple tokens, like
    /// "namespace::OkayType" (3 tokens) becoming "BetterType" (1 token) or vice
    /// versa.  We always reference the sequentially earliest token in a run of
    /// tokens.  In the case of evolving from 1 token to 3, all 3 would
    /// reference the 1 token.  But that is future work.
    ///
    /// This predecessor relationship lets us follow the history of a token back
    /// in time by loading the "history/annotated" files for the named path and
    /// revision.  Although related information will also be encoded in the
    /// "history/future" file, it's not necessary for us to consult it unless
    /// we want to pay additional attention to when the token moves between
    /// files.
    ///
    /// Only a single predecessor is tracked; if a token evolves again, the
    /// predecessor is replaced by the `introduced` of the token it evolved
    /// from, and the predecessor's predecessor can be found by following the
    /// chain.
    #[serde(rename = "p", borrow, default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<HyperTokenRef<'a>>,

    /// We track runs of removed tokens on the preceding token so that we can
    /// render a visual indicator in the blame sidebar for removals.
    #[serde(
        rename = "rm",
        borrow,
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub removal_marker: Option<RemovalMarker<'a>>,
}

impl<'a> HyperLineData<'a> {
    pub fn new_introduced(source_rev: &'a str, lineno: u32) -> Self {
        HyperLineData {
            introduced: HyperTokenRef::new_unchanged_path(source_rev, lineno),
            predecessor: None,
            removal_marker: None,
        }
    }

    pub fn parse(line: &'a str) -> Self {
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("Bad annotated line {:?}: {}", line, e))
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string(self).unwrap()
    }

    /// Replace all "%" paths with `from_path`; see the module docs.
    pub fn resolve_path(&mut self, from_path: &'a str) {
        self.introduced.resolve_path(from_path);
        if let Some(pred) = self.predecessor.as_mut() {
            pred.resolve_path(from_path);
        }
        if let Some(marker) = self.removal_marker.as_mut() {
            marker.resolve_path(from_path);
        }
    }

    /// Replace all paths that equal `into_path` with "%"; see the module docs.
    pub fn relativize_path(&mut self, into_path: &str) {
        self.introduced.relativize_path(into_path);
        if let Some(pred) = self.predecessor.as_mut() {
            pred.relativize_path(into_path);
        }
        if let Some(marker) = self.removal_marker.as_mut() {
            marker.relativize_path(into_path);
        }
    }

    /// Transplant the record from a file at `from_path` into a file at
    /// `into_path`, fixing up paths as needed.  This is a no-op if the paths
    /// are the same.
    pub fn transplant(&mut self, from_path: &'a str, into_path: &str) {
        if from_path != into_path {
            self.resolve_path(from_path);
            self.relativize_path(into_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_and_transplant() {
        let rev = "0123456789abcdef0123456789abcdef01234567";
        let mut data = HyperLineData::new_introduced(rev, 5);
        assert_eq!(
            data.serialize(),
            r#"{"i":{"sr":"0123456789abcdef0123456789abcdef01234567","p":"%","l":5}}"#
        );

        data.predecessor = Some(HyperTokenRef {
            source_rev: Cow::Borrowed(rev),
            path: Cow::Borrowed("b.cpp"),
            lineno: 2,
        });
        let serialized = data.serialize();
        let parsed = HyperLineData::parse(&serialized);
        assert_eq!(parsed, data);

        // Moving from a.cpp into b.cpp should make the "%" explicit and make the
        // explicit "b.cpp" into "%".
        let mut moved = parsed.clone();
        moved.transplant("a.cpp", "b.cpp");
        assert_eq!(moved.introduced.path, "a.cpp");
        assert_eq!(moved.predecessor.as_ref().unwrap().path, "%");
    }
}
