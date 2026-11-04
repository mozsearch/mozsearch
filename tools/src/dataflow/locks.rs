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

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use ustr::Ustr;

use super::ir::{Atom, CallKind, EventKind, FuncIr, LockKind, LockRegion, Place, Step, Val};

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
/// guard's or a proof parameter's region (see `LockRegion`), but where the
/// releasing guards of its mutex in it release it, and the lambdas made in it;
/// and what runs there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldRegion {
    pub kind: LockKind,
    pub guard: String,
    /// The mutex's symbol (see `LockRegion::mutex_sym`; none for a proof's,
    /// which its callers say), and as the function spells it.
    pub mutex: Option<Ustr>,
    pub mutex_text: String,
    /// For a proof, its parameter's index.
    pub param: Option<usize>,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldCall {
    pub line: u32,
    pub callee: Vec<Ustr>,
    pub callee_pretty: String,
    pub kind: CallKind,
}

/// A field accessed with a mutex held: the lines it's read on, written on,
/// and has methods called on (`mQueue.Push(x)`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FieldAccess {
    pub sym: Ustr,
    pub name: String,
    pub reads: Vec<u32>,
    pub writes: Vec<u32>,
    pub calls: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NestedLock {
    pub mutex: Option<Ustr>,
    pub mutex_text: String,
    pub guard: String,
    pub line: u32,
}

/// The fields in a place (`this->mA.mB`: mA and mB), C++'s (`F_…`).
fn place_fields(place: &Place) -> impl Iterator<Item = Ustr> + '_ {
    place.path.iter().filter_map(|step| match step {
        Step::Field(sym) if sym.starts_with("F_") => Some(*sym),
        _ => None,
    })
}

/// The lines of some of a function's events: (first, last).
fn event_lines(ir: &FuncIr, start: usize, end: usize) -> Option<(u32, u32)> {
    let events = ir.events.get(start..end.min(ir.events.len()))?;
    let first = events.iter().map(|e| e.line).min()?;
    let last = events.iter().map(|e| e.end_line.max(e.line)).max()?;
    Some((first, last))
}

/// The regions where a function holds mutexes (see `HeldRegion`): each
/// acquiring guard's and proof parameter's region, with what runs in it with
/// the mutex held.
pub fn held_regions(ir: &FuncIr) -> Vec<HeldRegion> {
    let guard_calls: BTreeSet<usize> = ir.locks.iter().filter_map(|l| l.call).collect();
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
        let held =
            |e: usize| !region.in_lambda(e) && !releases.iter().any(|l| l.start <= e && e < l.end);
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
        for e in region.start..region.end.min(ir.events.len()) {
            if !held(e) {
                continue;
            }
            let event = &ir.events[e];
            held_lines.insert(event.line);
            match &event.kind {
                EventKind::Assign { place, vals, .. } => {
                    // (Its last field's written; the ones it's reached
                    // through, read.)
                    let path: Vec<Ustr> = place_fields(place).collect();
                    if let Some((last, through)) = path.split_last() {
                        for sym in through {
                            access(*sym, event.line, 0);
                        }
                        access(*last, event.line, 1);
                    }
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
                    calls.push(HeldCall {
                        line: call.line,
                        callee: call.callee.clone(),
                        callee_pretty: call.callee_pretty.clone(),
                        kind: call.kind,
                    });
                    for arg in &call.args {
                        reads(&arg.vals, event.line, &mut access);
                    }
                    // (A method called on a field: `mQueue.Push(x)`.)
                    if let Some(receiver) = &call.receiver
                        && let Some(place) = &receiver.place
                    {
                        let path: Vec<Ustr> = place_fields(place).collect();
                        if let Some((last, through)) = path.split_last() {
                            for sym in through {
                                access(*sym, event.line, 0);
                            }
                            access(*last, event.line, 2);
                        }
                    }
                }
            }
        }
        let locks = ir
            .locks
            .iter()
            .filter(|l| {
                l.kind == LockKind::Acquire
                    && l.start < region.end
                    && !std::ptr::eq(*l, region)
                    && (l.mutex_sym() != mutex || mutex.is_none())
                    // (Made with it held.)
                    && l.start > region.start
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
        out.push(HeldRegion {
            kind: region.kind,
            guard: region.guard.clone(),
            mutex,
            mutex_text: match &region.mutex {
                Some(place) => ir.place_name(place),
                None => region.mutex_text.clone(),
            },
            param: region.param,
            line: region.line,
            end_line: region.end_line,
            released: releases.iter().map(|l| (l.line, l.end_line)).collect(),
            lambdas: region
                .lambdas
                .iter()
                .filter_map(|&(start, end)| event_lines(ir, start, end))
                .collect(),
            held_lines: held_lines.into_iter().collect(),
            calls,
            fields: fields.into_values().collect(),
            locks,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
