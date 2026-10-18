//! Slicing a patch (or patch stack) into the clusters of dataflow it
//! touches, so its changes can be shown as a series of linked diagrams
//! rather than one that conflates unrelated changes.
//!
//! A patch's changed lines are grouped into units (the contiguous changed
//! lines of a function, or of declarations outside functions), and units are
//! linked when they share a specific dataflow "key": the same field (the
//! most specific member of a place, not the objects it's reached through),
//! local, or parameter (an IPDL struct's field too, which its accessors read
//! and its constructors' arguments write; see `IpdlSlot`); a call of a
//! function another unit defines or changes (anywhere in a changed function's
//! body: the patch changes both a caller and its callee); or the same binding
//! (an IPDL message's Send and Recv, XPIDL's JS and C++).  Then values written by each unit are traced forwards a few hops,
//! linking the units they reach, for flows through unchanged code.  The
//! clusters are the connected components.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ustr::Ustr;

use super::frontend::FileIr;
use super::ir::*;
use super::render::DotOptions;
use super::trace::{Dir, IpdlSlot, Tracer, ipc_message_sym, short_pretty};
use crate::file_format::rollup::is_boring_callee;

/// The lines a patch changed, in the coordinates of the files we have the
/// analysis of (its new side; see `rev_lines`).
#[derive(Deserialize, Serialize)]
pub struct ChangedLines {
    /// Its commits, oldest first.
    #[serde(default)]
    pub commits: Vec<String>,
    pub files: BTreeMap<String, FileChange>,
    /// What isn't in `files` or `removed_lines`.
    #[serde(default)]
    pub notes: Vec<String>,
    /// The revision it changes (the old side), if known, and the lines it
    /// removes there, by file (all of a removed file's), for the units of
    /// what it removes, when the old side's analysis is there (ex: a try
    /// push's base; see `AbstractServer::old_side`).
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub removed_lines: BTreeMap<String, Vec<u32>>,
}

#[derive(Deserialize, Serialize)]
pub struct FileChange {
    #[serde(default)]
    pub added: Vec<u32>,
    /// Where removed lines were.
    #[serde(default)]
    pub removed: Vec<u32>,
    /// The commits making the added lines, by line, if known.
    #[serde(default)]
    pub commits: BTreeMap<u32, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Key {
    /// A member, by symbol.
    Field(Ustr),
    Global(Ustr),
    /// A function's local or parameter.
    Local(Ustr, Ustr),
    /// A function called, defined, or changed.
    Function(Ustr),
    /// A binding's owner (ex: an IPDL message), which its slots share.
    Binding(Ustr),
    /// A symbol the patch defines (ex: a new enum value, class, or
    /// function), mentioned.
    Mention(Ustr),
}

#[derive(Serialize, Deserialize)]
pub struct Unit {
    pub id: usize,
    pub path: String,
    pub lines: Vec<u32>,
    /// The function it's in, if any: its pretty name, symbol, and class.
    pub func: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub func_sym: Option<Ustr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<Ustr>,
    /// The commits making its lines, if known (see `FileChange::commits`),
    /// in the patch's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<String>,
    /// Whether it's lines the patch removes, in the old side's coordinates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    pub keys: BTreeSet<Key>,
    pub text: Vec<String>,
    /// Its function's callees anywhere in its body (but plumbing), and the
    /// IPDL messages it sends, for linking it to the patch's other functions
    /// and messages (see `slice`).
    #[serde(skip)]
    body_calls: BTreeSet<Ustr>,
    #[serde(skip)]
    body_sends: BTreeSet<Ustr>,
}

#[derive(Serialize, Deserialize)]
pub struct Link {
    pub from: usize,
    pub to: usize,
    /// Why (the shared key, or the trace).
    pub why: String,
}

#[derive(Serialize, Deserialize)]
pub struct Slicing {
    pub units: Vec<Unit>,
    pub links: Vec<Link>,
    /// Clusters of units, largest first.
    pub clusters: Vec<Vec<usize>>,
    /// The units without dataflow (no keys, or links), ex: `#include`s and
    /// namespaces' braces, which aren't clusters of their own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<usize>,
    pub notes: Vec<String>,
}

/// Keys whose sharing doesn't mean much (they're everywhere), by name.
fn is_hub(name: &str) -> bool {
    matches!(
        name,
        "this" | "rv" | "aRv" | "result" | "aResult" | "self" | "mActor" | "mLoadInfo"
    )
}

fn place_keys(f: &FuncIr, place: &Place, keys: &mut BTreeSet<Key>) {
    // (The most specific member: `mLoadInfo.mTimezoneOverride` is about
    // `mTimezoneOverride`, not everything in `mLoadInfo`.)
    if let Some(field) = place.last_field()
        && !field.starts_with("name:")
    {
        keys.insert(Key::Field(field));
        return;
    }
    match &place.root {
        Root::Local(sym) if !sym.starts_with("name:") => {
            let name = f.names.get(sym).map(|s| s.as_str()).unwrap_or("");
            if !is_hub(name) {
                keys.insert(Key::Local(f.sym, *sym));
            }
        }
        Root::Global(sym) => {
            keys.insert(Key::Global(*sym));
        }
        _ => {}
    }
}

fn vals_keys(f: &FuncIr, vals: &[Val], keys: &mut BTreeSet<Key>) {
    for v in vals {
        if let Atom::Read(p) | Atom::Addr(p) = &v.atom {
            place_keys(f, p, keys);
        }
    }
}

/// The keys of a function's events at some lines.
fn event_keys(f: &FuncIr, lines: &BTreeSet<u32>, keys: &mut BTreeSet<Key>) {
    for e in f.events.iter().filter(|e| lines.contains(&e.line)) {
        match &e.kind {
            EventKind::Assign { place, vals, .. } => {
                place_keys(f, place, keys);
                vals_keys(f, vals, keys);
            }
            EventKind::Call(c) => {
                let call = &f.calls[*c];
                for sym in &call.callee {
                    keys.insert(Key::Function(*sym));
                }
                if let Some(owner) = ipc_message_sym(&call.callee_pretty) {
                    keys.insert(Key::Binding(owner));
                }
                for a in &call.args {
                    vals_keys(f, &a.vals, keys);
                }
                if let Some(r) = &call.receiver {
                    vals_keys(f, &r.vals, keys);
                }
            }
            EventKind::Return { vals } | EventKind::Test { vals } => vals_keys(f, vals, keys),
        }
    }
}

/// The symbols defined on a line (see `FileRecords::symbols_defined_at`), but
/// files, directories and namespaces, which contain things rather than
/// carrying data (two new files in a namespace aren't linked by it).
fn defined_at(recs: &super::records::FileRecords, line: u32) -> Vec<Ustr> {
    recs.symbols_defined_at(line)
        .into_iter()
        .filter(|sym| !["FILE_", "DIR_", "NS_"].iter().any(|p| sym.starts_with(p)))
        .collect()
}

/// The IPDL fields a function's events on some lines read or write: their
/// accessors' calls, their structs' constructors' calls (whose arguments
/// are the fields'), and assignments through their mutable accessors (which
/// `place_keys` gives the accessors' symbols, not the fields').
async fn ipdl_keys(
    tracer: &mut Tracer<'_>,
    f: &FuncIr,
    lines: &BTreeSet<u32>,
    keys: &mut BTreeSet<Key>,
) {
    let on_lines =
        |line: u32, end_line: u32| lines.range(line..=end_line.max(line)).next().is_some();
    for e in &f.events {
        match &e.kind {
            EventKind::Call(c) if on_lines(e.line, e.end_line) => {
                let call = &f.calls[*c];
                match tracer.call_ipdl_slot(call).await {
                    Some(IpdlSlot::Accessor(field)) if on_lines(call.line, call.end_line) => {
                        keys.insert(Key::Field(field));
                    }
                    // (The fields whose arguments are on the lines: a
                    // constructor's call can span many.)
                    Some(IpdlSlot::Constructor(fields)) => {
                        for (arg, field) in call.args.iter().zip(fields) {
                            if on_lines(arg.line, arg.end_line) {
                                keys.insert(Key::Field(field));
                            }
                        }
                    }
                    _ => {}
                }
            }
            EventKind::Assign { place, .. } if lines.contains(&e.line) => {
                if let Some(field) = tracer.ipdl_field_of_place(place).await {
                    keys.insert(Key::Field(field));
                    if let Some(Step::Field(accessor)) = place.path.last() {
                        keys.remove(&Key::Field(*accessor));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Fetch, at once, what `file_units` looks up for files' lines one at a
/// time: the files, then the callees of the calls on the lines and the
/// functions they're in (for `ipdl_keys` and `Tracer::class_of`), and then
/// those's IPDL owners and classes.
async fn prefetch_units<'a>(
    tracer: &mut Tracer<'_>,
    files: impl Iterator<Item = (&'a String, BTreeSet<u32>)>,
) {
    let files: Vec<(&String, BTreeSet<u32>)> = files.collect();
    tracer
        .prefetch([], files.iter().map(|(path, _)| path.to_string()))
        .await;
    let mut syms: Vec<Ustr> = vec![];
    for (path, lines) in &files {
        let Some(file) = tracer.file(path).await else {
            continue;
        };
        for line in lines {
            if let Some(f) = file.func_at_line(*line) {
                syms.push(f.sym);
                syms.extend(
                    f.calls
                        .iter()
                        .filter(|c| c.line <= *line && *line <= c.end_line.max(c.line))
                        .flat_map(|c| c.callee.iter().copied()),
                );
            }
        }
    }
    tracer.prefetch(syms.iter().copied(), []).await;
    let mut owners: Vec<Ustr> = vec![];
    for sym in &syms {
        if let Some(meta) = tracer.cached_xref(sym).and_then(|xref| xref.meta.clone()) {
            owners.extend(meta.slot_owner.map(|owner| owner.sym));
            owners.extend(meta.parent_sym);
        }
    }
    tracer.prefetch(owners, []).await;
}

/// The units of a file's lines (of the patch's new side, or, `removed`, those
/// it removes, on the old side, which `tracer` is of), with their functions'
/// IR: a function's contiguous changed lines, or a run of declarations
/// outside functions, with their keys.
async fn file_units(
    tracer: &mut Tracer<'_>,
    path: &str,
    lines: &BTreeSet<u32>,
    commits: &(dyn Fn(&BTreeSet<u32>) -> Vec<String> + Sync),
    removed: bool,
) -> Vec<(Unit, Option<(Arc<FileIr>, usize)>)> {
    let mut out = vec![];
    let file = tracer.file(path).await;
    let source = tracer.source(path).await.unwrap_or_default();
    let source_lines: Vec<&str> = source.lines().collect();
    let text_of = |line: u32| -> String {
        source_lines
            .get(line as usize - 1)
            .map(|l| l.trim().to_string())
            .unwrap_or_default()
    };
    let Some(file) = file else {
        // (Files we have no frontend for, ex: IPDL: their declarations'
        // symbols, its structs' fields as fields.)
        let recs = tracer.records(path).await;
        let mut keys = BTreeSet::new();
        if let Some(recs) = &recs {
            for line in lines {
                for sym in defined_at(recs, *line) {
                    let kind = recs
                        .structured(&sym)
                        .and_then(|s| s.get("kind"))
                        .and_then(|k| k.as_str());
                    if sym.starts_with("IPC_") {
                        keys.insert(Key::Binding(sym));
                    } else if sym.starts_with("IPDL_") && kind == Some("field") {
                        keys.insert(Key::Field(sym));
                    } else {
                        keys.insert(Key::Function(sym));
                    }
                }
            }
        }
        out.push((
            Unit {
                id: 0,
                path: path.to_string(),
                lines: lines.iter().copied().collect(),
                func: None,
                func_sym: None,
                class: None,
                commits: commits(lines),
                removed,
                keys,
                text: lines.iter().map(|l| text_of(*l)).collect(),
                body_calls: BTreeSet::new(),
                body_sends: BTreeSet::new(),
            },
            None,
        ));
        return out;
    };
    // Group the lines by the function they're in (or none), and
    // contiguous runs outside functions.
    let mut groups: BTreeMap<(Option<usize>, u32), BTreeSet<u32>> = BTreeMap::new();
    let mut last: Option<(Option<usize>, u32, u32)> = None;
    for &line in lines {
        let func = file
            .func_at_line(line)
            .and_then(|f| file.by_sym.get(&f.sym).copied())
            .filter(|&i| file.funcs[i].line < file.funcs[i].end_line || file.funcs[i].line == line);
        let start = match (last, func) {
            (Some((lf, start, prev)), f) if lf == f && (f.is_some() || line <= prev + 2) => start,
            _ => line,
        };
        groups.entry((func, start)).or_default().insert(line);
        last = Some((func, start, line));
    }
    for ((func, _), group_lines) in groups {
        let mut keys = BTreeSet::new();
        let recs = tracer.records(path).await;
        // (Definitions and declarations on the lines: new fields,
        // methods, parameters.)
        if let Some(recs) = &recs {
            for line in &group_lines {
                for sym in defined_at(recs, *line) {
                    if sym.starts_with("F_") {
                        keys.insert(Key::Field(sym));
                    } else if sym.starts_with("V_") {
                        // (Parameters, by their functions' events.)
                    } else {
                        keys.insert(Key::Function(sym));
                    }
                }
            }
        }
        if let Some(fi) = func {
            let f = &file.funcs[fi];
            event_keys(f, &group_lines, &mut keys);
            ipdl_keys(tracer, f, &group_lines, &mut keys).await;
            // (A changed function's callers link to it.)
            keys.insert(Key::Function(f.sym));
            // (Parameters declared on the lines.)
            for p in &f.params {
                if let Root::Local(sym) = p.place.root
                    && group_lines.contains(&f.line)
                    && !is_hub(&p.name)
                {
                    keys.insert(Key::Local(f.sym, sym));
                }
            }
        }
        let (mut body_calls, mut body_sends) = (BTreeSet::new(), BTreeSet::new());
        if let Some(fi) = func {
            for call in &file.funcs[fi].calls {
                if is_boring_callee(&call.callee_pretty) {
                    continue;
                }
                body_calls.extend(call.callee.iter().copied());
                body_sends.extend(ipc_message_sym(&call.callee_pretty));
            }
        }
        out.push((
            Unit {
                id: 0,
                path: path.to_string(),
                lines: group_lines.iter().copied().collect(),
                func: func.map(|fi| file.funcs[fi].pretty.clone()),
                func_sym: func.map(|fi| file.funcs[fi].sym),
                class: match func {
                    Some(fi) => tracer.class_of(&file.funcs[fi].sym).await,
                    None => None,
                },
                commits: commits(&group_lines),
                removed,
                keys,
                text: group_lines.iter().map(|l| text_of(*l)).collect(),
                body_calls,
                body_sends,
            },
            func.map(|fi| (file.clone(), fi)),
        ));
    }
    out
}

/// Slice a patch's changed lines (with `tracer`, of its new side), and the lines
/// it removes, if there's an `old` tracer, of its old side (see
/// `ChangedLines::removed_lines`).
pub async fn slice(
    tracer: &mut Tracer<'_>,
    changes: &ChangedLines,
    trace_hops: usize,
    old: Option<&mut Tracer<'_>>,
) -> Slicing {
    // (Units link by traces' nodes on their lines, so traces enter accessors
    // rather than stepping over them by their summaries, whose nodes are only
    // at their definitions.)
    tracer.opts.summaries = false;
    let mut old = old;
    if let Some(old) = old.as_mut() {
        old.opts.summaries = false;
    }
    let mut units: Vec<Unit> = vec![];
    // (Units' functions and lines, for tracing.)
    let mut unit_funcs: Vec<Option<(Arc<FileIr>, usize)>> = vec![];
    let mut notes = changes.notes.clone();
    // (The commits making a file's lines, in the patch's order.)
    let commits_of = |change: &FileChange, lines: &BTreeSet<u32>| -> Vec<String> {
        let made: BTreeSet<&String> = lines.iter().filter_map(|l| change.commits.get(l)).collect();
        changes
            .commits
            .iter()
            .filter(|c| made.contains(c))
            .cloned()
            .collect()
    };
    let changed_lines = |change: &FileChange| -> BTreeSet<u32> {
        change
            .added
            .iter()
            .chain(&change.removed)
            .copied()
            .collect()
    };
    prefetch_units(
        tracer,
        changes
            .files
            .iter()
            .map(|(path, change)| (path, changed_lines(change))),
    )
    .await;
    if let Some(old) = old.as_deref_mut() {
        prefetch_units(
            old,
            changes
                .removed_lines
                .iter()
                .map(|(path, lines)| (path, lines.iter().copied().collect())),
        )
        .await;
    }
    for (path, change) in &changes.files {
        let mut lines: BTreeSet<u32> = change.added.iter().copied().collect();
        lines.extend(change.removed.iter().copied());
        if lines.is_empty() {
            continue;
        }
        let commits = |lines: &BTreeSet<u32>| commits_of(change, lines);
        for (unit, uf) in file_units(tracer, path, &lines, &commits, false).await {
            units.push(unit);
            unit_funcs.push(uf);
        }
    }
    // (What it removes, on the old side, if its analysis is there.)
    match old.as_deref_mut() {
        Some(old) => {
            for (path, lines) in &changes.removed_lines {
                let lines: BTreeSet<u32> = lines.iter().copied().collect();
                for (unit, _) in file_units(old, path, &lines, &|_| vec![], true).await {
                    units.push(unit);
                    // (Not traced: its values are gone.)
                    unit_funcs.push(None);
                }
            }
        }
        None if !changes.removed_lines.is_empty() => notes.push(format!(
            "The lines it removes ({} files') aren't sliced: their old side's analysis isn't here",
            changes.removed_lines.len()
        )),
        None => {}
    }
    for (i, unit) in units.iter_mut().enumerate() {
        unit.id = i;
    }

    // The symbols the patch defines or changes the definitions of: only
    // calls of those link units (everything calls `AssertIsOnMainThread`).
    let mut patch_syms: BTreeSet<Ustr> = BTreeSet::new();
    for unit in &units {
        patch_syms.extend(unit.func_sym);
        // (A removed unit's, on the old side.)
        let recs = match old.as_deref_mut() {
            Some(old) if unit.removed => old.records(&unit.path).await,
            _ => tracer.records(&unit.path).await,
        };
        if let Some(recs) = recs {
            for line in &unit.lines {
                patch_syms.extend(
                    defined_at(&recs, *line)
                        .into_iter()
                        .filter(|s| !s.starts_with("V_")),
                );
            }
        }
    }
    for unit in units.iter_mut() {
        unit.keys.retain(|k| match k {
            Key::Function(sym) => patch_syms.contains(sym),
            _ => true,
        });
        // (Its function's calls of the patch's other functions, on its
        // unchanged lines too: the patch changes a caller and its callee.)
        let calls: Vec<Key> = unit
            .body_calls
            .iter()
            .filter(|sym| patch_syms.contains(sym))
            .map(|sym| Key::Function(*sym))
            .collect();
        unit.keys.extend(calls);
        let recs = match old.as_deref_mut() {
            Some(old) if unit.removed => old.records(&unit.path).await,
            _ => tracer.records(&unit.path).await,
        };
        if let Some(recs) = recs {
            for line in &unit.lines {
                for sym in recs.symbols_at(*line) {
                    if patch_syms.contains(&sym) {
                        unit.keys.insert(Key::Mention(sym));
                    }
                }
            }
        }
    }
    // Units of nothing but blank lines, braces and comments join their
    // neighbors.
    let trivial = |unit: &Unit| {
        unit.keys.iter().all(|k| matches!(k, Key::Function(_)))
            && unit.text.iter().all(|t| {
                let t = t.trim();
                t.is_empty() || t.chars().all(|c| "{}();,".contains(c)) || t.starts_with("//")
            })
    };
    let mut merged_into: Vec<Option<usize>> = vec![None; units.len()];
    for i in 0..units.len() {
        if !trivial(&units[i]) {
            continue;
        }
        let first = units[i].lines.first().copied().unwrap_or(0);
        let last = units[i].lines.last().copied().unwrap_or(0);
        let neighbor = (0..units.len())
            .filter(|&j| {
                j != i
                    && units[j].path == units[i].path
                    && units[j].removed == units[i].removed
                    && !trivial(&units[j])
            })
            .filter_map(|j| {
                let jf = units[j].lines.first().copied().unwrap_or(0);
                let jl = units[j].lines.last().copied().unwrap_or(0);
                let gap = if jl < first {
                    first - jl
                } else {
                    jf.saturating_sub(last)
                };
                (gap <= 3).then_some((gap, j))
            })
            .min();
        if let Some((_, j)) = neighbor {
            merged_into[i] = Some(j);
        }
    }
    for i in 0..units.len() {
        if let Some(j) = merged_into[i] {
            let lines = std::mem::take(&mut units[i].lines);
            let text = std::mem::take(&mut units[i].text);
            let commits = std::mem::take(&mut units[i].commits);
            units[j].lines.extend(lines);
            units[j].text.extend(text);
            units[j].lines.sort();
            for commit in commits {
                if !units[j].commits.contains(&commit) {
                    units[j].commits.push(commit);
                }
            }
            let order = |c: &String| changes.commits.iter().position(|o| o == c);
            units[j].commits.sort_by_key(order);
        }
    }
    let keep: Vec<bool> = merged_into.iter().map(|m| m.is_none()).collect();
    let mut renumber = vec![usize::MAX; units.len()];
    let mut kept_units = vec![];
    let mut kept_funcs = vec![];
    for (i, (unit, uf)) in units.into_iter().zip(unit_funcs).enumerate() {
        if keep[i] {
            renumber[i] = kept_units.len();
            kept_units.push(unit);
            kept_funcs.push(uf);
        }
    }
    let mut units = kept_units;
    let unit_funcs = kept_funcs;
    for (i, unit) in units.iter_mut().enumerate() {
        unit.id = i;
    }

    // Bindings: functions which are slots of the same binding (ex: an IPDL
    // message's Recv method and the Send calls).
    let functions: Vec<Ustr> = units
        .iter()
        .flat_map(|u| u.keys.iter())
        .filter_map(|k| match k {
            Key::Function(sym) => Some(*sym),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut binding_of: HashMap<Ustr, Ustr> = HashMap::new();
    for sym in functions {
        if let Some(owner) = tracer.binding_owner(&sym).await {
            binding_of.insert(sym, owner);
        }
    }
    for unit in &mut units {
        let extra: Vec<Key> = unit
            .keys
            .iter()
            .filter_map(|k| match k {
                Key::Function(sym) => binding_of.get(sym).map(|o| Key::Binding(*o)),
                _ => None,
            })
            .collect();
        unit.keys.extend(extra);
    }
    // (The patch's IPDL messages, ones whose declarations or implementations
    // it changes, sent anywhere in its changed functions' bodies.)
    let mut messages: BTreeSet<Ustr> = BTreeSet::new();
    for unit in &units {
        match unit.func_sym {
            Some(sym) => messages.extend(binding_of.get(&sym)),
            None => messages.extend(unit.keys.iter().filter_map(|k| match k {
                Key::Binding(owner) => Some(*owner),
                _ => None,
            })),
        }
    }
    for unit in &mut units {
        let sends: Vec<Key> = unit
            .body_sends
            .iter()
            .filter(|owner| messages.contains(owner))
            .map(|owner| Key::Binding(*owner))
            .collect();
        unit.keys.extend(sends);
    }

    // Link units sharing keys.
    let mut links = vec![];
    let mut by_key: BTreeMap<&Key, Vec<usize>> = BTreeMap::new();
    for unit in &units {
        for key in &unit.keys {
            by_key.entry(key).or_default().push(unit.id);
        }
    }
    let mut parent: Vec<usize> = (0..units.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let key_name = |key: &Key, units: &[Unit]| -> String {
        let _ = units;
        match key {
            Key::Field(sym) | Key::Global(sym) => sym_short_name(sym),
            Key::Local(_, sym) => sym_short_name(sym),
            Key::Function(sym) | Key::Mention(sym) => sym_short_name(sym),
            Key::Binding(sym) => sym.to_string(),
        }
    };
    for (key, ids) in &by_key {
        if ids.len() < 2 {
            continue;
        }
        for pair in ids.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            links.push(Link {
                from: a,
                to: b,
                why: format!("{:?}", key)
                    .split('(')
                    .next()
                    .unwrap_or("")
                    .to_lowercase()
                    + " "
                    + &key_name(key, &units),
            });
            if ra != rb {
                parent[ra] = rb;
            }
        }
    }

    // Link units whose written values reach others (through unchanged code).
    if trace_hops > 0 {
        let unit_lines: Vec<(String, BTreeSet<u32>)> = units
            .iter()
            .map(|u| (u.path.clone(), u.lines.iter().copied().collect()))
            .collect();
        for (ui, uf) in unit_funcs.iter().enumerate() {
            let Some((file, fi)) = uf else {
                continue;
            };
            let f = &file.funcs[*fi];
            let written: Vec<(Place, u32)> = f
                .events
                .iter()
                .filter(|e| unit_lines[ui].1.contains(&e.line))
                .filter_map(|e| match &e.kind {
                    EventKind::Assign { place, .. } => Some((place.clone(), e.line)),
                    _ => None,
                })
                .collect();
            for (place, line) in written.into_iter().take(4) {
                let mut sub = tracer.fork(trace_hops);
                sub.seed(file.clone(), *fi, place, Some(line + 1), Dir::Forward);
                sub.run(Dir::Forward).await;
                for node in &sub.graph.nodes {
                    if !matches!(
                        node.kind,
                        super::trace::NodeKind::Assign
                            | super::trace::NodeKind::Call
                            | super::trace::NodeKind::Return
                            | super::trace::NodeKind::Test
                    ) {
                        continue;
                    }
                    for (uj, (path, lines)) in unit_lines.iter().enumerate() {
                        if uj != ui && *path == node.path && lines.contains(&node.line) {
                            let (ra, rb) = (find(&mut parent, ui), find(&mut parent, uj));
                            links.push(Link {
                                from: ui,
                                to: uj,
                                why: format!("flows: {}", node.label),
                            });
                            if ra != rb {
                                parent[ra] = rb;
                            }
                        }
                    }
                }
                tracer.absorb(sub);
            }
        }
    }

    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut other = vec![];
    for i in 0..units.len() {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(i);
    }
    let mut clusters: Vec<Vec<usize>> = clusters
        .into_values()
        .filter(|cluster| match cluster.as_slice() {
            [only] if units[*only].keys.is_empty() => {
                other.push(*only);
                false
            }
            _ => true,
        })
        .collect();
    clusters.sort_by_key(|c| std::cmp::Reverse(c.len()));
    links.sort_by_key(|l| (l.from, l.to));
    links.dedup_by(|a, b| a.from == b.from && a.to == b.to && a.why == b.why);
    if units.is_empty() {
        notes.push("No changed lines in files with analysis".to_string());
    }
    Slicing {
        units,
        links,
        clusters,
        other,
        notes,
    }
}

/// A slicing's cluster's commits' (abbreviated) SHAs, in the patch's order,
/// if its units' commits are known.
fn cluster_commits(slicing: &Slicing, cluster: &[usize]) -> String {
    let mut commits: Vec<&String> = vec![];
    for &ui in cluster {
        for commit in &slicing.units[ui].commits {
            if !commits.contains(&commit) {
                commits.push(commit);
            }
        }
    }
    commits
        .iter()
        .map(|c| &c[..c.len().min(12)])
        .collect::<Vec<_>>()
        .join(", ")
}

impl Slicing {
    /// Only its `n`th cluster (1 for the biggest), and the links among its
    /// units.
    pub fn only_cluster(&mut self, n: usize) -> Result<(), String> {
        if n == 0 || n > self.clusters.len() {
            return Err(format!("No cluster {} (of {})", n, self.clusters.len()));
        }
        let cluster = self.clusters.swap_remove(n - 1);
        let units: BTreeSet<usize> = cluster.iter().copied().collect();
        self.links
            .retain(|l| units.contains(&l.from) && units.contains(&l.to));
        self.clusters = vec![cluster];
        self.other.clear();
        Ok(())
    }
}

pub fn render_slicing_text(slicing: &Slicing) -> String {
    let mut out = String::new();
    for (ci, cluster) in slicing.clusters.iter().enumerate() {
        let commits = cluster_commits(slicing, cluster);
        let _ = writeln!(
            out,
            "== cluster {} ({} units){}",
            ci + 1,
            cluster.len(),
            if commits.is_empty() {
                String::new()
            } else {
                format!(" in {}", commits)
            }
        );
        for &ui in cluster {
            let unit = &slicing.units[ui];
            let lines = match (unit.lines.first(), unit.lines.last()) {
                (Some(a), Some(b)) if a != b => format!("{}-{}", a, b),
                (Some(a), _) => a.to_string(),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "  u{:<3} {}{}:{} {}",
                ui,
                if unit.removed { "removed " } else { "" },
                unit.path,
                lines,
                unit.func.as_deref().map(short_pretty).unwrap_or_default()
            );
            for text in unit.text.iter().filter(|t| !t.is_empty()).take(3) {
                let _ = writeln!(out, "         | {}", text);
            }
        }
        let cluster_set: BTreeSet<usize> = cluster.iter().copied().collect();
        let mut whys: BTreeMap<String, usize> = BTreeMap::new();
        for link in &slicing.links {
            if cluster_set.contains(&link.from) {
                *whys.entry(link.why.clone()).or_default() += 1;
            }
        }
        if !whys.is_empty() {
            let _ = writeln!(
                out,
                "  linked by: {}",
                whys.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
    }
    if !slicing.other.is_empty() {
        let _ = writeln!(out, "== without dataflow ({} units)", slicing.other.len());
        for &ui in &slicing.other {
            let unit = &slicing.units[ui];
            let _ = writeln!(
                out,
                "  u{:<3} {}:{}",
                ui,
                unit.path,
                unit.lines.first().copied().unwrap_or(0)
            );
        }
    }
    for note in &slicing.notes {
        let _ = writeln!(out, "note: {}", note);
    }
    out
}

/// A slicing as DOT: its clusters as subgraphs (labeled with their commits,
/// if known), its units as nodes (linked to their first lines, with
/// `options`' `link_tree`), and its links as edges.
pub fn render_slicing_dot(slicing: &Slicing, options: &DotOptions) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "digraph slicing {{");
    let _ = writeln!(
        out,
        "  node [shape=box, fontname=\"monospace\", fontsize=10, style=rounded];"
    );
    let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    // (And the units without dataflow, apart.)
    let mut groups: Vec<(String, &str, &[usize])> = vec![];
    for (ci, cluster) in slicing.clusters.iter().enumerate() {
        let commits = cluster_commits(slicing, cluster);
        let label = match commits.is_empty() {
            true => format!("cluster {}", ci + 1),
            false => format!("cluster {} ({})", ci + 1, commits),
        };
        groups.push((label, "solid", cluster));
    }
    if !slicing.other.is_empty() {
        groups.push(("without dataflow".to_string(), "dashed", &slicing.other));
    }
    for (ci, (label, style, cluster)) in groups.into_iter().enumerate() {
        let _ = writeln!(
            out,
            "  subgraph cluster_{} {{ label=\"{}\"; color=gray; style={};",
            ci,
            escape(&label),
            style
        );
        for &ui in cluster {
            let unit = &slicing.units[ui];
            let file = unit.path.rsplit('/').next().unwrap_or(&unit.path);
            let label = format!(
                "{}{}:{}\\n{}\\n{}",
                if unit.removed { "removed " } else { "" },
                file,
                unit.lines.first().copied().unwrap_or(0),
                escape(&unit.func.as_deref().map(short_pretty).unwrap_or_default()),
                escape(
                    unit.text
                        .iter()
                        .find(|t| !t.is_empty())
                        .map(|t| t.as_str())
                        .unwrap_or("")
                )
            );
            let first = unit.lines.first().copied().unwrap_or(0);
            let link = match &options.link_tree {
                Some(tree) if first > 0 => format!(
                    ", URL=\"/{}/{}/{}#{}\", tooltip=\"{}:{}\"",
                    escape(tree),
                    // (The lines it removes are the old side's, the indexed
                    // revision's.)
                    match &options.link_rev {
                        Some(rev) if !unit.removed => format!("rev/{}", escape(rev)),
                        _ => "source".to_string(),
                    },
                    escape(&unit.path),
                    first,
                    escape(file),
                    first
                ),
                _ => String::new(),
            };
            let style = if unit.removed {
                ", style=\"rounded,dashed\""
            } else {
                ""
            };
            let _ = writeln!(out, "    u{} [label=\"{}\"{}{}];", ui, label, link, style);
        }
        let _ = writeln!(out, "  }}");
    }
    let mut seen = BTreeSet::new();
    for link in &slicing.links {
        if seen.insert((link.from, link.to)) {
            let _ = writeln!(
                out,
                "  u{} -> u{} [label=\"{}\", dir=none, fontsize=8];",
                link.from,
                link.to,
                escape(&link.why)
            );
        }
    }
    let _ = writeln!(out, "}}");
    out
}
