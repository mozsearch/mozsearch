//! Which lines of the functions a patch changed are on the changes'
//! dataflow, and why (see `slicing_lines`): explore pages' excerpts of a try
//! push's changed definitions (see `format::outline_excerpts_json` and
//! static/js/outline-excerpts.js) prefer them to lines that aren't.
//!
//! Within each changed function (a slicing's unit; see `slice`), from the
//! events on its changed lines, in the function's events' order (which has no
//! control flow, so a loop's later writes don't reach its earlier reads):
//!
//! - the earlier events writing what they read (ex: where the field a changed
//!   line reads was written), and those writing what those read, a step
//!   further, which matter most;
//! - the later events reading what they write (ex: where a value a changed
//!   line computes is used), more if they test it (ex: a condition);
//! - the events reading or writing the values the changes' cluster carries
//!   elsewhere (the stations of its subway map's lines, see `subway`), which
//!   matter less.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::ir::*;
use super::slice::{Key, Slicing, place_keys};
use super::trace::Tracer;

/// How much each kind of line on a change's dataflow matters.
const WRITES_READ: f32 = 1.0;
const WRITES_READ_FURTHER: f32 = 0.7;
const TESTS_WRITTEN: f32 = 0.9;
const READS_WRITTEN: f32 = 0.8;
const CLUSTER_VALUE: f32 = 0.5;

/// The most reasons a line keeps.
const MAX_REASONS: usize = 3;

/// A line on a change's dataflow: how much it matters (0-1), and why.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LineImpact {
    #[serde(rename = "d")]
    pub score: f32,
    pub why: Vec<String>,
}

impl LineImpact {
    fn add(&mut self, score: f32, why: String) {
        self.score = self.score.max(score);
        if self.why.len() < MAX_REASONS && !self.why.contains(&why) {
            self.why.push(why);
        }
    }
}

/// The places an event writes: an assignment's place, and the places of a
/// call's arguments the callee may write through (references, pointers and
/// addresses; see `Arg::place`).
fn writes<'f>(f: &'f FuncIr, event: &'f Event) -> Vec<&'f Place> {
    match &event.kind {
        EventKind::Assign { place, .. } => vec![place],
        EventKind::Call(c) => f.calls[*c]
            .args
            .iter()
            .filter_map(|arg| arg.place.as_ref())
            .collect(),
        _ => vec![],
    }
}

/// The places an event reads.
fn reads<'f>(f: &'f FuncIr, event: &'f Event) -> Vec<&'f Place> {
    let of = |vals: &'f [Val]| {
        vals.iter().filter_map(|val| match &val.atom {
            Atom::Read(place) | Atom::Addr(place) => Some(place),
            _ => None,
        })
    };
    match &event.kind {
        EventKind::Assign { vals, .. } => of(vals).collect(),
        EventKind::Return { vals } | EventKind::Test { vals } => of(vals).collect(),
        EventKind::Call(c) => {
            let call = &f.calls[*c];
            call.args
                .iter()
                .chain(call.receiver.iter())
                .flat_map(|arg| of(&arg.vals))
                .collect()
        }
    }
}

/// Whether a place is worth following: not `this` itself, nor a name the
/// records don't have (ex: an unresolved macro's).
fn followed(place: &Place) -> bool {
    !(matches!(place.root, Root::This) && place.path.is_empty())
        && !matches!(&place.root, Root::Local(sym) if sym.starts_with("name:"))
}

/// The lines of the function `f` on the dataflow of its lines `changed`
/// (see the module's comment), with `cluster_keys`, the values their
/// cluster carries, but not the changed lines themselves.
pub fn func_impact(
    f: &FuncIr,
    changed: &BTreeSet<u32>,
    cluster_keys: &BTreeSet<Key>,
) -> BTreeMap<u32, LineImpact> {
    let mut lines: BTreeMap<u32, LineImpact> = BTreeMap::new();
    let on_changed = |event: &Event| (event.line..=event.end_line).any(|l| changed.contains(&l));
    let seeds: Vec<usize> = (0..f.events.len())
        .filter(|&i| on_changed(&f.events[i]))
        .collect();
    let mut mark = |event: &Event, score: f32, why: String| {
        if !on_changed(event) {
            lines.entry(event.line).or_default().add(score, why);
        }
    };

    // (Where what they read was written, and a step further.)
    let mut frontier: Vec<(usize, &Place, u32)> = seeds
        .iter()
        .flat_map(|&i| {
            let line = f.events[i].line;
            reads(f, &f.events[i])
                .into_iter()
                .map(move |place| (i, place, line))
        })
        .filter(|(_, place, _)| followed(place))
        .collect();
    for (step, score) in [(0, WRITES_READ), (1, WRITES_READ_FURTHER)] {
        let mut next = vec![];
        for (i, place, line) in frontier {
            for j in (0..i).rev() {
                let event = &f.events[j];
                if writes(f, event)
                    .iter()
                    .any(|written| written.overlaps(place))
                {
                    let name = f.place_name(place);
                    mark(
                        event,
                        score,
                        match step {
                            0 => format!("Writes {}, which line {} reads", name, line),
                            _ => format!(
                                "Writes {}, which line {} reads, on the way to the changes",
                                name, line
                            ),
                        },
                    );
                    next.extend(
                        reads(f, event)
                            .into_iter()
                            .filter(|place| followed(place))
                            .map(|place| (j, place, event.line)),
                    );
                    // (The nearest write is the one which reaches it.)
                    break;
                }
            }
        }
        frontier = next;
    }

    // (Where what they write is read, or tested.)
    for &i in &seeds {
        for place in writes(f, &f.events[i])
            .into_iter()
            .filter(|place| followed(place))
        {
            let name = f.place_name(place);
            let line = f.events[i].line;
            for event in &f.events[i + 1..] {
                if reads(f, event).iter().any(|read| read.overlaps(place)) {
                    match event.kind {
                        EventKind::Test { .. } => mark(
                            event,
                            TESTS_WRITTEN,
                            format!("Tests {}, which line {} writes", name, line),
                        ),
                        _ => mark(
                            event,
                            READS_WRITTEN,
                            format!("Reads {}, which line {} writes", name, line),
                        ),
                    }
                }
                if writes(f, event).iter().any(|written| written.within(place)) {
                    break;
                }
            }
        }
    }

    // (The stations of the cluster's values.)
    if !cluster_keys.is_empty() {
        for event in &f.events {
            let mut keys = BTreeSet::new();
            for place in writes(f, event).into_iter().chain(reads(f, event)) {
                place_keys(f, place, &mut keys);
            }
            if let Some(key) = keys.intersection(cluster_keys).next() {
                let name = match key {
                    Key::Field(sym) | Key::Global(sym) | Key::Local(_, sym) => {
                        f.names.get(sym).cloned().unwrap_or_else(|| sym.to_string())
                    }
                    _ => continue,
                };
                mark(
                    event,
                    CLUSTER_VALUE,
                    format!("Carries {}, as the changes' dataflow does", name),
                );
            }
        }
    }
    lines
}

/// For each file of a slicing's changed functions, the lines on their
/// changes' dataflow (see `func_impact`), as JSON: `{files: {path: {line:
/// {d, why}}}}`, in the slicing's revision's lines (a try push's rebased
/// head's).  (The `slice-format:lines` of `slice-rev:`.)
pub async fn slicing_lines(tracer: &mut Tracer<'_>, slicing: &Slicing) -> serde_json::Value {
    let mut files: BTreeMap<String, BTreeMap<u32, LineImpact>> = BTreeMap::new();
    for cluster in &slicing.clusters {
        let mut keys: BTreeSet<Key> = BTreeSet::new();
        for &id in cluster {
            keys.extend(
                slicing.units[id]
                    .keys
                    .iter()
                    .filter(|key| matches!(key, Key::Field(_) | Key::Global(_) | Key::Local(..)))
                    .cloned(),
            );
        }
        for &id in cluster {
            let unit = &slicing.units[id];
            if unit.removed || unit.lines.is_empty() {
                continue;
            }
            let Some(file) = tracer.file(&unit.path).await else {
                continue;
            };
            let func = unit
                .func_sym
                .and_then(|sym| file.func(&sym))
                .or_else(|| file.func_at_line(unit.lines[0]));
            let Some(func) = func else {
                continue;
            };
            let changed: BTreeSet<u32> = unit.lines.iter().copied().collect();
            let lines = files.entry(unit.path.clone()).or_default();
            for (line, impact) in func_impact(func, &changed, &keys) {
                let entry = lines.entry(line).or_default();
                for why in impact.why {
                    entry.add(impact.score, why);
                }
            }
        }
    }
    serde_json::json!({ "files": files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataflow::frontend::build_file_ir;
    use crate::dataflow::records::FileRecords;

    #[test]
    fn test_func_impact() {
        let source = "\
int f(int a) {
  int x = a + 1;
  int unrelated = 7;
  int y = x * 2;
  if (y > 3) {
    return y;
  }
  return unrelated;
}
";
        let ir = build_file_ir("test.cpp", source, &FileRecords::default()).unwrap();
        let f = &ir.funcs[0];
        // (Line 4 changed: it reads x, which line 2 writes from a, and
        // writes y, which lines 5 and 6 test and read.)
        let impact = func_impact(f, &BTreeSet::from([4]), &BTreeSet::new());
        let score = |line: u32| impact.get(&line).map(|i| i.score);
        assert_eq!(score(2), Some(WRITES_READ), "{:?}", impact);
        assert_eq!(score(5), Some(TESTS_WRITTEN), "{:?}", impact);
        assert_eq!(score(6), Some(READS_WRITTEN), "{:?}", impact);
        assert_eq!(score(3), None, "{:?}", impact);
        assert_eq!(score(4), None, "the changed line itself");
        assert!(
            impact[&2].why[0].contains("which line 4 reads"),
            "{:?}",
            impact
        );
    }
}
