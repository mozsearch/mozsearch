//! ecmarkup sources (ECMA-262's `spec.html`, ECMA-402's, and TC39 proposals'):
//! their definitions and references, as ecmarkup (25.x) links them, with
//! their positions.
//!
//! Definitions (and the ids they're the anchors of):
//! - clauses (`<emu-clause>`, `<emu-annex>`, `<emu-intro>`), by their ids,
//!   and their `oldids`, which ecmarkup keeps as anchors;
//! - abstract operations: a clause's `aoid`, or the name in its structured
//!   header (an `<h1>` followed by a `<dl class="header">`, ex: "ToString (
//!   _arg_ ... )", or "Runtime Semantics: Evaluation ( )") if its `type` is
//!   one of the kinds of abstract operations;
//! - terms (`<dfn>`s, with their `variants`), whose anchors are their ids, or
//!   else their clauses';
//! - grammar productions (the left-hand sides of `<emu-grammar
//!   type="definition">`s), whose anchors are `prod-<name>` (or
//!   `prod-<namespace>-<name>` in a clause's namespace, ex: Annex B's), the
//!   last definition of a production in a namespace taking the anchor;
//! - algorithm steps with ids (`1. [id="step-..."]`), and other elements with
//!   ids (ex: tables).
//!
//! References:
//! - ecmarkup's autolinking: in text outside of code, grammar, headings,
//!   links, variables and the like (but in structured headers' types, which
//!   ecmarkup copies into the paragraphs it generates), the longest of the
//!   terms and abstract operations (and equations' and tables of methods'
//!   operations) defined in the clause's namespace (or its parents') at each
//!   position, with ecmarkup's rules (ex: "Call" only before a parenthesis),
//!   but not to the id it's in, outside of algorithms, and invocations of
//!   internal methods (`_O_.[[GetOwnProperty]](...)`);
//! - ecmarkdown's nonterminals (`|Name|`), nonterminals in grammars, and
//!   `<emu-prodref>`s (whose copies of their productions, which ecmarkup
//!   generates, aren't in the source);
//! - `<emu-xref>`s (by `href` or `aoid`), and links.
//!
//! References can also be to the definitions of the specs whose "biblios" a
//! spec loads (`--load-biblio @tc39/ecma262-biblio` in its package.json's
//! build scripts, as TC39 proposals and ECMA-402 do; see `Biblio`), whose
//! terms, abstract operations, and productions it autolinks (after its own),
//! and whose ids its `<emu-xref>`s can be to.
//!
//! Records' contexts are their clauses, by their abstract operations' names
//! or titles, with the algorithm step (as rendered, ex: "ToString step 3.a").

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use super::markup::{Doc, NodeKind};
use super::{
    Combined, InContext, Lines, SpecsConfig, normalize_path, spec_records, spec_structured,
    spec_symbol,
};
use crate::file_format::analysis::Location;
use crate::semantic_analyzer::{Record, sort_records};

const CLAUSES: &[&str] = &["emu-clause", "emu-annex", "emu-intro"];

/// The kinds of clauses whose structured headers name abstract operations.
const AOID_TYPES: &[&str] = &[
    "abstract operation",
    "sdo",
    "syntax-directed operation",
    "host-defined abstract operation",
    "implementation-defined abstract operation",
    "numeric method",
];

/// Elements whose text isn't autolinked (ecmarkup's `NO_CLAUSE_AUTOLINK`).
const NO_AUTOLINK: &[&str] = &[
    "pre",
    "code",
    "script",
    "style",
    "emu-const",
    "emu-production",
    "emu-grammar",
    "emu-xref",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "emu-var",
    "emu-val",
    "var",
    "a",
    "dfn",
    "sub",
    "emu-not-ref",
];

/// Elements whose text is autolinked even within those.
const YES_AUTOLINK: &[&str] = &["emu-gmod"];

/// Abstract operations whose names are common words, so they're only linked
/// when they're called.
const COMMON_OPS: &[&str] = &["Call", "Set", "Type", "UTC", "remainder"];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A term or an abstract operation, for autolinking.
#[derive(Clone)]
struct Entry {
    /// The id its links go to.
    target: String,
    is_term: bool,
    /// The biblio it's from (its index among the spec's), if it's not the
    /// spec's own.
    biblio: Option<usize>,
}

/// The ecmarkup biblios that specs load (`--load-biblio <package>`), by the
/// rendered URLs of the specs they're of.
const BIBLIO_PACKAGES: &[(&str, &str)] = &[
    ("@tc39/ecma262-biblio", "https://tc39.es/ecma262/"),
    ("@tc39/ecma402-biblio", "https://tc39.es/ecma402/"),
];

/// The rendered URLs of the specs whose biblios a spec's package.json's
/// scripts load (ex: TC39 proposals' and ECMA-402's ECMA-262's), in
/// `BIBLIO_PACKAGES`' order (so that the specs they're of can be analyzed
/// first).  (ecmarkup before 14 had ECMA-262's biblio built in, which
/// proposals using it, ex: explicit-resource-management, still link to.)
pub fn loaded_biblios(package_json: &str) -> Vec<&'static str> {
    let package: serde_json::Value = serde_json::from_str(package_json).unwrap_or_default();
    let scripts: Vec<&str> = package["scripts"]
        .as_object()
        .map_or(vec![], |scripts| scripts.values().filter_map(|s| s.as_str()).collect());
    let ecmarkup_major = ["devDependencies", "dependencies"]
        .iter()
        .find_map(|deps| package[deps]["ecmarkup"].as_str())
        .and_then(|version| {
            let version = version.trim_start_matches(|c: char| !c.is_ascii_digit());
            version.split('.').next()?.parse::<u32>().ok()
        });
    BIBLIO_PACKAGES
        .iter()
        .enumerate()
        .filter(|&(i, (package, _))| {
            scripts.iter().any(|script| {
                script.contains(&format!("--load-biblio {}", package))
                    || script.contains(&format!("--load-biblio={}", package))
            }) || (i == 0 && ecmarkup_major.is_some_and(|major| major < 14))
        })
        .map(|(_, &(_, url))| url)
        .collect()
}

/// The URLs of the specs whose biblios other specs can load, in the order
/// they load each other's (ECMA-402 loads ECMA-262's).
pub fn biblio_urls() -> impl Iterator<Item = &'static str> {
    BIBLIO_PACKAGES.iter().map(|&(_, url)| url)
}

/// A spec's definitions, as other specs load them (ecmarkup's "biblio",
/// ex: `@tc39/ecma262-biblio`): its terms, abstract operations, and
/// productions (but for those in namespaces, ex: Annex B's), and its ids,
/// with what they are and their pretty names.
#[derive(Default)]
pub struct Biblio {
    shortname: String,
    /// (key, id), in document order.
    terms: Vec<(String, String)>,
    ops: Vec<(String, String)>,
    productions: HashMap<String, String>,
    ids: HashMap<String, (&'static str, String)>,
}

/// A namespace's definitions (ecmarkup's "biblio" environment records).
#[derive(Default)]
struct Namespace {
    parent: Option<String>,
    /// (key, entry), in document order.
    terms: Vec<(String, Entry)>,
    ops: Vec<(String, Entry)>,
    /// Production names and their anchors.
    productions: HashMap<String, String>,
}

/// The keys to link in a namespace, by their first character, longest first.
struct Autolinker {
    by_first: HashMap<char, Vec<(Vec<char>, Entry)>>,
}

impl Autolinker {
    /// ecmarkup's `getDefinedWords`: the namespace's and its parents' terms
    /// (and, for terms starting with a lowercase letter, their capitalized
    /// forms) and then abstract operations, which take precedence, nearer
    /// namespaces' first.
    fn new(namespaces: &HashMap<String, Namespace>, ns: &str, biblios: &[&Biblio]) -> Autolinker {
        let mut words: HashMap<String, Entry> = HashMap::new();
        for is_term in [true, false] {
            let mut seen: HashSet<String> = HashSet::new();
            let mut add = |key: &str, entry: &Entry| {
                let mut keys = vec![key.to_string()];
                if is_term && key.starts_with(|c: char| c.is_ascii_lowercase()) {
                    let mut chars = key.chars();
                    let first = chars.next().unwrap().to_ascii_uppercase();
                    keys.push(std::iter::once(first).chain(chars).collect());
                }
                for key in keys {
                    if seen.insert(key.clone()) {
                        words.insert(key, entry.clone());
                    }
                }
            };
            let mut current = Some(ns.to_string());
            while let Some(name) = current {
                let Some(namespace) = namespaces.get(&name) else {
                    break;
                };
                let entries = if is_term {
                    &namespace.terms
                } else {
                    &namespace.ops
                };
                for (key, entry) in entries {
                    add(key, entry);
                }
                current = namespace.parent.clone();
            }
            // (The biblios' definitions are in the outermost scope.)
            for (i, biblio) in biblios.iter().enumerate() {
                let entries = if is_term { &biblio.terms } else { &biblio.ops };
                for (key, id) in entries {
                    let entry = Entry {
                        target: id.clone(),
                        is_term,
                        biblio: Some(i),
                    };
                    add(key, &entry);
                }
            }
        }
        let mut by_first: HashMap<char, Vec<(Vec<char>, Entry)>> = HashMap::new();
        for (key, entry) in words {
            let chars: Vec<char> = key.chars().collect();
            if let Some(&first) = chars.first() {
                by_first.entry(first).or_default().push((chars, entry));
            }
        }
        for keys in by_first.values_mut() {
            keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        }
        Autolinker { by_first }
    }

    /// The links in `text`: (char range, entry), as ecmarkup's autolinker's
    /// regular expression would find them.
    fn links(&self, text: &[char]) -> Vec<(Range<usize>, &Entry)> {
        let mut out = vec![];
        let mut i = 0;
        while i < text.len() {
            let found = self.by_first.get(&text[i]).and_then(|keys| {
                keys.iter().find_map(|(key, entry)| {
                    // (Keys starting with a word character start words.)
                    if is_word(key[0]) && i > 0 && is_word(text[i - 1]) {
                        return None;
                    }
                    let end = match_key(text, i, key)?;
                    lookahead_ok(text, end, key, entry).then_some((end, entry))
                })
            });
            match found {
                Some((end, entry)) => {
                    out.push((i..end, entry));
                    i = end;
                }
                None => i += 1,
            }
        }
        out
    }
}

/// Where `key` ends if it's at `text[at..]`, where its whitespace matches any
/// whitespace (ecmarkup's `widenSpace`).
fn match_key(text: &[char], at: usize, key: &[char]) -> Option<usize> {
    let (mut i, mut k) = (at, 0);
    while k < key.len() {
        if key[k].is_whitespace() {
            if i >= text.len() || !text[i].is_whitespace() {
                return None;
            }
            while i < text.len() && text[i].is_whitespace() {
                i += 1;
            }
            while k < key.len() && key[k].is_whitespace() {
                k += 1;
            }
        } else {
            if text.get(i) != Some(&key[k]) {
                return None;
            }
            i += 1;
            k += 1;
        }
    }
    Some(i)
}

/// ecmarkup's `lookAheadBeyond`.
fn lookahead_ok(text: &[char], end: usize, key: &[char], entry: &Entry) -> bool {
    let next = text.get(end).copied();
    let key_str: String = key.iter().collect();
    if COMMON_OPS.contains(&key_str.as_str()) {
        return next == Some('(');
    }
    if !entry.is_term || is_word(key[0]) {
        let after = text.get(end + 1).copied();
        return !(next.is_some_and(is_word)
            || (next == Some('.') && after.is_some_and(is_word))
            || (next == Some('%') && after == Some('%'))
            || (next == Some(']') && after == Some(']')));
    }
    true
}

lazy_static! {
    /// An algorithm step's marker and attributes, ex: `1. [id="step-x"] `.
    static ref STEP: Regex =
        Regex::new(r#"(?m)^([ \t]*)(1\.|\*)[ \t]+(?:\[([^\]\n]*)\][ \t]*)?"#).unwrap();
    static ref ID_ATTR: Regex = Regex::new(r#"\bid="([^"]*)""#).unwrap();
    /// A production's left-hand side in a grammar.
    static ref LHS: Regex =
        Regex::new(r"(?m)^[ \t]*([A-Za-z][A-Za-z0-9_]*)(?:\[[^\]\n]*\])?[ \t]*:{1,3}").unwrap();
    static ref NAME: Regex = Regex::new(r"[A-Za-z][A-Za-z0-9_]*").unwrap();
}

/// The rendered label of the `n`th (1-based) step at ordered depth `depth`:
/// 1, a, i, 1, ...
fn step_label(depth: usize, n: usize) -> String {
    match depth % 3 {
        0 => n.to_string(),
        1 => {
            let mut s = String::new();
            let mut n = n;
            while n > 0 {
                n -= 1;
                s.insert(0, (b'a' + (n % 26) as u8) as char);
                n /= 26;
            }
            s
        }
        _ => {
            let table = [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ];
            let (mut n, mut s) = (n, String::new());
            for (value, numeral) in table {
                while n >= value {
                    s.push_str(numeral);
                    n -= value;
                }
            }
            s
        }
    }
}

/// An algorithm's steps: where each starts, its path (ex: "3.a"; "" for an
/// unordered item at the top), and its id, if it has one, and the bytes of
/// their markers and attributes, which aren't text.
#[derive(Default)]
struct Steps {
    starts: Vec<(usize, String, Option<String>)>,
    markup: Vec<Range<usize>>,
}

fn algorithm_steps(src: &str, content: Range<usize>) -> Steps {
    let body = &src[content.clone()];
    let mut steps = Steps::default();
    // The open lists: (indentation, ordered, count).
    let mut levels: Vec<(usize, bool, usize)> = vec![];
    for captures in STEP.captures_iter(body) {
        let whole = captures.get(0).unwrap();
        let indent = captures[1].len();
        let ordered = &captures[2] == "1.";
        while levels.last().is_some_and(|&(i, _, _)| i > indent) {
            levels.pop();
        }
        if levels.last().is_none_or(|&(i, _, _)| i < indent) {
            levels.push((indent, ordered, 0));
        }
        levels.last_mut().unwrap().2 += 1;
        let mut path = vec![];
        let mut depth = 0;
        for &(_, ordered, count) in &levels {
            if ordered {
                path.push(step_label(depth, count));
                depth += 1;
            }
        }
        let id = captures
            .get(3)
            .and_then(|attrs| ID_ATTR.captures(attrs.as_str()))
            .map(|c| c[1].to_string());
        let marker_start = content.start + whole.start() + indent;
        steps
            .starts
            .push((marker_start, path.join("."), id));
        steps.markup.push(marker_start..content.start + whole.end());
    }
    steps
}

impl Steps {
    fn path_at(&self, pos: usize) -> Option<&str> {
        let i = self.starts.partition_point(|(start, _, _)| *start <= pos);
        i.checked_sub(1).map(|i| self.starts[i].1.as_str())
    }
}

/// An ecmarkdown inline format.
#[derive(Debug, PartialEq)]
enum Emd {
    /// A variable `_x_`, value `*x*`, constant `~x~`, or code `` `x` ``.
    Format,
    /// `|Name|` (or ex: `|Name_opt|` or `|Name[?Yield]|`), by its name.
    NonTerminal(String),
    /// A field `[[Name]]`, by its text.
    Field(String),
}

/// The spans of ecmarkdown's inline formats in `text`, which aren't
/// autolinked, as char ranges.
fn emd_spans(text: &[char]) -> Vec<(Range<usize>, Emd)> {
    let mut out = vec![];
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == '[' && text.get(i + 1) == Some(&'[') {
            let close = (i + 2..text.len().saturating_sub(1))
                .take_while(|&j| text[j] != '\n')
                .find(|&j| text[j] == ']' && text[j + 1] == ']');
            if let Some(j) = close {
                out.push((i..j + 2, Emd::Field(text[i..j + 2].iter().collect())));
                i = j + 2;
                continue;
            }
        }
        let opens = matches!(c, '_' | '*' | '~' | '`' | '|')
            && (i == 0 || !text[i - 1].is_alphanumeric())
            && text.get(i + 1).is_some_and(|n| !n.is_whitespace() && *n != c);
        if !opens {
            i += 1;
            continue;
        }
        let close = (i + 2..text.len())
            .take_while(|&j| text[j] != '\n' || c == '`')
            .find(|&j| {
                text[j] == c
                    && !text[j - 1].is_whitespace()
                    && text.get(j + 1).is_none_or(|n| !n.is_alphanumeric())
            });
        match close {
            Some(j) => {
                let kind = if c == '|' {
                    let inner: String = text[i + 1..j].iter().collect();
                    let end = inner
                        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                        .unwrap_or(inner.len());
                    let name = inner[..end].trim_end_matches("_opt");
                    if name.is_empty() {
                        Emd::Format
                    } else {
                        Emd::NonTerminal(name.to_string())
                    }
                } else {
                    Emd::Format
                };
                out.push((i..j + 1, kind));
                i = j + 1;
            }
            None => i += 1,
        }
    }
    out
}

/// The analysis's counts, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    pub clauses: usize,
    pub operations: usize,
    pub terms: usize,
    pub productions: usize,
    pub autolinks: usize,
    pub xrefs: usize,
    pub unresolved: usize,
    pub nonterminals: usize,
    /// References to the definitions of the biblios the spec loads.
    pub biblio_references: usize,
}

struct Clause {
    node: usize,
    id: String,
    namespace: String,
    /// Its abstract operation's name, or else its title (without
    /// parameters).
    name: String,
    aoid: Option<String>,
}

/// A clause's `<h1>`, if it has one.
fn clause_h1(doc: &Doc, node: usize) -> Option<usize> {
    doc.nodes[node]
        .children
        .iter()
        .copied()
        .find(|&child| doc.is(child, "h1"))
}

/// The name in a structured header (ecmarkup's `parseH1`).
fn header_name(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text
        .strip_prefix("Static Semantics:")
        .or_else(|| text.strip_prefix("Runtime Semantics:"))
        .unwrap_or(text)
        .trim_start();
    let name: String = text
        .chars()
        .take_while(|c| *c != '(' && !c.is_whitespace())
        .collect();
    (!name.is_empty()).then_some(name)
}

/// A record to write.
struct Pending {
    range: Range<usize>,
    def: bool,
    sym: String,
    what: &'static str,
    pretty: String,
}

/// The records of an ecmarkup source, of the spec with `shortname`, among
/// `config`'s specs.
pub fn analyze(src: &str, shortname: &str, config: &SpecsConfig) -> (Vec<Record>, Stats) {
    let lines = Lines::new(src);
    let (mut files, stats, _) =
        analyze_placed(src, shortname, config, &[], 1, &|range: &Range<usize>| {
            Some((0, lines.location(range)))
        });
    (files.pop().unwrap_or_default(), stats)
}

lazy_static! {
    static ref EMU_IMPORT: Regex =
        Regex::new(r#"<emu-import\s+href=["']?([^"'\s>]+)["']?\s*(?:/>|>\s*</emu-import>)"#).unwrap();
}

/// A file's imports (`<emu-import href>`, which ecmarkup replaces with the
/// files' contents, relative to the importing file).
fn imports(path: &str, source: &str) -> Vec<(Range<usize>, String)> {
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    EMU_IMPORT
        .captures_iter(source)
        .map(|import| {
            let target = normalize_path(&format!("{}/{}", dir, &import[1]));
            (import.get(0).unwrap().range(), target)
        })
        .collect()
}

/// The records of an ecmarkup spec's source at `main` (a path in the tree,
/// which `read` reads) and of the files it imports, by their paths, as
/// ecmarkup builds the spec from them with the `biblios` it loads, and the
/// spec's own biblio.
pub fn analyze_with_imports(
    main: &str,
    read: &dyn Fn(&str) -> Option<String>,
    shortname: &str,
    config: &SpecsConfig,
    biblios: &[&Biblio],
) -> Option<(Vec<(String, Vec<Record>)>, Stats, Biblio)> {
    let combined = Combined::build(main, read, &imports)?;
    let (files, stats, biblio) = analyze_placed(
        &combined.text,
        shortname,
        config,
        biblios,
        combined.paths().count(),
        &|range: &Range<usize>| combined.place(range),
    );
    let paths = combined.paths().map(str::to_string);
    Some((paths.zip(files).collect(), stats, biblio))
}

/// The records of an ecmarkup source with the `biblios` it loads, for each
/// of the `files` its text is from, by where `place` says its ranges are,
/// and its own biblio.
fn analyze_placed(
    src: &str,
    shortname: &str,
    config: &SpecsConfig,
    biblios: &[&Biblio],
    files: usize,
    place: &dyn Fn(&Range<usize>) -> Option<(usize, Location)>,
) -> (Vec<Vec<Record>>, Stats, Biblio) {
    let doc = Doc::parse(src);
    let n = doc.nodes.len();
    let mut stats = Stats::default();

    // What each node is in: its clause, the id it's in, whether it's in an
    // algorithm, and whether its text is autolinked.
    let mut clause_of: Vec<Option<usize>> = vec![None; n];
    let mut id_of: Vec<Option<usize>> = vec![None; n];
    let mut in_alg = vec![false; n];
    let mut no_autolink = vec![false; n];
    let mut clauses: BTreeMap<usize, Clause> = BTreeMap::new();
    // Structured headers' `<h1>`s, whose parameters' and return values' types
    // ecmarkup copies into the paragraph it generates, which it autolinks.
    let mut structured_h1s: HashSet<usize> = HashSet::new();
    let mut namespaces: HashMap<String, Namespace> = HashMap::new();
    namespaces.insert(String::new(), Namespace::default());
    for node in 1..n {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        clause_of[node] = clause_of[parent];
        id_of[node] = id_of[parent];
        in_alg[node] = in_alg[parent];
        no_autolink[node] = no_autolink[parent];
        let Some(name) = doc.name(node) else {
            continue;
        };
        if doc.has_attr(node, "id") {
            id_of[node] = Some(node);
        }
        if name == "emu-alg" {
            in_alg[node] = true;
        }
        if YES_AUTOLINK.contains(&name) {
            no_autolink[node] = false;
        } else if NO_AUTOLINK.contains(&name) {
            no_autolink[node] = true;
        }
        if !CLAUSES.contains(&name) {
            continue;
        }
        let Some(id) = doc.attr(node, "id") else {
            continue;
        };
        let parent_ns = clause_of[parent].map_or(String::new(), |c| clauses[&c].namespace.clone());
        let namespace = match doc.attr(node, "namespace") {
            Some(ns) => {
                namespaces.entry(ns.clone()).or_insert_with(|| Namespace {
                    parent: Some(parent_ns),
                    ..Default::default()
                });
                ns
            }
            None => parent_ns,
        };
        let h1 = clause_h1(&doc, node);
        let title = h1.map_or(String::new(), |h1| {
            let words = doc
                .text_content(h1)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            match words.split_once(" (") {
                Some((name, _)) if !name.is_empty() => name.to_string(),
                _ => words,
            }
        });
        let structured = h1.is_some_and(|h1| {
            let siblings = &doc.nodes[node].children;
            let at = siblings.iter().position(|&c| c == h1).unwrap();
            siblings[at + 1..]
                .iter()
                .find(|&&c| doc.name(c).is_some())
                .is_some_and(|&c| doc.is(c, "dl") && doc.attr(c, "class").as_deref() == Some("header"))
        });
        if structured {
            structured_h1s.extend(h1);
        }
        let aoid = match doc.attr(node, "aoid") {
            Some(aoid) if aoid.is_empty() => Some(id.clone()),
            Some(aoid) => Some(aoid),
            None if structured
                && doc
                    .attr(node, "type")
                    .is_some_and(|t| AOID_TYPES.contains(&t.as_str())) =>
            {
                h1.and_then(|h1| header_name(&doc.text_content(h1)))
            }
            None => None,
        };
        clause_of[node] = Some(node);
        let name = aoid.clone().unwrap_or(title);
        clauses.insert(
            node,
            Clause {
                node,
                id,
                namespace,
                name,
                aoid,
            },
        );
    }

    // The terms, abstract operations, and productions, and every id.
    // Ids' definitions: what they are, and their pretty names.
    let mut defs: HashMap<String, (&'static str, String)> = HashMap::new();
    for clause in clauses.values() {
        stats.clauses += 1;
        defs.insert(
            clause.id.clone(),
            ("clause", format!("{} {}", shortname, clause.name)),
        );
        if let Some(aoid) = &clause.aoid {
            stats.operations += 1;
            let ns = namespaces.get_mut("").unwrap();
            if !ns.ops.iter().any(|(key, _)| key == aoid) {
                ns.ops.push((
                    aoid.clone(),
                    Entry {
                        target: clause.id.clone(),
                        is_term: false,
                        biblio: None,
                    },
                ));
            }
        }
    }
    let mut old_ids: Vec<(usize, String)> = vec![];
    // Internal methods (ex: `[[GetOwnProperty]]`), and their anchors, which
    // only invocations (`_O_.[[GetOwnProperty]](...)`) link to.
    let mut internal_methods: HashMap<String, String> = HashMap::new();
    // The structured kinds of operations' anchors other than clauses
    // (equations' and tables of methods' rows).
    let mut op_kinds: Vec<(String, &'static str)> = vec![];
    let mut grammars: Vec<(usize, bool)> = vec![];
    let mut algorithms: Vec<(usize, Steps)> = vec![];
    for node in 1..n {
        let Some(name) = doc.name(node) else {
            continue;
        };
        if let Some(oldids) = doc.attr(node, "oldids") {
            for old in oldids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                old_ids.push((node, old.to_string()));
            }
        }
        match name {
            "dfn" => {
                let Some(clause) = clause_of[node].map(|c| &clauses[&c]) else {
                    continue;
                };
                let term = doc.text_content(node);
                if term == "type" {
                    continue;
                }
                stats.terms += 1;
                let id = doc.attr(node, "id");
                if let Some(id) = &id {
                    defs.insert(id.clone(), ("dfn", format!("{} {}", shortname, term)));
                }
                let entry = Entry {
                    target: id.unwrap_or_else(|| clause.id.clone()),
                    is_term: true,
                    biblio: None,
                };
                let mut keys = vec![term];
                if let Some(variants) = doc.attr(node, "variants") {
                    keys.extend(variants.split(',').map(|v| v.trim().to_string()));
                }
                let ns = namespaces.get_mut(&clause.namespace).unwrap();
                for key in keys.into_iter().filter(|k| !k.is_empty()) {
                    ns.terms.push((key, entry.clone()));
                }
            }
            "emu-grammar" => {
                let definition = doc.attr(node, "type").as_deref() == Some("definition")
                    || doc.has_attr(node, "primary");
                grammars.push((node, definition));
            }
            "emu-alg" => {
                algorithms.push((node, algorithm_steps(src, doc.content_range(node))));
            }
            "emu-eqn" => {
                let Some(aoid) = doc.attr(node, "aoid") else {
                    continue;
                };
                let target = doc
                    .attr(node, "id")
                    .or_else(|| clause_of[node].map(|c| clauses[&c].id.clone()));
                if let Some(target) = target {
                    stats.operations += 1;
                    if doc.has_attr(node, "id") {
                        op_kinds.push((target.clone(), "specAbstractOp"));
                        defs.insert(target.clone(), ("operation", format!("{} {}", shortname, aoid)));
                    }
                    let ns = namespaces.get_mut("").unwrap();
                    if !ns.ops.iter().any(|(key, _)| *key == aoid) {
                        ns.ops.push((
                            aoid,
                            Entry {
                                target,
                                is_term: false,
                                biblio: None,
                            },
                        ));
                    }
                }
            }
            // Tables of abstract methods and internal methods define each
            // row's method, by the name in the header in its first cell.
            "emu-table" => {
                let internal = match doc.attr(node, "type").as_deref() {
                    Some("abstract methods") => false,
                    Some("internal methods") => true,
                    _ => continue,
                };
                let table_id = doc.attr(node, "id").unwrap_or_default();
                let rows: Vec<usize> = (node + 1..n)
                    .take_while(|&r| doc.nodes[r].range.start < doc.nodes[node].range.end)
                    // (Header rows have no `<td>`s.)
                    .filter(|&r| doc.is(r, "tr"))
                    .collect();
                for row in rows {
                    let Some(&cell) = doc.nodes[row].children.iter().find(|&&c| doc.is(c, "td"))
                    else {
                        continue;
                    };
                    let Some(name) = header_name(&doc.text_content(cell)) else {
                        continue;
                    };
                    let target = doc.attr(row, "id").unwrap_or_else(|| table_id.clone());
                    stats.operations += 1;
                    if doc.has_attr(row, "id") {
                        op_kinds.push((
                            target.clone(),
                            if internal { "method" } else { "specAbstractOp" },
                        ));
                        defs.insert(target.clone(), ("operation", format!("{} {}", shortname, name)));
                    }
                    if internal {
                        internal_methods.entry(name).or_insert(target);
                    } else {
                        let ns = namespaces.get_mut("").unwrap();
                        if !ns.ops.iter().any(|(key, _)| *key == name) {
                            ns.ops.push((
                                name,
                                Entry {
                                    target,
                                    is_term: false,
                                    biblio: None,
                                },
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // Productions: the last definition of each in a namespace takes its id.
    let namespace_of = |node: usize| -> String {
        clause_of[node].map_or(String::new(), |c| clauses[&c].namespace.clone())
    };
    let mut lhs_defs: HashMap<(String, String), Range<usize>> = HashMap::new();
    for &(node, definition) in &grammars {
        if !definition {
            continue;
        }
        let content = doc.content_range(node);
        let ns = namespace_of(node);
        for captures in LHS.captures_iter(&src[content.clone()]) {
            let name = captures.get(1).unwrap();
            let range = content.start + name.start()..content.start + name.end();
            lhs_defs.insert((ns.clone(), name.as_str().to_string()), range);
        }
    }
    let mut production_defs: Vec<(Range<usize>, String)> = vec![];
    for ((ns, name), range) in &lhs_defs {
        let id = if ns.is_empty() {
            format!("prod-{}", name)
        } else {
            format!("prod-{}-{}", ns, name)
        };
        stats.productions += 1;
        namespaces
            .get_mut(ns)
            .unwrap()
            .productions
            .insert(name.clone(), id.clone());
        defs.insert(id.clone(), ("production", format!("{} {}", shortname, name)));
        production_defs.push((range.clone(), id));
    }
    // (A production's anchor, and the biblio it's from, if it's not ours.)
    let production = |ns: &str, name: &str| -> Option<(Option<usize>, String)> {
        let mut current = Some(ns.to_string());
        while let Some(ns) = current {
            let Some(namespace) = namespaces.get(&ns) else {
                break;
            };
            if let Some(id) = namespace.productions.get(name) {
                return Some((None, id.clone()));
            }
            current = namespace.parent.clone();
        }
        biblios
            .iter()
            .enumerate()
            .find_map(|(i, biblio)| Some((Some(i), biblio.productions.get(name)?.clone())))
    };
    let mut ids: HashSet<String> = HashSet::new();
    for node in 1..n {
        if let Some(id) = doc.attr(node, "id") {
            ids.insert(id);
        }
    }
    ids.extend(old_ids.iter().map(|(_, id)| id.clone()));
    ids.extend(defs.keys().cloned());
    for (_, steps) in &algorithms {
        ids.extend(steps.starts.iter().filter_map(|(_, _, id)| id.clone()));
    }

    // Contexts: the innermost clause, and the step.
    let mut clause_ranges: Vec<(Range<usize>, usize)> = clauses
        .values()
        .map(|c| (doc.nodes[c.node].range.clone(), c.node))
        .collect();
    clause_ranges.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
    let alg_ranges: Vec<(Range<usize>, usize)> = algorithms
        .iter()
        .enumerate()
        .map(|(i, (node, _))| (doc.content_range(*node), i))
        .collect();
    let context_at = |pos: usize| -> InContext {
        let clause = clause_ranges
            .iter()
            .filter(|(range, _)| range.start <= pos && pos < range.end)
            .last()
            .map(|(_, node)| &clauses[node]);
        let Some(clause) = clause else {
            return InContext::default();
        };
        let mut name = clause.name.clone();
        let alg = alg_ranges
            .partition_point(|(range, _)| range.start <= pos)
            .checked_sub(1)
            .filter(|&i| pos < alg_ranges[i].0.end);
        if let Some(path) = alg.and_then(|i| algorithms[alg_ranges[i].1].1.path_at(pos))
            && !path.is_empty()
        {
            name = format!("{} step {}", name, path);
        }
        InContext {
            name,
            sym: spec_symbol(shortname, &clause.id),
        }
    };

    let mut pending: Vec<Pending> = vec![];
    // (A range's start, without leading whitespace, to its line's end.)
    let trimmed = |range: Range<usize>| -> Range<usize> {
        let text = &src[range.clone()];
        let start = range.start + (text.len() - text.trim_start().len());
        start..range.end.max(start)
    };
    let pretty_of = |id: &str| -> (&'static str, String) {
        defs.get(id).map_or(("anchor", format!("{} #{}", shortname, id)), |(what, pretty)| {
            (*what, pretty.clone())
        })
    };
    // A reference's symbol, what it is, and its pretty name, given its
    // anchor and the biblio it's from, if it's not ours.
    let referent = |biblio: Option<usize>, id: &str| -> (String, &'static str, String) {
        match biblio {
            None => {
                let (what, pretty) = pretty_of(id);
                (spec_symbol(shortname, id), what, pretty)
            }
            Some(i) => {
                let biblio = biblios[i];
                let (what, pretty) = biblio.ids.get(id).cloned().unwrap_or_else(|| {
                    ("anchor", format!("{} #{}", biblio.shortname, id))
                });
                (spec_symbol(&biblio.shortname, id), what, pretty)
            }
        }
    };

    // Definitions.
    for clause in clauses.values() {
        let range = clause_h1(&doc, clause.node)
            .map(|h1| trimmed(doc.content_range(h1)))
            .unwrap_or_else(|| doc.items[doc.nodes[clause.node].item].range().clone());
        let (what, pretty) = pretty_of(&clause.id);
        pending.push(Pending {
            range,
            def: true,
            sym: spec_symbol(shortname, &clause.id),
            what,
            pretty,
        });
    }
    for (node, old) in &old_ids {
        let pretty = doc
            .attr(*node, "id")
            .map_or_else(|| format!("{} #{}", shortname, old), |id| pretty_of(&id).1);
        pending.push(Pending {
            range: doc.items[doc.nodes[*node].item].range().clone(),
            def: true,
            sym: spec_symbol(shortname, old),
            what: "anchor",
            pretty,
        });
    }
    for node in 1..n {
        let Some(id) = doc.attr(node, "id") else {
            continue;
        };
        if clauses.contains_key(&node) {
            continue;
        }
        let (what, pretty) = match defs.get(&id) {
            Some((what, pretty)) => (*what, pretty.clone()),
            None => (
                "anchor",
                match doc.attr(node, "caption") {
                    Some(caption) => format!("{} {}", shortname, caption),
                    None => format!("{} #{}", shortname, id),
                },
            ),
        };
        let range = doc.content_range(node);
        let range = if range.is_empty() || what == "anchor" {
            doc.items[doc.nodes[node].item].range().clone()
        } else {
            range
        };
        pending.push(Pending {
            range,
            def: true,
            sym: spec_symbol(shortname, &id),
            what,
            pretty,
        });
    }
    for (range, id) in &production_defs {
        let (what, pretty) = pretty_of(id);
        pending.push(Pending {
            range: range.clone(),
            def: true,
            sym: spec_symbol(shortname, id),
            what,
            pretty,
        });
    }
    for (node, steps) in &algorithms {
        let clause_name = clause_of[*node].map_or(String::new(), |c| clauses[&c].name.clone());
        for (start, path, id) in &steps.starts {
            if let Some(id) = id {
                pending.push(Pending {
                    range: *start..*start + 2,
                    def: true,
                    sym: spec_symbol(shortname, id),
                    what: "step",
                    pretty: format!("{} {} step {}", shortname, clause_name, path),
                });
            }
        }
    }

    // References: autolinks, ecmarkdown's nonterminals, xrefs, and links.
    let mut autolinkers: HashMap<String, Autolinker> = HashMap::new();
    let alg_markup: Vec<Range<usize>> = algorithms
        .iter()
        .flat_map(|(_, steps)| steps.markup.iter().cloned())
        .collect();
    for node in 1..n {
        if doc.nodes[node].kind != NodeKind::Text {
            continue;
        }
        // (In a structured header, the text after its name's parenthesis.)
        let header = doc
            .ancestors(node)
            .take_while(|a| !structured_h1s.contains(a))
            .all(|a| !doc.name(a).is_some_and(|name| NO_AUTOLINK.contains(&name)))
            .then(|| doc.ancestors(node).find(|a| structured_h1s.contains(a)))
            .flatten();
        let text_start = match header {
            Some(h1) => {
                let content = doc.content_range(h1);
                match src[content.clone()].find('(') {
                    Some(paren) => content.start + paren + 1,
                    None => continue,
                }
            }
            None if no_autolink[node] => continue,
            None => 0,
        };
        let range = doc.nodes[node].range.clone();
        let text = &src[range.clone()];
        if text.trim().is_empty() || text_start >= range.end {
            continue;
        }
        let ns = namespace_of(node);
        let autolinker = autolinkers
            .entry(ns.clone())
            .or_insert_with(|| Autolinker::new(&namespaces, &ns, biblios));
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let char_text: Vec<char> = chars.iter().map(|&(_, c)| c).collect();
        let byte = |i: usize| -> usize {
            range.start + chars.get(i).map_or(text.len(), |&(b, _)| b)
        };
        // The text's spans which aren't text: ecmarkdown's formats, and
        // algorithms' step markers.
        let mut skip: Vec<Range<usize>> = vec![];
        if text_start > range.start {
            skip.push(0..chars.partition_point(|&(b, _)| range.start + b < text_start));
        }
        for (span, kind) in emd_spans(&char_text) {
            if let Emd::Field(name) = &kind
                && let Some(target) = internal_methods.get(name)
                && char_text[..span.start]
                    .iter()
                    .rev()
                    .find(|c| !c.is_whitespace())
                    == Some(&'.')
            {
                stats.autolinks += 1;
                let (what, pretty) = pretty_of(target);
                pending.push(Pending {
                    range: byte(span.start)..byte(span.end),
                    def: false,
                    sym: spec_symbol(shortname, target),
                    what,
                    pretty,
                });
            }
            if let Emd::NonTerminal(name) = kind {
                stats.nonterminals += 1;
                match production(&ns, &name) {
                    Some((biblio, id)) => {
                        stats.biblio_references += biblio.is_some() as usize;
                        let (sym, what, pretty) = referent(biblio, &id);
                        pending.push(Pending {
                            range: byte(span.start + 1)..byte(span.start + 1 + name.chars().count()),
                            def: false,
                            sym,
                            what,
                            pretty,
                        });
                    }
                    None => stats.unresolved += 1,
                }
            }
            skip.push(span);
        }
        for markup in &alg_markup {
            if markup.start < range.end && range.start < markup.end {
                let start = chars.partition_point(|&(b, _)| range.start + b < markup.start);
                let end = chars.partition_point(|&(b, _)| range.start + b < markup.end);
                skip.push(start..end);
            }
        }
        skip.sort_by_key(|r| r.start);
        let current_id = id_of[node].and_then(|id_node| doc.attr(id_node, "id"));
        let mut at = 0;
        let mut segments = vec![];
        for span in &skip {
            if span.start > at {
                segments.push(at..span.start);
            }
            at = at.max(span.end);
        }
        segments.push(at..char_text.len());
        for segment in segments {
            for (link, entry) in autolinker.links(&char_text[segment.clone()]) {
                if !in_alg[node]
                    && entry.biblio.is_none()
                    && current_id.as_deref() == Some(entry.target.as_str())
                {
                    continue;
                }
                stats.autolinks += 1;
                stats.biblio_references += entry.biblio.is_some() as usize;
                let (sym, what, pretty) = referent(entry.biblio, &entry.target);
                pending.push(Pending {
                    range: byte(segment.start + link.start)..byte(segment.start + link.end),
                    def: false,
                    sym,
                    what,
                    pretty,
                });
            }
        }
    }
    // Nonterminals in grammars (but definitions' left-hand sides).
    for &(node, _) in &grammars {
        let content = doc.content_range(node);
        let ns = namespace_of(node);
        let body = &src[content.clone()];
        // (Not in terminals or prose.  Parameters' names aren't productions'.)
        let mut excluded: Vec<Range<usize>> = vec![];
        for (open, close) in [('`', '`'), ('<', '>')] {
            let mut i = 0;
            while let Some(start) = body[i..].find(open).map(|s| s + i) {
                let Some(end) = body[start + 1..].find(close).map(|e| e + start + 1) else {
                    break;
                };
                excluded.push(start..end + 1);
                i = end + 1;
            }
        }
        for line in body.split_inclusive('\n') {
            let offset = line.as_ptr() as usize - body.as_ptr() as usize;
            if line.trim_start().starts_with('>') {
                excluded.push(offset..offset + line.len());
            }
        }
        for m in NAME.find_iter(body) {
            let range = content.start + m.start()..content.start + m.end();
            if excluded.iter().any(|e| e.start <= m.start() && m.start() < e.end)
                || lhs_defs.values().any(|def| *def == range)
                || (m.start() > 0 && body[..m.start()].ends_with(|c: char| is_word(c)))
            {
                continue;
            }
            if let Some((biblio, id)) = production(&ns, m.as_str()) {
                stats.biblio_references += biblio.is_some() as usize;
                let (sym, what, pretty) = referent(biblio, &id);
                pending.push(Pending {
                    range,
                    def: false,
                    sym,
                    what,
                    pretty,
                });
            }
        }
    }
    for node in 1..n {
        match doc.name(node) {
            // (ecmarkup copies the production here, ex: in Annex A.)
            Some("emu-prodref") => {
                let Some(name) = doc.attr(node, "name") else {
                    continue;
                };
                stats.nonterminals += 1;
                let Some((biblio, id)) = production(&namespace_of(node), &name) else {
                    stats.unresolved += 1;
                    continue;
                };
                stats.biblio_references += biblio.is_some() as usize;
                let (sym, what, pretty) = referent(biblio, &id);
                pending.push(Pending {
                    range: doc.items[doc.nodes[node].item].range().clone(),
                    def: false,
                    sym,
                    what,
                    pretty,
                });
            }
            Some("emu-xref") => {
                stats.xrefs += 1;
                // (Ours, or else the biblios'.)
                let target = if let Some(href) = doc.attr(node, "href") {
                    href.strip_prefix('#').and_then(|id| {
                        if ids.contains(id) {
                            return Some((None, id.to_string()));
                        }
                        let i = biblios.iter().position(|b| b.ids.contains_key(id))?;
                        Some((Some(i), id.to_string()))
                    })
                } else if let Some(aoid) = doc.attr(node, "aoid") {
                    namespaces[""]
                        .ops
                        .iter()
                        .find(|(key, _)| *key == aoid)
                        .map(|(_, entry)| (None, entry.target.clone()))
                        .or_else(|| {
                            biblios.iter().enumerate().find_map(|(i, biblio)| {
                                let (_, id) = biblio.ops.iter().find(|(key, _)| *key == aoid)?;
                                Some((Some(i), id.clone()))
                            })
                        })
                } else {
                    None
                };
                let Some((biblio, target)) = target else {
                    stats.unresolved += 1;
                    continue;
                };
                stats.biblio_references += biblio.is_some() as usize;
                let range = doc.content_range(node);
                let range = if src[range.clone()].trim().is_empty() {
                    doc.items[doc.nodes[node].item].range().clone()
                } else {
                    range
                };
                let (sym, what, pretty) = referent(biblio, &target);
                pending.push(Pending {
                    range,
                    def: false,
                    sym,
                    what,
                    pretty,
                });
            }
            Some("a") => {
                let Some(href) = doc.attr(node, "href") else {
                    continue;
                };
                let (sym, what, pretty) = match href.strip_prefix('#') {
                    Some(id) if ids.contains(id) => {
                        let (what, pretty) = pretty_of(id);
                        (spec_symbol(shortname, id), what, pretty)
                    }
                    Some(_) => continue,
                    None => match config.url_symbol(&href) {
                        Some(sym) => (sym, "anchor", href.clone()),
                        None => continue,
                    },
                };
                pending.push(Pending {
                    range: doc.content_range(node),
                    def: false,
                    sym,
                    what,
                    pretty,
                });
            }
            _ => {}
        }
    }

    // Structured records: clauses are abstract operations, algorithms (ex:
    // built-in functions), or sections; terms are dfns, and productions
    // grammar.
    let mut structured: HashMap<String, (&'static str, Option<String>)> = HashMap::new();
    let mut add_structured = |id: &str, kind: &'static str| {
        structured
            .entry(spec_symbol(shortname, id))
            .or_insert_with(|| (kind, config.rendered_url(shortname, id)));
    };
    for clause in clauses.values() {
        let kind = if clause.aoid.is_some() {
            "specAbstractOp"
        } else if doc.nodes[clause.node]
            .children
            .iter()
            .any(|&child| doc.is(child, "emu-alg"))
        {
            "specAlgorithm"
        } else {
            "specSection"
        };
        add_structured(&clause.id, kind);
    }
    for (id, kind) in &op_kinds {
        add_structured(id, kind);
    }
    for (id, (what, _)) in &defs {
        match *what {
            "dfn" => add_structured(id, "specDfn"),
            "production" => add_structured(id, "specGrammar"),
            _ => {}
        }
    }

    let mut per_file: Vec<Vec<Record>> = (0..files).map(|_| vec![]).collect();
    for p in pending {
        let range = if p.range.is_empty() {
            p.range.start..p.range.start + 1
        } else {
            p.range
        };
        let Some((file, loc)) = place(&range) else {
            continue;
        };
        let context = context_at(range.start);
        let records = &mut per_file[file];
        spec_records(records, loc, p.def, p.what, &p.pretty, &p.sym, &context);
        if p.def
            && let Some((kind, rendered)) = structured.remove(&p.sym)
        {
            spec_structured(records, loc, &p.pretty, &p.sym, kind, None, rendered.as_deref());
        }
    }
    for records in &mut per_file {
        sort_records(records);
    }

    // Our biblio: the root namespace's definitions, and our ids.
    let root = &namespaces[""];
    let own = |entries: &Vec<(String, Entry)>| -> Vec<(String, String)> {
        entries
            .iter()
            .filter(|(_, entry)| entry.biblio.is_none())
            .map(|(key, entry)| (key.clone(), entry.target.clone()))
            .collect()
    };
    let biblio = Biblio {
        shortname: shortname.to_string(),
        terms: own(&root.terms),
        ops: own(&root.ops),
        productions: root.productions.clone(),
        ids: ids
            .iter()
            .map(|id| (id.clone(), pretty_of(id)))
            .collect(),
    };
    (per_file, stats, biblio)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(records: &[Record]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) => Some(format!(
                    "{}:{}-{} {} {} | {}",
                    t.loc.lineno,
                    t.loc.col_start,
                    t.loc.col_end,
                    t.data.kind.to_ustr(),
                    t.data.sym,
                    t.data.context
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_step_labels() {
        assert_eq!(step_label(0, 12), "12");
        assert_eq!(step_label(1, 1), "a");
        assert_eq!(step_label(1, 27), "aa");
        assert_eq!(step_label(2, 4), "iv");
        assert_eq!(step_label(3, 2), "2");
    }

    #[test]
    fn test_emd_spans() {
        let text: Vec<char> =
            "If _arg_ is *undefined* or |Foo_opt|, a ~b~ `c`, _O_.[[Get]], but not x_y or 2*3."
                .chars()
                .collect();
        let spans: Vec<(String, Emd)> = emd_spans(&text)
            .into_iter()
            .map(|(r, kind)| (text[r].iter().collect(), kind))
            .collect();
        assert_eq!(
            spans,
            vec![
                ("_arg_".to_string(), Emd::Format),
                ("*undefined*".to_string(), Emd::Format),
                ("|Foo_opt|".to_string(), Emd::NonTerminal("Foo".to_string())),
                ("~b~".to_string(), Emd::Format),
                ("`c`".to_string(), Emd::Format),
                ("_O_".to_string(), Emd::Format),
                ("[[Get]]".to_string(), Emd::Field("[[Get]]".to_string())),
            ]
        );
    }

    #[test]
    fn test_biblios() {
        // (A proposal that loads ECMA-262's biblio, which its references link
        // to, after its own definitions.)
        let es = r##"<emu-clause id="sec-terms">
  <h1>Terms</h1>
  <p>A <dfn id="realm">realm</dfn> is a place.</p>
  <emu-grammar type="definition">
    Statement : `;`
  </emu-grammar>
</emu-clause>
<emu-clause id="sec-tostring" type="abstract operation">
  <h1>ToString ( _argument_ )</h1>
  <dl class="header"></dl>
</emu-clause>
<emu-clause id="sec-frob" aoid="Frob"><h1>Frob ( )</h1></emu-clause>
"##;
        let proposal = r##"<emu-clause id="sec-frob" aoid="Frob">
  <h1>Frob ( )</h1>
  <emu-alg>
    1. Let _s_ be ToString(the realm).
    1. Return Frob() of |Statement|.
  </emu-alg>
  <p>See <emu-xref href="#sec-tostring"></emu-xref>.</p>
</emu-clause>
"##;
        let config = SpecsConfig::default();
        let read = |path: &str| match path {
            "es/spec.html" => Some(es.to_string()),
            "p/spec.emu" => Some(proposal.to_string()),
            _ => None,
        };
        let (_, _, es_biblio) =
            analyze_with_imports("es/spec.html", &read, "es", &config, &[]).unwrap();
        let (files, stats, _) =
            analyze_with_imports("p/spec.emu", &read, "p", &config, &[&es_biblio]).unwrap();
        assert_eq!(
            summary(&files[0].1),
            vec![
                "2:6-14 def SPEC_p_sec@2Dfrob | Frob",
                "4:18-26 use SPEC_es_sec@2Dtostring | Frob step 1",
                "4:31-36 use SPEC_es_realm | Frob step 1",
                "5:14-18 use SPEC_p_sec@2Dfrob | Frob step 2",
                "5:25-34 use SPEC_es_prod@2DStatement | Frob step 2",
                "7:9-40 use SPEC_es_sec@2Dtostring | Frob",
            ]
        );
        assert_eq!((stats.biblio_references, stats.unresolved), (4, 0));
        let package = r#"{"scripts": {"build": "ecmarkup --load-biblio @tc39/ecma262-biblio --load-biblio @tc39/ecma402-biblio spec.emu out.html"}}"#;
        assert_eq!(
            loaded_biblios(package),
            vec!["https://tc39.es/ecma262/", "https://tc39.es/ecma402/"]
        );
        // (ecmarkup before 14 had ECMA-262's built in.)
        assert_eq!(
            loaded_biblios(r#"{"devDependencies": {"ecmarkup": "^12.1.0"}}"#),
            vec!["https://tc39.es/ecma262/"]
        );
        assert!(loaded_biblios(r#"{"devDependencies": {"ecmarkup": "^20.0.0"}}"#).is_empty());
    }

    #[test]
    fn test_imports() {
        let files: HashMap<&str, &str> = HashMap::from([
            (
                "e/spec/index.html",
                "<emu-clause id=\"sec-intro\">\n  <h1>Intro</h1>\n  <emu-import href=\"terms.html\"></emu-import>\n</emu-clause>\n<emu-import href=\"parts/use.html\"></emu-import>\n",
            ),
            (
                "e/spec/terms.html",
                "<p>A <dfn id=\"widget\">widget</dfn>.</p>\n<emu-clause id=\"sec-frob\" aoid=\"Frob\"><h1>Frob ( )</h1></emu-clause>\n",
            ),
            (
                "e/spec/parts/use.html",
                "<emu-clause id=\"sec-use\">\n  <h1>Use</h1>\n  <emu-alg>\n    1. Frob() a widget.\n  </emu-alg>\n  <emu-import href=\"../missing.html\"></emu-import>\n</emu-clause>\n",
            ),
        ]);
        let read = |path: &str| files.get(path).map(|s| s.to_string());
        let (records, _, _) =
            analyze_with_imports("e/spec/index.html", &read, "e", &SpecsConfig::default(), &[])
                .unwrap();
        let summaries: Vec<(String, Vec<String>)> = records
            .iter()
            .map(|(path, records)| (path.clone(), summary(records)))
            .collect();
        assert_eq!(
            summaries,
            vec![
                (
                    "e/spec/index.html".to_string(),
                    vec!["2:6-11 def SPEC_e_sec@2Dintro | Intro".to_string()]
                ),
                (
                    "e/spec/terms.html".to_string(),
                    vec![
                        "1:22-28 def SPEC_e_widget | Intro".to_string(),
                        "2:42-50 def SPEC_e_sec@2Dfrob | Frob".to_string(),
                    ]
                ),
                (
                    "e/spec/parts/use.html".to_string(),
                    vec![
                        "2:6-9 def SPEC_e_sec@2Duse | Use".to_string(),
                        "4:7-11 use SPEC_e_sec@2Dfrob | Use step 1".to_string(),
                        "4:16-22 use SPEC_e_widget | Use step 1".to_string(),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn test_operations() {
        let src = r##"<emu-clause id="sec-ops">
  <h1>Operations</h1>
  <p>Let <emu-eqn id="eqn-modulo" aoid="modulo">_x_ modulo _y_</emu-eqn> be it.</p>
  <emu-table id="table-methods" type="abstract methods">
    <table>
      <tr><th>Method</th><th>Purpose</th></tr>
      <tr id="abstract-frob"><td>Frob ( )</td><td>Frobs.</td></tr>
    </table>
  </emu-table>
  <emu-table id="table-internal" type="internal methods">
    <table>
      <tr id="internal-method-get"><td>[[Get]] ( _P_ )</td><td>Gets.</td></tr>
    </table>
  </emu-table>
  <emu-alg>
    1. Return modulo(1), ? _rec_.Frob(), and _O_.[[Get]](_P_), but not [[Get]].
  </emu-alg>
  <emu-grammar type="definition">
    Thing : `x`
  </emu-grammar>
  <emu-prodref name="Thing"></emu-prodref>
</emu-clause>
"##;
        let (records, stats) = analyze(src, "es", &SpecsConfig::default());
        let uses: Vec<String> = summary(&records)
            .into_iter()
            .filter(|s| s.contains(" use "))
            .collect();
        assert_eq!(
            uses,
            vec![
                "16:14-20 use SPEC_es_eqn@2Dmodulo | Operations step 1",
                "16:33-37 use SPEC_es_abstract@2Dfrob | Operations step 1",
                "16:49-56 use SPEC_es_internal@2Dmethod@2Dget | Operations step 1",
                "21:2-28 use SPEC_es_prod@2DThing | Operations",
            ]
        );
        assert_eq!((stats.operations, stats.unresolved), (3, 0));
    }

    #[test]
    fn test_analysis() {
        let src = r##"<emu-clause id="sec-terms">
  <h1>Terms</h1>
  <p>A <dfn id="value" variants="values">value</dfn> and a <dfn>widget</dfn>. Values, not valuesque, nor a widget.</p>
  <emu-clause id="sec-tostring" type="abstract operation" oldids="sec-old">
    <h1>
      ToString (
        _arg_: a value,
      ): a String
    </h1>
    <dl class="header">
    </dl>
    <emu-alg>
      1. If _arg_ is a value, return ToString(_arg_).
      1. [id="step-tostring-call"] Else:
        1. Return Call(_arg_) of |Thing|, not a Call.
    </emu-alg>
    <p>ToString(_arg_) is in its own clause.</p>
  </emu-clause>
  <emu-grammar type="definition">
    Thing :
      `thing` OtherThing?
    OtherThing :: one of
      `a` `b`
  </emu-grammar>
  <p>See <emu-xref href="#sec-tostring"></emu-xref>, <emu-xref aoid="ToString"></emu-xref> and <emu-xref href="#nope"></emu-xref>.</p>
</emu-clause>
<emu-clause id="sec-call" aoid="Call"><h1>Call</h1></emu-clause>
"##;
        let (records, stats) = analyze(src, "es", &SpecsConfig::default());
        assert_eq!(
            summary(&records),
            vec![
                "2:6-11 def SPEC_es_sec@2Dterms | Terms",
                "3:41-46 def SPEC_es_value | Terms",
                // ("A widget" is a link to the clause it's in, so it isn't.)
                "3:78-84 use SPEC_es_value | Terms",
                "4:2-75 def SPEC_es_sec@2Dold | ToString",
                "6:6-16 def SPEC_es_sec@2Dtostring | ToString",
                // (A structured header's types are autolinked.)
                "7:17-22 use SPEC_es_value | ToString",
                "13:23-28 use SPEC_es_value | ToString step 1",
                // (In an algorithm, a link to the clause it's in is.)
                "13:37-45 use SPEC_es_sec@2Dtostring | ToString step 1",
                "14:6-8 def SPEC_es_step@2Dtostring@2Dcall | ToString step 2",
                "15:18-22 use SPEC_es_sec@2Dcall | ToString step 2.a",
                "15:34-39 use SPEC_es_prod@2DThing | ToString step 2.a",
                "20:4-9 def SPEC_es_prod@2DThing | Terms",
                "21:14-24 use SPEC_es_prod@2DOtherThing | Terms",
                "22:4-14 def SPEC_es_prod@2DOtherThing | Terms",
                "25:9-40 use SPEC_es_sec@2Dtostring | Terms",
                "25:53-79 use SPEC_es_sec@2Dtostring | Terms",
                "27:42-46 def SPEC_es_sec@2Dcall | Call",
            ]
        );
        assert_eq!(
            (stats.clauses, stats.operations, stats.terms, stats.productions, stats.unresolved),
            (3, 2, 2, 2, 1)
        );
        let structured: Vec<String> = records
            .iter()
            .filter_map(|record| match record {
                Record::Structured(loc, value) => Some(format!(
                    "{} {} {}",
                    loc.lineno,
                    value["sym"].as_str().unwrap(),
                    value["kind"].as_str().unwrap()
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            structured,
            vec![
                "2 SPEC_es_sec@2Dterms specSection",
                "3 SPEC_es_value specDfn",
                "6 SPEC_es_sec@2Dtostring specAbstractOp",
                "20 SPEC_es_prod@2DThing specGrammar",
                "22 SPEC_es_prod@2DOtherThing specGrammar",
                "27 SPEC_es_sec@2Dcall specAbstractOp",
            ]
        );
    }
}
