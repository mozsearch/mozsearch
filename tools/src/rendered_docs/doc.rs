//! What rendering Markdown and reStructuredText have in common: Sphinx's
//! roles (ex: `searchfox`, `ref`, and `doc`), its directives (ex: notes,
//! code blocks, toctrees, and images), and its labels.  `Doc` renders a
//! document of either flavor; its flavors' methods are in `markdown.rs` and
//! `rst.rs`.

use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use super::markdown::Mode;
use super::{
    HtmlSanitizer, Links, OffsetMap, Writer, escape, find_doc, image_url, is_doc_path,
    resolve_path, searchfox_url, slugify,
};

lazy_static! {
    /// `TITLE <TARGET>` in roles, references, and toctree entries.
    static ref TITLE_TARGET: Regex = Regex::new(r"(?s)^(.*?)\s*<([^<>]*)>$").unwrap();
    /// An option of a MyST directive.
    static ref OPTION: Regex = Regex::new(r"^\s*:([\w.-]+):(?:\s+(.*?))?\s*$").unwrap();
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Flavor {
    Markdown,
    Rst,
}

/// A Sphinx label: a target, or a section's title (as Sphinx's
/// `autosectionlabel` makes them).
pub(super) struct Label {
    pub id: String,
    /// The section's title, for references to it without a title.
    pub title: Option<String>,
}

pub(super) enum Substitution {
    Text(String),
    Image(String),
}

pub(super) struct Doc<'a, 'w, 's> {
    pub w: &'w mut Writer<'s>,
    pub links: &'a dyn Links,
    /// The document's directory, for relative links.
    pub dir: String,
    /// The flavor of the content of directives.
    pub flavor: Flavor,
    /// Sphinx labels by lowercase label.
    pub labels: HashMap<String, Label>,
    /// The ids of the document's headings, by the offset in the file of their
    /// start.
    pub heading_ids: HashMap<usize, String>,
    /// Markdown's link reference definitions, by lowercase label, for the
    /// documents of directives' content.
    pub refs: HashMap<String, (String, String)>,
    /// reStructuredText's section adornment styles (the character, and
    /// whether it has an overline) in order of appearance, which is their
    /// level.
    pub styles: Vec<(char, bool)>,
    /// reStructuredText's named hyperlink targets' URLs, by lowercase name.
    pub link_targets: HashMap<String, String>,
    /// reStructuredText's substitutions, by name.
    pub substitutions: HashMap<String, Substitution>,
}

impl<'a, 'w, 's> Doc<'a, 'w, 's> {
    pub fn new(w: &'w mut Writer<'s>, links: &'a dyn Links, path: &str, flavor: Flavor) -> Self {
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
        Doc {
            w,
            links,
            dir,
            flavor,
            labels: HashMap::new(),
            heading_ids: HashMap::new(),
            refs: HashMap::new(),
            styles: vec![],
            link_targets: HashMap::new(),
            substitutions: HashMap::new(),
        }
    }

    /// Render the content of a directive, in the document's flavor.
    pub fn body(&mut self, text: &str, map: &OffsetMap) {
        match self.flavor {
            Flavor::Markdown => self.md_nested(text, map, false, Mode::Normal),
            Flavor::Rst => self.rst_nested(text, map),
        }
    }

    /// The id of a heading starting at the byte `start` of the file.
    pub fn heading_id(&mut self, start: usize, title: &str) -> String {
        match self.heading_ids.remove(&start) {
            Some(id) => id,
            None => self.w.unique_id(&slugify(title)),
        }
    }

    /// Give a heading an id (like MyST's `myst_heading_anchors`) before
    /// rendering, so that references to it can link to it.
    pub fn add_heading(&mut self, start: usize, title: &str, labels: &[String]) {
        let id = self.w.unique_id(&slugify(title));
        self.heading_ids.insert(start, id.clone());
        for label in [title.to_lowercase(), id.clone()] {
            self.labels.entry(label).or_insert(Label {
                id: id.clone(),
                title: Some(title.to_string()),
            });
        }
        for label in labels {
            if let Some(target) = self.labels.get_mut(&label.to_lowercase()) {
                target.title = Some(title.to_string());
            }
        }
    }

    /// Learn of a target's label before rendering.
    pub fn add_target(&mut self, label: &str) {
        let id = if self.w.claim_id(label) {
            label.to_string()
        } else {
            self.w.unique_id(label)
        };
        self.labels
            .insert(label.to_lowercase(), Label { id, title: None });
    }

    /// An anchor for a target.
    pub fn target_anchor(&mut self, label: &str) {
        let id = match self.labels.get(&label.to_lowercase()) {
            Some(label) => label.id.clone(),
            None => self.w.unique_id(label),
        };
        write!(
            self.w.html,
            r#"<span class="rd-target" id="{}"></span>"#,
            escape(&id)
        )
        .unwrap();
    }

    /// The text of a role or toctree entry: its title if it has one (`TITLE
    /// <TARGET>`), or else `default`, which are ranges of `code`, whose bytes
    /// in the file are at `real`.
    pub fn role_text(
        &mut self,
        code: &str,
        real: &Option<Range<usize>>,
        title: &Option<Range<usize>>,
        default: Range<usize>,
    ) {
        let range = title.clone().unwrap_or(default);
        let piece_real = real
            .as_ref()
            .map(|real| real.start + range.start..real.start + range.end);
        self.w.text(&code[range], piece_real);
    }

    /// The link of a Sphinx `ref` role to `target`, if it's in this document,
    /// and the title of its section.
    pub fn find_label(&self, target: &str) -> Option<(String, Option<String>)> {
        // Labels can be prefixed by their document (ex: with
        // `autosectionlabel_prefix_document`).
        let label = target.trim().to_lowercase();
        self.labels
            .get(&label)
            .or_else(|| self.labels.get(&slugify(&label)))
            .or_else(|| {
                label
                    .rsplit_once(':')
                    .and_then(|(_, label)| self.labels.get(label))
            })
            .map(|label| (label.id.clone(), label.title.clone()))
    }

    /// A Sphinx role, with the text `code` (between the backticks), whose
    /// bytes in the file are at `real`.
    pub fn role(&mut self, name: &str, code: &str, real: Option<Range<usize>>) {
        let (title, target) = title_target(code);
        let target_text = code[target.clone()].trim();
        let name = name.to_ascii_lowercase();
        let name = name.strip_prefix("std:").unwrap_or(&name);
        match name {
            "searchfox" => {
                let (url, path) = searchfox_url(self.links, target_text);
                write!(
                    self.w.html,
                    r#"<a class="rd-searchfox" href="{}">"#,
                    escape(&url)
                )
                .unwrap();
                let path_start = target.end - path.len().min(target.len());
                self.role_text(code, &real, &title, path_start..target.end);
                self.w.push("</a>");
            }
            "ref" | "numref" => {
                let found = self.find_label(target_text);
                let closer = match &found {
                    Some((id, _)) => {
                        write!(
                            self.w.html,
                            r##"<a class="rd-ref" href="#{}">"##,
                            escape(id)
                        )
                        .unwrap();
                        "</a>"
                    }
                    None => {
                        write!(
                            self.w.html,
                            r#"<span class="rd-ref rd-unresolved" title="Sphinx reference to {}">"#,
                            escape(target_text)
                        )
                        .unwrap();
                        "</span>"
                    }
                };
                match (&title, found.and_then(|(_, title)| title)) {
                    (None, Some(section)) => self.w.escaped(&section),
                    _ => self.role_text(code, &real, &title, target.clone()),
                }
                self.w.push(closer);
            }
            "doc" => {
                match find_doc(self.links, &self.dir, target_text) {
                    Some(doc) => write!(
                        self.w.html,
                        r#"<a class="rd-doc" href="{}">"#,
                        escape(&self.links.rendered_url(&doc))
                    )
                    .unwrap(),
                    None => self.w.push(r#"<a class="rd-doc">"#),
                }
                self.role_text(code, &real, &title, target);
                self.w.push("</a>");
            }
            "kbd" => {
                self.w.push("<kbd>");
                self.role_text(code, &real, &None, 0..code.len());
                self.w.push("</kbd>");
            }
            "abbr" => {
                // `TEXT (EXPLANATION)`.
                let (text, explanation) = match code.rfind(" (") {
                    Some(i) if code.ends_with(')') => (0..i, Some(&code[i + 2..code.len() - 1])),
                    _ => (0..code.len(), None),
                };
                match explanation {
                    Some(explanation) => {
                        write!(self.w.html, r#"<abbr title="{}">"#, escape(explanation)).unwrap()
                    }
                    None => self.w.push("<abbr>"),
                }
                self.role_text(code, &real, &None, text);
                self.w.push("</abbr>");
            }
            "pep" | "rfc" => {
                let number: String = target_text
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                let url = if name == "pep" {
                    format!("https://peps.python.org/pep-{:0>4}/", number)
                } else {
                    format!("https://datatracker.ietf.org/doc/html/rfc{}", number)
                };
                write!(
                    self.w.html,
                    r#"<a href="{}">{} "#,
                    escape(&url),
                    name.to_ascii_uppercase()
                )
                .unwrap();
                self.role_text(code, &real, &title, target);
                self.w.push("</a>");
            }
            "term" => {
                self.w.push(r#"<span class="rd-term">"#);
                self.role_text(code, &real, &title, target);
                self.w.push("</span>");
            }
            "emphasis" | "title-reference" | "t" => {
                self.w.push("<em>");
                self.role_text(code, &real, &None, 0..code.len());
                self.w.push("</em>");
            }
            "strong" => {
                self.w.push("<strong>");
                self.role_text(code, &real, &None, 0..code.len());
                self.w.push("</strong>");
            }
            "sub" | "subscript" | "sup" | "superscript" => {
                let tag = if name.starts_with("sub") {
                    "sub"
                } else {
                    "sup"
                };
                write!(self.w.html, "<{}>", tag).unwrap();
                self.role_text(code, &real, &None, 0..code.len());
                write!(self.w.html, "</{}>", tag).unwrap();
            }
            _ if is_xref_role(name) => {
                // `~a.b.c` shows `c`, and `!` means not to link.
                let shown = target_text.trim_start_matches(['~', '!']);
                let shown_start = if target_text.starts_with('~') {
                    shown.rfind('.').map_or(0, |i| i + 1)
                } else {
                    0
                };
                let name_start = target.end - shown.len().min(target.len()) + shown_start;
                let query = shown.trim_end_matches("()");
                write!(
                    self.w.html,
                    r#"<a class="rd-xref" href="{}"><code>"#,
                    escape(&self.links.search_url(query))
                )
                .unwrap();
                self.role_text(code, &real, &title, name_start.min(target.end)..target.end);
                self.w.push("</code></a>");
            }
            _ => {
                write!(
                    self.w.html,
                    r#"<code class="rd-role" title="{}">"#,
                    escape(name)
                )
                .unwrap();
                self.role_text(code, &real, &None, 0..code.len());
                self.w.push("</code>");
            }
        }
    }

    /// A Sphinx directive, whose argument `arg` is at `arg_real` in the file,
    /// and which is the bytes `fence` of the file.
    pub fn directive(
        &mut self,
        name: &str,
        arg: &str,
        arg_real: Option<Range<usize>>,
        content: &DirectiveContent,
        fence: Range<usize>,
    ) {
        let name = name.to_ascii_lowercase();
        let name = name.as_str();
        let opener = fence.start..self.w.line_end(fence.start).max(fence.start);
        let body = content.body();
        let body_map = content.body_map();
        let option = |key: &str| content.option(key);
        let admonition = match name {
            "note" => Some("Note"),
            "warning" => Some("Warning"),
            "important" => Some("Important"),
            "tip" => Some("Tip"),
            "hint" => Some("Hint"),
            "caution" => Some("Caution"),
            "attention" => Some("Attention"),
            "danger" => Some("Danger"),
            "error" => Some("Error"),
            "seealso" => Some("See also"),
            "todo" => Some("Todo"),
            "admonition" | "versionadded" | "versionchanged" | "deprecated" => Some(""),
            _ => None,
        };
        if let Some(title) = admonition {
            let class = if name == "admonition" {
                option("class")
                    .and_then(|class| class.split_whitespace().next())
                    .unwrap_or("note")
            } else {
                name
            };
            write!(
                self.w.html,
                r#"<div class="rd-admonition rd-{}">"#,
                escape(class)
            )
            .unwrap();
            let opened = self.w.open_block_div(opener, "rd-admonition-title");
            self.w.push("<p>");
            self.w.escaped(title);
            if !title.is_empty() && !arg.is_empty() {
                self.w.push(": ");
            }
            self.w.escaped(match name {
                "versionadded" => "New in version ",
                "versionchanged" => "Changed in version ",
                "deprecated" => "Deprecated since version ",
                _ => "",
            });
            self.w.text(arg, arg_real);
            self.w.push("</p>");
            self.w.close_block_div(opened);
            self.body(body, &body_map);
            self.w.push("</div>");
            return;
        }
        match name {
            "dropdown" | "topic" | "sidebar" | "tab-item" | "card" | "grid-item-card" => {
                let (outer, inner, end) = match (name, option("open")) {
                    ("dropdown", Some(_)) => (
                        r#"<details class="rd-dropdown" open><summary>"#,
                        "</summary>",
                        "</details>",
                    ),
                    ("dropdown", None) => (
                        r#"<details class="rd-dropdown"><summary>"#,
                        "</summary>",
                        "</details>",
                    ),
                    _ => (
                        r#"<div class="rd-topic"><div class="rd-topic-title">"#,
                        "</div>",
                        "</div>",
                    ),
                };
                self.w.push(outer);
                let opened = self.w.open_block_div(opener, "");
                self.w.text(arg, arg_real);
                self.w.close_block_div(opened);
                self.w.push(inner);
                self.body(body, &body_map);
                self.w.push(end);
            }
            "container" | "div" | "only" | "ifconfig" | "tab-set" | "tab-set-code" | "grid"
            | "grid-item" | "glossary" | "centered" | "hlist" | "table" | "compound" => {
                self.w.push(r#"<div class="rd-container">"#);
                if name == "table" && !arg.is_empty() {
                    self.w.push(r#"<p class="rd-rubric">"#);
                    self.w.escaped(arg);
                    self.w.push("</p>");
                }
                self.body(body, &body_map);
                self.w.push("</div>");
            }
            // Directives which don't render anything.
            "highlight" | "default-role" | "role" | "index" | "meta" | "sectionauthor"
            | "moduleauthor" | "currentmodule" | "module" | "py:module" | "py:currentmodule"
            | "js:module" | "tabularcolumns" | "orphan" => {}
            "rubric" => {
                let opened = self.w.open_block_div(fence, "");
                self.w.push(r#"<p class="rd-rubric">"#);
                self.w.text(arg, arg_real);
                self.w.push("</p>");
                self.w.close_block_div(opened);
            }
            "eval-rst" => {
                let flavor = self.flavor;
                self.flavor = Flavor::Rst;
                self.rst_nested(body, &body_map);
                self.flavor = flavor;
            }
            "toctree" => self.toctree(content, option("caption"), option("glob").is_some()),
            "image" | "figure" => {
                let header = fence.start..content.header_end(fence.end);
                let opened = self.w.open_block_div(header, "rd-image");
                self.image(arg, option("alt").unwrap_or(""), "", option("width"));
                self.w.close_block_div(opened);
                if !body.trim().is_empty() {
                    self.w.push(r#"<div class="rd-caption">"#);
                    self.body(body, &body_map);
                    self.w.push("</div>");
                }
            }
            "include" | "literalinclude" => {
                let opened = self.w.open_block_div(fence, "rd-include");
                self.w.escaped(if name == "include" {
                    "Includes "
                } else {
                    "Includes the code of "
                });
                match resolve_path(&self.dir, arg) {
                    Some(path) => {
                        let url = if is_doc_path(&path) {
                            self.links.rendered_url(&path)
                        } else {
                            self.links.source_url(&path)
                        };
                        write!(self.w.html, r#"<a href="{}">"#, escape(&url)).unwrap();
                        self.w.text(arg, arg_real);
                        self.w.push("</a>");
                    }
                    None => self.w.text(arg, arg_real),
                }
                self.w.close_block_div(opened);
            }
            "raw" if arg == "html" => {
                let opened = self.w.open_block_div(fence, "rd-html");
                let mut sanitizer = HtmlSanitizer::default();
                sanitizer.write(self.w, body, true, self.links, &self.dir);
                sanitizer.close_all(self.w);
                self.w.close_block_div(opened);
            }
            "list-table" => {
                let header_rows = option("header-rows")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                self.w.push(r#"<table class="rd-table rd-list-table">"#);
                if !arg.is_empty() {
                    self.w.push("<caption>");
                    self.w.escaped(arg);
                    self.w.push("</caption>");
                }
                self.w.push("<tbody>");
                match self.flavor {
                    Flavor::Markdown => {
                        self.md_nested(body, &body_map, false, Mode::ListTable { header_rows })
                    }
                    Flavor::Rst => self.rst_list_table(body, &body_map, header_rows),
                }
                self.w.push("</tbody></table>");
            }
            _ if name.starts_with("auto")
                || name.contains(":auto")
                || name == "mozbuildsymbols" =>
            {
                // Documentation that Sphinx generates from the code.
                let opened = self.w.open_block_div(fence, "rd-generated");
                self.w.push("Documentation generated from ");
                write!(
                    self.w.html,
                    r#"<a href="{}"><code>"#,
                    escape(&self.links.search_url(arg))
                )
                .unwrap();
                self.w.text(arg, arg_real);
                self.w.push("</code></a>");
                self.w.close_block_div(opened);
            }
            _ => {
                // Code, and directives we don't know, whose content we show
                // as is.
                let code_like = matches!(
                    name,
                    "code-block"
                        | "code"
                        | "sourcecode"
                        | "code-cell"
                        | "doctest"
                        | "testcode"
                        | "parsed-literal"
                        | "math"
                        | "csv-table"
                        | "mermaid"
                        | "graphviz"
                        | "digraph"
                        | "productionlist"
                );
                let (lang, caption) = if code_like {
                    let lang = if matches!(name, "mermaid" | "graphviz" | "digraph" | "math") {
                        name
                    } else {
                        arg
                    };
                    (lang.to_string(), option("caption").map(str::to_string))
                } else {
                    (
                        String::new(),
                        Some(format!("{} {}", name, arg).trim().to_string()),
                    )
                };
                let lines = content.body_lines();
                self.code(fence, &lang, caption.as_deref(), &lines);
            }
        }
    }

    /// A code block for the bytes `real` of the file, with its lines (and
    /// where they are in the file).
    pub fn code(
        &mut self,
        real: Range<usize>,
        lang: &str,
        caption: Option<&str>,
        lines: &[(&str, Option<Range<usize>>)],
    ) {
        let opened = self.w.open_block_div(real, "rd-code");
        if let Some(caption) = caption {
            self.w.push(r#"<div class="rd-code-caption">"#);
            self.w.escaped(caption);
            self.w.push("</div>");
        }
        self.w.push("<pre><code");
        if !lang.is_empty() {
            write!(self.w.html, r#" data-lang="{}""#, escape(lang)).unwrap();
        }
        self.w.push(">");
        for (text, real) in lines {
            self.w.text(text, real.clone());
        }
        self.w.push("</code></pre>");
        self.w.close_block_div(opened);
    }

    pub fn image(&mut self, src: &str, alt: &str, title: &str, width: Option<&str>) {
        match image_url(self.links, &self.dir, src) {
            Some(url) => {
                write!(
                    self.w.html,
                    r#"<img src="{}" alt="{}" loading="lazy""#,
                    escape(&url),
                    escape(alt)
                )
                .unwrap();
                if !title.is_empty() {
                    write!(self.w.html, r#" title="{}""#, escape(title)).unwrap();
                }
                if let Some(width) = width.filter(|w| {
                    !w.is_empty()
                        && w.chars()
                            .all(|c| c.is_ascii_digit() || matches!(c, '%' | '.' | 'p' | 'x'))
                }) {
                    let width = if width.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}px", width)
                    } else {
                        width.to_string()
                    };
                    write!(self.w.html, r#" style="max-width: {}""#, escape(&width)).unwrap();
                }
                self.w.push(">");
            }
            None => {
                self.w.push(r#"<span class="rd-dead-image">"#);
                self.w.escaped(if alt.is_empty() { src } else { alt });
                self.w.push("</span>");
            }
        }
    }

    /// A toctree: a list of links to documents, each line of which is a
    /// block.
    fn toctree(&mut self, content: &DirectiveContent, caption: Option<&str>, glob: bool) {
        self.w.push(r#"<div class="rd-toctree">"#);
        if let Some(caption) = caption {
            self.w.push(r#"<p class="rd-toctree-caption">"#);
            self.w.escaped(caption);
            self.w.push("</p>");
        }
        self.w.push("<ul>");
        for (line, real) in content.body_lines() {
            let entry = line.trim();
            if entry.is_empty() {
                continue;
            }
            let lead = line.len() - line.trim_start().len();
            let real = real.map(|r| r.start + lead..r.start + lead + entry.len());
            self.w.push("<li>");
            let opened = self
                .w
                .open_block_div(real.clone().unwrap_or(0..0), "rd-toc-entry");
            let (title, target) = title_target(entry);
            let target_text = entry[target.clone()].trim();
            let doc = if glob && target_text.contains('*') {
                None
            } else {
                find_doc(self.links, &self.dir, target_text)
            };
            match doc {
                Some(doc) => write!(
                    self.w.html,
                    r#"<a href="{}">"#,
                    escape(&self.links.rendered_url(&doc))
                )
                .unwrap(),
                None => self.w.push("<a>"),
            }
            self.role_text(entry, &real, &title, target);
            self.w.push("</a>");
            self.w.close_block_div(opened);
            self.w.push("</li>");
        }
        self.w.push("</ul></div>");
    }
}

/// The content of a directive: its lines (without its container's
/// indentation), its options, and its body.
pub(super) struct DirectiveContent {
    pub text: String,
    pub map: OffsetMap,
    pub options: Vec<(String, String)>,
    /// Where the body starts in `text`.
    pub body_start: usize,
}

impl DirectiveContent {
    /// The content of a MyST directive, from its lines (as pieces of the
    /// virtual document that `map` maps), which start with its options
    /// (`:name: value` lines or YAML between `---` lines).
    pub fn from_lines(lines: &[(&str, Range<usize>)], map: &OffsetMap) -> DirectiveContent {
        let text: String = lines.iter().map(|(t, _)| *t).collect();
        let pieces: Vec<(Range<usize>, usize)> = lines
            .iter()
            .map(|(t, range)| (range.clone(), t.len()))
            .collect();
        let map = map.compose_pieces(&pieces);

        let mut options = vec![];
        let mut pos = 0;
        let mut lines = text.split_inclusive('\n').peekable();
        if lines.peek().is_some_and(|line| line.trim() == "---") {
            // YAML options.
            let mut yaml_pos = 0;
            let mut closed = false;
            for (i, line) in text.split_inclusive('\n').enumerate() {
                yaml_pos += line.len();
                if i == 0 {
                    continue;
                }
                if line.trim() == "---" {
                    closed = true;
                    break;
                }
                if let Some((key, value)) = line.split_once(':') {
                    options.push((key.trim().to_string(), value.trim().to_string()));
                }
            }
            if closed {
                pos = yaml_pos;
            } else {
                options.clear();
            }
        } else {
            for line in lines {
                match OPTION.captures(line) {
                    Some(captures) => {
                        options.push((
                            captures[1].to_string(),
                            captures.get(2).map_or("", |m| m.as_str()).to_string(),
                        ));
                        pos += line.len();
                    }
                    None => break,
                }
            }
        }
        // A blank line after the options.
        if pos > 0 && text[pos..].starts_with('\n') {
            pos += 1;
        }
        DirectiveContent {
            text,
            map,
            options,
            body_start: pos,
        }
    }

    pub fn option(&self, key: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn body(&self) -> &str {
        &self.text[self.body_start..]
    }

    pub fn body_map(&self) -> OffsetMap {
        self.map.slice(self.body_start..self.text.len())
    }

    /// The end in the file of the opener and options, given the end of the
    /// directive.
    pub fn header_end(&self, fence_end: usize) -> usize {
        if self.body().trim().is_empty() {
            fence_end
        } else {
            self.map.real(self.body_start)
        }
    }

    /// The lines of the body, and where they are in the file.
    pub fn body_lines(&self) -> Vec<(&str, Option<Range<usize>>)> {
        let mut pos = self.body_start;
        self.body()
            .split_inclusive('\n')
            .map(|line| {
                let start = pos;
                pos += line.len();
                (line, self.map.exact(start..start + line.len()))
            })
            .collect()
    }
}

/// `TITLE <TARGET>`: the ranges of the title (if any) and the target in
/// `text`.
pub(super) fn title_target(text: &str) -> (Option<Range<usize>>, Range<usize>) {
    match TITLE_TARGET.captures(text) {
        Some(captures) if !captures[1].trim().is_empty() => {
            let title = captures.get(1).unwrap().range();
            (Some(title), captures.get(2).unwrap().range())
        }
        _ => {
            let start = text.len() - text.trim_start().len();
            (None, start..text.trim_end().len().max(start))
        }
    }
}

fn is_xref_role(name: &str) -> bool {
    name.contains(':')
        || matches!(
            name,
            "func"
                | "meth"
                | "class"
                | "mod"
                | "attr"
                | "obj"
                | "data"
                | "exc"
                | "const"
                | "any"
                | "envvar"
                | "option"
                | "program"
                | "command"
        )
}
