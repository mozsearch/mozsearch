//! The HTML Standard's Wattsi `source`, rendered as Wattsi renders it for
//! html.spec.whatwg.org's multipage edition (see `spec_analyze::wattsi`, which
//! resolves its definitions and cross-references):
//!
//! - content with `w-nohtml` or `w-dev` attributes isn't in that edition, and
//!   neither is content with `w-nosplit` when the source is split into pages;
//! - headings, `<dfn>`s, and cross-references get Wattsi's ids, and
//!   cross-references link to their definitions (or to the other spec's term,
//!   for a `<dfn>` with `data-x-href`), which can be on other pages;
//! - headings (but for `no-num` ones) get their section numbers, and
//!   `<!--toc-->` the table of contents (but for `no-toc` headings);
//! - `<ref>`s are references to the bibliography's entries, and
//!   `<!--INSERT TRACKING-->` notes link to "tracking vector";
//! - headings with `split-filename`s start pages (like the multipage
//!   edition's `{split-filename}.html`), the first page being what's before
//!   the first of them.
//!
//! Not rendered: what html-build generates (ex: the elements' "Tag omission"
//! entries, the indices), syntax highlighting, and inline SVG diagrams, which
//! need the spec's styles (they're described by their labels).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;

use super::{Hooks, HtmlRenderer, SpecLinks, fragment, link_url, page_url};
use crate::rendered_docs::{Rendered, escape};
use crate::spec_analyze::markup::NodeKind;
use crate::spec_analyze::wattsi::{Resolved, resolve};
use crate::spec_analyze::{SpecEntry, SpecsConfig};

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
}

/// A heading's entry in the table of contents.
struct TocEntry {
    level: usize,
    number: Option<String>,
    node: usize,
    text: String,
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h2" | "h3" | "h4" | "h5" | "h6")
}

fn has_class(resolved: &Resolved, node: usize, class: &str) -> bool {
    resolved
        .doc
        .attr(node, "class")
        .is_some_and(|classes| classes.split_whitespace().any(|c| c == class))
}

fn words(resolved: &Resolved, node: usize) -> String {
    resolved
        .doc
        .text_content(node)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

struct Wattsi<'r, 's> {
    resolved: &'r Resolved<'s>,
    hidden: Vec<bool>,
    ids: HashMap<usize, String>,
    /// The page of each id.
    id_pages: HashMap<String, usize>,
    /// Cross-references' links.
    xrefs: HashMap<usize, String>,
    numbers: HashMap<usize, String>,
    toc: Vec<TocEntry>,
    pages: Vec<Page>,
    page: usize,
    /// The URL of the rendered source.
    base: String,
    spec: &'r SpecEntry,
    config: &'r SpecsConfig,
    links: &'r dyn SpecLinks,
}

impl Wattsi<'_, '_> {
    fn page_of(&self, offset: usize) -> usize {
        self.pages
            .partition_point(|page| page.start <= offset)
            .saturating_sub(1)
    }

    /// The URL of a page.
    fn page_href(&self, page: usize) -> String {
        page_url(&self.base, self.pages[page].name.as_deref())
    }

    /// A link to an id of the source, on its page.
    fn local_href(&self, id: &str) -> String {
        match self.id_pages.get(id) {
            Some(&page) if page != self.page => {
                format!("{}{}", self.page_href(page), fragment(id))
            }
            _ => fragment(id),
        }
    }

    fn toc(&self, out: &mut HtmlRenderer) {
        let mut depth = 0;
        out.w.push(r#"<nav class="sp-toc-nav">"#);
        for entry in &self.toc {
            let level = entry.level;
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
            let href = self
                .ids
                .get(&entry.node)
                .map_or_else(String::new, |id| self.local_href(id));
            write!(out.w.html, r#"<a href="{}">"#, escape(&href)).unwrap();
            if let Some(number) = &entry.number {
                write!(out.w.html, r#"<span class="sp-secno">{}</span> "#, number).unwrap();
            }
            out.w.escaped(&entry.text);
            out.w.push("</a>");
        }
        for _ in 0..depth {
            out.w.push("</li></ol>");
        }
        out.w.push("</nav>");
    }
}

impl Hooks for Wattsi<'_, '_> {
    fn hidden(&self, node: usize) -> bool {
        self.hidden[node]
    }

    fn id(&self, node: usize) -> Option<&str> {
        self.ids.get(&node).map(String::as_str)
    }

    fn xref(&self, node: usize) -> Option<&str> {
        self.xrefs.get(&node).map(String::as_str)
    }

    fn href(&self, href: &str) -> Option<String> {
        if let Some(id) = href.strip_prefix('#') {
            return Some(self.local_href(&crate::spec_analyze::markup::decode_html(id)));
        }
        link_url(href, self.spec.rendered.as_deref(), self.config, self.links)
    }

    fn src(&self, src: &str) -> Option<String> {
        // (Images are the rendered spec's, ex: `/images/x.png`.)
        let base = url::Url::parse(self.spec.rendered.as_deref()?).ok()?;
        let url = base.join(src).ok()?;
        matches!(url.scheme(), "https" | "http").then(|| url.to_string())
    }

    fn is_flow(&self, node: usize) -> bool {
        self.resolved.doc.nodes[node].kind == NodeKind::Comment
            && matches!(self.comment(node), "<!--toc-->")
    }

    fn is_inline_content(&self, node: usize) -> bool {
        self.xrefs.contains_key(&node)
    }

    fn special(&self, out: &mut HtmlRenderer, node: usize) -> bool {
        let doc = &self.resolved.doc;
        match &doc.nodes[node].kind {
            NodeKind::Comment => {
                match self.comment(node) {
                    "<!--toc-->" => {
                        let opened = out.w.open_block_div(doc.nodes[node].range.clone(), "");
                        out.w
                            .push(r#"<h2 class="sp-no-num">Table of contents</h2>"#);
                        out.w.close_block_div(opened);
                        self.toc(out);
                    }
                    "<!--INSERT TRACKING-->" => {
                        if let Some(href) = self.xrefs.get(&node) {
                            out.w.push("<a");
                            if let Some(id) = self.ids.get(&node)
                                && out.w.claim_id(id)
                            {
                                write!(out.w.html, r#" id="{}""#, escape(id)).unwrap();
                            }
                            write!(
                                out.w.html,
                                r#" href="{}" class="sp-tracking-vector" title="There is a tracking vector here.">⚠</a>"#,
                                escape(href)
                            )
                            .unwrap();
                        }
                    }
                    _ => {}
                }
                true
            }
            NodeKind::Element(name) if is_heading(name) || name == "h1" => {
                let range = doc.nodes[node].range.clone();
                let opened = out.w.open_block_div(range, "");
                let tag = out.start_tag(self, node, name);
                if let Some(number) = self.numbers.get(&node) {
                    write!(out.w.html, r#"<span class="sp-secno">{}</span> "#, number).unwrap();
                }
                out.children(self, node);
                out.end_tag(tag);
                out.w.close_block_div(opened);
                true
            }
            NodeKind::Element(name) if name == "ref" => {
                let name = doc.text_content(node);
                let href = self.local_href(&format!("refs{}", name));
                out.w.push("[");
                write!(out.w.html, r#"<a href="{}">"#, escape(&href)).unwrap();
                out.in_link += 1;
                out.children(self, node);
                out.in_link -= 1;
                out.w.push("</a>]");
                true
            }
            NodeKind::Element(name) if name == "dfn" => {
                let tag = out.start_tag(self, node, name);
                let external = doc
                    .attr(node, "data-x-href")
                    .and_then(|url| self.href(&url));
                if let Some(url) = &external {
                    write!(out.w.html, r#"<a href="{}">"#, escape(url)).unwrap();
                    out.in_link += 1;
                }
                out.children(self, node);
                if external.is_some() {
                    out.in_link -= 1;
                    out.w.push("</a>");
                }
                out.end_tag(tag);
                true
            }
            NodeKind::Element(name) if name == "svg" => {
                // (An inline diagram, described by its label.)
                let label = doc.attr(node, "aria-label").unwrap_or_default();
                let opened = out.w.open_block_div(doc.nodes[node].range.clone(), "");
                out.w.push(r#"<p class="sp-diagram">Diagram: "#);
                out.w.escaped(&label);
                out.w.push("</p>");
                out.w.close_block_div(opened);
                true
            }
            _ => false,
        }
    }
}

impl Wattsi<'_, '_> {
    fn comment(&self, node: usize) -> &str {
        &self.resolved.doc.src[self.resolved.doc.nodes[node].range.clone()]
    }
}

/// The pages of a resolved source: the first, titled by the source's
/// `<title>`, and one for each heading with a `split-filename`.
fn pages(resolved: &Resolved) -> Vec<Page> {
    let doc = &resolved.doc;
    let title = (1..doc.nodes.len())
        .find(|&node| doc.is(node, "title") && !resolved.excluded[node])
        .map_or_else(|| "Untitled".to_string(), |node| words(resolved, node));
    let mut pages = vec![Page {
        name: None,
        title,
        start: 0,
    }];
    for node in 1..doc.nodes.len() {
        if resolved.excluded[node] || !doc.name(node).is_some_and(is_heading) {
            continue;
        }
        if let Some(name) = doc.attr(node, "split-filename") {
            pages.push(Page {
                name: Some(name),
                title: words(resolved, node),
                start: doc.nodes[node].range.start,
            });
        }
    }
    pages
}

/// The page of a source with an anchor, if it has the anchor.
pub fn anchor_page(src: &str, anchor: &str) -> Option<Page> {
    let resolved = resolve(src);
    let pages = pages(&resolved);
    let doc = &resolved.doc;
    let node = resolved
        .node_ids
        .iter()
        .find(|(_, id)| *id == anchor)
        .map(|(&node, _)| node)
        .or_else(|| {
            resolved
                .xrefs
                .iter()
                .find(|xref| xref.generated_id.as_deref() == Some(anchor))
                .map(|xref| xref.node)
        })
        .or_else(|| {
            (1..doc.nodes.len()).find(|&node| {
                !resolved.excluded[node] && doc.attr(node, "id").as_deref() == Some(anchor)
            })
        })?;
    let start = doc.nodes[node].range.start;
    let i = pages
        .partition_point(|page| page.start <= start)
        .saturating_sub(1);
    Some(pages[i].clone())
}

/// The page of a source with the byte `offset`.
pub fn offset_page(src: &str, offset: usize) -> Option<Page> {
    let pages = pages(&resolve(src));
    let i = pages
        .partition_point(|page| page.start <= offset)
        .saturating_sub(1);
    pages.get(i).cloned()
}

/// Render the page named `page` (or the first) of a Wattsi source, of
/// `spec`, which is at `path` in the tree, among `config`'s specs.
pub fn render(
    src: &str,
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    page: Option<&str>,
) -> Option<RenderedSpec> {
    let resolved = resolve(src);
    let doc = &resolved.doc;
    let pages = pages(&resolved);
    let page = match page {
        Some(name) => pages.iter().position(|p| p.name.as_deref() == Some(name))?,
        None => 0,
    };
    let paged = pages.len() > 1;

    // What's left out: what's not in the edition, and when it's split into
    // pages, what's not in the multipage edition.
    let mut hidden = resolved.excluded.clone();
    if paged {
        for node in 1..doc.nodes.len() {
            let parent = doc.nodes[node].parent.unwrap_or(0);
            hidden[node] |= hidden[parent] || doc.has_attr(node, "w-nosplit");
        }
    }

    let mut ids: HashMap<usize, String> = resolved
        .node_ids
        .iter()
        .map(|(&node, id)| (node, id.clone()))
        .collect();
    for xref in &resolved.xrefs {
        if let Some(id) = &xref.generated_id {
            ids.insert(xref.node, id.clone());
        }
    }
    for node in 1..doc.nodes.len() {
        if !resolved.excluded[node]
            && !ids.contains_key(&node)
            && let Some(id) = doc.attr(node, "id")
        {
            ids.insert(node, id);
        }
    }

    let mut wattsi = Wattsi {
        resolved: &resolved,
        hidden,
        ids: HashMap::new(),
        id_pages: HashMap::new(),
        xrefs: HashMap::new(),
        numbers: HashMap::new(),
        toc: vec![],
        pages: pages.clone(),
        page,
        base: links.rendered_url(path),
        spec,
        config,
        links,
    };
    wattsi.id_pages = ids
        .iter()
        .map(|(&node, id)| (id.clone(), wattsi.page_of(doc.nodes[node].range.start)))
        .collect();
    wattsi.ids = ids;

    // Cross-references link to their targets' ids, or to the other spec's
    // term that a `<dfn>` defers to.
    let mut xrefs = HashMap::new();
    for xref in &resolved.xrefs {
        let external = doc
            .attr(xref.target, "data-x-href")
            .and_then(|url| wattsi.href(&url));
        let href = external.or_else(|| {
            let id = resolved.node_ids.get(&xref.target)?;
            Some(wattsi.local_href(id))
        });
        if let Some(href) = href {
            xrefs.insert(xref.node, href);
        }
    }
    wattsi.xrefs = xrefs;

    // Section numbers, and the table of contents.
    let mut counters: Vec<usize> = vec![];
    for node in 1..doc.nodes.len() {
        let Some(name) = doc.name(node).filter(|name| is_heading(name)) else {
            continue;
        };
        if resolved.excluded[node] {
            continue;
        }
        let level = name[1..].parse::<usize>().unwrap_or(2) - 1;
        let number = if has_class(&resolved, node, "no-num") {
            None
        } else {
            counters.resize(level, 0);
            counters[level - 1] += 1;
            Some(
                counters
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join("."),
            )
        };
        if let Some(number) = &number {
            wattsi.numbers.insert(node, number.clone());
        }
        if !has_class(&resolved, node, "no-toc") {
            wattsi.toc.push(TocEntry {
                level,
                number,
                node,
                text: words(&resolved, node),
            });
        }
    }

    let range = pages[page].start..pages.get(page + 1).map_or(src.len(), |next| next.start);
    let mut out = HtmlRenderer::new(doc, range.clone());
    out.children(&wattsi, 0);
    let mut rendered = out.w.finish();
    rendered.title = Some(match page {
        0 => pages[0].title.clone(),
        _ => format!("{} - {}", pages[page].title, pages[0].title),
    });
    Some(RenderedSpec {
        rendered,
        pages,
        page,
        range,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestLinks;

    impl SpecLinks for TestLinks {
        fn rendered_url(&self, path: &str) -> String {
            format!("/t/rendered/{}", path)
        }
    }

    fn config() -> SpecsConfig {
        toml::from_str(
            r#"
            [[spec]]
            name = "example"
            format = "wattsi"
            rendered = "https://example.spec.whatwg.org/"
            [[spec]]
            name = "dom"
            rendered = "https://dom.spec.whatwg.org/"
            [[spec]]
            name = "other"
            path = "other"
            format = "wattsi"
            rendered = "https://other.spec.whatwg.org/"
            "#,
        )
        .unwrap()
    }

    /// The HTML, with the strips' places as `[N]`.
    fn marked(rendered: &Rendered) -> String {
        let mut out = String::new();
        let mut pos = 0;
        for (i, block) in rendered.blocks.iter().enumerate() {
            out.push_str(&rendered.html[pos..block.strip_at]);
            write!(out, "[{}]", i).unwrap();
            pos = block.strip_at;
        }
        out.push_str(&rendered.html[pos..]);
        out
    }

    fn render_page(src: &str, page: Option<&str>) -> RenderedSpec {
        let config = config();
        render(src, "source", &config.specs[0], &config, &TestLinks, page).unwrap()
    }

    #[test]
    fn test_render() {
        let src = r##"<!DOCTYPE html>
<html><head><title>Example</title><script>x()</script></head>
<body>
<h2>Widgets</h2>
<p>A <dfn>widget</dfn> is a <span>widget</span>, see <code><span>widget</span></code>.
<p w-dev>Developers only.
<div class="note" onclick="x()">A <span data-x="">note</span> &amp; <ref>DOM</ref>
<p>More.</div>
<ol><li>One<li><p>Two<ol><li>Three</ol></ol>
<table><tr><td>A<td><span>widget</span></tr></table>
<p>A <dfn data-x-href="https://dom.spec.whatwg.org/#concept-node">node</dfn> and a <span>node</span>,
<a href="https://other.spec.whatwg.org/#x">x</a>, <a href="javascript:x()">no</a>.
<dl><dt id="refsDOM">[DOM]<dd>DOM.</dl>
</body></html>
"##;
        let rendered = render_page(src, None);
        assert_eq!(
            marked(&rendered.rendered),
            [
                concat!(r##"<div class="rd-block">[0]<h2 id="widgets"><span class="sp-secno">1</span> <span data-l="4" data-c="4">Widgets</span></h2></div><div class="rd-block">[1]<p><span data-l="5" data-c="3">A </span><dfn id="widget"><span data-l="5" data-c="10">widget</span></dfn><span data-l="5" data-c="22"> is a </span><a id="widgets:widget" href="#widget"><span data-l="5" data-c="34">widget</span></a><span data-l="5" data-c="47">, see </span><code id="widgets:widget-2"><a href="#widget"><span><span data-l="5" data-c="65">widget</span></span></a></code><span data-l="5" data-c="85">.</span>"##, "\n"),
                concat!(r##"</p></div><div class="sp-note"><div class="rd-block">[2]<span data-l="7" data-c="32">A </span><span><span data-l="7" data-c="50">note</span></span><span data-l="7" data-c="61"> </span><span data-l="7" data-c="62" data-n="5">&amp;</span><span data-l="7" data-c="67"> </span>[<a href="#refsDOM"><span data-l="7" data-c="73">DOM</span></a>]</div><div class="rd-block">[3]<p><span data-l="8" data-c="3">More.</span></p></div></div><ol><li><div class="rd-block">[4]<span data-l="9" data-c="8">One</span></div></li><li><div class="rd-block">[5]<p><span data-l="9" data-c="18">Two</span></p></div><ol><li><div class="rd-block">[6]<span data-l="9" data-c="29">Three</span></div></li></ol></li></ol><table><tr><td>[7]<span data-l="10" data-c="15">A</span></td><td><a id="widgets:widget-3" href="#widget"><span data-l="10" data-c="26">widget</span></a></td></tr></table><div class="rd-block">[8]<p><span data-l="11" data-c="3">A </span><dfn id="node"><a href="https://dom.spec.whatwg.org/#concept-node"><span data-l="11" data-c="66">node</span></a></dfn><span data-l="11" data-c="76"> and a </span><a id="widgets:node" href="https://dom.spec.whatwg.org/#concept-node"><span data-l="11" data-c="89">node</span></a><span data-l="11" data-c="100">,</span>"##, "\n"),
                concat!(r##"<a href="/t/rendered/other/source/anchor/x"><span data-l="12" data-c="43">x</span></a><span data-l="12" data-c="48">, </span><a><span data-l="12" data-c="75">no</span></a><span data-l="12" data-c="81">.</span>"##, "\n"),
                r##"</p></div><dl><div class="rd-block">[9]<dt id="refsDOM"><span data-l="13" data-c="21">[DOM]</span></dt></div><dd><div class="rd-block">[10]<span data-l="13" data-c="30">DOM.</span></div></dd></dl>"##,
            ]
            .concat()
        );
        assert_eq!(rendered.rendered.title.as_deref(), Some("Example"));
        assert_eq!(rendered.pages.len(), 1);
    }

    #[test]
    fn test_pages() {
        let src = r##"<title>Example</title>
<p>Front matter.
<h2 class="no-num no-toc" w-nosplit>Contents</h2>
<!--toc-->
<h2 split-filename=widgets>Widgets</h2>
<p>A <dfn>widget</dfn>.
<h3>Kinds</h3>
<h3 class=no-num>Unnumbered</h3>
<h2 split-filename=gadgets>Gadgets</h2>
<p>A <span>widget</span>.
"##;
        let first = render_page(src, None);
        assert_eq!(
            first.pages,
            vec![
                Page {
                    name: None,
                    title: "Example".to_string(),
                    start: 0
                },
                Page {
                    name: Some("widgets".to_string()),
                    title: "Widgets".to_string(),
                    start: 101
                },
                Page {
                    name: Some("gadgets".to_string()),
                    title: "Gadgets".to_string(),
                    start: 213
                },
            ]
        );
        assert_eq!(first.range, 0..101);
        let html = marked(&first.rendered);
        assert!(
            html.contains(
                r#"<nav class="sp-toc-nav"><ol class="sp-toc"><li><a href="/t/rendered/source/widgets.html#widgets"><span class="sp-secno">1</span> Widgets</a><ol class="sp-toc"><li><a href="/t/rendered/source/widgets.html#kinds"><span class="sp-secno">1.1</span> Kinds</a></li><li><a href="/t/rendered/source/widgets.html#unnumbered">Unnumbered</a></li></ol></li><li><a href="/t/rendered/source/gadgets.html#gadgets"><span class="sp-secno">2</span> Gadgets</a></li></ol></nav>"#
            ),
            "{}",
            html
        );
        // (The `w-nosplit` heading isn't in the multipage edition.)
        assert!(!html.contains("Contents"), "{}", html);
        assert!(!html.contains("Widgets</h2>"), "{}", html);

        let gadgets = render_page(src, Some("gadgets"));
        assert_eq!(gadgets.range, 213..src.len());
        let html = marked(&gadgets.rendered);
        assert!(
            html.contains(
                r#"<a id="gadgets:widget" href="/t/rendered/source/widgets.html#widget">"#
            ),
            "{}",
            html
        );
        assert_eq!(gadgets.rendered.title.as_deref(), Some("Gadgets - Example"));
        let config = config();
        assert!(
            render(
                src,
                "source",
                &config.specs[0],
                &config,
                &TestLinks,
                Some("nope")
            )
            .is_none()
        );
        assert_eq!(
            anchor_page(src, "widget").and_then(|page| page.name),
            Some("widgets".to_string())
        );
        assert_eq!(
            anchor_page(src, "gadgets:widget").and_then(|page| page.name),
            Some("gadgets".to_string())
        );
        assert_eq!(anchor_page(src, "nope"), None);
    }
}
