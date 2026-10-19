//! What the frontends understand of functions: the places (storage, lvalues)
//! values flow into and out of, and the events (assignments, calls, returns,
//! tests) that move them, in source order.  This is deliberately naive: no
//! control flow graph, no aliasing beyond what's spelled (`*p`, `&x`,
//! references), and no knowledge of what unknown callees do.

use std::collections::HashMap;
use std::fmt;

use serde::Serialize;
use ustr::Ustr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Cpp,
    Js,
    Kotlin,
}

/// What a place is rooted at.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Root {
    /// A local variable or parameter, by symbol: C++'s `V_` symbols, scip's
    /// locals, or, without one, a key the frontend makes (ex: Kotlin's locals,
    /// "L_<line>_<name>").
    Local(Ustr),
    /// The object the function is a method of: `*this`, `this`, `self`.
    This,
    /// A global or static variable, by symbol.
    Global(Ustr),
}

/// A step from a place to a part of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Step {
    /// A member, by symbol (or "name:<name>" without one).
    Field(Ustr),
    /// What a pointer (or smart pointer) points to: `*p`, `p->`.
    Deref,
    /// An element of an array or container.
    Index,
}

/// A place: storage which values flow into and out of.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Place {
    pub root: Root,
    pub path: Vec<Step>,
}

impl Place {
    pub fn local(sym: Ustr) -> Self {
        Place {
            root: Root::Local(sym),
            path: vec![],
        }
    }

    pub fn this() -> Self {
        Place {
            root: Root::This,
            path: vec![],
        }
    }

    pub fn global(sym: Ustr) -> Self {
        Place {
            root: Root::Global(sym),
            path: vec![],
        }
    }

    pub fn with(&self, step: Step) -> Self {
        let mut place = self.clone();
        place.path.push(step);
        place
    }

    /// Whether `self` is `other` or a part of it.
    pub fn within(&self, other: &Place) -> bool {
        self.root == other.root && self.path.starts_with(&other.path)
    }

    /// Whether either is (part of) the other: writing one changes reads of
    /// the other.
    pub fn overlaps(&self, other: &Place) -> bool {
        self.within(other) || other.within(self)
    }

    /// The last field it's in, if any (ex: `mName` for `this->mName.mData`).
    pub fn last_field(&self) -> Option<Ustr> {
        self.path.iter().rev().find_map(|step| match step {
            Step::Field(sym) => Some(*sym),
            _ => None,
        })
    }
}

/// A source of a value.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Atom {
    /// The value in a place.
    Read(Place),
    /// A pointer or reference to a place (`&x`, `this`).
    Addr(Place),
    /// The result of one of the function's calls, by index.
    Call(usize),
    /// A literal, or a constant (ex: `NS_OK`), or a function or lambda used
    /// as a value: values that don't come from anywhere else.
    Const(String),
}

/// A value an expression has: one of its sources, and whether it's derived
/// from it (`a + 1`, `!a`, `a == b`) rather than it (`a`, `(T)a`,
/// `std::move(a)`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Val {
    pub atom: Atom,
    pub derived: bool,
}

impl Val {
    pub fn direct(atom: Atom) -> Self {
        Val {
            atom,
            derived: false,
        }
    }
}

pub fn derive(vals: Vec<Val>) -> Vec<Val> {
    vals.into_iter()
        .map(|v| Val {
            atom: v.atom,
            derived: true,
        })
        .collect()
}

/// An argument of a call.
#[derive(Clone, Debug, Serialize)]
pub struct Arg {
    pub vals: Vec<Val>,
    /// The place the argument is, if it's one (for references and pointers
    /// the callee may write through: `x` passed to a `T&` or a `T*`), or
    /// what it's the address of (`&x`, with `addr`).
    pub place: Option<Place>,
    pub addr: bool,
    pub text: String,
    /// Its first and last lines (0 for an implicit `this`).
    pub line: u32,
    pub end_line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    Call,
    /// `new T(...)`, `T x(...)`, `T(...)`, Kotlin's constructor calls.
    Construct,
    /// A macro we couldn't expand.
    Macro,
}

#[derive(Clone, Debug, Serialize)]
pub struct Call {
    pub id: usize,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    /// The callee's symbols (several for C++ overloads per platform, ex:
    /// `std::unique_ptr::get`'s).
    pub callee: Vec<Ustr>,
    pub callee_pretty: String,
    pub kind: CallKind,
    /// The object it's a method call on: its values and the place, if it's
    /// one (`obj` for `obj.m()`, `*p` for `p->m()`, this for `m()`).
    pub receiver: Option<Arg>,
    pub args: Vec<Arg>,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub enum EventKind {
    /// Values flow into a place: `=`, initialization, `+=` (whose values
    /// include the place's own).
    Assign {
        place: Place,
        vals: Vec<Val>,
        op: String,
    },
    /// One of the function's calls, by index (its arguments flow into the
    /// callee, and its result into what uses it).
    Call(usize),
    Return {
        vals: Vec<Val>,
    },
    /// Values decide control flow: conditions, switch subjects, loop tests.
    Test {
        vals: Vec<Val>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub line: u32,
    pub col: u32,
    /// The last line of the syntax it's from (ex: a call's arguments' lines).
    pub end_line: u32,
    pub kind: EventKind,
    /// Whether it's in a lambda (or local function) in the function, whose
    /// returns aren't the function's.
    pub in_lambda: bool,
    /// The macro it's from, if it's in a macro's expansion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macro_name: Option<String>,
    pub text: String,
}

impl Event {
    /// Whether it spans a line.
    pub fn on_line(&self, line: u32) -> bool {
        self.line <= line && line <= self.end_line
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Param {
    pub name: String,
    pub place: Place,
    pub type_pretty: Option<String>,
}

impl Param {
    /// Whether the callee can write the caller's place through it: a
    /// non-const pointer or reference.
    pub fn is_out(&self) -> bool {
        is_out_type(self.type_pretty.as_deref())
    }

    /// Whether it's a reference (an alias of the caller's place) rather than
    /// a pointer.
    pub fn is_ref(&self) -> bool {
        self.type_pretty
            .as_deref()
            .is_some_and(|t| t.trim_end().ends_with('&'))
    }
}

/// Whether a C++ type is a pointer or reference something can be written
/// through: not to const (`const T*`, `const T&`), and not a string pointer.
pub fn is_out_type(ty: Option<&str>) -> bool {
    let Some(ty) = ty else {
        return false;
    };
    let ty = ty.trim();
    // (Forwarding references, `T&& aResult` in templates, are whatever
    // they're passed: maybe pointers or references to write through.)
    if let Some(base) = ty.strip_suffix("&&") {
        return !base.contains("const");
    }
    if !(ty.ends_with('*') || ty.ends_with('&')) {
        return false;
    }
    let pointee = ty.trim_end_matches(['*', '&', ' ']);
    !(pointee.starts_with("const ") || pointee.ends_with(" const") || pointee == "char")
}

/// A type normalized for comparing arguments' and parameters' types: no
/// `const`, references, `struct`/`class`, or spaces.
pub fn normalize_type(ty: &str) -> String {
    ty.replace("const ", "")
        .replace("struct ", "")
        .replace("class ", "")
        .replace(['&', ' '], "")
}

/// Whether a type depends on a template parameter (ex: `T &&`, `const T *`),
/// by the convention that those are one or two uppercase letters.
pub fn is_dependent_type(ty: &str) -> bool {
    ty.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| !w.is_empty() && w.len() <= 2 && w.chars().all(|c| c.is_ascii_uppercase()))
}

/// The type of a part of a value of a type, for the parts we know: what a
/// pointer points to.
pub fn type_of_path(ty: &str, path: &[Step]) -> Option<String> {
    let mut ty = ty.trim().trim_end_matches('&').trim().to_string();
    for step in path {
        match step {
            Step::Deref => {
                ty = ty.strip_suffix('*')?.trim().to_string();
            }
            _ => return None,
        }
    }
    Some(ty)
}

/// The names of IPDL struct fields' symbols (`IPDL_…`), which the tracer
/// makes the steps of places reached through their accessors (see
/// `Tracer::canonicalize_ipdl_steps`) and which can't be derived from them
/// (their structs' and namespaces' names have underscores too), for
/// `sym_short_name`: process-wide, as a symbol's name is always the same.
static IPDL_NAMES: std::sync::LazyLock<std::sync::RwLock<HashMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

/// Remember an IPDL struct field's name (see `IPDL_NAMES`).
pub fn set_ipdl_name(sym: &str, name: &str) {
    IPDL_NAMES
        .write()
        .unwrap()
        .insert(sym.to_string(), name.to_string());
}

/// A short name for a symbol without a display name: a member's name from
/// its symbol (C++'s `F_<T_Foo>_mBar`, scip's `…#bar.`), or an IPDL struct
/// field's (see `IPDL_NAMES`), else the symbol.
pub fn sym_short_name(sym: &str) -> String {
    if let Some(name) = sym.strip_prefix("name:") {
        return name.to_string();
    }
    if sym.starts_with("IPDL_")
        && let Some(name) = IPDL_NAMES.read().unwrap().get(sym)
    {
        return name.clone();
    }
    if sym.starts_with("F_")
        && let Some((_, name)) = sym.rsplit_once(">_")
    {
        return name.to_string();
    }
    if sym.starts_with("S_") {
        let trimmed = sym.trim_end_matches(['.', '#', '/']);
        let name = trimmed
            .rsplit(['#', '/', '.'])
            .next()
            .unwrap_or(trimmed)
            .trim_end_matches("()");
        if !name.is_empty() {
            return name.to_string();
        }
    }
    sym.to_string()
}

/// Where something in a function needs knowledge we don't have: an oracle
/// (a human, a better analysis, or an LLM) could answer it.
#[derive(Clone, Debug, Serialize)]
pub struct Question {
    pub line: u32,
    pub kind: QuestionKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuestionKind {
    /// A call whose callee we couldn't resolve to a definition.
    UnresolvedCall,
    /// A macro we couldn't expand.
    Macro,
    /// Syntax we didn't understand (or tree-sitter misparsed).
    Unparsed,
}

/// A call whose success dominates some of a function's events, by the
/// syntax (see `frontend`'s guards): `if (!Validate(x)) { return …; }` (or
/// whose consequence otherwise exits: `throw`, `MOZ_CRASH`), the rest of its
/// block; `if (Validate(x)) { … }`, its consequence; and
/// `MOZ_RELEASE_ASSERT(Validate(x))` (and `MOZ_DIAGNOSTIC_ASSERT`,
/// `NS_ENSURE_TRUE`), the rest of its block.  (For taint: a validation
/// blesses what it dominates.)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Guard {
    pub call: usize,
    /// The events it dominates, `start..end`.
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FuncIr {
    pub sym: Ustr,
    pub pretty: String,
    pub path: String,
    pub lang: Lang,
    pub line: u32,
    pub end_line: u32,
    pub params: Vec<Param>,
    /// Whether it has a body (not an interface's or abstract class's
    /// declaration).
    pub has_body: bool,
    pub events: Vec<Event>,
    pub calls: Vec<Call>,
    pub questions: Vec<Question>,
    /// The calls whose success dominates some of its events (see `Guard`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<Guard>,
    /// Display names of the symbols in its places (locals, fields, globals).
    pub names: HashMap<Ustr, String>,
    /// The types of its locals and parameters, where the records have them
    /// (C++'s and scip-typescript's).
    pub types: HashMap<Ustr, String>,
}

impl FuncIr {
    /// Visit each of its places (events', and calls' arguments' and
    /// receivers').
    pub fn for_each_place_mut(&mut self, visit: &mut impl FnMut(&mut Place)) {
        let vals = |vals: &mut Vec<Val>, visit: &mut dyn FnMut(&mut Place)| {
            for val in vals {
                if let Atom::Read(place) | Atom::Addr(place) = &mut val.atom {
                    visit(place);
                }
            }
        };
        for event in &mut self.events {
            match &mut event.kind {
                EventKind::Assign { place, vals: v, .. } => {
                    visit(place);
                    vals(v, visit);
                }
                EventKind::Return { vals: v } | EventKind::Test { vals: v } => vals(v, visit),
                EventKind::Call(_) => {}
            }
        }
        for call in &mut self.calls {
            for arg in call.args.iter_mut().chain(call.receiver.iter_mut()) {
                if let Some(place) = &mut arg.place {
                    visit(place);
                }
                vals(&mut arg.vals, visit);
            }
        }
    }

    /// Visit each of its places, as `for_each_place_mut` does.
    pub fn for_each_place(&self, visit: &mut impl FnMut(&Place)) {
        let vals = |vals: &Vec<Val>, visit: &mut dyn FnMut(&Place)| {
            for val in vals {
                if let Atom::Read(place) | Atom::Addr(place) = &val.atom {
                    visit(place);
                }
            }
        };
        for event in &self.events {
            match &event.kind {
                EventKind::Assign { place, vals: v, .. } => {
                    visit(place);
                    vals(v, visit);
                }
                EventKind::Return { vals: v } | EventKind::Test { vals: v } => vals(v, visit),
                EventKind::Call(_) => {}
            }
        }
        for call in &self.calls {
            for arg in call.args.iter().chain(call.receiver.iter()) {
                if let Some(place) = &arg.place {
                    visit(place);
                }
                vals(&arg.vals, visit);
            }
        }
    }

    pub fn place_name(&self, place: &Place) -> String {
        let name = |sym: &Ustr| -> String {
            match self.names.get(sym) {
                Some(name) => name.clone(),
                None => sym_short_name(sym),
            }
        };
        let mut out = match &place.root {
            Root::Local(sym) | Root::Global(sym) => name(sym),
            Root::This => "this".to_string(),
        };
        let mut i = 0;
        while i < place.path.len() {
            match (&place.path[i], place.path.get(i + 1)) {
                (Step::Deref, Some(Step::Field(sym))) => {
                    out = format!("{}->{}", out, name(sym));
                    i += 1;
                }
                (Step::Deref, _) => out = format!("*{}", out),
                (Step::Field(sym), _) if i == 0 && place.root == Root::This => {
                    let sep = if self.lang == Lang::Cpp { "->" } else { "." };
                    out = format!("{}{}{}", out, sep, name(sym));
                }
                (Step::Field(sym), _) => out = format!("{}.{}", out, name(sym)),
                (Step::Index, _) => out = format!("{}[]", out),
            }
            i += 1;
        }
        out
    }

    /// The parameter a place is in, if any: its index.
    pub fn param_index(&self, place: &Place) -> Option<usize> {
        self.params.iter().position(|p| p.place.root == place.root)
    }

    pub fn atom_text(&self, atom: &Atom) -> String {
        match atom {
            Atom::Read(place) => self.place_name(place),
            Atom::Addr(place) if place.root == Root::This && place.path.is_empty() => {
                "this".to_string()
            }
            Atom::Addr(place) => format!("&{}", self.place_name(place)),
            Atom::Call(i) => format!("{}(…)", self.calls[*i].callee_pretty),
            Atom::Const(text) => text.clone(),
        }
    }
}

impl fmt::Display for FuncIr {
    /// The debugging view: what the frontend understood of the function.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} ({}:{}-{})",
            self.pretty, self.path, self.line, self.end_line
        )?;
        for (i, param) in self.params.iter().enumerate() {
            writeln!(
                f,
                "  param {}: {} : {}",
                i,
                param.name,
                param.type_pretty.as_deref().unwrap_or("?")
            )?;
        }
        let vals = |vals: &[Val]| -> String {
            vals.iter()
                .map(|v| {
                    let text = self.atom_text(&v.atom);
                    if v.derived {
                        format!("~{}", text)
                    } else {
                        text
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        for event in &self.events {
            let what = match &event.kind {
                EventKind::Assign { place, vals: v, op } => {
                    format!("{} {} {{{}}}", self.place_name(place), op, vals(v))
                }
                EventKind::Call(i) => {
                    let call = &self.calls[*i];
                    let args = call
                        .args
                        .iter()
                        .map(|a| {
                            let mut text = format!("{{{}}}", vals(&a.vals));
                            if let Some(place) = &a.place {
                                text = format!(
                                    "{}@{}{}",
                                    text,
                                    if a.addr { "&" } else { "" },
                                    self.place_name(place)
                                );
                            }
                            text
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let receiver = match &call.receiver {
                        Some(r) => match &r.place {
                            Some(place) => format!("{}.", self.place_name(place)),
                            None => format!("{{{}}}.", vals(&r.vals)),
                        },
                        None => String::new(),
                    };
                    format!(
                        "call#{} {:?} {}{}({}) -> {}",
                        i,
                        call.kind,
                        receiver,
                        call.callee_pretty,
                        args,
                        call.callee.first().map(|s| s.as_str()).unwrap_or("?")
                    )
                }
                EventKind::Return { vals: v } => format!("return {{{}}}", vals(v)),
                EventKind::Test { vals: v } => format!("test {{{}}}", vals(v)),
            };
            writeln!(
                f,
                "  {:>5}:{:<3} {}{}{}",
                event.line,
                event.col,
                if event.in_lambda { "λ " } else { "" },
                match &event.macro_name {
                    Some(name) => format!("[{}] ", name),
                    None => String::new(),
                },
                what
            )?;
        }
        for q in &self.questions {
            writeln!(f, "  ? {:>5} {:?}: {}", q.line, q.kind, q.text)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ustr::ustr;

    fn func(lang: Lang) -> FuncIr {
        let mut names = HashMap::new();
        names.insert(ustr("V_p"), "aRetVal".to_string());
        names.insert(ustr("F_name"), "mName".to_string());
        FuncIr {
            sym: ustr("f"),
            pretty: "f".to_string(),
            path: "a.cpp".to_string(),
            lang,
            line: 1,
            end_line: 1,
            params: vec![],
            has_body: true,
            events: vec![],
            calls: vec![],
            questions: vec![],
            guards: vec![],
            names,
            types: HashMap::new(),
        }
    }

    #[test]
    fn test_place_names() {
        let f = func(Lang::Cpp);
        let p = Place::local(ustr("V_p"));
        assert_eq!(f.place_name(&p), "aRetVal");
        assert_eq!(f.place_name(&p.with(Step::Deref)), "*aRetVal");
        assert_eq!(
            f.place_name(&p.with(Step::Deref).with(Step::Field(ustr("F_name")))),
            "aRetVal->mName"
        );
        assert_eq!(
            f.place_name(&Place::this().with(Step::Field(ustr("F_name")))),
            "this->mName"
        );
        assert_eq!(
            func(Lang::Js).place_name(&Place::this().with(Step::Field(ustr("F_name")))),
            "this.mName"
        );
        assert_eq!(f.place_name(&p.with(Step::Index)), "aRetVal[]");
        // (Members without display names, by their symbols.)
        assert_eq!(
            f.place_name(&Place::this().with(Step::Field(ustr("F_<T_Foo>_mBar")))),
            "this->mBar"
        );
    }

    #[test]
    fn test_places_overlap() {
        let p = Place::local(ustr("V_p"));
        let pointee = p.with(Step::Deref);
        assert!(pointee.within(&p));
        assert!(!p.within(&pointee));
        assert!(p.overlaps(&pointee));
        assert!(!pointee.overlaps(&Place::local(ustr("V_q"))));
    }

    #[test]
    fn test_types() {
        assert!(is_out_type(Some("_Bool *")));
        assert!(is_out_type(Some("nsACString &")));
        assert!(is_out_type(Some("T &&")));
        assert!(!is_out_type(Some("const char *")));
        assert!(!is_out_type(Some("const nsAString &")));
        assert!(!is_out_type(Some("uint8_t")));
        assert!(is_dependent_type("T &&"));
        assert!(is_dependent_type("const T *"));
        assert!(!is_dependent_type("_Bool *"));
        assert_eq!(normalize_type("const nsAString &"), "nsAString");
        assert_eq!(
            type_of_path("_Bool *", &[Step::Deref]).as_deref(),
            Some("_Bool")
        );
        assert_eq!(type_of_path("_Bool", &[Step::Deref]), None);
    }

    #[test]
    fn test_sym_short_names() {
        assert_eq!(sym_short_name("F_<T_mozilla::Foo>_mBar"), "mBar");
        assert_eq!(
            sym_short_name("S_js_typescript_generated/lib.gecko.services.d.ts/JSServices#prefs."),
            "prefs"
        );
        assert_eq!(
            sym_short_name("S_jvm_org/mozilla/fenix/utils/Settings#dohProviderUrl."),
            "dohProviderUrl"
        );
        assert_eq!(sym_short_name("name:foo"), "foo");
    }
}
