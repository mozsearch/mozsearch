use std::borrow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use include_dir::{Dir, include_dir};

use crate::file_format::history::syntax_files::TokenClass;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::tree_sitter_support::boilerplate::{FinishedTokens, RawToken, finish_tokens};
use crate::tree_sitter_support::config_tokenizer::{tokenize_ini, tokenize_toml};

use tree_sitter::StreamingIterator as _;

static QUERIES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/languages/tokenizer_queries");

/// Compiled container queries by grammar.  Compiling a query takes tens of
/// milliseconds, which dominated the time to tokenize small files when we did it
/// for every file.  (A query is only valid for the grammar it was compiled for,
/// and some grammars share query files.)
static CONTAINER_QUERIES: LazyLock<Mutex<HashMap<Grammar, Arc<tree_sitter::Query>>>> =
    LazyLock::new(Default::default);

fn container_query(
    grammar: Grammar,
    ts_lang: &tree_sitter::Language,
    lang_str: &str,
) -> Result<Arc<tree_sitter::Query>, String> {
    if let Some(query) = CONTAINER_QUERIES.lock().unwrap().get(&grammar) {
        return Ok(query.clone());
    }
    // Compile without holding the lock; racing threads just compile twice.
    let query = Arc::new(load_language_queries(ts_lang, lang_str)?);
    Ok(CONTAINER_QUERIES
        .lock()
        .unwrap()
        .entry(grammar)
        .or_insert(query)
        .clone())
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Grammar {
    Cpp,
    TypeScript,
    Tsx,
    Python,
    Rust,
    Webidl,
    Ipdl,
    /// See `config_tokenizer.rs`.
    Ini,
    /// See `config_tokenizer.rs`.
    Toml,
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
        lang: "webidl",
        namespace: "webidl",
        grammar: Grammar::Webidl,
    },
    LanguageProfile {
        lang: "ipdl",
        namespace: "ipdl",
        grammar: Grammar::Ipdl,
    },
    // INI and TOML share a namespace because they are tokenized equivalently so
    // that history can follow Firefox's conversion of manifests from .ini to
    // .toml.
    LanguageProfile {
        lang: "ini",
        namespace: "config",
        grammar: Grammar::Ini,
    },
    LanguageProfile {
        lang: "toml",
        namespace: "config",
        grammar: Grammar::Toml,
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
///
/// - 2: WebIDL and IPDL grammars; `TokenClass::Text` leaves (ex: C++ `#define`
///   arguments) are split on whitespace; named leaves without alphanumeric
///   characters are operators.
/// - 3: INI and TOML tokenizers.
/// - 4: License headers and modelines are `TokenClass::Boilerplate`.
/// - 5: Whitespace in structural context names is normalized; see
///   `context_name`.
/// - 6: Names with parse errors are the innermost names without any, or don't
///   make containers (see `clean_name_node`), and C++ calls of statement macros
///   which tree-sitter-cpp takes for function definitions (ex: `QM_TRY_UNWRAP`
///   with a lambda) aren't containers (see cpp.scm).
/// - 7: INI and TOML top-level section names which are URLs with a query or
///   fragment are split into tokens and context segments for them; see
///   `config_tokenizer::UrlParts`.
/// - 8: INI nested sections' names are words and `;`s rather than a single
///   token; see `config_tokenizer::Output::push_name_words`.
pub const TOKENIZER_VERSION: u32 = 8;

/// Normalize the text of a container's name node for use in a context.  Names
/// can contain whitespace (ex: C++ template arguments in out-of-line method
/// definitions, possibly spanning lines), but contexts can't contain whitespace
/// (see `syntax_files.rs`).  So we drop whitespace except between two word
/// characters, where it becomes an escaped space ("%20", with "%" escaped as
/// "%25" like `config_tokenizer` does).  This also makes contexts insensitive
/// to reformatting, ex: `Foo<A, B>` and `Foo<A,B>` are the same.
/// The node to name a container by, given the node its query captured as its
/// name.  tree-sitter's error recovery can make that span unrecognized macros
/// and the comments between them, ex: tree-sitter-cpp's name for
/// `ALWAYS_INLINE ATTRIBUTE_NO_SANITIZE_ALL void TracePC::HandleCmp(...)` is
/// all of it before the parameters, with ERROR nodes in its scope.  So for a
/// name with errors, we use the innermost node along its `name` fields without
/// any (`HandleCmp`), or if there isn't one, it doesn't name a container.
fn clean_name_node(mut name: tree_sitter::Node) -> Option<tree_sitter::Node> {
    while name.has_error() {
        name = name.child_by_field_name("name")?;
    }
    Some(name)
}

fn context_name(name: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut out = String::with_capacity(name.len());
    let mut pending_space = false;
    for c in name.chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && out.chars().last().is_some_and(is_word) && is_word(c) {
            out.push_str("%20");
        }
        pending_space = false;
        if c == '%' {
            out.push_str("%25");
        } else {
            out.push(c);
        }
    }
    out
}

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
        "webidl" => "webidl",
        "ipdl" | "ipdlh" => "ipdl",
        "ini" => "ini",
        "toml" => "toml",
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
/// Per-grammar adjustments to `classify_leaf` for cases the general rules get
/// wrong.
#[derive(Default)]
struct ClassQuirks {
    /// Classes for named leaf node kinds, ex: tree-sitter-webidl's
    /// `or_keyword`.
    named_kinds: Vec<(u16, TokenClass)>,
    /// Classes for anonymous leaves with the given parent node kinds.  Ex:
    /// tree-sitter-ipdl's builtin type names like `nsString` and `uint32_t` are
    /// anonymous children of `type_name`, but they are type names like C++'s
    /// named `primitive_type` nodes, so we want them to be identifiers rather
    /// than keywords.
    parent_kinds: Vec<(u16, TokenClass)>,
}

impl ClassQuirks {
    fn new(
        ts_lang: &tree_sitter::Language,
        named_kinds: &[(&str, TokenClass)],
        parent_kinds: &[(&str, TokenClass)],
    ) -> Self {
        // Unknown kinds (ex: renamed by a grammar update) resolve to 0 and are
        // dropped; `test_token_classes` should catch that.
        let resolve = |kinds: &[(&str, TokenClass)]| {
            kinds
                .iter()
                .map(|(kind, class)| (ts_lang.id_for_node_kind(kind, true), *class))
                .filter(|(id, _)| *id != 0)
                .collect()
        };
        ClassQuirks {
            named_kinds: resolve(named_kinds),
            parent_kinds: resolve(parent_kinds),
        }
    }

    fn lookup(list: &[(u16, TokenClass)], kind_id: u16) -> Option<TokenClass> {
        list.iter()
            .find(|(id, _)| *id == kind_id)
            .map(|(_, class)| *class)
    }
}

fn classify_leaf(
    node: &tree_sitter::Node,
    token: &str,
    in_comment: bool,
    parent_kind_id: Option<u16>,
    quirks: &ClassQuirks,
) -> TokenClass {
    if in_comment {
        return TokenClass::Comment;
    }
    let has_alnum = token.chars().any(|c| c.is_alphanumeric());
    if node.is_named()
        && let Some(class) = ClassQuirks::lookup(&quirks.named_kinds, node.kind_id())
    {
        return class;
    }
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
        } else if let Some(class) =
            parent_kind_id.and_then(|parent| ClassQuirks::lookup(&quirks.parent_kinds, parent))
        {
            class
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
    } else if !has_alnum {
        // Ex: tree-sitter-ipdl's `nullable_suffix` for `?`.
        TokenClass::Operator
    } else {
        TokenClass::Identifier
    }
}

pub struct HyperTokenized {
    pub profile: LanguageProfile,
    /// The `history/syntax/files` lines for the tokens.
    pub tokenized: Vec<String>,
    /// The byte offset of each token in the source; see
    /// `FinishedTokens::offsets`.
    pub offsets: Vec<Option<u32>>,
    pub structure: Vec<FileStructureRow>,
}

impl HyperTokenized {
    fn new(
        profile: LanguageProfile,
        tokens: FinishedTokens,
        structure: Vec<FileStructureRow>,
    ) -> Self {
        HyperTokenized {
            profile,
            tokenized: tokens.lines,
            offsets: tokens.offsets,
            structure,
        }
    }
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
    //
    // ### quirks
    //
    // See `ClassQuirks`.
    let (ts_lang, ts_query_filename, atom_nodes, ignore_nodes, quirks) = match profile.grammar {
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
                ClassQuirks::default(),
            )
        }
        Grammar::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            vec![],
            vec![],
            ClassQuirks::default(),
        ),
        Grammar::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "typescript",
            vec![],
            vec![],
            ClassQuirks::default(),
        ),
        Grammar::Python => (
            tree_sitter_python::LANGUAGE.into(),
            "python",
            vec![],
            vec![],
            ClassQuirks::default(),
        ),
        Grammar::Rust => (
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            vec![],
            vec![],
            ClassQuirks::default(),
        ),
        Grammar::Webidl => {
            let ts_lang: tree_sitter::Language = tree_sitter_webidl::LANGUAGE.into();
            let quirks = ClassQuirks::new(
                &ts_lang,
                &[("or_keyword", TokenClass::Keyword)],
                &[
                    // Builtin types like `long`, `DOMString`, `Promise`,
                    // `sequence`, and `Uint8Array` are anonymous.
                    ("primitive_type", TokenClass::Identifier),
                    ("string_type", TokenClass::Identifier),
                    ("type_base", TokenClass::Identifier),
                    ("array_type", TokenClass::Identifier),
                    // `Infinity`, `-Infinity`, and `NaN`.
                    ("float_literal", TokenClass::Number),
                ],
            );
            (ts_lang, "webidl", vec![], vec![], quirks)
        }
        Grammar::Ipdl => {
            let ts_lang: tree_sitter::Language = tree_sitter_ipdl::LANGUAGE.into();
            let quirks = ClassQuirks::new(
                &ts_lang,
                // These are entire lines like `#ifdef MOZ_ENABLE_SKIA`, which we
                // split into words like other text.
                &[("preprocessor_directive", TokenClass::Text)],
                // Builtin types like `nsString`, `uint32_t`, and `Endpoint` are
                // anonymous.
                &[("type_name", TokenClass::Identifier)],
            );
            (ts_lang, "ipdl", vec![], vec![], quirks)
        }
        Grammar::Ini | Grammar::Toml => {
            let (tokens, structure) = match profile.grammar {
                Grammar::Ini => tokenize_ini(source_contents),
                _ => tokenize_toml(source_contents),
            };
            return Ok(HyperTokenized::new(profile, tokens, structure));
        }
        Grammar::PlainText => {
            let tokens = source_contents
                .split_whitespace()
                .map(|text| RawToken {
                    context: "%".to_string(),
                    class: TokenClass::Text,
                    text,
                })
                .collect();
            return Ok(HyperTokenized::new(
                profile,
                finish_tokens(source_contents, tokens),
                vec![],
            ));
        }
    };
    parser
        .set_language(&ts_lang)
        .expect("Error loading grammar");
    let container_query = container_query(profile.grammar, &ts_lang, ts_query_filename)?;

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
    // For each node whose children we are currently visiting, its kind and
    // whether it is (or is inside of) an "extra" node, which is how comments
    // are represented.  We need the latter because comments can have children
    // which aren't themselves extra, like tree-sitter-rust's `doc_comment`.
    let mut parent_stack: Vec<(u16, bool)> = vec![];

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
                parent_stack.pop();

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
                if let Some(name_node) = clean_name_node(name_node) {
                    let name = name_node.utf8_text(source_contents.as_bytes()).unwrap();
                    context_stack.push(context_name(name));
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
                }

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
            // Grammars conventionally mark comments as "extra" nodes, but not all
            // of them do (ex: tree-sitter-webidl), so we also go by the name.
            let in_comment = node.is_extra()
                || (node.is_named() && node.kind().contains("comment"))
                || parent_stack.last().is_some_and(|p| p.1);
            if ignore_nodes.contains(&node_kind_id) {
                // ignore this node!
                visited_children = true;
            } else if !atom_nodes.contains(&node_kind_id) && cursor.goto_first_child() {
                visited_children = false;
                _depth += 1;
                parent_stack.push((node_kind_id, in_comment));
            } else {
                let token = node.utf8_text(source_contents.as_bytes()).unwrap().trim();
                let parent_kind_id = parent_stack.last().map(|p| p.0);
                let class = classify_leaf(&node, token, in_comment, parent_kind_id, &quirks);
                // Comments and text don't get further tokenized by tree-sitter, so we
                // perform additional whitespace tokenization for them.
                //
                // We also perform whitespace tokenization for any token that contains a newline
                // (ex: multi-line string literals) because our output format is one token per
                // line.
                if token.is_empty() {
                    // ignore empty tokens!
                } else if in_comment || class == TokenClass::Text || token.contains('\n') {
                    // TODO: probably better to use the regex crate here to avoid a bunch of empty
                    // matches for consecutive whitespace.
                    for piece in token.split(char::is_whitespace) {
                        if piece.is_empty() {
                            continue;
                        }
                        tokenized.push(RawToken {
                            context: context_pretty.clone(),
                            class,
                            text: piece,
                        });
                    }
                } else {
                    tokenized.push(RawToken {
                        context: context_pretty.clone(),
                        class,
                        text: token,
                    });
                }
                visited_children = true;
            }
        }
    }

    Ok(HyperTokenized::new(
        profile,
        finish_tokens(source_contents, tokenized),
        structure,
    ))
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
            (
                "a.webidl",
                "interface Foo { readonly attribute long x; undefined go(); };",
                "webidl",
            ),
            (
                "a.ipdl",
                "namespace mozilla { protocol PFoo { parent: async Go(); }; }",
                "ipdl",
            ),
            ("a.ini", "[test.html]\nskip-if = os == 'win'", "ini"),
            (
                "a.toml",
                "[\"test.html\"]\nskip-if = [\"os == 'win'\"]",
                "toml",
            ),
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
            // Token blame relies on every token having its source offset.
            for (line, offset) in tokenized.tokenized.iter().zip(&tokenized.offsets) {
                let token = split_token_line(line).token;
                let offset = offset.unwrap_or_else(|| panic!("{} {:?}", filename, line)) as usize;
                assert_eq!(source.get(offset..offset + token.len()), Some(token));
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

    /// Boilerplate is marked through every tokenizer, which also checks that
    /// line starts are recovered for the line-based rules.
    #[test]
    fn test_boilerplate_classes() {
        let unmarked = |filename: &str, source: &str| -> Vec<String> {
            let all = classes(filename, source);
            assert!(all.iter().any(|c| c.starts_with("b:")), "{}", filename);
            all.into_iter().filter(|c| !c.starts_with("b:")).collect()
        };
        assert_eq!(
            unmarked(
                "a.cpp",
                "/* -*- Mode: C++; tab-width: 2 -*- */\n\
                 /* This Source Code Form is subject to the terms of the Mozilla Public\n \
                  * License, v. 2.0. If a copy of the MPL was not distributed with this\n \
                  * file, You can obtain one at http://mozilla.org/MPL/2.0/. */\n\
                 /* Design notes. */\n\
                 int x;"
            ),
            vec![
                "c:/*", "c:Design", "c:notes.", "c:*/", "i:int", "i:x", "o:;"
            ]
        );
        assert_eq!(
            unmarked(
                "a.js",
                "/**\n * @license\n * Copyright 2017 Google Inc.\n * SPDX-License-Identifier: Apache-2.0\n */\n\
                 // Keep me\nlet copyright = 2017;"
            ),
            vec![
                "c:/**",
                "c:*/",
                "c://",
                "c:Keep",
                "c:me",
                "k:let",
                "i:copyright",
                "o:=",
                "n:2017",
                "o:;"
            ]
        );
        assert_eq!(
            unmarked(
                "a.toml",
                "# Any copyright is dedicated to the Public Domain.\n\
                 # http://creativecommons.org/publicdomain/zero/1.0/\n\
                 # Keep\n[a]"
            ),
            vec!["c:#", "c:Keep", "o:[", "i:a", "o:]"]
        );
        // Boilerplate right after an INI section header.
        assert_eq!(
            unmarked("a.ini", "[a]\n# Copyright 2020 Foo\n# Keep\nx = 1"),
            vec!["o:[", "i:a", "o:]", "c:#", "c:Keep", "i:x", "o:=", "t:1"]
        );
        assert_eq!(
            unmarked("a.txt", "vim: set ts=2 et:\nplain words"),
            vec!["t:plain", "t:words"]
        );
    }

    #[test]
    fn test_context_names() {
        assert_eq!(
            context_name("MapField<Derived, Key,\n              int>::SyncMap"),
            "MapField<Derived,Key,int>::SyncMap"
        );
        assert_eq!(context_name("operator new"), "operator%20new");
        assert_eq!(context_name("Foo<unsigned  int>"), "Foo<unsigned%20int>");
        assert_eq!(context_name("100%"), "100%25");
        // End to end: contexts in token lines never contain whitespace.
        let tokenized = hypertokenize_source_file(
            "a.cpp",
            "template <typename D, typename K>\nvoid MapField<D, K,\n  int>::Sync() const {\n  mX = 1;\n}\n",
        )
        .unwrap();
        assert!(
            tokenized
                .tokenized
                .iter()
                .any(|l| l.starts_with("MapField<D,K,int>::Sync i mX")),
            "{:?}",
            tokenized.tokenized
        );
        for line in &tokenized.tokenized {
            assert_eq!(line.split(' ').count(), 3, "{:?}", line);
        }
        assert_eq!(tokenized.structure[0].pretty, "MapField<D,K,int>::Sync");
    }

    #[test]
    fn test_misparsed_cpp_definitions() {
        let prettys = |source: &str| -> Vec<String> {
            hypertokenize_source_file("a.cpp", source)
                .unwrap()
                .structure
                .into_iter()
                .map(|row| row.pretty)
                .collect()
        };
        // tree-sitter-cpp takes a statement macro with a lambda argument (here
        // when another follows it) for a function definition named by the
        // macro call, whose tokens belong to the enclosing function.  (From
        // dom/quota/QuotaManagerService.cpp.)
        assert_eq!(
            prettys(
                "NS_IMETHODIMP\nQuotaManagerService::TemporaryOriginInitialized(\n    const nsACString& aPersistenceType, nsIPrincipal* aPrincipal,\n    nsIQuotaRequest** _retval) {\n  MOZ_ASSERT(NS_IsMainThread());\n  MOZ_ASSERT(aPrincipal);\n  MOZ_ASSERT(nsContentUtils::IsCallerChrome());\n\n  QM_TRY(MOZ_TO_RESULT(StaticPrefs::dom_quotaManager_testing()),\n         NS_ERROR_UNEXPECTED);\n\n  QM_TRY(MOZ_TO_RESULT(EnsureBackgroundActor()));\n\n  QM_TRY_INSPECT(\n      const auto& persistenceType,\n      ([&aPersistenceType]() -> Result<PersistenceType, nsresult> {\n        const auto persistenceType =\n            PersistenceTypeFromString(aPersistenceType, fallible);\n        QM_TRY(MOZ_TO_RESULT(persistenceType.isSome()),\n               Err(NS_ERROR_INVALID_ARG));\n\n        QM_TRY(\n            MOZ_TO_RESULT(IsBestEffortPersistenceType(persistenceType.ref())),\n            Err(NS_ERROR_INVALID_ARG));\n\n        return persistenceType.ref();\n      }()));\n\n  QM_TRY_INSPECT(const auto& principalInfo,\n                 ([&aPrincipal]() -> Result<PrincipalInfo, nsresult> {\n                   PrincipalInfo principalInfo;\n                   QM_TRY(MOZ_TO_RESULT(\n                       PrincipalToPrincipalInfo(aPrincipal, &principalInfo)));\n\n                   QM_TRY(MOZ_TO_RESULT(IsPrincipalInfoValid(principalInfo)),\n                          Err(NS_ERROR_INVALID_ARG));\n\n                   return principalInfo;\n                 }()));\n\n  RefPtr<Request> request = new Request();\n\n  mBackgroundActor\n      ->SendTemporaryOriginInitialized(persistenceType, principalInfo)\n      ->Then(GetCurrentSerialEventTarget(), __func__,\n             BoolResponsePromiseResolveOrRejectCallback(request));\n\n  request.forget(_retval);\n  return NS_OK;\n}\n"
            ),
            vec!["QuotaManagerService::TemporaryOriginInitialized"]
        );
        // Macros it doesn't recognize before a definition, and comments after
        // them, end up in the definition's name, along with ERROR nodes.
        assert_eq!(
            prettys(
                "U_CDECL_BEGIN\nstatic char16_t U_CALLCONV\nCharAt(int32_t offset, void *context) {\n    return 0;\n}\nU_CDECL_END\n\nU_NAMESPACE_BEGIN\n\n/* The Replaceable virtual destructor can't be defined in the header\n   due to how AIX works. */\nReplaceable::~Replaceable() {}\n"
            ),
            vec!["CharAt", "Replaceable::~Replaceable"]
        );
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
        assert_eq!(
            classes(
                "a.webidl",
                "// A comment\ninterface Foo { readonly attribute (DOMString or long)? x; undefined go(optional float y = NaN); };"
            ),
            vec![
                "c://",
                "c:A",
                "c:comment",
                "k:interface",
                "i:Foo",
                "o:{",
                "k:readonly",
                "k:attribute",
                "o:(",
                "i:DOMString",
                "k:or",
                "i:long",
                "o:)",
                "o:?",
                "i:x",
                "o:;",
                "i:undefined",
                "i:go",
                "o:(",
                "k:optional",
                "i:float",
                "i:y",
                "o:=",
                "n:NaN",
                "o:)",
                "o:;",
                "o:}",
                "o:;"
            ]
        );
        assert_eq!(
            classes(
                "a.ipdl",
                "#ifdef MOZ_FOO\nprotocol PFoo { parent: async Go(nsString s, FooId? id); };\n#endif"
            ),
            vec![
                "t:#ifdef",
                "t:MOZ_FOO",
                "k:protocol",
                "i:PFoo",
                "o:{",
                "k:parent",
                "o::",
                "k:async",
                "i:Go",
                "o:(",
                "i:nsString",
                "i:s",
                "o:,",
                "i:FooId",
                "o:?",
                "i:id",
                "o:)",
                "o:;",
                "o:}",
                "o:;",
                "k:#endif"
            ]
        );
    }

    fn structure(filename: &str, source: &str) -> Vec<String> {
        hypertokenize_source_file(filename, source)
            .unwrap()
            .structure
            .iter()
            .map(|row| format!("{}:{}", row.kind, row.pretty))
            .collect()
    }

    #[test]
    fn test_webidl_ipdl_structure() {
        assert_eq!(
            structure(
                "a.webidl",
                "interface Foo { constructor(); readonly attribute long x; undefined go(); \
                 const long K = 1; };\n\
                 partial interface Foo { undefined more(); };\n\
                 dictionary FooInit { long count; };\n\
                 enum FooMode { \"a\" };\n\
                 callback FooCallback = undefined (long x);"
            ),
            vec![
                "class:Foo",
                "method:Foo::constructor",
                "field:Foo::x",
                "method:Foo::go",
                "field:Foo::K",
                "class:Foo",
                "method:Foo::more",
                "struct:FooInit",
                "field:FooInit::count",
                "enum:FooMode",
                "function:FooCallback",
            ]
        );
        assert_eq!(
            structure(
                "a.ipdl",
                "namespace mozilla { namespace dom {\n\
                 struct FooArgs { nsString name; };\n\
                 union FooResult { nsresult; FooArgs; };\n\
                 protocol PFoo { manager PBackground; parent: async Start(FooArgs a) returns (bool ok); };\n\
                 } }"
            ),
            vec![
                "namespace:mozilla",
                "namespace:mozilla::dom",
                "struct:mozilla::dom::FooArgs",
                "field:mozilla::dom::FooArgs::name",
                "union:mozilla::dom::FooResult",
                "class:mozilla::dom::PFoo",
                "method:mozilla::dom::PFoo::Start",
            ]
        );
    }
}
