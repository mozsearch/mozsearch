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
//! Records' contexts are their clauses, by their abstract operations' names
//! or titles, with the algorithm step (as rendered, ex: "ToString step 3.a").

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use lazy_static::lazy_static;
use regex::Regex;

use super::markup::{Doc, NodeKind};
use super::{InContext, Lines, SpecsConfig, spec_records, spec_symbol};
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
    fn new(namespaces: &HashMap<String, Namespace>, ns: &str) -> Autolinker {
        let mut words: HashMap<String, Entry> = HashMap::new();
        for is_term in [true, false] {
            let mut seen: HashSet<String> = HashSet::new();
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
                    let mut keys = vec![key.clone()];
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
                }
                current = namespace.parent.clone();
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
                    },
                ));
            }
        }
    }
    let mut old_ids: Vec<(usize, String)> = vec![];
    // Internal methods (ex: `[[GetOwnProperty]]`), and their anchors, which
    // only invocations (`_O_.[[GetOwnProperty]](...)`) link to.
    let mut internal_methods: HashMap<String, String> = HashMap::new();
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
                    let ns = namespaces.get_mut("").unwrap();
                    if !ns.ops.iter().any(|(key, _)| *key == aoid) {
                        ns.ops.push((
                            aoid,
                            Entry {
                                target,
                                is_term: false,
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
    let production = |ns: &str, name: &str| -> Option<String> {
        let mut current = Some(ns.to_string());
        while let Some(ns) = current {
            let namespace = namespaces.get(&ns)?;
            if let Some(id) = namespace.productions.get(name) {
                return Some(id.clone());
            }
            current = namespace.parent.clone();
        }
        None
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
    let lines = Lines::new(src);
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
            .or_insert_with(|| Autolinker::new(&namespaces, &ns));
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
                    Some(id) => {
                        let (what, pretty) = pretty_of(&id);
                        pending.push(Pending {
                            range: byte(span.start + 1)..byte(span.start + 1 + name.chars().count()),
                            def: false,
                            sym: spec_symbol(shortname, &id),
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
                if !in_alg[node] && current_id.as_deref() == Some(entry.target.as_str()) {
                    continue;
                }
                stats.autolinks += 1;
                let (what, pretty) = pretty_of(&entry.target);
                pending.push(Pending {
                    range: byte(segment.start + link.start)..byte(segment.start + link.end),
                    def: false,
                    sym: spec_symbol(shortname, &entry.target),
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
            if let Some(id) = production(&ns, m.as_str()) {
                let (what, pretty) = pretty_of(&id);
                pending.push(Pending {
                    range,
                    def: false,
                    sym: spec_symbol(shortname, &id),
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
                let Some(id) = production(&namespace_of(node), &name) else {
                    stats.unresolved += 1;
                    continue;
                };
                let (what, pretty) = pretty_of(&id);
                pending.push(Pending {
                    range: doc.items[doc.nodes[node].item].range().clone(),
                    def: false,
                    sym: spec_symbol(shortname, &id),
                    what,
                    pretty,
                });
            }
            Some("emu-xref") => {
                stats.xrefs += 1;
                let target = if let Some(href) = doc.attr(node, "href") {
                    href.strip_prefix('#')
                        .filter(|id| ids.contains(*id))
                        .map(str::to_string)
                } else if let Some(aoid) = doc.attr(node, "aoid") {
                    namespaces[""]
                        .ops
                        .iter()
                        .find(|(key, _)| *key == aoid)
                        .map(|(_, entry)| entry.target.clone())
                } else {
                    None
                };
                let Some(target) = target else {
                    stats.unresolved += 1;
                    continue;
                };
                let range = doc.content_range(node);
                let range = if src[range.clone()].trim().is_empty() {
                    doc.items[doc.nodes[node].item].range().clone()
                } else {
                    range
                };
                let (what, pretty) = pretty_of(&target);
                pending.push(Pending {
                    range,
                    def: false,
                    sym: spec_symbol(shortname, &target),
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

    let mut records = vec![];
    for p in pending {
        let range = if p.range.is_empty() {
            p.range.start..p.range.start + 1
        } else {
            p.range
        };
        let context = context_at(range.start);
        spec_records(
            &mut records,
            lines.location(&range),
            p.def,
            p.what,
            &p.pretty,
            &p.sym,
            &context,
        );
    }
    sort_records(&mut records);
    (records, stats)
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
    }
}
