//! Roll-ups of a tree's edges between functions into edges between classes
//! and between subsystems (see crossref's `rollup_calls`): how many calls go
//! from one class's methods to another's, and from one subsystem's code to
//! another's, with the biggest contributors as examples to drill down into.
//! They're what block diagrams are drawn from (see `cmd_block_diagram`), and
//! they summarize how areas of the code interact without walking every
//! method.
//!
//! Fields (and functions) get data roll-ups too, of the edges between them
//! and other objects' fields (see `dataflow::summary::FieldEdge`), for
//! diagrams of how data moves between data structures' fields.
//!
//! Subsystems (from the files' per-file info, ex: "Core/DOM/Workers") get
//! pseudo-symbols of their own, `SUBSYS_<mangled subsystem>`, whose crossref
//! entries have their roll-ups, and `SUBSYS_` (no subsystem) is the whole
//! tree's: its members are the subsystems, and its `edges` the biggest edges
//! between them.

use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

use super::analysis_manglings::mangle_file;

/// The prefix of subsystems' pseudo-symbols.
pub const SUBSYSTEM_PREFIX: &str = "SUBSYS_";

/// A subsystem's pseudo-symbol (ex: `SUBSYS_Core/DOM/Workers`).
pub fn subsystem_sym(subsystem: &str) -> Ustr {
    ustr(&format!("{}{}", SUBSYSTEM_PREFIX, mangle_file(subsystem)))
}

/// The whole tree's pseudo-symbol, whose members are its subsystems.
pub fn root_subsystem_sym() -> Ustr {
    ustr(SUBSYSTEM_PREFIX)
}

/// Whether a function (by its pretty name) is plumbing whose calls don't say
/// how areas interact: refcounting, assertions, logging.  (Roll-ups leave
/// out calls to them, as dataflow summaries do; and callees labeled
/// `calls-diagram:stop`.)
pub fn is_boring_callee(pretty: &str) -> bool {
    let name = pretty.rsplit("::").next().unwrap_or(pretty);
    matches!(
        name,
        "AddRef" | "Release" | "log_print" | "log_test" | "__builtin_expect" | "printf_stderr"
    ) || name.starts_with("Assert")
        || name.starts_with("MOZ_")
        || pretty.starts_with("mozilla::detail::log")
}

/// An edge to (or from) another class or subsystem.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupEdge {
    /// The other end: a class's symbol, or a subsystem's pseudo-symbol.
    pub sym: Ustr,
    pub pretty: Ustr,
    /// How many calls (call sites), or, for data, flows (functions with
    /// them).
    pub count: u32,
    /// How many of the calls are of accessors (getters, setters, trivial
    /// methods, by their dataflow summaries), which say little about how
    /// the areas interact.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub accessors: u32,
    /// How many of the calls are IPDL messages (from generated `Send`
    /// methods to actors' `Recv` implementations).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ipc: u32,
    /// How many distinct pairs of functions (for subsystems, of classes or
    /// free functions) the calls are between.
    pub pairs: u32,
    /// The pairs with the most calls, for drilling down.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<RollupExample>,
    /// For a field's or function's data edges: some of the functions the
    /// data flows in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<Ustr>,
}

/// A pair of functions (for subsystems, of classes or free functions) an
/// edge's calls are between, and how many.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupExample {
    pub from: Ustr,
    #[serde(rename = "fromPretty")]
    pub from_pretty: Ustr,
    pub to: Ustr,
    #[serde(rename = "toPretty")]
    pub to_pretty: Ustr,
    pub count: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A member of a subsystem (a class, or a free function) and its calls in
/// and out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupMember {
    pub sym: Ustr,
    pub pretty: Ustr,
    pub count: u32,
}

/// An edge between two subsystems, for the whole tree's roll-up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollupPair {
    pub from: Ustr,
    pub to: Ustr,
    pub count: u32,
    pub pairs: u32,
}

/// A class's or subsystem's roll-up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rollup {
    /// Its edges to others, most calls first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out: Vec<RollupEdge>,
    /// Others' edges to it, most calls first.
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "in")]
    pub into: Vec<RollupEdge>,
    /// How many edges there were in each direction before only the biggest
    /// were kept.
    #[serde(default, rename = "outTotal")]
    pub out_total: u32,
    #[serde(default, rename = "inTotal")]
    pub in_total: u32,
    /// Calls within it (between its own methods, or its own code).
    #[serde(default)]
    pub internal: u32,
    /// For subsystems: their members with the most calls in, out and within.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<RollupMember>,
    /// For the whole tree: the biggest edges between subsystems.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<RollupPair>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_boring_callee() {
        assert!(is_boring_callee(
            "mozilla::dom::WorkerPrivate::AssertIsOnWorkerThread"
        ));
        assert!(is_boring_callee("nsISupports::AddRef"));
        assert!(!is_boring_callee("mozilla::dom::WorkerPrivate::Dispatch"));
    }

    #[test]
    fn test_subsystem_sym() {
        assert_eq!(
            subsystem_sym("Core/DOM/Workers").as_str(),
            "SUBSYS_Core/DOM/Workers"
        );
        assert_eq!(
            subsystem_sym("Core/DOM/Core HTML").as_str(),
            "SUBSYS_Core/DOM/Core@20HTML"
        );
    }
}
