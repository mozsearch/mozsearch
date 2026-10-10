//! Rendered views of documentation (Markdown, with Sphinx's MyST extensions,
//! and reStructuredText), whose blocks (paragraphs, headings, code blocks,
//! table rows, ...) each get a blame strip, like the lines of source listings.
//!
//! Rendering keeps track of which bytes of the file each block came from, so
//! that `block_lines` can give each block the lines it covers (and the lines
//! which don't render as anything, like link definitions), and each run of
//! text that's in the file verbatim says where it is (its line and UTF-16
//! column, as `data-l` and `data-c` attributes), so that the blame popup can
//! find the tokens of the hyperblame data files (whose positions are UTF-16
//! columns of lines) in the rendered text.  The popup works on the visual
//! lines of the block, as the browser wrapped them; see `RenderedBlocks` in
//! hyperblame.js.

use std::collections::HashSet;
use std::fmt::Write;
use std::ops::{Range, RangeInclusive};

use lazy_static::lazy_static;
use regex::Regex;

mod doc;
pub mod markdown;
pub mod rst;

/// How rendered documents link to other files.  Paths are relative to the
/// root of the tree.
pub trait Links {
    /// Whether the tree has the file (or directory) at `path`.
    fn exists(&self, path: &str) -> bool;
    /// The URL of the source listing of `path`.
    fn source_url(&self, path: &str) -> String;
    /// The URL of the rendered view of the document at `path`.
    fn rendered_url(&self, path: &str) -> String;
    /// The URL of the raw contents of `path`, for images.
    fn raw_url(&self, path: &str) -> Option<String>;
    /// The URL of a search for `query`.
    fn search_url(&self, query: &str) -> String;
    /// The tree's name, for Sphinx's `searchfox` role, which links to the
    /// tree's source listings by default.
    fn tree_name(&self) -> &str;
    /// The source path of a document by its path in the documentation's
    /// staged layout (ex: Firefox's `SPHINX_TREES`), without an extension.
    fn staged_doc(&self, _staged: &str) -> Option<String> {
        None
    }
}

/// A block of the rendered document, which gets a blame strip.
#[derive(Debug, PartialEq)]
pub struct Block {
    /// The bytes of the file it came from.
    pub range: Range<usize>,
    /// Where its strip goes in the HTML.
    pub strip_at: usize,
}

/// A rendered document.
#[derive(Debug)]
pub struct Rendered {
    /// The HTML, without the blocks' strips, which go at their `strip_at`.
    pub html: String,
    pub blocks: Vec<Block>,
    /// The text of the first heading, if any.
    pub title: Option<String>,
}

/// Maps offsets in a virtual document (ex: the content of a directive,
/// without the indentation of its container) to offsets in the file.
#[derive(Clone, Debug)]
pub struct OffsetMap {
    /// Pieces, contiguous in the virtual document.
    pieces: Vec<Piece>,
}

#[derive(Clone, Debug)]
struct Piece {
    virt: usize,
    real: usize,
    len: usize,
    /// Whether the piece is in the file verbatim, so that its offsets map one
    /// to one.
    exact: bool,
}

impl OffsetMap {
    pub fn identity(len: usize) -> OffsetMap {
        OffsetMap {
            pieces: vec![Piece {
                virt: 0,
                real: 0,
                len,
                exact: true,
            }],
        }
    }

    /// A map built from pieces of the virtual document that `self` maps
    /// (their ranges there, and their lengths in the new document, which
    /// differ if they're not verbatim), which are concatenated to make a new
    /// virtual document.
    pub fn compose_pieces(&self, pieces: &[(Range<usize>, usize)]) -> OffsetMap {
        let mut out = vec![];
        let mut start = 0;
        for (range, len) in pieces {
            let exact = range.len() == *len && self.exact(range.clone()).is_some();
            out.push(Piece {
                virt: start,
                real: self.real(range.start),
                len: *len,
                exact,
            });
            start += len;
        }
        if out.is_empty() {
            out.push(Piece {
                virt: 0,
                real: 0,
                len: 0,
                exact: false,
            });
        }
        OffsetMap { pieces: out }
    }

    pub fn compose(&self, pieces: &[Range<usize>]) -> OffsetMap {
        let pieces: Vec<_> = pieces
            .iter()
            .map(|range| (range.clone(), range.len()))
            .collect();
        self.compose_pieces(&pieces)
    }

    /// The map of a slice of the virtual document.
    pub fn slice(&self, range: Range<usize>) -> OffsetMap {
        let mut out = vec![];
        for piece in &self.pieces {
            let start = piece.virt.max(range.start);
            let end = (piece.virt + piece.len).min(range.end);
            if start < end {
                out.push(Piece {
                    virt: start - range.start,
                    real: piece.real + if piece.exact { start - piece.virt } else { 0 },
                    len: end - start,
                    exact: piece.exact,
                });
            }
        }
        if out.is_empty() {
            out.push(Piece {
                virt: 0,
                real: self.real(range.start),
                len: 0,
                exact: false,
            });
        }
        OffsetMap { pieces: out }
    }

    fn piece(&self, offset: usize) -> &Piece {
        let i = self
            .pieces
            .partition_point(|piece| piece.virt <= offset)
            .saturating_sub(1);
        &self.pieces[i]
    }

    /// The offset in the file of an offset in the virtual document.
    pub fn real(&self, offset: usize) -> usize {
        let piece = self.piece(offset);
        if piece.exact {
            piece.real + (offset.saturating_sub(piece.virt)).min(piece.len)
        } else {
            piece.real
        }
    }

    /// The range of the file of a range of the virtual document, if it's all
    /// in one verbatim piece.
    pub fn exact(&self, range: Range<usize>) -> Option<Range<usize>> {
        let piece = self.piece(range.start);
        if !piece.exact || range.start < piece.virt || range.end > piece.virt + piece.len {
            return None;
        }
        Some(piece.real + range.start - piece.virt..piece.real + range.end - piece.virt)
    }

    /// The range of the file of a range of the virtual document, which may
    /// span pieces.
    pub fn span(&self, range: Range<usize>) -> Range<usize> {
        let start = self.real(range.start);
        let end = if range.end > range.start {
            self.real(range.end - 1) + 1
        } else {
            start
        };
        start..end.max(start)
    }
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    escape_into(&mut out, text);
    out
}

fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The HTML of a rendered document as it's written, with its blocks.
pub struct Writer<'s> {
    /// The file.
    src: &'s str,
    line_starts: Vec<usize>,
    pub html: String,
    pub blocks: Vec<Block>,
    /// The open block, which can't have blocks within it.
    open: Option<usize>,
    /// The ids of elements, which need to be unique.
    ids: HashSet<String>,
    pub title: Option<String>,
}

impl<'s> Writer<'s> {
    pub fn new(src: &'s str) -> Writer<'s> {
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Writer {
            src,
            line_starts,
            html: String::new(),
            blocks: vec![],
            open: None,
            ids: HashSet::new(),
            title: None,
        }
    }

    /// The file.
    pub fn src(&self) -> &'s str {
        self.src
    }

    pub fn finish(self) -> Rendered {
        Rendered {
            html: self.html,
            blocks: self.blocks,
            title: self.title,
        }
    }

    pub fn push(&mut self, html: &str) {
        self.html.push_str(html);
    }

    pub fn escaped(&mut self, text: &str) {
        escape_into(&mut self.html, text);
    }

    pub fn in_block(&self) -> bool {
        self.open.is_some()
    }

    /// Start a block for the bytes `range` of the file, unless one is already
    /// open, returning whether it did.  The strip goes here, unless `place` is
    /// false, in which case it goes wherever `place_strip` is next called (ex:
    /// a table row's first cell).
    pub fn start_block(&mut self, range: Range<usize>, place: bool) -> bool {
        if self.open.is_some() {
            return false;
        }
        self.open = Some(self.blocks.len());
        self.blocks.push(Block {
            range,
            strip_at: usize::MAX,
        });
        if place {
            self.place_strip();
        }
        true
    }

    pub fn place_strip(&mut self) {
        if let Some(i) = self.open
            && self.blocks[i].strip_at == usize::MAX
        {
            self.blocks[i].strip_at = self.html.len();
        }
    }

    /// Extend the open block to cover `range` too.
    pub fn extend_block(&mut self, range: Range<usize>) {
        if let Some(i) = self.open {
            let block = &mut self.blocks[i].range;
            block.start = block.start.min(range.start);
            block.end = block.end.max(range.end);
        }
    }

    pub fn end_block(&mut self) {
        if let Some(i) = self.open.take()
            && self.blocks[i].strip_at == usize::MAX
        {
            self.blocks[i].strip_at = self.html.len();
        }
    }

    /// `<div class="rd-block ...">` and the strip, if a block can start here.
    pub fn open_block_div(&mut self, range: Range<usize>, class: &str) -> bool {
        if self.open.is_some() {
            return false;
        }
        write!(
            self.html,
            r#"<div class="rd-block{}{}">"#,
            if class.is_empty() { "" } else { " " },
            class
        )
        .unwrap();
        self.start_block(range, true)
    }

    pub fn close_block_div(&mut self, opened: bool) {
        if opened {
            self.end_block();
            self.html.push_str("</div>");
        }
    }

    /// The (0-based) line of a byte of the file.
    pub fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= offset) - 1
    }

    /// The end of the line with the byte `offset` (before its newline).
    pub fn line_end(&self, offset: usize) -> usize {
        let line = self.line_of(offset);
        self.line_starts
            .get(line + 1)
            .map_or(self.src.len(), |next| next - 1)
    }

    /// The start of the line after the one with the byte `offset`.
    pub fn next_line_start(&self, offset: usize) -> usize {
        let line = self.line_of(offset);
        self.line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.src.len())
    }

    /// Text from the bytes `real` of the file, if known: a run that the popup
    /// can find tokens in (see the module's comment), if it's in a block and
    /// on one line, or just the text.
    pub fn text(&mut self, text: &str, real: Option<Range<usize>>) {
        // Each line of a code block's text is its own run.
        if text.contains('\n') {
            let mut start = 0;
            for piece in text.split_inclusive('\n') {
                let line = piece.trim_end_matches('\n');
                let real = real
                    .as_ref()
                    .filter(|real| real.len() == text.len())
                    .map(|real| real.start + start..real.start + start + line.len());
                self.text(line, real);
                self.html.push_str(&piece[line.len()..]);
                start += piece.len();
            }
            return;
        }
        let real = real.filter(|real| {
            self.open.is_some()
                && !text.is_empty()
                && real.end <= self.src.len()
                && self.src.is_char_boundary(real.start)
                && self.src.is_char_boundary(real.end)
                && !self.src[real.clone()].contains('\n')
        });
        let Some(real) = real else {
            self.escaped(text);
            return;
        };
        let line = self.line_of(real.start);
        let col = utf16_len(&self.src[self.line_starts[line]..real.start]);
        let len = utf16_len(&self.src[real.clone()]);
        write!(self.html, r#"<span data-l="{}" data-c="{}""#, line + 1, col).unwrap();
        if len != utf16_len(text) {
            write!(self.html, r#" data-n="{}""#, len).unwrap();
        }
        self.html.push('>');
        self.escaped(text);
        self.html.push_str("</span>");
    }

    /// Text which is the bytes `real` of the file.
    pub fn src_text(&mut self, real: Range<usize>) {
        let text = &self.src[real.clone()];
        self.text(text, Some(real));
    }

    /// A unique id based on `base`.
    pub fn unique_id(&mut self, base: &str) -> String {
        let base = if base.is_empty() { "section" } else { base };
        let mut id = base.to_string();
        let mut n = 1;
        while self.ids.contains(&id) {
            id = format!("{}-{}", base, n);
            n += 1;
        }
        self.ids.insert(id.clone());
        id
    }

    /// Claim an id (ex: a Sphinx target's label), if it's not taken.
    pub fn claim_id(&mut self, id: &str) -> bool {
        self.ids.insert(id.to_string())
    }
}

/// The anchor of a heading, like MyST's `myst_heading_anchors` (GitHub's
/// slugs): lowercase, without punctuation, and with dashes for spaces.
pub fn slugify(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Resolve a path relative to the directory `dir` (or the root, if it starts
/// with "/"), unless it leaves the tree.
pub fn resolve_path(dir: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = if path.starts_with('/') {
        vec![]
    } else {
        dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

pub fn is_doc_path(path: &str) -> bool {
    path.ends_with(".md") || path.ends_with(".rst") || path.ends_with(".markdown")
}

/// The file of a Sphinx document named relative to `dir` (or the root of the
/// documentation, if it starts with "/"), without its extension, as Sphinx's
/// `doc` role and `toctree` directive do.
pub fn find_doc(links: &dyn Links, dir: &str, name: &str) -> Option<String> {
    let name = name.trim();
    let candidates = |base: &str| {
        let mut paths = vec![];
        if is_doc_path(base) {
            paths.push(base.to_string());
        }
        for ext in ["md", "rst"] {
            paths.push(format!("{}.{}", base, ext));
        }
        for ext in ["md", "rst"] {
            paths.push(format!("{}/index.{}", base, ext));
        }
        paths
    };
    let mut bases = vec![];
    if let Some(staged) = name.strip_prefix('/') {
        if let Some(source) = links.staged_doc(staged.trim_end_matches('/')) {
            bases.push(source);
        }
        bases.push(staged.to_string());
    } else if let Some(path) = resolve_path(dir, name) {
        bases.push(path);
    }
    bases
        .iter()
        .flat_map(|base| candidates(base))
        .find(|path| links.exists(path))
}

/// The URL for a link's destination in the document at a path in `dir`: as is
/// for URLs with safe schemes and fragments, the rendered view of documents,
/// and the source listing of other files.  None for unsafe schemes (ex:
/// `javascript:`).
pub fn link_url(links: &dyn Links, dir: &str, dest: &str) -> Option<String> {
    lazy_static! {
        static ref SCHEME: Regex = Regex::new(r"^([A-Za-z][A-Za-z0-9+.-]*):").unwrap();
    }
    let dest = dest.trim();
    if dest.is_empty() {
        return None;
    }
    if dest.starts_with('#') || dest.starts_with("//") {
        return Some(dest.to_string());
    }
    let mut dest = dest;
    if let Some(captures) = SCHEME.captures(dest) {
        match captures[1].to_ascii_lowercase().as_str() {
            "http" | "https" | "mailto" | "ftp" | "irc" | "ircs" => {
                return Some(dest.to_string());
            }
            // MyST's links to files of the project.
            "project" | "path" => dest = &dest[captures[0].len()..],
            _ => return None,
        }
    }
    let (path, fragment) = match dest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (dest, None),
    };
    let path = percent_decode(path);
    let resolved = resolve_path(dir, &path)?;
    let with_fragment = |url: String| match fragment {
        Some(fragment) => format!("{}#{}", url, fragment),
        None => url,
    };
    if is_doc_path(&resolved) {
        return Some(with_fragment(links.rendered_url(&resolved)));
    }
    if !links.exists(&resolved)
        && let Some(doc) = find_doc(links, dir, &path)
    {
        return Some(with_fragment(links.rendered_url(&doc)));
    }
    Some(with_fragment(links.source_url(&resolved)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// The URL of an image, which we load from the raw contents of files in the
/// tree, or from https URLs.
pub fn image_url(links: &dyn Links, dir: &str, src: &str) -> Option<String> {
    let src = src.trim();
    if src.starts_with("https://") {
        return Some(src.to_string());
    }
    if src.contains(':') || src.starts_with("//") {
        return None;
    }
    let path = resolve_path(dir, &percent_decode(src))?;
    links.raw_url(&path)
}

/// The URL of the `searchfox` role's target, which is `PATH`,
/// `TREE:PATH`, or `TREE/rev/REV:PATH` (see `python/mozbuild/mozbuild/sphinx.py`
/// in Firefox).  The role links to firefox-main by default, but we link to the
/// tree we're in.
pub fn searchfox_url(links: &dyn Links, target: &str) -> (String, String) {
    let (source, path) = match target.split_once(':') {
        Some((source, path)) if !source.contains('#') => (source.to_string(), path.to_string()),
        _ => (links.tree_name().to_string(), target.to_string()),
    };
    let source = if source.contains("/rev/") {
        source
    } else {
        format!("{}/source", source)
    };
    let path = path.trim_start_matches('/');
    let url = format!("/{}/{}", source, path);
    (url, path.to_string())
}

/// Allowed HTML elements in documents' raw HTML, which only keeps their
/// allowed attributes.  Inline ones are allowed anywhere, and the others only
/// in blocks of HTML.
const INLINE_ELEMENTS: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "br", "cite", "code", "data", "del", "dfn", "em", "i", "img",
    "ins", "kbd", "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "u",
    "var", "wbr",
];
const BLOCK_ELEMENTS: &[&str] = &[
    "blockquote",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "li",
    "ol",
    "p",
    "picture",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];
const VOID_ELEMENTS: &[&str] = &["br", "col", "hr", "img", "wbr"];
const ATTRIBUTES: &[&str] = &[
    "align", "alt", "colspan", "height", "href", "open", "rowspan", "src", "start", "title",
    "width",
];

/// Sanitizes documents' raw HTML: only the allowed elements and attributes,
/// with safe URLs, and only closing elements it opened, so that it can't
/// break the page.
#[derive(Default)]
pub struct HtmlSanitizer {
    open: Vec<String>,
}

impl HtmlSanitizer {
    /// Write the sanitized `html` (allowing block elements if `block`) to
    /// `out`, with the text in it escaped.  `dir` is the document's directory,
    /// for relative URLs.
    pub fn write(
        &mut self,
        out: &mut Writer,
        html: &str,
        block: bool,
        links: &dyn Links,
        dir: &str,
    ) {
        lazy_static! {
            static ref TAG: Regex = Regex::new(
                r#"(?s)<!--.*?-->|<(/?)([A-Za-z][A-Za-z0-9-]*)((?:\s+[^\s"'>/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'=<>`]+))?)*)\s*(/?)>"#
            )
            .unwrap();
            static ref ATTR: Regex = Regex::new(
                r#"([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?"#
            )
            .unwrap();
        }
        let mut pos = 0;
        for m in TAG.find_iter(html) {
            self.write_text(out, &html[pos..m.start()]);
            pos = m.end();
            let Some(captures) = TAG.captures(m.as_str()) else {
                continue;
            };
            let Some(name) = captures.get(2) else {
                // A comment.
                continue;
            };
            let name = name.as_str().to_ascii_lowercase();
            let allowed = INLINE_ELEMENTS.contains(&name.as_str())
                || (block && BLOCK_ELEMENTS.contains(&name.as_str()));
            if !allowed {
                continue;
            }
            if &captures[1] == "/" {
                if let Some(i) = self.open.iter().rposition(|open| *open == name) {
                    for open in self.open.drain(i..).rev() {
                        write!(out.html, "</{}>", open).unwrap();
                    }
                }
                continue;
            }
            write!(out.html, "<{}", name).unwrap();
            for attr in ATTR.captures_iter(&captures[3]) {
                let attr_name = attr[1].to_ascii_lowercase();
                if !ATTRIBUTES.contains(&attr_name.as_str()) {
                    continue;
                }
                let value = attr
                    .get(2)
                    .or(attr.get(3))
                    .or(attr.get(4))
                    .map_or("", |v| v.as_str());
                let value = decode_entities(value);
                let value = match attr_name.as_str() {
                    "href" => link_url(links, dir, &value),
                    "src" => image_url(links, dir, &value),
                    _ => Some(value),
                };
                if let Some(value) = value {
                    write!(out.html, r#" {}="{}""#, attr_name, escape(&value)).unwrap();
                }
            }
            out.html.push('>');
            if !VOID_ELEMENTS.contains(&name.as_str()) && captures[4].is_empty() {
                self.open.push(name);
            }
        }
        self.write_text(out, &html[pos..]);
    }

    /// Text of raw HTML, which may have entities.
    fn write_text(&self, out: &mut Writer, text: &str) {
        lazy_static! {
            static ref ENTITY: Regex =
                Regex::new(r"^&(?:[A-Za-z][A-Za-z0-9]*|#[0-9]+|#[xX][0-9A-Fa-f]+);").unwrap();
        }
        let mut rest = text;
        while let Some(i) = rest.find(['&', '<', '>', '"']) {
            out.html.push_str(&rest[..i]);
            rest = &rest[i..];
            if let Some(m) = ENTITY.find(rest) {
                out.html.push_str(m.as_str());
                rest = &rest[m.end()..];
                continue;
            }
            let c = rest.chars().next().unwrap();
            escape_into(&mut out.html, &rest[..c.len_utf8()]);
            rest = &rest[c.len_utf8()..];
        }
        out.html.push_str(rest);
    }

    /// Close the elements it opened.
    pub fn close_all(&mut self, out: &mut Writer) {
        for open in self.open.drain(..).rev() {
            write!(out.html, "</{}>", open).unwrap();
        }
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Assign the file's lines to the blocks for their blame: each block's lines,
/// plus the lines with tokens that aren't in any block (ex: link definitions,
/// the delimiter row of a table, Sphinx targets, the ends of directives), which
/// go with the block before them if they're right after it, or else with the
/// block right after them, or else the block before them.  `has_tokens` says
/// whether a (0-based) line has tokens.
pub fn block_lines(
    blocks: &[Block],
    line_of: impl Fn(usize) -> usize,
    num_lines: usize,
    has_tokens: impl Fn(usize) -> bool,
) -> Vec<Option<RangeInclusive<usize>>> {
    // Each block's own lines, without overlapping the previous block's.
    let mut ranges: Vec<Option<(usize, usize)>> = vec![];
    let mut next_free = 0;
    for block in blocks {
        let first = line_of(block.range.start).max(next_free);
        let last = line_of(block.range.end.saturating_sub(1).max(block.range.start));
        if first <= last && first < num_lines {
            let last = last.min(num_lines - 1);
            ranges.push(Some((first, last)));
            next_free = last + 1;
        } else {
            ranges.push(None);
        }
    }

    // The block covering each line.
    let mut owner: Vec<Option<usize>> = vec![None; num_lines];
    for (i, range) in ranges.iter().enumerate() {
        if let Some((first, last)) = range {
            owner[*first..=*last].fill(Some(i));
        }
    }

    // Runs of lines with tokens that no block covers.
    let mut line = 0;
    while line < num_lines {
        if owner[line].is_some() || !has_tokens(line) {
            line += 1;
            continue;
        }
        let start = line;
        while line < num_lines && owner[line].is_none() {
            line += 1;
        }
        // Trailing lines without tokens don't count.
        let mut end = line - 1;
        while end > start && !has_tokens(end) {
            end -= 1;
        }
        let before = (0..start).rev().find_map(|l| owner[l]);
        let after = (end + 1..num_lines).find_map(|l| owner[l]);
        let adjacent_before = start > 0 && owner[start - 1].is_some();
        let adjacent_after = end + 1 < num_lines && owner[end + 1].is_some();
        let target = match (before, after) {
            (Some(before), _) if adjacent_before => Some(before),
            (_, Some(after)) if adjacent_after => Some(after),
            (Some(before), _) => Some(before),
            (None, after) => after,
        };
        if let Some(target) = target {
            let range = ranges[target].as_mut().unwrap();
            range.0 = range.0.min(start);
            range.1 = range.1.max(end);
            owner[range.0..=range.1].fill(Some(target));
        }
    }

    ranges
        .into_iter()
        .map(|range| range.map(|(first, last)| first..=last))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub struct TestLinks;

    impl Links for TestLinks {
        fn exists(&self, path: &str) -> bool {
            matches!(
                path,
                "docs/other.md" | "docs/sub/index.rst" | "dom/base/Element.cpp"
            )
        }
        fn source_url(&self, path: &str) -> String {
            format!("/t/source/{}", path)
        }
        fn rendered_url(&self, path: &str) -> String {
            format!("/t/rendered/{}", path)
        }
        fn raw_url(&self, path: &str) -> Option<String> {
            Some(format!("https://raw.example/{}", path))
        }
        fn search_url(&self, query: &str) -> String {
            format!("/t/search?q={}", query)
        }
        fn tree_name(&self) -> &str {
            "t"
        }
    }

    #[test]
    fn test_offset_map() {
        // "ab\n  cd\n" with the second line's indentation removed: "ab\ncd\n".
        let map = OffsetMap::identity(8).compose(&[0..3, 5..8]);
        assert_eq!(map.real(0), 0);
        assert_eq!(map.real(3), 5);
        assert_eq!(map.exact(3..5), Some(5..7));
        assert_eq!(map.exact(1..4), None);
        assert_eq!(map.span(1..5), 1..7);
        let slice = map.slice(3..6);
        assert_eq!(slice.real(0), 5);
        assert_eq!(slice.exact(0..2), Some(5..7));
    }

    #[test]
    fn test_links() {
        let links = TestLinks;
        assert_eq!(
            resolve_path("docs/a", "../b/./c.md"),
            Some("docs/b/c.md".into())
        );
        assert_eq!(resolve_path("docs", "../../x"), None);
        assert_eq!(
            link_url(&links, "docs", "other.md#x"),
            Some("/t/rendered/docs/other.md#x".into())
        );
        assert_eq!(
            link_url(&links, "docs", "sub"),
            Some("/t/rendered/docs/sub/index.rst".into())
        );
        assert_eq!(
            link_url(&links, "docs", "/dom/base/Element.cpp"),
            Some("/t/source/dom/base/Element.cpp".into())
        );
        assert_eq!(link_url(&links, "docs", "javascript:alert(1)"), None);
        assert_eq!(
            link_url(&links, "docs", "https://example.com/"),
            Some("https://example.com/".into())
        );
        assert_eq!(
            find_doc(&links, "docs", "other"),
            Some("docs/other.md".into())
        );
        assert_eq!(
            searchfox_url(&links, "dom/base/Element.cpp").0,
            "/t/source/dom/base/Element.cpp"
        );
        assert_eq!(
            searchfox_url(&links, "firefox-beta:dom/x.cpp#4").0,
            "/firefox-beta/source/dom/x.cpp#4"
        );
        assert_eq!(
            searchfox_url(&links, "firefox-main/rev/abc:dom/x.cpp").0,
            "/firefox-main/rev/abc/dom/x.cpp"
        );
    }

    #[test]
    fn test_sanitizer() {
        let links = TestLinks;
        let mut out = Writer::new("");
        let mut sanitizer = HtmlSanitizer::default();
        sanitizer.write(
            &mut out,
            r#"<div onclick="x()" class="a"><b>Hi</b> & <script>alert(1)</script><a href="javascript:x">y</a><img src="pic.png" alt="P">"#,
            true,
            &links,
            "docs",
        );
        sanitizer.close_all(&mut out);
        assert_eq!(
            out.html,
            r#"<div><b>Hi</b> &amp; alert(1)<a>y</a><img src="https://raw.example/docs/pic.png" alt="P"></div>"#
        );

        // Closing tags it didn't open are dropped, and inline HTML can't have
        // block elements.
        let mut out = Writer::new("");
        let mut sanitizer = HtmlSanitizer::default();
        sanitizer.write(&mut out, "</div><kbd>A</kbd><div>", false, &links, "");
        assert_eq!(out.html, "<kbd>A</kbd>");
    }

    #[test]
    fn test_block_lines() {
        // Lines: 0 "(target)=" 1 "# Heading" 2 "" 3 "| a |" 4 "|---|"
        // 5 "| 1 |" 6 "" 7 "[ref]: x"
        let line_starts = [0, 10, 20, 21, 27, 33, 39, 40];
        let line_of = |offset: usize| line_starts.partition_point(|&s| s <= offset) - 1;
        let blocks = [
            Block {
                range: 10..20,
                strip_at: 0,
            },
            Block {
                range: 21..27,
                strip_at: 0,
            },
            Block {
                range: 33..39,
                strip_at: 0,
            },
        ];
        let has_tokens = |line: usize| ![2, 6].contains(&line);
        assert_eq!(
            block_lines(&blocks, line_of, 8, has_tokens),
            vec![Some(0..=1), Some(3..=4), Some(5..=7)]
        );
    }
}
