//! This file defines the ND-JSON records we write into files under
//! `history/timeline/tokens/ab/cd/` where "abcd" starts a hash of the
//! (lowercased) token, which keeps the directories small and evenly sized; see
//! `token_timeline_path`.  (git fast-import handles big directories slowly, and
//! directories named by the tokens' first characters grew huge: ex: with
//! "tokens/ab/cd/ef/" for the first 6 characters, the whole of firefox-main had
//! 17k journals in "tokens/ep/ox/y_/" and 314k first-level directories, mostly
//! for pairs of CJK characters from ICU's CJK dictionary.)
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
//! Only tokens selected by `tracked_token_keys` get files (by the rules of
//! the history's timeline, which a root file says; see `KeyRules`), and we
//! only emit a record for a revision if the token was added/removed/evolved;
//! pure moves are not recorded here because they are noise for these
//! use-cases, but they can be found in the rev-summaries and files-delta
//! records.  The same selection is used for the per-symbol `token_changes` in
//! files-delta records and rev-summaries.
//!
//! The first line of each file is a `TokenHeader` and the remaining lines are
//! `TokenDeltaRecord`s ordered from newest to oldest.

use std::borrow::Cow;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use super::syntax_files::{TokenClass, TokenLine};

use super::timeline_common::{
    DetailRecordRef, JournalVersionRef, RecordKind, SummaryRecordRef, TimelineRecord,
    TokenDeltaDetails, detail_record_ref, summary_record_ref,
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TokenHeader {
    /// The years of the journal's segments, newest first; see
    /// `hyperblame::segments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<i32>,
    /// The token's instances (see `hyperblame::token_counts`), which the
    /// journal's records sum to, so that readers needn't read its segments for
    /// it (ex: `token_timeline`'s absolute counts).  Histories from before
    /// journals' sums were exact (merges' records; see build-timeline-tree's
    /// "Merges") don't have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
}

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
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum TokenDeltaRecord {
    Detail(TokenDeltaDetailRecord),
    Summary(TokenDeltaSummaryRecord),
}

/// Every field a `TokenDeltaRecord` line can have (see `RecordKind`).
#[derive(Deserialize)]
struct TokenDeltaRecordFields {
    #[serde(rename = "type")]
    kind: RecordKind,
    source_rev: Option<String>,
    syntax_rev: Option<String>,
    iso_date: Option<String>,
    #[serde(default)]
    backs_out: Vec<String>,
    source_revs: Option<Vec<String>>,
    preds: Option<Vec<JournalVersionRef>>,
    iso_week_range: Option<(u16, u8, u8)>,
    #[serde(default)]
    added: u32,
    #[serde(default)]
    moved: u32,
    #[serde(default)]
    evolved_from: u32,
    #[serde(default)]
    evolved_into: u32,
    #[serde(default)]
    removed: u32,
}

impl<'de> Deserialize<'de> for TokenDeltaRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let f = TokenDeltaRecordFields::deserialize(deserializer)?;
        let delta = TokenDeltaDetails {
            added: f.added,
            moved: f.moved,
            evolved_from: f.evolved_from,
            evolved_into: f.evolved_into,
            removed: f.removed,
        };
        Ok(match f.kind {
            RecordKind::Detail => TokenDeltaRecord::Detail(TokenDeltaDetailRecord {
                desc: detail_record_ref(f.source_rev, f.syntax_rev, f.iso_date, f.backs_out)?,
                delta,
            }),
            RecordKind::Summary => TokenDeltaRecord::Summary(TokenDeltaSummaryRecord {
                desc: summary_record_ref(f.source_revs, f.preds, f.iso_week_range)?,
                delta,
            }),
        })
    }
}

impl TimelineRecord for TokenDeltaRecord {
    fn iso_date(&self) -> Option<&str> {
        match self {
            TokenDeltaRecord::Detail(d) => Some(&d.desc.iso_date),
            TokenDeltaRecord::Summary(_) => None,
        }
    }

    fn detail_source_rev(&self) -> Option<&str> {
        match self {
            TokenDeltaRecord::Detail(d) => Some(&d.desc.source_rev),
            TokenDeltaRecord::Summary(_) => None,
        }
    }

    fn summary_ref(&self) -> Option<&SummaryRecordRef> {
        match self {
            TokenDeltaRecord::Detail(_) => None,
            TokenDeltaRecord::Summary(s) => Some(&s.desc),
        }
    }
}

/// Does this text look like an identifier?  Specifically: at least 2
/// characters long, made up of alphanumeric characters, "_", and "$", not
/// starting with a digit, and with at least one alphabetic character.
pub fn is_trackable_token(token: &str) -> bool {
    token.len() >= 2
        && token.len() <= 128
        && !token.starts_with(|c: char| c.is_ascii_digit())
        && token.chars().all(is_word_char)
        && token.chars().any(|c| c.is_alphabetic())
}

/// Whether a character can be part of a word (see `is_trackable_token`).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Which keys a history's token journals track tokens under (see
/// `tracked_token_keys`), as its timeline says (see `KeyRules::of_tree`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyRules {
    /// Identifiers, and the words in strings, comments and plain text which
    /// look like them (a timeline without a `token-keys` file: histories made
    /// before Part 690).
    V1,
    /// Also compound words, whole and each of their parts (see
    /// `compound_keys`), in strings, comments, plain text, and identifiers
    /// which aren't words (ex: CSS's `background-color`, and markup's
    /// attribute values).
    #[default]
    V2,
}

/// The file at a timeline's root which says its key rules (`2`, for V2),
/// which build-timeline-tree writes in the root commits of a timeline it
/// starts (and every commit has from its first parent).
pub const KEY_RULES_FILE: &str = "token-keys";

impl KeyRules {
    /// The rules of the timeline commit whose tree is `tree`.
    pub fn of_tree(repo: &git2::Repository, tree: &git2::Tree) -> Result<KeyRules, String> {
        let Ok(entry) = tree.get_path(std::path::Path::new(KEY_RULES_FILE)) else {
            return Ok(KeyRules::V1);
        };
        let blob = repo.find_blob(entry.id()).map_err(|e| e.to_string())?;
        match std::str::from_utf8(blob.content()).map(str::trim) {
            Ok("2") => Ok(KeyRules::V2),
            _ => Err(format!(
                "The timeline's {} has rules this doesn't know: {:?}",
                KEY_RULES_FILE,
                String::from_utf8_lossy(blob.content())
            )),
        }
    }

    /// What a timeline's `token-keys` file has for these rules (V1's
    /// timelines have none).
    pub fn file_contents(self) -> Option<&'static str> {
        match self {
            KeyRules::V1 => None,
            KeyRules::V2 => Some("2\n"),
        }
    }
}

/// Words which tree-sitter classifies as identifiers (or which we classify as
/// identifiers for consistency) but which are ubiquitous in their language and
/// so are as uninteresting to track as keywords.
fn namespace_value_words(namespace: &str) -> &'static [&'static str] {
    match namespace {
        "cpp" => &["this", "nullptr", "NULL", "true", "false"],
        "js" | "web" => &["this", "super", "null", "undefined", "true", "false"],
        "webidl" | "udl" => &["true", "false", "null"],
        "config" => &["true", "false"],
        "json" => &["true", "false", "null"],
        "py" => &["self", "None", "True", "False"],
        "rust" => &[
            "self", "Self", "super", "crate", "true", "false", "Some", "None", "Ok", "Err",
        ],
        _ => &[],
    }
}

/// English words which are too common in comments (and plain text) to be
/// worth tracking.  Compared case-insensitively.  This is only applied to
/// comment/text words so that, for example, Python's `is` and `not` keywords
/// or a variable named `from` are handled by their token class.  "bug" and
/// "see" are included because bug references ("See bug 1620052") are tracked
/// as `bug-NNNNNNN` keys instead.
const PROSE_STOPWORDS: &[&str] = &[
    "bug", "bugs", "see", "a", "about", "after", "all", "also", "an", "and", "any", "are", "as",
    "at", "be", "because", "been", "before", "but", "by", "can", "could", "do", "does", "doesn",
    "don", "each", "for", "from", "has", "have", "here", "how", "if", "in", "into", "is", "isn",
    "it", "its", "just", "may", "more", "most", "must", "need", "no", "not", "now", "of", "on",
    "one", "only", "or", "other", "our", "out", "should", "so", "some", "such", "than", "that",
    "the", "their", "them", "then", "there", "these", "they", "this", "those", "to", "up", "us",
    "use", "used", "using", "want", "was", "way", "we", "were", "what", "when", "where", "whether",
    "which", "while", "who", "why", "will", "with", "would", "you",
];

pub fn is_prose_stopword(word: &str) -> bool {
    use std::sync::OnceLock;
    static SORTED: OnceLock<Vec<&'static [u8]>> = OnceLock::new();
    // (None is longer than 7 bytes, which most words are, so a word is
    // lowercased into a buffer and looked for in them, sorted, for every
    // comment's words.)
    const LONGEST: usize = 7;
    let bytes = word.as_bytes();
    if bytes.len() > LONGEST {
        return false;
    }
    let sorted = SORTED.get_or_init(|| {
        let mut sorted: Vec<&[u8]> = PROSE_STOPWORDS.iter().map(|stop| stop.as_bytes()).collect();
        assert!(sorted.iter().all(|stop| stop.len() <= LONGEST));
        sorted.sort();
        sorted
    });
    let mut lower = [0u8; LONGEST];
    for (to, from) in lower.iter_mut().zip(bytes) {
        *to = from.to_ascii_lowercase();
    }
    sorted.binary_search(&&lower[..bytes.len()]).is_ok()
}

fn trim_punctuation(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
}

fn is_bug_digits(digits: &str) -> bool {
    (4..=8).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
}

/// Recognize bug references like "Bug 1620052" (where `prev` is "Bug"),
/// "bug1620052", "bug-1620052", and bugzilla `show_bug.cgi?id=1620052` URLs,
/// returning the bug number.
fn bug_number<'a>(token: &'a str, prev: Option<&str>) -> Option<&'a str> {
    // (Looking for a `?` first is cheaper than for the URL's path in every
    // token.)
    if token.as_bytes().contains(&b'?')
        && let Some(idx) = token.find("show_bug.cgi?id=")
    {
        let rest = &token[idx + "show_bug.cgi?id=".len()..];
        let digits = &rest[..rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len())];
        return (!digits.is_empty()).then_some(digits);
    }
    let word = trim_punctuation(token);
    // (`get` avoids panicking when byte 3 isn't a char boundary, ex: "It’s".)
    if let Some(prefix) = word.get(..3)
        && prefix.eq_ignore_ascii_case("bug")
    {
        let digits = word[3..].trim_start_matches(['-', '_']);
        if is_bug_digits(digits) {
            return Some(digits);
        }
    }
    let prev_is_bug = prev.is_some_and(|p| trim_punctuation(p).eq_ignore_ascii_case("bug"));
    if prev_is_bug && is_bug_digits(word) {
        return Some(word);
    }
    None
}

/// The parts of a qualified name, `Foo::Bar`, `a->b` or `a.b` (a destructor's
/// `~` aside), if `word` is one.
fn qualified_parts(word: &str) -> Option<Vec<&str>> {
    if !word.contains("::") && !word.contains("->") && !word.contains('.') {
        return None;
    }
    let parts: Vec<&str> = word
        .split("::")
        .flat_map(|part| part.split("->"))
        .flat_map(|part| part.split('.'))
        .map(|part| part.strip_prefix('~').unwrap_or(part))
        .collect();
    let identifier = |part: &&str| {
        part.starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$')
            && part
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    (parts.len() >= 2 && parts.iter().all(identifier)).then_some(parts)
}

/// A string literal's text without its quotes and prefix (ex: `u8"..."`,
/// `R"(...)"`, `'...'`, `` `...` ``), as C and C++ tokens have them (other
/// languages' grammars give the text without them).
fn string_text(token: &str) -> &str {
    let quote = token.find(['"', '\'', '`']);
    let token = match quote {
        Some(at) if at <= 3 && token[..at].chars().all(|c| c.is_ascii_alphanumeric()) => {
            &token[at..]
        }
        _ => token,
    };
    let token = token.trim_matches(['"', '\'', '`']);
    // (A raw string's delimiters.)
    token
        .strip_prefix('(')
        .and_then(|t| t.strip_suffix(')'))
        .unwrap_or(token)
}

/// Call `run` with each run of words in `text` joined by `-` or `.` (each
/// between two word characters) which has one: ex: `moz-icon` and `gtk-foo`
/// in `moz-icon://stock/gtk-foo`, `moz-box` in `-moz-box`, and
/// `network.proxy.type`.
fn joined_runs<'a>(text: &'a str, mut run: impl FnMut(&'a str)) {
    runs(text, false, |found, _| run(found));
}

/// Call `run` with each of `text`'s runs of words joined by `-` or `.` (see
/// `joined_runs`), and whether it has a joiner, and with each word which isn't
/// in one, if `singles`.
fn runs<'a>(text: &'a str, singles: bool, mut run: impl FnMut(&'a str, bool)) {
    if !singles && !text.contains(['-', '.']) {
        return;
    }
    // (The current run's start, whether it has a joiner, and where the joiner
    // after its last word character is, if one is.)
    let mut start = None;
    let mut joined = false;
    let mut joiner = None;
    for (at, c) in text.char_indices() {
        if is_word_char(c) {
            if start.is_none() {
                start = Some(at);
                joined = false;
            }
            joined |= joiner.take().is_some();
            continue;
        }
        let Some(from) = start else { continue };
        if joiner.is_none() && (c == '-' || c == '.') {
            joiner = Some(at);
            continue;
        }
        if joined || singles {
            run(&text[from..joiner.unwrap_or(at)], joined);
        }
        start = None;
        joiner = None;
    }
    if let Some(from) = start
        && (joined || singles)
    {
        run(&text[from..joiner.unwrap_or(text.len())], joined);
    }
}

/// Whether `run` is a compound of 3 or more parts joined by dots (and words
/// joined by hyphens), which starts like a word: ex: `network.proxy.type`,
/// `layout.css.moz-document.enabled` and `bugzilla.mozilla.org`, but not
/// `1.2.3`, or file names like `Foo.h` and members like `this.foo`.
fn is_dotted_compound(run: &str) -> bool {
    run.len() <= 128
        && run.starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$')
        && run.split('.').count() >= 3
        && run.split('.').all(|part| {
            !part.is_empty()
                && part
                    .split('-')
                    .all(|word| !word.is_empty() && word.chars().all(is_word_char))
        })
}

/// Words which V2's keys don't track as compounds' parts (see
/// `compound_keys`), any case: hosts' `www`, `w3`, `org` and `com`, which are
/// nearly always next to more distinctive parts (`www.mozilla.org`,
/// `www.w3.org`), and whose instances there would be most of theirs (in a 10%
/// sample of firefox's files, 98% of `www`'s, 99% of `w3`'s, 90% of `org`'s
/// and 75% of `com`'s: 17k, 11k, 38k and 7k; asuth agreed to leave them out).
const HOST_PARTS: &[&str] = &["www", "w3", "org", "com"];

/// Whether `word` is one of the `HOST_PARTS`.
pub fn is_host_part(word: &str) -> bool {
    HOST_PARTS
        .iter()
        .any(|part| part.eq_ignore_ascii_case(word))
}

/// How long a word of hex digits must be to be a hash (see `is_hex_hash`):
/// as long as hg's short revisions.
const HEX_HASH_MIN: usize = 12;
/// How many of a hash's first characters name its bucket (see `hex_bucket`).
const HEX_BUCKET_CHARS: usize = 4;
/// How many of an SRI digest's first characters name its bucket (see
/// `sri_digest`).
const B64_BUCKET_CHARS: usize = 2;

/// Whether `word` is a hash of hex digits, which V2's keys track by its
/// bucket (see `hex_bucket`): 12 or more hex digits, a digit and a letter
/// among them (ex: a revision `9631de8e5f1a`, a sha256 checksum), whatever
/// its first character.
pub fn is_hex_hash(word: &str) -> bool {
    word.len() >= HEX_HASH_MIN
        && word.bytes().all(|b| b.is_ascii_hexdigit())
        && word.bytes().any(|b| b.is_ascii_digit())
        && word.bytes().any(|b| b.is_ascii_alphabetic())
}

/// The key of the bucket of hashes (and UUIDs; see `is_uuid`) starting with
/// `hex`'s first 4 characters, which share its journal, ex: `~hex-9631` (no
/// other key has a `~`).  In the full firefox history's (V1's) journals, its
/// 319,424 hex ones (4.5% of them, but only those starting with a letter)
/// would be 24,576 buckets of 12 each (the median).
pub fn hex_bucket(hex: &str) -> String {
    format!("~hex-{}", hex[..HEX_BUCKET_CHARS].to_ascii_lowercase())
}

/// Whether `run` is a UUID (`8-4-4-4-12` hex digits), which V2's keys track by
/// the bucket of its first group's characters (see `hex_bucket`), not by its
/// parts.
fn is_uuid(run: &str) -> bool {
    let mut groups = run.split('-');
    [8, 4, 4, 4, 12].iter().all(|&len| {
        groups
            .next()
            .is_some_and(|group| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
    }) && groups.next().is_none()
}

/// An SRI integrity value's algorithm and digest (its base64's first word),
/// ex: `sha512` and `UzGt8zg7` in package-lock.json's
/// `"sha512-UzGt8zg7Ny8d/jbY+…"`, which V2's keys track as the algorithm and
/// the bucket of the digest's first 2 characters (`~b64-Uz`; see
/// `b64_bucket`), rather than as a compound and a word each unique.
fn sri_digest(run: &str) -> Option<(&str, &str)> {
    let (algorithm, digest) = run.split_once('-')?;
    (["sha1", "sha256", "sha384", "sha512"]
        .iter()
        .any(|known| known.eq_ignore_ascii_case(algorithm))
        && !digest.is_empty()
        && !digest.contains(['-', '.']))
    .then_some((algorithm, digest))
}

/// The key of the bucket of SRI digests starting with `digest`'s first 2
/// characters (see `sri_digest`), ex: `~b64-Uz`.
pub fn b64_bucket(digest: &str) -> String {
    let end = digest
        .char_indices()
        .nth(B64_BUCKET_CHARS)
        .map_or(digest.len(), |(at, _)| at);
    format!("~b64-{}", &digest[..end])
}

/// The bucket whose journal V2's histories track `token` by, if it's a hash,
/// a UUID or an SRI value (see `is_hex_hash`, `is_uuid` and `sri_digest`).
pub fn hash_bucket(token: &str) -> Option<String> {
    if is_hex_hash(token) || is_uuid(token) {
        return Some(hex_bucket(token));
    }
    sri_digest(token).map(|(_, digest)| b64_bucket(digest))
}

/// The bug number of a compound of hyphens `bug-NNNNNNN` (any case), which is
/// the bug's key (see `bug_number`).
fn bug_compound(word: &str) -> Option<&str> {
    let (bug, digits) = word.split_once('-')?;
    (bug.eq_ignore_ascii_case("bug") && is_bug_digits(digits)).then_some(digits)
}

/// The keys of the compound words of `text` (V2's; see `tracked_token_keys`),
/// whole and each part, as postgres's text search parser gives a hyphenated
/// word and its parts, for `key`.  In each of `text`'s runs of words joined by
/// `-` or `.` (see `joined_runs`):
/// - a compound of 3 or more parts joined by dots (see `is_dotted_compound`),
///   whole, and each part that's a word and `tracked`;
/// - each compound of hyphens (the run, or its parts joined by dots: ex:
///   `moz-icon`, or `moz-document` in `layout.css.moz-document.enabled`) whole,
///   if one of its words could be a token (ex: not `x-1`), and each word that's
///   `tracked`, but a `bug-NNNNNNN` is the bug's key (see `bug_number`), if
///   `bugs`;
/// - but no part which is one of the `HOST_PARTS` (`www.mozilla.org` is
///   itself and `mozilla`).
///
/// And hashes, in runs or alone, are their buckets (see `hash_bucket`): a hex
/// hash (`is_hex_hash`), a UUID (`is_uuid`, not its parts), an SRI value's
/// digest (`sri_digest`, with its algorithm), and a compound with a hash isn't
/// one whole (ex: `0220-ca351238d719fd07` is `0220`'s and the hash's).
///
/// (2 parts joined by a dot aren't a compound, as file names and members
/// would be noise, ex: `foo.js`, `this.foo`, but a whole token which is a
/// qualified name has its parts; see `qualified_parts`.)
fn compound_keys<'a>(
    text: &'a str,
    tracked: &dyn Fn(&str) -> bool,
    bugs: bool,
    key: &mut impl FnMut(Cow<'a, str>),
) {
    let tracked = |word: &str| tracked(word) && !is_host_part(word);
    runs(text, true, |run, joined| {
        if !joined || is_uuid(run) {
            if is_hex_hash(run) || is_uuid(run) {
                key(Cow::Owned(hex_bucket(run)));
            }
            return;
        }
        if let Some((algorithm, digest)) = sri_digest(run) {
            if tracked(algorithm) {
                key(Cow::Borrowed(algorithm));
            }
            key(Cow::Owned(b64_bucket(digest)));
            return;
        }
        let hashes = run.split(['-', '.']).any(is_hex_hash) || run.split('.').any(is_uuid);
        let dotted = !hashes && is_dotted_compound(run);
        if dotted {
            key(Cow::Borrowed(run));
        }
        for part in run.split('.') {
            if !part.contains('-') {
                if is_hex_hash(part) {
                    key(Cow::Owned(hex_bucket(part)));
                } else if dotted && tracked(part) {
                    key(Cow::Borrowed(part));
                }
                continue;
            }
            if is_uuid(part) {
                key(Cow::Owned(hex_bucket(part)));
                continue;
            }
            if let Some(bug) = bug_compound(part) {
                if bugs {
                    key(Cow::Owned(format!("bug-{}", bug)));
                }
                continue;
            }
            if !part.split('-').any(is_hex_hash)
                && part.len() <= 128
                && part.split('-').any(is_trackable_token)
            {
                key(Cow::Borrowed(part));
            }
            for word in part.split('-') {
                if is_hex_hash(word) {
                    key(Cow::Owned(hex_bucket(word)));
                } else if tracked(word) {
                    key(Cow::Borrowed(word));
                }
            }
        }
    });
}

/// Whether `token` is a bug's key (see `bug_number`).
pub fn is_bug_key(token: &str) -> bool {
    token.strip_prefix("bug-").is_some_and(is_bug_digits)
}

/// The key of `token`'s journal, if it could have one (see `is_journal_key`):
/// itself, or without its leading hyphens, as compounds' keys are (ex: CSS's
/// `-moz-box-align`, `--my-var`).
pub fn journal_key(token: &str) -> Option<&str> {
    let key = match is_journal_key(token) {
        true => token,
        false => token.trim_start_matches('-'),
    };
    is_journal_key(key).then_some(key)
}

/// Whether `token` could have a token journal (see `tracked_token_keys`): a
/// word (see `is_trackable_token`), or a compound word, whose journals V2's
/// histories have (see `compound_keys`; ex: `moz-icon`,
/// `network.proxy.type`), or a bug's key (`bug-1620052`), or a hash, whose
/// bucket's journal V2's histories have (see `hash_bucket`).
pub fn is_journal_key(token: &str) -> bool {
    if is_trackable_token(token) || hash_bucket(token).is_some() {
        return true;
    }
    let mut whole = false;
    joined_runs(token, |run| whole |= run.len() == token.len());
    whole
        && (is_dotted_compound(token)
            || !token.contains('.')
                && token.len() <= 128
                && token.split('-').any(is_trackable_token)
            || bug_compound(token).is_some())
}

/// Decide which keys a token is tracked under in the per-token timeline and
/// itemized under in per-symbol `token_changes`, if it's interesting enough,
/// calling `key` for each: usually one, but the parts of a qualified name in a
/// comment or a string are tracked (ex: `SystemGlobal::Resolve`, or
/// `"SystemGlobal.h"`, which a search for `SystemGlobal` finds).  `prev` is the
/// preceding token in the file, if any, which is used to recognize "Bug
/// NNNNNNN" references.
///
/// - Keywords, operators, number literals, and boilerplate (license headers
///   and modelines) are never tracked.
/// - Identifiers and words in strings must look like identifiers (see
///   `is_trackable_token`) and not be one of the namespace's ubiquitous value
///   words like `this` or `self`.  A string literal's quotes don't count (C
///   and C++ tokens have them).
/// - Words in comments and plain text have surrounding punctuation trimmed,
///   and must look like identifiers and not be English stopwords.
/// - The parts of qualified names (`Foo::Bar`, `a->b`, `a.b`; see
///   `qualified_parts`) in strings, comments and plain text are tracked as
///   their words would be.  Not URLs and paths, nor prose in strings: in a 1%
///   sample of firefox-shared's files (2026-10-21), the qualified names and
///   quoted identifiers added ~3% to the ~1.1M instances tracked, where
///   comments' URLs and paths would add ~11% and strings' other words ~8%.
/// - Bug references in comments, plain text, and strings are tracked under a
///   `bug-NNNNNNN` key, which can't collide with identifiers.
/// - With V2's rules (see `KeyRules`), compound words in strings, comments,
///   plain text, and identifiers which aren't words are tracked whole and by
///   each of their parts (see `compound_keys`): ex: `"moz-icon://stock"` as
///   `moz-icon`, `moz` and `icon`, `"layout.css.moz-document.enabled"` as
///   itself, `layout`, `css`, `moz-document`, `moz`, `document` and
///   `enabled`, and CSS's `-moz-box-align` as `moz-box-align`, `moz`, `box`
///   and `align`, so that a part's journal has its uses in compounds too.  In
///   a 10% sample of firefox's files (2026-11-06), where 11.0M instances were
///   tracked, the compounds added 5.5% and their parts 12.3% (identifiers' in
///   CSS and markup 4.3%: `color`, `background`), where URLs' paths' words
///   would add ~1.6% and strings' other words ~3% more (ex: `0px`,
///   `doesn't`), and whole URLs 0.4%, mostly unique.
/// - With V2's rules, hashes are tracked by buckets of them (see
///   `hash_bucket`), in code, strings (their words: ex: a URL's revision),
///   comments and plain text: hex ones (`9631de8e5f1a…` as `~hex-9631`), UUIDs
///   and SRI values (`sha512-UzGt…` as `sha512` and `~b64-Uz`), so that
///   they're searchable without a journal each.
///
/// Lines lacking a class (from older syntax repos) are treated as identifiers.
pub fn tracked_token_keys<'a>(
    rules: KeyRules,
    namespace: &str,
    line: &TokenLine<'a>,
    prev: Option<&TokenLine>,
    key: &mut impl FnMut(Cow<'a, str>),
) {
    let class = match line.class {
        TokenClass::Unknown => TokenClass::Identifier,
        class => class,
    };
    let value_word = |word: &str| namespace_value_words(namespace).contains(&word);
    let compounds = rules >= KeyRules::V2;
    match class {
        TokenClass::Keyword
        | TokenClass::Operator
        | TokenClass::Number
        | TokenClass::Boilerplate => {}
        TokenClass::Identifier => {
            let token = line.token;
            if compounds && is_hex_hash(token) {
                key(Cow::Owned(hex_bucket(token)));
            } else if is_trackable_token(token) {
                if !value_word(token) {
                    key(Cow::Borrowed(token));
                }
            } else if compounds {
                compound_keys(
                    token,
                    &|word| is_trackable_token(word) && !value_word(word),
                    true,
                    key,
                );
            }
        }
        TokenClass::String | TokenClass::Comment | TokenClass::Text => {
            let (word, prose) = match class {
                TokenClass::String if is_trackable_token(line.token) => (line.token, false),
                TokenClass::String => (string_text(line.token), false),
                _ => (trim_punctuation(line.token), true),
            };
            let tracked = |word: &str| {
                is_trackable_token(word) && !(prose && is_prose_stopword(word)) && !value_word(word)
            };
            if let Some(bug) = bug_number(line.token, prev.map(|p| p.token)) {
                key(Cow::Owned(format!("bug-{}", bug)));
                // (Ex: a bug's URL's host, `bugzilla.mozilla.org`.)
                if compounds {
                    compound_keys(word, &tracked, false, key);
                }
                return;
            }
            if compounds && is_hex_hash(word) {
                key(Cow::Owned(hex_bucket(word)));
            } else if tracked(word) {
                key(Cow::Borrowed(word));
            } else if let Some(parts) = qualified_parts(word) {
                let hashes = compounds && parts.iter().any(|part| is_hex_hash(part));
                for part in &parts {
                    if hashes && is_hex_hash(part) {
                        key(Cow::Owned(hex_bucket(part)));
                    } else if tracked(part) && !(compounds && is_host_part(part)) {
                        key(Cow::Borrowed(part));
                    }
                }
                if compounds && !hashes && is_dotted_compound(word) {
                    key(Cow::Borrowed(word));
                }
            } else if compounds {
                compound_keys(word, &tracked, true, key);
            }
        }
        TokenClass::Unknown => unreachable!(),
    }
}

/// Derive the path of the token's timeline file relative to the timeline root,
/// ex: "nsresult" => "tokens/84/da/nsresult.ndjson", where "84da" starts the
/// git blob id of the lowercased token (which `printf %s nsresult | git
/// hash-object --stdin` also computes), so case variants share a directory.
pub fn token_timeline_path(token: &str) -> PathBuf {
    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, token.to_lowercase().as_bytes())
        .unwrap()
        .to_string();
    let mut path = PathBuf::from("tokens");
    path.push(&hash[0..2]);
    path.push(&hash[2..4]);
    path.push(format!("{}.ndjson", token));
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::syntax_files::{TokenClass, TokenLine};

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

    fn keys(namespace: &str, class: TokenClass, token: &str, prev: Option<&str>) -> Vec<String> {
        keys_by(KeyRules::V1, namespace, class, token, prev)
    }

    fn keys_by(
        rules: KeyRules,
        namespace: &str,
        class: TokenClass,
        token: &str,
        prev: Option<&str>,
    ) -> Vec<String> {
        let line = TokenLine {
            context: "%",
            class,
            token,
        };
        let prev = prev.map(|p| TokenLine {
            context: "%",
            class,
            token: p,
        });
        let mut keys = vec![];
        tracked_token_keys(rules, namespace, &line, prev.as_ref(), &mut |k| {
            keys.push(k.into_owned())
        });
        keys
    }

    /// The only key a token has, if any.
    fn key(namespace: &str, class: TokenClass, token: &str, prev: Option<&str>) -> Option<String> {
        let keys = keys(namespace, class, token, prev);
        assert!(keys.len() <= 1, "{:?}", keys);
        keys.into_iter().next()
    }

    #[test]
    fn test_tracked_token_key() {
        use TokenClass::*;
        let some = |s: &str| Some(s.to_string());
        // Classes that are never tracked.
        assert_eq!(key("rust", Keyword, "match", None), None);
        assert_eq!(key("cpp", Operator, "::", None), None);
        assert_eq!(key("cpp", Number, "1620052", None), None);
        assert_eq!(key("cpp", Boilerplate, "License", None), None);
        // Identifiers, minus the namespace's ubiquitous values.
        assert_eq!(key("cpp", Identifier, "mCount", None), some("mCount"));
        assert_eq!(key("cpp", Identifier, "this", None), None);
        assert_eq!(key("py", Identifier, "self", None), None);
        assert_eq!(key("js", Identifier, "self", None), some("self"));
        assert_eq!(key("rust", Identifier, "Some", None), None);
        assert_eq!(key("cpp", Identifier, "Some", None), some("Some"));
        // Comment words get trimmed and stopwords are dropped.
        assert_eq!(key("cpp", Comment, "The", None), None);
        assert_eq!(key("cpp", Comment, "the", None), None);
        assert_eq!(key("cpp", Comment, "Bug", None), None);
        assert_eq!(
            key("cpp", Comment, "ServiceWorker,", None),
            some("ServiceWorker")
        );
        assert_eq!(key("cpp", Comment, "`Foo()`.", None), some("Foo"));
        // Qualified names' parts, but not URLs, paths, or stopwords.
        let words = |w: &[&str]| w.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(
            keys("cpp", Comment, "mozilla::dom", None),
            words(&["mozilla", "dom"])
        );
        assert_eq!(
            keys("cpp", Comment, "`SystemGlobal::Resolve()`,", None),
            words(&["SystemGlobal", "Resolve"])
        );
        assert_eq!(
            keys("cpp", Comment, "Foo::~Foo", None),
            words(&["Foo", "Foo"])
        );
        assert_eq!(
            keys("cpp", Comment, "mWidget->Frob", None),
            words(&["mWidget", "Frob"])
        );
        assert_eq!(keys("cpp", Comment, "this->mFoo", None), words(&["mFoo"]));
        assert_eq!(keys("cpp", Comment, "the.end", None), words(&["end"]));
        assert_eq!(
            keys("cpp", Comment, "https://example.com/foo", None),
            words(&[])
        );
        assert_eq!(keys("cpp", Comment, "dom/base/Foo.cpp", None), words(&[]));
        assert_eq!(keys("cpp", Comment, "e.g.", None), words(&[]));
        assert_eq!(key("none", Text, "skip-if", None), None);
        assert_eq!(key("config", Text, "true", None), None);
        assert_eq!(key("config", Text, "condprof", None), some("condprof"));
        // Bug references.
        assert_eq!(
            key("none", Text, "1620052", Some("Bug")),
            some("bug-1620052")
        );
        assert_eq!(
            key("cpp", Comment, "123456).", Some("(bug")),
            some("bug-123456")
        );
        assert_eq!(key("cpp", Comment, "bug1620052", None), some("bug-1620052"));
        assert_eq!(
            key("cpp", Comment, "Bug-1620052:", None),
            some("bug-1620052")
        );
        assert_eq!(
            key(
                "cpp",
                Comment,
                "https://bugzilla.mozilla.org/show_bug.cgi?id=1620052#c3",
                None
            ),
            some("bug-1620052")
        );
        assert_eq!(
            key("js", String, "1620052", Some("bug")),
            some("bug-1620052")
        );
        // Non-ASCII text doesn't cause problems.
        assert_eq!(key("cpp", Comment, "It’s", None), None);
        assert_eq!(key("cpp", Comment, "bu’g", None), None);
        assert_eq!(key("cpp", Comment, "Größe", None), some("Größe"));
        // Other numbers in comments aren't bug references.
        assert_eq!(key("cpp", Comment, "65536", Some("up")), None);
        // Words in strings must look like identifiers, quotes aside, or be
        // qualified names.
        assert_eq!(key("js", String, "error", None), some("error"));
        assert_eq!(
            key("cpp", String, "\"SystemGlobal\"", None),
            some("SystemGlobal")
        );
        assert_eq!(key("cpp", String, "u8\"Widget\"", None), some("Widget"));
        assert_eq!(key("cpp", String, "R\"(Widget)\"", None), some("Widget"));
        assert_eq!(key("py", String, "'Widget'", None), some("Widget"));
        assert_eq!(
            keys("cpp", String, "\"SystemGlobal.h\"", None),
            words(&["SystemGlobal"])
        );
        assert_eq!(
            keys("cpp", String, "\"dom.foo\"", None),
            words(&["dom", "foo"])
        );
        assert_eq!(keys("cpp", String, "\"Hello, world\"", None), words(&[]));
        assert_eq!(keys("cpp", String, "\"*SystemGlobal*\"", None), words(&[]));
        assert_eq!(
            keys("cpp", String, "\"@mozilla.org/foo;1\"", None),
            words(&[])
        );
        assert_eq!(key("cpp", String, "\"this\"", None), None);
        // Lines from older syntax repos lack a class.
        assert_eq!(key("cpp", Unknown, "mCount", None), some("mCount"));
        // V1 has no compounds.
        assert_eq!(keys("cpp", String, "\"moz-icon\"", None), words(&[]));
        assert_eq!(key("css", Identifier, "background-color", None), None);
    }

    #[test]
    fn test_compound_keys() {
        use TokenClass::*;
        let keys = |namespace: &str, class: TokenClass, token: &str, prev: Option<&str>| {
            keys_by(KeyRules::V2, namespace, class, token, prev)
        };
        let words = |w: &[&str]| w.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        // Compounds of hyphens, whole and by their words.
        assert_eq!(
            keys("cpp", String, "\"moz-icon\"", None),
            words(&["moz-icon", "moz", "icon"])
        );
        assert_eq!(
            keys(
                "cpp",
                String,
                "u8\"moz-icon://stock/gtk-foo?size=16\"",
                None
            ),
            words(&["moz-icon", "moz", "icon", "gtk-foo", "gtk", "foo"])
        );
        assert_eq!(
            keys("cpp", Comment, "SchemeIs(\"moz-icon\")", None),
            words(&["moz-icon", "moz", "icon"])
        );
        assert_eq!(
            keys("css", Identifier, "-moz-box-align", None),
            words(&["moz-box-align", "moz", "box", "align"])
        );
        assert_eq!(
            keys("css", Identifier, "background-color", None),
            words(&["background-color", "background", "color"])
        );
        assert_eq!(
            keys("css", Identifier, "--my-var", None),
            words(&["my-var", "my", "var"])
        );
        // (Markup's attribute values are identifiers.)
        assert_eq!(
            keys("html", Identifier, "moz-icon://stock/foo?size=16", None),
            words(&["moz-icon", "moz", "icon"])
        );
        assert_eq!(
            keys("none", Identifier, "utf-8", None),
            words(&["utf-8", "utf"])
        );
        // (Prose's stopwords aren't words of compounds either.)
        assert_eq!(
            keys("cpp", Comment, "out-of-date,", None),
            words(&["out-of-date", "date"])
        );
        assert_eq!(
            keys("none", Text, "skip-if", None),
            words(&["skip-if", "skip"])
        );
        // Compounds of dots, of 3 or more parts.
        assert_eq!(
            keys("js", String, "\"network.proxy.type\"", None),
            words(&["network", "proxy", "type", "network.proxy.type"])
        );
        assert_eq!(
            keys("js", String, "\"layout.css.moz-document.enabled\"", None),
            words(&[
                "layout.css.moz-document.enabled",
                "layout",
                "css",
                "moz-document",
                "moz",
                "document",
                "enabled"
            ])
        );
        // (Pieces of a string, as Python's and Rust's tokens are.)
        assert_eq!(
            keys("py", String, "moz-icon://stock network.proxy.type", None),
            words(&[
                "moz-icon",
                "moz",
                "icon",
                "network.proxy.type",
                "network",
                "proxy",
                "type"
            ])
        );
        assert_eq!(
            keys("js", String, "\"foo.js\"", None),
            words(&["foo", "js"])
        );
        assert_eq!(keys("js", String, "\"see foo.js\"", None), words(&[]));
        assert_eq!(keys("js", String, "\"1.2.3\"", None), words(&[]));
        assert_eq!(keys("cpp", Comment, "x-1", None), words(&[]));
        assert_eq!(keys("cpp", Comment, "a--b", None), words(&[]));
        assert_eq!(keys("cpp", Comment, "e.g.", None), words(&[]));
        assert_eq!(keys("cpp", Comment, "-->", None), words(&[]));
        // (Not hosts' `www`, `w3`, `org` and `com`, V1's qualified names' parts
        // either.)
        assert_eq!(
            keys("cpp", String, "\"https://www.w3.org/TR/css-text-3/\"", None),
            words(&["www.w3.org", "css-text-3", "css", "text"])
        );
        assert_eq!(
            keys("js", String, "\"www.mozilla.org\"", None),
            words(&["mozilla", "www.mozilla.org"])
        );
        assert_eq!(
            keys_by(KeyRules::V1, "js", String, "\"www.mozilla.org\"", None),
            words(&["www", "mozilla", "org"])
        );
        assert_eq!(
            keys("cpp", String, "\"WWW-Authenticate\"", None),
            words(&["WWW-Authenticate", "Authenticate"])
        );
        // Bugs' keys, with the compounds of their URLs.
        assert_eq!(
            keys(
                "cpp",
                Comment,
                "https://bugzilla.mozilla.org/show_bug.cgi?id=1620052#c3",
                None
            ),
            words(&["bug-1620052", "bugzilla.mozilla.org", "bugzilla", "mozilla"])
        );
        assert_eq!(
            keys("cpp", Comment, "Bug-1620052:", None),
            words(&["bug-1620052"])
        );
        assert_eq!(
            keys("cpp", Comment, "(see:Bug-123456)", None),
            words(&["bug-123456"])
        );
    }

    #[test]
    fn test_hash_buckets() {
        use TokenClass::*;
        let keys = |namespace: &str, class: TokenClass, token: &str| {
            keys_by(KeyRules::V2, namespace, class, token, None)
        };
        let words = |w: &[&str]| w.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        // Hex hashes, whatever their first character, by their first 4.
        assert_eq!(keys("cpp", Comment, "9631de8e5f1a,"), words(&["~hex-9631"]));
        assert_eq!(
            keys("cpp", Identifier, "ABCDEF012345"),
            words(&["~hex-abcd"])
        );
        assert_eq!(
            keys_by(KeyRules::V1, "cpp", Identifier, "ABCDEF012345", None),
            words(&["ABCDEF012345"])
        );
        assert_eq!(
            keys(
                "cpp",
                Comment,
                "https://hg.mozilla.org/mozilla-central/rev/9631de8e5f1a"
            ),
            words(&[
                "hg.mozilla.org",
                "hg",
                "mozilla",
                "mozilla-central",
                "mozilla",
                "central",
                "~hex-9631"
            ])
        );
        assert_eq!(
            keys(
                "js",
                String,
                "\"Landed as 0f2a4938884c2ce7e2637363c5577f095ef0d074.\""
            ),
            words(&["~hex-0f2a"])
        );
        // (A compound with one isn't one whole.)
        assert_eq!(
            keys("cpp", String, "\"0220-ca351238d719fd07\""),
            words(&["~hex-ca35"])
        );
        assert_eq!(
            keys("js", String, "\"main.3fa4b2c1d9e0.js\""),
            words(&["~hex-3fa4"])
        );
        assert_eq!(
            keys("cpp", Comment, "obj.abcdef012345"),
            words(&["obj", "~hex-abcd"])
        );
        // UUIDs by their first group's, not their parts.
        assert_eq!(
            keys("cpp", String, "\"{d5b4b2a0-1234-4567-89ab-cdef01234567}\""),
            words(&["~hex-d5b4"])
        );
        assert_eq!(
            keys(
                "cpp",
                String,
                "\"0d7668a8-c3f4-cfee-cbc8-536511528937.json\""
            ),
            words(&["~hex-0d76"])
        );
        // SRI values by their algorithm and their digest's first 2.
        assert_eq!(
            keys("json", String, "\"sha512-UzGt8zg7Ny8d/jbY+abc==\""),
            words(&["sha512", "~b64-Uz"])
        );
        // Not shorter hex words, or those without a digit or a letter.
        assert_eq!(keys("cpp", String, "\"deadbeef\""), words(&["deadbeef"]));
        assert_eq!(
            keys("cpp", String, "\"abcdefabcdefab\""),
            words(&["abcdefabcdefab"])
        );
        assert_eq!(keys("cpp", Comment, "123456789012"), words(&[]));
        assert_eq!(
            hash_bucket("d5b4b2a0-1234-4567-89ab-cdef01234567").as_deref(),
            Some("~hex-d5b4")
        );
        assert_eq!(hash_bucket("sha256-Ab").as_deref(), Some("~b64-Ab"));
        assert_eq!(hash_bucket("sha256-A").as_deref(), Some("~b64-A"));
        assert_eq!(hash_bucket("cafe-1234-4567"), None);
        assert!(is_journal_key("9631de8e5f1a"));
    }

    #[test]
    fn test_journal_keys() {
        for token in [
            "nsresult",
            "moz-icon",
            "background-color",
            "network.proxy.type",
            "layout.css.moz-document.enabled",
            "bug-1620052",
            "utf-8",
        ] {
            assert!(is_journal_key(token), "{}", token);
        }
        for token in [
            "",
            "-moz-box",
            "moz-",
            "foo.js",
            "x-1",
            "a--b",
            "moz-icon://stock",
            "1.2.3",
            "foo bar",
        ] {
            assert!(!is_journal_key(token), "{}", token);
        }
        assert_eq!(journal_key("-moz-box-align"), Some("moz-box-align"));
        assert_eq!(journal_key("--my-var"), Some("my-var"));
        assert_eq!(journal_key("nsresult"), Some("nsresult"));
        assert_eq!(journal_key("-x"), None);
        assert!(is_bug_key("bug-1620052") && !is_bug_key("bug-fix") && !is_bug_key("Bug-1620052"));
    }

    #[test]
    fn test_paths() {
        assert_eq!(
            token_timeline_path("nsresult"),
            PathBuf::from("tokens/84/da/nsresult.ndjson")
        );
        assert_eq!(
            token_timeline_path("rv"),
            PathBuf::from("tokens/91/3c/rv.ndjson")
        );
        assert_eq!(
            token_timeline_path("RefPtr"),
            PathBuf::from("tokens/b5/24/RefPtr.ndjson")
        );
        assert_eq!(
            token_timeline_path("refptr"),
            PathBuf::from("tokens/b5/24/refptr.ndjson")
        );
    }
}
