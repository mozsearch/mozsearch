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

/// A version of a journal: the journal at `path` (relative to the root of the
/// timeline repo, ex: "future/dom/base/nsINode.cpp.ndjson") in the timeline
/// commit `timeline_rev`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JournalVersionRef {
    pub timeline_rev: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryRecordRef {
    /// List of all of the source revisions whose data is aggregated into this
    /// summary record ordered from newest to oldest.  It's possible to have a
    /// length of 1 as our policy is to aggregate at a week-based granularity
    /// for now.
    pub source_revs: Vec<String>,

    /// The versions of this journal which contain the detail records for
    /// `source_revs`, or summaries of them whose own `preds` do, and so on;
    /// see `hyperblame::journals` for how summaries get expanded.  Usually
    /// this is the journal in the timeline commit preceding the commit that
    /// created this summary, preceded by the preds of the summaries it
    /// summarized again, if any, and a summary created when merging has the
    /// preds of the summaries it merged (see `consolidation::flattened_preds`;
    /// histories from before that pointed at the summaries' versions).  The
    /// path differs if the journal was renamed or copied since (files-delta
    /// journals follow their files).
    pub preds: Vec<JournalVersionRef>,

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

/// A journal record line's "type" tag.  The record enums deserialize a line
/// into a struct with every field it can have (the tag's, `DetailRecordRef`'s
/// and `SummaryRecordRef`'s, and those of the records' other parts) and build
/// the record from that, since serde's derived deserialization of an internally
/// tagged enum buffers a line's fields to find the tag, and then flattened
/// fields buffer them again, which was a lot of the timeline's time.  (Their
/// serialization is derived.)
#[derive(Deserialize)]
pub(super) enum RecordKind {
    Detail,
    Summary,
}

/// The `DetailRecordRef` of a detail record line's fields (see `RecordKind`).
pub(super) fn detail_record_ref<E: serde::de::Error>(
    source_rev: Option<String>,
    syntax_rev: Option<String>,
    iso_date: Option<String>,
    backs_out: Vec<String>,
) -> Result<DetailRecordRef, E> {
    Ok(DetailRecordRef {
        source_rev: source_rev.ok_or_else(|| E::missing_field("source_rev"))?,
        syntax_rev: syntax_rev.ok_or_else(|| E::missing_field("syntax_rev"))?,
        iso_date: iso_date.ok_or_else(|| E::missing_field("iso_date"))?,
        backs_out,
    })
}

/// The `SummaryRecordRef` of a summary record line's fields (see `RecordKind`).
pub(super) fn summary_record_ref<E: serde::de::Error>(
    source_revs: Option<Vec<String>>,
    preds: Option<Vec<JournalVersionRef>>,
    iso_week_range: Option<(u16, u8, u8)>,
) -> Result<SummaryRecordRef, E> {
    Ok(SummaryRecordRef {
        source_revs: source_revs.ok_or_else(|| E::missing_field("source_revs"))?,
        preds: preds.ok_or_else(|| E::missing_field("preds"))?,
        iso_week_range: iso_week_range.ok_or_else(|| E::missing_field("iso_week_range"))?,
    })
}

/// Common interface over the "Detail"/"Summary" record enums stored in timeline
/// journal files, for reading, consolidating, and merging journals generically;
/// see `hyperblame::journals` and `hyperblame::consolidation`.
pub trait TimelineRecord {
    /// The ISO 8601 date of detail records.
    fn iso_date(&self) -> Option<&str>;

    /// The source revision of a detail record.
    fn detail_source_rev(&self) -> Option<&str>;

    /// The common summary fields of a summary record.
    fn summary_ref(&self) -> Option<&SummaryRecordRef>;
}

/// Test support for the record enums' `Deserialize` implementations (see
/// `RecordKind`).
#[cfg(test)]
pub(crate) mod test_support {
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};

    use super::super::timeline_files_delta::{
        FileDeltaDetailRecord, FileDeltaRecord, FileDeltaSummaryRecord,
    };
    use super::super::timeline_future::{FutureDetailRecord, FutureRecord, FutureSummaryRecord};
    use super::super::timeline_tokens::{
        TokenDeltaDetailRecord, TokenDeltaRecord, TokenDeltaSummaryRecord,
    };

    /// The record enums with serde's derived deserialization.
    #[derive(Serialize, Deserialize)]
    #[serde(tag = "type")]
    pub enum DerivedTokenDeltaRecord {
        Detail(TokenDeltaDetailRecord),
        Summary(TokenDeltaSummaryRecord),
    }

    #[derive(Serialize, Deserialize)]
    #[serde(tag = "type")]
    pub enum DerivedFutureRecord {
        Detail(FutureDetailRecord),
        Summary(FutureSummaryRecord),
    }

    #[derive(Serialize, Deserialize)]
    #[serde(tag = "type")]
    pub enum DerivedFileDeltaRecord {
        Detail(FileDeltaDetailRecord),
        Summary(FileDeltaSummaryRecord),
    }

    /// Check that `line` deserializes the same for a journal of `kind`
    /// ("tokens", "future" or "files-delta") as with the derived
    /// deserialization (see `assert_parses_like`).
    pub fn assert_record_parses_like(kind: &str, line: &str) {
        match kind {
            "tokens" => assert_parses_like::<DerivedTokenDeltaRecord, TokenDeltaRecord>(line),
            "future" => assert_parses_like::<DerivedFutureRecord, FutureRecord>(line),
            "files-delta" => assert_parses_like::<DerivedFileDeltaRecord, FileDeltaRecord>(line),
            _ => panic!("unknown journal kind {}", kind),
        }
    }

    /// Check that `line` deserializes as a `New` the same as it does as an
    /// `Old` (the record enum with its derived deserialization), comparing
    /// their serializations, or that neither can deserialize it.
    pub fn assert_parses_like<Old, New>(line: &str)
    where
        Old: DeserializeOwned + Serialize,
        New: DeserializeOwned + Serialize,
    {
        let old = serde_json::from_str::<Old>(line).map(|r| serde_json::to_string(&r).unwrap());
        let new = serde_json::from_str::<New>(line).map(|r| serde_json::to_string(&r).unwrap());
        match (old, new) {
            (Ok(old), Ok(new)) => assert_eq!(old, new, "for {}", line),
            (Err(_), Err(_)) => {}
            (old, new) => panic!("for {}: derived {:?} but {:?}", line, old, new),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_deserialization_matches_derived() {
        use super::super::timeline_files_delta::{
            FileDeltaDetailRecord, FileDeltaRecord, FileDeltaSummaryRecord,
        };
        use super::super::timeline_future::{
            FutureDetailRecord, FutureFileChanges, FutureRecord, FutureSummaryRecord,
        };
        use super::super::timeline_tokens::{
            TokenDeltaDetailRecord, TokenDeltaRecord, TokenDeltaSummaryRecord,
        };
        use test_support::assert_record_parses_like;

        let detail = DetailRecordRef {
            source_rev: "abc".to_string(),
            syntax_rev: "def".to_string(),
            iso_date: "2024-01-14T23:59:59Z".to_string(),
            backs_out: vec!["fed".to_string()],
        };
        let summary = SummaryRecordRef {
            source_revs: vec!["abc".to_string(), "123".to_string()],
            preds: vec![JournalVersionRef {
                timeline_rev: "456".to_string(),
                path: "future/iso_week_range\":[.ndjson".to_string(),
            }],
            iso_week_range: (2024, 2, 2),
        };
        let delta = TokenDeltaDetails {
            added: 1,
            moved: 2,
            evolved_from: 3,
            evolved_into: 4,
            removed: 5,
        };
        let mut tokens = TokenRefSet::new();
        token_ref_set_insert(&mut tokens, "abc", "iso_date", 3);
        token_ref_set_insert(&mut tokens, "abc", "%", 5);
        let file_changes = FutureFileChanges {
            file_deleted: true,
            file_moved_to: Some("to".to_string()),
            file_moved_from: Some("from".to_string()),
            file_copied: true,
        };
        let mut symbol = SymbolSyntaxDelta::new(ChangeKind::Evolved);
        symbol.evolved_from = Some("Old".to_string());
        symbol.token_totals = delta.clone();
        symbol
            .token_changes
            .insert("iso_date".to_string(), delta.clone());
        let symbol_group = SymbolSyntaxDeltaGroup {
            symbol_deltas: BTreeMap::from([("New".to_string(), symbol)]),
        };

        let mut lines = vec![];
        let mut add = |kind: &str, json: String| lines.push((kind.to_string(), json));
        for detail in [
            detail.clone(),
            DetailRecordRef {
                backs_out: vec![],
                ..detail.clone()
            },
        ] {
            add(
                "tokens",
                serde_json::to_string(&TokenDeltaRecord::Detail(TokenDeltaDetailRecord {
                    desc: detail.clone(),
                    delta: delta.clone(),
                }))
                .unwrap(),
            );
            add(
                "future",
                serde_json::to_string(&FutureRecord::Detail(FutureDetailRecord {
                    desc: detail.clone(),
                    file_changes: file_changes.clone(),
                    extinguished_tokens: tokens.clone(),
                    moved_out_tokens: tokens.clone(),
                    moved_in_tokens: tokens.clone(),
                    evolved_tokens: tokens.clone(),
                    added_tokens: "1-3,7".parse().unwrap(),
                }))
                .unwrap(),
            );
            add(
                "future",
                serde_json::to_string(&FutureRecord::Detail(FutureDetailRecord::new(
                    detail.clone(),
                )))
                .unwrap(),
            );
            add(
                "files-delta",
                serde_json::to_string(&FileDeltaRecord::Detail(FileDeltaDetailRecord {
                    desc: detail.clone(),
                    delta: FileSyntaxDelta {
                        change: ChangeKind::Evolved,
                        moved_from: Some("old/path".to_string()),
                        copied: true,
                        symbol_group: symbol_group.clone(),
                    },
                }))
                .unwrap(),
            );
        }
        add(
            "tokens",
            serde_json::to_string(&TokenDeltaRecord::Summary(TokenDeltaSummaryRecord {
                desc: summary.clone(),
                delta: TokenDeltaDetails::default(),
            }))
            .unwrap(),
        );
        add(
            "future",
            serde_json::to_string(&FutureRecord::Summary(FutureSummaryRecord {
                desc: summary.clone(),
                file_changes: file_changes.clone(),
                removed_token_revs: BTreeSet::from(["abc".to_string()]),
                moved_token_revs: BTreeSet::from(["def".to_string()]),
                evolved_token_revs: BTreeSet::from(["fed".to_string()]),
            }))
            .unwrap(),
        );
        add(
            "files-delta",
            serde_json::to_string(&FileDeltaRecord::Summary(FileDeltaSummaryRecord {
                desc: summary.clone(),
                symbol_group: symbol_group.clone(),
            }))
            .unwrap(),
        );

        for (kind, line) in &lines {
            assert_record_parses_like(kind, line);
            // Lines missing a required field (or with an unknown one, or with
            // the tag elsewhere), which neither should (or both should) read.
            for field in [
                "iso_date",
                "source_rev",
                "syntax_rev",
                "source_revs",
                "preds",
                "iso_week_range",
                "change",
                "symbol_deltas",
            ] {
                let renamed = line.replacen(&format!("\"{}\":", field), "\"unknown\":", 1);
                assert_record_parses_like(kind, &renamed);
            }
            assert_record_parses_like(kind, &line.replacen("\"Detail\"", "\"Bogus\"", 1));
            let untagged = line.replacen("\"type\":\"Detail\",", "", 1).replacen(
                "\"type\":\"Summary\",",
                "",
                1,
            );
            let tag = if line.contains("\"Detail\"") {
                "Detail"
            } else {
                "Summary"
            };
            let moved_tag = format!(
                "{},\"type\":\"{}\"}}",
                untagged.strip_suffix('}').unwrap(),
                tag
            );
            assert!(serde_json::from_str::<serde_json::Value>(&moved_tag).is_ok());
            assert_record_parses_like(kind, &moved_tag);
        }
    }

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
