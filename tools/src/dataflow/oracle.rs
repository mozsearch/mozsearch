//! Oracles: what the dataflow analysis can't work out itself (callees it
//! can't see into or follow, calls it can't resolve), asked of someone who
//! can (a person, a better analysis, an LLM) in batches, with the answers
//! kept in a tree of their own for review and reuse.
//!
//! The analysis decides where an oracle is needed (`questions`), so the
//! questions are specific and checkable: "where do this function's
//! arguments' values go?", with its signature, definition, and calls.  The
//! answers (`Summary`) are files in a tree parallel to the source tree's (a
//! JSON file for each file of definitions, keyed by symbol, like the history
//! config trees' `.gitattributes`), each with the hash of the source it's
//! about (so stale answers are ignored and re-asked), its rationale, who
//! answered, and who reviewed it.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

/// Where values go in a function.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryFlow {
    /// "this", "argN" (the value of argument N), or "*argN" (what it points
    /// to or refers to).
    pub from: String,
    /// "return", "this", or "*argN" (written through argument N).
    pub to: String,
    /// Whether it's derived from it (computed) rather than it (copied).
    #[serde(default)]
    pub derived: bool,
}

/// An oracle's answer about a function.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    #[serde(default)]
    pub pretty: String,
    /// The hash of the source the answer is about (see `source_hash`): its
    /// definition's text, if we have it.
    #[serde(default)]
    pub source_hash: Option<String>,
    pub flows: Vec<SummaryFlow>,
    /// Functions it calls with its arguments that the analysis can't see
    /// (ex: a visitor's `operator()` for `Variant::match`), free-form.
    #[serde(default)]
    pub calls: Vec<String>,
    pub rationale: String,
    /// Who answered (ex: "claude-opus-5-5 2026-10-16", "asuth").
    pub by: String,
    #[serde(default)]
    pub reviewed_by: Option<String>,
}

/// An oracle's answer about a call we couldn't resolve.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiteAnswer {
    /// What it can call, by qualified pretty names (ex: "nsIURI::schemeIs").
    pub targets: Vec<String>,
    pub rationale: String,
    pub by: String,
    #[serde(default)]
    pub reviewed_by: Option<String>,
}

#[derive(Default)]
pub struct Oracle {
    pub summaries: HashMap<Ustr, Summary>,
    /// By the calling file's path and line.
    pub sites: HashMap<(String, u32), SiteAnswer>,
}

/// The hash of a definition's text, for noticing answers about older
/// versions.
pub fn source_hash(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.trim().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

impl Oracle {
    /// Load the answers in an oracle tree: `summaries/`, JSON files of maps
    /// from symbols to `Summary`s (named for the files defining them), and
    /// `sites/`, JSON files of maps from "line:<N>" to `SiteAnswer`s (named
    /// for the files with the calls).
    pub fn load(dir: &Path) -> Result<Oracle, String> {
        let mut oracle = Oracle::default();
        for (rel, path) in json_files(&dir.join("summaries"))? {
            let _ = rel;
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{:?}: {}", path, e))?;
            let map: BTreeMap<String, Summary> =
                serde_json::from_str(&text).map_err(|e| format!("{:?}: {}", path, e))?;
            for (sym, summary) in map {
                oracle.summaries.insert(ustr(&sym), summary);
            }
        }
        for (rel, path) in json_files(&dir.join("sites"))? {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{:?}: {}", path, e))?;
            let map: BTreeMap<String, SiteAnswer> =
                serde_json::from_str(&text).map_err(|e| format!("{:?}: {}", path, e))?;
            let source = rel.strip_suffix(".json").unwrap_or(&rel).to_string();
            for (key, answer) in map {
                if let Some(line) = key.strip_prefix("line:").and_then(|l| l.parse().ok()) {
                    oracle.sites.insert((source.clone(), line), answer);
                }
            }
        }
        Ok(oracle)
    }

    pub fn site(&self, path: &str, line: u32) -> Option<&SiteAnswer> {
        self.sites.get(&(path.to_string(), line))
    }

    pub fn summary(&self, syms: &[Ustr]) -> Option<&Summary> {
        syms.iter().find_map(|s| self.summaries.get(s))
    }
}

/// The JSON files under a directory, with their paths relative to it (none
/// if it doesn't exist).
fn json_files(dir: &Path) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let mut out = vec![];
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = std::fs::read_dir(&path).map_err(|e| format!("{:?}: {}", path, e))?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "json") {
                let rel = p
                    .strip_prefix(dir)
                    .map(|r| r.to_string_lossy().to_string())
                    .unwrap_or_default();
                out.push((rel, p));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// What a summary's "this", "argN", "*argN", "argN..." names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    This,
    Arg(usize),
    ArgPointee(usize),
    /// Arguments N and after (ex: a printf-style function's).
    ArgsFrom(usize),
    Return,
}

impl Slot {
    /// Whether it's (the value of, or what's pointed to by) argument `i`.
    pub fn is_arg(&self, i: usize) -> bool {
        match self {
            Slot::Arg(n) | Slot::ArgPointee(n) => *n == i,
            Slot::ArgsFrom(n) => i >= *n,
            _ => false,
        }
    }
}

pub fn parse_slot(spec: &str) -> Option<Slot> {
    match spec {
        "this" => Some(Slot::This),
        "return" => Some(Slot::Return),
        _ => {
            if let Some(n) = spec.strip_prefix("*arg") {
                return n.parse().ok().map(Slot::ArgPointee);
            }
            let n = spec.strip_prefix("arg")?;
            if let Some(n) = n.strip_suffix("...") {
                return n.parse().ok().map(Slot::ArgsFrom);
            }
            n.parse().ok().map(Slot::Arg)
        }
    }
}

/// A question for an oracle.
#[derive(Clone, Debug, Serialize)]
pub struct OracleQuestion {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sym: Option<Ustr>,
    pub pretty: String,
    /// Its parameters, if we know them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// How many calls it's in the way of.
    pub count: usize,
    /// Some of those calls: "path:line: text".
    pub sites: Vec<String>,
    /// Its definition, if the index has it: "path:line" and its text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    pub ask: String,
}

pub const SUMMARY_ASK: &str = "Where do this function's inputs' values go? For each of `this` and \
its arguments (by index, `argN` for the value, `*argN` for what a pointer or reference argument \
points to, `argN...` for arguments N and after), say whether its value flows to the return value (`return`), into what a pointer or \
reference argument points to (`*argN`), or into `this`, and whether it's copied or derived \
(computed from it). Answer as a JSON `Summary`: {\"flows\": [{\"from\", \"to\", \"derived\"}], \
\"calls\": [...functions it calls with its arguments that its callers can't see...], \
\"rationale\": \"...\"}.";

pub const SITE_ASK: &str = "What can this call reach? It calls a function pointer, a callable \
stored somewhere, or a method on a value whose type the indexer didn't know. Name the functions \
(by their qualified names) it can call, and why.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slots() {
        assert_eq!(parse_slot("this"), Some(Slot::This));
        assert_eq!(parse_slot("return"), Some(Slot::Return));
        assert_eq!(parse_slot("arg2"), Some(Slot::Arg(2)));
        assert_eq!(parse_slot("*arg0"), Some(Slot::ArgPointee(0)));
        assert_eq!(parse_slot("argx"), None);
        assert_eq!(parse_slot("arg1..."), Some(Slot::ArgsFrom(1)));
        assert!(Slot::ArgsFrom(1).is_arg(5));
        assert!(!Slot::ArgsFrom(1).is_arg(0));
    }
}
