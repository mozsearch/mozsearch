//! The HTML Standard's Wattsi `source`: its definitions and its
//! cross-references, resolved as Wattsi does for html.spec.whatwg.org (see
//! wattsi.pas, whose names are in quotes below), with their positions, which
//! Wattsi's output doesn't keep.
//!
//! - Content with `w-nohtml` or `w-dev` attributes isn't in that edition, so
//!   it's skipped (and doesn't take ids).
//! - A term's topic ("GetTopicIdentifier") is its element's `data-x`, or its
//!   only child element's topic, or its text, lowercased and with runs of
//!   spaces collapsed ("MungeStringToTopic").
//! - Headings (`<h2>`..`<h6>`) and `<dfn>`s get ids from their text and
//!   topics ("MungeTopicToID"), unless they have one, with "-2", "-3", ...
//!   for ids already taken, in document order ("GenerateID").  A `<dfn>` in a
//!   heading defines its topic as the heading, and gets no id.
//! - Cross-references are `<span>`s (but for `pubdate` and `pubyear` ones),
//!   `<code>`s, and `<i>`s and `<var>`s with `data-x`, outside `<dfn>`s
//!   ("SaveCrossReference"), whose topics are a `<dfn>`'s (or else a
//!   `subdfn`'s), unless they're in a link (or a cross-reference which
//!   resolved, which becomes one).  `<!--INSERT TRACKING-->` notes become
//!   references to "tracking vector".
//!
//! A `<dfn>` with `data-x-href` defers to another spec's term (ex: DOM's
//! "node"), which references to it link to, so they're uses of that term's
//! symbol, as is the `<dfn>` itself (besides defining its own id), if the URL
//! is to one of the tree's specs (see `SpecsConfig::url_symbol`).  Links
//! (`<a href>`) to an id or one of the specs, and bibliography references
//! (`<ref>`) are uses too.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::markup::{Doc, NodeKind};
use super::{
    InContext, Lines, SpecsConfig, spec_records, spec_symbol,
    unescape_context_name as context_name,
};
use crate::semantic_analyzer::{Record, sort_records};
use crate::tree_sitter_support::spec_tokenizer::html_contexts;

/// "MungeStringToTopic": lowercase ASCII letters, drop `#`s, and collapse
/// runs of spaces and newlines into a space (dropping a trailing one).
pub fn munge_topic(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut had_space = false;
    for c in text.chars() {
        match c {
            '#' => {}
            ' ' | '\n' => {
                if !had_space {
                    out.push(' ');
                    had_space = true;
                }
            }
            c => {
                out.push(c.to_ascii_lowercase());
                had_space = false;
            }
        }
    }
    if had_space {
        out.pop();
    }
    out
}

/// "MungeTopicToID": lowercase ASCII letters, drop `"`, `?`, and `` ` ``, and
/// turn runs of spaces, newlines, and `<>[\]^{|}%` into a `-` (dropping
/// leading and trailing ones).
pub fn munge_id(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut had_space = true;
    for c in text.chars() {
        match c {
            ' ' | '\n' | '<' | '>' | '[' | '\\' | ']' | '^' | '{' | '|' | '}' | '%' => {
                if !had_space {
                    out.push('-');
                    had_space = true;
                }
            }
            '"' | '?' | '`' => {}
            c => {
                out.push(c.to_ascii_lowercase());
                had_space = false;
            }
        }
    }
    if had_space && !out.is_empty() {
        out.pop();
    }
    out
}

/// The ids in the document, taken and generated ("GenerateID").
#[derive(Default)]
struct Ids {
    taken: HashSet<String>,
}

impl Ids {
    fn generate(&mut self, base: &str) -> String {
        let mut id = base.to_string();
        let mut n = 2;
        while self.taken.contains(&id) {
            id = format!("{}-{}", base, n);
            n += 1;
        }
        self.taken.insert(id.clone());
        id
    }
}

/// What defines a topic.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Dfn(usize),
    Heading(usize),
}

/// The analysis's counts, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    pub headings: usize,
    pub dfns: usize,
    pub references: usize,
    pub unresolved: usize,
    pub links: usize,
}

struct Wattsi<'a, 'd> {
    doc: &'d Doc<'a>,
    excluded: Vec<bool>,
    ids: Ids,
    /// Elements' ids: headings', `<dfn>`s', and `subdfn`s'.
    node_ids: HashMap<usize, String>,
    dfns: HashMap<String, Target>,
    subdfns: HashMap<String, usize>,
    /// The cross-references, and their topics, in document order.
    xrefs: Vec<(usize, String)>,
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h2" | "h3" | "h4" | "h5" | "h6")
}

impl<'a, 'd> Wattsi<'a, 'd> {
    /// "GetTopicIdentifier".
    fn topic(&self, node: usize) -> String {
        if let Some(x) = self.doc.attr(node, "data-x") {
            return munge_topic(&x);
        }
        let children = &self.doc.nodes[node].children;
        if children.len() == 1 && self.doc.name(children[0]).is_some() {
            return self.topic(children[0]);
        }
        munge_topic(&self.doc.text_content(node))
    }

    fn in_element(&self, node: usize, name: &str) -> bool {
        self.doc.ancestors(node).any(|n| self.doc.is(n, name))
    }

    fn heading_of(&self, node: usize) -> Option<usize> {
        self.doc
            .ancestors(node)
            .find(|&n| self.doc.name(n).is_some_and(is_heading))
    }

    /// "SaveCrossReference".
    fn save_xref(&mut self, node: usize) {
        let topic = self.topic(node);
        if topic.is_empty() {
            return;
        }
        if self.doc.has_attr(node, "subdfn") && !self.subdfns.contains_key(&topic) {
            self.subdfns.insert(topic.clone(), node);
            // (Wattsi's ids for them in its non-developer editions.)
            let id = match self.doc.attr(node, "id") {
                Some(id) => id,
                None => self.ids.generate(&munge_id(&format!("{}-dev", topic))),
            };
            self.node_ids.insert(node, id);
        }
        self.xrefs.push((node, topic));
    }

    /// Find the definitions and cross-references, giving elements their ids.
    fn scan(&mut self) {
        let doc = self.doc;
        for node in 1..doc.nodes.len() {
            let Some(name) = doc.name(node) else {
                continue;
            };
            if self.excluded[node] {
                continue;
            }
            match name {
                heading if is_heading(heading) => {
                    let id = match doc.attr(node, "id") {
                        Some(id) => id,
                        None => {
                            let base = munge_id(&doc.text_content(node));
                            let base = if base.is_empty() {
                                "blank-heading".to_string()
                            } else {
                                base
                            };
                            self.ids.generate(&base)
                        }
                    };
                    self.node_ids.insert(node, id);
                }
                "dfn" => {
                    let topic = self.topic(node);
                    let explicit = doc.attr(node, "id");
                    if topic.is_empty() {
                        if let Some(id) = explicit {
                            self.node_ids.insert(node, id);
                        }
                        continue;
                    }
                    // (A `<dfn>` in a heading keeps an id it has, as an anchor,
                    // but references to its topic are to the heading.)
                    if let Some(heading) = self.heading_of(node) {
                        if let Some(id) = explicit {
                            self.node_ids.insert(node, id);
                        }
                        self.dfns.entry(topic).or_insert(Target::Heading(heading));
                        continue;
                    }
                    let id = match explicit {
                        Some(id) => id,
                        None => self.ids.generate(&munge_id(&topic)),
                    };
                    self.node_ids.insert(node, id);
                    self.dfns.entry(topic).or_insert(Target::Dfn(node));
                }
                "code" if !self.in_element(node, "dfn") => self.save_xref(node),
                "span" if !self.in_element(node, "dfn") => {
                    let class = doc.attr(node, "class").unwrap_or_default();
                    if class != "pubdate" && class != "pubyear" {
                        self.save_xref(node);
                    }
                }
                "i" | "var" if doc.has_attr(node, "data-x") && !self.in_element(node, "dfn") => {
                    self.save_xref(node)
                }
                _ => {}
            }
        }
    }

    /// The element which defines the topic of the cross-reference, if any.
    fn resolve(&self, node: usize, topic: &str) -> Option<usize> {
        let target = match self.dfns.get(topic) {
            Some(Target::Dfn(n) | Target::Heading(n)) => *n,
            None => *self.subdfns.get(topic)?,
        };
        (target != node).then_some(target)
    }
}

/// A definition: its symbol, what it is, and its pretty name.
struct Def {
    sym: String,
    what: &'static str,
    pretty: String,
    /// For a `<dfn>` deferring to another spec's term, that term's symbol.
    defers_to: Option<String>,
}

/// The text of an element, with whitespace collapsed.
fn words(doc: &Doc, node: usize) -> String {
    doc.text_content(node)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The records of a Wattsi source, of the spec with `shortname`, among
/// `config`'s specs.
pub fn analyze(src: &str, shortname: &str, config: &SpecsConfig) -> (Vec<Record>, Stats) {
    let doc = Doc::parse(src);
    let mut excluded = vec![false; doc.nodes.len()];
    let mut ids = Ids::default();
    for node in 1..doc.nodes.len() {
        let parent = doc.nodes[node].parent.unwrap_or(0);
        excluded[node] = excluded[parent]
            || doc.has_attr(node, "w-nohtml")
            || doc.has_attr(node, "w-dev");
        if !excluded[node]
            && let Some(id) = doc.attr(node, "id")
        {
            ids.taken.insert(id);
        }
    }
    let mut wattsi = Wattsi {
        doc: &doc,
        excluded,
        ids,
        node_ids: HashMap::new(),
        dfns: HashMap::new(),
        subdfns: HashMap::new(),
        xrefs: vec![],
    };
    wattsi.scan();

    let mut stats = Stats::default();
    let mut defs: BTreeMap<usize, Def> = BTreeMap::new();
    for (&node, id) in &wattsi.node_ids {
        let name = doc.name(node).unwrap_or("");
        let sym = spec_symbol(shortname, id);
        let def = if is_heading(name) {
            stats.headings += 1;
            Def {
                sym,
                what: "section",
                pretty: format!("{} {}", shortname, words(&doc, node)),
                defers_to: None,
            }
        } else {
            stats.dfns += 1;
            let text = words(&doc, node);
            let pretty = match doc.attr(node, "data-dfn-for").or(doc.attr(node, "for")) {
                Some(dfn_for) if !dfn_for.is_empty() => format!("{} {}/{}", shortname, dfn_for, text),
                _ => format!("{} {}", shortname, text),
            };
            Def {
                sym,
                what: "dfn",
                pretty,
                defers_to: doc
                    .attr(node, "data-x-href")
                    .and_then(|url| config.url_symbol(&url)),
            }
        };
        defs.insert(node, def);
    }
    let by_id: HashMap<&str, usize> = wattsi
        .node_ids
        .iter()
        .map(|(&node, id)| (id.as_str(), node))
        .collect();

    // The records to write, by their elements (for their locations and
    // contexts).
    struct Pending {
        node: usize,
        def: bool,
        sym: String,
        what: &'static str,
        pretty: String,
    }
    let mut pending: Vec<Pending> = vec![];
    for (&node, def) in &defs {
        pending.push(Pending {
            node,
            def: true,
            sym: def.sym.clone(),
            what: def.what,
            pretty: def.pretty.clone(),
        });
        if let Some(external) = &def.defers_to {
            pending.push(Pending {
                node,
                def: false,
                sym: external.clone(),
                what: def.what,
                pretty: def.pretty.clone(),
            });
        }
    }
    // Cross-references, in document order, so that one in another which
    // resolved (and so became a link) is skipped.
    let mut linked: HashSet<usize> = HashSet::new();
    for (node, topic) in &wattsi.xrefs {
        let node = *node;
        if doc
            .ancestors(node)
            .any(|n| doc.is(n, "a") || linked.contains(&n))
        {
            continue;
        }
        stats.references += 1;
        let Some(target) = wattsi.resolve(node, topic) else {
            stats.unresolved += 1;
            continue;
        };
        let Some(def) = defs.get(&target) else {
            stats.unresolved += 1;
            continue;
        };
        linked.insert(node);
        pending.push(Pending {
            node,
            def: false,
            sym: def.defers_to.clone().unwrap_or_else(|| def.sym.clone()),
            what: def.what,
            pretty: def.pretty.clone(),
        });
    }
    // Links, bibliography references, and tracking vector notes (which are
    // references to "tracking vector").
    for node in 1..doc.nodes.len() {
        if wattsi.excluded[node] {
            continue;
        }
        if doc.nodes[node].kind == NodeKind::Comment
            && doc.src[doc.nodes[node].range.clone()] == *"<!--INSERT TRACKING-->"
        {
            if let Some(def) = wattsi
                .resolve(node, "tracking vector")
                .and_then(|target| defs.get(&target))
            {
                pending.push(Pending {
                    node,
                    def: false,
                    sym: def.sym.clone(),
                    what: def.what,
                    pretty: def.pretty.clone(),
                });
            }
            continue;
        }
        match doc.name(node) {
            Some("a") => {
                let Some(href) = doc.attr(node, "href") else {
                    continue;
                };
                let (sym, what, pretty) = if let Some(id) = href.strip_prefix('#') {
                    match by_id.get(id).and_then(|n| defs.get(n)) {
                        Some(def) => (def.sym.clone(), def.what, def.pretty.clone()),
                        None if wattsi.ids.taken.contains(id) => (
                            spec_symbol(shortname, id),
                            "anchor",
                            format!("{} #{}", shortname, id),
                        ),
                        None => continue,
                    }
                } else if let Some(sym) = config.url_symbol(&href) {
                    (sym, "anchor", href.clone())
                } else {
                    continue;
                };
                stats.links += 1;
                pending.push(Pending {
                    node,
                    def: false,
                    sym,
                    what,
                    pretty,
                });
            }
            Some("ref") => {
                let name = doc.text_content(node);
                if name.is_empty() {
                    continue;
                }
                pending.push(Pending {
                    node,
                    def: false,
                    sym: spec_symbol(shortname, &format!("refs{}", name)),
                    what: "reference",
                    pretty: format!("{} [{}]", shortname, name),
                });
            }
            _ => {}
        }
    }

    // Contexts: the innermost algorithm (named and symbolized by its first
    // `<dfn>`), with the step, or else the section.
    let contexts = html_contexts(src, &doc.items);
    let heading_syms: HashMap<usize, &str> = defs
        .iter()
        .filter(|(_, def)| def.what == "section")
        .map(|(&node, def)| (doc.nodes[node].range.start, def.sym.as_str()))
        .collect();
    let mut dfn_starts: Vec<(usize, &str)> = defs
        .iter()
        .filter(|&(&node, def)| def.what == "dfn" && doc.is(node, "dfn"))
        .map(|(&node, def)| (doc.nodes[node].range.start, def.sym.as_str()))
        .collect();
    dfn_starts.sort();
    let sections: Vec<(usize, InContext)> = contexts
        .iter()
        .filter(|c| c.end.is_none())
        .map(|c| {
            (
                c.start,
                InContext {
                    name: context_name(&c.name),
                    sym: heading_syms.get(&c.start).unwrap_or(&"").to_string(),
                },
            )
        })
        .collect();
    let mut algorithms: Vec<(usize, usize, InContext)> = contexts
        .iter()
        .filter_map(|c| {
            let end = c.end?;
            let first = dfn_starts.partition_point(|&(start, _)| start < c.start);
            let sym = dfn_starts
                .get(first)
                .filter(|&&(start, _)| start < end)
                .map_or("", |&(_, sym)| sym);
            Some((
                c.start,
                end,
                InContext {
                    name: context_name(&c.name),
                    sym: sym.to_string(),
                },
            ))
        })
        .collect();
    algorithms.sort_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)));
    // (The furthest end of the algorithms up to each, to stop looking back.)
    let mut furthest = Vec::with_capacity(algorithms.len());
    let mut max_end = 0;
    for (_, end, _) in &algorithms {
        max_end = max_end.max(*end);
        furthest.push(max_end);
    }
    let context_at = |node: usize| -> InContext {
        let pos = doc.nodes[node].range.start;
        let mut i = algorithms.partition_point(|&(start, _, _)| start <= pos);
        while i > 0 && furthest[i - 1] > pos {
            i -= 1;
            let (start, end, context) = &algorithms[i];
            if *end > pos {
                let path = step_path(&doc, node, *start);
                return if path.is_empty() {
                    context.clone()
                } else {
                    InContext {
                        name: format!("{} step {}", context.name, path),
                        sym: context.sym.clone(),
                    }
                };
            }
        }
        let i = sections.partition_point(|&(start, _)| start <= pos);
        i.checked_sub(1)
            .map_or_else(InContext::default, |i| sections[i].1.clone())
    };

    let lines = Lines::new(src);
    let mut records = vec![];
    for p in pending {
        let range = doc.content_range(p.node);
        let range = if range.is_empty() {
            doc.items[doc.nodes[p.node].item].range().clone()
        } else {
            range
        };
        let context = context_at(p.node);
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

/// The number of the algorithm step (starting at `algorithm_start`) which
/// `node` is in, ex: "12.6", or "" if it's not in a step: its `<li>`s' numbers
/// in their `<ol>`s (with their `start`s and `value`s), outermost first.
fn step_path(doc: &Doc, node: usize, algorithm_start: usize) -> String {
    let mut numbers = vec![];
    let mut at = Some(node);
    while let Some(n) = at {
        if doc.nodes[n].range.start < algorithm_start {
            break;
        }
        let parent = doc.nodes[n].parent;
        if doc.is(n, "li")
            && let Some(ol) = parent.filter(|&p| doc.is(p, "ol"))
        {
            let mut number: i64 = doc
                .attr(ol, "start")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(1)
                - 1;
            for &child in &doc.nodes[ol].children {
                if !doc.is(child, "li") {
                    continue;
                }
                number = doc
                    .attr(child, "value")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(number + 1);
                if child == n {
                    break;
                }
            }
            numbers.push(number.to_string());
        }
        at = parent;
    }
    numbers.reverse();
    numbers.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_munging() {
        assert_eq!(munge_topic("The  #Document\n object "), "the document object");
        assert_eq!(munge_topic(" leading"), " leading");
        assert_eq!(munge_id("the document's <base> [URL]?"), "the-document's-base-url");
        assert_eq!(munge_id("  a%b  "), "a-b");
        assert_eq!(munge_id("Ünicode"), "Ünicode");
    }

    /// The target records as "line:col-col kind sym context (contextsym)".
    fn summary(records: &[Record]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) => Some(format!(
                    "{}:{}-{} {} {} {} ({})",
                    t.loc.lineno,
                    t.loc.col_start,
                    t.loc.col_end,
                    t.data.kind.to_ustr(),
                    t.data.sym,
                    t.data.context,
                    t.data.contextsym
                )),
                _ => None,
            })
            .collect()
    }

    fn config() -> SpecsConfig {
        toml::from_str(
            r#"
            [[spec]]
            name = "dom"
            rendered = "https://dom.spec.whatwg.org/"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn test_resolution() {
        let src = r##"<h2>Intro</h2>
<p>A <dfn>widget</dfn>, a <dfn data-x="concept-gadget">gadget</dfn>, and a <dfn id=w2>widget</dfn>.
<p>A <span>widget</span>, <span data-x="concept-gadget">gadgets</span>, <code><span>widget</span></code>, <a href="#w2"><span>widget</span></a>, <span>missing</span>.
<h3>The <dfn>doohickey</dfn> section</h3>
<p>See <span>doohickey</span>, <i data-x="widget">it</i>, and <a href="https://dom.spec.whatwg.org/#concept-node">nodes</a>.
<p w-dev>A <dfn>dev-only</dfn> term.
<p>A <dfn>dev-only</dfn> one, and an <dfn data-x-href="https://dom.spec.whatwg.org/#concept-tree">tree</dfn>, <span>tree</span>.
"##;
        let (records, stats) = analyze(src, "test", &config());
        assert_eq!(
            summary(&records),
            vec![
                "1:4-9 def SPEC_test_intro Intro (SPEC_test_intro)",
                "2:10-16 def SPEC_test_widget Intro (SPEC_test_intro)",
                "2:55-61 def SPEC_test_concept@2Dgadget Intro (SPEC_test_intro)",
                "2:86-92 def SPEC_test_w2 Intro (SPEC_test_intro)",
                // (The `<code>`'s `<span>` is in a link, and the `<a>`'s too.)
                "3:11-17 use SPEC_test_widget Intro (SPEC_test_intro)",
                "3:56-63 use SPEC_test_concept@2Dgadget Intro (SPEC_test_intro)",
                "3:78-97 use SPEC_test_widget Intro (SPEC_test_intro)",
                "3:120-139 use SPEC_test_w2 Intro (SPEC_test_intro)",
                "4:4-36 def SPEC_test_the@2Ddoohickey@2Dsection The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                "5:13-22 use SPEC_test_the@2Ddoohickey@2Dsection The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                "5:50-52 use SPEC_test_widget The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                "5:114-119 use SPEC_dom_concept@2Dnode The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                // (The `w-dev` one doesn't take the id.)
                "7:10-18 def SPEC_test_dev@2Donly The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                "7:98-102 def SPEC_test_tree The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                "7:98-102 use SPEC_dom_concept@2Dtree The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
                "7:116-120 use SPEC_dom_concept@2Dtree The doohickey section (SPEC_test_the@2Ddoohickey@2Dsection)",
            ]
        );
        assert_eq!((stats.headings, stats.dfns, stats.unresolved), (2, 5, 1));
    }

    #[test]
    fn test_ids() {
        // Generated ids skip ids that are taken, and the second definition of
        // a topic gets an id but isn't its definition.
        let src = r#"<h2 id=widget>Widgets</h2>
<p><dfn>Widget</dfn> and <dfn>widget</dfn>, <dfn subdfn>x</dfn>.
<p><span subdfn>gizmo</span> and <span>gizmo</span>.
"#;
        let (records, _) = analyze(src, "t", &SpecsConfig::default());
        let syms: Vec<String> = summary(&records)
            .into_iter()
            .map(|s| s.split(' ').take(3).collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(
            syms,
            vec![
                "1:14-21 def SPEC_t_widget",
                "2:8-14 def SPEC_t_widget@2D2",
                "2:30-36 def SPEC_t_widget@2D3",
                "2:56-57 def SPEC_t_x",
                "3:16-21 def SPEC_t_gizmo@2Ddev",
                "3:39-44 use SPEC_t_gizmo@2Ddev",
            ]
        );
    }

    #[test]
    fn test_steps() {
        let src = r#"<h2>Algorithms</h2>
<p>To <dfn>poke</dfn> a <var>thing</var>:</p>
<ol>
 <li><p>Let <var>x</var> be a <span>poke</span>.
 <li><p>Then:
  <ol start=3><li>Do a <span>poke</span>.<li value=7>And <span>poke</span>.<li><span>poke</span></ol>
</ol>
<p>Not a step: <span>poke</span>.
"#;
        let (records, _) = analyze(src, "t", &SpecsConfig::default());
        let contexts: Vec<String> = summary(&records)
            .into_iter()
            .map(|s| s.splitn(4, ' ').nth(3).unwrap().to_string())
            .collect();
        assert_eq!(
            contexts,
            vec![
                "Algorithms (SPEC_t_algorithms)",
                "poke (SPEC_t_poke)",
                "poke step 1 (SPEC_t_poke)",
                "poke step 2.3 (SPEC_t_poke)",
                "poke step 2.7 (SPEC_t_poke)",
                "poke step 2.8 (SPEC_t_poke)",
                "Algorithms (SPEC_t_algorithms)",
            ]
        );
    }
}
