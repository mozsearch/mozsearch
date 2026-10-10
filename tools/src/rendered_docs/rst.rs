//! Rendering reStructuredText, with tree-sitter-rst's trees, whose sections
//! are flat (a title and its adornments, with the section's content after
//! it), and whose directives' content isn't parsed (so we parse it as a
//! document of its own, without its indentation).  See the module's comment
//! in `mod.rs` for blocks and runs.

use std::fmt::Write;
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;
use tree_sitter::{Node, Parser, Tree};

use super::doc::{DirectiveContent, Doc, Flavor, Substitution, title_target};
use super::{Links, OffsetMap, Rendered, Writer, escape, link_url, slugify};

lazy_static! {
    /// The first line of a grid table, or the border of a simple table.
    static ref TABLE_BORDER: Regex = Regex::new(r"^\s*(\+[-=+]+\+|=+(\s+=+)+)\s*$").unwrap();
    static ref MARKUP: Regex = Regex::new(r"``|\*\*|[*`|]|_+\b|\b_+").unwrap();
}

/// Render the reStructuredText document `src`, which is at `path`.
pub fn render(src: &str, path: &str, links: &dyn Links) -> Rendered {
    let mut w = Writer::new(src);
    let mut doc = Doc::new(&mut w, links, path, Flavor::Rst);
    doc.rst_document(src, &OffsetMap::identity(src.len()), true);
    w.finish()
}

fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rst::LANGUAGE.into())
        .ok()?;
    parser.parse(text, None)
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    named_children(node).into_iter().find(|c| c.kind() == kind)
}

fn node_text<'t>(text: &'t str, node: Node) -> &'t str {
    text.get(node.byte_range()).unwrap_or("")
}

/// Text without inline markup, for headings' ids.
fn plain(text: &str) -> String {
    MARKUP.replace_all(text, "").trim().to_string()
}

/// The lines of `text[range]` as pieces without up to `indent` columns of
/// indentation (the first line starts at the range's start).
fn dedent(text: &str, range: Range<usize>, indent: usize) -> Vec<Range<usize>> {
    let mut pieces = vec![];
    let mut pos = range.start;
    let Some(slice) = text.get(range.clone()) else {
        return pieces;
    };
    for (i, line) in slice.split_inclusive('\n').enumerate() {
        let strip = if i == 0 {
            0
        } else {
            line.len() - line.trim_start_matches(' ').len()
        }
        .min(indent);
        pieces.push(pos + strip..pos + line.len());
        pos += line.len();
    }
    pieces
}

/// The smallest indentation of the lines after the first of `text[range]`
/// that aren't blank.
fn min_indent(text: &str, range: Range<usize>) -> usize {
    text.get(range)
        .unwrap_or("")
        .split_inclusive('\n')
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0)
}

impl Doc<'_, '_, '_> {
    /// Render reStructuredText (ex: the content of a directive, or of MyST's
    /// `eval-rst`), whose text is `text`, which `map` maps to the file.
    pub(super) fn rst_nested(&mut self, text: &str, map: &OffsetMap) {
        self.rst_document(text, map, false);
    }

    fn rst_document(&mut self, text: &str, map: &OffsetMap, top: bool) {
        let Some(tree) = parse(text) else {
            let lines: Vec<(&str, Option<Range<usize>>)> = dedent(text, 0..text.len(), 0)
                .into_iter()
                .map(|range| (&text[range.clone()], map.exact(range)))
                .collect();
            self.code(map.span(0..text.len()), "", None, &lines);
            return;
        };
        let root = tree.root_node();
        if top {
            let mut pending = vec![];
            self.rst_collect(root, text, map, &mut pending);
        }
        self.rst_children(root, text, map);
    }

    /// Learn the document's labels, link targets, and substitutions, and give
    /// its sections ids, before rendering it.
    fn rst_collect(&mut self, node: Node, text: &str, map: &OffsetMap, pending: &mut Vec<String>) {
        for child in named_children(node) {
            match child.kind() {
                "target" => {
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| node_text(text, n))
                        .unwrap_or("");
                    let name = name.trim_start_matches('_').trim_end_matches(':').trim();
                    let name = name.trim_matches('`');
                    match child.child_by_field_name("link") {
                        Some(link) => {
                            self.link_targets.insert(
                                name.to_lowercase(),
                                node_text(text, link).trim().to_string(),
                            );
                        }
                        None if !name.is_empty() => {
                            self.add_target(name);
                            pending.push(name.to_string());
                        }
                        None => {}
                    }
                }
                "substitution_definition" => {
                    let name = child_of_kind(child, "substitution")
                        .map(|n| node_text(text, n).trim_matches('|').to_string());
                    let directive = child_of_kind(child, "directive");
                    if let (Some(name), Some(directive)) = (name, directive) {
                        let kind = child_of_kind(directive, "type")
                            .map(|t| node_text(text, t))
                            .unwrap_or("");
                        let args = directive
                            .child_by_field_name("body")
                            .and_then(|body| child_of_kind(body, "arguments"))
                            .map(|a| node_text(text, a).trim().to_string())
                            .unwrap_or_default();
                        let substitution = match kind {
                            "image" => Substitution::Image(args),
                            _ => Substitution::Text(args),
                        };
                        self.substitutions.insert(name, substitution);
                    }
                }
                "section" => {
                    let title = child_of_kind(child, "title")
                        .map(|t| plain(node_text(text, t)))
                        .unwrap_or_default();
                    let labels = std::mem::take(pending);
                    self.add_heading(map.real(child.start_byte()), &title, &labels);
                }
                "comment" => {}
                _ => {
                    pending.clear();
                    self.rst_collect(child, text, map, pending);
                }
            }
        }
    }

    fn rst_children(&mut self, node: Node, text: &str, map: &OffsetMap) {
        for child in named_children(node) {
            self.rst_block(child, text, map);
        }
    }

    fn rst_block(&mut self, node: Node, text: &str, map: &OffsetMap) {
        let real = map.span(node.byte_range());
        match node.kind() {
            "section" => {
                let parts = children(node);
                let title_index = parts.iter().position(|c| c.kind() == "title");
                let overline = title_index.is_some_and(|i| i > 0);
                let style_char = parts
                    .iter()
                    .rev()
                    .find(|c| c.kind() == "adornment")
                    .and_then(|c| node_text(text, *c).chars().next())
                    .unwrap_or('=');
                let style = (style_char, overline);
                let level = match self.styles.iter().position(|s| *s == style) {
                    Some(i) => i + 1,
                    None => {
                        self.styles.push(style);
                        self.styles.len()
                    }
                };
                let level = level.min(6);
                let title = title_index.map(|i| parts[i]);
                let title_text = title.map(|t| plain(node_text(text, t))).unwrap_or_default();
                let id = self.heading_id(real.start, &title_text);
                if self.w.title.is_none() {
                    self.w.title = Some(title_text);
                }
                let opened = self.w.open_block_div(real, "");
                write!(self.w.html, r#"<h{} id="{}">"#, level, escape(&id)).unwrap();
                if let Some(title) = title {
                    self.rst_inline(title, text, map, None);
                }
                write!(self.w.html, "</h{}>", level).unwrap();
                self.w.close_block_div(opened);
            }
            "paragraph" => self.rst_paragraph(node, text, map),
            "bullet_list" | "enumerated_list" => {
                let ordered = node.kind() == "enumerated_list";
                if ordered {
                    let start = named_children(node)
                        .first()
                        .and_then(|item| item.child(0))
                        .map(|bullet| node_text(text, bullet))
                        .and_then(|bullet| {
                            bullet
                                .trim_matches(|c: char| !c.is_ascii_digit())
                                .parse::<u64>()
                                .ok()
                        });
                    match start {
                        Some(n) if n != 1 => write!(self.w.html, r#"<ol start="{}">"#, n).unwrap(),
                        _ => self.w.push("<ol>"),
                    }
                } else {
                    self.w.push("<ul>");
                }
                for item in named_children(node) {
                    self.w.push("<li>");
                    for child in named_children(item) {
                        if child.kind() == "body" {
                            self.rst_children(child, text, map);
                        } else {
                            self.rst_block(child, text, map);
                        }
                    }
                    self.w.push("</li>");
                }
                self.w.push(if ordered { "</ol>" } else { "</ul>" });
            }
            "definition_list" => {
                self.w.push("<dl>");
                for item in named_children(node) {
                    let parts = named_children(item);
                    let term_end = parts
                        .iter()
                        .filter(|c| matches!(c.kind(), "term" | "classifier"))
                        .map(|c| c.end_byte())
                        .max();
                    if let Some(term) = parts.iter().find(|c| c.kind() == "term") {
                        let term_real =
                            map.span(term.start_byte()..term_end.unwrap_or(term.end_byte()));
                        let opened = self.w.open_block_div(term_real, "rd-dt");
                        self.w.push("<dt>");
                        self.rst_inline(*term, text, map, None);
                        for classifier in parts.iter().filter(|c| c.kind() == "classifier") {
                            self.w.push(r#" : <span class="rd-classifier">"#);
                            self.rst_inline(*classifier, text, map, None);
                            self.w.push("</span>");
                        }
                        self.w.push("</dt>");
                        self.w.close_block_div(opened);
                    }
                    for definition in parts.iter().filter(|c| c.kind() == "definition") {
                        self.w.push("<dd>");
                        self.rst_children(*definition, text, map);
                        self.w.push("</dd>");
                    }
                }
                self.w.push("</dl>");
            }
            "field_list" | "options" => {
                self.w.push(r#"<dl class="rd-fields">"#);
                for field in named_children(node) {
                    self.rst_field(field, text, map);
                }
                self.w.push("</dl>");
            }
            "block_quote" => {
                self.w.push("<blockquote>");
                self.rst_children(node, text, map);
                self.w.push("</blockquote>");
            }
            "attribution" => {
                let opened = self.w.open_block_div(real, "");
                self.w.push(r#"<p class="rd-attribution">— "#);
                self.rst_inline(node, text, map, None);
                self.w.push("</p>");
                self.w.close_block_div(opened);
            }
            "line_block" => {
                let opened = self.w.open_block_div(real, "rd-line-block");
                self.w.push("<p>");
                for line in named_children(node) {
                    self.rst_inline(line, text, map, None);
                    self.w.push("<br>");
                }
                self.w.push("</p>");
                self.w.close_block_div(opened);
            }
            "literal_block" | "doctest_block" => self.rst_literal(node, text, map),
            "transition" => {
                let opened = self.w.open_block_div(real, "rd-rule");
                self.w.push("<hr>");
                self.w.close_block_div(opened);
            }
            "comment" | "substitution_definition" => {}
            "target" => {
                if node.child_by_field_name("link").is_none() {
                    let name = node
                        .child_by_field_name("name")
                        .map(|n| node_text(text, n))
                        .unwrap_or("");
                    let name = name.trim_start_matches('_').trim_end_matches(':').trim();
                    let name = name.trim_matches('`');
                    if !name.is_empty() {
                        self.target_anchor(name);
                    }
                }
            }
            "footnote" | "citation" => {
                let label = child_of_kind(node, "label")
                    .map(|l| node_text(text, l).trim_matches(['[', ']']).to_string())
                    .unwrap_or_default();
                write!(
                    self.w.html,
                    r#"<div class="rd-footnote" id="fn-{}"><span class="rd-footnote-label">{}</span>"#,
                    escape(&slugify(&label)),
                    escape(&label)
                )
                .unwrap();
                if let Some(body) = node.child_by_field_name("body") {
                    self.rst_children(body, text, map);
                }
                self.w.push("</div>");
            }
            "directive" => self.rst_directive(node, text, map),
            _ if node.named_child_count() == 0 => {
                // Something we don't know (ex: an error), as text.
                let opened = self.w.open_block_div(real, "");
                self.w.push("<p>");
                self.rst_text(text, node.byte_range(), map);
                self.w.push("</p>");
                self.w.close_block_div(opened);
            }
            _ => self.rst_children(node, text, map),
        }
    }

    fn rst_field(&mut self, field: Node, text: &str, map: &OffsetMap) {
        let name = child_of_kind(field, "field_name");
        let body = child_of_kind(field, "field_body");
        let body_parts = body.map(named_children).unwrap_or_default();
        // A field whose value is a paragraph on its line is one block.
        let one_line = match (name, body_parts.as_slice()) {
            (Some(name), [paragraph]) => {
                paragraph.kind() == "paragraph"
                    && paragraph.start_position().row == name.start_position().row
                    && paragraph.end_position().row == paragraph.start_position().row
            }
            (_, []) => true,
            _ => false,
        };
        let name_real = name.map(|name| map.span(name.byte_range()));
        let block_real = if one_line {
            map.span(field.byte_range())
        } else {
            name_real
                .clone()
                .unwrap_or_else(|| map.span(field.byte_range()))
        };
        let opened = self.w.open_block_div(block_real, "rd-dt");
        self.w.push("<dt>");
        if let Some(name) = name {
            self.rst_inline(name, text, map, None);
        }
        self.w.push("</dt>");
        if one_line {
            self.w.push("<dd>");
            if let Some(paragraph) = body_parts.first() {
                self.rst_inline(*paragraph, text, map, None);
            }
            self.w.push("</dd>");
            self.w.close_block_div(opened);
        } else {
            self.w.close_block_div(opened);
            self.w.push("<dd>");
            for part in body_parts {
                self.rst_block(part, text, map);
            }
            self.w.push("</dd>");
        }
    }

    fn rst_paragraph(&mut self, node: Node, text: &str, map: &OffsetMap) {
        let node_str = node_text(text, node);
        // Tables (which tree-sitter-rst doesn't parse) as they are.
        if node_str
            .lines()
            .next()
            .is_some_and(|line| TABLE_BORDER.is_match(line))
        {
            self.rst_literal(node, text, map);
            return;
        }
        let literal = child_of_kind(node, "literal_block");
        let marker = children(node).into_iter().find(|c| c.kind() == "::");
        let end = match (marker, literal) {
            (Some(marker), _) => marker.end_byte(),
            (None, Some(literal)) => literal.start_byte(),
            (None, None) => node.end_byte(),
        };
        // ("::" alone is just the literal block's marker.)
        let only_marker =
            marker.is_some_and(|m| text[node.start_byte()..m.start_byte()].trim().is_empty());
        if !only_marker {
            let opened = self.w.open_block_div(map.span(node.start_byte()..end), "");
            self.w.push("<p>");
            self.rst_inline(node, text, map, Some(end));
            self.w.push("</p>");
            self.w.close_block_div(opened);
        }
        if let Some(literal) = literal {
            self.rst_literal(literal, text, map);
        }
    }

    /// A literal block, or something else to show as is.
    fn rst_literal(&mut self, node: Node, text: &str, map: &OffsetMap) {
        let range = node.byte_range();
        let indent = node.start_position().column;
        let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let indent = indent.min(range.start - line_start);
        let lines: Vec<(&str, Option<Range<usize>>)> = dedent(text, range.clone(), indent)
            .into_iter()
            .map(|piece| (&text[piece.clone()], map.exact(piece)))
            .filter(|(line, _)| !line.is_empty())
            .collect();
        self.code(map.span(range), "", None, &lines);
    }

    /// The text of `text[range]`, as runs for each of its lines.
    fn rst_text(&mut self, text: &str, range: Range<usize>, map: &OffsetMap) {
        let Some(slice) = text.get(range.clone()) else {
            return;
        };
        let mut pos = range.start;
        for piece in slice.split_inclusive('\n') {
            let line = piece.trim_end_matches('\n');
            self.w.text(line, map.exact(pos..pos + line.len()));
            self.w.push(&piece[line.len()..]);
            pos += piece.len();
        }
    }

    /// The inline content of `node` (up to `end`, if given).
    fn rst_inline(&mut self, node: Node, text: &str, map: &OffsetMap, end: Option<usize>) {
        let end = end.unwrap_or(node.end_byte());
        let mut pos = node.start_byte();
        for child in children(node) {
            if child.start_byte() >= end || child.start_byte() < pos {
                continue;
            }
            if !child.is_named() {
                if child.kind() == "::" {
                    // "text::" is "text:", and "text ::" is "text".
                    self.rst_text(text, pos..child.start_byte(), map);
                    let before = text[..child.start_byte()].chars().next_back();
                    if before.is_some_and(|c| !c.is_whitespace()) {
                        self.w.push(":");
                    }
                    pos = child.end_byte();
                }
                continue;
            }
            self.rst_text(text, pos..child.start_byte(), map);
            self.rst_markup(child, text, map);
            pos = child.end_byte();
        }
        if pos < end {
            self.rst_text(text, pos..end, map);
        }
    }

    fn rst_markup(&mut self, node: Node, text: &str, map: &OffsetMap) {
        let range = node.byte_range();
        let inner =
            |n: usize| (range.end >= range.start + 2 * n).then(|| range.start + n..range.end - n);
        match node.kind() {
            "emphasis" | "strong" | "literal" => {
                let (n, tag) = match node.kind() {
                    "emphasis" => (1, "em"),
                    "strong" => (2, "strong"),
                    _ => (2, "code"),
                };
                write!(self.w.html, "<{}>", tag).unwrap();
                match inner(n) {
                    Some(inner) => self.rst_text(text, inner, map),
                    None => self.rst_text(text, range, map),
                }
                write!(self.w.html, "</{}>", tag).unwrap();
            }
            "interpreted_text" => {
                let parts = children(node);
                let role_before = parts
                    .iter()
                    .find(|c| c.kind() == "role" && c.start_byte() == range.start);
                let role_after = parts
                    .iter()
                    .find(|c| c.kind() == "role" && c.end_byte() == range.end);
                let start = role_before.map_or(range.start, |r| r.end_byte());
                let end = role_after.map_or(range.end, |r| r.start_byte());
                if end < start + 2 {
                    self.rst_text(text, range, map);
                    return;
                }
                let code_range = start + 1..end - 1;
                let code = &text[code_range.clone()];
                let role = role_before
                    .or(role_after)
                    .map(|r| node_text(text, *r).trim_matches(':'));
                match role {
                    Some(role) => self.role(role, code, map.exact(code_range)),
                    None => {
                        self.w.push("<cite>");
                        self.rst_text(text, code_range, map);
                        self.w.push("</cite>");
                    }
                }
            }
            "reference" => self.rst_reference(node, text, map),
            "standalone_hyperlink" => {
                let url = node_text(text, node);
                match link_url(self.links, &self.dir, url) {
                    Some(url) => write!(self.w.html, r#"<a href="{}">"#, escape(&url)).unwrap(),
                    None => self.w.push("<a>"),
                }
                self.rst_text(text, range, map);
                self.w.push("</a>");
            }
            "footnote_reference" | "citation_reference" => {
                let label = node_text(text, node)
                    .trim_end_matches('_')
                    .trim_matches(['[', ']']);
                write!(
                    self.w.html,
                    r##"<sup class="rd-footnote-ref"><a href="#fn-{}">{}</a></sup>"##,
                    escape(&slugify(label)),
                    escape(label)
                )
                .unwrap();
            }
            "substitution_reference" => {
                let name = node_text(text, node)
                    .trim_end_matches('_')
                    .trim_matches('|');
                match self.substitutions.get(name) {
                    Some(Substitution::Text(replacement)) => {
                        let replacement = replacement.clone();
                        self.w.escaped(&replacement);
                    }
                    Some(Substitution::Image(src)) => {
                        let src = src.clone();
                        self.image(&src, name, "", None);
                    }
                    None => self.rst_text(text, range, map),
                }
            }
            "inline_target" => {
                // _`name`
                let name_range =
                    (range.end >= range.start + 3).then(|| range.start + 2..range.end - 1);
                let name = name_range.clone().map_or("", |r| &text[r]);
                write!(
                    self.w.html,
                    r#"<span class="rd-target" id="{}">"#,
                    escape(&slugify(name))
                )
                .unwrap();
                match name_range {
                    Some(name_range) => self.rst_text(text, name_range, map),
                    None => self.rst_text(text, range, map),
                }
                self.w.push("</span>");
            }
            _ => self.rst_text(text, range, map),
        }
    }

    /// `` `title <target>`_ ``, `` `name`_ ``, or `name_` (and with `__`).
    fn rst_reference(&mut self, node: Node, text: &str, map: &OffsetMap) {
        let range = node.byte_range();
        let s = node_text(text, node);
        let underscores = s.len() - s.trim_end_matches('_').len();
        let body_end = range.end - underscores;
        let (code_range, quoted) = if s.starts_with('`') && body_end >= range.start + 2 {
            (range.start + 1..body_end - 1, true)
        } else {
            (range.start..body_end, false)
        };
        let code = &text[code_range.clone()];
        let real = map.exact(code_range);
        let (title, target) = if quoted && code.trim_end().ends_with('>') {
            title_target(code)
        } else {
            (None, 0..code.len())
        };
        let embedded = quoted && code.trim_end().ends_with('>');
        let target_text = code[target.clone()].trim();
        let named = |doc: &Self, name: &str| -> Option<String> {
            match doc.link_targets.get(&name.to_lowercase()) {
                Some(url) => link_url(doc.links, &doc.dir, url),
                None => doc.find_label(name).map(|(id, _)| format!("#{}", id)),
            }
        };
        let url = if embedded && !target_text.ends_with('_') {
            link_url(self.links, &self.dir, target_text)
        } else {
            named(self, target_text.trim_end_matches('_'))
        };
        match url {
            Some(url) => write!(self.w.html, r#"<a href="{}">"#, escape(&url)).unwrap(),
            None => self.w.push(r#"<a class="rd-unresolved">"#),
        }
        self.role_text(code, &real, &title, target);
        self.w.push("</a>");
    }

    fn rst_directive(&mut self, node: Node, text: &str, map: &OffsetMap) {
        let real = map.span(node.byte_range());
        let name = node
            .child_by_field_name("name")
            .or_else(|| child_of_kind(node, "type"))
            .map(|n| node_text(text, n).trim().to_string())
            .unwrap_or_default();
        let body = node.child_by_field_name("body");
        let part = |kind: &str| body.and_then(|body| child_of_kind(body, kind));
        let arguments = part("arguments");
        let content = part("content");
        let options: Vec<(String, String)> = part("options")
            .map(|options| {
                named_children(options)
                    .into_iter()
                    .map(|field| {
                        let name = child_of_kind(field, "field_name")
                            .map(|n| node_text(text, n).trim().to_string())
                            .unwrap_or_default();
                        let value = child_of_kind(field, "field_body")
                            .map(|b| node_text(text, b).trim().to_string())
                            .unwrap_or_default();
                        (name, value)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Admonitions' arguments are the start of their content.
        let lowercase = name.to_ascii_lowercase();
        let args_are_content = matches!(
            lowercase.as_str(),
            "note"
                | "warning"
                | "important"
                | "tip"
                | "hint"
                | "caution"
                | "attention"
                | "danger"
                | "error"
                | "seealso"
                | "todo"
        );
        let (arg, arg_real) = match arguments {
            Some(arguments) if !args_are_content => {
                let raw = node_text(text, arguments);
                let trimmed = raw.trim();
                let start = arguments.start_byte() + (raw.len() - raw.trim_start().len());
                (trimmed.to_string(), map.exact(start..start + trimmed.len()))
            }
            _ => (String::new(), None),
        };
        let content_start = match (arguments, content) {
            (Some(arguments), _) if args_are_content => Some(arguments.start_byte()),
            (_, Some(content)) => Some(content.start_byte()),
            _ => None,
        };
        let content_end = content
            .map(|c| c.end_byte())
            .or(arguments.filter(|_| args_are_content).map(|a| a.end_byte()));
        let pieces = match (content_start, content_end) {
            (Some(start), Some(end)) if start < end => {
                let indent = match content {
                    Some(content) => content.start_position().column,
                    None => min_indent(text, start..end),
                };
                let indent = if args_are_content && arguments.is_some() {
                    min_indent(text, start..end).max(1)
                } else {
                    indent
                };
                dedent(text, start..end, indent)
            }
            _ => vec![],
        };
        let content_text: String = pieces.iter().map(|piece| &text[piece.clone()]).collect();
        let content = DirectiveContent {
            text: content_text,
            map: map.compose(&pieces),
            options,
            body_start: 0,
        };
        self.directive(&name, &arg, arg_real, &content, real);
    }

    /// The content of a `list-table` directive: a list of rows, each a list
    /// of cells.
    pub(super) fn rst_list_table(&mut self, text: &str, map: &OffsetMap, header_rows: usize) {
        let Some(tree) = parse(text) else {
            return;
        };
        let root = tree.root_node();
        let Some(rows) = child_of_kind(root, "bullet_list") else {
            self.rst_children(root, text, map);
            return;
        };
        for (i, row) in named_children(rows).into_iter().enumerate() {
            let tag = if i < header_rows { "th" } else { "td" };
            self.w.push("<tr>");
            let opened = self.w.start_block(map.span(row.byte_range()), false);
            let body = child_of_kind(row, "body");
            let cells = body
                .and_then(|body| child_of_kind(body, "bullet_list"))
                .map(named_children)
                .unwrap_or_default();
            if cells.is_empty() {
                write!(self.w.html, "<{}>", tag).unwrap();
                self.w.place_strip();
                if let Some(body) = body {
                    self.rst_children(body, text, map);
                }
                write!(self.w.html, "</{}>", tag).unwrap();
            }
            for cell in cells {
                write!(self.w.html, "<{}>", tag).unwrap();
                self.w.place_strip();
                if let Some(body) = child_of_kind(cell, "body") {
                    self.rst_children(body, text, map);
                }
                write!(self.w.html, "</{}>", tag).unwrap();
            }
            self.w.push("</tr>");
            if opened {
                self.w.end_block();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::markdown::tests::with_strips;
    use super::super::tests::TestLinks;
    use super::*;

    #[test]
    fn test_rst() {
        let src = concat!(
            "=======\n",
            " Title\n",
            "=======\n",
            "\n",
            "Intro *em* ``lit`` :searchfox:`a/b.cpp` `link <http://x>`_ and :ref:`label`.\n",
            "\n",
            ".. _label:\n",
            "\n",
            "Section\n",
            "-------\n",
            "\n",
            "- Item\n",
            "\n",
            "  .. note::\n",
            "\n",
            "     A *note*.\n",
            "\n",
            "Code::\n",
            "\n",
            "    x = 1\n",
        );
        let rendered = render(src, "docs/page.rst", &TestLinks);
        assert_eq!(rendered.title.as_deref(), Some("Title"));
        let html = with_strips(&rendered);
        assert!(html.starts_with(r#"<div class="rd-block">[0]<h1 id="title"><span data-l="2" data-c="1">Title</span></h1></div>"#), "{}", html);
        assert!(
            html.contains(r#"<em><span data-l="5" data-c="7">em</span></em>"#),
            "{}",
            html
        );
        assert!(
            html.contains(r#"<code><span data-l="5" data-c="13">lit</span></code>"#),
            "{}",
            html
        );
        assert!(html.contains(r#"<a class="rd-searchfox" href="/t/source/a/b.cpp"><span data-l="5" data-c="31">a/b.cpp</span></a>"#), "{}", html);
        assert!(
            html.contains(r#"<a href="http://x"><span data-l="5" data-c="41">link</span></a>"#),
            "{}",
            html
        );
        assert!(
            html.contains(r##"<a class="rd-ref" href="#label">Section</a>"##),
            "{}",
            html
        );
        assert!(html.contains(r#"<span class="rd-target" id="label"></span><div class="rd-block">[2]<h2 id="section">"#), "{}", html);
        assert!(html.contains(r#"<div class="rd-admonition rd-note"><div class="rd-block rd-admonition-title">[4]<p>Note</p></div><div class="rd-block">[5]<p><span data-l="16" data-c="5">A </span><em><span data-l="16" data-c="8">note</span></em><span data-l="16" data-c="13">.</span></p></div></div>"#), "{}", html);
        assert!(html.contains(r#"<div class="rd-block">[6]<p><span data-l="18" data-c="0">Code</span>:</p></div><div class="rd-block rd-code">[7]<pre><code><span data-l="20" data-c="4">x = 1</span>"#), "{}", html);
    }
}
