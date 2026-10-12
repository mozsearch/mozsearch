//! The document tree of a spec's HTML-based source, from the spec tokenizer's
//! lexer (see `spec_tokenizer::lex`), with enough of HTML's parsing rules
//! (void elements and implied end tags, ex: a `<p>` ended by the next block,
//! or an `<li>` by the next `<li>`) to tell which elements contain which.

use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use crate::tree_sitter_support::spec_tokenizer::{Item, attr, lex};

/// Elements which have no end tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param",
    "source", "track", "wbr",
];

/// Elements whose start ends an open `<p>` (HTML's "close a p element").
const ENDS_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dd",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "summary",
    "table",
    "ul",
    "xmp",
];

/// Elements which an implied end can't reach past, ex: an `<li>` doesn't end
/// an `<li>` outside of the list it's in.
const SCOPES: &[&str] = &[
    "ol", "ul", "dl", "table", "td", "th", "caption", "button", "object", "template", "html",
    "body",
];

/// The open elements (by name) which a start tag ends, if they're open in its
/// scope.
fn implied_ends(name: &str) -> &'static [&'static str] {
    match name {
        "li" => &["li"],
        "dt" | "dd" => &["dt", "dd"],
        "tr" => &["tr", "td", "th"],
        "td" | "th" => &["td", "th"],
        "thead" | "tbody" | "tfoot" => &["thead", "tbody", "tfoot", "tr", "td", "th"],
        "option" => &["option"],
        _ => &[],
    }
}

#[derive(Debug, PartialEq)]
pub enum NodeKind {
    Root,
    /// An element, by its (lowercase) name.
    Element(String),
    Text,
    Comment,
}

#[derive(Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// For elements, their start tag's item, and for text and comments, their
    /// item.
    pub item: usize,
    /// For elements, their end tag's item, if they have one.
    pub end_item: Option<usize>,
    /// The bytes of the source it spans (for an element without an end tag,
    /// up to the end of its last descendant).
    pub range: Range<usize>,
}

/// A spec source's document tree.  Node 0 is the root.
pub struct Doc<'a> {
    pub src: &'a str,
    pub(crate) items: Vec<Item>,
    pub nodes: Vec<Node>,
}

impl<'a> Doc<'a> {
    pub fn parse(src: &'a str) -> Doc<'a> {
        Doc::from_items(src, lex(src, false))
    }

    /// The tree of `items`, `src`'s markup (ex: a Bikeshed source's, see
    /// `spec_tokenizer::bikeshed_contexts`).
    pub(crate) fn from_items(src: &'a str, items: Vec<Item>) -> Doc<'a> {
        let mut nodes = vec![Node {
            kind: NodeKind::Root,
            parent: None,
            children: vec![],
            item: 0,
            end_item: None,
            range: 0..src.len(),
        }];
        // The open elements, innermost last, as node indexes.
        let mut open: Vec<usize> = vec![0];
        let name_of = |nodes: &[Node], node: usize| -> Option<String> {
            match &nodes[node].kind {
                NodeKind::Element(name) => Some(name.clone()),
                _ => None,
            }
        };
        // Close the open elements down to (and including) `open[depth]`.
        let close_to = |nodes: &mut Vec<Node>, open: &mut Vec<usize>, depth: usize| {
            while open.len() > depth {
                let node = open.pop().unwrap();
                let end = nodes[node]
                    .children
                    .last()
                    .map_or(nodes[node].range.end, |&last| nodes[last].range.end);
                nodes[node].range.end = nodes[node].range.end.max(end);
            }
        };
        for (index, item) in items.iter().enumerate() {
            match item {
                Item::Start { name, range, attrs } => {
                    // Implied ends: ex: an open `<li>` (and whatever's open in
                    // it, like a `<p>`) for a new one, and an open `<p>`.
                    let ends_p: &[&str] = if ENDS_P.contains(&name.as_str()) {
                        &["p"]
                    } else {
                        &[]
                    };
                    for ends in [implied_ends(name), ends_p] {
                        // (The elements it ends aren't the scope's edge, ex: a
                        // `<td>` ends an open `<td>`.  A table's part ends the
                        // outermost of them in the scope, ex: a `<tr>` the
                        // open `<tr>`, with the `<td>` open in it.)
                        let outermost = ends.contains(&"td");
                        let mut found = None;
                        for (depth, &node) in open.iter().enumerate().rev() {
                            let Some(open_name) = name_of(&nodes, node) else {
                                break;
                            };
                            if ends.contains(&open_name.as_str()) {
                                found = Some(depth);
                                if !outermost {
                                    break;
                                }
                            } else if SCOPES.contains(&open_name.as_str()) {
                                break;
                            }
                        }
                        let depth = found;
                        if let Some(depth) = depth {
                            close_to(&mut nodes, &mut open, depth);
                        }
                    }
                    let parent = *open.last().unwrap();
                    let node = nodes.len();
                    nodes.push(Node {
                        kind: NodeKind::Element(name.clone()),
                        parent: Some(parent),
                        children: vec![],
                        item: index,
                        end_item: None,
                        range: range.clone(),
                    });
                    nodes[parent].children.push(node);
                    // (But in `<a for=/>`, the slash is the attribute's value.)
                    let self_closing = src[range.clone()].ends_with("/>")
                        && !attrs.last().is_some_and(|(_, value)| {
                            value
                                .as_ref()
                                .is_some_and(|value| value.end == range.end - 1)
                        });
                    if !VOID.contains(&name.as_str()) && !self_closing {
                        open.push(node);
                    }
                }
                Item::End { name, range } => {
                    let depth = open
                        .iter()
                        .rposition(|&node| name_of(&nodes, node).as_deref() == Some(name.as_str()));
                    if let Some(depth) = depth.filter(|&depth| depth > 0) {
                        let node = open[depth];
                        close_to(&mut nodes, &mut open, depth);
                        nodes[node].end_item = Some(index);
                        nodes[node].range.end = range.end;
                    }
                }
                Item::Text(range) | Item::Comment(range) => {
                    let parent = *open.last().unwrap();
                    let node = nodes.len();
                    let kind = match item {
                        Item::Text(_) => NodeKind::Text,
                        _ => NodeKind::Comment,
                    };
                    nodes.push(Node {
                        kind,
                        parent: Some(parent),
                        children: vec![],
                        item: index,
                        end_item: None,
                        range: range.clone(),
                    });
                    nodes[parent].children.push(node);
                }
                Item::Declaration(_) => {}
            }
        }
        close_to(&mut nodes, &mut open, 1);
        Doc { src, items, nodes }
    }

    /// The element's name, if it's an element.
    pub fn name(&self, node: usize) -> Option<&str> {
        match &self.nodes[node].kind {
            NodeKind::Element(name) => Some(name),
            _ => None,
        }
    }

    pub fn is(&self, node: usize, name: &str) -> bool {
        self.name(node) == Some(name)
    }

    /// The element's attribute's raw value (`Some("")` for an attribute
    /// without one).
    pub fn raw_attr(&self, node: usize, name: &str) -> Option<&'a str> {
        match self.nodes[node].kind {
            NodeKind::Element(_) => attr(self.src, &self.items[self.nodes[node].item], name),
            _ => None,
        }
    }

    pub fn has_attr(&self, node: usize, name: &str) -> bool {
        self.raw_attr(node, name).is_some()
    }

    /// The element's attributes, as their names (as written) and their
    /// values (with their character references decoded), in order.
    pub fn attrs(&self, node: usize) -> Vec<(&'a str, Option<String>)> {
        let Item::Start { attrs, .. } = &self.items[self.nodes[node].item] else {
            return vec![];
        };
        attrs
            .iter()
            .map(|(name, value)| {
                let name = self.src[name.clone()]
                    .trim_end()
                    .trim_end_matches('=')
                    .trim_end();
                (
                    name,
                    value.as_ref().map(|v| decode_html(&self.src[v.clone()])),
                )
            })
            .collect()
    }

    /// The element's attribute's value, with its character references
    /// decoded.
    pub fn attr(&self, node: usize, name: &str) -> Option<String> {
        self.raw_attr(node, name).map(decode_html)
    }

    /// The bytes between the element's start tag and its end tag (or its
    /// end, if it has none).
    pub fn content_range(&self, node: usize) -> Range<usize> {
        let n = &self.nodes[node];
        let start = self.items[n.item].range().end;
        let end = n
            .end_item
            .map_or(n.range.end, |end| self.items[end].range().start);
        start..end.max(start)
    }

    /// The node's text, with character references decoded, as the DOM's
    /// `textContent`.
    pub fn text_content(&self, node: usize) -> String {
        let mut raw = String::new();
        self.raw_text(node, &mut raw);
        decode_html(&raw)
    }

    fn raw_text(&self, node: usize, out: &mut String) {
        match self.nodes[node].kind {
            NodeKind::Text => out.push_str(&self.src[self.nodes[node].range.clone()]),
            NodeKind::Comment => {}
            NodeKind::Root | NodeKind::Element(_) => {
                for &child in &self.nodes[node].children {
                    self.raw_text(child, out);
                }
            }
        }
    }

    /// The node's ancestors, innermost first, not including the root.
    pub fn ancestors(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.nodes[node].parent, |&n| self.nodes[n].parent)
            .take_while(|&n| n != 0)
    }
}

lazy_static! {
    static ref CHAR_REF: Regex =
        Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);").unwrap();
}

/// The named character references which specs' sources use, and their
/// characters.  (Others are left as they are.)
const NAMED_REFS: &[(&str, char)] = &[
    ("AElig", 'Æ'),
    ("Alpha", 'Α'),
    ("Beta", 'Β'),
    ("Ccedil", 'Ç'),
    ("Delta", 'Δ'),
    ("Eacute", 'É'),
    ("Gamma", 'Γ'),
    ("Oslash", 'Ø'),
    ("Prime", '″'),
    ("aacute", 'á'),
    ("aelig", 'æ'),
    ("agrave", 'à'),
    ("amp", '&'),
    ("apos", '\''),
    ("aring", 'å'),
    ("atilde", 'ã'),
    ("auml", 'ä'),
    ("ccedil", 'ç'),
    ("copy", '©'),
    ("dagger", '†'),
    ("darr", '↓'),
    ("deg", '°'),
    ("eacute", 'é'),
    ("egrave", 'è'),
    ("euml", 'ë'),
    ("euro", '€'),
    ("grave", '`'),
    ("gt", '>'),
    ("hellip", '…'),
    ("iacute", 'í'),
    ("infin", '∞'),
    ("isin", '∈'),
    ("iuml", 'ï'),
    ("larr", '←'),
    ("ldquo", '“'),
    ("le", '≤'),
    ("ge", '≥'),
    ("lsquo", '‘'),
    ("lt", '<'),
    ("mdash", '—'),
    ("middot", '·'),
    ("minus", '−'),
    ("mldr", '…'),
    ("nbsp", '\u{a0}'),
    ("ndash", '–'),
    ("not", '¬'),
    ("notin", '∉'),
    ("ntilde", 'ñ'),
    ("oacute", 'ó'),
    ("oelig", 'œ'),
    ("omega", 'ω'),
    ("oslash", 'ø'),
    ("ouml", 'ö'),
    ("pi", 'π'),
    ("pound", '£'),
    ("quot", '"'),
    ("rarr", '→'),
    ("rdquo", '”'),
    ("reg", '®'),
    ("rsquo", '’'),
    ("sbquo", '‚'),
    ("sigma", 'σ'),
    ("sup1", '¹'),
    ("sup2", '²'),
    ("sup3", '³'),
    ("thinsp", '\u{2009}'),
    ("times", '×'),
    ("trade", '™'),
    ("uacute", 'ú'),
    ("uarr", '↑'),
    ("uuml", 'ü'),
];

/// The pieces of `src[range]` (ex: a text node's), split at its character
/// references, as their ranges and their decoded text: verbatim text, and
/// each character reference.
pub fn text_pieces(src: &str, range: std::ops::Range<usize>) -> Vec<(Range<usize>, String)> {
    let text = &src[range.clone()];
    if !text.contains('&') {
        return vec![(range, text.to_string())];
    }
    let mut pieces = vec![];
    let mut pos = 0;
    for m in CHAR_REF.find_iter(text) {
        let decoded = decode_html(m.as_str());
        // (A reference we don't know stays verbatim text.)
        if decoded == m.as_str() {
            continue;
        }
        if pos < m.start() {
            pieces.push((
                range.start + pos..range.start + m.start(),
                text[pos..m.start()].to_string(),
            ));
        }
        pieces.push((range.start + m.start()..range.start + m.end(), decoded));
        pos = m.end();
    }
    if pos < text.len() {
        pieces.push((range.start + pos..range.end, text[pos..].to_string()));
    }
    pieces
}

/// `text` with its character references decoded, as an HTML parser would
/// (but for the named ones specs don't use).
pub fn decode_html(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    CHAR_REF
        .replace_all(text, |captures: &regex::Captures| {
            let name = &captures[1];
            let c = if let Some(hex) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(decimal) = name.strip_prefix('#') {
                decimal.parse().ok().and_then(char::from_u32)
            } else {
                NAMED_REFS
                    .iter()
                    .find(|(named, _)| *named == name)
                    .map(|&(_, c)| c)
            };
            c.map_or_else(|| captures[0].to_string(), |c| c.to_string())
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree as `name(children)`, with text as `"..."`.
    fn shape(doc: &Doc, node: usize) -> String {
        let n = &doc.nodes[node];
        match &n.kind {
            NodeKind::Text => format!("{:?}", doc.src[n.range.clone()].trim()),
            NodeKind::Comment => "comment".to_string(),
            NodeKind::Root | NodeKind::Element(_) => {
                let children: Vec<String> = n
                    .children
                    .iter()
                    .map(|&c| shape(doc, c))
                    .filter(|s| s != "\"\"")
                    .collect();
                let name = doc.name(node).unwrap_or("root");
                format!("{}({})", name, children.join(" "))
            }
        }
    }

    #[test]
    fn test_implied_ends() {
        let doc = Doc::parse("<p>One<p>Two <b>x</b><ol><li>A<li>B<ol><li>C</ol><li>D</ol>after");
        assert_eq!(
            shape(&doc, 0),
            r#"root(p("One") p("Two" b("x")) ol(li("A") li("B" ol(li("C"))) li("D")) "after")"#
        );
        // (A new `<li>` ends the last one, with the `<p>` open in it.)
        let doc = Doc::parse("<ol><li><p>A<li><p>B<ol><li>C</ol></ol>");
        assert_eq!(
            shape(&doc, 0),
            r#"root(ol(li(p("A")) li(p("B") ol(li("C")))))"#
        );
        let doc = Doc::parse("<dl><dt>T<dd>D<dt>U</dl><br><img src=x>z<x/>y");
        assert_eq!(
            shape(&doc, 0),
            r#"root(dl(dt("T") dd("D") dt("U")) br() img() "z" x() "y")"#
        );
        // (Bikeshed's `for=/` isn't a self-closing tag.)
        let doc = Doc::parse("<a for=/>node</a> and <a for='/'/>");
        assert_eq!(shape(&doc, 0), r#"root(a("node") "and" a())"#);
        // (A stray end tag is ignored.)
        let doc = Doc::parse("<div>a</span>b</div>");
        assert_eq!(shape(&doc, 0), r#"root(div("a" "b"))"#);
        // (Cells end cells, and rows rows, but not outside their tables.)
        let doc =
            Doc::parse("<table><tr><td>A<td>B<table><tr><td>C<tr><td>D</table><tr><td>E</table>");
        assert_eq!(
            shape(&doc, 0),
            r#"root(table(tr(td("A") td("B" table(tr(td("C")) tr(td("D"))))) tr(td("E"))))"#
        );
    }

    #[test]
    fn test_ranges_and_text() {
        let src = "<p>A <dfn data-x=\"x&amp;y\">B&nbsp;&lt;c></dfn><!-- no --> d<p>e";
        let doc = Doc::parse(src);
        let p = doc.nodes[0].children[0];
        let dfn = doc.nodes[p].children[1];
        assert_eq!(doc.name(dfn), Some("dfn"));
        assert_eq!(doc.attr(dfn, "data-x").as_deref(), Some("x&y"));
        assert_eq!(doc.text_content(dfn), "B\u{a0}<c>");
        assert_eq!(&src[doc.content_range(dfn)], "B&nbsp;&lt;c>");
        assert_eq!(doc.text_content(p), "A B\u{a0}<c> d");
        // (The first `<p>` ends where the second starts.)
        assert_eq!(&src[doc.nodes[p].range.clone()], &src[..src.len() - 4]);
        assert_eq!(doc.ancestors(dfn).collect::<Vec<_>>(), vec![p]);
    }
}
