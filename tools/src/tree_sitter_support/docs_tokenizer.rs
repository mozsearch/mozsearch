//! Tokenizers for documentation: Markdown (with tree-sitter-md's block and
//! inline grammars, and the MyST roles and directives which Firefox's Sphinx
//! docs use) and reStructuredText (with tree-sitter-rst).  They tokenize the
//! same content the same, so that history follows Firefox's conversion of its
//! docs from RST to Markdown (they share the "docs" namespace):
//! - Contexts are the paths of the headings a token is under, by their titles'
//!   words (without markup, ex: ``Using ``mach`` `` and `` Using `mach` `` are
//!   both `Using%20mach`).  Markdown's levels are its headings', and RST's are
//!   its adornment styles' in the order they first appear, like docutils'.
//!   Headings are structure rows.
//! - Prose is words (split on whitespace, whatever the grammars' leaves, so
//!   ex: `(googletest)` is a token in both), as text.
//! - Markup is operators (ex: `#`, a heading's underline, `**`, list markers,
//!   code fences, a link's brackets), inline code's and literals' contents are
//!   identifiers, as are links' destinations and roles' names and targets: ex:
//!   Markdown's `` {searchfox}`a/b.js` `` and RST's `` :searchfox:`a/b.js` ``
//!   (Firefox's role for searchfox links, see mozbuild/sphinx.py) are both
//!   `searchfox` and `a/b.js` between operators, and a role's
//!   `` `title <target>` `` is the title's words and the target.
//! - Code blocks' and directives' contents are words, and comments' words are
//!   comments.
//!
//! Every token is a verbatim substring of the source, in source order.

use std::collections::HashSet;
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use crate::file_format::history::syntax_files::TokenClass;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::tree_sitter_support::boilerplate::{
    FinishedTokens, RawToken, finish_tokens, prose_words,
};
use crate::tree_sitter_support::config_tokenizer::escape_name;

/// How much of a document's text a piece of markup is.
#[derive(Clone, Copy)]
enum Piece {
    /// A single token.
    Token(TokenClass),
    /// Words.
    Words(TokenClass),
}

/// A document's markup and headings, from its parse, which `finish` makes
/// tokens of (filling in the rest with prose words).
struct Docs<'a> {
    src: &'a str,
    pieces: Vec<(Range<usize>, Piece)>,
    /// (where it starts, its level, its context name).
    headings: Vec<(usize, usize, String)>,
}

lazy_static! {
    /// A MyST role's name right before its code span, ex: `{searchfox}`.
    static ref MYST_ROLE: Regex = Regex::new(r"\{([A-Za-z][\w:.+-]*)\}$").unwrap();
    /// A role's (or link's) title and target, ex: `this file <a/b.js>`.
    static ref TITLE_TARGET: Regex = Regex::new(r"(?s)^(.*\S)\s*<([^<>]+)>$").unwrap();
    static ref MD_LINK: Regex = Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").unwrap();
    static ref RST_REFERENCE: Regex = Regex::new(r"`([^`<]*?)\s*<[^>]*>`_{1,2}").unwrap();
    static ref ROLE_PREFIX: Regex = Regex::new(r"(\{[A-Za-z][\w:.+-]*\}|:[A-Za-z][\w:.+-]*:)`").unwrap();
}

/// The directives of code blocks, which are markup like Markdown's code fences
/// (their contents are words, like other directives').
const CODE_DIRECTIVES: &[&str] = &["code-block", "code", "sourcecode"];

/// A heading's context name: its title's words, without markup.
fn heading_name(title: &str) -> String {
    let title = MD_LINK.replace_all(title, "$1");
    let title = RST_REFERENCE.replace_all(&title, "$1");
    let title = ROLE_PREFIX.replace_all(&title, "`");
    let words: Vec<&str> = title
        .split_whitespace()
        .map(|word| word.trim_matches(|c| matches!(c, '`' | '*' | '_' | '|')))
        .filter(|word| !word.is_empty())
        .collect();
    escape_name(&words.join(" "))
}

impl<'a> Docs<'a> {
    fn new(src: &'a str) -> Docs<'a> {
        Docs {
            src,
            pieces: vec![],
            headings: vec![],
        }
    }

    fn text(&self, range: &Range<usize>) -> &'a str {
        // (Error recovery can make odd ranges.)
        self.src.get(range.clone()).unwrap_or_default()
    }

    fn piece(&mut self, range: Range<usize>, piece: Piece) {
        if range.start < range.end {
            self.pieces.push((range, piece));
        }
    }

    fn node_piece(&mut self, node: &tree_sitter::Node, piece: Piece) {
        self.piece(node.byte_range(), piece);
    }

    /// A role's (or a link's) target, or `title <target>`: the title's words
    /// and the target.  (A link without a target is its text's words.)
    fn role_target(&mut self, range: Range<usize>) {
        self.title_target(range, TokenClass::Identifier);
    }

    fn title_target(&mut self, range: Range<usize>, plain: TokenClass) {
        if range.start >= range.end {
            return;
        }
        let text = self.text(&range);
        if let Some(m) = TITLE_TARGET.captures(text) {
            let title = m.get(1).unwrap();
            let target = m.get(2).unwrap();
            self.piece(
                range.start + title.start()..range.start + title.end(),
                Piece::Words(TokenClass::Text),
            );
            self.piece(
                range.start + title.end()..range.start + target.start(),
                Piece::Words(TokenClass::Operator),
            );
            self.piece(
                range.start + target.start()..range.start + target.end(),
                Piece::Words(TokenClass::Identifier),
            );
            self.piece(
                range.start + target.end()..range.end,
                Piece::Token(TokenClass::Operator),
            );
        } else {
            self.piece(range, Piece::Words(plain));
        }
    }

    /// A role's name (`:name:` or `{name}` at `range`): its delimiters and
    /// name.
    fn role_name(&mut self, range: Range<usize>) {
        if range.end - range.start < 3 {
            return;
        }
        self.piece(
            range.start..range.start + 1,
            Piece::Token(TokenClass::Operator),
        );
        self.piece(
            range.start + 1..range.end - 1,
            Piece::Token(TokenClass::Identifier),
        );
        self.piece(range.end - 1..range.end, Piece::Token(TokenClass::Operator));
    }

    /// The tokens: the pieces, and the prose between them, in the contexts of
    /// their headings.
    fn finish(mut self) -> (FinishedTokens, Vec<FileStructureRow>) {
        self.pieces
            .sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
        self.headings.sort_by_key(|heading| heading.0);
        let pieces = std::mem::take(&mut self.pieces);
        let mut out = Emitter {
            src: self.src,
            headings: self.headings,
            next_heading: 0,
            stack: vec![],
            context: "%".to_string(),
            tokens: vec![],
            structure: vec![],
            rows: HashSet::new(),
        };
        let mut pos = 0;
        for (range, piece) in pieces {
            // (Overlapping pieces, ex: a role's name in markup already covered,
            // are left out.)
            if range.start < pos {
                continue;
            }
            out.prose(pos, range.start);
            out.headings_until(range.start);
            let (class, words) = match piece {
                Piece::Token(class) => (class, false),
                Piece::Words(class) => (class, true),
            };
            out.push(class, &self.src[range.clone()], words);
            pos = range.end;
        }
        out.prose(pos, self.src.len());
        (finish_tokens(self.src, out.tokens), out.structure)
    }
}

/// Tokens in the contexts of the headings before them.
struct Emitter<'a> {
    src: &'a str,
    headings: Vec<(usize, usize, String)>,
    next_heading: usize,
    /// The headings (level, name) the current position is under.
    stack: Vec<(usize, String)>,
    context: String,
    tokens: Vec<RawToken<'a>>,
    structure: Vec<FileStructureRow>,
    rows: HashSet<String>,
}

impl<'a> Emitter<'a> {
    /// Enter the headings which start at or before `pos`.
    fn headings_until(&mut self, pos: usize) {
        while let Some((start, level, name)) = self.headings.get(self.next_heading)
            && *start <= pos
        {
            while self.stack.last().is_some_and(|(l, _)| l >= level) {
                self.stack.pop();
            }
            self.stack.push((*level, name.clone()));
            self.next_heading += 1;
            self.context = self
                .stack
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join("::");
            if self.context.is_empty() {
                self.context = "%".to_string();
            }
            if self.rows.insert(self.context.clone()) {
                self.structure.push(FileStructureRow {
                    pretty: self.context.clone(),
                    is_def: true,
                    kind: "section".to_string(),
                    pp: vec![],
                });
            }
        }
    }

    fn push(&mut self, class: TokenClass, text: &'a str, words: bool) {
        if words {
            // (Prose's punctuation is its own tokens; see `prose_words`.)
            let words: Box<dyn Iterator<Item = &str>> =
                if matches!(class, TokenClass::Text | TokenClass::Comment) {
                    Box::new(prose_words(text))
                } else {
                    Box::new(text.split_whitespace())
                };
            for word in words {
                self.tokens.push(RawToken {
                    context: self.context.clone(),
                    class,
                    text: word,
                });
            }
        } else if !text.trim().is_empty() {
            self.tokens.push(RawToken {
                context: self.context.clone(),
                class,
                text: text.trim(),
            });
        }
    }

    /// Prose words from `from` to `to`, in the contexts of the headings
    /// starting among them.
    fn prose(&mut self, mut from: usize, to: usize) {
        while let Some((start, _, _)) = self.headings.get(self.next_heading)
            && *start < to
        {
            let start = (*start).max(from);
            self.push(TokenClass::Text, &self.src[from..start], true);
            self.headings_until(start);
            from = start;
        }
        if from < to {
            self.push(TokenClass::Text, &self.src[from..to], true);
        }
    }
}

// ## Markdown

/// The Markdown parse of a document: its block tree, and the inline trees of
/// its `inline` nodes (and table cells), each parsed with the inline grammar
/// over the node's range without its named children (ex: a block quote's `>`
/// on its continuation lines), like tree-sitter-md's `MarkdownParser`.
fn md_walk(docs: &mut Docs, node: tree_sitter::Node, inline: &mut tree_sitter::Parser) {
    let kind = node.kind();
    match kind {
        "atx_heading" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    marker if marker.starts_with("atx_h") && marker.ends_with("_marker") => {
                        let level = marker[5..6].parse().unwrap_or(1);
                        let title = node
                            .child_by_field_name("heading_content")
                            .map(|content| docs.text(&content.byte_range()))
                            .unwrap_or_default();
                        // (Without a closing sequence.)
                        let title = title.trim().trim_end_matches('#');
                        docs.headings
                            .push((node.start_byte(), level, heading_name(title)));
                        docs.node_piece(&child, Piece::Token(TokenClass::Operator));
                    }
                    "inline" => md_inline(docs, child, inline),
                    _ => md_walk(docs, child, inline),
                }
            }
        }
        "setext_heading" => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let level = if children.iter().any(|c| c.kind() == "setext_h2_underline") {
                2
            } else {
                1
            };
            let title = children
                .iter()
                .find(|c| c.kind() == "paragraph")
                .map(|p| docs.text(&p.byte_range()))
                .unwrap_or_default();
            docs.headings
                .push((node.start_byte(), level, heading_name(title)));
            for child in children {
                md_walk(docs, child, inline);
            }
        }
        "inline" | "pipe_table_cell" => md_inline(docs, node, inline),
        "fenced_code_block" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "info_string" => {
                        let text = docs.text(&child.byte_range());
                        // (A MyST directive, ex: ```{note}.)
                        match MYST_ROLE.find(text.split_whitespace().next().unwrap_or_default()) {
                            Some(m) if text.trim_start().starts_with('{') => {
                                let start = child.start_byte() + text.find('{').unwrap();
                                let name = &docs.src[start + 1..start + m.len() - 1];
                                if CODE_DIRECTIVES.contains(&name) {
                                    docs.piece(
                                        start..start + m.len(),
                                        Piece::Token(TokenClass::Operator),
                                    );
                                } else {
                                    docs.role_name(start..start + m.len());
                                }
                                docs.piece(
                                    start + m.len()..child.end_byte(),
                                    Piece::Words(TokenClass::Identifier),
                                );
                            }
                            _ => docs.node_piece(&child, Piece::Words(TokenClass::Identifier)),
                        }
                    }
                    "code_fence_content" => match myst_directive(docs, &node) {
                        // (MyST's directives' contents are Markdown, but for
                        // `{eval-rst}`'s, which are RST, and code blocks'.)
                        Some("eval-rst") => rst_nested(docs, child.range(), &mut vec![]),
                        Some(name) if !CODE_DIRECTIVES.contains(&name) => {
                            md_nested(docs, child.range(), inline)
                        }
                        _ => docs.node_piece(&child, Piece::Words(TokenClass::Text)),
                    },
                    _ => docs.node_piece(&child, Piece::Words(TokenClass::Operator)),
                }
            }
        }
        "indented_code_block" => docs.node_piece(&node, Piece::Words(TokenClass::Text)),
        "html_block" => {
            let class = if docs
                .text(&node.byte_range())
                .trim_start()
                .starts_with("<!--")
            {
                TokenClass::Comment
            } else {
                TokenClass::Text
            };
            docs.node_piece(&node, Piece::Words(class));
        }
        "link_destination" => docs.node_piece(&node, Piece::Token(TokenClass::Identifier)),
        "minus_metadata" | "plus_metadata" => {
            docs.node_piece(&node, Piece::Words(TokenClass::Text))
        }
        "list_marker_minus"
        | "list_marker_plus"
        | "list_marker_star"
        | "list_marker_dot"
        | "list_marker_parenthesis"
        | "block_quote_marker"
        | "block_continuation"
        | "thematic_break"
        | "setext_h1_underline"
        | "setext_h2_underline"
        | "task_list_marker_checked"
        | "task_list_marker_unchecked"
        | "pipe_table_delimiter_row" => docs.node_piece(&node, Piece::Words(TokenClass::Operator)),
        _ => {
            if node.child_count() == 0 {
                if !node.is_named() {
                    docs.node_piece(&node, Piece::Words(TokenClass::Operator));
                }
                return;
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                md_walk(docs, child, inline);
            }
        }
    }
}

/// The name of the MyST directive which a fenced code block is (ex: `note`
/// for ```{note}), if any.
fn myst_directive<'a>(docs: &Docs<'a>, block: &tree_sitter::Node) -> Option<&'a str> {
    let mut cursor = block.walk();
    let info = block
        .children(&mut cursor)
        .find(|c| c.kind() == "info_string")?;
    let text = docs.text(&info.byte_range()).trim_start();
    let m = MYST_ROLE.find(text.split_whitespace().next()?)?;
    text.starts_with('{').then(|| &text[1..m.len() - 1])
}

/// Markdown within a document (ex: a MyST directive's content), parsed on its
/// own.
fn md_nested(docs: &mut Docs, range: tree_sitter::Range, inline: &mut tree_sitter::Parser) {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_md::LANGUAGE.into())
        .is_err()
        || parser.set_included_ranges(&[range]).is_err()
    {
        docs.piece(
            range.start_byte..range.end_byte,
            Piece::Words(TokenClass::Text),
        );
        return;
    }
    match parser.parse(docs.src, None) {
        Some(tree) => md_walk(docs, tree.root_node(), inline),
        None => docs.piece(
            range.start_byte..range.end_byte,
            Piece::Words(TokenClass::Text),
        ),
    }
}

/// MyST's markup at the starts of a paragraph's lines (which tree-sitter-md
/// takes for text): colon fences (`:::{name} arguments`, and their closing
/// `:::`s), targets (`(label)=`), and field lists (`:name: value`).
fn myst_lines(docs: &mut Docs, range: Range<usize>) {
    lazy_static! {
        static ref COLON_FENCE: Regex =
            Regex::new(r"^[ \t]*(:{3,})(\{[A-Za-z][\w:.+-]*\})?([^\n]*)").unwrap();
        static ref TARGET: Regex = Regex::new(r"^[ \t]*(\()([^()\s]+)(\)=)[ \t]*$").unwrap();
        static ref FIELD: Regex = Regex::new(r"^[ \t]*(:)([\w][\w .-]*?)(:)(\s|$)").unwrap();
    }
    let mut line_start = range.start;
    let text = docs.text(&range);
    for line in text.split_inclusive('\n') {
        let line_text = line.trim_end_matches('\n');
        let at = |m: regex::Match| line_start + m.start()..line_start + m.end();
        if let Some(c) = COLON_FENCE.captures(line_text) {
            docs.piece(at(c.get(1).unwrap()), Piece::Token(TokenClass::Operator));
            if let Some(name) = c.get(2) {
                docs.role_name(at(name));
            }
            if let Some(args) = c.get(3) {
                docs.piece(at(args), Piece::Words(TokenClass::Identifier));
            }
        } else if let Some(c) = TARGET.captures(line_text) {
            docs.piece(at(c.get(1).unwrap()), Piece::Token(TokenClass::Operator));
            docs.piece(at(c.get(2).unwrap()), Piece::Token(TokenClass::Identifier));
            docs.piece(at(c.get(3).unwrap()), Piece::Token(TokenClass::Operator));
        } else if let Some(c) = FIELD.captures(line_text) {
            docs.piece(at(c.get(1).unwrap()), Piece::Token(TokenClass::Operator));
            docs.piece(at(c.get(2).unwrap()), Piece::Words(TokenClass::Identifier));
            docs.piece(at(c.get(3).unwrap()), Piece::Token(TokenClass::Operator));
        }
        line_start += line.len();
    }
}

fn md_inline(docs: &mut Docs, node: tree_sitter::Node, inline: &mut tree_sitter::Parser) {
    if node.parent().is_some_and(|p| p.kind() == "paragraph") {
        myst_lines(docs, node.byte_range());
    }
    let mut ranges = vec![];
    let mut range = node.range();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        ranges.push(tree_sitter::Range {
            start_byte: range.start_byte,
            start_point: range.start_point,
            end_byte: child.start_byte(),
            end_point: child.start_position(),
        });
        range.start_byte = child.end_byte();
        range.start_point = child.end_position();
        md_walk(docs, child, inline);
    }
    ranges.push(range);
    ranges.retain(|r| r.start_byte < r.end_byte);
    if ranges.is_empty() || inline.set_included_ranges(&ranges).is_err() {
        return;
    }
    let Some(tree) = inline.parse(docs.src, None) else {
        return;
    };
    md_inline_walk(docs, tree.root_node());
}

fn md_inline_walk(docs: &mut Docs, node: tree_sitter::Node) {
    match node.kind() {
        "emphasis_delimiter" | "code_span_delimiter" | "latex_span_delimiter" => {
            docs.node_piece(&node, Piece::Token(TokenClass::Operator))
        }
        "code_span" => {
            let mut cursor = node.walk();
            let delimiters: Vec<_> = node
                .children(&mut cursor)
                .filter(|c| c.kind() == "code_span_delimiter")
                .collect();
            let (Some(open), Some(close)) = (delimiters.first(), delimiters.last()) else {
                return;
            };
            docs.node_piece(open, Piece::Token(TokenClass::Operator));
            if open.id() != close.id() {
                docs.node_piece(close, Piece::Token(TokenClass::Operator));
            }
            let content = open.end_byte()..close.start_byte().max(open.end_byte());
            // (A MyST role, ex: {searchfox}`a/b.js`.)
            let line_start = docs.src[..node.start_byte()]
                .rfind('\n')
                .map_or(0, |i| i + 1);
            let before = &docs.src[line_start..node.start_byte()];
            match MYST_ROLE.find(before) {
                Some(m) => {
                    docs.role_name(line_start + m.start()..line_start + m.end());
                    docs.role_target(content);
                }
                None => docs.piece(content, Piece::Words(TokenClass::Identifier)),
            }
        }
        "link_destination" | "uri_autolink" | "email_autolink" => {
            let text = docs.text(&node.byte_range());
            if text.starts_with('<') && text.ends_with('>') && text.len() > 2 {
                docs.piece(
                    node.start_byte()..node.start_byte() + 1,
                    Piece::Token(TokenClass::Operator),
                );
                docs.piece(
                    node.start_byte() + 1..node.end_byte() - 1,
                    Piece::Token(TokenClass::Identifier),
                );
                docs.piece(
                    node.end_byte() - 1..node.end_byte(),
                    Piece::Token(TokenClass::Operator),
                );
            } else {
                docs.node_piece(&node, Piece::Words(TokenClass::Identifier));
            }
        }
        "link_title" => docs.node_piece(&node, Piece::Words(TokenClass::Text)),
        "inline_link"
        | "image"
        | "full_reference_link"
        | "collapsed_reference_link"
        | "shortcut_link" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    md_inline_walk(docs, child);
                } else {
                    docs.node_piece(&child, Piece::Token(TokenClass::Operator));
                }
            }
        }
        "html_tag" => docs.node_piece(&node, Piece::Words(TokenClass::Text)),
        "latex_block" => docs.node_piece(&node, Piece::Words(TokenClass::Text)),
        _ => {
            // (Other leaves, ex: punctuation, are prose.)
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                md_inline_walk(docs, child);
            }
        }
    }
}

pub fn tokenize_markdown(source: &str) -> (FinishedTokens, Vec<FileStructureRow>) {
    let mut docs = Docs::new(source);
    let mut parser = tree_sitter::Parser::new();
    let mut inline = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_md::LANGUAGE.into())
        .is_ok()
        && inline
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .is_ok()
        && let Some(tree) = parser.parse(source, None)
    {
        md_walk(&mut docs, tree.root_node(), &mut inline);
    }
    docs.finish()
}

// ## reStructuredText

fn rst_walk(docs: &mut Docs, node: tree_sitter::Node, styles: &mut Vec<(char, bool)>) {
    let kind = node.kind();
    match kind {
        "section" => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let title_index = children.iter().position(|c| c.kind() == "title");
            let overline = title_index.is_some_and(|i| i > 0);
            let style_char = children
                .iter()
                .rev()
                .find(|c| c.kind() == "adornment")
                .and_then(|c| docs.text(&c.byte_range()).chars().next())
                .unwrap_or('=');
            let style = (style_char, overline);
            let level = match styles.iter().position(|s| *s == style) {
                Some(i) => i + 1,
                None => {
                    styles.push(style);
                    styles.len()
                }
            };
            let title = title_index
                .map(|i| docs.text(&children[i].byte_range()))
                .unwrap_or_default();
            docs.headings
                .push((node.start_byte(), level, heading_name(title)));
            for child in children {
                if child.kind() == "adornment" {
                    docs.node_piece(&child, Piece::Token(TokenClass::Operator));
                } else {
                    rst_walk(docs, child, styles);
                }
            }
        }
        "emphasis" | "strong" => {
            let n = if kind == "strong" { 2 } else { 1 };
            let range = node.byte_range();
            if range.end - range.start > 2 * n {
                docs.piece(
                    range.start..range.start + n,
                    Piece::Token(TokenClass::Operator),
                );
                docs.piece(range.end - n..range.end, Piece::Token(TokenClass::Operator));
            }
        }
        "literal" => {
            let range = node.byte_range();
            if range.end - range.start > 4 {
                docs.piece(
                    range.start..range.start + 2,
                    Piece::Token(TokenClass::Operator),
                );
                docs.piece(
                    range.start + 2..range.end - 2,
                    Piece::Words(TokenClass::Identifier),
                );
                docs.piece(range.end - 2..range.end, Piece::Token(TokenClass::Operator));
            }
        }
        "interpreted_text" => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in &children {
                if child.kind() == "role" {
                    docs.role_name(child.byte_range());
                }
            }
            // The text between the backticks (which is the node's text outside
            // of its role).
            let text_start = children
                .iter()
                .find(|c| c.kind() == "role" && c.start_byte() == node.start_byte())
                .map_or(node.start_byte(), |role| role.end_byte());
            let text_end = children
                .iter()
                .find(|c| c.kind() == "role" && c.end_byte() == node.end_byte())
                .map_or(node.end_byte(), |role| role.start_byte());
            if text_end - text_start >= 2 {
                docs.piece(
                    text_start..text_start + 1,
                    Piece::Token(TokenClass::Operator),
                );
                docs.role_target(text_start + 1..text_end - 1);
                docs.piece(text_end - 1..text_end, Piece::Token(TokenClass::Operator));
            }
        }
        "reference" => {
            // `` `title <target>`_ ``, `` `name`__ ``, or `name_`.
            let range = node.byte_range();
            let text = docs.text(&range);
            let underscores = text.len() - text.trim_end_matches('_').len();
            let body_end = range.end - underscores;
            if text.starts_with('`') && text.len() > underscores + 1 && body_end >= range.start + 2
            {
                docs.piece(
                    range.start..range.start + 1,
                    Piece::Token(TokenClass::Operator),
                );
                docs.title_target(range.start + 1..body_end - 1, TokenClass::Text);
                docs.piece(body_end - 1..range.end, Piece::Token(TokenClass::Operator));
            } else {
                docs.piece(body_end..range.end, Piece::Token(TokenClass::Operator));
            }
        }
        "standalone_hyperlink" => docs.node_piece(&node, Piece::Token(TokenClass::Identifier)),
        "footnote_reference"
        | "citation_reference"
        | "substitution_reference"
        | "inline_target" => docs.node_piece(&node, Piece::Words(TokenClass::Identifier)),
        "literal_block" | "doctest_block" | "content" | "arguments" => {
            let class = if kind == "arguments" {
                TokenClass::Identifier
            } else {
                TokenClass::Text
            };
            docs.node_piece(&node, Piece::Words(class))
        }
        "comment" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !child.is_named() && child.kind() == ".." {
                    docs.node_piece(&child, Piece::Token(TokenClass::Operator));
                }
            }
            docs.node_piece(&node, Piece::Words(TokenClass::Comment));
        }
        // (Code blocks' directives are markup, like Markdown's fences.)
        "type" if CODE_DIRECTIVES.contains(&docs.text(&node.byte_range())) => {
            docs.node_piece(&node, Piece::Words(TokenClass::Operator))
        }
        "type" => docs.node_piece(&node, Piece::Words(TokenClass::Identifier)),
        "target" => {
            // `.. _name: url` names a link (like Markdown's `[name]: url`),
            // whose name is words; `.. _label:` is a label (like MyST's
            // `(label)=`).
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let has_link = children.iter().any(|c| c.kind() == "link");
            for child in children {
                match child.kind() {
                    "name" => {
                        let range = child.byte_range();
                        let text = docs.text(&range);
                        let prefix = text.len() - text.trim_start_matches('_').len();
                        let suffix = text.len() - text.trim_end_matches(':').len();
                        if prefix + suffix >= text.len() {
                            docs.node_piece(&child, Piece::Words(TokenClass::Operator));
                            continue;
                        }
                        docs.piece(
                            range.start..range.start + prefix,
                            Piece::Token(TokenClass::Operator),
                        );
                        let class = if has_link {
                            TokenClass::Text
                        } else {
                            TokenClass::Identifier
                        };
                        docs.piece(
                            range.start + prefix..range.end - suffix,
                            Piece::Words(class),
                        );
                        docs.piece(
                            range.end - suffix..range.end,
                            Piece::Token(TokenClass::Operator),
                        );
                    }
                    "link" => docs.node_piece(&child, Piece::Token(TokenClass::Identifier)),
                    _ if !child.is_named() => {
                        docs.node_piece(&child, Piece::Words(TokenClass::Operator))
                    }
                    _ => rst_walk(docs, child, styles),
                }
            }
        }
        "directive" => {
            let is_code = node
                .child_by_field_name("name")
                .is_some_and(|name| CODE_DIRECTIVES.contains(&docs.text(&name.byte_range())));
            let mut stack = vec![node];
            // (The contents of directives other than code blocks are RST, ex:
            // a note's.)
            while let Some(n) = stack.pop() {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children.into_iter().rev() {
                    if child.kind() == "content" && !is_code {
                        rst_nested(docs, child.range(), styles);
                    } else if child.kind() == "body" {
                        stack.push(child);
                    } else {
                        rst_walk(docs, child, styles);
                    }
                }
            }
        }
        "name" => docs.node_piece(&node, Piece::Words(TokenClass::Identifier)),
        "field_name" | "label" => docs.node_piece(&node, Piece::Words(TokenClass::Identifier)),
        _ => {
            if node.child_count() == 0 {
                // (Prose's leaves are `text`; other anonymous ones are
                // markup, ex: `..`, `::`, and bullets.)
                if !node.is_named() && kind != "text" {
                    docs.node_piece(&node, Piece::Words(TokenClass::Operator));
                }
                return;
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                rst_walk(docs, child, styles);
            }
        }
    }
}

/// RST within a document (ex: a directive's content, or MyST's `{eval-rst}`),
/// parsed on its own.
fn rst_nested(docs: &mut Docs, range: tree_sitter::Range, styles: &mut Vec<(char, bool)>) {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rst::LANGUAGE.into())
        .is_err()
        || parser.set_included_ranges(&[range]).is_err()
    {
        docs.piece(
            range.start_byte..range.end_byte,
            Piece::Words(TokenClass::Text),
        );
        return;
    }
    match parser.parse(docs.src, None) {
        Some(tree) => rst_walk(docs, tree.root_node(), styles),
        None => docs.piece(
            range.start_byte..range.end_byte,
            Piece::Words(TokenClass::Text),
        ),
    }
}

pub fn tokenize_rst(source: &str) -> (FinishedTokens, Vec<FileStructureRow>) {
    let mut docs = Docs::new(source);
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rst::LANGUAGE.into())
        .is_ok()
        && let Some(tree) = parser.parse(source, None)
    {
        rst_walk(&mut docs, tree.root_node(), &mut vec![]);
    }
    docs.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(lines: Vec<String>) -> Vec<String> {
        lines
    }

    #[test]
    fn test_markdown_and_rst_agree() {
        let md = "# Title\n\nSee {searchfox}`this file <a/b.js>` and [a link](http://x) with `code`.\n\n## Using `mach`\n\n- One **two**\n\n```python\nprint(1)\n```\n";
        let rst = "Title\n=====\n\nSee :searchfox:`this file <a/b.js>` and `a link <http://x>`__ with ``code``.\n\nUsing ``mach``\n--------------\n\n- One **two**\n\n.. code-block:: python\n\n   print(1)\n";
        let md_tokens = tokens(tokenize_markdown(md).0.lines);
        let rst_tokens = tokens(tokenize_rst(rst).0.lines);
        // (The words, identifiers, and contexts, without markup.)
        let content = |lines: &[String]| -> Vec<String> {
            lines
                .iter()
                .filter(|line| !line.contains(" o "))
                .cloned()
                .collect()
        };
        assert_eq!(
            content(&md_tokens),
            content(&rst_tokens),
            "\n{}\n---\n{}",
            md_tokens.join("\n"),
            rst_tokens.join("\n")
        );
        assert_eq!(
            content(&md_tokens),
            vec![
                "Title t Title",
                "Title t See",
                "Title i searchfox",
                "Title t this",
                "Title t file",
                "Title i a/b.js",
                "Title t and",
                "Title t a",
                "Title t link",
                "Title i http://x",
                "Title t with",
                "Title i code",
                "Title t .",
                "Title::Using%20mach t Using",
                "Title::Using%20mach i mach",
                "Title::Using%20mach t One",
                "Title::Using%20mach t two",
                "Title::Using%20mach i python",
                "Title::Using%20mach t print(1)",
            ]
        );
        // MyST's directives (and their fields), targets, and embedded RST, and
        // RST's directives, targets, and references.
        let md2 = "(intro)=\n# Intro\n\n:owner: Someone\n\n```{note}\nA *note*.\n```\n\n```{eval-rst}\n.. automodule:: mozfile\n   :members:\n```\n\nSee [the docs] and [x](http://x).\n\n[the docs]: http://docs\n";
        let rst2 = ".. _intro:\n\nIntro\n=====\n\n:owner: Someone\n\n.. note::\n\n   A *note*.\n\n.. automodule:: mozfile\n   :members:\n\nSee `the docs`_ and `x <http://x>`__.\n\n.. _the docs: http://docs\n";
        let md_tokens = tokenize_markdown(md2).0.lines;
        let rst_tokens = tokenize_rst(rst2).0.lines;
        let content = |lines: &[String]| -> Vec<String> {
            lines
                .iter()
                .filter(|line| !line.contains(" o ") && !line.ends_with(" i eval-rst"))
                .map(|line| line.split_once(' ').unwrap().1.to_string())
                .collect()
        };
        assert_eq!(
            content(&md_tokens),
            content(&rst_tokens),
            "\n{}\n---\n{}",
            md_tokens.join("\n"),
            rst_tokens.join("\n")
        );
        let structure: Vec<String> = tokenize_markdown(md)
            .1
            .into_iter()
            .map(|row| format!("{}:{}", row.kind, row.pretty))
            .collect();
        assert_eq!(
            structure,
            vec!["section:Title", "section:Title::Using%20mach"]
        );
    }
}
