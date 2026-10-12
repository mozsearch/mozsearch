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
use super::webidl_defs::{IdlDef, idl_defs, idl_references};
use super::{
    ForResolver, InContext, Lines, SpecsConfig, dfn_kind, spec_records, spec_structured,
    spec_symbol, unescape_context_name as context_name,
};
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
    /// ReSpec's inline shorthands: `[[[#id]]]` (and `[[[SPEC]]]`),
    /// `[=for/term|text=]`, `{{Name/member}}`, `[[SPEC]]` (and `[[!SPEC]]`,
    /// `[[SPEC|text]]`), `[^element^]`.
    static ref SHORTHAND: Regex = Regex::new(
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
fn shown_text(text: &str) -> String {
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
}

impl Config {
    fn of(src: &str) -> Config {
        lazy_static! {
            static ref SHORT_NAME: Regex =
                Regex::new(r#"\bshortName\s*:\s*["']([^"']+)["']"#).unwrap();
            static ref NO_TOC: Regex = Regex::new(r"\bnoTOC\s*:\s*true").unwrap();
            static ref MAX_TOC_LEVEL: Regex = Regex::new(r"\bmaxTocLevel\s*:\s*(\d+)").unwrap();
        }
        Config {
            short_name: SHORT_NAME.captures(src).map(|c| c[1].to_string()),
            no_toc: NO_TOC.is_match(src),
            max_toc_level: MAX_TOC_LEVEL
                .captures(src)
                .and_then(|c| c[1].parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(usize::MAX),
        }
    }
}

/// The records of a ReSpec source, of the spec with `shortname`, among
/// `config`'s specs.
pub fn analyze(src: &str, shortname: &str, config: &SpecsConfig) -> (Vec<Record>, Stats) {
    let doc = Doc::parse(src);
    let n = doc.nodes.len();
    let respec = Config::of(src);
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
    // (Their references to definitions by name, ex: arguments' types.)
    let mut idl_type_refs: Vec<(Range<usize>, String)> = vec![];
    for node in shown() {
        if !doc.is(node, "pre")
            || !(has_class(&doc, node, "idl") || has_class(&doc, node, "webidl"))
        {
            continue;
        }
        let content = doc.content_range(node);
        let text = idl_text(&src[content.clone()]);
        for (range, name) in idl_references(&text) {
            idl_type_refs.push((content.start + range.start..content.start + range.end, name));
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
            let normative = builder.generated("Normative references", 3, usize::MAX, vec![]);
            let references = builder.generated("References", 1, usize::MAX, vec![normative]);
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
            add_id(&mut taken, "", &text);
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
                let link_for = f.or_else(|| closest_link_for(parent));
                find_matching(&t2d, &names, link_for.as_deref(), Some("dfn|abstract-op"))
                    .and_then(dfn_target)
            } else if let Some(idl_ref) = captures.get(3) {
                if in_dfn || idl_ref.as_str().starts_with('\\') {
                    continue;
                }
                stats.references += 1;
                let idl_ref = decode_html(idl_ref.as_str());
                let (f, name) = match idl_ref.rsplit_once('/') {
                    Some((f, name)) => (norm(f), name.trim().to_string()),
                    None => (String::new(), idl_ref.trim().to_string()),
                };
                idl_link(&t2d, &idl_targets, &f, &name).and_then(dfn_target)
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
                None => None,
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
            .and_then(dfn_target);
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
        // (A `<dfn data-cite>` defers to the other spec's term.)
        if let Some(node) = d.node
            && let Some(cite) = doc.attr(node, "data-cite")
            && let Some((spec, anchor)) = cite.trim_start_matches('!').split_once('#')
            && let Some(sym) = external(spec, anchor)
        {
            pending.push(Pending {
                range: d.range.clone(),
                def: false,
                sym,
                what: "dfn",
                pretty: d.pretty(shortname),
            });
        }
    }
    // (IDL names a `<dfn>` defines are references to it, and so are names of
    // definitions in IDL blocks, ex: types.)
    let type_links = idl_type_refs.iter().filter_map(|(range, name)| {
        Some((
            range.clone(),
            *idl_targets.get(&(String::new(), name.clone()))?,
        ))
    });
    let idl_links: Vec<(Range<usize>, usize)> =
        idl_links.iter().cloned().chain(type_links).collect();
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
    for (node, id, pretty) in &heading_ids {
        pending.push(Pending {
            range: doc.content_range(*node),
            def: true,
            sym: spec_symbol(shortname, id),
            what: "section",
            pretty: pretty.clone(),
        });
    }

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

    let lines = Lines::new(src);
    let mut records = vec![];
    for p in pending {
        let range = if p.range.is_empty() {
            p.range.start..p.range.start + 1
        } else {
            p.range
        };
        let loc = lines.location(&range);
        let context = context_at(range.start);
        spec_records(
            &mut records,
            loc,
            p.def,
            p.what,
            &p.pretty,
            &p.sym,
            &context,
        );
        if p.def
            && let Some((kind, parent, rendered)) = structured.remove(&p.sym)
        {
            spec_structured(
                &mut records,
                loc,
                &p.pretty,
                &p.sym,
                kind,
                parent.as_deref(),
                rendered.as_deref(),
            );
        }
    }
    sort_records(&mut records);
    (records, stats)
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
    let names = if name.contains('(') {
        vec![format!("{}()", bare), bare]
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
            let elsewhere = self.generated("Terms defined by reference", 3, end, vec![]);
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
    let (records, _) = analyze(src, shortname, config);
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
        let (records, stats) = analyze(&src, "example-respec", &config());
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
        assert_eq!((stats.dfns, stats.idl, stats.unresolved), (8, 7, 0));
    }
}
