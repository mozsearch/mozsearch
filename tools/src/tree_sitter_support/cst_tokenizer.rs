use std::borrow;
use std::path::Path;

use include_dir::{Dir, include_dir};

use crate::file_format::history::syntax_files::{TokenClass, format_token_line};
use crate::file_format::history::syntax_files_struct::FileStructureRow;

use tree_sitter::StreamingIterator as _;

static QUERIES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/languages/tokenizer_queries");

fn load_language_queries(
    ts_lang: &tree_sitter::Language,
    lang_str: &str,
) -> Result<tree_sitter::Query, String> {
    match QUERIES_DIR.get_file(format!("{}.scm", lang_str)) {
        Some(file) => {
            let maybe_contents = file.contents_utf8().map(borrow::Cow::from);
            match maybe_contents {
                Some(contents) => {
                    tree_sitter::Query::new(ts_lang, &contents).map_err(|ts_err| ts_err.message)
                }
                _ => Err(format!("No queries for lang: {}", lang_str)),
            }
        }
        _ => Err(format!("No queries for lang: {}", lang_str)),
    }
}

/// The tree-sitter grammar (or lack thereof) used to tokenize a language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grammar {
    Cpp,
    TypeScript,
    Tsx,
    Python,
    Rust,
    /// Whitespace-delimited words.
    PlainText,
}

/// A language we know how to tokenize for history purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguageProfile {
    /// Stable identifier recorded in "files-struct" headers and used as the
    /// value of the `searchfox-lang` history attribute.
    pub lang: &'static str,
    /// Languages in the same namespace are tokenized compatibly, so tokens can
    /// be inferred to move between files in different languages of the same
    /// namespace and their symbols share a symdex.  For example, C and C++
    /// headers and source files are all "cpp", and JS/TS with and without JSX
    /// are all "js".  If we supported both MarkDown and rST in a first-class
    /// fashion, it could make sense to put them in the same namespace.
    pub namespace: &'static str,
    grammar: Grammar,
}

/// All of the languages we can tokenize.
pub const LANGUAGE_PROFILES: &[LanguageProfile] = &[
    LanguageProfile {
        lang: "cpp",
        namespace: "cpp",
        grammar: Grammar::Cpp,
    },
    LanguageProfile {
        lang: "js",
        namespace: "js",
        grammar: Grammar::TypeScript,
    },
    LanguageProfile {
        lang: "jsx",
        namespace: "js",
        grammar: Grammar::Tsx,
    },
    LanguageProfile {
        lang: "py",
        namespace: "py",
        grammar: Grammar::Python,
    },
    LanguageProfile {
        lang: "rust",
        namespace: "rust",
        grammar: Grammar::Rust,
    },
    LanguageProfile {
        lang: "none",
        namespace: "none",
        grammar: Grammar::PlainText,
    },
];

/// The version of the tokenizer's output, recorded in "files-struct" headers.
/// This should be bumped whenever the tokens produced for the same input
/// change, which lets consumers of history know that a change in tokens may be
/// due to the tokenizer rather than the source.
pub const TOKENIZER_VERSION: u32 = 1;

pub fn profile_for_lang(lang: &str) -> Option<LanguageProfile> {
    LANGUAGE_PROFILES.iter().find(|p| p.lang == lang).copied()
}

/// The profile we use for a path in the absence of any overrides, or None for
/// binary files, which we can't tokenize.
pub fn default_profile_for_path(path: &Path) -> Option<LanguageProfile> {
    let ext = match path.extension() {
        Some(ext) => ext.to_str().unwrap_or(""),
        None => "",
    };
    let lang = match ext {
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hxx" | "hpp" => "cpp",
        "js" | "jsm" | "json" | "mjs" | "sjs" | "ts" => "js",
        "jsx" | "tsx" => "jsx",
        "py" | "build" | "configure" => "py",
        "rs" => "rust",
        // Explicitly skip things we know are binary; this list copied from
        // "languages.rs".
        "ogg" | "ttf" | "xpi" | "png" | "bcmap" | "gif" | "ogv" | "jpg" | "jpeg" | "bmp"
        | "icns" | "ico" | "mp4" | "sqlite" | "jar" | "webm" | "webp" | "woff" | "class"
        | "m4s" | "mgif" | "wav" | "opus" | "mp3" | "otf" | "car" => return None,
        _ => "none",
    };
    profile_for_lang(lang)
}

/// The namespace for a path in the absence of any overrides.  Prefer the
/// namespace recorded in the "files-struct" header when available since it
/// accounts for overrides.
pub fn namespace_for_file(path: &Path) -> &'static str {
    default_profile_for_path(path).map_or("", |p| p.namespace)
}

/// Words fixed by grammars which we want to treat as values (like `this` and
/// `self` which tree-sitter makes named nodes) rather than keywords for
/// consistency across grammars.  Ex: tree-sitter-cpp's `null` node has
/// anonymous `NULL`/`nullptr` children and tree-sitter-rust's `boolean_literal`
/// has anonymous `true`/`false` children, whereas in other grammars these are
/// named nodes.
const VALUE_WORDS: &[&str] = &[
    "true",
    "false",
    "null",
    "nullptr",
    "NULL",
    "None",
    "undefined",
];

/// Classify a tree-sitter leaf node for the `history/syntax/files`
/// representation.  See `TokenClass`.
///
/// The primary distinction is that anonymous nodes are strings fixed by the
/// grammar (keywords and punctuation) whereas named nodes have text chosen by
/// the author (identifiers and literals), with some adjustments for grammar
/// quirks.
fn classify_leaf(node: &tree_sitter::Node, token: &str, in_comment: bool) -> TokenClass {
    if in_comment {
        return TokenClass::Comment;
    }
    let has_alnum = token.chars().any(|c| c.is_alphanumeric());
    if node.is_error() {
        return if has_alnum {
            TokenClass::Identifier
        } else {
            TokenClass::Operator
        };
    }
    if !node.is_named() {
        return if !has_alnum {
            TokenClass::Operator
        } else if VALUE_WORDS.contains(&token) {
            TokenClass::Identifier
        } else {
            TokenClass::Keyword
        };
    }
    let kind = node.kind();
    if kind.contains("string")
        || kind.contains("char")
        || kind.contains("escape")
        || kind.contains("regex")
        || kind.contains("template")
    {
        TokenClass::String
    } else if kind.contains("number") || kind.contains("integer") || kind.contains("float") {
        TokenClass::Number
    } else if kind.ends_with("_specifier") || kind == "preproc_directive" {
        // Ex: tree-sitter-rust's `mutable_specifier` for `mut`, whereas `const`
        // is anonymous.
        TokenClass::Keyword
    } else if kind.contains("text") || kind == "preproc_arg" {
        TokenClass::Text
    } else {
        TokenClass::Identifier
    }
}

pub struct HyperTokenized {
    pub profile: LanguageProfile,
    pub tokenized: Vec<String>,
    pub structure: Vec<FileStructureRow>,
}

/// Process a source file with tree-sitter to derive the structurally-bound
/// syntax tokens and an outline of the structure of the file, using the default
/// language for the filename.
pub fn hypertokenize_source_file(
    filename: &str,
    source_contents: &str,
) -> Result<HyperTokenized, String> {
    match default_profile_for_path(Path::new(filename)) {
        Some(profile) => hypertokenize_with_profile(profile, source_contents),
        None => Err("Binary files can't be tokenized".to_string()),
    }
}

/// Process source contents with the given language profile.
pub fn hypertokenize_with_profile(
    profile: LanguageProfile,
    source_contents: &str,
) -> Result<HyperTokenized, String> {
    let mut tokenized = Vec::new();
    let mut structure = Vec::new();

    let mut parser = tree_sitter::Parser::new();
    // ### atom_nodes ###
    //
    // We borrow difftastic's terminology to deal with awkward tree-sitter nodes
    // like tree-sitter-cpp's `string_literal` where we want to use the contents
    // of the node and ignore the fact that it has children because there are
    // only children for the opening and closing `"` characters but no node for
    // the actual contents of the string.
    //
    // See https://github.com/tree-sitter/tree-sitter/issues/1156 for more
    // information on the underlying tree-sitter issue.
    //
    // Specific example details:
    // - `#include "big_header.h"` has 3 children:
    //   - `#include"`: 0 children
    //   - `"big_header.h"`: 2 children, both of which are the quotes?!  This
    //     differs from `<stdlib.h>` which is just a single monolithic string
    //     with no children.
    //   - `\n`: 0 children
    //
    // ### ignore_nodes
    //
    // As noted in https://github.com/tree-sitter/tree-sitter-c/issues/97 the
    // C preprocessor nodes currently are weird and include the trailing
    // newline.  For our purposes, we never actually want to emit a newline
    // token, so it's easy enough for us to just forbid that node.
    let (ts_lang, ts_query_filename, atom_nodes, ignore_nodes) = match profile.grammar {
        Grammar::Cpp => {
            let ts_lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
            let string_literal = ts_lang.id_for_node_kind("string_literal", true);
            let char_literal = ts_lang.id_for_node_kind("char_literal", true);
            let newline = ts_lang.id_for_node_kind("\n", false);
            (
                ts_lang,
                "cpp",
                vec![string_literal, char_literal],
                vec![newline],
            )
        }
        Grammar::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            vec![],
            vec![],
        ),
        Grammar::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "typescript",
            vec![],
            vec![],
        ),
        Grammar::Python => (
            tree_sitter_python::LANGUAGE.into(),
            "python",
            vec![],
            vec![],
        ),
        Grammar::Rust => (tree_sitter_rust::LANGUAGE.into(), "rust", vec![], vec![]),
        Grammar::PlainText => {
            return Ok(HyperTokenized {
                profile,
                tokenized: source_contents
                    .split_whitespace()
                    .map(|s| format_token_line("%", TokenClass::Text, s))
                    .collect(),
                structure: vec![],
            });
        }
    };
    parser
        .set_language(&ts_lang)
        .expect("Error loading grammar");
    let container_query = load_language_queries(&ts_lang, ts_query_filename)?;

    let name_capture_ix = container_query.capture_index_for_name("name").unwrap();
    let container_capture_ix = container_query.capture_index_for_name("container").unwrap();

    let parse_tree = match parser.parse(source_contents.as_bytes(), None) {
        Some(t) => t,
        _ => {
            return Err("Parse failed!".to_string());
        }
    };

    // The cursor traversal logic here is derived from the tree-sitter-cli
    // parse_file_at_path logic: https://github.com/tree-sitter/tree-sitter/blob/master/cli/src/parse.rs
    //
    // A good resource if you are interested in what this class is doing is to instead look at
    // https://github.com/Wilfred/difftastic/blob/master/src/parse/tree_sitter_parser.rs which
    // I discovered after running into problems with the node modeling of tree-sitter-cpp's
    // `string_literal` node and found https://github.com/tree-sitter/tree-sitter/issues/1156
    // and related issues and discussion.  Note that it is explicitly mapping tree-sitter's
    // pseudo-CST to its own tree rep, whereas we are just linearizing tokens here, but the
    // general desire to have all tokens remains.
    let mut cursor = parse_tree.walk();
    let mut _depth = 0;
    let mut visited_children = false;
    // Tracks whether the node whose children we are currently visiting is (or
    // is inside of) an "extra" node, which is how comments are represented.
    // We need this because comments can have children which aren't themselves
    // extra, like tree-sitter-rust's `doc_comment`.
    let mut in_comment_stack: Vec<bool> = vec![];

    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut query_matches = query_cursor.matches(
        &container_query,
        parse_tree.root_node(),
        source_contents.as_bytes(),
    );

    let mut next_container_match = query_matches.next();
    let mut next_container_id = usize::MAX;
    if let Some(container_match) = &next_container_match {
        next_container_id = container_match
            .nodes_for_capture_index(container_capture_ix)
            .next()
            .unwrap()
            .id();
    }

    let mut context_stack: Vec<String> = vec![];
    let empty_context = "%".to_string();
    let mut context_pretty = empty_context.clone();
    let mut id_stack: Vec<usize> = vec![];

    loop {
        let node = cursor.node();
        if visited_children {
            if cursor.goto_next_sibling() {
                visited_children = false;
            } else if cursor.goto_parent() {
                visited_children = true;
                _depth -= 1;
                in_comment_stack.pop();

                if let Some(container_id) = id_stack.last()
                    && cursor.node().id() == *container_id
                {
                    context_stack.pop();
                    context_pretty = if context_stack.is_empty() {
                        empty_context.clone()
                    } else {
                        context_stack.join("::")
                    };
                    id_stack.pop();
                }
            } else {
                break;
            }
        } else {
            // We are considering this node for the first time and before any of
            // its children.

            // Handle if this is our next container.
            if node.id() == next_container_id {
                let pattern_index = next_container_match.as_ref().unwrap().pattern_index;
                let name_node = next_container_match
                    .as_ref()
                    .unwrap()
                    .nodes_for_capture_index(name_capture_ix)
                    .next()
                    .unwrap();
                let name = name_node.utf8_text(source_contents.as_bytes()).unwrap();
                context_stack.push(name.to_string());
                context_pretty = if context_stack.is_empty() {
                    empty_context.clone()
                } else {
                    context_stack.join("::")
                };
                // We're assuming there's only one `#set!` directive right now and that it's
                // "structure.kind" and that it exists.  We do require it to exist, but...
                // TODO: It likely makes sense to preprocess the query by iterating over
                // its patterns and explicitly mapping based on the key so that we can
                // have the kind already available as a string we can clone.
                let structure_kind = container_query
                    .property_settings(pattern_index)
                    .first()
                    .unwrap()
                    .value
                    .as_ref()
                    .unwrap()
                    .to_string();
                structure.push(FileStructureRow {
                    pretty: context_pretty.clone(),
                    // TODO: This should come from a `#set!` directive too but this nuance
                    // won't matter for a bit, so I'm punting because there's a potential
                    // the SCM queries would need to get a little more complex in order to
                    // differentiate between decl and def and when making the change it
                    // would probably be ideal to add more test coverage.
                    is_def: true,
                    kind: structure_kind.to_string(),
                });
                id_stack.push(next_container_id);

                next_container_match = query_matches.next();
                if let Some(container_match) = &next_container_match {
                    next_container_id = container_match
                        .nodes_for_capture_index(container_capture_ix)
                        .next()
                        .unwrap()
                        .id();
                } else {
                    next_container_id = usize::MAX;
                }
            }
            let node_kind_id = node.kind_id();
            let in_comment = node.is_extra() || in_comment_stack.last() == Some(&true);
            if ignore_nodes.contains(&node_kind_id) {
                // ignore this node!
                visited_children = true;
            } else if !atom_nodes.contains(&node_kind_id) && cursor.goto_first_child() {
                visited_children = false;
                _depth += 1;
                in_comment_stack.push(in_comment);
            } else {
                let token = node.utf8_text(source_contents.as_bytes()).unwrap().trim();
                let class = classify_leaf(&node, token, in_comment);
                // Comments don't get further tokenized by tree-sitter, so we perform
                // additional whitespace tokenization for comments.
                //
                // We also perform whitespace tokenization for any token that contains a newline
                // (ex: multi-line string literals) because our output format is one token per
                // line.
                if token.is_empty() {
                    // ignore empty tokens!
                } else if in_comment || token.contains('\n') {
                    // TODO: probably better to use the regex crate here to avoid a bunch of empty
                    // matches for consecutive whitespace.
                    for piece in token.split(char::is_whitespace) {
                        if piece.is_empty() {
                            continue;
                        }
                        tokenized.push(format_token_line(&context_pretty, class, piece));
                    }
                } else {
                    tokenized.push(format_token_line(&context_pretty, class, token));
                }
                visited_children = true;
            }
        }
    }

    Ok(HyperTokenized {
        profile,
        tokenized,
        structure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::syntax_files::split_token_line;

    /// Tree-sitter grammar upgrades can rename node types, which makes our
    /// queries fail to compile, which makes every file in that language fail to
    /// tokenize.  So make sure every language can tokenize something.
    #[test]
    fn test_all_languages_tokenize() {
        for (filename, source, lang) in [
            (
                "a.cpp",
                "namespace ns { int Foo::bar() { return 1; } }",
                "cpp",
            ),
            (
                "a.js",
                "class C { m() {} } function f(a) { return a > 1; }",
                "js",
            ),
            ("a.tsx", "const f = (a: number) => <div>{a}</div>;", "jsx"),
            (
                "a.py",
                "class C:\n    def m(self):\n        return 1\n",
                "py",
            ),
            ("a.rs", "impl Foo { fn bar() -> u32 { 1 } }", "rust"),
            ("a.txt", "just some words", "none"),
        ] {
            let tokenized = hypertokenize_source_file(filename, source)
                .unwrap_or_else(|e| panic!("{} failed to tokenize: {}", filename, e));
            assert_eq!(tokenized.profile.lang, lang);
            assert!(!tokenized.tokenized.is_empty(), "{}", filename);
            if lang != "none" {
                assert!(
                    !tokenized.structure.is_empty(),
                    "{} has no structure",
                    filename
                );
            }
        }
    }

    fn classes(filename: &str, source: &str) -> Vec<String> {
        hypertokenize_source_file(filename, source)
            .unwrap()
            .tokenized
            .iter()
            .map(|line| {
                let parsed = split_token_line(line);
                format!("{}:{}", parsed.class.as_char(), parsed.token)
            })
            .collect()
    }

    #[test]
    fn test_token_classes() {
        assert_eq!(
            classes(
                "a.cpp",
                "// Hi there\nint* f() { if (x > 1) return nullptr; return \"s\"; }"
            ),
            vec![
                "c://",
                "c:Hi",
                "c:there",
                "i:int",
                "o:*",
                "i:f",
                "o:(",
                "o:)",
                "o:{",
                "k:if",
                "o:(",
                "i:x",
                "o:>",
                "n:1",
                "o:)",
                "k:return",
                "i:nullptr",
                "o:;",
                "k:return",
                "s:\"s\"",
                "o:;",
                "o:}"
            ]
        );
        assert_eq!(
            classes(
                "a.rs",
                "/// Doc comment.\nfn f(&mut self) -> bool { let p: *const u8; true }"
            ),
            vec![
                "c://",
                "c:/",
                "c:Doc",
                "c:comment.",
                "k:fn",
                "i:f",
                "o:(",
                "o:&",
                "k:mut",
                "i:self",
                "o:)",
                "o:->",
                "i:bool",
                "o:{",
                "k:let",
                "i:p",
                "o::",
                "o:*",
                "k:const",
                "i:u8",
                "o:;",
                "i:true",
                "o:}"
            ]
        );
        assert_eq!(
            classes("a.js", "var x = this.y || null; // done"),
            vec![
                "k:var", "i:x", "o:=", "i:this", "o:.", "i:y", "o:||", "i:null", "o:;", "c://",
                "c:done"
            ]
        );
        assert_eq!(classes("a.txt", "plain words"), vec!["t:plain", "t:words"]);
    }
}
