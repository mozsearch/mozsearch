//! A test's JS environments, as a diagram's scope (see `traverse --test`):
//! the files of the environments it's the root of (and of the workers and
//! frames they start), out of the tree's files in any environment (its
//! `js-environments-paths`).  A file of another environment (another test, or
//! its head files) can't call into the test's code, nor the test into it, as
//! they don't share a global: its uses aren't callers, ex: the other tests'
//! calls of their harness's `ok`.  But a file in no environment can (ex: a
//! module, product code).

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct EnvironmentScope {
    /// The test (an environment's root).
    pub root: String,
    /// Its environments' files.
    pub files: HashSet<String>,
    /// The tree's files in any environment.
    pub in_any: Arc<HashSet<String>>,
}

impl EnvironmentScope {
    /// The scope of a test, from its per-file information's
    /// `js_environments` (see per-file-info.toml: its `roots`, the trees of
    /// the environments it's the root of), if it's the root of any.
    pub fn of_test(
        root: &str,
        js_environments: &Value,
        in_any: Arc<HashSet<String>>,
    ) -> Option<EnvironmentScope> {
        let roots = js_environments.get("roots")?.as_array()?;
        if roots.is_empty() {
            return None;
        }
        let mut files = HashSet::new();
        files.insert(root.to_string());
        let mut trees: Vec<&Value> = roots.iter().collect();
        while let Some(tree) = trees.pop() {
            if let Some(path) = tree.get("root").and_then(Value::as_str) {
                files.insert(path.to_string());
            }
            for script in tree
                .get("scripts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(path) = script.get("path").and_then(Value::as_str) {
                    files.insert(path.to_string());
                }
            }
            trees.extend(
                tree.get("children")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            );
        }
        Some(EnvironmentScope {
            root: root.to_string(),
            files,
            in_any,
        })
    }

    /// Whether a file's code can call into the test's, or the test's into it:
    /// if it's one of its environments' files, or in no environment.
    pub fn allows(&self, path: &str) -> bool {
        self.files.contains(path) || !self.in_any.contains(path)
    }

    /// Whether a file is one of its environments'.
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_scope_of_test() {
        let in_any: Arc<HashSet<String>> = Arc::new(
            [
                "t/test_a.js",
                "t/head.js",
                "t/test_b.js",
                "t/worker.js",
                "t/test_page.html",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        );
        let info = json!({
            "path": "t/test_a.js",
            "roots": [{
                "root": "t/test_a.js",
                "scripts": [{"path": "t/head.js"}, {"path": "t/test_a.js"}],
                "children": [{"root": "t/worker.js", "scripts": [{"path": "t/worker.js"}]}],
            }],
            "in": [{"root": "t/test_page.html"}],
        });
        let scope = EnvironmentScope::of_test("t/test_a.js", &info, in_any.clone()).unwrap();
        assert!(scope.allows("t/head.js"));
        assert!(scope.allows("t/worker.js"));
        // (Another test's, or a page's it's in, which isn't its own.)
        assert!(!scope.allows("t/test_b.js"));
        assert!(!scope.allows("t/test_page.html"));
        // (A module's, in no environment.)
        assert!(scope.allows("t/Helper.sys.mjs"));
        // (Not a root.)
        assert!(EnvironmentScope::of_test("t/head.js", &json!({"in": []}), in_any).is_none());
    }
}
