use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetailRecordRef {
    /// Source revision this record contains details for.
    pub source_rev: String,
    /// The syntax revision that corresponds to that source revision.
    pub syntax_rev: String,
    /// ISO 8601 date of the commit as told to us by git; git cinnabar seems to
    /// give us the autoland date, which is nice.
    pub iso_date: String,
    /// If this revision is a backout (see `hyperblame::backouts`), the source
    /// revisions it backs out.  Records are never modified after they're
    /// written, so readers wanting to hide backouts should hide records for
    /// revisions that newer records say they back out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backs_out: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDeltaDetails {
    /// Number of times this token was present in a "+" diff delta that could
    /// not be attributed to a matching syntactically bound "-" and thereby
    /// counted as "moved".  Unlike something like `git log -S` which looks at
    /// the net change in tokens, it's completely possible for this record to
    /// have both a >0 "added" and "removed".
    #[serde(default, skip_serializing_if = "is_zero")]
    pub added: u32,
    /// Fuzzy heuristic concept where we have reason to believe that a pair of
    /// "+" and "-" diff deltas for a token correspond to moved or very lightly
    /// refactored code.  This covers both tokens whose syntax binding scope
    /// changed (ex: the method containing them was renamed) and tokens that
    /// were moved within or between files.  Also keep in mind that because we
    /// start from the diff algorithm's attempt to find a minimal delta,
    /// semantically it might be that some other greater number of changes
    /// should instead be counted as moved.
    ///
    /// This is only counted for the "+" side of the move, so a move does not
    /// double-count.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub moved: u32,
    /// Heuristic concept where we believe this token was the result of another
    /// token evolving into this token; ex: a type or variable being renamed,
    /// `>` becoming `>=`.  This is counted on the "+" side of the evolution; the
    /// "-" side counts `evolved_into`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub evolved_from: u32,
    /// Counterpart to `evolved_from`; the number of times this token was
    /// present in a "-" diff delta that we believe evolved into another token.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub evolved_into: u32,
    /// Counterpart to "added"; the number of times this token was present in a
    /// "-" diff delta that was not attributed to "moved" or "evolved_into".
    #[serde(default, skip_serializing_if = "is_zero")]
    pub removed: u32,
}

impl TokenDeltaDetails {
    pub fn is_empty(&self) -> bool {
        self.added == 0
            && self.moved == 0
            && self.evolved_from == 0
            && self.evolved_into == 0
            && self.removed == 0
    }

    /// Does this represent anything other than moves?  Moves are interesting
    /// for summaries but for cases like the per-token timeline where we are
    /// trying to approximate `git log -S`, moves are just noise.
    pub fn has_non_move_changes(&self) -> bool {
        self.added != 0 || self.evolved_from != 0 || self.evolved_into != 0 || self.removed != 0
    }

    pub fn accumulate(&mut self, other: &TokenDeltaDetails) {
        self.added += other.added;
        self.moved += other.moved;
        self.evolved_from += other.evolved_from;
        self.evolved_into += other.evolved_into;
        self.removed += other.removed;
    }
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolSyntaxDelta {
    pub change: ChangeKind,

    /// For `ChangeKind::Evolved` symbols, the pretty identifier of the symbol
    /// we believe this symbol evolved from.  Currently this means that the
    /// majority of the tokens in this symbol were moved from that symbol and
    /// that symbol no longer exists, which is what a rename looks like.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evolved_from: Option<String>,

    /// For `ChangeKind::Removed` symbols, the pretty identifier of the symbol
    /// we believe this symbol evolved into.  The counterpart to `evolved_from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evolved_into: Option<String>,

    /// Aggregate changes across all tokens within the owning scope, including
    /// punctuation and other tokens which are not interesting enough to be
    /// individually listed in `token_changes`.
    #[serde(default, skip_serializing_if = "TokenDeltaDetails::is_empty")]
    pub token_totals: TokenDeltaDetails,

    /// Changes to identifier-like tokens within the owning scope corresponding
    /// to this pretty identifier.
    pub token_changes: BTreeMap<String, TokenDeltaDetails>,
}

impl SymbolSyntaxDelta {
    pub fn new(change: ChangeKind) -> Self {
        SymbolSyntaxDelta {
            change,
            evolved_from: None,
            evolved_into: None,
            token_totals: TokenDeltaDetails::default(),
            token_changes: BTreeMap::new(),
        }
    }
}

/// Holds aggregated changes to symbols.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SymbolSyntaxDeltaGroup {
    /// Maps symbols to the deltas observed related to the symbol.  Note that
    /// "%" is a sentinel corresponding to there being no scope
    /// which is arbitrarily derived from prior blame processing logic.
    pub symbol_deltas: BTreeMap<String, SymbolSyntaxDelta>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// The per-file payload shared by `FileDeltaDetailRecord` and
/// `RevFileSummaryRecord`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileSyntaxDelta {
    /// Added: the file was created.  Changed: the file was modified.  Removed:
    /// the file was deleted.  Evolved: the file was renamed/moved or copied from
    /// `moved_from` and potentially also modified.
    pub change: ChangeKind,

    /// For `ChangeKind::Evolved`, the path of the file this file was
    /// renamed/copied from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<String>,

    /// True if `moved_from` was a copy and the original file still exists.
    #[serde(default, skip_serializing_if = "is_false")]
    pub copied: bool,

    #[serde(flatten)]
    pub symbol_group: SymbolSyntaxDeltaGroup,
}

/// A set of 1-based token line numbers ("lineno" in `HyperTokenRef` terms)
/// which serializes to a compact string like "1-5,8,10-12" in the spirit of
/// IMAP UID sets.  Because tokens are minted with consecutive line numbers when
/// they are introduced, we expect sets of tokens introduced in the same
/// revision to be highly compressible via ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenLinenoSet(pub BTreeSet<u32>);

impl TokenLinenoSet {
    pub fn insert(&mut self, lineno: u32) {
        self.0.insert(lineno);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn contains(&self, lineno: u32) -> bool {
        self.0.contains(&lineno)
    }
}

impl fmt::Display for TokenLinenoSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut iter = self.0.iter().copied().peekable();
        let mut first = true;
        while let Some(start) = iter.next() {
            let mut end = start;
            while iter.peek() == Some(&(end + 1)) {
                end = iter.next().unwrap();
            }
            if !first {
                f.write_str(",")?;
            }
            first = false;
            if start == end {
                write!(f, "{}", start)?;
            } else {
                write!(f, "{}-{}", start, end)?;
            }
        }
        Ok(())
    }
}

impl FromStr for TokenLinenoSet {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut set = BTreeSet::new();
        for piece in s.split(',').filter(|p| !p.is_empty()) {
            let parse = |v: &str| v.parse::<u32>().map_err(|e| format!("{}: {}", v, e));
            match piece.split_once('-') {
                Some((start, end)) => {
                    set.extend(parse(start)?..=parse(end)?);
                }
                None => {
                    set.insert(parse(piece)?);
                }
            }
        }
        Ok(TokenLinenoSet(set))
    }
}

impl Serialize for TokenLinenoSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TokenLinenoSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Map from token source revision to token path (with "%" meaning the path of
/// the file this data is stored in) to the set of token line numbers.  This is
/// our compact representation for a set of canonical `HyperTokenRef`s.
pub type TokenRefSet = BTreeMap<String, BTreeMap<String, TokenLinenoSet>>;

pub fn token_ref_set_insert(set: &mut TokenRefSet, source_rev: &str, path: &str, lineno: u32) {
    set.entry(source_rev.to_string())
        .or_default()
        .entry(path.to_string())
        .or_default()
        .insert(lineno);
}

/// Common interface over the "Detail"/"Summary" record enums stored in timeline
/// journal files so that we can generically merge journal files when processing
/// merge commits.
pub trait TimelineRecord {
    /// Key used to de-duplicate records when merging journals from multiple
    /// parents.
    fn dedupe_key(&self) -> String;

    /// The ISO 8601 date of detail records, used to keep journals ordered from
    /// newest to oldest.  Summary records return None and sort after detail
    /// records because they are always older.
    fn iso_date(&self) -> Option<&str>;
}

/// Merge the records from multiple versions of a journal file, de-duplicating
/// and ordering them newest to oldest.  Order is stable for records with the
/// same date, favoring the order of the earlier versions.
pub fn merge_journal_records<R: TimelineRecord>(versions: Vec<Vec<R>>) -> Vec<R> {
    let mut seen = std::collections::HashSet::new();
    let mut merged: Vec<R> = vec![];
    for records in versions {
        for record in records {
            if seen.insert(record.dedupe_key()) {
                merged.push(record);
            }
        }
    }
    // `sort_by` is stable.  Detail records (Some) sort before summaries (None),
    // and newer dates sort first.
    merged.sort_by(|a, b| match (a.iso_date(), b.iso_date()) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lineno_set_roundtrip() {
        let set: TokenLinenoSet = "1-5,8,10-12".parse().unwrap();
        assert_eq!(set.len(), 9);
        assert!(set.contains(3));
        assert!(!set.contains(9));
        assert_eq!(set.to_string(), "1-5,8,10-12");

        let empty: TokenLinenoSet = "".parse().unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.to_string(), "");

        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, "\"1-5,8,10-12\"");
        let back: TokenLinenoSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, set);
    }
}
