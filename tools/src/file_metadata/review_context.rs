//! `review-context.toml`'s rules (see the tree's
//! docs/ai-agent-tools/code-review-context.md, and bugbug's
//! docs/code-review-skills.md), evaluated for a file as if a patch changed
//! only it, for the review guidance (skills, docs) that applies to it.
//!
//! A rule's `when` predicate composes file predicates (`any_file`,
//! `all_files`: `include`/`exclude` globs with Python `fnmatchcase`
//! semantics, `ext` suffixes, or a `ref` to `definitions.files.*`) with
//! `all`/`any`/`not`, and metadata predicates, of which only
//! `bugzilla.component` matches at review time (against the patch's bug's
//! component; here, the file's).  A rule the file satisfies only part of
//! (ex: a .webidl and a .cpp change) is a partial match.

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use toml::de::{DeTable, DeValue};

use super::{LineIndex, fnmatch_regex};

/// What a rule loads: a file (of the tree, or another GitHub repository's),
/// or another revision's diff.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Load {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub name: String,
    pub description: Option<String>,
    pub line: usize,
    when: Value,
    pub loads: Vec<Load>,
}

/// How a rule applies to a file.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Applies {
    /// To a patch changing only the file.
    Fully,
    /// To a patch changing the file and others (ex: a rule for .webidl and
    /// .cpp changes, for a .cpp file).
    Partially,
}

pub struct ReviewRules {
    pub rules: Vec<Rule>,
    definitions: Value,
    globs: std::cell::RefCell<std::collections::HashMap<String, Regex>>,
}

/// A TOML value as JSON.
fn to_json(value: &DeValue) -> Value {
    match value {
        DeValue::String(s) => Value::String(s.to_string()),
        DeValue::Integer(i) => i
            .as_str()
            .parse::<i64>()
            .map_or(Value::Null, |i| Value::Number(i.into())),
        DeValue::Float(f) => f
            .as_str()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Null, Value::Number),
        DeValue::Boolean(b) => Value::Bool(*b),
        DeValue::Datetime(d) => Value::String(d.to_string()),
        DeValue::Array(a) => Value::Array(a.iter().map(|e| to_json(e.get_ref())).collect()),
        DeValue::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.get_ref().to_string(), to_json(v.get_ref())))
                .collect::<Map<String, Value>>(),
        ),
    }
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Read a `review-context.toml`.
pub fn read_rules(source: &str) -> Result<ReviewRules, String> {
    let table = DeTable::parse(source).map_err(|e| e.to_string())?;
    let lines = LineIndex::new(source);
    // (The `[[rules]]` headers' lines, for the rules in order.)
    lazy_static! {
        static ref HEADER: Regex = Regex::new(r"^\s*\[\[\s*rules\s*\]\]").unwrap();
    }
    let headers: Vec<usize> = source
        .split('\n')
        .enumerate()
        .filter(|(_, line)| HEADER.is_match(line))
        .map(|(i, _)| i + 1)
        .collect();
    let mut rules = vec![];
    let mut definitions = Value::Null;
    for (key, value) in table.get_ref() {
        match key.get_ref().as_ref() {
            "definitions" => definitions = to_json(value.get_ref()),
            "rules" => {
                let Some(array) = value.get_ref().as_array() else {
                    continue;
                };
                for (i, rule) in array.iter().enumerate() {
                    let json = to_json(rule.get_ref());
                    let line = if headers.len() == array.len() {
                        headers[i]
                    } else {
                        lines.line(rule.span().start)
                    };
                    rules.push(Rule {
                        name: opt_str(&json, "name").unwrap_or_default(),
                        description: opt_str(&json, "description"),
                        line,
                        when: json.get("when").cloned().unwrap_or(Value::Null),
                        loads: json
                            .get("load")
                            .and_then(Value::as_array)
                            .map(|loads| {
                                loads
                                    .iter()
                                    .map(|l| Load {
                                        kind: opt_str(l, "type").unwrap_or_else(|| "file".to_string()),
                                        path: opt_str(l, "path"),
                                        repo: opt_str(l, "repo"),
                                        branch: opt_str(l, "branch"),
                                        revision: opt_str(l, "revision"),
                                        hash: opt_str(l, "hash"),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                }
            }
            _ => {}
        }
    }
    rules.sort_by_key(|r| r.line);
    Ok(ReviewRules {
        rules,
        definitions,
        globs: Default::default(),
    })
}

/// Three-valued: a predicate holds for a patch changing only the file, holds
/// if the patch changes other files too, or doesn't.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Truth {
    True,
    Partial,
    False,
}

impl ReviewRules {
    fn glob(&self, pattern: &str, path: &str) -> bool {
        let mut globs = self.globs.borrow_mut();
        globs
            .entry(pattern.to_string())
            .or_insert_with(|| fnmatch_regex(pattern))
            .is_match(path)
    }

    /// A `ref` predicate's definition, in its namespace.
    fn resolve<'a>(&'a self, pred: &'a Value, namespace: &str) -> &'a Value {
        match pred.get("ref").and_then(Value::as_str) {
            Some(r) => match r.split_once('.') {
                Some((ns, name)) if ns == namespace => {
                    self.definitions.get(ns).and_then(|d| d.get(name)).unwrap_or(&Value::Null)
                }
                _ => &Value::Null,
            },
            None => pred,
        }
    }

    fn file_matches(&self, pred: &Value, path: &str) -> bool {
        let pred = self.resolve(pred, "files");
        if !pred.is_object() {
            return false;
        }
        let strings = |key: &str| -> Vec<&str> {
            pred.get(key)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default()
        };
        if pred.get("include").is_some() && !strings("include").iter().any(|g| self.glob(g, path)) {
            return false;
        }
        if strings("exclude").iter().any(|g| self.glob(g, path)) {
            return false;
        }
        if pred.get("ext").is_some() && !strings("ext").iter().any(|e| path.ends_with(e)) {
            return false;
        }
        true
    }

    fn holds(&self, pred: &Value, path: &str, component: Option<&str>) -> Truth {
        if let Some(p) = pred.get("any_file") {
            return if self.file_matches(p, path) {
                Truth::True
            } else {
                Truth::Partial
            };
        }
        if let Some(p) = pred.get("all_files") {
            return if self.file_matches(p, path) {
                Truth::True
            } else {
                Truth::False
            };
        }
        let children = |key: &str| -> Vec<Truth> {
            pred.get(key)
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|p| self.holds(p, path, component)).collect())
                .unwrap_or_default()
        };
        if pred.get("all").is_some() {
            let results = children("all");
            return if results.contains(&Truth::False) {
                Truth::False
            } else if results.iter().all(|r| *r == Truth::True) {
                Truth::True
            } else {
                Truth::Partial
            };
        }
        if pred.get("any").is_some() {
            let results = children("any");
            return if results.contains(&Truth::True) {
                Truth::True
            } else if results.contains(&Truth::Partial) {
                Truth::Partial
            } else {
                Truth::False
            };
        }
        if let Some(p) = pred.get("not") {
            return match self.holds(p, path, component) {
                Truth::True => Truth::False,
                Truth::False => Truth::True,
                Truth::Partial => Truth::Partial,
            };
        }
        if let Some(b) = pred.get("bugzilla") {
            let b = self.resolve(b, "bugzilla");
            // (Only component predicates match at review time.)
            return match (b.get("component").and_then(Value::as_array), component) {
                (Some(components), Some(component)) => {
                    if components.iter().any(|c| c.as_str() == Some(component)) {
                        Truth::True
                    } else {
                        Truth::False
                    }
                }
                _ => Truth::False,
            };
        }
        Truth::False
    }

    /// The rule's file predicates that count toward it holding which the
    /// file satisfies.
    fn touching<'a>(&'a self, pred: &'a Value, path: &str, out: &mut Vec<&'a Value>) {
        if let Some(p) = pred.get("any_file").or_else(|| pred.get("all_files")) {
            if self.file_matches(p, path) {
                out.push(self.resolve(p, "files"));
            }
            return;
        }
        for key in ["all", "any"] {
            for p in pred.get(key).and_then(Value::as_array).into_iter().flatten() {
                self.touching(p, path, out);
            }
        }
    }

    /// Whether a file predicate takes in every C or C++ file of its
    /// extensions anywhere in the tree (it has no `include`, or only ones
    /// constraining files' names, like `**/*.cpp`), which isn't guidance for
    /// a particular C or C++ file (ex: the WebIDL rule's .cpp/.h half, though
    /// its .webidl half is fine for .webidl files).
    fn is_catch_all_for(pred: &Value, path: &str) -> bool {
        const C_FAMILY: &[&str] = &[".c", ".cc", ".cpp", ".cxx", ".h", ".hh", ".hpp", ".hxx", ".mm"];
        if !C_FAMILY.iter().any(|ext| path.ends_with(ext)) {
            return false;
        }
        pred.get("include")
            .and_then(Value::as_array)
            .is_none_or(|includes| {
                includes.iter().filter_map(Value::as_str).all(|glob| {
                    let mut rest = glob;
                    while let Some(r) = rest.strip_prefix("**/").or_else(|| rest.strip_prefix("*/")) {
                        rest = r;
                    }
                    !rest.contains('/')
                })
            })
    }

    /// The rules applying to a file (with its Bugzilla component as
    /// "Product::Component", if any), by index.
    pub fn applying(&self, path: &str, component: Option<&str>) -> Vec<(usize, Applies)> {
        self.rules
            .iter()
            .enumerate()
            .filter_map(|(i, rule)| {
                let truth = self.holds(&rule.when, path, component);
                if truth == Truth::False {
                    return None;
                }
                let mut touching = vec![];
                self.touching(&rule.when, path, &mut touching);
                // (Only by catch-all predicates is no match; nor a partial
                // one by no file predicate.)
                if !touching.is_empty() && touching.iter().all(|p| Self::is_catch_all_for(p, path)) {
                    return None;
                }
                match truth {
                    Truth::True => Some((i, Applies::Fully)),
                    Truth::Partial if !touching.is_empty() => Some((i, Applies::Partially)),
                    _ => None,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = r#"version = 1

[definitions.files.media_impl]
include = ["dom/media/**"]
exclude = ["dom/media/test/**"]
ext = [".cpp", ".h"]

[[rules]]
name = "Frontend"
description = "Front-end conventions."
when = { any_file = { include = ["browser/**"], exclude = ["**/test/**"], ext = [".js"] } }
load = [
  { type = "file", path = ".claude/skills/frontend/SKILL.md" },
]

[[rules]]
name = "WebIDL"
when = { all = [
  { any_file = { ext = [".webidl"] } },
  { any_file = { ext = [".h", ".cpp"] } },
] }
load = [{ type = "file", path = "docs/webidl.md", repo = "mozilla/other" }]

[[rules]]
name = "Media"
when = { any = [{ any_file = { ref = "files.media_impl" } }, { bugzilla = { component = ["Core::Audio/Video"] } }] }
load = [{ type = "fetch_revision", revision = "D12345" }]

[[rules]]
name = "Docs only"
when = { all_files = { ext = [".md"] } }
load = []

[[rules]]
name = "All C++"
when = { any_file = { include = ["**/*.cpp"], exclude = ["third_party/**"] } }
load = []

[[rules]]
name = "JS engine"
when = { any_file = { include = ["js/**"], ext = [".cpp", ".h"] } }
load = []
"#;

    #[test]
    fn test_rules() {
        let rules = read_rules(RULES).unwrap();
        let names: Vec<(&str, usize)> = rules.rules.iter().map(|r| (r.name.as_str(), r.line)).collect();
        assert_eq!(
            names,
            vec![
                ("Frontend", 8),
                ("WebIDL", 16),
                ("Media", 24),
                ("Docs only", 29),
                ("All C++", 34),
                ("JS engine", 39),
            ]
        );
        assert_eq!(rules.rules[1].loads[0].repo.as_deref(), Some("mozilla/other"));
        assert_eq!(rules.rules[2].loads[0].revision.as_deref(), Some("D12345"));

        let applying = |path: &str, component: Option<&str>| -> Vec<(&str, Applies)> {
            rules
                .applying(path, component)
                .into_iter()
                .map(|(i, a)| (rules.rules[i].name.as_str(), a))
                .collect()
        };
        assert_eq!(applying("browser/base/x.js", None), vec![("Frontend", Applies::Fully)]);
        assert_eq!(applying("browser/base/test/x.js", None), vec![]);
        // (Rules taking in every C++ file, like the WebIDL rule's C++ half, or
        // "All C++", don't apply to C++ files, but their other halves do.)
        assert_eq!(applying("dom/base/Element.cpp", None), vec![]);
        assert_eq!(
            applying("dom/webidl/Element.webidl", None),
            vec![("WebIDL", Applies::Partially)]
        );
        assert_eq!(applying("dom/media/AudioNode.cpp", None), vec![("Media", Applies::Fully)]);
        assert_eq!(applying("dom/media/test/x.cpp", None), vec![]);
        assert_eq!(applying("js/src/jsapi.cpp", None), vec![("JS engine", Applies::Fully)]);
        assert_eq!(
            applying("dom/media/x.js", Some("Core::Audio/Video")),
            vec![("Media", Applies::Fully)]
        );
        assert_eq!(applying("README.md", None), vec![("Docs only", Applies::Fully)]);
    }
}
