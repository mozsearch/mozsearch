//! Rendering Markdown, with the MyST extensions that Sphinx documentation
//! uses (see Firefox's `docs/conf.py`): directives (in backtick or colon
//! fences, ex: ```` ```{note} ```` and `:::{list-table}`), roles (ex:
//! `` {searchfox}`path` ``), targets (`(label)=`), field lists, and front
//! matter.  See the module's comment in `mod.rs` for blocks and runs.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;

use lazy_static::lazy_static;
use pulldown_cmark::{
    Alignment, BlockQuoteKind, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser,
    Tag, TagEnd,
};
use regex::Regex;

use super::doc::{DirectiveContent, Doc, Flavor};
use super::{HtmlSanitizer, Links, OffsetMap, Rendered, Writer, escape, link_url, slugify};

type Events = Vec<(Event<'static>, Range<usize>)>;

lazy_static! {
    /// A MyST role's name at the end of the text before its code span.
    static ref ROLE: Regex = Regex::new(r"\{([A-Za-z][\w:.+-]*)\}$").unwrap();
    static ref ROLE_ANYWHERE: Regex = Regex::new(r"\{[A-Za-z][\w:.+-]*\}").unwrap();
    /// A MyST directive's fence's info string.
    static ref DIRECTIVE: Regex = Regex::new(r"^\{([A-Za-z][\w:.+-]*)\}\s*(.*)$").unwrap();
    /// A MyST target.
    static ref TARGET: Regex = Regex::new(r"^\(([^()\s]+)\)=$").unwrap();
    /// A field of a MyST field list.
    static ref FIELD: Regex = Regex::new(r"^:([^:\s][^:]*):(\s+|$)").unwrap();
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_GFM
}

/// Render the Markdown document `src`, which is at `path`.
pub fn render(src: &str, path: &str, links: &dyn Links) -> Rendered {
    let mut w = Writer::new(src);
    let mut doc = Doc::new(&mut w, links, path, Flavor::Markdown);
    doc.md_nested(src, &OffsetMap::identity(src.len()), true, Mode::Normal);
    w.finish()
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Normal,
    /// The content of a `list-table` directive, whose list of lists is a
    /// table, whose first `header_rows` rows are headers.
    ListTable {
        header_rows: usize,
    },
}

#[derive(Default)]
struct State {
    /// For each leaf element being rendered (ex: a paragraph), whether it
    /// opened a block.
    opened: Vec<bool>,
    /// For each paragraph, whether it's a field list.
    fields: Vec<bool>,
    /// Whether a field list's field is open.
    in_field: bool,
    /// Whether we're at the start of a line of a paragraph.
    line_start: bool,
    /// The closing tags of links.
    closers: Vec<&'static str>,
    /// For each list item (and definition), whether a block for its text is
    /// open (for tight lists, whose items' text isn't in paragraphs), and
    /// whether it opened.
    tight: Vec<Option<bool>>,
    table_head: bool,
    aligns: Vec<Alignment>,
    cell: usize,
    /// The depth of lists in a list table.
    list_depth: usize,
    rows: usize,
    sanitizer: HtmlSanitizer,
}

impl Doc<'_, '_, '_> {
    fn md_parse(&self, text: &str) -> (Events, HashMap<String, (String, String)>) {
        let refs = &self.refs;
        let callback = |link: BrokenLink| -> Option<(CowStr, CowStr)> {
            refs.get(&link.reference.to_lowercase())
                .map(|(url, title)| (url.clone().into(), title.clone().into()))
        };
        let mut iter = Parser::new_with_broken_link_callback(text, options(), Some(callback))
            .into_offset_iter();
        let events = iter
            .by_ref()
            .map(|(event, range)| (event.into_static(), range))
            .collect();
        let defs = iter
            .reference_definitions()
            .iter()
            .map(|(label, def)| {
                let title = def.title.as_ref().map_or(String::new(), |t| t.to_string());
                (label.to_lowercase(), (def.dest.to_string(), title))
            })
            .collect();
        (events, defs)
    }

    /// Render a Markdown document (or the content of a directive), whose text
    /// is `text`, which `map` maps to the file.
    pub(super) fn md_nested(&mut self, text: &str, map: &OffsetMap, top: bool, mode: Mode) {
        let text = colon_fences(text);
        let (events, defs) = self.md_parse(&text);
        if top {
            self.refs = defs;
            self.md_collect_labels(&events, map);
        } else {
            for (label, def) in defs {
                self.refs.entry(label).or_insert(def);
            }
        }
        let mut st = State::default();
        let mut i = 0;
        while i < events.len() {
            let (event, range) = &events[i];
            if is_block_level(event) {
                if let Some(top) = st.tight.last_mut()
                    && let Some(opened) = top.take()
                {
                    self.w.close_block_div(opened);
                }
            } else if self.w.in_block() {
                self.w.extend_block(map.span(range.clone()));
            }
            i = self.md_event(&events, i, map, &text, mode, &mut st);
        }
    }

    /// Give the document's headings ids, and learn its labels, before
    /// rendering it.
    fn md_collect_labels(&mut self, events: &Events, map: &OffsetMap) {
        let mut pending: Vec<String> = vec![];
        for (i, (event, range)) in events.iter().enumerate() {
            match event {
                Event::Start(Tag::Heading { .. }) => {
                    let title = inline_text(events, i);
                    // A target right before a section labels the section.
                    let labels = std::mem::take(&mut pending);
                    self.add_heading(map.real(range.start), &title, &labels);
                }
                Event::Start(Tag::Paragraph) => {
                    let (targets, next) = leading_targets(events, i);
                    for target in &targets {
                        self.add_target(target);
                    }
                    // (Only paragraphs that are all targets label sections.)
                    pending = if matches!(events.get(next), Some((Event::End(_), _))) {
                        targets
                    } else {
                        vec![]
                    };
                }
                Event::Start(_) | Event::Rule => pending.clear(),
                _ => {}
            }
        }
    }

    /// Render the event at `i`, returning the index of the next event.
    fn md_event(
        &mut self,
        events: &Events,
        i: usize,
        map: &OffsetMap,
        text: &str,
        mode: Mode,
        st: &mut State,
    ) -> usize {
        let (event, range) = &events[i];
        let real = map.span(range.clone());
        let list_table = match mode {
            Mode::ListTable { header_rows } => Some(header_rows),
            Mode::Normal => None,
        };
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => return self.md_paragraph(events, i, map, st),
                Tag::Heading { level, id, .. } => {
                    let title = inline_text(events, i);
                    let anchor = match id {
                        Some(id) => self.w.unique_id(id),
                        None => self.heading_id(real.start, &title),
                    };
                    if self.w.title.is_none() {
                        self.w.title = Some(title);
                    }
                    st.opened.push(self.w.open_block_div(real, ""));
                    write!(
                        self.w.html,
                        r#"<h{} id="{}">"#,
                        *level as usize,
                        escape(&anchor)
                    )
                    .unwrap();
                }
                Tag::BlockQuote(kind) => match kind {
                    Some(kind) => {
                        let (class, title) = match kind {
                            BlockQuoteKind::Note => ("note", "Note"),
                            BlockQuoteKind::Tip => ("tip", "Tip"),
                            BlockQuoteKind::Important => ("important", "Important"),
                            BlockQuoteKind::Warning => ("warning", "Warning"),
                            BlockQuoteKind::Caution => ("caution", "Caution"),
                        };
                        write!(
                            self.w.html,
                            r#"<div class="rd-admonition rd-{}"><p class="rd-admonition-title">{}</p>"#,
                            class, title
                        )
                        .unwrap();
                    }
                    None => self.w.push("<blockquote>"),
                },
                Tag::CodeBlock(kind) => return self.md_code_block(events, i, map, kind),
                Tag::HtmlBlock => {
                    let end = end_of(events, i);
                    let html: String = events[i + 1..end]
                        .iter()
                        .filter_map(|(event, _)| match event {
                            Event::Html(html) | Event::Text(html) => Some(html.as_ref()),
                            _ => None,
                        })
                        .collect();
                    let opened = self.w.open_block_div(real, "rd-html");
                    st.sanitizer
                        .write(self.w, &html, true, self.links, &self.dir);
                    st.sanitizer.close_all(self.w);
                    self.w.close_block_div(opened);
                    return end + 1;
                }
                Tag::List(start) => {
                    if list_table.is_some() {
                        st.list_depth += 1;
                        if st.list_depth <= 2 {
                            return i + 1;
                        }
                    }
                    match start {
                        Some(1) => self.w.push("<ol>"),
                        Some(n) => write!(self.w.html, r#"<ol start="{}">"#, n).unwrap(),
                        None => self.w.push("<ul>"),
                    }
                }
                Tag::Item => {
                    if let Some(header_rows) = list_table {
                        if st.list_depth == 1 {
                            self.w.push("<tr>");
                            st.opened.push(self.w.start_block(real, false));
                            st.rows += 1;
                            return i + 1;
                        }
                        if st.list_depth == 2 {
                            let tag = if st.rows <= header_rows { "th" } else { "td" };
                            write!(self.w.html, "<{}>", tag).unwrap();
                            self.w.place_strip();
                            st.tight.push(None);
                            return i + 1;
                        }
                    }
                    self.w.push("<li>");
                    self.md_tight_start(events, i, map, st);
                }
                Tag::FootnoteDefinition(label) => {
                    write!(
                        self.w.html,
                        r#"<div class="rd-footnote" id="fn-{}"><span class="rd-footnote-label">{}</span>"#,
                        escape(&slugify(label)),
                        escape(label)
                    )
                    .unwrap();
                }
                Tag::DefinitionList => self.w.push("<dl>"),
                Tag::DefinitionListTitle => {
                    st.opened.push(self.w.open_block_div(real, "rd-dt"));
                    self.w.push("<dt>");
                }
                Tag::DefinitionListDefinition => {
                    self.w.push("<dd>");
                    self.md_tight_start(events, i, map, st);
                }
                Tag::Table(aligns) => {
                    st.aligns = aligns.clone();
                    self.w.push(r#"<table class="rd-table">"#);
                }
                Tag::TableHead => {
                    st.table_head = true;
                    st.cell = 0;
                    self.w.push("<thead><tr>");
                    // The head's block includes the delimiter row.
                    let end = self.w.line_end(real.end);
                    st.opened
                        .push(self.w.start_block(real.start..end.max(real.end), false));
                }
                Tag::TableRow => {
                    st.cell = 0;
                    self.w.push("<tr>");
                    st.opened.push(self.w.start_block(real, false));
                }
                Tag::TableCell => {
                    let tag = if st.table_head { "th" } else { "td" };
                    let align = match st.aligns.get(st.cell) {
                        Some(Alignment::Left) => r#" style="text-align: left""#,
                        Some(Alignment::Center) => r#" style="text-align: center""#,
                        Some(Alignment::Right) => r#" style="text-align: right""#,
                        _ => "",
                    };
                    write!(self.w.html, "<{}{}>", tag, align).unwrap();
                    self.w.place_strip();
                    st.cell += 1;
                }
                Tag::Emphasis => self.w.push("<em>"),
                Tag::Strong => self.w.push("<strong>"),
                Tag::Strikethrough => self.w.push("<del>"),
                Tag::Superscript => self.w.push("<sup>"),
                Tag::Subscript => self.w.push("<sub>"),
                Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    ..
                } => {
                    let dest = if *link_type == LinkType::Email {
                        format!("mailto:{}", dest_url)
                    } else {
                        dest_url.to_string()
                    };
                    match link_url(self.links, &self.dir, &dest) {
                        Some(url) => {
                            write!(self.w.html, r#"<a href="{}""#, escape(&url)).unwrap();
                            if !title.is_empty() {
                                write!(self.w.html, r#" title="{}""#, escape(title)).unwrap();
                            }
                            self.w.push(">");
                            st.closers.push("</a>");
                        }
                        None => {
                            self.w.push(r#"<span class="rd-dead-link">"#);
                            st.closers.push("</span>");
                        }
                    }
                }
                Tag::Image {
                    dest_url, title, ..
                } => {
                    let alt = inline_text(events, i);
                    self.image(dest_url, &alt, title, None);
                    return end_of(events, i) + 1;
                }
                Tag::MetadataBlock(_) => {
                    let end = end_of(events, i);
                    let opened = self.w.open_block_div(real, "rd-front-matter");
                    self.w.push("<details><summary>Front matter</summary><pre>");
                    for (event, range) in &events[i + 1..end] {
                        if let Event::Text(t) = event {
                            let virt = verbatim(text, t, range);
                            self.w.text(t, virt.and_then(|r| map.exact(r)));
                        }
                    }
                    self.w.push("</pre></details>");
                    self.w.close_block_div(opened);
                    return end + 1;
                }
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    if st.fields.pop() == Some(true) {
                        if st.in_field {
                            self.w.push("</dd>");
                        }
                        st.in_field = false;
                        self.w.push("</dl>");
                    } else {
                        self.w.push("</p>");
                    }
                    st.sanitizer.close_all(self.w);
                    let opened = st.opened.pop().unwrap_or(false);
                    self.w.close_block_div(opened);
                }
                TagEnd::Heading(level) => {
                    write!(self.w.html, "</h{}>", *level as usize).unwrap();
                    st.sanitizer.close_all(self.w);
                    let opened = st.opened.pop().unwrap_or(false);
                    self.w.close_block_div(opened);
                }
                TagEnd::BlockQuote(kind) => {
                    self.w.push(if kind.is_some() {
                        "</div>"
                    } else {
                        "</blockquote>"
                    });
                }
                TagEnd::List(ordered) => {
                    if list_table.is_some() {
                        st.list_depth -= 1;
                        if st.list_depth < 2 {
                            return i + 1;
                        }
                    }
                    self.w.push(if *ordered { "</ol>" } else { "</ul>" });
                }
                TagEnd::Item => {
                    if let Some(header_rows) = list_table {
                        if st.list_depth == 1 {
                            self.w.push("</tr>");
                            if st.opened.pop().unwrap_or(false) {
                                self.w.end_block();
                            }
                            return i + 1;
                        }
                        if st.list_depth == 2 {
                            st.tight.pop();
                            self.w.push(if st.rows <= header_rows {
                                "</th>"
                            } else {
                                "</td>"
                            });
                            return i + 1;
                        }
                    }
                    st.tight.pop();
                    self.w.push("</li>");
                }
                TagEnd::FootnoteDefinition => self.w.push("</div>"),
                TagEnd::DefinitionList => self.w.push("</dl>"),
                TagEnd::DefinitionListTitle => {
                    self.w.push("</dt>");
                    st.sanitizer.close_all(self.w);
                    let opened = st.opened.pop().unwrap_or(false);
                    self.w.close_block_div(opened);
                }
                TagEnd::DefinitionListDefinition => {
                    st.tight.pop();
                    self.w.push("</dd>");
                }
                TagEnd::Table => self.w.push("</tbody></table>"),
                TagEnd::TableHead => {
                    st.table_head = false;
                    self.w.push("</tr></thead><tbody>");
                    if st.opened.pop().unwrap_or(false) {
                        self.w.end_block();
                    }
                }
                TagEnd::TableRow => {
                    self.w.push("</tr>");
                    if st.opened.pop().unwrap_or(false) {
                        self.w.end_block();
                    }
                }
                TagEnd::TableCell => self.w.push(if st.table_head { "</th>" } else { "</td>" }),
                TagEnd::Emphasis => self.w.push("</em>"),
                TagEnd::Strong => self.w.push("</strong>"),
                TagEnd::Strikethrough => self.w.push("</del>"),
                TagEnd::Superscript => self.w.push("</sup>"),
                TagEnd::Subscript => self.w.push("</sub>"),
                TagEnd::Link => {
                    let closer = st.closers.pop().unwrap_or("");
                    self.w.push(closer);
                }
                _ => {}
            },
            Event::Text(t) => {
                // A role is its name before a code span.
                if let Some((Event::Code(code), code_range)) = events.get(i + 1)
                    && let Some(m) = ROLE.captures(t)
                {
                    let start = m.get(0).unwrap().start();
                    let before = verbatim(text, t, range).map(|r| r.start..r.start + start);
                    self.w.text(&t[..start], before.and_then(|r| map.exact(r)));
                    let inner = code_range_in(text, code, code_range);
                    self.role(&m[1], code, inner.and_then(|r| map.exact(r)));
                    st.line_start = false;
                    return i + 2;
                }
                let mut t: &str = t;
                let mut virt = verbatim(text, t, range);
                let mut stripped = false;
                if st.fields.last() == Some(&true)
                    && st.line_start
                    && let Some(m) = FIELD.captures(t)
                {
                    if st.in_field {
                        self.w.push("</dd>");
                    }
                    let name = m.get(1).unwrap().range();
                    self.w.push("<dt>");
                    let name_real = virt
                        .clone()
                        .and_then(|r| map.exact(r.start + name.start..r.start + name.end));
                    self.w.text(&t[name], name_real);
                    self.w.push("</dt><dd>");
                    st.in_field = true;
                    let rest = m.get(0).unwrap().end();
                    t = &t[rest..];
                    virt = virt.map(|r| r.start + rest..r.end);
                    stripped = true;
                }
                st.line_start = false;
                // Text that isn't verbatim (ex: entities, and smart quotes) is
                // still at its range, if it's all of the event's text.
                let virt = match virt {
                    Some(virt) => Some(virt),
                    None if !stripped => Some(range.clone()),
                    None => None,
                };
                self.w.text(t, virt.and_then(|r| map.exact(r)));
            }
            Event::Code(code) => {
                self.w.push("<code>");
                let inner = code_range_in(text, code, range);
                self.w.text(code, inner.and_then(|r| map.exact(r)));
                self.w.push("</code>");
                st.line_start = false;
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                self.w.push(r#"<code class="rd-math">"#);
                self.w.escaped(math);
                self.w.push("</code>");
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                st.sanitizer
                    .write(self.w, html, false, self.links, &self.dir);
            }
            Event::FootnoteReference(label) => {
                write!(
                    self.w.html,
                    r##"<sup class="rd-footnote-ref"><a href="#fn-{}">{}</a></sup>"##,
                    escape(&slugify(label)),
                    escape(label)
                )
                .unwrap();
            }
            Event::SoftBreak => {
                self.w.push("\n");
                st.line_start = true;
            }
            Event::HardBreak => {
                self.w.push("<br>");
                st.line_start = true;
            }
            Event::Rule => {
                let opened = self.w.open_block_div(real, "rd-rule");
                self.w.push("<hr>");
                self.w.close_block_div(opened);
            }
            Event::TaskListMarker(checked) => {
                write!(
                    self.w.html,
                    r#"<input type="checkbox" disabled{}> "#,
                    if *checked { " checked" } else { "" }
                )
                .unwrap();
            }
        }
        i + 1
    }

    /// A list item's (or definition's) text, if it's not in a paragraph (in
    /// tight lists), gets a block of its own.
    fn md_tight_start(&mut self, events: &Events, i: usize, map: &OffsetMap, st: &mut State) {
        match events.get(i + 1) {
            Some((event, range)) if !is_block_level(event) => {
                let opened = self.w.open_block_div(map.span(range.clone()), "rd-tight");
                st.tight.push(Some(opened));
            }
            _ => st.tight.push(None),
        }
    }

    fn md_paragraph(
        &mut self,
        events: &Events,
        i: usize,
        map: &OffsetMap,
        st: &mut State,
    ) -> usize {
        let range = &events[i].1;
        // Sphinx targets are anchors.
        let (targets, next) = leading_targets(events, i);
        for target in &targets {
            self.target_anchor(target);
        }
        if matches!(
            events.get(next),
            Some((Event::End(TagEnd::Paragraph), _)) | None
        ) {
            return next + 1;
        }
        let start = if targets.is_empty() {
            range.start
        } else {
            events[next].1.start
        };
        let fields = matches!(&events[next].0, Event::Text(t) if FIELD.is_match(t));
        st.opened
            .push(self.w.open_block_div(map.span(start..range.end), ""));
        st.fields.push(fields);
        st.in_field = false;
        st.line_start = true;
        self.w.push(if fields {
            r#"<dl class="rd-fields">"#
        } else {
            "<p>"
        });
        next
    }

    fn md_code_block(
        &mut self,
        events: &Events,
        i: usize,
        map: &OffsetMap,
        kind: &CodeBlockKind,
    ) -> usize {
        let real = map.span(events[i].1.clone());
        let end = end_of(events, i);
        let content: Vec<(&str, Range<usize>)> = events[i + 1..end]
            .iter()
            .filter_map(|(event, range)| match event {
                Event::Text(t) => Some((t.as_ref(), range.clone())),
                _ => None,
            })
            .collect();
        let info = match kind {
            CodeBlockKind::Fenced(info) => info.trim(),
            CodeBlockKind::Indented => "",
        };
        if let Some(captures) = DIRECTIVE.captures(info) {
            let arg = captures[2].trim();
            // Where the argument is in the file, for its runs.
            let opener = real.start..self.w.line_end(real.start).max(real.start);
            let line = &self.w.src()[opener.clone()];
            let arg_real = line
                .find('}')
                .and_then(|brace| line[brace..].find(arg).map(|i| brace + i))
                .filter(|_| !arg.is_empty())
                .map(|i| opener.start + i..opener.start + i + arg.len());
            let content = DirectiveContent::from_lines(&content, map);
            self.directive(&captures[1], arg, arg_real, &content, real);
        } else {
            let lines: Vec<(&str, Option<Range<usize>>)> = content
                .iter()
                .map(|(t, range)| {
                    let real = (range.len() == t.len())
                        .then(|| map.exact(range.clone()))
                        .flatten();
                    (*t, real)
                })
                .collect();
            let lang = info.split_whitespace().next().unwrap_or("");
            self.code(real, lang, None, &lines);
        }
        end + 1
    }
}

/// The range of `text` (an event's text) in the virtual document `doc`, if
/// it's there verbatim at `range`.
fn verbatim(doc: &str, text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    (doc.get(range.clone()) == Some(text)).then(|| range.clone())
}

/// Where a code span's text is in the virtual document `doc`, given the
/// span's range (with its backticks).
fn code_range_in(doc: &str, code: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let span = doc.get(range.clone())?;
    let start = span.find(code)?;
    Some(range.start + start..range.start + start + code.len())
}

fn is_block_level(event: &Event) -> bool {
    match event {
        Event::Start(tag) => matches!(
            tag,
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::List(_)
                | Tag::Item
                | Tag::FootnoteDefinition(_)
                | Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::MetadataBlock(_)
        ),
        Event::End(tag) => matches!(
            tag,
            TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::FootnoteDefinition
                | TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::MetadataBlock(_)
        ),
        Event::Rule => true,
        _ => false,
    }
}

/// The index of the end of the element starting at `i`.
fn end_of(events: &Events, i: usize) -> usize {
    let mut depth = 0;
    for (j, (event, _)) in events.iter().enumerate().skip(i) {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    events.len() - 1
}

/// The text of the element starting at `i` (ex: a heading's title).
fn inline_text(events: &Events, i: usize) -> String {
    let end = end_of(events, i);
    let mut text = String::new();
    for (event, _) in &events[i + 1..end] {
        match event {
            Event::Text(t) | Event::Code(t) => text.push_str(t),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }
    // Roles' names aren't part of the text.
    ROLE_ANYWHERE.replace_all(&text, "").into_owned()
}

/// The labels of the Sphinx targets (`(label)=` lines) at the start of the
/// paragraph starting at `i`, and the index of its first event after them.
fn leading_targets(events: &Events, i: usize) -> (Vec<String>, usize) {
    let mut targets = vec![];
    let mut j = i + 1;
    loop {
        let mut k = j;
        let mut line = String::new();
        while let Some((Event::Text(t), _)) = events.get(k) {
            line.push_str(t);
            k += 1;
        }
        let Some(captures) = TARGET.captures(line.trim()) else {
            break;
        };
        match events.get(k) {
            Some((Event::SoftBreak, _)) if k > j => {
                targets.push(captures[1].to_string());
                j = k + 1;
            }
            Some((Event::End(TagEnd::Paragraph), _)) if k > j => {
                targets.push(captures[1].to_string());
                j = k;
                break;
            }
            _ => break,
        }
    }
    (targets, j)
}

/// MyST's colon fences (`:::{name}`) as tilde fences, which are the same
/// length, so that they're fenced code blocks, which we render as directives.
/// (pulldown-cmark doesn't know about colon fences, so they'd otherwise be
/// paragraphs, or definition lists.)
fn colon_fences(text: &str) -> Cow<'_, str> {
    lazy_static! {
        static ref OPEN: Regex = Regex::new(r"^\s*(:{3,})\{").unwrap();
        static ref CLOSE: Regex = Regex::new(r"^\s*(:{3,})\s*$").unwrap();
        static ref CODE: Regex = Regex::new(r"^\s*(`{3,}|~{3,})").unwrap();
    }
    if !text.contains(":::") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut code: Option<(char, usize)> = None;
    let mut open: Vec<usize> = vec![];
    for line in text.split_inclusive('\n') {
        if let Some((fence, len)) = code {
            let trimmed = line.trim();
            if trimmed.len() >= len && trimmed.chars().all(|c| c == fence) {
                code = None;
            }
            out.push_str(line);
            continue;
        }
        if let Some(captures) = CODE.captures(line) {
            let fence = &captures[1];
            code = Some((fence.chars().next().unwrap(), fence.len()));
            out.push_str(line);
            continue;
        }
        let colons = if let Some(captures) = OPEN.captures(line) {
            let colons = captures.get(1).unwrap();
            open.push(colons.len());
            Some(colons.range())
        } else if let Some(captures) = CLOSE.captures(line)
            && open.last().is_some_and(|&len| captures[1].len() >= len)
        {
            open.pop();
            Some(captures.get(1).unwrap().range())
        } else {
            None
        };
        match colons {
            Some(colons) => {
                out.push_str(&line[..colons.start]);
                out.extend(std::iter::repeat_n('~', colons.len()));
                out.push_str(&line[colons.end..]);
            }
            None => out.push_str(line),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::tests::TestLinks;
    use super::*;

    fn render_test(src: &str) -> Rendered {
        render(src, "docs/page.md", &TestLinks)
    }

    /// The HTML with each block's strip as `[N]`.
    pub fn with_strips(rendered: &Rendered) -> String {
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

    #[test]
    fn test_blocks_and_runs() {
        let src = "# Title *here*\n\nSome text that's\nwrapped &amp; more.\n\n- one\n- two\n";
        let rendered = render_test(src);
        assert_eq!(rendered.title.as_deref(), Some("Title here"));
        assert_eq!(
            with_strips(&rendered),
            concat!(
                r#"<div class="rd-block">[0]<h1 id="title-here"><span data-l="1" data-c="2">Title </span><em><span data-l="1" data-c="9">here</span></em></h1></div>"#,
                r#"<div class="rd-block">[1]<p><span data-l="3" data-c="0">Some text that</span><span data-l="3" data-c="14">’</span><span data-l="3" data-c="15">s</span>"#,
                "\n",
                r#"<span data-l="4" data-c="0">wrapped </span><span data-l="4" data-c="8" data-n="5">&amp;</span><span data-l="4" data-c="13"> more.</span></p></div>"#,
                r#"<ul><li><div class="rd-block rd-tight">[2]<span data-l="6" data-c="2">one</span></div></li>"#,
                r#"<li><div class="rd-block rd-tight">[3]<span data-l="7" data-c="2">two</span></div></li></ul>"#,
            )
        );
        let ranges: Vec<_> = rendered.blocks.iter().map(|b| b.range.clone()).collect();
        assert_eq!(ranges, vec![0..15, 16..53, 56..59, 62..65]);
    }

    #[test]
    fn test_myst() {
        let src = concat!(
            "(intro)=\n",
            "# Intro\n",
            "\n",
            "See {searchfox}`dom/base/Element.cpp`, {searchfox}`this <dom/x.cpp>`,\n",
            "{ref}`intro`, {doc}`other`, and {js:func}`~a.b.c`.\n",
            "\n",
            ":::{note}\n",
            "A *note*.\n",
            ":::\n",
            "\n",
            "```{code-block} js\n",
            ":caption: Code\n",
            "\n",
            "let x;\n",
            "```\n",
        );
        let rendered = render_test(src);
        let html = with_strips(&rendered);
        assert!(html.starts_with(r#"<span class="rd-target" id="intro"></span><div class="rd-block">[0]<h1 id="intro-1">"#), "{}", html);
        assert!(html.contains(r#"See </span><a class="rd-searchfox" href="/t/source/dom/base/Element.cpp"><span data-l="4" data-c="16">dom/base/Element.cpp</span></a>"#), "{}", html);
        assert!(html.contains(r#"<a class="rd-searchfox" href="/t/source/dom/x.cpp"><span data-l="4" data-c="51">this</span></a>"#), "{}", html);
        assert!(
            html.contains(r##"<a class="rd-ref" href="#intro">Intro</a>"##),
            "{}",
            html
        );
        assert!(html.contains(r#"<a class="rd-doc" href="/t/rendered/docs/other.md"><span data-l="5" data-c="20">other</span></a>"#), "{}", html);
        assert!(html.contains(r#"<a class="rd-xref" href="/t/search?q=a.b.c"><code><span data-l="5" data-c="47">c</span></code></a>"#), "{}", html);
        assert!(html.contains(r#"<div class="rd-admonition rd-note"><div class="rd-block rd-admonition-title">[2]<p>Note</p></div><div class="rd-block">[3]<p><span data-l="8" data-c="0">A </span><em><span data-l="8" data-c="3">note</span></em><span data-l="8" data-c="8">.</span></p></div></div>"#), "{}", html);
        assert!(html.contains(r#"<div class="rd-block rd-code">[4]<div class="rd-code-caption">Code</div><pre><code data-lang="js"><span data-l="14" data-c="0">let x;</span>"#), "{}", html);
    }

    #[test]
    fn test_tables() {
        let src = "| a | b |\n|---|--:|\n| 1 | 2 |\n\n:::{list-table}\n:header-rows: 1\n\n* - H\n  - I\n* - x\n  - y\n:::\n";
        let html = with_strips(&render_test(src));
        assert!(html.starts_with(r#"<table class="rd-table"><thead><tr><th>[0]<span data-l="1" data-c="2">a</span></th><th style="text-align: right"><span data-l="1" data-c="6">b</span></th></tr></thead><tbody><tr><td>[1]"#), "{}", html);
        assert!(html.contains(r#"<table class="rd-table rd-list-table"><tbody><tr><th>[2]<span data-l="8" data-c="4">H</span></th><th><span data-l="9" data-c="4">I</span></th></tr><tr><td>[3]"#), "{}", html);
    }

    #[test]
    fn test_eval_rst() {
        let src = "Text.\n\n```{eval-rst}\n.. note::\n\n   An *rst* note.\n```\n";
        let html = with_strips(&render_test(src));
        assert!(html.contains(r#"<div class="rd-admonition rd-note"><div class="rd-block rd-admonition-title">[1]<p>Note</p></div><div class="rd-block">[2]<p><span data-l="6" data-c="3">An </span><em><span data-l="6" data-c="7">rst</span></em><span data-l="6" data-c="11"> note.</span></p></div></div>"#), "{}", html);
    }

    #[test]
    fn test_colon_fences() {
        assert_eq!(
            colon_fences("::::{a}\n:::{b}\nx\n:::\n::::\n"),
            "~~~~{a}\n~~~{b}\nx\n~~~\n~~~~\n"
        );
        // Not in code blocks.
        assert_eq!(colon_fences("```\n:::{a}\n```\n"), "```\n:::{a}\n```\n");
    }
}
