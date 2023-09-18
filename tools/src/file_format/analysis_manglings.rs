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
        return (pretty.split_inclusive("/").map(|s| s.to_string()).collect(), "");
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
                if state != SplitState::NotInArg {
                    warn!(
                        "In arg state with depth {} when ran out of chars.",
                        arg_depth
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
                warn!("Saw '>' when not in an argument while parsing {}.", pretty);
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
                warn!(
                    "Saw a single colon when not in arg while parsing {}",
                    pretty
                );
                token.push(')');
                token.push(c);
                state = SplitState::NotInArg;
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
        (ts(vec!["foo", "bar", "Baz.h"]), "/")
    );

    assert_eq!(
        split_pretty("mozilla::dom::locks::LockRequest", "T_LockRequest"),
        (ts(vec!["mozilla", "dom", "locks", "LockRequest"]), "::")
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
