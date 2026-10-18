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
    redirect: String,
    exists: Option<String>,
}

struct Rule {
    name: String,
    description: String,
    pattern: Regex,
    redirect: String,
    exists: Option<String>,
}

lazy_static! {
    static ref RULES: Vec<Rule> = {
        let config: RulesConfig = toml::from_str(include_str!("dwim.toml")).unwrap();
        config
            .rule
            .into_iter()
            .map(|rule| Rule {
                pattern: Regex::new(&rule.pattern).unwrap(),
                name: rule.name,
                description: rule.description,
                redirect: rule.redirect,
                exists: rule.exists,
            })
            .collect()
    };
}

/// Where a DWIM rule says the input may be: its URL (ex: "/{tree}/source/...",
/// or another site's), and the rule's name and description.
#[derive(Debug, PartialEq)]
pub struct DwimTarget {
    pub rule: String,
    pub description: String,
    pub url: String,
}

/// Where the first DWIM rule matching the input says it may be in the tree,
/// whose paths `has_path` says it has, if one does.
pub fn dwim_target(tree: &str, input: &str, has_path: &dyn Fn(&str) -> bool) -> Option<DwimTarget> {
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
        return Some(DwimTarget {
            rule: rule.name.clone(),
            description: rule.description.clone(),
            url: substitute(&rule.redirect, &capture),
        });
    }
    None
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

    fn redirect(input: &str) -> Option<String> {
        let has_path = |path: &str| path.starts_with("dom/");
        dwim_target("firefox-main", input, &has_path).map(|target| target.url)
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
        // (Paths the tree doesn't have are searched for.)
        assert_eq!(
            redirect("https://github.com/x/y/blob/main/widget/Foo.cpp"),
            None
        );
        assert_eq!(redirect("widget/Foo.cpp:12"), None);
        assert_eq!(redirect("Foo::Bar"), None);
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
