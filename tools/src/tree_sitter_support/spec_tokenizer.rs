//! Tokenizers for web specs' sources, in a shared "spec" namespace, so that
//! history follows specs between formats (ex: if the HTML Standard moves from
//! Wattsi to Bikeshed, as Firefox's docs moved from RST to Markdown):
//! - `spec-html`: HTML-based sources, ex: the HTML Standard's Wattsi `source`,
//!   ECMA-262's ecmarkup `spec.html`, and older specs' Anolis `.src.html`
//!   and XML sources (and the generated HTML their repositories had early
//!   on).
//!
//! Contexts are the document's hierarchy, like `docs_tokenizer`'s:
//! - sections, nested by their headings' ranks (`<h2>`..`<h6>`, since an
//!   `<h1>` is the document's title), named by their titles' words, without markup or section numbers (so a generated
//!   `index`'s "4.2 The fetch algorithm" is the source's "The fetch
//!   algorithm"), or for ecmarkup, its nested `<emu-clause>`s (and annexes),
//!   named by their abstract operations' names (`aoid`) or their titles
//!   (without operations' parameters);
//! - and within them, algorithms: a `<div algorithm>`, or a "To
//!   <dfn>X</dfn> ...:" paragraph and the `<ol>` of steps after it, named by
//!   the `algorithm` attribute's value or the first `<dfn>`'s words.
//!
//! Sections and algorithms are structure rows.  Tokens:
//! - text is prose words (see `prose_words`);
//! - markup is operators: a start tag's `<name`, each attribute's `name=`
//!   (or `name`), its value's quotes, and the tag's `>`, and end tags;
//! - attributes' values are identifier words (ex: `data-x`'s and `id`'s, and
//!   old specs' `title=` references);
//! - comments' words are comments, and declarations' are operators.
//!
//! Every token is a verbatim substring of the source, in source order.  (We
//! lex the markup ourselves rather than with tree-sitter-html, since all we
//! need is tags, attributes, text, and comments, and their positions, and
//! specs' sources aren't always HTML, ex: Bikeshed's.)

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

// ## Lexing

/// A piece of a document's markup.
#[derive(Debug, Clone, PartialEq)]
enum Item {
    /// A start tag (lowercased name), with its attributes' ranges (name,
    /// value without quotes).
    Start {
        range: Range<usize>,
        name: String,
        attrs: Vec<(Range<usize>, Option<Range<usize>>)>,
    },
    End {
        range: Range<usize>,
        name: String,
    },
    /// `<!-- ... -->`.
    Comment(Range<usize>),
    /// `<!DOCTYPE ...>`, `<?xml ...?>`, `<![CDATA[`'s markup, and the like.
    Declaration(Range<usize>),
    Text(Range<usize>),
}

impl Item {
    fn range(&self) -> &Range<usize> {
        match self {
            Item::Start { range, .. } | Item::End { range, .. } => range,
            Item::Comment(range) | Item::Declaration(range) | Item::Text(range) => range,
        }
    }
}

/// Elements whose contents are text, not markup.
const RAW_TEXT: &[&str] = &["script", "style", "xmp", "textarea", "title"];

fn is_name_char(b: u8) -> bool {
    !b.is_ascii_whitespace() && b != b'>' && b != b'/' && b != b'=' && b != b'<'
}

/// Lex HTML (or XML) into items covering all of `src`.  Markup that isn't
/// well formed is text.
fn lex(src: &str) -> Vec<Item> {
    let bytes = src.as_bytes();
    let mut items = vec![];
    let mut text_start = 0;
    let mut i = 0;
    let flush_text = |items: &mut Vec<Item>, start: usize, end: usize| {
        if start < end {
            items.push(Item::Text(start..end));
        }
    };
    while i < bytes.len() {
        let Some(offset) = bytes[i..].iter().position(|&b| b == b'<') else {
            break;
        };
        i += offset;
        let rest = &src[i..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            let end = comment.find("-->").map_or(src.len(), |e| i + 4 + e + 3);
            flush_text(&mut items, text_start, i);
            items.push(Item::Comment(i..end));
            i = end;
            text_start = i;
        } else if rest.starts_with("<![CDATA[") {
            flush_text(&mut items, text_start, i);
            items.push(Item::Declaration(i..i + 9));
            let content_end = rest.find("]]>").map_or(src.len(), |e| i + e);
            flush_text(&mut items, i + 9, content_end);
            let end = (content_end + 3).min(src.len());
            if content_end < src.len() {
                items.push(Item::Declaration(content_end..end));
            }
            i = end;
            text_start = i;
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            let end = rest.find('>').map_or(src.len(), |e| i + e + 1);
            flush_text(&mut items, text_start, i);
            items.push(Item::Declaration(i..end));
            i = end;
            text_start = i;
        } else if let Some((item, end)) = lex_tag(src, i) {
            flush_text(&mut items, text_start, i);
            // (XHTML's self-closing `<script .../>`s have no contents.)
            let raw = match &item {
                Item::Start { name, range, .. }
                    if RAW_TEXT.contains(&name.as_str()) && !src[range.clone()].ends_with("/>") =>
                {
                    Some(name.clone())
                }
                _ => None,
            };
            items.push(item);
            i = end;
            text_start = i;
            // (A raw text element's contents are text up to its end tag.)
            if let Some(name) = raw {
                let close = format!("</{}", name);
                let content_end = src[i..]
                    .char_indices()
                    .find(|(at, _)| {
                        src[i + at..]
                            .get(..close.len())
                            .is_some_and(|s| s.eq_ignore_ascii_case(&close))
                    })
                    .map_or(src.len(), |(at, _)| i + at);
                flush_text(&mut items, i, content_end);
                i = content_end;
                text_start = i;
                if let Some((end_item, end)) = lex_tag(src, i) {
                    items.push(end_item);
                    i = end;
                    text_start = i;
                }
            }
        } else {
            i += 1;
        }
    }
    flush_text(&mut items, text_start, src.len());
    items
}

/// The start or end tag at `at` (a `<`), and where it ends.
fn lex_tag(src: &str, at: usize) -> Option<(Item, usize)> {
    let bytes = src.as_bytes();
    let mut i = at + 1;
    let end_tag = bytes.get(i) == Some(&b'/');
    if end_tag {
        i += 1;
    }
    if !bytes.get(i)?.is_ascii_alphabetic() {
        return None;
    }
    let name_start = i;
    while i < bytes.len() && is_name_char(bytes[i]) {
        i += 1;
    }
    let name = src[name_start..i].to_ascii_lowercase();
    if end_tag {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'>') {
            return None;
        }
        return Some((
            Item::End {
                range: at..i + 1,
                name,
            },
            i + 1,
        ));
    }
    let mut attrs = vec![];
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i)? {
            b'>' => {
                i += 1;
                break;
            }
            b'/' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                break;
            }
            b'/' => {
                i += 1;
                continue;
            }
            // (A `<` in a tag is a tag that didn't end.)
            b'<' => return None,
            _ => {}
        }
        let attr_start = i;
        while i < bytes.len() && is_name_char(bytes[i]) {
            i += 1;
        }
        if i == attr_start {
            // (A stray `=`.)
            i += 1;
            continue;
        }
        let attr_name_end = i;
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if bytes.get(j) != Some(&b'=') {
            attrs.push((attr_start..attr_name_end, None));
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        match bytes.get(j)? {
            quote @ (b'"' | b'\'') => {
                let close = src[j + 1..].find(*quote as char)? + j + 1;
                attrs.push((attr_start..j, Some(j + 1..close)));
                i = close + 1;
            }
            _ => {
                let value_start = j;
                while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b'>' {
                    j += 1;
                }
                attrs.push((attr_start..value_start, Some(value_start..j)));
                i = j;
            }
        }
    }
    Some((
        Item::Start {
            range: at..i,
            name,
            attrs,
        },
        i,
    ))
}

// ## Contexts

lazy_static! {
    /// A section number at the start of a title, ex: "4.2.1 ".
    static ref SECTION_NUMBER: Regex = Regex::new(r"^(?:[A-Z]\.)?\d+(?:\.\d+)*\.?\s+").unwrap();
}

/// The words of the text of `src[range]`, without its markup.  (Tags don't
/// separate words, ex: "<code>img</code>, and" is "img, and".)
fn plain_words(src: &str, range: Range<usize>) -> String {
    let mut out = String::new();
    for item in lex(&src[range.clone()]) {
        if let Item::Text(r) = item {
            out.push_str(&src[range.start + r.start..range.start + r.end]);
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    words.join(" ")
}

lazy_static! {
    static ref CHAR_REF: Regex = Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z]+);").unwrap();
}

/// `text` with its common character references decoded, ex: "&lt;" as "<".
fn decode_char_refs(text: &str) -> String {
    CHAR_REF
        .replace_all(text, |captures: &regex::Captures| {
            let name = &captures[1];
            let code = if let Some(hex) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()
            } else if let Some(decimal) = name.strip_prefix('#') {
                decimal.parse().ok()
            } else {
                match name {
                    "lt" => Some(0x3c),
                    "gt" => Some(0x3e),
                    "amp" => Some(0x26),
                    "quot" => Some(0x22),
                    "apos" => Some(0x27),
                    "nbsp" => Some(0x20),
                    "mdash" => Some(0x2014),
                    "ndash" => Some(0x2013),
                    "hellip" => Some(0x2026),
                    _ => None,
                }
            };
            code.and_then(char::from_u32)
                .map_or_else(|| captures[0].to_string(), |c| c.to_string())
        })
        .into_owned()
}

/// A section's or algorithm's context name.
fn context_name(words: &str) -> String {
    let words = decode_char_refs(words);
    let words = SECTION_NUMBER.replace(&words, "");
    let words: Vec<&str> = words.split_whitespace().collect();
    escape_name(&words.join(" "))
}

/// A start tag's attribute's value, if it has the attribute.
fn attr<'a>(src: &'a str, item: &Item, name: &str) -> Option<&'a str> {
    let Item::Start { attrs, .. } = item else {
        return None;
    };
    attrs.iter().find_map(|(name_range, value)| {
        let attr_name = src[name_range.clone()].trim_end_matches(['=', ' ', '\t', '\n']);
        attr_name
            .eq_ignore_ascii_case(name)
            .then(|| value.as_ref().map_or("", |value| &src[value.clone()]))
    })
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn is_start(item: &Item, tag: &str) -> bool {
    matches!(item, Item::Start { name, .. } if name == tag)
}

fn is_end(item: &Item, tag: &str) -> bool {
    matches!(item, Item::End { name, .. } if name == tag)
}

/// The index of the end tag of the element starting at `items[start]`
/// (counting nested elements of the same name), if it has one.
fn matching_end(items: &[Item], start: usize, tag: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, item) in items.iter().enumerate().skip(start) {
        if is_start(item, tag) {
            depth += 1;
        } else if is_end(item, tag) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// The words of the first `<dfn>` in `items[from..to]`.
fn first_dfn(src: &str, items: &[Item], from: usize, to: usize) -> Option<String> {
    let start = (from..to).find(|&i| is_start(&items[i], "dfn"))?;
    let end = (start..to).find(|&i| is_end(&items[i], "dfn"))?;
    let words = plain_words(src, items[start].range().end..items[end].range().start);
    (!words.is_empty()).then_some(words)
}

/// Block-level elements, which end a paragraph whose end tag is left out.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "ol",
    "ul",
    "dl",
    "pre",
    "table",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "hr",
    "blockquote",
    "figure",
];

/// A context: a section (by heading, until the next heading of the same or a
/// higher rank) or a range (an algorithm, or an ecmarkup clause).
#[derive(Debug)]
struct Context {
    start: usize,
    /// For ranges, where they end.
    end: Option<usize>,
    /// For sections, their headings' ranks.
    rank: u8,
    name: String,
    kind: &'static str,
}

/// The contexts of an HTML spec source.
fn html_contexts(src: &str, items: &[Item]) -> Vec<Context> {
    let mut contexts = vec![];
    let ecmarkup = items
        .iter()
        .any(|item| is_start(item, "emu-clause") || is_start(item, "emu-annex"));
    for (i, item) in items.iter().enumerate() {
        let Item::Start { name, range, .. } = item else {
            continue;
        };
        match name.as_str() {
            "emu-clause" | "emu-annex" | "emu-intro" => {
                let Some(end) = matching_end(items, i, name) else {
                    continue;
                };
                let title = attr(src, item, "aoid")
                    .filter(|aoid| !aoid.trim().is_empty())
                    .map(|aoid| aoid.to_string())
                    .or_else(|| {
                        let h = (i + 1..end).find(|&j| is_start(&items[j], "h1"))?;
                        let h_end = (h..end).find(|&j| is_end(&items[j], "h1"))?;
                        // (Without an operation's parameters, ex:
                        // "OrdinaryGet ( _O_, _P_ )" is "OrdinaryGet".)
                        let title =
                            plain_words(src, items[h].range().end..items[h_end].range().start);
                        Some(match title.split_once(" (") {
                            Some((name, _)) if !name.is_empty() => name.to_string(),
                            _ => title,
                        })
                    })
                    .unwrap_or_else(|| name.clone());
                contexts.push(Context {
                    start: range.start,
                    end: Some(items[end].range().end),
                    rank: 0,
                    name: context_name(&title),
                    kind: "section",
                });
            }
            // (An `<h1>` is the document's title, not a section.)
            heading if !ecmarkup && heading.len() == 2 && heading.starts_with('h') => {
                let Some(rank) = heading[1..]
                    .parse::<u8>()
                    .ok()
                    .filter(|r| (2..=6).contains(r))
                else {
                    continue;
                };
                // (Up to the next heading, if its end tag is missing.)
                let Some(end) = (i + 1..items.len())
                    .take_while(
                        |&j| !matches!(&items[j], Item::Start { name, .. } if is_heading(name)),
                    )
                    .find(|&j| is_end(&items[j], heading))
                else {
                    continue;
                };
                let title = plain_words(src, range.end..items[end].range().start);
                let title = context_name(&title);
                if title.is_empty() {
                    continue;
                }
                contexts.push(Context {
                    start: range.start,
                    end: None,
                    rank,
                    name: title,
                    kind: "section",
                });
            }
            "div" if attr(src, item, "algorithm").is_some() => {
                let Some(end) = matching_end(items, i, "div") else {
                    continue;
                };
                let name = attr(src, item, "algorithm")
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .or_else(|| first_dfn(src, items, i, end))
                    .unwrap_or_else(|| "algorithm".to_string());
                contexts.push(Context {
                    start: range.start,
                    end: Some(items[end].range().end),
                    rank: 0,
                    name: context_name(&name),
                    kind: "algorithm",
                });
            }
            "p" => {
                // "To <dfn>X</dfn> ...:" and the `<ol>` after the paragraph.
                let starts_with_to = match items.get(i + 1) {
                    Some(Item::Text(text)) => src[text.clone()].trim_start().starts_with("To "),
                    _ => false,
                };
                if !starts_with_to || !items.get(i + 2).is_some_and(|it| is_start(it, "dfn")) {
                    continue;
                }
                let Some(para_end) = (i + 1..items.len()).find(|&j| {
                    is_end(&items[j], "p")
                        || matches!(&items[j], Item::Start { name, .. } if BLOCKS.contains(&name.as_str()))
                }) else {
                    continue;
                };
                let intro = src[range.end..items[para_end].range().start].trim_end();
                if !intro.ends_with(':') {
                    continue;
                }
                let ol = (para_end..items.len()).find(|&j| {
                    !matches!(&items[j], Item::Text(t) if src[t.clone()].trim().is_empty())
                        && !is_end(&items[j], "p")
                });
                let Some(ol) = ol.filter(|&j| is_start(&items[j], "ol")) else {
                    continue;
                };
                let Some(ol_end) = matching_end(items, ol, "ol") else {
                    continue;
                };
                let Some(name) = first_dfn(src, items, i, para_end) else {
                    continue;
                };
                contexts.push(Context {
                    start: range.start,
                    end: Some(items[ol_end].range().end),
                    rank: 0,
                    name: context_name(&name),
                    kind: "algorithm",
                });
            }
            _ => {}
        }
    }
    contexts
}

// ## Emitting

/// Tokens in their contexts.
struct Emitter<'a> {
    src: &'a str,
    contexts: Vec<Context>,
    next: usize,
    /// The sections (rank, name) and the ranges (end, name) the position is
    /// in.
    sections: Vec<(u8, String)>,
    ranges: Vec<(usize, String)>,
    context: String,
    tokens: Vec<RawToken<'a>>,
    structure: Vec<FileStructureRow>,
    rows: HashSet<String>,
}

impl<'a> Emitter<'a> {
    fn new(src: &'a str, mut contexts: Vec<Context>) -> Emitter<'a> {
        contexts.sort_by_key(|c| (c.start, std::cmp::Reverse(c.end)));
        Emitter {
            src,
            contexts,
            next: 0,
            sections: vec![],
            ranges: vec![],
            context: "%".to_string(),
            tokens: vec![],
            structure: vec![],
            rows: HashSet::new(),
        }
    }

    /// Enter and leave the contexts for a token at `pos`.
    fn move_to(&mut self, pos: usize) {
        let mut changed = false;
        while self.ranges.last().is_some_and(|(end, _)| *end <= pos) {
            self.ranges.pop();
            changed = true;
        }
        while let Some(context) = self.contexts.get(self.next)
            && context.start <= pos
        {
            self.next += 1;
            changed = true;
            match context.end {
                // (A range which already ended had no tokens, and an
                // algorithm's own "To <dfn>X</dfn>" paragraph in its `<div
                // algorithm>` is the same algorithm.)
                Some(end) if end <= pos => continue,
                Some(_)
                    if self
                        .ranges
                        .last()
                        .is_some_and(|(_, name)| *name == context.name) =>
                {
                    continue;
                }
                Some(end) => self.ranges.push((end, context.name.clone())),
                None => {
                    // (A section's heading ends the ranges it's in, ex: an
                    // algorithm without its end tag.)
                    self.ranges.clear();
                    while self
                        .sections
                        .last()
                        .is_some_and(|(r, _)| *r >= context.rank)
                    {
                        self.sections.pop();
                    }
                    self.sections.push((context.rank, context.name.clone()));
                }
            }
            let kind = context.kind;
            self.update_context();
            if self.rows.insert(self.context.clone()) {
                self.structure.push(FileStructureRow {
                    pretty: self.context.clone(),
                    is_def: true,
                    kind: kind.to_string(),
                    pp: vec![],
                });
            }
        }
        if changed {
            self.update_context();
        }
    }

    fn update_context(&mut self) {
        let names: Vec<&str> = self
            .sections
            .iter()
            .map(|(_, name)| name.as_str())
            .chain(self.ranges.iter().map(|(_, name)| name.as_str()))
            .collect();
        self.context = if names.is_empty() {
            "%".to_string()
        } else {
            names.join("::")
        };
    }

    fn token(&mut self, class: TokenClass, range: Range<usize>) {
        let text = &self.src[range.clone()];
        if text.trim().is_empty() {
            return;
        }
        self.move_to(range.start);
        self.tokens.push(RawToken {
            context: self.context.clone(),
            class,
            text: text.trim(),
        });
    }

    /// Words of `range`: prose's (with their punctuation; see
    /// `prose_words`) for text and comments, or else split on whitespace.
    fn words(&mut self, class: TokenClass, range: Range<usize>) {
        let text = &self.src[range.clone()];
        let base = text.as_ptr() as usize;
        let words: Vec<&'a str> = if matches!(class, TokenClass::Text | TokenClass::Comment) {
            prose_words(text).collect()
        } else {
            text.split_whitespace().collect()
        };
        for word in words {
            let start = range.start + (word.as_ptr() as usize - base);
            self.move_to(start);
            self.tokens.push(RawToken {
                context: self.context.clone(),
                class,
                text: word,
            });
        }
    }

    /// A start tag's tokens.
    fn start_tag(&mut self, range: &Range<usize>, attrs: &[(Range<usize>, Option<Range<usize>>)]) {
        let src = self.src;
        let name_end = src[range.start + 1..range.end]
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .map_or(range.end, |i| range.start + 1 + i);
        self.token(TokenClass::Operator, range.start..name_end);
        let mut pos = name_end;
        for (name, value) in attrs {
            self.token(TokenClass::Operator, name.clone());
            pos = name.end;
            if let Some(value) = value {
                // (Quotes are operators of their own.)
                self.token(TokenClass::Operator, pos..value.start);
                self.words(TokenClass::Identifier, value.clone());
                pos = value.end;
                let quoted = src[pos..range.end].starts_with(['"', '\'']);
                if quoted {
                    self.token(TokenClass::Operator, pos..pos + 1);
                    pos += 1;
                }
            }
        }
        // `>` or `/>`.
        self.token(TokenClass::Operator, pos..range.end);
    }

    fn finish(self) -> (FinishedTokens, Vec<FileStructureRow>) {
        (finish_tokens(self.src, self.tokens), self.structure)
    }
}

/// Emit the tokens of lexed markup.
fn emit_items(out: &mut Emitter, items: &[Item]) {
    for item in items {
        match item {
            Item::Start { range, attrs, .. } => out.start_tag(range, attrs),
            Item::End { range, .. } => out.token(TokenClass::Operator, range.clone()),
            Item::Comment(range) => {
                let inner_start = (range.start + 4).min(range.end);
                let inner_end =
                    if out.src[range.clone()].ends_with("-->") && range.end - 3 >= inner_start {
                        range.end - 3
                    } else {
                        range.end
                    };
                out.token(TokenClass::Operator, range.start..inner_start);
                out.words(TokenClass::Comment, inner_start..inner_end);
                out.token(TokenClass::Operator, inner_end..range.end);
            }
            Item::Declaration(range) => out.words(TokenClass::Operator, range.clone()),
            Item::Text(range) => out.words(TokenClass::Text, range.clone()),
        }
    }
}

/// Tokenize an HTML-based spec source (see the module's documentation).
pub fn tokenize_spec_html(source: &str) -> (FinishedTokens, Vec<FileStructureRow>) {
    let items = lex(source);
    let contexts = html_contexts(source, &items);
    let mut out = Emitter::new(source, contexts);
    emit_items(&mut out, &items);
    out.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens as (context, class letter, text).
    fn tokens(source: &str) -> Vec<(String, char, String)> {
        let (finished, _) = tokenize_spec_html(source);
        finished
            .lines
            .iter()
            .map(|line| {
                let token = crate::file_format::history::syntax_files::split_token_line(line);
                (
                    token.context.to_string(),
                    token.class.as_char(),
                    token.token.to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn test_lex() {
        let src = "<p class=note>A <a href=\"#x\" title='t u'>b</a> &amp; c<br/><!-- d --></p>";
        let items = lex(src);
        let kinds: Vec<String> = items
            .iter()
            .map(|item| match item {
                Item::Start { name, attrs, .. } => format!("<{} {}>", name, attrs.len()),
                Item::End { name, .. } => format!("</{}>", name),
                Item::Comment(_) => "comment".to_string(),
                Item::Declaration(_) => "decl".to_string(),
                Item::Text(r) => format!("{:?}", &src[r.clone()]),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "<p 1>",
                "\"A \"",
                "<a 2>",
                "\"b\"",
                "</a>",
                "\" &amp; c\"",
                "<br 0>",
                "comment",
                "</p>"
            ]
        );
        // (Not tags.)
        let items = lex("a < b, c <3, <?xml?>");
        assert!(matches!(items[0], Item::Text(_)));
        assert!(matches!(items.last(), Some(Item::Declaration(_))));
    }

    #[test]
    fn test_tokens_and_contexts() {
        let src = concat!(
            "<h2 id=infra>2 Infrastructure</h2>\n",
            "<p>Some <dfn data-x=concept-x>x</dfn>.</p>\n",
            "<h3>Fetching <span>things</span></h3>\n",
            "<p>To <dfn>fetch</dfn> a <var>thing</var>:</p>\n",
            "<ol><li><p>Do it.</p></li></ol>\n",
            "<p>After.\n",
            "<div algorithm>\n<p>To <dfn>wait</dfn>, run:</p>\n</div>\n",
            "<h2>Next</h2><!-- note -->\n",
        );
        let tokens = tokens(src);
        let find = |text: &str| tokens.iter().find(|t| t.2 == text).unwrap().clone();
        // (A heading's markup is in its section.)
        assert_eq!(
            find("<h2"),
            ("Infrastructure".to_string(), 'o', "<h2".to_string())
        );
        assert_eq!(
            find("id="),
            ("Infrastructure".to_string(), 'o', "id=".to_string())
        );
        assert_eq!(
            find("concept-x"),
            ("Infrastructure".to_string(), 'i', "concept-x".to_string())
        );
        assert_eq!(
            find("Some"),
            ("Infrastructure".to_string(), 't', "Some".to_string())
        );
        assert_eq!(find("Do").0, "Infrastructure::Fetching%20things::fetch");
        assert_eq!(find("After").0, "Infrastructure::Fetching%20things");
        assert_eq!(find("run").0, "Infrastructure::Fetching%20things::wait");
        assert_eq!(find("note"), ("Next".to_string(), 'c', "note".to_string()));
        // (Punctuation is its own token.)
        assert!(tokens.iter().any(|t| t.2 == "." && t.1 == 't'));
        // Every token is in the source, in order.
        let mut pos = 0;
        for (_, _, text) in &tokens {
            let at = src[pos..].find(text.as_str()).unwrap();
            pos += at + text.len();
        }
        let (_, structure) = tokenize_spec_html(src);
        let rows: Vec<(&str, &str)> = structure
            .iter()
            .map(|row| (row.pretty.as_str(), row.kind.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Infrastructure", "section"),
                ("Infrastructure::Fetching%20things", "section"),
                ("Infrastructure::Fetching%20things::fetch", "algorithm"),
                ("Infrastructure::Fetching%20things::wait", "algorithm"),
                ("Next", "section"),
            ]
        );
    }

    #[test]
    fn test_ecmarkup() {
        let src = concat!(
            "<emu-clause id=\"sec-a\">\n<h1>Objects</h1>\n",
            "<emu-clause id=\"sec-get\" aoid=\"OrdinaryGet\">\n<h1>OrdinaryGet ( _O_ )</h1>\n",
            "<emu-alg>\n1. Return _O_.\n</emu-alg>\n</emu-clause>\n",
            "<emu-clause id=\"sec-set\" type=\"abstract operation\">\n",
            "<h1>OrdinarySet ( _O_: an Object )</h1>\n<p>Sets.</p>\n</emu-clause>\n",
            "<p>More.</p>\n</emu-clause>\n",
        );
        let tokens = tokens(src);
        let find = |text: &str| tokens.iter().find(|t| t.2 == text).unwrap().0.clone();
        assert_eq!(find("Objects"), "Objects");
        assert_eq!(find("Return"), "Objects::OrdinaryGet");
        // (Without its parameters.)
        assert_eq!(find("Sets"), "Objects::OrdinarySet");
        assert_eq!(find("More"), "Objects");
    }

    #[test]
    fn test_titles_and_nesting() {
        let src = concat!(
            "<h1>The Standard</h1>\n",
            "<h2>Attributes common to <code>img</code>, and <code>link</code></h2>\n",
            "<div algorithm>\n<p>To <dfn>go</dfn>, run:</p>\n<ol><li>Go.</li></ol>\n</div>\n",
            "<script src=x.js/><h3>After</h3>\n",
        );
        let tokens = tokens(src);
        let find = |text: &str| tokens.iter().find(|t| t.2 == text).unwrap().0.clone();
        // (The document's title isn't a section, and tags don't separate
        // words.)
        assert_eq!(find("The"), "%");
        assert_eq!(
            find("common"),
            "Attributes%20common%20to%20img,%20and%20link"
        );
        // (An algorithm's own paragraph doesn't nest it again.)
        assert_eq!(
            find("Go"),
            "Attributes%20common%20to%20img,%20and%20link::go"
        );
        assert_eq!(
            decode_char_refs("a &lt;b&gt; &#x41;&#66; &bogus; &"),
            "a <b> AB &bogus; &"
        );
        // (A self-closing script has no contents.)
        assert_eq!(
            find("After"),
            "Attributes%20common%20to%20img,%20and%20link::After"
        );
    }

    #[test]
    fn test_raw_text_and_unquoted() {
        let src =
            "<pre class=idl>interface X {};</pre><script>if (a < b) {}</script><h4 id=x>X</h4>";
        let tokens = tokens(src);
        let texts: Vec<&str> = tokens.iter().map(|t| t.2.as_str()).collect();
        assert!(texts.contains(&"idl"));
        assert!(texts.contains(&"<"));
        assert!(texts.contains(&"</script>"));
        assert_eq!(tokens.last().unwrap().0, "X");
    }
}
