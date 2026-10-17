//! Following values through `FuncIr`s: forwards (where does this value go?)
//! and backwards (where does it come from?), across calls (into callees'
//! parameters and back out of their returns and out-parameters, matching
//! returns to the calls they came from when we know them), callers (from
//! crossref's uses), fields and globals (from crossref's uses of them,
//! wherever they're written or read), binding layers (XPIDL's JS methods to
//! their C++ implementations, from binding slots), and IPDL structs' fields
//! (see `IpdlSlot`).
//!
//! This is demand-driven and lazy: only the functions a trace reaches are
//! parsed, and only the symbols it reaches are looked up.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde::Serialize;
use tokio_stream::StreamExt;
use ustr::{Ustr, ustr};

use super::frontend::{FileIr, build_file_ir, lang_for_path};
use super::ir::*;
use super::oracle::{Oracle, OracleQuestion, SITE_ASK, SUMMARY_ASK, Slot, parse_slot};
use super::records::{FileRecords, short_name};
use crate::abstract_server::AbstractServer;
use crate::file_format::analysis::BindingSlotKind;
use crate::file_format::crossref::CrossrefData;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Forward,
    Backward,
}

#[derive(Clone, Debug)]
pub struct TraceOpts {
    /// How many work items to process at most.
    pub max_items: usize,
    /// How many calls, returns, fields and bindings to cross at most.
    pub max_hops: usize,
    /// How many callers, implementations, or uses of a field to follow from
    /// each.
    pub fanout: usize,
    /// Whether to only consider events after (forwards) or before
    /// (backwards) where a value is, in source order.
    pub ordered: bool,
}

impl Default for TraceOpts {
    fn default() -> Self {
        TraceOpts {
            max_items: 600,
            max_hops: 6,
            fanout: 8,
            ordered: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Seed,
    Assign,
    Call,
    Return,
    Test,
    Param,
    Const,
    /// A call we couldn't see into: we assume its result comes from its
    /// arguments (and receiver).
    Opaque,
}

#[derive(Clone, Debug, Serialize)]
pub struct TraceNode {
    pub id: usize,
    pub kind: NodeKind,
    pub path: String,
    pub line: u32,
    pub func: String,
    pub func_sym: Ustr,
    pub lang: Lang,
    /// What happens there (ex: "rv = …", "call GetBoolPref", "param aRetVal").
    pub label: String,
    /// The source line.
    pub text: String,
    /// For calls, the call's text (several can share a line).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// Within a function.
    Local,
    /// Into a callee (argument to parameter, receiver to `this`).
    Call,
    /// Out of a callee (its return or out-parameter to the call).
    Return,
    /// Through a field or global, between functions.
    Field,
    /// Through a binding layer (ex: XPIDL's JS to C++).
    Binding,
    /// Through a call we couldn't see into.
    Assumed,
    /// Through a call an oracle told us about (see `oracle`).
    Oracle,
}

#[derive(Clone, Debug, Serialize)]
pub struct TraceEdge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    /// The place or value carried (ex: "*aRetVal").
    pub label: String,
}

#[derive(Default, Serialize)]
pub struct TraceGraph {
    pub dir: Option<Dir>,
    pub nodes: Vec<TraceNode>,
    pub edges: Vec<TraceEdge>,
    /// Where we stopped or guessed: limits hit, calls we couldn't resolve.
    pub notes: Vec<String>,
}

/// How a call's arguments map to its callee's parameters.
#[derive(Clone, Debug, PartialEq)]
enum ArgMap {
    Direct,
    /// XPIDL's JS-to-C++ binding: JS's arguments are C++'s first parameters,
    /// then `_argc` (for `[optional_argc]`), then `_retval` (the JS result).
    Xpidl {
        argc: Option<usize>,
        retval: Option<usize>,
    },
}

/// A function a call reaches.
#[derive(Clone)]
struct Target {
    file: Arc<FileIr>,
    func: usize,
    map: ArgMap,
    /// How we got there (ex: "override", "xpidl").
    via: Option<String>,
}

/// A call we entered a callee from, for returning to it.
#[derive(Clone)]
struct Site {
    file: Arc<FileIr>,
    func: usize,
    call: usize,
    map: ArgMap,
    /// The parameter a backwards trace entered the callee through, to find
    /// what the callee writes through it (see `backward_place`).
    entered: Option<usize>,
}

/// What a C++ function is of the code IPDL generates for a struct or union
/// (by ipdl-analyze's binding slots), for following values through their
/// fields rather than into the generated code implementing them: a field's
/// values go in through the struct's constructors' arguments and
/// assignments to its mutable accessor's result (`x.f() = v`), and come out
/// of its accessors' calls, wherever they are (by crossref's uses of the
/// slots, as fields' reads and writes are found; see `forward_field`).
#[derive(Clone, Debug, PartialEq)]
pub enum IpdlSlot {
    /// An accessor of a field (or of a union's variant), by the field's
    /// `IPDL_` symbol.
    Accessor(Ustr),
    /// A struct's constructor from its fields: its arguments are their
    /// values, in order (their `IPDL_` symbols).
    Constructor(Vec<Ustr>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Flow {
    Place(Place),
    /// A call's result.
    Result(usize),
}

struct Item {
    file: Arc<FileIr>,
    func: usize,
    flow: Flow,
    /// Events after (forwards) or before (backwards) this position count;
    /// None for all (entering a function).
    pos: Option<isize>,
    /// The node it's from (forwards) or to (backwards).
    node: usize,
    label: String,
    kind: EdgeKind,
    stack: Vec<Site>,
    hops: usize,
    /// Whether the value was put in the place here (assigned, returned to,
    /// or the seed), rather than being there when we entered (a parameter's,
    /// or a field's read elsewhere): only then does it leave the function
    /// through it.
    fresh: bool,
    /// The concrete type of the place's root, if the function's is a
    /// template parameter's (ex: `bool *` for `T&& aResult`), from the call
    /// that entered it.
    root_ty: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum NodeKey {
    Event(usize),
    Param(usize),
    Const(String, u32),
    Seed(String),
    Opaque(usize),
}

pub struct Tracer<'s> {
    server: &'s (dyn AbstractServer + Send + Sync),
    pub opts: TraceOpts,
    files: HashMap<String, Option<Arc<FileIr>>>,
    sources: HashMap<String, Option<Arc<String>>>,
    records: HashMap<String, Option<Arc<FileRecords>>>,
    xrefs: HashMap<Ustr, Option<Arc<CrossrefData>>>,
    ipdl_slots: HashMap<Ustr, Option<IpdlSlot>>,
    pub oracle: Option<Arc<Oracle>>,
    pub graph: TraceGraph,
    node_ids: HashMap<(String, Ustr, NodeKey), usize>,
    edge_set: HashSet<(usize, usize, String)>,
    queue: VecDeque<Item>,
    seen: HashSet<(String, Ustr, String, bool)>,
    noted: HashSet<String>,
}

impl<'s> Tracer<'s> {
    pub fn new(server: &'s (dyn AbstractServer + Send + Sync), opts: TraceOpts) -> Self {
        Tracer {
            server,
            opts,
            files: HashMap::new(),
            sources: HashMap::new(),
            records: HashMap::new(),
            xrefs: HashMap::new(),
            ipdl_slots: HashMap::new(),
            oracle: None,
            graph: TraceGraph::default(),
            node_ids: HashMap::new(),
            edge_set: HashSet::new(),
            queue: VecDeque::new(),
            seen: HashSet::new(),
            noted: HashSet::new(),
        }
    }

    /// The IR of a file's functions, if we have a frontend for its language
    /// and its source and analysis.
    pub async fn file(&mut self, path: &str) -> Option<Arc<FileIr>> {
        if let Some(cached) = self.files.get(path) {
            return cached.clone();
        }
        let start = std::time::Instant::now();
        let built = self.build_file(path).await;
        if std::env::var("DATAFLOW_DEBUG").is_ok() {
            eprintln!("file {} {:?} ok={}", path, start.elapsed(), built.is_ok());
        }
        if let Err(e) = &built {
            self.note(format!("Couldn't analyze {}: {}", path, e));
        }
        let file = built.ok().map(Arc::new);
        self.files.insert(path.to_string(), file.clone());
        file
    }

    async fn build_file(&mut self, path: &str) -> Result<FileIr, String> {
        if lang_for_path(path).is_none() {
            return Err("no dataflow frontend for its language".to_string());
        }
        let source = self.source(path).await.ok_or("no source")?;
        let recs = self.records(path).await.ok_or("no analysis")?;
        build_file_ir(path, &source, &recs)
    }

    /// A file's source.
    pub async fn source(&mut self, path: &str) -> Option<Arc<String>> {
        if let Some(cached) = self.sources.get(path) {
            return cached.clone();
        }
        let source = self.server.fetch_raw_source(path).await.ok().map(Arc::new);
        self.sources.insert(path.to_string(), source.clone());
        source
    }

    /// A file's analysis records.
    pub async fn records(&mut self, path: &str) -> Option<Arc<FileRecords>> {
        if let Some(cached) = self.records.get(path) {
            return cached.clone();
        }
        let records = match self.server.fetch_raw_analysis(path).await {
            Ok(stream) => {
                let values: Vec<serde_json::Value> = stream.collect().await;
                Some(Arc::new(FileRecords::from_values(values)))
            }
            Err(_) => None,
        };
        self.records.insert(path.to_string(), records.clone());
        records
    }

    /// A tracer for a sub-trace with at most `max_hops`, sharing this one's
    /// caches (until `absorb`ed back).
    pub fn fork(&mut self, max_hops: usize) -> Tracer<'s> {
        let mut opts = self.opts.clone();
        opts.max_hops = max_hops;
        opts.max_items = opts.max_items.min(200);
        let mut sub = Tracer::new(self.server, opts);
        sub.oracle = self.oracle.clone();
        sub.files = std::mem::take(&mut self.files);
        sub.sources = std::mem::take(&mut self.sources);
        sub.records = std::mem::take(&mut self.records);
        sub.xrefs = std::mem::take(&mut self.xrefs);
        sub.ipdl_slots = std::mem::take(&mut self.ipdl_slots);
        sub
    }

    pub fn absorb(&mut self, sub: Tracer<'s>) {
        self.files = sub.files;
        self.sources = sub.sources;
        self.records = sub.records;
        self.xrefs = sub.xrefs;
        self.ipdl_slots = sub.ipdl_slots;
    }

    /// Questions for an oracle about the calls in some files that we can't
    /// see into (see `oracle`): one about each callee without a definition
    /// we can follow (that the oracle doesn't already answer), and one about
    /// each call we couldn't resolve at all, most-blocking first.
    pub async fn questions(&mut self, paths: &[String], max: usize) -> Vec<OracleQuestion> {
        let mut by_callee: HashMap<Ustr, OracleQuestion> = HashMap::new();
        let mut sites: Vec<OracleQuestion> = vec![];
        for path in paths {
            let Some(file) = self.file(path).await else {
                continue;
            };
            let source = self.source(path).await.unwrap_or_default();
            let lines: Vec<&str> = source.lines().collect();
            for f in &file.funcs {
                for call in &f.calls {
                    let site = format!(
                        "{}:{}: {}",
                        path,
                        call.line,
                        lines
                            .get(call.line as usize - 1)
                            .map(|l| l.trim())
                            .unwrap_or("")
                    );
                    if call.callee.is_empty() {
                        if call.kind == CallKind::Macro {
                            continue;
                        }
                        let start = f.line.saturating_sub(1) as usize;
                        let end = (f.end_line as usize).min(lines.len()).min(start + 80);
                        sites.push(OracleQuestion {
                            id: format!("site:{}:{}", path, call.line),
                            kind: "indirect-call".to_string(),
                            sym: None,
                            pretty: call.text.clone(),
                            params: vec![],
                            count: 1,
                            sites: vec![site],
                            definition: Some(format!("{}:{} {}", path, f.line, f.pretty)),
                            source_hash: None,
                            excerpt: Some(lines[start..end].join("\n")),
                            ask: SITE_ASK.to_string(),
                        });
                        continue;
                    }
                    let sym = call.callee[0];
                    if let Some(q) = by_callee.get_mut(&sym) {
                        q.count += 1;
                        if q.sites.len() < 3 {
                            q.sites.push(site);
                        }
                        continue;
                    }
                    if self
                        .oracle
                        .as_ref()
                        .is_some_and(|o| o.summary(&call.callee).is_some())
                    {
                        continue;
                    }
                    let mut resolved = false;
                    for s in call.callee.iter().take(self.opts.fanout) {
                        if !self.resolve(s).await.is_empty() {
                            resolved = true;
                            break;
                        }
                    }
                    if resolved || ipc_message_sym(&call.callee_pretty).is_some() {
                        // (Count it as answered, so it's not looked up again.)
                        by_callee.insert(
                            sym,
                            OracleQuestion {
                                id: String::new(),
                                kind: "resolved".to_string(),
                                sym: Some(sym),
                                pretty: String::new(),
                                params: vec![],
                                count: 1,
                                sites: vec![],
                                definition: None,
                                source_hash: None,
                                excerpt: None,
                                ask: String::new(),
                            },
                        );
                        continue;
                    }
                    let mut params = vec![];
                    let mut pretty = call.callee_pretty.clone();
                    if let Some(xref) = self.xref(&sym).await
                        && let Some(meta) = &xref.meta
                    {
                        pretty = meta.pretty.to_string();
                        if let Some(args) = meta.extra.get("args").and_then(|a| a.as_array()) {
                            for arg in args {
                                params.push(format!(
                                    "{} {}",
                                    arg.get("type").and_then(|t| t.as_str()).unwrap_or("?"),
                                    arg.get("name").and_then(|n| n.as_str()).unwrap_or("")
                                ));
                            }
                        }
                    }
                    by_callee.insert(
                        sym,
                        OracleQuestion {
                            id: format!("summary:{}", sym),
                            kind: "summary".to_string(),
                            sym: Some(sym),
                            pretty,
                            params,
                            count: 1,
                            sites: vec![site],
                            definition: None,
                            source_hash: None,
                            excerpt: None,
                            ask: SUMMARY_ASK.to_string(),
                        },
                    );
                }
            }
        }
        let mut out: Vec<OracleQuestion> = by_callee
            .into_values()
            .filter(|q| q.kind != "resolved")
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
        out.truncate(max);
        sites.truncate(max / 4);
        out.extend(sites);
        out
    }

    /// The owner of the binding a function is a slot of (ex: an XPIDL
    /// method's, an IPDL message's), or implements one of (ex: an actor's
    /// override of the generated `Recv` declaration), if any.
    pub async fn binding_owner(&mut self, sym: &Ustr) -> Option<Ustr> {
        let xref = self.xref(sym).await?;
        let meta = xref.meta.as_ref()?;
        if let Some(owner) = &meta.slot_owner {
            return Some(owner.sym);
        }
        if let Some(owner) = ipc_message_sym(&meta.pretty) {
            return Some(owner);
        }
        for overridden in &meta.overrides {
            if let Some(owner) = self
                .xref(&overridden.sym)
                .await
                .and_then(|xref| Some(xref.meta.as_ref()?.slot_owner.as_ref()?.sym))
            {
                return Some(owner);
            }
        }
        None
    }

    async fn xref(&mut self, sym: &Ustr) -> Option<Arc<CrossrefData>> {
        if let Some(cached) = self.xrefs.get(sym) {
            return cached.clone();
        }
        let start = std::time::Instant::now();
        let data = self
            .server
            .crossref_lookup(sym)
            .await
            .ok()
            .flatten()
            .map(Arc::new);
        if std::env::var("DATAFLOW_DEBUG").is_ok() {
            eprintln!(
                "xref {} {:?} found={}",
                sym,
                start.elapsed(),
                data.is_some()
            );
        }
        self.xrefs.insert(*sym, data.clone());
        data
    }

    /// What a symbol is of the code IPDL generates for a struct or union,
    /// if anything.
    pub async fn ipdl_slot(&mut self, sym: &Ustr) -> Option<IpdlSlot> {
        // (C++ functions only.)
        if !sym.starts_with("_Z") {
            return None;
        }
        if let Some(cached) = self.ipdl_slots.get(sym) {
            return cached.clone();
        }
        let owner = self
            .xref(sym)
            .await
            .and_then(|xref| xref.meta.as_ref()?.slot_owner.clone());
        let slot = match owner {
            Some(owner) if owner.sym.starts_with("IPDL_") => match owner.props.slot_kind {
                BindingSlotKind::Getter | BindingSlotKind::Setter => {
                    Some(IpdlSlot::Accessor(owner.sym))
                }
                BindingSlotKind::Constructor => {
                    // (Its fields are in order; see ipdl-analyze.)
                    let fields: Vec<Ustr> = self
                        .xref(&owner.sym)
                        .await
                        .and_then(|xref| {
                            Some(xref.meta.as_ref()?.fields.iter().map(|f| f.sym).collect())
                        })
                        .unwrap_or_default();
                    (!fields.is_empty()).then_some(IpdlSlot::Constructor(fields))
                }
                _ => None,
            },
            _ => None,
        };
        self.ipdl_slots.insert(*sym, slot.clone());
        slot
    }

    /// The class a function is a method of, if any.
    pub async fn class_of(&mut self, sym: &Ustr) -> Option<Ustr> {
        let parent = self.xref(sym).await?.meta.as_ref()?.parent_sym?;
        parent.starts_with("T_").then_some(parent)
    }

    /// A call's IPDL slot (its first callee's that is one), if any.
    pub async fn call_ipdl_slot(&mut self, call: &Call) -> Option<IpdlSlot> {
        for sym in &call.callee {
            if let Some(slot) = self.ipdl_slot(sym).await {
                return Some(slot);
            }
        }
        None
    }

    /// The IPDL field a place is (a part of), if it's one assigned to through
    /// a mutable accessor (see the C++ frontend's assignments).
    pub async fn ipdl_field_of_place(&mut self, p: &Place) -> Option<Ustr> {
        for step in &p.path {
            if let Step::Field(sym) = step
                && let Some(IpdlSlot::Accessor(field)) = self.ipdl_slot(sym).await
            {
                return Some(field);
            }
        }
        None
    }

    /// An IPDL field's accessors, and its struct's constructors with the
    /// field's index among their parameters.
    async fn ipdl_field_slots(&mut self, field: Ustr) -> (Vec<Ustr>, Vec<(Ustr, usize)>) {
        let Some(xref) = self.xref(&field).await else {
            return (vec![], vec![]);
        };
        let Some(meta) = &xref.meta else {
            return (vec![], vec![]);
        };
        let accessors = meta
            .binding_slots
            .iter()
            .filter(|s| {
                matches!(
                    s.props.slot_kind,
                    BindingSlotKind::Getter | BindingSlotKind::Setter
                )
            })
            .map(|s| s.sym)
            .collect();
        let mut constructors = vec![];
        if let Some(parent) = meta.parent_sym
            && let Some(owner) = self.xref(&parent).await
            && let Some(owner_meta) = &owner.meta
            && let Some(index) = owner_meta.fields.iter().position(|f| f.sym == field)
        {
            constructors.extend(
                owner_meta
                    .binding_slots
                    .iter()
                    .filter(|s| s.props.slot_kind == BindingSlotKind::Constructor)
                    .map(|s| (s.sym, index)),
            );
        }
        (accessors, constructors)
    }

    /// Whether a call of an IPDL field's accessor reads (a part of) the
    /// value in a flow: the receiver's whole value, or that field of it.
    async fn reads_ipdl_field(&mut self, call: &Call, flow: &Flow, field: Ustr) -> bool {
        let (
            Flow::Place(p),
            Some(Arg {
                place: Some(rp), ..
            }),
        ) = (flow, &call.receiver)
        else {
            return true;
        };
        if !p.within(rp) || p.path.len() == rp.path.len() {
            return true;
        }
        match &p.path[rp.path.len()] {
            Step::Field(sym) => match self.ipdl_slot(sym).await {
                Some(IpdlSlot::Accessor(of)) => of == field,
                _ => true,
            },
            _ => true,
        }
    }

    fn note(&mut self, note: String) {
        if self.noted.insert(note.clone()) {
            self.graph.notes.push(note);
        }
    }

    fn node(
        &mut self,
        file: &FileIr,
        func: usize,
        key: NodeKey,
        kind: NodeKind,
        line: u32,
        label: String,
    ) -> usize {
        let f = &file.funcs[func];
        let map_key = (file.path.clone(), f.sym, key);
        if let Some(&id) = self.node_ids.get(&map_key) {
            return id;
        }
        let id = self.graph.nodes.len();
        let text = f
            .events
            .iter()
            .find(|e| e.line == line)
            .map(|e| e.text.clone())
            .unwrap_or_default();
        self.graph.nodes.push(TraceNode {
            id,
            kind,
            path: file.path.clone(),
            line,
            func: f.pretty.clone(),
            func_sym: f.sym,
            lang: f.lang,
            label,
            text,
            detail: String::new(),
        });
        self.node_ids.insert(map_key, id);
        id
    }

    fn event_node(&mut self, file: &FileIr, func: usize, event: usize) -> usize {
        let f = &file.funcs[func];
        let e = &f.events[event];
        let (kind, label) = match &e.kind {
            EventKind::Assign { place, op, .. } => (
                NodeKind::Assign,
                format!(
                    "{} {}",
                    f.place_name(place),
                    if op == "init" { "=" } else { op }
                ),
            ),
            EventKind::Call(c) => (
                NodeKind::Call,
                format!("call {}", short_pretty(&f.calls[*c].callee_pretty)),
            ),
            EventKind::Return { .. } => (NodeKind::Return, "return".to_string()),
            EventKind::Test { .. } => (NodeKind::Test, "test".to_string()),
        };
        let line = e.line;
        let detail = match &e.kind {
            EventKind::Call(c) => f.calls[*c].text.clone(),
            _ => String::new(),
        };
        let id = self.node(file, func, NodeKey::Event(event), kind, line, label);
        if self.graph.nodes[id].detail.is_empty() {
            self.graph.nodes[id].detail = detail;
        }
        id
    }

    /// An edge in the direction values flow.
    fn edge(&mut self, from: usize, to: usize, kind: EdgeKind, label: String) {
        if from == to {
            return;
        }
        if self.edge_set.insert((from, to, label.clone())) {
            self.graph.edges.push(TraceEdge {
                from,
                to,
                kind,
                label,
            });
        }
    }

    // ## Seeds

    /// Start tracing a place in a function, at a line (forwards: from its
    /// start; backwards: from its end), or anywhere in the function.
    pub fn seed(
        &mut self,
        file: Arc<FileIr>,
        func: usize,
        place: Place,
        line: Option<u32>,
        dir: Dir,
    ) {
        self.graph.dir = Some(dir);
        let f = &file.funcs[func];
        let name = f.place_name(&place);
        let seed_line = line.unwrap_or(f.line);
        let node = self.node(
            &file,
            func,
            NodeKey::Seed(name.clone()),
            NodeKind::Seed,
            seed_line,
            name.clone(),
        );
        // (Forwards: after the events before the line; backwards: before
        // the events after it.)
        let pos = line.map(|l| match dir {
            Dir::Forward => {
                f.events
                    .iter()
                    .position(|e| e.end_line >= l)
                    .unwrap_or(f.events.len()) as isize
                    - 1
            }
            Dir::Backward => f
                .events
                .iter()
                .rposition(|e| e.line <= l)
                .map_or(0, |i| i as isize + 1),
        });
        self.queue.push_back(Item {
            file,
            func,
            flow: Flow::Place(place),
            pos,
            node,
            label: name,
            kind: EdgeKind::Local,
            stack: vec![],
            hops: 0,
            fresh: true,
            root_ty: None,
        });
    }

    /// Start tracing a symbol's value from a line of a file, as a context
    /// menu knows it: a local or parameter (in the function at the line), a
    /// field or global (the place at the line with it in that function), an
    /// IPDL field's accessor (its call's result at the line), or, on a line
    /// outside functions (ex: a field's declaration, or an IPDL field's), a
    /// field or global everywhere it's read (forwards) or written
    /// (backwards).
    pub async fn seed_symbol(
        &mut self,
        path: &str,
        line: u32,
        sym: Ustr,
        dir: Dir,
    ) -> Result<(), String> {
        self.graph.dir = Some(dir);
        if sym.starts_with("IPDL_") {
            // (Where the IPDL file declares it, which has no IR.)
            let name = self.symbol_name(&sym).await;
            let text = self
                .source(path)
                .await
                .and_then(|s| {
                    s.lines()
                        .nth(line as usize - 1)
                        .map(|l| l.trim().to_string())
                })
                .unwrap_or_default();
            let id = self.graph.nodes.len();
            self.graph.nodes.push(TraceNode {
                id,
                kind: NodeKind::Seed,
                path: path.to_string(),
                line,
                func: name.clone(),
                func_sym: sym,
                lang: Lang::Cpp,
                label: name,
                text,
                detail: String::new(),
            });
            match dir {
                Dir::Forward => self.forward_ipdl_field(sym, id, 0).await,
                Dir::Backward => self.backward_ipdl_field(sym, id, 0).await,
            }
            return Ok(());
        }
        let Some(file) = self.file(path).await else {
            return Err(format!("Can't follow values in {}", path));
        };
        let func = file
            .funcs
            .iter()
            .enumerate()
            .filter(|(_, f)| f.line <= line && line <= f.end_line)
            .min_by_key(|(_, f)| f.end_line - f.line)
            .map(|(i, _)| i);
        if let Some(fi) = func {
            let f = &file.funcs[fi];
            let is_local = sym.starts_with("V_")
                || sym.starts_with("L_")
                || super::frontend::is_scip_local(&sym);
            if is_local
                && (f.names.contains_key(&sym)
                    || f.params.iter().any(|p| p.place == Place::local(sym)))
            {
                self.seed(file.clone(), fi, Place::local(sym), Some(line), dir);
                return Ok(());
            }
            let mut places = places_with_field(f, line, sym, true);
            for place in places_with_field(f, line, sym, false) {
                if !places.contains(&place) {
                    places.push(place);
                }
            }
            if let Some(place) = places.into_iter().next() {
                self.seed(file.clone(), fi, place, Some(line), dir);
                return Ok(());
            }
            // (An IPDL field's accessor's call: the field's value it reads.)
            if let Some(IpdlSlot::Accessor(_)) = self.ipdl_slot(&sym).await
                && let Some(&(c, ei)) = ipdl_reads(f, line, sym).first()
            {
                let name = format!("{}(…)", short_pretty(&f.calls[c].callee_pretty));
                let node = self.node(
                    &file,
                    fi,
                    NodeKey::Seed(name.clone()),
                    NodeKind::Seed,
                    line,
                    name.clone(),
                );
                self.queue.push_back(Item {
                    file: file.clone(),
                    func: fi,
                    flow: Flow::Result(c),
                    pos: Some(ei as isize),
                    node,
                    label: name,
                    kind: EdgeKind::Local,
                    stack: vec![],
                    hops: 0,
                    fresh: true,
                    root_ty: None,
                });
                return Ok(());
            }
            if is_local {
                return Err(format!("{} isn't in {}", sym, f.pretty));
            }
        }
        // (A field or global where it's declared: everywhere.)
        let name = self.symbol_name(&sym).await;
        let text = self
            .source(path)
            .await
            .and_then(|s| {
                s.lines()
                    .nth(line as usize - 1)
                    .map(|l| l.trim().to_string())
            })
            .unwrap_or_default();
        let id = self.graph.nodes.len();
        self.graph.nodes.push(TraceNode {
            id,
            kind: NodeKind::Seed,
            path: path.to_string(),
            line,
            func: name.clone(),
            func_sym: sym,
            lang: file.lang,
            label: name,
            text,
            detail: String::new(),
        });
        match dir {
            Dir::Forward => self.forward_field(&file, usize::MAX, sym, id, 0).await,
            Dir::Backward => self.backward_field(&file, usize::MAX, sym, id, 0).await,
        }
        Ok(())
    }

    /// A symbol's pretty name (from crossref), or a name from the symbol.
    async fn symbol_name(&mut self, sym: &Ustr) -> String {
        match self.xref(sym).await {
            Some(xref) => match &xref.meta {
                Some(meta) if !meta.pretty.is_empty() => meta.pretty.to_string(),
                _ => sym_short_name(sym),
            },
            None => sym_short_name(sym),
        }
    }

    /// Start tracing backwards from a function's returns.
    pub fn seed_returns(&mut self, file: Arc<FileIr>, func: usize) {
        self.graph.dir = Some(Dir::Backward);
        let f = &file.funcs[func];
        let returns: Vec<usize> = f
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.kind, EventKind::Return { .. }) && !e.in_lambda)
            .map(|(i, _)| i)
            .collect();
        for ei in returns {
            let node = self.event_node(&file, func, ei);
            let f = &file.funcs[func];
            let e = &f.events[ei];
            let EventKind::Return { vals } = &e.kind else {
                continue;
            };
            let pos = ei as isize;
            let vals = vals.clone();
            self.push_sources(&file, func, &vals, Some(pos), node, &[], 0, EdgeKind::Local);
        }
    }

    pub async fn run(&mut self, dir: Dir) {
        let mut processed = 0;
        while let Some(item) = self.queue.pop_front() {
            processed += 1;
            if processed > self.opts.max_items {
                self.note(format!(
                    "Stopped after {} steps ({} left)",
                    self.opts.max_items,
                    self.queue.len() + 1
                ));
                break;
            }
            let f = &item.file.funcs[item.func];
            let key = (
                item.file.path.clone(),
                f.sym,
                format!("{:?}@{:?}", item.flow, item.pos),
                dir == Dir::Forward,
            );
            if std::env::var("DATAFLOW_DEBUG").is_ok() {
                eprintln!(
                    "item {} {} {:?} pos={:?} hops={} stack={}",
                    item.file.path,
                    f.pretty,
                    item.flow,
                    item.pos,
                    item.hops,
                    item.stack.len()
                );
            }
            if !self.seen.insert(key) {
                // (Still connect what led here.)
                continue;
            }
            match dir {
                Dir::Forward => self.forward(item).await,
                Dir::Backward => self.backward(item).await,
            }
        }
    }

    // ## Resolving calls

    /// The functions a call can reach.
    async fn targets(&mut self, file: &Arc<FileIr>, func: usize, call: usize) -> Vec<Target> {
        let callee = file.funcs[func].calls[call].callee.clone();
        // (Several symbols are overloads per platform, or, in templates, per
        // instantiation: ex: `pref->GetValue(aKind, std::forward<T>(aResult))`
        // calls each of `GetValue`'s overloads.)
        let mut targets: Vec<Target> = vec![];
        for sym in callee.iter().take(self.opts.fanout) {
            for target in self.resolve(sym).await {
                if !targets
                    .iter()
                    .any(|t| Arc::ptr_eq(&t.file, &target.file) && t.func == target.func)
                {
                    targets.push(target);
                }
            }
        }
        // (Generated IPDL `Send` methods, when the generated code's records
        // aren't in the index: by the message's name.)
        if targets.is_empty()
            && let Some(owner) = ipc_message_sym(&file.funcs[func].calls[call].callee_pretty)
        {
            targets.extend(self.ipc_recvs(owner).await);
        }
        // (Calls an oracle says what they reach.)
        if targets.is_empty()
            && callee.is_empty()
            && let Some(answer) = self.oracle.as_ref().and_then(|o| {
                let c = &file.funcs[func].calls[call];
                o.site(&file.path, c.line).cloned()
            })
        {
            for pretty in &answer.targets {
                let Ok(matches) = self
                    .server
                    .search_identifiers(pretty, true, false, 50)
                    .await
                else {
                    continue;
                };
                let mut syms: Vec<Ustr> = matches.into_iter().map(|(_, sym)| sym).collect();
                syms.sort();
                syms.dedup();
                for sym in syms.into_iter().take(self.opts.fanout) {
                    for mut target in self.resolve(&sym).await {
                        target.via = Some(format!(
                            "{} (oracle)",
                            target.via.as_deref().unwrap_or("site")
                        ));
                        targets.push(target);
                    }
                }
            }
        }
        // (JS method calls on values scip-typescript couldn't type, ex:
        // untyped parameters, `Cc[...].createInstance(...)`'s: by name, if
        // only one XPCOM interface has a method of that name.)
        if callee.is_empty()
            && file.funcs[func].lang == Lang::Js
            && file.funcs[func].calls[call].receiver.is_some()
        {
            let name = file.funcs[func].calls[call].callee_pretty.clone();
            if let Some(sym) = self.xpcom_method_by_name(&name).await {
                for mut target in self.resolve(&sym).await {
                    target.via = Some(format!(
                        "{} (by name)",
                        target.via.as_deref().unwrap_or("xpidl")
                    ));
                    targets.push(target);
                }
            }
        }
        if targets.is_empty() {
            let pretty = file.funcs[func].calls[call].callee_pretty.clone();
            self.note(format!(
                "Couldn't see into {} (assumed its result comes from its arguments)",
                short_pretty(&pretty)
            ));
        }
        targets
    }

    /// The XPCOM interface method (its JS binding's symbol) with a name, if
    /// only one interface has one.
    pub async fn xpcom_method_by_name(&mut self, name: &str) -> Option<Ustr> {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let matches = self
            .server
            .search_identifiers(name, true, false, 200)
            .await
            .ok()?;
        let suffix = format!("#{}().", name);
        let mut syms: Vec<Ustr> = matches
            .into_iter()
            .map(|(_, sym)| sym)
            .filter(|sym| {
                sym.starts_with("S_js_typescript_generated/lib.gecko.xpcom.d.ts/")
                    && sym.ends_with(&suffix)
            })
            .collect();
        syms.sort();
        syms.dedup();
        match syms.as_slice() {
            [sym] => Some(*sym),
            _ => None,
        }
    }

    async fn defs_of(&mut self, sym: &Ustr, via: Option<String>, map: ArgMap) -> Vec<Target> {
        let mut out = vec![];
        let Some(xref) = self.xref(sym).await else {
            return out;
        };
        let defs: Vec<(String, u32)> = xref
            .definitions
            .iter()
            .flatten()
            .flat_map(|p| p.lines.iter().map(move |l| (p.path.to_string(), l.lineno)))
            .collect();
        for (path, line) in defs {
            let Some(file) = self.file(&path).await else {
                continue;
            };
            let func = file.by_sym.get(sym).copied().or_else(|| {
                file.funcs
                    .iter()
                    .position(|f| f.line <= line && line <= f.end_line && f.line + 3 >= line)
            });
            if let Some(func) = func {
                out.push(Target {
                    file,
                    func,
                    map: map.clone(),
                    via: via.clone(),
                });
            }
        }
        out
    }

    async fn resolve(&mut self, sym: &Ustr) -> Vec<Target> {
        // An IPDL message's generated `Send` method: the actor's `Recv`
        // methods, which IPC calls with the same arguments, rather than the
        // generated method's body (which serializes them, ex: with
        // `IPC::WriteParam`).
        if let Some(xref) = self.xref(sym).await
            && let Some(meta) = &xref.meta
            && let Some(owner) = &meta.slot_owner
            && owner.props.slot_kind == crate::file_format::analysis::BindingSlotKind::Send
        {
            let recvs = self.ipc_recvs(owner.sym).await;
            if !recvs.is_empty() {
                return recvs;
            }
        }
        let mut out = self.defs_of(sym, None, ArgMap::Direct).await;
        // (Interfaces' and abstract classes' declarations aren't what's
        // called: their overrides are.)
        out.retain(|t| t.file.funcs[t.func].has_body);
        if !out.is_empty() {
            return out;
        }
        let Some(xref) = self.xref(sym).await else {
            return out;
        };
        let Some(meta) = &xref.meta else {
            return out;
        };
        // An IPDL message's generated `Send` method: the actor's `Recv`
        // method, which IPC calls with the same arguments.
        let ipc_owner = match &meta.slot_owner {
            Some(owner)
                if owner.props.slot_kind == crate::file_format::analysis::BindingSlotKind::Send =>
            {
                Some(owner.sym)
            }
            _ => ipc_message_sym(&meta.pretty),
        };
        if let Some(owner) = ipc_owner {
            out.extend(self.ipc_recvs(owner).await);
            if !out.is_empty() {
                return out;
            }
        }
        // A virtual method without a definition of its own (ex: an XPCOM
        // interface's): its overrides'.
        let overridden_by: Vec<Ustr> = meta.overridden_by_syms.clone();
        // A binding slot (ex: XPIDL's JS method): the C++ slots' overrides,
        // with XPIDL's argument mapping.
        let slot_owner = meta.slot_owner.clone();
        if !overridden_by.is_empty() {
            if overridden_by.len() > self.opts.fanout {
                self.note(format!(
                    "{} has {} overrides; followed {}",
                    short_pretty(&meta.pretty),
                    overridden_by.len(),
                    self.opts.fanout
                ));
            }
            for o in overridden_by.iter().take(self.opts.fanout) {
                out.extend(
                    self.defs_of(o, Some("override".to_string()), ArgMap::Direct)
                        .await,
                );
            }
        }
        if out.is_empty()
            && let Some(owner) = slot_owner
            && owner.props.slot_lang != crate::file_format::analysis::BindingSlotLang::Cpp
        {
            let Some(owner_xref) = self.xref(&owner.sym).await else {
                return out;
            };
            let Some(owner_meta) = &owner_xref.meta else {
                return out;
            };
            let cpp_slots: Vec<Ustr> = owner_meta
                .binding_slots
                .iter()
                .filter(|s| s.props.slot_lang == crate::file_format::analysis::BindingSlotLang::Cpp)
                .map(|s| s.sym)
                .collect();
            for cpp in cpp_slots {
                let Some(cpp_xref) = self.xref(&cpp).await else {
                    continue;
                };
                let Some(cpp_meta) = &cpp_xref.meta else {
                    continue;
                };
                let arg_names: Vec<String> = cpp_meta
                    .extra
                    .get("args")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|arg| {
                                arg.get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let map = ArgMap::Xpidl {
                    argc: arg_names.iter().position(|n| n == "_argc"),
                    retval: arg_names.iter().position(|n| n == "_retval"),
                };
                let impls: Vec<Ustr> = cpp_meta.overridden_by_syms.clone();
                let via = Some(format!(
                    "xpidl {}",
                    owner.sym.strip_prefix("XPIDL_").unwrap_or(&owner.sym)
                ));
                for i in impls.iter().take(self.opts.fanout) {
                    out.extend(self.defs_of(i, via.clone(), map.clone()).await);
                }
                if impls.is_empty() {
                    out.extend(self.defs_of(&cpp, via.clone(), map.clone()).await);
                }
            }
        }
        out
    }

    /// The `Recv` methods of an IPDL message.
    async fn ipc_recvs(&mut self, owner: Ustr) -> Vec<Target> {
        let mut out = vec![];
        let Some(owner_xref) = self.xref(&owner).await else {
            return out;
        };
        let Some(owner_meta) = &owner_xref.meta else {
            return out;
        };
        let recvs: Vec<Ustr> = owner_meta
            .binding_slots
            .iter()
            .filter(|s| s.props.slot_kind == crate::file_format::analysis::BindingSlotKind::Recv)
            .map(|s| s.sym)
            .collect();
        for recv in recvs {
            let via = Some(format!(
                "ipc {}",
                owner.strip_prefix("IPC_").unwrap_or(&owner)
            ));
            let mut defs = self.defs_of(&recv, via.clone(), ArgMap::Direct).await;
            defs.retain(|t| t.file.funcs[t.func].has_body);
            // (A generated actor base class's declaration: its overrides,
            // the actor's.)
            if defs.is_empty()
                && let Some(xref) = self.xref(&recv).await
                && let Some(meta) = &xref.meta
            {
                let overridden_by: Vec<Ustr> = meta.overridden_by_syms.clone();
                for o in overridden_by.iter().take(self.opts.fanout) {
                    defs.extend(self.defs_of(o, via.clone(), ArgMap::Direct).await);
                }
            }
            out.extend(defs);
        }
        out
    }

    /// The calls of a function (and of what it overrides, and of the binding
    /// slots it implements), as sites to return to.
    async fn callers(&mut self, file: &Arc<FileIr>, func: usize) -> Vec<Site> {
        let f = &file.funcs[func];
        let sym = f.sym;
        let mut syms: Vec<(Ustr, ArgMap)> = vec![(sym, ArgMap::Direct)];
        if let Some(xref) = self.xref(&sym).await
            && let Some(meta) = &xref.meta
        {
            for o in &meta.overrides {
                syms.push((o.sym, ArgMap::Direct));
            }
        }
        // (An IPDL message's `Recv` method (or an override of one): the
        // message's `Send` methods' callers, which IPC passes the same
        // arguments, rather than the generated code that calls it (which
        // deserializes them, ex: with `IPC::ReadParam`).)
        let mut sends: Vec<(Ustr, ArgMap)> = vec![];
        for (s, _) in syms.clone() {
            let Some(xref) = self.xref(&s).await else {
                continue;
            };
            let Some(owner) = xref.meta.as_ref().and_then(|m| m.slot_owner.clone()) else {
                continue;
            };
            if owner.props.slot_kind != crate::file_format::analysis::BindingSlotKind::Recv {
                continue;
            }
            let Some(owner_xref) = self.xref(&owner.sym).await else {
                continue;
            };
            for slot in owner_xref.meta.iter().flat_map(|m| &m.binding_slots) {
                if slot.props.slot_kind == crate::file_format::analysis::BindingSlotKind::Send
                    && !sends.iter().any(|(x, _)| *x == slot.sym)
                {
                    sends.push((slot.sym, ArgMap::Direct));
                }
            }
        }
        if !sends.is_empty() {
            syms = sends;
        }
        // (A C++ implementation of an XPIDL method: its JS callers.)
        let mut i = 0;
        while i < syms.len() {
            let (s, _) = syms[i].clone();
            i += 1;
            if s == sym {
                continue;
            }
            let Some(xref) = self.xref(&s).await else {
                continue;
            };
            let Some(meta) = &xref.meta else {
                continue;
            };
            let Some(owner) = meta.slot_owner.clone() else {
                continue;
            };
            if owner.props.slot_lang != crate::file_format::analysis::BindingSlotLang::Cpp {
                continue;
            }
            let arg_names: Vec<String> = meta
                .extra
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .map(|arg| {
                            arg.get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let map = ArgMap::Xpidl {
                argc: arg_names.iter().position(|n| n == "_argc"),
                retval: arg_names.iter().position(|n| n == "_retval"),
            };
            if let Some(owner_xref) = self.xref(&owner.sym).await
                && let Some(owner_meta) = &owner_xref.meta
            {
                for slot in &owner_meta.binding_slots {
                    if slot.props.slot_lang == crate::file_format::analysis::BindingSlotLang::JS
                        && !syms.iter().any(|(s, _)| *s == slot.sym)
                    {
                        syms.push((slot.sym, map.clone()));
                    }
                }
            }
        }
        let mut sites = vec![];
        let mut total = 0;
        for (s, map) in syms {
            let Some(xref) = self.xref(&s).await else {
                continue;
            };
            let uses: Vec<(String, u32, Ustr)> = xref
                .uses
                .iter()
                .flatten()
                .flat_map(|p| {
                    p.lines
                        .iter()
                        .map(move |l| (p.path.to_string(), l.lineno, l.contextsym))
                })
                .collect();
            for (path, line, contextsym) in uses {
                total += 1;
                if sites.len() >= self.opts.fanout {
                    continue;
                }
                let Some(caller_file) = self.file(&path).await else {
                    continue;
                };
                let caller = caller_file.by_sym.get(&contextsym).copied().or_else(|| {
                    caller_file
                        .func_at_line(line)
                        .and_then(|cf| caller_file.by_sym.get(&cf.sym).copied())
                });
                let Some(caller) = caller else {
                    continue;
                };
                let cf = &caller_file.funcs[caller];
                for (ci, call) in cf.calls.iter().enumerate() {
                    if call.line <= line && line <= call.end_line && call.callee.contains(&s) {
                        sites.push(Site {
                            file: caller_file.clone(),
                            func: caller,
                            call: ci,
                            map: map.clone(),
                            entered: None,
                        });
                    }
                }
            }
        }
        if total > self.opts.fanout {
            self.note(format!(
                "{} has {} callers; followed {}",
                short_pretty(&f.pretty),
                total,
                self.opts.fanout
            ));
        }
        sites
    }

    // ## Moving between caller and callee

    /// The callee parameter an argument is.
    fn param_for_arg(map: &ArgMap, arg: usize) -> Option<usize> {
        match map {
            ArgMap::Direct => Some(arg),
            ArgMap::Xpidl { argc, retval } => {
                let first_extra = [*argc, *retval].into_iter().flatten().min();
                match first_extra {
                    Some(extra) if arg >= extra => None,
                    _ => Some(arg),
                }
            }
        }
    }

    /// The caller's argument a callee parameter is, or None for XPIDL's
    /// `_argc` and `_retval`.
    fn arg_for_param(map: &ArgMap, param: usize) -> Option<usize> {
        match map {
            ArgMap::Direct => Some(param),
            ArgMap::Xpidl { argc, retval } => {
                if Some(param) == *argc || Some(param) == *retval {
                    None
                } else {
                    Some(param)
                }
            }
        }
    }

    /// The callee's place for a caller's place `p` passed as `arg` (whose
    /// value the callee's `param` is).
    fn into_callee(arg: &Arg, p: &Place, param: &Place) -> Option<Place> {
        if let Some(ap) = &arg.place {
            if p.within(ap) {
                let rest = &p.path[ap.path.len()..];
                let mut place = param.clone();
                if arg.addr {
                    place.path.push(Step::Deref);
                }
                place.path.extend(rest.iter().cloned());
                return Some(place);
            }
            if ap.within(p) {
                // (A part of `p` is passed.)
                return Some(param.clone());
            }
        }
        if reads(&arg.vals, &Flow::Place(p.clone())) {
            return Some(param.clone());
        }
        None
    }

    /// The caller's place for a callee's place `q` in `param`, passed as
    /// `arg`: where the callee writing `q` writes in the caller, if it does.
    fn into_caller(arg: &Arg, q: &Place, param: &Param, lang: Lang) -> Option<Place> {
        let ap = arg.place.as_ref()?;
        let rest = &q.path;
        if arg.addr {
            // (`&x` passed: `*param` is `x`.)
            if rest.first() == Some(&Step::Deref) {
                let mut place = ap.clone();
                place.path.extend(rest[1..].iter().cloned());
                return Some(place);
            }
            return None;
        }
        let escapes = match lang {
            Lang::Cpp => rest.first() == Some(&Step::Deref) || param.is_ref(),
            // (Objects are references: writing their members writes the
            // caller's.)
            _ => !rest.is_empty(),
        };
        if !escapes {
            return None;
        }
        let mut place = ap.clone();
        place.path.extend(rest.iter().cloned());
        Some(place)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        flow: Flow,
        pos: Option<isize>,
        node: usize,
        stack: Vec<Site>,
        hops: usize,
        kind: EdgeKind,
    ) -> bool {
        if hops > self.opts.max_hops {
            self.note(format!("Stopped at {} hops", self.opts.max_hops));
            return false;
        }
        let f = &file.funcs[func];
        let label = match &flow {
            Flow::Place(p) => f.place_name(p),
            Flow::Result(c) => format!("{}(…)", short_pretty(&f.calls[*c].callee_pretty)),
        };
        self.queue.push_back(Item {
            file: file.clone(),
            func,
            flow,
            pos,
            node,
            label,
            kind,
            stack,
            hops,
            fresh: true,
            root_ty: None,
        });
        true
    }

    /// Push a place a value is in on entering a function (see
    /// `Item::fresh`).
    #[allow(clippy::too_many_arguments)]
    fn push_entered(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        place: Place,
        pos: Option<isize>,
        node: usize,
        stack: Vec<Site>,
        hops: usize,
        kind: EdgeKind,
    ) {
        self.push_entered_typed(file, func, place, pos, node, stack, hops, kind, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn push_entered_typed(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        place: Place,
        pos: Option<isize>,
        node: usize,
        stack: Vec<Site>,
        hops: usize,
        kind: EdgeKind,
        root_ty: Option<String>,
    ) {
        if self.push(file, func, Flow::Place(place), pos, node, stack, hops, kind)
            && let Some(item) = self.queue.back_mut()
        {
            item.fresh = false;
            item.root_ty = root_ty;
        }
    }

    /// The type of an argument's value, if we know it: its place's root's
    /// type (or `root_ty` for `root`), through its path.
    fn arg_type(f: &FuncIr, arg: &Arg, root: Option<&(Root, String)>) -> Option<String> {
        let ap = arg.place.as_ref()?;
        let root_type = match (&ap.root, root) {
            (r, Some((rr, ty))) if r == rr => ty.clone(),
            (Root::Local(sym), _) => f.types.get(sym)?.clone(),
            _ => return None,
        };
        if is_dependent_type(&root_type) {
            return None;
        }
        let ty = type_of_path(&root_type, &ap.path)?;
        Some(if arg.addr { format!("{} *", ty) } else { ty })
    }

    /// Of several functions a call could reach (overloads, or a template's
    /// call's per instantiation), those whose parameters' types match the
    /// arguments' (that we know), if any do.
    fn prune_overloads(
        f: &FuncIr,
        call: &Call,
        args: &[usize],
        targets: Vec<Target>,
        root: Option<&(Root, String)>,
    ) -> Vec<Target> {
        if targets.len() < 2 {
            return targets;
        }
        let arg_types: Vec<(usize, String)> = args
            .iter()
            .filter_map(|&ai| {
                Some((
                    ai,
                    normalize_type(&Self::arg_type(f, call.args.get(ai)?, root)?),
                ))
            })
            .collect();
        if arg_types.is_empty() {
            return targets;
        }
        let matching: Vec<Target> = targets
            .iter()
            .filter(|t| {
                let tf = &t.file.funcs[t.func];
                arg_types.iter().all(|(ai, ty)| {
                    let Some(pi) = Self::param_for_arg(&t.map, *ai) else {
                        return true;
                    };
                    match tf.params.get(pi).and_then(|p| p.type_pretty.as_deref()) {
                        Some(pty) if !is_dependent_type(pty) => normalize_type(pty) == *ty,
                        _ => true,
                    }
                })
            })
            .cloned()
            .collect();
        if matching.is_empty() {
            targets
        } else {
            matching
        }
    }

    /// Push the sources of values (backwards).
    #[allow(clippy::too_many_arguments)]
    fn push_sources(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        vals: &[Val],
        pos: Option<isize>,
        node: usize,
        stack: &[Site],
        hops: usize,
        kind: EdgeKind,
    ) {
        for val in vals {
            match &val.atom {
                Atom::Read(place) | Atom::Addr(place) => {
                    self.push(
                        file,
                        func,
                        Flow::Place(place.clone()),
                        pos,
                        node,
                        stack.to_vec(),
                        hops,
                        kind,
                    );
                }
                Atom::Call(c) => {
                    self.push(
                        file,
                        func,
                        Flow::Result(*c),
                        pos,
                        node,
                        stack.to_vec(),
                        hops,
                        kind,
                    );
                }
                Atom::Const(text) => {
                    let f = &file.funcs[func];
                    let line = pos
                        .and_then(|i| f.events.get(i.max(0) as usize))
                        .map_or(f.line, |e| e.line);
                    let c = self.node(
                        file,
                        func,
                        NodeKey::Const(text.clone(), line),
                        NodeKind::Const,
                        line,
                        text.clone(),
                    );
                    self.edge(c, node, EdgeKind::Local, text.clone());
                }
            }
        }
    }

    // ## Forwards

    async fn forward(&mut self, item: Item) {
        let Item {
            file,
            func,
            flow,
            pos,
            node,
            label,
            kind,
            stack,
            hops,
            fresh,
            root_ty,
        } = item;
        let root_typed: Option<(Root, String)> = match (&flow, root_ty) {
            (Flow::Place(p), Some(ty)) => Some((p.root.clone(), ty)),
            _ => None,
        };
        let lang = file.funcs[func].lang;
        // (Values put in out-parameters, `this`'s members and globals leave
        // the function.)
        if let Flow::Place(p) = &flow
            && fresh
        {
            self.forward_escapes(&file, func, p, node, &stack, hops)
                .await;
            // (Put in an IPDL field, through its mutable accessor: to its
            // reads.)
            if let Some(field) = self.ipdl_field_of_place(p).await {
                self.forward_ipdl_field(field, node, hops).await;
            }
        }
        let n_events = file.funcs[func].events.len();
        for ei in 0..n_events {
            let f = &file.funcs[func];
            let e = &f.events[ei];
            if self.opts.ordered
                && let Some(pos) = pos
                && (ei as isize) <= pos
            {
                continue;
            }
            let epos = Some(ei as isize);
            match e.kind.clone() {
                EventKind::Assign { place, vals, .. } => {
                    if !reads(&vals, &flow) {
                        continue;
                    }
                    let n = self.event_node(&file, func, ei);
                    self.edge(node, n, kind, label.clone());
                    // (Copying the whole of something the value is part of:
                    // the value's part of the copy, as backwards in
                    // `backward_place`.)
                    let place = match (&flow, vals.as_slice()) {
                        (
                            Flow::Place(p),
                            [
                                Val {
                                    atom: Atom::Read(r),
                                    derived: false,
                                },
                            ],
                        ) if p.within(r) && p.path.len() > r.path.len() => {
                            let mut part = place;
                            part.path.extend(p.path[r.path.len()..].iter().cloned());
                            part
                        }
                        _ => place,
                    };
                    if Flow::Place(place.clone()) != flow {
                        self.push(
                            &file,
                            func,
                            Flow::Place(place),
                            epos,
                            n,
                            stack.clone(),
                            hops,
                            EdgeKind::Local,
                        );
                    }
                }
                EventKind::Call(c) => {
                    let call = f.calls[c].clone();
                    let arg_hits: Vec<usize> = call
                        .args
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| reads(&a.vals, &flow) || arg_place_hit(a, &flow))
                        .map(|(i, _)| i)
                        .collect();
                    let receiver_hit = call
                        .receiver
                        .as_ref()
                        .is_some_and(|r| reads(&r.vals, &flow) || arg_place_hit(r, &flow));
                    if arg_hits.is_empty() && !receiver_hit {
                        continue;
                    }
                    // (Through IPDL fields rather than the generated code.)
                    match self.call_ipdl_slot(&call).await {
                        Some(IpdlSlot::Constructor(fields)) => {
                            let hit: Vec<Ustr> = arg_hits
                                .iter()
                                .filter_map(|&ai| fields.get(ai).copied())
                                .collect();
                            if !hit.is_empty() {
                                let n = self.event_node(&file, func, ei);
                                self.edge(node, n, kind, label.clone());
                                for field in hit {
                                    self.forward_ipdl_field(field, n, hops).await;
                                }
                            }
                            continue;
                        }
                        Some(IpdlSlot::Accessor(field)) => {
                            // (The field of the value, read.)
                            if receiver_hit
                                && !is_assigned(&file.funcs[func], c)
                                && self.reads_ipdl_field(&call, &flow, field).await
                            {
                                let n = self.event_node(&file, func, ei);
                                self.edge(node, n, kind, label.clone());
                                self.push(
                                    &file,
                                    func,
                                    Flow::Result(c),
                                    epos,
                                    n,
                                    stack.clone(),
                                    hops,
                                    EdgeKind::Local,
                                );
                            }
                            continue;
                        }
                        None => {}
                    }
                    let targets = self.targets(&file, func, c).await;
                    let targets = Self::prune_overloads(
                        &file.funcs[func],
                        &call,
                        &arg_hits,
                        targets,
                        root_typed.as_ref(),
                    );
                    if targets.is_empty()
                        && let Some(summary) = self
                            .oracle
                            .as_ref()
                            .and_then(|o| o.summary(&call.callee).cloned())
                    {
                        // (Where the oracle says the value goes.)
                        let n = self.event_node(&file, func, ei);
                        self.edge(node, n, kind, label.clone());
                        for flow in &summary.flows {
                            let hit = match parse_slot(&flow.from) {
                                Some(Slot::This) => receiver_hit,
                                Some(slot) => arg_hits.iter().any(|&i| slot.is_arg(i)),
                                None => false,
                            };
                            if !hit {
                                continue;
                            }
                            let to = match parse_slot(&flow.to) {
                                Some(Slot::Return) => Some(Flow::Result(c)),
                                Some(Slot::ArgPointee(i)) => call.args.get(i).and_then(|a| {
                                    let ap = a.place.clone()?;
                                    Some(Flow::Place(if a.addr {
                                        ap
                                    } else {
                                        ap.with(Step::Deref)
                                    }))
                                }),
                                Some(Slot::This) => call
                                    .receiver
                                    .as_ref()
                                    .and_then(|r| r.place.clone())
                                    .map(Flow::Place),
                                _ => None,
                            };
                            if let Some(to) = to {
                                let o = self.node(
                                    &file,
                                    func,
                                    NodeKey::Opaque(c),
                                    NodeKind::Opaque,
                                    e_line(&file, func, ei),
                                    format!("{}(…) per oracle", short_pretty(&call.callee_pretty)),
                                );
                                self.edge(
                                    n,
                                    o,
                                    EdgeKind::Oracle,
                                    format!("{} → {}", flow.from, flow.to),
                                );
                                self.push(
                                    &file,
                                    func,
                                    to,
                                    epos,
                                    o,
                                    stack.clone(),
                                    hops,
                                    EdgeKind::Local,
                                );
                            }
                        }
                        continue;
                    }
                    if targets.is_empty() {
                        // (Assume the result comes from the arguments.)
                        let n = self.event_node(&file, func, ei);
                        self.edge(node, n, kind, label.clone());
                        let o = self.node(
                            &file,
                            func,
                            NodeKey::Opaque(c),
                            NodeKind::Opaque,
                            e_line(&file, func, ei),
                            format!("{}(…) result (assumed)", short_pretty(&call.callee_pretty)),
                        );
                        self.edge(n, o, EdgeKind::Assumed, "?".to_string());
                        self.push(
                            &file,
                            func,
                            Flow::Result(c),
                            epos,
                            o,
                            stack.clone(),
                            hops,
                            EdgeKind::Local,
                        );
                        continue;
                    }
                    // (Where the value enters each callee: its parameter (or
                    // `this`), the callee's place, and the edge's label.)
                    let mut entries: Vec<(Target, Option<usize>, Place, String, EdgeKind)> = vec![];
                    for target in &targets {
                        let tf = &target.file.funcs[target.func];
                        let edge_kind = if target
                            .via
                            .as_deref()
                            .is_some_and(|v| v.starts_with("xpidl") || v.starts_with("ipc"))
                        {
                            EdgeKind::Binding
                        } else {
                            EdgeKind::Call
                        };
                        for &ai in &arg_hits {
                            let Some(pi) = Self::param_for_arg(&target.map, ai) else {
                                continue;
                            };
                            let Some(param) = tf.params.get(pi) else {
                                continue;
                            };
                            let Flow::Place(p) = &flow else {
                                // (A call result passed straight in.)
                                entries.push((
                                    target.clone(),
                                    Some(pi),
                                    param.place.clone(),
                                    format!("arg {}", ai),
                                    edge_kind,
                                ));
                                continue;
                            };
                            let Some(cp) = Self::into_callee(&call.args[ai], p, &param.place)
                            else {
                                continue;
                            };
                            // (A part of an object passed: only if the callee
                            // reads it.)
                            if cp.path.len() > usize::from(call.args[ai].addr)
                                && !reads_place(tf, &cp)
                            {
                                continue;
                            }
                            let label = tf.place_name(&cp);
                            entries.push((target.clone(), Some(pi), cp, label, edge_kind));
                        }
                        if receiver_hit
                            && let (Some(receiver), Flow::Place(p)) = (&call.receiver, &flow)
                            && let Some(cp) = Self::into_callee(receiver, p, &Place::this())
                            && (cp.path.is_empty() || reads_place(tf, &cp))
                        {
                            let label = format!("this ({})", tf.place_name(&cp));
                            entries.push((target.clone(), None, cp, label, edge_kind));
                        }
                    }
                    if entries.is_empty() {
                        continue;
                    }
                    let n = self.event_node(&file, func, ei);
                    self.edge(node, n, kind, label.clone());
                    for (target, pi, cp, edge_label, edge_kind) in entries {
                        let tf = &target.file.funcs[target.func];
                        // (A template's parameter: the argument's type.)
                        let carried = pi.and_then(|pi| {
                            let pty = tf.params.get(pi)?.type_pretty.as_deref()?;
                            if !is_dependent_type(pty) {
                                return None;
                            }
                            let ai = (0..call.args.len())
                                .find(|&ai| Self::param_for_arg(&target.map, ai) == Some(pi))?;
                            Self::arg_type(&file.funcs[func], &call.args[ai], root_typed.as_ref())
                        });
                        let mut new_stack = stack.clone();
                        new_stack.push(Site {
                            file: file.clone(),
                            func,
                            call: c,
                            map: target.map.clone(),
                            entered: None,
                        });
                        let pn = match pi {
                            Some(pi) => {
                                let name = tf.params[pi].name.clone();
                                self.node(
                                    &target.file,
                                    target.func,
                                    NodeKey::Param(pi),
                                    NodeKind::Param,
                                    tf.line,
                                    format!("param {}", name),
                                )
                            }
                            None => self.node(
                                &target.file,
                                target.func,
                                NodeKey::Seed("this".to_string()),
                                NodeKind::Param,
                                tf.line,
                                "this".to_string(),
                            ),
                        };
                        self.edge(n, pn, edge_kind, edge_label);
                        self.push_entered_typed(
                            &target.file,
                            target.func,
                            cp,
                            None,
                            pn,
                            new_stack,
                            hops + 1,
                            EdgeKind::Local,
                            carried,
                        );
                    }
                }
                EventKind::Return { vals } => {
                    if e.in_lambda || !reads(&vals, &flow) {
                        continue;
                    }
                    let n = self.event_node(&file, func, ei);
                    self.edge(node, n, kind, label.clone());
                    self.forward_return(&file, func, n, &stack, hops).await;
                }
                EventKind::Test { vals } => {
                    if reads(&vals, &flow) {
                        let n = self.event_node(&file, func, ei);
                        self.edge(node, n, kind, label.clone());
                    }
                }
            }
        }
        let _ = lang;
    }

    /// A function's return: to the call we came from, or to all its calls.
    async fn forward_return(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        n: usize,
        stack: &[Site],
        hops: usize,
    ) {
        let sites = match stack.last() {
            Some(site) => vec![(site.clone(), stack[..stack.len() - 1].to_vec(), hops)],
            None => self
                .callers(file, func)
                .await
                .into_iter()
                .map(|s| (s, vec![], hops + 1))
                .collect(),
        };
        for (site, rest, hops) in sites {
            // (XPIDL's C++ returns an nsresult, which isn't JS's value; that's
            // `_retval`'s.)
            if matches!(site.map, ArgMap::Xpidl { .. }) {
                continue;
            }
            let call_event = event_of_call(&site.file, site.func, site.call);
            let cn = match call_event {
                Some(ei) => self.event_node(&site.file, site.func, ei),
                None => continue,
            };
            self.edge(n, cn, EdgeKind::Return, "result".to_string());
            let _call = &site.file.funcs[site.func].calls[site.call];
            let pos = event_of_call(&site.file, site.func, site.call).map(|i| i as isize);
            self.push(
                &site.file,
                site.func,
                Flow::Result(site.call),
                pos,
                cn,
                rest,
                hops,
                EdgeKind::Local,
            );
        }
    }

    /// Values in places that outlive the function: out-parameters, `this`'s
    /// members, globals.
    async fn forward_escapes(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        p: &Place,
        node: usize,
        stack: &[Site],
        hops: usize,
    ) {
        let f = &file.funcs[func];
        let lang = f.lang;
        if let Some(pi) = f.param_index(p) {
            let param = f.params[pi].clone();
            let sites = match stack.last() {
                Some(site) => vec![(site.clone(), stack[..stack.len() - 1].to_vec(), hops)],
                None => {
                    // (Only out-parameters' values go to all callers.)
                    let escapes = match lang {
                        Lang::Cpp => {
                            p.path.first() == Some(&Step::Deref)
                                || (param.is_ref() && param.is_out())
                        }
                        _ => false,
                    };
                    if !escapes {
                        return;
                    }
                    self.callers(file, func)
                        .await
                        .into_iter()
                        .map(|s| (s, vec![], hops + 1))
                        .collect()
                }
            };
            for (site, rest, hops) in sites {
                let call = site.file.funcs[site.func].calls[site.call].clone();
                let Some(cn) = event_of_call(&site.file, site.func, site.call)
                    .map(|ei| self.event_node(&site.file, site.func, ei))
                else {
                    continue;
                };
                let pos = event_of_call(&site.file, site.func, site.call).map(|i| i as isize);
                match Self::arg_for_param(&site.map, pi) {
                    Some(ai) => {
                        let Some(arg) = call.args.get(ai) else {
                            continue;
                        };
                        let Some(cp) = Self::into_caller(arg, p, &param, lang) else {
                            continue;
                        };
                        let cf = &site.file.funcs[site.func];
                        self.edge(
                            node,
                            cn,
                            EdgeKind::Return,
                            format!("out {}", cf.place_name(&cp)),
                        );
                        self.push(
                            &site.file,
                            site.func,
                            Flow::Place(cp),
                            pos,
                            cn,
                            rest,
                            hops,
                            EdgeKind::Local,
                        );
                    }
                    None => {
                        // (XPIDL's `*_retval` is the JS call's value.)
                        if let ArgMap::Xpidl {
                            retval: Some(r), ..
                        } = site.map
                            && r == pi
                            && p.path.first() == Some(&Step::Deref)
                        {
                            self.edge(node, cn, EdgeKind::Binding, "retval".to_string());
                            self.push(
                                &site.file,
                                site.func,
                                Flow::Result(site.call),
                                pos,
                                cn,
                                rest,
                                hops,
                                EdgeKind::Local,
                            );
                        }
                    }
                }
            }
            return;
        }
        match &p.root {
            Root::This if !p.path.is_empty() => {
                // (Back to the receiver at the call we came from.)
                if let Some(site) = stack.last() {
                    let call = site.file.funcs[site.func].calls[site.call].clone();
                    if let Some(receiver) = &call.receiver
                        && let Some(rp) = &receiver.place
                        && let Some(cn) = event_of_call(&site.file, site.func, site.call)
                            .map(|ei| self.event_node(&site.file, site.func, ei))
                    {
                        let mut cp = rp.clone();
                        cp.path.extend(p.path.iter().cloned());
                        let cf = &site.file.funcs[site.func];
                        self.edge(
                            node,
                            cn,
                            EdgeKind::Return,
                            format!("out {}", cf.place_name(&cp)),
                        );
                        let pos =
                            event_of_call(&site.file, site.func, site.call).map(|i| i as isize);
                        self.push(
                            &site.file,
                            site.func,
                            Flow::Place(cp),
                            pos,
                            cn,
                            stack[..stack.len() - 1].to_vec(),
                            hops,
                            EdgeKind::Local,
                        );
                    }
                }
                if let Some(field) = p.last_field() {
                    self.forward_field(file, func, field, node, hops).await;
                }
            }
            Root::Global(sym) => {
                let sym = *sym;
                self.forward_field(file, func, sym, node, hops).await;
            }
            _ => {}
        }
    }

    /// The reads of a field (or global) in other functions.
    async fn forward_field(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        field: Ustr,
        node: usize,
        hops: usize,
    ) {
        if field.starts_with("name:") {
            return;
        }
        let uses = self.field_uses(field).await;
        let total = uses.len();
        let mut followed = 0;
        for (path, line, contextsym) in uses {
            let Some(use_file) = self.file(&path).await else {
                continue;
            };
            let Some(uf) = use_file.by_sym.get(&contextsym).copied() else {
                continue;
            };
            if Arc::ptr_eq(&use_file, file) && uf == func {
                continue;
            }
            // (The places read at that line with the field.)
            let places = places_with_field(&use_file.funcs[uf], line, field, false);
            if places.is_empty() {
                continue;
            }
            followed += 1;
            if followed > self.opts.fanout {
                break;
            }
            for place in places {
                let before = use_file.funcs[uf]
                    .events
                    .iter()
                    .position(|e| e.end_line >= line)
                    .unwrap_or(0) as isize
                    - 1;
                self.push_entered(
                    &use_file,
                    uf,
                    place,
                    Some(before),
                    node,
                    vec![],
                    hops + 1,
                    EdgeKind::Field,
                );
            }
        }
        if followed > self.opts.fanout {
            let pretty = self.names_of(file, func, field);
            self.note(format!(
                "{} is read in {} places; followed {}",
                pretty, total, self.opts.fanout
            ));
        }
    }

    fn names_of(&self, file: &FileIr, func: usize, sym: Ustr) -> String {
        file.funcs
            .get(func)
            .and_then(|f| f.names.get(&sym))
            .cloned()
            .unwrap_or_else(|| sym_short_name(&sym))
    }

    async fn field_uses(&mut self, field: Ustr) -> Vec<(String, u32, Ustr)> {
        let Some(xref) = self.xref(&field).await else {
            return vec![];
        };
        xref.uses
            .iter()
            .flatten()
            .chain(xref.assignments.iter().flatten())
            .flat_map(|p| {
                p.lines
                    .iter()
                    .map(move |l| (p.path.to_string(), l.lineno, l.contextsym))
            })
            .collect()
    }

    // ## Backwards

    async fn backward(&mut self, item: Item) {
        let Item {
            file,
            func,
            flow,
            pos,
            node,
            label,
            kind,
            stack,
            hops,
            root_ty,
            ..
        } = item;
        match flow {
            Flow::Result(c) => {
                self.backward_result(&file, func, c, node, &label, kind, &stack, hops)
                    .await
            }
            Flow::Place(p) => {
                let root_typed = root_ty.map(|ty| (p.root.clone(), ty));
                self.backward_place(
                    &file, func, p, pos, node, &label, kind, &stack, hops, root_typed,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn backward_result(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        c: usize,
        node: usize,
        label: &str,
        kind: EdgeKind,
        stack: &[Site],
        hops: usize,
    ) {
        let Some(ei) = event_of_call(file, func, c) else {
            return;
        };
        let n = self.event_node(file, func, ei);
        self.edge(n, node, kind, label.to_string());
        let call = file.funcs[func].calls[c].clone();
        let epos = Some(ei as isize);
        // (Through IPDL fields rather than the generated code: a field's
        // value comes from its writes, and a constructed struct's from its
        // fields' arguments.)
        match self.call_ipdl_slot(&call).await {
            Some(IpdlSlot::Accessor(field)) => {
                self.backward_ipdl_field(field, n, hops).await;
                return;
            }
            Some(IpdlSlot::Constructor(_)) => {
                for arg in &call.args {
                    self.push_sources(file, func, &arg.vals, epos, n, stack, hops, EdgeKind::Local);
                }
                return;
            }
            None => {}
        }
        let targets = self.targets(file, func, c).await;
        if targets.is_empty()
            && let Some(summary) = self
                .oracle
                .as_ref()
                .and_then(|o| o.summary(&call.callee).cloned())
        {
            let o = self.node(
                file,
                func,
                NodeKey::Opaque(c),
                NodeKind::Opaque,
                call.line,
                format!("{}(…) per oracle", short_pretty(&call.callee_pretty)),
            );
            self.edge(o, n, EdgeKind::Oracle, "return".to_string());
            for flow in summary.flows.iter().filter(|f| f.to == "return") {
                let vals: Vec<Val> = match parse_slot(&flow.from) {
                    Some(Slot::This) => call.receiver.iter().flat_map(|r| r.vals.clone()).collect(),
                    Some(Slot::Arg(i)) => {
                        call.args.get(i).map(|a| a.vals.clone()).unwrap_or_default()
                    }
                    Some(Slot::ArgsFrom(n)) => call
                        .args
                        .iter()
                        .skip(n)
                        .flat_map(|a| a.vals.clone())
                        .collect(),
                    Some(Slot::ArgPointee(i)) => call
                        .args
                        .get(i)
                        .and_then(|a| a.place.clone().map(|ap| (ap, a.addr)))
                        .map(|(ap, addr)| {
                            vec![Val::direct(Atom::Read(if addr {
                                ap
                            } else {
                                ap.with(Step::Deref)
                            }))]
                        })
                        .unwrap_or_default(),
                    _ => vec![],
                };
                self.push_sources(file, func, &vals, epos, o, stack, hops, EdgeKind::Oracle);
            }
            return;
        }
        if targets.is_empty() {
            let o = self.node(
                file,
                func,
                NodeKey::Opaque(c),
                NodeKind::Opaque,
                call.line,
                format!(
                    "{}(…) (assumed from its arguments)",
                    short_pretty(&call.callee_pretty)
                ),
            );
            self.edge(o, n, EdgeKind::Assumed, "?".to_string());
            let mut vals: Vec<Val> = call.args.iter().flat_map(|a| a.vals.clone()).collect();
            if let Some(r) = &call.receiver {
                vals.extend(r.vals.clone());
            }
            self.push_sources(file, func, &vals, epos, o, stack, hops, EdgeKind::Assumed);
            return;
        }
        for target in targets {
            let mut new_stack = stack.to_vec();
            new_stack.push(Site {
                file: file.clone(),
                func,
                call: c,
                map: target.map.clone(),
                entered: None,
            });
            let tf = &target.file.funcs[target.func];
            match target.map {
                ArgMap::Xpidl {
                    retval: Some(r), ..
                } => {
                    // (JS's value is what C++ writes to `*_retval`.)
                    let Some(param) = tf.params.get(r) else {
                        continue;
                    };
                    let pn = self.node(
                        &target.file,
                        target.func,
                        NodeKey::Param(r),
                        NodeKind::Param,
                        tf.line,
                        format!("param {}", param.name),
                    );
                    self.edge(
                        pn,
                        n,
                        EdgeKind::Binding,
                        format!("*{} (retval)", param.name),
                    );
                    let place = param.place.with(Step::Deref);
                    self.push(
                        &target.file,
                        target.func,
                        Flow::Place(place),
                        None,
                        pn,
                        new_stack,
                        hops + 1,
                        EdgeKind::Local,
                    );
                }
                _ => {
                    let returns: Vec<usize> = tf
                        .events
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| matches!(e.kind, EventKind::Return { .. }) && !e.in_lambda)
                        .map(|(i, _)| i)
                        .collect();
                    for ri in returns {
                        let rn = self.event_node(&target.file, target.func, ri);
                        let edge_kind = if target.via.is_some() {
                            EdgeKind::Binding
                        } else {
                            EdgeKind::Return
                        };
                        self.edge(rn, n, edge_kind, "result".to_string());
                        let tf = &target.file.funcs[target.func];
                        let e = &tf.events[ri];
                        let EventKind::Return { vals } = &e.kind else {
                            continue;
                        };
                        let vals = vals.clone();
                        let rpos = Some(ri as isize);
                        self.push_sources(
                            &target.file,
                            target.func,
                            &vals,
                            rpos,
                            rn,
                            &new_stack,
                            hops + 1,
                            EdgeKind::Local,
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn backward_place(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        p: Place,
        pos: Option<isize>,
        node: usize,
        label: &str,
        kind: EdgeKind,
        stack: &[Site],
        hops: usize,
        root_typed: Option<(Root, String)>,
    ) {
        let n_events = file.funcs[func].events.len();
        let lang = file.funcs[func].lang;
        for ei in (0..n_events).rev() {
            let f = &file.funcs[func];
            let e = &f.events[ei];
            if self.opts.ordered
                && let Some(pos) = pos
                && (ei as isize) >= pos
            {
                continue;
            }
            let epos = Some(ei as isize);
            match e.kind.clone() {
                EventKind::Assign { place: q, vals, .. } => {
                    if !q.overlaps(&p) {
                        continue;
                    }
                    let n = self.event_node(file, func, ei);
                    self.edge(n, node, kind, label.to_string());
                    // (Writing the whole of something `p` is part of: `p`'s
                    // part of what's written.)
                    let suffix: Vec<Step> = if p.within(&q) {
                        p.path[q.path.len()..].to_vec()
                    } else {
                        vec![]
                    };
                    let vals: Vec<Val> = vals
                        .into_iter()
                        .map(|v| match v.atom {
                            Atom::Read(mut r) if !suffix.is_empty() => {
                                r.path.extend(suffix.iter().cloned());
                                Val {
                                    atom: Atom::Read(r),
                                    derived: v.derived,
                                }
                            }
                            _ => v,
                        })
                        .collect();
                    self.push_sources(file, func, &vals, epos, n, stack, hops, EdgeKind::Local);
                }
                EventKind::Call(c) => {
                    let call = f.calls[c].clone();
                    // (Arguments the callee can write `p` through.)
                    let writes: Vec<usize> = call
                        .args
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| may_write(a, &p, lang))
                        .map(|(i, _)| i)
                        .collect();
                    let receiver_writes = call.receiver.as_ref().is_some_and(|r| {
                        r.place
                            .as_ref()
                            .is_some_and(|rp| p.within(rp) && p.path.len() > rp.path.len())
                    });
                    if writes.is_empty() && !receiver_writes {
                        continue;
                    }
                    let targets = self.targets(file, func, c).await;
                    let targets = Self::prune_overloads(
                        &file.funcs[func],
                        &call,
                        &writes,
                        targets,
                        root_typed.as_ref(),
                    );
                    if targets.is_empty()
                        && let Some(summary) = self
                            .oracle
                            .as_ref()
                            .and_then(|o| o.summary(&call.callee).cloned())
                    {
                        let mut entered = false;
                        for flow in &summary.flows {
                            let Some(Slot::ArgPointee(to)) = parse_slot(&flow.to) else {
                                continue;
                            };
                            if !writes.contains(&to) {
                                continue;
                            }
                            if !entered {
                                let n = self.event_node(file, func, ei);
                                self.edge(n, node, kind, label.to_string());
                                entered = true;
                            }
                            let n = self.event_node(file, func, ei);
                            let o = self.node(
                                file,
                                func,
                                NodeKey::Opaque(c),
                                NodeKind::Opaque,
                                call.line,
                                format!("{}(…) per oracle", short_pretty(&call.callee_pretty)),
                            );
                            self.edge(
                                o,
                                n,
                                EdgeKind::Oracle,
                                format!("{} → {}", flow.from, flow.to),
                            );
                            let vals: Vec<Val> = match parse_slot(&flow.from) {
                                Some(Slot::This) => {
                                    call.receiver.iter().flat_map(|r| r.vals.clone()).collect()
                                }
                                Some(Slot::Arg(i)) => {
                                    call.args.get(i).map(|a| a.vals.clone()).unwrap_or_default()
                                }
                                Some(Slot::ArgsFrom(n)) => call
                                    .args
                                    .iter()
                                    .skip(n)
                                    .flat_map(|a| a.vals.clone())
                                    .collect(),
                                _ => vec![],
                            };
                            self.push_sources(
                                file,
                                func,
                                &vals,
                                epos,
                                o,
                                stack,
                                hops,
                                EdgeKind::Oracle,
                            );
                        }
                        continue;
                    }
                    if targets.is_empty() {
                        if writes.is_empty() {
                            // (Methods we can't see into might change their
                            // objects, but assuming every one does would
                            // drown the trace.)
                            continue;
                        }
                        let n = self.event_node(file, func, ei);
                        self.edge(n, node, kind, label.to_string());
                        let o = self.node(
                            file,
                            func,
                            NodeKey::Opaque(c),
                            NodeKind::Opaque,
                            call.line,
                            format!(
                                "{}(…) may write {} (assumed from its other arguments)",
                                short_pretty(&call.callee_pretty),
                                f.place_name(&p)
                            ),
                        );
                        self.edge(o, n, EdgeKind::Assumed, "?".to_string());
                        let mut vals: Vec<Val> = call
                            .args
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| !writes.contains(i))
                            .flat_map(|(_, a)| a.vals.clone())
                            .collect();
                        if let Some(r) = &call.receiver {
                            vals.extend(r.vals.clone());
                        }
                        self.push_sources(
                            file,
                            func,
                            &vals,
                            epos,
                            o,
                            stack,
                            hops,
                            EdgeKind::Assumed,
                        );
                        continue;
                    }
                    let mut entered = false;
                    for target in targets {
                        let tf = &target.file.funcs[target.func];
                        let mut new_stack = stack.to_vec();
                        new_stack.push(Site {
                            file: file.clone(),
                            func,
                            call: c,
                            map: target.map.clone(),
                            entered: None,
                        });
                        let mut places = vec![];
                        for &ai in &writes {
                            let Some(pi) = Self::param_for_arg(&target.map, ai) else {
                                continue;
                            };
                            let Some(param) = tf.params.get(pi) else {
                                continue;
                            };
                            if lang == Lang::Cpp && !param.is_out() && !call.args[ai].addr {
                                continue;
                            }
                            if let Some(cp) = Self::into_callee(&call.args[ai], &p, &param.place)
                                && (cp.path.first() == Some(&Step::Deref)
                                    || param.is_ref()
                                    || lang != Lang::Cpp)
                            {
                                places.push((pi, cp));
                            }
                        }
                        if receiver_writes
                            && let Some(receiver) = &call.receiver
                            && let Some(cp) = Self::into_callee(receiver, &p, &Place::this())
                        {
                            places.push((usize::MAX, cp));
                        }
                        if places.is_empty() {
                            continue;
                        }
                        // (Only if the callee writes it.)
                        let written: Vec<(usize, Place)> = places
                            .into_iter()
                            .filter(|(_, cp)| writes_place(tf, cp))
                            .collect();
                        if written.is_empty() {
                            continue;
                        }
                        if !entered {
                            let n = self.event_node(file, func, ei);
                            self.edge(n, node, kind, label.to_string());
                            entered = true;
                        }
                        let n = self.event_node(file, func, ei);
                        for (pi, cp) in written {
                            let pn = if pi == usize::MAX {
                                self.node(
                                    &target.file,
                                    target.func,
                                    NodeKey::Seed("this".to_string()),
                                    NodeKind::Param,
                                    tf.line,
                                    "this".to_string(),
                                )
                            } else {
                                let name = tf.params[pi].name.clone();
                                self.node(
                                    &target.file,
                                    target.func,
                                    NodeKey::Param(pi),
                                    NodeKind::Param,
                                    tf.line,
                                    format!("param {}", name),
                                )
                            };
                            let tf = &target.file.funcs[target.func];
                            self.edge(
                                pn,
                                n,
                                EdgeKind::Return,
                                format!("out {}", tf.place_name(&cp)),
                            );
                            let carried = (pi != usize::MAX)
                                .then(|| tf.params.get(pi).and_then(|p| p.type_pretty.clone()))
                                .flatten()
                                .filter(|pty| is_dependent_type(pty))
                                .and_then(|_| {
                                    let ai = (0..call.args.len()).find(|&ai| {
                                        Self::param_for_arg(&target.map, ai) == Some(pi)
                                    })?;
                                    Self::arg_type(
                                        &file.funcs[func],
                                        &call.args[ai],
                                        root_typed.as_ref(),
                                    )
                                });
                            let mut entered_stack = new_stack.clone();
                            if let Some(site) = entered_stack.last_mut()
                                && pi != usize::MAX
                            {
                                site.entered = Some(pi);
                            }
                            self.push_entered_typed(
                                &target.file,
                                target.func,
                                cp,
                                None,
                                pn,
                                entered_stack,
                                hops + 1,
                                EdgeKind::Local,
                                carried,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        // (Where values in parameters, `this`'s members and globals come from
        // outside the function.)
        let f = &file.funcs[func];
        if let Some(pi) = f.param_index(&p) {
            let param = f.params[pi].clone();
            let pn = self.node(
                file,
                func,
                NodeKey::Param(pi),
                NodeKind::Param,
                f.line,
                format!("param {}", param.name),
            );
            self.edge(pn, node, kind, label.to_string());
            // (What was in an out-parameter before the call we came from:
            // the caller's backwards scan, which entered here, goes on past
            // the call to that.)
            if stack.last().is_some_and(|site| site.entered == Some(pi)) && !p.path.is_empty() {
                return;
            }
            let sites = match stack.last() {
                Some(site) => vec![(site.clone(), stack[..stack.len() - 1].to_vec(), hops)],
                None => self
                    .callers(file, func)
                    .await
                    .into_iter()
                    .map(|s| (s, vec![], hops + 1))
                    .collect(),
            };
            for (site, rest, hops) in sites {
                let call = site.file.funcs[site.func].calls[site.call].clone();
                let Some(ai) = Self::arg_for_param(&site.map, pi) else {
                    continue;
                };
                let Some(arg) = call.args.get(ai) else {
                    // (An argument the caller left out: its default.)
                    continue;
                };
                let Some(cn) = event_of_call(&site.file, site.func, site.call)
                    .map(|ei| self.event_node(&site.file, site.func, ei))
                else {
                    continue;
                };
                let edge_kind = if matches!(site.map, ArgMap::Xpidl { .. }) {
                    EdgeKind::Binding
                } else {
                    EdgeKind::Call
                };
                let pos = event_of_call(&site.file, site.func, site.call).map(|i| i as isize);
                self.edge(cn, pn, edge_kind, format!("arg {}", ai));
                match &arg.place {
                    Some(ap) => {
                        let mut cp = ap.clone();
                        let rest_path: &[Step] = if arg.addr && p.path.first() == Some(&Step::Deref)
                        {
                            &p.path[1..]
                        } else {
                            &p.path
                        };
                        cp.path.extend(rest_path.iter().cloned());
                        if arg.addr && p.path.first() != Some(&Step::Deref) {
                            // (The pointer itself is `&x`.)
                            self.push_sources(
                                &site.file,
                                site.func,
                                &arg.vals,
                                pos,
                                cn,
                                &rest,
                                hops,
                                EdgeKind::Local,
                            );
                        } else {
                            self.push(
                                &site.file,
                                site.func,
                                Flow::Place(cp),
                                pos,
                                cn,
                                rest,
                                hops,
                                EdgeKind::Local,
                            );
                        }
                    }
                    None => {
                        let vals = arg.vals.clone();
                        self.push_sources(
                            &site.file,
                            site.func,
                            &vals,
                            pos,
                            cn,
                            &rest,
                            hops,
                            EdgeKind::Local,
                        );
                    }
                }
            }
            return;
        }
        match &p.root {
            Root::This if !p.path.is_empty() => {
                if let Some(site) = stack.last() {
                    let call = site.file.funcs[site.func].calls[site.call].clone();
                    if let Some(receiver) = &call.receiver
                        && let Some(rp) = &receiver.place
                        && let Some(cn) = event_of_call(&site.file, site.func, site.call)
                            .map(|ei| self.event_node(&site.file, site.func, ei))
                    {
                        let mut cp = rp.clone();
                        cp.path.extend(p.path.iter().cloned());
                        self.edge(cn, node, EdgeKind::Call, "this".to_string());
                        let pos =
                            event_of_call(&site.file, site.func, site.call).map(|i| i as isize);
                        self.push(
                            &site.file,
                            site.func,
                            Flow::Place(cp),
                            pos,
                            cn,
                            stack[..stack.len() - 1].to_vec(),
                            hops,
                            EdgeKind::Local,
                        );
                    }
                }
                if let Some(field) = p.last_field() {
                    self.backward_field(file, func, field, node, hops).await;
                }
            }
            Root::Global(sym) => {
                let sym = *sym;
                self.backward_field(file, func, sym, node, hops).await;
            }
            _ => {}
        }
    }

    /// The writes of a field (or global) in other functions.
    async fn backward_field(
        &mut self,
        file: &Arc<FileIr>,
        func: usize,
        field: Ustr,
        node: usize,
        hops: usize,
    ) {
        if field.starts_with("name:") || hops + 1 > self.opts.max_hops {
            return;
        }
        let uses = self.field_uses(field).await;
        let mut followed = 0;
        let mut writes_total = 0;
        for (path, line, contextsym) in uses {
            let Some(use_file) = self.file(&path).await else {
                continue;
            };
            let Some(uf) = use_file.by_sym.get(&contextsym).copied() else {
                continue;
            };
            if Arc::ptr_eq(&use_file, file) && uf == func {
                continue;
            }
            let wf = &use_file.funcs[uf];
            let writes: Vec<usize> = wf
                .events
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.on_line(line)
                        && matches!(&e.kind, EventKind::Assign { place, .. } if place.path.contains(&Step::Field(field)) || place.root == Root::Global(field))
                })
                .map(|(i, _)| i)
                .collect();
            if writes.is_empty() {
                continue;
            }
            writes_total += 1;
            if followed >= self.opts.fanout {
                continue;
            }
            followed += 1;
            for ei in writes {
                let n = self.event_node(&use_file, uf, ei);
                self.edge(
                    n,
                    node,
                    EdgeKind::Field,
                    short_name(&self.names_of(&use_file, uf, field)).to_string(),
                );
                let e = &use_file.funcs[uf].events[ei];
                let EventKind::Assign { vals, .. } = &e.kind else {
                    continue;
                };
                let vals = vals.clone();
                let epos = Some(ei as isize);
                self.push_sources(
                    &use_file,
                    uf,
                    &vals,
                    epos,
                    n,
                    &[],
                    hops + 1,
                    EdgeKind::Local,
                );
            }
        }
        if writes_total > self.opts.fanout {
            self.note(format!(
                "{} is written in {} places; followed {}",
                self.names_of(file, func, field),
                writes_total,
                self.opts.fanout
            ));
        }
    }

    // ## IPDL fields (see `IpdlSlot`)

    /// The reads of an IPDL field, wherever they are: its accessors' calls'
    /// results (but for those assigned to).
    async fn forward_ipdl_field(&mut self, field: Ustr, node: usize, hops: usize) {
        if hops + 1 > self.opts.max_hops {
            self.note(format!("Stopped at {} hops", self.opts.max_hops));
            return;
        }
        let name = short_name(&self.symbol_name(&field).await).to_string();
        let (accessors, _) = self.ipdl_field_slots(field).await;
        let mut funcs: HashSet<(String, Ustr)> = HashSet::new();
        for accessor in accessors {
            for (path, line, contextsym) in self.field_uses(accessor).await {
                // (Not the generated code's own, ex: its serialization's
                // `ParamTraits<…>::Write`: IPC hops carry values across.)
                if path.starts_with("__GENERATED__/") {
                    continue;
                }
                let Some(use_file) = self.file(&path).await else {
                    continue;
                };
                let Some(uf) = use_file.by_sym.get(&contextsym).copied() else {
                    continue;
                };
                let reads = ipdl_reads(&use_file.funcs[uf], line, accessor);
                if reads.is_empty()
                    || (funcs.insert((path.clone(), contextsym)) && funcs.len() > self.opts.fanout)
                {
                    continue;
                }
                for (c, ei) in reads {
                    let n = self.event_node(&use_file, uf, ei);
                    self.edge(node, n, EdgeKind::Field, name.clone());
                    self.push(
                        &use_file,
                        uf,
                        Flow::Result(c),
                        Some(ei as isize),
                        n,
                        vec![],
                        hops + 1,
                        EdgeKind::Local,
                    );
                }
            }
        }
        if funcs.len() > self.opts.fanout {
            self.note(format!(
                "{} is read in {} functions; followed {}",
                name,
                funcs.len(),
                self.opts.fanout
            ));
        }
    }

    /// The writes of an IPDL field, wherever they are: its struct's
    /// constructors' arguments, and assignments through its mutable
    /// accessor.
    async fn backward_ipdl_field(&mut self, field: Ustr, node: usize, hops: usize) {
        if hops + 1 > self.opts.max_hops {
            self.note(format!("Stopped at {} hops", self.opts.max_hops));
            return;
        }
        let name = short_name(&self.symbol_name(&field).await).to_string();
        let (accessors, constructors) = self.ipdl_field_slots(field).await;
        // (The constructors' calls, with the field's argument, and the
        // accessors' assignments.)
        let slots: Vec<(Ustr, Option<usize>)> = constructors
            .into_iter()
            .map(|(sym, index)| (sym, Some(index)))
            .chain(accessors.into_iter().map(|sym| (sym, None)))
            .collect();
        let mut funcs: HashSet<(String, Ustr)> = HashSet::new();
        for (slot, index) in slots {
            for (path, line, contextsym) in self.field_uses(slot).await {
                // (Not the generated code's own; see `forward_ipdl_field`.)
                if path.starts_with("__GENERATED__/") {
                    continue;
                }
                let Some(use_file) = self.file(&path).await else {
                    continue;
                };
                let Some(uf) = use_file.by_sym.get(&contextsym).copied() else {
                    continue;
                };
                let wf = &use_file.funcs[uf];
                let writes: Vec<(usize, Vec<Val>)> = match index {
                    Some(index) => wf
                        .events
                        .iter()
                        .enumerate()
                        .filter_map(|(ei, e)| match e.kind {
                            EventKind::Call(c)
                                if e.on_line(line) && wf.calls[c].callee.contains(&slot) =>
                            {
                                Some((ei, wf.calls[c].args.get(index)?.vals.clone()))
                            }
                            _ => None,
                        })
                        .collect(),
                    None => wf
                        .events
                        .iter()
                        .enumerate()
                        .filter_map(|(ei, e)| match &e.kind {
                            EventKind::Assign { place, vals, .. }
                                if e.on_line(line)
                                    && place.path.last() == Some(&Step::Field(slot)) =>
                            {
                                Some((ei, vals.clone()))
                            }
                            _ => None,
                        })
                        .collect(),
                };
                if writes.is_empty()
                    || (funcs.insert((path.clone(), contextsym)) && funcs.len() > self.opts.fanout)
                {
                    continue;
                }
                for (ei, vals) in writes {
                    let n = self.event_node(&use_file, uf, ei);
                    self.edge(n, node, EdgeKind::Field, name.clone());
                    self.push_sources(
                        &use_file,
                        uf,
                        &vals,
                        Some(ei as isize),
                        n,
                        &[],
                        hops + 1,
                        EdgeKind::Local,
                    );
                }
            }
        }
        if funcs.len() > self.opts.fanout {
            self.note(format!(
                "{} is written in {} functions; followed {}",
                name,
                funcs.len(),
                self.opts.fanout
            ));
        }
    }
}

/// The calls of an IPDL field's accessor on a line of a function which read
/// it (as call and event indexes): not those whose results are assigned to
/// (`x.f() = v`; see `is_assigned`).
fn ipdl_reads(f: &FuncIr, line: u32, accessor: Ustr) -> Vec<(usize, usize)> {
    f.events
        .iter()
        .enumerate()
        .filter_map(|(ei, e)| match e.kind {
            EventKind::Call(c)
                if e.on_line(line)
                    && f.calls[c].callee.contains(&accessor)
                    && !is_assigned(f, c) =>
            {
                Some((c, ei))
            }
            _ => None,
        })
        .collect()
}

/// Whether a call's result is assigned to (`x.f() = v`, an assignment to
/// `x`'s part named by `f`'s symbol; see the C++ frontend's assignments).
fn is_assigned(f: &FuncIr, c: usize) -> bool {
    let call = &f.calls[c];
    let (
        Some(Arg {
            place: Some(receiver),
            ..
        }),
        Some(sym),
    ) = (&call.receiver, call.callee.first())
    else {
        return false;
    };
    let place = receiver.with(Step::Field(*sym));
    f.events.iter().any(|e| {
        e.on_line(call.line) && matches!(&e.kind, EventKind::Assign { place: q, .. } if *q == place)
    })
}

fn e_line(file: &FileIr, func: usize, ei: usize) -> u32 {
    file.funcs[func].events[ei].line
}

fn event_of_call(file: &FileIr, func: usize, call: usize) -> Option<usize> {
    file.funcs[func]
        .events
        .iter()
        .position(|e| matches!(e.kind, EventKind::Call(c) if c == call))
}

/// Whether values read a flow: a place overlapping it, or its call result.
fn reads(vals: &[Val], flow: &Flow) -> bool {
    vals.iter().any(|v| match (&v.atom, flow) {
        (Atom::Read(r), Flow::Place(p)) => r.overlaps(p),
        (Atom::Addr(r), Flow::Place(p)) => r.overlaps(p),
        (Atom::Call(c), Flow::Result(d)) => c == d,
        _ => false,
    })
}

/// Whether an argument passes (a part of) a place (by reference, or its
/// address).
fn arg_place_hit(arg: &Arg, flow: &Flow) -> bool {
    match (&arg.place, flow) {
        (Some(ap), Flow::Place(p)) => ap.overlaps(p),
        _ => false,
    }
}

/// Whether a callee could write a place through an argument: `&x` for `x`,
/// a pointer `p` for `*p`, a reference or object `o` for `o` (and their
/// parts).
fn may_write(arg: &Arg, p: &Place, lang: Lang) -> bool {
    let Some(ap) = &arg.place else {
        return false;
    };
    if arg.addr {
        return p.within(ap);
    }
    match lang {
        Lang::Cpp => p.within(&ap.with(Step::Deref)) || p.within(ap),
        _ => p.within(ap) && p.path.len() > ap.path.len(),
    }
}

/// Whether a function reads a place (or a part of it, or what it's part
/// of), or passes it on.
fn reads_place(f: &FuncIr, place: &Place) -> bool {
    let flow = Flow::Place(place.clone());
    f.events.iter().any(|e| match &e.kind {
        EventKind::Assign { vals, .. } | EventKind::Return { vals } | EventKind::Test { vals } => {
            reads(vals, &flow)
        }
        EventKind::Call(c) => {
            let call = &f.calls[*c];
            call.args
                .iter()
                .chain(call.receiver.iter())
                .any(|a| reads(&a.vals, &flow) || arg_place_hit(a, &flow))
        }
    })
}

/// Whether a function writes a place (or a part of it, or what it's part of).
fn writes_place(f: &FuncIr, place: &Place) -> bool {
    f.events.iter().any(|e| match &e.kind {
        EventKind::Assign { place: q, .. } => q.overlaps(place),
        // (It might pass it on to something that does.)
        EventKind::Call(c) => f.calls[*c].args.iter().any(|a| {
            a.place
                .as_ref()
                .is_some_and(|ap| ap.overlaps(place) || place.within(&ap.with(Step::Deref)))
        }),
        _ => false,
    })
}

/// The places with a field read (or written) at a line of a function.
fn places_with_field(f: &FuncIr, line: u32, field: Ustr, writes: bool) -> Vec<Place> {
    let mut out = vec![];
    let has = |p: &Place| p.path.contains(&Step::Field(field)) || p.root == Root::Global(field);
    let mut add = |p: &Place| {
        if has(p) {
            // (Up to the field.)
            let mut place = p.clone();
            if let Some(i) = place.path.iter().position(|s| *s == Step::Field(field)) {
                place.path.truncate(i + 1);
            }
            if !out.contains(&place) {
                out.push(place);
            }
        }
    };
    for e in f.events.iter().filter(|e| e.on_line(line)) {
        let vals: Vec<&Val> = match &e.kind {
            EventKind::Assign { place, vals, .. } => {
                if writes {
                    add(place);
                }
                vals.iter().collect()
            }
            EventKind::Call(c) => {
                let call = &f.calls[*c];
                let mut vals: Vec<&Val> = call.args.iter().flat_map(|a| a.vals.iter()).collect();
                if let Some(r) = &call.receiver {
                    vals.extend(r.vals.iter());
                }
                vals
            }
            EventKind::Return { vals } | EventKind::Test { vals } => vals.iter().collect(),
        };
        if !writes {
            for v in vals {
                if let Atom::Read(p) | Atom::Addr(p) = &v.atom {
                    add(p);
                }
            }
        }
    }
    out
}

/// The IPDL message symbol for a generated `Send` method's pretty name (ex:
/// `IPC_mozilla_dom_PSharedWorker_UpdateTimezoneOverride` for
/// `mozilla::dom::PSharedWorkerChild::SendUpdateTimezoneOverride`), for
/// when the generated code's records aren't in the index.
pub fn ipc_message_sym(pretty: &str) -> Option<Ustr> {
    let (class_path, method) = pretty.rsplit_once("::")?;
    let message = method.strip_prefix("Send")?;
    let (ns, class) = match class_path.rsplit_once("::") {
        Some((ns, class)) => (ns, class),
        None => ("", class_path),
    };
    let protocol = class
        .strip_suffix("Child")
        .or_else(|| class.strip_suffix("Parent"))?;
    if !protocol.starts_with('P') {
        return None;
    }
    let ns = ns.replace("::", "_");
    let sym = if ns.is_empty() {
        format!("IPC_{}_{}", protocol, message)
    } else {
        format!("IPC_{}_{}_{}", ns, protocol, message)
    };
    Some(ustr(&sym))
}

/// The last two segments of a pretty name (ex: "Preferences::GetBool").
pub fn short_pretty(pretty: &str) -> String {
    let parts: Vec<&str> = pretty.split("::").collect();
    if parts.len() <= 2 {
        return pretty.to_string();
    }
    parts[parts.len() - 2..].join("::")
}

/// Find a place in a function by its name as `FuncIr::place_name` shows it
/// (ex: "aRetVal", "*aRetVal", "this->mName").
pub fn find_place(f: &FuncIr, name: &str) -> Option<Place> {
    fn add_vals(places: &mut Vec<Place>, vals: &[Val]) {
        for v in vals {
            if let Atom::Read(p) | Atom::Addr(p) = &v.atom {
                places.push(p.clone());
            }
        }
    }
    let mut places: Vec<Place> = f.params.iter().map(|p| p.place.clone()).collect();
    for e in &f.events {
        match &e.kind {
            EventKind::Assign { place, vals, .. } => {
                places.push(place.clone());
                add_vals(&mut places, vals);
            }
            EventKind::Call(c) => {
                let call = &f.calls[*c];
                for a in &call.args {
                    add_vals(&mut places, &a.vals);
                    if let Some(p) = &a.place {
                        places.push(p.clone());
                    }
                }
                if let Some(r) = &call.receiver {
                    add_vals(&mut places, &r.vals);
                }
            }
            EventKind::Return { vals } | EventKind::Test { vals } => add_vals(&mut places, vals),
        }
    }
    // (Prefixes of what's there are places too: `*p` for `p->x`.)
    let mut all = vec![];
    for p in places {
        for len in 0..=p.path.len() {
            all.push(Place {
                root: p.root.clone(),
                path: p.path[..len].to_vec(),
            });
        }
    }
    all.into_iter().find(|p| f.place_name(p) == name)
}

#[allow(dead_code)]
fn unused(_: Ustr) -> Ustr {
    ustr("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_message_syms() {
        assert_eq!(
            ipc_message_sym("mozilla::dom::PSharedWorkerChild::SendUpdateTimezoneOverride")
                .as_deref(),
            Some("IPC_mozilla_dom_PSharedWorker_UpdateTimezoneOverride")
        );
        assert_eq!(
            ipc_message_sym("PFooParent::SendBar").as_deref(),
            Some("IPC_PFoo_Bar")
        );
        assert_eq!(ipc_message_sym("mozilla::dom::SharedWorker::SendFoo"), None);
        assert_eq!(ipc_message_sym("PFooChild::RecvBar"), None);
    }

    #[test]
    fn test_short_pretty() {
        assert_eq!(
            short_pretty("mozilla::Preferences::GetBool"),
            "Preferences::GetBool"
        );
        assert_eq!(short_pretty("GetBool"), "GetBool");
    }

    #[test]
    fn test_xpidl_arg_map() {
        let map = ArgMap::Xpidl {
            argc: Some(2),
            retval: Some(3),
        };
        assert_eq!(Tracer::param_for_arg(&map, 1), Some(1));
        assert_eq!(Tracer::param_for_arg(&map, 2), None);
        assert_eq!(Tracer::arg_for_param(&map, 3), None);
        assert_eq!(Tracer::arg_for_param(&map, 0), Some(0));
    }
}
