//! This file defines the format of the token-per-line files under
//! `history/syntax/files` produced by `hypertokenize_source_file`.
//!
//! Each line is `{context} {class} {token}` where:
//! - `context` is the pretty identifier of the structural context of the token
//!   (ex: `mozilla::Widget::Compute`) or "%" if there is none.
//! - `class` is a single character `TokenClass`.
//! - `token` is the token text, which may contain spaces (ex: C++ string
//!   literals) but never newlines.
//!
//! The class is intentionally coarse.  It's derived from tree-sitter's
//! distinction between anonymous nodes (fixed strings in the grammar, like
//! keywords and punctuation) and named nodes (identifiers and literals whose
//! text is chosen by the author), which is mostly stable across grammars and
//! grammar upgrades.  Because the class is part of the line, a change in class
//! looks like a change in context to the diff, which the hyperblame
//! re-contexting pass already handles.

/// Coarse classification of a token.  See `hypertokenize_source_file` for how
/// these are derived from tree-sitter nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenClass {
    /// Words fixed by the grammar: `for`, `let`, `const`, `#include`, Rust's
    /// `mut`.
    Keyword,
    /// Operators and punctuation.
    Operator,
    /// Identifiers and named values: variables, types (including primitive
    /// types like `int`), fields, and values like `this`, `self`, `true`,
    /// `null`, and `nullptr`.
    Identifier,
    /// String and character literals and their pieces.
    String,
    /// Numeric literals.
    Number,
    /// Words in comments.
    Comment,
    /// License headers and editor modelines in comments or plain text.  These
    /// are shared by huge numbers of files, so they shouldn't count as
    /// evidence that files are related and aren't worth tracking.  See
    /// `tree_sitter_support::boilerplate`.
    Boilerplate,
    /// Plain text from files we don't have a tree-sitter grammar for, or other
    /// raw text like preprocessor arguments.
    Text,
    /// The line lacked a class, which means it was produced by an older version
    /// of the tokenizer.  Use `TokenClass::guess` to approximate one.
    Unknown,
}

impl TokenClass {
    pub fn as_char(self) -> char {
        match self {
            TokenClass::Keyword => 'k',
            TokenClass::Operator => 'o',
            TokenClass::Identifier => 'i',
            TokenClass::String => 's',
            TokenClass::Number => 'n',
            TokenClass::Comment => 'c',
            TokenClass::Boilerplate => 'b',
            TokenClass::Text => 't',
            TokenClass::Unknown => '?',
        }
    }

    pub fn from_char(c: char) -> Option<TokenClass> {
        Some(match c {
            'k' => TokenClass::Keyword,
            'o' => TokenClass::Operator,
            'i' => TokenClass::Identifier,
            's' => TokenClass::String,
            'n' => TokenClass::Number,
            'c' => TokenClass::Comment,
            'b' => TokenClass::Boilerplate,
            't' => TokenClass::Text,
            '?' => TokenClass::Unknown,
            _ => return None,
        })
    }

    /// Approximate a class from the token text alone for lines that lack one.
    /// This can't tell keywords from identifiers or comments from code.
    pub fn guess(token: &str) -> TokenClass {
        if !token.chars().any(|c| c.is_alphanumeric()) {
            return TokenClass::Operator;
        }
        // Allow a short literal prefix (ex: `r`, `b`, `u8`, `L`, `rb`) and Rust
        // raw string `#`s before the opening quote.
        let after_prefix = token.trim_start_matches(|c: char| c.is_ascii_alphanumeric());
        let prefix_len = token.len() - after_prefix.len();
        if prefix_len <= 3
            && after_prefix
                .trim_start_matches('#')
                .starts_with(['"', '\'', '`'])
        {
            return TokenClass::String;
        }
        let unsigned = token.strip_prefix(['-', '+']).unwrap_or(token);
        let unsigned = unsigned.strip_prefix('.').unwrap_or(unsigned);
        if unsigned.starts_with(|c: char| c.is_ascii_digit()) {
            TokenClass::Number
        } else {
            TokenClass::Identifier
        }
    }
}

/// A parsed line from a `history/syntax/files` token-per-line file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenLine<'a> {
    /// The pretty identifier of the structural context of the token, or "%" if
    /// there is none.
    pub context: &'a str,
    pub class: TokenClass,
    pub token: &'a str,
}

impl TokenLine<'_> {
    /// The class, guessed from the token text if the line lacked one.
    pub fn effective_class(&self) -> TokenClass {
        match self.class {
            TokenClass::Unknown => TokenClass::guess(self.token),
            class => class,
        }
    }
}

pub fn format_token_line(context: &str, class: TokenClass, token: &str) -> String {
    format!("{} {} {}", context, class.as_char(), token)
}

/// Parse a token file line.  Lines in the older `{context} {token}` format (or
/// that otherwise lack a valid class) get `TokenClass::Unknown`.  Lines without
/// any space are treated as having an empty context.
pub fn split_token_line(line: &str) -> TokenLine<'_> {
    let Some((context, rest)) = line.split_once(' ') else {
        return TokenLine {
            context: "",
            class: TokenClass::Unknown,
            token: line,
        };
    };
    let bytes = rest.as_bytes();
    if bytes.len() >= 3
        && bytes[1] == b' '
        && let Some(class) = TokenClass::from_char(bytes[0] as char)
    {
        return TokenLine {
            context,
            class,
            token: &rest[2..],
        };
    }
    TokenLine {
        context,
        class: TokenClass::Unknown,
        token: rest,
    }
}

/// Split the contents of a `history/syntax/files` token file into lines.
pub fn token_file_lines(contents: &str) -> Vec<&str> {
    let contents = contents.strip_suffix('\n').unwrap_or(contents);
    if contents.is_empty() {
        vec![]
    } else {
        contents.split('\n').collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_token_line() {
        assert_eq!(
            split_token_line("Foo::bar s \"hello world\""),
            TokenLine {
                context: "Foo::bar",
                class: TokenClass::String,
                token: "\"hello world\""
            }
        );
        assert_eq!(
            split_token_line(&format_token_line("%", TokenClass::Keyword, "for")),
            TokenLine {
                context: "%",
                class: TokenClass::Keyword,
                token: "for"
            }
        );
        // Old format lines lack a class.
        assert_eq!(
            split_token_line("Foo::bar mCount"),
            TokenLine {
                context: "Foo::bar",
                class: TokenClass::Unknown,
                token: "mCount"
            }
        );
        // A single character token in the old format isn't mistaken for a class.
        assert_eq!(split_token_line("% i").class, TokenClass::Unknown);
        assert_eq!(split_token_line("% i").token, "i");
        assert_eq!(
            split_token_line("continuation"),
            TokenLine {
                context: "",
                class: TokenClass::Unknown,
                token: "continuation"
            }
        );
        assert_eq!(token_file_lines(""), Vec::<&str>::new());
        assert_eq!(token_file_lines("% i a\n% i b"), vec!["% i a", "% i b"]);
        assert_eq!(token_file_lines("% i a\n% i b\n"), vec!["% i a", "% i b"]);
    }

    #[test]
    fn test_guess() {
        use TokenClass::*;
        for (token, class) in [
            ("foo", Identifier),
            ("#include", Identifier),
            ("$el", Identifier),
            ("\"loc\"", String),
            ("'a'", String),
            ("`tmpl`", String),
            ("u8\"x\"", String),
            ("r#\"raw\"#", String),
            ("0", Number),
            ("0x10", Number),
            ("-1", Number),
            (".5", Number),
            (">=", Operator),
            ("::", Operator),
        ] {
            assert_eq!(TokenClass::guess(token), class, "{}", token);
        }
    }
}
