//! ecmarkup sources (ECMA-262's `spec.html`, ECMA-402's, and TC39
//! proposals'), rendered as ecmarkup renders them, with the references our
//! analysis finds (see `spec_analyze::ecmarkup`):
//!
//! - clauses (`<emu-clause>`, `<emu-annex>`, and `<emu-intro>`) are
//!   sections, with their ids and `oldids`, whose `<h1>`s are their
//!   headings, with their numbers (1.2.3, and A.1 for annexes; introductions
//!   have none); the first page has the table of contents;
//! - algorithms (`<emu-alg>`) are their ecmarkdown lists, each step a block,
//!   with its id;
//! - ecmarkdown's formats (`_variables_`, `*values*`, `~constants~`,
//!   `` `code` ``, and `|Nonterminals|`) are formatted;
//! - autolinked terms and abstract operations, nonterminals, invocations of
//!   internal methods, `<emu-xref>`s (with their targets' numbers or names,
//!   if they're empty), and `<emu-prodref>`s link to their definitions, in
//!   the spec, or in the specs whose biblios it loads;
//! - tables and figures have their numbered captions;
//! - when the spec's package.json builds it with `--multipage` (as
//!   ECMA-262's does), each top-level clause starts a page, named by its id
//!   without `sec-`, like ecmarkup's multipage edition's;
//! - the files it imports (`<emu-import>`) are in it, but their blocks don't
//!   have blame, since their lines are their own files'.
//!
//! Not rendered: what ecmarkup generates (ex: structured headers'
//! descriptions of operations' parameters, and the productions that
//! `<emu-prodref>`s copy), and grammars' formatting (they're preformatted
//! text, with links).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, LazyLock, Mutex};

use lazy_static::lazy_static;
use regex::Regex;

use super::{
    Hooks, HtmlRenderer, Page, RenderedSpec, SpecLinks, anchor_url, fragment, has_rendered_view,
    is_paged, link_url, page_url, write_toc,
};
use crate::rendered_docs::escape;
use crate::spec_analyze::Combined;
use crate::spec_analyze::ecmarkup::{
    Biblio, CLAUSES, Emd, Reference, algorithm_steps, analyze_with_imports, biblio_urls, clause_h1,
    emd_spans, imports, loaded_biblios, resolve_with_imports,
};
use crate::spec_analyze::markup::{Doc, NodeKind};
use crate::spec_analyze::{SpecEntry, SpecFormat, SpecsConfig, spec_symbol};

/// Elements whose text isn't ecmarkdown.
const NO_EMD: &[&str] = &[
    "pre",
    "code",
    "emu-grammar",
    "emu-production",
    "script",
    "style",
    "emu-val",
    "emu-const",
    "var",
];

/// A clause.
struct Clause {
    id: Option<String>,
    number: Option<String>,
    title: String,
    /// How many clauses it's in.
    depth: usize,
}

/// The letters of an annex's number: A, B, ..., Z, AA, ...
fn annex_letters(mut n: usize) -> String {
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

fn words(doc: &Doc, node: usize) -> String {
    doc.text_content(node)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

lazy_static! {
    /// An ecmarkdown variable, for titles' text.
    static ref VARIABLE: Regex = Regex::new(r"\b_([A-Za-z0-9]+)_\b").unwrap();
}

/// A clause's title, for the table of contents: its heading's text, without
/// ecmarkdown's formats, and for a structured header, its name and its
/// parameters' names, as ecmarkup shows it (ex: "ToString ( argument )").
fn clause_title(doc: &Doc, h1: usize, structured: bool) -> String {
    let text = VARIABLE.replace_all(&words(doc, h1), "$1").into_owned();
    if !structured {
        return text;
    }
    let Some((name, rest)) = text.split_once(" (") else {
        return text;
    };
    let params = rest.rsplit_once(')').map_or(rest, |(params, _)| params);
    let names: Vec<&str> = params
        .split(',')
        .map(|param| {
            let param = param.split(':').next().unwrap_or("").trim();
            param.strip_prefix("optional ").unwrap_or(param).trim()
        })
        .filter(|param| !param.is_empty())
        .collect();
    if names.is_empty() {
        format!("{} ( )", name)
    } else {
        format!("{} ( {} )", name, names.join(", "))
    }
}

/// Whether a clause's `<h1>` is a structured header (followed by a `<dl
/// class="header">`).
fn is_structured(doc: &Doc, clause: usize, h1: usize) -> bool {
    let siblings = &doc.nodes[clause].children;
    let Some(at) = siblings.iter().position(|&c| c == h1) else {
        return false;
    };
    siblings[at + 1..]
        .iter()
        .find(|&&c| doc.name(c).is_some())
        .is_some_and(|&c| doc.is(c, "dl") && doc.attr(c, "class").as_deref() == Some("header"))
}

/// The clauses of a source, by their nodes, with their numbers.
fn clauses(doc: &Doc) -> HashMap<usize, Clause> {
    let mut clauses: HashMap<usize, Clause> = HashMap::new();
    // How many numbered clauses and annexes each clause (or the root) has.
    let mut counts: HashMap<usize, (usize, usize)> = HashMap::new();
    for node in 1..doc.nodes.len() {
        let Some(name) = doc.name(node).filter(|name| CLAUSES.contains(name)) else {
            continue;
        };
        let parent = doc
            .ancestors(node)
            .find(|&a| doc.name(a).is_some_and(|name| CLAUSES.contains(&name)));
        let parent_clause = parent.and_then(|p| clauses.get(&p));
        let depth = parent_clause.map_or(0, |p| p.depth + 1);
        let parent_number = parent_clause.and_then(|p| p.number.clone());
        let count = counts.entry(parent.unwrap_or(0)).or_default();
        let number = match name {
            "emu-intro" => None,
            "emu-annex" if parent.is_none() => {
                count.1 += 1;
                Some(annex_letters(count.1))
            }
            // (An introduction's clauses aren't numbered.)
            _ if parent.is_some() && parent_number.is_none() => None,
            _ => {
                count.0 += 1;
                Some(match parent_number {
                    Some(parent) => format!("{}.{}", parent, count.0),
                    None => count.0.to_string(),
                })
            }
        };
        let title = clause_h1(doc, node).map_or_else(String::new, |h1| {
            clause_title(doc, h1, is_structured(doc, node, h1))
        });
        clauses.insert(
            node,
            Clause {
                id: doc.attr(node, "id"),
                number,
                title,
                depth,
            },
        );
    }
    clauses
}

struct Ecmarkup<'r> {
    doc: &'r Doc<'r>,
    /// The references, by where they start.
    references: &'r [Reference],
    clauses: HashMap<usize, Clause>,
    /// The `<h1>`s of the clauses.
    headings: HashMap<usize, usize>,
    hidden: Vec<bool>,
    no_emd: Vec<bool>,
    /// Where each id is.
    id_offsets: HashMap<String, usize>,
    /// The paths of algorithm steps with ids, ex: "3.a".
    step_paths: HashMap<String, String>,
    /// Tables' and figures' numbers.
    captions: HashMap<usize, String>,
    pages: Vec<Page>,
    /// The bytes of the source that the view has (a page's, or an imported
    /// file's).
    view: Range<usize>,
    /// The first elements of the files imported in the view, which start
    /// with links to the files' own views, and the files' paths.
    imports: HashMap<usize, String>,
    /// The URL of the rendered source (the main file's).
    base: String,
    /// The URLs of the views of the clauses with algorithms beside the code
    /// citing them.
    algorithm_urls: HashMap<usize, String>,
    spec: &'r SpecEntry,
    config: &'r SpecsConfig,
    links: &'r dyn SpecLinks,
    biblios: &'r [&'r Biblio],
}

impl Ecmarkup<'_> {
    fn local_href(&self, id: &str) -> String {
        match self.id_offsets.get(id) {
            Some(&at) if !self.view.contains(&at) => {
                let page = &self.pages[Page::index_of(&self.pages, at)];
                format!(
                    "{}{}",
                    page_url(&self.base, page.name.as_deref()),
                    fragment(id)
                )
            }
            _ => fragment(id),
        }
    }

    /// The URL of a reference's target.
    fn reference_href(&self, reference: &Reference) -> Option<String> {
        let Some(shortname) = &reference.spec else {
            return Some(self.local_href(&reference.anchor));
        };
        let spec = self
            .config
            .specs
            .iter()
            .find(|spec| spec.shortname() == shortname)?;
        if let Some(format) = spec.format
            && has_rendered_view(format)
            && let Some(path) = spec.source_path()
        {
            let base = self.links.rendered_url(&path);
            return Some(if is_paged(format) {
                anchor_url(&base, &reference.anchor)
            } else {
                format!("{}{}", base, fragment(&reference.anchor))
            });
        }
        Some(format!(
            "{}{}",
            spec.rendered.as_deref()?,
            fragment(&reference.anchor)
        ))
    }

    /// The reference whose range is `range`, if any.
    fn reference_at(&self, range: &Range<usize>) -> Option<&Reference> {
        let i = self
            .references
            .partition_point(|r| r.range.start < range.start);
        self.references[i..]
            .iter()
            .take_while(|r| r.range.start == range.start)
            .find(|r| r.range.end == range.end && !r.def)
    }

    /// Text, with its references' links.
    fn linked(&self, out: &mut HtmlRenderer, range: Range<usize>) {
        let mut pos = range.start;
        let first = self
            .references
            .partition_point(|r| r.range.start < range.start);
        for reference in &self.references[first..] {
            if reference.range.start >= range.end {
                break;
            }
            if reference.range.start < pos || reference.range.end > range.end {
                continue;
            }
            out.text(pos..reference.range.start);
            if reference.def {
                out.w.push("<span");
                if out.w.claim_id(&reference.anchor) {
                    write!(out.w.html, r#" id="{}""#, escape(&reference.anchor)).unwrap();
                }
                out.w.push(r#" class="sp-emu-nt">"#);
                out.text(reference.range.clone());
                out.w.push("</span>");
                pos = reference.range.end;
                continue;
            }
            match self.reference_href(reference).filter(|_| out.in_link == 0) {
                Some(href) => {
                    write!(out.w.html, r#"<a href="{}">"#, escape(&href)).unwrap();
                    out.in_link += 1;
                    out.text(reference.range.clone());
                    out.in_link -= 1;
                    out.w.push("</a>");
                }
                None => out.text(reference.range.clone()),
            }
            pos = reference.range.end;
        }
        out.text(pos..range.end);
    }

    /// Text, with ecmarkdown's formats (unless `emd` is false), and links.
    fn inline(&self, out: &mut HtmlRenderer, range: Range<usize>, emd: bool) {
        let text = &self.doc.src[range.clone()];
        if !emd {
            self.linked(out, range);
            return;
        }
        let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        let byte = |c: usize| range.start + starts.get(c).copied().unwrap_or(text.len());
        let chars: Vec<char> = text.chars().collect();
        let mut pos = range.start;
        for (span, kind) in emd_spans(&chars) {
            // (Fields are text, ex: links to internal methods.)
            if matches!(kind, Emd::Field(_)) {
                continue;
            }
            let (start, end) = (byte(span.start), byte(span.end));
            let delimiter = chars[span.start];
            let (open, close) = match delimiter {
                '_' => ("<var>", "</var>"),
                '*' => (r#"<span class="sp-emu-val">"#, "</span>"),
                '~' => (r#"<span class="sp-emu-const">"#, "</span>"),
                '`' => ("<code>", "</code>"),
                _ => (r#"<span class="sp-emu-nt">"#, "</span>"),
            };
            self.linked(out, pos..start);
            out.w.push(open);
            let width = delimiter.len_utf8();
            self.linked(out, start + width..end - width);
            out.w.push(close);
            pos = end;
        }
        self.linked(out, pos..range.end);
    }

    /// An algorithm's steps, as nested lists, each step a block.
    fn algorithm(&self, out: &mut HtmlRenderer, node: usize) {
        let src = self.doc.src;
        let content = self.doc.content_range(node);
        let steps = algorithm_steps(src, content.clone());
        if steps.starts.is_empty() {
            let opened = out.open_block(self.doc.nodes[node].range.clone(), "");
            let tag = out.start_tag(self, node, "emu-alg");
            out.children(self, node);
            out.end_tag(tag);
            out.w.close_block_div(opened);
            return;
        }
        let tag = out.start_tag(self, node, "emu-alg");
        // The open lists: (indentation, the list's tag).
        let mut open: Vec<(usize, &str)> = vec![];
        for (i, (start, _, id)) in steps.starts.iter().enumerate() {
            let line_start = src[..*start].rfind('\n').map_or(0, |n| n + 1);
            let indent = start - line_start;
            let list = if src[*start..].starts_with("1.") {
                "ol"
            } else {
                "ul"
            };
            while open.last().is_some_and(|&(at, _)| at > indent) {
                let (_, list) = open.pop().unwrap();
                write!(out.w.html, "</li></{}>", list).unwrap();
            }
            if open.last().is_none_or(|&(at, _)| at < indent) {
                open.push((indent, list));
                write!(out.w.html, "<{}>", list).unwrap();
            } else {
                out.w.push("</li>");
            }
            out.w.push("<li");
            if let Some(id) = id
                && out.w.claim_id(id)
            {
                write!(out.w.html, r#" id="{}""#, escape(id)).unwrap();
            }
            out.w.push(">");
            let text_start = steps.markup[i].end;
            let next = steps.starts.get(i + 1).map_or(content.end, |s| s.0);
            let text_end = text_start + src[text_start..next.max(text_start)].trim_end().len();
            let opened = out.open_block(*start..text_end.max(*start + 1), "");
            self.part(out, node, text_start..text_end);
            out.w.close_block_div(opened);
        }
        while let Some((_, list)) = open.pop() {
            write!(out.w.html, "</li></{}>", list).unwrap();
        }
        out.end_tag(tag);
    }

    /// The part of a node's contents in `range` (ex: an algorithm step's):
    /// its text there, and its elements there (or the parts of those which
    /// are partly there).
    fn part(&self, out: &mut HtmlRenderer, node: usize, range: Range<usize>) {
        for &child in &self.doc.nodes[node].children {
            let r = self.doc.nodes[child].range.clone();
            if r.end <= range.start || r.start >= range.end || self.hidden[child] {
                continue;
            }
            match self.doc.nodes[child].kind {
                NodeKind::Text => self.inline(
                    out,
                    r.start.max(range.start)..r.end.min(range.end),
                    !self.no_emd[child],
                ),
                NodeKind::Element(_) if r.start >= range.start && r.end <= range.end => {
                    out.node(self, child)
                }
                NodeKind::Element(_) => self.part(out, child, range.clone()),
                _ => {}
            }
        }
    }

    /// The generated text of an empty `<emu-xref>` to an anchor: its clause's
    /// number (or title, with a `title` attribute), its step's path, or its
    /// production's name.
    fn xref_text(&self, node: usize, reference: &Reference) -> String {
        let anchor = &reference.anchor;
        if let Some(aoid) = self.doc.attr(node, "aoid") {
            return aoid;
        }
        if let Some(shortname) = &reference.spec {
            return self
                .biblios
                .iter()
                .find(|biblio| biblio.shortname() == shortname)
                .and_then(|biblio| biblio.pretty(anchor))
                .unwrap_or(anchor)
                .to_string();
        }
        if let Some(clause) = self
            .clauses
            .values()
            .find(|clause| clause.id.as_deref() == Some(anchor.as_str()))
        {
            return match (&clause.number, self.doc.has_attr(node, "title")) {
                (Some(number), false) => number.clone(),
                _ => clause.title.clone(),
            };
        }
        if let Some(path) = self.step_paths.get(anchor) {
            return format!("step {}", path);
        }
        anchor
            .strip_prefix("prod-")
            .map_or(anchor.as_str(), |name| {
                name.rsplit_once('-').map_or(name, |(_, name)| name)
            })
            .to_string()
    }
}

impl Hooks for Ecmarkup<'_> {
    fn hidden(&self, node: usize) -> bool {
        self.hidden[node]
    }

    fn id(&self, node: usize) -> Option<&str> {
        self.doc.raw_attr(node, "id").filter(|id| !id.contains('&'))
    }

    fn xref(&self, _node: usize) -> Option<&str> {
        None
    }

    fn href(&self, href: &str) -> Option<String> {
        if let Some(id) = href.strip_prefix('#') {
            return Some(self.local_href(&crate::spec_analyze::markup::decode_html(id)));
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
                let range = doc.nodes[node].range.clone();
                self.inline(out, range, !self.no_emd[node]);
                return true;
            }
            NodeKind::Element(name) => name.as_str(),
            _ => return false,
        };
        if let Some(file) = self.imports.get(&node) {
            write!(
                out.w.html,
                r#"<div class="sp-imported">From <a href="{}">{}</a>:</div>"#,
                escape(&self.links.rendered_url(file)),
                escape(file)
            )
            .unwrap();
        }
        if self.clauses.contains_key(&node) {
            let tag = out.start_tag(self, node, name);
            // (Old ids are anchors too.)
            for old in doc
                .attr(node, "oldids")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|old| !old.is_empty())
            {
                if out.w.claim_id(old) {
                    write!(out.w.html, r#"<span id="{}"></span>"#, escape(old)).unwrap();
                }
            }
            out.children(self, node);
            out.end_tag(tag);
            return true;
        }
        if let Some(&clause) = self.headings.get(&node) {
            let clause = &self.clauses[&clause];
            let level = (clause.depth + 2).min(6);
            let opened = out.open_block(doc.nodes[node].range.clone(), "");
            write!(out.w.html, "<h{}>", level).unwrap();
            if let Some(number) = &clause.number {
                write!(out.w.html, r#"<span class="sp-secno">{}</span> "#, number).unwrap();
            }
            out.children(self, node);
            if let Some(url) = self.algorithm_urls.get(&self.headings[&node]) {
                super::algorithm_link(out, url);
            }
            write!(out.w.html, "</h{}>", level).unwrap();
            out.w.close_block_div(opened);
            return true;
        }
        match name {
            "emu-alg" => {
                self.algorithm(out, node);
                true
            }
            "emu-xref" => {
                let content = doc.content_range(node);
                let empty = doc.src[content.clone()].trim().is_empty();
                let start_tag = doc.items[doc.nodes[node].item].range().clone();
                let reference = self.reference_at(if empty { &start_tag } else { &content });
                let href = reference.and_then(|r| self.reference_href(r));
                if let Some(href) = href.as_ref().filter(|_| out.in_link == 0) {
                    write!(out.w.html, r#"<a href="{}">"#, escape(href)).unwrap();
                }
                out.in_link += 1;
                match (empty, reference) {
                    (true, Some(reference)) => out.w.escaped(&self.xref_text(node, reference)),
                    (true, None) => out.w.escaped(
                        &doc.attr(node, "href")
                            .or(doc.attr(node, "aoid"))
                            .unwrap_or_default(),
                    ),
                    (false, _) => out.children(self, node),
                }
                out.in_link -= 1;
                if href.is_some() && out.in_link == 0 {
                    out.w.push("</a>");
                }
                true
            }
            "emu-prodref" => {
                let start_tag = doc.items[doc.nodes[node].item].range().clone();
                let name = doc.attr(node, "name").unwrap_or_default();
                match self
                    .reference_at(&start_tag)
                    .and_then(|r| self.reference_href(r))
                    .filter(|_| out.in_link == 0)
                {
                    Some(href) => write!(
                        out.w.html,
                        r#"<a class="sp-emu-prodref" href="{}">{}</a>"#,
                        escape(&href),
                        escape(&name)
                    )
                    .unwrap(),
                    None => out.w.escaped(&name),
                }
                true
            }
            "emu-table" | "emu-figure" => {
                let tag = out.start_tag(self, node, name);
                if let Some(caption) = self.captions.get(&node) {
                    let opened = out.open_block(
                        doc.items[doc.nodes[node].item].range().clone(),
                        "sp-caption",
                    );
                    out.w.escaped(caption);
                    out.w.close_block_div(opened);
                }
                out.children(self, node);
                out.end_tag(tag);
                true
            }
            _ => false,
        }
    }
}

/// The biblios made last, by the URLs of their specs and their texts' hashes
/// (see `load_biblios`).
static BIBLIOS: LazyLock<Mutex<Vec<((String, u64), Arc<Biblio>)>>> =
    LazyLock::new(|| Mutex::new(vec![]));

/// The biblios that an ecmarkup spec's package.json loads, made from the
/// tree's specs they're of, read with `read` (ex: ECMA-262's, for ECMA-402
/// and proposals).
pub fn load_biblios(
    config: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
    package_json: &str,
) -> Vec<Arc<Biblio>> {
    let wanted = loaded_biblios(package_json);
    // (url, the cache's key's hash, biblio)
    let mut made: Vec<(&str, u64, Arc<Biblio>)> = vec![];
    for url in biblio_urls() {
        // (Each loads the ones before it, as ECMA-402 loads ECMA-262's.)
        if !wanted.contains(&url) && !made.is_empty() {
            continue;
        }
        let Some(spec) = config.specs.iter().find(|spec| {
            spec.format == Some(SpecFormat::Ecmarkup) && spec.rendered.as_deref() == Some(url)
        }) else {
            continue;
        };
        let Some(path) = spec.source_path() else {
            continue;
        };
        // (A spec's biblio is cached by its text, with its imports, and the
        // biblios it loads.)
        let Some(combined) = Combined::build(&path, read, &imports) else {
            continue;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        combined.text.hash(&mut hasher);
        for (_, hash, _) in &made {
            hash.hash(&mut hasher);
        }
        let key = (url.to_string(), hasher.finish());
        let cached = BIBLIOS
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, biblio)| biblio.clone());
        let hash = key.1;
        let biblio = match cached {
            Some(biblio) => biblio,
            None => {
                let loaded: Vec<&Biblio> = made.iter().map(|(_, _, biblio)| &**biblio).collect();
                let Some((_, _, biblio)) =
                    analyze_with_imports(&path, read, spec.shortname(), config, &loaded)
                else {
                    continue;
                };
                let biblio = Arc::new(biblio);
                let mut cache = BIBLIOS.lock().unwrap();
                cache.retain(|(k, _)| k.0 != key.0);
                cache.push((key, biblio.clone()));
                biblio
            }
        };
        made.push((url, hash, biblio));
    }
    made.into_iter()
        .filter(|(url, _, _)| wanted.contains(url))
        .map(|(_, _, biblio)| biblio)
        .collect()
}

/// The package.json of the spec with its source at `path`, in its directory
/// or a parent.
pub fn package_json(path: &str, read: &dyn Fn(&str) -> Option<String>) -> String {
    std::path::Path::new(path)
        .ancestors()
        .skip(1)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| read(&dir.join("package.json").to_string_lossy()))
        .unwrap_or_default()
}

/// The pages of a spec: the first, titled by its metadata's title, and if
/// it's built with `--multipage`, one for each top-level clause.
fn pages(doc: &Doc, clauses: &HashMap<usize, Clause>, multipage: bool) -> Vec<Page> {
    lazy_static! {
        static ref TITLE: Regex = Regex::new(r"(?m)^\s*title:\s*(.+?)\s*$").unwrap();
    }
    let title = (1..doc.nodes.len())
        .find(|&node| doc.is(node, "pre") && doc.attr(node, "class").as_deref() == Some("metadata"))
        .and_then(|node| {
            let text = doc.text_content(node);
            TITLE.captures(&text).map(|c| c[1].to_string())
        })
        .or_else(|| {
            (1..doc.nodes.len())
                .find(|&node| doc.is(node, "title"))
                .map(|node| words(doc, node))
        })
        .unwrap_or_else(|| "Untitled".to_string());
    let mut pages = vec![Page {
        name: None,
        title,
        start: 0,
    }];
    if multipage {
        let mut tops: Vec<(usize, &Clause)> = clauses
            .iter()
            .filter(|(node, clause)| clause.depth == 0 && !doc.is(**node, "emu-intro"))
            .map(|(&node, clause)| (node, clause))
            .collect();
        tops.sort_by_key(|&(node, _)| node);
        for (node, clause) in tops {
            let Some(id) = &clause.id else {
                continue;
            };
            pages.push(Page {
                name: Some(id.strip_prefix("sec-").unwrap_or(id).to_string()),
                title: match &clause.number {
                    Some(number) => format!("{} {}", number, clause.title),
                    None => clause.title.clone(),
                },
                start: doc.nodes[node].range.start,
            });
        }
    }
    pages
}

/// A resolved ecmarkup spec: its source with its imports, its tree, and its
/// references.
struct Resolved {
    combined: crate::spec_analyze::Combined,
    references: Vec<Reference>,
    multipage: bool,
    biblios: Vec<Arc<Biblio>>,
}

fn resolve(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
) -> Option<Resolved> {
    let package = package_json(path, read);
    let biblios = load_biblios(config, read, &package);
    let loaded: Vec<&Biblio> = biblios.iter().map(|biblio| &**biblio).collect();
    let (combined, references) =
        resolve_with_imports(path, read, spec.shortname(), config, &loaded)?;
    let multipage = package.contains("--multipage");
    Some(Resolved {
        combined,
        references,
        multipage,
        biblios,
    })
}

/// The page of a spec with an anchor, if it has the anchor.
pub fn anchor_page(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
    anchor: &str,
) -> Option<Page> {
    let resolved = resolve(path, spec, config, read)?;
    let doc = Doc::parse(&resolved.combined.text);
    let clauses = clauses(&doc);
    let pages = pages(&doc, &clauses, resolved.multipage);
    let at = (1..doc.nodes.len())
        .find(|&node| {
            doc.attr(node, "id").as_deref() == Some(anchor)
                || doc
                    .attr(node, "oldids")
                    .is_some_and(|old| old.split(',').any(|old| old.trim() == anchor))
        })
        .map(|node| doc.nodes[node].range.start)
        .or_else(|| {
            // (Steps' ids are in their algorithms' text.)
            let needle = format!("[id=\"{}\"]", anchor);
            resolved.combined.text.find(&needle)
        })?;
    Some(pages[Page::index_of(&pages, at)].clone())
}

/// The page of a spec with the byte `offset` of its main file.
pub fn offset_page(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
    offset: usize,
) -> Option<Page> {
    let resolved = resolve(path, spec, config, read)?;
    let doc = Doc::parse(&resolved.combined.text);
    let clauses = clauses(&doc);
    let pages = pages(&doc, &clauses, resolved.multipage);
    let at = resolved.combined.text_offset(0, offset).unwrap_or(0);
    Some(pages[Page::index_of(&pages, at)].clone())
}

/// Render the page named `page` (or the first) of the ecmarkup spec `spec`,
/// whose source is at `path` in the tree, among `config`'s specs, reading
/// its imports, its package.json, and the specs whose biblios it loads with
/// `read`.
pub fn render(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    page: Option<&str>,
) -> Option<RenderedSpec> {
    render_view(path, spec, config, links, read, View::Page(page))
}

/// Render the part of the ecmarkup spec `spec` (whose source is at `path`)
/// that the file at `file` has, which the source imports (or a file it
/// imports does), as `render` renders the spec's pages.
pub fn render_file(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    file: &str,
) -> Option<RenderedSpec> {
    render_view(path, spec, config, links, read, View::File(file))
}

/// What a view of a spec is of.
enum View<'a> {
    /// A page, by its name, or the first.
    Page(Option<&'a str>),
    /// An imported file, by its path.
    File(&'a str),
    /// The clause of an algorithm (or abstract operation), by its symbol.
    Algorithm(&'a str),
}

/// Render the clause whose symbol is `sym` (ex: an abstract operation's),
/// with its algorithm's steps' numbers and where they start, for its view
/// beside the code citing them (see `format::format_algorithm`).
pub fn render_algorithm(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    sym: &str,
) -> Option<RenderedSpec> {
    render_view(path, spec, config, links, read, View::Algorithm(sym))
}

fn render_view(
    path: &str,
    spec: &SpecEntry,
    config: &SpecsConfig,
    links: &dyn SpecLinks,
    read: &dyn Fn(&str) -> Option<String>,
    view: View,
) -> Option<RenderedSpec> {
    let resolved = resolve(path, spec, config, read)?;
    let combined = &resolved.combined;
    let doc = Doc::parse(&combined.text);
    let n = doc.nodes.len();
    let clauses = clauses(&doc);
    let pages = pages(&doc, &clauses, resolved.multipage);
    // The page, the file, and the bytes of the source they have.
    let mut algorithm = None;
    let (page, file, range) = match view {
        View::Page(name) => {
            let page = match name {
                Some(name) => pages.iter().position(|p| p.name.as_deref() == Some(name))?,
                None => 0,
            };
            let range =
                pages[page].start..pages.get(page + 1).map_or(combined.text.len(), |p| p.start);
            (Some(page), 0, range)
        }
        View::File(file) => {
            let file = combined.file(file).filter(|&file| file > 0)?;
            (None, file, combined.region(file)?)
        }
        View::Algorithm(sym) => {
            let (&node, _) = clauses.iter().find(|(_, clause)| {
                clause
                    .id
                    .as_deref()
                    .is_some_and(|id| spec_symbol(spec.shortname(), id) == sym)
            })?;
            let range = doc.nodes[node].range.clone();
            let (file, _) = combined.place(&range)?;
            algorithm = Some(node);
            (None, file, range)
        }
    };

    let mut hidden = vec![false; n];
    let mut no_emd = vec![false; n];
    for node in 1..n {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        hidden[node] = hidden[parent]
            || (doc.is(node, "pre") && doc.attr(node, "class").as_deref() == Some("metadata"));
        no_emd[node] = no_emd[parent] || doc.name(node).is_some_and(|name| NO_EMD.contains(&name));
    }
    let headings: HashMap<usize, usize> = clauses
        .keys()
        .filter_map(|&clause| Some((clause_h1(&doc, clause)?, clause)))
        .collect();

    // Ids' places, steps' paths, and captions' numbers.
    let mut id_offsets: HashMap<String, usize> = HashMap::new();
    let mut step_paths: HashMap<String, String> = HashMap::new();
    let mut captions: HashMap<usize, String> = HashMap::new();
    let (mut tables, mut figures) = (0, 0);
    for node in 1..n {
        let start = doc.nodes[node].range.start;
        if let Some(id) = doc.attr(node, "id") {
            id_offsets.entry(id).or_insert(start);
        }
        for old in doc.attr(node, "oldids").unwrap_or_default().split(',') {
            if !old.trim().is_empty() {
                id_offsets.entry(old.trim().to_string()).or_insert(start);
            }
        }
        match doc.name(node) {
            Some("emu-alg") => {
                let steps = algorithm_steps(&combined.text, doc.content_range(node));
                for (step_start, path, id) in steps.starts {
                    if let Some(id) = id {
                        id_offsets.entry(id.clone()).or_insert(step_start);
                        step_paths.insert(id, path);
                    }
                }
            }
            Some(kind @ ("emu-table" | "emu-figure")) => {
                if let Some(caption) = doc.attr(node, "caption") {
                    let (label, count) = if kind == "emu-table" {
                        tables += 1;
                        ("Table", tables)
                    } else {
                        figures += 1;
                        ("Figure", figures)
                    };
                    captions.insert(node, format!("{} {}: {}", label, count, caption));
                }
            }
            _ => {}
        }
    }

    // (Productions' anchors are their left-hand sides'.)
    for reference in resolved.references.iter().filter(|r| r.def) {
        id_offsets
            .entry(reference.anchor.clone())
            .or_insert(reference.range.start);
    }
    // The files imported in the view (but the view's own), whose first
    // elements start with links to their own views.
    let paths: Vec<&str> = combined.paths().collect();
    let mut imports: HashMap<usize, String> = HashMap::new();
    for (imported, region) in combined.regions() {
        if *imported == file || region.start < range.start || region.end > range.end {
            continue;
        }
        let first = (1..n).find(|&node| {
            let r = &doc.nodes[node].range;
            doc.name(node).is_some()
                && r.start >= region.start
                && r.start < region.end
                && doc.nodes[node]
                    .parent
                    .is_none_or(|p| doc.nodes[p].range.start < region.start)
        });
        if let Some(first) = first {
            imports.insert(first, paths[*imported].to_string());
        }
    }
    let biblios: Vec<&Biblio> = resolved.biblios.iter().map(|biblio| &**biblio).collect();
    let hooks = Ecmarkup {
        doc: &doc,
        references: &resolved.references,
        clauses,
        headings,
        hidden,
        no_emd,
        id_offsets,
        step_paths,
        captions,
        pages: pages.clone(),
        view: range.clone(),
        imports,
        base: links.rendered_url(path),
        algorithm_urls: HashMap::new(),
        spec,
        config,
        links,
        biblios: &biblios,
    };

    let mut hooks = hooks;
    hooks.algorithm_urls = hooks
        .clauses
        .iter()
        .filter(|&(&node, _)| {
            doc.nodes[node]
                .children
                .iter()
                .any(|&child| doc.is(child, "emu-alg"))
        })
        .filter_map(|(&node, clause)| {
            let url = links.algorithm_url(&spec_symbol(spec.shortname(), clause.id.as_deref()?))?;
            Some((node, url))
        })
        .collect();
    let map = |range: &Range<usize>| combined.file_range(file, range);
    let mut out = HtmlRenderer::for_file(&doc, range.clone(), combined.source(file), &map);
    if page == Some(0) {
        // (The first page has the title and the table of contents, before
        // the clauses.)
        write!(out.w.html, "<h1>{}</h1>", escape(&pages[0].title)).unwrap();
        let mut entries: Vec<(usize, &Clause)> = hooks
            .clauses
            .iter()
            .filter(|(_, clause)| clause.id.is_some())
            .map(|(&node, clause)| (node, clause))
            .collect();
        entries.sort_by_key(|&(node, _)| node);
        let entries: Vec<(usize, Option<String>, String, String)> = entries
            .into_iter()
            .map(|(_, clause)| {
                (
                    clause.depth + 1,
                    clause.number.clone(),
                    hooks.local_href(clause.id.as_deref().unwrap_or_default()),
                    clause.title.clone(),
                )
            })
            .collect();
        out.w
            .push(r#"<h2 class="sp-no-num">Table of contents</h2>"#);
        write_toc(&mut out, &entries);
    }
    if page.is_none() && algorithm.is_none() {
        // (An imported file's view is a part of the spec.)
        write!(
            out.w.html,
            r#"<div class="sp-imported">Part of <a href="{}">{}</a>, imported by <a href="{}">{}</a>.</div>"#,
            escape(&hooks.base),
            escape(&pages[0].title),
            escape(&hooks.base),
            escape(path)
        )
        .unwrap();
    }
    out.children(&hooks, 0);
    let mut rendered = out.w.finish();
    if let Some(clause) = algorithm {
        // (The steps of its algorithms, but not its clauses'.)
        let mut steps = vec![];
        for node in 1..n {
            let in_clause =
                doc.ancestors(node).find(|a| hooks.clauses.contains_key(a)) == Some(clause);
            if !doc.is(node, "emu-alg") || !in_clause {
                continue;
            }
            for (start, path, _) in algorithm_steps(&combined.text, doc.content_range(node)).starts
            {
                if let Some(at) = combined.file_range(file, &(start..start + 1)) {
                    steps.push((path, at.start));
                }
            }
        }
        let clause = &hooks.clauses[&clause];
        rendered.title = Some(match &clause.number {
            Some(number) => format!("{} {} - {}", number, clause.title, pages[0].title),
            None => format!("{} - {}", clause.title, pages[0].title),
        });
        return Some(RenderedSpec {
            rendered,
            page: Page::index_of(&pages, range.start),
            pages,
            range: combined.file_bounds(file, &range).unwrap_or(0..0),
            steps,
            file: (file > 0).then(|| paths[file].to_string()),
        });
    }
    Some(match page {
        Some(page) => {
            rendered.title = Some(match page {
                0 => pages[0].title.clone(),
                _ => format!("{} - {}", pages[page].title, pages[0].title),
            });
            // (The page's bytes of the main file, for its blame.)
            let file_range = combined.file_bounds(0, &range).unwrap_or(0..0);
            RenderedSpec {
                rendered,
                pages,
                page,
                range: file_range,
                steps: vec![],
                file: None,
            }
        }
        None => {
            // (The first clause's title, else the file's name.)
            let clause_title = hooks
                .clauses
                .iter()
                .filter(|(node, _)| range.contains(&doc.nodes[**node].range.start))
                .min_by_key(|(node, _)| **node)
                .map(|(_, clause)| match &clause.number {
                    Some(number) => format!("{} {}", number, clause.title),
                    None => clause.title.clone(),
                });
            let title = clause_title.unwrap_or_else(|| paths[file].to_string());
            rendered.title = Some(format!("{} - {}", title, pages[0].title));
            RenderedSpec {
                rendered,
                pages: vec![Page {
                    name: None,
                    title,
                    start: 0,
                }],
                page: 0,
                range: 0..combined.source(file).len(),
                steps: vec![],
                file: None,
            }
        }
    })
}

/// The ecmarkup spec, and its source's path, which imports the file at
/// `path` (or a file it imports does), among `config`'s specs, reading their
/// sources with `read`.
pub fn importing_spec<'a>(
    config: &'a SpecsConfig,
    path: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> Option<(&'a SpecEntry, String)> {
    config.specs.iter().find_map(|spec| {
        if spec.format != Some(SpecFormat::Ecmarkup) {
            return None;
        }
        let source = spec.source_path()?;
        let dir = spec.dir();
        if source == path || (!dir.is_empty() && !path.starts_with(&format!("{}/", dir))) {
            return None;
        }
        imported_files(&source, read)
            .contains(&path.to_string())
            .then_some((spec, source))
    })
}

/// The files that an ecmarkup spec's source (at `source`) imports, and that
/// those import.
pub fn imported_files(source: &str, read: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut found: Vec<String> = vec![];
    let mut queue = vec![source.to_string()];
    while let Some(file) = queue.pop() {
        let Some(text) = read(&file) else {
            continue;
        };
        for (_, imported) in crate::spec_analyze::ecmarkup::imports(&file, &text) {
            if imported != source && !found.contains(&imported) && found.len() < 1000 {
                found.push(imported.clone());
                queue.push(imported);
            }
        }
    }
    found
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
            name = "es"
            shortname = "ecmascript"
            format = "ecmarkup"
            rendered = "https://tc39.es/ecma262/"
            "#,
        )
        .unwrap()
    }

    const SPEC: &str = r##"<pre class="metadata">
title: Example
</pre>
<emu-intro id="sec-intro">
  <h1>Introduction</h1>
  <p>Hello.</p>
</emu-intro>
<emu-clause id="sec-values" oldids="sec-old">
  <h1>Values</h1>
  <p>A <dfn id="value">value</dfn> is `code` or *true* or ~empty~.</p>
  <emu-clause id="sec-tostring" type="abstract operation">
    <h1>ToString ( _arg_: a value )</h1>
    <dl class="header"></dl>
    <emu-alg>
      1. If _arg_ is a value, then
        1. [id="step-return"] Return |Thing|.
      1. Return ToString(_arg_).
    </emu-alg>
  </emu-clause>
</emu-clause>
<emu-import href="more.html"></emu-import>
<emu-annex id="sec-grammar">
  <h1>Grammar</h1>
  <emu-grammar type="definition">
    Thing : `thing`
  </emu-grammar>
  <p>See <emu-xref href="#sec-tostring"></emu-xref>, <emu-xref href="#step-return"></emu-xref>, <emu-xref aoid="ToString"></emu-xref>.</p>
</emu-annex>
"##;

    const MORE: &str = r#"<emu-clause id="sec-more">
  <h1>More</h1>
  <p>A value.</p>
</emu-clause>
"#;

    fn read(package: &'static str) -> impl Fn(&str) -> Option<String> {
        move |path: &str| match path {
            "es/spec.html" => Some(SPEC.to_string()),
            "es/more.html" => Some(MORE.to_string()),
            "es/package.json" => Some(package.to_string()),
            _ => None,
        }
    }

    /// The HTML without the runs' spans.
    fn plain(html: &str) -> String {
        let run = Regex::new(r#"<span data-l="\d+" data-c="\d+"(?: data-n="\d+")?>([^<]*)</span>"#)
            .unwrap();
        run.replace_all(html, "$1").into_owned()
    }

    #[test]
    fn test_render() {
        let config = config();
        let spec = &config.specs[0];
        let read = read("{}");
        let rendered = render("es/spec.html", spec, &config, &TestLinks, &read, None).unwrap();
        assert_eq!(rendered.pages.len(), 1);
        assert_eq!(rendered.rendered.title.as_deref(), Some("Example"));
        let html = plain(&rendered.rendered.html);
        for expected in [
            // The table of contents, with clauses' numbers.
            r##"<a href="#sec-intro">Introduction</a>"##,
            r##"<a href="#sec-tostring"><span class="sp-secno">1.1</span> ToString ( arg )</a>"##,
            r##"<a href="#sec-more"><span class="sp-secno">2</span> More</a>"##,
            r##"<a href="#sec-grammar"><span class="sp-secno">A</span> Grammar</a>"##,
            // Clauses, with their old ids, and their headings.
            r##"<div id="sec-values" class="sp-emu-clause"><span id="sec-old"></span><div class="rd-block"><h2><span class="sp-secno">1</span> Values</h2></div>"##,
            r##"<h3><span class="sp-secno">1.1</span> ToString ( <var>arg</var>: a <a href="#value">value</a> )</h3>"##,
            // ecmarkdown's formats.
            r##"is <code>code</code> or <span class="sp-emu-val">true</span> or <span class="sp-emu-const">empty</span>."##,
            // Steps, nested, with their ids.
            r##"<div class="sp-emu-alg"><ol><li><div class="rd-block">If <var>arg</var> is a <a href="#value">value</a>, then</div><ol><li id="step-return"><div class="rd-block">Return <span class="sp-emu-nt"><a href="#prod-Thing">Thing</a></span>.</div></li></ol></li><li><div class="rd-block">Return <a href="#sec-tostring">ToString</a>(<var>arg</var>).</div></li></ol></div>"##,
            // Imported files' content, with a link to their views.
            r##"<div class="sp-imported">From <a href="/t/rendered/es/more.html">es/more.html</a>:</div><div id="sec-more" class="sp-emu-clause">"##,
            // Productions' anchors, and cross-references' generated text.
            r##"<span id="prod-Thing" class="sp-emu-nt">Thing</span> : `thing`"##,
            r##"See <a href="#sec-tostring">1.1</a>, <a href="#step-return">step 1.a</a>, <a href="#sec-tostring">ToString</a>."##,
        ] {
            assert!(
                html.contains(expected),
                "{}\n\nnot in\n\n{}",
                expected,
                html
            );
        }
        // (The imported file's blocks aren't the main file's.)
        let more = html.find(r#"id="sec-more""#).unwrap();
        let grammar = html.find(r#"id="sec-grammar""#).unwrap();
        assert!(
            !html[more..grammar].contains("rd-block"),
            "{}",
            &html[more..grammar]
        );
        assert_eq!(rendered.range, 0..SPEC.len());
    }

    #[test]
    fn test_algorithm() {
        let config = config();
        let spec = &config.specs[0];
        let read = read("{}");
        let view = render_algorithm(
            "es/spec.html",
            spec,
            &config,
            &TestLinks,
            &read,
            "SPEC_ecmascript_sec@2Dtostring",
        )
        .unwrap();
        let html = plain(&view.rendered.html);
        assert!(html.contains("ToString ( <var>arg</var>"), "{}", html);
        assert!(!html.contains("Values</h2>"), "{}", html);
        let steps: Vec<(&str, &str)> = view
            .steps
            .iter()
            .map(|(path, at)| (path.as_str(), &SPEC[*at..*at + 2]))
            .collect();
        assert_eq!(steps, vec![("1", "1."), ("1.a", "1."), ("2", "1.")]);
        assert_eq!(view.file, None);
        assert_eq!(
            view.rendered.title.as_deref(),
            Some("1.1 ToString ( arg ) - Example")
        );
    }

    #[test]
    fn test_pages_and_files() {
        let config = config();
        let spec = &config.specs[0];
        let read = read(r#"{"scripts": {"build": "ecmarkup --multipage spec.html out"}}"#);
        let first = render("es/spec.html", spec, &config, &TestLinks, &read, None).unwrap();
        let names: Vec<Option<&str>> = first.pages.iter().map(|p| p.name.as_deref()).collect();
        assert_eq!(
            names,
            vec![None, Some("values"), Some("more"), Some("grammar")]
        );
        // (The introduction is on the first page.)
        let html = plain(&first.rendered.html);
        assert!(html.contains("Hello."), "{}", html);
        assert!(
            html.contains(r#"<a href="/t/rendered/es/spec.html/values.html#sec-tostring">"#),
            "{}",
            html
        );

        let grammar = render(
            "es/spec.html",
            spec,
            &config,
            &TestLinks,
            &read,
            Some("grammar"),
        )
        .unwrap();
        let html = plain(&grammar.rendered.html);
        assert!(
            html.contains(
                r#"<a href="/t/rendered/es/spec.html/values.html#step-return">step 1.a</a>"#
            ),
            "{}",
            html
        );
        assert_eq!(
            grammar.rendered.title.as_deref(),
            Some("A Grammar - Example")
        );
        assert_eq!(
            anchor_page("es/spec.html", spec, &config, &read, "step-return")
                .and_then(|page| page.name),
            Some("values".to_string())
        );
        assert_eq!(
            anchor_page("es/spec.html", spec, &config, &read, "sec-old").and_then(|page| page.name),
            Some("values".to_string())
        );

        // An imported file's view is its part of the spec.
        let more = render_file(
            "es/spec.html",
            spec,
            &config,
            &TestLinks,
            &read,
            "es/more.html",
        )
        .unwrap();
        let html = plain(&more.rendered.html);
        assert!(
            html.contains(r#"<h2><span class="sp-secno">2</span> More</h2>"#),
            "{}",
            html
        );
        assert!(
            html.contains(r#"A <a href="/t/rendered/es/spec.html/values.html#value">value</a>."#),
            "{}",
            html
        );
        assert_eq!(more.range, 0..MORE.len());
        assert_eq!(more.rendered.title.as_deref(), Some("2 More - Example"));
        assert!(
            more.rendered
                .html
                .contains(r#"<span data-l="3" data-c="5">"#)
        );
        assert_eq!(
            importing_spec(&config, "es/more.html", &read).map(|(_, main)| main),
            Some("es/spec.html".to_string())
        );
        assert!(importing_spec(&config, "es/other.html", &read).is_none());
    }
}
