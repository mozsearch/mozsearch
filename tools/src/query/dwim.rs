//! DWIM ("do what I mean") rules (dwim.toml; see bug 1908776): where what
//! a person searches for (in a tree searching with queries, `/search`'s
//! input; see web-server.rs) may be, if it's something one has (ex: another
//! tool's URL for a file), which its query's page says, beside the results of
//! searching for it (a search for a URL is of the URL: code cites them), and
//! goes to if the search finds nothing (see pipeline-server.rs).  And the
//! suggestions of OpenSearch engines (`/suggest`).

use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;

use super::chew_query::{is_qualified_name, is_term};

#[derive(Deserialize)]
struct RulesConfig {
    rule: Vec<RuleConfig>,
}

#[derive(Deserialize)]
struct RuleConfig {
    name: String,
    description: String,
    pattern: String,
    redirect: Option<String>,
    lookup: Option<String>,
    exists: Option<String>,
    go: Option<String>,
}

struct Rule {
    name: String,
    description: String,
    pattern: Regex,
    redirect: Option<String>,
    lookup: Option<String>,
    exists: Option<String>,
    always: bool,
}

lazy_static! {
    static ref RULES: Vec<Rule> = {
        let config: RulesConfig = toml::from_str(include_str!("dwim.toml")).unwrap();
        config
            .rule
            .into_iter()
            .map(|rule| {
                assert!(
                    rule.redirect.is_some() != rule.lookup.is_some(),
                    "DWIM rule {} needs a redirect or a lookup",
                    rule.name
                );
                Rule {
                    pattern: Regex::new(&rule.pattern).unwrap(),
                    always: match rule.go.as_deref() {
                        None => false,
                        Some("always") => true,
                        Some(go) => panic!("DWIM rule {}'s go = {:?}", rule.name, go),
                    },
                    name: rule.name,
                    description: rule.description,
                    redirect: rule.redirect,
                    lookup: rule.lookup,
                    exists: rule.exists,
                }
            })
            .collect()
    };
}

/// Where a DWIM rule says the input may be: its URL (ex: "/{tree}/source/...",
/// or another site's), the rule's name and description, and whether to go
/// there at once, without searching (its `go = "always"`).
#[derive(Debug, PartialEq)]
pub struct DwimTarget {
    pub rule: String,
    pub description: String,
    pub url: String,
    pub always: bool,
}

/// A lookup rule's lookup (ex: "spec") of the input's captures (by name).
pub type DwimLookup<'a> = &'a dyn Fn(&str, &dyn Fn(&str) -> Option<String>) -> Option<String>;

/// Where the first DWIM rule matching the input says it may be in the tree,
/// whose paths `has_path` says it has, and which `lookup` does the lookups of,
/// if one does.
pub fn dwim_target(
    tree: &str,
    input: &str,
    has_path: &dyn Fn(&str) -> bool,
    lookup: DwimLookup,
) -> Option<DwimTarget> {
    let input = input.trim();
    for rule in RULES.iter() {
        let Some(captures) = rule.pattern.captures(input) else {
            continue;
        };
        let capture = |name: &str| -> Option<String> {
            match name {
                "tree" => Some(tree.to_string()),
                name => captures.name(name).map(|m| m.as_str().to_string()),
            }
        };
        if let Some(exists) = &rule.exists
            && !capture(exists).is_some_and(|path| has_path(path.trim_end_matches('/')))
        {
            continue;
        }
        let url = match (&rule.redirect, &rule.lookup) {
            (Some(redirect), _) => substitute(redirect, &capture),
            (None, Some(kind)) => match lookup(kind, &capture) {
                Some(url) => url,
                None => continue,
            },
            (None, None) => continue,
        };
        return Some(DwimTarget {
            rule: rule.name.clone(),
            description: rule.description.clone(),
            url,
            always: rule.always,
        });
    }
    None
}

/// The URL of a target a search went to at once (see `DwimTarget::always`),
/// saying what was searched for (`searched`) and where its search is
/// (`searched-search`), for the page to offer it (see search.js), if it's
/// searchfox's (a path); before its fragment.
pub fn searched_url(url: &str, input: &str, search: &str) -> String {
    if !url.starts_with('/') || url.starts_with("//") {
        return url.to_string();
    }
    let (url, fragment) = match url.find('#') {
        Some(at) => url.split_at(at),
        None => (url, ""),
    };
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("searched", input.trim())
        .append_pair("searched-search", search)
        .finish();
    format!(
        "{}{}{}{}",
        url,
        if url.contains('?') { "&" } else { "?" },
        params,
        fragment
    )
}

/// What to suggest completions of for the input, as an OpenSearch engine's
/// suggestions (see web-server.rs's `suggest` route): if it's one word, it
/// and the identifiers it's the prefix of, or a term's value (ex:
/// `calls-to:Foo`), what's before it (`calls-to:`) and the value.
pub fn suggestion_needle(input: &str) -> Option<(&str, &str)> {
    if input.chars().any(char::is_whitespace) || input.contains(['\'', '"']) {
        return None;
    }
    let (before, needle) = match input.split_once(':') {
        Some((key, value)) if is_term(key) && !is_qualified_name(value) => {
            (&input[..key.len() + 1], value)
        }
        _ => ("", input),
    };
    (needle.chars().count() >= 2).then_some((before, needle))
}

/// An identifier as a suggestion, after what was before the needle (see
/// `suggestion_needle`): Firefox's address bar leaves out suggestions which
/// look like URLs (with no whitespace and a `/`, `@`, `:`, `[` or `.`; see its
/// UrlbarProviderSearchSuggestions's `looksLikeUrl`), as qualified names and
/// paths do, so those end with a space (which `/search` trims, in a tree
/// searching with queries; see `normalize::normalize_search`).
pub fn suggestion(before: &str, id: &str) -> String {
    let suggestion = format!("{}{}", before, id);
    match suggestion.contains(['/', '@', ':', '[', '.']) {
        true => suggestion + " ",
        false => suggestion,
    }
}

/// The template with its `{name}`s' values, leaving out its `[...]`s whose
/// names have none.
fn substitute(template: &str, value: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    // (The optional part being written, and whether its names have values.)
    let mut optional: Option<(String, bool)> = None;
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        let text = match c {
            '[' => {
                optional = Some((String::new(), true));
                continue;
            }
            ']' => {
                if let Some((text, true)) = optional.take() {
                    out.push_str(&text);
                }
                continue;
            }
            '{' => {
                let (name, after) = rest.split_once('}').unwrap_or((rest, ""));
                rest = after;
                match value(name) {
                    Some(value) => value,
                    None => {
                        if let Some((_, complete)) = &mut optional {
                            *complete = false;
                        }
                        String::new()
                    }
                }
            }
            c => c.to_string(),
        };
        match &mut optional {
            Some((optional, _)) => optional.push_str(&text),
            None => out.push_str(&text),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The target, with a spec lookup of "https://example.spec.whatwg.org/"'s
    /// URLs, and the other lookups' kinds and captures (see
    /// `format::dwim_lookup`), as "lookup:KIND:CAPTURE,...".
    fn target(input: &str) -> Option<DwimTarget> {
        let has_path = |path: &str| path.starts_with("dom/");
        let lookup = |kind: &str, capture: &dyn Fn(&str) -> Option<String>| {
            if kind != "spec" {
                let names: &[&str] = match kind {
                    "spec-pr" => &["repo", "number"],
                    "treeherder-try" => &["params"],
                    "explore-bug" => &["bug"],
                    "rev-at" => &["time"],
                    _ => &["rev"],
                };
                let values: Vec<String> = names.iter().filter_map(|name| capture(name)).collect();
                return Some(format!("lookup:{}:{}", kind, values.join(",")));
            }
            let url = capture("url")?;
            url.starts_with("https://example.spec.whatwg.org/")
                .then(|| {
                    format!(
                        "/specs/rendered/example.html{}",
                        capture("anchor").map_or_else(String::new, |a| format!("#{}", a))
                    )
                })
        };
        dwim_target("firefox-main", input, &has_path, &lookup)
    }

    fn redirect(input: &str) -> Option<String> {
        target(input).map(|target| target.url)
    }

    #[test]
    fn test_rules() {
        assert_eq!(
            redirect("https://github.com/web-platform-tests/wpt/pull/47169").as_deref(),
            Some("https://bugzilla.mozilla.org/buglist.cgi?quicksearch=%22Sync%20PR%2047169%22")
        );
        assert_eq!(
            redirect("https://searchfox.org/firefox-disco/source/dom/base/Foo.cpp#12").as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp#12")
        );
        assert_eq!(
            redirect("https://searchfox.org/firefox-disco/rev/abc123/dom/base/Foo.cpp").as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp")
        );
        assert_eq!(
            redirect(
                "https://github.com/mozilla-firefox/firefox/blob/main/dom/base/Foo.cpp#L12-L20"
            )
            .as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp#12-20")
        );
        assert_eq!(
            redirect("https://github.com/mozilla-firefox/firefox/blob/main/dom/base/Foo.cpp#L12")
                .as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp#12")
        );
        assert_eq!(
            redirect("https://hg.mozilla.org/mozilla-central/file/0123abcd/dom/base/Foo.cpp#l7")
                .as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp#7")
        );
        assert_eq!(
            redirect("/builds/worker/checkouts/gecko/dom/base/Foo.cpp:123:5:").as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp#123")
        );
        // (A crash-stats signature's stack: a query of its signature, as it is,
        // encoded.)
        assert_eq!(
            redirect(
                "https://crash-stats.mozilla.org/signature/?product=Firefox&signature=OOM%20%7C%20large%20%7C%20IPC%3A%3AParamTraits%3CJSStructuredCloneData%3E%3A%3ARead"
            )
            .as_deref(),
            Some(
                "/firefox-main/query/default?q=crash-signature%3A%27OOM%20%7C%20large%20%7C%20IPC%3A%3AParamTraits%3CJSStructuredCloneData%3E%3A%3ARead%27"
            )
        );
        assert_eq!(
            redirect("https://crash-stats.mozilla.org/signature/?signature=Foo%3A%3ABar&date=%3E%3D2026-10-13")
                .as_deref(),
            Some("/firefox-main/query/default?q=crash-signature%3A%27Foo%3A%3ABar%27")
        );
        // (A crash-stats report's stack: a query of its ID; with "all", every
        // thread's; and Bugzilla's crash IDs, but not bare UUIDs, which could
        // be interfaces'.)
        let report =
            "/firefox-main/query/default?q=crash-report%3Ab40697bb-fa73-46e6-bcd3-0f1ce0261030";
        assert_eq!(
            redirect("https://crash-stats.mozilla.org/report/index/b40697bb-fa73-46e6-bcd3-0f1ce0261030#tab-details")
                .as_deref(),
            Some(report)
        );
        assert_eq!(
            redirect("bp-b40697bb-fa73-46e6-bcd3-0f1ce0261030").as_deref(),
            Some(report)
        );
        assert_eq!(redirect("b40697bb-fa73-46e6-bcd3-0f1ce0261030"), None);
        assert_eq!(
            redirect("all https://crash-stats.mozilla.org/report/index/b40697bb-fa73-46e6-bcd3-0f1ce0261030")
                .as_deref(),
            Some(&*format!("{}%20crash-threads%3Aall", report))
        );
        assert!(
            target("bp-b40697bb-fa73-46e6-bcd3-0f1ce0261030")
                .unwrap()
                .always
        );
        // (Other tools' pages of commits, bugs, Phabricator revisions, spec
        // repositories' pull requests and try pushes are the lookups'.)
        for (input, lookup) in [
            (
                "https://github.com/whatwg/html/pull/13007",
                "lookup:spec-pr:whatwg/html,13007",
            ),
            (
                "https://github.com/whatwg/html/pull/13007/files#diff-1",
                "lookup:spec-pr:whatwg/html,13007",
            ),
            (
                "https://treeherder.mozilla.org/jobs?repo=try&revision=0123456789abcdef",
                "lookup:treeherder-try:repo=try&revision=0123456789abcdef",
            ),
            (
                "https://github.com/mozilla-firefox/firefox/commit/7af81fe47ac81825fa31f451dddfaeb736e109ba",
                "lookup:commit:7af81fe47ac81825fa31f451dddfaeb736e109ba",
            ),
            (
                "https://hg.mozilla.org/mozilla-central/rev/0123456789ab",
                "lookup:hg-commit:0123456789ab",
            ),
            (
                "https://bugzilla.mozilla.org/show_bug.cgi?id=1517978",
                "lookup:explore-bug:1517978",
            ),
            (
                "https://bugzilla.mozilla.org/1517978",
                "lookup:explore-bug:1517978",
            ),
            ("https://bugzil.la/1517978", "lookup:explore-bug:1517978"),
            (
                "https://phabricator.services.mozilla.com/D123456?id=789",
                "lookup:explore-phab:D123456",
            ),
            ("rev-at:2024-08-30", "lookup:rev-at:2024-08-30"),
            (
                "rev-at: 2024-08-30T07:26Z",
                "lookup:rev-at:2024-08-30T07:26Z",
            ),
        ] {
            assert_eq!(redirect(input).as_deref(), Some(lookup), "{}", input);
            assert!(target(input).unwrap().always, "{}", input);
        }
        // (A bug's comment is its view, at once.)
        for input in [
            "https://bugzilla.mozilla.org/show_bug.cgi?id=1915788#c20",
            "https://bugzilla.mozilla.org/show_bug.cgi?id=1915788&foo=bar#c20",
            "https://bugzilla.mozilla.org/1915788#c20",
            "https://bugzil.la/1915788#c20",
        ] {
            assert_eq!(
                redirect(input).as_deref(),
                Some("/firefox-main/query/default?q=bug-comment%3A1915788%2F20"),
                "{}",
                input
            );
            assert!(target(input).unwrap().always);
        }
        // (WPT's pull requests are its syncs', and files on hg aren't commits.)
        assert!(
            redirect("https://github.com/web-platform-tests/wpt/pull/47169")
                .unwrap()
                .starts_with("https://bugzilla")
        );
        assert_eq!(
            redirect("https://hg.mozilla.org/mozilla-central/file/0123abcd/dom/base/Foo.cpp")
                .as_deref(),
            Some("/firefox-main/source/dom/base/Foo.cpp")
        );
        // (Paths the tree doesn't have are searched for.)
        assert_eq!(
            redirect("https://github.com/x/y/blob/main/widget/Foo.cpp"),
            None
        );
        assert_eq!(redirect("widget/Foo.cpp:12"), None);
        assert_eq!(redirect("Foo::Bar"), None);
    }

    #[test]
    fn test_go_always() {
        // (A spec's URL is its rendered view, at once; a URL that isn't a
        // spec's isn't the rule's.)
        let spec = target("https://example.spec.whatwg.org/#widgets").unwrap();
        assert_eq!(spec.url, "/specs/rendered/example.html#widgets");
        assert!(spec.always);
        assert_eq!(redirect("https://example.org/#widgets"), None);
        // (Crash-stats signatures and WPT sync PRs go at once too; files'
        // URLs only if the search finds nothing.)
        assert!(
            target("https://crash-stats.mozilla.org/signature/?signature=Foo")
                .unwrap()
                .always
        );
        assert!(
            target("https://github.com/web-platform-tests/wpt/pull/1")
                .unwrap()
                .always
        );
        assert!(
            !target("https://searchfox.org/firefox-disco/source/dom/base/Foo.cpp")
                .unwrap()
                .always
        );
    }

    #[test]
    fn test_searched_url() {
        // (Searchfox's pages say what was searched for and where its search
        // is, before their fragments; others' don't.)
        assert_eq!(
            searched_url(
                "/specs/rendered/example.html#widgets",
                " https://x/#a ",
                "/t/query/default?q=%27x%27"
            ),
            "/specs/rendered/example.html?searched=https%3A%2F%2Fx%2F%23a&searched-search=%2Ft%2Fquery%2Fdefault%3Fq%3D%2527x%2527#widgets"
        );
        assert_eq!(
            searched_url("/t/query/default?q=a", "x", "/s"),
            "/t/query/default?q=a&searched=x&searched-search=%2Fs"
        );
        assert_eq!(
            searched_url("https://bugzilla.mozilla.org/x", "x", "/s"),
            "https://bugzilla.mozilla.org/x"
        );
        assert_eq!(
            searched_url("//evil.example/x", "x", "/s"),
            "//evil.example/x"
        );
    }

    #[test]
    fn test_suggestions() {
        assert_eq!(suggestion_needle("WorkerP"), Some(("", "WorkerP")));
        assert_eq!(suggestion_needle("Worker::Get"), Some(("", "Worker::Get")));
        assert_eq!(
            suggestion_needle("calls-to:Wor"),
            Some(("calls-to:", "Wor"))
        );
        assert_eq!(suggestion_needle("C::Foo"), Some(("", "C::Foo")));
        assert_eq!(suggestion_needle("path:dom foo"), None);
        assert_eq!(suggestion_needle("W"), None);
        assert_eq!(suggestion("", "WorkerPrivate"), "WorkerPrivate");
        assert_eq!(suggestion("", "WorkerPrivate::Get"), "WorkerPrivate::Get ");
        assert_eq!(suggestion("calls-to:", "Get"), "calls-to:Get ");
    }
}
