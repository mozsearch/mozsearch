//! Rendered views of web specs' sources (`/{tree}/rendered/{path}`, see
//! `format::format_rendered`), like documents' (see `rendered_docs`): their
//! blocks (paragraphs, headings, algorithm steps, table rows, ...) each get a
//! blame strip, and each run of text that's in the source verbatim says where
//! it is, for the blame popup.
//!
//! HTML-based sources (the HTML Standard's Wattsi `source`, see `wattsi`) are
//! rendered from our own parse (see `spec_analyze::markup`), with the links
//! our analysis resolves (see `spec_analyze`), rather than from their build
//! tools' output, which doesn't say where anything came from.  Only elements
//! and attributes which can't do anything but present the spec are kept
//! (classes are prefixed with `sp-`, so that they can't be the page's), and
//! links to the tree's other specs go to their rendered views.

pub mod bikeshed;
pub mod ecmarkup;
pub mod wattsi;

use std::fmt::Write as _;
use std::ops::Range;

use crate::rendered_docs::{Rendered, Writer, escape};
use crate::spec_analyze::markup::{Doc, NodeKind, text_pieces};
use crate::spec_analyze::{SpecEntry, SpecFormat, SpecsConfig};

/// How a rendered spec links to the rendered views of the tree's specs.
pub trait SpecLinks {
    /// The URL of the rendered view of the source at `path` (without a
    /// page; see `page_url`).
    fn rendered_url(&self, path: &str) -> String;
    /// The URL of the view of the algorithm `sym` defines beside the code
    /// citing its steps (see `format::format_algorithm`), if there's one.
    fn algorithm_url(&self, _sym: &str) -> Option<String> {
        None
    }
}

/// A link to the view of an algorithm beside the code citing it.
pub fn algorithm_link(out: &mut HtmlRenderer, url: &str) {
    write!(
        out.w.html,
        r#"<a class="sp-algorithm-link" href="{}" title="Show the algorithm beside the code citing its steps">⇄</a>"#,
        escape(url)
    )
    .unwrap();
}

/// Whether specs of a format have rendered views.
pub fn has_rendered_view(format: SpecFormat) -> bool {
    matches!(
        format,
        SpecFormat::Wattsi | SpecFormat::Ecmarkup | SpecFormat::Bikeshed
    )
}

/// Whether specs of a format can be split into pages (ex: the HTML Standard,
/// like its multipage edition), so that links to their anchors go through
/// `anchor_url`, which finds the anchor's page.
pub fn is_paged(format: SpecFormat) -> bool {
    matches!(format, SpecFormat::Wattsi | SpecFormat::Ecmarkup)
}

/// Render the page named `page` (or the first) of the source `src` of
/// `spec`, at `path` in the tree, among `config`'s specs, reading other files
/// of the tree (ex: ecmarkup's imports) with `read`, if it has the page.
pub fn render(
    spec: &SpecEntry,
    src: &str,
    path: &str,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    page: Option<&str>,
) -> Option<RenderedSpec> {
    match spec.format? {
        SpecFormat::Wattsi => wattsi::render(src, path, spec, config, links, page),
        SpecFormat::Ecmarkup => ecmarkup::render(path, spec, config, links, read, page),
        SpecFormat::Bikeshed | SpecFormat::Respec => None,
    }
}

/// Render the algorithm which defines `sym` in the source `src` of `spec`
/// (see `render`), beside the code citing its steps (see
/// `format::format_algorithm`), given Bikeshed's `output`, for a Bikeshed
/// spec.
#[allow(clippy::too_many_arguments)]
pub fn render_algorithm(
    spec: &SpecEntry,
    src: &str,
    path: &str,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    output: Option<&str>,
    sym: &str,
) -> Option<RenderedSpec> {
    match spec.format? {
        SpecFormat::Wattsi => wattsi::render_algorithm(src, path, spec, config, links, sym),
        SpecFormat::Ecmarkup => ecmarkup::render_algorithm(path, spec, config, links, read, sym),
        SpecFormat::Bikeshed => {
            bikeshed::render_algorithm(output?, src, path, spec, config, links, sym)
        }
        SpecFormat::Respec => None,
    }
}

/// The page of a spec's source with an anchor, if it has the anchor.
pub fn anchor_page(
    spec: &SpecEntry,
    src: &str,
    path: &str,
    config: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
    anchor: &str,
) -> Option<Page> {
    match spec.format? {
        SpecFormat::Wattsi => wattsi::anchor_page(src, anchor),
        SpecFormat::Ecmarkup => ecmarkup::anchor_page(path, spec, config, read, anchor),
        SpecFormat::Bikeshed | SpecFormat::Respec => None,
    }
}

/// The page of a spec's source with its byte `offset`.
pub fn offset_page(
    spec: &SpecEntry,
    src: &str,
    path: &str,
    config: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
    offset: usize,
) -> Option<Page> {
    match spec.format? {
        SpecFormat::Wattsi => wattsi::offset_page(src, offset),
        SpecFormat::Ecmarkup => ecmarkup::offset_page(path, spec, config, read, offset),
        SpecFormat::Bikeshed | SpecFormat::Respec => None,
    }
}

/// The URL of a page (by its name, or the first, `None`) of the rendered view
/// at `base` (see `SpecLinks::rendered_url`): `{base}/{page}.html`, like the
/// multipage HTML Standard's.
pub fn page_url(base: &str, page: Option<&str>) -> String {
    match page {
        Some(page) => format!("{}/{}.html", base, urlencoding::encode(page)),
        None => base.to_string(),
    }
}

/// The URL of an anchor of the paged rendered view at `base`, which redirects
/// to its page (see `format::format_rendered`).
pub fn anchor_url(base: &str, anchor: &str) -> String {
    format!("{}/anchor/{}", base, urlencoding::encode(anchor))
}

/// `#` and an anchor, escaped for a URL's fragment.
pub fn fragment(anchor: &str) -> String {
    let mut out = String::from("#");
    for c in anchor.chars() {
        match c {
            ' ' | '"' | '<' | '>' | '`' | '#' | '%' | '\n' | '\t' => {
                let mut buf = [0; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    write!(out, "%{:02X}", b).unwrap();
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// The URL that a link to `url` in a spec (rendered at `rendered`, if
/// known) goes to: the rendered view of one of the tree's specs, if it's to
/// one which has one, or else the URL as an absolute one with a safe scheme.
pub fn link_url(
    url: &str,
    rendered: Option<&str>,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
) -> Option<String> {
    let absolute = match rendered.and_then(|base| url::Url::parse(base).ok()) {
        Some(base) => base.join(url).ok()?,
        None => url::Url::parse(url).ok()?,
    };
    if !matches!(absolute.scheme(), "http" | "https" | "mailto") {
        return None;
    }
    if let Some((spec, anchor)) = config.url_spec(absolute.as_str())
        && let Some(format) = spec.format
        && has_rendered_view(format)
        && let Some(path) = spec.source_path()
    {
        let base = links.rendered_url(&path);
        return Some(if is_paged(format) {
            anchor_url(&base, &anchor)
        } else {
            format!("{}{}", base, fragment(&anchor))
        });
    }
    Some(absolute.to_string())
}

/// A page of a rendered source.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Its name, as in `page_url` (none for the first page).
    pub name: Option<String>,
    pub title: String,
    /// The byte of the source where it starts.
    pub start: usize,
}

/// A page of a rendered spec source.
pub struct RenderedSpec {
    pub rendered: Rendered,
    /// The pages of the source (one, if it's not split).
    pub pages: Vec<Page>,
    /// The index of the page rendered.
    pub page: usize,
    /// The bytes of the source that the page covers.
    pub range: Range<usize>,
    /// For an algorithm, its steps' numbers (ex: "12.6", or ecmarkup's
    /// "3.a"), and where they start in the file.
    pub steps: Vec<(String, usize)>,
    /// The file whose bytes the blocks and `range` are, if it's not the
    /// source (ex: for an ecmarkup algorithm, the file it imports with it).
    pub file: Option<String>,
}

impl Page {
    /// The index of the page with the byte `offset` of the source.
    pub fn index_of(pages: &[Page], offset: usize) -> usize {
        pages
            .partition_point(|page| page.start <= offset)
            .saturating_sub(1)
    }
}

/// A table of contents, of its entries' (1-based) levels, numbers, links,
/// and titles.
pub fn write_toc(out: &mut HtmlRenderer, entries: &[(usize, Option<String>, String, String)]) {
    let mut depth = 0;
    out.w.push(r#"<nav class="sp-toc-nav">"#);
    for (level, number, href, text) in entries {
        let level = *level;
        if level > depth {
            for _ in depth..level {
                out.w.push(r#"<ol class="sp-toc">"#);
                out.w.push("<li>");
            }
        } else {
            for _ in level..depth {
                out.w.push("</li></ol>");
            }
            out.w.push("</li><li>");
        }
        depth = level;
        write!(out.w.html, r#"<a href="{}">"#, escape(href)).unwrap();
        if let Some(number) = number {
            write!(out.w.html, r#"<span class="sp-secno">{}</span> "#, number).unwrap();
        }
        out.w.escaped(text);
        out.w.push("</a>");
    }
    for _ in 0..depth {
        out.w.push("</li></ol>");
    }
    out.w.push("</nav>");
}

/// What an element is, for rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    /// Left out, with its contents (ex: scripts, form controls).
    Omit,
    /// Just its contents (ex: `<body>`).
    Transparent,
    /// Contains blocks (ex: lists, `<div>`s), and its own runs of inline
    /// content are blocks.
    Container,
    /// A block (ex: a paragraph or heading).
    Leaf,
    /// A block whose strip goes inside it (ex: a `<summary>`, which a
    /// `<details>` needs as its first child).
    InnerLeaf,
    /// A table row, a block whose strip is in its first cell.
    Row,
    /// Inline content.
    Inline,
}

pub fn role(name: &str) -> Role {
    match name {
        "head" | "script" | "style" | "template" | "title" | "meta" | "link" | "base"
        | "noscript" | "iframe" | "object" | "embed" | "input" | "select" | "textarea"
        | "button" | "form" | "audio" | "video" | "canvas" | "math" | "svg" | "option"
        | "optgroup" | "datalist" | "output" | "progress" | "meter" | "slot" | "frame"
        | "frameset" | "param" | "source" | "track" | "area" | "map" | "picture" => Role::Omit,
        "html" | "body" => Role::Transparent,
        // (ecmarkup's; see `ecmarkup`.)
        "emu-biblio" | "emu-import" => Role::Omit,
        "emu-clause" | "emu-annex" | "emu-intro" | "emu-note" | "emu-example" | "emu-table"
        | "emu-figure" => Role::Container,
        "emu-alg" | "emu-grammar" | "emu-production" => Role::Leaf,
        "div" | "section" | "article" | "aside" | "nav" | "header" | "footer" | "main" | "ol"
        | "ul" | "dl" | "li" | "dd" | "blockquote" | "figure" | "details" | "table" | "thead"
        | "tbody" | "tfoot" | "colgroup" | "fieldset" | "hgroup" | "center" | "menu" | "search"
        | "address" => Role::Container,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "dt" | "pre" | "figcaption" | "hr" => {
            Role::Leaf
        }
        "summary" | "caption" | "legend" => Role::InnerLeaf,
        "tr" => Role::Row,
        _ => Role::Inline,
    }
}

/// The tag an element is written as, if it's written as an element: as it
/// is, if it can only present, or as a `<div>` or `<span>` (ex: a
/// `<header>`, which the page's own styles are for).
pub fn output_tag(name: &str) -> Option<&'static str> {
    const AS_IS: &[&str] = &[
        "a",
        "abbr",
        "b",
        "bdi",
        "bdo",
        "blockquote",
        "br",
        "caption",
        "cite",
        "code",
        "col",
        "colgroup",
        "data",
        "dd",
        "del",
        "details",
        "dfn",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "ins",
        "kbd",
        "li",
        "mark",
        "ol",
        "p",
        "pre",
        "q",
        "rp",
        "rt",
        "ruby",
        "s",
        "samp",
        "small",
        "span",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "time",
        "tr",
        "u",
        "ul",
        "var",
        "wbr",
    ];
    if let Some(tag) = AS_IS.iter().find(|tag| **tag == name) {
        return Some(tag);
    }
    match role(name) {
        Role::Container | Role::Leaf | Role::InnerLeaf => Some("div"),
        Role::Inline if name == "label" || name.starts_with("emu-") => Some("span"),
        _ => None,
    }
}

/// Elements without contents.
pub fn is_void(name: &str) -> bool {
    matches!(name, "br" | "wbr" | "hr" | "img" | "col")
}

/// The page's own elements' ids, which a spec's can't have.
const PAGE_IDS: &[&str] = &[
    "content",
    "data",
    "file",
    "fixed-header",
    "foot",
    "info-boxes-container",
    "open-help",
    "panel",
    "panel-content",
    "panel-toggle",
    "path",
    "query",
    "regexp",
    "rendered-doc",
    "revision",
    "scrolling",
    "search-box",
    "spinner",
    "tree-switcher",
];

/// How a format's renderer renders its elements: what's left out, ids, and
/// links.
pub trait Hooks {
    /// Whether a node is left out.
    fn hidden(&self, node: usize) -> bool;
    /// The id an element gets.
    fn id(&self, node: usize) -> Option<&str>;
    /// The URL an element links to, if it's a cross-reference.
    fn xref(&self, node: usize) -> Option<&str>;
    /// The URL a link's `href` goes to, if it's safe.
    fn href(&self, href: &str) -> Option<String>;
    /// The URL an image's `src` loads, if it's safe.
    fn src(&self, src: &str) -> Option<String>;
    /// Render a node specially (ex: a heading with its number, or a comment
    /// for the table of contents), returning whether it did.
    fn special(&self, out: &mut HtmlRenderer, node: usize) -> bool;
    /// Whether a node is a block (or blocks), if not by its element's role
    /// (ex: a comment for the table of contents).
    fn is_flow(&self, _node: usize) -> bool {
        false
    }
    /// Whether a node which isn't an element or text renders as inline
    /// content (ex: a comment for a tracking vector).
    fn is_inline_content(&self, _node: usize) -> bool {
        false
    }
    /// The bytes of the file that a node's block is of (or, for a run of
    /// inline content, its parent's), if not where the source's range is in
    /// it (ex: Bikeshed's output's, by its elements' `bs-pos`): `Some(None)`
    /// if it has no block.
    fn block_range(&self, _node: usize) -> Option<Option<Range<usize>>> {
        None
    }
}

/// Where ranges of a source are in the file that a rendering is for, if
/// they're in it (ex: of an ecmarkup spec's source with its imports, in its
/// main file).
pub type FileMap<'d> = &'d dyn Fn(&Range<usize>) -> Option<Range<usize>>;

/// Renders an HTML-based spec source's tree (or the part of it in `range`,
/// ex: a page's).
pub struct HtmlRenderer<'s, 'd> {
    pub w: Writer<'s>,
    pub doc: &'d Doc<'d>,
    pub range: Range<usize>,
    /// How deep we are in links, whose cross-references don't link.
    pub in_link: usize,
    /// Where the source's ranges are in the file the rendering is for, if
    /// it's not the source (only blocks and text in it say where they are).
    map: Option<FileMap<'d>>,
}

impl<'s, 'd> HtmlRenderer<'s, 'd> {
    pub fn new(doc: &'d Doc<'s>, range: Range<usize>) -> HtmlRenderer<'s, 'd> {
        HtmlRenderer {
            w: Writer::new(doc.src),
            doc,
            range,
            in_link: 0,
            map: None,
        }
    }

    /// A renderer for the file `file`, where `map` says the source's ranges
    /// are.
    pub fn for_file(
        doc: &'d Doc<'d>,
        range: Range<usize>,
        file: &'s str,
        map: FileMap<'d>,
    ) -> HtmlRenderer<'s, 'd> {
        HtmlRenderer {
            w: Writer::new(file),
            doc,
            range,
            in_link: 0,
            map: Some(map),
        }
    }

    /// Where a range of the source is in the file, if it's in it.
    pub fn file_range(&self, range: &Range<usize>) -> Option<Range<usize>> {
        match self.map {
            Some(map) => map(range),
            None => Some(range.clone()),
        }
    }

    /// `<div class="rd-block ...">` and the strip, for the range of the
    /// source, if a block can start here and the range is in the file.
    pub fn open_block(&mut self, range: Range<usize>, class: &str) -> bool {
        match self.file_range(&range) {
            Some(range) => self.w.open_block_div(range, class),
            None => false,
        }
    }

    /// `open_block`, for a node's block (see `Hooks::block_range`).
    pub fn open_node_block(
        &mut self,
        hooks: &dyn Hooks,
        node: usize,
        range: Range<usize>,
        class: &str,
    ) -> bool {
        match hooks.block_range(node) {
            Some(Some(range)) => self.w.open_block_div(range, class),
            Some(None) => false,
            None => self.open_block(range, class),
        }
    }

    /// Start a block whose strip goes wherever `w.place_strip` is next
    /// called (ex: a table row's), if one can start here and the range is in
    /// the file.
    pub fn start_block(&mut self, range: Range<usize>) -> bool {
        match self.file_range(&range) {
            Some(range) if !self.w.in_block() => self.w.start_block(range, false),
            _ => false,
        }
    }

    fn element_role(&self, node: usize) -> Option<Role> {
        self.doc.name(node).map(role)
    }

    /// Whether a node is a block, or contains blocks.
    fn is_flow(&self, hooks: &dyn Hooks, node: usize) -> bool {
        hooks.is_flow(node)
            || matches!(
                self.element_role(node),
                Some(
                    Role::Transparent | Role::Container | Role::Leaf | Role::InnerLeaf | Role::Row
                )
            )
    }

    /// Whether a node renders as inline content that isn't just whitespace.
    fn is_inline_content(&self, hooks: &dyn Hooks, node: usize) -> bool {
        if hooks.hidden(node) {
            return false;
        }
        match &self.doc.nodes[node].kind {
            NodeKind::Text => !self.doc.src[self.doc.nodes[node].range.clone()]
                .trim()
                .is_empty(),
            NodeKind::Element(name) => role(name) == Role::Inline,
            _ => hooks.is_inline_content(node),
        }
    }

    /// Whether a node is to be rendered, given the range: it starts in it, or
    /// it contains blocks and overlaps it (ex: the `<body>` of a page).
    fn in_range(&self, hooks: &dyn Hooks, node: usize) -> bool {
        let range = &self.doc.nodes[node].range;
        if self.range.contains(&range.start) || (range.is_empty() && range.start == self.range.end)
        {
            return true;
        }
        range.start < self.range.end
            && range.end > self.range.start
            && matches!(
                self.element_role(node),
                Some(Role::Transparent | Role::Container)
            )
            && !hooks.hidden(node)
    }

    /// Render a node's children: blocks, and runs of inline content, which
    /// are blocks of their own, unless they're in a block.
    pub fn children(&mut self, hooks: &dyn Hooks, node: usize) {
        let kids = self.doc.nodes[node].children.clone();
        let mut i = 0;
        while i < kids.len() {
            let kid = kids[i];
            if !self.in_range(hooks, kid) || hooks.hidden(kid) {
                i += 1;
                continue;
            }
            if self.w.in_block() || self.is_flow(hooks, kid) {
                self.node(hooks, kid);
                i += 1;
                continue;
            }
            // A run of inline content, up to the next block.
            let mut j = i;
            let (mut first, mut last) = (None, None);
            while j < kids.len()
                && (!self.in_range(hooks, kids[j]) || !self.is_flow(hooks, kids[j]))
            {
                if self.in_range(hooks, kids[j]) && self.is_inline_content(hooks, kids[j]) {
                    first.get_or_insert(j);
                    last = Some(j);
                }
                j += 1;
            }
            if let (Some(first), Some(last)) = (first, last) {
                let range =
                    self.doc.nodes[kids[first]].range.start..self.doc.nodes[kids[last]].range.end;
                let opened = self.open_node_block(hooks, node, range, "");
                for &kid in &kids[first..=last] {
                    if self.in_range(hooks, kid) && !hooks.hidden(kid) {
                        self.node(hooks, kid);
                    }
                }
                self.w.close_block_div(opened);
            }
            i = j;
        }
    }

    pub fn node(&mut self, hooks: &dyn Hooks, node: usize) {
        if hooks.hidden(node) || hooks.special(self, node) {
            return;
        }
        match &self.doc.nodes[node].kind {
            NodeKind::Text => {
                let range = self.doc.nodes[node].range.clone();
                self.text(range);
            }
            NodeKind::Element(_) => self.element(hooks, node),
            NodeKind::Root => self.children(hooks, node),
            NodeKind::Comment => {}
        }
    }

    /// Text from the source, as runs which say where they are (if it's in a
    /// block), split at character references.
    pub fn text(&mut self, range: Range<usize>) {
        if !self.w.in_block() {
            let text = crate::spec_analyze::markup::decode_html(&self.doc.src[range]);
            self.w.escaped(&text);
            return;
        }
        for (piece, text) in text_pieces(self.doc.src, range) {
            let real = self.file_range(&piece);
            self.w.text(&text, real);
        }
    }

    /// An element, as its role says.
    pub fn element(&mut self, hooks: &dyn Hooks, node: usize) {
        let name = self.doc.name(node).unwrap_or("").to_string();
        let range = self.doc.nodes[node].range.clone();
        match role(&name) {
            Role::Omit => {}
            Role::Transparent => self.children(hooks, node),
            Role::Container => {
                let tag = self.start_tag(hooks, node, &name);
                self.children(hooks, node);
                self.end_tag(tag);
            }
            Role::Leaf => {
                let opened = self.open_node_block(hooks, node, range, "");
                let tag = self.start_tag(hooks, node, &name);
                self.children(hooks, node);
                self.end_tag(tag);
                self.w.close_block_div(opened);
            }
            Role::InnerLeaf => {
                let tag = self.start_tag(hooks, node, &name);
                let opened = self.open_node_block(hooks, node, range, "");
                self.children(hooks, node);
                self.w.close_block_div(opened);
                self.end_tag(tag);
            }
            Role::Row => {
                let tag = self.start_tag(hooks, node, &name);
                let opened = match hooks.block_range(node) {
                    Some(Some(range)) if !self.w.in_block() => self.w.start_block(range, false),
                    Some(_) => false,
                    None => self.start_block(range),
                };
                for kid in self.doc.nodes[node].children.clone() {
                    if hooks.hidden(kid) {
                        continue;
                    }
                    match self.doc.name(kid) {
                        Some(cell @ ("td" | "th")) => {
                            let cell = cell.to_string();
                            let tag = self.start_tag(hooks, kid, &cell);
                            self.w.place_strip();
                            self.children(hooks, kid);
                            self.end_tag(tag);
                        }
                        _ => self.node(hooks, kid),
                    }
                }
                if opened {
                    self.w.end_block();
                }
                self.end_tag(tag);
            }
            Role::Inline => {
                if let Some(href) = hooks.xref(node).filter(|_| self.in_link == 0) {
                    let href = href.to_string();
                    self.xref(hooks, node, &name, &href);
                    return;
                }
                let is_link = name == "a";
                let tag = self.start_tag(hooks, node, &name);
                if is_link {
                    self.in_link += 1;
                }
                if !is_void(&name) {
                    self.children(hooks, node);
                }
                if is_link {
                    self.in_link -= 1;
                }
                self.end_tag(tag);
            }
        }
    }

    /// A cross-reference which links to `href`: a `<span>` becomes the link,
    /// and other elements' contents are in it.
    fn xref(&mut self, hooks: &dyn Hooks, node: usize, name: &str, href: &str) {
        let outer = if name == "span" {
            None
        } else {
            self.start_tag(hooks, node, name)
        };
        self.w.push("<a");
        if outer.is_none() {
            self.write_id(hooks, node);
        }
        write!(self.w.html, r#" href="{}">"#, escape(href)).unwrap();
        self.in_link += 1;
        self.children(hooks, node);
        self.in_link -= 1;
        self.w.push("</a>");
        self.end_tag(outer);
    }

    fn write_id(&mut self, hooks: &dyn Hooks, node: usize) {
        if let Some(id) = hooks.id(node)
            && !PAGE_IDS.contains(&id)
            && self.w.claim_id(id)
        {
            write!(self.w.html, r#" id="{}""#, escape(id)).unwrap();
        }
    }

    /// The element's start tag, as `output_tag` says, with the attributes
    /// which can only present (and its id, classes prefixed, and safe
    /// URLs), returning the tag to end it with.
    pub fn start_tag(
        &mut self,
        hooks: &dyn Hooks,
        node: usize,
        name: &str,
    ) -> Option<&'static str> {
        let tag = output_tag(name)?;
        write!(self.w.html, "<{}", tag).unwrap();
        self.write_id(hooks, node);
        let mut classes: Vec<String> = vec![];
        if tag != name {
            classes.push(format!("sp-{}", name));
        }
        for (attr, value) in self.doc.attrs(node) {
            let attr = attr.to_ascii_lowercase();
            let value = value.unwrap_or_default();
            let allowed = match attr.as_str() {
                "class" => {
                    classes.extend(value.split_whitespace().map(|c| format!("sp-{}", c)));
                    false
                }
                "title" | "lang" | "dir" | "colspan" | "rowspan" | "abbr" => true,
                "start" | "reversed" => tag == "ol",
                "value" => tag == "li" || tag == "data",
                "type" => tag == "ol" || tag == "li",
                "open" => tag == "details",
                "datetime" => tag == "time" || tag == "del" || tag == "ins",
                "alt" | "width" | "height" => tag == "img",
                "href" if tag == "a" => {
                    if let Some(url) = hooks.href(&value) {
                        write!(self.w.html, r#" href="{}""#, escape(&url)).unwrap();
                    }
                    false
                }
                "src" if tag == "img" => {
                    if let Some(url) = hooks.src(&value) {
                        write!(self.w.html, r#" src="{}" loading="lazy""#, escape(&url)).unwrap();
                    }
                    false
                }
                _ => false,
            };
            if allowed {
                write!(self.w.html, r#" {}="{}""#, attr, escape(&value)).unwrap();
            }
        }
        if !classes.is_empty() {
            write!(self.w.html, r#" class="{}""#, escape(&classes.join(" "))).unwrap();
        }
        self.w.push(">");
        Some(tag)
    }

    pub fn end_tag(&mut self, tag: Option<&'static str>) {
        if let Some(tag) = tag.filter(|tag| !is_void(tag)) {
            write!(self.w.html, "</{}>", tag).unwrap();
        }
    }
}
