//! A tokenizer for YAML files, after `json_tokenizer`: contexts come from
//! their structure, so a key's tokens and its value's are in the context of
//! its key's path (ex: `tasks::linux64/opt` in a taskcluster kind.yml), and the
//! tokens of a record, a sequence's map item with an identity (see
//! `json_tokenizer::IDENTITY_KEYS`, ex: StaticPrefList.yaml's `- name:`), are
//! in the context of its identity, rather than of its position.  Top-level
//! keys and top-level records are structure rows.
//!
//! Tokens: keys (without their quotes) as identifiers (or words, if they're
//! prose), scalars' words as text (and numbers as numbers, and booleans and
//! nulls as identifiers), anchors, aliases, and tags as identifiers,
//! indicators (`:`, `-`, `[`, `{`, `|`, `---`, ...) as operators, and
//! comments' words, in the context of what they're about like code's (see
//! `cst_tokenizer::trivia_container`): a comment after code on its line has
//! that code's context, and a run of comments starting their lines, without
//! blank lines, the context of what's right after them (ex: a pref's
//! description before its `- name:`), but for one starting the file (ex: a
//! license header).  Every token is a verbatim substring of the source, in
//! source order; the text that tree-sitter-yaml's tree doesn't have nodes for
//! (quoted and block scalars' contents) is words.  Files are parsed like the
//! semantic analyzer parses them (see `semantic_analyzer::preprocess_yaml` and
//! `parse_all`), with searchfox's tree-sitter-yaml fork.

use std::collections::HashSet;

use crate::file_format::history::syntax_files::TokenClass;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::semantic_analyzer::{parse_all, preprocess_yaml};
use crate::tree_sitter_support::boilerplate::{
    FinishedTokens, RawToken, finish_tokens, prose_words,
};
use crate::tree_sitter_support::config_tokenizer::escape_name;
use crate::tree_sitter_support::json_tokenizer::{IDENTITY_KEYS, context_of};

struct YamlTokenizer<'a> {
    src: &'a str,
    tokens: Vec<RawToken<'a>>,
    structure: Vec<FileStructureRow>,
    /// The structure rows' pretty identifiers so far, to not repeat them.
    rows: HashSet<String>,
    /// The byte offsets of the source's lines.
    line_starts: Vec<usize>,
    /// Comments starting their lines, until the next token decides their
    /// context: (the context they're in, the comment, its line).
    pending: Vec<(String, &'a str, usize)>,
    /// Whether there have been tokens other than comments yet.
    seen_code: bool,
}

/// The value of a scalar key or identity: its text without its quotes.
fn unquoted(text: &str) -> &str {
    let text = text.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = text.strip_prefix(quote).and_then(|t| t.strip_suffix(quote)) {
            return inner;
        }
    }
    text
}

/// The scalar that a node (a `flow_node`, or a `block_node` or `flow_node`
/// containing one) is, if any.
fn scalar(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut node = node;
    loop {
        if node.kind().ends_with("_scalar") {
            return Some(node);
        }
        if !matches!(node.kind(), "flow_node" | "block_node") {
            return None;
        }
        let mut cursor = node.walk();
        node = node
            .named_children(&mut cursor)
            .find(|child| !matches!(child.kind(), "anchor" | "tag" | "comment"))?;
    }
}

impl<'a> YamlTokenizer<'a> {
    fn text(&self, node: &tree_sitter::Node) -> &'a str {
        &self.src[node.start_byte()..node.end_byte()]
    }

    fn line_of(&self, text: &str) -> usize {
        let offset = text.as_ptr() as usize - self.src.as_ptr() as usize;
        self.line_starts.partition_point(|start| *start <= offset) - 1
    }

    fn push(&mut self, context: &str, class: TokenClass, text: &'a str) {
        if text.is_empty() {
            return;
        }
        if !self.pending.is_empty() {
            let line = self.line_of(text);
            self.flush_comments(Some((line, context)));
        }
        self.seen_code = true;
        self.tokens.push(RawToken {
            context: context.to_string(),
            class,
            text,
        });
    }

    fn comment(&mut self, context: &str, text: &'a str) {
        let line = self.line_of(text);
        let offset = text.as_ptr() as usize - self.src.as_ptr() as usize;
        if self.src[self.line_starts[line]..offset].trim().is_empty() {
            self.pending.push((context.to_string(), text, line));
            return;
        }
        // (After code on its line: the code's, which tree-sitter-yaml can
        // leave the comment outside of, ex: `- name: a  # comment`.)
        let context = match self.tokens.last() {
            Some(last) if self.line_of(last.text) == line => last.context.clone(),
            _ => context.to_string(),
        };
        self.push_comment_words(&context, text);
    }

    fn push_comment_words(&mut self, context: &str, text: &'a str) {
        for word in prose_words(text) {
            self.tokens.push(RawToken {
                context: context.to_string(),
                class: TokenClass::Comment,
                text: word,
            });
        }
    }

    /// Tokenize the pending comments, those in a run right before `next` (the
    /// next token's line and context) in its context.
    fn flush_comments(&mut self, next: Option<(usize, &str)>) {
        let pending = std::mem::take(&mut self.pending);
        // (The run's first comment.)
        let mut run_start = pending.len();
        if let Some((line, _)) = next {
            let mut expected = line;
            while run_start > 0 && pending[run_start - 1].2 + 1 == expected {
                run_start -= 1;
                expected = pending[run_start].2;
            }
            // (Not a run starting the file.)
            if !self.seen_code && run_start == 0 {
                run_start = pending.len();
            }
        }
        for (i, (context, text, _)) in pending.into_iter().enumerate() {
            let context = match next {
                Some((_, next_context)) if i >= run_start => next_context.to_string(),
                _ => context,
            };
            self.push_comment_words(&context, text);
        }
    }

    fn push_words(&mut self, context: &str, class: TokenClass, text: &'a str) {
        // (Prose's punctuation is its own tokens; see `prose_words`.)
        if matches!(class, TokenClass::Text | TokenClass::Comment) {
            for word in prose_words(text) {
                self.push(context, class, word);
            }
            return;
        }
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

    /// The identity of a sequence item that's a map with one (see
    /// `IDENTITY_KEYS`).
    fn identity(&self, item: tree_sitter::Node) -> Option<&'a str> {
        let mut node = item;
        let map = loop {
            let mut cursor = node.walk();
            let child = node
                .named_children(&mut cursor)
                .find(|child| !matches!(child.kind(), "anchor" | "tag" | "comment"))?;
            match child.kind() {
                "block_mapping" | "flow_mapping" => break child,
                "block_node" | "flow_node" => node = child,
                _ => return None,
            }
        };
        let mut best: Option<(usize, &'a str)> = None;
        let mut cursor = map.walk();
        for pair in map.named_children(&mut cursor) {
            let (Some(key), Some(value)) = (
                pair.child_by_field_name("key").and_then(scalar),
                pair.child_by_field_name("value").and_then(scalar),
            ) else {
                continue;
            };
            let Some(rank) = IDENTITY_KEYS
                .iter()
                .position(|id| *id == unquoted(self.text(&key)))
            else {
                continue;
            };
            let value = unquoted(self.text(&value));
            if !value.is_empty() && best.is_none_or(|(best_rank, _)| rank < best_rank) {
                best = Some((rank, value));
            }
        }
        best.map(|(_, identity)| identity)
    }

    /// Tokenize `node`, in the context of `path` (and as a key, if `in_key`).
    fn walk(&mut self, node: tree_sitter::Node, path: &mut Vec<String>, in_key: bool) {
        let context = context_of(path);
        // (Records and pairs change the context of all of their tokens.)
        let pushed = match node.kind() {
            "block_mapping_pair" | "flow_pair" => {
                node.child_by_field_name("key").and_then(scalar).map(|key| {
                    let name = unquoted(self.text(&key));
                    path.push(escape_name(name));
                    if path.len() == 1 {
                        self.row(&context_of(path), "section");
                    }
                })
            }
            "block_sequence_item" | "flow_node"
                if node.parent().is_some_and(|parent| {
                    matches!(parent.kind(), "block_sequence" | "flow_sequence")
                }) =>
            {
                self.identity(node).map(|identity| {
                    let top_level = path.is_empty();
                    path.push(escape_name(identity));
                    if top_level {
                        self.row(&context_of(path), "record");
                    }
                })
            }
            _ => None,
        };
        let context = if pushed.is_some() {
            context_of(path)
        } else {
            context
        };
        if node.child_count() == 0 {
            let text = self.text(&node);
            match node.kind() {
                "comment" => self.comment(&context, text),
                // (A key can be prose, ex: a test's name.)
                _ if in_key && node.is_named() => {
                    if text.contains(char::is_whitespace) {
                        self.push_words(&context, TokenClass::Text, text);
                    } else {
                        self.push(&context, TokenClass::Identifier, text);
                    }
                }
                "integer_scalar" | "float_scalar" => self.push(&context, TokenClass::Number, text),
                "boolean_scalar" | "null_scalar" | "anchor_name" | "alias_name" | "tag" => {
                    self.push(&context, TokenClass::Identifier, text)
                }
                _ if node.is_named() => self.push_words(&context, TokenClass::Text, text),
                _ => self.push_words(&context, TokenClass::Operator, text),
            }
        } else {
            // Children, and the text between them that isn't in the tree
            // (ex: a quoted scalar's contents), as words.
            let mut covered = node.start_byte();
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for (i, child) in children.into_iter().enumerate() {
                if child.start_byte() > covered {
                    self.push_words(
                        &context,
                        TokenClass::Text,
                        &self.src[covered..child.start_byte()],
                    );
                }
                let is_key = in_key
                    || (matches!(node.kind(), "block_mapping_pair" | "flow_pair")
                        && node.field_name_for_child(i as u32) == Some("key"));
                self.walk(child, path, is_key);
                covered = covered.max(child.end_byte());
            }
            if node.end_byte() > covered {
                self.push_words(
                    &context,
                    TokenClass::Text,
                    &self.src[covered..node.end_byte()],
                );
            }
        }
        if pushed.is_some() {
            path.pop();
        }
    }
}

pub fn tokenize_yaml(source: &str) -> (FinishedTokens, Vec<FileStructureRow>) {
    let mut tokenizer = YamlTokenizer {
        src: source,
        tokens: vec![],
        structure: vec![],
        rows: HashSet::new(),
        line_starts: std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| i + 1))
            .collect(),
        pending: vec![],
        seen_code: false,
    };
    // (Tokens are the source's text at the parse's nodes' offsets, which
    // preprocessing doesn't change.)
    let preprocessed = preprocess_yaml(source);
    let parsed = preprocessed.as_deref().unwrap_or(source);
    let mut parser = tree_sitter::Parser::new();
    let trees = parser
        .set_language(&tree_sitter_yaml::LANGUAGE.into())
        .ok()
        .and_then(|_| parse_all(&mut parser, parsed, true))
        .unwrap_or_default();
    let mut covered = 0;
    for tree in &trees {
        let root = tree.root_node();
        if root.start_byte() > covered {
            tokenizer.push_words("%", TokenClass::Text, &source[covered..root.start_byte()]);
        }
        tokenizer.walk(root, &mut vec![], false);
        covered = covered.max(root.end_byte());
    }
    // (What the parses left out, ex: the rest of a line that error recovery
    // ended a tree in.)
    if source.len() > covered {
        tokenizer.push_words("%", TokenClass::Text, &source[covered..]);
    }
    tokenizer.flush_comments(None);
    (finish_tokens(source, tokenizer.tokens), tokenizer.structure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<String> {
        tokenize_yaml(source).0.lines
    }

    fn structure(source: &str) -> Vec<String> {
        tokenize_yaml(source)
            .1
            .into_iter()
            .map(|row| format!("{}:{}", row.kind, row.pretty))
            .collect()
    }

    #[test]
    fn test_contexts() {
        let source = "# Tasks.\njobs:\n  build-linux:\n    worker-type: b-linux\n    'max time': 7200\n    run: {using: mozharness}\n";
        assert_eq!(
            tokens(source),
            vec![
                "% c #",
                "% c Tasks",
                "% c .",
                "jobs i jobs",
                "jobs o :",
                "jobs::build-linux i build-linux",
                "jobs::build-linux o :",
                "jobs::build-linux::worker-type i worker-type",
                "jobs::build-linux::worker-type o :",
                "jobs::build-linux::worker-type t b-linux",
                "jobs::build-linux::max%20time o '",
                "jobs::build-linux::max%20time t max",
                "jobs::build-linux::max%20time t time",
                "jobs::build-linux::max%20time o '",
                "jobs::build-linux::max%20time o :",
                "jobs::build-linux::max%20time n 7200",
                "jobs::build-linux::run i run",
                "jobs::build-linux::run o :",
                "jobs::build-linux::run o {",
                "jobs::build-linux::run i using",
                "jobs::build-linux::run o :",
                "jobs::build-linux::run t mozharness",
                "jobs::build-linux::run o }",
            ]
        );
        assert_eq!(structure(source), vec!["section:jobs"]);
    }

    #[test]
    fn test_comments() {
        // A license header stays at the top level, a pref's description is
        // its, a trailing comment is its line's, and a section's comment
        // followed by a blank line is its enclosing context's.
        let source = "# License.\n\n# About a.\n- name: a  # Trailing.\n  type: bool\n  # Section.\n\n  value: 1\n";
        assert_eq!(
            tokens(source),
            vec![
                "% c #",
                "% c License",
                "% c .",
                "a c #",
                "a c About",
                "a c a",
                "a c .",
                "a o -",
                "a::name i name",
                "a::name o :",
                "a::name t a",
                "a::name c #",
                "a::name c Trailing",
                "a::name c .",
                "a::type i type",
                "a::type o :",
                "a::type t bool",
                "a c #",
                "a c Section",
                "a c .",
                "a::value i value",
                "a::value o :",
                "a::value n 1",
            ]
        );
    }

    #[test]
    fn test_records() {
        // A sequence's maps with identities are records, even with Mozilla's
        // preprocessor substitutions, whose tokens are verbatim.
        let source = "- name: a.b\n  value: @IS_NIGHTLY@\n- name: c\n  type: |\n    multi line\n";
        assert_eq!(
            tokens(source),
            vec![
                "a.b o -",
                "a.b::name i name",
                "a.b::name o :",
                "a.b::name t a.b",
                "a.b::value i value",
                "a.b::value o :",
                "a.b::value o @",
                "a.b::value t IS_NIGHTLY",
                "a.b::value o @",
                "c o -",
                "c::name i name",
                "c::name o :",
                "c::name t c",
                "c::type i type",
                "c::type o :",
                "c::type o |",
                "c::type t multi",
                "c::type t line",
            ]
        );
        assert_eq!(structure(source), vec!["record:a.b", "record:c"]);
    }
}
