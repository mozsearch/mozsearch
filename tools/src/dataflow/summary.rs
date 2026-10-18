//! Per-function dataflow summaries: which of a function's inputs (its
//! parameters, the fields of its object and of its parameters, globals, and
//! what its calls return) flow into which of its outputs (fields it writes,
//! what it returns, and its calls' arguments), ignoring the order of its
//! events and what flows only through its locals.  Summaries are what's
//! worth precomputing for a whole tree (see `dataflow-summarize`): composed
//! across calls they say how data moves between fields (ex: a constructor's
//! parameter stored in a member, from a caller's member), which crossref
//! rolls up into edges between classes and subsystems.
//!
//! "Boring" plumbing is left out: logging and assertion macros' expansions,
//! refcounting, and assertion methods.  The code IPDL generates is seen
//! through, to the struct fields and message parameters it carries (see
//! `IpdlSlots`).

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

use super::ir::*;

/// An end of a flow in a function.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "camelCase")]
pub enum Port {
    /// A parameter, or the fields reached through it (ex: `aData.mFoo`,
    /// `aData->mFoo`), by its index.
    Param {
        i: usize,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<Ustr>,
    },
    /// The object the function is a method of, or its fields (`this->mFoo`,
    /// `mFoo.mBar`).
    This {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<Ustr>,
    },
    /// A global (or static) variable, or its fields.
    Global {
        sym: Ustr,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<Ustr>,
    },
    /// What the function returns.
    Return,
    /// An argument of a call, by the callee's symbol and its index (the
    /// object a method is called on is `None`).
    Arg {
        callee: Ustr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        i: Option<usize>,
    },
    /// What a call returns (or writes through an argument it's given the
    /// address of, `i`).
    Result {
        callee: Ustr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        i: Option<usize>,
    },
}

impl Port {
    /// The port with more fields (ex: `this->mFoo` with `mBar` is
    /// `this->mFoo.mBar`), for the ports that have fields.
    fn with_fields(&self, more: &[Ustr]) -> Port {
        if more.is_empty() {
            return self.clone();
        }
        let extend = |fields: &Vec<Ustr>| {
            let mut fields = fields.clone();
            fields.extend(more.iter().copied());
            // (Fields of fields of fields are rarely worth distinguishing.)
            fields.truncate(MAX_FIELDS);
            fields
        };
        match self {
            Port::Param { i, fields } => Port::Param {
                i: *i,
                fields: extend(fields),
            },
            Port::This { fields } => Port::This {
                fields: extend(fields),
            },
            Port::Global { sym, fields } => Port::Global {
                sym: *sym,
                fields: extend(fields),
            },
            other => other.clone(),
        }
    }

    /// The field it is, if it's (in) one: its last field.
    pub fn field(&self) -> Option<Ustr> {
        match self {
            Port::Param { fields, .. } | Port::This { fields } | Port::Global { fields, .. } => {
                fields.last().copied()
            }
            _ => None,
        }
    }
}

const MAX_FIELDS: usize = 3;

/// A flow from one port to another, and whether the value is derived from
/// the source (`a + 1`, `a == b`) rather than it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Flow {
    pub from: Port,
    pub to: Port,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
}

/// What kind of function it is, by what it does with data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// Returns a field (of its object, or of a parameter), and does nothing
    /// else worth noting.
    Getter,
    /// Stores a parameter in a field, and does nothing else worth noting.
    Setter,
    /// Moves no data worth noting (ex: an empty method, logging).
    Trivial,
    /// A few events.
    Small,
    /// Many events.
    Big,
}

/// A function's summary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub sym: Ustr,
    pub pretty: String,
    pub path: String,
    pub line: u32,
    pub category: Category,
    /// How many events (assignments, calls, returns, tests) it has, beyond
    /// boring ones.
    pub events: usize,
    pub flows: Vec<Flow>,
    /// Whether it had more flows than `MAX_FLOWS`, so it has none.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flows_omitted: bool,
    /// The (virtual) methods it overrides, whose calls can reach it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<Ustr>,
    /// For accessors (getters, setters and trivial methods): the constants
    /// it returns or writes (ex: `mFoo = 0`, `return true`), which flows,
    /// between ports, leave out, but traces stepping over its calls by its
    /// summary show (see `dataflow::trace`'s `accessor_summary`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consts: Vec<ConstFlow>,
}

/// A constant a function returns or writes (see `Summary::consts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConstFlow {
    pub text: String,
    pub to: Port,
}

/// The most constants an accessor's summary keeps.
const MAX_CONSTS: usize = 8;

/// The methods a function's structured record says it overrides.
pub fn structured_overrides(structured: &serde_json::Value) -> Vec<Ustr> {
    structured
        .get("overrides")
        .and_then(|o| o.as_array())
        .map(|overrides| {
            overrides
                .iter()
                .filter_map(|o| o.get("sym").and_then(|s| s.as_str()).map(ustr))
                .collect()
        })
        .unwrap_or_default()
}

/// Macros whose expansions are plumbing rather than data's movement.
const BORING_MACROS: &[&str] = &[
    "LOG",
    "LOGV",
    "LOGD",
    "LOGE",
    "LOGW",
    "LOGI",
    "MOZ_LOG",
    "MOZ_LOG_FMT",
    "MOZ_ASSERT",
    "MOZ_ASSERT_IF",
    "MOZ_DIAGNOSTIC_ASSERT",
    "MOZ_RELEASE_ASSERT",
    "MOZ_ASSERT_UNREACHABLE",
    "MOZ_CRASH",
    "MOZ_CRASH_UNSAFE",
    "MOZ_DIAGNOSTIC_CRASH",
    "MOZ_ASSERT_DEBUG_OR_FUZZING",
    "NS_ASSERTION",
    "NS_WARNING",
    "NS_WARNING_ASSERTION",
    "NS_ENSURE_SUCCESS",
    "NS_ENSURE_TRUE",
    "NS_ENSURE_ARG",
    "NS_ENSURE_ARG_POINTER",
    "NS_ENSURE_STATE",
    "NS_WARN_IF",
    "AUTO_PROFILER_LABEL",
    "AUTO_PROFILER_MARKER_TEXT",
    "PROFILER_MARKER_TEXT",
    "MOZ_COUNT_CTOR",
    "MOZ_COUNT_DTOR",
    "NS_LOG_ADDREF",
    "NS_LOG_RELEASE",
    "NS_ASSERT_OWNINGTHREAD",
    "MOZ_DIAGNOSTIC_ASSERT_IF",
];

/// Whether a macro's expansion is plumbing (see `BORING_MACROS`; and
/// gtest's assertions, and modules' own `LOG`-like macros).
pub fn is_boring_macro(name: &str) -> bool {
    BORING_MACROS.contains(&name)
        || name.starts_with("EXPECT_")
        || name.starts_with("ASSERT_")
        || name.ends_with("_LOG")
        || name.starts_with("LOG_")
}

pub use crate::file_format::rollup::is_boring_callee;

/// What the C++ code IPDL generates for structs, unions and messages is (by
/// ipdl-analyze's binding slots: its `IPDL_` and `IPC_` owners), so that
/// summaries see through it to the data it carries, as data of its own: a
/// struct's field (or a union's variant) is written by the struct's
/// constructors' arguments and through its mutable accessor (`x.f() = v`),
/// and read through its accessors' calls; a message's parameter is its
/// `Send` callers' argument and its `Recv` implementations' parameter.  Each
/// is a `Port::Global` wherever the struct or message is (context
/// insensitively, as the tracer's `IpdlSlot`s are), so field edges go from
/// the sender's fields into the struct's fields (or the message's
/// parameters), and out of them into the receiver's: data edges cross IPC.
/// (Not in the generated code itself, whose serializers read every field.)
#[derive(Clone, Debug, Default)]
pub struct IpdlSlots {
    /// The accessors (getter and setter slots) of struct fields and union
    /// variants: the field's (or variant's) `IPDL_` symbol.
    pub accessors: HashMap<Ustr, Ustr>,
    /// Structs' constructors from their fields: the fields, in order; and
    /// unions' from a variant: the variant.
    pub constructors: HashMap<Ustr, Vec<Ustr>>,
    /// Struct fields' generated members (`F_<T_…>_field_`): the field.
    pub members: HashMap<Ustr, Ustr>,
    /// Messages' generated `Send` methods: their parameters, by argument.
    pub sends: HashMap<Ustr, Vec<Option<IpcParam>>>,
    /// Messages' `Recv` methods (the generated base classes' declarations,
    /// which the actors' implementations override, or the implementations
    /// the generated code calls directly): their parameters, by argument.
    pub recvs: HashMap<Ustr, Vec<Option<IpcParam>>>,
}

/// An IPDL message's parameter: its `IPC_` symbol, and whether it's an
/// out-parameter (a sync message's reply, which the `Recv` implementation
/// writes and the `Send` caller reads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IpcParam {
    pub sym: Ustr,
    pub out: bool,
}

impl IpdlSlots {
    /// From the structured records of `.ipdl` and `.ipdlh` files' analysis
    /// (or crossref's entries of the same owners).
    pub fn from_structured<'a>(records: impl IntoIterator<Item = &'a serde_json::Value>) -> Self {
        let str_of = |v: &serde_json::Value, key: &str| v.get(key)?.as_str().map(ustr);
        let slots_of = |record: &'a serde_json::Value| {
            record
                .get("bindingSlots")
                .and_then(|slots| slots.as_array())
                .into_iter()
                .flatten()
                .filter_map(move |slot| {
                    Some((slot.get("slotKind")?.as_str()?, str_of(slot, "sym")?))
                })
        };
        let mut table = IpdlSlots::default();
        // (Messages' `Send` and `Recv` slots, for their parameters.)
        let mut messages: HashMap<Ustr, Vec<(&str, Ustr)>> = HashMap::new();
        let mut params: Vec<&serde_json::Value> = vec![];
        for record in records {
            let (Some(sym), Some(kind)) = (str_of(record, "sym"), str_of(record, "kind")) else {
                continue;
            };
            match (kind.as_str(), sym.starts_with("IPDL_")) {
                ("field", true) => {
                    for (slot_kind, slot) in slots_of(record) {
                        match slot_kind {
                            "getter" | "setter" => {
                                table.accessors.insert(slot, sym);
                            }
                            "member" => {
                                table.members.insert(slot, sym);
                            }
                            // (A union's constructor from the variant.)
                            "constructor" => {
                                table.constructors.insert(slot, vec![sym]);
                            }
                            _ => {}
                        }
                    }
                }
                ("struct", true) => {
                    let fields: Vec<Ustr> = record
                        .get("fields")
                        .and_then(|fields| fields.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|field| str_of(field, "sym"))
                        .collect();
                    for (slot_kind, slot) in slots_of(record) {
                        if slot_kind == "constructor" && !fields.is_empty() {
                            table.constructors.insert(slot, fields.clone());
                        }
                    }
                }
                ("ipc", _) => {
                    messages.insert(sym, slots_of(record).collect());
                }
                ("parameter", _) if sym.starts_with("IPC_") => params.push(record),
                _ => {}
            }
        }
        for record in params {
            let (Some(sym), Some(message), Some(index)) = (
                str_of(record, "sym"),
                str_of(record, "parentsym"),
                record.get("ipdlIndex").and_then(|i| i.as_u64()),
            ) else {
                continue;
            };
            let param = IpcParam {
                sym,
                out: record.get("ipdlOut").and_then(|o| o.as_bool()) == Some(true),
            };
            for (slot_kind, slot) in messages.get(&message).into_iter().flatten() {
                let by_slot = match *slot_kind {
                    "send" => &mut table.sends,
                    "recv" => &mut table.recvs,
                    _ => continue,
                };
                let args = by_slot.entry(*slot).or_default();
                let index = index as usize;
                if args.len() <= index {
                    args.resize(index + 1, None);
                }
                args[index] = Some(param);
            }
        }
        table
    }

    pub fn is_empty(&self) -> bool {
        self.accessors.is_empty()
            && self.constructors.is_empty()
            && self.sends.is_empty()
            && self.recvs.is_empty()
    }

    /// The field a call accesses, if it's of an IPDL struct field's (or
    /// union variant's) accessor.
    fn accessor(&self, call: &Call) -> Option<Ustr> {
        call.callee
            .iter()
            .find_map(|callee| self.accessors.get(callee).copied())
    }

    /// The fields a call's arguments are, if it's of an IPDL struct's (or
    /// union's) constructor.
    fn constructor(&self, call: &Call) -> Option<&[Ustr]> {
        call.callee.iter().find_map(|callee| {
            self.constructors
                .get(callee)
                .map(|fields| fields.as_slice())
        })
    }

    /// The field a field symbol is, if it's an IPDL struct field's accessor
    /// (see the C++ frontend's assignments through one) or member, or the
    /// field itself (as the tracer's places have it; see
    /// `Tracer::canonicalize_ipdl_steps`).
    fn field(&self, sym: &Ustr) -> Option<Ustr> {
        if sym.starts_with("IPDL_") {
            return Some(*sym);
        }
        self.accessors
            .get(sym)
            .or_else(|| self.members.get(sym))
            .copied()
    }

    /// The field a step through an IPDL struct field's mutable accessor (or
    /// the field itself) writes, if it's one.
    fn written_field(&self, sym: &Ustr) -> Option<Ustr> {
        if sym.starts_with("IPDL_") {
            return Some(*sym);
        }
        self.accessors.get(sym).copied()
    }
}

/// Whether a path is of generated code (see `IpdlSlots`).
fn is_generated_path(path: &str) -> bool {
    path.starts_with("__GENERATED__/")
}

/// Whether a field edge's end is a field (a C++ member, an IPDL struct's
/// field or union's variant, or an IPDL message's parameter; see
/// `IpdlSlots`) rather than a function (or a global).
pub fn is_field_sym(sym: &str) -> bool {
    sym.starts_with("F_") || is_ipdl_sym(sym)
}

/// Whether a symbol is an IPDL struct's field, union's variant, or message's
/// parameter (see `IpdlSlots`).
fn is_ipdl_sym(sym: &str) -> bool {
    sym.starts_with("IPDL_") || sym.starts_with("IPC_")
}

/// Whether a call is a method of a value type (see `is_value_type_class`;
/// ex: `nsTArray::AppendElement`, `RefPtr::operator->`), which summaries see
/// through: its result is its object's (and arguments') data, and, unless
/// it's a read (by its name), its arguments' data goes into its object.
fn value_type_call(call: &Call) -> Option<bool> {
    let receiver_is_value = call.receiver.is_some();
    let (class, method) = call.callee_pretty.rsplit_once("::")?;
    let class = class.split('<').next().unwrap_or(class);
    if !receiver_is_value || !is_value_type_class(&format!("T_{}", class)) {
        return None;
    }
    const READS: &[&str] = &[
        "Get",
        "get",
        "Is",
        "Contains",
        "Index",
        "Find",
        "Equals",
        "Compare",
        "Has",
        "Length",
        "Count",
        "Lookup",
        "IsEmpty",
        "Elements",
        "operator",
        "begin",
        "end",
        "size",
        "empty",
        "at",
        "front",
        "back",
        "Last",
        "First",
        "Value",
        "value",
        "ref",
        "isSome",
        "isNothing",
        "Clone",
        "Hash",
        "Data",
        "BeginReading",
        "EndReading",
        "Substring",
        "Find",
    ];
    let is_read = READS.iter().any(|prefix| method.starts_with(prefix));
    Some(!is_read && !call.args.is_empty())
}

/// The sources (and whether derived) of values: ports, after following
/// locals.
type Sources = BTreeSet<(Port, bool)>;

struct Builder<'f> {
    f: &'f FuncIr,
    /// The code IPDL generates (none in generated code; see `IpdlSlots`).
    ipdl: Option<&'f IpdlSlots>,
    /// If it implements an IPDL message's `Recv` method: the message's
    /// parameters, by its parameters.
    recv_params: &'f [Option<IpcParam>],
    /// What each local (by symbol) holds.
    locals: HashMap<Ustr, Sources>,
    /// The function's events and calls that aren't plumbing.
    events: Vec<&'f Event>,
    boring_calls: Vec<bool>,
}

impl<'f> Builder<'f> {
    fn new(f: &'f FuncIr, ipdl: &'f IpdlSlots, overrides: &[Ustr]) -> Self {
        let ipdl = (!ipdl.is_empty() && !is_generated_path(&f.path)).then_some(ipdl);
        let recv_params = ipdl
            .and_then(|ipdl| {
                std::iter::once(&f.sym)
                    .chain(overrides)
                    .find_map(|sym| ipdl.recvs.get(sym))
            })
            .map_or(&[][..], |params| params.as_slice());
        let boring_calls: Vec<bool> = f
            .calls
            .iter()
            .map(|c| is_boring_callee(&c.callee_pretty))
            .collect();
        let events = f
            .events
            .iter()
            .filter(|e| {
                !e.in_lambda
                    && !e.macro_name.as_deref().is_some_and(is_boring_macro)
                    && !matches!(e.kind, EventKind::Call(c) if boring_calls[c])
            })
            .collect();
        Builder {
            f,
            ipdl,
            recv_params,
            locals: HashMap::new(),
            events,
            boring_calls,
        }
    }

    /// The fields in a path (ignoring dereferences and indexing), an IPDL
    /// struct field's accessor's as the field (see `IpdlSlots`).
    fn path_fields(&self, path: &[Step]) -> Vec<Ustr> {
        let mut fields: Vec<Ustr> = path
            .iter()
            .filter_map(|step| match step {
                Step::Field(sym) => {
                    Some(self.ipdl.and_then(|ipdl| ipdl.field(sym)).unwrap_or(*sym))
                }
                _ => None,
            })
            .collect();
        fields.truncate(MAX_FIELDS);
        fields
    }

    /// The IPDL message's parameter a parameter is, if the function
    /// implements a message's `Recv` method.
    fn recv_param(&self, i: usize) -> Option<IpcParam> {
        self.recv_params.get(i).copied().flatten()
    }

    /// The IPDL message's parameter a call's argument is, if it's of a
    /// message's `Send` method.
    fn send_param(&self, callee: &Ustr, i: usize) -> Option<IpcParam> {
        self.ipdl?.sends.get(callee)?.get(i).copied().flatten()
    }

    /// The port a place is (for parameters, `this`, globals), or the ports
    /// it may alias (a local holding a reference or pointer to one); and an
    /// IPDL struct's field it's (a part of), assigned to through its mutable
    /// accessor (`x.f() = v`), or an IPDL message's parameter, wherever they
    /// are (see `IpdlSlots`).
    fn place_ports(&self, place: &Place) -> Vec<(Port, bool)> {
        let fields = self.path_fields(&place.path);
        let mut ports = match &place.root {
            Root::This => vec![(Port::This { fields }, false)],
            Root::Global(sym) => vec![(Port::Global { sym: *sym, fields }, false)],
            Root::Local(sym) => match self.f.param_index(place) {
                Some(i) => {
                    let mut ports = vec![];
                    if let Some(param) = self.recv_param(i) {
                        let fields = fields.clone();
                        ports.push((
                            Port::Global {
                                sym: param.sym,
                                fields,
                            },
                            false,
                        ));
                    }
                    ports.push((Port::Param { i, fields }, false));
                    ports
                }
                None => match self.locals.get(sym) {
                    Some(sources) => sources
                        .iter()
                        .map(|(port, derived)| (port.with_fields(&fields), *derived))
                        .collect(),
                    None => vec![],
                },
            },
        };
        if let Some(ipdl) = self.ipdl
            && let Some((k, field)) = place
                .path
                .iter()
                .enumerate()
                .find_map(|(k, step)| match step {
                    Step::Field(sym) => Some((k, ipdl.written_field(sym)?)),
                    _ => None,
                })
        {
            let fields = self.path_fields(&place.path[k + 1..]);
            ports.push((Port::Global { sym: field, fields }, false));
        }
        ports
    }

    fn val_sources(&self, val: &Val, out: &mut Sources) {
        match &val.atom {
            Atom::Read(place) | Atom::Addr(place) => {
                for (port, derived) in self.place_ports(place) {
                    out.insert((port, derived || val.derived));
                }
            }
            Atom::Call(id) => {
                let call = &self.f.calls[*id];
                if self.boring_calls[*id] {
                    return;
                }
                if let Some(field) = self.ipdl.and_then(|ipdl| ipdl.accessor(call)) {
                    // (An IPDL struct field's accessor reads the field: of
                    // the object it's called on, and wherever it is.)
                    out.insert((
                        Port::Global {
                            sym: field,
                            fields: vec![],
                        },
                        val.derived,
                    ));
                    if let Some(receiver) = &call.receiver {
                        let mut inner = Sources::new();
                        for v in &receiver.vals {
                            self.val_sources(v, &mut inner);
                        }
                        out.extend(inner.into_iter().filter_map(|(p, d)| match p {
                            Port::Param { .. } | Port::This { .. } | Port::Global { .. } => {
                                Some((p.with_fields(&[field]), d || val.derived))
                            }
                            _ => None,
                        }));
                    }
                    return;
                }
                if value_type_call(call).is_some() {
                    // (A value type's method's result is its object's data,
                    // and derived from its arguments'.)
                    if let Some(receiver) = &call.receiver {
                        for v in &receiver.vals {
                            let mut inner = Sources::new();
                            self.val_sources(v, &mut inner);
                            out.extend(inner.into_iter().map(|(p, d)| (p, d || val.derived)));
                        }
                    }
                    for arg in &call.args {
                        for v in &arg.vals {
                            let mut inner = Sources::new();
                            self.val_sources(v, &mut inner);
                            out.extend(inner.into_iter().map(|(p, _)| (p, true)));
                        }
                    }
                    return;
                }
                for callee in call.callee.iter().take(1) {
                    out.insert((
                        Port::Result {
                            callee: *callee,
                            i: None,
                        },
                        val.derived,
                    ));
                }
                if call.callee.is_empty() {
                    // (An unknown callee: its result comes from its
                    // arguments and object.)
                    for arg in call.args.iter().chain(call.receiver.iter()) {
                        for v in &arg.vals {
                            let mut inner = Sources::new();
                            self.val_sources(v, &mut inner);
                            out.extend(inner.into_iter().map(|(p, _)| (p, true)));
                        }
                    }
                }
            }
            Atom::Const(_) => {}
        }
    }

    fn sources(&self, vals: &[Val]) -> Sources {
        let mut out = Sources::new();
        for val in vals {
            self.val_sources(val, &mut out);
        }
        out
    }

    /// What locals hold, to a fixpoint (a few rounds; locals assigned from
    /// locals assigned later in loops need more, rarely).
    fn propagate_locals(&mut self) {
        for _ in 0..4 {
            let mut changed = false;
            for event in &self.events {
                let EventKind::Assign { place, vals, .. } = &event.kind else {
                    continue;
                };
                let Root::Local(sym) = &place.root else {
                    continue;
                };
                if self.f.param_index(place).is_some() || !place.path.is_empty() {
                    continue;
                }
                let sources = self.sources(vals);
                let entry = self.locals.entry(*sym).or_default();
                let before = entry.len();
                entry.extend(sources);
                changed |= entry.len() != before;
            }
            // (A local a value type's method writes, ex:
            // `list.AppendElement(x)`.)
            for (ci, call) in self.f.calls.iter().enumerate() {
                if self.boring_calls[ci] || value_type_call(call) != Some(true) {
                    continue;
                }
                let Some(place) = call.receiver.as_ref().and_then(|r| r.place.as_ref()) else {
                    continue;
                };
                let Root::Local(sym) = &place.root else {
                    continue;
                };
                if self.f.param_index(place).is_some() {
                    continue;
                }
                let mut sources = Sources::new();
                for arg in &call.args {
                    sources.extend(self.sources(&arg.vals));
                }
                let entry = self.locals.entry(*sym).or_default();
                let before = entry.len();
                entry.extend(sources);
                changed |= entry.len() != before;
            }
            // (A local an out-parameter call writes, ex: `Foo(&local)`.)
            for (ci, call) in self.f.calls.iter().enumerate() {
                if self.boring_calls[ci] {
                    continue;
                }
                let Some(callee) = call.callee.first() else {
                    continue;
                };
                for (ai, arg) in call.args.iter().enumerate() {
                    if !arg.addr {
                        continue;
                    }
                    let Some(place) = &arg.place else {
                        continue;
                    };
                    let Root::Local(sym) = &place.root else {
                        continue;
                    };
                    if self.f.param_index(place).is_some() || !place.path.is_empty() {
                        continue;
                    }
                    // (A sync IPDL message's reply, ex: `SendFoo(&reply)`.)
                    let reply = self.send_param(callee, ai).filter(|param| param.out);
                    let entry = self.locals.entry(*sym).or_default();
                    changed |= entry.insert((
                        Port::Result {
                            callee: *callee,
                            i: Some(ai),
                        },
                        false,
                    ));
                    if let Some(reply) = reply {
                        changed |= entry.insert((
                            Port::Global {
                                sym: reply.sym,
                                fields: vec![],
                            },
                            false,
                        ));
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// The constants the function returns or writes to ports (see
    /// `Summary::consts`).
    fn consts(&self) -> Vec<ConstFlow> {
        let mut consts: Vec<ConstFlow> = vec![];
        for event in &self.events {
            let (vals, tos) = match &event.kind {
                EventKind::Assign { place, vals, .. } => {
                    let is_plain_local = matches!(place.root, Root::Local(_))
                        && self.f.param_index(place).is_none()
                        && place.path.is_empty();
                    if is_plain_local {
                        continue;
                    }
                    let tos: Vec<Port> = self
                        .place_ports(place)
                        .into_iter()
                        .map(|(p, _)| p)
                        .collect();
                    (vals, tos)
                }
                EventKind::Return { vals } => (vals, vec![Port::Return]),
                _ => continue,
            };
            for val in vals {
                let Atom::Const(text) = &val.atom else {
                    continue;
                };
                for to in &tos {
                    let flow = ConstFlow {
                        text: text.clone(),
                        to: to.clone(),
                    };
                    if consts.len() < MAX_CONSTS && !consts.contains(&flow) {
                        consts.push(flow);
                    }
                }
            }
        }
        consts
    }

    /// The function's flows, or None if it has more than `MAX_FLOWS`.
    fn flows(&self) -> Option<BTreeSet<Flow>> {
        let mut flows = BTreeSet::new();
        let mut overflowed = false;
        // (A `Recv` implementation's parameter is its message's too (see
        // `place_ports`): a flow between one's parameter and another's
        // message's parameter is the same as those between the parameters and
        // between the message's.)
        let is_recv_param = |port: &Port| match port {
            Port::Param { i, .. } => self.recv_param(*i).is_some(),
            _ => false,
        };
        let is_message_param = |port: &Port| match port {
            Port::Global { sym, .. } => self.recv_params.iter().flatten().any(|p| p.sym == *sym),
            _ => false,
        };
        let mut add = |sources: &Sources, to: &Port| {
            for (from, derived) in sources {
                if overflowed {
                    return;
                }
                if from == to
                    || (is_recv_param(from) && is_message_param(to))
                    || (is_message_param(from) && is_recv_param(to))
                {
                    continue;
                }
                flows.insert(Flow {
                    from: from.clone(),
                    to: to.clone(),
                    derived: *derived,
                });
                overflowed = flows.len() > MAX_FLOWS;
            }
        };
        for event in &self.events {
            match &event.kind {
                EventKind::Assign { place, vals, .. } => {
                    let is_plain_local = matches!(place.root, Root::Local(_))
                        && self.f.param_index(place).is_none()
                        && place.path.is_empty();
                    if is_plain_local {
                        continue;
                    }
                    let sources = self.sources(vals);
                    for (to, _) in self.place_ports(place) {
                        add(&sources, &to);
                    }
                }
                EventKind::Return { vals } => {
                    let sources = self.sources(vals);
                    add(&sources, &Port::Return);
                }
                EventKind::Call(ci) => {
                    let call = &self.f.calls[*ci];
                    if let Some(ipdl) = self.ipdl {
                        // (An IPDL struct field's accessor is a read, or,
                        // assigned to, a write; see `val_sources` and
                        // `place_ports`.)
                        if ipdl.accessor(call).is_some() {
                            continue;
                        }
                        // (An IPDL struct's constructor writes its fields.)
                        if let Some(fields) = ipdl.constructor(call) {
                            for (arg, field) in call.args.iter().zip(fields) {
                                let to = Port::Global {
                                    sym: *field,
                                    fields: vec![],
                                };
                                add(&self.sources(&arg.vals), &to);
                            }
                        }
                    }
                    if let Some(writes) = value_type_call(call) {
                        // (A value type's mutator writes its object, ex:
                        // `mList.AppendElement(aItem)`.)
                        if writes
                            && let Some(place) =
                                call.receiver.as_ref().and_then(|r| r.place.as_ref())
                        {
                            let mut sources = Sources::new();
                            for arg in &call.args {
                                sources.extend(self.sources(&arg.vals));
                            }
                            for (to, _) in self.place_ports(place) {
                                add(&sources, &to);
                            }
                        }
                        continue;
                    }
                    let Some(callee) = call.callee.first() else {
                        continue;
                    };
                    for (ai, arg) in call.args.iter().enumerate() {
                        let sources = self.sources(&arg.vals);
                        add(
                            &sources,
                            &Port::Arg {
                                callee: *callee,
                                i: Some(ai),
                            },
                        );
                        // (An IPDL message's parameter, for its `Send`
                        // method; a reply comes back, below.)
                        let param = self.send_param(callee, ai);
                        if let Some(param) = param.filter(|param| !param.out) {
                            let to = Port::Global {
                                sym: param.sym,
                                fields: vec![],
                            };
                            add(&sources, &to);
                        }
                        // (What the callee writes through `&x`: `x`'s new
                        // value, for parameters and fields.)
                        if arg.addr
                            && let Some(place) = &arg.place
                            && !(place.root == Root::This && place.path.is_empty())
                        {
                            let mut result: Sources = [(
                                Port::Result {
                                    callee: *callee,
                                    i: Some(ai),
                                },
                                false,
                            )]
                            .into_iter()
                            .collect();
                            if let Some(reply) = param.filter(|param| param.out) {
                                let from = Port::Global {
                                    sym: reply.sym,
                                    fields: vec![],
                                };
                                result.insert((from, false));
                            }
                            if !matches!(place.root, Root::Local(_))
                                || self.f.param_index(place).is_some()
                            {
                                for (to, _) in self.place_ports(place) {
                                    add(&result, &to);
                                }
                            }
                        }
                    }
                    if let Some(receiver) = &call.receiver {
                        let sources = self.sources(&receiver.vals);
                        add(
                            &sources,
                            &Port::Arg {
                                callee: *callee,
                                i: None,
                            },
                        );
                    }
                }
                EventKind::Test { .. } => {}
            }
        }
        (!overflowed).then_some(flows)
    }
}

/// The most flows a function's summary has: more (ex: a minified bundle's
/// function, whose locals each carry most of its inputs, 1.7 million for
/// one of the newtab vendor bundle's) aren't a summary of anything, and were
/// most of firefox's summaries' 17.8 GB (66 of its 1.06 million functions
/// have more than 10000; 22 more than 100000).
pub const MAX_FLOWS: usize = 10_000;

/// A function's summary, without IPDL's (see `summarize_in`).
pub fn summarize(f: &FuncIr) -> Summary {
    summarize_in(f, &IpdlSlots::default(), vec![])
}

/// A function's summary, seeing through the code IPDL generates (see
/// `IpdlSlots`), given the methods it overrides (for messages' `Recv`
/// implementations; the summary's `overrides`).
pub fn summarize_in(f: &FuncIr, ipdl: &IpdlSlots, overrides: Vec<Ustr>) -> Summary {
    let mut builder = Builder::new(f, ipdl, &overrides);
    builder.propagate_locals();
    let (flows, flows_omitted) = match builder.flows() {
        Some(flows) => (flows.into_iter().collect(), false),
        None => (vec![], true),
    };
    let events = builder.events.len();
    let category = match flows_omitted {
        true => Category::Big,
        false => categorize(&flows, events),
    };
    let consts = match category {
        Category::Getter | Category::Setter | Category::Trivial => builder.consts(),
        _ => vec![],
    };
    Summary {
        sym: f.sym,
        pretty: f.pretty.clone(),
        path: f.path.clone(),
        line: f.line,
        category,
        events,
        flows,
        flows_omitted,
        overrides,
        consts,
    }
}

/// What kind of function flows say it is.
fn categorize(flows: &[Flow], events: usize) -> Category {
    let field_out = |p: &Port| matches!(p, Port::This { fields } if !fields.is_empty());
    let field_in = |p: &Port| matches!(p, Port::This { fields } | Port::Param { fields, .. } if !fields.is_empty());
    if flows.is_empty() {
        return if events <= 2 {
            Category::Trivial
        } else if events <= 12 {
            Category::Small
        } else {
            Category::Big
        };
    }
    if events <= 3
        && flows
            .iter()
            .all(|fl| fl.to == Port::Return && field_in(&fl.from))
    {
        return Category::Getter;
    }
    if events <= 3
        && flows
            .iter()
            .all(|fl| field_out(&fl.to) && matches!(fl.from, Port::Param { .. }))
    {
        return Category::Setter;
    }
    if events <= 12 {
        Category::Small
    } else {
        Category::Big
    }
}

/// Classes whose fields are a value's representation rather than data
/// structures' (strings, arrays, smart pointers, containers): flows into or
/// out of their fields are their owners' fields' (`this->mName.mData` is
/// `mName`), and their own methods' flows between their fields aren't
/// edges.
pub fn is_value_type_class(class: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "T_nsTSubstring",
        "T_nsTString",
        "T_nsTAutoString",
        "T_nsTLiteralString",
        "T_nsTDependentString",
        "T_nsTDependentSubstring",
        "T_nsTArray",
        "T_AutoTArray",
        "T_CopyableTArray",
        "T_FallibleTArray",
        "T_nsTHashtable",
        "T_nsBaseHashtable",
        "T_nsTHashMap",
        "T_nsRefPtrHashtable",
        "T_nsInterfaceHashtable",
        "T_nsClassHashtable",
        "T_RefPtr",
        "T_nsCOMPtr",
        "T_nsMainThreadPtrHandle",
        "T_mozilla::UniquePtr",
        "T_mozilla::Maybe",
        "T_mozilla::Variant",
        "T_mozilla::Atomic",
        "T_mozilla::Span",
        "T_mozilla::Vector",
        "T_mozilla::HashMap",
        "T_mozilla::HashSet",
        "T_mozilla::LinkedList",
        "T_mozilla::Result",
        "T_mozilla::NotNull",
        "T_mozilla::detail::",
        "T_std::",
    ];
    PREFIXES.iter().any(|prefix| class.starts_with(prefix))
}

/// The field (or global) a port is, for edges between fields: its deepest
/// field that isn't in a value type (see `is_value_type_class`), or a
/// global's symbol.
pub fn port_node(port: &Port) -> Option<Ustr> {
    let fields = match port {
        Port::Param { fields, .. } | Port::This { fields } => fields,
        Port::Global { sym, fields } => {
            if fields.is_empty() {
                return is_data_symbol(sym).then_some(*sym);
            }
            fields
        }
        _ => return None,
    };
    let fields: Vec<Ustr> = fields
        .iter()
        .copied()
        .filter(|f| is_data_symbol(f))
        .collect();
    let mut node = None;
    for field in &fields {
        match field_class(field) {
            Some(class) if is_value_type_class(&class) => break,
            _ => node = Some(*field),
        }
    }
    node.or_else(|| fields.first().copied())
}

/// Whether a symbol names a field or global worth an edge: not a JS
/// property name (`#name`, any object's), or a local the frontend took for a
/// global (js-analyze's `<file>-<n>`, a `name:` placeholder).
fn is_data_symbol(sym: &str) -> bool {
    !(sym.starts_with('#')
        || sym.starts_with("name:")
        || sym.starts_with("V_")
        || sym.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

/// Whether a port is the function's own object's field (`this->mFoo`, and
/// value types' parts of it), rather than another object's (`this->mFoo->mBar`,
/// `aData->mBar`).
fn is_own_field(port: &Port) -> bool {
    match port {
        Port::This { fields } => {
            let first = fields.first().copied();
            first.is_some() && port_node(port) == first
        }
        _ => false,
    }
}

/// An edge between two fields (or globals), or between a function and
/// another object's field (or a global) it writes or reads: data in one
/// flows into the other, in the functions `via` (some of them), directly or
/// through the functions they call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldEdge {
    pub from: Ustr,
    pub to: Ustr,
    /// How many functions it's in.
    pub count: u32,
    /// Some of those functions.
    pub via: Vec<Ustr>,
    /// Whether the data is derived (ex: `a + 1`) in all of them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
}

/// How many rounds of composing summaries through calls (each composes one
/// more level of calls), how many flows a function's composed summary keeps,
/// and how many implementations of a virtual method its calls compose with.
const COMPOSE_ROUNDS: usize = 3;
const MAX_IMPLEMENTATIONS: usize = 8;
const MAX_COMPOSED_FLOWS: usize = 400;
const MAX_VIA: usize = 4;

/// Compose the summaries through their calls (see `compose_round`), and the
/// edges between fields that the composed summaries' flows are; an IPDL
/// struct field's generated member is the field (see `IpdlSlots`), whose
/// edges in the generated code are left out.
pub fn field_edges(summaries: &[Summary], ipdl: &IpdlSlots) -> Vec<FieldEdge> {
    // (A callee's summary, or, for a virtual method, its implementations'.)
    let mut by_sym: HashMap<Ustr, Vec<usize>> = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| (s.sym, vec![i]))
        .collect();
    // (A virtual method with more implementations than that is too generic
    // to say where its arguments go, ex: `nsIRunnable::Run`.)
    let mut implementations: HashMap<Ustr, Vec<usize>> = HashMap::new();
    for (i, summary) in summaries.iter().enumerate() {
        for base in &summary.overrides {
            implementations.entry(*base).or_default().push(i);
        }
    }
    for (base, impls) in implementations {
        if impls.len() <= MAX_IMPLEMENTATIONS {
            by_sym.entry(base).or_default().extend(impls);
        }
    }
    let mut composed: Vec<BTreeSet<Flow>> = summaries
        .iter()
        .map(|s| s.flows.iter().cloned().collect())
        .collect();
    for _ in 0..COMPOSE_ROUNDS {
        let next: Vec<BTreeSet<Flow>> = (0..summaries.len())
            .map(|i| compose_round(&composed[i], &composed, &by_sym))
            .collect();
        let changed = next.iter().zip(&composed).any(|(a, b)| a.len() != b.len());
        composed = next;
        if !changed {
            break;
        }
    }

    let mut edges: HashMap<(Ustr, Ustr), FieldEdge> = HashMap::new();
    for (summary, flows) in summaries.iter().zip(&composed) {
        // (A value type's own methods move its representation around.)
        if summary_class(summary).is_some_and(|class| is_value_type_class(&class)) {
            continue;
        }
        // (An accessor's reads and writes are its callers'; the composed
        // summaries have them.)
        let is_accessor = matches!(
            summary.category,
            Category::Getter | Category::Setter | Category::Trivial
        );
        let generated = is_generated_path(&summary.path);
        let node = |port: &Port| {
            port_node(port).map(|node| ipdl.members.get(&node).copied().unwrap_or(node))
        };
        let mut seen: BTreeSet<(Ustr, Ustr)> = BTreeSet::new();
        for flow in flows {
            // (Between fields; or into another object's field from the
            // function's other data, which the function writes; or out of
            // another object's field into the function's other outputs, which
            // it reads.)
            let (from, to) = match (node(&flow.from), node(&flow.to)) {
                (Some(from), Some(to)) => (from, to),
                (None, Some(to)) if !is_accessor && !is_own_field(&flow.to) => (summary.sym, to),
                (Some(from), None) if !is_accessor && !is_own_field(&flow.from) => {
                    (from, summary.sym)
                }
                _ => continue,
            };
            if from == to || !seen.insert((from, to)) {
                continue;
            }
            // (IPDL's serializers and dispatch read and write every field and
            // parameter.)
            if generated && (is_ipdl_sym(&from) || is_ipdl_sym(&to)) {
                continue;
            }
            let edge = edges.entry((from, to)).or_insert_with(|| FieldEdge {
                from,
                to,
                count: 0,
                via: vec![],
                derived: true,
            });
            edge.count += 1;
            if edge.via.len() < MAX_VIA {
                edge.via.push(summary.sym);
            }
            edge.derived &= flow.derived;
        }
    }
    let mut edges: Vec<FieldEdge> = edges.into_values().collect();
    edges.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
    edges
}

/// The class a function's summary is of, by its fields (the class of a
/// field of `this` it uses), for value types' methods.
fn summary_class(summary: &Summary) -> Option<Ustr> {
    summary.flows.iter().find_map(|flow| {
        [&flow.from, &flow.to]
            .into_iter()
            .find_map(|port| match port {
                Port::This { fields } => fields.first().and_then(|f| field_class(f)),
                _ => None,
            })
    })
}

/// One more level of a function's flows through its calls: what its calls
/// return (`Result`) comes from what the callees' composed summaries return,
/// in the caller's terms (the callee's parameters are the caller's
/// arguments, its object the caller's receiver), and what it passes its
/// calls (`Arg`) goes where the callees' composed summaries put their
/// parameters.
fn compose_round(
    flows: &BTreeSet<Flow>,
    composed: &[BTreeSet<Flow>],
    by_sym: &HashMap<Ustr, Vec<usize>>,
) -> BTreeSet<Flow> {
    let mut out = flows.clone();
    // (What the function passes each call: its arguments' and receivers'
    // sources, by callee.)
    let mut args: HashMap<(Ustr, Option<usize>), Vec<(Port, bool)>> = HashMap::new();
    for flow in flows {
        if let Port::Arg { callee, i } = &flow.to {
            args.entry((*callee, *i))
                .or_default()
                .push((flow.from.clone(), flow.derived));
        }
    }
    // (Where what each call returns goes, for constructors' objects.)
    let mut results: HashMap<Ustr, Vec<(Port, bool)>> = HashMap::new();
    for flow in flows {
        if let Port::Result { callee, i: None } = &flow.from {
            results
                .entry(*callee)
                .or_default()
                .push((flow.to.clone(), flow.derived));
        }
    }
    // A callee's port in the caller's terms.
    let in_caller = |callee: Ustr, port: &Port| -> Vec<(Port, bool)> {
        match port {
            // (A constructor's object is where its result goes.)
            Port::This { fields } if !args.contains_key(&(callee, None)) => results
                .get(&callee)
                .into_iter()
                .flatten()
                .map(|(p, d)| (p.with_fields(fields), *d))
                .collect(),
            Port::Param { i, fields } => args
                .get(&(callee, Some(*i)))
                .into_iter()
                .flatten()
                .map(|(p, d)| (p.with_fields(fields), *d))
                .collect(),
            Port::This { fields } => args
                .get(&(callee, None))
                .into_iter()
                .flatten()
                .map(|(p, d)| (p.with_fields(fields), *d))
                .collect(),
            Port::Global { .. } => vec![(port.clone(), false)],
            _ => vec![],
        }
    };
    for flow in flows {
        if out.len() >= MAX_COMPOSED_FLOWS {
            break;
        }
        // (What a call returns flows where the call's result goes.)
        if let Port::Result { callee, i: None } = &flow.from
            && let Some(callees) = by_sym.get(callee)
        {
            for callee_flow in callees.iter().flat_map(|&ci| &composed[ci]) {
                if callee_flow.to != Port::Return {
                    continue;
                }
                for (from, derived) in in_caller(*callee, &callee_flow.from) {
                    out.insert(Flow {
                        from,
                        to: flow.to.clone(),
                        derived: derived || flow.derived || callee_flow.derived,
                    });
                }
            }
        }
        // (What's passed to a call goes where the callee puts it.)
        if let Port::Arg {
            callee,
            i: Some(ai),
        } = &flow.to
            && let Some(callees) = by_sym.get(callee)
        {
            for callee_flow in callees.iter().flat_map(|&ci| &composed[ci]) {
                let Port::Param { i, fields } = &callee_flow.from else {
                    continue;
                };
                if i != ai {
                    continue;
                }
                if matches!(callee_flow.to, Port::Return | Port::Arg { .. }) {
                    continue;
                }
                for (to, _) in in_caller(*callee, &callee_flow.to) {
                    out.insert(Flow {
                        from: flow.from.with_fields(fields),
                        to,
                        derived: flow.derived || callee_flow.derived,
                    });
                }
            }
        }
    }
    out
}

/// The class a field symbol is a member of (ex: `T_mozilla::dom::Foo` for
/// `F_<T_mozilla::dom::Foo>_mBar`), for C++'s field symbols.
pub fn field_class(field: &str) -> Option<Ustr> {
    let rest = field.strip_prefix("F_<")?;
    let (class, _) = rest.rsplit_once(">_")?;
    Some(ustr(class))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_class() {
        assert_eq!(
            field_class("F_<T_mozilla::dom::Foo>_mBar").as_deref(),
            Some("T_mozilla::dom::Foo")
        );
        assert_eq!(
            field_class("F_<T_Pair<A, B>>_mFirst").as_deref(),
            Some("T_Pair<A, B>")
        );
        assert_eq!(field_class("V_1234_0"), None);
    }

    #[test]
    fn test_ipdl_slots() {
        // (As ipdl-analyze writes them, for a struct, a union's variant, and a
        // sync message with a reply.)
        let records: Vec<serde_json::Value> = [
            r#"{"kind":"struct","sym":"IPDL_S","fields":[{"sym":"IPDL_S_a"},{"sym":"IPDL_S_b"}],
                "bindingSlots":[{"slotKind":"class","sym":"T_S"},{"slotKind":"constructor","sym":"_ZS_ctor"}]}"#,
            r#"{"kind":"field","sym":"IPDL_S_a","parentsym":"IPDL_S","ipdlIndex":0,
                "bindingSlots":[{"slotKind":"getter","sym":"_ZNK_a"},{"slotKind":"setter","sym":"_ZN_a"},
                                {"slotKind":"member","sym":"F_<T_S>_a_"}]}"#,
            r#"{"kind":"field","sym":"IPDL_U_int","parentsym":"IPDL_U","ipdlIndex":0,
                "bindingSlots":[{"slotKind":"constructor","sym":"_ZU_ctor_int"},{"slotKind":"getter","sym":"_ZNK_get_int"}]}"#,
            r#"{"kind":"ipc","sym":"IPC_P_M",
                "bindingSlots":[{"slotKind":"send","sym":"_ZSendM"},{"slotKind":"recv","sym":"_ZRecvM"}]}"#,
            r#"{"kind":"parameter","sym":"IPC_P_M__reply","parentsym":"IPC_P_M","ipdlIndex":1,"ipdlOut":true}"#,
            r#"{"kind":"parameter","sym":"IPC_P_M__arg","parentsym":"IPC_P_M","ipdlIndex":0,"ipdlOut":false}"#,
        ]
        .iter()
        .map(|r| serde_json::from_str(r).unwrap())
        .collect();
        let slots = IpdlSlots::from_structured(&records);
        let u = |s: &str| ustr(s);
        assert_eq!(slots.accessors.get(&u("_ZN_a")), Some(&u("IPDL_S_a")));
        assert_eq!(
            slots.accessors.get(&u("_ZNK_get_int")),
            Some(&u("IPDL_U_int"))
        );
        assert_eq!(slots.members.get(&u("F_<T_S>_a_")), Some(&u("IPDL_S_a")));
        assert_eq!(
            slots.constructors.get(&u("_ZS_ctor")),
            Some(&vec![u("IPDL_S_a"), u("IPDL_S_b")])
        );
        assert_eq!(
            slots.constructors.get(&u("_ZU_ctor_int")),
            Some(&vec![u("IPDL_U_int")])
        );
        let params = vec![
            Some(IpcParam {
                sym: u("IPC_P_M__arg"),
                out: false,
            }),
            Some(IpcParam {
                sym: u("IPC_P_M__reply"),
                out: true,
            }),
        ];
        assert_eq!(slots.sends.get(&u("_ZSendM")), Some(&params));
        assert_eq!(slots.recvs.get(&u("_ZRecvM")), Some(&params));
    }

    #[test]
    fn test_boring() {
        assert!(is_boring_macro("MOZ_LOG"));
        assert!(is_boring_macro("EXPECT_EQ"));
        assert!(is_boring_macro("WORKER_LOG"));
        assert!(!is_boring_macro("NS_IMPL_ISUPPORTS"));
    }
}
