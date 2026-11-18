//! What calls diagrams leave out (see `traverse`'s callees), from
//! diagram-mapping.toml (config_defaults', unless the config repo has its
//! own; see the file for what it says).  It's read when a diagram is made,
//! as dataflow-mapping.toml is when a trace runs, so that changing it doesn't
//! need a reindex.
//!
//! It also says what test code is, beyond test files (`[tests]`: test
//! frameworks' code, and tests' subsystems), and which subsystems are
//! infrastructure (`[overview]`), for roll-ups (crossref reads it when
//! indexing) and diagrams.
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
    #[serde(default)]
    tests: TestsToml,
    #[serde(default)]
    overview: OverviewToml,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestsToml {
    #[serde(default)]
    frameworks: Vec<String>,
    #[serde(default)]
    subsystems: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverviewToml {
    #[serde(default)]
    infrastructure: Vec<String>,
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
    /// The groups' functions in any class or namespace, by their names'
    /// last components (`*::Name`), or their prefixes (`*::Prefix*`, whose
    /// `bool` is true), longest first.
    last_names: Vec<(String, bool, usize)>,
    /// The groups' prefixes of the paths of functions' definitions (ex:
    /// "xpcom/string/", or "xpcom/ds/nsTArray" for nsTArray.h and
    /// nsTArray-inl.h), longest first.
    paths: Vec<(String, usize)>,
    /// The group of getters (see `group_of_symbol`), if there's one.
    getters: Option<usize>,
    /// Test frameworks' paths' prefixes (see `is_test_code`).
    test_frameworks: Vec<String>,
    /// Tests' subsystems, by name or, ending in "/", prefix (see
    /// `is_test_subsystem`).
    test_subsystems: Vec<String>,
    /// Infrastructure subsystems (see `is_infrastructure_subsystem`).
    infrastructure: Vec<String>,
}

impl DiagramMapping {
    pub fn parse(text: &str) -> Result<DiagramMapping, String> {
        let toml: MappingToml = toml::from_str(text).map_err(|e| e.to_string())?;
        let trim = |list: Vec<String>| -> Vec<String> {
            list.into_iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        };
        let mut mapping = DiagramMapping {
            groups: vec![],
            functions: HashMap::new(),
            prefixes: vec![],
            last_names: vec![],
            paths: vec![],
            getters: None,
            test_frameworks: trim(toml.tests.frameworks),
            test_subsystems: trim(toml.tests.subsystems),
            infrastructure: trim(toml.overview.infrastructure),
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
                if let Some(name) = function.trim().strip_prefix("*::") {
                    let (name, prefix) = match name.strip_suffix('*') {
                        Some(name) => (name, true),
                        None => (name, false),
                    };
                    if name.is_empty() || name.contains(['*', ':']) {
                        return Err(format!("{:?} isn't a name or a name's prefix", function));
                    }
                    mapping.last_names.push((name.to_string(), prefix, i));
                    continue;
                }
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
            .last_names
            .sort_by_key(|(name, _, _)| std::cmp::Reverse(name.len()));
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
    /// prefix's, else its last component's (ex: `*::AssertIsOn*`'s, for
    /// `mozilla::ipc::AssertIsOnBackgroundThread`).  (Which a callee's
    /// crossref entry needn't be looked up for.)
    pub fn group_of_name(&self, pretty: &str) -> Option<usize> {
        let pretty = untemplated_pretty(pretty);
        self.functions.get(&pretty).copied().or_else(|| {
            self.prefixes
                .iter()
                .find(|(prefix, _)| pretty.starts_with(prefix.as_str()))
                .map(|(_, i)| *i)
                .or_else(|| {
                    let last = pretty.rsplit("::").next().unwrap_or_default();
                    self.last_names
                        .iter()
                        .find(|(name, prefix, _)| match prefix {
                            true => last.starts_with(name.as_str()),
                            false => last == name,
                        })
                        .map(|(_, _, i)| *i)
                })
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

    /// Whether a file is a test framework's (by `[tests]`'s `frameworks`,
    /// ex: googletest's).
    pub fn is_test_framework_path(&self, path: &str) -> bool {
        self.test_frameworks
            .iter()
            .any(|prefix| path.starts_with(prefix.as_str()))
    }

    /// Whether a file (by its path kind and path) is test code's: a test
    /// file (per-file-info.toml's `pathkind.test`) or a test framework's.
    pub fn is_test_path(&self, path_kind: &str, path: &str) -> bool {
        path_kind == "test" || self.is_test_framework_path(path)
    }

    /// Whether a symbol, by its definitions' path kinds and paths, is test
    /// code: some of its definitions are a test framework's (whose macros
    /// define tests: a gtest's TEST is defined in its file, wherever that
    /// is, and in googletest's headers), or all of them are in test files.
    /// (A symbol without definitions isn't.)  Roll-ups leave out test code's
    /// calls, and the calls of it (see crossref's `test_code`), and diagrams
    /// test files' uses but of test code (see `Traverse::include_tests`).
    pub fn is_test_code<'a>(
        &self,
        definitions: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> bool {
        let mut any = false;
        let mut all_tests = true;
        for (path_kind, path) in definitions {
            any = true;
            if self.is_test_framework_path(path) {
                return true;
            }
            all_tests &= path_kind == "test";
        }
        any && all_tests
    }

    /// Whether a symbol is test code (see `is_test_code`), by its crossref
    /// entry's definitions.
    pub fn is_test_xref(&self, xref: &crate::file_format::crossref::CrossrefData) -> bool {
        xref.is_test_code_with(&|path| self.is_test_framework_path(path))
    }

    /// Whether a subsystem is tests' (by `[tests]`'s `subsystems`: names, or
    /// prefixes ending in "/", ex: "Testing/"), which block diagrams leave
    /// out (see `cmd_block_diagram`).
    pub fn is_test_subsystem(&self, subsystem: &str) -> bool {
        Self::names(&self.test_subsystems, subsystem)
    }

    /// Whether a subsystem is infrastructure (by `[overview]`'s
    /// `infrastructure`, as `is_test_subsystem`'s), which the whole tree's
    /// block diagram draws as it does the omnipresent subsystems, without
    /// the calls into it.
    pub fn is_infrastructure_subsystem(&self, subsystem: &str) -> bool {
        Self::names(&self.infrastructure, subsystem)
    }

    fn names(list: &[String], subsystem: &str) -> bool {
        list.iter().any(|name| match name.ends_with('/') {
            true => subsystem.starts_with(name.as_str()),
            false => subsystem == name,
        })
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
    fn test_last_names() {
        let mapping = DiagramMapping::parse(
            r#"
[[left-out]]
group = "checks"
name = "Checks"
why = "Checks."
functions = ["*::AssertIsOn*", "*::Exactly", "Foo::"]
"#,
        )
        .unwrap();
        let left_out = |pretty: &str| mapping.group_of_name(pretty).is_some();
        // (In any class or namespace, or none, by a prefix or the whole name.)
        assert!(left_out("mozilla::ipc::AssertIsOnBackgroundThread"));
        assert!(left_out("Bar::AssertIsOnOwningThread"));
        assert!(left_out("AssertIsOnMainThread"));
        assert!(left_out("AssertIsOn"));
        assert!(left_out("Bar::Exactly"));
        assert!(left_out("Foo::Anything"));
        assert!(!left_out("Bar::ExactlyNot"));
        assert!(!left_out("Bar::IsAssertIsOn"));
        // (Only the last component: not a class of that name's functions, nor
        // the class itself, as block diagrams match classes.)
        assert!(!left_out("AssertIsOnFoo::Bar"));
        assert!(!left_out("AssertIsOnFoo::"));
        for bad in ["*::", "*::*", "*::A*B*", "*::a::b"] {
            let toml = format!(
                "[[left-out]]\ngroup = \"g\"\nname = \"G\"\nwhy = \"W\"\nfunctions = [{:?}]\n",
                bad
            );
            assert!(DiagramMapping::parse(&toml).is_err(), "{}", bad);
        }
    }

    #[test]
    fn test_tests_and_overview() {
        let toml = format!(
            "{}{}",
            TOML,
            r#"
[tests]
frameworks = ["third_party/googletest/"]
subsystems = ["Testing/", "Core/Test Things"]

[overview]
infrastructure = ["Core/IPC"]
"#
        );
        let mapping = DiagramMapping::parse(&toml).unwrap();
        // (A gtest's TEST: in its file, and in googletest's macros.)
        assert!(mapping.is_test_code([
            ("normal", "ipc/ports_unittest.cpp"),
            (
                "third_party",
                "third_party/googletest/include/gtest/internal/gtest-internal.h"
            ),
        ]));
        assert!(mapping.is_test_code([("test", "dom/tests/a.cpp"), ("test", "dom/tests/b.cpp")]));
        assert!(!mapping.is_test_code([("test", "dom/tests/a.cpp"), ("normal", "dom/a.cpp")]));
        assert!(!mapping.is_test_code([]));
        assert!(mapping.is_test_path("third_party", "third_party/googletest/src/gtest.cc"));
        assert!(mapping.is_test_path("test", "dom/tests/a.cpp"));
        assert!(!mapping.is_test_path("third_party", "third_party/libwebrtc/a.cc"));
        assert!(mapping.is_test_subsystem("Testing/GTest"));
        assert!(mapping.is_test_subsystem("Core/Test Things"));
        assert!(!mapping.is_test_subsystem("Core/Test Things Too"));
        assert!(!mapping.is_test_subsystem("Core/Testing"));
        assert!(mapping.is_infrastructure_subsystem("Core/IPC"));
        assert!(!mapping.is_infrastructure_subsystem("Core/IPC/Other"));
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
            // (SafeRefPtr, which dom/indexedDB has, and dom/cache and
            // dom/quota use, and its classes' SafeRefPtrFromThis.)
            (
                "mozilla::SafeRefPtr::operator->",
                "dom/indexedDB/SafeRefPtr.h",
                "pointers",
            ),
            (
                "mozilla::MakeSafeRefPtr",
                "dom/indexedDB/SafeRefPtr.h",
                "pointers",
            ),
            (
                "mozilla::dom::cache::ReadStream::Inner::SafeRefPtrFromThis",
                "dom/indexedDB/SafeRefPtr.h",
                "pointers",
            ),
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
            (
                "IPC::ParamTraits::Write",
                "dom/ipc/PermissionMessageUtils.h",
                "ipc",
            ),
            (
                "mozilla::ipc::IPCResult::Ok",
                "ipc/glue/ProtocolUtils.h",
                "ipc",
            ),
            (
                "mozilla::binding_danger::TErrorResult::Throw",
                "dom/bindings/ErrorResult.h",
                "bindings",
            ),
            (
                "mozilla::dom::Optional_base::WasPassed",
                "dom/bindings/BindingDeclarations.h",
                "bindings",
            ),
            // (QuotaManager::ShutdownInstance's thread assertion, time
            // measurements and runnable.)
            (
                "mozilla::ipc::AssertIsOnBackgroundThread",
                "ipc/glue/BackgroundParent.h",
                "assertions",
            ),
            (
                "mozilla::dom::quota::BackgroundThreadObject::AssertIsOnOwningThread",
                "dom/quota/QuotaCommon.h",
                "assertions",
            ),
            (
                "mozilla::OffTheBooksMutex::AssertCurrentThreadOwns",
                "xpcom/threads/Mutex.h",
                "assertions",
            ),
            (
                "mozilla::NowExcludingSuspendMs",
                "mozglue/misc/Now.h",
                "time",
            ),
            (
                "mozilla::BaseTimeDuration::FromMilliseconds",
                "mozglue/misc/TimeStamp.h",
                "time",
            ),
            (
                "mozilla::TimeStamp::Now",
                "mozglue/misc/TimeStamp.h",
                "time",
            ),
            (
                "NS_NewRunnableFunction",
                "xpcom/threads/nsThreadUtils.h",
                "runnables",
            ),
            (
                "mozilla::detail::RunnableFunction::RunnableFunction<StoredFunction>",
                "xpcom/threads/nsThreadUtils.h",
                "runnables",
            ),
            (
                "NS_DispatchToMainThread",
                "xpcom/threads/nsThreadUtils.cpp",
                "runnables",
            ),
            // (And QuotaManager::Shutdown's tracing span and log context.)
            (
                "mozilla::gecko_trace::Tracer::StartSpan",
                "toolkit/components/gecko-trace/GeckoTrace.h",
                "logging",
            ),
            (
                "mozilla::dom::quota::ScopedLogExtraInfo::ScopedLogExtraInfo",
                "dom/quota/ScopedLogExtraInfo.h",
                "logging",
            ),
        ] {
            assert_eq!(group(pretty, path), Some(expected), "{}", pretty);
        }
        // (Classes, as block diagrams match them: their functions' prefix.)
        for (class, path, expected) in [
            ("nsTArray_Impl::", "xpcom/ds/nsTArray.h", "containers"),
            ("nsTSubstring::", "xpcom/string/nsTSubstring.h", "strings"),
            (
                "mozilla::binding_danger::TErrorResult::",
                "dom/bindings/ErrorResult.h",
                "bindings",
            ),
            (
                "mozilla::ipc::IPCResult::",
                "ipc/glue/ProtocolUtils.h",
                "ipc",
            ),
        ] {
            assert_eq!(group(class, path), Some(expected), "{}", class);
        }
        assert_eq!(
            group(
                "mozilla::dom::LocalStorageCache::",
                "dom/storage/LocalStorageCache.h"
            ),
            None
        );
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
            // (IPC's own logic, and actors' receiving of messages.)
            (
                "mozilla::ipc::MessageChannel::Send",
                "ipc/glue/MessageChannel.cpp",
            ),
            (
                "mozilla::ipc::BackgroundParentImpl::RecvPBroadcastChannelConstructor",
                "ipc/glue/BackgroundParentImpl.cpp",
            ),
            // (Telemetry, a nested event loop, and the code's own helpers.)
            (
                "mozilla::glean::impl::TimingDistributionStandalone::AccumulateRawDuration",
                "toolkit/components/glean/bindings/private/TimingDistributionStandalone.h",
            ),
            (
                "mozilla::SpinEventLoopUntil",
                "xpcom/threads/nsThreadUtils.h",
            ),
            (
                "mozilla::dom::quota::RecordTimeDeltaHelper::Start",
                "dom/quota/ActorsParent.cpp",
            ),
        ] {
            assert_eq!(group(pretty, path), None, "{}", pretty);
        }
        // (The overview's infrastructure that isn't omnipresent.)
        assert!(mapping.is_infrastructure_subsystem("Core/IPC"));
        assert!(mapping.is_infrastructure_subsystem("Core/mozglue"));
    }
}
