//! Taint: values of types of interest (ex: principals) that parent actors
//! receive from content processes, which may be compromised, are tainted
//! until a validator (ex: `ContentParent::ValidatePrincipal`) checks them.
//! The subway map (see `subway`) draws tainted lines hazard-striped, from
//! their sources (the parameters of the `Recv` methods of messages
//! ipdl-analyze says come from content processes: `fromContent`, by their
//! protocols' toplevel managers' `[ChildProc=...]`) to the validations
//! blessing them.
//!
//! - What's tainted and what validates it: ontology-mapping.toml's labels,
//!   "taint:content" on types, and "detaint:N" (its Nth argument, from 0),
//!   "detaint:this" (its object) or "detaint" (all of them) on validators;
//!   and for a query, ad hoc (`taint-type:`, `detaint-by:`), to try a type or
//!   validator out.
//! - A value derived from a tainted one is tainted, and taint is tracked by
//!   its sources (lineage roots): a validation blesses its value's whole
//!   genealogy (the source and everything derived from it, before or after)
//!   in what it dominates (see `ir::Guard`: a failed validation ends the
//!   control flow, ex: by returning `IPC_FAIL`, which kills the content
//!   process), and in the functions called from there.
//! - A parameter whose type's fields are tainted (a C++ class's members of
//!   tainted types, an IPDL struct's or union's fields, also by their own
//!   fields) has a source for each of them, so that validating one field
//!   (`ValidatePrincipal(aInit.principal())`) doesn't bless the others; the
//!   parameter carries them all (see `TypeTaint`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use ustr::Ustr;

use super::frontend::FileIr;
use super::ir::{Atom, Call, EventKind, FuncIr, Root, Val};
use super::slice::{Slicing, Unit, place_keys};
use super::subway::{qualified_type, semantic_type};
use super::summary::untemplated_pretty;
use super::trace::Tracer;
use crate::file_format::analysis::BindingSlotKind;

/// The label of types whose values from content processes are tainted.
pub const TAINT_LABEL: &str = "taint:content";
/// The prefix of validators' labels (see the module's comment).
pub const DETAINT_LABEL: &str = "detaint";

/// How deeply to look into IPDL structs' fields for tainted types.
const MAX_STRUCT_DEPTH: usize = 3;
/// How deeply to look into callees for validators' wrappers (see
/// `Taint::wrapper_validated`), and for how many callees at most.
const MAX_WRAPPER_DEPTH: usize = 2;
const MAX_WRAPPER_LOOKUPS: usize = 32;

/// What's tainted, and what validates it, besides the ontology's labels.
#[derive(Clone, Debug, Default, Hash, PartialEq)]
pub struct TaintSpec {
    /// Types (by name, qualified or not; ex: "nsIPrincipal",
    /// "mozilla::ipc::PrincipalInfo").
    pub types: Vec<String>,
    /// Validators (by pretty name, or its last components; ex:
    /// "ContentParent::ValidatePrincipal"), which validate all of their
    /// arguments (and their objects).
    pub validators: Vec<String>,
    /// Functions (by pretty name, or its last components) whose parameters
    /// of tainted types are tainted, besides the `Recv` methods of messages
    /// from content processes (ex: entry points from JS, or for trying
    /// validators out on any code).
    pub sources: Vec<String>,
}

/// Which of a call's values a validator validates.
#[derive(Clone, Debug, PartialEq)]
pub enum Validated {
    Arg(usize),
    This,
    All,
}

/// Whether a pretty name (of a type, or a function) is `name`, or ends with
/// it as its last components.
fn names(pretty: &str, name: &str) -> bool {
    let pretty = untemplated_pretty(pretty);
    let name = name.trim();
    !name.is_empty()
        && (pretty == name
            || (pretty.len() > name.len()
                && pretty.ends_with(name)
                && pretty[..pretty.len() - name.len()].ends_with("::")))
}

/// The symbols of a type (by its last component and qualified name): by its
/// qualified name if it has one, else any type's of its name; and if only
/// aliases have its qualified name (ex: IPDL protocols' `typedef
/// mozilla::_foo::Structs Structs`, which Recv methods' parameters have), by
/// its name.  (Aliases have no structured records.)
async fn named_type_syms(tracer: &mut Tracer<'_>, short: &str, qualified: &str) -> Vec<Ustr> {
    let all = tracer.type_syms(short).await;
    let syms: Vec<Ustr> = all
        .iter()
        .filter(|(pretty, sym)| {
            !sym.starts_with("TA_") && (!qualified.contains("::") || names(pretty, qualified))
        })
        .map(|(_, sym)| *sym)
        .collect();
    if !syms.is_empty() {
        return syms;
    }
    all.into_iter()
        .filter(|(_, sym)| !sym.starts_with("TA_"))
        .map(|(_, sym)| sym)
        .collect()
}

/// The type of a container's or an array's elements, if it's one: IPDL's
/// `T[]`, `T?` and `nullable T`, and Gecko's arrays and spans (ex:
/// `nsTArray<mozilla::ipc::PrincipalInfo>`, `const nsTArray<T>&`).
fn element_type(ty: &str) -> Option<String> {
    const CONTAINERS: [&str; 7] = [
        "nsTArray",
        "CopyableTArray",
        "AutoTArray",
        "nsCOMArray",
        "Span",
        "FallibleTArray",
        "nsTArrayView",
    ];
    let ty = ty.trim();
    if let Some(element) = ty.strip_suffix("[]").or_else(|| ty.strip_suffix('?')) {
        return Some(element.trim().to_string());
    }
    if let Some(element) = ty.strip_prefix("nullable ") {
        return Some(element.trim().to_string());
    }
    let qualified = qualified_type(ty)?;
    let name = qualified.rsplit("::").next().unwrap_or(&qualified);
    if !CONTAINERS.contains(&name) {
        return None;
    }
    // (Its first template argument, at the top level.)
    let start = ty.find('<')? + 1;
    let mut depth = 0;
    for (i, c) in ty[start..].char_indices() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            '>' | ',' if depth == 0 => return Some(ty[start..start + i].trim().to_string()),
            _ => {}
        }
    }
    None
}

/// The most functions a taint audit maps (see `audit_slicing`).
pub const MAX_AUDITED: usize = 48;

/// A taint audit's slicing (for `subway::subway_map`, with the type
/// tainted): a cluster for each function receiving a message from content
/// processes with parameters of a type (by name, qualified or not), or of
/// IPDL structs (or unions) containing it, of those parameters' values.  The
/// functions are found by the type's uses (in their declarations'
/// parameters, and in IPDL structs' fields): the `Recv` methods (or the
/// generated declarations they override) of messages marked `fromContent`.
pub async fn audit_slicing(tracer: &mut Tracer<'_>, ty: &str) -> Slicing {
    let mut notes = vec![];
    let (Some(short), Some(qualified)) = (semantic_type(ty), qualified_type(ty)) else {
        notes.push(format!("{:?} isn't a type.", ty));
        return Slicing::empty(notes);
    };
    let type_syms: Vec<Ustr> = named_type_syms(tracer, &short, &qualified).await;
    if type_syms.is_empty() {
        notes.push(format!("No type is named {:?}.", ty));
    }
    // (The functions whose declarations use it (or its aliases), by name
    // `Recv…`; the messages whose parameters it is, for an IPDL type; and the
    // IPDL structs and unions containing it (by their fields' uses of it in
    // .ipdl files, or their generated classes' members'), whose uses are
    // followed in turn, at most `MAX_STRUCT_DEPTH` deep.)
    let mut candidates: BTreeSet<Ustr> = BTreeSet::new();
    let mut messages: BTreeSet<Ustr> = BTreeSet::new();
    let mut seen: BTreeSet<Ustr> = type_syms.iter().copied().collect();
    let mut level: Vec<Ustr> = type_syms.clone();
    for _ in 0..=MAX_STRUCT_DEPTH {
        let mut contexts: BTreeSet<(Ustr, Ustr)> = BTreeSet::new();
        for sym in &level {
            let Some(xref) = tracer.crossref(sym).await else {
                continue;
            };
            for result in xref.uses.iter().flatten() {
                for line in &result.lines {
                    if !line.contextsym.is_empty() {
                        contexts.insert((line.contextsym, line.context));
                    }
                }
            }
        }
        // (The fields' and classes' crossref entries, at once.)
        tracer
            .prefetch(
                contexts
                    .iter()
                    .map(|(sym, _)| *sym)
                    .filter(|sym| sym.starts_with("IPDL_") || sym.starts_with("T_")),
                [],
            )
            .await;
        let mut next = vec![];
        for (contextsym, context) in contexts {
            let name = context.rsplit("::").next().unwrap_or("");
            if contextsym.starts_with("IPC_")
                && let Some((message, _param)) = contextsym.rsplit_once("__")
            {
                messages.insert(ustr::ustr(message));
            } else if contextsym.starts_with("IPDL_") || contextsym.starts_with("T_") {
                // (An IPDL field's struct, or a generated class's.)
                let container = tracer.crossref(&contextsym).await.and_then(|xref| {
                    let meta = xref.meta.as_ref()?;
                    match contextsym.starts_with("IPDL_") {
                        true => meta.parent_sym,
                        false => Some(meta.slot_owner.as_ref()?.sym),
                    }
                });
                if let Some(container) = container
                    && container.starts_with("IPDL_")
                    && seen.insert(container)
                {
                    next.push(container);
                }
            } else if name.starts_with("Recv") {
                candidates.insert(contextsym);
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    for message in messages {
        if let Some(xref) = tracer.crossref(&message).await
            && let Some(meta) = &xref.meta
        {
            candidates.extend(
                meta.binding_slots
                    .iter()
                    .filter(|slot| slot.props.slot_kind == BindingSlotKind::Recv)
                    .map(|slot| slot.sym),
            );
        }
    }
    // (Their implementations: themselves, or the overrides of generated
    // declarations.)
    let mut impls: BTreeMap<Ustr, Ustr> = BTreeMap::new();
    for sym in candidates {
        let Some(message) = tracer.recv_message(&sym).await else {
            continue;
        };
        if !tracer.from_content(&message).await {
            continue;
        }
        let Some(xref) = tracer.crossref(&sym).await else {
            continue;
        };
        let generated = xref
            .definitions
            .iter()
            .flatten()
            .next()
            .is_none_or(|def| def.path.starts_with("__GENERATED__/"));
        if !generated {
            impls.insert(sym, message);
            continue;
        }
        for imp in xref
            .meta
            .as_ref()
            .map(|meta| meta.overridden_by_syms.clone())
            .unwrap_or_default()
        {
            impls.insert(imp, message);
        }
    }
    if impls.len() > MAX_AUDITED {
        notes.push(format!(
            "The audit maps {} of the {} functions receiving {} from content processes.",
            MAX_AUDITED,
            impls.len(),
            ty
        ));
    }
    let mut taint = Taint::new(TaintSpec {
        types: vec![ty.to_string()],
        ..Default::default()
    });
    let mut units: Vec<Unit> = vec![];
    let mut clusters: Vec<Vec<usize>> = vec![];
    for (sym, message) in impls.into_iter().take(MAX_AUDITED) {
        let Some(path) = tracer
            .crossref(&sym)
            .await
            .and_then(|xref| Some(xref.definitions.as_ref()?.first()?.path.to_string()))
        else {
            continue;
        };
        let Some(file) = tracer.file(&path).await else {
            continue;
        };
        let Some(&fi) = file.by_sym.get(&sym) else {
            continue;
        };
        let f = &file.funcs[fi];
        let mut keys = BTreeSet::new();
        for i in taint.tainted_params(tracer, f, Some(&message)).await {
            place_keys(f, &f.params[i].place, &mut keys);
        }
        if keys.is_empty() {
            continue;
        }
        clusters.push(vec![units.len()]);
        units.push(Unit::of_values(units.len(), &path, f, keys));
    }
    if units.is_empty() {
        notes.push(format!(
            "No functions receive {} from content processes (in their messages' parameters).",
            ty
        ));
    }
    Slicing {
        units,
        links: vec![],
        clusters,
        other: vec![],
        notes,
    }
}

/// Whether a type is a builtin or fixed-width integer one (ex: `int`,
/// `uint32_t`), which no labels name.
fn is_builtin(ty: &str) -> bool {
    const BUILTINS: [&str; 14] = [
        "bool", "char", "char16_t", "char32_t", "short", "int", "long", "float", "double",
        "unsigned", "signed", "size_t", "void", "_Bool",
    ];
    BUILTINS.contains(&ty)
        || ty.starts_with("unsigned ")
        || ty.starts_with("long ")
        || (ty.ends_with("_t")
            && !ty.contains("::")
            && ty.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
}

/// A type's taint: whether its values are tainted (the spec's type, or
/// labeled "taint:content"), else its fields that are.
#[derive(Clone, Debug, Default)]
pub struct TypeTaint {
    pub whole: bool,
    pub fields: Vec<TaintedField>,
}

/// A tainted field of a type: its name, the symbols its places' keys have
/// (a C++ member's `F_` symbol; an IPDL field's `IPDL_` one, and its C++
/// member's), and its type.
#[derive(Clone, Debug)]
pub struct TaintedField {
    pub name: String,
    pub syms: Vec<Ustr>,
    pub ty: String,
}

/// A type's fields, by its symbol: a C++ class's members, or an IPDL struct's
/// (or union's) fields: (name, their keys' symbols, type).
async fn type_fields(tracer: &mut Tracer<'_>, sym: &Ustr) -> Vec<(String, Vec<Ustr>, String)> {
    let Some(xref) = tracer.crossref(sym).await else {
        return vec![];
    };
    let Some(meta) = &xref.meta else {
        return vec![];
    };
    let short = |pretty: &str| pretty.rsplit("::").next().unwrap_or(pretty).to_string();
    let mut out = vec![];
    if sym.starts_with("IPDL_") {
        let fields: Vec<Ustr> = meta.fields.iter().map(|f| f.sym).collect();
        for field in fields {
            let Some(fxref) = tracer.crossref(&field).await else {
                continue;
            };
            let Some(fmeta) = &fxref.meta else {
                continue;
            };
            let Some(ty) = &fmeta.type_pretty else {
                continue;
            };
            let mut syms = vec![field];
            syms.extend(
                fmeta
                    .binding_slots
                    .iter()
                    .filter(|slot| slot.props.slot_kind == BindingSlotKind::Member)
                    .map(|slot| slot.sym),
            );
            out.push((short(&fmeta.pretty), syms, ty.to_string()));
        }
    } else {
        for field in &meta.fields {
            if field.type_pretty.is_empty() || field.sym.is_empty() {
                continue;
            }
            // (IPDL's generated members are named with a trailing `_`.)
            let name = short(&field.pretty).trim_end_matches('_').to_string();
            out.push((name, vec![field.sym], field.type_pretty.to_string()));
        }
    }
    out
}

/// A taint spec's answers, cached.
pub struct Taint {
    pub spec: TaintSpec,
    types: HashMap<String, TypeTaint>,
    validators: HashMap<Ustr, Option<Vec<Validated>>>,
    /// The parameters functions validate when they succeed (see
    /// `wrapper_validated`), by symbol.
    wrappers: HashMap<Ustr, BTreeSet<usize>>,
    wrapper_lookups: usize,
}

impl Taint {
    pub fn new(spec: TaintSpec) -> Self {
        Taint {
            spec,
            types: HashMap::new(),
            validators: HashMap::new(),
            wrappers: HashMap::new(),
            wrapper_lookups: 0,
        }
    }

    /// Whether values of a type (as a declaration has it, ex: `const
    /// nsCOMPtr<nsIPrincipal>&`) from content processes are tainted, whole or
    /// by their fields (see `type_taint`).
    pub async fn is_tainted_type(&mut self, tracer: &mut Tracer<'_>, ty: &str) -> bool {
        let taint = self.type_taint(tracer, ty).await;
        taint.whole || !taint.fields.is_empty()
    }

    /// A type's taint (see `TypeTaint`): the spec's, or labeled so, whole;
    /// or for a C++ class, its members of types tainted whole, and for an
    /// IPDL struct or union, its fields of tainted types (whole, or by their
    /// fields, which taint the field whole).  (Not C++ members' members:
    /// classes like ContentParent have hundreds, of types with as many.)
    pub async fn type_taint(&mut self, tracer: &mut Tracer<'_>, ty: &str) -> TypeTaint {
        Box::pin(self.type_taint_at(tracer, ty, 0, true)).await
    }

    async fn type_taint_at(
        &mut self,
        tracer: &mut Tracer<'_>,
        ty: &str,
        depth: usize,
        with_fields: bool,
    ) -> TypeTaint {
        let (Some(short), Some(qualified)) = (semantic_type(ty), qualified_type(ty)) else {
            return TypeTaint::default();
        };
        let key = match with_fields {
            true => qualified.clone(),
            false => format!("{} (whole)", qualified),
        };
        if let Some(cached) = self.types.get(&key) {
            return cached.clone();
        }
        if is_builtin(&qualified) && !self.spec.types.contains(&qualified) {
            return TypeTaint::default();
        }
        // (A container's or an array's values are its elements': tainted
        // whole if they're tainted at all.)
        if let Some(element) = element_type(ty) {
            let element = Box::pin(self.type_taint_at(tracer, &element, depth, with_fields)).await;
            let taint = TypeTaint {
                whole: element.whole || !element.fields.is_empty(),
                fields: vec![],
            };
            self.types.insert(key, taint.clone());
            return taint;
        }
        // (Not tainted while it's being looked at, for recursive types.)
        self.types.insert(key.clone(), TypeTaint::default());
        let mut taint = TypeTaint {
            whole: self
                .spec
                .types
                .iter()
                .any(|name| names(&qualified, name) || name == &short),
            fields: vec![],
        };
        // (Its symbols; see `named_type_syms`.)
        let syms: Vec<Ustr> = match taint.whole {
            true => vec![],
            false => named_type_syms(tracer, &short, &qualified).await,
        };
        for sym in &syms {
            if tracer.labels(sym).await.contains(&ustr::ustr(TAINT_LABEL)) {
                taint.whole = true;
                break;
            }
        }
        if !taint.whole && with_fields && depth < MAX_STRUCT_DEPTH {
            for sym in syms {
                let ipdl = sym.starts_with("IPDL_");
                for (name, field_syms, field_ty) in type_fields(tracer, &sym).await {
                    let field_taint =
                        Box::pin(self.type_taint_at(tracer, &field_ty, depth + 1, ipdl)).await;
                    if !field_taint.whole && field_taint.fields.is_empty() {
                        continue;
                    }
                    match taint.fields.iter_mut().find(|f| f.name == name) {
                        Some(field) => {
                            for sym in field_syms {
                                if !field.syms.contains(&sym) {
                                    field.syms.push(sym);
                                }
                            }
                        }
                        None => taint.fields.push(TaintedField {
                            name,
                            syms: field_syms,
                            ty: field_ty,
                        }),
                    }
                }
            }
        }
        self.types.insert(key, taint.clone());
        taint
    }

    /// What a call validates, if its callee is a validator: the spec's (all
    /// of its values), or labeled so.
    pub async fn validated(&mut self, tracer: &mut Tracer<'_>, call: &Call) -> Vec<Validated> {
        if self
            .spec
            .validators
            .iter()
            .any(|name| names(&call.callee_pretty, name))
        {
            return vec![Validated::All];
        }
        let mut out = vec![];
        for sym in &call.callee {
            if let Some(cached) = self.validators.get(sym) {
                out.extend(cached.iter().flatten().cloned());
                continue;
            }
            let mut validated = vec![];
            for label in tracer.labels(sym).await {
                let Some(rest) = label.strip_prefix(DETAINT_LABEL) else {
                    continue;
                };
                match rest.strip_prefix(':') {
                    None if rest.is_empty() => validated.push(Validated::All),
                    Some("this") => validated.push(Validated::This),
                    Some(n) => {
                        if let Ok(n) = n.parse() {
                            validated.push(Validated::Arg(n));
                        }
                    }
                    None => {}
                }
            }
            let validated = (!validated.is_empty()).then_some(validated);
            out.extend(validated.iter().flatten().cloned());
            self.validators.insert(*sym, validated);
        }
        out
    }

    /// What a call of a function validates, if its callee wraps validators:
    /// the arguments of the parameters its callee validates whenever it
    /// succeeds (see `wraps`).
    pub async fn wrapper_validated(
        &mut self,
        tracer: &mut Tracer<'_>,
        file: &Arc<FileIr>,
        fi: usize,
        c: usize,
    ) -> Vec<Validated> {
        Box::pin(self.wrapper_validated_at(tracer, file, fi, c, 0)).await
    }

    async fn wrapper_validated_at(
        &mut self,
        tracer: &mut Tracer<'_>,
        file: &Arc<FileIr>,
        fi: usize,
        c: usize,
        depth: usize,
    ) -> Vec<Validated> {
        let mut out = vec![];
        let targets = tracer.call_targets(file, fi, c).await;
        for (tfile, tfi, params) in targets {
            let tsym = tfile.funcs[tfi].sym;
            let validated = match self.wrappers.get(&tsym) {
                Some(cached) => cached.clone(),
                None => {
                    if self.wrapper_lookups >= MAX_WRAPPER_LOOKUPS {
                        continue;
                    }
                    self.wrapper_lookups += 1;
                    // (Not validating while it's being looked at, for
                    // recursion.)
                    self.wrappers.insert(tsym, BTreeSet::new());
                    let validated = self.wraps(tracer, &tfile, tfi, depth).await;
                    self.wrappers.insert(tsym, validated.clone());
                    validated
                }
            };
            for (ai, pi) in params.iter().enumerate() {
                if let Some(pi) = pi
                    && validated.contains(pi)
                {
                    out.push(Validated::Arg(ai));
                }
            }
        }
        out
    }

    /// The parameters a function validates whenever it succeeds: by a call
    /// of a validator (labeled, the spec's, or a wrapper, at most
    /// `MAX_WRAPPER_DEPTH` deep) of the parameter (or of what's made from it
    /// by a call, ex: `aPrincipal.get()`), where each of its returns is
    /// either dominated by the validation (see `ir::Guard`), or the
    /// validation's result itself (`return Validate(aPrincipal);`), or a
    /// failure (`false`, `nullptr`, an `NS_ERROR_…`, `IPC_FAIL`).  (Not
    /// functions without returns, which would validate their callers'
    /// arguments in the rest of their blocks: guards are conditions'.)
    async fn wraps(
        &mut self,
        tracer: &mut Tracer<'_>,
        file: &Arc<FileIr>,
        fi: usize,
        depth: usize,
    ) -> BTreeSet<usize> {
        let f = &file.funcs[fi];
        let returns: Vec<(usize, &[Val])> = f
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.in_lambda)
            .filter_map(|(ei, e)| match &e.kind {
                EventKind::Return { vals } => Some((ei, vals.as_slice())),
                _ => None,
            })
            .collect();
        if returns.is_empty() {
            return BTreeSet::new();
        }
        let mut out = BTreeSet::new();
        for (pi, param) in f.params.iter().enumerate() {
            let root = &param.place.root;
            // (The calls validating it.)
            let mut validating: BTreeSet<usize> = BTreeSet::new();
            for (c, call) in f.calls.iter().enumerate() {
                if !call
                    .args
                    .iter()
                    .chain(call.receiver.iter())
                    .any(|arg| reads_root(f, &arg.vals, root, 1))
                {
                    continue;
                }
                let mut validated = self.validated(tracer, call).await;
                if validated.is_empty() && depth + 1 < MAX_WRAPPER_DEPTH {
                    validated =
                        Box::pin(self.wrapper_validated_at(tracer, file, fi, c, depth + 1)).await;
                }
                let validates = validated.iter().any(|v| {
                    let vals: Vec<&[Val]> = match v {
                        Validated::Arg(i) => call
                            .args
                            .get(*i)
                            .map(|a| a.vals.as_slice())
                            .into_iter()
                            .collect(),
                        Validated::This => {
                            call.receiver.iter().map(|r| r.vals.as_slice()).collect()
                        }
                        Validated::All => call
                            .args
                            .iter()
                            .chain(call.receiver.iter())
                            .map(|a| a.vals.as_slice())
                            .collect(),
                    };
                    vals.into_iter().any(|vals| reads_root(f, vals, root, 1))
                });
                if validates {
                    validating.insert(c);
                }
            }
            if validating.is_empty() {
                continue;
            }
            let guards: Vec<(usize, usize)> = f
                .guards
                .iter()
                .filter(|g| validating.contains(&g.call))
                .map(|g| (g.start, g.end))
                .collect();
            let all = returns.iter().all(|(ei, vals)| {
                guards.iter().any(|(start, end)| start <= ei && ei < end)
                    || is_failure(f, vals)
                    || matches!(vals, [Val { atom: Atom::Call(c), derived: false }] if validating.contains(c))
            });
            if all {
                out.insert(pi);
            }
        }
        out
    }

    /// Why a function's parameters are taint sources: the IPC message whose
    /// `Recv` method it is (or implements), if its parent actor receives it
    /// from content processes.
    pub async fn source_message(&mut self, tracer: &mut Tracer<'_>, f: &FuncIr) -> Option<Ustr> {
        let message = tracer.recv_message(&f.sym).await?;
        tracer.from_content(&message).await.then_some(message)
    }

    /// Whether the spec's `sources` name a function.
    pub fn is_source(&self, f: &FuncIr) -> bool {
        self.spec.sources.iter().any(|name| names(&f.pretty, name))
    }

    /// The tainted parameters (by index) of a function receiving a message
    /// from content processes (see `source_message`), or a source of the
    /// spec's: its parameters of tainted types, but for a message, only the
    /// message's (by its `param` binding slots, not its sync reply's out
    /// parameters).  (Not by their types' constness: IPDL passes interfaces as
    /// non-const pointers, ex: `nsIPrincipal* aPrincipal`.)
    pub async fn tainted_params(
        &mut self,
        tracer: &mut Tracer<'_>,
        f: &FuncIr,
        message: Option<&Ustr>,
    ) -> BTreeSet<usize> {
        let params = match message {
            Some(message) => tracer.message_params(message).await,
            None => BTreeSet::new(),
        };
        let mut out = BTreeSet::new();
        for (i, param) in f.params.iter().enumerate() {
            let in_message = params.is_empty() || params.contains(&i);
            if in_message
                && let Some(ty) = &param.type_pretty
                && self.is_tainted_type(tracer, ty).await
            {
                out.insert(i);
            }
        }
        out
    }
}

/// Whether values read a place with a root (or are calls' results of
/// calls reading it, `depth` calls deep).
fn reads_root(f: &FuncIr, vals: &[Val], root: &Root, depth: usize) -> bool {
    vals.iter().any(|v| match &v.atom {
        Atom::Read(p) | Atom::Addr(p) => &p.root == root,
        Atom::Call(c) if depth > 0 => f.calls.get(*c).is_some_and(|call| {
            call.args
                .iter()
                .chain(call.receiver.iter())
                .any(|arg| reads_root(f, &arg.vals, root, depth - 1))
        }),
        _ => false,
    })
}

/// Whether a return's values are a failure: `false`, `nullptr`, `0`, an
/// `NS_ERROR_…`, or `IPC_FAIL`'s `IPCResult::Fail`.
fn is_failure(f: &FuncIr, vals: &[Val]) -> bool {
    !vals.is_empty()
        && vals.iter().all(|v| match &v.atom {
            Atom::Const(c) => {
                matches!(c.as_str(), "false" | "nullptr" | "NULL" | "0")
                    || c.starts_with("NS_ERROR_")
            }
            Atom::Call(c) => f.calls.get(*c).is_some_and(|call| {
                call.callee_pretty.ends_with("IPCResult::Fail")
                    || call.callee_pretty.ends_with("IPCResult::FailForTesting")
            }),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_type() {
        assert_eq!(element_type("SActors[]").as_deref(), Some("SActors"));
        assert_eq!(element_type("nullable PFoo").as_deref(), Some("PFoo"));
        assert_eq!(
            element_type("PrincipalInfo?").as_deref(),
            Some("PrincipalInfo")
        );
        assert_eq!(
            element_type("const nsTArray<mozilla::ipc::PrincipalInfo> &").as_deref(),
            Some("mozilla::ipc::PrincipalInfo")
        );
        assert_eq!(
            element_type("AutoTArray<RefPtr<nsIPrincipal>, 4>").as_deref(),
            Some("RefPtr<nsIPrincipal>")
        );
        assert_eq!(element_type("nsIPrincipal *"), None);
    }

    #[test]
    fn test_names() {
        assert!(names(
            "mozilla::dom::ContentParent::ValidatePrincipal",
            "ContentParent::ValidatePrincipal"
        ));
        assert!(names(
            "mozilla::dom::ContentParent::ValidatePrincipal",
            "ValidatePrincipal"
        ));
        assert!(!names(
            "mozilla::dom::ContentParent::ValidatePrincipal",
            "Principal"
        ));
        assert!(names("mozilla::ipc::PrincipalInfo", "PrincipalInfo"));
        assert!(!names("mozilla::ipc::PrincipalInfo", ""));
    }
}
