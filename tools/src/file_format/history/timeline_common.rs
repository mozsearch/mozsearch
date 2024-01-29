use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DetailRecordRef {
    /// Source revision this record contains details for.
    pub source_rev: String,
    /// The syntax revision that corresponds to that source revision.
    pub syntax_rev: String,
    /// ISO 8601 date of the commit as told to us by git; git cinnabar seems to
    /// give us the autoland date, which is nice.
    pub iso_date: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SummaryRecordRef {
    /// List of all of the source revisions whose data is aggregated into this
    /// summary record ordered from newest to oldest.  It's possible to have a
    /// length of 1 as our policy is to aggregate at a week-based granularity
    /// for now.
    pub source_revs: Vec<String>,

    /// The timeline revision that precedes the creation of the revision that
    /// holds this summary record.  So if you look at this revision, you will
    /// find all of the detail records that were an input to the creation of
    /// this summary record.
    pub pred_timeline_rev: String,

    /// The [year, newest iso week inclusive, oldest iso week inclusive] time
    /// range that this summary is intended to cover.  For now we expect that
    /// all summary records will cover a single week, so the 2nd and 3rd values
    /// will be the same.  In the future we might imagine quantizing to a month
    /// granularity as a second pass, but it's not clear the additional
    /// decimation would be useful.
    ///
    /// Summary records should never overlap, so sorting by the tuple should
    /// work acceptably.
    pub iso_week_range: (u16, u8, u8),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenDeltaDetails {
    /// Number of times this token was present in a "+" diff delta that could
    /// not be attributed to a matching syntactically bound "-" and thereby
    /// counted as "moved".  Unlike something like `git log -S` which looks at
    /// the net change in tokens, it's completely possible for this record to
    /// have both a >0 "added" and "removed".
    pub added: u32,
    /// Fuzzy heuristic concept where we have reason to believe that a pair of
    /// "+" and "-" diff deltas for a token correspond to moved or very lightly
    /// refactored code.  Initially this means that the tokens had the same
    /// structural syntax binding scope, but in the future we could also
    /// potentially allow for changes in binding scope due to inferred method
    /// renames explaining the scope change.  Also keep in mind that because we
    /// initially will be only looking at what the diff deltas are, we are
    /// looking at the diff algorithm's attempt to find a minimal delta, but
    /// semantically it might be that some other greater number of changes
    /// should instead be counted as moved.
    pub moved: u32,
    /// Heuristic concept where we believe this token
    pub evolved_from: u32,
    /// Counterpart to "added"; the number of times this token was present in a
    /// "-" diff delta that was not attributed to "moved".
    pub removed: u32,
}

/// Indicate whether a symbol/token was added/changed/evolved/removed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// Newly added symbol/whatever.
    Added,
    /// The symbol/token/whatever existed before this and it still exists, but
    /// for a symbol, things inside it changed or its position changed, and for
    /// a token, its position changed.
    ///
    /// Arguably for the token, it would be less confusing to call this "moved",
    /// but from an implementation perspective it seems better to avoid creating
    /// another kind at this time.
    ///
    /// Note that when it comes to diffs, there's always the issue that a
    /// reordering of [A, B] to [B, A] is inherently semantically different and
    /// edit distance decides what happens.  We currently don't attempt to do
    /// anything to mark up what the diff algorithm decides stayed the same;
    /// we're just explaining what the diff algorithm decided.  This could
    /// change in the future if there's a good reason to be more clever, but in
    /// general the idea is that by having semantically bound tokens, we're
    /// already clever enough to avoid having things be misleading due to
    /// repurposing of tokens.
    Changed,
    /// The symbol/token/whatever was renamed or otherwise fundamentally
    /// changed, but we think we can tell you what the thing was before.
    Evolved,
    /// The symbol/whatever was removed.
    Removed,
}

/// Summarized changes at symbol granularity, with the "pretty" being assumed to
/// be stored externally in a map key that owns this value or in a wrapper if a
/// map is not involved.
#[derive(Debug, Serialize, Deserialize)]
pub struct SymbolSyntaxDelta {
    pub change: ChangeKind,

    /// Changes to tokens within the owning scope corresponding to this pretty
    /// identifier.
    pub token_changes: BTreeMap<String, TokenDeltaDetails>,
}

/// Holds aggregated changes to symbols.
#[derive(Debug, Serialize, Deserialize)]
pub struct SymbolSyntaxDeltaGroup {
    /// Maps symbols to the deltas observed related to the symbol.  Note that
    /// "%" is a sentinel corresponding to there being no scope
    /// which is arbitrarily derived from prior blame processing logic.
    pub symbol_deltas: BTreeMap<String, SymbolSyntaxDelta>,
}
