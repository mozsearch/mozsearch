//! The DWIM ("do what I mean") endpoint's logic (`/{tree}/dwim?q=...`; see
//! bug 1908776): human input, from a keyword bookmark, an OpenSearch engine or
//! a query tree's search box, goes where its rule (dwim.toml) says, if it's
//! something one has (ex: another tool's URL for a file), and is otherwise
//! searched for, as the query language reads it (see `normalize`).

use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;

use super::chew_query::{is_qualified_name, is_term};
use super::normalize::{Normalization, normalize_input};

#[derive(Deserialize)]
struct RulesConfig {
    rule: Vec<RuleConfig>,
}

#[derive(Deserialize)]
struct RuleConfig {
    name: String,
    pattern: String,
    redirect: String,
    exists: Option<String>,
}

struct Rule {
    name: String,
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
                redirect: rule.redirect,
                exists: rule.exists,
            })
            .collect()
    };
}

/// What to do with the input.
#[derive(Debug, PartialEq)]
pub enum Dwim {
    /// Go to the URL (ex: "/{tree}/source/..."), by the named rule.
    Redirect { rule: String, url: String },
    /// Search for it (with the query language, in a tree searching with it).
    Search(Normalization),
}

/// What to do with the input in the tree, whose paths `has_path` says it has.
pub fn dwim(tree: &str, input: &str, has_path: &dyn Fn(&str) -> bool) -> Dwim {
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
        return Dwim::Redirect {
            rule: rule.name.clone(),
            url: substitute(&rule.redirect, &capture),
        };
    }
    Dwim::Search(normalize_input(input))
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
/// paths do, so those end with a space (which the DWIM endpoint trims).
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
        match dwim("firefox-main", input, &has_path) {
            Dwim::Redirect { url, .. } => Some(url),
            Dwim::Search(_) => None,
        }
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

    #[test]
    fn test_search() {
        let has_path = |_: &str| false;
        assert_eq!(
            dwim("t", " Foo::Bar ", &has_path),
            Dwim::Search(normalize_input("Foo::Bar"))
        );
    }
}
