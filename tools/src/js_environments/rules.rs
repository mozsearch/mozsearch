//! A tree's JS environment rules (ex: config's firefox-shared/js-environments.toml):
//! its test flavors, by the moz.build variables declaring their manifests,
//! with their globals and harness scripts; the URLs files are loaded at; and
//! web-platform-tests' roots and harness scripts (and Test262's).

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)]
    pub flavors: Vec<Flavor>,
    #[serde(default)]
    pub path_flavors: Vec<PathFlavor>,
    /// The globals with a `load(path)` function loading a script into them
    /// (ex: xpcshell's, and the JS shell's).
    #[serde(default)]
    pub load_globals: Vec<String>,
    /// URL prefixes, and the directories of the files they load (`""`: the
    /// tree's root), ex: `"/tests/SimpleTest/" =
    /// "testing/mochitest/tests/SimpleTest/"`; the longest matching prefix
    /// applies.
    #[serde(default)]
    pub urls: BTreeMap<String, String>,
    pub wpt: Option<WptRules>,
    /// Each global kind's TypeScript configuration, for its environments'
    /// projects (see `projects`), by kind (ex: "window", "xpcshell").
    #[serde(default)]
    pub globals: BTreeMap<String, GlobalTypes>,
    /// The scripts whose functions are in functions they call at once,
    /// exposing them as their global's properties (ex: web-platform-tests'
    /// testharness.js, `(function (global_scope) { ... })(self);`), which
    /// their projects unwrap (scip-typescript's `scipUnwrapFiles`), so that
    /// their declarations are the globals the scripts after them see; by
    /// globs.
    #[serde(default)]
    pub unwrap: Vec<String>,
}

/// Globs' set (`*` not matching `/`; an invalid glob matching nothing).
pub fn glob_set(patterns: &[String]) -> globset::GlobSet {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        if let Ok(glob) = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
        {
            builder.add(glob);
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| globset::GlobSet::empty())
}

/// A global kind's TypeScript configuration: a tsconfig.json to extend
/// (ex: Gecko's tools/@types/tsconfig.json, with its DOM and XPCOM types),
/// or TypeScript's libraries (ex: `["ESNext", "WebWorker"]`), or both, and
/// compiler options for its projects, over the extended configuration's (ex:
/// `{ paths = {} }` for web content, whose tests import none of the modules
/// Gecko's chrome:// and resource:// paths map, which tools/@types' typings'
/// module types otherwise add to each of their programs).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalTypes {
    pub extends: Option<String>,
    #[serde(default)]
    pub lib: Vec<String>,
    #[serde(default)]
    pub compiler_options: serde_json::Map<String, serde_json::Value>,
}

/// A test flavor's scope (ex: browser-chrome's): its harness loads a test
/// into an object, rather than into a global, whose members are the names it
/// sees, before its global's: those its `constructor`'s instances have (its
/// `this.name = ...` assignments, and its prototype's members), in `file`,
/// and its `modules`' exports (by name: a module URL, ex:
/// `BrowserTestUtils = "resource://testing-common/BrowserTestUtils.sys.mjs"`,
/// or `"new <URL>"` for an instance of its exported class, ex: `Assert`).
/// Its environments' projects declare them as globals in a generated shim,
/// whose declarations are their definitions' (see `projects::scope_shim`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub file: String,
    pub constructor: String,
    #[serde(default)]
    pub modules: BTreeMap<String, String>,
}

/// A test flavor: the tests its manifests list run in a global of its kind,
/// after its harness's scripts (and then their `head` files, if they're JS;
/// pages load their own scripts).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flavor {
    pub name: String,
    /// The moz.build variables declaring its manifests (ex:
    /// `XPCSHELL_TESTS_MANIFESTS`).
    pub manifest_variables: Vec<String>,
    /// Its global's kind (ex: "xpcshell", "window", "chrome-window").
    pub global: String,
    /// Its harness's scripts, before a JS test's head files.
    #[serde(default)]
    pub harness: Vec<String>,
    /// Head files its harness loads from a JS test's directory, if it has
    /// them, whatever its manifest says (ex: browser-chrome's `head.js`),
    /// before its manifest's.
    #[serde(default)]
    pub implicit_head: Vec<String>,
    /// Its tests' scope, if its harness loads them into an object.
    pub scope: Option<Scope>,
}

/// Tests found by their paths rather than by manifests (ex: SpiderMonkey's
/// jit-tests and jstests): each a JS test in a global of its kind, after its
/// harness's scripts, and its directories' `dir_heads` files, from
/// `dir_heads_root`'s down (as jstests' runner loads each directory's
/// shell.js).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathFlavor {
    pub name: String,
    /// Its tests' globs, and those which aren't tests.
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub global: String,
    #[serde(default)]
    pub harness: Vec<String>,
    #[serde(default)]
    pub dir_heads: Vec<String>,
    pub dir_heads_root: Option<String>,
    /// Variables which its scripts' loads concatenate a path to (ex:
    /// jit-tests' `load(libdir + "asserts.js")`), with their values: tree
    /// directories (`{dir}` being the test's).
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WptRules {
    /// The tests' root (their URLs' `/`), ex: "testing/web-platform/tests".
    pub root: String,
    /// testharness's scripts in a window, relative to the root (ex:
    /// "resources/testharness.js", "resources/testharnessreport.js"), before
    /// a `.any.js` test's `META: script`s.
    pub window_harness: Vec<String>,
    /// And in a worker.
    pub worker_harness: Vec<String>,
    /// And for a `.extension.js` test (tools/serve/serve.py's
    /// ExtensionHandler's: testharness's, testdriver's and
    /// web-extensions-helper.js), else the window's.
    #[serde(default)]
    pub extension_harness: Vec<String>,
    /// Other roots of tests, by their URLs' prefixes (ex: `"/_mozilla/" =
    /// "testing/web-platform/mozilla/tests"`), whose tests' path-absolute
    /// URLs are the root's but those with one of the prefixes.
    #[serde(default)]
    pub extra_roots: BTreeMap<String, String>,
    /// Its Test262 tests, if it runs them.
    pub test262: Option<Test262Rules>,
}

/// web-platform-tests' Test262 tests: each `.js` file with frontmatter in
/// `tests`' directories (but `_FIXTURE.js` files, which tests import), which
/// it runs in a window after `harness`, the test's frontmatter's `includes`
/// (in `includes_dir`), and, if it's `async`, `async_harness`, as a module if
/// it's a `module` one (tools/serve/serve.py's Test262WindowTestBaseHandler;
/// paths relative to the root).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Test262Rules {
    pub tests: Vec<String>,
    pub harness: Vec<String>,
    pub includes_dir: String,
    #[serde(default)]
    pub async_harness: Vec<String>,
}

impl Rules {
    pub fn parse(source: &str) -> Result<Rules, String> {
        toml::from_str(source).map_err(|e| e.to_string())
    }

    /// The file a URL loads, by the longest URL prefix matching it (without
    /// its query or fragment).
    pub fn url_file(&self, url: &str) -> Option<String> {
        let url = url.split(['?', '#']).next().unwrap_or(url);
        self.urls
            .iter()
            .filter(|(prefix, _)| url.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(prefix, dir)| format!("{}{}", dir, &url[prefix.len()..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_file() {
        let rules = Rules::parse(
            r#"
[urls]
"/tests/" = ""
"/tests/SimpleTest/" = "testing/mochitest/tests/SimpleTest/"
"#,
        )
        .unwrap();
        assert_eq!(
            rules.url_file("/tests/SimpleTest/SimpleTest.js").as_deref(),
            Some("testing/mochitest/tests/SimpleTest/SimpleTest.js")
        );
        assert_eq!(
            rules
                .url_file("/tests/dom/indexedDB/test/helpers.js?x=1")
                .as_deref(),
            Some("dom/indexedDB/test/helpers.js")
        );
        assert_eq!(rules.url_file("helpers.js"), None);
    }

    #[test]
    fn test_global_compiler_options() {
        let rules = Rules::parse(
            r#"
[globals.window]
extends = "tools/@types/tsconfig.json"
compiler_options = { paths = {}, noImplicitAny = false }
"#,
        )
        .unwrap();
        let options = &rules.globals["window"].compiler_options;
        assert_eq!(options["paths"], serde_json::json!({}));
        assert_eq!(options["noImplicitAny"], serde_json::json!(false));
    }
}
