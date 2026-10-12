//! Bikeshed specs, rendered from Bikeshed's own output, as mozsearch's fork
//! writes it with `bikeshed index --html` (which spec-analyze runs, into the
//! index's `rendered-specs` directory; see scripts/spec-analyze.sh), where
//! the elements with positions in the source have them as `bs-pos`
//! attributes:
//!
//! - each block (paragraph, heading, list item, table row, ...) with a
//!   position in the spec's source is of its lines, up to the next block's;
//! - text which is verbatim in its block's lines (most prose, but not, ex,
//!   what shorthands become) says where it is there, for the blame popup;
//! - the output is sanitized like other specs' renderings (see
//!   `HtmlRenderer`), and its links to the tree's specs go to their rendered
//!   views.
//!
//! Only the latest revision has a rendered view (the index's output), and the
//! content of the files a spec includes doesn't have blame.  Not rendered:
//! Bikeshed's scripts (ex: its definitions' panels) and its SVG (ex: railroad
//! diagrams).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use super::{Hooks, HtmlRenderer, Page, RenderedSpec, Role, SpecLinks, fragment, link_url, role};
use crate::spec_analyze::markup::{Doc, NodeKind, decode_html};
use crate::spec_analyze::{SpecEntry, SpecsConfig};

/// How many lines a block can have at most, if the next block is far away
/// (ex: the last in a section, before generated content).
const MAX_BLOCK_LINES: usize = 40;

struct Bikeshed<'r> {
    doc: &'r Doc<'r>,
    source: &'r str,
    /// Blocks' bytes of the source, by their nodes.
    blocks: HashMap<usize, Range<usize>>,
    /// How far into each block's bytes (by their start) its text was found.
    cursors: RefCell<HashMap<usize, usize>>,
    spec: &'r SpecEntry,
    config: &'r SpecsConfig,
    links: &'r dyn SpecLinks,
}

impl Bikeshed<'_> {
    /// The bytes of the source that the block a node is in is of, if any.
    fn block_of(&self, node: usize) -> Option<&Range<usize>> {
        std::iter::once(node)
            .chain(self.doc.ancestors(node))
            .find_map(|n| self.blocks.get(&n))
    }

    /// Text, as runs where it's in its block's lines verbatim (in order).
    fn text(&self, out: &mut HtmlRenderer, node: usize) {
        let range = self.doc.nodes[node].range.clone();
        let text = decode_html(&self.doc.src[range]);
        let block = self.block_of(node).cloned();
        let Some(block) = block.filter(|_| out.w.in_block() && !text.trim().is_empty()) else {
            out.w.escaped(&text);
            return;
        };
        let mut cursors = self.cursors.borrow_mut();
        let cursor = cursors.entry(block.start).or_insert(block.start);
        for piece in text.split_inclusive('\n') {
            let content = piece.trim();
            let found = (!content.is_empty())
                .then(|| self.source[*cursor..block.end].find(content))
                .flatten();
            match found {
                Some(at) => {
                    let start = *cursor + at;
                    let lead = piece.len() - piece.trim_start().len();
                    out.w.escaped(&piece[..lead]);
                    out.w.text(content, Some(start..start + content.len()));
                    out.w.escaped(&piece[lead + content.len()..]);
                    *cursor = start + content.len();
                }
                None => out.w.escaped(piece),
            }
        }
    }
}

impl Hooks for Bikeshed<'_> {
    fn hidden(&self, _node: usize) -> bool {
        false
    }

    fn id(&self, node: usize) -> Option<&str> {
        self.doc.raw_attr(node, "id").filter(|id| !id.contains('&'))
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
        if self.doc.nodes[node].kind == NodeKind::Text {
            self.text(out, node);
            return true;
        }
        false
    }

    fn block_range(&self, node: usize) -> Option<Option<Range<usize>>> {
        Some(self.blocks.get(&node).cloned())
    }
}

/// The blocks of Bikeshed's output with positions in the spec's source (but
/// not in files it includes), as the bytes of the source from their line to
/// the next block's.
fn blocks(doc: &Doc, source: &str) -> HashMap<usize, Range<usize>> {
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    let mut lines: Vec<(usize, usize)> = vec![];
    for node in 1..doc.nodes.len() {
        let Some(name) = doc.name(node) else {
            continue;
        };
        if !matches!(
            role(name),
            Role::Container | Role::Leaf | Role::InnerLeaf | Role::Row
        ) {
            continue;
        }
        let Some(line) = doc
            .raw_attr(node, "bs-pos")
            .and_then(|pos| pos.parse::<usize>().ok())
            .filter(|&line| line >= 1 && line <= line_starts.len())
        else {
            continue;
        };
        lines.push((node, line));
    }
    let mut boundaries: Vec<usize> = lines.iter().map(|&(_, line)| line).collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let line_start = |line: usize| line_starts.get(line - 1).copied().unwrap_or(source.len());
    lines
        .into_iter()
        .map(|(node, line)| {
            let next = boundaries
                .get(boundaries.partition_point(|&b| b <= line))
                .copied()
                .unwrap_or(usize::MAX)
                .min(line + MAX_BLOCK_LINES);
            let start = line_start(line);
            let end = line_start(next).max(start);
            let end = start + source[start..end].trim_end().len();
            (node, start..end.max(start + 1).min(source.len().max(start)))
        })
        .collect()
}

/// Render Bikeshed's `output` for the spec `spec`, whose source `source` is
/// at `path` in the tree, among `config`'s specs.
pub fn render(
    output: &str,
    source: &str,
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
) -> Option<RenderedSpec> {
    let doc = Doc::parse(output);
    let title = (1..doc.nodes.len())
        .find(|&node| doc.is(node, "title"))
        .map(|node| {
            doc.text_content(node)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| path.to_string());
    let hooks = Bikeshed {
        doc: &doc,
        source,
        blocks: blocks(&doc, source),
        cursors: RefCell::new(HashMap::new()),
        spec,
        config,
        links,
    };
    let map = |_: &Range<usize>| None;
    let mut out = HtmlRenderer::for_file(&doc, 0..output.len(), source, &map);
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
        let source = "Title: Example\n\n# Widgets # {#widgets}\n\nA <dfn>widget</dfn> is a\n[=widget=] thing.\n\n1. One\n2. Two\n";
        let output = r###"<!doctype html><html><head><title>Example</title><style>x</style></head>
<body><h2 bs-pos="3" id="widgets"><span class="secno">1. </span><span class="content">Widgets</span></h2>
<p bs-pos="5">A <dfn id="widget">widget</dfn> is a
<a href="#widget">widget</a> thing.</p>
<ol bs-pos="8"><li bs-pos="8">One<li bs-pos="9">Two</ol>
<p bs-pos="other.md:3">Included.</p>
<p>Generated, with <a href="https://dom.spec.whatwg.org/#concept-node">a link</a>.</p>
<script>panels()</script></body></html>"###;
        let config: SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "example"
            format = "bikeshed"
            rendered = "https://example.org/"
            [[spec]]
            name = "dom"
            format = "wattsi"
            rendered = "https://dom.spec.whatwg.org/"
            "#,
        )
        .unwrap();
        let rendered = render(
            output,
            source,
            "example/index.bs",
            &config.specs[0],
            &config,
            &TestLinks,
        )
        .unwrap();
        let r = &rendered.rendered;
        assert_eq!(r.title.as_deref(), Some("Example"));
        // Blocks of their lines, up to the next block's: the heading, the
        // paragraph, and the list items (but not the included or generated
        // paragraphs).
        let lines: Vec<(usize, usize)> = r
            .blocks
            .iter()
            .map(|b| {
                (
                    source[..b.range.start].matches('\n').count() + 1,
                    source[..b.range.end].matches('\n').count() + 1,
                )
            })
            .collect();
        assert_eq!(lines, vec![(3, 3), (5, 6), (8, 8), (9, 9)]);
        // Verbatim text says where it is, but generated text doesn't.
        assert!(r.html.contains(r#"<span data-l="5" data-c="0">A</span> <dfn id="widget"><span data-l="5" data-c="7">widget</span></dfn>"#), "{}", r.html);
        assert!(r.html.contains(r##"<a href="#widget"><span data-l="6" data-c="2">widget</span></a> <span data-l="6" data-c="11">thing.</span>"##), "{}", r.html);
        assert!(r.html.contains("<p>Included.</p>"), "{}", r.html);
        assert!(
            r.html.contains(r#"<span class="sp-secno">1. </span>"#),
            "{}",
            r.html
        );
        // Links to the tree's specs go to their rendered views.
        assert!(
            r.html
                .contains(r#"<a href="/t/rendered/dom/source/anchor/concept-node">a link</a>"#),
            "{}",
            r.html
        );
        assert!(
            !r.html.contains("panels()") && !r.html.contains("<style"),
            "{}",
            r.html
        );
    }
}
