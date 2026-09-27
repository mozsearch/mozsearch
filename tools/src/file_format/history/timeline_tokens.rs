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
//! Only tokens selected by `tracked_token_key` get files, and we only emit a
//! record for a revision if the token was added/removed/evolved; pure moves are
//! not recorded here because they are noise for these use-cases, but they can
//! be found in the rev-summaries and files-delta records.  The same selection is
//! used for the per-symbol `token_changes` in files-delta records and
//! rev-summaries.
//!
//! The first line of each file is a `TokenHeader` and the remaining lines are
//! `TokenDeltaRecord`s ordered from newest to oldest.

use std::borrow::Cow;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::syntax_files::{TokenClass, TokenLine};

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
        && token
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && token.chars().any(|c| c.is_alphabetic())
}

/// Words which tree-sitter classifies as identifiers (or which we classify as
/// identifiers for consistency) but which are ubiquitous in their language and
/// so are as uninteresting to track as keywords.
fn namespace_value_words(namespace: &str) -> &'static [&'static str] {
    match namespace {
        "cpp" => &["this", "nullptr", "NULL", "true", "false"],
        "js" => &["this", "super", "null", "undefined", "true", "false"],
        "webidl" => &["true", "false", "null"],
        "config" => &["true", "false"],
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

fn is_prose_stopword(word: &str) -> bool {
    PROSE_STOPWORDS
        .iter()
        .any(|stop| stop.eq_ignore_ascii_case(word))
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
    if let Some(idx) = token.find("show_bug.cgi?id=") {
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

/// Decide whether a token is interesting enough to be tracked in the per-token
/// timeline and itemized in per-symbol `token_changes`, returning the key it is
/// tracked under.  `prev` is the preceding token in the file, if any, which is
/// used to recognize "Bug NNNNNNN" references.
///
/// - Keywords, operators, number literals, and boilerplate (license headers
///   and modelines) are never tracked.
/// - Identifiers and words in strings must look like identifiers (see
///   `is_trackable_token`) and not be one of the namespace's ubiquitous value
///   words like `this` or `self`.
/// - Words in comments and plain text have surrounding punctuation trimmed,
///   and must look like identifiers and not be English stopwords.
/// - Bug references in comments, plain text, and strings are tracked under a
///   `bug-NNNNNNN` key, which can't collide with identifiers.
///
/// Lines lacking a class (from older syntax repos) are treated as identifiers.
pub fn tracked_token_key<'a>(
    namespace: &str,
    line: &TokenLine<'a>,
    prev: Option<&TokenLine>,
) -> Option<Cow<'a, str>> {
    let class = match line.class {
        TokenClass::Unknown => TokenClass::Identifier,
        class => class,
    };
    match class {
        TokenClass::Keyword
        | TokenClass::Operator
        | TokenClass::Number
        | TokenClass::Boilerplate => None,
        TokenClass::Identifier => {
            let token = line.token;
            (is_trackable_token(token) && !namespace_value_words(namespace).contains(&token))
                .then_some(Cow::Borrowed(token))
        }
        TokenClass::String | TokenClass::Comment | TokenClass::Text => {
            if let Some(bug) = bug_number(line.token, prev.map(|p| p.token)) {
                return Some(Cow::Owned(format!("bug-{}", bug)));
            }
            if class == TokenClass::String {
                let token = line.token;
                return (is_trackable_token(token)
                    && !namespace_value_words(namespace).contains(&token))
                .then_some(Cow::Borrowed(token));
            }
            let word = trim_punctuation(line.token);
            (is_trackable_token(word)
                && !is_prose_stopword(word)
                && !namespace_value_words(namespace).contains(&word))
            .then_some(Cow::Borrowed(word))
        }
        TokenClass::Unknown => unreachable!(),
    }
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

    fn key(namespace: &str, class: TokenClass, token: &str, prev: Option<&str>) -> Option<String> {
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
        tracked_token_key(namespace, &line, prev.as_ref()).map(|k| k.into_owned())
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
        assert_eq!(key("cpp", Comment, "mozilla::dom", None), None);
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
        // Words in strings must look like identifiers.
        assert_eq!(key("js", String, "error", None), some("error"));
        assert_eq!(key("cpp", String, "\"dom.foo\"", None), None);
        // Lines from older syntax repos lack a class.
        assert_eq!(key("cpp", Unknown, "mCount", None), some("mCount"));
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
