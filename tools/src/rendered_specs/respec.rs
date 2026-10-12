//! ReSpec specs (see `spec_analyze::respec`), rendered from their sources
//! (with the files they include) as ReSpec renders them in the browser, but
//! offline, with the analysis's `Rendering`:
//!
//! - the ids ReSpec gives definitions, sections and headings, sections'
//!   numbers, and the headings it generates (ex: the conformance section's);
//! - shorthands as ReSpec shows them, linked to what they're to: `[=term=]`,
//!   `{{Name/member}}` (as code), `[[SPEC]]` (to the references), and `[^el^]`,
//!   and backticks' code and `|variable|`s;
//! - links without `href`s to what they're to, and IDL blocks' definitions'
//!   ids and names' links;
//! - a table of contents (after the status of the document), and a
//!   references section listing the specs it cites.
//!
//! References to other specs' terms which ReSpec finds by its xref service
//! link to them only with those specs' exported definitions (`terms`); the
//! web server's views don't have those, so those are plain text.  Not
//! rendered: ReSpec's boilerplate (ex: the status's and the conformance
//! section's text), its IDL index, and its references' titles.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use super::{Hooks, HtmlRenderer, Page, RenderedSpec, SpecLinks, fragment, link_url};
use crate::rendered_docs::escape;
use crate::spec_analyze::markup::{Doc, NodeKind, decode_html};
use crate::spec_analyze::respec::{
    GeneratedHeading, Rendering, SHORTHAND, local_scripts, rendering, shown_text,
};
use crate::spec_analyze::xref::Terms;
use crate::spec_analyze::{SpecEntry, SpecsConfig, spec_anchor};

lazy_static! {
    /// What ReSpec shows specially in text: shorthands, backticks' code, and
    /// `|variable|`s (core/inlines).
    static ref INLINE: Regex = Regex::new(&format!(
        r"{}|`([^`]+)`|\|(\w[\w ]*)(?::[^|]*)?\|",
        SHORTHAND.as_str()
    ))
    .unwrap();
}

struct Respec<'r> {
    doc: &'r Doc<'r>,
    rendering: &'r Rendering,
    hidden: Vec<bool>,
    /// References' targets' URLs, by their bytes' starts and ends.
    targets: HashMap<(usize, usize), String>,
    /// The definitions in IDL blocks, by their bytes' starts.
    idl_dfns: HashMap<usize, (usize, String)>,
    /// Where the table of contents goes: after the status of the document
    /// (or else the abstract), if there's one.
    toc_after: Option<usize>,
    spec: &'r SpecEntry,
    config: &'r SpecsConfig,
    links: &'r dyn SpecLinks,
}

impl Respec<'_> {
    /// The URL a symbol's anchor is at: this spec's, as a fragment, or another
    /// of the tree's specs' (its rendered view, if it has one).
    fn symbol_url(&self, sym: &str) -> Option<String> {
        let (spec, anchor) = self
            .config
            .specs
            .iter()
            .find_map(|spec| Some((spec, spec_anchor(spec.shortname(), sym)?)))?;
        if spec.shortname() == self.spec.shortname() {
            return Some(fragment(&anchor));
        }
        let url = format!("{}{}", spec.rendered.as_deref()?, fragment(&anchor));
        link_url(&url, None, self.config, self.links)
    }

    fn target(&self, range: &Range<usize>) -> Option<&str> {
        self.targets
            .get(&(range.start, range.end))
            .map(String::as_str)
    }

    /// A text node with what ReSpec shows specially in it (see `INLINE`), and
    /// IDL blocks' definitions and links.
    fn text(&self, out: &mut HtmlRenderer, node: usize) {
        let range = self.doc.nodes[node].range.clone();
        let in_idl = self.doc.ancestors(node).any(|a| self.doc.is(a, "pre"));
        let plain = self
            .doc
            .ancestors(node)
            .any(|a| matches!(self.doc.name(a), Some("pre" | "svg" | "code" | "a")));
        // The pieces: (bytes, how to show them).
        let mut pieces: Vec<(Range<usize>, Piece)> = vec![];
        let text = &self.doc.src[range.clone()];
        if in_idl {
            // (IDL blocks' definitions and links, by their names' bytes.)
            let mut at = range.start;
            while at < range.end {
                if let Some((end, id)) = self.idl_dfns.get(&at) {
                    pieces.push((at..*end, Piece::Dfn(id.clone())));
                    at = *end;
                    continue;
                }
                let next = self
                    .targets
                    .keys()
                    .filter(|(start, end)| *start == at && *end <= range.end)
                    .map(|(_, end)| *end)
                    .max();
                if let Some(end) = next {
                    pieces.push((
                        at..end,
                        Piece::Link(self.target(&(at..end)).unwrap().to_string()),
                    ));
                    at = end;
                    continue;
                }
                at += self.doc.src[at..].chars().next().map_or(1, char::len_utf8);
            }
        } else if !plain {
            for captures in INLINE.captures_iter(text) {
                let whole = captures.get(0).unwrap();
                let bytes = range.start + whole.start()..range.start + whole.end();
                let piece = if let Some(code) = captures.get(6) {
                    Piece::Code(code.as_str().to_string())
                } else if let Some(var) = captures.get(7) {
                    Piece::Var(var.as_str().trim().to_string())
                } else {
                    // (A `[[[#id]]]`'s is its anchor's.)
                    let shown = match captures.get(1) {
                        Some(inner) => decode_html(inner.as_str().trim_start_matches('#')),
                        None => decode_html(&shown_text(whole.as_str())),
                    };
                    let url = self.target(&bytes).map(str::to_string);
                    let is_code = captures.get(3).is_some() || captures.get(5).is_some();
                    match (captures.get(4), url) {
                        (Some(_), Some(url)) => Piece::Cite(shown, url),
                        (_, Some(url)) if is_code => Piece::CodeLink(shown, url),
                        (_, Some(url)) => Piece::ShownLink(shown, url),
                        (_, None) if is_code => Piece::Code(shown),
                        (_, None) => Piece::Shown(shown),
                    }
                };
                pieces.push((bytes, piece));
            }
        }
        let mut at = range.start;
        for (bytes, piece) in pieces {
            if bytes.start > at {
                out.text(at..bytes.start);
            }
            let real = out.file_range(&bytes);
            match piece {
                Piece::Dfn(id) => {
                    out.w.push("<dfn");
                    if out.w.claim_id(&id) {
                        write!(out.w.html, r#" id="{}""#, escape(&id)).unwrap();
                    }
                    out.w.push(">");
                    out.text(bytes.clone());
                    out.w.push("</dfn>");
                }
                Piece::Link(url) => {
                    write!(out.w.html, r#"<a href="{}">"#, escape(&url)).unwrap();
                    out.text(bytes.clone());
                    out.w.push("</a>");
                }
                Piece::Code(code) => {
                    out.w.push("<code>");
                    out.w.text(&code, real);
                    out.w.push("</code>");
                }
                Piece::Var(var) => {
                    out.w.push("<var>");
                    out.w.text(&var, real);
                    out.w.push("</var>");
                }
                Piece::Shown(shown) => out.w.text(&shown, real),
                Piece::ShownLink(shown, url) => {
                    write!(out.w.html, r#"<a href="{}">"#, escape(&url)).unwrap();
                    out.w.text(&shown, real);
                    out.w.push("</a>");
                }
                Piece::CodeLink(shown, url) => {
                    write!(out.w.html, r#"<code><a href="{}">"#, escape(&url)).unwrap();
                    out.w.text(&shown, real);
                    out.w.push("</a></code>");
                }
                Piece::Cite(shown, url) => {
                    write!(out.w.html, r#"<a href="{}">"#, escape(&url)).unwrap();
                    out.w.text(&shown, real);
                    out.w.push("</a>");
                }
            }
            at = bytes.end;
        }
        if at < range.end {
            out.text(at..range.end);
        }
    }
}

/// How a piece of text is shown.
enum Piece {
    /// An IDL block's definition, with its id.
    Dfn(String),
    /// An IDL block's name, linked.
    Link(String),
    Code(String),
    Var(String),
    /// A shorthand's text, without a link.
    Shown(String),
    ShownLink(String, String),
    CodeLink(String, String),
    /// A citation of a spec (`[SPEC]`), to the references.
    Cite(String, String),
}

impl Hooks for Respec<'_> {
    fn hidden(&self, node: usize) -> bool {
        self.hidden[node]
    }

    fn id(&self, node: usize) -> Option<&str> {
        self.rendering.ids.get(&node).map(String::as_str)
    }

    fn xref(&self, _node: usize) -> Option<&str> {
        None
    }

    fn href(&self, href: &str) -> Option<String> {
        if let Some(id) = href.strip_prefix('#') {
            return Some(fragment(&decode_html(id)));
        }
        link_url(href, self.spec.rendered.as_deref(), self.config, self.links)
    }

    fn src(&self, src: &str) -> Option<String> {
        let base = url::Url::parse(self.spec.rendered.as_deref()?).ok()?;
        let url = base.join(src).ok()?;
        matches!(url.scheme(), "https" | "http").then(|| url.to_string())
    }

    fn special(&self, out: &mut HtmlRenderer, node: usize) -> bool {
        let doc = self.doc;
        let name = match &doc.nodes[node].kind {
            NodeKind::Text => {
                self.text(out, node);
                return true;
            }
            NodeKind::Element(name) => name.as_str(),
            _ => return false,
        };
        match name {
            // (Links without `href`s, to what they're to.)
            "a" if !doc.has_attr(node, "href") => {
                let content = doc.content_range(node);
                let range = if content.is_empty() {
                    doc.nodes[node].range.clone()
                } else {
                    content
                };
                match self.target(&range).map(str::to_string) {
                    Some(url) if out.in_link == 0 => {
                        out.w.push("<a");
                        if let Some(id) = self.id(node)
                            && out.w.claim_id(id)
                        {
                            write!(out.w.html, r#" id="{}""#, escape(id)).unwrap();
                        }
                        write!(out.w.html, r#" href="{}">"#, escape(&url)).unwrap();
                        out.in_link += 1;
                        out.children(self, node);
                        out.in_link -= 1;
                        out.w.push("</a>");
                    }
                    _ => out.children(self, node),
                }
                true
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let opened = out.open_block(doc.nodes[node].range.clone(), "");
                // (A section ReSpec makes from a heading has its id before it.)
                if let Some(id) = self.rendering.section_ids.get(&node)
                    && out.w.claim_id(id)
                {
                    write!(out.w.html, r#"<span id="{}"></span>"#, escape(id)).unwrap();
                }
                let tag = out.start_tag(self, node, name);
                if let Some(number) = self.rendering.numbers.get(&node) {
                    write!(
                        out.w.html,
                        r#"<span class="sp-secno">{}</span> "#,
                        escape(number)
                    )
                    .unwrap();
                }
                out.children(self, node);
                out.end_tag(tag);
                out.w.close_block_div(opened);
                true
            }
            "section" => {
                let tag = out.start_tag(self, node, name);
                if let Some(heading) = self.rendering.generated.get(&node) {
                    self.generated_heading(out, heading);
                }
                out.children(self, node);
                out.end_tag(tag);
                if self.toc_after == Some(node) {
                    self.toc(out);
                }
                true
            }
            "body" => {
                if self.toc_after.is_none() {
                    self.toc(out);
                }
                out.children(self, node);
                self.references(out);
                true
            }
            _ => false,
        }
    }
}

impl Respec<'_> {
    fn toc(&self, out: &mut HtmlRenderer) {
        if self.rendering.toc.is_empty() {
            return;
        }
        out.w
            .push(r#"<h2 class="sp-no-num">Table of contents</h2>"#);
        let entries: Vec<(usize, Option<String>, String, String)> = self
            .rendering
            .toc
            .iter()
            .map(|(level, number, id, text)| (*level, number.clone(), fragment(id), text.clone()))
            .collect();
        super::write_toc(out, &entries);
    }

    /// A heading ReSpec generates (as an `<h2>` and lower).
    fn generated_heading(&self, out: &mut HtmlRenderer, heading: &GeneratedHeading) {
        let level = heading.level.clamp(2, 6);
        write!(out.w.html, "<h{}", level).unwrap();
        if let Some(id) = &heading.id
            && out.w.claim_id(id)
        {
            write!(out.w.html, r#" id="{}""#, escape(id)).unwrap();
        }
        out.w.push(">");
        if let Some(number) = &heading.number {
            write!(
                out.w.html,
                r#"<span class="sp-secno">{}</span> "#,
                escape(number)
            )
            .unwrap();
        }
        out.w.escaped(&heading.text);
        write!(out.w.html, "</h{}>", level).unwrap();
    }

    /// The references section: the specs the spec cites, with their ids.
    fn references(&self, out: &mut HtmlRenderer) {
        if self.rendering.cited.is_empty() {
            return;
        }
        let section = |out: &mut HtmlRenderer, id: &str| {
            out.w.push(r#"<div class="sp-section""#);
            if out.w.claim_id(id) {
                write!(out.w.html, r#" id="{}""#, escape(id)).unwrap();
            }
            out.w.push(">");
        };
        section(out, "references");
        match &self.rendering.references {
            Some(heading) => self.generated_heading(out, heading),
            None => out.w.push("<h2>References</h2>"),
        }
        section(out, "normative-references");
        if let Some(heading) = &self.rendering.normative_references {
            let heading = GeneratedHeading {
                level: 3,
                ..heading.clone()
            };
            self.generated_heading(out, &heading);
        }
        out.w.push("<dl>");
        for (spec, id) in &self.rendering.cited {
            out.w.push("<dt");
            if out.w.claim_id(id) {
                write!(out.w.html, r#" id="{}""#, escape(id)).unwrap();
            }
            out.w.push(">[");
            out.w.escaped(spec);
            out.w.push("]</dt>");
        }
        out.w.push("</dl></div></div>");
    }
}

/// Render the ReSpec spec `spec`, whose source is at `path` in the tree,
/// among `config`'s specs, reading its files (and the files it includes and
/// the local scripts it loads) with `read`, with the other specs' exported
/// definitions `terms`, if known, and the targets of its references which
/// the index's analysis of it has, by their locations in its source (lines,
/// and columns in bytes): those of other specs' terms, which only spec-analyze
/// finds (see `format::format_rendered`).
pub fn render(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    terms: &Terms,
    index_targets: &[(u32, u32, u32, String)],
) -> Option<RenderedSpec> {
    let source = read(path)?;
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let scripts: String = local_scripts(&source)
        .iter()
        .filter_map(|script| {
            read(&crate::spec_analyze::normalize_path(&format!(
                "{}/{}",
                dir, script
            )))
        })
        .filter(|text| text.contains("respecConfig"))
        .collect::<Vec<_>>()
        .join("\n");
    let (combined, rendering) = rendering(path, read, &scripts, spec.shortname(), config, terms)?;
    let doc = Doc::parse(&combined.text);
    let n = doc.nodes.len();
    let mut hidden = vec![false; n];
    for node in 1..n {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        let class = doc.attr(node, "class").unwrap_or_default();
        hidden[node] = hidden[parent]
            || class
                .split_whitespace()
                .any(|c| c == "remove" || c == "removeOnSave")
            || (doc.is(node, "section") && doc.attr(node, "id").as_deref() == Some("toc"));
    }
    let title = (1..n)
        .find(|&node| doc.is(node, "title"))
        .map(|node| {
            doc.text_content(node)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| path.to_string());
    let section_with_id = |id: &str| {
        (1..n).find(|&node| doc.is(node, "section") && doc.attr(node, "id").as_deref() == Some(id))
    };
    let mut hooks = Respec {
        doc: &doc,
        rendering: &rendering,
        hidden,
        targets: HashMap::new(),
        idl_dfns: rendering
            .idl_dfns
            .iter()
            .map(|(range, id)| (range.start, (range.end, id.clone())))
            .collect(),
        toc_after: section_with_id("sotd").or_else(|| section_with_id("abstract")),
        spec,
        config,
        links,
    };
    hooks.targets = rendering
        .targets
        .iter()
        .filter_map(|(range, sym)| Some(((range.start, range.end), hooks.symbol_url(sym)?)))
        .collect();
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    for (lineno, col_start, col_end, sym) in index_targets {
        let Some(&line) = line_starts.get((*lineno as usize).saturating_sub(1)) else {
            continue;
        };
        let (start, end) = (line + *col_start as usize, line + *col_end as usize);
        let Some(text_start) = combined.text_offset(0, start) else {
            continue;
        };
        let key = (text_start, text_start + end.saturating_sub(start));
        if !hooks.targets.contains_key(&key)
            && let Some(url) = hooks.symbol_url(sym)
        {
            hooks.targets.insert(key, url);
        }
    }
    let map = |range: &Range<usize>| combined.file_range(0, range);
    let mut out = HtmlRenderer::for_file(&doc, 0..combined.text.len(), &source, &map);
    out.children(&hooks, 0);
    let mut rendered = out.w.finish();
    rendered.title = Some(title.clone());
    Some(RenderedSpec {
        rendered,
        pages: vec![Page {
            name: None,
            title,
            start: 0,
        }],
        page: 0,
        range: 0..source.len(),
        steps: vec![],
        file: None,
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

    #[test]
    fn test_render() {
        let source = r#"<!DOCTYPE html>
<html><head><title>Gizmos</title>
<script class="remove">var respecConfig = { shortName: "gizmos" };</script></head>
<body>
<section id="abstract"><p>Gizmos.</p></section>
<section id="sotd"><p>A draft.</p></section>
<section>
  <h2>Introduction</h2>
  <p>A <dfn>gizmo</dfn> is poked by {{Gizmo/poke()}}, see [[DOM]] and `code`.</p>
  <pre class="idl">
interface Gizmo {
  undefined poke();
};
dictionary GizmoOptions {};
  </pre>
  <p>The <dfn>poke()</dfn> method steps are to poke |gizmo|, a [=gizmo=].</p>
  <p><a>Gizmos</a> are <a href="https://dom.spec.whatwg.org/#concept-node">nodes</a>.</p>
</section>
<section id="conformance"></section>
</body></html>
"#;
        let config: SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "gizmos"
            path = "gizmos"
            format = "respec"
            rendered = "https://example.org/gizmos/"
            [[spec]]
            name = "dom"
            format = "wattsi"
            rendered = "https://dom.spec.whatwg.org/"
            "#,
        )
        .unwrap();
        let read = |file: &str| (file == "gizmos/index.html").then(|| source.to_string());
        let rendered = render(
            "gizmos/index.html",
            &config.specs[0],
            &config,
            &TestLinks,
            &read,
            &Terms::default(),
            &[],
        )
        .unwrap();
        let r = &rendered.rendered;
        // (Without the runs' spans, for checking.)
        let html = Regex::new(r#"<span data-l="\d+" data-c="\d+"[^>]*>([^<]*)</span>"#)
            .unwrap()
            .replace_all(&r.html, "$1")
            .into_owned();
        let has = |part: &str| assert!(html.contains(part), "{} isn't in {}", part, html);
        assert_eq!(r.title.as_deref(), Some("Gizmos"));
        // Generated and numbered headings, with ReSpec's ids.
        has(r#"<h2>Abstract</h2>"#);
        has(r#"<h2 id="x1-introduction"><span class="sp-secno">1.</span> Introduction</h2>"#);
        has(r#"<h2 id="x2-conformance"><span class="sp-secno">2.</span> Conformance</h2>"#);
        // A table of contents, after the status of the document.
        has(r##"<a href="#introduction"><span class="sp-secno">1.</span> Introduction</a>"##);
        // Definitions' ids, and the IDL block's (which the prose's don't
        // define, without `data-dfn-for`s).
        has(r#"<dfn id="dfn-gizmo">gizmo</dfn>"#);
        has(r#"<dfn id="dfn-poke">poke()</dfn>"#);
        has(r#"interface <dfn id="dom-gizmo">Gizmo</dfn>"#);
        has(r#"<dfn id="dom-gizmo-poke">poke</dfn>"#);
        has(r#"<dfn id="dom-gizmooptions">GizmoOptions</dfn>"#);
        // Shorthands, as ReSpec shows them, linked.
        has(r##"<code><a href="#dom-gizmo-poke">poke()</a></code>"##);
        has(r##"<a href="#dfn-gizmo">gizmo</a>"##);
        has(r##"<a href="#bib-dom">[DOM]</a>"##);
        has("<code>code</code>");
        has("<var>gizmo</var>");
        // Links, without `href`s and to the tree's specs' views.
        has(r##"<a href="#dfn-gizmo">Gizmos</a>"##);
        has(r#"<a href="/t/rendered/dom/source/anchor/concept-node">nodes</a>"#);
        // The references.
        has(r#"<dt id="bib-dom">[DOM]</dt>"#);
        assert!(!html.contains("respecConfig"), "{}", html);
        // Blocks are of the source's lines.
        assert!(!r.blocks.is_empty());
    }
}
