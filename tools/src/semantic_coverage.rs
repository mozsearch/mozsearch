//! Which of a tree's files lack the semantic analysis their language usually
//! has, and why, for their source listings' "No Semantic Analysis" info box
//! (`config_defaults/info_boxes/semantic.liquid`; bug 1789515), so that a
//! file's missing analysis says whether it's known (ex: firefox's vendored
//! Python which scip-python leaves out, bug 1999118) or unexpected (with a
//! Bugzilla search for it), rather than people having to read the indexer's
//! log.
//!
//! A tree's configuration names its coverage file (`semantic_coverage`, a
//! path in the config repo, which trees can share, ex: firefox-main's and
//! firefox-disco's `firefox-shared/semantic-coverage.toml`):
//!
//! ```toml
//! # The languages the tree analyzes semantically: their files' extensions,
//! # their analyzer, the prefixes of the symbols their analysis gives files,
//! # and terms to search Bugzilla with for a file which doesn't have it.
//! [[language]]
//! name = "Python"
//! extensions = ["py"]
//! analyzer = "scip-python"
//! symbol_prefixes = ["S_py_"]
//! bug_search = "searchfox python"
//!
//! # Files of those languages known to be left out of their analysis: their
//! # paths (a directory's files, or a file), why, and the bug about it.
//! [[gap]]
//! paths = ["third_party/python/pip/pip/_vendor"]
//! reason = "Vendored code which scip-python leaves out (it runs out of memory with it)."
//! bug = 1999118
//! ```

use serde::{Deserialize, Serialize};
use ustr::Ustr;

/// A tree's semantic coverage (see the module's documentation).
#[derive(Debug, Default, Deserialize)]
pub struct SemanticCoverage {
    #[serde(default, rename = "language")]
    pub languages: Vec<CoverageLanguage>,
    #[serde(default, rename = "gap")]
    pub gaps: Vec<CoverageGap>,
}

/// A language the tree analyzes semantically.
#[derive(Debug, Deserialize)]
pub struct CoverageLanguage {
    pub name: String,
    /// Its files' extensions, without their dots.
    pub extensions: Vec<String>,
    pub analyzer: String,
    /// The prefixes of the symbols its analysis gives its files (ex:
    /// scip-python's "S_py_"), by which a file has it: other analyzers' (ex:
    /// a semantic mapping's) don't count.
    pub symbol_prefixes: Vec<String>,
    /// Terms to search Bugzilla with for a file without it.
    #[serde(default)]
    pub bug_search: Option<String>,
}

/// Files known to be left out of their language's analysis.
#[derive(Debug, Deserialize)]
pub struct CoverageGap {
    /// Directories (whose files it is) and files.
    pub paths: Vec<String>,
    pub reason: String,
    #[serde(default)]
    pub bug: Option<u64>,
}

/// A file's missing analysis, for its info box: its language, the analyzer,
/// why (a gap's reason and bug, if a gap has it), and Bugzilla's search for
/// it otherwise.  (All of its fields are the info box's, missing ones as
/// empty, since liquid fails on missing keys.)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MissingAnalysis {
    pub language: String,
    pub analyzer: String,
    pub reason: String,
    pub bug: Option<u64>,
    pub bug_search: String,
}

impl SemanticCoverage {
    /// The coverage file `path` (in the config repo at `config_repo`).
    pub fn load(config_repo: &str, path: &str) -> Result<SemanticCoverage, String> {
        let file = format!("{}/{}", config_repo, path);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {}", file, e))?;
        toml::from_str(&text).map_err(|e| format!("{}: {}", file, e))
    }

    /// The missing analysis of the file at `path`, whose analysis's symbols
    /// are `syms`, if it's of a language the tree analyzes and its analysis
    /// doesn't have any of the language's symbols.
    pub fn missing<'a>(
        &self,
        path: &str,
        syms: impl IntoIterator<Item = &'a Ustr>,
    ) -> Option<MissingAnalysis> {
        let extension = path.rsplit_once('.')?.1;
        if path
            .rsplit_once('/')
            .map_or(path, |(_, name)| name)
            .starts_with('.')
        {
            return None;
        }
        let language = self
            .languages
            .iter()
            .find(|language| language.extensions.iter().any(|e| e == extension))?;
        let analyzed = syms.into_iter().any(|sym| {
            language
                .symbol_prefixes
                .iter()
                .any(|prefix| sym.starts_with(prefix.as_str()))
        });
        if analyzed {
            return None;
        }
        let gap = self.gaps.iter().find(|gap| {
            gap.paths.iter().any(|gap_path| {
                let gap_path = gap_path.trim_end_matches('/');
                path == gap_path
                    || path
                        .strip_prefix(gap_path)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        });
        Some(MissingAnalysis {
            language: language.name.clone(),
            analyzer: language.analyzer.clone(),
            reason: gap.map(|gap| gap.reason.clone()).unwrap_or_default(),
            bug: gap.and_then(|gap| gap.bug),
            bug_search: match gap {
                Some(_) => String::new(),
                None => language.bug_search.clone().unwrap_or_default(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ustr::ustr;

    #[test]
    fn test_missing() {
        let coverage: SemanticCoverage = toml::from_str(
            r#"
            [[language]]
            name = "Python"
            extensions = ["py"]
            analyzer = "scip-python"
            symbol_prefixes = ["S_py_"]
            bug_search = "searchfox python"

            [[gap]]
            paths = ["third_party/python/pip/pip/_vendor", "tools/one.py"]
            reason = "Vendored."
            bug = 1999118
            "#,
        )
        .unwrap();
        let none: Vec<Ustr> = vec![];
        // (A file with its language's analysis, and one without it, which
        // other analyzers' symbols don't count as.)
        assert_eq!(
            coverage.missing("taskcluster/task.py", &[ustr("S_py_python_x/task/f().")]),
            None
        );
        let unexpected = MissingAnalysis {
            language: "Python".into(),
            analyzer: "scip-python".into(),
            reason: String::new(),
            bug: None,
            bug_search: "searchfox python".into(),
        };
        assert_eq!(
            coverage.missing("taskcluster/task.py", &[ustr("TASK_kind/name")]),
            Some(unexpected)
        );
        // (A gap's directory's files, and its file, but not a file whose name
        // starts with the directory's.)
        let known = coverage
            .missing("third_party/python/pip/pip/_vendor/rich/x.py", &none)
            .unwrap();
        assert_eq!(
            (known.reason.as_str(), known.bug),
            ("Vendored.", Some(1999118))
        );
        assert_eq!(known.bug_search, "");
        assert_eq!(
            coverage.missing("tools/one.py", &none).unwrap().bug,
            Some(1999118)
        );
        assert_eq!(
            coverage
                .missing("third_party/python/pip/pip/_vendor2/x.py", &none)
                .unwrap()
                .bug,
            None
        );
        // (Other languages', and dotfiles, aren't.)
        assert_eq!(coverage.missing("dom/base/Element.cpp", &none), None);
        assert_eq!(coverage.missing("tools/.py", &none), None);
    }

    /// The info box (config_defaults/info_boxes/semantic.liquid), for a file
    /// with its analysis, a known gap's (with and without a bug), and an
    /// unexpected one's (with and without a Bugzilla search).
    #[test]
    fn test_info_box() {
        // (Not in the nix build's sandbox, which has only the tools' source;
        // webtest test_SemanticCoverage renders it too.)
        let Ok(source) = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../config_defaults/info_boxes/semantic.liquid"
        )) else {
            eprintln!("Skipping: no config_defaults/info_boxes/semantic.liquid here");
            return;
        };
        let template = crate::templating::builder::build_and_parse(&source);
        let render = |missing: Option<MissingAnalysis>| {
            let semantic = missing.map_or(liquid::model::Value::Nil, |missing| {
                liquid::model::to_value(&missing).unwrap()
            });
            template
                .render(&liquid::object!({ "semantic": semantic }))
                .unwrap()
        };
        assert_eq!(render(None).trim(), "");
        let mut missing = MissingAnalysis {
            language: "Python".into(),
            analyzer: "scip-python".into(),
            reason: "Vendored & excluded.".into(),
            bug: Some(1999118),
            bug_search: String::new(),
        };
        let known = render(Some(missing.clone()));
        assert!(known.contains("info-box-info"), "{}", known);
        assert!(
            known.contains("scip-python doesn't analyze this Python file"),
            "{}",
            known
        );
        assert!(known.contains("Vendored &amp; excluded."), "{}", known);
        assert!(
            known.contains(
                r#"href="https://bugzilla.mozilla.org/show_bug.cgi?id=1999118">Bug 1999118</a>"#
            ),
            "{}",
            known
        );
        missing.bug = None;
        assert!(!render(Some(missing.clone())).contains("show_bug.cgi"));
        missing.reason = String::new();
        missing.bug_search = "searchfox python".into();
        let unexpected = render(Some(missing.clone()));
        assert!(unexpected.contains("info-box-warning"), "{}", unexpected);
        assert!(
            unexpected.contains("but this one has no analysis"),
            "{}",
            unexpected
        );
        assert!(
            unexpected.contains("buglist.cgi?quicksearch=searchfox%20python"),
            "{}",
            unexpected
        );
        missing.bug_search = String::new();
        assert!(!render(Some(missing)).contains("buglist.cgi"));
    }
}
