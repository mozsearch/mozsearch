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
pub struct HyperTokenRef<'a> {
    pub source_rev: Cow<'a, str>,
    // XXX timestamp of authorship / change?
    // XXX relatedly, should the file summary info potential note the extinguished
    // token revisions?  like when we lose a deletion marker, do we kick it into
    // there so we can be faster about identifying when a token was removed?
    pub path: Cow<'a, str>,
    pub lineno: Cow<'a, str>,
}

/// XXX figuring out the algorithms; right now the use case is just for the
/// blame sidebar where just saying "100 tokens were removed here in rev FOO"
/// with our current blame idiom of "go to the removal rev at the point of
/// removal" and "go to the start of the run of removed tokens in the removal
/// rev's parent"
///
/// Identifies the revision in which a run of tokens were deleted without any
/// corresponding additions, the first of the run of tokens that was removed
/// (by ref), and the number of tokens that were removed.
///
/// This means that we can load the parent revision of the annotated file for
/// the path, locate the line with the `introduced` for the given ref, and then
/// count that many tokens and we will have located the exact run of tokens that
/// were removed without having to generate a diff.
pub struct RemovalMarker<'a> {
    pub source_rev: Cow<'a, str>,
    // XXX timestamp of removal commit?
    pub path: Cow<'a, str>,
    // XXX the line number of the removed token in the parent rev
    pub lineno: Cow<'a, str>,
    // XXX the removed, but which we could find by looking at that lineno in
    // the parent rev;
    pub first_removed: HyperTokenRef<'a>,
    pub num_removed: Cow<'a, str>,
}

pub struct HyperLineData<'a> {
    /// When was this token with its current string value introduced?
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
    pub predecessor: Option<HyperTokenRef<'a>>,
    /// We track runs of removed tokens on the preceding token so that we can
    /// render a visual indicator in the blame sidebar for removals.  This is
    /// not used
    pub removal_marker: Option<RemovalMarker<'a>>,
}
