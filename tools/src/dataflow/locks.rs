//! Mutexes held by scoped lock guards (RAII): which types are guards (see
//! `guard_kind`), for the frontend's lock regions (see `ir::LockRegion`).
//!
//! Gecko's guards are classes marked `MOZ_SCOPED_CAPABILITY` (clang's
//! thread-safety analysis' scoped capabilities), whose constructors acquire
//! (or release) the mutex they're given, and whose destructors release (or
//! reacquire) it: `MutexAutoLock` (a `detail::BaseAutoLock<Mutex&>`, as are
//! `StaticMutexAutoLock` and `OffTheBooksMutexAutoLock`), `MonitorAutoLock`,
//! `ReentrantMonitorAutoEnter`, `RecursiveMutexAutoLock`, and RW locks'
//! `AutoReadLock`/`AutoWriteLock`; and their releasing counterparts
//! (`MutexAutoUnlock`, …).  The standard library's (in third-party code) too.
//! And a `DataMutex`'s `AutoLock`, which its `Lock()` returns (`auto lock =
//! mData.Lock();`), holding the DataMutex (the call's receiver) until its
//! destructor.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ustr::Ustr;

use super::ir::{Atom, CallKind, EventKind, FuncIr, LockKind, LockRegion, Place, Root, Step, Val};

/// The classes (as their records' type symbols name them, without `T_`) of
/// guards which hold their mutexes, and of those which release them.
pub const ACQUIRING_GUARDS: &[&str] = &[
    "mozilla::detail::BaseAutoLock",
    "mozilla::detail::ReleasableBaseAutoLock",
    "mozilla::detail::BaseAutoTryLock",
    "mozilla::MonitorAutoLockBase",
    "mozilla::ReleasableMonitorAutoLockBase",
    "mozilla::ReentrantMonitorAutoEnter",
    "mozilla::RecursiveMutexAutoLock",
    "mozilla::BaseAutoReadLock",
    "mozilla::BaseAutoWriteLock",
    "mozilla::BaseAutoTryReadLock",
    "mozilla::BaseAutoTryWriteLock",
    // (A DataMutex's, which its `Lock()` and `ConstLock()` return: `auto lock
    // = mData.Lock();`, of the call's receiver.)
    "mozilla::DataMutexBase::AutoLockBase",
    "std::lock_guard",
    "std::unique_lock",
    "std::scoped_lock",
    "std::shared_lock",
];
pub const RELEASING_GUARDS: &[&str] = &[
    "mozilla::detail::BaseAutoUnlock",
    "mozilla::MonitorAutoUnlockBase",
    "mozilla::ReentrantMonitorAutoExit",
    "mozilla::RecursiveMutexAutoUnlock",
];

/// Their aliases' names, for records without type symbols (by their types'
/// pretty names, without `const`, references and template arguments).
const ACQUIRING_ALIASES: &[&str] = &[
    "mozilla::MutexAutoLock",
    "mozilla::StaticMutexAutoLock",
    "mozilla::OffTheBooksMutexAutoLock",
    "mozilla::ReleasableMutexAutoLock",
    "mozilla::MutexAutoTryLock",
    "mozilla::MonitorAutoLock",
    "mozilla::ReleasableMonitorAutoLock",
    "mozilla::AutoReadLock",
    "mozilla::AutoWriteLock",
    "mozilla::StaticAutoReadLock",
    "mozilla::StaticAutoWriteLock",
];
const RELEASING_ALIASES: &[&str] = &[
    "mozilla::MutexAutoUnlock",
    "mozilla::StaticMutexAutoUnlock",
    "mozilla::OffTheBooksMutexAutoUnlock",
    "mozilla::MonitorAutoUnlock",
];

/// A type's pretty name without `const`, `&`, `*` and template arguments.
fn bare_type(ty: &str) -> String {
    let ty = ty.split('<').next().unwrap_or(ty);
    ty.replace("const ", "")
        .replace(['&', '*'], "")
        .trim()
        .to_string()
}

/// Whether a type (by its record's type symbol, else its pretty name) is a
/// lock guard, and which kind: one holding its mutex, or releasing it.
pub fn guard_kind(type_sym: Option<&str>, type_pretty: Option<&str>) -> Option<LockKind> {
    let in_list = |list: &[&str], name: &str| list.contains(&name);
    if let Some(class) = type_sym.and_then(|sym| sym.strip_prefix("T_")) {
        if in_list(ACQUIRING_GUARDS, class) {
            return Some(LockKind::Acquire);
        }
        if in_list(RELEASING_GUARDS, class) {
            return Some(LockKind::Release);
        }
    }
    let ty = bare_type(type_pretty?);
    // (Without a namespace, as a type may be spelled: `MutexAutoLock`.)
    let matches = |list: &[&str]| {
        list.iter()
            .any(|name| *name == ty || name.rsplit("::").next() == Some(ty.as_str()))
    };
    if matches(ACQUIRING_ALIASES) || matches(ACQUIRING_GUARDS) {
        Some(LockKind::Acquire)
    } else if matches(RELEASING_ALIASES) || matches(RELEASING_GUARDS) {
        Some(LockKind::Release)
    } else {
        None
    }
}

/// Whether a parameter's type is a reference to a holding guard: a proof
/// that a mutex is held (`const MutexAutoLock& aProofOfLock`).
pub fn is_proof_type(type_sym: Option<&str>, type_pretty: Option<&str>) -> bool {
    type_pretty.is_some_and(|ty| ty.trim_end().ends_with('&'))
        && guard_kind(type_sym, type_pretty) == Some(LockKind::Acquire)
}

/// Where a function holds a mutex (see `held_regions`): an acquiring
/// guard's or a proof parameter's region (see `LockRegion`), or a whole body
/// run with it held (see `held_body`), but where the releasing guards of its
/// mutex in it release it, and the lambdas made in it; and what runs there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeldRegion {
    pub kind: LockKind,
    pub guard: String,
    /// The mutex's symbol (see `LockRegion::mutex_sym`; none for a proof's,
    /// which its callers say), and as the function spells it.
    pub mutex: Option<Ustr>,
    pub mutex_text: String,
    /// For a proof, its parameter's index; and the local holding it (a
    /// guard's local, or the proof parameter), which calls pass on as a proof.
    pub param: Option<usize>,
    pub local: Option<Ustr>,
    pub line: u32,
    pub end_line: u32,
    /// Its parts which aren't held: released (by unlocks), and lambdas
    /// made in it, as lines (first, last).
    pub released: Vec<(u32, u32)>,
    pub lambdas: Vec<(u32, u32)>,
    /// The lines of what runs with it held.
    pub held_lines: Vec<u32>,
    /// The calls made with it held (but lock guards' constructions).
    pub calls: Vec<HeldCall>,
    /// The fields accessed with it held (but its mutex), by symbol.
    pub fields: Vec<FieldAccess>,
    /// The other mutexes acquired with it held.
    pub locks: Vec<NestedLock>,
}

/// A call made with a mutex held: its line, its callee, and the IR's call
/// (`id`, for its arguments), and which of its arguments pass the proof that
/// the mutex is held on (the region's `local`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeldCall {
    pub id: usize,
    pub line: u32,
    pub callee: Vec<Ustr>,
    pub callee_pretty: String,
    pub kind: CallKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proof_args: Vec<usize>,
}

/// A field accessed with a mutex held: the lines it's read on, written on,
/// and has methods called on (`mQueue.Push(x)`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldAccess {
    pub sym: Ustr,
    pub name: String,
    pub reads: Vec<u32>,
    pub writes: Vec<u32>,
    pub calls: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NestedLock {
    pub mutex: Option<Ustr>,
    pub mutex_text: String,
    pub guard: String,
    pub line: u32,
}

/// The fields in a place (`this->mA.mB`: mA and mB), C++'s (`F_…`).
/// The state in a place: its global, if it's in one (ex: a static
/// variable), and its fields (`this->mA.mB`: mA and mB; C++'s, `F_…`).
fn place_fields(place: &Place) -> impl Iterator<Item = Ustr> + '_ {
    let global = match &place.root {
        Root::Global(sym) => Some(*sym),
        _ => None,
    };
    global
        .into_iter()
        .chain(place.path.iter().filter_map(|step| match step {
            Step::Field(sym) if sym.starts_with("F_") => Some(*sym),
            _ => None,
        }))
}

/// The lambdas (of `lambdas`, by index) which the function calls in the
/// events `start..end` (`later()`, of a local initialized with a lambda),
/// whose events run there, rather than later.
fn called_lambdas(
    ir: &FuncIr,
    lambdas: &[(usize, usize)],
    start: usize,
    end: usize,
) -> BTreeSet<usize> {
    // (The locals initialized with lambdas, by name, and the lambdas' lines.)
    let mut locals: BTreeMap<String, u32> = BTreeMap::new();
    for event in &ir.events {
        if let EventKind::Assign { place, vals, .. } = &event.kind
            && place.path.is_empty()
            && let Root::Local(sym) = &place.root
            && let Some(line) = vals.iter().find_map(|val| match &val.atom {
                Atom::Const(text) => text.strip_prefix("lambda@")?.parse::<u32>().ok(),
                _ => None,
            })
            && let Some(name) = ir.names.get(sym)
        {
            locals.insert(name.clone(), line);
        }
    }
    let mut called = BTreeSet::new();
    for event in ir
        .events
        .get(start..end.min(ir.events.len()))
        .unwrap_or_default()
    {
        let EventKind::Call(c) = &event.kind else {
            continue;
        };
        let Some(call) = ir.calls.get(*c) else {
            continue;
        };
        let Some(&line) = locals.get(&call.callee_pretty) else {
            continue;
        };
        if !call.callee.is_empty() {
            continue;
        }
        if let Some(i) = lambdas
            .iter()
            .position(|&(s, _)| ir.events.get(s).is_some_and(|e| e.line == line))
        {
            called.insert(i);
        }
    }
    called
}

/// The lines of some of a function's events: (first, last).
fn event_lines(ir: &FuncIr, start: usize, end: usize) -> Option<(u32, u32)> {
    let events = ir.events.get(start..end.min(ir.events.len()))?;
    let first = events.iter().map(|e| e.line).min()?;
    let last = events.iter().map(|e| e.end_line.max(e.line)).max()?;
    Some((first, last))
}

/// The function's lambdas' events (`start..end`), the outermost ones: its
/// events' runs in lambdas.
fn lambda_runs(ir: &FuncIr) -> Vec<(usize, usize)> {
    let mut runs = vec![];
    let mut start = None;
    for (i, event) in ir.events.iter().enumerate() {
        match (event.in_lambda, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, ir.events.len()));
    }
    runs
}

/// What a held region (see `HeldRegion`) needs to be analyzed: what it is,
/// and its events (`start..end`), its lambdas', and the releases in it.
struct Span<'r> {
    kind: LockKind,
    guard: String,
    mutex: Option<Ustr>,
    mutex_text: String,
    param: Option<usize>,
    local: Option<Ustr>,
    start: usize,
    end: usize,
    line: u32,
    end_line: u32,
    lambdas: Vec<(usize, usize)>,
    releases: Vec<&'r LockRegion>,
}

/// What runs in a held region with its mutex held (see `HeldRegion`).
fn analyze(ir: &FuncIr, mut span: Span) -> HeldRegion {
    let guard_calls: BTreeSet<usize> = ir.locks.iter().filter_map(|l| l.call).collect();
    let mutex = span.mutex;
    // (The lambdas called in it run with it held.)
    let called = called_lambdas(ir, &span.lambdas, span.start, span.end);
    span.lambdas = span
        .lambdas
        .iter()
        .enumerate()
        .filter(|(i, _)| !called.contains(i))
        .map(|(_, range)| *range)
        .collect();
    let held = |e: usize| {
        !span.lambdas.iter().any(|&(s, end)| s <= e && e < end)
            && !span.releases.iter().any(|l| l.start <= e && e < l.end)
    };
    let mut held_lines = BTreeSet::new();
    let mut calls = vec![];
    let mut fields: BTreeMap<Ustr, FieldAccess> = BTreeMap::new();
    let mut access = |sym: Ustr, line: u32, how: u8| {
        if Some(sym) == mutex {
            return;
        }
        let field = fields.entry(sym).or_insert_with(|| FieldAccess {
            sym,
            name: ir
                .names
                .get(&sym)
                .cloned()
                .unwrap_or_else(|| super::ir::sym_short_name(&sym)),
            ..Default::default()
        });
        let lines = match how {
            0 => &mut field.reads,
            1 => &mut field.writes,
            _ => &mut field.calls,
        };
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    };
    let reads = |vals: &[Val], line: u32, access: &mut dyn FnMut(Ustr, u32, u8)| {
        for val in vals {
            if let Atom::Read(place) | Atom::Addr(place) = &val.atom {
                for sym in place_fields(place) {
                    access(sym, line, 0);
                }
            }
        }
    };
    // (A place's last field, how it's accessed; the ones it's reached
    // through, read.)
    let path_access = |place: &Place, line: u32, how: u8, access: &mut dyn FnMut(Ustr, u32, u8)| {
        let path: Vec<Ustr> = place_fields(place).collect();
        if let Some((last, through)) = path.split_last() {
            for sym in through {
                access(*sym, line, 0);
            }
            access(*last, line, how);
        }
    };
    for e in span.start..span.end.min(ir.events.len()) {
        if !held(e) {
            continue;
        }
        let event = &ir.events[e];
        held_lines.insert(event.line);
        match &event.kind {
            EventKind::Assign { place, vals, .. } => {
                path_access(place, event.line, 1, &mut access);
                reads(vals, event.line, &mut access);
            }
            EventKind::Return { vals } | EventKind::Test { vals } => {
                reads(vals, event.line, &mut access)
            }
            EventKind::Call(c) => {
                if guard_calls.contains(c) {
                    continue;
                }
                let Some(call) = ir.calls.get(*c) else {
                    continue;
                };
                let proof_args = call
                    .args
                    .iter()
                    .enumerate()
                    .filter(|(_, arg)| {
                        span.local.is_some_and(|local| {
                            arg.place
                                .as_ref()
                                .is_some_and(|p| p.path.is_empty() && p.root == Root::Local(local))
                        })
                    })
                    .map(|(i, _)| i)
                    .collect();
                calls.push(HeldCall {
                    id: call.id,
                    line: call.line,
                    callee: call.callee.clone(),
                    callee_pretty: call.callee_pretty.clone(),
                    kind: call.kind,
                    proof_args,
                });
                for arg in &call.args {
                    reads(&arg.vals, event.line, &mut access);
                }
                // (A method called on a field: `mQueue.Push(x)`.)
                if let Some(receiver) = &call.receiver
                    && let Some(place) = &receiver.place
                {
                    path_access(place, event.line, 2, &mut access);
                }
            }
        }
    }
    let locks = ir
        .locks
        .iter()
        .filter(|l| {
            l.kind == LockKind::Acquire
                && l.start > span.start
                && l.start < span.end
                && (l.mutex_sym() != mutex || mutex.is_none())
                // (Made with it held.)
                && held(l.start - 1)
        })
        .map(|l| NestedLock {
            mutex: l.mutex_sym(),
            mutex_text: match &l.mutex {
                Some(place) => ir.place_name(place),
                None => l.mutex_text.clone(),
            },
            guard: l.guard.clone(),
            line: l.line,
        })
        .collect();
    HeldRegion {
        kind: span.kind,
        guard: span.guard,
        mutex,
        mutex_text: span.mutex_text,
        param: span.param,
        local: span.local,
        line: span.line,
        end_line: span.end_line,
        released: span.releases.iter().map(|l| (l.line, l.end_line)).collect(),
        lambdas: span
            .lambdas
            .iter()
            .filter_map(|&(start, end)| event_lines(ir, start, end))
            .collect(),
        held_lines: held_lines.into_iter().collect(),
        calls,
        fields: fields.into_values().collect(),
        locks,
    }
}

/// The regions where a function holds mutexes (see `HeldRegion`): each
/// acquiring guard's and proof parameter's region, with what runs in it with
/// the mutex held.
pub fn held_regions(ir: &FuncIr) -> Vec<HeldRegion> {
    let mut out = vec![];
    for region in &ir.locks {
        if region.kind == LockKind::Release {
            continue;
        }
        let mutex = region.mutex_sym();
        // (Its mutex's unlocks in it; a proof's, any's.)
        let releases: Vec<&LockRegion> = ir
            .locks
            .iter()
            .filter(|l| {
                l.kind == LockKind::Release
                    && region.start <= l.start
                    && l.start < region.end
                    && (region.kind == LockKind::Proof
                        || l.mutex_sym() == mutex
                        || (mutex.is_none() && l.mutex_text == region.mutex_text))
            })
            .collect();
        out.push(analyze(
            ir,
            Span {
                kind: region.kind,
                guard: region.guard.clone(),
                mutex,
                mutex_text: match &region.mutex {
                    Some(place) => ir.place_name(place),
                    None => region.mutex_text.clone(),
                },
                param: region.param,
                local: region.local,
                start: region.start,
                end: region.end,
                line: region.line,
                end_line: region.end_line,
                lambdas: region.lambdas.clone(),
                releases,
            },
        ));
    }
    out
}

/// A function's whole body run with `mutex` held (ex: one called with it
/// held), as a held region (of kind `Proof`, without a parameter): but its
/// unlocks of the mutex, and the lambdas it makes.
pub fn held_body(ir: &FuncIr, mutex: Option<Ustr>, mutex_text: &str) -> HeldRegion {
    let releases: Vec<&LockRegion> = ir
        .locks
        .iter()
        .filter(|l| l.kind == LockKind::Release && mutex.is_some() && l.mutex_sym() == mutex)
        .collect();
    analyze(
        ir,
        Span {
            kind: LockKind::Proof,
            guard: String::new(),
            mutex,
            mutex_text: mutex_text.to_string(),
            param: None,
            local: None,
            start: 0,
            end: ir.events.len(),
            line: ir.line,
            end_line: ir.end_line,
            lambdas: lambda_runs(ir),
            releases,
        },
    )
}

/// The class and the name of a field's symbol
/// (`F_<T_mozilla::dom::Worker>_mQueue`: "mozilla::dom::Worker", "mQueue"),
/// or else none and its short name.
pub fn field_parts(sym: &str) -> (String, String) {
    match sym
        .strip_prefix("F_<T_")
        .and_then(|rest| rest.split_once(">_"))
    {
        Some((class, name)) => (class.to_string(), name.to_string()),
        None => (String::new(), super::ir::sym_short_name(sym)),
    }
}

/// How a mutex is named for people (ex: "Worker::mMutex"): its field's
/// class's last name and its name, else (a global, or an expression) as the
/// code spells it.
pub fn mutex_name(sym: Option<&str>, text: &str) -> String {
    match sym {
        Some(sym) if sym.starts_with("F_") => {
            let (class, field) = field_parts(sym);
            let class = class.rsplit("::").next().unwrap_or(&class).to_string();
            match class.is_empty() {
                true => field,
                false => format!("{}::{}", class, field),
            }
        }
        _ => text.to_string(),
    }
}

/// Where a file's mutexes are held, as lines (see `file_locks`), for search
/// results' "Lock held" context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileLocks {
    pub path: String,
    pub locks: Vec<LineLock>,
}

/// A region holding a mutex (see `HeldRegion`), as lines: the ranges held
/// (first, last; its lines but its unlocks' and its lambdas'), the mutex,
/// and its name (see `mutex_name`; a proof's, its parameter's).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineLock {
    pub held: Vec<(u32, u32)>,
    pub mutex: Option<Ustr>,
    pub name: String,
    pub kind: LockKind,
}

impl FileLocks {
    /// The names of the mutexes held at a line, the outer ones (the earlier
    /// regions) first.
    pub fn held_at(&self, line: u32) -> Vec<String> {
        let mut names: Vec<String> = vec![];
        for lock in &self.locks {
            if lock
                .held
                .iter()
                .any(|&(first, last)| first <= line && line <= last)
                && !names.contains(&lock.name)
            {
                names.push(lock.name.clone());
            }
        }
        names
    }
}

/// Lines `first..=last` but the ranges `out`.
pub fn ranges_without(first: u32, last: u32, out: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut ranges = vec![(first, last)];
    for &(a, b) in out {
        ranges = ranges
            .into_iter()
            .flat_map(|(start, end)| {
                let mut parts = vec![];
                if start < a {
                    parts.push((start, end.min(a - 1)));
                }
                if end > b {
                    parts.push((start.max(b + 1), end));
                }
                if b < start || a > end {
                    parts = vec![(start, end)];
                }
                parts
            })
            .collect();
    }
    ranges
}

/// Where a file's functions hold mutexes (see `held_regions`), as lines, if
/// they do.
pub fn file_locks(file: &super::frontend::FileIr) -> Option<FileLocks> {
    let mut locks = vec![];
    for func in &file.funcs {
        for region in held_regions(func) {
            let mut out = region.released.clone();
            out.extend(region.lambdas.iter().copied());
            let name = match region.kind {
                LockKind::Proof => format!(
                    "{}'s mutex",
                    region
                        .param
                        .and_then(|param| func.params.get(param))
                        .map_or("a proof", |param| param.name.as_str())
                ),
                _ => mutex_name(region.mutex.as_deref(), &region.mutex_text),
            };
            locks.push(LineLock {
                held: ranges_without(region.line, region.end_line, &out),
                mutex: region.mutex,
                name,
                kind: region.kind,
            });
        }
    }
    (!locks.is_empty()).then(|| FileLocks {
        path: file.path.clone(),
        locks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_locks() {
        assert_eq!(
            ranges_without(1, 14, &[(8, 10), (11, 11)]),
            vec![(1, 7), (12, 14)]
        );
        assert_eq!(ranges_without(5, 9, &[]), vec![(5, 9)]);
        assert_eq!(ranges_without(5, 9, &[(1, 3)]), vec![(5, 9)]);
        assert_eq!(ranges_without(5, 9, &[(5, 9)]), vec![]);
        assert_eq!(
            mutex_name(Some("F_<T_mozilla::dom::Worker>_mMutex"), "mMutex"),
            "Worker::mMutex"
        );
        assert_eq!(mutex_name(Some("_ZL6sMutex"), "sMutex"), "sMutex");
        let locks = FileLocks {
            path: "a.cpp".to_string(),
            locks: vec![
                LineLock {
                    held: vec![(2, 9)],
                    mutex: None,
                    name: "Worker::mMonitor".to_string(),
                    kind: LockKind::Acquire,
                },
                LineLock {
                    held: vec![(4, 5), (8, 9)],
                    mutex: None,
                    name: "Worker::mMutex".to_string(),
                    kind: LockKind::Acquire,
                },
            ],
        };
        assert_eq!(locks.held_at(4), vec!["Worker::mMonitor", "Worker::mMutex"]);
        assert_eq!(locks.held_at(6), vec!["Worker::mMonitor"]);
        assert!(locks.held_at(10).is_empty());
    }

    #[test]
    fn test_guard_kinds() {
        assert_eq!(
            guard_kind(
                Some("T_mozilla::detail::BaseAutoLock"),
                Some("mozilla::MutexAutoLock")
            ),
            Some(LockKind::Acquire)
        );
        assert_eq!(
            guard_kind(
                Some("T_mozilla::detail::BaseAutoUnlock"),
                Some("mozilla::MutexAutoUnlock")
            ),
            Some(LockKind::Release)
        );
        assert_eq!(
            guard_kind(None, Some("const mozilla::MonitorAutoLock &")),
            Some(LockKind::Acquire)
        );
        assert_eq!(
            guard_kind(None, Some("MutexAutoUnlock")),
            Some(LockKind::Release)
        );
        assert_eq!(
            guard_kind(
                Some("T_std::lock_guard"),
                Some("std::lock_guard<std::mutex>")
            ),
            Some(LockKind::Acquire)
        );
        assert_eq!(
            guard_kind(Some("T_nsCOMPtr"), Some("nsCOMPtr<nsIFile>")),
            None
        );
        assert_eq!(guard_kind(None, None), None);
        assert!(is_proof_type(
            Some("T_mozilla::detail::BaseAutoLock"),
            Some("const MutexAutoLock &")
        ));
        assert!(!is_proof_type(
            Some("T_mozilla::detail::BaseAutoLock"),
            Some("mozilla::MutexAutoLock")
        ));
        assert!(!is_proof_type(
            Some("T_mozilla::detail::BaseAutoUnlock"),
            Some("const MutexAutoUnlock &")
        ));
    }
}
