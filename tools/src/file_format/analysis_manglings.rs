use regex::{Captures, Regex};

pub fn mangle_file(filename: &str) -> String {
    lazy_static! {
        // The column portion can potentially be singular I think so we just
        // treat the half as its own group.
        static ref RE: Regex = Regex::new(r"[^a-zA-Z0-9_/]").unwrap();
    }
    RE.replace_all(filename, |c: &Captures| {
        format!("@{:02X}", c[0].as_bytes()[0])
    })
    .to_string()
}

/// A symbol with its commas, which separate symbols in source records' "sym"s
/// (see `AnalysisSource::sym`) and pages' `data-symbols` attributes, escaped
/// like `mangle_file`'s, as "@2C" (ex: in the clang plugin's symbols for C++
/// templates' types, "T_Pair<A,_B>", before it escaped them).
pub fn escape_symbol_commas(sym: &str) -> std::borrow::Cow<'_, str> {
    if sym.contains(',') {
        std::borrow::Cow::Owned(sym.replace(',', "@2C"))
    } else {
        std::borrow::Cow::Borrowed(sym)
    }
}

/// Split a source record's comma-separated symbols, but not at commas inside
/// a symbol's brackets (ex: "T_Pair<A,_B>"'s; see `escape_symbol_commas`):
/// only at commas outside of "<>", "()", and "[]", where a ">" only closes a
/// "<" (so that "(N_>_0)" doesn't), and operators' "<" and ">" (ex:
/// "operator<<") don't count.  If the brackets don't balance, all commas
/// separate, as before.
pub fn split_symbol_list(syms: &str) -> Vec<&str> {
    let mut pieces = vec![];
    let mut stack: Vec<char> = vec![];
    let mut start = 0;
    for (i, c) in syms.char_indices() {
        let before = &syms[..i];
        let in_operator = ["operator", "operator<", "operator>", "operator<=", "operator-"]
            .iter()
            .any(|op| before.ends_with(op));
        match c {
            '<' | '>' if in_operator => {}
            '<' | '(' | '[' => stack.push(c),
            '>' => {
                if stack.last() == Some(&'<') {
                    stack.pop();
                }
            }
            ')' | ']' => {
                let open = if c == ')' { '(' } else { '[' };
                match stack.iter().rposition(|&o| o == open) {
                    Some(pos) => stack.truncate(pos),
                    None => return syms.split(',').collect(),
                }
            }
            ',' if stack.is_empty() => {
                pieces.push(&syms[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return syms.split(',').collect();
    }
    pieces.push(&syms[start..]);
    pieces
}

#[test]
fn test_symbol_commas() {
    assert_eq!(split_symbol_list("_ZN3fooEv,_ZN3barEv"), vec!["_ZN3fooEv", "_ZN3barEv"]);
    assert_eq!(
        split_symbol_list("TA_Pair<A,_B<C,_D>>::type,_ZN3barEv"),
        vec!["TA_Pair<A,_B<C,_D>>::type", "_ZN3barEv"]
    );
    assert_eq!(
        split_symbol_list("F_<T_Foo<(N_>_0),_int>>_mBar,T_Baz"),
        vec!["F_<T_Foo<(N_>_0),_int>>_mBar", "T_Baz"]
    );
    assert_eq!(
        split_symbol_list("T_ns::operator<<(A,_B)::Local,_ZN3barEv"),
        vec!["T_ns::operator<<(A,_B)::Local", "_ZN3barEv"]
    );
    // (Unbalanced brackets: all commas separate.)
    assert_eq!(split_symbol_list("T_a<b,T_c"), vec!["T_a<b", "T_c"]);
    assert_eq!(escape_symbol_commas("TA_Pair<A,_B>::type"), "TA_Pair<A@2C_B>::type");
    assert_eq!(escape_symbol_commas("_ZN3fooEv"), "_ZN3fooEv");
}

/// Escape the commas in an analysis record's symbols (see
/// `escape_symbol_commas`) from producers which didn't (ex: the clang plugin's
/// for C++ templates' types, from mozilla-central's copy of it), as JSON, as
/// `read_analyses` reads records (whether merge-analyses merged their files or
/// not, ex: generated files which differ between platforms): a source record's "sym" is its symbols, separated
/// by the commas outside their brackets (see `split_symbol_list`), and other
/// fields named "...sym" (in any records, and their parts) are one symbol,
/// and "...syms", "subclasses", and "overriddenBy" lists of them.  (Source
/// records' expansions are keyed by lists of macros' symbols, which have no
/// commas.)
pub fn escape_record_symbol_commas(obj: &mut serde_json::Value) {
    use itertools::Itertools;
    use serde_json::Value;
    fn escape(value: &mut Value) {
        if let Value::String(sym) = value
            && sym.contains(',')
        {
            *sym = escape_symbol_commas(sym).into_owned();
        }
    }
    fn walk(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, value) in map.iter_mut() {
                    let key = key.to_ascii_lowercase();
                    if key.ends_with("sym") {
                        escape(value);
                    } else if key.ends_with("syms") || key == "subclasses" || key == "overriddenby"
                    {
                        if let Value::Array(syms) = value {
                            syms.iter_mut().for_each(escape);
                        }
                    } else {
                        walk(value);
                    }
                }
            }
            Value::Array(values) => values.iter_mut().for_each(walk),
            _ => {}
        }
    }
    if obj.get("source").is_none() {
        walk(obj);
        return;
    }
    if let Some(Value::String(syms)) = obj.get_mut("sym")
        && syms.contains(',')
    {
        *syms = split_symbol_list(syms)
            .into_iter()
            .map(escape_symbol_commas)
            .join(",");
    }
    if let Some(typesym) = obj.get_mut("typesym") {
        escape(typesym);
    }
}

#[test]
fn test_escape_record_symbol_commas() {
    let mut source = serde_json::json!({"source": 1, "pretty": "type Pair<A, B>::type", "sym": "TA_Pair<A,_B>::type,_ZN3fooEv", "typesym": "T_Pair<A,_B>"});
    escape_record_symbol_commas(&mut source);
    assert_eq!(source["sym"], "TA_Pair<A@2C_B>::type,_ZN3fooEv");
    assert_eq!(source["typesym"], "T_Pair<A@2C_B>");
    let mut target = serde_json::json!({"target": 1, "pretty": "Pair<A, B>::type", "sym": "TA_Pair<A,_B>::type", "contextsym": "T_Pair<A,_B>"});
    escape_record_symbol_commas(&mut target);
    assert_eq!(target["sym"], "TA_Pair<A@2C_B>::type");
    assert_eq!(target["contextsym"], "T_Pair<A@2C_B>");
    let mut structured = serde_json::json!({"structured": 1, "sym": "T_Pair<A,_B>", "fields": [{"sym": "F_<T_Pair<A,_B>>_mFirst", "typesym": "T_X<C,_D>", "pointerInfo": [{"sym": "T_Y<E,_F>"}]}], "subclasses": ["T_Sub<G,_H>"], "ontologySlots": [{"syms": ["T_Z<I,_J>"]}]});
    escape_record_symbol_commas(&mut structured);
    assert_eq!(structured["sym"], "T_Pair<A@2C_B>");
    assert_eq!(structured["fields"][0]["sym"], "F_<T_Pair<A@2C_B>>_mFirst");
    assert_eq!(structured["fields"][0]["typesym"], "T_X<C@2C_D>");
    assert_eq!(structured["fields"][0]["pointerInfo"][0]["sym"], "T_Y<E@2C_F>");
    assert_eq!(structured["subclasses"][0], "T_Sub<G@2C_H>");
    assert_eq!(structured["ontologySlots"][0]["syms"][0], "T_Z<I@2C_J>");
}

// NOTE: The generated files can have different symbol than this.
//       Please consider using analysis::collect_file_syms_from_target or
//       analysis::collect_file_syms_from_source whenever possible.
pub fn make_file_sym_from_path(path: &str) -> String {
    format!("FILE_{}", mangle_file(path))
}

#[test]
fn test_mangle_file() {
    assert_eq!(mangle_file("path/foo.h"), "path/foo@2Eh");
    assert_eq!(make_file_sym_from_path("path/foo.h"), "FILE_path/foo@2Eh");
    assert_eq!(
        make_file_sym_from_path("subdir/header@with,many^strange~chars.h"),
        "FILE_subdir/header@40with@2Cmany@5Estrange@7Echars@2Eh"
    );
}

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
enum SplitState {
    // We're not in a template argument and the last character we processed is
    // not interesting.
    NotInArg,
    // We're not in a template argument and we've seen one colon and we expect
    // the next thing we see to be a colon completing a delimiter.
    NotInArgSawColon,
    // We're in a template argument, with the specific depth expressed by
    // `arg_depth`, and the last character we processed is not interesting.
    InArg,
    // We're in a template argument and the last character we saw was a "<".
    // If we see another "<" we know it's a left-shift because although there
    // are in-domain reasons for multiple consecutive ">" characters, there are
    // none for for "<" (other than left shift).
    InArgSawLT,
}

/// Pretty identifier segmentation that improves on naive splitting on "::",
/// and which relies on the provided symbol for some context for cases like
/// files.  It's not clear if this should also handle situations like Python
/// modules currently being "." delimited (although we then use "::" to combine
/// on top of that).
///
/// The primary motivation for this helper is to deal with cases like
/// `TemplatedClass<Foo::Bar>::Method` where the naive splitting will go wrong.
/// Additionally, we have to handle real world cases with bitshifts like
/// `Array<std::pair<uint8_t, uint8_t>, 1 << sizeof(AnonymousContentKey) * 8>`.
///
/// Note that although searchfox effectively understands JS-style "Foo.bar"
/// hierarchy, this is currently accomplished via `js-analyze.js` emitting 2
/// records: `{ pretty: "Foo", sym: "#Foo", ...}` and `{ pretty: "Foo.bar", sym:
/// "Foo#bar", ...}`.  This approach will likely be revisited when we move to
/// using LSIF/similar indexing, in which case this method will likely want to
/// become language aware and we would start only emitting a single record for
/// a single symbol.
///
/// ## Observed problems:
///
/// On LLVM:
/// - "In arg state with depth 1 when ran out of chars." seems to be happening
///   on "llvm::raw_ostream::operator<<".
pub fn split_pretty(pretty: &str, sym: &str) -> (Vec<String>, &'static str) {
    // Split files based on their path delimiter.  It would be too weird for us
    // to map them to using "::".  We're also now using split_inclusive so the
    // directories can have a distinct trailing slash to distinguish them from
    // actual pretty symbols.  (Not that we should have a heterogeneous diagram
    // that has files and symbols at the same time, but I think this will help
    // make the diagram more legible since it will be immediately clear what
    // we're dealing with.)
    if sym.starts_with("FILE_") {
        return (
            pretty.split_inclusive("/").map(|s| s.to_string()).collect(),
            "",
        );
    }

    // Web specs' terms are "<spec> <term>", where the term's "for"s (what it's
    // defined for) are "/"-separated (ex: "html Navigation/navigate(url,
    // options)"), so they split after the spec's name and each "/", with their
    // delimiters, like paths.
    if sym.starts_with("SPEC_") {
        let (spec, term) = match pretty.split_once(' ') {
            Some((spec, term)) => (Some(spec), term),
            None => (None, pretty),
        };
        let mut pieces: Vec<String> = spec.map(|spec| format!("{} ", spec)).into_iter().collect();
        pieces.extend(term.split_inclusive('/').map(|s| s.to_string()));
        return (pieces, "");
    }

    let mut pieces = vec![];
    let mut state = SplitState::NotInArg;
    let mut arg_depth = 0;
    let mut pretty_chars = pretty.chars();
    let mut token = String::new();
    loop {
        let next_c = pretty_chars.next();

        match (state, next_c) {
            (_, None) => {
                // (A trailing lone colon is part of the last piece.)
                if state == SplitState::NotInArgSawColon {
                    token.push(':');
                } else if state != SplitState::NotInArg {
                    info!(
                        "In arg state with depth {} when ran out of chars on {}.",
                        arg_depth, pretty,
                    );
                }
                if !token.is_empty() {
                    pieces.push(token);
                }
                break;
            }
            (SplitState::NotInArg, Some(':')) => {
                state = SplitState::NotInArgSawColon;
                // we will either end up eating this ":" if it's the first of a
                // pair, or put it back in if we see a different char in the
                // next state.
            }
            (SplitState::NotInArg, Some('<')) => {
                state = SplitState::InArg;
                arg_depth = 1;
                token.push('<');
            }
            (SplitState::NotInArg, Some('>')) => {
                // Don't warn on operator->, operator>, operator>=, operator>>, etc.
                // Also, we expect to get called on a bunch of garbage, so no warnings.
                if !token.starts_with("operator") {
                    info!("Saw '>' when not in an argument while parsing {}.", pretty);
                }
                token.push('>');
            }
            (SplitState::NotInArg, Some(c)) => {
                token.push(c);
            }
            (SplitState::NotInArgSawColon, Some(':')) => {
                pieces.push(token);
                token = String::new();
                state = SplitState::NotInArg;
            }
            (SplitState::NotInArgSawColon, Some(c)) => {
                // A lone colon, which is part of the piece, ex: in observer
                // topics ("AboutReader:Ready"), URLs, and things like
                // NSPRLogModulesParser_RawArg_Test::TestBody::(anonymous)::(lambda at /builds/worker/checkouts/gecko/xpcom/tests/gtest/TestNSPRLogModulesParser.cpp:130:19)
                // NameTableKey::(anonymous)::(unnamed union at /builds/worker/checkouts/gecko/xpcom/ds/nsStaticNameTable.cpp:33:3)
                token.push(':');
                token.push(c);
                if c == '<' {
                    state = SplitState::InArg;
                    arg_depth = 1;
                } else {
                    state = SplitState::NotInArg;
                }
            }
            (SplitState::InArg, Some('<')) => {
                token.push('<');
                state = SplitState::InArgSawLT;
            }
            (SplitState::InArg, Some('>')) => {
                token.push('>');
                arg_depth -= 1;
                if arg_depth == 0 {
                    state = SplitState::NotInArg;
                }
            }
            (SplitState::InArg, Some(c)) => {
                token.push(c);
            }
            (SplitState::InArgSawLT, Some('<')) => {
                // Okay, this was almost certainly a left-shift, so we don't
                // bump the depth.
                token.push('<');
                state = SplitState::InArg;
            }
            (SplitState::InArgSawLT, Some(c)) => {
                token.push(c);
                // Since we didn't see two in a row, then we probably increased
                // our depth.
                arg_depth += 1;
                state = SplitState::InArg;
            }
        }
    }

    (pieces, "::")
}

#[test]
fn test_split_pretty() {
    let ts = |vs: Vec<&str>| -> Vec<String> { vs.into_iter().map(|s| s.to_string()).collect() };

    assert_eq!(
        split_pretty("foo/bar/Baz.h", "FILE_foo_bar_Baz.h"),
        (ts(vec!["foo/", "bar/", "Baz.h"]), "")
    );

    assert_eq!(
        split_pretty("mozilla::dom::locks::LockRequest", "T_LockRequest"),
        (ts(vec!["mozilla", "dom", "locks", "LockRequest"]), "::")
    );

    assert_eq!(
        split_pretty("nsCOMPtr::operator->", "_ZNK8nsCOMPtrptEv&redirect=false"),
        (ts(vec!["nsCOMPtr", "operator->"]), "::")
    );

    assert_eq!(
        split_pretty(
            "Deserializer<mozilla::UniquePtr<char[], JS::FreePolicy>>::Read",
            "T_blah"
        ),
        (
            ts(vec![
                "Deserializer<mozilla::UniquePtr<char[], JS::FreePolicy>>",
                "Read"
            ]),
            "::"
        )
    );

    assert_eq!(
        split_pretty(
            "html Navigation/navigate(url, options)",
            "SPEC_html_dom@2Dnavigation@2Dnavigate"
        ),
        (
            ts(vec!["html ", "Navigation/", "navigate(url, options)"]),
            ""
        )
    );
    assert_eq!(
        split_pretty("html navigate", "SPEC_html_navigate"),
        (ts(vec!["html ", "navigate"]), "")
    );

    // Lone colons are part of their pieces.
    assert_eq!(
        split_pretty("AboutReader:Ready", "OBSTOPIC_AboutReader@3AReady"),
        (ts(vec!["AboutReader:Ready"]), "::")
    );
    assert_eq!(
        split_pretty(
            "Test::TestBody::(anonymous)::(lambda at a/Test.cpp:130:19)",
            "_ZZblah"
        ),
        (
            ts(vec![
                "Test",
                "TestBody",
                "(anonymous)",
                "(lambda at a/Test.cpp:130:19)"
            ]),
            "::"
        )
    );
    assert_eq!(
        split_pretty("a::b:", "T_blah"),
        (ts(vec!["a", "b:"]), "::")
    );
    assert_eq!(
        split_pretty("a:<b::c>::d", "T_blah"),
        (ts(vec!["a:<b::c>", "d"]), "::")
    );

    assert_eq!(
        split_pretty(
            "Array<std::pair<uint8_t, uint8_t>, 1 << sizeof(AnonymousContentKey) * 8>::DoStuff",
            "T_blah"
        ),
        (
            ts(vec![
                "Array<std::pair<uint8_t, uint8_t>, 1 << sizeof(AnonymousContentKey) * 8>",
                "DoStuff"
            ]),
            "::"
        )
    );
}
