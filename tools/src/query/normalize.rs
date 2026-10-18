//! Normalizing what people search for into the query language's canonical
//! form (see `chew_query`): what a search box, a keyword or OpenSearch engine
//! or router.py's search's URLs (with its form's path field and case and
//! regexp options) send to `/search` (see web-server.rs) was typed without any
//! help with the query language's syntax, so it's read the way people type
//! searches (and router.py read them), and the canonical query is what's run
//! (and shown, so that it teaches the syntax): qualified names (`Foo::Bar`)
//! and phrases quoted, router.py's terms translated.  The `/query/` endpoint
//! itself stays strict.

use query_parser::{TermValue, parse};

use super::chew_query::{is_qualified_name, is_term};

/// A normalized query, and what normalizing it changed, for showing.
#[derive(Debug, Default, PartialEq)]
pub struct Normalization {
    pub query: String,
    pub changes: Vec<String>,
}

/// router.py's search form's fields, besides its query (`q`): a path glob,
/// case-sensitivity, and whether the text to search for is a regexp.
#[derive(Debug, Default)]
pub struct SearchForm<'a> {
    pub path: Option<&'a str>,
    pub case: bool,
    pub regexp: bool,
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

/// What a person searched for (`/search`'s `q`, and router.py's form's
/// fields), as a query:
/// - the query language's terms stay terms (`path:dom`, `calls-to:Foo`,
///   `-path:dom`);
/// - router.py's `re:`, `text:` and `symbol:` take the rest of the input, as
///   they did, after only terms, but for terms at its end (ex: `re:a b
///   path:dom` is `re:'a b' path:dom`), unless it quotes anything;
/// - qualified names are quoted (`Foo::Bar`, which the query language would
///   take as the term `Foo`);
/// - the rest is the text to search for, as one phrase, as router.py read it
///   (`foo bar`, which the query language would take as two searches), unless
///   the input quotes anything (then it's in the query language already);
/// - a word starting with a dash is text (ex: `-moz-appearance`), as is a
///   dash before a qualified name, but not one before a term (`-path:foo`);
/// - and the form's fields are terms (`path:`, `case:sensitive`), and with
///   its regexp option, the phrase is a regexp (`re:`).
pub fn normalize_search(input: &str, form: &SearchForm) -> Normalization {
    let input = input.trim();
    let phrase_prefix = if form.regexp { "re:" } else { "" };
    let mut normalization = rest_of_input(input).unwrap_or_else(|| words(input, phrase_prefix));
    let mut terms: Vec<String> = vec![normalization.query];
    if let Some(path) = form.path.map(str::trim).filter(|path| !path.is_empty()) {
        terms.push(format!("path:{}", quote(path)));
    }
    if form.case {
        terms.push("case:sensitive".to_string());
    }
    terms.retain(|term| !term.is_empty());
    normalization.query = terms.join(" ");
    normalization
}

/// What a person typed, without router.py's form's fields (see
/// `normalize_search`).
pub fn normalize_input(input: &str) -> Normalization {
    normalize_search(input, &SearchForm::default())
}

/// router.py's terms whose values are the rest of the input.
const REST_OF_INPUT: [&str; 3] = ["re:", "text:", "symbol:"];

/// The input as router.py's `re:`, `text:` or `symbol:` would have it (see
/// `normalize_search`), if it has one of them after only terms (and no
/// quotes).
fn rest_of_input(input: &str) -> Option<Normalization> {
    if input.contains(['\'', '"']) {
        return None;
    }
    let pieces: Vec<&str> = input.split(' ').collect();
    let rest_of_input = |piece: &str| {
        REST_OF_INPUT
            .into_iter()
            .find(|prefix| piece.starts_with(prefix))
    };
    let is_term_piece = |piece: &str| {
        let piece = piece.strip_prefix('-').unwrap_or(piece);
        rest_of_input(piece).is_none()
            && piece.split_once(':').is_some_and(|(key, value)| {
                is_term(key) && !value.is_empty() && !is_qualified_name(value)
            })
    };
    let at = pieces
        .iter()
        .position(|piece| rest_of_input(piece).is_some())?;
    if !pieces[..at]
        .iter()
        .all(|piece| piece.is_empty() || is_term_piece(piece))
    {
        return None;
    }
    let prefix = rest_of_input(pieces[at])?;
    let mut end = pieces.len();
    while end > at + 1 && is_term_piece(pieces[end - 1]) {
        end -= 1;
    }
    let value = pieces[at..end].join(" ")[prefix.len()..].to_string();
    let value = match prefix {
        "symbol:" => value.trim().to_string(),
        _ => value,
    };
    let (before, after) = (
        words(&pieces[..at].join(" "), ""),
        words(&pieces[end..].join(" "), ""),
    );
    let terms: Vec<String> = [
        before.query,
        format!("{}{}", prefix, quote(&value)),
        after.query,
    ]
    .into_iter()
    .filter(|term| !term.is_empty())
    .collect();
    Some(Normalization {
        query: terms.join(" "),
        changes: before.changes.into_iter().chain(after.changes).collect(),
    })
}

/// The input's terms, and its words, as one phrase (after `phrase_prefix`,
/// ex: `re:`), unless it quotes anything (see `normalize_search`).
fn words(input: &str, phrase_prefix: &str) -> Normalization {
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
        terms[index] = format!("{}{}", phrase_prefix, quote(&text));
    }
    Normalization {
        query: terms.join(" "),
        changes,
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
    fn test_normalize_search() {
        // router.py's search's URLs, with its form's fields.
        let n = |q: &str, path: Option<&str>, case: bool, regexp: bool| {
            normalize_search(q, &SearchForm { path, case, regexp }).query
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
        assert_eq!(n("a b", None, false, true), "re:'a b'");
        assert_eq!(n("x", None, true, false), "x case:sensitive");
        assert_eq!(
            n("Widget::Frob", Some("widget"), true, false),
            "'Widget::Frob' path:widget case:sensitive"
        );
        // router.py's `re:`, `text:` and `symbol:` take the rest of the input,
        // but terms at its end.
        let n = |q: &str| normalize_input(q).query;
        assert_eq!(n("re:foo bar"), "re:'foo bar'");
        assert_eq!(n("context:3 re:foo bar"), "context:3 re:'foo bar'");
        assert_eq!(n("path:dom re:a.*b c"), "path:dom re:'a.*b c'");
        assert_eq!(n("text:foo bar path:dom"), "text:'foo bar' path:dom");
        assert_eq!(n("text:if ( -path:dom"), "text:'if (' -path:dom");
        assert_eq!(n("symbol:_ZN3FooEv"), "symbol:_ZN3FooEv");
        assert_eq!(n("symbol:_ZN3FooEv "), "symbol:_ZN3FooEv");
        // (Not after the text to search for, nor quoted.)
        assert_eq!(n("calls-to:Foo depth:3"), "calls-to:Foo depth:3");
        assert_eq!(n("re:'a b' c"), "re:'a b' c");
    }
}
