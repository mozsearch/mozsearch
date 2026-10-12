//! ReSpec specs' sources (an `index.html` with a `respecConfig`, ex: W3C's
//! Payment Request): their definitions and the references to them, with the
//! ids ReSpec gives them when it renders the spec in the browser, offline,
//! by following its modules in their order:
//!
//! - `<dfn>`s are registered by their linking texts (`data-lt`'s, and
//!   `data-abbr`'s abbreviation, else their text as ReSpec shows it, with its
//!   shorthands as their text, and `data-local-lt`'s), case-insensitively, in
//!   document order;
//! - WebIDL blocks' definitions (`<pre class=idl>`) claim the `<dfn>`s with
//!   their names (`name()` first for operations) which are for their parents
//!   (by the closest `data-dfn-for`), and give them `dom-<parent>-<name>`
//!   ids (overloads with `!overload-N`), and those no `<dfn>` defines (but
//!   partial definitions) are defined in their blocks;
//! - the other `<dfn>`s get `dfn-<linking text>` ids, by their linking
//!   texts' order, with ReSpec's `addId`: lowercased, without diacritics,
//!   with each run of non-word characters a `-` and none at the ends, with
//!   an `x` in front if it doesn't start with a letter, and `-0`, `-1`, ...
//!   if it's taken;
//! - sections are ReSpec's (headings outside of `<section>`s make them, and
//!   it generates the conformance section's, the IDL and CDDL indexes', the
//!   index's, the best practices and issue summaries', and the references'
//!   headings), and get ids from their headings' text, and are numbered (ex:
//!   `2.1`, appendices `A`), and their headings get ids from their numbers
//!   and text (ex: `x2-1-the-gizmo-interface`);
//! - references: `[=term=]` and `[=for/term=]`, `{{Name}}` and
//!   `{{Name/member}}`, `<a>`s without `href`s (by their linking texts and
//!   what they're for), `<a href="#id">`s, `[[SPEC]]` (`#bib-spec`),
//!   `[[[#id]]]`, and `data-cite`s (ex: `data-cite="DOM#concept-node"`, of
//!   one of the tree's specs).
//!
//! References to other specs' terms which ReSpec finds by its xref service
//! (ex: `[=tree=]` of DOM's) aren't resolved yet, and are counted.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use super::markup::{Doc, NodeKind, decode_html};
use super::wattsi::step_path;
use super::webidl_defs::{IdlDef, IdlReference, idl_defs, idl_references};
use super::xref::{Export, Query, Terms};
use super::{
    Combined, ForResolver, InContext, Lines, SpecsConfig, dfn_kind, normalize_path, spec_records,
    spec_structured, spec_symbol, unescape_context_name as context_name,
};
use crate::file_format::analysis::Location;
use crate::semantic_analyzer::{Record, sort_records};
use crate::tree_sitter_support::spec_tokenizer::html_contexts;

/// The analysis's counts, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    pub sections: usize,
    pub dfns: usize,
    pub idl: usize,
    pub references: usize,
    pub unresolved: usize,
    pub external: usize,
    /// References to other specs' terms (see `xref`).
    pub xrefs: usize,
}

/// What a ReSpec source's rendered view needs (see `rendered_specs::respec`),
/// by where it is in the analyzed text (with what the source includes): its
/// elements' ids, its sections' numbers and the headings ReSpec generates,
/// its table of contents, and its references' targets.
#[derive(Debug, Default)]
pub struct Rendering {
    /// Elements' ids (definitions', sections', headings'), by their nodes.
    pub ids: HashMap<usize, String>,
    /// The ids of the sections which ReSpec makes from headings, by the
    /// headings' nodes (which an anchor before them has).
    pub section_ids: HashMap<usize, String>,
    /// Headings' numbers (ex: "2.1", "A."), by their nodes.
    pub numbers: HashMap<usize, String>,
    /// The headings ReSpec generates for sections (ex: the conformance
    /// section's), by the sections' nodes.
    pub generated: HashMap<usize, GeneratedHeading>,
    /// The table of contents: its entries' levels, numbers, ids, and text.
    pub toc: Vec<(usize, Option<String>, String, String)>,
    /// References' targets, symbols, by their bytes (shorthands', links',
    /// and IDL blocks' names').
    pub targets: Vec<(Range<usize>, String)>,
    /// The definitions ReSpec makes in IDL blocks, by their bytes.
    pub idl_dfns: Vec<(Range<usize>, String)>,
    /// The specs which `[[SPEC]]`s cite, and their references' ids.
    pub cited: Vec<(String, String)>,
    /// The headings of the references section ReSpec generates, and of its
    /// normative references (whose sections' ids are "references" and
    /// "normative-references").
    pub references: Option<GeneratedHeading>,
    pub normative_references: Option<GeneratedHeading>,
}

/// A heading which ReSpec generates for a section.
#[derive(Clone, Debug, Default)]
pub struct GeneratedHeading {
    pub text: String,
    pub number: Option<String>,
    pub level: usize,
    pub id: Option<String>,
}

/// ReSpec's `toId`: `text` lowercased, without diacritics, with each run of
/// non-word characters a dash, and none at the ends.
fn to_id(text: &str) -> String {
    let lower = text.trim().to_lowercase();
    let mut id = String::new();
    let mut dash = false;
    for c in lower.chars().map(base_letter) {
        if c.is_ascii_alphanumeric() || c == '_' {
            if dash && !id.is_empty() {
                id.push('-');
            }
            dash = false;
            id.push(c);
        } else {
            dash = true;
        }
    }
    id
}

/// ReSpec's `addId`'s id for `text` with `prefix`, given the ids taken.
fn add_id(taken: &mut HashSet<String>, prefix: &str, text: &str) -> String {
    let id = to_id(text);
    let mut id = if id.is_empty() {
        "generatedID".to_string()
    } else if !prefix
        .chars()
        .chain(id.chars())
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
    {
        format!("x{}", id)
    } else {
        id
    };
    if !prefix.is_empty() {
        id = format!("{}-{}", prefix, id);
    }
    if taken.contains(&id) {
        let mut n = 0;
        while taken.contains(&format!("{}-{}", id, n)) {
            n += 1;
        }
        id = format!("{}-{}", id, n);
    }
    taken.insert(id.clone());
    id
}

/// A Latin letter without its diacritic (as ReSpec's NFD normalization and
/// removal of combining marks gives), or the character.
fn base_letter(c: char) -> char {
    const FROM: &str =
        "àáâãäåçèéêëìíîïñòóôõöùúûüýÿāăąćĉċčďēĕėęěĝğġģĥĩīĭįĵķĺļľńņňōŏőŕŗřśŝşšţťũūŭůűųŵŷźżž";
    const TO: &str =
        "aaaaaaceeeeiiiinooooouuuuyyaaaccccdeeeeegggghiiiijklllnnnooorrrssssttuuuuuuwyzzz";
    FROM.chars()
        .zip(TO.chars())
        .find(|(f, _)| *f == c)
        .map_or(c, |(_, t)| t)
}

/// Text with runs of whitespace a space, and none at the ends (ReSpec's
/// `norm`).
fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The plural of a concept's linking text, as ReSpec adds them (roughly).
fn plural(text: &str) -> String {
    if let Some(stem) = text.strip_suffix('y')
        && !stem.ends_with(['a', 'e', 'i', 'o', 'u'])
    {
        return format!("{}ies", stem);
    }
    if text.ends_with(['s', 'x', 'z']) || text.ends_with("ch") || text.ends_with("sh") {
        return format!("{}es", text);
    }
    format!("{}s", text)
}

/// An IDL block's text for parsing, with the same byte offsets: its
/// character references decoded (padded with spaces), and markup (ex: links)
/// blanked.
fn idl_text(text: &str) -> String {
    lazy_static! {
        static ref MARKUP: Regex =
            Regex::new(r"&(?:lt|gt|amp|quot|#[0-9]+|#x[0-9a-fA-F]+);|</?[A-Za-z][^>]*>").unwrap();
    }
    MARKUP
        .replace_all(text, |captures: &regex::Captures| {
            let m = &captures[0];
            let decoded = if m.starts_with('&') {
                decode_html(m)
            } else {
                String::new()
            };
            // (A decoded character no longer than the reference, padded.)
            let decoded = if decoded.len() <= m.len() {
                decoded
            } else {
                String::new()
            };
            format!("{}{}", decoded, " ".repeat(m.len() - decoded.len()))
        })
        .into_owned()
}

lazy_static! {
    /// An enum value's `Enum["value"]`, for its enum.
    static ref ENUM_FOR: Regex = Regex::new(r#"^(\w+)\[""#).unwrap();
    /// ReSpec's inline shorthands: `[[[#id]]]` (and `[[[SPEC]]]`),
    /// `[=for/term|text=]`, `{{Name/member}}`, `[[SPEC]]` (and `[[!SPEC]]`,
    /// `[[SPEC|text]]`), `[^element^]`.
    pub(crate) static ref SHORTHAND: Regex = Regex::new(
        r#"\[\[\[([^\]]+)\]\]\]|\[=([^=\]]+)=\]|\{\{\s*([^}]+?)\s*\}\}|\[\[[!?]?([A-Za-z0-9._-]+)(?:\|[^\]]*)?\]\]|\[\^([^^\]]+)\^\]"#
    )
    .unwrap();
    /// An internal slot's name, ex: `[[state]]` or `[[run]](arg)`.
    static ref SLOT: Regex = Regex::new(r"^\[\[[^\]]+\]\](\(.*\))?$").unwrap();
}

/// The dfn types ReSpec takes from `<dfn>`s' classes.
const CLASS_TYPES: &[&str] = &[
    "abstract-op",
    "attr-value",
    "element",
    "element-attr",
    "element-state",
    "event",
    "http-header",
    "media-type",
    "scheme",
    "permission",
];

/// The IDL types whose definitions' `<dfn>`s needn't be for anything.
const TOP_LEVEL: &[&str] = &[
    "callback interface",
    "callback",
    "dictionary",
    "enum",
    "interface mixin",
    "interface",
    "typedef",
];

/// A definition: a `<dfn>`, or one ReSpec makes in an IDL block for an IDL
/// definition no `<dfn>` defines.
struct Dfn {
    /// Its `<dfn>`, if it's one.
    node: Option<usize>,
    /// Where it is (a `<dfn>`'s content, or the IDL definition's name).
    range: Range<usize>,
    /// Its text (as its `textContent`).
    text: String,
    /// Its (first) linking text, or its IDL definition's name.
    title: String,
    /// What it's for (its `data-dfn-for`'s, or its IDL definition's parent).
    fors: Vec<String>,
    /// The closest `data-dfn-for` (its own or an ancestor's), as IDL
    /// definitions find their `<dfn>`s by.
    closest_for: Option<String>,
    dfn_type: Option<String>,
    id: Option<String>,
    /// Whether an IDL definition defines it.
    idl: bool,
    /// The linking texts other specs' references find it by (see `xref`).
    exported_texts: Vec<String>,
}

impl Dfn {
    fn pretty(&self, shortname: &str) -> String {
        match self.fors.first().filter(|f| !f.is_empty()) {
            Some(f) => format!("{} {}/{}", shortname, f, self.title),
            None => format!("{} {}", shortname, self.title),
        }
    }
}

/// ReSpec's map of linking texts (lowercased) to definitions, in the order
/// they were registered.
#[derive(Default)]
struct Titles {
    order: Vec<String>,
    map: HashMap<String, Vec<usize>>,
}

impl Titles {
    fn register(&mut self, dfn: usize, names: &[String]) {
        for name in names {
            let key = name.to_lowercase();
            if !self.map.contains_key(&key) {
                self.order.push(key.clone());
            }
            let dfns = self.map.entry(key).or_default();
            if !dfns.contains(&dfn) {
                dfns.push(dfn);
            }
        }
    }

    fn get(&self, name: &str) -> &[usize] {
        self.map.get(&name.to_lowercase()).map_or(&[], |dfns| dfns)
    }
}

/// An element's text as ReSpec shows it (see `shown_text`).
fn shown_content(doc: &Doc, node: usize) -> String {
    shown_text(&doc.text_content(node))
}

/// An element's linking texts, as ReSpec's `getDfnTitles` (with core/
/// data-abbr's abbreviations, first).
fn dfn_titles(doc: &Doc, node: usize) -> Vec<String> {
    lazy_static! {
        static ref INITIAL: Regex = Regex::new(r"\b([A-Za-z])").unwrap();
    }
    let mut titles: Vec<String> = vec![];
    let mut add = |title: String| {
        if !title.is_empty() && !titles.contains(&title) {
            titles.push(title);
        }
    };
    let text = shown_content(doc, node);
    let mut lt = doc.attr(node, "data-lt");
    if doc.is(node, "dfn")
        && let Some(abbr) = doc.attr(node, "data-abbr")
    {
        let abbr = if abbr.is_empty() {
            INITIAL
                .captures_iter(&doc.text_content(node))
                .map(|c| c[1].to_uppercase())
                .collect()
        } else {
            abbr
        };
        let mut items: Vec<String> = lt
            .unwrap_or_default()
            .split('|')
            .filter(|item| !item.trim().is_empty())
            .map(str::to_string)
            .collect();
        items.push(abbr);
        lt = Some(items.join("|"));
    }
    let elements: Vec<usize> = doc.nodes[node]
        .children
        .iter()
        .copied()
        .filter(|&c| doc.nodes[c].kind != NodeKind::Comment)
        .collect();
    if let Some(lt) = lt.filter(|lt| !lt.is_empty()) {
        for item in lt.split('|') {
            add(norm(item));
        }
    } else if let [child] = elements[..]
        && doc.is(child, "abbr")
        && let Some(title) = doc.attr(child, "title")
    {
        add(title);
    } else if text == "\"\"" {
        add("the-empty-string".to_string());
    }
    if !doc.has_attr(node, "data-lt-nodefault") {
        add(norm(&text));
    }
    if let Some(local) = doc.attr(node, "data-local-lt") {
        for item in local.split('|') {
            add(norm(item));
        }
    }
    titles
}

/// The definitions by their linking texts, what they're for, and their
/// types, as ReSpec's `link-to-dfn` finds them.
type TitleToDfns = HashMap<(String, String), Vec<(String, usize)>>;

/// The definition a link with `titles`, for `link_for` (the closest
/// `data-link-for`, if there's one), and of `link_type` (ex:
/// "dfn|abstract-op") is to, as ReSpec's `findMatchingDfn` finds it.
fn find_matching(
    t2d: &TitleToDfns,
    titles: &[String],
    link_for: Option<&str>,
    link_type: Option<&str>,
) -> Option<usize> {
    let mut targets: Vec<(String, String)> = vec![];
    let lf = link_for.unwrap_or("");
    for title in titles {
        let mut push = |title: &str| {
            if let [f, t] = title.split('.').collect::<Vec<_>>()[..] {
                targets.push((f.to_string(), t.to_string()));
            }
            targets.push((lf.to_string(), title.to_string()));
            if link_for.is_none() {
                targets.push((title.to_string(), title.to_string()));
            }
            if !lf.is_empty() {
                targets.push((String::new(), title.to_string()));
            }
        };
        push(title);
        // (And without a possessive suffix.)
        if let Some(stripped) = title
            .strip_suffix("'s")
            .or_else(|| title.strip_suffix("\u{2019}s"))
            .filter(|s| !s.is_empty())
        {
            push(stripped);
        }
    }
    let (f, title) = targets
        .iter()
        .find(|(f, title)| t2d.contains_key(&(title.to_lowercase(), f.clone())))?;
    let by_type = &t2d[&(title.to_lowercase(), f.clone())];
    let get = |ty: &str| by_type.iter().find(|(t, _)| t == ty).map(|(_, d)| *d);
    match link_type {
        Some(types) => types.split('|').find_map(get).or_else(|| get("dfn")),
        None => get(if f.is_empty() { "dfn" } else { "idl" }).or_else(|| get("idl")),
    }
}

/// A section, as ReSpec restructures the document: a `<section>`, or one it
/// makes from a heading outside of one, or one it generates (ex: the
/// references).
#[derive(Default)]
struct Section {
    element: Option<usize>,
    heading: Option<Heading>,
    /// Where it is in the document, for ordering generated headings.
    pos: usize,
    intro: bool,
    appendix: bool,
    notoc: bool,
    subs: Vec<usize>,
    id: Option<String>,
    /// Its number (ex: "2.1", "A."), if it's numbered.
    secno: Option<String>,
    /// Its depth, for its heading's level.
    depth: usize,
}

enum Heading {
    Node(usize),
    /// A heading ReSpec generates: its text and level.
    Generated(String, usize),
}

/// A heading's level, if it's a heading.
fn heading_level(doc: &Doc, node: usize) -> Option<usize> {
    match doc.name(node)? {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn has_class(doc: &Doc, node: usize, class: &str) -> bool {
    doc.attr(node, "class")
        .is_some_and(|c| c.split_whitespace().any(|c| c == class))
}

/// Text as ReSpec shows it, with its inline shorthands as their text (ex:
/// "member" for `{{Name/member}}`), and its backticks' code without them.
pub(crate) fn shown_text(text: &str) -> String {
    lazy_static! {
        static ref CODE: Regex = Regex::new(r"`([^`]+)`").unwrap();
    }
    let text = CODE.replace_all(text, "$1");
    SHORTHAND
        .replace_all(&text, |captures: &regex::Captures| {
            if let Some(term) = captures.get(2) {
                let content = term
                    .as_str()
                    .rsplit_once('/')
                    .map_or(term.as_str(), |(_, t)| t);
                content
                    .split_once('|')
                    .map_or(content, |(_, t)| t)
                    .trim()
                    .to_string()
            } else if let Some(idl) = captures.get(3) {
                idl.as_str()
                    .rsplit_once('/')
                    .map_or(idl.as_str(), |(_, t)| t)
                    .to_string()
            } else if let Some(spec) = captures.get(4) {
                format!("[{}]", spec.as_str())
            } else if let Some(element) = captures.get(5) {
                element.as_str().trim().to_string()
            } else {
                captures[0].to_string()
            }
        })
        .into_owned()
}

/// What the `respecConfig` sets which changes ids.
struct Config {
    short_name: Option<String>,
    no_toc: bool,
    max_toc_level: usize,
    /// The specs other specs' terms are found in (`xref`'s, lowercased; all
    /// the tree's, if it's empty), if references find them.
    xref: Option<Vec<String>>,
}

impl Config {
    fn of(src: &str) -> Config {
        lazy_static! {
            static ref SHORT_NAME: Regex =
                Regex::new(r#"\bshortName\s*:\s*["']([^"']+)["']"#).unwrap();
            static ref NO_TOC: Regex = Regex::new(r"\bnoTOC\s*:\s*true").unwrap();
            static ref MAX_TOC_LEVEL: Regex = Regex::new(r"\bmaxTocLevel\s*:\s*(\d+)").unwrap();
            static ref XREF: Regex =
                Regex::new(r#"\bxref\s*:\s*(\[[^\]]*\]|"[^"]*"|'[^']*'|true|false|\{[^}]*\})"#)
                    .unwrap();
            static ref QUOTED: Regex = Regex::new(r#"["']([^"']+)["']"#).unwrap();
        }
        // (`xref: true` (the default), `false`, a profile, specs, or an
        // object with a profile and specs.)
        let xref = match XREF.captures(src).map(|c| c[1].to_string()).as_deref() {
            Some("false") => None,
            None | Some("true") => Some(vec![]),
            Some(value) => Some(
                QUOTED
                    .captures_iter(value)
                    .map(|c| c[1].to_lowercase())
                    .filter(|v| !v.contains([':', '/']))
                    .flat_map(|v| match v.as_str() {
                        "web-platform" => ["html", "infra", "url", "webidl", "dom", "fetch"]
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                        _ => vec![v],
                    })
                    .collect(),
            ),
        };
        Config {
            short_name: SHORT_NAME.captures(src).map(|c| c[1].to_string()),
            no_toc: NO_TOC.is_match(src),
            max_toc_level: MAX_TOC_LEVEL
                .captures(src)
                .and_then(|c| c[1].parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(usize::MAX),
            xref,
        }
    }
}

/// The local scripts a ReSpec source loads (their `src`s, relative to it),
/// which may have its `respecConfig` (ex: WebRTC's `webrtc.js`), but local
/// copies of ReSpec (ex: `respec-w3c-common.js`).
pub fn local_scripts(src: &str) -> Vec<String> {
    let doc = Doc::parse(src);
    (1..doc.nodes.len())
        .filter(|&node| doc.is(node, "script"))
        .filter_map(|node| doc.attr(node, "src"))
        .filter(|s| !s.contains("://") && !s.starts_with("//") && !s.starts_with('/'))
        .filter(|s| !s.rsplit('/').next().unwrap_or(s).starts_with("respec"))
        .collect()
}

/// Whether a source (with the text of the local scripts it loads) is
/// ReSpec's: it has a `respecConfig`, or loads ReSpec.  (Specs' other `.html`
/// sources, ex: css-color-3's `Overview.html`, aren't.)
pub fn is_respec(src: &str, scripts: &str) -> bool {
    if src.contains("respecConfig") || scripts.contains("respecConfig") {
        return true;
    }
    let doc = Doc::parse(src);
    (1..doc.nodes.len())
        .filter(|&node| doc.is(node, "script"))
        .filter_map(|node| doc.attr(node, "src"))
        .any(|src| src.rsplit('/').next().unwrap_or(&src).starts_with("respec"))
}

/// The records of a ReSpec source, of the spec with `shortname`, among
/// `config`'s specs, whose exported definitions are `terms` (see `xref`),
/// with the text of the local scripts it loads (see `local_scripts`).
pub fn analyze(
    src: &str,
    scripts: &str,
    shortname: &str,
    config: &SpecsConfig,
    terms: &Terms,
) -> (Vec<Record>, Stats) {
    let lines = Lines::new(src);
    let place = |range: &Range<usize>| Some((0, lines.location(range)));
    let (mut records, stats, _, _) =
        analyze_placed(src, scripts, shortname, config, terms, 1, &place);
    (records.pop().unwrap_or_default(), stats)
}

/// A ReSpec source's exported definitions, for other specs' references (see
/// `xref`): IDL's, typed ones, and `export`ed ones, but `noexport`ed ones,
/// and those in informative sections.
pub fn exports(src: &str, scripts: &str, shortname: &str, config: &SpecsConfig) -> Vec<Export> {
    let lines = Lines::new(src);
    let place = |range: &Range<usize>| Some((0, lines.location(range)));
    analyze_placed(
        src,
        scripts,
        shortname,
        config,
        &Terms::default(),
        1,
        &place,
    )
    .2
}

/// The records of a ReSpec source at `main` (a path in the tree, which `read`
/// reads), with the files it includes (`data-include`s of HTML, relative to
/// it, which ReSpec puts in the elements, but text and Markdown ones, ex:
/// examples), for each of the files, and its exports (see `exports`).
#[allow(clippy::type_complexity)]
pub fn analyze_with_includes(
    main: &str,
    read: &dyn Fn(&str) -> Option<String>,
    scripts: &str,
    shortname: &str,
    config: &SpecsConfig,
    terms: &Terms,
) -> Option<(Vec<(String, Vec<Record>)>, Stats, Vec<Export>)> {
    let combined = combined(main, read)?;
    let files = combined.paths().count();
    let place = |range: &Range<usize>| combined.place(range);
    let (records, stats, exports, _) = analyze_placed(
        &combined.text,
        scripts,
        shortname,
        config,
        terms,
        files,
        &place,
    );
    let paths = combined.paths().map(str::to_string);
    Some((paths.zip(records).collect(), stats, exports))
}

/// What the rendered view of a ReSpec source at `main` (with what it
/// includes) needs (see `Rendering`), with the source and its includes.
pub fn rendering(
    main: &str,
    read: &dyn Fn(&str) -> Option<String>,
    scripts: &str,
    shortname: &str,
    config: &SpecsConfig,
    terms: &Terms,
) -> Option<(Combined, Rendering)> {
    let combined = combined(main, read)?;
    let files = combined.paths().count();
    let place = |range: &Range<usize>| combined.place(range);
    let (_, _, _, rendering) = analyze_placed(
        &combined.text,
        scripts,
        shortname,
        config,
        terms,
        files,
        &place,
    );
    Some((combined, rendering))
}

/// A ReSpec source at `main` with its HTML includes inlined (see
/// `analyze_with_includes`).
pub fn combined(main: &str, read: &dyn Fn(&str) -> Option<String>) -> Option<Combined> {
    let dir = main.rsplit_once('/').map_or("", |(dir, _)| dir);
    let includes = |_: &str, source: &str| -> Vec<(Range<usize>, String)> {
        let doc = Doc::parse(source);
        let mut includes: Vec<(Range<usize>, String)> = (1..doc.nodes.len())
            .filter(|&node| !doc.ancestors(node).any(|a| doc.has_attr(a, "data-include")))
            .filter_map(|node| {
                let target = doc.attr(node, "data-include")?;
                let format = doc.attr(node, "data-include-format").unwrap_or_default();
                if !(format.is_empty() || format == "html")
                    || target.contains("://")
                    || target.starts_with('/')
                {
                    return None;
                }
                Some((
                    doc.content_range(node),
                    normalize_path(&format!("{}/{}", dir, target)),
                ))
            })
            .collect();
        includes.sort_by_key(|(range, _)| range.start);
        includes
    };
    Combined::build(main, read, &includes)
}

/// The records of a ReSpec source's text (with what it includes), for each
/// of the `files` it's from, by where `place` says its ranges are, and its
/// exports.
#[allow(clippy::type_complexity)]
fn analyze_placed(
    src: &str,
    scripts: &str,
    shortname: &str,
    config: &SpecsConfig,
    terms: &Terms,
    files: usize,
    place: &dyn Fn(&Range<usize>) -> Option<(usize, Location)>,
) -> (Vec<Vec<Record>>, Stats, Vec<Export>, Rendering) {
    let doc = Doc::parse(src);
    let mut rendering = Rendering::default();
    let n = doc.nodes.len();
    let respec = Config::of(&format!("{}\n{}", src, scripts));
    let mut stats = Stats::default();

    // What's left out: scripts, styles, and the head.
    let mut hidden = vec![false; n];
    for node in 1..n {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        hidden[node] = hidden[parent]
            || doc
                .name(node)
                .is_some_and(|name| matches!(name, "script" | "style" | "head" | "template"));
    }
    let shown = || (1..n).filter(|&node| !hidden[node]);
    // The ids taken: the source's, and those of what ReSpec generates.
    let mut taken: HashSet<String> = (1..n)
        .filter(|&node| !hidden[node])
        .filter_map(|node| doc.attr(node, "id"))
        .collect();
    for id in [
        "title",
        "abstract",
        "sotd",
        "toc",
        "back-to-top",
        "references",
        "normative-references",
        "informative-references",
    ] {
        taken.insert(id.to_string());
    }

    // ## `<dfn>`s, registered by their linking texts (core/dfn).
    let mut dfns: Vec<Dfn> = vec![];
    let mut titles = Titles::default();
    for node in shown() {
        if !doc.is(node, "dfn") || doc.ancestors(node).any(|a| doc.is(a, "del")) {
            continue;
        }
        let names = dfn_titles(&doc, node);
        let title = names.first().cloned().unwrap_or_default();
        let local: Vec<String> = doc
            .attr(node, "data-local-lt")
            .unwrap_or_default()
            .split('|')
            .map(norm)
            .collect();
        let exported_texts: Vec<String> = names
            .iter()
            .filter(|n| !local.contains(n))
            .cloned()
            .collect();
        let closest = std::iter::once(node)
            .chain(doc.ancestors(node))
            .find(|&a| doc.has_attr(a, "data-dfn-for"));
        let closest_for = closest.and_then(|a| doc.attr(a, "data-dfn-for"));
        let mut own_for = doc.attr(node, "data-dfn-for");
        let own_type = doc.attr(node, "data-dfn-type");
        let class_type = doc
            .attr(node, "class")
            .unwrap_or_default()
            .split_whitespace()
            .find(|c| CLASS_TYPES.contains(c))
            .map(str::to_string);
        let derived = if class_type.is_some() {
            class_type
        } else if SLOT.is_match(&title) {
            // (Internal slots are for the closest `data-dfn-for`.)
            if closest != Some(node) && closest_for.as_deref().is_some_and(|f| !f.is_empty()) {
                own_for = closest_for.clone();
            }
            Some(
                if title.ends_with(')') {
                    "method"
                } else {
                    "attribute"
                }
                .to_string(),
            )
        } else if own_type.is_none() {
            doc.ancestors(node)
                .find_map(|a| doc.attr(a, "data-dfn-type"))
        } else {
            None
        };
        let fors: Vec<String> = own_for
            .unwrap_or_default()
            .split(',')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect();
        let index = dfns.len();
        dfns.push(Dfn {
            node: Some(node),
            range: doc.content_range(node),
            text: shown_content(&doc, node),
            title,
            fors,
            closest_for,
            dfn_type: own_type.or(derived),
            id: doc.attr(node, "id"),
            idl: false,
            exported_texts,
        });
        titles.register(index, &names);
        stats.dfns += 1;
    }

    // ## WebIDL blocks' definitions: the `<dfn>`s they claim, or theirs
    // (core/webidl).  (IDL definitions' names which link to `<dfn>`s: their
    // bytes and the `<dfn>`s.)
    let mut idl_links: Vec<(Range<usize>, usize)> = vec![];
    // (What `{{Name/member}}`s link to: (parent, name) -> definition.)
    let mut idl_targets: HashMap<(String, String), usize> = HashMap::new();
    let mut overloads: HashMap<String, usize> = HashMap::new();
    // (Their references, ex: arguments' types, by their blocks, and partial
    // definitions' names, which are references to other specs'.)
    let mut idl_type_refs: Vec<(usize, Range<usize>, String, IdlReference)> = vec![];
    let mut idl_partials: Vec<(usize, Range<usize>, String, &'static str)> = vec![];
    for node in shown() {
        if !doc.is(node, "pre")
            || !(has_class(&doc, node, "idl") || has_class(&doc, node, "webidl"))
        {
            continue;
        }
        let content = doc.content_range(node);
        let text = idl_text(&src[content.clone()]);
        for (range, name, kind) in idl_references(&text) {
            idl_type_refs.push((
                node,
                content.start + range.start..content.start + range.end,
                name,
                kind,
            ));
        }
        for def in idl_defs(&text) {
            let range = content.start + def.range.start..content.start + def.range.end;
            let parent = def.parent.clone().unwrap_or_default();
            let mut name = def.name.clone();
            if matches!(def.idl_type, "constructor" | "operation") {
                let count = overloads.entry(format!("{}.{}", parent, name)).or_insert(0);
                if *count > 0 {
                    name = format!("{}!overload-{}", name, count);
                }
                *count += 1;
            }
            let found = find_idl_dfn(&dfns, &mut titles, &def, &name, &parent);
            let dfn = match found {
                Some(dfn) => {
                    if !def.partial {
                        dfns[dfn].dfn_type = Some(def.dfn_type().to_string());
                    }
                    idl_links.push((range.clone(), dfn));
                    dfn
                }
                None => {
                    // (But `[Default] toJSON()`s, which link to WebIDL's
                    // default toJSON steps, and partial definitions.)
                    let member_start = src[..range.start].rfind([';', '{']).map_or(0, |at| at + 1);
                    let default_json = def.idl_type == "operation"
                        && def.name == "toJSON"
                        && src[member_start..range.start].contains("Default");
                    if def.partial {
                        idl_partials.push((node, range.clone(), def.name.clone(), def.dfn_type()));
                    }
                    if default_json || def.partial {
                        continue;
                    }
                    let index = dfns.len();
                    dfns.push(Dfn {
                        node: None,
                        range: range.clone(),
                        text: def.name.clone(),
                        title: def.name.clone(),
                        fors: vec![],
                        closest_for: None,
                        dfn_type: Some(def.dfn_type().to_string()),
                        id: None,
                        idl: true,
                        exported_texts: vec![],
                    });
                    titles.register(index, std::slice::from_ref(&name));
                    stats.idl += 1;
                    index
                }
            };
            // (Decorated: its id, what it's for, and its other names.)
            let d = &mut dfns[dfn];
            if d.id.is_none() {
                let mut last: String = name
                    .to_lowercase()
                    .replace(['(', ')'], "")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join("-");
                if last.is_empty() {
                    last = "the-empty-string".to_string();
                }
                let id = if parent.is_empty() {
                    format!("dom-{}", last)
                } else {
                    format!("dom-{}-{}", parent.to_lowercase(), last)
                };
                taken.insert(id.clone());
                d.id = Some(id);
            }
            d.idl = true;
            d.fors = if parent.is_empty() {
                vec![]
            } else {
                vec![parent.clone()]
            };
            d.closest_for = Some(parent.clone());
            let exported = match def.idl_type {
                "constructor" | "operation" => format!("{}()", name),
                "enum-value" if def.name.is_empty() => String::new(),
                _ => name.clone(),
            };
            if !exported.is_empty() && !d.exported_texts.contains(&exported) {
                d.exported_texts.push(exported);
            }
            let qualified = format!("{}.{}", parent, name);
            let other_names: Vec<String> = match def.idl_type {
                "attribute" => vec![qualified, name.clone()],
                "constructor" | "operation" => vec![
                    qualified.clone(),
                    format!("{}()", qualified),
                    name.clone(),
                    format!("{}()", name),
                ],
                _ => vec![],
            };
            titles.register(dfn, &other_names);
            let key_name = if def.idl_type == "enum-value" && def.name.is_empty() {
                "the-empty-string".to_string()
            } else {
                def.name.clone()
            };
            idl_targets.entry((parent, key_name)).or_insert(dfn);
        }
    }

    // ## The other `<dfn>`s' ids, by their linking texts' order, and the map
    // links find them by (core/link-to-dfn).
    let mut t2d: TitleToDfns = HashMap::new();
    for key in titles.order.clone() {
        for &dfn in titles.get(&key).to_vec().iter() {
            let d = &mut dfns[dfn];
            let ty = d.dfn_type.clone().unwrap_or_else(|| "dfn".to_string());
            let fors = if d.fors.is_empty() {
                vec![String::new()]
            } else {
                d.fors.clone()
            };
            for f in fors {
                // (A duplicate of a definition, of the same type and for the
                // same thing, gets no id by this text.)
                let entry = t2d.entry((key.clone(), f)).or_default();
                if entry.iter().any(|(t, _)| *t == ty) {
                    continue;
                }
                entry.push((ty.clone(), dfn));
                if d.idl || ty != "dfn" {
                    entry.retain(|(t, _)| t != "idl");
                    entry.push(("idl".to_string(), dfn));
                }
                if d.id.is_none() {
                    d.id = Some(add_id(&mut taken, "dfn", &key));
                }
            }
        }
    }
    // (Concepts' plurals link to them too, as ReSpec's pluralize adds them.)
    for key in titles.order.clone() {
        for &dfn in titles.get(&key) {
            let d = &dfns[dfn];
            if d.idl || d.dfn_type.as_deref().is_some_and(|t| t != "dfn") {
                continue;
            }
            let fors = if d.fors.is_empty() {
                vec![String::new()]
            } else {
                d.fors.clone()
            };
            for f in fors {
                t2d.entry((plural(&key), f))
                    .or_insert_with(|| vec![("dfn".to_string(), dfn)]);
            }
        }
    }

    // ## Sections, as ReSpec restructures them (core/sections), with the
    // headings it generates (w3c/abstract, w3c/headers, w3c/conformance,
    // core/webidl-index, core/dfn-index, core/render-biblio).
    let body = (1..n)
        .find(|&node| doc.is(node, "body"))
        .or_else(|| (1..n).find(|&node| doc.is(node, "html")))
        .unwrap_or(0);
    let mut sections: Vec<Section> = vec![];
    let mut cited = false;
    for node in 1..n {
        if !hidden[node] && doc.nodes[node].kind == NodeKind::Text {
            let text = &src[doc.nodes[node].range.clone()];
            cited |= SHORTHAND.captures_iter(text).any(|c| c.get(4).is_some());
        }
    }
    let roots = {
        let classed = |classes: &[&str]| {
            shown().any(|node| classes.iter().any(|class| has_class(&doc, node, class)))
        };
        let mut builder = Builder {
            doc: &doc,
            hidden: &hidden,
            sections: &mut sections,
            practices: classed(&["practicelab"]),
            notes: classed(&["issue", "note", "warning", "ednote"]),
        };
        let mut items: Vec<Item> = doc.nodes[body]
            .children
            .iter()
            .map(|&c| Item::Node(c))
            .collect();
        if cited {
            // (With their ids, as core/render-biblio gives them.)
            let normative = builder.generated("Normative references", 3, usize::MAX, vec![]);
            builder.sections[normative].id = Some("normative-references".to_string());
            let references = builder.generated("References", 1, usize::MAX, vec![normative]);
            builder.sections[references].id = Some("references".to_string());
            builder.sections[references].appendix = true;
            items.push(Item::Section(references));
        }
        builder.structure(items)
    };

    // Their ids and numbers (core/structure).
    set_depths(&mut sections, &roots, false, 0);
    if !respec.no_toc {
        let tree = section_tree(&doc, &mut sections, &roots, &mut taken);
        number_sections(&mut sections, &tree, respec.max_toc_level, "");
        if !tree.is_empty() {
            taken.insert("table-of-contents".to_string());
        }
        toc_entries(&doc, &sections, &tree, 1, &mut rendering.toc);
    }

    // Headings' ids (core/id-headers): those of the sections (but the
    // abstract's and the status of the document's), and any other `<h3>`s to
    // `<h6>`s.
    let mut headings: Vec<(usize, usize, Option<usize>, Option<usize>)> = vec![];
    let mut section_headings: HashSet<usize> = HashSet::new();
    for (index, section) in sections.iter().enumerate() {
        let (pos, node) = match &section.heading {
            Some(Heading::Node(h)) => {
                section_headings.insert(*h);
                (doc.nodes[*h].range.start, Some(*h))
            }
            Some(Heading::Generated(..)) => (section.pos, None),
            None => continue,
        };
        headings.push((pos, headings.len(), Some(index), node));
    }
    for node in shown() {
        if section_headings.contains(&node) {
            continue;
        }
        // (And `<h2>`s in sections but the abstract and the status of the
        // document.)
        let in_section = || {
            doc.ancestors(node).any(|a| {
                doc.is(a, "section")
                    && !has_class(&doc, a, "head")
                    && !matches!(doc.attr(a, "id").as_deref(), Some("abstract" | "sotd"))
            })
        };
        match heading_level(&doc, node) {
            Some(3..) => {}
            Some(2) if in_section() => {}
            _ => continue,
        }
        headings.push((
            doc.nodes[node].range.start,
            headings.len(),
            None,
            Some(node),
        ));
    }
    headings.sort();
    let mut heading_ids: Vec<(usize, String, String)> = vec![];
    // (The generated headings' ids, by their sections.)
    let mut generated_ids: HashMap<usize, String> = HashMap::new();
    for (_, _, section, node) in headings {
        let text = match (section, node) {
            (Some(s), _) => {
                let section = &sections[s];
                // (Sections' headings are `<h2>`s at the top, `<h3>`s in
                // them, ...; but for introductory ones', which keep theirs.)
                let level = match (&section.heading, section.intro) {
                    (Some(Heading::Node(h)), true) => heading_level(&doc, *h).unwrap_or(2),
                    (Some(Heading::Generated(_, level)), true) => *level,
                    _ => (section.depth + 2).min(6),
                };
                let excluded = section
                    .element
                    .and_then(|e| doc.attr(e, "id"))
                    .is_some_and(|id| id == "abstract" || id == "sotd");
                if level < 2 || (level == 2 && excluded) {
                    continue;
                }
                let text = match &section.heading {
                    Some(Heading::Node(h)) => shown_text(&doc.text_content(*h)),
                    Some(Heading::Generated(text, _)) => text.clone(),
                    None => continue,
                };
                match &section.secno {
                    Some(secno) => format!("{} {}", secno, text),
                    None => text,
                }
            }
            (None, Some(h)) => shown_text(&doc.text_content(h)),
            (None, None) => continue,
        };
        let Some(node) = node else {
            let id = add_id(&mut taken, "", &text);
            if let Some(s) = section {
                generated_ids.insert(s, id);
            }
            continue;
        };
        if hidden[node] {
            continue;
        }
        let id = match doc.attr(node, "id") {
            Some(id) => id,
            None => match doc.attr(node, "title") {
                Some(title) => add_id(&mut taken, "", &title),
                None => add_id(&mut taken, "", &text),
            },
        };
        heading_ids.push((
            node,
            id,
            format!(
                "{} {}",
                shortname,
                norm(&shown_text(&doc.text_content(node)))
            ),
        ));
    }

    // ## References.
    struct Pending {
        range: Range<usize>,
        def: bool,
        sym: String,
        what: &'static str,
        pretty: String,
    }
    let mut pending: Vec<Pending> = vec![];
    let mut by_id: HashMap<String, String> = HashMap::new();
    for d in &dfns {
        if let Some(id) = &d.id {
            by_id
                .entry(id.clone())
                .or_insert_with(|| d.pretty(shortname));
        }
    }
    for section in &sections {
        if let (Some(id), Some(Heading::Node(h))) = (&section.id, &section.heading) {
            by_id.entry(id.clone()).or_insert_with(|| {
                format!("{} {}", shortname, norm(&shown_text(&doc.text_content(*h))))
            });
        }
    }
    for (_, id, pretty) in &heading_ids {
        by_id.entry(id.clone()).or_insert_with(|| pretty.clone());
    }
    let local = |id: &str| -> (String, String) {
        (
            spec_symbol(shortname, id),
            by_id
                .get(id)
                .map_or_else(|| format!("{} #{}", shortname, id), |p| p.to_string()),
        )
    };
    let dfn_target = |dfn: usize| -> Option<(String, String)> {
        let d = &dfns[dfn];
        Some((
            spec_symbol(shortname, d.id.as_deref()?),
            d.pretty(shortname),
        ))
    };
    let external = |spec: &str, anchor: &str| -> Option<String> {
        let spec = config.specs.iter().find(|s| {
            s.shortname().eq_ignore_ascii_case(spec) || s.name.eq_ignore_ascii_case(spec)
        })?;
        Some(spec_symbol(spec.shortname(), anchor))
    };
    let closest_link_for = |node: usize| -> Option<String> {
        std::iter::once(node)
            .chain(doc.ancestors(node))
            .find_map(|a| doc.attr(a, "data-link-for"))
    };
    // Other specs' terms (core/xref): the specs each section cites
    // (`[[SPEC]]`s, lowercased), and the specs a reference's context cites,
    // in order: its own `data-cite`'s, or else its ancestors' (the body's
    // with the config's `xref` specs), each with their unversioned names
    // after them, then its section's.
    let mut section_cites: HashMap<usize, Vec<String>> = HashMap::new();
    for node in shown() {
        if doc.nodes[node].kind != NodeKind::Text {
            continue;
        }
        for captures in SHORTHAND.captures_iter(&src[doc.nodes[node].range.clone()]) {
            if let Some(spec) = captures.get(4) {
                for a in doc.ancestors(node).filter(|&a| doc.is(a, "section")) {
                    let cites = section_cites.entry(a).or_default();
                    let spec = spec.as_str().to_lowercase();
                    if !cites.contains(&spec) {
                        cites.push(spec);
                    }
                }
            }
        }
    }
    // (core/webidl adds WebIDL to the specs its blocks' closest elements with
    // `data-cite`s, or else the body, cite.)
    let webidl_cited: HashSet<usize> = shown()
        .filter(|&node| {
            doc.is(node, "pre") && (has_class(&doc, node, "idl") || has_class(&doc, node, "webidl"))
        })
        .map(|node| {
            std::iter::once(node)
                .chain(doc.ancestors(node))
                .find(|&a| doc.has_attr(a, "data-cite"))
                .unwrap_or(body)
        })
        .collect();
    // (For a reference which is `node` itself (an `<a>` or `<dfn>`), or else
    // in it (a shorthand, or an IDL block's).)
    let xref_specs = |node: usize, element: bool| -> Vec<Vec<String>> {
        lazy_static! {
            static ref VERSION: Regex = Regex::new(r"-\d+$").unwrap();
        }
        let config_specs = respec.xref.clone().unwrap_or_default();
        let mut levels: Vec<Vec<String>> = vec![];
        let mut push = |cites: Vec<String>| {
            if cites.is_empty() {
                return;
            }
            let unversioned: Vec<String> = cites
                .iter()
                .map(|c| VERSION.replace(c, "").into_owned())
                .filter(|c| !cites.contains(c))
                .collect();
            levels.push(cites);
            levels.push(unversioned);
        };
        let cites_of = |a: usize| -> Vec<String> {
            let mut cites: Vec<String> = doc
                .attr(a, "data-cite")
                .unwrap_or_default()
                .to_lowercase()
                .replace(['!', '?'], "")
                .split_whitespace()
                .filter(|c| !c.contains('#'))
                .map(str::to_string)
                .collect();
            if a == body {
                cites.extend(config_specs.iter().cloned());
            }
            if webidl_cited.contains(&a) && !cites.iter().any(|c| c == "webidl") {
                cites.push("webidl".to_string());
            }
            cites
        };
        let own = if element { cites_of(node) } else { vec![] };
        if !own.is_empty() && node != body {
            push(own);
        } else {
            for a in std::iter::once(node).chain(doc.ancestors(node)) {
                push(cites_of(a));
            }
            if body == 0 {
                let mut cites = config_specs.clone();
                if webidl_cited.contains(&0) {
                    cites.push("webidl".to_string());
                }
                push(cites);
            }
            if let Some(section) = std::iter::once(node)
                .chain(doc.ancestors(node))
                .find(|&a| doc.is(a, "section"))
            {
                push(section_cites.get(&section).cloned().unwrap_or_default());
            }
        }
        // (Each spec in its first level.)
        let mut seen: HashSet<String> = HashSet::new();
        levels
            .into_iter()
            .map(|level| {
                let mut level: Vec<String> = level
                    .into_iter()
                    .filter(|s| seen.insert(s.clone()))
                    .collect();
                level.sort();
                level
            })
            .filter(|level| !level.is_empty())
            .collect()
    };
    // (A term of another spec's, as `<spec> <for>/<term>`.)
    let xref_find =
        |node: usize, element: bool, term: &str, types: &[&str], dfn_for: Option<&str>| {
            if respec.xref.is_none() || terms.is_empty() || term.is_empty() {
                return None;
            }
            let specs = xref_specs(node, element);
            let found = terms.find(
                &Query {
                    term,
                    types,
                    specs: &specs,
                    dfn_for,
                },
                shortname,
            )?;
            let pretty = match dfn_for {
                Some(f) if !f.is_empty() => format!("{} {}/{}", found.shortname, f, term),
                _ => format!("{} {}", found.shortname, term),
            };
            Some((spec_symbol(&found.shortname, &found.anchor), pretty))
        };
    let mut xrefs = 0;
    // Shorthands, in text outside of `<pre>`s, `<svg>`s, and links.
    for node in shown() {
        if doc.nodes[node].kind != NodeKind::Text {
            continue;
        }
        if doc.ancestors(node).any(|a| {
            doc.name(a)
                .is_some_and(|name| matches!(name, "pre" | "svg" | "a"))
                || has_class(&doc, a, "head")
        }) {
            continue;
        }
        let in_dfn = doc.ancestors(node).any(|a| doc.is(a, "dfn"));
        let range = doc.nodes[node].range.clone();
        let text = &src[range.clone()];
        let parent = doc.nodes[node].parent.unwrap_or(0);
        for captures in SHORTHAND.captures_iter(text) {
            let whole = captures.get(0).unwrap();
            let at = range.start + whole.start()..range.start + whole.end();
            let target: Option<(String, String)> = if let Some(inner) = captures.get(1) {
                let inner = decode_html(inner.as_str());
                let inner = inner.split('|').next().unwrap_or("").trim();
                stats.references += 1;
                inner.strip_prefix('#').map(local)
            } else if let Some(term) = captures.get(2) {
                stats.references += 1;
                let term = decode_html(term.as_str());
                let (f, content) = match term.rsplit_once('/') {
                    Some((f, t)) => (Some(norm(f)), t.to_string()),
                    None => (None, term.clone()),
                };
                let names: Vec<String> = match content.split_once('|') {
                    Some((lt, shown)) => vec![norm(lt), norm(shown)],
                    None => vec![norm(&content)],
                };
                let link_for = f.clone().or_else(|| closest_link_for(parent));
                find_matching(&t2d, &names, link_for.as_deref(), Some("dfn|abstract-op"))
                    .and_then(dfn_target)
                    .or_else(|| {
                        // (Its linking text, or else its text without a
                        // possessive suffix.)
                        let term = match content.split_once('|') {
                            Some((lt, _)) => norm(lt),
                            None => strip_possessive(&norm(&content)),
                        };
                        let found = xref_find(
                            parent,
                            false,
                            &term.to_lowercase(),
                            &["_CONCEPT_"],
                            f.as_deref(),
                        );
                        xrefs += found.is_some() as usize;
                        found
                    })
            } else if let Some(idl_ref) = captures.get(3) {
                if in_dfn || idl_ref.as_str().starts_with('\\') {
                    continue;
                }
                stats.references += 1;
                let (f, name) = split_idl(&decode_html(idl_ref.as_str()));
                idl_link(&t2d, &idl_targets, &f, &name)
                    .and_then(dfn_target)
                    .or_else(|| {
                        let (term, types) = idl_query(&f, &name)?;
                        let enum_for = ENUM_FOR.captures(&name).map(|c| c[1].to_string());
                        let dfn_for = enum_for
                            .as_deref()
                            .or((!f.is_empty()).then_some(f.as_str()));
                        let found = xref_find(parent, false, &term, &types, dfn_for);
                        xrefs += found.is_some() as usize;
                        found
                    })
            } else if let Some(spec) = captures.get(4) {
                if respec
                    .short_name
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(spec.as_str()))
                {
                    continue;
                }
                stats.references += 1;
                Some(local(&format!("bib-{}", to_id(spec.as_str()))))
            } else if let Some(element) = captures.get(5) {
                // (`[^element^]`, `[^element/attribute^]`,
                // `[^element/attribute/value^]`, `[^/global-attribute^]`.)
                stats.references += 1;
                let value = decode_html(element.as_str());
                let value = value.trim();
                let parts: Vec<&str> = value
                    .splitn(3, '/')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .collect();
                let (types, dfn_for, term): (&[&str], Option<String>, &str) =
                    match (value.starts_with('/'), &parts[..]) {
                        (true, [attr, ..]) => (&["element-attr"], None, attr),
                        (false, [el, attr, val]) => {
                            (&["attr-value"], Some(format!("{}/{}", el, attr)), val)
                        }
                        (false, [el, attr]) => (&["element-attr"], Some(el.to_string()), attr),
                        (false, [el]) => (&["element"], None, el),
                        _ => (&[], None, ""),
                    };
                let found = xref_find(parent, false, term, types, dfn_for.as_deref());
                xrefs += found.is_some() as usize;
                found
            } else {
                stats.references += 1;
                None
            };
            match target {
                Some((sym, pretty)) => pending.push(Pending {
                    range: at,
                    def: false,
                    sym,
                    what: "reference",
                    pretty,
                }),
                None => stats.unresolved += 1,
            }
        }
    }
    // Links: without `href`s (by their linking texts, or `data-cite`), and
    // to ids.
    for node in shown() {
        if !doc.is(node, "a") {
            continue;
        }
        let content = doc.content_range(node);
        let range = if content.is_empty() {
            doc.items[doc.nodes[node].item].range().clone()
        } else {
            content
        };
        let cite = doc.attr(node, "data-cite").filter(|c| !c.is_empty());
        let target = if let Some(cite) = cite {
            // (`SPEC#anchor`, of one of the tree's specs.)
            let cite = cite.trim_start_matches(['!', '?']);
            match cite.split_once('#') {
                Some((spec, anchor)) => {
                    match external(spec.split('/').next().unwrap_or(spec), anchor) {
                        Some(sym) => {
                            stats.external += 1;
                            Some((sym, cite.to_string()))
                        }
                        None => None,
                    }
                }
                // (`SPEC`: the term in that spec.)
                None => {
                    let found = xref_find(
                        node,
                        true,
                        &xref_term(&doc, node),
                        &xref_types(&doc, node),
                        xref_for(&doc, node).as_deref(),
                    );
                    xrefs += found.is_some() as usize;
                    found
                }
            }
        } else if let Some(href) = doc.attr(node, "href") {
            match href.strip_prefix('#') {
                Some(id) => Some(local(id)),
                None => config.url_symbol(&href).map(|sym| (sym, href.clone())),
            }
        } else {
            stats.references += 1;
            let names = dfn_titles(&doc, node);
            let link_type = doc.attr(node, "data-link-type");
            let found = find_matching(
                &t2d,
                &names,
                closest_link_for(node).as_deref(),
                link_type.as_deref(),
            )
            .and_then(dfn_target)
            .or_else(|| {
                let found = xref_find(
                    node,
                    true,
                    &xref_term(&doc, node),
                    &xref_types(&doc, node),
                    xref_for(&doc, node).as_deref(),
                );
                xrefs += found.is_some() as usize;
                found
            });
            if found.is_none() {
                stats.unresolved += 1;
            }
            found
        };
        if let Some((sym, pretty)) = target {
            pending.push(Pending {
                range,
                def: false,
                sym,
                what: "reference",
                pretty,
            });
        }
    }
    // Definitions' records.
    for d in &dfns {
        let Some(id) = &d.id else {
            continue;
        };
        pending.push(Pending {
            range: d.range.clone(),
            def: true,
            sym: spec_symbol(shortname, id),
            what: if d.node.is_some() { "dfn" } else { "idl" },
            pretty: d.pretty(shortname),
        });
        // (A `<dfn data-cite>` defers to the other spec's term, by its
        // anchor, or else as ReSpec finds it in the spec.)
        if let Some(node) = d.node
            && let Some(cite) = doc.attr(node, "data-cite").filter(|c| !c.is_empty())
        {
            let sym = match cite.trim_start_matches(['!', '?']).split_once('#') {
                Some((spec, anchor)) => external(spec, anchor),
                None => {
                    let found = xref_find(
                        node,
                        true,
                        &xref_term(&doc, node),
                        &xref_types(&doc, node),
                        xref_for(&doc, node).as_deref(),
                    );
                    xrefs += found.is_some() as usize;
                    found.map(|(sym, _)| sym)
                }
            };
            if let Some(sym) = sym {
                pending.push(Pending {
                    range: d.range.clone(),
                    def: false,
                    sym,
                    what: "dfn",
                    pretty: d.pretty(shortname),
                });
            }
        }
    }

    // (IDL names a `<dfn>` defines are references to it, and so are names of
    // definitions in IDL blocks, ex: types.)
    let type_links = idl_type_refs.iter().filter_map(|(_, range, name, kind)| {
        let target = (*kind == IdlReference::Name)
            .then(|| idl_targets.get(&(String::new(), name.clone())))
            .flatten()?;
        Some((range.clone(), *target))
    });
    let idl_links: Vec<(Range<usize>, usize)> =
        idl_links.iter().cloned().chain(type_links).collect();
    // (And to other specs': their names, built-in types, generic types, and
    // extended attributes, with WebIDL among their blocks' cited specs, and
    // partial definitions' names.)
    let idl_xref = |block: usize, term: &str, types: &[&str], own_cite: Option<&str>| {
        if respec.xref.is_none() || terms.is_empty() {
            return None;
        }
        let specs = match own_cite {
            Some(cite) => vec![vec![cite.to_string()]],
            None => {
                let mut specs = xref_specs(block, false);
                match specs.first_mut() {
                    Some(first) if !first.iter().any(|s| s == "webidl") => {
                        first.push("webidl".to_string());
                        first.sort();
                    }
                    Some(_) => {}
                    None => specs.push(vec!["webidl".to_string()]),
                }
                specs
            }
        };
        let found = terms.find(
            &Query {
                term,
                types,
                specs: &specs,
                dfn_for: None,
            },
            shortname,
        )?;
        Some((
            spec_symbol(&found.shortname, &found.anchor),
            format!("{} {}", found.shortname, term),
        ))
    };
    for (block, range, name, kind) in &idl_type_refs {
        let (types, own_cite): (&[&str], Option<&str>) = match kind {
            IdlReference::Name => {
                if idl_targets.contains_key(&(String::new(), name.clone())) {
                    continue;
                }
                (&["_IDL_"], None)
            }
            IdlReference::Builtin if name == "object" => (&["interface"], Some("webidl")),
            IdlReference::Builtin => (&["_IDL_"], None),
            IdlReference::Generic if name.starts_with(char::is_uppercase) => {
                (&["interface"], Some("webidl"))
            }
            IdlReference::Generic => (&["dfn"], Some("webidl")),
            IdlReference::ExtendedAttribute => (&["extended-attribute"], None),
        };
        if let Some((sym, pretty)) = idl_xref(*block, name, types, own_cite) {
            xrefs += 1;
            pending.push(Pending {
                range: range.clone(),
                def: false,
                sym,
                what: "idl",
                pretty,
            });
        }
    }
    for (block, range, name, dfn_type) in &idl_partials {
        if let Some((sym, pretty)) = idl_xref(*block, name, &[dfn_type], None) {
            xrefs += 1;
            pending.push(Pending {
                range: range.clone(),
                def: false,
                sym,
                what: "idl",
                pretty,
            });
        }
    }
    for (range, dfn) in &idl_links {
        if let Some((sym, pretty)) = dfn_target(*dfn) {
            pending.push(Pending {
                range: range.clone(),
                def: false,
                sym,
                what: "idl",
                pretty,
            });
        }
    }
    for section in &sections {
        let Some(id) = &section.id else {
            continue;
        };
        // (A section ReSpec makes from a heading is where its heading is.)
        let range = match (section.element, &section.heading) {
            (Some(e), _) => doc.items[doc.nodes[e].item].range().clone(),
            (None, Some(Heading::Node(h))) => doc.content_range(*h),
            _ => continue,
        };
        let pretty = match &section.heading {
            Some(Heading::Node(h)) => {
                format!("{} {}", shortname, norm(&shown_text(&doc.text_content(*h))))
            }
            Some(Heading::Generated(text, _)) => format!("{} {}", shortname, text),
            None => format!("{} #{}", shortname, id),
        };
        stats.sections += 1;
        pending.push(Pending {
            range,
            def: true,
            sym: spec_symbol(shortname, id),
            what: "section",
            pretty,
        });
    }
    stats.xrefs = xrefs;
    for (node, id, pretty) in &heading_ids {
        pending.push(Pending {
            range: doc.content_range(*node),
            def: true,
            sym: spec_symbol(shortname, id),
            what: "section",
            pretty: pretty.clone(),
        });
    }

    // What the rendered view needs (see `Rendering`).
    for d in &dfns {
        match (d.node, &d.id) {
            (Some(node), Some(id)) => {
                rendering.ids.insert(node, id.clone());
            }
            (None, Some(id)) => rendering.idl_dfns.push((d.range.clone(), id.clone())),
            _ => {}
        }
    }
    for (index, section) in sections.iter().enumerate() {
        match (&section.heading, section.element) {
            (Some(Heading::Node(h)), element) => {
                if let Some(number) = &section.secno {
                    rendering.numbers.insert(*h, number.clone());
                }
                match (element, &section.id) {
                    (Some(e), Some(id)) => {
                        rendering.ids.insert(e, id.clone());
                    }
                    (None, Some(id)) => {
                        rendering.section_ids.insert(*h, id.clone());
                    }
                    _ => {}
                }
            }
            (Some(Heading::Generated(text, level)), None) => {
                let heading = GeneratedHeading {
                    text: text.clone(),
                    number: section.secno.clone(),
                    level: *level,
                    id: generated_ids.get(&index).cloned(),
                };
                match section.id.as_deref() {
                    Some("references") => rendering.references = Some(heading),
                    Some("normative-references") => rendering.normative_references = Some(heading),
                    _ => {}
                }
            }
            (Some(Heading::Generated(text, level)), Some(e)) => {
                if let Some(id) = &section.id {
                    rendering.ids.insert(e, id.clone());
                }
                rendering.generated.insert(
                    e,
                    GeneratedHeading {
                        text: text.clone(),
                        number: section.secno.clone(),
                        level: *level,
                        id: generated_ids.get(&index).cloned(),
                    },
                );
            }
            _ => {}
        }
    }
    for (node, id, _) in &heading_ids {
        rendering.ids.insert(*node, id.clone());
    }
    for p in &pending {
        if !p.def && p.what != "dfn" {
            rendering.targets.push((p.range.clone(), p.sym.clone()));
        }
    }
    rendering
        .targets
        .sort_by_key(|(range, _)| (range.start, range.end));
    for node in shown() {
        if doc.nodes[node].kind != NodeKind::Text {
            continue;
        }
        for captures in SHORTHAND.captures_iter(&src[doc.nodes[node].range.clone()]) {
            if let Some(spec) = captures.get(4) {
                let id = format!("bib-{}", to_id(spec.as_str()));
                if !rendering.cited.iter().any(|(_, i)| *i == id) {
                    rendering.cited.push((spec.as_str().to_string(), id));
                }
            }
        }
    }
    rendering.cited.sort_by_key(|(spec, _)| spec.to_lowercase());

    // Contexts: the innermost algorithm (by its first dfn) with the step, or
    // the section.
    let contexts = html_contexts(src, &doc.items);
    // (Sections are symbolized by their ids, or else their headings'.)
    let mut heading_syms: HashMap<usize, String> = heading_ids
        .iter()
        .map(|(node, id, _)| (doc.nodes[*node].range.start, spec_symbol(shortname, id)))
        .collect();
    for section in &sections {
        if let (Some(id), Some(Heading::Node(h))) = (&section.id, &section.heading) {
            heading_syms.insert(doc.nodes[*h].range.start, spec_symbol(shortname, id));
        }
    }
    let mut section_contexts: Vec<(usize, InContext)> = contexts
        .iter()
        .filter(|c| c.end.is_none())
        .map(|c| {
            (
                c.start,
                InContext {
                    name: context_name(&c.name),
                    sym: heading_syms.get(&c.start).cloned().unwrap_or_default(),
                },
            )
        })
        .collect();
    section_contexts.sort_by_key(|(start, _)| *start);
    let dfn_at: Vec<(usize, String)> = {
        let mut v: Vec<(usize, String)> = dfns
            .iter()
            .filter_map(|d| {
                let node = d.node?;
                Some((
                    doc.nodes[node].range.start,
                    spec_symbol(shortname, d.id.as_deref()?),
                ))
            })
            .collect();
        v.sort();
        v
    };
    let algorithms: Vec<(Range<usize>, InContext)> = contexts
        .iter()
        .filter_map(|c| {
            let range = c.start..c.end?;
            let first = dfn_at.partition_point(|(start, _)| *start < range.start);
            let sym = dfn_at
                .get(first)
                .filter(|(start, _)| *start < range.end)
                .map_or_else(String::new, |(_, sym)| sym.clone());
            Some((
                range,
                InContext {
                    name: context_name(&c.name),
                    sym,
                },
            ))
        })
        .collect();
    let li_starts: Vec<(usize, usize)> = (1..n)
        .filter(|&node| doc.is(node, "li"))
        .map(|node| (doc.nodes[node].range.start, node))
        .collect();
    let context_at = |pos: usize| -> InContext {
        if let Some((range, context)) = algorithms
            .iter()
            .filter(|(range, _)| range.contains(&pos))
            .min_by_key(|(range, _)| range.len())
        {
            // (The step: the innermost list item with the position.)
            let li = li_starts
                .iter()
                .rev()
                .filter(|(start, node)| *start <= pos && doc.nodes[*node].range.end > pos)
                .map(|(_, node)| *node)
                .next();
            let path = li.map_or_else(String::new, |li| step_path(&doc, li, range.start));
            return if path.is_empty() {
                context.clone()
            } else {
                InContext {
                    name: format!("{} step {}", context.name, path),
                    sym: context.sym.clone(),
                }
            };
        }
        let i = section_contexts.partition_point(|(start, _)| *start <= pos);
        i.checked_sub(1)
            .map_or_else(InContext::default, |i| section_contexts[i].1.clone())
    };

    // Structured records: definitions' kinds and parents.
    let mut resolver = ForResolver::default();
    for d in &dfns {
        let Some(id) = &d.id else {
            continue;
        };
        let kind = dfn_kind(d.dfn_type.as_deref().unwrap_or("dfn"));
        resolver.add(
            &spec_symbol(shortname, id),
            kind,
            std::slice::from_ref(&d.title),
            &d.fors,
        );
    }
    let mut structured: HashMap<String, (&'static str, Option<String>, Option<String>)> =
        HashMap::new();
    for d in &dfns {
        let Some(id) = &d.id else {
            continue;
        };
        let sym = spec_symbol(shortname, id);
        let kind = dfn_kind(d.dfn_type.as_deref().unwrap_or("dfn"));
        let parent = d
            .fors
            .first()
            .and_then(|f| resolver.parent(f, kind))
            .filter(|parent| *parent != sym)
            .map(str::to_string);
        structured.insert(sym, (kind, parent, config.rendered_url(shortname, id)));
    }

    let mut file_records: Vec<Vec<Record>> = (0..files).map(|_| vec![]).collect();
    for p in pending {
        let range = if p.range.is_empty() {
            p.range.start..p.range.start + 1
        } else {
            p.range
        };
        let Some((file, loc)) = place(&range) else {
            continue;
        };
        let records = &mut file_records[file];
        let context = context_at(range.start);
        spec_records(records, loc, p.def, p.what, &p.pretty, &p.sym, &context);
        if p.def
            && let Some((kind, parent, rendered)) = structured.remove(&p.sym)
        {
            spec_structured(
                records,
                loc,
                &p.pretty,
                &p.sym,
                kind,
                parent.as_deref(),
                rendered.as_deref(),
            );
        }
    }
    for records in &mut file_records {
        sort_records(records);
    }

    // Exports (core/dfn's, core/webidl's, and core/dfn-contract's): IDL's,
    // typed ones, and `export`ed ones, but `noexport`ed ones (and those in
    // informative sections, unless they're `export`ed), and those which
    // defer to other specs' (`data-cite`).
    let exports = dfns
        .iter()
        .filter_map(|d| {
            let id = d.id.clone()?;
            let flagged = |attr: &str, class: &str| {
                d.node
                    .is_some_and(|node| doc.has_attr(node, attr) || has_class(&doc, node, class))
            };
            let export = flagged("data-export", "export");
            let informative = d.node.is_some_and(|node| {
                doc.ancestors(node)
                    .find(|&a| {
                        doc.is(a, "section")
                            && (has_class(&doc, a, "informative")
                                || has_class(&doc, a, "normative"))
                    })
                    .is_some_and(|a| has_class(&doc, a, "informative"))
            });
            let noexport = flagged("data-noexport", "no-export") || (informative && !export);
            let cited = d.node.is_some_and(|node| doc.has_attr(node, "data-cite"));
            let dfn_type = d.dfn_type.clone().unwrap_or_else(|| "dfn".to_string());
            let exported = !noexport && (export || d.idl || (dfn_type != "dfn" && !cited));
            (exported && !d.exported_texts.is_empty()).then(|| Export {
                texts: d.exported_texts.clone(),
                dfn_type,
                fors: d.fors.clone(),
                anchor: id,
            })
        })
        .collect();
    (file_records, stats, exports, rendering)
}

/// The table of contents' entries of sections (see `section_tree`), at
/// `level`: their levels, numbers, ids, and headings' text.
fn toc_entries(
    doc: &Doc,
    sections: &[Section],
    list: &[usize],
    level: usize,
    out: &mut Vec<(usize, Option<String>, String, String)>,
) {
    for &s in list {
        let section = &sections[s];
        let text = match &section.heading {
            Some(Heading::Node(h)) => norm(&shown_text(&doc.text_content(*h))),
            Some(Heading::Generated(text, _)) => text.clone(),
            None => continue,
        };
        // (But introductory ones, ex: the abstract.)
        if section.intro {
            continue;
        }
        if let Some(id) = &section.id {
            out.push((level, section.secno.clone(), id.clone(), text));
        }
        toc_entries(doc, sections, &section.subs, level + 1, out);
    }
}

/// A text without a possessive suffix (ex: "gizmo" for "gizmo's").
fn strip_possessive(text: &str) -> String {
    text.strip_suffix("'s")
        .or_else(|| text.strip_suffix("\u{2019}s"))
        .filter(|t| !t.is_empty())
        .unwrap_or(text)
        .to_string()
}

/// An `<a>`'s (or `<dfn>`'s) term for finding other specs' terms
/// (core/xref's `getTermFromElement`): its first linking text, or else its
/// text without a possessive suffix, lowercased unless it's IDL.
fn xref_term(doc: &Doc, node: usize) -> String {
    let idl = doc.has_attr(node, "data-xref-type");
    let term = match doc.attr(node, "data-lt").filter(|lt| !lt.is_empty()) {
        Some(lt) => norm(lt.split('|').next().unwrap_or("")),
        None => strip_possessive(&norm(&shown_content(doc, node))),
    };
    let term = if term == "the-empty-string" {
        String::new()
    } else {
        term
    };
    if idl { term } else { term.to_lowercase() }
}

/// Its types: its `data-xref-type`'s, or concepts'.
fn xref_types(doc: &Doc, node: usize) -> Vec<&'static str> {
    match doc.attr(node, "data-xref-type") {
        Some(types) => types
            .split('|')
            .filter_map(|t| {
                super::xref::IDL_TYPES
                    .iter()
                    .chain(super::xref::CONCEPT_TYPES)
                    .chain(&["_IDL_", "_CONCEPT_"])
                    .find(|known| **known == t.trim())
                    .copied()
            })
            .collect(),
        None => vec!["_CONCEPT_"],
    }
}

/// What it's for: its `data-xref-for`.
fn xref_for(doc: &Doc, node: usize) -> Option<String> {
    doc.attr(node, "data-xref-for").map(|f| norm(&f))
}

/// A `{{...}}`'s parts, as core/inline-idl-parser splits them: what it's
/// for, and its name (ex: ("Gizmo", "poke()") for `{{Gizmo/poke()}}` or
/// `{{Gizmo.poke()}}`).
fn split_idl(text: &str) -> (String, String) {
    lazy_static! {
        static ref METHOD: Regex = Regex::new(r"\.?(\w+\(.*\)$)").unwrap();
    }
    let text = text.trim();
    if text.contains("[[") {
        return match text.split_once('/') {
            Some((f, name)) => (norm(f), name.trim().to_string()),
            None => (String::new(), text.to_string()),
        };
    }
    let (for_part, child) = match METHOD.find(text) {
        Some(m) => (
            &text[..m.start()],
            Some(text[m.start()..].trim_start_matches('.')),
        ),
        None => (text, None),
    };
    let mut tokens: Vec<&str> = for_part
        .split(['.', '/'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    tokens.extend(child);
    match tokens.split_last() {
        Some((name, [])) => (String::new(), name.to_string()),
        Some((name, fors)) => (fors.last().unwrap().to_string(), name.to_string()),
        None => (String::new(), String::new()),
    }
}

/// The term and types of a `{{for/name}}` for finding other specs' terms
/// (core/inline-idl-parser's): an interface's (or other definition's), an
/// attribute's (or dictionary member's or constant's), a method's (with its
/// arguments), an enum value's (`{{Enum/"value"}}`, `{{Enum["value"]}}`),
/// an exception's (`{{"AbortError"}}`), or a primitive type's, but not an
/// internal slot's.
fn idl_query(f: &str, name: &str) -> Option<(String, Vec<&'static str>)> {
    lazy_static! {
        static ref ENUM: Regex = Regex::new(r#"^(\w+)\["([\w\- ]*)"\]$"#).unwrap();
        static ref PRIMITIVE: Regex = Regex::new(r"^[a-z]+(\s+[a-z]+)+\??$").unwrap();
    }
    let name = name.trim();
    if name.starts_with("[[") {
        return None;
    }
    if let Some(value) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        return Some(if f.is_empty() {
            (value.to_string(), vec!["exception"])
        } else {
            (value.to_string(), vec!["enum-value"])
        });
    }
    if let Some(captures) = ENUM.captures(name) {
        return Some((captures[2].to_string(), vec!["enum-value"]));
    }
    if PRIMITIVE.is_match(name) {
        return Some((name.trim_end_matches('?').to_string(), vec!["interface"]));
    }
    if let Some((method, args)) = name.split_once('(') {
        let args: Vec<&str> = args
            .trim_end_matches(')')
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .collect();
        return Some((
            format!("{}({})", method.trim(), args.join(", ")),
            vec!["method"],
        ));
    }
    Some(if f.is_empty() {
        (name.trim_end_matches('?').to_string(), vec!["_IDL_"])
    } else {
        (name.to_string(), vec!["attribute", "dict-member", "const"])
    })
}

/// The `<dfn>` an IDL definition with `name` (with its overload's suffix)
/// and `parent` claims, as ReSpec's `findDfn` finds it.
fn find_idl_dfn(
    dfns: &[Dfn],
    titles: &mut Titles,
    def: &IdlDef,
    name: &str,
    parent: &str,
) -> Option<usize> {
    let names: Vec<String> =
        if matches!(def.idl_type, "constructor" | "operation") && !name.contains("!overload") {
            vec![format!("{}()", name), name.to_string()]
        } else {
            vec![name.to_string()]
        };
    for original in names {
        let resolved = if def.idl_type == "enum-value" && original.is_empty() {
            "the-empty-string".to_string()
        } else {
            original.clone()
        };
        let found = titles.get(&resolved).to_vec();
        let mut matched: Vec<usize> = found
            .iter()
            .copied()
            .filter(|&d| {
                dfns[d].dfn_type.as_deref() != Some("dfn")
                    && dfns[d].closest_for.as_deref() == Some(parent)
            })
            .collect();
        if matched.is_empty() && parent.is_empty() && found.len() == 1 {
            matched = if dfns[found[0]].text == original {
                found.clone()
            } else {
                vec![]
            };
        } else if TOP_LEVEL.contains(&def.idl_type)
            && let Some(&d) = found.iter().find(|&&d| dfns[d].text.trim() == original)
        {
            matched = vec![d];
        }
        // (Or a dotted definition, "parent.name".)
        if matched.is_empty() && !parent.is_empty() {
            let dotted = format!("{}.{}", parent, resolved);
            let alternatives = titles.get(&dotted).to_vec();
            if let [d] = alternatives[..] {
                titles.register(d, &[dotted]);
                matched = vec![d];
            }
        }
        if let Some(&d) = matched.first() {
            return Some(d);
        }
    }
    None
}

/// The definition a `{{for/name}}` links to.
fn idl_link(
    t2d: &TitleToDfns,
    idl_targets: &HashMap<(String, String), usize>,
    f: &str,
    name: &str,
) -> Option<usize> {
    // (An enum value's in quotes; an operation's with its arguments, or
    // not.)
    let name = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        Some("") => "the-empty-string".to_string(),
        Some(value) => value.to_string(),
        None => name.to_string(),
    };
    let bare = name.split('(').next().unwrap_or(&name).trim().to_string();
    if let Some(&d) = idl_targets.get(&(f.to_string(), bare.clone())) {
        return Some(d);
    }
    // (A method's by `name()`, as its `<dfn>` is registered.)
    let names = if name.contains('(') {
        vec![format!("{}()", bare)]
    } else {
        vec![name]
    };
    find_matching(t2d, &names, Some(f), Some("idl"))
}

/// An item in a container ReSpec restructures: a node, or a section it
/// generates.
enum Item {
    Node(usize),
    Section(usize),
}

/// ReSpec's restructuring of a document's sections (core/sections).
struct Builder<'b, 'a> {
    doc: &'b Doc<'a>,
    hidden: &'b [bool],
    sections: &'b mut Vec<Section>,
    /// Whether the document has best practices (`.practicelab`s), and notes
    /// or issues, for their summaries' headings.
    practices: bool,
    notes: bool,
}

impl Builder<'_, '_> {
    fn generated(&mut self, text: &str, level: usize, pos: usize, subs: Vec<usize>) -> usize {
        self.sections.push(Section {
            heading: Some(Heading::Generated(text.to_string(), level)),
            pos,
            subs,
            ..Default::default()
        });
        self.sections.len() - 1
    }

    fn level(&self, section: usize) -> usize {
        match &self.sections[section].heading {
            Some(Heading::Node(h)) => heading_level(self.doc, *h).unwrap_or(1),
            Some(Heading::Generated(_, level)) => *level,
            None => 1,
        }
    }

    /// The sections in a container's items, at its top (ReSpec's
    /// `structure`).
    fn structure(&mut self, items: Vec<Item>) -> Vec<usize> {
        let mut roots: Vec<usize> = vec![];
        // (The sections headings made, by their levels: where the next ones
        // go.)
        let mut stack: Vec<Option<usize>> = vec![None];
        let mut add = |sections: &mut Vec<Section>,
                       stack: &[Option<usize>],
                       position: usize,
                       section: usize| {
            let parent = stack[..position.min(stack.len())]
                .iter()
                .rev()
                .find_map(|s| *s);
            match parent {
                Some(parent) => sections[parent].subs.push(section),
                None => roots.push(section),
            }
        };
        for item in items {
            match item {
                // (ReSpec moves the title and subtitle into the document's
                // header, and removes a best practices summary without best
                // practices.)
                Item::Node(node)
                    if self.hidden[node]
                        || match self.doc.attr(node, "id").as_deref() {
                            Some("title" | "subtitle") => true,
                            Some("bp-summary") => !self.practices,
                            _ => false,
                        } => {}
                Item::Node(node) if heading_level(self.doc, node).is_some() => {
                    let position = heading_level(self.doc, node).unwrap();
                    self.sections.push(Section {
                        heading: Some(Heading::Node(node)),
                        pos: self.doc.nodes[node].range.start,
                        appendix: has_class(self.doc, node, "appendix"),
                        notoc: has_class(self.doc, node, "notoc"),
                        ..Default::default()
                    });
                    let section = self.sections.len() - 1;
                    add(self.sections, &stack, position, section);
                    stack.resize(position + 1, None);
                    stack[position] = Some(section);
                    stack.truncate(position + 1);
                }
                Item::Node(node) if self.doc.is(node, "section") => {
                    let section = self.section(node);
                    let position = self.level(section);
                    add(self.sections, &stack, position, section);
                }
                Item::Section(section) => {
                    let position = self.level(section);
                    add(self.sections, &stack, position, section);
                }
                Item::Node(_) => {}
            }
        }
        roots
    }

    /// A `<section>`, with its heading (its first, or the one ReSpec
    /// generates) and its sections.
    fn section(&mut self, node: usize) -> usize {
        let doc = self.doc;
        let id = doc.attr(node, "id");
        let children: Vec<usize> = doc.nodes[node]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.name(c).is_some())
            .collect();
        let descendants_heading = |levels: &[usize]| -> bool {
            fn any(doc: &Doc, node: usize, levels: &[usize]) -> bool {
                doc.nodes[node].children.iter().any(|&c| {
                    heading_level(doc, c).is_some_and(|l| levels.contains(&l))
                        || any(doc, c, levels)
                })
            }
            any(doc, node, levels)
        };
        // (The first heading in it, as ReSpec's `querySelector`.)
        let first_heading = {
            fn first(doc: &Doc, node: usize) -> Option<usize> {
                doc.nodes[node].children.iter().find_map(|&c| {
                    if heading_level(doc, c).is_some_and(|l| l >= 2) {
                        Some(c)
                    } else {
                        first(doc, c)
                    }
                })
            }
            first(doc, node)
        };
        let generated: Option<(&str, usize)> = match id.as_deref() {
            Some("conformance") if !has_class(doc, node, "override") => Some(("Conformance", 1)),
            Some("abstract") if !descendants_heading(&[2]) => Some(("Abstract", 2)),
            Some("sotd") => Some(("Status of This Document", 2)),
            Some("index") if !descendants_heading(&[1, 2]) => Some(("Index", 1)),
            Some("bp-summary") if self.practices && first_heading.is_none() => {
                Some(("Best Practices Summary", 1))
            }
            Some("issue-summary")
                if self.notes && first_heading.is_none_or(|h| children.first() != Some(&h)) =>
            {
                Some(("Issue summary", 1))
            }
            Some("cddl-index")
                if !children
                    .first()
                    .is_some_and(|&c| heading_level(doc, c).is_some_and(|l| l >= 2)) =>
            {
                Some((
                    if doc.has_attr(node, "title") {
                        ""
                    } else {
                        "CDDL Index"
                    },
                    2,
                ))
            }
            Some("idl-index")
                if !children
                    .iter()
                    .any(|&c| heading_level(doc, c).is_some_and(|l| l >= 2)) =>
            {
                let title = doc.attr(node, "title");
                // (Its title's, if it has one.)
                Some((if title.is_some() { "" } else { "IDL Index" }, 2))
            }
            _ => None,
        };
        let mut items: Vec<Item> = vec![];
        let heading = match generated {
            Some((text, level)) => {
                let text = if text.is_empty() {
                    doc.attr(node, "title").unwrap_or_default()
                } else {
                    text.to_string()
                };
                items.extend(children.iter().map(|&c| Item::Node(c)));
                Some(Heading::Generated(text, level))
            }
            None => {
                let first = children
                    .iter()
                    .copied()
                    .find(|&c| heading_level(doc, c).is_some());
                items.extend(
                    children
                        .iter()
                        .filter(|&&c| Some(c) != first)
                        .map(|&c| Item::Node(c)),
                );
                first.map(Heading::Node)
            }
        };
        if id.as_deref() == Some("index") {
            let end = doc.nodes[node].range.end;
            let here = self.generated("Terms defined by this specification", 3, end, vec![]);
            self.sections[here].id = Some("index-defined-here".to_string());
            let elsewhere = self.generated("Terms defined by reference", 3, end, vec![]);
            self.sections[elsewhere].id = Some("index-defined-elsewhere".to_string());
            items.push(Item::Section(here));
            items.push(Item::Section(elsewhere));
        }
        let subs = self.structure(items);
        let class = |c: &str| has_class(doc, node, c);
        let notoc = class("notoc")
            || doc
                .attr(node, "data-max-toc")
                .is_some_and(|m| m.trim() == "0");
        let max_toc = doc
            .attr(node, "data-max-toc")
            .and_then(|m| m.trim().parse::<usize>().ok())
            .filter(|&m| m > 0);
        self.sections.push(Section {
            element: Some(node),
            heading,
            pos: doc.nodes[node].range.start,
            intro: class("introductory") || matches!(id.as_deref(), Some("abstract" | "sotd")),
            appendix: class("appendix") || id.as_deref() == Some("index"),
            notoc,
            id: id.clone(),
            subs,
            ..Default::default()
        });
        let section = self.sections.len() - 1;
        if let Some(max_toc) = max_toc {
            self.skip_from_toc(section, max_toc);
        }
        section
    }

    /// Marks the sections `depth` levels into a section (and deeper) as not
    /// in the table of contents (`data-max-toc`).
    fn skip_from_toc(&mut self, section: usize, depth: usize) {
        for sub in self.sections[section].subs.clone() {
            if depth <= 1 {
                self.sections[sub].notoc = true;
            } else {
                self.skip_from_toc(sub, depth - 1);
            }
        }
    }
}

/// Sets the sections' depths, and makes those in introductory ones
/// introductory.
fn set_depths(sections: &mut [Section], list: &[usize], intro: bool, depth: usize) {
    for &s in list {
        sections[s].intro |= intro;
        sections[s].depth = depth;
        let subs = sections[s].subs.clone();
        let intro = sections[s].intro;
        set_depths(sections, &subs, intro, depth + 1);
    }
}

/// The sections ReSpec numbers (core/structure's `getSectionTree`), with
/// their ids.
fn section_tree(
    doc: &Doc,
    sections: &mut [Section],
    list: &[usize],
    taken: &mut HashSet<String>,
) -> Vec<usize> {
    let mut tree = vec![];
    for &s in list {
        if sections[s].notoc || sections[s].heading.is_none() {
            continue;
        }
        let title = match &sections[s].heading {
            Some(Heading::Node(h)) => shown_text(&doc.text_content(*h)),
            Some(Heading::Generated(text, _)) => text.clone(),
            None => continue,
        };
        if sections[s].id.is_none() {
            sections[s].id = Some(add_id(taken, "", &title));
        }
        let subs = sections[s].subs.clone();
        sections[s].subs = section_tree(doc, sections, &subs, taken);
        tree.push(s);
    }
    tree
}

/// The sections' numbers (core/structure's `scanSections`).
fn number_sections(sections: &mut [Section], list: &[usize], max_level: usize, prefix: &str) {
    let prefix = if !prefix.is_empty() && !prefix.ends_with('.') {
        format!("{}.", prefix)
    } else {
        prefix.to_string()
    };
    let mut appendix_mode = false;
    let mut last_non_appendix = 0;
    let mut index = 1;
    for &s in list {
        if sections[s].appendix && prefix.is_empty() && !appendix_mode {
            last_non_appendix = index;
            appendix_mode = true;
        }
        let mut secno = if sections[s].intro {
            String::new()
        } else if appendix_mode {
            appendix_number(index - last_non_appendix + 1)
        } else {
            format!("{}{}", prefix, index)
        };
        let level = secno.split('.').count();
        if level == 1 {
            secno.push('.');
        }
        if !sections[s].intro {
            index += 1;
            sections[s].secno = Some(secno.clone());
        }
        if level <= max_level {
            let subs = sections[s].subs.clone();
            number_sections(sections, &subs, max_level, &secno);
        }
    }
}

/// An appendix's letters (ex: "A", ..., "Z", "AA").
fn appendix_number(mut num: usize) -> String {
    let mut s = String::new();
    while num > 0 {
        num -= 1;
        s.insert(0, (b'A' + (num % 26) as u8) as char);
        num /= 26;
    }
    s
}

/// The ids a ReSpec source's elements get (definitions, sections, headings,
/// IDL definitions), for checking them against the rendered spec's.
pub fn ids(src: &str, shortname: &str, config: &SpecsConfig) -> HashSet<String> {
    let (records, _) = analyze(src, "", shortname, config, &Terms::default());
    records
        .iter()
        .filter_map(|record| match record {
            Record::Target(t) if t.data.kind.to_ustr() == "def" => {
                super::spec_anchor(shortname, &t.data.sym)
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test tree's ReSpec spec (tests/tests/files/specs/respec/index.html,
    /// which the tools' build doesn't have).
    const FIXTURE: &str = r##"<!DOCTYPE html>
<html>
<head>
<!-- A small spec in ReSpec's format, for its analysis (see
     tools/src/spec_analyze/respec.rs and tests/tests/specs.toml). -->
<title>Example ReSpec</title>
<script class="remove">
var respecConfig = { shortName: "example-respec", specStatus: "ED" };
</script>
</head>
<body>
<h1 id="title">Example ReSpec</h1>
<h2 id="subtitle">An example</h2>
<section id="abstract"><p>A small spec.</p></section>
<section id="sotd"><p>Its status.</p></section>
<section class="informative">
  <h2>Introduction</h2>
  <p>A <dfn>gizmo</dfn> has a <dfn data-dfn-for="gizmo">state</dfn>, see [[DOM]].</p>
  <p>Its <dfn data-abbr>gizmo state machine</dfn> pokes it.</p>
  <h3>Goals</h3>
  <p>Poking [=gizmos=].</p>
</section>
<section data-dfn-for="Gizmo">
  <h2>The <code>Gizmo</code> interface</h2>
  <pre class="idl">
  [Exposed=Window]
  interface Gizmo {
    constructor(DOMString name);
    readonly attribute DOMString name;
    undefined poke(optional GizmoOptions options = {});
    undefined poke(long times);
  };
  dictionary GizmoOptions {
    boolean force = false;
  };
  enum GizmoState { "", "poked" };
  partial interface Navigator {
    readonly attribute Gizmo gizmo;
  };
  </pre>
  <p>The <dfn>Gizmo</dfn> interface's <dfn>constructor()</dfn> steps are to
  set its <dfn>`name`</dfn> attribute to the [=gizmo/state=] of the
  [=gizmo=].</p>
  <p>The <dfn>poke()</dfn> method steps are:</p>
  <ol>
    <li>Let |options| be the {{GizmoOptions}}.</li>
    <li>If |options|.{{GizmoOptions/force}}, <a>poke</a> it; see <a href="#dfn-gizmo">gizmos</a>.</li>
    <li>Set the <a data-cite="DOM#concept-node">node</a>'s {{Gizmo/name}}, and its {{GizmoState/""}}.</li>
  </ol>
</section>
<section>
  <h2>Poking</h2>
  <p>To <dfn>poke</dfn> a [=gizmo=], call {{Gizmo/poke()}}.</p>
  <p data-cite="examplebs">It's like a {{Gadget}}'s {{Gadget/poke()}}.</p>
</section>
<section id="conformance"></section>
<section class="appendix">
  <h2>Acknowledgements</h2>
  <p>Thanks.</p>
</section>
</body>
</html>
"##;

    fn fixture() -> String {
        FIXTURE.to_string()
    }

    fn config() -> SpecsConfig {
        toml::from_str(
            r#"
            [[spec]]
            name = "example-respec"
            rendered = "https://example.org/respec/"
            [[spec]]
            name = "dom"
            rendered = "https://dom.spec.whatwg.org/"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn test_includes() {
        let read = |file: &str| -> Option<String> {
            Some(
                match file {
                    "spec/index.html" => {
                        "<body><section><h2>Intro</h2><p>A <dfn>gizmo</dfn>.</p></section>\n<section data-include=\"parts/more.html\"></section>\n<pre data-include=\"example.html\" data-include-format=\"text\"></pre>\n<p>See [=widget=].</p></body>\n"
                    }
                    "spec/parts/more.html" => "<h2>More</h2>\n<p>A <dfn>widget</dfn> of a [=gizmo=].</p>\n",
                    "spec/example.html" => "<dfn>example</dfn>",
                    _ => return None,
                }
                .to_string(),
            )
        };
        let (files, _, _) = analyze_with_includes(
            "spec/index.html",
            &read,
            "",
            "example-respec",
            &config(),
            &Terms::default(),
        )
        .unwrap();
        let summary: Vec<(String, Vec<String>)> = files
            .iter()
            .map(|(file, records)| {
                let targets = records
                    .iter()
                    .filter_map(|record| match record {
                        Record::Target(t) => Some(format!(
                            "{}:{} {} {}",
                            t.loc.lineno,
                            t.loc.col_start,
                            t.data.kind.to_ustr(),
                            super::super::spec_anchor("example-respec", &t.data.sym)
                                .unwrap_or_default()
                        )),
                        _ => None,
                    })
                    .collect();
                (file.clone(), targets)
            })
            .collect();
        // (The HTML include's content is the included file's, with its
        // positions there, and its section is the including element; the
        // text one's isn't analyzed.)
        assert_eq!(
            summary,
            vec![
                (
                    "spec/index.html".to_string(),
                    vec![
                        "1:6 def intro".to_string(),
                        "1:19 def x1-intro".to_string(),
                        "1:39 def dfn-gizmo".to_string(),
                        "2:0 def more".to_string(),
                        "4:7 use dfn-widget".to_string(),
                    ]
                ),
                (
                    "spec/parts/more.html".to_string(),
                    vec![
                        "1:4 def x2-more".to_string(),
                        "2:10 def dfn-widget".to_string(),
                        "2:28 use dfn-gizmo".to_string(),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn test_is_respec() {
        assert!(is_respec(FIXTURE, ""));
        assert!(is_respec(
            r#"<script src="https://www.w3.org/Tools/respec/respec-w3c" class="remove"></script>"#,
            ""
        ));
        assert!(is_respec(
            r#"<script src="config.js"></script>"#,
            "var respecConfig = {};"
        ));
        assert!(!is_respec(
            "<html><body><h1>CSS Color</h1></body></html>",
            ""
        ));
    }

    #[test]
    fn test_add_id() {
        let mut taken = HashSet::new();
        assert_eq!(
            add_id(&mut taken, "dfn", "Gizmo's  state"),
            "dfn-gizmo-s-state"
        );
        assert_eq!(
            add_id(&mut taken, "dfn", "gizmo’s state"),
            "dfn-gizmo-s-state-0"
        );
        assert_eq!(add_id(&mut taken, "", "1.2 Café"), "x1-2-cafe");
        assert_eq!(add_id(&mut taken, "", "A. Références"), "a-references");
        assert_eq!(add_id(&mut taken, "dfn", "()"), "dfn-generatedID");
        assert_eq!(appendix_number(1), "A");
        assert_eq!(appendix_number(27), "AA");
    }

    #[test]
    fn test_ids() {
        let mut ids: Vec<String> = ids(&fixture(), "example-respec", &config())
            .into_iter()
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "a-acknowledgements",
                "abstract",
                "acknowledgements",
                "conformance",
                "dfn-gizmo",
                // (`data-abbr`'s abbreviation is its first linking text.)
                "dfn-gsm",
                "dfn-poke",
                "dfn-state",
                "dom-gizmo",
                "dom-gizmo-constructor",
                "dom-gizmo-name",
                "dom-gizmo-poke",
                // (An overload no `<dfn>` defines is defined in its block.)
                "dom-gizmo-poke!overload-1",
                "dom-gizmooptions",
                "dom-gizmooptions-force",
                "dom-gizmostate",
                "dom-gizmostate-poked",
                "dom-gizmostate-the-empty-string",
                // (A partial interface's member, but not the interface.)
                "dom-navigator-gizmo",
                // (The `<h3>` makes a section in the introduction.)
                "goals",
                "introduction",
                "poking",
                "sotd",
                "the-gizmo-interface",
                "x1-1-goals",
                "x1-introduction",
                "x2-the-gizmo-interface",
                "x3-poking",
            ]
        );
    }

    #[test]
    fn test_references() {
        let src = fixture();
        let (records, stats) = analyze(&src, "", "example-respec", &config(), &Terms::default());
        let lines: Vec<&str> = src.lines().collect();
        let uses: Vec<String> = records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) if t.data.kind.to_ustr() == "use" => {
                    let line = lines[t.loc.lineno as usize - 1];
                    let text = &line[t.loc.col_start as usize..t.loc.col_end as usize];
                    let anchor = super::super::spec_anchor("example-respec", &t.data.sym)
                        .unwrap_or_else(|| t.data.sym.to_string());
                    Some(format!("{} {}", text, anchor))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            uses,
            [
                "[[DOM]] bib-dom",
                "[=gizmos=] dfn-gizmo",
                // (The IDL block's names, of the `<dfn>`s which define them.)
                "Gizmo dom-gizmo",
                "constructor dom-gizmo-constructor",
                "name dom-gizmo-name",
                "poke dom-gizmo-poke",
                // (And names of definitions in IDL blocks, ex: types.)
                "GizmoOptions dom-gizmooptions",
                "Gizmo dom-gizmo",
                "[=gizmo/state=] dfn-state",
                "[=gizmo=] dfn-gizmo",
                "{{GizmoOptions}} dom-gizmooptions",
                "{{GizmoOptions/force}} dom-gizmooptions-force",
                "poke dfn-poke",
                "gizmos dfn-gizmo",
                "node SPEC_dom_concept@2Dnode",
                "{{Gizmo/name}} dom-gizmo-name",
                "{{GizmoState/\"\"}} dom-gizmostate-the-empty-string",
                "[=gizmo=] dfn-gizmo",
                "{{Gizmo/poke()}} dom-gizmo-poke",
            ]
        );
        // (Without the other specs' terms, its references to them don't
        // resolve; see `test_xrefs`.)
        assert_eq!((stats.dfns, stats.idl, stats.unresolved), (8, 7, 2));
    }

    fn export(text: &str, dfn_type: &str, fors: &[&str], anchor: &str) -> Export {
        Export {
            texts: vec![text.to_string()],
            dfn_type: dfn_type.to_string(),
            fors: fors.iter().map(|f| f.to_string()).collect(),
            anchor: anchor.to_string(),
        }
    }

    #[test]
    fn test_xrefs() {
        let config: SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "dom"
            [[spec]]
            name = "html"
            [[spec]]
            name = "webidl"
            [[spec]]
            name = "examplebs"
            "#,
        )
        .unwrap();
        let mut terms = Terms::default();
        let specs: HashMap<&str, &crate::spec_analyze::SpecEntry> =
            config.specs.iter().map(|s| (s.name.as_str(), s)).collect();
        terms.add(
            specs["dom"],
            &[
                export("tree", "dfn", &[], "concept-tree"),
                export("Node", "interface", &[], "interface-node"),
                export("EventTarget", "interface", &[], "eventtarget"),
            ],
        );
        terms.add(
            specs["html"],
            &[
                export("iframe", "element", &[], "the-iframe-element"),
                export("allow", "element-attr", &["iframe"], "attr-iframe-allow"),
                export("in parallel", "dfn", &[], "in-parallel"),
            ],
        );
        terms.add(
            specs["webidl"],
            &[
                export("DOMString", "interface", &[], "idl-DOMString"),
                export("unsigned long", "interface", &[], "idl-unsigned-long"),
                export("Promise", "interface", &[], "idl-promise"),
                export("Exposed", "extended-attribute", &[], "Exposed"),
                export("AbortError", "exception", &[], "aborterror"),
            ],
        );
        terms.add(
            specs["examplebs"],
            &[
                export("Gadget", "interface", &[], "gadget"),
                export("poke()", "method", &["Gadget"], "dom-gadget-poke"),
            ],
        );
        let uses = |src: &str| -> Vec<String> {
            let (records, _) = analyze(src, "", "example-respec", &config, &terms);
            let lines: Vec<&str> = src.lines().collect();
            records
                .iter()
                .filter_map(|record| match record {
                    Record::Target(t)
                        if t.data.kind.to_ustr() == "use"
                            && !t.data.sym.starts_with("SPEC_example@2Drespec_") =>
                    {
                        let line = lines[t.loc.lineno as usize - 1];
                        let text = &line[t.loc.col_start as usize..t.loc.col_end as usize];
                        Some(format!("{} {}", text, t.data.sym))
                    }
                    _ => None,
                })
                .collect()
        };
        let src = r#"<html><head><script>var respecConfig = { xref: "web-platform" };</script></head><body>
<section><h2>Uses</h2>
<p>A [=tree=]'s {{Node}}, an [^iframe^]'s [^iframe/allow^], <a>in parallel</a>, {{"AbortError"}}.</p>
<pre class="idl">
[Exposed=Window]
interface Thing : EventTarget {
  Promise&lt;undefined&gt; run(DOMString s, unsigned long n);
};
</pre>
<p data-cite="examplebs">Unlike a {{Gadget}}'s {{Gadget/poke()}}, a [=tree=] isn't its spec's.</p>
</section></body></html>
"#;
        assert_eq!(
            uses(src),
            [
                "[=tree=] SPEC_dom_concept@2Dtree",
                "{{Node}} SPEC_dom_interface@2Dnode",
                "[^iframe^] SPEC_html_the@2Diframe@2Delement",
                "[^iframe/allow^] SPEC_html_attr@2Diframe@2Dallow",
                "in parallel SPEC_html_in@2Dparallel",
                "{{\"AbortError\"}} SPEC_webidl_aborterror",
                // (The IDL block's references, with WebIDL among its cited
                // specs.)
                "Exposed SPEC_webidl_Exposed",
                "EventTarget SPEC_dom_eventtarget",
                "Promise SPEC_webidl_idl@2Dpromise",
                "DOMString SPEC_webidl_idl@2DDOMString",
                "unsigned long SPEC_webidl_idl@2Dunsigned@2Dlong",
                // (The paragraph's cited spec first, then the body's.)
                "{{Gadget}} SPEC_examplebs_gadget",
                "{{Gadget/poke()}} SPEC_examplebs_dom@2Dgadget@2Dpoke",
                "[=tree=] SPEC_dom_concept@2Dtree",
            ]
        );
        // (The specs a context cites are the only ones searched: here, the
        // body cites only WebIDL, for the IDL block.)
        let without_profile = src.replace(r#"{ xref: "web-platform" }"#, "{}");
        assert_eq!(
            uses(&without_profile)
                .iter()
                .filter(|u| u.contains("tree"))
                .count(),
            0
        );
        // (And none with `xref: false`.)
        let off = src.replace(r#""web-platform""#, "false");
        assert!(uses(&off).is_empty());
        // (The test tree's fixture's: its IDL block's, its `data-cite`d
        // anchor, and the Bikeshed fixture's terms.)
        assert_eq!(
            uses(FIXTURE),
            [
                "Exposed SPEC_webidl_Exposed",
                "DOMString SPEC_webidl_idl@2DDOMString",
                "DOMString SPEC_webidl_idl@2DDOMString",
                "node SPEC_dom_concept@2Dnode",
                "{{Gadget}} SPEC_examplebs_gadget",
                "{{Gadget/poke()}} SPEC_examplebs_dom@2Dgadget@2Dpoke",
            ]
        );
    }
}
