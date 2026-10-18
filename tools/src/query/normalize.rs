//! Normalizing human-authored search input into the query language's
//! canonical form (see `chew_query`): what a keyword bookmark, an OpenSearch
//! engine or the classic search form (router.py's `/search`, with its path
//! field and case and regexp checkboxes) sends was typed without any help
//! with the query language's syntax, so it's read the way people type
//! searches, and the canonical query is what's run (and shown, so that it
//! teaches the syntax): qualified names (`Foo::Bar`) and phrases quoted,
//! classic terms translated.  The `/query/` endpoint itself stays strict.

use query_parser::{TermValue, parse};

use super::chew_query::{is_qualified_name, is_term};

/// A normalized query, and what normalizing it changed, for showing.
#[derive(Debug, Default, PartialEq)]
pub struct Normalization {
    pub query: String,
    pub changes: Vec<String>,
}

/// A value as the query language reads it back: bare if that's unambiguous,
/// else single-quoted (with backslashes before quotes and backslashes).
pub fn quote(value: &str) -> String {
    let bare = !value.is_empty()
        && !value.starts_with('-')
        && !value.contains(['\'', '"', '\\', ':'])
        && !value.chars().any(char::is_whitespace);
    if bare {
        return value.to_string();
    }
    let mut out = String::from("'");
    for c in value.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// Human-typed input (ex: a keyword bookmark's `%s`): the query language's
/// terms stay terms (`path:dom`, `calls-to:Foo`), qualified names are quoted
/// (`Foo::Bar`, which the query language would take as the term `Foo`), and
/// the rest is the text to search for, as one phrase, the way the classic
/// search reads it (`foo bar`, which the query language would take as two
/// searches), unless the input quotes anything (then it's in the query
/// language already, which a query's page's search box has, and a phrase is
/// quoted).  A word starting with a dash is text (ex: `-moz-appearance`; the
/// query language takes it as a negated term, and has no negation yet), as is
/// a dash before a qualified name, but not one before a term (`-path:foo`).
pub fn normalize_input(input: &str) -> Normalization {
    let mut terms: Vec<String> = vec![];
    // (Where the phrase goes, and its words.)
    let mut phrase: Option<(usize, Vec<String>)> = None;
    let mut changes: Vec<String> = vec![];
    let quoted = input.contains(['\'', '"']);
    for term in parse(input).terms {
        let TermValue::Simple(value) = term.value;
        let negation = if term.negated { "-" } else { "" };
        let text = match term.key {
            Some(key) if is_term(&key) && !is_qualified_name(&value) => {
                terms.push(format!("{}{}:{}", negation, key, quote(&value)));
                continue;
            }
            Some(key) => {
                let text = format!("{}:{}", key, value);
                if is_qualified_name(&value) {
                    changes.push(format!("quoted the qualified name {}", text));
                }
                text
            }
            None => value,
        };
        if quoted {
            terms.push(format!("{}{}", negation, quote(&text)));
            continue;
        }
        let text = format!("{}{}", negation, text);
        match &mut phrase {
            Some((_, words)) => words.push(text),
            None => {
                phrase = Some((terms.len(), vec![text]));
                terms.push(String::new());
            }
        }
    }
    if let Some((index, words)) = phrase {
        let text = words.join(" ");
        if words.len() > 1 {
            changes.push(format!("searched for the phrase \"{}\"", text));
        }
        terms[index] = quote(&text);
    }
    Normalization {
        query: terms.join(" "),
        changes,
    }
}

/// The classic search's input (router.py's `/search`): its query string's
/// terms (`path:`, `pathre:`, `context:N`, `id:`, and `symbol:`, `re:` and
/// `text:`, which take the rest of it; else the rest of it is the text to
/// search for), and its form's fields (a path glob, case-sensitivity, and
/// whether the text is a regexp), as the query language has them.
pub fn normalize_classic(q: &str, path: Option<&str>, case: bool, regexp: bool) -> Normalization {
    let mut terms: Vec<String> = vec![];
    let pieces: Vec<&str> = q.split(' ').collect();
    let rest = |i: usize, prefix: &str| pieces[i..].join(" ")[prefix.len()..].to_string();
    let mut text: Option<String> = None;
    for (i, piece) in pieces.iter().enumerate() {
        if piece.is_empty() {
            continue;
        }
        let simple = ["path:", "pathre:", "context:", "id:"]
            .into_iter()
            .find(|prefix| piece.starts_with(prefix));
        if let Some(prefix) = simple {
            terms.push(format!(
                "{}:{}",
                &prefix[..prefix.len() - 1],
                quote(&piece[prefix.len()..])
            ));
            continue;
        }
        if let Some(prefix) = ["symbol:", "re:", "text:"]
            .into_iter()
            .find(|prefix| piece.starts_with(prefix))
        {
            terms.push(format!(
                "{}:{}",
                &prefix[..prefix.len() - 1],
                quote(rest(i, prefix).trim())
            ));
            break;
        }
        text = Some(rest(i, ""));
        break;
    }
    if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
        if regexp {
            terms.push(format!("re:{}", quote(&text)));
        } else {
            terms.push(quote(&text));
        }
    }
    if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
        terms.push(format!("path:{}", quote(path)));
    }
    if case {
        terms.push("case:sensitive".to_string());
    }
    Normalization {
        query: terms.join(" "),
        changes: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote() {
        assert_eq!(quote("Foo"), "Foo");
        assert_eq!(quote("Foo::Bar"), "'Foo::Bar'");
        assert_eq!(quote("foo bar"), "'foo bar'");
        assert_eq!(quote("it's"), "'it\\'s'");
        assert_eq!(quote("-x"), "'-x'");
    }

    #[test]
    fn test_normalize_input() {
        let n = |input: &str| normalize_input(input).query;
        assert_eq!(
            n("serviceworkercontainer::register"),
            "'serviceworkercontainer::register'"
        );
        assert_eq!(n("C::Foo"), "'C::Foo'");
        assert_eq!(n("WorkerPrivate"), "WorkerPrivate");
        assert_eq!(n("foo bar"), "'foo bar'");
        assert_eq!(
            n("path:dom/workers Foo::Bar"),
            "path:dom/workers 'Foo::Bar'"
        );
        assert_eq!(
            n("calls-to:'Foo::Bar' depth:3"),
            "calls-to:'Foo::Bar' depth:3"
        );
        assert_eq!(n("https://example.com/x"), "'https://example.com/x'");
        assert_eq!(n("-moz-appearance"), "'-moz-appearance'");
        assert_eq!(n("foo -bar baz"), "'foo -bar baz'");
        assert_eq!(n("-path:foo bar"), "-path:foo bar");
        // (Queries stay as they are.)
        for query in [
            "'foo bar' baz",
            "path:dom 'Foo::Bar'",
            "'Foo::Bar' -path:'a b'",
            "calls-to:'Foo::Bar' depth:3",
            "calls-to:'Foo::Bar' colorize-callees:'::isFriendlyCat'",
        ] {
            assert_eq!(n(query), query);
        }
        assert_eq!(
            normalize_input("Foo::Bar").changes,
            vec!["quoted the qualified name Foo::Bar".to_string()]
        );
    }

    #[test]
    fn test_normalize_classic() {
        let n = |q: &str, path: Option<&str>, case: bool, regexp: bool| {
            normalize_classic(q, path, case, regexp).query
        };
        assert_eq!(n("Foo::Bar", None, false, false), "'Foo::Bar'");
        assert_eq!(
            n("path:dom foo bar", None, false, false),
            "path:dom 'foo bar'"
        );
        assert_eq!(
            n("foo", Some("dom/*.cpp"), false, false),
            "foo path:dom/*.cpp"
        );
        assert_eq!(
            n("foo", Some("dom/a b"), false, false),
            "foo path:'dom/a b'"
        );
        assert_eq!(n("a.*b", None, false, true), "re:a.*b");
        assert_eq!(
            n("context:3 re:foo bar", None, false, false),
            "context:3 re:'foo bar'"
        );
        assert_eq!(
            n("symbol:_ZN3FooEv", None, false, false),
            "symbol:_ZN3FooEv"
        );
        assert_eq!(n("x", None, true, false), "x case:sensitive");
    }
}
