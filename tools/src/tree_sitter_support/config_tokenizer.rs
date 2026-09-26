//! Tokenizers for INI and TOML files which produce equivalent tokens for
//! equivalent content, so that history can follow files which are converted
//! from one format to the other, like Firefox's conversion of its
//! manifestparser test manifests from `.ini` to `.toml`.  (We don't use
//! tree-sitter grammars because they would produce structurally different
//! tokens for the two formats.)
//!
//! Both formats are tokenized into:
//! - Section headers: `[`, the section name as a single identifier token, `]`.
//!   The context of the header tokens is the section name.  TOML quoted keys
//!   have their quotes dropped, so `["test_foo.html"]` and `[test_foo.html]`
//!   produce the same tokens.
//! - Keys: the key as an identifier and its `=` or `:` separator as an
//!   operator, with the values as whitespace-delimited text words.  The
//!   context of these tokens is `section::key`, so changes to a key like
//!   `skip-if` get their own symbol-level history.  TOML's array brackets,
//!   commas, and string quotes are purely syntax which doesn't exist in INI,
//!   so they don't produce tokens, which means `skip-if = ["os == 'win'"]` and
//!   `skip-if = os == 'win'` produce the same tokens.
//! - Comments: comment words, in the context of the section or key they
//!   appear in.
//!
//! Every token is a verbatim substring of the source and tokens are produced
//! in source order.
//!
//! For INI we support both manifestparser's format, where values can continue
//! on subsequent indented lines, and web-platform-tests' metadata format,
//! where sections nest by indentation and keys use `:`.

use crate::file_format::history::syntax_files::{TokenClass, format_token_line};
use crate::file_format::history::syntax_files_struct::FileStructureRow;

/// Escape a section or key name for use in a context: contexts can't contain
/// spaces (see `syntax_files.rs`) and "::" is the context delimiter.
fn escape_name(name: &str) -> String {
    name.replace('%', "%25")
        .replace(' ', "%20")
        .replace('\t', "%09")
        .replace("::", "%3A%3A")
}

struct Output {
    tokens: Vec<String>,
    structure: Vec<FileStructureRow>,
}

impl Output {
    fn push(&mut self, context: &str, class: TokenClass, token: &str) {
        if !token.is_empty() {
            self.tokens.push(format_token_line(context, class, token));
        }
    }

    fn push_words(&mut self, context: &str, class: TokenClass, text: &str) {
        for word in text.split_whitespace() {
            self.push(context, class, word);
        }
    }

    fn section(&mut self, pretty: &str) {
        self.structure.push(FileStructureRow {
            pretty: pretty.to_string(),
            is_def: true,
            kind: "section".to_string(),
        });
    }
}

fn join_context(section: &str, key: &str) -> String {
    let key = escape_name(key);
    if section.is_empty() || section == "%" {
        key
    } else {
        format!("{}::{}", section, key)
    }
}

/// Split INI value text into the value and an inline comment, which starts at
/// a `#` at the start of the value or preceded by whitespace.
fn split_inline_comment(text: &str) -> (&str, &str) {
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return (&text[..i], &text[i..]);
        }
    }
    (text, "")
}

pub fn tokenize_ini(source: &str) -> (Vec<String>, Vec<FileStructureRow>) {
    let mut out = Output {
        tokens: vec![],
        structure: vec![],
    };
    // Nested sections as (indentation, escaped name).
    let mut sections: Vec<(usize, String)> = vec![];
    let mut section_context = "%".to_string();
    // The key whose value may continue on more deeply indented lines, as
    // (indentation, context).
    let mut current_key: Option<(usize, String)> = None;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let continuing = current_key
            .as_ref()
            .filter(|(key_indent, _)| indent > *key_indent)
            .map(|(_, context)| context.clone());

        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            let context = continuing.as_deref().unwrap_or(&section_context);
            out.push_words(context, TokenClass::Comment, trimmed);
            continue;
        }

        if trimmed.starts_with('[') && continuing.is_none() {
            let close = trimmed.rfind(']');
            let (name, rest) = match close {
                Some(end) => (&trimmed[1..end], &trimmed[end + 1..]),
                None => (&trimmed[1..], ""),
            };
            while sections.last().is_some_and(|(i, _)| *i >= indent) {
                sections.pop();
            }
            sections.push((indent, escape_name(name)));
            section_context = sections
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<_>>()
                .join("::");
            out.section(&section_context);
            out.push(&section_context, TokenClass::Operator, "[");
            out.push(&section_context, TokenClass::Identifier, name.trim());
            if close.is_some() {
                out.push(&section_context, TokenClass::Operator, "]");
            }
            let (_, comment) = split_inline_comment(rest);
            out.push_words(&section_context, TokenClass::Comment, comment);
            current_key = None;
            continue;
        }

        if let Some(context) = continuing {
            let (value, comment) = split_inline_comment(trimmed);
            out.push_words(&context, TokenClass::Text, value);
            out.push_words(&context, TokenClass::Comment, comment);
            continue;
        }

        match trimmed.find(['=', ':']) {
            Some(sep) => {
                let key = trimmed[..sep].trim();
                let context = join_context(&section_context, key);
                out.push(&context, TokenClass::Identifier, key);
                out.push(&context, TokenClass::Operator, &trimmed[sep..sep + 1]);
                let (value, comment) = split_inline_comment(&trimmed[sep + 1..]);
                out.push_words(&context, TokenClass::Text, value);
                out.push_words(&context, TokenClass::Comment, comment);
                current_key = Some((indent, context));
            }
            None => {
                // Not a key; just words.
                let (value, comment) = split_inline_comment(trimmed);
                out.push_words(&section_context, TokenClass::Text, value);
                out.push_words(&section_context, TokenClass::Comment, comment);
                current_key = None;
            }
        }
    }

    (out.tokens, out.structure)
}

struct TomlTokenizer<'a> {
    src: &'a str,
    pos: usize,
    out: Output,
    table_context: String,
}

impl<'a> TomlTokenizer<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src.as_bytes()[self.pos..].starts_with(s.as_bytes())
    }

    /// Skip spaces and tabs (and newlines if `newlines`).
    fn skip_ws(&mut self, newlines: bool) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || (newlines && (b == b'\n' || b == b'\r')) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Consume a comment through the end of the line.
    fn comment(&mut self, context: &str) {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'\n' {
                break;
            }
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        self.out.push_words(context, TokenClass::Comment, text);
    }

    /// Skip whitespace, newlines, and comments (emitting the comment words).
    fn skip_ws_and_comments(&mut self, context: &str) {
        loop {
            self.skip_ws(true);
            if self.peek() == Some(b'#') {
                self.comment(context);
            } else {
                break;
            }
        }
    }

    /// Advance past one character, which is necessary for error recovery.
    fn advance_char(&mut self) {
        let len = self.src[self.pos..]
            .chars()
            .next()
            .map_or(1, |c| c.len_utf8());
        self.pos += len;
    }

    /// Parse a string starting at the current quote, returning the (verbatim)
    /// contents.
    fn string(&mut self) -> &'a str {
        let quote = self.peek().unwrap();
        let triple = if quote == b'"' { "\"\"\"" } else { "'''" };
        let multiline = self.starts_with(triple);
        self.pos += if multiline { 3 } else { 1 };
        let start = self.pos;
        loop {
            match self.peek() {
                None => return &self.src[start..self.pos],
                Some(b'\\') if quote == b'"' => {
                    self.pos += 1;
                    if self.peek().is_some() {
                        self.advance_char();
                    }
                }
                Some(b'\n') if !multiline => return &self.src[start..self.pos],
                Some(b) if b == quote => {
                    if !multiline {
                        let content = &self.src[start..self.pos];
                        self.pos += 1;
                        return content;
                    }
                    if self.starts_with(triple) {
                        // A closing triple quote can be followed by up to 2
                        // more quotes which are part of the content.
                        let mut end = self.pos;
                        while self.src.as_bytes().get(end + 3) == Some(&quote) {
                            end += 1;
                        }
                        let content = &self.src[start..end];
                        self.pos = end + 3;
                        return content;
                    }
                    self.pos += 1;
                }
                Some(_) => self.advance_char(),
            }
        }
    }

    /// Parse a (possibly dotted, possibly quoted) key, returning its segments.
    fn key(&mut self) -> Vec<&'a str> {
        let mut segments = vec![];
        loop {
            self.skip_ws(false);
            match self.peek() {
                Some(b'"') | Some(b'\'') => segments.push(self.string()),
                Some(_) => {
                    let start = self.pos;
                    while let Some(b) = self.peek() {
                        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b >= 0x80 {
                            self.advance_char();
                        } else {
                            break;
                        }
                    }
                    if self.pos == start {
                        break;
                    }
                    segments.push(&self.src[start..self.pos]);
                }
                None => break,
            }
            self.skip_ws(false);
            if self.peek() == Some(b'.') {
                self.pos += 1;
            } else {
                break;
            }
        }
        segments
    }

    /// Parse a value, emitting its words in the given context.
    fn value(&mut self, context: &str, depth: usize) {
        self.skip_ws(false);
        match self.peek() {
            Some(b'"') | Some(b'\'') => {
                let content = self.string();
                self.out.push_words(context, TokenClass::Text, content);
            }
            Some(b'[') if depth < 64 => {
                self.pos += 1;
                loop {
                    self.skip_ws_and_comments(context);
                    match self.peek() {
                        None => break,
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        Some(b',') => self.pos += 1,
                        Some(_) => {
                            let before = self.pos;
                            self.value(context, depth + 1);
                            if self.pos == before {
                                self.advance_char();
                            }
                        }
                    }
                }
            }
            Some(b'{') if depth < 64 => {
                self.pos += 1;
                loop {
                    self.skip_ws(false);
                    match self.peek() {
                        None | Some(b'\n') => break,
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        Some(b',') => self.pos += 1,
                        Some(_) => {
                            let before = self.pos;
                            let segments = self.key();
                            for segment in &segments {
                                self.out.push(context, TokenClass::Identifier, segment);
                            }
                            self.skip_ws(false);
                            if self.peek() == Some(b'=') {
                                self.out.push(context, TokenClass::Operator, "=");
                                self.pos += 1;
                                self.value(context, depth + 1);
                            }
                            if self.pos == before {
                                self.advance_char();
                            }
                        }
                    }
                }
            }
            Some(_) => {
                // A bare scalar (number, boolean, date) runs until whitespace
                // or structural punctuation.
                let start = self.pos;
                while let Some(b) = self.peek() {
                    if b.is_ascii_whitespace() || matches!(b, b',' | b']' | b'}' | b'#') {
                        break;
                    }
                    self.advance_char();
                }
                self.out
                    .push(context, TokenClass::Text, &self.src[start..self.pos]);
            }
            None => {}
        }
    }

    /// Consume the rest of the line, which should only contain a comment, but
    /// emit anything unexpected as text.
    fn end_of_line(&mut self, context: &str) {
        loop {
            self.skip_ws(false);
            match self.peek() {
                None | Some(b'\n') => return,
                Some(b'#') => {
                    self.comment(context);
                    return;
                }
                Some(_) => {
                    let start = self.pos;
                    while let Some(b) = self.peek() {
                        if b.is_ascii_whitespace() || b == b'#' {
                            break;
                        }
                        self.advance_char();
                    }
                    let text = &self.src[start..self.pos];
                    self.out.push(context, TokenClass::Text, text);
                }
            }
        }
    }

    fn table_header(&mut self) {
        let double = self.starts_with("[[");
        let (open, close) = if double { ("[[", "]]") } else { ("[", "]") };
        self.pos += open.len();
        let segments = self.key();
        let name = segments.join(".");
        self.table_context = escape_name(&name);
        let context = self.table_context.clone();
        self.out.section(&context);
        self.out.push(&context, TokenClass::Operator, open);
        for segment in &segments {
            self.out.push(&context, TokenClass::Identifier, segment);
        }
        self.skip_ws(false);
        if self.starts_with(close) {
            self.out.push(&context, TokenClass::Operator, close);
            self.pos += close.len();
        }
        self.end_of_line(&context);
    }

    fn key_value(&mut self) {
        let segments = self.key();
        if segments.is_empty() {
            // Garbage; emit what's there as text and move on.
            let context = self.table_context.clone();
            self.end_of_line(&context);
            if self.peek().is_some() && self.peek() != Some(b'\n') {
                self.advance_char();
            }
            return;
        }
        let context = join_context(&self.table_context, &segments.join("."));
        for segment in &segments {
            self.out.push(&context, TokenClass::Identifier, segment);
        }
        self.skip_ws(false);
        if self.peek() == Some(b'=') {
            self.out.push(&context, TokenClass::Operator, "=");
            self.pos += 1;
            self.value(&context, 0);
        }
        self.end_of_line(&context);
    }

    fn run(mut self) -> (Vec<String>, Vec<FileStructureRow>) {
        loop {
            let context = self.table_context.clone();
            self.skip_ws_and_comments(&context);
            match self.peek() {
                None => break,
                Some(b'[') => self.table_header(),
                Some(_) => {
                    let before = self.pos;
                    self.key_value();
                    if self.pos == before {
                        self.advance_char();
                    }
                }
            }
        }
        (self.out.tokens, self.out.structure)
    }
}

pub fn tokenize_toml(source: &str) -> (Vec<String>, Vec<FileStructureRow>) {
    TomlTokenizer {
        src: source,
        pos: 0,
        out: Output {
            tokens: vec![],
            structure: vec![],
        },
        table_context: "%".to_string(),
    }
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ini(source: &str) -> Vec<String> {
        tokenize_ini(source).0
    }

    fn toml(source: &str) -> Vec<String> {
        tokenize_toml(source).0
    }

    #[test]
    fn test_manifest_equivalence() {
        // From Firefox's conversion of dom/serviceworkers/test/mochitest.ini.
        let before = "[DEFAULT]\n\
            # Comment about prefs.\n\
            prefs =\n  network.cookie.cookieBehavior=0\n\
            dupe-manifest = true\n\
            tags = condprof\n\
            \n\
            [test_eventsource_intercept.html]\n\
            skip-if =\n  http3\n  http2\n\
            [test_https_fetch.html]\n\
            skip-if = condprof  #: timed out\n\
            [test_openWindow.html]\n\
            skip-if =\n  toolkit == 'android' # Bug 1620052\n  xorigin # Bug 1792790\n";
        let after = "[DEFAULT]\n\
            # Comment about prefs.\n\
            prefs = [\"network.cookie.cookieBehavior=0\"]\n\
            dupe-manifest = true\n\
            tags = \"condprof\"\n\
            \n\
            [\"test_eventsource_intercept.html\"]\n\
            skip-if = [\n  \"http3\",\n  \"http2\",\n]\n\
            \n\
            [\"test_https_fetch.html\"]\n\
            skip-if = [\"condprof\"]  #: timed out\n\
            \n\
            [\"test_openWindow.html\"]\n\
            skip-if = [\n  \"os == 'android'\", # Bug 1620052\n  \"xorigin\", # Bug 1792790\n]\n";
        let before_tokens = ini(before);
        let after_tokens = toml(after);
        // The only difference should be the condition the conversion rewrote.
        let diffs: Vec<(&String, &String)> = before_tokens
            .iter()
            .zip(after_tokens.iter())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(before_tokens.len(), after_tokens.len());
        assert_eq!(
            diffs,
            vec![(
                &"test_openWindow.html::skip-if t toolkit".to_string(),
                &"test_openWindow.html::skip-if t os".to_string()
            )]
        );
        assert!(before_tokens.contains(&"test_https_fetch.html::skip-if c #:".to_string()));
        assert!(after_tokens.contains(&"test_https_fetch.html o [".to_string()));
        assert!(after_tokens.contains(&"test_openWindow.html::skip-if c 1792790".to_string()));

        let structure = |rows: Vec<FileStructureRow>| -> Vec<String> {
            rows.into_iter().map(|r| r.pretty).collect()
        };
        assert_eq!(
            structure(tokenize_ini(before).1),
            structure(tokenize_toml(after).1)
        );
    }

    #[test]
    fn test_wpt_metadata() {
        let source = "[test.html]\n  expected: ERROR\n  [Some subtest: with spaces]\n    expected:\n      if os == \"win\": FAIL\n      PASS\n  [Another::subtest]\n    expected: FAIL\n";
        let (tokens, structure) = tokenize_ini(source);
        assert_eq!(
            structure
                .iter()
                .map(|r| r.pretty.as_str())
                .collect::<Vec<_>>(),
            vec![
                "test.html",
                "test.html::Some%20subtest:%20with%20spaces",
                "test.html::Another%3A%3Asubtest",
            ]
        );
        assert_eq!(
            tokens,
            vec![
                "test.html o [",
                "test.html i test.html",
                "test.html o ]",
                "test.html::expected i expected",
                "test.html::expected o :",
                "test.html::expected t ERROR",
                "test.html::Some%20subtest:%20with%20spaces o [",
                "test.html::Some%20subtest:%20with%20spaces i Some subtest: with spaces",
                "test.html::Some%20subtest:%20with%20spaces o ]",
                "test.html::Some%20subtest:%20with%20spaces::expected i expected",
                "test.html::Some%20subtest:%20with%20spaces::expected o :",
                "test.html::Some%20subtest:%20with%20spaces::expected t if",
                "test.html::Some%20subtest:%20with%20spaces::expected t os",
                "test.html::Some%20subtest:%20with%20spaces::expected t ==",
                "test.html::Some%20subtest:%20with%20spaces::expected t \"win\":",
                "test.html::Some%20subtest:%20with%20spaces::expected t FAIL",
                "test.html::Some%20subtest:%20with%20spaces::expected t PASS",
                "test.html::Another%3A%3Asubtest o [",
                "test.html::Another%3A%3Asubtest i Another::subtest",
                "test.html::Another%3A%3Asubtest o ]",
                "test.html::Another%3A%3Asubtest::expected i expected",
                "test.html::Another%3A%3Asubtest::expected o :",
                "test.html::Another%3A%3Asubtest::expected t FAIL",
            ]
        );
    }

    #[test]
    fn test_toml_features() {
        let tokens = toml(
            "name = 'literal'  # trailing\n\
             [package.metadata]\n\
             desc = \"\"\"multi\nline \"quoted\" \"\"\"\n\
             esc = \"a\\\"b\"\n\
             inline = { x = 1, y = [2, 3] }\n\
             [[bin]]\n\
             \"quoted.key\" = false\n",
        );
        assert_eq!(
            tokens,
            vec![
                "name i name",
                "name o =",
                "name t literal",
                "name c #",
                "name c trailing",
                "package.metadata o [",
                "package.metadata i package",
                "package.metadata i metadata",
                "package.metadata o ]",
                "package.metadata::desc i desc",
                "package.metadata::desc o =",
                "package.metadata::desc t multi",
                "package.metadata::desc t line",
                "package.metadata::desc t \"quoted\"",
                "package.metadata::esc i esc",
                "package.metadata::esc o =",
                "package.metadata::esc t a\\\"b",
                "package.metadata::inline i inline",
                "package.metadata::inline o =",
                "package.metadata::inline i x",
                "package.metadata::inline o =",
                "package.metadata::inline t 1",
                "package.metadata::inline i y",
                "package.metadata::inline o =",
                "package.metadata::inline t 2",
                "package.metadata::inline t 3",
                "bin o [[",
                "bin i bin",
                "bin o ]]",
                "bin::quoted.key i quoted.key",
                "bin::quoted.key o =",
                "bin::quoted.key t false",
            ]
        );
    }

    #[test]
    fn test_no_panics_on_garbage() {
        // Tiny deterministic PRNG so we don't need a dependency.
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let alphabet: Vec<char> = "[]=:#;\"'{},. \t\n\\ab1é’".chars().collect();
        for _ in 0..2000 {
            let len = (next() % 80) as usize;
            let source: String = (0..len)
                .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                .collect();
            for tokens in [tokenize_ini(&source).0, tokenize_toml(&source).0] {
                // Every token must be a verbatim substring of the source.
                for line in tokens {
                    let token = line.splitn(3, ' ').nth(2).unwrap();
                    assert!(source.contains(token), "{:?} not in {:?}", token, source);
                }
            }
        }
    }
}
