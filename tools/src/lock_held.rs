//! The `lock-held:SYMBOL` view: the code that runs with a mutex held (ex:
//! `F_<T_mozilla::dom::WorkerPrivate>_mMutex`), for auditing its uses (see
//! asuth-gecko-notes' tooling/searchfox/mutex-audit-plan.md): the regions its
//! scoped lock guards hold it in (see `dataflow::locks`), the functions its
//! guards' proofs are passed to (`const MutexAutoLock& aProofOfLock`), and
//! the functions called with it held, to `CALLEE_DEPTH`; with what they do
//! with it held (the fields they access, the calls they make, the other
//! mutexes they acquire), as cards of their code, which the navigation
//! panel's facets filter (see `view_facets`), under a summary of the fields
//! accessed with it held.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ustr::{Ustr, UstrMap, ustr};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::crash_threads::slug;
use crate::dataflow::frontend::FileIr;
use crate::dataflow::ir::{LockKind, sym_short_name};
use crate::dataflow::locks::{HeldRegion, field_parts, held_body, held_regions};
use crate::dataflow::trace::{TraceOpts, Tracer};
use crate::rendered_docs::escape;

/// The MIME type of the view's HTML, which the query page shows as it is.
pub const LOCK_HELD_MIME: &str = "application/vnd.searchfox.lock-held+html";

/// How many calls deep the functions called with the mutex held go (calls
/// passing its proof on don't count: they're held by contract), how many
/// regions the view has at most, and how many functions a call can run (a
/// virtual method's overrides).
pub const CALLEE_DEPTH: usize = 2;
pub const MAX_REGIONS: usize = 300;
const CALL_FANOUT: usize = 4;

/// How many of a card's lines it shows (the others fold), at most.
const MAX_CARD_LINES: u32 = 400;

/// Callees whose bodies aren't worth cards of their own (smart pointers',
/// containers', strings', assertions'), by their pretty names' prefixes:
/// their calls are still listed (and faceted).
const UNCARDED_CALLEES: &[&str] = &[
    "RefPtr",
    "nsCOMPtr",
    "already_AddRefed",
    "mozilla::UniquePtr",
    "mozilla::Maybe",
    "mozilla::Atomic",
    "mozilla::detail::",
    "nsTArray",
    "AutoTArray",
    "nsTSubstring",
    "nsTString",
    "nsTAutoString",
    "nsTDependentString",
    "mozilla::Span",
    "std::",
    "MOZ_",
];

/// How a region holds the mutex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    /// A scoped guard's region.
    Guard,
    /// A function a guard's proof is passed to, held for its whole body.
    Proof,
    /// A function called with the mutex held.
    Callee,
    /// A method requiring it held (`MOZ_REQUIRES(mMutex)`; see
    /// `Annotations`), for its whole body.
    Contract,
}

/// A step of how a region's function is reached from a guard's: the
/// function making the call, where, and the call's line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViaStep {
    pub func: Ustr,
    pub pretty: String,
    pub path: String,
    pub line: u32,
}

/// One of the view's regions (see `HeldRegion`): how it holds the mutex,
/// how deep in calls it is (see `CALLEE_DEPTH`), the calls reaching it from a
/// guard's region, and its function.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewRegion {
    pub how: How,
    pub depth: usize,
    pub via: Vec<ViaStep>,
    pub func: Ustr,
    pub func_pretty: String,
    pub path: String,
    pub func_line: u32,
    pub func_end_line: u32,
    pub region: HeldRegion,
}

impl ViewRegion {
    /// Its card's ID (`data-path`): its file and lines.
    pub fn id(&self) -> String {
        format!(
            "{}:{}-{}",
            self.path, self.region.line, self.region.end_line
        )
    }

    /// The lines its card shows: a guard's region's (from the lambdas made
    /// before it which it calls, if any), or a function's.
    pub fn lines(&self) -> (u32, u32) {
        match self.how {
            How::Guard => (
                self.region
                    .called_lambdas
                    .iter()
                    .map(|&(first, _)| first)
                    .fold(self.region.line, u32::min),
                self.region.end_line,
            ),
            _ => (self.func_line, self.func_end_line),
        }
    }
}

/// The view's model (see the module's documentation): the mutex, its
/// regions, its files' subsystems, and what it left out.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LockHeldView {
    pub mutex: Ustr,
    pub mutex_pretty: String,
    pub regions: Vec<ViewRegion>,
    #[serde(default)]
    pub subsystems: BTreeMap<String, String>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// What its class's thread-safety annotations say of it (see
    /// `Annotations`).
    #[serde(default)]
    pub annotations: Annotations,
    /// The fields which are it too: initialized (or assigned) from it, ex: a
    /// `SharedMutex` copy, or a reference or pointer to it (see `Alias`).
    #[serde(default)]
    pub aliases: Vec<Alias>,
    /// Its guarded fields' accesses outside the regions holding it (but in
    /// the methods requiring it, those without the analysis, and
    /// constructors and destructors), at most `MAX_UNGUARDED`, and how many
    /// there are.
    #[serde(default)]
    pub unguarded: Vec<UnguardedUse>,
    #[serde(default)]
    pub unguarded_count: usize,
}

/// What Gecko's thread-safety annotations (clang's attributes) in the
/// mutex's class's source say of it: its fields it guards
/// (`MOZ_GUARDED_BY(mMutex)`, `MOZ_PT_GUARDED_BY`), its methods requiring it
/// held (`MOZ_REQUIRES(mMutex)`, `MOZ_REQUIRES_SHARED`), and its methods the
/// analysis is off for (`MOZ_NO_THREAD_SAFETY_ANALYSIS`).  (From the source
/// text, the annotations' arguments as the mutex's name; see
/// `annotations`.)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Annotations {
    pub guarded: Vec<Ustr>,
    pub requires: Vec<Ustr>,
    pub unchecked: Vec<Ustr>,
}

/// A field which is the mutex too (see `LockHeldView::aliases`), and where
/// it's initialized from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alias {
    pub sym: Ustr,
    pub name: String,
    pub path: String,
    pub line: u32,
}

/// A guarded field's access outside the regions holding the mutex (see
/// `LockHeldView::unguarded`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnguardedUse {
    pub field: Ustr,
    pub path: String,
    pub line: u32,
    pub context: String,
}

/// How many guarded fields' accesses outside the regions holding the mutex
/// the view lists, at most.
const MAX_UNGUARDED: usize = 200;

impl LockHeldView {
    /// Its files' subsystems, by `subsystem_of` (ex: the tree's per-file
    /// information's; see `AbstractServer::file_facet_info`).
    pub fn add_subsystems(&mut self, subsystem_of: &dyn Fn(&str) -> Option<String>) {
        let paths: BTreeSet<String> = self.regions.iter().map(|r| r.path.clone()).collect();
        for path in paths {
            if let Some(subsystem) = subsystem_of(&path) {
                self.subsystems.insert(path, subsystem);
            }
        }
    }
}

fn problem(layer: ErrorLayer, message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails { layer, message })
}

/// The class and the method of a function's pretty name.
fn method_parts(pretty: &str) -> (String, String) {
    let pretty = pretty.split('(').next().unwrap_or(pretty);
    match pretty.rsplit_once("::") {
        Some((class, method)) => (class.to_string(), method.to_string()),
        None => (String::new(), pretty.to_string()),
    }
}

/// A name's last `::` segment.
fn short(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// The view of the mutex `mutex` (a field's or a global's symbol; see the
/// module's documentation), from the tree's dataflow IR of the files using
/// it, and of the functions its guards' proofs are passed to and those called
/// with it held.
pub async fn lock_held_view(
    server: &(dyn AbstractServer + Send + Sync),
    mutex: &str,
) -> Result<LockHeldView> {
    let sym = ustr(mutex);
    let mut tracer = Tracer::new(server, TraceOpts::default());
    let xref = tracer.crossref(&sym).await.ok_or_else(|| {
        problem(
            ErrorLayer::BadInput,
            format!("The tree doesn't know the symbol {}", mutex),
        )
    })?;
    let mutex_pretty = xref
        .meta
        .as_ref()
        .map(|meta| meta.pretty.to_string())
        .filter(|pretty| !pretty.is_empty())
        .unwrap_or_else(|| sym_short_name(mutex));
    let mut view = LockHeldView {
        mutex: sym,
        mutex_pretty,
        ..Default::default()
    };
    // Its class's annotations (see `Annotations`).
    view.annotations = annotations(&mut tracer, sym).await;
    // The guards' regions of it, in the files using it, and of the fields
    // which are it too, in theirs.
    let mut work: VecDeque<(Arc<FileIr>, usize, usize)> = VecDeque::new();
    let mut seen: HashSet<Ustr> = HashSet::new();
    let mut mutexes: Vec<Ustr> = vec![sym];
    let mut done: HashSet<Ustr> = HashSet::new();
    while let Some(mutex) = mutexes.iter().find(|m| !done.contains(*m)).copied() {
        done.insert(mutex);
        let Some(xref) = tracer.crossref(&mutex).await else {
            continue;
        };
        let paths: BTreeSet<String> = xref
            .uses
            .iter()
            .flatten()
            .map(|uses| uses.path.to_string())
            .collect();
        for path in paths {
            let Some(file) = tracer.file(&path).await else {
                continue;
            };
            for (index, func) in file.funcs.iter().enumerate() {
                for region in held_regions(func) {
                    if region.kind != LockKind::Acquire || region.mutex != Some(mutex) {
                        continue;
                    }
                    seen.insert(func.sym);
                    view.regions.push(ViewRegion {
                        how: How::Guard,
                        depth: 0,
                        via: vec![],
                        func: func.sym,
                        func_pretty: func.pretty.clone(),
                        path: func.path.clone(),
                        func_line: func.line,
                        func_end_line: func.end_line,
                        region,
                    });
                    work.push_back((file.clone(), index, view.regions.len() - 1));
                }
                // (A field initialized or assigned from it, or its address.)
                for alias in aliases_in(func, mutex) {
                    if mutexes.contains(&alias.sym) {
                        continue;
                    }
                    mutexes.push(alias.sym);
                    view.aliases.push(alias);
                }
            }
        }
    }
    // The methods requiring it, for their whole bodies.
    for method in view.annotations.requires.clone() {
        if seen.contains(&method) {
            continue;
        }
        for (file, index) in tracer.callee_functions(&method).await.into_iter().take(1) {
            let func = &file.funcs[index];
            seen.insert(func.sym);
            view.regions.push(ViewRegion {
                how: How::Contract,
                depth: 0,
                via: vec![],
                func: func.sym,
                func_pretty: func.pretty.clone(),
                path: func.path.clone(),
                func_line: func.line,
                func_end_line: func.end_line,
                region: held_body(func, Some(sym), &short(&view.mutex_pretty).to_string()),
            });
            work.push_back((file.clone(), index, view.regions.len() - 1));
        }
    }
    // The functions its proofs are passed to, and those called with it held,
    // breadth first.
    while let Some((file, index, at)) = work.pop_front() {
        let caller = &file.funcs[index];
        let from = view.regions[at].clone();
        for call in &from.region.calls {
            let proof = !call.proof_args.is_empty();
            // (Not macros' expansions' calls, ex: logging's, assertions',
            // profiler markers'.)
            if !proof
                && (from.depth >= CALLEE_DEPTH
                    || call.in_macro
                    || UNCARDED_CALLEES
                        .iter()
                        .any(|prefix| call.callee_pretty.starts_with(prefix)))
            {
                continue;
            }
            let mut targets = vec![];
            for callee in &call.callee {
                targets = tracer.callee_functions(callee).await;
                if !targets.is_empty() {
                    break;
                }
            }
            // (Nor an interface's method without a body of its own, whose
            // overrides the call would be followed to, if it has more than
            // that's followed to: some of them, not those called; ex:
            // nsIEventTarget::Dispatch's.)
            if !proof
                && !targets
                    .iter()
                    .any(|(file, index)| call.callee.contains(&file.funcs[*index].sym))
                && overrides(&mut tracer, &call.callee).await > CALL_FANOUT
            {
                continue;
            }
            for (target_file, target) in targets.into_iter().take(CALL_FANOUT) {
                let func = &target_file.funcs[target];
                if func.sym == caller.sym || !func.has_body || seen.contains(&func.sym) {
                    continue;
                }
                let region = match proof {
                    true => held_regions(func).into_iter().find(|r| {
                        r.kind == LockKind::Proof
                            && r.param
                                .is_some_and(|param| call.proof_args.contains(&param))
                    }),
                    false => None,
                };
                let (how, depth, mut region) = match region {
                    Some(region) => (How::Proof, from.depth, region),
                    None => (
                        How::Callee,
                        from.depth + 1,
                        held_body(func, Some(sym), &from.region.mutex_text),
                    ),
                };
                if how == How::Callee && depth > CALLEE_DEPTH {
                    continue;
                }
                if view.regions.len() >= MAX_REGIONS {
                    view.notes.push(format!(
                        "Only the first {} regions are shown; there are more.",
                        MAX_REGIONS
                    ));
                    work.clear();
                    break;
                }
                region.mutex = Some(sym);
                region.mutex_text = from.region.mutex_text.clone();
                seen.insert(func.sym);
                let mut via = from.via.clone();
                via.push(ViaStep {
                    func: caller.sym,
                    pretty: caller.pretty.clone(),
                    path: caller.path.clone(),
                    line: call.line,
                });
                view.regions.push(ViewRegion {
                    how,
                    depth,
                    via,
                    func: func.sym,
                    func_pretty: func.pretty.clone(),
                    path: func.path.clone(),
                    func_line: func.line,
                    func_end_line: func.end_line,
                    region,
                });
                work.push_back((target_file.clone(), target, view.regions.len() - 1));
            }
        }
    }
    // Its guarded fields' accesses outside the regions holding it.
    unguarded_uses(&mut tracer, &mut view).await;
    Ok(view)
}

/// How many overrides a call of `callees` has (their most).
async fn overrides(tracer: &mut Tracer<'_>, callees: &[Ustr]) -> usize {
    let mut most = 0;
    for callee in callees {
        if let Some(xref) = tracer.crossref(callee).await {
            most = most.max(
                xref.meta
                    .as_ref()
                    .map_or(0, |meta| meta.overridden_by_syms.len()),
            );
        }
    }
    most
}

/// The fields `func` initializes or assigns from `mutex` (its value, or its
/// address): which are it too (see `Alias`).
fn aliases_in(func: &crate::dataflow::ir::FuncIr, mutex: Ustr) -> Vec<Alias> {
    use crate::dataflow::ir::{Atom, EventKind};
    let mut out = vec![];
    for event in &func.events {
        let EventKind::Assign { place, vals, .. } = &event.kind else {
            continue;
        };
        let Some(field) = place.last_field() else {
            continue;
        };
        if field == mutex || out.iter().any(|alias: &Alias| alias.sym == field) {
            continue;
        }
        if vals.iter().any(|val| match &val.atom {
            Atom::Read(from) | Atom::Addr(from) => !val.derived && from.last_field() == Some(mutex),
            _ => false,
        }) {
            out.push(Alias {
                sym: field,
                name: crate::dataflow::locks::mutex_name(Some(&field), &func.place_name(place)),
                path: func.path.clone(),
                line: event.line,
            });
        }
    }
    out
}

/// The arguments of an annotation's call in a text (ex: `mMutex` of
/// `MOZ_GUARDED_BY(mMutex)`), each.
fn annotation_args<'t>(text: &'t str, macro_name: &str) -> Vec<&'t str> {
    let mut out = vec![];
    let mut rest = text;
    let open = format!("{}(", macro_name);
    while let Some(at) = rest.find(&open) {
        // (Not a longer macro's name's end, ex: MOZ_PT_GUARDED_BY's.)
        let before = rest[..at].chars().last();
        let after = &rest[at + open.len()..];
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            rest = after;
            continue;
        }
        let Some(close) = after.find(')') else {
            break;
        };
        out.extend(after[..close].split(',').map(|arg| {
            arg.trim()
                .trim_start_matches("this->")
                .trim_start_matches('&')
        }));
        rest = &after[close..];
    }
    out
}

/// What the mutex's class's thread-safety annotations say of it (see
/// `Annotations`): its fields' declarations' lines (`lineRange`, in the
/// class's file unless they say another's), and its methods' declarations'
/// (or else definitions') lines up to their ends.
async fn annotations(tracer: &mut Tracer<'_>, mutex: Ustr) -> Annotations {
    let mut out = Annotations::default();
    let (class, name) = field_parts(&mutex);
    if class.is_empty() {
        return out;
    }
    let class_sym = ustr(&format!("T_{}", class));
    let Some(class_xref) = tracer.crossref(&class_sym).await else {
        return out;
    };
    let Some(meta) = class_xref.meta.clone() else {
        return out;
    };
    let class_path = class_xref
        .definitions
        .iter()
        .flatten()
        .map(|def| def.path.to_string())
        .next();
    for field in &meta.fields {
        let (path, range) = match field.line_range.split_once('#') {
            Some((path, range)) if !path.is_empty() => (path.to_string(), range.to_string()),
            Some((_, range)) => match &class_path {
                Some(path) => (path.clone(), range.to_string()),
                None => continue,
            },
            None => continue,
        };
        let (first, last) = match range.split_once('-') {
            Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
            None => {
                let line = range.parse().unwrap_or(0);
                (line, line)
            }
        };
        if first == 0 {
            continue;
        }
        // (To its declaration's end: the annotation can be on the next line,
        // as `lineRange` is its name's.)
        let text = source_lines(tracer, &path, first, last.max(first) + 3).await;
        let text = match text.find(';') {
            Some(end) => &text[..end],
            None => &text,
        };
        if ["MOZ_GUARDED_BY", "MOZ_PT_GUARDED_BY"]
            .iter()
            .any(|annotation| annotation_args(text, annotation).contains(&name.as_str()))
        {
            out.guarded.push(field.sym);
        }
    }
    for method in &meta.methods {
        let (requires, unchecked) = symbol_annotations(tracer, method.sym, &name).await;
        if requires {
            out.requires.push(method.sym);
        }
        if unchecked {
            out.unchecked.push(method.sym);
        }
    }
    out
}

/// A file's lines `first..=last`.
async fn source_lines(tracer: &mut Tracer<'_>, path: &str, first: u32, last: u32) -> String {
    let Some(source) = tracer.source(path).await else {
        return String::new();
    };
    source
        .lines()
        .skip(first.saturating_sub(1) as usize)
        .take((last.saturating_sub(first) + 1) as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a function's (or a lambda's) declaration or definition says it
/// requires the mutex `name` held (`MOZ_REQUIRES`, `MOZ_REQUIRES_SHARED`),
/// and whether it turns the thread-safety analysis off
/// (`MOZ_NO_THREAD_SAFETY_ANALYSIS`): their texts from their lines to their
/// ends (`;` or `{`, a few lines at most), as either can have them.
async fn symbol_annotations(tracer: &mut Tracer<'_>, sym: Ustr, name: &str) -> (bool, bool) {
    let Some(xref) = tracer.crossref(&sym).await else {
        return (false, false);
    };
    let mut places: Vec<(String, u32)> = [&xref.declarations, &xref.definitions]
        .into_iter()
        .filter_map(|found| {
            let found = found.as_ref()?.first()?;
            Some((found.path.to_string(), found.lines.first()?.lineno))
        })
        .collect();
    // (A lambda's operator() has neither: where it's written, its use.)
    if places.is_empty()
        && xref
            .meta
            .as_ref()
            .is_none_or(|meta| meta.pretty.contains("(lambda"))
        && let Some(found) = xref.uses.as_ref().and_then(|uses| uses.first())
        && let Some(line) = found.lines.first()
    {
        places.push((found.path.to_string(), line.lineno));
    }
    let (mut requires, mut unchecked) = (false, false);
    for (path, line) in places {
        let text = source_lines(tracer, &path, line, line + 4).await;
        let text = match text.find([';', '{']) {
            Some(end) => &text[..end],
            None => &text,
        };
        requires |= ["MOZ_REQUIRES", "MOZ_REQUIRES_SHARED"]
            .iter()
            .any(|annotation| annotation_args(text, annotation).contains(&name));
        unchecked |= text.contains("MOZ_NO_THREAD_SAFETY_ANALYSIS");
    }
    (requires, unchecked)
}

/// The view's guarded fields' accesses outside the regions holding the
/// mutex (see `LockHeldView::unguarded`): their uses (by their crossref)
/// whose lines aren't in a region's held lines (a guard's region's but its
/// released parts and lambdas, or a function's), but in the methods requiring
/// it, those without the analysis, and constructors and destructors.
async fn unguarded_uses(tracer: &mut Tracer<'_>, view: &mut LockHeldView) {
    let mut held: BTreeMap<&str, Vec<(u32, u32)>> = BTreeMap::new();
    for region in &view.regions {
        let ranges = held.entry(region.path.as_str()).or_default();
        match region.how {
            How::Guard => {
                let mut out = region.region.released.clone();
                out.extend(region.region.lambdas.iter().copied());
                ranges.extend(crate::dataflow::locks::ranges_without(
                    region.region.line,
                    region.region.end_line,
                    &out,
                ));
                ranges.extend(region.region.called_lambdas.iter().copied());
            }
            _ => ranges.push((region.func_line, region.func_end_line)),
        }
    }
    let exempt: HashSet<Ustr> = view
        .annotations
        .requires
        .iter()
        .chain(&view.annotations.unchecked)
        .copied()
        .collect();
    let mut unguarded = vec![];
    let mut seen: HashSet<(Ustr, String, u32)> = HashSet::new();
    // (The functions and lambdas whose own annotations exempt them, by
    // symbol; see `symbol_annotations`.)
    let (_, name) = field_parts(&view.mutex);
    let mut annotated: BTreeMap<Ustr, bool> = BTreeMap::new();
    for field in view.annotations.guarded.clone() {
        let Some(xref) = tracer.crossref(&field).await else {
            continue;
        };
        for uses in xref.uses.iter().flatten() {
            for line in &uses.lines {
                let in_held = held.get(uses.path.as_str()).is_some_and(|ranges| {
                    ranges
                        .iter()
                        .any(|&(first, last)| first <= line.lineno && line.lineno <= last)
                });
                if in_held
                    || exempt.contains(&line.contextsym)
                    || !seen.insert((field, uses.path.to_string(), line.lineno))
                {
                    continue;
                }
                // (A function or lambda requiring it, or without the analysis,
                // by its own annotations, ex: a lambda's `MOZ_REQUIRES(mMutex)`,
                // or a definition's `MOZ_NO_THREAD_SAFETY_ANALYSIS`.)
                let exempted = match annotated.get(&line.contextsym) {
                    Some(exempted) => *exempted,
                    None => {
                        let (requires, unchecked) =
                            symbol_annotations(tracer, line.contextsym, &name).await;
                        annotated.insert(line.contextsym, requires || unchecked);
                        requires || unchecked
                    }
                };
                if exempted {
                    continue;
                }
                // (Constructors and destructors, which nothing shares the
                // object with yet, or anymore.)
                let (class, method) = method_parts(&line.context);
                let class_name = short(&class);
                if !class_name.is_empty()
                    && (method == class_name || method.strip_prefix('~') == Some(class_name))
                {
                    continue;
                }
                unguarded.push(UnguardedUse {
                    field,
                    path: uses.path.to_string(),
                    line: line.lineno,
                    context: line.context.to_string(),
                });
            }
        }
    }
    view.unguarded_count = unguarded.len();
    unguarded.truncate(MAX_UNGUARDED);
    view.unguarded = unguarded;
}

/// A facet value's path from its facet's top (see `view_facets`): each level's
/// value and name.
type ValuePath = Vec<(String, String)>;

/// A path of nested values from `/`-separated parts (ex: a subsystem's, or a
/// directory's parents'), under `top`'s values.
fn nested_path(top: &str, parts: &[&str]) -> ValuePath {
    let mut path = vec![];
    let mut value = top.to_string();
    for part in parts {
        value = match value.is_empty() {
            true => slug(part),
            false => format!("{}/{}", value, slug(part)),
        };
        path.push((value.clone(), part.to_string()));
    }
    path
}

/// A region's facets' values (see `view_facets`), as paths from their facets'
/// tops: how it holds the mutex (a guard, by its type; a proof; a callee, by
/// its depth); the fields it accesses (by class, then field, then how); the
/// calls it makes (by class, then method); the other mutexes it acquires; its
/// function (by class, then method); its file's subsystem and directories.
fn region_facet_values(
    view: &LockHeldView,
    region: &ViewRegion,
) -> Vec<(&'static str, Vec<ValuePath>)> {
    let how = match region.how {
        How::Guard => vec![
            ("guard".to_string(), "By a guard".to_string()),
            (
                format!("guard/{}", slug(short(&region.region.guard))),
                short(&region.region.guard).to_string(),
            ),
        ],
        How::Proof => vec![("proof".to_string(), "Proven held".to_string())],
        How::Contract => vec![("contract".to_string(), "Required held".to_string())],
        How::Callee => vec![
            ("callee".to_string(), "Called with it held".to_string()),
            (
                format!("callee/{}", region.depth),
                format!(
                    "{} call{} deep",
                    region.depth,
                    if region.depth == 1 { "" } else { "s" }
                ),
            ),
        ],
    };
    let mut fields = vec![];
    for field in &region.region.fields {
        let (class, _) = field_parts(&field.sym);
        let name = field.name.clone();
        // (A global's under "(globals)".)
        let (class_value, class_name) = match class.is_empty() {
            true => ("globals".to_string(), "(globals)".to_string()),
            false => (slug(&class), short(&class).to_string()),
        };
        let field_value = format!("{}/{}", class_value, slug(&name));
        let mut levels = vec![
            (class_value.clone(), class_name),
            (field_value.clone(), name.clone()),
        ];
        for (lines, value, label) in [
            (&field.reads, "read", "read"),
            (&field.writes, "written", "written"),
            (&field.calls, "methods", "methods called"),
        ] {
            if !lines.is_empty() {
                levels.push((format!("{}/{}", field_value, value), label.to_string()));
                fields.push(levels.clone());
                levels.pop();
            }
        }
    }
    let calls = region
        .region
        .calls
        .iter()
        .map(|call| {
            let (class, method) = method_parts(&call.callee_pretty);
            let class_value = slug(&class);
            vec![
                (
                    class_value.clone(),
                    match class.is_empty() {
                        true => "(functions)".to_string(),
                        false => short(&class).to_string(),
                    },
                ),
                (format!("{}/{}", class_value, slug(&method)), method),
            ]
        })
        .collect();
    let locks = region
        .region
        .locks
        .iter()
        .map(|lock| {
            let name = lock
                .mutex
                .map(|sym| {
                    let (class, field) = field_parts(&sym);
                    match class.is_empty() {
                        true => field,
                        false => format!("{}::{}", short(&class), field),
                    }
                })
                .unwrap_or_else(|| lock.mutex_text.clone());
            vec![(slug(&name), name)]
        })
        .collect();
    let (class, method) = method_parts(&region.func_pretty);
    let function = vec![vec![
        (
            slug(&class),
            match class.is_empty() {
                true => "(functions)".to_string(),
                false => short(&class).to_string(),
            },
        ),
        (format!("{}/{}", slug(&class), slug(&method)), method),
    ]];
    let mut out = vec![
        ("how", vec![how]),
        ("field", fields),
        ("call", calls),
        ("lock", locks),
        ("function", function),
    ];
    if let Some(subsystem) = view.subsystems.get(&region.path) {
        let parts: Vec<&str> = subsystem.split('/').collect();
        out.push(("subsystem", vec![nested_path("", &parts)]));
    }
    let dirs: Vec<&str> = region.path.split('/').collect();
    if dirs.len() > 1 {
        out.push(("directory", vec![nested_path("", &dirs[..dirs.len() - 1])]));
    }
    out
}

/// The values of a region's facets, for its card's `data-facets` (the
/// values of its paths, ancestors included).
fn flat_values(paths: &[ValuePath]) -> Vec<String> {
    paths
        .iter()
        .flatten()
        .map(|(value, _)| value.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The navigation panel's facets of the view (see `TextFile::facets`), whose
/// items are its regions' cards (see `region_facet_values`), with how many
/// regions have each value.
pub fn view_facets(view: &LockHeldView) -> Value {
    use crate::cmd_pipeline::facets::{FacetValueView, FacetView};
    #[derive(Default)]
    struct Level {
        name: String,
        count: u32,
        nested: BTreeMap<String, Level>,
    }
    let mut roots: BTreeMap<&'static str, Level> = BTreeMap::new();
    for region in &view.regions {
        for (key, paths) in region_facet_values(view, region) {
            let root = roots.entry(key).or_default();
            let mut counted: BTreeSet<String> = BTreeSet::new();
            for path in paths {
                let mut level = &mut *root;
                for (value, name) in path {
                    level = level.nested.entry(value.clone()).or_default();
                    level.name = name;
                    if counted.insert(value) {
                        level.count += 1;
                    }
                }
            }
        }
    }
    fn views(level: Level) -> Vec<FacetValueView> {
        let mut values: Vec<(String, Level)> = level.nested.into_iter().collect();
        values.sort_by(|(a_value, a), (b_value, b)| {
            b.count.cmp(&a.count).then_with(|| a_value.cmp(b_value))
        });
        values
            .into_iter()
            .map(|(value, level)| FacetValueView {
                value,
                name: level.name.clone(),
                title: level.name.clone(),
                count: level.count,
                values: views(level),
                ..Default::default()
            })
            .collect()
    }
    let mut facets = vec![];
    for (key, label) in [
        ("how", "How it's held"),
        ("field", "Fields accessed"),
        ("call", "Calls made"),
        ("lock", "Other locks acquired"),
        ("function", "Function"),
        ("subsystem", "Subsystem"),
        ("directory", "Directory"),
    ] {
        if let Some(level) = roots.remove(key)
            && !level.nested.is_empty()
        {
            facets.push(FacetView::new(key, label.to_string(), views(level)));
        }
    }
    json!({ "facets": facets, "noun": "region", "nouns": "regions" })
}

/// A field's accesses across the view's regions: in how many regions it's
/// read, written, and has methods called.
#[derive(Default)]
struct FieldSummary {
    reads: usize,
    writes: usize,
    calls: usize,
    regions: usize,
}

/// The rows of the view's cards' lines (see `fetch_rows`): each region's
/// card's lines (see `ViewRegion::lines`), at most `MAX_CARD_LINES` of them.
pub fn card_line_requests(view: &LockHeldView) -> BTreeMap<Ustr, BTreeSet<u32>> {
    let mut requests: BTreeMap<Ustr, BTreeSet<u32>> = BTreeMap::new();
    for region in &view.regions {
        let (first, last) = region.lines();
        let last = last.min(first + MAX_CARD_LINES - 1);
        requests
            .entry(ustr(&region.path))
            .or_default()
            .extend(first..=last);
    }
    requests
}

/// How many diagrams the view has at most (the biggest).
const MAX_GRAPHS: usize = 40;

/// A node of the view's diagrams (see `region_graphs`): a guard's region (or,
/// for a function holding the mutex in several blocks, the function, over
/// its regions), a function its proof is passed to or called with it held,
/// an unlock releasing it in a region, or another mutex acquired with it
/// held; its label (a function's pretty name, or the mutex's), what it is,
/// a link, and the cards (by index) it stands for.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphNode {
    pub kind: &'static str,
    pub label: String,
    pub sub: String,
    pub link: String,
    pub cards: Vec<usize>,
}

/// An edge of a diagram: a call made with the mutex held (from its region's
/// node to the function called), a region's in its function, or its unlock
/// or another mutex's acquisition; by its nodes' indices, with its lines.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
    pub lines: Vec<u32>,
}

/// A connected part of the view's regions' graph (see `region_graphs`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RegionGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// The view's diagrams: the graph of its regions (see `GraphNode`), callers
/// over callees, each connected part of it but those of a single node (a
/// region making no calls, which its card is enough for), the biggest first,
/// at most `MAX_GRAPHS`.
pub fn region_graphs(tree_name: &str, view: &LockHeldView) -> Vec<RegionGraph> {
    let mut nodes: Vec<GraphNode> = vec![];
    let mut keys: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges: BTreeMap<(usize, usize), BTreeSet<u32>> = BTreeMap::new();
    let mut node =
        |key: String, make: &dyn Fn() -> GraphNode, nodes: &mut Vec<GraphNode>| -> usize {
            *keys.entry(key).or_insert_with(|| {
                nodes.push(make());
                nodes.len() - 1
            })
        };
    // (The guards' regions by function, for the functions holding it in
    // several blocks.)
    let mut guards: BTreeMap<Ustr, Vec<usize>> = BTreeMap::new();
    for (i, region) in view.regions.iter().enumerate() {
        if region.how == How::Guard {
            guards.entry(region.func).or_default().push(i);
        }
    }
    // Each region's node.
    let mut region_nodes = vec![0; view.regions.len()];
    for (i, region) in view.regions.iter().enumerate() {
        let held = &region.region;
        let link = line_link(tree_name, &region.path, region.lines().0);
        region_nodes[i] = match region.how {
            How::Guard => {
                let several = guards.get(&region.func).is_some_and(|g| g.len() > 1);
                let at = node(
                    format!("region:{}", i),
                    &|| GraphNode {
                        kind: "guard",
                        label: match several {
                            true => short(&held.guard).to_string(),
                            false => region.func_pretty.clone(),
                        },
                        sub: format!(
                            "{}lines {}-{}",
                            match several {
                                true => String::new(),
                                false => format!("{}, ", short(&held.guard)),
                            },
                            held.line,
                            held.end_line
                        ),
                        link: link.clone(),
                        cards: vec![i],
                    },
                    &mut nodes,
                );
                if several {
                    let count = guards[&region.func].len();
                    let function = node(
                        format!("func:{}", region.func),
                        &|| GraphNode {
                            kind: "function",
                            label: region.func_pretty.clone(),
                            sub: format!("holds it in {} blocks", count),
                            link: line_link(tree_name, &region.path, region.func_line),
                            cards: guards[&region.func].clone(),
                        },
                        &mut nodes,
                    );
                    edges.entry((function, at)).or_default().insert(held.line);
                }
                at
            }
            how => node(
                format!("func:{}", region.func),
                &|| GraphNode {
                    kind: match how {
                        How::Proof => "proof",
                        How::Contract => "contract",
                        _ => "callee",
                    },
                    label: region.func_pretty.clone(),
                    sub: match how {
                        How::Proof => "passed its proof".to_string(),
                        How::Contract => "requires it held".to_string(),
                        _ => format!(
                            "called with it held, {} call{} deep",
                            region.depth,
                            if region.depth == 1 { "" } else { "s" }
                        ),
                    },
                    link: link.clone(),
                    cards: vec![i],
                },
                &mut nodes,
            ),
        };
    }
    for (i, region) in view.regions.iter().enumerate() {
        let held = &region.region;
        let at = region_nodes[i];
        // The call reaching it, from its caller's region containing the
        // call's line (or its caller's function).
        if let Some(step) = region.via.last() {
            let caller = view
                .regions
                .iter()
                .enumerate()
                .find(|(_, r)| {
                    r.func == step.func
                        && (r.how != How::Guard
                            || (r.region.line <= step.line && step.line <= r.region.end_line))
                })
                .map(|(c, _)| region_nodes[c]);
            if let Some(caller) = caller {
                edges.entry((caller, at)).or_default().insert(step.line);
            }
        }
        // Its unlocks.
        for &(first, last) in &held.released {
            let release = node(
                format!("release:{}:{}", i, first),
                &|| GraphNode {
                    kind: "release",
                    label: "releases it".to_string(),
                    sub: format!("lines {}-{}", first, last),
                    link: line_link(tree_name, &region.path, first),
                    cards: vec![i],
                },
                &mut nodes,
            );
            edges.entry((at, release)).or_default().insert(first);
        }
        // The other mutexes it acquires (the same mutex, one node).
        for lock in &held.locks {
            let name = lock
                .mutex
                .map(|sym| {
                    let (class, field) = field_parts(&sym);
                    match class.is_empty() {
                        true => field,
                        false => format!("{}::{}", short(&class), field),
                    }
                })
                .unwrap_or_else(|| lock.mutex_text.clone());
            let acquired = node(
                format!("lock:{}", name),
                &|| GraphNode {
                    kind: "lock",
                    label: name.clone(),
                    sub: format!("acquired with it held ({})", short(&lock.guard)),
                    link: line_link(tree_name, &region.path, lock.line),
                    cards: vec![],
                },
                &mut nodes,
            );
            if !nodes[acquired].cards.contains(&i) {
                nodes[acquired].cards.push(i);
            }
            edges.entry((at, acquired)).or_default().insert(lock.line);
        }
    }
    // Its connected parts (union-find).
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        let mut i = i;
        while parent[i] != root {
            let next = parent[i];
            parent[i] = root;
            i = next;
        }
        root
    }
    for &(from, to) in edges.keys() {
        let (a, b) = (find(&mut parent, from), find(&mut parent, to));
        if a != b {
            parent[a] = b;
        }
    }
    let mut parts: BTreeMap<usize, RegionGraph> = BTreeMap::new();
    let mut local: Vec<usize> = vec![0; nodes.len()];
    for (i, graph_node) in nodes.iter().enumerate() {
        let part = parts.entry(find(&mut parent, i)).or_default();
        local[i] = part.nodes.len();
        part.nodes.push(graph_node.clone());
    }
    for ((from, to), lines) in edges {
        let part = parts.get_mut(&find(&mut parent, from)).unwrap();
        part.edges.push(GraphEdge {
            from: local[from],
            to: local[to],
            lines: lines.into_iter().collect(),
        });
    }
    let mut graphs: Vec<RegionGraph> = parts
        .into_values()
        .filter(|graph| graph.nodes.len() > 1)
        .collect();
    graphs.sort_by(|a, b| b.nodes.len().cmp(&a.nodes.len()));
    graphs.truncate(MAX_GRAPHS);
    graphs
}

/// The view's HTML (for the query page): its notice (the mutex, how many
/// regions hold it how, and the fields accessed with it held), and its
/// regions' cards (see `card_html`), which the navigation panel's facets
/// filter.
pub fn render_lock_held(
    tree_name: &str,
    view: &LockHeldView,
    rows: &UstrMap<crate::crash_stack::FileRows>,
) -> String {
    let count = |how: How| view.regions.iter().filter(|r| r.how == how).count();
    let plural = |n: usize, one: &str, many: &str| match n {
        1 => format!("1 {}", one),
        n => format!("{} {}", n, many),
    };
    let functions: BTreeSet<Ustr> = view
        .regions
        .iter()
        .filter(|r| r.how == How::Guard)
        .map(|r| r.func)
        .collect();
    let mut html = String::from(r#"<div class="lock-held-view">"#);
    html.push_str(&format!(
        r#"<div class="lock-held-notice"><p>The code that runs with <a class="lock-held-mutex" href="/{tree}/query/default?q={query}"><code>{pretty}</code></a> held: {guards} in {functions}{proofs}{callees}.  Each card is a region of code with it held (but where an unlock releases it, and the lambdas made there, which run later); the navigation panel's facets filter them, ex: by the fields they access.</p>"#,
        tree = escape(tree_name),
        query = crate::crash_stack::url_encode_query(&format!("symbol:{}", view.mutex)),
        pretty = escape(&view.mutex_pretty),
        guards = plural(count(How::Guard), "guard's region", "guards' regions"),
        functions = plural(functions.len(), "function", "functions"),
        proofs = match count(How::Proof) {
            0 => String::new(),
            n => format!(", {} its guards' proofs are passed to", plural(n, "function", "functions")),
        },
        callees = match count(How::Callee) {
            0 => String::new(),
            n => format!(
                ", and {} called with it held (to {} calls deep)",
                plural(n, "function", "functions"),
                CALLEE_DEPTH
            ),
        },
    ));
    // (What its class's annotations say, and the fields which are it too.)
    let mut said = vec![];
    if count(How::Contract) > 0 {
        said.push(format!(
            "{} of its class {} it held (<code>MOZ_REQUIRES</code>)",
            plural(count(How::Contract), "method", "methods"),
            if count(How::Contract) == 1 {
                "requires"
            } else {
                "require"
            }
        ));
    }
    if !view.annotations.guarded.is_empty() {
        said.push(format!(
            "it guards {} (<code>MOZ_GUARDED_BY</code>)",
            plural(view.annotations.guarded.len(), "field", "fields")
        ));
    }
    if !view.annotations.unchecked.is_empty() {
        said.push(format!(
            "{} of its class {} the thread-safety analysis off (<code>MOZ_NO_THREAD_SAFETY_ANALYSIS</code>)",
            plural(view.annotations.unchecked.len(), "method", "methods"),
            if view.annotations.unchecked.len() == 1 { "turns" } else { "turn" }
        ));
    }
    if !said.is_empty() {
        let mut said = said.join("; ");
        if let Some(first) = said.get(..1) {
            said = format!("{}{}", first.to_uppercase(), &said[1..]);
        }
        html.push_str(&format!(
            r#"<p class="lock-held-annotations">{}.</p>"#,
            said
        ));
    }
    if !view.aliases.is_empty() {
        html.push_str(&format!(
            r#"<p class="lock-held-aliases">It's also {}, initialized from it; their guards' regions are here too.</p>"#,
            view.aliases
                .iter()
                .map(|alias| format!(
                    r#"<a href="{}"><code>{}</code></a>"#,
                    line_link(tree_name, &alias.path, alias.line),
                    escape(&alias.name)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for note in &view.notes {
        html.push_str(&format!(
            r#"<p class="lock-held-note">{}</p>"#,
            escape(note)
        ));
    }
    // The fields accessed with it held.
    let mut fields: BTreeMap<Ustr, FieldSummary> = BTreeMap::new();
    let mut names: BTreeMap<Ustr, String> = BTreeMap::new();
    for region in &view.regions {
        for field in &region.region.fields {
            names.entry(field.sym).or_insert_with(|| field.name.clone());
            let summary = fields.entry(field.sym).or_default();
            summary.regions += 1;
            summary.reads += usize::from(!field.reads.is_empty());
            summary.writes += usize::from(!field.writes.is_empty());
            summary.calls += usize::from(!field.calls.is_empty());
        }
    }
    if !fields.is_empty() {
        let mut fields: Vec<(Ustr, FieldSummary)> = fields.into_iter().collect();
        fields.sort_by(|(a_sym, a), (b_sym, b)| {
            b.regions.cmp(&a.regions).then_with(|| a_sym.cmp(b_sym))
        });
        html.push_str(&format!(
            r#"<details class="lock-held-fields" open><summary>The fields accessed with it held ({})</summary><table><thead><tr><th>Field</th><th>Regions</th><th>Read in</th><th>Written in</th><th>Methods called in</th><th title="Whether its declaration says the mutex guards it (MOZ_GUARDED_BY)">Guarded by it</th></tr></thead><tbody>"#,
            fields.len()
        ));
        for (sym, summary) in &fields {
            let (class, _) = field_parts(sym);
            let name = names
                .get(sym)
                .cloned()
                .unwrap_or_else(|| sym_short_name(sym));
            let cell = |n: usize| match n {
                0 => String::new(),
                n => n.to_string(),
            };
            html.push_str(&format!(
                r#"<tr><td><a href="/{}/query/default?q={}">{}{}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                escape(tree_name),
                crate::crash_stack::url_encode_query(&format!("symbol:{}", sym)),
                match class.is_empty() {
                    true => String::new(),
                    false => format!("{}::", escape(short(&class))),
                },
                escape(&name),
                summary.regions,
                cell(summary.reads),
                cell(summary.writes),
                cell(summary.calls),
                match view.annotations.guarded.contains(sym) {
                    true => "✓",
                    false => "",
                },
            ));
        }
        html.push_str("</tbody></table></details>");
    }
    // Its guarded fields' accesses without it held.
    if view.unguarded_count > 0 {
        html.push_str(&format!(
            r#"<details class="lock-held-unguarded"><summary>Its guarded fields' accesses without it held, as far as this analysis sees ({})</summary><p class="lock-held-about">Outside the regions here (but in the methods requiring it, those without the thread-safety analysis, and constructors and destructors): ex: under an assertion that the current thread owns it, or in code only one thread runs.</p><ul>"#,
            view.unguarded_count
        ));
        for use_ in &view.unguarded {
            let (_, name) = field_parts(&use_.field);
            html.push_str(&format!(
                r#"<li><a href="{}"><code>{}</code> in {}</a></li>"#,
                line_link(tree_name, &use_.path, use_.line),
                escape(&name),
                escape(&use_.context)
            ));
        }
        if view.unguarded.len() < view.unguarded_count {
            html.push_str(&format!(
                "<li>… and {} more</li>",
                view.unguarded_count - view.unguarded.len()
            ));
        }
        html.push_str("</ul></details>");
    }
    html.push_str("</div>");
    // The diagrams of how they're called (see `region_graphs`; drawn by
    // lock-held.js).
    let graphs = region_graphs(tree_name, view);
    if !graphs.is_empty() {
        html.push_str(&format!(
            r#"<section class="lock-held-graphs-box"><h3 class="lock-held-heading">How they're called</h3><p class="lock-held-about">Each connected part of the graph of the regions holding it (a guard's, or a function's holding it in several blocks over them) and the functions they call with it held (and pass its proof to), callers over callees, with their unlocks and the other locks they acquire: each node's color says how many of the cards shown it stands for; clicking one goes to its card.</p><div class="crash-graphs lock-held-graphs"></div><script type="application/json" id="lock-held-graphs">{}</script></section>"#,
            serde_json::to_string(&graphs)
                .unwrap_or_default()
                .replace('<', "\\u003c")
        ));
    }
    html.push_str(r#"<div class="facet-items lock-held-cards">"#);
    for (i, region) in view.regions.iter().enumerate() {
        html.push_str(&card_html(tree_name, view, region, rows, i));
    }
    html.push_str("</div></div>");
    // (graph.js's layout, for the diagrams.)
    html.push_str(&format!(
        r#"<script defer src="/{tree}/static/js/graph.js"></script><script defer src="/{tree}/static/js/lock-held.js"></script>"#,
        tree = escape(tree_name)
    ));
    html
}

/// A link to a function's line.
fn line_link(tree_name: &str, path: &str, line: u32) -> String {
    format!(
        "/{}/source/{}#{}",
        escape(tree_name),
        escape(&crate::url_encode_path::url_encode_path(path)),
        line
    )
}

/// A region's card: its function and how it holds the mutex, its lines
/// (those with the mutex released, and in lambdas, dimmed; folded but around
/// what it does), and what it does with it held.
fn card_html(
    tree_name: &str,
    view: &LockHeldView,
    region: &ViewRegion,
    rows: &UstrMap<crate::crash_stack::FileRows>,
    index: usize,
) -> String {
    let held = &region.region;
    let facets: BTreeMap<&str, Vec<String>> = region_facet_values(view, region)
        .into_iter()
        .map(|(key, paths)| (key, flat_values(&paths)))
        .filter(|(_, values)| !values.is_empty())
        .collect();
    let caller = region.via.last();
    let caller_html = |what: &str| match caller {
        Some(step) => format!(
            r#"{}, from <a href="{}">{}</a>"#,
            what,
            line_link(tree_name, &step.path, step.line),
            escape(short(&step.pretty))
        ),
        None => what.to_string(),
    };
    let how = match region.how {
        How::Guard => format!(
            r#"holds it by <code>{}</code>{}"#,
            escape(short(&held.guard)),
            match held.released.as_slice() {
                [] => String::new(),
                released => format!(
                    ", but releases it at {}",
                    released
                        .iter()
                        .map(|(first, last)| match first == last {
                            true => format!("line {}", first),
                            false => format!("lines {}-{}", first, last),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        ),
        How::Proof => caller_html("is passed its proof"),
        How::Callee => caller_html("is called with it held"),
        How::Contract => "requires it held (<code>MOZ_REQUIRES</code>)".to_string(),
    };
    // (The lambdas made before it which it runs.)
    let how = match held.called_lambdas.as_slice() {
        [] => how,
        called => format!(
            "{}, and runs the lambda{} made at {} with it held",
            how,
            if called.len() == 1 { "" } else { "s" },
            called
                .iter()
                .map(|(first, last)| match first == last {
                    true => format!("line {}", first),
                    false => format!("lines {}-{}", first, last),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    // (Where its unlocks release it, a guard's region's or a function's.)
    let how = match (region.how, held.released.as_slice()) {
        (How::Guard, _) | (_, []) => how,
        (_, released) => format!(
            "{}, but releases it at {}",
            how,
            released
                .iter()
                .map(|(first, last)| match first == last {
                    true => format!("line {}", first),
                    false => format!("lines {}-{}", first, last),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let via = match region.via.len() {
        0 | 1 => String::new(),
        _ => format!(
            r#" <span class="lock-held-via">(via {})</span>"#,
            region
                .via
                .iter()
                .map(|step| escape(short(&step.pretty)))
                .collect::<Vec<_>>()
                .join(" → ")
        ),
    };
    let mut html = format!(
        r#"<section class="lock-held-card lock-held-{how_class}" id="lock-held-card-{index}" data-path="{id}" data-facets="{facets}"><div class="lock-held-card-header"><a class="lock-held-func" href="{link}">{func}</a> <span class="lock-held-how">{how}</span>{via}</div>"#,
        how_class = match region.how {
            How::Guard => "guard",
            How::Proof => "proof",
            How::Callee => "callee",
            How::Contract => "contract",
        },
        id = escape(&region.id()),
        facets = escape(&serde_json::to_string(&facets).unwrap_or_default()),
        link = line_link(tree_name, &region.path, region.func_line),
        func = escape(&region.func_pretty),
    );
    // Its lines: what it does folded around.
    let (first, last) = region.lines();
    let shown_last = last.min(first + MAX_CARD_LINES - 1);
    let in_ranges =
        |line: u32, ranges: &[(u32, u32)]| ranges.iter().any(|&(a, b)| a <= line && line <= b);
    let mut marked: BTreeSet<u32> = BTreeSet::new();
    marked.insert(held.line);
    marked.extend(held.calls.iter().map(|call| call.line));
    marked.extend(
        held.fields
            .iter()
            .flat_map(|field| field.reads.iter().chain(&field.writes).chain(&field.calls)),
    );
    marked.extend(held.locks.iter().map(|lock| lock.line));
    let marked: Vec<u32> = marked.into_iter().collect();
    let (_, shown) = crate::crash_stack::excerpt_lines(first, shown_last, 2, &marked, 1);
    let url = line_link(tree_name, &region.path, region.func_line);
    let url = url
        .rsplit_once('#')
        .map_or(url.clone(), |(url, _)| url.to_string());
    match rows.get(&ustr(&region.path)) {
        Some(file_rows) => {
            html.push_str(&format!(
                r#"<div class="file lock-held-rows" role="table" data-tree="{}" data-search="{}">"#,
                escape(tree_name),
                escape(&crate::file_format::config::search_url(tree_name))
            ));
            let mut folded = String::new();
            let mut folded_count = 0;
            let flush = |folded: &mut String, count: &mut usize, html: &mut String| {
                if *count == 0 {
                    return;
                }
                html.push_str(&format!(
                    r#"<div class="lock-held-fold" role="rowgroup" hidden>{}</div><button type="button" class="lock-held-unfold">Show {} more line{}</button>"#,
                    folded,
                    count,
                    if *count == 1 { "" } else { "s" }
                ));
                folded.clear();
                *count = 0;
            };
            for line in first..=shown_last {
                let Some(row) = file_rows.get(&line) else {
                    continue;
                };
                let class = if region.how == How::Guard && line == held.line {
                    Some("lock-held-guard-line")
                } else if in_ranges(line, &held.released)
                    || (region.how == How::Guard
                        && line < held.line
                        && !in_ranges(line, &held.called_lambdas))
                {
                    // (Released, or before its guard: not held.)
                    Some("lock-held-released")
                } else if in_ranges(line, &held.lambdas) {
                    Some("lock-held-lambda")
                } else {
                    None
                };
                let row_html = crate::format::excerpt_row_html(
                    &row.html,
                    line,
                    &url,
                    "sp-citation-lineno",
                    0,
                    class,
                );
                if shown.contains(&line) {
                    flush(&mut folded, &mut folded_count, &mut html);
                    html.push_str(&row_html);
                } else {
                    folded.push_str(&row_html);
                    folded_count += 1;
                }
            }
            flush(&mut folded, &mut folded_count, &mut html);
            if shown_last < last {
                html.push_str(&format!(
                    r#"<p class="lock-held-more"><a href="{}#{}">{} more lines</a></p>"#,
                    url,
                    shown_last + 1,
                    last - shown_last
                ));
            }
            html.push_str("</div>");
        }
        None => html.push_str(r#"<p class="lock-held-no-rows">Its lines couldn't be fetched.</p>"#),
    }
    // What it does with the mutex held.
    let mut does = vec![];
    if !held.fields.is_empty() {
        does.push(format!(
            "accesses {}",
            held.fields
                .iter()
                .map(|field| {
                    let mut how = vec![];
                    if !field.writes.is_empty() {
                        how.push("written");
                    }
                    if !field.calls.is_empty() {
                        how.push("methods");
                    }
                    if !field.reads.is_empty() && how.is_empty() {
                        how.push("read");
                    }
                    format!("<code>{}</code> ({})", escape(&field.name), how.join(", "))
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !held.locks.is_empty() {
        does.push(format!(
            "acquires {}",
            held.locks
                .iter()
                .map(|lock| format!(
                    "<code>{}</code> (line {})",
                    escape(&lock.mutex_text),
                    lock.line
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !does.is_empty() {
        html.push_str(&format!(
            r#"<p class="lock-held-does">With it held, it {}.</p>"#,
            does.join("; ")
        ));
    }
    html.push_str("</section>");
    html
}

/// The script of the view's cards' folded lines.
pub const CARDS_SCRIPT: &str = r#"<script>
document.addEventListener("click", event => {
  const unfold = event.target.closest(".lock-held-unfold");
  if (!unfold) {
    return;
  }
  unfold.previousElementSibling.hidden = false;
  unfold.remove();
});
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotation_args() {
        assert_eq!(
            annotation_args("  int mStatus MOZ_GUARDED_BY(mMutex);", "MOZ_GUARDED_BY"),
            vec!["mMutex"]
        );
        // (Not a longer macro's.)
        assert!(annotation_args("T* mP MOZ_PT_GUARDED_BY(mMutex);", "MOZ_GUARDED_BY").is_empty());
        assert_eq!(
            annotation_args(
                "void F() MOZ_REQUIRES(mMutex, this->mOther)",
                "MOZ_REQUIRES"
            ),
            vec!["mMutex", "mOther"]
        );
    }

    #[test]
    fn test_parts() {
        assert_eq!(
            field_parts("F_<T_mozilla::dom::Worker>_mQueue"),
            ("mozilla::dom::Worker".to_string(), "mQueue".to_string())
        );
        assert_eq!(
            method_parts("mozilla::dom::Worker::Dispatch"),
            ("mozilla::dom::Worker".to_string(), "Dispatch".to_string())
        );
        assert_eq!(method_parts("Wake"), (String::new(), "Wake".to_string()));
        assert_eq!(
            nested_path("", &["Core", "DOM: Workers"]),
            vec![
                ("core".to_string(), "Core".to_string()),
                ("core/dom-workers".to_string(), "DOM: Workers".to_string())
            ]
        );
    }
}
