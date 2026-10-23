//! This file defines the records we write into files under
//! `history/timeline/annotated`, and their encoding (see "Encoding" below).
//! These are the token-centric analog to the classic line-centric blame files.
//!
//! ## File Layout
//!
//! An annotated file has a `HyperLineData` record for each line index:
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
//!
//! ## Encoding
//!
//! Each record has a self-contained "line form" (`HyperLineData::serialize`),
//! of space-separated fields: the introduced ref's revision, path and line
//! number, then optionally `p` and the predecessor's, then optionally `rm` and
//! the removal marker's revision, path and line number, its first removed
//! token's, and how many tokens were removed and moved.  Ex:
//! `0123abcd… % 5 p 4567cdef… b.cpp 2`.  A field which is empty or has a
//! space, a backslash, a control character, or a leading `"` is written as a
//! JSON string (ex: a path with a space).
//!
//! A file (`encode_annotated`) dedupes the (revision, path) pairs of its
//! records into a table, since a file's tokens come from comparatively few
//! commits, and numbers runs of tokens:
//! - First, a `@HASH REV PATH` line for each pair the records use, sorted by
//!   hash.  `HASH` is the hex of a 64-bit FNV-1a hash of `REV PATH` (the
//!   fields as written), its first 6 digits, or more in a file whose pairs'
//!   first 6 collide, so a pair's hash is the same in every version of the
//!   file (unlike an ordinal), which keeps git's deltas between versions
//!   small.
//! - Then a line per record, in order (so line N of the file, past the
//!   table, is the record of line index N above): `HASH L`, where `L` is `+`
//!   if it's the line number after that of the previous record introduced by
//!   the same pair, then `p HASH L` and `rm HASH L HASH L NR NM` as above.
//!
//! On mozsearch's history (2026-10-21, 1.1M tokens) this was 10.1 bytes a
//! token rather than JSON's 82.6, and its annotated files packed (all of their
//! versions, `git repack -F`) into 2.2 MB rather than 8.0 MB: zlib compresses
//! the runs of `HASH +` well, and the full objects at the bases of git's delta
//! chains are most of a pack.  (See blame-evolution-notes.md's "Annotated
//! files' encoding".)
//!
//! Older histories' files are JSON lines, a `HyperLineData` per line, which
//! `decode_annotated`, `annotated_lines`, `annotated_line`, `find_token` and
//! `HyperLineData::parse` read too.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

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

    /// Parse a record's line form (see the module docs' "Encoding"), or an
    /// older history's JSON line.
    pub fn parse(line: &'a str) -> Self {
        Self::try_parse(line).unwrap_or_else(|e| panic!("Bad annotated line {:?}: {}", line, e))
    }

    pub fn try_parse(line: &'a str) -> Result<Self, String> {
        if line.starts_with('{') {
            return serde_json::from_str(line).map_err(|e| e.to_string());
        }
        let mut fields = Fields::new(line);
        let introduced = fields.token_ref()?;
        let mut data = HyperLineData {
            introduced,
            predecessor: None,
            removal_marker: None,
        };
        while let Some(marker) = fields.next_raw()? {
            match marker {
                "p" => data.predecessor = Some(fields.token_ref()?),
                "rm" => {
                    let at = fields.token_ref()?;
                    data.removal_marker = Some(RemovalMarker {
                        source_rev: at.source_rev,
                        path: at.path,
                        lineno: at.lineno,
                        first_removed: fields.token_ref()?,
                        num_removed: fields.number()?,
                        num_moved: fields.number()?,
                    });
                }
                _ => return Err(format!("unknown field {:?}", marker)),
            }
        }
        Ok(data)
    }

    /// The record's line form; see the module docs' "Encoding".
    pub fn serialize(&self) -> String {
        let mut line = String::with_capacity(64);
        push_ref(&mut line, &self.introduced);
        if let Some(pred) = &self.predecessor {
            line.push_str(" p ");
            push_ref(&mut line, pred);
        }
        if let Some(marker) = &self.removal_marker {
            line.push_str(" rm ");
            push_field(&mut line, &marker.source_rev);
            line.push(' ');
            push_field(&mut line, &marker.path);
            let _ = write!(line, " {} ", marker.lineno);
            push_ref(&mut line, &marker.first_removed);
            let _ = write!(line, " {} {}", marker.num_removed, marker.num_moved);
        }
        line
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

/// Whether a field needs writing as a JSON string; see the module docs'
/// "Encoding".
fn needs_quoting(field: &str) -> bool {
    field.is_empty()
        || field.starts_with('"')
        || field
            .bytes()
            .any(|b| b == b' ' || b == b'\\' || b.is_ascii_control())
}

fn push_field(out: &mut String, field: &str) {
    if needs_quoting(field) {
        out.push_str(&serde_json::to_string(field).unwrap());
    } else {
        out.push_str(field);
    }
}

fn push_ref(out: &mut String, token_ref: &HyperTokenRef) {
    push_field(out, &token_ref.source_rev);
    out.push(' ');
    push_field(out, &token_ref.path);
    let _ = write!(out, " {}", token_ref.lineno);
}

/// A field as written, unquoted.
fn unquote(raw: &str) -> Result<Cow<'_, str>, String> {
    if raw.starts_with('"') {
        serde_json::from_str::<String>(raw)
            .map(Cow::Owned)
            .map_err(|e| format!("bad quoted field {:?}: {}", raw, e))
    } else {
        Ok(Cow::Borrowed(raw))
    }
}

/// The space-separated fields of a line form or of a table line.
struct Fields<'a> {
    rest: &'a str,
}

impl<'a> Fields<'a> {
    fn new(line: &'a str) -> Self {
        Fields { rest: line }
    }

    /// The next field as written (quoted, if it is), or None at the end.
    fn next_raw(&mut self) -> Result<Option<&'a str>, String> {
        let rest = self.rest.strip_prefix(' ').unwrap_or(self.rest);
        if rest.is_empty() {
            self.rest = rest;
            return Ok(None);
        }
        let end = if rest.starts_with('"') {
            let bytes = rest.as_bytes();
            let mut i = 1;
            loop {
                match bytes.get(i) {
                    Some(b'\\') => i += 2,
                    Some(b'"') => break i + 1,
                    Some(_) => i += 1,
                    None => return Err(format!("unterminated field {:?}", rest)),
                }
            }
        } else {
            rest.find(' ').unwrap_or(rest.len())
        };
        let (field, rest) = rest.split_at(end);
        self.rest = rest;
        Ok(Some(field))
    }

    fn field(&mut self) -> Result<&'a str, String> {
        self.next_raw()?.ok_or_else(|| "missing field".to_string())
    }

    fn number(&mut self) -> Result<u32, String> {
        let field = self.field()?;
        field.parse().map_err(|_| format!("bad number {:?}", field))
    }

    fn token_ref(&mut self) -> Result<HyperTokenRef<'a>, String> {
        Ok(HyperTokenRef {
            source_rev: unquote(self.field()?)?,
            path: unquote(self.field()?)?,
            lineno: self.number()?,
        })
    }
}

/// The 64-bit FNV-1a hash of a pair's fields as written; see the module docs'
/// "Encoding".
fn pair_hash(rev: &str, path: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in rev.as_bytes().iter().chain(b" ").chain(path.as_bytes()) {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The hashes (as written) of a file's (revision, path) pairs (as written): 6
/// hex digits, or more if the file has pairs whose first 6 collide (and in
/// the unlikely case that two pairs' whole hashes collide, an `.N` suffix, in
/// the pairs' order).
fn pair_hashes<'p>(
    pairs: &[(&'p str, &'p str)],
    hash: impl Fn(&str, &str) -> u64,
) -> HashMap<(&'p str, &'p str), String> {
    let full: Vec<String> = pairs
        .iter()
        .map(|(rev, path)| format!("{:016x}", hash(rev, path)))
        .collect();
    let digits = (6..=16)
        .find(|&n| {
            let mut seen = HashSet::new();
            full.iter().all(|hash| seen.insert(&hash[..n]))
        })
        .unwrap_or(16);
    let mut sorted: Vec<usize> = (0..pairs.len()).collect();
    sorted.sort_by_key(|&i| pairs[i]);
    let mut uses: HashMap<&str, usize> = HashMap::new();
    let mut hashes = HashMap::new();
    for i in sorted {
        let hash = &full[i][..digits];
        let n = uses.entry(hash).or_default();
        let written = if *n == 0 {
            hash.to_string()
        } else {
            format!("{}.{}", hash, n)
        };
        *n += 1;
        hashes.insert(pairs[i], written);
    }
    hashes
}

/// Encode a file's records, given as their line forms (or an older history's
/// JSON lines), as its table form; see the module docs' "Encoding".
pub fn encode_annotated<S: AsRef<str>>(lines: &[S]) -> String {
    let records: Vec<HyperLineData> = lines
        .iter()
        .map(|line| HyperLineData::parse(line.as_ref()))
        .collect();
    // (Each ref's revision and path as written.)
    let written = |rev: &str, path: &str| -> (String, String) {
        let (mut r, mut p) = (String::new(), String::new());
        push_field(&mut r, rev);
        push_field(&mut p, path);
        (r, p)
    };
    let mut pair_fields: Vec<(String, String)> = vec![];
    let mut pair_index: HashMap<(&str, &str), usize> = HashMap::new();
    let mut record_pairs: Vec<[Option<usize>; 4]> = Vec::with_capacity(records.len());
    for record in &records {
        let mut refs = [None; 4];
        let marker = record.removal_marker.as_ref();
        let pairs = [
            Some((&*record.introduced.source_rev, &*record.introduced.path)),
            record
                .predecessor
                .as_ref()
                .map(|pred| (&*pred.source_rev, &*pred.path)),
            marker.map(|m| (&*m.source_rev, &*m.path)),
            marker.map(|m| (&*m.first_removed.source_rev, &*m.first_removed.path)),
        ];
        for (slot, pair) in refs.iter_mut().zip(pairs) {
            let Some(pair) = pair else {
                continue;
            };
            let next = pair_fields.len();
            let index = *pair_index.entry(pair).or_insert(next);
            if index == next {
                pair_fields.push(written(pair.0, pair.1));
            }
            *slot = Some(index);
        }
        record_pairs.push(refs);
    }
    let pairs: Vec<(&str, &str)> = pair_fields
        .iter()
        .map(|(r, p)| (r.as_str(), p.as_str()))
        .collect();
    let hashes = pair_hashes(&pairs, pair_hash);
    let hash_of = |index: usize| hashes[&pairs[index]].as_str();

    let mut table: Vec<(&str, usize)> = (0..pairs.len()).map(|i| (hash_of(i), i)).collect();
    table.sort();
    let mut out = String::with_capacity(records.len() * 12 + table.len() * 64);
    for (hash, index) in table {
        let (rev, path) = pairs[index];
        let _ = writeln!(out, "@{} {} {}", hash, rev, path);
    }
    let mut last_lineno: Vec<Option<u32>> = vec![None; pairs.len()];
    for (record, refs) in records.iter().zip(&record_pairs) {
        let introduced = refs[0].unwrap();
        out.push_str(hash_of(introduced));
        let lineno = record.introduced.lineno;
        if lineno > 0 && last_lineno[introduced] == Some(lineno - 1) {
            out.push_str(" +");
        } else {
            let _ = write!(out, " {}", lineno);
        }
        last_lineno[introduced] = Some(lineno);
        if let (Some(pred), Some(index)) = (&record.predecessor, refs[1]) {
            let _ = write!(out, " p {} {}", hash_of(index), pred.lineno);
        }
        if let (Some(marker), Some(at), Some(first)) = (&record.removal_marker, refs[2], refs[3]) {
            let _ = write!(
                out,
                " rm {} {} {} {} {} {}",
                hash_of(at),
                marker.lineno,
                hash_of(first),
                marker.first_removed.lineno,
                marker.num_removed,
                marker.num_moved
            );
        }
        out.push('\n');
    }
    out
}

/// What `encode_annotated_from` needs of a file it encoded to encode the
/// file's next version from it: the (revision, path) pairs (as written) its
/// records use and their hashes, and its records' (see `EncodedRecord`).
pub struct EncodedFile {
    pairs: Vec<(String, String)>,
    hashes: Vec<String>,
    records: Vec<EncodedRecord>,
}

impl EncodedFile {
    /// How many records the file has.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// A record of an `EncodedFile`: the indexes of its pairs (its introduced
/// ref's, its predecessor's, its removal marker's, and its first removed
/// token's, or `NO_PAIR`), its line number, the line number of the record
/// before it with the same introduced pair (or `NO_LINENO`), which decides
/// whether its line number is written as `+`, and where its record line is in
/// the file: its start, after its hash and line number, and its end (after
/// its newline).
#[derive(Clone, Copy)]
struct EncodedRecord {
    refs: [u32; 4],
    lineno: u32,
    prev: u32,
    start: u32,
    rest: u32,
    end: u32,
}

const NO_PAIR: u32 = u32::MAX;
const NO_LINENO: u32 = u32::MAX;

/// `encode_annotated`, with what `encode_annotated_from` needs to encode the
/// file's next version from this one.
pub fn encode_annotated_with_info<S: AsRef<str>>(lines: &[S]) -> (String, EncodedFile) {
    encode_annotated_from(lines, &[], None)
}

/// `encode_annotated` of `lines` (the same text), and what encoding the next
/// version from this one needs, copying the record lines of the lines which
/// are a `previous` version's (its lines, `EncodedFile` and text) from it
/// rather than encoding them again: `copied[i]`, if given, is the previous
/// version's line which line `i` may be, which it is if their texts are the
/// same.  A copied record line is the same unless the record before it with
/// the same introduced pair isn't, which decides whether its line number is
/// `+`, so only that is written again then, and the others are copied as they
/// are.  (Unless a pair's hash changes, ex: if a new pair's first 6 digits are
/// another's, when the whole file is encoded again.)  A revision of a big file
/// mostly copies its lines, ex: the HTML Standard's `source`, millions of
/// tokens, which encoding again at nearly every revision was most of the
/// wubspecs reblame's main thread.
pub fn encode_annotated_from<S: AsRef<str>>(
    lines: &[S],
    copied: &[Option<u32>],
    previous: Option<(&[String], &EncodedFile, &str)>,
) -> (String, EncodedFile) {
    // ## Each line's previous version's line, if it's a copy (or `NO_COPY`,
    // and then its record is the next of `new_records`).
    const NO_COPY: u32 = u32::MAX;
    let copy_of = |i: usize, line: &str| -> Option<u32> {
        let (previous_lines, _, _) = previous?;
        let j = (*copied.get(i)?)?;
        (previous_lines.get(j as usize).map(String::as_str) == Some(line)).then_some(j)
    };
    let mut new_records: Vec<HyperLineData> = vec![];
    let copies: Vec<u32> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = line.as_ref();
            copy_of(i, line).unwrap_or_else(|| {
                new_records.push(HyperLineData::parse(line));
                NO_COPY
            })
        })
        .collect();
    let previous_records: &[EncodedRecord] = previous.map_or(&[], |(_, file, _)| &file.records);
    let previous_text = previous.map_or("", |(_, _, text)| text);

    // ## The pairs: the previous version's, and the new records' (by their
    // fields as written, which `encode_annotated` dedupes them by the values
    // of).
    let mut pairs: Vec<(String, String)> = previous
        .map(|(_, file, _)| file.pairs.clone())
        .unwrap_or_default();
    let mut by_written: HashMap<(String, String), u32> = pairs
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, pair)| (pair, i as u32))
        .collect();
    let mut by_value: HashMap<(&str, &str), u32> = HashMap::new();
    let mut new_refs: Vec<[u32; 4]> = Vec::with_capacity(new_records.len());
    for record in &new_records {
        let marker = record.removal_marker.as_ref();
        let values = [
            Some((&*record.introduced.source_rev, &*record.introduced.path)),
            record
                .predecessor
                .as_ref()
                .map(|pred| (&*pred.source_rev, &*pred.path)),
            marker.map(|m| (&*m.source_rev, &*m.path)),
            marker.map(|m| (&*m.first_removed.source_rev, &*m.first_removed.path)),
        ];
        let mut refs = [NO_PAIR; 4];
        for (slot, value) in refs.iter_mut().zip(values) {
            let Some(value) = value else {
                continue;
            };
            *slot = *by_value.entry(value).or_insert_with(|| {
                let (mut rev, mut path) = (String::new(), String::new());
                push_field(&mut rev, value.0);
                push_field(&mut path, value.1);
                let next = pairs.len() as u32;
                let index = *by_written
                    .entry((rev.clone(), path.clone()))
                    .or_insert(next);
                if index == next {
                    pairs.push((rev, path));
                }
                index
            });
        }
        new_refs.push(refs);
    }

    // ## The pairs the records use, and their hashes.
    let mut used = vec![false; pairs.len()];
    let copied_refs = copies
        .iter()
        .filter(|&&j| j != NO_COPY)
        .map(|&j| previous_records[j as usize].refs);
    for refs in copied_refs.chain(new_refs.iter().copied()) {
        for &index in refs.iter().filter(|&&index| index != NO_PAIR) {
            used[index as usize] = true;
        }
    }
    let used_pairs: Vec<(&str, &str)> = (0..pairs.len())
        .filter(|&i| used[i])
        .map(|i| (pairs[i].0.as_str(), pairs[i].1.as_str()))
        .collect();
    let hash_map = pair_hashes(&used_pairs, pair_hash);
    let hashes: Vec<String> = (0..pairs.len())
        .map(|i| match used[i] {
            true => hash_map[&(pairs[i].0.as_str(), pairs[i].1.as_str())].clone(),
            false => String::new(),
        })
        .collect();
    // (The copied record lines have the previous version's hashes of their
    // pairs, which must still be theirs.)
    if let Some((_, file, _)) = previous
        && (0..file.pairs.len()).any(|i| used[i] && file.hashes[i] != hashes[i])
    {
        return encode_annotated_from(lines, &[], None);
    }

    // ## The table, and the record lines.
    let mut table: Vec<(&str, usize)> = (0..pairs.len())
        .filter(|&i| used[i])
        .map(|i| (hashes[i].as_str(), i))
        .collect();
    table.sort();
    let mut out = String::with_capacity(lines.len() * 12 + table.len() * 64);
    for (hash, index) in table {
        let _ = writeln!(out, "@{} {} {}", hash, pairs[index].0, pairs[index].1);
    }
    let mut last = vec![NO_LINENO; pairs.len()];
    let mut records = Vec::with_capacity(lines.len());
    let mut new_records_and_refs = new_records.iter().zip(&new_refs);
    for &j in &copies {
        let start = out.len();
        if j != NO_COPY {
            let record = &previous_records[j as usize];
            let introduced = record.refs[0] as usize;
            let (record_start, record_rest, record_end) = (
                record.start as usize,
                record.rest as usize,
                record.end as usize,
            );
            let rest = if last[introduced] == record.prev {
                out.push_str(&previous_text[record_start..record_end]);
                start + record_rest - record_start
            } else {
                push_lineno(
                    &mut out,
                    &hashes[introduced],
                    record.lineno,
                    last[introduced],
                );
                let rest = out.len();
                out.push_str(&previous_text[record_rest..record_end]);
                rest
            };
            records.push(EncodedRecord {
                refs: record.refs,
                lineno: record.lineno,
                prev: last[introduced],
                start: start as u32,
                rest: rest as u32,
                end: out.len() as u32,
            });
            last[introduced] = record.lineno;
            continue;
        }
        let (record, refs) = new_records_and_refs.next().unwrap();
        let introduced = refs[0] as usize;
        let lineno = record.introduced.lineno;
        push_lineno(&mut out, &hashes[introduced], lineno, last[introduced]);
        let rest = out.len();
        if let Some(pred) = &record.predecessor {
            let _ = write!(out, " p {} {}", hashes[refs[1] as usize], pred.lineno);
        }
        if let Some(marker) = &record.removal_marker {
            let _ = write!(
                out,
                " rm {} {} {} {} {} {}",
                hashes[refs[2] as usize],
                marker.lineno,
                hashes[refs[3] as usize],
                marker.first_removed.lineno,
                marker.num_removed,
                marker.num_moved
            );
        }
        out.push('\n');
        records.push(EncodedRecord {
            refs: *refs,
            lineno,
            prev: last[introduced],
            start: start as u32,
            rest: rest as u32,
            end: out.len() as u32,
        });
        last[introduced] = lineno;
    }

    // ## Only the pairs the records use, for the next version.
    let mut new_index = vec![NO_PAIR; pairs.len()];
    let mut kept_pairs = vec![];
    let mut kept_hashes = vec![];
    for (i, (pair, hash)) in pairs.into_iter().zip(hashes).enumerate() {
        if used[i] {
            new_index[i] = kept_pairs.len() as u32;
            kept_pairs.push(pair);
            kept_hashes.push(hash);
        }
    }
    for record in &mut records {
        for index in record.refs.iter_mut().filter(|index| **index != NO_PAIR) {
            *index = new_index[*index as usize];
        }
    }
    (
        out,
        EncodedFile {
            pairs: kept_pairs,
            hashes: kept_hashes,
            records,
        },
    )
}

/// A record line's hash and line number: `+` if the record before it with
/// the same introduced pair has the line number before it (see
/// `encode_annotated`).
fn push_lineno(out: &mut String, hash: &str, lineno: u32, prev: u32) {
    out.push_str(hash);
    if lineno > 0 && prev != NO_LINENO && prev == lineno - 1 {
        out.push_str(" +");
    } else {
        let _ = write!(out, " {}", lineno);
    }
}

/// The table of a file's table form: each hash's revision and path as
/// written, and the record lines after it.
fn read_table(text: &str) -> Result<(HashMap<&str, (&str, &str)>, std::str::Lines<'_>), String> {
    let mut table = HashMap::new();
    let mut lines = text.lines();
    let mut records = lines.clone();
    while let Some(line) = lines.next() {
        let Some(entry) = line.strip_prefix('@') else {
            break;
        };
        let mut fields = Fields::new(entry);
        let (hash, rev, path) = (fields.field()?, fields.field()?, fields.field()?);
        table.insert(hash, (rev, path));
        records = lines.clone();
    }
    Ok((table, records))
}

/// A record line of a table form, as its pairs (as written) and line numbers:
/// the introduced ref's (with `+` resolved through `last_lineno`), then the
/// predecessor's and the removal marker's, if it has them.
struct RecordLine<'a> {
    introduced: ((&'a str, &'a str), u32),
    predecessor: Option<((&'a str, &'a str), u32)>,
    /// The marker's revision, path and line number, its first removed token's,
    /// and how many tokens were removed and moved.
    removal: Option<(
        ((&'a str, &'a str), u32),
        ((&'a str, &'a str), u32),
        u32,
        u32,
    )>,
}

fn read_record_line<'a>(
    line: &'a str,
    table: &HashMap<&'a str, (&'a str, &'a str)>,
    last_lineno: &mut HashMap<&'a str, u32>,
) -> Result<RecordLine<'a>, String> {
    let mut fields = Fields::new(line);
    let pair = |fields: &mut Fields<'a>| -> Result<(&'a str, (&'a str, &'a str)), String> {
        let hash = fields.field()?;
        let pair = *table
            .get(hash)
            .ok_or_else(|| format!("unknown hash {:?}", hash))?;
        Ok((hash, pair))
    };
    let (hash, introduced) = pair(&mut fields)?;
    let lineno = match fields.field()? {
        "+" => last_lineno
            .get(hash)
            .map(|l| l + 1)
            .ok_or_else(|| format!("+ without an earlier line for {}", hash))?,
        lineno => lineno
            .parse()
            .map_err(|_| format!("bad number {:?}", lineno))?,
    };
    last_lineno.insert(hash, lineno);
    let mut record = RecordLine {
        introduced: (introduced, lineno),
        predecessor: None,
        removal: None,
    };
    while let Some(marker) = fields.next_raw()? {
        match marker {
            "p" => record.predecessor = Some((pair(&mut fields)?.1, fields.number()?)),
            "rm" => {
                let at = (pair(&mut fields)?.1, fields.number()?);
                let first = (pair(&mut fields)?.1, fields.number()?);
                record.removal = Some((at, first, fields.number()?, fields.number()?));
            }
            _ => return Err(format!("unknown field {:?}", marker)),
        }
    }
    Ok(record)
}

fn decoded_ref<'a>(
    ((rev, path), lineno): ((&'a str, &'a str), u32),
) -> Result<HyperTokenRef<'a>, String> {
    Ok(HyperTokenRef {
        source_rev: unquote(rev)?,
        path: unquote(path)?,
        lineno,
    })
}

fn line_form(record: &RecordLine) -> String {
    let mut line = String::with_capacity(64);
    let push = |line: &mut String, ((rev, path), lineno): ((&str, &str), u32)| {
        let _ = write!(line, "{} {} {}", rev, path, lineno);
    };
    push(&mut line, record.introduced);
    if let Some(pred) = record.predecessor {
        line.push_str(" p ");
        push(&mut line, pred);
    }
    if let Some((at, first, removed, moved)) = record.removal {
        line.push_str(" rm ");
        push(&mut line, at);
        line.push(' ');
        push(&mut line, first);
        let _ = write!(line, " {} {}", removed, moved);
    }
    line
}

fn is_json(text: &str) -> bool {
    text.starts_with('{')
}

/// Decode an annotated file's records (of either form; see the module docs'
/// "Encoding"), borrowing from `text` where it can.
pub fn decode_annotated(text: &str) -> Result<Vec<HyperLineData<'_>>, String> {
    if is_json(text) {
        return text.lines().map(HyperLineData::try_parse).collect();
    }
    let (table, lines) = read_table(text)?;
    let mut last_lineno = HashMap::new();
    lines
        .map(|line| {
            let record = read_record_line(line, &table, &mut last_lineno)?;
            Ok(HyperLineData {
                introduced: decoded_ref(record.introduced)?,
                predecessor: record.predecessor.map(decoded_ref).transpose()?,
                removal_marker: match record.removal {
                    Some((at, first, num_removed, num_moved)) => {
                        let at = decoded_ref(at)?;
                        Some(RemovalMarker {
                            source_rev: at.source_rev,
                            path: at.path,
                            lineno: at.lineno,
                            first_removed: decoded_ref(first)?,
                            num_removed,
                            num_moved,
                        })
                    }
                    None => None,
                },
            })
        })
        .collect()
}

/// An annotated file's records' line forms (an older history's JSON lines
/// converted), as `HyperLineData::serialize` writes them, for working with
/// its lines as text (ex: copying them unchanged into another version).
pub fn annotated_lines(text: &str) -> Result<Vec<String>, String> {
    if is_json(text) {
        return text
            .lines()
            .map(|line| HyperLineData::try_parse(line).map(|data| data.serialize()))
            .collect();
    }
    let (table, lines) = read_table(text)?;
    let mut last_lineno = HashMap::new();
    lines
        .map(|line| {
            Ok(line_form(&read_record_line(
                line,
                &table,
                &mut last_lineno,
            )?))
        })
        .collect()
}

/// The line form (or an older history's JSON line) of the record of line
/// index `index` (see the module docs), if the file has it.
pub fn annotated_line(text: &str, index: usize) -> Option<Cow<'_, str>> {
    if is_json(text) {
        return text.lines().nth(index).map(Cow::Borrowed);
    }
    let (table, lines) = read_table(text).ok()?;
    let mut last_lineno = HashMap::new();
    for (i, line) in lines.enumerate() {
        let record = read_record_line(line, &table, &mut last_lineno).ok()?;
        if i == index {
            return Some(Cow::Owned(line_form(&record)));
        }
    }
    None
}

/// The token whose canonical ref is `identity`, as its (0-based) index among
/// the tokens of `text` (the annotated file of `path`; the sentinel isn't a
/// token), or if `as_predecessor`, the token whose predecessor it is.
pub fn find_token(
    text: &str,
    path: &str,
    identity: &HyperTokenRef,
    as_predecessor: bool,
) -> Option<usize> {
    let mut relative = identity.clone();
    relative.relativize_path(path);
    if is_json(text) {
        return find_json_token(text, &relative, as_predecessor);
    }
    let (rev, path) = {
        let (mut rev, mut path) = (String::new(), String::new());
        push_field(&mut rev, &relative.source_rev);
        push_field(&mut path, &relative.path);
        (rev, path)
    };
    let (table, lines) = read_table(text).ok()?;
    // (Only the lines naming the pair's hash, whose `+`s only depend on its
    // own earlier lines.)
    let hash = table
        .iter()
        .find_map(|(hash, pair)| (*pair == (rev.as_str(), path.as_str())).then_some(*hash))?;
    let mut last_lineno = None;
    for (index, line) in lines.enumerate() {
        let mut fields = line.split(' ');
        let introduced = fields.next() == Some(hash);
        let lineno = fields.next()?;
        if introduced {
            let lineno = match lineno {
                "+" => last_lineno? + 1,
                _ => lineno.parse::<u32>().ok()?,
            };
            last_lineno = Some(lineno);
            if !as_predecessor && lineno == relative.lineno {
                return index.checked_sub(1);
            }
        }
        if as_predecessor
            && fields.next() == Some("p")
            && fields.next() == Some(hash)
            && fields.next().and_then(|l| l.parse::<u32>().ok()) == Some(relative.lineno)
        {
            return index.checked_sub(1);
        }
    }
    None
}

/// `find_token` for an older history's JSON lines, by searching for the
/// start of the token's line.
fn find_json_token(text: &str, relative: &HyperTokenRef, as_predecessor: bool) -> Option<usize> {
    let json = serde_json::to_string(relative).unwrap();
    let needle = if as_predecessor {
        format!(",\"p\":{}", json)
    } else {
        format!("{{\"i\":{}", json)
    };
    let mut from = 0;
    while let Some(found) = text[from..].find(&needle) {
        let at = from + found;
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        // The introduced ref is at the start of the line, and the predecessor
        // comes right after it.
        let matches = if as_predecessor {
            text[line_start..at].starts_with("{\"i\":") && !text[line_start..at].contains("},")
        } else {
            line_start == at
        };
        if matches {
            let line = text.as_bytes()[..line_start]
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            // Line 0 is the file sentinel, so line N is token N.
            return line.checked_sub(1);
        }
        from = at + 1;
    }
    None
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
            "0123456789abcdef0123456789abcdef01234567 % 5"
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

    fn token_ref(rev: &'static str, path: &'static str, lineno: u32) -> HyperTokenRef<'static> {
        HyperTokenRef {
            source_rev: Cow::Borrowed(rev),
            path: Cow::Borrowed(path),
            lineno,
        }
    }

    fn marker(rev: &'static str, first: HyperTokenRef<'static>) -> RemovalMarker<'static> {
        RemovalMarker {
            source_rev: Cow::Borrowed(rev),
            path: Cow::Borrowed("%"),
            lineno: 4,
            first_removed: first,
            num_removed: 3,
            num_moved: 1,
        }
    }

    #[test]
    fn test_line_form() {
        let mut data = HyperLineData::new_introduced("aaa", 3);
        data.predecessor = Some(token_ref("bbb", "dir/with space.cpp", 2));
        data.removal_marker = Some(marker("ccc", token_ref("ddd", "\"quoted\".h", 9)));
        let line = data.serialize();
        assert_eq!(
            line,
            r#"aaa % 3 p bbb "dir/with space.cpp" 2 rm ccc % 4 ddd "\"quoted\".h" 9 3 1"#
        );
        assert_eq!(HyperLineData::parse(&line), data);
        // (Paths without special characters are borrowed.)
        assert!(matches!(
            HyperLineData::parse("aaa a.cpp 1").introduced.path,
            Cow::Borrowed(_)
        ));
        // (Empty fields and backslashes are quoted too.)
        let odd = HyperLineData {
            introduced: token_ref("", "back\\slash", 0),
            predecessor: None,
            removal_marker: None,
        };
        assert_eq!(odd.serialize(), r#""" "back\\slash" 0"#);
        assert_eq!(HyperLineData::parse(&odd.serialize()), odd);
        // (Older histories' JSON lines.)
        let json = r#"{"i":{"sr":"aaa","p":"%","l":3},"p":{"sr":"bbb","p":"x.h","l":2}}"#;
        let parsed = HyperLineData::parse(json);
        assert_eq!(parsed.introduced, token_ref("aaa", "%", 3));
        assert_eq!(parsed.predecessor, Some(token_ref("bbb", "x.h", 2)));
        assert!(HyperLineData::try_parse("aaa % x").is_err());
        assert!(HyperLineData::try_parse("aaa % 3 q").is_err());
    }

    /// A file's records: a sentinel and tokens from two commits, with a
    /// predecessor and a removal marker.
    fn records() -> Vec<HyperLineData<'static>> {
        let mut records: Vec<HyperLineData> = (0..4)
            .map(|lineno| HyperLineData::new_introduced("rev-a", lineno))
            .collect();
        let mut evolved = HyperLineData::new_introduced("rev-b", 7);
        evolved.predecessor = Some(token_ref("rev-a", "other file.rs", 2));
        records.push(evolved);
        // (rev-a's line after its previous one, past rev-b's.)
        records.push(HyperLineData::new_introduced("rev-a", 4));
        let mut moved = HyperLineData::new_introduced("rev-a", 12);
        moved.removal_marker = Some(marker("rev-c", token_ref("rev-a", "%", 5)));
        records.push(moved);
        records
    }

    #[test]
    fn test_encode_decode() {
        let records = records();
        let lines: Vec<String> = records.iter().map(|r| r.serialize()).collect();
        let text = encode_annotated(&lines);
        let hash =
            |rev: &str, path: &str| format!("{:016x}", pair_hash(rev, path))[..6].to_string();
        let (a, b, c) = (hash("rev-a", "%"), hash("rev-b", "%"), hash("rev-c", "%"));
        let other = hash("rev-a", r#""other file.rs""#);
        let mut table = vec![
            format!("@{} rev-a %", a),
            format!("@{} rev-b %", b),
            format!("@{} rev-c %", c),
            format!(r#"@{} rev-a "other file.rs""#, other),
        ];
        table.sort();
        let expected = format!(
            "{}\n{a} 0\n{a} +\n{a} +\n{a} +\n{b} 7 p {other} 2\n{a} +\n{a} 12 rm {c} 4 {a} 5 3 1\n",
            table.join("\n"),
            a = a,
            b = b,
            c = c,
            other = other
        );
        assert_eq!(text, expected);
        assert_eq!(decode_annotated(&text).unwrap(), records);
        assert_eq!(annotated_lines(&text).unwrap(), lines);
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(annotated_line(&text, index).as_deref(), Some(line.as_str()));
        }
        assert_eq!(annotated_line(&text, lines.len()), None);
        // (Encoding again, from the decoded lines, gives the same file, so
        // unchanged versions stay identical.)
        assert_eq!(encode_annotated(&annotated_lines(&text).unwrap()), text);
        assert_eq!(decode_annotated("").unwrap(), vec![]);
        assert!(decode_annotated("@abc rev %\nzzz 1\n").is_err());
        assert!(decode_annotated("@abc rev %\nabc +\n").is_err());
    }

    /// `encode_annotated_from` gives `encode_annotated`'s text for versions
    /// of a file made by random edits of the previous version (copied runs,
    /// whose `+` line numbers change where lines before them were removed or
    /// added, new records, changed lines whose `copied` still says they're
    /// copies, and wrong `copied` lines), each encoded from the one before.
    #[test]
    fn test_encode_annotated_from() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let revs = ["rev-a", "rev-b", "rev-c", "r d", ""];
        let paths = ["%", "a.cpp", "with space.txt"];
        let random_line = |next: &mut dyn FnMut(usize) -> usize, lineno: u32| -> String {
            let mut data = HyperLineData::new_introduced("", lineno);
            data.introduced.source_rev = Cow::Owned(revs[next(revs.len())].to_string());
            data.introduced.path = Cow::Owned(paths[next(paths.len())].to_string());
            if next(4) == 0 {
                data.predecessor = Some(HyperTokenRef {
                    source_rev: Cow::Owned(revs[next(revs.len())].to_string()),
                    path: Cow::Owned(paths[next(paths.len())].to_string()),
                    lineno: next(50) as u32,
                });
            }
            if next(8) == 0 {
                data.removal_marker = Some(RemovalMarker {
                    source_rev: Cow::Owned(revs[next(revs.len())].to_string()),
                    path: Cow::Borrowed("%"),
                    lineno: next(50) as u32,
                    first_removed: HyperTokenRef::new_unchanged_path("rev-b", next(50) as u32),
                    num_removed: next(5) as u32,
                    num_moved: next(5) as u32,
                });
            }
            data.serialize()
        };
        let (mut copies, mut steps) = (0, 0);
        for _ in 0..40 {
            // (Runs of a revision's tokens, whose line numbers are mostly `+`.)
            let mut lines: Vec<String> = (0..(1 + next(200)))
                .map(|i| {
                    let rev = ["rev-a", "rev-b"][(i / 17) % 2];
                    HyperLineData::new_introduced(rev, i as u32).serialize()
                })
                .collect();
            let (mut text, mut info) = encode_annotated_with_info(&lines);
            assert_eq!(text, encode_annotated(&lines));
            for _ in 0..8 {
                let mut new_lines = vec![];
                let mut copied = vec![];
                for (j, line) in lines.iter().enumerate() {
                    match next(20) {
                        0 => {}
                        1 => {
                            let lineno = next(300) as u32;
                            new_lines.push(random_line(&mut next, lineno));
                            copied.push(None);
                        }
                        2 => {
                            // (A changed line, which `copied` still says is a copy.)
                            new_lines.push(random_line(&mut next, j as u32));
                            copied.push(Some(j as u32));
                        }
                        3 => {
                            // (A wrong `copied` line.)
                            new_lines.push(line.clone());
                            copied.push(Some(next(lines.len()) as u32));
                        }
                        _ => {
                            new_lines.push(line.clone());
                            copied.push(Some(j as u32));
                            copies += 1;
                        }
                    }
                }
                let (new_text, new_info) =
                    encode_annotated_from(&new_lines, &copied, Some((&lines, &info, &text)));
                assert_eq!(new_text, encode_annotated(&new_lines));
                assert_eq!(new_info.len(), new_lines.len());
                (lines, text, info) = (new_lines, new_text, new_info);
                steps += 1;
            }
        }
        assert!(copies > 0 && steps == 320);
    }

    /// How long encoding a version of a 3M-line file (like the HTML Standard's
    /// annotated files) with a few edits takes from the previous version,
    /// rather than all of it.
    #[test]
    #[ignore]
    fn bench_encode_annotated_from() {
        let lines: Vec<String> = (0..3_000_000u32)
            .map(|i| {
                let rev = format!("{:040x}", (i / 9973) % 300);
                HyperLineData::new_introduced(&rev, i).serialize()
            })
            .collect();
        let (text, info) = encode_annotated_with_info(&lines);
        // (A few tokens changed, and a few added.)
        let mut new_lines = vec![];
        let mut copied = vec![];
        for (j, line) in lines.iter().enumerate() {
            if j % 300_000 == 1 {
                new_lines.push(HyperLineData::new_introduced("new-rev", j as u32).serialize());
                copied.push(None);
            }
            new_lines.push(line.clone());
            copied.push(Some(j as u32));
        }
        let start = std::time::Instant::now();
        let full = encode_annotated(&new_lines);
        eprintln!("all of it: {:?}", start.elapsed());
        let start = std::time::Instant::now();
        let (from, _) = encode_annotated_from(&new_lines, &copied, Some((&lines, &info, &text)));
        eprintln!("from the previous version: {:?}", start.elapsed());
        assert_eq!(from, full);
    }

    #[test]
    fn test_older_json_files() {
        let records = records();
        let json: Vec<String> = records
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        let text = json.join("\n") + "\n";
        assert_eq!(decode_annotated(&text).unwrap(), records);
        let lines: Vec<String> = records.iter().map(|r| r.serialize()).collect();
        assert_eq!(annotated_lines(&text).unwrap(), lines);
        assert_eq!(annotated_line(&text, 4).as_deref(), Some(json[4].as_str()));
        // (And encoding them gives the table form.)
        assert_eq!(encode_annotated(&json), encode_annotated(&lines));
    }

    #[test]
    fn test_find_token() {
        let records = records();
        let lines: Vec<String> = records.iter().map(|r| r.serialize()).collect();
        let json: Vec<String> = records
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        for text in [encode_annotated(&lines), json.join("\n") + "\n"] {
            let find = |identity: HyperTokenRef, as_predecessor| {
                find_token(&text, "f.rs", &identity, as_predecessor)
            };
            // (Token N is line N, past the sentinel; paths relative to f.rs.)
            assert_eq!(find(token_ref("rev-a", "f.rs", 1), false), Some(0));
            assert_eq!(find(token_ref("rev-a", "f.rs", 3), false), Some(2));
            assert_eq!(find(token_ref("rev-a", "f.rs", 4), false), Some(4));
            assert_eq!(find(token_ref("rev-a", "f.rs", 12), false), Some(5));
            assert_eq!(find(token_ref("rev-b", "f.rs", 7), false), Some(3));
            assert_eq!(find(token_ref("rev-a", "other file.rs", 2), true), Some(3));
            assert_eq!(find(token_ref("rev-a", "other file.rs", 2), false), None);
            assert_eq!(find(token_ref("rev-a", "f.rs", 2), true), None);
            assert_eq!(find(token_ref("rev-z", "f.rs", 1), false), None);
            // (The sentinel isn't a token.)
            assert_eq!(find(token_ref("rev-a", "f.rs", 0), false), None);
        }
    }

    #[test]
    fn test_pair_hash_collisions() {
        let pairs = [("r1", "%"), ("r2", "%"), ("r3", "%")];
        // (Hashes whose first 6 digits collide, then whole ones.)
        let hashes = pair_hashes(&pairs, |rev, _| match rev {
            "r1" => 0xabcdef0000000001,
            "r2" => 0xabcdef0000000002,
            _ => 0x1234560000000000,
        });
        assert_eq!(hashes[&("r1", "%")], "abcdef0000000001");
        assert_eq!(hashes[&("r3", "%")], "1234560000000000");
        let hashes = pair_hashes(&pairs, |rev, _| if rev == "r3" { 1 } else { 0 });
        assert_eq!(hashes[&("r1", "%")], "0000000000000000");
        assert_eq!(hashes[&("r2", "%")], "0000000000000000.1");
        assert_eq!(hashes[&("r3", "%")], "0000000000000001");
        // (Usually, 6 digits.)
        let hashes = pair_hashes(&pairs, pair_hash);
        assert!(hashes.values().all(|h| h.len() == 6));
    }
}
