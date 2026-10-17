//! Roll-ups of a tree's edges between functions into edges between classes
//! and between subsystems (see crossref's `rollup_calls`): how many calls go
//! from one class's methods to another's, and from one subsystem's code to
//! another's, with the biggest contributors as examples to drill down into.
//! They're what block diagrams are drawn from (see `cmd_block_diagram`), and
//! they summarize how areas of the code interact without walking every
//! method.
//!
//! Fields (and functions) get data roll-ups too, of the edges between them
//! and other objects' fields (see `dataflow::summary::FieldEdge`), for
//! diagrams of how data moves between data structures' fields.
//!
//! Subsystems (from the files' per-file info, ex: "Core/DOM/Workers") get
//! pseudo-symbols of their own, `SUBSYS_<mangled subsystem>`, whose crossref
//! entries have their roll-ups, and `SUBSYS_` (no subsystem) is the whole
//! tree's: its members are the subsystems, and its `edges` the biggest edges
//! between them.

use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

use super::analysis_manglings::mangle_file;

/// How many edges (and members) a class's or subsystem's roll-up keeps in
/// each direction (enough that hubs' are complete, for finding paths between
/// classes), how many of them keep examples and how many each keeps, and how
/// many edges between subsystems the whole tree's keeps.
pub const ROLLUP_EDGES: usize = 256;
pub const ROLLUP_EXAMPLE_EDGES: usize = 32;
pub const ROLLUP_EXAMPLES: usize = 6;
pub const ROLLUP_MEMBERS: usize = 64;
pub const ROLLUP_TREE_EDGES: usize = 4000;

/// The prefix of subsystems' pseudo-symbols.
pub const SUBSYSTEM_PREFIX: &str = "SUBSYS_";

/// A subsystem's pseudo-symbol (ex: `SUBSYS_Core/DOM/Workers`).
pub fn subsystem_sym(subsystem: &str) -> Ustr {
    ustr(&format!("{}{}", SUBSYSTEM_PREFIX, mangle_file(subsystem)))
}

/// The whole tree's pseudo-symbol, whose members are its subsystems.
pub fn root_subsystem_sym() -> Ustr {
    ustr(SUBSYSTEM_PREFIX)
}

/// Whether a function (by its pretty name) is plumbing whose calls don't say
/// how areas interact: refcounting, assertions, logging.  (Roll-ups leave
/// out calls to them, as dataflow summaries do; and callees labeled
/// `calls-diagram:stop`.)
pub fn is_boring_callee(pretty: &str) -> bool {
    let name = pretty.rsplit("::").next().unwrap_or(pretty);
    matches!(
        name,
        "AddRef" | "Release" | "log_print" | "log_test" | "__builtin_expect" | "printf_stderr"
    ) || name.starts_with("Assert")
        || name.starts_with("MOZ_")
        || pretty.starts_with("mozilla::detail::log")
}

/// An edge to (or from) another class or subsystem.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupEdge {
    /// The other end: a class's symbol, or a subsystem's pseudo-symbol.
    pub sym: Ustr,
    pub pretty: Ustr,
    /// How many calls (call sites), or, for data, flows (functions with
    /// them).
    pub count: u32,
    /// How many of the calls are of accessors (getters, setters, trivial
    /// methods, by their dataflow summaries), which say little about how
    /// the areas interact.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub accessors: u32,
    /// How many of the calls are IPDL messages (from generated `Send`
    /// methods to actors' `Recv` implementations).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ipc: u32,
    /// How many distinct pairs of functions (for subsystems, of classes or
    /// free functions) the calls are between.
    pub pairs: u32,
    /// The pairs with the most calls, for drilling down.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<RollupExample>,
    /// For a field's or function's data edges: some of the functions the
    /// data flows in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<Ustr>,
}

/// A pair of functions (for subsystems, of classes or free functions) an
/// edge's calls are between, and how many.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupExample {
    pub from: Ustr,
    #[serde(rename = "fromPretty")]
    pub from_pretty: Ustr,
    pub to: Ustr,
    #[serde(rename = "toPretty")]
    pub to_pretty: Ustr,
    pub count: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A member of a subsystem (a class, or a free function) and its calls in
/// and out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupMember {
    pub sym: Ustr,
    pub pretty: Ustr,
    pub count: u32,
}

/// An edge between two subsystems, for the whole tree's roll-up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupPair {
    pub from: Ustr,
    pub to: Ustr,
    pub count: u32,
    pub pairs: u32,
}

/// A class's or subsystem's roll-up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rollup {
    /// Its edges to others, most calls first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out: Vec<RollupEdge>,
    /// Others' edges to it, most calls first.
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "in")]
    pub into: Vec<RollupEdge>,
    /// How many edges there were in each direction before only the biggest
    /// were kept.
    #[serde(default, rename = "outTotal")]
    pub out_total: u32,
    #[serde(default, rename = "inTotal")]
    pub in_total: u32,
    /// Calls within it (between its own methods, or its own code).
    #[serde(default)]
    pub internal: u32,
    /// For subsystems: their members with the most calls in, out and within.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<RollupMember>,
    /// For the whole tree: the biggest edges between subsystems.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<RollupPair>,
}

/// What a try push changes about a class's or subsystem's roll-up (see
/// `build-try-delta`): the roll-up of the calls in the files the push
/// changes, before it (the base's) and after it (the push's).  A function's
/// calls are in the file defining it, so the tree's roll-up, less the
/// before, plus the after, is the push's (`Rollup::with_delta`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Rollup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Rollup>,
}

/// Counts (signed, while adding and subtracting) of an edge.
#[derive(Default)]
struct EdgeSum {
    pretty: Ustr,
    count: i64,
    accessors: i64,
    ipc: i64,
    pairs: i64,
    via: Vec<Ustr>,
    examples: Vec<(RollupExample, i64)>,
}

impl EdgeSum {
    fn add(&mut self, edge: &RollupEdge, sign: i64) {
        if self.pretty.is_empty() {
            self.pretty = edge.pretty;
        }
        self.count += sign * edge.count as i64;
        self.accessors += sign * edge.accessors as i64;
        self.ipc += sign * edge.ipc as i64;
        self.pairs += sign * edge.pairs as i64;
        if self.via.is_empty() {
            self.via = edge.via.clone();
        }
        for example in &edge.examples {
            let delta = sign * example.count as i64;
            match self
                .examples
                .iter_mut()
                .find(|(e, _)| e.from == example.from && e.to == example.to)
            {
                Some((_, count)) => *count += delta,
                None => self.examples.push((example.clone(), delta)),
            }
        }
    }

    fn edge(self, sym: Ustr) -> Option<RollupEdge> {
        let clamp = |n: i64| n.max(0) as u32;
        if self.count <= 0 {
            return None;
        }
        let mut examples: Vec<RollupExample> = self
            .examples
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(mut example, count)| {
                example.count = count as u32;
                example
            })
            .collect();
        examples.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });
        examples.truncate(ROLLUP_EXAMPLES);
        Some(RollupEdge {
            sym,
            pretty: self.pretty,
            count: clamp(self.count),
            accessors: clamp(self.accessors),
            ipc: clamp(self.ipc),
            pairs: clamp(self.pairs).max(1),
            examples,
            via: self.via,
        })
    }
}

/// Edges' sums by their other ends, in the order they were first seen.
fn sum_edges(lists: &[(&[RollupEdge], i64)]) -> Vec<RollupEdge> {
    let mut order: Vec<Ustr> = vec![];
    let mut sums: std::collections::HashMap<Ustr, EdgeSum> = std::collections::HashMap::new();
    for (edges, sign) in lists {
        for edge in edges.iter() {
            sums.entry(edge.sym)
                .or_insert_with(|| {
                    order.push(edge.sym);
                    EdgeSum::default()
                })
                .add(edge, *sign);
        }
    }
    let mut edges: Vec<RollupEdge> = order
        .into_iter()
        .filter_map(|sym| sums.remove(&sym).unwrap().edge(sym))
        .collect();
    edges.sort_by(|a, b| b.count.cmp(&a.count).then(a.sym.cmp(&b.sym)));
    edges.truncate(ROLLUP_EDGES);
    for edge in edges.iter_mut().skip(ROLLUP_EXAMPLE_EDGES) {
        edge.examples.clear();
    }
    edges
}

impl Rollup {
    /// The roll-up a try push makes this (the tree's) one (see
    /// `RollupDelta`), or None if it leaves it nothing.
    pub fn with_delta(&self, delta: &RollupDelta) -> Option<Rollup> {
        let empty = Rollup::default();
        let before = delta.before.as_ref().unwrap_or(&empty);
        let after = delta.after.as_ref().unwrap_or(&empty);
        let out = sum_edges(&[(&self.out, 1), (&before.out, -1), (&after.out, 1)]);
        let into = sum_edges(&[(&self.into, 1), (&before.into, -1), (&after.into, 1)]);
        // (The edges the tree's lists left out, which the push leaves.)
        let out_hidden = self.out_total.saturating_sub(self.out.len() as u32);
        let in_hidden = self.in_total.saturating_sub(self.into.len() as u32);
        let signed = |n: u32| n as i64;
        let internal = signed(self.internal) - signed(before.internal) + signed(after.internal);

        let mut member_order: Vec<Ustr> = vec![];
        let mut members: std::collections::HashMap<Ustr, (Ustr, i64)> =
            std::collections::HashMap::new();
        for (list, sign) in [
            (&self.members, 1),
            (&before.members, -1),
            (&after.members, 1),
        ] {
            for member in list {
                let entry = members.entry(member.sym).or_insert_with(|| {
                    member_order.push(member.sym);
                    (member.pretty, 0)
                });
                entry.1 += sign * member.count as i64;
            }
        }
        let mut members: Vec<RollupMember> = member_order
            .into_iter()
            .filter_map(|sym| {
                let (pretty, count) = members[&sym];
                (count > 0).then_some(RollupMember {
                    sym,
                    pretty,
                    count: count as u32,
                })
            })
            .collect();
        members.sort_by(|a, b| b.count.cmp(&a.count).then(a.sym.cmp(&b.sym)));
        members.truncate(ROLLUP_MEMBERS);

        let mut pair_order: Vec<(Ustr, Ustr)> = vec![];
        let mut pairs: std::collections::HashMap<(Ustr, Ustr), (i64, i64)> =
            std::collections::HashMap::new();
        for (list, sign) in [(&self.edges, 1), (&before.edges, -1), (&after.edges, 1)] {
            for pair in list {
                let entry = pairs.entry((pair.from, pair.to)).or_insert_with(|| {
                    pair_order.push((pair.from, pair.to));
                    (0, 0)
                });
                entry.0 += sign * pair.count as i64;
                entry.1 += sign * pair.pairs as i64;
            }
        }
        let mut edges: Vec<RollupPair> = pair_order
            .into_iter()
            .filter_map(|(from, to)| {
                let (count, pair_count) = pairs[&(from, to)];
                (count > 0).then_some(RollupPair {
                    from,
                    to,
                    count: count as u32,
                    pairs: pair_count.max(1) as u32,
                })
            })
            .collect();
        edges.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });
        edges.truncate(ROLLUP_TREE_EDGES);

        let rollup = Rollup {
            out_total: out.len() as u32 + out_hidden,
            in_total: into.len() as u32 + in_hidden,
            out,
            into,
            internal: internal.max(0) as u32,
            members,
            edges,
        };
        let is_empty = rollup.out.is_empty()
            && rollup.into.is_empty()
            && rollup.members.is_empty()
            && rollup.edges.is_empty()
            && rollup.internal == 0;
        (!is_empty).then_some(rollup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(sym: &str, count: u32, pairs: u32, examples: &[(&str, &str, u32)]) -> RollupEdge {
        RollupEdge {
            sym: ustr(sym),
            pretty: ustr(sym),
            count,
            pairs,
            examples: examples
                .iter()
                .map(|(from, to, count)| RollupExample {
                    from: ustr(from),
                    from_pretty: ustr(from),
                    to: ustr(to),
                    to_pretty: ustr(to),
                    count: *count,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_with_delta() {
        // The tree: A calls B 5 times (by a::f in a changed file 2, by a::g in
        // an unchanged one 3) and C once (in the changed file).
        let tree = Rollup {
            out: vec![
                edge("B", 5, 2, &[("a::g", "b::h", 3), ("a::f", "b::h", 2)]),
                edge("C", 1, 1, &[("a::f", "c::k", 1)]),
            ],
            out_total: 2,
            internal: 4,
            ..Default::default()
        };
        // The push's changed file: a::f calls B 3 times (was 2), and D once
        // (new), and no longer C.
        let delta = RollupDelta {
            before: Some(Rollup {
                out: vec![
                    edge("B", 2, 1, &[("a::f", "b::h", 2)]),
                    edge("C", 1, 1, &[("a::f", "c::k", 1)]),
                ],
                out_total: 2,
                internal: 1,
                ..Default::default()
            }),
            after: Some(Rollup {
                out: vec![
                    edge("B", 3, 1, &[("a::f", "b::h", 3)]),
                    edge("D", 1, 1, &[("a::f", "d::m", 1)]),
                ],
                out_total: 2,
                ..Default::default()
            }),
        };
        let pushed = tree.with_delta(&delta).unwrap();
        let summary: Vec<(&str, u32, u32, Vec<(String, u32)>)> = pushed
            .out
            .iter()
            .map(|e| {
                (
                    e.sym.as_str(),
                    e.count,
                    e.pairs,
                    e.examples
                        .iter()
                        .map(|x| (format!("{}->{}", x.from, x.to), x.count))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "B",
                    6,
                    2,
                    vec![("a::f->b::h".to_string(), 3), ("a::g->b::h".to_string(), 3)]
                ),
                ("D", 1, 1, vec![("a::f->d::m".to_string(), 1)]),
            ]
        );
        assert_eq!(pushed.out_total, 2);
        assert_eq!(pushed.internal, 3);

        // A push removing everything leaves nothing.
        let gone = RollupDelta {
            before: Some(tree.clone()),
            after: None,
        };
        assert_eq!(tree.with_delta(&gone), None);
        // And a class only the push has gets the push's.
        let new = RollupDelta {
            before: None,
            after: delta.after.clone(),
        };
        assert_eq!(
            Rollup::default().with_delta(&new).map(|r| r.out.len()),
            Some(2)
        );
    }

    #[test]
    fn test_is_boring_callee() {
        assert!(is_boring_callee(
            "mozilla::dom::WorkerPrivate::AssertIsOnWorkerThread"
        ));
        assert!(is_boring_callee("nsISupports::AddRef"));
        assert!(!is_boring_callee("mozilla::dom::WorkerPrivate::Dispatch"));
    }

    #[test]
    fn test_subsystem_sym() {
        assert_eq!(
            subsystem_sym("Core/DOM/Workers").as_str(),
            "SUBSYS_Core/DOM/Workers"
        );
        assert_eq!(
            subsystem_sym("Core/DOM/Core HTML").as_str(),
            "SUBSYS_Core/DOM/Core@20HTML"
        );
    }
}
