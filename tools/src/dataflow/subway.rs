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
use ustr::{Ustr, UstrMap, ustr};

use super::frontend::FileIr;
use super::ir::*;
use super::slice::{Key, Slicing, cluster_commits, place_keys};
use super::summary::{Port, Summary};
use super::taint::{Taint, TaintSpec, TaintedField, Validated};
use super::trace::{Tracer, ipc_message_sym};
use crate::abstract_server::sym_info::add_html_sym_info;
use crate::file_format::jumpref::JumprefData;

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
    /// The tainted values' sources and their validations, if any (see
    /// `taint`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taint: Option<TaintInfo>,
    /// The SYM_INFO (see `format::format_code`) of the symbols in the rows'
    /// code (see `Row::html`), for their context menus.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sym_info: BTreeMap<Ustr, Option<JumprefData>>,
}

#[derive(Debug, Serialize)]
pub struct TaintInfo {
    pub sources: Vec<TaintSource>,
    pub validations: Vec<Validation>,
}

/// A tainted parameter of a function receiving a message from content
/// processes.
#[derive(Debug, Serialize)]
pub struct TaintSource {
    pub card: usize,
    pub line: usize,
    /// The field of its parameter it is, if it's one (of a struct whose
    /// fields are tainted, rather than its type).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub why: String,
    /// Whether its card's validations bless it before all of its uses there
    /// ("validated"), some ("partly"), or none ("unvalidated"), or it has no
    /// uses there ("unused"): its stops (the stations of the lines it
    /// taints, but its validations'), and those where it's still tainted.
    pub status: &'static str,
    pub stops: usize,
    pub tainted_stops: usize,
}

/// A validator's call, validating its lines (and their genealogies) in what
/// it dominates (`guards`), or not (its result unchecked, or checked in a way
/// the analysis doesn't see).
#[derive(Debug, Serialize)]
pub struct Validation {
    pub card: usize,
    pub station: usize,
    pub lines: Vec<usize>,
    pub by: String,
    pub guards: bool,
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
    /// The lines tainted at its end (going on to what it returns to, or
    /// what reads the fields it writes).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tainted_out: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct Station {
    pub rows: Vec<Row>,
    /// The lines stopping here, and those of them written here.
    pub stops: Vec<usize>,
    pub writes: Vec<usize>,
    /// The lines tainted where it starts (see `taint`), and those its
    /// validations bless (for what they dominate).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tainted: Vec<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub validates: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct Row {
    pub line: u32,
    pub text: String,
    /// Its code as the source listing has it, highlighted, with its symbols
    /// (see `add_rows_html`), if the server has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
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
    /// Its places' symbols (its fields', globals' and locals'), whose tokens
    /// the map's code highlights with its stations (see subway.js).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub syms: Vec<Ustr>,
    /// Whether it's tainted anywhere (see `taint`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub tainted: bool,
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
    /// The lines tainted along it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tainted: Vec<usize>,
}

/// A type without its qualifiers, pointers and references, and the smart
/// pointers and wrappers around it, and its namespaces (ex: "nsIPrincipal"
/// for `const nsCOMPtr<nsIPrincipal>&`, "PrincipalInfo" for
/// `const mozilla::ipc::PrincipalInfo&`), which the view colors families by.
pub fn semantic_type(ty: &str) -> Option<String> {
    let ty = qualified_type(ty)?;
    let name = ty.rsplit("::").next().unwrap_or(&ty).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// A type without its qualifiers, pointers and references, and the smart
/// pointers and wrappers around it, and its template arguments, but with its
/// namespaces (ex: "mozilla::ipc::PrincipalInfo" for
/// `const mozilla::ipc::PrincipalInfo&`).
pub fn qualified_type(ty: &str) -> Option<String> {
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
    let name = name.trim().trim_start_matches("::");
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
    // (A port's fields, of an argument that's a place, are its fields' keys:
    // a getter's `aArgs.principal()` reads the field, rather than all of
    // `aArgs`.)
    let keys = |arg: &Arg, fields: &[Ustr]| -> Vec<(Key, bool)> {
        match &arg.place {
            Some(place) if !fields.is_empty() => {
                let mut place = place.clone();
                for field in fields {
                    place = place.with(Step::Field(*field));
                }
                keys_of(f, &place)
                    .into_iter()
                    .map(|key| (key, false))
                    .collect()
            }
            _ => val_keys(f, &arg.vals, sums),
        }
    };
    match port {
        Port::Param { i, fields } => call
            .args
            .get(*i)
            .map(|arg| keys(arg, fields))
            .unwrap_or_default(),
        Port::This { fields } => call.receiver.iter().flat_map(|r| keys(r, fields)).collect(),
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

/// A call from a function of the clusters to another (by their indexes in
/// `funcs`), at its event, and the values it moves.
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
/// `only_cluster` left), in the tracer's tree (or try push), with the taint
/// of the ontology's labels and `taint`'s (see `taint`).
pub async fn subway_map(
    tracer: &mut Tracer<'_>,
    slicing: &Slicing,
    first_cluster: usize,
    taint: &TaintSpec,
) -> SubwayMap {
    // (Calls are stepped over by their summaries (accessors', hints'),
    // which slicing doesn't use.)
    let summaries = tracer.opts.summaries;
    tracer.opts.summaries = true;
    let mut taint = Taint::new(taint.clone());
    let map = subway_map_inner(tracer, slicing, first_cluster, &mut taint).await;
    tracer.opts.summaries = summaries;
    map
}

async fn subway_map_inner(
    tracer: &mut Tracer<'_>,
    slicing: &Slicing,
    first_cluster: usize,
    taint: &mut Taint,
) -> SubwayMap {
    let mut map = SubwayMap {
        cards: vec![],
        lines: vec![],
        families: vec![],
        connectors: vec![],
        clusters: vec![],
        notes: slicing.notes.clone(),
        taint: None,
        sym_info: BTreeMap::new(),
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
                tainted_out: vec![],
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
    // (Not the locals only macros' expansions have, ex: MOZ_TRY's
    // temporaries, which aren't the code's values.)
    let mut in_code: BTreeSet<Key> = BTreeSet::new();
    let mut in_macros: BTreeSet<Key> = BTreeSet::new();
    for func in &funcs {
        let f = &func.file.funcs[func.fi];
        for (ei, flows) in func.flows.iter().enumerate() {
            let keys = flows
                .reads
                .iter()
                .chain(flows.writes.iter())
                .filter(|key| matches!(key, Key::Local(..)))
                .cloned();
            match f.events[ei].macro_name {
                Some(_) => in_macros.extend(keys),
                None => in_code.extend(keys),
            }
        }
    }
    let macro_only: BTreeSet<Key> = in_macros.difference(&in_code).cloned().collect();
    let present: BTreeSet<Key> = present.difference(&macro_only).cloned().collect();
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
    values.retain(|value| !macro_only.contains(value));

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
        let syms: BTreeSet<Ustr> = keys
            .iter()
            .filter_map(|key| match key {
                Key::Field(sym) | Key::Global(sym) | Key::Local(_, sym) => Some(*sym),
                _ => None,
            })
            .filter(|sym| !sym.starts_with("name:"))
            .collect();
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
            syms: syms.into_iter().collect(),
            tainted: false,
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

                    html: None,
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
                tainted: vec![],
                validates: vec![],
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

                    html: None,
                })
                .collect();
            stations.push(Station {
                rows,
                writes: stops.clone(),
                stops,
                tainted: vec![],
                validates: vec![],
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
            tainted_out: vec![],
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
                tainted: vec![],
            },
        )
        .collect();
    apply_taint(
        tracer,
        taint,
        &mut map,
        &funcs,
        &key_line,
        &call_moves,
        &event_station,
    )
    .await;
    add_rows_html(tracer, &mut map).await;
    map
}

/// The cards' rows' code as the server's source listings (or for a try
/// push's files, its own; see `TryOverlay::fetch_html_lines`) highlight it,
/// with its symbols (`Row::html`), and their SYM_INFO (`SubwayMap::sym_info`),
/// so that the map's code has the listings' syntax highlighting and context
/// menus.
async fn add_rows_html(tracer: &Tracer<'_>, map: &mut SubwayMap) {
    let server = tracer.server();
    let mut file_lines: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for card in &map.cards {
        for row in card.stations.iter().flat_map(|station| station.rows.iter()) {
            file_lines
                .entry(card.path.clone())
                .or_default()
                .insert(row.line);
        }
    }
    let mut sym_info: BTreeMap<Ustr, Option<JumprefData>> = BTreeMap::new();
    let mut traversed = UstrMap::default();
    for (path, lines) in file_lines {
        let key = ustr(&path);
        let Some(rows) = server
            .fetch_html_lines(vec![(key, lines)])
            .await
            .ok()
            .and_then(|mut rows| rows.remove(&key))
        else {
            continue;
        };
        let mut html = String::new();
        for card in map.cards.iter_mut().filter(|card| card.path == path) {
            for row in card
                .stations
                .iter_mut()
                .flat_map(|station| station.rows.iter_mut())
            {
                if let Some(code) = rows.get(&row.line).and_then(|r| code_html(&r.html)) {
                    html.push_str(&code);
                    row.html = Some(code);
                }
            }
        }
        add_html_sym_info(server, &html, &mut sym_info, &mut traversed).await;
    }
    // (The history digests are for `/query/`'s results.)
    for jumpref in sym_info.values_mut().flatten() {
        jumpref.recency = None;
        jumpref.recency_from = None;
    }
    map.sym_info = sym_info;
}

/// The inner HTML of a source listing's row's code (see `chunked_gzip::Row`).
fn code_html(row: &str) -> Option<String> {
    let start = row.find("<code")?;
    let open_end = start + row[start..].find('>')? + 1;
    let end = row.rfind("</code>")?;
    (end >= open_end).then(|| row[open_end..end].to_string())
}

/// The lines tainted in each of a map's cards (see `taint`), by their
/// sources (indexes in `TaintInfo::sources`).
type CardTaint = Vec<BTreeMap<usize, BTreeSet<usize>>>;

/// A function's validator call: the events it dominates (if it guards
/// them), its event, and the lines it validates.
struct FuncValidation {
    range: Option<(usize, usize)>,
    event: Option<usize>,
    lines: BTreeSet<usize>,
    by: String,
}

/// The sources (by index) a card's validations bless at an event.
fn blessed_at(
    validations: &[FuncValidation],
    tainted: &BTreeMap<usize, BTreeSet<usize>>,
    event: usize,
) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for v in validations {
        if let Some((start, end)) = v.range
            && start <= event
            && event < end
        {
            for line in &v.lines {
                out.extend(tainted.get(line).into_iter().flatten().copied());
            }
        }
    }
    out
}

/// The lines tainted at an event (those with sources not blessed there).
fn tainted_at(
    validations: &[FuncValidation],
    tainted: &BTreeMap<usize, BTreeSet<usize>>,
    event: usize,
) -> BTreeMap<usize, BTreeSet<usize>> {
    let blessed = blessed_at(validations, tainted, event);
    tainted
        .iter()
        .filter_map(|(&line, sources)| {
            let left: BTreeSet<usize> = sources.difference(&blessed).copied().collect();
            (!left.is_empty()).then_some((line, left))
        })
        .collect()
}

/// Mark a map's tainted lines, at its stations and along its connectors:
/// from their sources (the tainted parameters of the cards receiving
/// messages from content processes), through what's derived from them and
/// the calls, returns and fields' connectors to other cards, until the
/// validations dominating them bless them (and what's derived from their
/// sources).
async fn apply_taint(
    tracer: &mut Tracer<'_>,
    taint: &mut Taint,
    map: &mut SubwayMap,
    funcs: &[Func],
    key_line: &BTreeMap<Key, usize>,
    call_moves: &[CallMove],
    event_station: &HashMap<(usize, usize), usize>,
) {
    let mut info = TaintInfo {
        sources: vec![],
        validations: vec![],
    };
    let mut tainted: CardTaint = vec![BTreeMap::new(); map.cards.len()];
    // (Each source's field's keys' symbols, if it's a field.)
    let mut source_fields: Vec<Vec<Ustr>> = vec![];
    // ## The sources.
    for func in funcs {
        let f = &func.file.funcs[func.fi];
        let message = taint.source_message(tracer, f).await;
        let from = match &message {
            Some(message) => format!(
                "from content processes ({})",
                tracer
                    .crossref(message)
                    .await
                    .and_then(|xref| Some(xref.meta.as_ref()?.pretty.to_string()))
                    .unwrap_or_else(|| message.to_string())
            ),
            None if taint.is_source(f) => "(taint-from)".to_string(),
            None => continue,
        };
        for i in taint.tainted_params(tracer, f, message.as_ref()).await {
            let param = &f.params[i];
            let ty = param.type_pretty.clone().unwrap_or_default();
            let type_taint = taint.type_taint(tracer, &ty).await;
            let param_lines: BTreeSet<usize> = keys_of(f, &param.place)
                .iter()
                .filter_map(|key| key_line.get(key).copied())
                .collect();
            // (Its value, or its tainted fields, each a source of its own, so
            // that validating one doesn't bless the others; the parameter
            // carries them all.)
            let parts: Vec<Option<&TaintedField>> = match type_taint.whole {
                true => vec![None],
                false => type_taint.fields.iter().map(Some).collect(),
            };
            for part in parts {
                let (lines, name, field_syms) = match part {
                    None => (param_lines.clone(), param.name.clone(), vec![]),
                    Some(field) => (
                        field
                            .syms
                            .iter()
                            .filter_map(|sym| key_line.get(&Key::Field(*sym)).copied())
                            .collect::<BTreeSet<usize>>(),
                        format!("{}.{}", param.name, field.name),
                        field.syms.clone(),
                    ),
                };
                let Some(&line) = lines.iter().chain(param_lines.iter()).next() else {
                    map.notes.push(format!(
                        "{}'s {} is tainted ({}), but isn't on the map.",
                        f.pretty, name, from
                    ));
                    continue;
                };
                let si = info.sources.len();
                for &l in lines.iter().chain(param_lines.iter()) {
                    tainted[func.card].entry(l).or_default().insert(si);
                }
                source_fields.push(field_syms);
                info.sources.push(TaintSource {
                    card: func.card,
                    line,
                    field: part.map(|field| field.name.clone()),
                    why: format!(
                        "{} receives {} ({}) {}",
                        f.pretty,
                        name,
                        part.map_or(ty.as_str(), |field| field.ty.as_str()),
                        from
                    ),
                    status: "unused",
                    stops: 0,
                    tainted_stops: 0,
                });
            }
        }
    }
    if info.sources.is_empty() {
        return;
    }
    // ## The validations: calls of validators, or of their wrappers (of
    // the tainted lines' families' values).
    let source_families: BTreeSet<usize> = info
        .sources
        .iter()
        .map(|source| map.lines[source.line].family)
        .collect();
    let mut validations: Vec<Vec<FuncValidation>> = (0..map.cards.len()).map(|_| vec![]).collect();
    for func in funcs {
        let f = &func.file.funcs[func.fi];
        for (c, call) in f.calls.iter().enumerate() {
            let mut validated = taint.validated(tracer, call).await;
            if validated.is_empty()
                && call
                    .args
                    .iter()
                    .chain(call.receiver.iter())
                    .flat_map(|arg| val_keys(f, &arg.vals, &func.sums))
                    .filter_map(|(key, _)| key_line.get(&key))
                    .any(|&line| source_families.contains(&map.lines[line].family))
            {
                validated = taint
                    .wrapper_validated(tracer, &func.file, func.fi, c)
                    .await;
            }
            if validated.is_empty() {
                continue;
            }
            let mut vals: Vec<&[Val]> = vec![];
            for v in &validated {
                match v {
                    Validated::Arg(i) => vals.extend(call.args.get(*i).map(|a| a.vals.as_slice())),
                    Validated::This => vals.extend(call.receiver.iter().map(|r| r.vals.as_slice())),
                    Validated::All => vals.extend(
                        call.args
                            .iter()
                            .chain(call.receiver.iter())
                            .map(|a| a.vals.as_slice()),
                    ),
                }
            }
            let lines: BTreeSet<usize> = vals
                .into_iter()
                .flat_map(|vals| val_keys(f, vals, &func.sums))
                .filter_map(|(key, _)| key_line.get(&key).copied())
                .collect();
            let event = f
                .events
                .iter()
                .position(|e| matches!(e.kind, EventKind::Call(ec) if ec == c));
            let ranges: Vec<(usize, usize)> = f
                .guards
                .iter()
                .filter(|g| g.call == c)
                .map(|g| (g.start, g.end))
                .collect();
            let by = call.callee_pretty.clone();
            if ranges.is_empty() {
                validations[func.card].push(FuncValidation {
                    range: None,
                    event,
                    lines,
                    by,
                });
            } else {
                for range in ranges {
                    validations[func.card].push(FuncValidation {
                        range: Some(range),
                        event,
                        lines: lines.clone(),
                        by: by.clone(),
                    });
                }
            }
        }
    }
    // ## Propagation, until it settles: to what's derived in a card, and
    // along the connectors, what isn't blessed where they leave.
    let last_event = |card: usize| -> usize {
        funcs
            .iter()
            .find(|func| func.card == card)
            .map_or(0, |func| {
                func.file.funcs[func.fi].events.len().saturating_sub(1)
            })
    };
    let mut station_events: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (&(card, ei), &station) in event_station {
        station_events.entry((card, station)).or_default().push(ei);
    }
    for events in station_events.values_mut() {
        events.sort();
    }
    let line_parents: Vec<Vec<usize>> = map.lines.iter().map(|l| l.parents.clone()).collect();
    let card_lines: Vec<BTreeSet<usize>> = map
        .cards
        .iter()
        .map(|card| {
            card.stations
                .iter()
                .flat_map(|s| s.stops.iter().copied())
                .collect()
        })
        .collect();
    // (The lines that are parts of others in each card, with the parts'
    // fields: a field of an object (`aTicket.mId`, its own line, by its
    // field's key; or a getter's, by its summary) is the object's, for
    // taint.)
    let mut card_parts: Vec<BTreeSet<(usize, usize, Ustr)>> =
        vec![BTreeSet::new(); map.cards.len()];
    for func in funcs {
        let f = &func.file.funcs[func.fi];
        let mut add_part = |whole: &Place, part: &Place| {
            for whole in keys_of(f, whole) {
                for part in keys_of(f, part) {
                    if let Key::Field(field) = part
                        && let (Some(&w), Some(&p)) = (key_line.get(&whole), key_line.get(&part))
                        && w != p
                    {
                        card_parts[func.card].insert((w, p, field));
                    }
                }
            }
        };
        f.for_each_place(&mut |place| {
            if !place.path.is_empty() {
                let whole = Place {
                    root: place.root.clone(),
                    path: vec![],
                };
                add_part(&whole, place);
            }
        });
        for (c, summary) in &func.sums {
            let call = &f.calls[*c];
            for flow in &summary.flows {
                let (arg, fields) = match &flow.from {
                    Port::This { fields } => (call.receiver.as_ref(), fields),
                    Port::Param { i, fields } => (call.args.get(*i), fields),
                    _ => continue,
                };
                if let Some(place) = arg.and_then(|arg| arg.place.as_ref())
                    && !fields.is_empty()
                {
                    let mut part = place.clone();
                    for field in fields {
                        part = part.with(Step::Field(*field));
                    }
                    add_part(place, &part);
                }
            }
        }
    }
    // (A connector's tainted lines, by its from, from station, to, to
    // station and kind.)
    type ConnectorKey = (usize, Option<usize>, usize, Option<usize>, &'static str);
    let mut connector_taint: BTreeMap<ConnectorKey, BTreeSet<usize>> = BTreeMap::new();
    for _ in 0..(map.cards.len() + 4) {
        let mut changed = false;
        let mut add =
            |tainted: &mut CardTaint, card: usize, line: usize, sources: &BTreeSet<usize>| {
                let entry = tainted[card].entry(line).or_default();
                let before = entry.len();
                entry.extend(sources.iter().copied());
                if entry.len() != before {
                    changed = true;
                }
            };
        // (Derived values, in the cards they're in.)
        for card in 0..map.cards.len() {
            for &line in &card_lines[card] {
                let mut sources = BTreeSet::new();
                for parent in &line_parents[line] {
                    sources.extend(tainted[card].get(parent).into_iter().flatten().copied());
                }
                if !sources.is_empty() {
                    add(&mut tainted, card, line, &sources);
                }
            }
            // (A part has its whole's taint, but that of its other fields.)
            for &(whole, part, field) in &card_parts[card] {
                if let Some(sources) = tainted[card].get(&whole) {
                    let sources: BTreeSet<usize> = sources
                        .iter()
                        .copied()
                        .filter(|&si| {
                            source_fields[si].is_empty() || source_fields[si].contains(&field)
                        })
                        .collect();
                    if !sources.is_empty() {
                        add(&mut tainted, card, part, &sources);
                    }
                }
            }
        }
        // (Calls' arguments, and their returns.)
        for call in call_moves {
            let (from, to) = (funcs[call.from].card, funcs[call.to].card);
            let station = event_station.get(&(from, call.event)).copied();
            let at_call = tainted_at(&validations[from], &tainted[from], call.event);
            let args: BTreeSet<usize> = call
                .args
                .iter()
                .filter_map(|(a, b, _)| Some(*key_line.get(a).or_else(|| key_line.get(b))?))
                .collect();
            for line in args {
                if let Some(sources) = at_call.get(&line) {
                    let sources = sources.clone();
                    add(&mut tainted, to, line, &sources);
                    connector_taint
                        .entry((from, station, to, None, call.kind))
                        .or_default()
                        .insert(line);
                }
                // (A message's tainted parameters, along its IPC edge.)
                if call.kind == "ipc"
                    && info
                        .sources
                        .iter()
                        .any(|source| source.card == to && source.line == line)
                {
                    connector_taint
                        .entry((from, station, to, None, call.kind))
                        .or_default()
                        .insert(line);
                }
            }
            let at_return = tainted_at(&validations[to], &tainted[to], last_event(to));
            let returns: BTreeSet<usize> = call
                .returns
                .iter()
                .filter_map(|(a, b, _)| Some(*key_line.get(a).or_else(|| key_line.get(b))?))
                .collect();
            for line in returns {
                if let Some(sources) = at_return.get(&line) {
                    let sources = sources.clone();
                    add(&mut tainted, from, line, &sources);
                    connector_taint
                        .entry((to, None, from, station, "return"))
                        .or_default()
                        .insert(line);
                }
            }
        }
        // (Fields, from the stations writing them.)
        for connector in &map.connectors {
            if connector.kind != "field" {
                continue;
            }
            let Some(station) = connector.from_station else {
                continue;
            };
            let event = station_events
                .get(&(connector.from, station))
                .and_then(|events| events.last().copied())
                .unwrap_or_else(|| last_event(connector.from));
            let at_write = tainted_at(
                &validations[connector.from],
                &tainted[connector.from],
                event,
            );
            for &line in &connector.lines {
                if let Some(sources) = at_write.get(&line) {
                    let sources = sources.clone();
                    add(&mut tainted, connector.to, line, &sources);
                    connector_taint
                        .entry((
                            connector.from,
                            connector.from_station,
                            connector.to,
                            connector.to_station,
                            connector.kind,
                        ))
                        .or_default()
                        .insert(line);
                }
            }
        }
        if !changed {
            break;
        }
    }
    // ## The map's taint: its stations', cards', connectors' and lines'.
    for card in 0..map.cards.len() {
        if tainted[card].is_empty() && validations[card].is_empty() {
            continue;
        }
        for (station, s) in map.cards[card].stations.iter_mut().enumerate() {
            let Some(events) = station_events.get(&(card, station)) else {
                continue;
            };
            let first = events[0];
            s.tainted = tainted_at(&validations[card], &tainted[card], first)
                .into_keys()
                .collect();
            let mut validates = BTreeSet::new();
            for v in &validations[card] {
                if let Some(event) = v.event
                    && events.contains(&event)
                {
                    if v.range.is_some() {
                        validates.extend(v.lines.iter().copied());
                    }
                    info.validations.push(Validation {
                        card,
                        station,
                        lines: v.lines.iter().copied().collect(),
                        by: v.by.clone(),
                        guards: v.range.is_some(),
                    });
                }
            }
            s.validates = validates.into_iter().collect();
        }
        map.cards[card].tainted_out =
            tainted_at(&validations[card], &tainted[card], last_event(card))
                .into_keys()
                .collect();
        for &line in tainted[card].keys() {
            map.lines[line].tainted = true;
        }
    }
    info.validations.dedup_by(|a, b| {
        a.card == b.card && a.station == b.station && a.by == b.by && a.lines == b.lines
    });
    // (The sources' statuses, in their cards.)
    for (si, source) in info.sources.iter_mut().enumerate() {
        let card = source.card;
        let carrying: BTreeSet<usize> = tainted[card]
            .iter()
            .filter(|(_, sources)| sources.contains(&si))
            .map(|(&line, _)| line)
            .collect();
        let validated_here = |v: &FuncValidation| {
            v.range.is_some() && v.lines.iter().any(|line| carrying.contains(line))
        };
        let blessed = validations[card].iter().any(validated_here);
        for (station, s) in map.cards[card].stations.iter().enumerate() {
            let Some(events) = station_events.get(&(card, station)) else {
                continue;
            };
            if !s.stops.iter().any(|line| carrying.contains(line))
                || validations[card]
                    .iter()
                    .any(|v| validated_here(v) && v.event.is_some_and(|e| events.contains(&e)))
            {
                continue;
            }
            source.stops += 1;
            let at = tainted_at(&validations[card], &tainted[card], events[0]);
            if s.stops.iter().any(|line| {
                carrying.contains(line) && at.get(line).is_some_and(|ss| ss.contains(&si))
            }) {
                source.tainted_stops += 1;
            }
        }
        source.status = match (source.stops, source.tainted_stops, blessed) {
            (0, _, _) => "unused",
            (_, 0, true) => "validated",
            (_, _, false) => "unvalidated",
            _ => "partly",
        };
    }
    for connector in &mut map.connectors {
        if let Some(lines) = connector_taint.get(&(
            connector.from,
            connector.from_station,
            connector.to,
            connector.to_station,
            connector.kind,
        )) {
            connector.tainted = lines
                .iter()
                .copied()
                .filter(|line| connector.lines.contains(line))
                .collect();
        }
    }
    map.taint = Some(info);
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
