//! A tokenizer for JSON data files (and JSON-lines, and JSON with comments),
//! whose contexts come from their structure, so that changes to them have
//! symbol-level history like code's: a member's tokens are in the context of
//! its key's path (ex: `dependencies::react` in a `package.json`), and the
//! tokens of a record, an object in an array or a JSON-lines line, are in
//! the context of its identity (see `IDENTITY_KEYS`), rather than of its
//! position, so that the history pairs records' changes by identity even when
//! records are added, removed, or reordered around them (ex: searchfox's
//! analysis records, `{"loc":...,"sym":"T_Foo",...}`, are in their symbols'
//! contexts).
//!
//! Tokens: keys (without their quotes) as identifiers (or words, if they're
//! prose), strings' words (without
//! their quotes) as text, numbers as numbers, `true`, `false`, and `null` as
//! identifiers, punctuation as operators, and comments' words.  Every token is
//! a verbatim substring of the source, in source order (but quotes aren't
//! tokens, like TOML's; see `config_tokenizer`).  Malformed input is
//! tokenized as words, in the context at it.

use std::collections::HashSet;

use crate::file_format::history::syntax_files::TokenClass;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::tree_sitter_support::boilerplate::{FinishedTokens, RawToken, finish_tokens};
use crate::tree_sitter_support::config_tokenizer::escape_name;

/// The keys whose (string or number) values identify a record, most preferred
/// first.
const IDENTITY_KEYS: &[&str] = &[
    "id", "guid", "uuid", "sym", "name", "key", "path", "url", "test", "slug",
];

/// How many segments contexts have at most: deeper members are in the
/// contexts of their ancestors at this depth, since every token's line has
/// its context (see `syntax_files.rs`).
const MAX_CONTEXT_SEGMENTS: usize = 3;

struct JsonTokenizer<'a> {
    src: &'a str,
    pos: usize,
    tokens: Vec<RawToken<'a>>,
    structure: Vec<FileStructureRow>,
    /// The structure rows' pretty identifiers so far, to not repeat them.
    rows: HashSet<String>,
}

fn context_of(path: &[String]) -> String {
    if path.is_empty() {
        "%".to_string()
    } else {
        path[..path.len().min(MAX_CONTEXT_SEGMENTS)].join("::")
    }
}

impl<'a> JsonTokenizer<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn push(&mut self, context: &str, class: TokenClass, text: &'a str) {
        if !text.is_empty() {
            self.tokens.push(RawToken {
                context: context.to_string(),
                class,
                text,
            });
        }
    }

    fn push_words(&mut self, context: &str, class: TokenClass, text: &'a str) {
        for word in text.split_whitespace() {
            self.push(context, class, word);
        }
    }

    fn row(&mut self, pretty: &str, kind: &str) {
        if self.rows.insert(pretty.to_string()) {
            self.structure.push(FileStructureRow {
                pretty: pretty.to_string(),
                is_def: true,
                kind: kind.to_string(),
                pp: vec![],
            });
        }
    }

    /// Skip whitespace and comments (`//` and `/* */`, whose words are
    /// tokens in `context`).
    fn skip_ws(&mut self, context: &str) {
        loop {
            while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
                self.pos += 1;
            }
            let rest = &self.src[self.pos..];
            let end = if rest.starts_with("//") {
                rest.find('\n').unwrap_or(rest.len())
            } else if let Some(body) = rest.strip_prefix("/*") {
                body.find("*/").map_or(rest.len(), |i| i + 4)
            } else {
                return;
            };
            let comment = &self.src[self.pos..self.pos + end];
            self.push_words(context, TokenClass::Comment, comment);
            self.pos += end;
        }
    }

    /// The contents of the string at `pos` (without its quotes), moving past
    /// it.  (Up to the end of the line, if it's unterminated.)
    fn string(&mut self) -> &'a str {
        let bytes = self.src.as_bytes();
        let start = self.pos + 1;
        let mut i = start;
        while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
            i += if bytes[i] == b'\\' { 2 } else { 1 };
        }
        let end = i.min(bytes.len());
        self.pos = if end < bytes.len() && bytes[end] == b'"' {
            end + 1
        } else {
            end
        };
        &self.src[start..end]
    }

    /// The bare word (ex: a number, `true`, or JSON5's unquoted key) at `pos`,
    /// moving past it.
    fn bare_word(&mut self) -> &'a str {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| !b.is_ascii_whitespace() && !b"{}[]:,\"/".contains(&b))
        {
            self.pos += 1;
        }
        if self.pos == start {
            // (A lone `/` or `:` out of place; take it as a word.)
            self.pos += self.src[start..].chars().next().map_or(1, char::len_utf8);
        }
        &self.src[start..self.pos]
    }

    fn value(&mut self, path: &mut Vec<String>) {
        let context = context_of(path);
        self.skip_ws(&context);
        match self.peek() {
            None => {}
            Some(b'{') => self.object(path),
            Some(b'[') => self.array(path),
            Some(b'"') => {
                let text = self.string();
                self.push_words(&context, TokenClass::Text, text);
            }
            Some(_) => {
                let word = self.bare_word();
                let class = match word {
                    "true" | "false" | "null" => TokenClass::Identifier,
                    // (Out of place.)
                    "]" | "}" | ":" | "," => TokenClass::Operator,
                    _ if word.starts_with(|c: char| c.is_ascii_digit() || c == '-') => {
                        TokenClass::Number
                    }
                    _ => TokenClass::Text,
                };
                self.push(&context, class, word);
            }
        }
    }

    fn object(&mut self, path: &mut Vec<String>) {
        let context = context_of(path);
        self.push(
            &context,
            TokenClass::Operator,
            &self.src[self.pos..self.pos + 1],
        );
        self.pos += 1;
        loop {
            self.skip_ws(&context);
            let Some(b) = self.peek() else {
                return;
            };
            match b {
                b'}' => {
                    self.push(
                        &context,
                        TokenClass::Operator,
                        &self.src[self.pos..self.pos + 1],
                    );
                    self.pos += 1;
                    return;
                }
                b',' => {
                    self.push(
                        &context,
                        TokenClass::Operator,
                        &self.src[self.pos..self.pos + 1],
                    );
                    self.pos += 1;
                }
                b']' => {
                    // (Mismatched; leave it to the enclosing array, if any.)
                    return;
                }
                _ => {
                    let key = if b == b'"' {
                        self.string()
                    } else {
                        self.bare_word()
                    };
                    path.push(escape_name(key));
                    let member = context_of(path);
                    if path.len() == 1 {
                        self.row(&member, "section");
                    }
                    // (Keys can be prose, ex: tests' names, whose words are
                    // tokens like INI's nested sections'.)
                    if key.contains(char::is_whitespace) {
                        self.push_words(&member, TokenClass::Text, key);
                    } else {
                        self.push(&member, TokenClass::Identifier, key);
                    }
                    self.skip_ws(&member);
                    if self.peek() == Some(b':') {
                        self.push(
                            &member,
                            TokenClass::Operator,
                            &self.src[self.pos..self.pos + 1],
                        );
                        self.pos += 1;
                        self.value(path);
                    }
                    path.pop();
                }
            }
        }
    }

    fn array(&mut self, path: &mut Vec<String>) {
        let context = context_of(path);
        self.push(
            &context,
            TokenClass::Operator,
            &self.src[self.pos..self.pos + 1],
        );
        self.pos += 1;
        loop {
            self.skip_ws(&context);
            match self.peek() {
                None => return,
                Some(b']') => {
                    self.push(
                        &context,
                        TokenClass::Operator,
                        &self.src[self.pos..self.pos + 1],
                    );
                    self.pos += 1;
                    return;
                }
                Some(b',') => {
                    self.push(
                        &context,
                        TokenClass::Operator,
                        &self.src[self.pos..self.pos + 1],
                    );
                    self.pos += 1;
                }
                Some(b'}') => {
                    // (Mismatched; take it as a word.)
                    let word = self.bare_word();
                    self.push(&context, TokenClass::Operator, word);
                }
                Some(_) => self.element(path, None),
            }
        }
    }

    /// An array's element (or a JSON-lines line), which is a record in the
    /// context of its identity if it's an object with one (and a structure
    /// row of `row_kind`, if any).
    fn element(&mut self, path: &mut Vec<String>, row_kind: Option<&str>) {
        let identity = (self.peek() == Some(b'{'))
            .then(|| self.identity())
            .flatten();
        match identity {
            Some(identity) => {
                path.push(escape_name(identity));
                if let Some(kind) = row_kind {
                    let pretty = context_of(path);
                    self.row(&pretty, kind);
                }
                self.value(path);
                path.pop();
            }
            None => self.value(path),
        }
    }

    /// The identity of the object at `pos` (see `IDENTITY_KEYS`), if it has
    /// one, without moving past it.
    fn identity(&self) -> Option<&'a str> {
        let bytes = self.src.as_bytes();
        let mut i = self.pos + 1;
        let mut depth = 0usize;
        // (The best identity so far, by its key's place in IDENTITY_KEYS.)
        let mut best: Option<(usize, &'a str)> = None;
        let mut expecting_key = true;
        let mut key: Option<&'a str> = None;
        let string_end = |start: usize| {
            let mut j = start + 1;
            while j < bytes.len() && bytes[j] != b'"' && bytes[j] != b'\n' {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            j.min(bytes.len())
        };
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    let end = string_end(i);
                    let text = &self.src[i + 1..end];
                    if depth == 0 {
                        if expecting_key {
                            key = Some(text);
                        } else if let Some(k) = key.take()
                            && let Some(rank) = IDENTITY_KEYS.iter().position(|id| *id == k)
                            && best.is_none_or(|(best_rank, _)| rank < best_rank)
                            && !text.is_empty()
                        {
                            best = Some((rank, text));
                        }
                    }
                    i = end + 1;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth == 0 => break,
                b'}' | b']' => depth -= 1,
                b':' if depth == 0 => expecting_key = false,
                b',' if depth == 0 => {
                    expecting_key = true;
                    key = None;
                }
                b if depth == 0 && !expecting_key && (b.is_ascii_digit() || b == b'-') => {
                    let end = bytes[i..]
                        .iter()
                        .position(|c| !(c.is_ascii_alphanumeric() || b"+-.".contains(c)))
                        .map_or(bytes.len(), |n| i + n);
                    if let Some(k) = key.take()
                        && let Some(rank) = IDENTITY_KEYS.iter().position(|id| *id == k)
                        && best.is_none_or(|(best_rank, _)| rank < best_rank)
                    {
                        best = Some((rank, &self.src[i..end]));
                    }
                    i = end;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        best.map(|(_, identity)| identity)
    }
}

/// Whether `source` is JSON-lines: its first two non-blank lines are objects.
fn is_json_lines(source: &str) -> bool {
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let is_object =
        |line: Option<&str>| line.is_some_and(|l| l.starts_with('{') && l.ends_with('}'));
    is_object(lines.next()) && is_object(lines.next())
}

pub fn tokenize_json(source: &str) -> (FinishedTokens, Vec<FileStructureRow>) {
    let mut tokenizer = JsonTokenizer {
        src: source,
        pos: 0,
        tokens: vec![],
        structure: vec![],
        rows: HashSet::new(),
    };
    let json_lines = is_json_lines(source);
    let mut path = vec![];
    loop {
        tokenizer.skip_ws("%");
        if tokenizer.pos >= source.len() {
            break;
        }
        let start = tokenizer.pos;
        if json_lines {
            tokenizer.element(&mut path, Some("record"));
        } else {
            tokenizer.value(&mut path);
        }
        // (Stray closing brackets and the like, after a value.)
        if tokenizer.pos == start {
            let word = tokenizer.bare_word();
            tokenizer.push("%", TokenClass::Operator, word);
        }
    }
    (finish_tokens(source, tokenizer.tokens), tokenizer.structure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<String> {
        tokenize_json(source).0.lines
    }

    fn structure(source: &str) -> Vec<String> {
        tokenize_json(source)
            .1
            .into_iter()
            .map(|row| format!("{}:{}", row.kind, row.pretty))
            .collect()
    }

    #[test]
    fn test_contexts() {
        let source = "{\n  \"name\": \"activity-stream\",\n  \"dependencies\": {\n    \"react\": \"19.2.0\",\n    \"react-dom\": \"19.2.0\"\n  },\n  \"private\": true,\n  \"files\": [\"a b\", 2]\n}\n";
        assert_eq!(
            tokens(source),
            vec![
                "% o {",
                "name i name",
                "name o :",
                "name t activity-stream",
                "% o ,",
                "dependencies i dependencies",
                "dependencies o :",
                "dependencies o {",
                "dependencies::react i react",
                "dependencies::react o :",
                "dependencies::react t 19.2.0",
                "dependencies o ,",
                "dependencies::react-dom i react-dom",
                "dependencies::react-dom o :",
                "dependencies::react-dom t 19.2.0",
                "dependencies o }",
                "% o ,",
                "private i private",
                "private o :",
                "private i true",
                "% o ,",
                "files i files",
                "files o :",
                "files o [",
                "files t a",
                "files t b",
                "files o ,",
                "files n 2",
                "files o ]",
                "% o }",
            ]
        );
        assert_eq!(
            structure(source),
            vec![
                "section:name",
                "section:dependencies",
                "section:private",
                "section:files"
            ]
        );
    }

    #[test]
    fn test_records() {
        // Records in arrays, and JSON-lines lines, are in their identities'
        // contexts (preferring `id` to `name`), however deep, up to
        // MAX_CONTEXT_SEGMENTS.
        let source =
            "{\"tests\": [{\"name\": \"b\", \"id\": 2, \"a\": {\"b\": {\"c\": 1}}}, {\"x\": 1}]}";
        let lines = tokens(source);
        assert!(
            lines.contains(&"tests::2::name i name".to_string()),
            "{:?}",
            lines
        );
        assert!(
            lines.contains(&"tests::2::a n 1".to_string()),
            "{:?}",
            lines
        );
        assert!(lines.contains(&"tests::x i x".to_string()), "{:?}", lines);
        let records = "{\"loc\":\"00001:0\",\"kind\":\"def\",\"sym\":\"FILE_a::b\"}\n{\"loc\":\"00002:3\",\"sym\":\"T_Foo\"}\n";
        let lines = tokens(records);
        assert_eq!(lines[0], "FILE_a%3A%3Ab o {");
        assert!(
            lines.contains(&"T_Foo::loc t 00002:3".to_string()),
            "{:?}",
            lines
        );
        assert_eq!(
            structure(records),
            vec!["record:FILE_a%3A%3Ab", "record:T_Foo"]
        );
    }

    #[test]
    fn test_comments_and_garbage() {
        let lines = tokens("// A comment.\n{\"a\": 1, /* b */ c: d}\n]");
        assert!(lines.contains(&"% c A".to_string()), "{:?}", lines);
        assert!(lines.contains(&"% c b".to_string()), "{:?}", lines);
        assert!(lines.contains(&"c t d".to_string()), "{:?}", lines);
        assert_eq!(lines.last().unwrap(), "% o ]");
        // Tokens are the source's text (but quotes and whitespace).
        for source in [
            "{\"a\": \"b\\\"c",
            "[1, {\"x\": [}",
            "\"unterminated\n{",
            "}}]]::",
        ] {
            let joined: String = tokens(source)
                .iter()
                .map(|line| line.splitn(3, ' ').nth(2).unwrap().to_string())
                .collect();
            let text: String = source
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '"')
                .collect();
            assert_eq!(joined.replace('"', ""), text, "{}", source);
        }
    }
}
