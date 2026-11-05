//! A tree's JS environment rules (ex: config's firefox-shared/js-environments.toml):
//! its test flavors, by the moz.build variables declaring their manifests,
//! with their globals and harness scripts; the URLs files are loaded at; and
//! web-platform-tests' root and harness scripts.

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
}
