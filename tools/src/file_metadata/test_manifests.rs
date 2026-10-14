//! Test manifests' (manifestparser's TOML ones) sections and their
//! conditions, with their lines, and which apply to a test the way
//! manifestparser combines them: the manifest's `[DEFAULT]` and, for a
//! manifest another includes (`["include:child.toml"]`), that section of the
//! including manifest (combined with its `[DEFAULT]`).  Their combination
//! matched test-info-all-tests.json's for every test.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use toml::de::{DeTable, DeValue};

use super::{LineIndex, split_comment};

/// The keys whose conditions we keep.
pub const CONDITION_KEYS: &[&str] = &["skip-if", "run-if", "fail-if"];

/// A condition (an element of a condition key's array, or its string), as
/// manifestparser has it (stripped, without an inline ` # ...`), with its line
/// and the comment after it on its line (ex: "Bug 1234").
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Condition {
    pub text: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Section {
    pub name: String,
    pub line: usize,
    /// Its `path` key, if any (else its name is its file).
    pub path: Option<String>,
    pub conditions: BTreeMap<String, Vec<Condition>>,
}

#[derive(Debug, Default)]
pub struct Manifest {
    pub default: Option<Section>,
    /// Its sections but `[DEFAULT]`, in order.
    pub sections: Vec<Section>,
}

/// Read a TOML test manifest.
pub fn read_manifest(source: &str) -> Result<Manifest, String> {
    let table = DeTable::parse(source).map_err(|e| e.to_string())?;
    let lines = LineIndex::new(source);
    let source_lines: Vec<&str> = source.split('\n').collect();
    let mut manifest = Manifest::default();
    let mut sections: Vec<Section> = vec![];
    for (key, value) in table.get_ref() {
        let Some(entries) = value.get_ref().as_table() else {
            continue;
        };
        let mut section = Section {
            name: key.get_ref().to_string(),
            line: lines.line(key.span().start),
            ..Default::default()
        };
        for (k, v) in entries {
            let k = k.get_ref().as_ref();
            if k == "path" {
                section.path = v.get_ref().as_str().map(str::to_string);
                continue;
            }
            if !CONDITION_KEYS.contains(&k) {
                continue;
            }
            let values: Vec<(&DeValue, std::ops::Range<usize>)> = match v.get_ref() {
                DeValue::Array(array) => array.iter().map(|e| (e.get_ref(), e.span())).collect(),
                other => vec![(other, v.span())],
            };
            let conditions = values
                .into_iter()
                .map(|(value, span)| {
                    let line = lines.line(span.start);
                    // (The comment after it on its line.)
                    let line_text = source_lines.get(line - 1).copied().unwrap_or("");
                    let line_start = source[..span.start].rfind('\n').map_or(0, |i| i + 1);
                    let after = line_text.get(span.end.saturating_sub(line_start)..).unwrap_or("");
                    let comment = split_comment(after)
                        .1
                        .filter(|c| !c.is_empty())
                        .map(str::to_string);
                    Condition {
                        text: manifest_value(value),
                        line,
                        comment,
                    }
                })
                .collect();
            section.conditions.insert(k.to_string(), conditions);
        }
        if section.name.eq_ignore_ascii_case("DEFAULT") {
            manifest.default = Some(section);
        } else {
            sections.push(section);
        }
    }
    sections.sort_by_key(|s| s.line);
    manifest.sections = sections;
    Ok(manifest)
}

/// A value as manifestparser has it (see its toml.py's `read_toml`):
/// stripped, without an inline ` # ...`.
fn manifest_value(value: &DeValue) -> String {
    let text = match value {
        DeValue::String(s) => s.to_string(),
        DeValue::Boolean(b) => if *b { "True" } else { "False" }.to_string(),
        DeValue::Integer(i) => i.as_str().to_string(),
        DeValue::Float(f) => f.as_str().to_string(),
        _ => String::new(),
    };
    let text = text.trim();
    // (manifestparser's inline_comment_rx, `\s#.*$`, cut from its whitespace.)
    let cut = text
        .char_indices()
        .find(|&(i, c)| c == '#' && i > 0 && text[..i].ends_with(char::is_whitespace))
        .and_then(|(i, _)| text[..i].char_indices().last().map(|(j, _)| j))
        .unwrap_or(text.len());
    text[..cut].to_string()
}

/// Where a condition applying to a test comes from.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum From {
    /// The including manifest's `[DEFAULT]`.
    AncestorDefault,
    /// The including manifest's `include:` section.
    Include,
    /// The test's manifest's `[DEFAULT]`.
    Default,
    /// The test's section.
    Test,
}

/// A condition applying to a test, with its manifest.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppliedCondition {
    pub from: From,
    pub manifest: String,
    #[serde(flatten)]
    pub condition: Condition,
}

/// A test's entry in a manifest.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TestEntry {
    pub manifest: String,
    /// The manifest including the test's manifest, if it's included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ancestor: Option<String>,
    /// Its section's line.
    pub line: usize,
    /// The conditions applying to it, by key, in manifestparser's order.
    /// For skip-if, all apply (any skips); for run-if, the most local
    /// layer's conditions each combine with each of the others' (`&&`);
    /// for fail-if, only the most local layer's apply.
    pub conditions: BTreeMap<String, Vec<AppliedCondition>>,
}

impl TestEntry {
    /// The key's value as manifestparser has it, ex: test-info-all-tests.json's.
    pub fn manifestparser_value(&self, key: &str) -> String {
        let conditions = self.conditions.get(key).map_or(&[][..], |c| &c[..]);
        if key != "run-if" {
            return conditions
                .iter()
                .map(|c| c.condition.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
        }
        // (Each layer's conditions, combined with the next's by
        // combine_run_if's `(g) && (l)` cross product.)
        let mut layers: Vec<Vec<String>> = vec![];
        let mut last: Option<&From> = None;
        for c in conditions {
            if last != Some(&c.from) {
                layers.push(vec![]);
                last = Some(&c.from);
            }
            layers.last_mut().unwrap().extend(
                c.condition
                    .text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string),
            );
        }
        let mut combined: Vec<String> = vec![];
        for layer in layers {
            combined = if combined.is_empty() {
                layer
            } else if layer.is_empty() {
                combined
            } else {
                combined
                    .iter()
                    .flat_map(|g| layer.iter().map(move |l| format!("({}) && ({})", g, l)))
                    .collect()
            };
        }
        combined.join("\n")
    }
}

/// Combine a layer of conditions into those so far, like manifestparser's
/// `combine_fields`: skip-if and run-if accumulate (see `TestEntry`),
/// others replace.
fn combine(
    so_far: &mut BTreeMap<String, Vec<AppliedCondition>>,
    section: Option<&Section>,
    from: From,
    manifest: &str,
) {
    let Some(section) = section else {
        return;
    };
    for (key, conditions) in &section.conditions {
        let applied: Vec<AppliedCondition> = conditions
            .iter()
            .map(|c| AppliedCondition {
                from: from.clone(),
                manifest: manifest.to_string(),
                condition: c.clone(),
            })
            .collect();
        let entry = so_far.entry(key.clone()).or_default();
        if key == "skip-if" || key == "run-if" {
            entry.extend(applied);
        } else {
            *entry = applied;
        }
    }
}

/// A directory-relative path, normalized (`a/./b/../c` -> `a/c`).
pub fn join_normalized(dir: &str, path: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() {
        vec![]
    } else {
        dir.split('/').collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// A tree's test manifests (by path), for the entries of the tests they list.
#[derive(Default)]
pub struct Manifests {
    pub manifests: BTreeMap<String, Manifest>,
}

impl Manifests {
    /// The manifests a manifest includes (its `include:` sections), as
    /// (section, path).
    pub fn includes<'a>(path: &str, manifest: &'a Manifest) -> Vec<(&'a Section, String)> {
        manifest
            .sections
            .iter()
            .filter_map(|s| {
                s.name
                    .strip_prefix("include:")
                    .map(|child| (s, join_normalized(dir_of(path), child)))
            })
            .collect()
    }

    /// The tests' entries in the manifests, by test path, given the
    /// manifests' including manifests (by manifest path), and the manifests
    /// read directly (ex: the ones moz.build files declare; a manifest can be
    /// both, and has entries for each way).
    pub fn entries(
        &self,
        ancestors: &BTreeMap<String, Vec<String>>,
        top_level: &BTreeSet<String>,
    ) -> BTreeMap<String, Vec<TestEntry>> {
        let mut out: BTreeMap<String, Vec<TestEntry>> = BTreeMap::new();
        for (path, manifest) in &self.manifests {
            let mut parents: Vec<Option<&String>> = vec![];
            if top_level.contains(path) || !ancestors.contains_key(path) {
                parents.push(None);
            }
            parents.extend(ancestors.get(path).into_iter().flatten().map(Some));
            for parent in parents {
                let mut defaults: BTreeMap<String, Vec<AppliedCondition>> = BTreeMap::new();
                if let Some(parent) = parent
                    && let Some(pm) = self.manifests.get(parent)
                {
                    combine(&mut defaults, pm.default.as_ref(), From::AncestorDefault, parent);
                    let include = Self::includes(parent, pm)
                        .into_iter()
                        .find(|(_, child)| child == path)
                        .map(|(s, _)| s);
                    combine(&mut defaults, include, From::Include, parent);
                }
                combine(&mut defaults, manifest.default.as_ref(), From::Default, path);
                let dir = dir_of(path);
                for section in &manifest.sections {
                    if section.name.starts_with("include:") {
                        continue;
                    }
                    let file = section.path.as_deref().unwrap_or(&section.name);
                    if file.contains("://") {
                        continue;
                    }
                    let mut conditions = defaults.clone();
                    combine(&mut conditions, Some(section), From::Test, path);
                    out.entry(join_normalized(dir, file)).or_default().push(TestEntry {
                        manifest: path.clone(),
                        ancestor: parent.cloned(),
                        line: section.line,
                        conditions,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARENT: &str = r#"[DEFAULT]
skip-if = ["os == 'android'"]
run-if = ["!msix"]

["include:sub/child.toml"]
skip-if = [
  "verify", # Bug 1
]
"#;

    const CHILD: &str = r#"[DEFAULT]
support-files = ["helper.js"]
skip-if = ["http3"]
run-if = ["os == 'linux'", "os == 'mac'"]

["test_a.html"]
skip-if = [
  "debug", # Bug 123 - can't "run"
  "asan",
]
fail-if = "xorigin"

["test_b.html"]
path = "../other/test_b.html"
"#;

    fn manifests() -> Manifests {
        let mut m = Manifests::default();
        m.manifests.insert("dir/parent.toml".to_string(), read_manifest(PARENT).unwrap());
        m.manifests
            .insert("dir/sub/child.toml".to_string(), read_manifest(CHILD).unwrap());
        m
    }

    #[test]
    fn test_read_manifest() {
        let m = read_manifest(CHILD).unwrap();
        assert_eq!(m.default.as_ref().unwrap().line, 1);
        let names: Vec<(&str, usize)> = m.sections.iter().map(|s| (s.name.as_str(), s.line)).collect();
        assert_eq!(names, vec![("test_a.html", 6), ("test_b.html", 13)]);
        assert_eq!(
            m.sections[0].conditions["skip-if"],
            vec![
                Condition {
                    text: "debug".to_string(),
                    line: 8,
                    comment: Some("Bug 123 - can't \"run\"".to_string())
                },
                Condition {
                    text: "asan".to_string(),
                    line: 9,
                    comment: None
                },
            ]
        );
        assert_eq!(m.sections[0].conditions["fail-if"][0].line, 11);
        assert_eq!(m.sections[1].path.as_deref(), Some("../other/test_b.html"));
    }

    #[test]
    fn test_entries() {
        let m = manifests();
        let mut ancestors = BTreeMap::new();
        ancestors.insert("dir/sub/child.toml".to_string(), vec!["dir/parent.toml".to_string()]);
        // (The child manifest is also read directly.)
        let top_level: BTreeSet<String> = ["dir/parent.toml", "dir/sub/child.toml"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let entries = m.entries(&ancestors, &top_level);
        assert_eq!(entries["dir/sub/test_a.html"].len(), 2);
        assert_eq!(entries["dir/sub/test_a.html"][0].ancestor, None);
        assert_eq!(
            entries["dir/sub/test_a.html"][0].manifestparser_value("skip-if"),
            "http3\ndebug\nasan"
        );
        let a = &entries["dir/sub/test_a.html"][1];
        assert_eq!(a.ancestor.as_deref(), Some("dir/parent.toml"));
        assert_eq!(a.line, 6);
        assert_eq!(
            a.manifestparser_value("skip-if"),
            "os == 'android'\nverify\nhttp3\ndebug\nasan"
        );
        assert_eq!(
            a.conditions["skip-if"]
                .iter()
                .map(|c| (&c.from, c.manifest.as_str(), c.condition.line))
                .collect::<Vec<_>>(),
            vec![
                (&From::AncestorDefault, "dir/parent.toml", 2),
                (&From::Include, "dir/parent.toml", 7),
                (&From::Default, "dir/sub/child.toml", 3),
                (&From::Test, "dir/sub/child.toml", 8),
                (&From::Test, "dir/sub/child.toml", 9),
            ]
        );
        assert_eq!(
            a.manifestparser_value("run-if"),
            "(!msix) && (os == 'linux')\n(!msix) && (os == 'mac')"
        );
        assert_eq!(a.manifestparser_value("fail-if"), "xorigin");
        assert!(entries.contains_key("dir/other/test_b.html"));
    }

    #[test]
    fn test_manifest_value() {
        assert_eq!(manifest_value(&DeValue::String(" a # b ".into())), "a");
        assert_eq!(manifest_value(&DeValue::String("a#b".into())), "a#b");
    }
}
