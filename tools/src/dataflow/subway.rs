//! A slicing's clusters (see `slice`) as a subway map, the clusters' default
//! view (see static/js/subway.js): the clusters' functions are cards of their
//! stations, the statement blocks where the clusters' values are read or
//! written, and the values are lines through them.
//!
//! - A value and its copies are one line: its places in each function
//!   (locals, parameters, its most specific field, globals, as slicing's keys
//!   are; see `slice::place_keys`), joined by assignments that copy them, and
//!   by the calls between the cards (arguments to parameters, by the tracer's
//!   targets: also virtual calls, IPC messages' Send to Recv, and XPIDL's
//!   bindings) and their returns.
//! - A value derived from one (`a + 1`, `Foo(a)`) is a line of its own, a
//!   child of the line(s) it's derived from; a family is the lines related by
//!   derivation, from their roots, which the view colors alike (by the
//!   roots' types: see `semantic_type`).
//! - A line stops at a station (a dot) where its places are read or written,
//!   and passes through a card's stations between its stops.
//! - Connectors are the calls and returns between cards that lines take, and
//!   fields' writes to their reads in other cards.
//!
//! The values are the clusters' units' (see `Unit::keys`), and the values
//! their functions copy them to or from (all of their copies, in the
//! clusters' functions) or derive them from or to (one step).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use serde::Serialize;
use ustr::Ustr;

use super::frontend::FileIr;
use super::ir::*;
use super::slice::{Key, Slicing, cluster_commits, place_keys};
use super::summary::{Port, Summary};
use super::trace::{Tracer, ipc_message_sym};

/// Events at most this many lines apart are one station.
const STATION_GAP: u32 = 2;
/// The most lines a map has (those stopping most).
const MAX_LINES: usize = 32;
/// The most rows a station shows (its events' lines, then).
const MAX_STATION_ROWS: usize = 24;

#[derive(Debug, Serialize)]
pub struct SubwayMap {
    pub cards: Vec<Card>,
    pub lines: Vec<Line>,
    pub families: Vec<Family>,
    pub connectors: Vec<Connector>,
    /// The clusters (1 for the biggest, or the one asked for), with their
    /// commits.
    pub clusters: Vec<ClusterInfo>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ClusterInfo {
    pub n: usize,
    pub commits: String,
}

/// A function of a cluster (or a file's changed declarations).
#[derive(Debug, Serialize)]
pub struct Card {
    pub id: usize,
    pub func: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sym: Option<Ustr>,
    pub path: String,
    /// Whether the clusters change it (rather than only passing values).
    pub changed: bool,
    pub cluster: usize,
    pub stations: Vec<Station>,
}

#[derive(Debug, Serialize)]
pub struct Station {
    pub rows: Vec<Row>,
    /// The lines stopping here, and those of them written here.
    pub stops: Vec<usize>,
    pub writes: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct Row {
    pub line: u32,
    pub text: String,
    /// Whether the clusters' patch changes it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
    /// The lines of the file not shown before it (in its station).
    #[serde(skip_serializing_if = "is_zero")]
    pub skipped: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Serialize)]
pub struct Line {
    pub id: usize,
    pub label: String,
    /// Its places' names (ex: an argument's, and its parameter's).
    pub names: Vec<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    /// Its type without qualifiers and wrappers (see `semantic_type`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
    pub family: usize,
    /// The lines it's derived from.
    pub parents: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct Family {
    pub id: usize,
    pub root: usize,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
    pub cluster: usize,
}

/// Lines going from one card (at a station, if it's a call's or a write's)
/// to another (at a station, for a return to a call).
#[derive(Debug, Serialize)]
pub struct Connector {
    pub from: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_station: Option<usize>,
    pub to: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_station: Option<usize>,
    /// "call", "ipc", "return" or "field".
    pub kind: &'static str,
    pub lines: Vec<usize>,
}

/// A type without its qualifiers, pointers and references, and the smart
/// pointers and wrappers around it, and its namespaces (ex: "nsIPrincipal"
/// for `const nsCOMPtr<nsIPrincipal>&`, "PrincipalInfo" for
/// `const mozilla::ipc::PrincipalInfo&`), which the view colors families by.
pub fn semantic_type(ty: &str) -> Option<String> {
    const WRAPPERS: [&str; 12] = [
        "RefPtr",
        "nsCOMPtr",
        "already_AddRefed",
        "NotNull",
        "UniquePtr",
        "Maybe",
        "MovingNotNull",
        "StaticRefPtr",
        "nsMainThreadPtrHandle",
        "SafeRefPtr",
        "CheckedUnsafePtr",
        "Unused",
    ];
    let mut ty = ty.trim().to_string();
    loop {
        let before = ty.clone();
        for qualifier in ["const ", "volatile ", "struct ", "class ", "enum "] {
            if let Some(rest) = ty.strip_prefix(qualifier) {
                ty = rest.to_string();
            }
        }
        ty = ty
            .trim_end_matches(['&', '*', ' '])
            .trim_end_matches(" const")
            .trim_end_matches(['&', '*', ' '])
            .to_string();
        if let Some((outer, inner)) = ty.split_once('<')
            && inner.ends_with('>')
        {
            let outer = outer.rsplit("::").next().unwrap_or(outer);
            if WRAPPERS.contains(&outer) {
                ty = inner[..inner.len() - 1].to_string();
            }
        }
        if ty == before {
            break;
        }
    }
    let name = match ty.split_once('<') {
        Some((outer, _)) => outer,
        None => ty.as_str(),
    };
    let name = name.rsplit("::").next().unwrap_or(name).trim();
    (!name.is_empty() && name != "auto").then(|| name.to_string())
}

/// What an event reads and writes (by its places' keys), and the values it
/// moves between them: (from, to, derived).
struct EventFlows {
    reads: BTreeSet<Key>,
    writes: BTreeSet<Key>,
    moves: Vec<(Key, Key, bool)>,
}

fn keys_of(f: &FuncIr, place: &Place) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    place_keys(f, place, &mut keys);
    keys
}

/// The summaries of a function's calls that have them (see
/// `Tracer::call_summary`: accessors', and hints'), by call.
type CallSummaries = HashMap<usize, Summary>;

/// The keys of the places a port of a call's callee's summary is (an
/// argument's values, or its object's), and whether derived.
fn port_keys(f: &FuncIr, call: &Call, port: &Port, sums: &CallSummaries) -> Vec<(Key, bool)> {
    match port {
        Port::Param { i, .. } => call
            .args
            .get(*i)
            .map(|arg| val_keys(f, &arg.vals, sums))
            .unwrap_or_default(),
        Port::This { .. } => call
            .receiver
            .iter()
            .flat_map(|r| val_keys(f, &r.vals, sums))
            .collect(),
        _ => vec![],
    }
}

/// The keys of the places values are read from (and whether derived): a
/// call's result's are those its summary says it returns (or for a
/// constructor, puts in its object), else its arguments' and its object's,
/// derived.
fn val_keys(f: &FuncIr, vals: &[Val], sums: &CallSummaries) -> Vec<(Key, bool)> {
    let mut out = vec![];
    for v in vals {
        match &v.atom {
            Atom::Read(p) | Atom::Addr(p) => {
                for key in keys_of(f, p) {
                    out.push((key, v.derived));
                }
            }
            Atom::Call(c) => {
                let call = &f.calls[*c];
                let ctor = call.kind == CallKind::Construct;
                match sums.get(c) {
                    Some(summary) => {
                        for flow in &summary.flows {
                            let result = flow.to == Port::Return
                                || (ctor && matches!(flow.to, Port::This { .. }));
                            if result {
                                for (key, derived) in port_keys(f, call, &flow.from, sums) {
                                    out.push((key, derived || flow.derived || v.derived));
                                }
                            }
                        }
                    }
                    None => {
                        for arg in call.args.iter().chain(call.receiver.iter()) {
                            for (key, _) in val_keys(f, &arg.vals, sums) {
                                out.push((key, true));
                            }
                        }
                    }
                }
            }
            Atom::Const(_) => {}
        }
    }
    out
}

fn event_flows(f: &FuncIr, e: &Event, sums: &CallSummaries) -> EventFlows {
    let mut flows = EventFlows {
        reads: BTreeSet::new(),
        writes: BTreeSet::new(),
        moves: vec![],
    };
    match &e.kind {
        EventKind::Assign { place, vals, .. } => {
            flows.writes = keys_of(f, place);
            for (key, derived) in val_keys(f, vals, sums) {
                for to in &flows.writes {
                    if *to != key {
                        flows.moves.push((key.clone(), to.clone(), derived));
                    }
                }
                flows.reads.insert(key);
            }
        }
        EventKind::Call(c) => {
            let call = &f.calls[*c];
            for arg in call.args.iter().chain(call.receiver.iter()) {
                for (key, _) in val_keys(f, &arg.vals, sums) {
                    flows.reads.insert(key);
                }
            }
            // (What it writes, by its summary: its object (ex: a string's
            // `Assign`), or through its arguments.)
            if let Some(summary) = sums.get(c) {
                for flow in &summary.flows {
                    let (written, fields) = match &flow.to {
                        Port::This { fields } if call.kind != CallKind::Construct => {
                            (call.receiver.as_ref().and_then(|r| r.place.clone()), fields)
                        }
                        Port::Param { i, fields } => {
                            (call.args.get(*i).and_then(|a| a.place.clone()), fields)
                        }
                        _ => continue,
                    };
                    let Some(mut written) = written else {
                        continue;
                    };
                    // (Its fields, ex: a setter's `this->mFoo`.)
                    for field in fields {
                        written = written.with(Step::Field(*field));
                    }
                    let to_keys = keys_of(f, &written);
                    for (key, derived) in port_keys(f, call, &flow.from, sums) {
                        for to in &to_keys {
                            if *to != key {
                                flows.moves.push((
                                    key.clone(),
                                    to.clone(),
                                    derived || flow.derived,
                                ));
                            }
                        }
                    }
                    flows.writes.extend(to_keys);
                }
            }
        }
        EventKind::Return { vals } | EventKind::Test { vals } => {
            for (key, _) in val_keys(f, vals, sums) {
                flows.reads.insert(key);
            }
        }
    }
    flows
}

fn is_value_key(key: &Key) -> bool {
    matches!(key, Key::Field(_) | Key::Global(_) | Key::Local(..))
}

/// A function of the clusters, with its IR.
struct Func {
    card: usize,
    file: Arc<FileIr>,
    fi: usize,
    changed: BTreeSet<u32>,
    flows: Vec<EventFlows>,
    sums: CallSummaries,
}

/// Union-find over keys.
struct Union {
    parent: HashMap<Key, Key>,
}

impl Union {
    fn find(&mut self, key: &Key) -> Key {
        let mut key = key.clone();
        let mut path = vec![];
        while let Some(parent) = self.parent.get(&key) {
            if *parent == key {
                break;
            }
            path.push(key.clone());
            key = parent.clone();
        }
        for k in path {
            self.parent.insert(k, key.clone());
        }
        key
    }

    fn union(&mut self, a: &Key, b: &Key) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // (The smaller key is the root, so roots don't depend on order.)
            if ra < rb {
                self.parent.insert(rb, ra);
            } else {
                self.parent.insert(ra, rb);
            }
        }
    }
}

/// The subway map of a slicing's clusters (all of them, or the one
/// `only_cluster` left), in the tracer's tree (or try push).
pub async fn subway_map(
    tracer: &mut Tracer<'_>,
    slicing: &Slicing,
    first_cluster: usize,
) -> SubwayMap {
    // (Calls are stepped over by their summaries (accessors', hints'),
    // which slicing doesn't use.)
    let summaries = tracer.opts.summaries;
    tracer.opts.summaries = true;
    let map = subway_map_inner(tracer, slicing, first_cluster).await;
    tracer.opts.summaries = summaries;
    map
}

async fn subway_map_inner(
    tracer: &mut Tracer<'_>,
    slicing: &Slicing,
    first_cluster: usize,
) -> SubwayMap {
    let mut map = SubwayMap {
        cards: vec![],
        lines: vec![],
        families: vec![],
        connectors: vec![],
        clusters: vec![],
        notes: slicing.notes.clone(),
    };

    // ## The cards: the clusters' functions, and files' declarations.
    let mut funcs: Vec<Func> = vec![];
    let mut func_index: HashMap<Ustr, usize> = HashMap::new();
    let mut decl_cards: BTreeMap<(usize, String), Vec<usize>> = BTreeMap::new();
    let mut unit_cluster: HashMap<usize, usize> = HashMap::new();
    let mut removed = 0;
    let mut seeds: BTreeSet<Key> = BTreeSet::new();
    for (ci, cluster) in slicing.clusters.iter().enumerate() {
        let n = first_cluster + ci;
        map.clusters.push(ClusterInfo {
            n,
            commits: cluster_commits(slicing, cluster),
        });
        for &ui in cluster {
            unit_cluster.insert(ui, n);
            let unit = &slicing.units[ui];
            if unit.removed {
                removed += 1;
                continue;
            }
            seeds.extend(unit.keys.iter().filter(|k| is_value_key(k)).cloned());
            let Some(sym) = unit.func_sym else {
                decl_cards
                    .entry((n, unit.path.clone()))
                    .or_default()
                    .push(ui);
                continue;
            };
            if let Some(&i) = func_index.get(&sym) {
                funcs[i].changed.extend(unit.lines.iter().copied());
                continue;
            }
            let Some(file) = tracer.file(&unit.path).await else {
                continue;
            };
            let Some(&fi) = file.by_sym.get(&sym) else {
                continue;
            };
            let f = &file.funcs[fi];
            let card = map.cards.len();
            map.cards.push(Card {
                id: card,
                func: f.pretty.clone(),
                sym: Some(sym),
                path: unit.path.clone(),
                changed: true,
                cluster: n,
                stations: vec![],
            });
            let mut sums = CallSummaries::new();
            for (c, call) in f.calls.iter().enumerate() {
                if let Some(summary) = tracer.call_summary(call).await {
                    sums.insert(c, summary);
                }
            }
            let flows = f.events.iter().map(|e| event_flows(f, e, &sums)).collect();
            func_index.insert(sym, funcs.len());
            funcs.push(Func {
                card,
                file: file.clone(),
                fi,
                changed: unit.lines.iter().copied().collect(),
                flows,
                sums,
            });
        }
    }
    if removed > 0 {
        map.notes.push(format!(
            "{} of the clusters' changes are removals, which the map leaves out (see the dataflow graph).",
            removed
        ));
    }

    // ## Moves between the cards: calls' arguments to their parameters, and
    // their returns to their results.
    // (from card, call's event, to card, (from key, to key) pairs, kind)
    struct CallMove {
        from: usize,
        event: usize,
        to: usize,
        /// (argument's key, parameter's key, derived)
        args: Vec<(Key, Key, bool)>,
        /// (returned key, the result's key in the caller, derived)
        returns: Vec<(Key, Key, bool)>,
        kind: &'static str,
    }
    let mut call_moves: Vec<CallMove> = vec![];
    for i in 0..funcs.len() {
        let (file, fi) = (funcs[i].file.clone(), funcs[i].fi);
        let f = &file.funcs[fi];
        let sums = funcs[i].sums.clone();
        for (ei, e) in f.events.iter().enumerate() {
            let EventKind::Call(c) = e.kind else {
                continue;
            };
            let call = &f.calls[c];
            // (Only calls of the clusters' functions.)
            if !call.callee.iter().any(|sym| func_index.contains_key(sym))
                && ipc_message_sym(&call.callee_pretty).is_none()
            {
                continue;
            }
            for (tfile, tfi, params) in tracer.call_targets(&file, fi, c).await {
                let tf = &tfile.funcs[tfi];
                let Some(&ti) = func_index.get(&tf.sym) else {
                    continue;
                };
                let tsums = &funcs[ti].sums;
                let mut args = vec![];
                for (ai, arg) in call.args.iter().enumerate() {
                    let Some(Some(pi)) = params.get(ai) else {
                        continue;
                    };
                    let Some(param) = tf.params.get(*pi) else {
                        continue;
                    };
                    for to in keys_of(tf, &param.place) {
                        for (from, derived) in val_keys(f, &arg.vals, &sums) {
                            args.push((from, to.clone(), derived));
                        }
                    }
                }
                // (Its returns' values, to where the caller puts its result.)
                let mut results: BTreeSet<Key> = BTreeSet::new();
                for e2 in &f.events {
                    if let EventKind::Assign { place, vals, .. } = &e2.kind
                        && vals.iter().any(|v| v.atom == Atom::Call(c))
                    {
                        results.extend(keys_of(f, place));
                    }
                }
                let mut returns = vec![];
                for e2 in tf.events.iter().filter(|e2| !e2.in_lambda) {
                    if let EventKind::Return { vals } = &e2.kind {
                        for (from, derived) in val_keys(tf, vals, tsums) {
                            for to in &results {
                                returns.push((from.clone(), to.clone(), derived));
                            }
                        }
                    }
                }
                let kind = match ipc_message_sym(&call.callee_pretty) {
                    Some(_) => "ipc",
                    None => "call",
                };
                call_moves.push(CallMove {
                    from: i,
                    event: ei,
                    to: ti,
                    args,
                    returns,
                    kind,
                });
            }
        }
    }

    // ## The values: the units', and all of their copies in the clusters'
    // functions (and between them), and what's derived from them or what
    // they're derived from, a step away.
    let mut local_copies: Vec<(Key, Key)> = vec![];
    let mut derivations: Vec<(Key, Key)> = vec![];
    for func in &funcs {
        for flows in &func.flows {
            for (from, to, derived) in &flows.moves {
                match derived {
                    true => derivations.push((from.clone(), to.clone())),
                    false => local_copies.push((from.clone(), to.clone())),
                }
            }
        }
    }
    // (Copies between the cards: arguments to parameters, returns to
    // results; the derived ones are derivations.)
    let mut call_copies: Vec<(Key, Key)> = vec![];
    for call in &call_moves {
        for (from, to, derived) in call.args.iter().chain(call.returns.iter()) {
            match derived {
                true => derivations.push((from.clone(), to.clone())),
                false => call_copies.push((from.clone(), to.clone())),
            }
        }
    }
    let present: BTreeSet<Key> = funcs
        .iter()
        .flat_map(|func| func.flows.iter())
        .flat_map(|flows| flows.reads.iter().chain(flows.writes.iter()))
        .cloned()
        .collect();
    let mut values: BTreeSet<Key> = seeds.intersection(&present).cloned().collect();
    loop {
        let before = values.len();
        for (a, b) in local_copies.iter().chain(call_copies.iter()) {
            if values.contains(a) || values.contains(b) {
                values.insert(a.clone());
                values.insert(b.clone());
            }
        }
        if values.len() == before {
            break;
        }
    }
    let copied = values.clone();
    for (a, b) in &derivations {
        if copied.contains(a) || copied.contains(b) {
            values.insert(a.clone());
            values.insert(b.clone());
        }
    }

    // ## The lines: the values joined by copies in their functions, and a
    // parameter (or a call's result) with what it's given if it's always
    // the same line, else it's a line of its own, merging them (as if
    // derived from them).
    let mut union = Union {
        parent: HashMap::new(),
    };
    for value in &values {
        union.find(value);
    }
    for (a, b) in &local_copies {
        if values.contains(a) && values.contains(b) {
            union.union(a, b);
        }
    }
    let mut given: BTreeMap<Key, BTreeSet<Key>> = BTreeMap::new();
    for (from, to) in &call_copies {
        if values.contains(from) && values.contains(to) {
            given.entry(to.clone()).or_default().insert(from.clone());
        }
    }
    // (Until it settles: a parameter's arguments can be parameters.)
    for _ in 0..4 {
        let mut changed = false;
        for (to, froms) in &given {
            let roots: BTreeSet<Key> = froms.iter().map(|from| union.find(from)).collect();
            if roots.len() == 1 && union.find(to) != *roots.iter().next().unwrap() {
                union.union(to, froms.iter().next().unwrap());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (to, froms) in &given {
        let to_root = union.find(to);
        for from in froms {
            if union.find(from) != to_root {
                derivations.push((from.clone(), to.clone()));
            }
        }
    }
    // (Each value's line, by its root, and the order lines first stop in.)
    let mut line_of_root: BTreeMap<Key, usize> = BTreeMap::new();
    let mut line_keys: Vec<Vec<Key>> = vec![];
    let mut stops_count: Vec<usize> = vec![];
    let mut line_of = |key: &Key, union: &mut Union, line_keys: &mut Vec<Vec<Key>>| -> usize {
        let root = union.find(key);
        *line_of_root.entry(root).or_insert_with(|| {
            line_keys.push(vec![]);
            line_keys.len() - 1
        })
    };
    let mut key_line: BTreeMap<Key, usize> = BTreeMap::new();
    for func in &funcs {
        for flows in &func.flows {
            for key in flows.reads.iter().chain(flows.writes.iter()) {
                if values.contains(key) && !key_line.contains_key(key) {
                    let line = line_of(key, &mut union, &mut line_keys);
                    line_keys[line].push(key.clone());
                    key_line.insert(key.clone(), line);
                }
            }
        }
    }
    stops_count.resize(line_keys.len(), 0);
    for func in &funcs {
        for flows in &func.flows {
            let lines: BTreeSet<usize> = flows
                .reads
                .iter()
                .chain(flows.writes.iter())
                .filter_map(|key| key_line.get(key).copied())
                .collect();
            for line in lines {
                stops_count[line] += 1;
            }
        }
    }
    // (The most stopping lines, if there are too many; in their order.)
    let mut kept: Vec<usize> = (0..line_keys.len()).collect();
    if kept.len() > MAX_LINES {
        kept.sort_by_key(|&l| std::cmp::Reverse(stops_count[l]));
        kept.truncate(MAX_LINES);
        kept.sort();
        map.notes.push(format!(
            "The map shows the {} values stopping most, of {}.",
            MAX_LINES,
            line_keys.len()
        ));
    }
    let renumber: HashMap<usize, usize> = kept
        .iter()
        .enumerate()
        .map(|(new, &old)| (old, new))
        .collect();
    key_line = key_line
        .into_iter()
        .filter_map(|(key, line)| Some((key, *renumber.get(&line)?)))
        .collect();

    // (Names and types of keys, from the functions they're in.)
    let mut names: BTreeMap<Key, String> = BTreeMap::new();
    let mut types: BTreeMap<Key, String> = BTreeMap::new();
    for func in &funcs {
        let f = &func.file.funcs[func.fi];
        for param in &f.params {
            for key in keys_of(f, &param.place) {
                if let Some(ty) = &param.type_pretty {
                    types.entry(key.clone()).or_insert_with(|| ty.clone());
                }
            }
        }
        for key in key_line.keys() {
            let name = match key {
                Key::Local(func_sym, sym) if *func_sym == f.sym => f.names.get(sym).cloned(),
                Key::Local(..) => None,
                Key::Field(sym) | Key::Global(sym) => Some(
                    f.names
                        .get(sym)
                        .cloned()
                        .unwrap_or_else(|| sym_short_name(sym)),
                ),
                _ => None,
            };
            if let Some(name) = name {
                names.entry(key.clone()).or_insert(name);
            }
            if let Key::Local(func_sym, sym) = key
                && *func_sym == f.sym
                && let Some(ty) = f.types.get(sym)
            {
                types.entry(key.clone()).or_insert_with(|| ty.clone());
            }
        }
    }
    // (A line's name is its source's: a place no copies flow into, ex: a
    // parameter its function copies to locals and passes on.)
    let copied_into: BTreeSet<&Key> = local_copies
        .iter()
        .chain(call_copies.iter())
        .map(|(_, to)| to)
        .collect();
    for (new, &old) in kept.iter().enumerate() {
        let mut keys = line_keys[old].clone();
        keys.sort_by_key(|key| copied_into.contains(key));
        let mut line_names: Vec<String> = vec![];
        for key in &keys {
            if let Some(name) = names.get(key)
                && !line_names.contains(name)
            {
                line_names.push(name.clone());
            }
        }
        let ty = keys.iter().find_map(|key| types.get(key).cloned());
        map.lines.push(Line {
            id: new,
            label: line_names
                .first()
                .cloned()
                .unwrap_or_else(|| format!("value {}", new + 1)),
            names: line_names,
            semantic: ty.as_deref().and_then(semantic_type),
            ty,
            family: 0,
            parents: vec![],
        });
    }
    // (Derivations between lines.)
    for (a, b) in &derivations {
        if let (Some(&la), Some(&lb)) = (key_line.get(a), key_line.get(b))
            && la != lb
            && !map.lines[lb].parents.contains(&la)
        {
            map.lines[lb].parents.push(la);
        }
    }
    // ## The families: lines related by derivation, from their roots.
    let mut family_union: Vec<usize> = (0..map.lines.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for line in 0..map.lines.len() {
        for parent in map.lines[line].parents.clone() {
            let (a, b) = (
                find(&mut family_union, line),
                find(&mut family_union, parent),
            );
            if a != b {
                family_union[a.max(b)] = a.min(b);
            }
        }
    }
    let mut family_of_root: BTreeMap<usize, usize> = BTreeMap::new();
    for line in 0..map.lines.len() {
        let root = find(&mut family_union, line);
        let family = *family_of_root.entry(root).or_insert_with(|| {
            map.families.push(Family {
                id: map.families.len(),
                root: line,
                label: String::new(),
                semantic: None,
                cluster: 0,
            });
            map.families.len() - 1
        });
        map.lines[line].family = family;
    }
    // (A family's root: its line without parents stopping most.)
    for family in &mut map.families {
        let root = map
            .lines
            .iter()
            .filter(|line| line.family == family.id && line.parents.is_empty())
            .max_by_key(|line| (stops_count[kept[line.id]], std::cmp::Reverse(line.id)))
            .map(|line| line.id)
            .unwrap_or(family.root);
        family.root = root;
        family.label = map.lines[root].label.clone();
        family.semantic = map.lines[root].semantic.clone();
    }

    // ## The stations: the cards' events reading or writing the lines'
    // values (or on changed lines), by statement blocks.
    // (Each card's event's station, for connectors.)
    let mut event_station: HashMap<(usize, usize), usize> = HashMap::new();
    for func in &funcs {
        let f = &func.file.funcs[func.fi];
        let source = tracer.source(&f.path).await;
        let source_lines: Vec<&str> = source
            .as_deref()
            .map(|s| s.lines().collect())
            .unwrap_or_default();
        let relevant: Vec<usize> = (0..f.events.len())
            .filter(|&ei| {
                let flows = &func.flows[ei];
                flows
                    .reads
                    .iter()
                    .chain(flows.writes.iter())
                    .any(|key| key_line.contains_key(key))
                    || func.changed.contains(&f.events[ei].line)
            })
            .collect();
        let mut order = relevant.clone();
        order.sort_by_key(|&ei| (f.events[ei].line, f.events[ei].col));
        let mut groups: Vec<(u32, u32, Vec<usize>)> = vec![];
        for ei in order {
            let e = &f.events[ei];
            match groups.last_mut() {
                Some((_, end, events)) if e.line <= *end + STATION_GAP => {
                    *end = (*end).max(e.end_line.max(e.line));
                    events.push(ei);
                }
                _ => groups.push((e.line, e.end_line.max(e.line), vec![ei])),
            }
        }
        let card = &mut map.cards[func.card];
        card.changed = !func.changed.is_empty();
        for (first, last, events) in groups {
            let event_lines: BTreeSet<u32> = events.iter().map(|&ei| f.events[ei].line).collect();
            let all: Vec<u32> = (first..=last).collect();
            let shown: Vec<u32> = match all.len() > MAX_STATION_ROWS {
                true => event_lines.iter().copied().collect(),
                false => all,
            };
            let mut rows = vec![];
            let mut prev: Option<u32> = None;
            for line in shown {
                rows.push(Row {
                    line,
                    text: source_lines
                        .get(line as usize - 1)
                        .map(|s| s.trim_end().to_string())
                        .unwrap_or_default(),
                    changed: func.changed.contains(&line),
                    skipped: prev.map_or(0, |p| line - p - 1),
                });
                prev = Some(line);
            }
            let mut stops: BTreeSet<usize> = BTreeSet::new();
            let mut writes: BTreeSet<usize> = BTreeSet::new();
            for &ei in &events {
                let flows = &func.flows[ei];
                stops.extend(
                    flows
                        .reads
                        .iter()
                        .filter_map(|key| key_line.get(key).copied()),
                );
                writes.extend(
                    flows
                        .writes
                        .iter()
                        .filter_map(|key| key_line.get(key).copied()),
                );
                event_station.insert((func.card, ei), card.stations.len());
            }
            stops.extend(writes.iter().copied());
            card.stations.push(Station {
                rows,
                stops: stops.into_iter().collect(),
                writes: writes.into_iter().collect(),
            });
        }
    }
    // (Declarations: a file's changed lines outside functions, with their
    // fields.)
    for ((n, path), units) in decl_cards {
        let source = tracer.source(&path).await;
        let source_lines: Vec<&str> = source
            .as_deref()
            .map(|s| s.lines().collect())
            .unwrap_or_default();
        let mut stations = vec![];
        for ui in units {
            let unit = &slicing.units[ui];
            let stops: Vec<usize> = unit
                .keys
                .iter()
                .filter_map(|key| key_line.get(key).copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let rows = unit
                .lines
                .iter()
                .take(MAX_STATION_ROWS)
                .map(|&line| Row {
                    line,
                    text: source_lines
                        .get(line as usize - 1)
                        .map(|s| s.trim_end().to_string())
                        .unwrap_or_default(),
                    changed: true,
                    skipped: 0,
                })
                .collect();
            stations.push(Station {
                rows,
                writes: stops.clone(),
                stops,
            });
        }
        let id = map.cards.len();
        map.cards.push(Card {
            id,
            func: format!(
                "{} (declarations)",
                path.rsplit('/').next().unwrap_or(&path)
            ),
            sym: None,
            path,
            changed: true,
            cluster: n,
            stations,
        });
    }
    // (Families' clusters: their roots' first cards'.)
    let mut line_cluster: HashMap<usize, usize> = HashMap::new();
    for card in &map.cards {
        for station in &card.stations {
            for &line in &station.stops {
                line_cluster.entry(line).or_insert(card.cluster);
            }
        }
    }
    for family in &mut map.families {
        family.cluster = line_cluster
            .get(&family.root)
            .copied()
            .unwrap_or(first_cluster);
    }

    // ## The connectors.
    let mut connectors: BTreeMap<
        (usize, Option<usize>, usize, Option<usize>, &'static str),
        BTreeSet<usize>,
    > = BTreeMap::new();
    for call in &call_moves {
        let (from_card, to_card) = (funcs[call.from].card, funcs[call.to].card);
        let station = event_station.get(&(from_card, call.event)).copied();
        let args: BTreeSet<usize> = call
            .args
            .iter()
            .filter_map(|(a, b, _)| Some(key_line.get(a).or_else(|| key_line.get(b))?))
            .copied()
            .collect();
        if !args.is_empty() {
            connectors
                .entry((from_card, station, to_card, None, call.kind))
                .or_default()
                .extend(args);
        }
        let returns: BTreeSet<usize> = call
            .returns
            .iter()
            .filter_map(|(a, b, _)| Some(key_line.get(a).or_else(|| key_line.get(b))?))
            .copied()
            .collect();
        if !returns.is_empty() {
            connectors
                .entry((to_card, None, from_card, station, "return"))
                .or_default()
                .extend(returns);
        }
    }
    // (Fields: from the cards writing them to those reading them.)
    for line in 0..map.lines.len() {
        let is_field = kept.get(line).is_some_and(|&old| {
            line_keys[old]
                .iter()
                .any(|key| matches!(key, Key::Field(_)))
        });
        if !is_field {
            continue;
        }
        let mut writers = vec![];
        let mut readers = vec![];
        for card in &map.cards {
            let write = card.stations.iter().position(|s| s.writes.contains(&line));
            let read = card
                .stations
                .iter()
                .position(|s| s.stops.contains(&line) && !s.writes.contains(&line));
            if let Some(station) = write {
                writers.push((card.id, station));
            } else if let Some(station) = read {
                readers.push((card.id, station));
            }
        }
        for &(writer, wstation) in writers.iter().take(4) {
            for &(reader, rstation) in readers.iter().take(4) {
                connectors
                    .entry((writer, Some(wstation), reader, Some(rstation), "field"))
                    .or_default()
                    .insert(line);
            }
        }
    }
    map.connectors = connectors
        .into_iter()
        .filter(|((from, _, to, _, _), _)| from != to)
        .map(
            |((from, from_station, to, to_station, kind), lines)| Connector {
                from,
                from_station,
                to,
                to_station,
                kind,
                lines: lines.into_iter().collect(),
            },
        )
        .collect();
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_type() {
        assert_eq!(
            semantic_type("nsIPrincipal *").as_deref(),
            Some("nsIPrincipal")
        );
        assert_eq!(
            semantic_type("const nsCOMPtr<nsIPrincipal> &").as_deref(),
            Some("nsIPrincipal")
        );
        assert_eq!(
            semantic_type("const mozilla::ipc::PrincipalInfo &").as_deref(),
            Some("PrincipalInfo")
        );
        assert_eq!(
            semantic_type("RefPtr<mozilla::dom::Promise>").as_deref(),
            Some("Promise")
        );
        assert_eq!(semantic_type("nsTArray<int>").as_deref(), Some("nsTArray"));
        assert_eq!(semantic_type("auto").as_deref(), None);
    }
}
