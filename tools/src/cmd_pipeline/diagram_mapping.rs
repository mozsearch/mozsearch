//! What calls diagrams leave out (see `traverse`'s callees), from
//! diagram-mapping.toml (config_defaults', unless the config repo has its
//! own; see the file for what it says).  It's read when a diagram is made,
//! as dataflow-mapping.toml is when a trace runs, so that changing it doesn't
//! need a reindex.
//!
//! A group's functions (ex: mfbt's assertions, `Result`'s methods, nsCOMPtr's
//! operators, strings' and arrays' methods) are infrastructure and generic
//! data manipulation that most functions call: their nodes and edges crowd
//! out what a calls diagram is for, how the code's logic calls itself.  The
//! traversal leaves out their calls but records them (see `LeftOutCalls` in
//! cmd_traverse.rs), for the diagram's legend, which says what it left out
//! and links to the diagram with them (`show-calls:`), and for its nodes
//! (ex: badges saying which call them).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::dataflow::summary::untemplated_pretty;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingToml {
    #[serde(default, rename = "left-out")]
    left_out: Vec<GroupToml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupToml {
    group: String,
    name: String,
    why: String,
    #[serde(default)]
    functions: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    getters: bool,
}

/// A group of calls diagrams leave out (see the module's documentation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOutGroup {
    /// Its key, for `show-calls:` (ex: "strings").
    pub group: String,
    /// Its name, for people (ex: "Strings").
    pub name: String,
    pub why: String,
}

pub struct DiagramMapping {
    pub groups: Vec<LeftOutGroup>,
    /// The groups' functions, by their pretty names without template
    /// arguments (see `untemplated_pretty`).
    functions: HashMap<String, usize>,
    /// The groups' prefixes of functions' pretty names (ending in "::", a
    /// class's or namespace's functions), longest first.
    prefixes: Vec<(String, usize)>,
    /// The groups' prefixes of the paths of functions' definitions (ex:
    /// "xpcom/string/", or "xpcom/ds/nsTArray" for nsTArray.h and
    /// nsTArray-inl.h), longest first.
    paths: Vec<(String, usize)>,
    /// The group of getters (see `group_of_symbol`), if there's one.
    getters: Option<usize>,
}

impl DiagramMapping {
    pub fn parse(text: &str) -> Result<DiagramMapping, String> {
        let toml: MappingToml = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut mapping = DiagramMapping {
            groups: vec![],
            functions: HashMap::new(),
            prefixes: vec![],
            paths: vec![],
            getters: None,
        };
        for (i, group) in toml.left_out.into_iter().enumerate() {
            if mapping
                .groups
                .iter()
                .any(|other| other.group == group.group)
            {
                return Err(format!("group {:?} is defined twice", group.group));
            }
            if group.group == "all" || group.group.contains(',') {
                return Err(format!("{:?} can't be a group's key", group.group));
            }
            for function in group.functions {
                let function = untemplated_pretty(function.trim());
                if function.ends_with("::") {
                    mapping.prefixes.push((function, i));
                } else {
                    mapping.functions.insert(function, i);
                }
            }
            for path in group.paths {
                mapping.paths.push((path.trim().to_string(), i));
            }
            if group.getters {
                if mapping.getters.is_some() {
                    return Err("two groups are of getters".to_string());
                }
                mapping.getters = Some(i);
            }
            mapping.groups.push(LeftOutGroup {
                group: group.group,
                name: group.name,
                why: group.why,
            });
        }
        mapping
            .prefixes
            .sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        mapping
            .paths
            .sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        Ok(mapping)
    }

    /// The mapping of diagram-mapping.toml's `text`, parsed once for each
    /// text (so that a changed file's mapping is its new one), or None if it
    /// doesn't parse (which it logs).
    pub fn cached(text: &str) -> Option<Arc<DiagramMapping>> {
        type Cached = Option<(String, Option<Arc<DiagramMapping>>)>;
        static CACHED: Mutex<Cached> = Mutex::new(None);
        let mut cached = CACHED.lock().unwrap();
        if let Some((cached_text, mapping)) = cached.as_ref()
            && cached_text == text
        {
            return mapping.clone();
        }
        let mapping = match DiagramMapping::parse(text) {
            Ok(mapping) => Some(Arc::new(mapping)),
            Err(e) => {
                error!("diagram-mapping.toml: {}", e);
                None
            }
        };
        *cached = Some((text.to_string(), mapping.clone()));
        mapping
    }

    /// The group a function is in by its pretty name (with or without
    /// template arguments), if any: its whole name's, else its longest
    /// prefix's.  (Which a callee's crossref entry needn't be looked up for.)
    pub fn group_of_name(&self, pretty: &str) -> Option<usize> {
        let pretty = untemplated_pretty(pretty);
        self.functions.get(&pretty).copied().or_else(|| {
            self.prefixes
                .iter()
                .find(|(prefix, _)| pretty.starts_with(prefix.as_str()))
                .map(|(_, i)| *i)
        })
    }

    /// The group a function is in, if any: by its pretty name (see
    /// `group_of_name`), else by its definitions' paths, if they're all in
    /// the same group (by their longest prefixes), else, if there's a group
    /// of getters, whether it's one (`getter`: its dataflow summary says it
    /// only returns a field, and it calls nothing).
    pub fn group_of_symbol(&self, pretty: &str, paths: &[&str], getter: bool) -> Option<usize> {
        if let Some(i) = self.group_of_name(pretty) {
            return Some(i);
        }
        let of_path = |path: &str| {
            self.paths
                .iter()
                .find(|(prefix, _)| path.starts_with(prefix.as_str()))
                .map(|(_, i)| *i)
        };
        if let Some((first, rest)) = paths.split_first()
            && let Some(i) = of_path(first)
            && rest.iter().all(|path| of_path(path) == Some(i))
        {
            return Some(i);
        }
        match getter {
            true => self.getters,
            false => None,
        }
    }

    /// The groups `show-calls:`'s value names (separated by commas, or
    /// "all"), by their indices.
    pub fn shown_groups(&self, list: Option<&str>) -> Vec<bool> {
        let names: Vec<&str> = list
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        self.groups
            .iter()
            .map(|group| {
                names
                    .iter()
                    .any(|name| *name == "all" || *name == group.group)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
[[left-out]]
group = "assertions"
name = "Assertions"
why = "Checks."
functions = ["MOZ_ReportAssertionFailure"]
paths = ["mfbt/Assertions.h"]

[[left-out]]
group = "mfbt"
name = "MFBT"
why = "Templates."
functions = ["mozilla::Result::"]
paths = ["mfbt/"]

[[left-out]]
group = "getters"
name = "Getters"
why = "Fields."
getters = true
"#;

    #[test]
    fn test_group_matching() {
        let mapping = DiagramMapping::parse(TOML).unwrap();
        let group = |i: Option<usize>| i.map(|i| mapping.groups[i].group.as_str());
        assert_eq!(
            group(mapping.group_of_name("MOZ_ReportAssertionFailure")),
            Some("assertions")
        );
        assert_eq!(
            group(mapping.group_of_name("mozilla::Result<V, E>::isErr")),
            Some("mfbt")
        );
        assert_eq!(
            group(mapping.group_of_name("mozilla::ResultExtensions")),
            None
        );
        // (The longest path prefix's, for all of its definitions.)
        let paths =
            |pretty: &str, paths: &[&str]| group(mapping.group_of_symbol(pretty, paths, false));
        assert_eq!(
            paths("MOZ_CrashSequence", &["mfbt/Assertions.h"]),
            Some("assertions")
        );
        assert_eq!(
            paths("mozilla::Maybe::emplace", &["mfbt/Maybe.h"]),
            Some("mfbt")
        );
        assert_eq!(
            paths("MOZ_Other", &["mfbt/Assertions.h", "mfbt/Other.h"]),
            None
        );
        assert_eq!(paths("Foo::Bar", &["dom/Foo.cpp"]), None);
        assert_eq!(paths("Foo::Bar", &[]), None);
        // (Getters by their summaries, last.)
        assert_eq!(
            group(mapping.group_of_symbol("Foo::GetBar", &["dom/Foo.h"], true)),
            Some("getters")
        );
        assert_eq!(
            group(mapping.group_of_symbol("Foo::GetBar", &["mfbt/Maybe.h"], true)),
            Some("mfbt")
        );
    }

    #[test]
    fn test_shown_groups() {
        let mapping = DiagramMapping::parse(TOML).unwrap();
        assert_eq!(mapping.shown_groups(None), vec![false, false, false]);
        assert_eq!(
            mapping.shown_groups(Some("getters, assertions")),
            vec![true, false, true]
        );
        assert_eq!(mapping.shown_groups(Some("all")), vec![true, true, true]);
    }

    #[test]
    fn test_bad_groups() {
        let twice = format!("{}{}", TOML, TOML);
        assert!(DiagramMapping::parse(&twice).is_err());
        assert!(
            DiagramMapping::parse("[[left-out]]\ngroup = \"all\"\nname = \"\"\nwhy = \"\"\n")
                .is_err()
        );
    }

    /// config_defaults/diagram-mapping.toml parses, and its groups have the
    /// QueryCache diagram's infrastructure (see the file).
    #[test]
    fn test_defaults_parse() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../config_defaults/diagram-mapping.toml"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("Skipping: no config_defaults/diagram-mapping.toml here");
            return;
        };
        let mapping = DiagramMapping::parse(&text).unwrap();
        let group = |pretty: &str, path: &str| {
            mapping
                .group_of_symbol(pretty, &[path], false)
                .map(|i| mapping.groups[i].group.as_str())
        };
        for (pretty, path, expected) in [
            ("AnnotateMozCrashReason", "mfbt/Assertions.h", "assertions"),
            ("MOZ_AssertAssignmentTest", "", "assertions"),
            ("MOZ_CrashSequence", "mfbt/Assertions.h", "assertions"),
            ("MOZ_NoReturn", "mfbt/Assertions.h", "assertions"),
            (
                "MOZ_ReportAssertionFailure",
                "mfbt/Assertions.h",
                "assertions",
            ),
            ("abort", "memory/mozalloc/mozalloc_abort.cpp", "assertions"),
            (
                "NS_IsMainThread",
                "xpcom/threads/nsThreadManager.cpp",
                "assertions",
            ),
            ("mozilla::Result::Result<V, E>", "mfbt/Result.h", "result"),
            ("mozilla::Result::inspect", "mfbt/Result.h", "result"),
            ("mozilla::ToResult", "mfbt/ResultExtensions.h", "result"),
            (
                "mozilla::ToResultGet",
                "dom/quota/ResultExtensions.h",
                "result",
            ),
            (
                "mozilla::ToResultInvoke",
                "mfbt/ResultExtensions.h",
                "result",
            ),
            (
                "mozilla::dom::quota::HandleError",
                "dom/quota/QuotaCommon.h",
                "result",
            ),
            ("nsCOMPtr::nsCOMPtr<T>", "xpcom/base/nsCOMPtr.h", "pointers"),
            ("nsCOMPtr::operator->", "xpcom/base/nsCOMPtr.h", "pointers"),
            ("RefPtr::operator T *", "mfbt/RefPtr.h", "pointers"),
            (
                "operator\"\"_ns",
                "xpcom/string/nsLiteralString.h",
                "strings",
            ),
            (
                "nsTSubstring::AppendLiteral",
                "xpcom/string/nsTSubstring.h",
                "strings",
            ),
            (
                "nsTAutoStringN::nsTAutoStringN<T, N>",
                "xpcom/string/nsTString.h",
                "strings",
            ),
            (
                "mozilla::detail::nsTStringRepr::LowerCaseEqualsLiteral",
                "xpcom/string/nsTStringRepr.h",
                "strings",
            ),
            ("AutoTArray::AutoTArray<E, N>", "", "containers"),
            (
                "nsTArray_Impl::AppendElement",
                "xpcom/ds/nsTArray.h",
                "containers",
            ),
            ("mozilla::Maybe::isSome", "mfbt/Maybe.h", "containers"),
        ] {
            assert_eq!(group(pretty, path), Some(expected), "{}", pretty);
        }
        // (Not the code's own logic, nor what it does with others'.)
        for (pretty, path) in [
            (
                "mozilla::dom::cache::db::QueryCache",
                "dom/cache/DBSchema.cpp",
            ),
            (
                "mozilla::dom::cache::db::HashCString",
                "dom/cache/DBSchema.cpp",
            ),
            ("mozilla::CollectWhile", "dom/quota/QuotaCommon.h"),
            (
                "mozIStorageConnection::CreateStatement",
                "__GENERATED__/dist/include/mozIStorageConnection.h",
            ),
            (
                "nsObserverService::NotifyObservers",
                "xpcom/ds/nsObserverService.cpp",
            ),
        ] {
            assert_eq!(group(pretty, path), None, "{}", pretty);
        }
    }
}
