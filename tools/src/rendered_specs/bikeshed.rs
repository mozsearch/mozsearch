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
//! Only the latest revision has a rendered view (the index's output).  The
//! files a spec includes (`<pre class=include>`, whose elements' positions
//! are `bs-pos="<file>:<line>"`) have their own views, of their parts of the
//! output, with their blame (`render_file`); in the spec's view, their
//! content doesn't have blame.  Not rendered:
//! Bikeshed's scripts (ex: its definitions' panels) and its SVG (ex: railroad
//! diagrams).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;

use super::{Hooks, HtmlRenderer, Page, RenderedSpec, Role, SpecLinks, fragment, link_url, role};
use crate::rendered_docs::escape;
use crate::spec_analyze::markup::{Doc, NodeKind, decode_html};
use crate::spec_analyze::wattsi::step_path;
use crate::spec_analyze::{SpecEntry, SpecsConfig, spec_symbol};
use crate::tree_sitter_support::spec_tokenizer::{idl_steps_lead_end, opens_idl_steps};

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
    /// For a view of a part of the spec (an algorithm's), the URL of its
    /// rendered view, which its links to its ids are to.
    base: Option<String>,
    /// Algorithms' first `<dfn>`s, and the URLs of their views beside the
    /// code citing them.
    algorithm_dfns: HashMap<usize, String>,
    /// The first elements with the content of the files the spec includes,
    /// and the files' paths in the tree (for links to their views).
    includes: HashMap<usize, String>,
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
            let fragment = fragment(&decode_html(id));
            return Some(match &self.base {
                Some(base) => format!("{}{}", base, fragment),
                None => fragment,
            });
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
        if let Some(file) = self.includes.get(&node) {
            write!(
                out.w.html,
                r#"<div class="sp-imported">From <a href="{}">{}</a>:</div>"#,
                escape(&self.links.rendered_url(file)),
                escape(file)
            )
            .unwrap();
        }
        if let Some(url) = self.algorithm_dfns.get(&node) {
            let tag = out.start_tag(self, node, "dfn");
            out.children(self, node);
            out.end_tag(tag);
            super::algorithm_link(out, url);
            return true;
        }
        false
    }

    fn block_range(&self, node: usize) -> Option<Option<Range<usize>>> {
        Some(self.blocks.get(&node).cloned())
    }
}

/// A `bs-pos`'s file (relative to the spec's source, for a file it includes)
/// and line.
fn position(pos: &str) -> Option<(Option<&str>, usize)> {
    match pos.rsplit_once(':') {
        Some((file, line)) if !file.is_empty() => Some((Some(file), line.parse().ok()?)),
        _ => Some((None, pos.parse().ok()?)),
    }
}

/// The files (relative to the spec's source) whose content Bikeshed's output
/// has, but the source's, in order (but those which are only styles or
/// scripts, ex: a stylesheet included in a `<style>`).
pub fn included_files(output: &str) -> Vec<String> {
    let mut files: Vec<String> = vec![];
    for (at, _) in output.match_indices(" bs-pos=\"") {
        let rest = &output[at + 9..];
        let Some(pos) = rest.split('"').next() else {
            continue;
        };
        let tag = output[..at]
            .rfind('<')
            .map(|start| &output[start + 1..at])
            .and_then(|tag| tag.split_whitespace().next())
            .unwrap_or("");
        if matches!(tag, "style" | "script" | "link" | "meta") {
            continue;
        }
        if let Some((Some(file), _)) = position(pos)
            && !files.iter().any(|f| f == file)
        {
            files.push(file.to_string());
        }
    }
    files
}

/// Where the content of a file a spec includes (relative to its source) is
/// in Bikeshed's output: its first element and the bytes of the elements.
fn included_region(doc: &Doc, file: &str) -> Option<(usize, Range<usize>)> {
    // (The elements with the file's content where it's included: the
    // most of them which are siblings (Bikeshed copies some elsewhere, ex:
    // into its indexes), from the first of them up to another position (a
    // file can be included more than once, ex: filter-effects'
    // templates).)
    let of_file = |node: usize| {
        doc.raw_attr(node, "bs-pos")
            .and_then(position)
            .is_some_and(|(f, _)| f == Some(file))
    };
    let mut by_parent: Vec<(usize, Vec<usize>)> = vec![];
    for node in 1..doc.nodes.len() {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        if !of_file(node) || of_file(parent) {
            continue;
        }
        match by_parent.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, nodes)) => nodes.push(node),
            None => by_parent.push((parent, vec![node])),
        }
    }
    let (parent, nodes) = by_parent
        .iter()
        .rev()
        .max_by_key(|(_, nodes)| nodes.len())?;
    let first = nodes[0];
    let siblings = &doc.nodes[*parent].children;
    let at = siblings.iter().position(|&s| s == first)?;
    let mut end = doc.nodes[first].range.end;
    for &sibling in &siblings[at + 1..] {
        match doc.raw_attr(sibling, "bs-pos").and_then(position) {
            Some((f, _)) if f == Some(file) => end = doc.nodes[sibling].range.end,
            Some(_) => break,
            None => {}
        }
    }
    Some((first, doc.nodes[first].range.start..end))
}

/// The blocks of Bikeshed's output with positions in the spec's source (or
/// with `file`, in a file it includes), as the bytes of the source (or the
/// file) from their line to the next block's.
fn blocks(doc: &Doc, source: &str, file: Option<&str>) -> HashMap<usize, Range<usize>> {
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
            .and_then(position)
            .filter(|&(f, _)| f == file)
            .map(|(_, line)| line)
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

/// The algorithms of Bikeshed's output: elements with `data-algorithm`
/// (ex: `<div algorithm>`'s), and else paragraphs starting "To <dfn>" with
/// the lists after them, named by their first `<dfn>`s, and IDL members'
/// steps (ex: "The <dfn>appendChild(node)</dfn> method steps are to ...", as
/// spec-analyze has them, see `spec_tokenizer::idl_steps`), named by each
/// member their leads define or link to: their nodes' bytes, and their names'
/// nodes and ids.
fn algorithms(doc: &Doc) -> Vec<(Range<usize>, usize, String)> {
    let mut algorithms = vec![];
    // The lists after a paragraph (its siblings, but whitespace).
    let with_lists = |node: usize| -> Option<usize> {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        let siblings = &doc.nodes[parent].children;
        let at = siblings.iter().position(|&s| s == node).unwrap_or(0);
        let mut end = None;
        for &sibling in &siblings[at + 1..] {
            match &doc.nodes[sibling].kind {
                NodeKind::Text if doc.src[doc.nodes[sibling].range.clone()].trim().is_empty() => {}
                NodeKind::Element(name) if name == "ol" => end = Some(doc.nodes[sibling].range.end),
                _ => break,
            }
        }
        end
    };
    let in_algorithm = |node: usize| {
        doc.ancestors(node)
            .any(|a| doc.has_attr(a, "data-algorithm"))
    };
    for node in 1..doc.nodes.len() {
        let range = doc.nodes[node].range.clone();
        if doc.has_attr(node, "data-algorithm")
            || (doc.is(node, "p")
                && doc.text_content(node).trim_start().starts_with("To ")
                && !in_algorithm(node))
        {
            let algorithm = if doc.is(node, "p") {
                let Some(end) = with_lists(node) else {
                    continue;
                };
                range.start..end
            } else {
                range
            };
            let dfn = (node..doc.nodes.len())
                .take_while(|&n| doc.nodes[n].range.start < algorithm.end)
                .find(|&n| doc.is(n, "dfn") && doc.has_attr(n, "id"));
            if let Some(dfn) = dfn
                && let Some(id) = doc.attr(dfn, "id")
            {
                algorithms.push((algorithm, dfn, id));
            }
        } else if doc.is(node, "p")
            && !in_algorithm(node)
            && let Some(listed) = opens_idl_steps(&doc.text_content(node))
        {
            let end = if listed { with_lists(node) } else { None };
            let algorithm = range.start..end.unwrap_or(range.end);
            let lead_end = range.start + idl_steps_lead_end(&doc.src[range.clone()]);
            for n in
                (node + 1..doc.nodes.len()).take_while(|&n| doc.nodes[n].range.start < lead_end)
            {
                let id = if doc.is(n, "dfn") {
                    doc.attr(n, "id")
                } else if doc.is(n, "a")
                    && doc
                        .attr(n, "data-link-type")
                        .is_some_and(|t| IDL_MEMBER_LINK_TYPES.contains(&t.as_str()))
                {
                    doc.attr(n, "href")
                        .and_then(|href| href.strip_prefix('#').map(str::to_string))
                } else {
                    None
                };
                if let Some(id) = id {
                    algorithms.push((algorithm.clone(), n, id));
                }
            }
        }
    }
    algorithms
}

/// The types of Bikeshed's links to IDL members, which name the members whose
/// steps a paragraph opens.
const IDL_MEMBER_LINK_TYPES: &[&str] = &[
    "method",
    "constructor",
    "attribute",
    "idl",
    "stringifier",
    "iterator",
];

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
    render_part(output, source, path, spec, config, links, Part::All)
}

/// Render the part of Bikeshed's output for the spec `spec`, whose source is
/// at `path`, that has the content of a file it includes, at `file` in the
/// tree (whose text is `file_source`), with its links to the spec's ids to
/// the spec's view.
pub fn render_file(
    output: &str,
    file_source: &str,
    path: &str,
    file: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
) -> Option<RenderedSpec> {
    let dir = std::path::Path::new(path)
        .parent()
        .and_then(|dir| dir.to_str())
        .unwrap_or("");
    let relative = match dir {
        "" => file,
        dir => file.strip_prefix(dir)?.strip_prefix('/')?,
    };
    let mut rendered = render_part(
        output,
        file_source,
        path,
        spec,
        config,
        links,
        Part::File(relative),
    )?;
    rendered.file = Some(file.to_string());
    Some(rendered)
}

/// What part of Bikeshed's output to render.
#[derive(Clone, Copy)]
enum Part<'a> {
    All,
    /// The algorithm which defines the symbol.
    Algorithm(&'a str),
    /// The content of the file the spec includes (relative to its source).
    File(&'a str),
}

/// Render the algorithm in Bikeshed's `output` which defines `sym` (see
/// `render`), with its steps' numbers and where they start, for its view
/// beside the code citing them (see `format::format_algorithm`).
pub fn render_algorithm(
    output: &str,
    source: &str,
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    sym: &str,
) -> Option<RenderedSpec> {
    render_part(
        output,
        source,
        path,
        spec,
        config,
        links,
        Part::Algorithm(sym),
    )
}

fn render_part(
    output: &str,
    source: &str,
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    part: Part,
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
    let algorithms = algorithms(&doc);
    let file = match part {
        Part::File(file) => Some(file),
        _ => None,
    };
    let blocks = blocks(&doc, source, file);
    // (In the spec's view, where each included file's content starts.)
    let dir = std::path::Path::new(path)
        .parent()
        .and_then(|dir| dir.to_str())
        .unwrap_or("");
    let mut includes: HashMap<usize, String> = HashMap::new();
    if matches!(part, Part::All) {
        let mut seen: Vec<&str> = vec![];
        for node in 1..doc.nodes.len() {
            let Some((Some(included), _)) = doc.raw_attr(node, "bs-pos").and_then(position) else {
                continue;
            };
            if !seen.contains(&included) {
                seen.push(included);
                let file = match dir {
                    "" => included.to_string(),
                    dir => format!("{}/{}", dir, included),
                };
                includes.insert(node, file);
            }
        }
    }
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    // The bytes of the output to render, and for an algorithm, its steps.
    let (range, steps) = match part {
        Part::All => (0..output.len(), vec![]),
        Part::File(file) => (included_region(&doc, file)?.1, vec![]),
        Part::Algorithm(sym) => {
            let (range, _, _) = algorithms
                .iter()
                .filter(|(_, _, id)| spec_symbol(spec.shortname(), id) == sym)
                .min_by_key(|(range, _, _)| range.len())?;
            let steps: Vec<(String, usize)> = (1..doc.nodes.len())
                .filter(|&node| {
                    doc.is(node, "li")
                        && doc.nodes[node].parent.is_some_and(|p| doc.is(p, "ol"))
                        && range.contains(&doc.nodes[node].range.start)
                })
                .filter_map(|node| {
                    let line: usize = doc.raw_attr(node, "bs-pos")?.parse().ok()?;
                    let at = *line_starts.get(line.checked_sub(1)?)?;
                    Some((step_path(&doc, node, range.start), at))
                })
                .collect();
            (range.clone(), steps)
        }
    };
    // The bytes of the source that it covers: an algorithm's blocks', so that
    // its blame doesn't take in the lines before and after it.
    let source_range = match part {
        Part::Algorithm(_) => blocks
            .iter()
            .filter(|(node, _)| range.contains(&doc.nodes[**node].range.start))
            .map(|(_, block)| block.clone())
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .unwrap_or(0..0),
        _ => 0..source.len(),
    };
    let hooks = Bikeshed {
        doc: &doc,
        source,
        blocks,
        cursors: RefCell::new(HashMap::new()),
        spec,
        config,
        links,
        base: (!matches!(part, Part::All)).then(|| links.rendered_url(path)),
        algorithm_dfns: algorithms
            .iter()
            .filter_map(|(_, node, id)| {
                let url = links.algorithm_url(&spec_symbol(spec.shortname(), id))?;
                Some((*node, url))
            })
            .collect(),
        includes,
    };
    let map = |_: &Range<usize>| None;
    let mut out = HtmlRenderer::for_file(&doc, range, source, &map);
    if file.is_some() {
        // (An included file's view is a part of the spec.)
        write!(
            out.w.html,
            r#"<div class="sp-imported">Part of <a href="{}">{}</a>, included by <a href="{}">{}</a>.</div>"#,
            escape(&links.rendered_url(path)),
            escape(&title),
            escape(&links.rendered_url(path)),
            escape(path)
        )
        .unwrap();
    }
    out.children(&hooks, 0);
    let mut rendered = out.w.finish();
    // (A file's view is titled by the file, in the spec.)
    let title = match file {
        Some(file) => format!("{} ({})", file, title),
        None => title,
    };
    rendered.title = Some(title.clone());
    Some(RenderedSpec {
        rendered,
        pages: vec![Page {
            name: None,
            title,
            start: 0,
        }],
        page: 0,
        range: source_range,
        steps,
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
        // (Which links to its file's view.)
        assert!(
            r.html
                .contains(r#"From <a href="/t/rendered/example/other.md">example/other.md</a>"#),
            "{}",
            r.html
        );
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

        // The included file's view: its part, with its blocks.
        assert_eq!(included_files(output), vec!["other.md"]);
        let file_source = "Other\n\nIncluded.\n";
        let view = render_file(
            output,
            file_source,
            "example/index.bs",
            "example/other.md",
            &config.specs[0],
            &config,
            &TestLinks,
        )
        .unwrap();
        let r = &view.rendered;
        assert_eq!(view.file.as_deref(), Some("example/other.md"));
        assert_eq!(r.title.as_deref(), Some("other.md (Example)"));
        assert!(
            !r.html.contains("Widgets") && !r.html.contains("Generated"),
            "{}",
            r.html
        );
        assert!(
            r.html
                .contains(r#"<span data-l="3" data-c="0">Included.</span>"#),
            "{}",
            r.html
        );
        let ranges: Vec<&str> = r
            .blocks
            .iter()
            .map(|b| &file_source[b.range.clone()])
            .collect();
        assert_eq!(ranges, vec!["Included."]);
        assert!(
            r.html
                .contains(r#"Part of <a href="/t/rendered/example/index.bs">Example</a>"#),
            "{}",
            r.html
        );
    }

    #[test]
    fn test_algorithm() {
        let source = "To poke:\n\n1. One\n2. Two\n\nAfter.\n";
        let output = r###"<html><head><title>Example</title></head><body>
<div bs-pos="1" data-algorithm="poke"><p bs-pos="1">To <dfn id="poke">poke</dfn>, see <a href="#poke">it</a>:</p>
<ol bs-pos="3"><li bs-pos="3">One<li bs-pos="4">Two</ol></div>
<p bs-pos="6">After.</p></body></html>"###;
        let config: SpecsConfig = toml::from_str(
            "[[spec]]\nname = \"example\"\nformat = \"bikeshed\"\nrendered = \"https://example.org/\"\n",
        )
        .unwrap();
        let view = render_algorithm(
            output,
            source,
            "example/index.bs",
            &config.specs[0],
            &config,
            &TestLinks,
            "SPEC_example_poke",
        )
        .unwrap();
        let html = &view.rendered.html;
        assert!(!html.contains("After"), "{}", html);
        // (Its links to its ids are to the spec's rendered view.)
        assert!(
            html.contains(r##"href="/t/rendered/example/index.bs#poke""##),
            "{}",
            html
        );
        let steps: Vec<(&str, &str)> = view
            .steps
            .iter()
            .map(|(path, at)| (path.as_str(), &source[*at..*at + 2]))
            .collect();
        assert_eq!(steps, vec![("1", "1."), ("2", "2.")]);
        // (Its bytes of the source are its own, not the file's.)
        assert_eq!(&source[view.range.clone()], "To poke:\n\n1. One\n2. Two");
    }
}
