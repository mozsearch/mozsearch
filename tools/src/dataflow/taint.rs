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
//! - A parameter of an IPDL struct (or union) type with tainted fields is
//!   tainted, as a whole.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use ustr::Ustr;

use super::frontend::FileIr;
use super::ir::{Atom, Call, EventKind, FuncIr, Root, Val};
use super::subway::{qualified_type, semantic_type};
use super::summary::untemplated_pretty;
use super::trace::Tracer;

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

/// A taint spec's answers, cached.
pub struct Taint {
    pub spec: TaintSpec,
    types: HashMap<String, bool>,
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
    /// nsCOMPtr<nsIPrincipal>&`) from content processes are tainted: the
    /// spec's, or labeled so, or an IPDL struct or union with fields that
    /// are.
    pub async fn is_tainted_type(&mut self, tracer: &mut Tracer<'_>, ty: &str) -> bool {
        self.tainted_type_at(tracer, ty, 0).await
    }

    async fn tainted_type_at(&mut self, tracer: &mut Tracer<'_>, ty: &str, depth: usize) -> bool {
        let (Some(short), Some(qualified)) = (semantic_type(ty), qualified_type(ty)) else {
            return false;
        };
        if let Some(&cached) = self.types.get(&qualified) {
            return cached;
        }
        if is_builtin(&qualified) && !self.spec.types.contains(&qualified) {
            return false;
        }
        // (Not tainted while it's being looked at, for recursive types.)
        self.types.insert(qualified.clone(), false);
        let mut tainted = self
            .spec
            .types
            .iter()
            .any(|name| names(&qualified, name) || name == &short);
        if !tainted {
            // (Its symbols: by its qualified name if it has one, else any
            // type of its name.)
            let syms: Vec<Ustr> = tracer
                .type_syms(&short)
                .await
                .into_iter()
                .filter(|(pretty, _)| !qualified.contains("::") || names(pretty, &qualified))
                .map(|(_, sym)| sym)
                .collect();
            for sym in syms {
                if tracer.labels(&sym).await.contains(&ustr::ustr(TAINT_LABEL)) {
                    tainted = true;
                    break;
                }
                // (An IPDL struct's or union's fields.)
                if depth < MAX_STRUCT_DEPTH && sym.starts_with("IPDL_") {
                    let field_types: Vec<String> = match tracer.crossref(&sym).await {
                        Some(xref) => {
                            let fields: Vec<Ustr> = xref
                                .meta
                                .as_ref()
                                .map(|meta| meta.fields.iter().map(|f| f.sym).collect())
                                .unwrap_or_default();
                            let mut out = vec![];
                            for field in fields {
                                if let Some(ty) = tracer.crossref(&field).await.and_then(|x| {
                                    Some(x.meta.as_ref()?.type_pretty.as_ref()?.to_string())
                                }) {
                                    out.push(ty);
                                }
                            }
                            out
                        }
                        None => vec![],
                    };
                    for field_ty in field_types {
                        if Box::pin(self.tainted_type_at(tracer, &field_ty, depth + 1)).await {
                            tainted = true;
                            break;
                        }
                    }
                    if tainted {
                        break;
                    }
                }
            }
        }
        self.types.insert(qualified, tainted);
        tainted
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
