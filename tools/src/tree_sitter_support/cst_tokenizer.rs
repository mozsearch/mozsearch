use std::borrow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use include_dir::{Dir, include_dir};

use crate::file_format::history::syntax_files::TokenClass;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::tree_sitter_support::boilerplate::{FinishedTokens, RawToken, finish_tokens};
use crate::tree_sitter_support::config_tokenizer::{tokenize_ini, tokenize_toml};
use crate::tree_sitter_support::json_tokenizer::tokenize_json;
use crate::tree_sitter_support::preprocessor::{ConditionalStacks, literal_condition};

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
    Xpidl,
    /// Objective-C, with tree-sitter-objc (see `tokenize_objcpp`).
    Objc,
    /// Objective-C++: its Objective-C sections with tree-sitter-objc, and the
    /// rest as C++ (see `tokenize_objcpp`).
    ObjCpp,
    Java,
    Kotlin,
    /// See `config_tokenizer.rs`.
    Ini,
    /// See `config_tokenizer.rs`.
    Toml,
    /// See `json_tokenizer.rs`.
    Json,
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
    // UniFFI's interface definitions, a dialect of WebIDL (which UniFFI parses
    // with weedle), with tree-sitter-webidl, but in a namespace of their own,
    // since their symbols are Rust components' bindings, not WebIDL's.
    LanguageProfile {
        lang: "udl",
        namespace: "udl",
        grammar: Grammar::Webidl,
    },
    LanguageProfile {
        lang: "ipdl",
        namespace: "ipdl",
        grammar: Grammar::Ipdl,
    },
    LanguageProfile {
        lang: "xpidl",
        namespace: "xpidl",
        grammar: Grammar::Xpidl,
    },
    // Objective-C and Objective-C++ share C++'s namespace, since their files
    // define and use C and C++ symbols.
    LanguageProfile {
        lang: "objc",
        namespace: "cpp",
        grammar: Grammar::Objc,
    },
    LanguageProfile {
        lang: "objcpp",
        namespace: "cpp",
        grammar: Grammar::ObjCpp,
    },
    // Java and Kotlin share a namespace because they share symbols (the JVM's
    // fully qualified names), so that symbols' histories continue across
    // conversions from Java to Kotlin, as moved comments and strings do.
    LanguageProfile {
        lang: "java",
        namespace: "jvm",
        grammar: Grammar::Java,
    },
    LanguageProfile {
        lang: "kotlin",
        namespace: "jvm",
        grammar: Grammar::Kotlin,
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
    // JSON data files (and JSON-lines), with contexts from their structure;
    // see `json_tokenizer`.
    LanguageProfile {
        lang: "json",
        namespace: "json",
        grammar: Grammar::Json,
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
/// - 9: C++ is parsed with tree-sitter-mozcpp, which knows Gecko's macros,
///   with only preprocessor conditionals' first branches, and their
///   directives and other branches separately (see `tokenize_cpp`); and
///   functions returning pointers or references, and classes with qualified
///   names, are containers (see cpp.scm).  Containers after a node matching
///   more than once (ex: `int a, b;`) aren't skipped.  Rust's plain comments
///   are words, its item macros' items are tokenized as items (see
///   `rust_item_macro_body`), and its functions without bodies are
///   containers; JS's private methods are containers, and its strings are
///   single tokens.  Java, Kotlin, XPIDL, Objective-C, and Objective-C++
///   are tokenized with grammars, rather than as plain text (Objective-C++'s
///   methods' bodies as C++; see `tokenize_objcpp`).  Text that grammars
///   leave out of their trees is tokenized (with searchfox's forks of
///   tree-sitter-rust and tree-sitter-kotlin-ng, atoms, and ERROR nodes'
///   text; see `push_error_gap_tokens`).  Function pointers are named by
///   their names (see `clean_name_node`), C and C++'s enums and classes are
///   containers only where they're defined, and C++'s fields are containers
///   whatever their declarators (see cpp.scm).  Containers' comments (and
///   Rust's attributes) have their contexts (see `trivia_container`).  C,
///   C++, Objective-C, and Objective-C++'s "files-struct" rows have their
///   preprocessor conditionals (see `ConditionalStacks`).  ERROR nodes which
///   error recovery makes extras aren't comments.  C and C++ files with
///   Objective-C declarations are tokenized like Objective-C++, whose
///   declarations' C++ is tokenized as C++ (see `tokenize_objcpp`).  JS and
///   TS class fields and Rust's named fields are containers (see
///   typescript.scm and rust.scm), and trailing comments after separators
///   (ex: a Rust field's `,`) have the contexts of the code before them.
///   C++'s conversion operators are containers (see `name_text`).  UniFFI's
///   `.udl` files are tokenized as WebIDL (as "udl"), rather than plain text,
///   and JSON files (and JSON-lines), rather than as JS, by `json_tokenizer`,
///   with contexts from their structure.
///   Dead
///   preprocessor branches (`#if 0`'s) are comments (see `tokenize_cpp`),
///   and ANGLE's `ANGLE_MTL_OBJC_SCOPE`s don't make their blocks compound
///   literals (see `SCOPE_MACROS`).
pub const TOKENIZER_VERSION: u32 = 9;

/// The node to name a container by, given the node its query captured as its
/// name.  tree-sitter's error recovery can make that span unrecognized macros
/// and the comments between them, ex: tree-sitter-cpp's name for
/// `ALWAYS_INLINE ATTRIBUTE_NO_SANITIZE_ALL void TracePC::HandleCmp(...)` is
/// all of it before the parameters, with ERROR nodes in its scope.  So for a
/// name with errors, we use the innermost node along its `name` fields without
/// any (`HandleCmp`), or if there isn't one, it doesn't name a container.
///
/// And C and C++'s declarators in parentheses (function pointers, ex: `void
/// (*xFunc)(int);`, and `void (*(*nested)(int))(int);`, and names in
/// parentheses, ex: `static T (max)();`) are named by the names in them
/// (`xFunc`, `nested`, `max`), not `(*xFunc)`.
fn clean_name_node(mut name: tree_sitter::Node) -> Option<tree_sitter::Node> {
    if name.kind() == "parenthesized_declarator" {
        loop {
            name = match name.kind() {
                "parenthesized_declarator"
                | "pointer_declarator"
                | "reference_declarator"
                | "function_declarator" => match name.child_by_field_name("declarator") {
                    Some(declarator) => declarator,
                    // (A parenthesized declarator's and a reference
                    // declarator's aren't fields.)
                    None => {
                        let mut cursor = name.walk();
                        name.named_children(&mut cursor)
                            .filter(|child| !child.is_error() && !child.is_extra())
                            .last()?
                    }
                },
                _ => break,
            };
        }
    }
    while name.has_error() {
        name = name.child_by_field_name("name")?;
    }
    Some(name)
}

/// The text of a container's name node, but for C++'s conversion operators
/// (`operator_cast`, or a qualified name ending in one), which have no name
/// node: their text without their parameters and qualifiers, ex: `operator
/// bool` for `operator bool() const`, and `Foo::operator const char*`.
fn name_text<'t>(name: tree_sitter::Node, text: &'t str) -> &'t str {
    let full = name.utf8_text(text.as_bytes()).unwrap();
    let mut node = name;
    while node.kind() == "qualified_identifier" {
        let Some(inner) = node.child_by_field_name("name") else {
            return full;
        };
        node = inner;
    }
    if node.kind() != "operator_cast" {
        return full;
    }
    let mut declarator = node.child_by_field_name("declarator");
    while let Some(inner) = declarator {
        if inner.kind() == "abstract_function_declarator" {
            return text[name.start_byte()..inner.start_byte()].trim_end();
        }
        // (A reference declarator's declarator isn't a field.)
        declarator = inner.child_by_field_name("declarator").or_else(|| {
            let mut cursor = inner.walk();
            inner.named_children(&mut cursor).last()
        });
    }
    full
}

/// Normalize the text of a container's name node for use in a context.  Names
/// can contain whitespace (ex: C++ template arguments in out-of-line method
/// definitions, possibly spanning lines), but contexts can't contain whitespace
/// (see `syntax_files.rs`).  So we drop whitespace except between two word
/// characters, where it becomes an escaped space ("%20", with "%" escaped as
/// "%25" like `config_tokenizer` does).  This also makes contexts insensitive
/// to reformatting, ex: `Foo<A, B>` and `Foo<A,B>` are the same.
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
        "js" | "jsm" | "mjs" | "sjs" | "ts" => "js",
        "json" | "jsonl" | "ndjson" | "json5" | "webmanifest" | "har" | "geojson" => "json",
        "jsx" | "tsx" => "jsx",
        "py" | "build" | "configure" => "py",
        "rs" => "rust",
        "webidl" => "webidl",
        "udl" => "udl",
        "ipdl" | "ipdlh" => "ipdl",
        // (mozilla-central's other `.idl` files, ex: web-platform-tests'
        // WebIDL, need `searchfox-lang` attributes.)
        "idl" => "xpidl",
        // (Objective-C++ mixes C++ and Objective-C, which no grammar parses
        // both of; see `tokenize_objcpp`.)
        "m" => "objc",
        "mm" => "objcpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
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

/// What tokenizing a language with tree-sitter takes.
struct TreeSitterSetup {
    grammar: Grammar,
    ts_lang: tree_sitter::Language,
    query: Arc<tree_sitter::Query>,
    name_capture_ix: u32,
    container_capture_ix: u32,
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
    atom_nodes: Vec<u16>,
    // ### ignore_nodes
    //
    // As noted in https://github.com/tree-sitter/tree-sitter-c/issues/97 the
    // C preprocessor nodes currently are weird and include the trailing
    // newline.  For our purposes, we never actually want to emit a newline
    // token, so it's easy enough for us to just forbid that node.
    ignore_nodes: Vec<u16>,
    // ### quirks
    //
    // See `ClassQuirks`.
    quirks: ClassQuirks,
}

/// Tokens and structure from (possibly several) parses of a file, in any
/// order, with their offsets in the file for putting them in order.
#[derive(Default)]
struct Walked<'s> {
    tokens: Vec<(usize, RawToken<'s>)>,
    structure: Vec<(usize, FileStructureRow)>,
}

/// A container from a parse: its byte range in the file, and its context's
/// segments.
struct ContainerSpan {
    range: std::ops::Range<usize>,
    context: Vec<String>,
}

/// Process source contents with the given language profile.
pub fn hypertokenize_with_profile(
    profile: LanguageProfile,
    source_contents: &str,
) -> Result<HyperTokenized, String> {
    match profile.grammar {
        Grammar::Ini | Grammar::Toml | Grammar::Json => {
            let (tokens, structure) = match profile.grammar {
                Grammar::Ini => tokenize_ini(source_contents),
                Grammar::Toml => tokenize_toml(source_contents),
                _ => tokenize_json(source_contents),
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
        _ => {}
    }
    let mut walked = Walked::default();
    // (C and C++ files with Objective-C declarations, ex: headers of
    // Objective-C++, are tokenized like Objective-C++, which is the same as
    // C++ for files without any.)
    if matches!(
        profile.grammar,
        Grammar::ObjCpp | Grammar::Objc | Grammar::Cpp
    ) {
        let rest = match profile.grammar {
            Grammar::Objc => Grammar::Objc,
            _ => Grammar::Cpp,
        };
        tokenize_objcpp(source_contents, rest, &mut walked)?;
    } else {
        walk_tree(
            &tree_sitter_setup(profile.grammar)?,
            source_contents,
            source_contents,
            &|offset| Some(offset),
            &[],
            &mut walked,
            &mut vec![],
        )?;
    }
    // (Parts of files can be tokenized separately, ex: C++'s conditionals'
    // other branches, and XPIDL's C++ code blocks.  Stable, so tokens at the
    // same offset, if any, keep their order.)
    walked.tokens.sort_by_key(|(offset, _)| *offset);
    walked.structure.sort_by_key(|(offset, _)| *offset);
    if matches!(
        profile.grammar,
        Grammar::Cpp | Grammar::ObjCpp | Grammar::Objc
    ) {
        let conditionals = ConditionalStacks::new(source_contents);
        if !conditionals.is_empty() {
            for (offset, row) in &mut walked.structure {
                row.pp = conditionals.at_offset(*offset).to_vec();
            }
        }
    }

    Ok(HyperTokenized::new(
        profile,
        finish_tokens(
            source_contents,
            walked.tokens.into_iter().map(|(_, token)| token).collect(),
        ),
        walked.structure.into_iter().map(|(_, row)| row).collect(),
    ))
}

/// The setup for tokenizing a language with a tree-sitter grammar.
fn tree_sitter_setup(grammar: Grammar) -> Result<TreeSitterSetup, String> {
    let (ts_lang, ts_query_filename, atom_nodes, ignore_nodes, quirks) = match grammar {
        Grammar::Cpp => {
            let ts_lang: tree_sitter::Language = tree_sitter_mozcpp::LANGUAGE.into();
            let string_literal = ts_lang.id_for_node_kind("string_literal", true);
            let char_literal = ts_lang.id_for_node_kind("char_literal", true);
            // (Objective-C's `@"..."` and `@selector(foo:bar:)`, in
            // Objective-C++, are single tokens, as in tree-sitter-objc.)
            let objc_string_literal = ts_lang.id_for_node_kind("objc_string_literal", true);
            let selector_expression = ts_lang.id_for_node_kind("selector_expression", true);
            let newline = ts_lang.id_for_node_kind("\n", false);
            (
                ts_lang,
                "cpp",
                vec![
                    string_literal,
                    char_literal,
                    objc_string_literal,
                    selector_expression,
                ],
                vec![newline],
                ClassQuirks::default(),
            )
        }
        Grammar::TypeScript | Grammar::Tsx => {
            let ts_lang: tree_sitter::Language = match grammar {
                Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                _ => tree_sitter_typescript::LANGUAGE_TSX.into(),
            };
            // Strings are single tokens, like C++'s, so that changing their
            // quotes (`'` to `"`) changes them, rather than being two
            // evolutions.  (Not template strings, whose substitutions are
            // code.)
            let string = ts_lang.id_for_node_kind("string", true);
            (
                ts_lang,
                "typescript",
                vec![string],
                vec![],
                ClassQuirks::default(),
            )
        }
        Grammar::Python => {
            // Strings' contents are tokens (with their escape sequences,
            // which are their children, between which the text isn't), as
            // are f-strings' format specifiers (ex: `:>10`, of which only the
            // `:` is a child).
            let ts_lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
            let string_content = ts_lang.id_for_node_kind("string_content", true);
            let format_specifier = ts_lang.id_for_node_kind("format_specifier", true);
            let quirks =
                ClassQuirks::new(&ts_lang, &[("format_specifier", TokenClass::String)], &[]);
            (
                ts_lang,
                "python",
                vec![string_content, format_specifier],
                vec![],
                quirks,
            )
        }
        Grammar::Rust => {
            // tree-sitter-rust's comments only have children for their
            // delimiters and doc comments' text, not plain comments' text, so
            // we split their whole text into words.  And raw strings'
            // delimiters (`r#"`, `"#`) aren't in its trees, so raw strings are
            // single tokens, like other languages' strings.
            let ts_lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
            let line_comment = ts_lang.id_for_node_kind("line_comment", true);
            let block_comment = ts_lang.id_for_node_kind("block_comment", true);
            let raw_string_literal = ts_lang.id_for_node_kind("raw_string_literal", true);
            (
                ts_lang,
                "rust",
                vec![line_comment, block_comment, raw_string_literal],
                vec![],
                ClassQuirks::default(),
            )
        }
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
            // Comments only have children for their delimiters, so we split
            // their whole text into words, like Rust's.
            let comment = ts_lang.id_for_node_kind("comment", true);
            (ts_lang, "ipdl", vec![comment], vec![], quirks)
        }
        Grammar::Objc => {
            let ts_lang: tree_sitter::Language = tree_sitter_objc::LANGUAGE.into();
            // (Like C++'s.  `@selector(foo:bar:)`'s selector isn't in
            // tree-sitter-objc's tree, so it's a single token too.)
            let string_literal = ts_lang.id_for_node_kind("string_literal", true);
            let char_literal = ts_lang.id_for_node_kind("char_literal", true);
            let selector_expression = ts_lang.id_for_node_kind("selector_expression", true);
            let newline = ts_lang.id_for_node_kind("\n", false);
            (
                ts_lang,
                "objc",
                vec![string_literal, char_literal, selector_expression],
                vec![newline],
                ClassQuirks::default(),
            )
        }
        Grammar::Xpidl => {
            let ts_lang: tree_sitter::Language = tree_sitter_xpidl::LANGUAGE.into();
            let quirks = ClassQuirks::new(
                &ts_lang,
                // `native` declarations' C++ types, ex: `const nsAString`,
                // which we split into words like other text.  (Historical
                // XPIDL's preprocessor lines, ex: `#ifndef nsIFoo_h__`, are
                // extras, like comments, so they're comments' words.)
                &[("native_type", TokenClass::Text)],
                &[],
            );
            (ts_lang, "xpidl", vec![], vec![], quirks)
        }
        Grammar::Java => {
            let ts_lang: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
            // (Like C++'s.)
            let string_literal = ts_lang.id_for_node_kind("string_literal", true);
            (
                ts_lang,
                "java",
                vec![string_literal],
                vec![],
                ClassQuirks::default(),
            )
        }
        // (Kotlin's strings have interpolations, so they aren't atoms, but its
        // characters are, whose contents aren't in its trees.)
        Grammar::Kotlin => {
            let ts_lang: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
            let character_literal = ts_lang.id_for_node_kind("character_literal", true);
            (
                ts_lang,
                "kotlin",
                vec![character_literal],
                vec![],
                ClassQuirks::default(),
            )
        }
        Grammar::Ini | Grammar::Toml | Grammar::Json | Grammar::PlainText | Grammar::ObjCpp => {
            return Err(format!("{:?} isn't a tree-sitter grammar", grammar));
        }
    };
    let query = container_query(grammar, &ts_lang, ts_query_filename)?;
    Ok(TreeSitterSetup {
        grammar,
        name_capture_ix: query.capture_index_for_name("name").unwrap(),
        container_capture_ix: query.capture_index_for_name("container").unwrap(),
        ts_lang,
        query,
        atom_nodes,
        ignore_nodes,
        quirks,
    })
}

/// Parse `text` and add its tokens and structure to `walked`, for `source`, of
/// which `text` is a version: `to_source` maps offsets in `text` to offsets in
/// `source` (with the same text), or None for text which isn't the source's
/// (whose tokens are dropped).  Contexts are nested in `outer`, and the
/// containers are added to `containers`.
fn walk_tree<'s>(
    setup: &TreeSitterSetup,
    text: &str,
    source: &'s str,
    to_source: &dyn Fn(usize) -> Option<usize>,
    outer: &[String],
    walked: &mut Walked<'s>,
    containers: &mut Vec<ContainerSpan>,
) -> Result<(), String> {
    let parse_tree = parse(setup, text)?;
    walk_parsed(
        setup,
        &parse_tree,
        text,
        source,
        to_source,
        outer,
        walked,
        containers,
    )
}

fn parse(setup: &TreeSitterSetup, text: &str) -> Result<tree_sitter::Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&setup.ts_lang)
        .expect("Error loading grammar");
    parser
        .parse(text.as_bytes(), None)
        .ok_or_else(|| "Parse failed!".to_string())
}

/// See `walk_tree`, for a parse of `text`.
#[allow(clippy::too_many_arguments)]
fn walk_parsed<'s>(
    setup: &TreeSitterSetup,
    parse_tree: &tree_sitter::Tree,
    text: &str,
    source: &'s str,
    to_source: &dyn Fn(usize) -> Option<usize>,
    outer: &[String],
    walked: &mut Walked<'s>,
    containers: &mut Vec<ContainerSpan>,
) -> Result<(), String> {
    let container_query = &setup.query;

    // The source's version of a slice of `text`, if any.
    let source_slice = |slice: &str| -> Option<(usize, &'s str)> {
        let start = slice.as_ptr() as usize - text.as_ptr() as usize;
        let source_start = to_source(start)?;
        Some((
            source_start,
            source.get(source_start..source_start + slice.len())?,
        ))
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

    // The containers, in the order the traversal visits them (by start, outer
    // ones first), with the first match for each: tree-sitter reports matches
    // as they finish, so an inner node's (ex: `enum Foo` in `enum Foo mFoo;`)
    // can come before an outer one's at the same position, and a node can
    // match more than once (ex: a field declaration with two declarators, as
    // in `int a, b;`).
    let mut container_matches: Vec<(tree_sitter::Node, tree_sitter::Node, usize)> = vec![];
    {
        let mut query_cursor = tree_sitter::QueryCursor::new();
        let mut query_matches =
            query_cursor.matches(container_query, parse_tree.root_node(), text.as_bytes());
        let mut seen = std::collections::HashSet::new();
        while let Some(container_match) = query_matches.next() {
            let container = container_match
                .nodes_for_capture_index(setup.container_capture_ix)
                .next()
                .unwrap();
            if !seen.insert(container.id()) {
                continue;
            }
            let name = container_match
                .nodes_for_capture_index(setup.name_capture_ix)
                .next()
                .unwrap();
            container_matches.push((container, name, container_match.pattern_index));
        }
    }
    container_matches.sort_by_key(|(container, _, _)| {
        (
            container.start_byte(),
            std::cmp::Reverse(container.end_byte()),
        )
    });
    // The containers' names, for the comments and attributes which belong to
    // them (see `trivia_container`).
    let container_names: Vec<ContainerName> = container_matches
        .iter()
        .filter_map(|(container, name, _)| {
            let name = clean_name_node(*name)?;
            Some(ContainerName {
                start_byte: container.start_byte(),
                start_row: container.start_position().row,
                end_row: last_row(container),
                name: context_name(name_text(name, text)),
            })
        })
        .collect();
    let mut container_matches = container_matches.into_iter().peekable();

    let mut context_stack: Vec<String> = outer.to_vec();
    let empty_context = "%".to_string();
    let pretty_for = |stack: &Vec<String>| {
        if stack.is_empty() {
            empty_context.clone()
        } else {
            stack.join("::")
        }
    };
    let mut context_pretty = pretty_for(&context_stack);
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
                    context_pretty = pretty_for(&context_stack);
                    id_stack.pop();
                }
            } else {
                break;
            }
        } else {
            // We are considering this node for the first time and before any of
            // its children.

            // (Skipping containers the traversal didn't get to, ex: in ignored
            // nodes.)
            while container_matches
                .peek()
                .is_some_and(|(container, _, _)| container.start_byte() < node.start_byte())
            {
                container_matches.next();
            }
            // Handle if this is our next container.
            if let Some((_, name_node, pattern_index)) =
                container_matches.next_if(|(container, _, _)| container.id() == node.id())
                && let Some(name_node) = clean_name_node(name_node)
            {
                context_stack.push(context_name(name_text(name_node, text)));
                context_pretty = pretty_for(&context_stack);
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
                if let (Some(start), Some(end)) =
                    (to_source(node.start_byte()), to_source(node.end_byte()))
                {
                    walked.structure.push((
                        start,
                        FileStructureRow {
                            pretty: context_pretty.clone(),
                            // TODO: This should come from a `#set!` directive too but this nuance
                            // won't matter for a bit, so I'm punting because there's a potential
                            // the SCM queries would need to get a little more complex in order to
                            // differentiate between decl and def and when making the change it
                            // would probably be ideal to add more test coverage.
                            is_def: true,
                            kind: structure_kind.to_string(),
                            pp: vec![],
                        },
                    ));
                    containers.push(ContainerSpan {
                        range: start..end,
                        context: context_stack.clone(),
                    });
                }
                id_stack.push(node.id());
            }
            let node_kind_id = node.kind_id();
            // Grammars conventionally mark comments as "extra" nodes, but not all
            // of them do (ex: tree-sitter-webidl), so we also go by the name.
            // (Error recovery can make ERROR nodes extras too, ex: a lone
            // `mozilla::Foo*`, but they aren't comments.)
            let in_comment = (node.is_extra() && !node.is_error())
                || (node.is_named() && node.kind().contains("comment"))
                || parent_stack.last().is_some_and(|p| p.1);
            // A comment (or attribute) which belongs to a container next to it
            // has its context.
            let trivia_context = if is_trivia(&node) && !parent_stack.last().is_some_and(|p| p.1) {
                trivia_container(&node, &container_names).map(|name| {
                    let mut stack = context_stack.clone();
                    stack.push(name.to_string());
                    stack
                })
            } else {
                None
            };
            let leaf_context = trivia_context.as_ref().map(&pretty_for);
            let parent_kind_id = parent_stack.last().map(|p| p.0);
            if (node.is_error() || (node.parent().is_none() && node.has_error()))
                && node.child_count() > 0
            {
                push_error_gap_tokens(&node, text, &source_slice, &context_pretty, walked);
            }
            if setup.ignore_nodes.contains(&node_kind_id) {
                // ignore this node!
                visited_children = true;
            } else if setup.grammar == Grammar::Xpidl
                && node.kind() == "code_text"
                && let (Some(start), Some(end)) =
                    (to_source(node.start_byte()), to_source(node.end_byte()))
            {
                // XPIDL's code blocks are C++, which goes in the generated
                // headers, so we tokenize them as C++, in the context here.
                let cpp = tree_sitter_setup(Grammar::Cpp)?;
                tokenize_cpp(
                    &cpp,
                    source,
                    &source[start..end],
                    start,
                    &context_stack,
                    0,
                    walked,
                )?;
                visited_children = true;
            } else if let Some((open, body, close)) = rust_item_macro_body(setup, &node)
                && let Some(items) = parse_rust_items(setup, &text[body.clone()])
            {
                // Items in a macro, which we tokenize as items (see
                // `rust_item_macro_body`), between its delimiters.
                push_leaf_tokens(
                    &setup.quirks,
                    &open,
                    text,
                    &source_slice,
                    &context_pretty,
                    in_comment,
                    Some(node_kind_id),
                    walked,
                );
                walk_parsed(
                    setup,
                    &items,
                    &text[body.clone()],
                    source,
                    &|offset| to_source(body.start + offset),
                    &context_stack,
                    walked,
                    containers,
                )?;
                push_leaf_tokens(
                    &setup.quirks,
                    &close,
                    text,
                    &source_slice,
                    &context_pretty,
                    in_comment,
                    Some(node_kind_id),
                    walked,
                );
                visited_children = true;
            } else if !setup.atom_nodes.contains(&node_kind_id) && cursor.goto_first_child() {
                visited_children = false;
                _depth += 1;
                parent_stack.push((node_kind_id, in_comment));
                if let Some(stack) = trivia_context {
                    context_stack = stack;
                    context_pretty = pretty_for(&context_stack);
                    id_stack.push(node.id());
                }
            } else {
                push_leaf_tokens(
                    &setup.quirks,
                    &node,
                    text,
                    &source_slice,
                    leaf_context.as_deref().unwrap_or(&context_pretty),
                    in_comment,
                    parent_kind_id,
                    walked,
                );
                visited_children = true;
            }
        }
    }
    Ok(())
}

/// A container's span and name, for `trivia_container`.
struct ContainerName {
    start_byte: usize,
    start_row: usize,
    end_row: usize,
    name: String,
}

/// The last row of a node's text (not the row after a node ending with its
/// line's newline, as some grammars' comments do).
fn last_row(node: &tree_sitter::Node) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

/// Whether a node is a comment or a Rust attribute (`#[derive(...)]`), which
/// can belong to a container next to it (see `trivia_container`).
fn is_trivia(node: &tree_sitter::Node) -> bool {
    node.is_extra()
        || (node.is_named() && node.kind().contains("comment"))
        || node.kind() == "attribute_item"
}

/// The name of the container which a comment (or attribute) belongs to, if
/// any, of `containers` (in the walk's order), so that a container's comments'
/// changes are its own, not its enclosing container's (ex: a field's comment
/// isn't its class's):
/// - a comment after code on its line belongs to the outermost container in
///   that code ending on that line (ex: `int mX;  // The x.`, or `}  //
///   namespace mozilla`);
/// - a comment starting its line, in a run of comments and attributes without
///   blank lines, belongs to the outermost container starting on the first
///   line of what's right after them (or in it, for C++'s templates and
///   Python's decorated definitions), ex: a doc comment and `#[derive(...)]`
///   before a Rust struct.  (Not, ex, a section's comment followed by a blank
///   line, or a license header which a declaration follows.)
fn trivia_container<'a>(
    node: &tree_sitter::Node,
    containers: &'a [ContainerName],
) -> Option<&'a str> {
    let start_row = node.start_position().row;
    let first_at = |start: usize| containers.partition_point(|c| c.start_byte < start);
    // (Past separators after code on the line which aren't in its nodes, ex: a
    // Rust field's `,` or a JS class field's `;`.)
    let mut prev = node.prev_sibling();
    while let Some(separator) = prev
        && !separator.is_named()
        && separator
            .prev_sibling()
            .is_some_and(|before| last_row(&before) == start_row)
    {
        prev = separator.prev_sibling();
    }
    if let Some(prev) = prev
        && last_row(&prev) == start_row
    {
        if is_trivia(&prev) {
            return None;
        }
        let i = first_at(prev.start_byte());
        return containers[i..]
            .iter()
            .take_while(|c| c.start_byte < prev.end_byte())
            .find(|c| c.end_row == start_row)
            .map(|c| c.name.as_str());
    }
    // (A run of comments starting the file, ex: a license header, belongs to
    // no container.)
    let mut first = *node;
    while let Some(prev) = first.prev_sibling()
        && is_trivia(&prev)
        && last_row(&prev) + 1 >= first.start_position().row
    {
        first = prev;
    }
    if first.prev_sibling().is_none() && first.parent().is_some_and(|p| p.parent().is_none()) {
        return None;
    }
    let mut last = *node;
    let mut next = node.next_sibling()?;
    while is_trivia(&next) {
        if next.start_position().row > last_row(&last) + 1 {
            return None;
        }
        last = next;
        next = next.next_sibling()?;
    }
    if next.start_position().row > last_row(&last) + 1 {
        return None;
    }
    let wrapper = matches!(next.kind(), "template_declaration" | "decorated_definition");
    let container = containers.get(first_at(next.start_byte()))?;
    (container.start_byte < next.end_byte()
        && (wrapper || container.start_row == next.start_position().row))
        .then_some(container.name.as_str())
}

/// A Rust macro's body (between its delimiters) where items could be, for
/// tokenizing as items, ex: `feature! { #![feature = "fs"] pub fn open() {} }`
/// (nix) or `thread_local! { static FOO: Cell<u32> = Cell::new(0); }`, whose
/// items tree-sitter-rust leaves as tokens: a token tree of a macro invocation
/// at the top level of a module, or in an `impl`, trait, or `extern` block.
fn rust_item_macro_body<'t>(
    setup: &TreeSitterSetup,
    node: &tree_sitter::Node<'t>,
) -> Option<(
    tree_sitter::Node<'t>,
    std::ops::Range<usize>,
    tree_sitter::Node<'t>,
)> {
    if setup.grammar != Grammar::Rust || node.kind() != "token_tree" {
        return None;
    }
    let invocation = node
        .parent()
        .filter(|parent| parent.kind() == "macro_invocation")?;
    let scope = invocation.parent()?;
    if !matches!(scope.kind(), "source_file" | "declaration_list") {
        return None;
    }
    let count = node.child_count();
    if count < 2 {
        return None;
    }
    let open = node.child(0)?;
    let close = node.child(count - 1)?;
    Some((open, open.end_byte()..close.start_byte(), close))
}

/// A parse of a Rust macro's body as items (see `rust_item_macro_body`), if it
/// is that, without errors: ex: not `macro_rules!` patterns, `bitflags!`'s
/// `struct Flags: u32 {...}`, or `cfg_if!`'s `if #[cfg(...)] {...}`.
fn parse_rust_items(setup: &TreeSitterSetup, text: &str) -> Option<tree_sitter::Tree> {
    let tree = parse(setup, text).ok()?;
    let is_items = {
        let root = tree.root_node();
        let mut cursor = root.walk();
        !root.has_error()
            && root
                .named_children(&mut cursor)
                .any(|child| child.kind().ends_with("_item"))
    };
    is_items.then_some(tree)
}

/// Add the tokens of an ERROR node's (or an erroneous parse's root's) text
/// which isn't its children's to `walked`, in the context `context_pretty`:
/// tree-sitter's error recovery can leave text out of them (ex: with
/// tree-sitter-kotlin-ng 1.1.0, giving up on the rest of a file, it made an
/// ERROR node of it with only its first token, or left it out of the tree).
fn push_error_gap_tokens<'s>(
    node: &tree_sitter::Node,
    text: &str,
    source_slice: &dyn Fn(&str) -> Option<(usize, &'s str)>,
    context_pretty: &str,
    walked: &mut Walked<'s>,
) {
    let mut cursor = node.walk();
    let mut covered = node.start_byte();
    let mut gaps = vec![];
    for child in node.children(&mut cursor) {
        if child.start_byte() > covered {
            gaps.push(covered..child.start_byte());
        }
        covered = covered.max(child.end_byte());
    }
    if node.end_byte() > covered {
        gaps.push(covered..node.end_byte());
    }
    for gap in gaps {
        let Some(gap_text) = text.get(gap) else {
            continue;
        };
        for piece in gap_text.split_whitespace() {
            if let Some((offset, source_text)) = source_slice(piece) {
                let class = if piece.chars().any(|c| c.is_alphanumeric()) {
                    TokenClass::Identifier
                } else {
                    TokenClass::Operator
                };
                walked.tokens.push((
                    offset,
                    RawToken {
                        context: context_pretty.to_string(),
                        class,
                        text: source_text,
                    },
                ));
            }
        }
    }
}

/// Add a leaf node's tokens to `walked`, in the context `context_pretty`.
#[allow(clippy::too_many_arguments)]
fn push_leaf_tokens<'s>(
    quirks: &ClassQuirks,
    node: &tree_sitter::Node,
    text: &str,
    source_slice: &dyn Fn(&str) -> Option<(usize, &'s str)>,
    context_pretty: &str,
    in_comment: bool,
    parent_kind_id: Option<u16>,
    walked: &mut Walked<'s>,
) {
    let token = node.utf8_text(text.as_bytes()).unwrap().trim();
    let class = classify_leaf(node, token, in_comment, parent_kind_id, quirks);
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
            if let Some((offset, text)) = source_slice(piece) {
                walked.tokens.push((
                    offset,
                    RawToken {
                        context: context_pretty.to_string(),
                        class,
                        text,
                    },
                ));
            }
        }
    } else if let Some((offset, text)) = source_slice(token) {
        walked.tokens.push((
            offset,
            RawToken {
                context: context_pretty.to_string(),
                class,
                text,
            },
        ));
    }
}

/// How deeply `tokenize_cpp` recurses into conditionals' branches, ex:
/// `#else` branches inside `#else` branches, before it gives up on their
/// conditionals.
const MAX_CONDITIONAL_DEPTH: u32 = 16;

/// Tokenize C++ (a range of `source`): tree-sitter-cpp only parses
/// preprocessor conditionals around whole declarations and statements, and
/// conditionals elsewhere (ex: `#ifdef DEBUG` in a constructor's initializers,
/// or `#else` between a declaration and a definition) make it misparse the
/// rest of the file.  So we parse the code with only conditionals' first
/// branches (blanking their directives and other branches, keeping offsets),
/// and then tokenize the directives (see `tokenize_directive`) and the other
/// branches (each the same way) separately, in the contexts at their
/// locations in the first parse.
///
/// Dead branches (literally false ones, `#if 0`'s, and ones after literally
/// true ones) are tokenized as comments' words (see `ConditionalPlan::dead`).
///
/// `text` is the source's text at `base` (or a version of it of the same
/// length, ex: with Objective-C++'s Objective-C blanked; see
/// `tokenize_objcpp`).  Returns the first parse's containers.  (Objective-C,
/// with tree-sitter-objc's `setup`, is tokenized this way too.)
fn tokenize_cpp<'s>(
    setup: &TreeSitterSetup,
    source: &'s str,
    text: &str,
    base: usize,
    outer: &[String],
    depth: u32,
    walked: &mut Walked<'s>,
) -> Result<Vec<ContainerSpan>, String> {
    let to_source = |offset: usize| Some(base + offset);
    let plan = if depth < MAX_CONDITIONAL_DEPTH {
        plan_conditionals(text)
    } else {
        None
    };
    let mut containers = vec![];
    // (Statement macros before blocks are blanked for the parse; see
    // `SCOPE_MACROS`.)
    let parse_text = plan.as_ref().map_or(text, |plan| &plan.first_branches);
    let scope_macros = scope_macros(parse_text);
    let blanked;
    let parse_text = if scope_macros.is_empty() {
        parse_text
    } else {
        let mut bytes = parse_text.as_bytes().to_vec();
        for range in &scope_macros {
            bytes[range.clone()].fill(b' ');
        }
        // (Blanking ASCII names keeps the text UTF-8.)
        blanked = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        &blanked
    };
    walk_tree(
        setup,
        parse_text,
        source,
        &to_source,
        outer,
        walked,
        &mut containers,
    )?;
    // The context at a location: its innermost container's.
    let context_at = |offset: usize| -> Vec<String> {
        containers
            .iter()
            .filter(|container| container.range.contains(&offset))
            .max_by_key(|container| (container.range.start, container.context.len()))
            .map_or_else(|| outer.to_vec(), |container| container.context.clone())
    };
    for range in scope_macros {
        let offset = base + range.start;
        let context = context_at(offset);
        walked.tokens.push((
            offset,
            RawToken {
                context: if context.is_empty() {
                    "%".to_string()
                } else {
                    context.join("::")
                },
                class: TokenClass::Identifier,
                text: &source[offset..base + range.end],
            },
        ));
    }
    let Some(plan) = plan else {
        return Ok(containers);
    };
    for directive in plan.directives {
        let directive = base + directive.start..base + directive.end;
        tokenize_directive(
            setup,
            source,
            directive.clone(),
            &context_at(directive.start),
            walked,
        )?;
    }
    for branch in plan.other_branches {
        tokenize_cpp(
            setup,
            source,
            &text[branch.clone()],
            base + branch.start,
            &context_at(base + branch.start),
            depth + 1,
            walked,
        )?;
    }
    // Dead branches (ex: `#if 0`'s, which Gecko rarely has, since it removes
    // dead code instead) are like comments: their words, in the contexts at
    // them.  (Not parsed as code, which they may no longer be, and so code
    // that becomes dead doesn't keep its context.)
    for branch in plan.dead {
        let context = context_at(base + branch.start);
        let context = if context.is_empty() {
            "%".to_string()
        } else {
            context.join("::")
        };
        let branch_text = &text[branch.clone()];
        for word in branch_text.split_whitespace() {
            let offset =
                base + branch.start + (word.as_ptr() as usize - branch_text.as_ptr() as usize);
            if let Some(text) = source.get(offset..offset + word.len()) {
                walked.tokens.push((
                    offset,
                    RawToken {
                        context: context.clone(),
                        class: TokenClass::Comment,
                        text,
                    },
                ));
            }
        }
    }
    Ok(containers)
}

/// Tokenize Objective-C++ or Objective-C: tree-sitter-objc doesn't parse
/// C++, and tree-sitter-mozcpp only parses Objective-C's expressions and
/// statements, so Objective-C++'s Objective-C sections (`@interface`,
/// `@implementation`, and `@protocol` through `@end`, and lines like `@class
/// Foo;`) are parsed with tree-sitter-objc, and the `rest` with
/// tree-sitter-mozcpp, with the sections blanked (keeping offsets), and its
/// methods' bodies, which are C++, with tree-sitter-mozcpp too, in the
/// methods' contexts (blanked for tree-sitter-objc), as is C++ in its
/// declarations (see `objc_cpp_groups`).  C and C++ files with Objective-C
/// declarations (ex: headers of Objective-C++) are tokenized this way too
/// (which is the same as `tokenize_cpp` for files without any).  Objective-C
/// files' rest
/// is parsed with tree-sitter-objc too, separately, which keeps errors in one
/// part from affecting the other.  Objective-C declarations can only be at
/// the top level, so the sections' contexts are their own.  All of them are
/// parsed with preprocessor conditionals' first branches (see
/// `tokenize_cpp`).
///
/// On firefox-disco (2026-10-08), Objective-C++ (422 files) this way had
/// containers for 99.5% of the Objective-C methods and 99.9% of the analyzed
/// C++ definitions (and 8% of the tokens at the top level), vs 92% and 73%
/// (29%) parsing it all with tree-sitter-objc, or 0% and 94% (34%) as C++;
/// Objective-C (106 files) had 100%, 100%, and 17%.
fn tokenize_objcpp<'s>(
    source: &'s str,
    rest: Grammar,
    walked: &mut Walked<'s>,
) -> Result<(), String> {
    let sections = objc_sections(source);
    let mut cpp_text = source.as_bytes().to_vec();
    for section in &sections {
        for b in &mut cpp_text[section.clone()] {
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }
    }
    // (Blanking whole lines keeps the text UTF-8.)
    let cpp_text = String::from_utf8(cpp_text).map_err(|e| e.to_string())?;
    tokenize_cpp(
        &tree_sitter_setup(rest)?,
        source,
        &cpp_text,
        0,
        &[],
        0,
        walked,
    )?;
    if sections.is_empty() {
        return Ok(());
    }
    let objc = tree_sitter_setup(Grammar::Objc)?;
    for section in sections {
        let start = section.start;
        // Objective-C++'s methods' bodies are C++ (with Objective-C), which
        // tree-sitter-objc can't parse, so we blank them (between their
        // braces) for it, and tokenize them as C++, in the contexts at them.
        let bodies: Vec<std::ops::Range<usize>> = if rest == Grammar::Cpp {
            objc_method_bodies(&source[section.clone()])
                .into_iter()
                .map(|body| start + body.start + 1..start + body.end - 1)
                .collect()
        } else {
            vec![]
        };
        let mut text = source[section.clone()].as_bytes().to_vec();
        for body in &bodies {
            for b in &mut text[body.start - start..body.end - start] {
                if *b != b'\n' && *b != b'\r' {
                    *b = b' ';
                }
            }
        }
        // C++ in Objective-C++'s declarations (ex: instance variables'
        // `std::unique_ptr<Foo>` types, in their blocks, and methods'
        // parameters' types) is too, blanked and tokenized as C++.
        let cpp_groups: Vec<std::ops::Range<usize>> = if rest == Grammar::Cpp {
            objc_cpp_groups(&text)
                .into_iter()
                .map(|group| start + group.start..start + group.end)
                .collect()
        } else {
            vec![]
        };
        for group in &cpp_groups {
            for b in &mut text[group.start - start..group.end - start] {
                if *b != b'\n' && *b != b'\r' {
                    *b = b' ';
                }
            }
        }
        // (And name macros' names and parentheses; see `OBJC_NAME_MACROS`.)
        let macro_tokens = objc_name_macros(&text);
        for (range, _) in &macro_tokens {
            for b in &mut text[range.clone()] {
                *b = b' ';
            }
        }
        // (Blanking between ASCII braces and parentheses, and ASCII names,
        // keeps the text UTF-8.)
        let text = String::from_utf8(text).map_err(|e| e.to_string())?;
        let containers = tokenize_cpp(&objc, source, &text, start, &[], 0, walked)?;
        let innermost_context = |offset: usize| -> Vec<String> {
            containers
                .iter()
                .filter(|container| container.range.contains(&offset))
                .max_by_key(|container| (container.range.start, container.context.len()))
                .map_or_else(Vec::new, |container| container.context.clone())
        };
        for (range, class) in macro_tokens {
            let offset = start + range.start;
            let context = innermost_context(offset);
            walked.tokens.push((
                offset,
                RawToken {
                    context: if context.is_empty() {
                        "%".to_string()
                    } else {
                        context.join("::")
                    },
                    class,
                    text: &source[offset..start + range.end],
                },
            ));
        }
        if bodies.is_empty() && cpp_groups.is_empty() {
            continue;
        }
        let cpp = tree_sitter_setup(Grammar::Cpp)?;
        for body in bodies.into_iter().chain(cpp_groups) {
            // (The innermost container's, ex: the method's.)
            let context = innermost_context(body.start);
            tokenize_cpp(
                &cpp,
                source,
                &source[body.clone()],
                body.start,
                &context,
                0,
                walked,
            )?;
        }
    }
    Ok(())
}

/// The byte ranges (`{` through `}`) of the methods' bodies in an Objective-C
/// section (see `tokenize_objcpp`): braces after lines starting with `-` or
/// `+` outside of braces, in preprocessor conditionals' first branches (as
/// `tokenize_cpp` parses them), outside of comments, strings, and characters
/// (or none, if its braces don't balance).
fn objc_method_bodies(text: &str) -> Vec<std::ops::Range<usize>> {
    let first_branches = plan_conditionals(text).map(|plan| plan.first_branches);
    let bytes = first_branches.as_deref().unwrap_or(text).as_bytes();
    let mut bodies = vec![];
    let mut depth = 0;
    let mut in_method = false;
    let mut body_start = 0;
    let mut line_start = true;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if line_start && !b.is_ascii_whitespace() {
            line_start = false;
            if depth == 0 {
                match b {
                    b'-' | b'+' => in_method = true,
                    b'@' => in_method = false,
                    _ => {}
                }
            }
        }
        match b {
            b'\n' => line_start = true,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i + 1 < bytes.len() && bytes[i + 1] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            // (Up to the closing quote, or before the newline ending an
            // unterminated one.)
            b'"' | b'\'' => {
                while i + 1 < bytes.len() && bytes[i + 1] != b && bytes[i + 1] != b'\n' {
                    if bytes[i + 1] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if bytes.get(i + 1) == Some(&b) {
                    i += 1;
                }
            }
            b'{' => {
                if depth == 0 && in_method {
                    body_start = i;
                }
                depth += 1;
            }
            b'}' => {
                if depth == 0 {
                    return vec![];
                }
                depth -= 1;
                if depth == 0 && in_method {
                    bodies.push(body_start..i + 1);
                    in_method = false;
                }
            }
            // (A declaration, ex: `- (void)foo;`.)
            b';' if depth == 0 => in_method = false,
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return vec![];
    }
    bodies
}

/// Which of `text`'s bytes are code: not in comments, strings, or characters
/// (up to their ends, or the ends of their lines if they're unterminated).
fn code_bytes(text: &[u8]) -> Vec<bool> {
    let mut code = vec![true; text.len()];
    let mut i = 0;
    while i < text.len() {
        let start = i;
        match text[i] {
            b'/' if text.get(i + 1) == Some(&b'/') => {
                while i + 1 < text.len() && text[i + 1] != b'\n' {
                    i += 1;
                }
            }
            b'/' if text.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < text.len() && !(text[i] == b'*' && text[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            quote @ (b'"' | b'\'') => {
                while i + 1 < text.len() && text[i + 1] != quote && text[i + 1] != b'\n' {
                    if text[i + 1] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if text.get(i + 1) == Some(&quote) {
                    i += 1;
                }
            }
            _ => {
                i += 1;
                continue;
            }
        }
        let end = (i + 1).min(text.len());
        for b in &mut code[start..end] {
            *b = false;
        }
        i = end;
    }
    code
}

/// Statement macros before blocks, which expand to statements taking them,
/// ex: ANGLE's `ANGLE_MTL_OBJC_SCOPE { ... }` (62 uses), which is
/// `@autoreleasepool`, but which tree-sitter-mozcpp takes for a compound
/// literal (since a name before a block on the next line can't be a
/// statement in general, ex: flex's `YY_DECL` starts a function definition),
/// so that the block's statements were an initializer list's (ex: `if` and
/// `else` were identifiers).  They're blanked for the parse (so their blocks
/// are blocks), and their own tokens get the contexts at them.
const SCOPE_MACROS: &[&str] = &["ANGLE_MTL_OBJC_SCOPE", "ANGLE_APPLE_OBJC_SCOPE"];

/// The byte ranges of `SCOPE_MACROS` before blocks in `text`, outside of
/// comments, strings, and characters.
fn scope_macros(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = vec![];
    let mut code = None;
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for name in SCOPE_MACROS {
        for (at, _) in text.match_indices(name) {
            let end = at + name.len();
            if at > 0 && is_word(bytes[at - 1]) || bytes.get(end).is_some_and(|b| is_word(*b)) {
                continue;
            }
            let next = text[end..].trim_start();
            if !next.starts_with('{') {
                continue;
            }
            let code = code.get_or_insert_with(|| code_bytes(bytes));
            if code[at] {
                ranges.push(at..end);
            }
        }
    }
    ranges
}

/// The byte ranges of the contents of the outermost parentheses and braces
/// in an Objective-C++ section (with its methods' bodies blanked; see
/// `tokenize_objcpp`) which have C++'s `::` (ex: `std::unique_ptr<Foo>`
/// types), outside of comments, strings, and characters (or none, if they
/// don't balance).
fn objc_cpp_groups(text: &[u8]) -> Vec<std::ops::Range<usize>> {
    let code = code_bytes(text);
    let mut groups = vec![];
    let mut depth = 0;
    let mut group_start = 0;
    let mut has_scope = false;
    for (i, b) in text.iter().enumerate() {
        if !code[i] {
            continue;
        }
        match b {
            b'(' | b'{' => {
                if depth == 0 {
                    group_start = i + 1;
                    has_scope = false;
                }
                depth += 1;
            }
            b')' | b'}' => {
                if depth == 0 {
                    return vec![];
                }
                depth -= 1;
                if depth == 0 && has_scope {
                    groups.push(group_start..i);
                }
            }
            b':' if depth > 0 && text.get(i + 1) == Some(&b':') => has_scope = true,
            _ => {}
        }
    }
    if depth != 0 {
        return vec![];
    }
    groups
}

/// Macros wrapping names in Objective-C declarations, which tree-sitter-objc
/// can't parse, ex: libwebrtc's `RTC_OBJC_TYPE(RTCVideoFrame)` (2041 uses in
/// 338 files), which prefixes them, in `@protocol RTC_OBJC_TYPE
/// (RTCVideoEncoder)<NSObject>`, types, and protocol lists.  Its Objective-C
/// sections are parsed with them blanked but for the names (see
/// `objc_name_macros`), whose own tokens get the contexts at them.
const OBJC_NAME_MACROS: &[&str] = &["RTC_OBJC_TYPE"];

/// The byte ranges and classes of the tokens of `OBJC_NAME_MACROS` in `text`
/// but their names: the macros' names and parentheses.
fn objc_name_macros(text: &[u8]) -> Vec<(std::ops::Range<usize>, TokenClass)> {
    let code = code_bytes(text);
    let mut tokens = vec![];
    let skip_space = |mut i: usize| {
        while i < text.len() && text[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for name in OBJC_NAME_MACROS {
        let mut from = 0;
        while let Some(at) = text[from..]
            .windows(name.len())
            .position(|window| window == name.as_bytes())
            .map(|i| from + i)
        {
            from = at + name.len();
            if !code[at]
                || at > 0 && is_word(text[at - 1])
                || text.get(from).is_some_and(|b| is_word(*b))
            {
                continue;
            }
            let open = skip_space(from);
            if text.get(open) != Some(&b'(') {
                continue;
            }
            let word_start = skip_space(open + 1);
            let mut word_end = word_start;
            while word_end < text.len() && is_word(text[word_end]) {
                word_end += 1;
            }
            let close = skip_space(word_end);
            if word_end == word_start || text.get(close) != Some(&b')') {
                continue;
            }
            tokens.push((at..from, TokenClass::Identifier));
            tokens.push((open..open + 1, TokenClass::Operator));
            tokens.push((close..close + 1, TokenClass::Operator));
            from = close + 1;
        }
    }
    tokens
}

/// The byte ranges (whole lines) of Objective-C sections in `source` (see
/// `tokenize_objcpp`): `@interface`, `@implementation`, and `@protocol`
/// through `@end`, and other lines starting with Objective-C's declarations'
/// keywords (ex: `@class Foo;`, and forward declarations of protocols and
/// classes through their `;`, ex: `@protocol RTC_OBJC_TYPE\n(Foo);`), but not
/// with its statements and expressions (ex: `@try {`, `@"..."`,
/// `@protocol(Foo)`), which are the rest's, nor in block comments (ex:
/// Doxygen's `@class`).
fn objc_sections(source: &str) -> Vec<std::ops::Range<usize>> {
    // Whether the protocol or class declaration starting `rest` is a forward
    // declaration: if all it has before its first `;` is names, commas,
    // parentheses, and angle brackets (ex: not a method, `@end`, or a
    // comment), the end of the line with it.
    let forward_declaration_end = |rest: &str| -> Option<usize> {
        let semicolon = rest.find(';')?;
        let only_names = rest[1..semicolon]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || "_,()<>".contains(c));
        if !only_names {
            return None;
        }
        Some(
            rest[semicolon..]
                .find('\n')
                .map_or(rest.len(), |i| semicolon + i + 1),
        )
    };
    let mut sections = vec![];
    let mut open: Option<usize> = None;
    let mut in_block_comment = false;
    let mut pos = 0;
    while pos < source.len() {
        let end = source[pos..].find('\n').map_or(source.len(), |i| pos + i);
        let mut next = (end + 1).min(source.len());
        let in_comment = in_block_comment;
        in_block_comment = ends_in_block_comment(&source[pos..end], in_block_comment);
        let line = source[pos..end].trim();
        let word = line
            .strip_prefix('@')
            .filter(|_| !in_comment)
            .and_then(|rest| {
                let word = rest
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                let declaration = matches!(
                    word,
                    "interface"
                        | "implementation"
                        | "protocol"
                        | "end"
                        | "class"
                        | "compatibility_alias"
                        | "import"
                        | "property"
                        | "synthesize"
                        | "dynamic"
                        | "optional"
                        | "required"
                        | "public"
                        | "private"
                        | "protected"
                        | "package"
                );
                (declaration && !rest[word.len()..].trim_start().starts_with('(')).then_some(word)
            });
        let at = pos + source[pos..end].find('@').unwrap_or(0);
        match (open, word) {
            (None, Some("implementation")) => open = Some(pos),
            (None, Some("interface" | "protocol")) => {
                match forward_declaration_end(&source[at..]) {
                    Some(declaration_end) => {
                        next = at + declaration_end;
                        sections.push(pos..next);
                        in_block_comment = false;
                    }
                    None => open = Some(pos),
                }
            }
            (None, Some(_)) => sections.push(pos..next),
            (Some(start), Some("end")) => {
                sections.push(start..next);
                open = None;
            }
            _ => {}
        }
        if next == end {
            break;
        }
        pos = next;
    }
    if let Some(start) = open {
        sections.push(start..source.len());
    }
    sections
}

/// A preprocessor conditional directive (`#if ...`, `#else`, ...) by itself:
/// we parse it in a conditional of its own, ex: `#if 0\n#else\n#endif\n` for
/// `#else`, so that its tokens are what they'd be in a whole conditional, and
/// keep its tokens.
fn tokenize_directive<'s>(
    setup: &TreeSitterSetup,
    source: &'s str,
    range: std::ops::Range<usize>,
    context: &[String],
    walked: &mut Walked<'s>,
) -> Result<(), String> {
    let directive = &source[range.clone()];
    let (prefix, suffix) = match directive_word(directive) {
        Some("if" | "ifdef" | "ifndef") => ("", "\n#endif\n"),
        Some("endif") => ("#if 0\n", "\n"),
        _ => ("#if 0\n", "\n#endif\n"),
    };
    let text = format!("{}{}{}", prefix, directive, suffix);
    let kept = prefix.len()..prefix.len() + directive.len();
    walk_tree(
        setup,
        &text,
        source,
        &|offset| {
            kept.contains(&offset)
                .then(|| range.start + offset - kept.start)
        },
        context,
        walked,
        &mut vec![],
    )
}

/// The word of a preprocessor directive line, ex: "ifdef" for `#  ifdef FOO`.
fn directive_word(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('#')?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// See `tokenize_cpp`.
struct ConditionalPlan {
    /// The text with only conditionals' first branches: their directives and
    /// other branches are blanked (with spaces, keeping newlines).
    first_branches: String,
    /// The (blanked) directives, without their lines' newlines.
    directives: Vec<std::ops::Range<usize>>,
    /// The (blanked) other branches (including any conditionals in them).
    other_branches: Vec<std::ops::Range<usize>>,
    /// The (blanked) dead branches: literally false ones (`#if 0`), and ones
    /// after literally true ones (`#if 1`'s `#else`), including any
    /// conditionals in them.
    dead: Vec<std::ops::Range<usize>>,
}

/// Plan how to tokenize C++ text with preprocessor conditionals (see
/// `tokenize_cpp`), or None if it has none.  Directives are lines starting with
/// `#` (and their continuation lines), outside of block comments.
fn plan_conditionals(text: &str) -> Option<ConditionalPlan> {
    let bytes = text.as_bytes();
    let mut blanks: Vec<std::ops::Range<usize>> = vec![];
    let mut directives = vec![];
    let mut other_branches: Vec<std::ops::Range<usize>> = vec![];
    let mut dead: Vec<std::ops::Range<usize>> = vec![];
    // Each open conditional's state.
    struct Level {
        /// Whether we're past its first branch.
        past_first: bool,
        /// Whether a branch so far is literally true (`#if 1`).
        taken: bool,
        /// Whether its current branch is dead: literally false (`#if 0`), or
        /// after one that's literally true.
        dead: bool,
    }
    let mut stack: Vec<Level> = vec![];
    let mut in_block_comment = false;
    let mut any = false;
    let mut pos = 0;
    while pos < bytes.len() {
        let start = pos;
        // The line, through its newline, with continuation lines if it's a
        // directive.
        let mut end = start;
        let is_directive = !in_block_comment && directive_word(line_at(text, start)).is_some();
        loop {
            while end < bytes.len() && bytes[end] != b'\n' {
                end += 1;
            }
            let content = text[start..end].trim_end();
            if is_directive && content.ends_with('\\') && end < bytes.len() {
                end += 1;
                continue;
            }
            break;
        }
        let line_end = end;
        let next = (end + 1).min(bytes.len());
        // The outermost conditional whose current branch isn't its first or
        // is dead: whether we're in a branch other than the first of some
        // conditional (and its level), or in a dead branch (and its level).
        let blocking = stack
            .iter()
            .position(|level| level.past_first || level.dead);
        let (inactive, dead_level) = match blocking {
            Some(level) if stack[level].dead => (None, Some(level)),
            Some(level) => (Some(level), None),
            None => (None, None),
        };
        let mut directive_here = false;
        // (Directives in dead branches are dead too, but the dead
        // conditional's own.)
        let mut dead_here = dead_level.is_some();
        if is_directive {
            let line = &text[start..line_end];
            let literal = literal_condition(line);
            match directive_word(line) {
                Some("if" | "ifdef" | "ifndef") => {
                    any = true;
                    directive_here = inactive.is_none() && dead_level.is_none();
                    stack.push(Level {
                        past_first: false,
                        taken: literal == Some(true),
                        dead: literal == Some(false),
                    });
                }
                Some(word @ ("elif" | "elifdef" | "elifndef" | "else")) => {
                    let level = stack.len().checked_sub(1);
                    directive_here = match (inactive, dead_level, level) {
                        (_, Some(dead_level), Some(level)) => dead_level == level,
                        (Some(inactive), _, Some(level)) => inactive >= level,
                        _ => true,
                    };
                    if let Some(level) = level
                        && directive_here
                    {
                        let state = &mut stack[level];
                        state.past_first = true;
                        state.dead = state.taken || (word == "elif" && literal == Some(false));
                        state.taken |= literal == Some(true);
                    }
                }
                Some("endif") => {
                    let level = stack.len().checked_sub(1);
                    directive_here = match (inactive, dead_level, level) {
                        (_, Some(dead_level), Some(level)) => dead_level == level,
                        (Some(inactive), _, Some(level)) => inactive >= level,
                        _ => true,
                    };
                    stack.pop();
                }
                _ => {}
            }
            dead_here &= !directive_here;
        }
        if directive_here {
            directives.push(start..line_end);
            blanks.push(start..line_end);
        } else if dead_here {
            // (Consecutive lines of a dead branch make one.)
            match dead.last_mut() {
                Some(range) if range.end == start => range.end = next,
                _ => dead.push(start..next),
            }
            blanks.push(start..next);
        } else if inactive.is_some() {
            // (Consecutive lines of a branch make one.)
            match other_branches.last_mut() {
                Some(branch) if branch.end == start => branch.end = next,
                _ => other_branches.push(start..next),
            }
            blanks.push(start..next);
        }
        if !is_directive {
            in_block_comment = ends_in_block_comment(&text[start..line_end], in_block_comment);
        }
        pos = next;
        if next == line_end {
            break;
        }
    }
    if !any {
        return None;
    }
    let mut first_branches = bytes.to_vec();
    for blank in blanks {
        for b in &mut first_branches[blank] {
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }
    }
    // (Blanking ASCII bytes or whole characters keeps the text UTF-8: blanks
    // are whole lines.)
    let first_branches = String::from_utf8(first_branches).ok()?;
    Some(ConditionalPlan {
        first_branches,
        directives,
        other_branches,
        dead,
    })
}

/// The line of `text` starting at `start`.
fn line_at(text: &str, start: usize) -> &str {
    let rest = &text[start..];
    &rest[..rest.find('\n').unwrap_or(rest.len())]
}

/// Whether a block comment is open at the end of a line, given whether one was
/// at its start, skipping strings, characters, and line comments.
pub(crate) fn ends_in_block_comment(line: &str, mut in_block_comment: bool) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if in_block_comment {
            if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => return false,
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                in_block_comment = true;
                i += 2;
            }
            quote @ (b'"' | b'\'') => {
                // (Not a C++14 digit separator, ex: `1'000`.)
                if quote == b'\'' && i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
                    i += 1;
                    continue;
                }
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    in_block_comment
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
            (
                "a.udl",
                "namespace tabs {};\n[Error]\ninterface TabsApiError { SyncError(string reason); };\n\
                 interface TabsStore { constructor(string path); sequence<RemoteTabRecord> get_all(); };",
                "udl",
            ),
            ("a.ini", "[test.html]\nskip-if = os == 'win'", "ini"),
            (
                "a.json",
                "{\"dependencies\": {\"react\": \"19.2.0\"}}",
                "json",
            ),
            (
                "a.toml",
                "[\"test.html\"]\nskip-if = [\"os == 'win'\"]",
                "toml",
            ),
            (
                "a.idl",
                "[scriptable, uuid(a88e5a60-205a-4bb1-94e1-2628daf51eae)]\ninterface nsIFoo : nsISupports { void go(in long a); };",
                "xpidl",
            ),
            (
                "a.java",
                "class C { int mX; void m(int a) { mX = a; } }",
                "java",
            ),
            (
                "a.kt",
                "class C {\n    fun m(a: Int) = a + 1\n}\n",
                "kotlin",
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
        // Function pointers, and names in parentheses, are named by the names
        // in them (see `clean_name_node`).
        let prettys: Vec<String> = hypertokenize_source_file(
            "a.cpp",
            "struct S {\n  void (*xFunc)(int);\n  int (*const fp)(int);\n  \
             static int (max)();\n  void (*(*nested)(int))(int);\n  \
             void (Foo::*mMethod)();\n};\n\
             void (*signal(int sig, void (*func)(int)))(int) { return 0; }\n",
        )
        .unwrap()
        .structure
        .into_iter()
        .map(|row| row.pretty)
        .collect();
        assert_eq!(
            prettys,
            vec![
                "S",
                "S::xFunc",
                "S::fp",
                "S::max",
                "S::nested",
                "S::mMethod",
                "signal"
            ]
        );
        // Enums are containers where they're defined, not where they're
        // mentioned (ex: parameters' and fields' types, and forward
        // declarations).
        let prettys: Vec<String> = hypertokenize_source_file(
            "a.cpp",
            "enum class Mode : uint8_t;\nenum Color { Red, Green };\n\
             struct T {\n  enum Color mColor;\n  void Set(enum Color aColor);\n};\n",
        )
        .unwrap()
        .structure
        .into_iter()
        .map(|row| row.pretty)
        .collect();
        assert_eq!(prettys, vec!["Color", "T", "T::mColor", "T::Set"]);
        // Classes too (ex: forward declarations, and `friend class`), but
        // explicit instantiations, which are definitions, are.
        let prettys: Vec<String> = hypertokenize_source_file(
            "a.cpp",
            "class nsIFoo;\nclass Foo {\n  friend class Bar;\n  class Baz mBaz;\n};\n\
             template class Holder<Foo>;\n",
        )
        .unwrap()
        .structure
        .into_iter()
        .map(|row| row.pretty)
        .collect();
        assert_eq!(prettys, vec!["Foo", "Foo::mBaz", "Holder<Foo>"]);
        // Fields are their own contexts whatever their declarators (with their
        // annotations, ex: `MOZ_GUARDED_BY`).
        let tokenized = hypertokenize_source_file(
            "a.cpp",
            "struct S {\n  int mA;\n  Bar* mP MOZ_GUARDED_BY(mMutex);\n  Foo& mR;\n  \
             int mArr[4];\n  const char* const mName;\n};\n",
        )
        .unwrap();
        let prettys: Vec<&str> = tokenized
            .structure
            .iter()
            .map(|row| row.pretty.as_str())
            .collect();
        assert_eq!(
            prettys,
            vec!["S", "S::mA", "S::mP", "S::mR", "S::mArr", "S::mName"]
        );
        assert!(
            tokenized
                .tokenized
                .iter()
                .any(|line| line == "S::mP i MOZ_GUARDED_BY"),
            "{:?}",
            tokenized.tokenized
        );
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

    /// tree-sitter-mozcpp's macros, and preprocessor conditionals inside
    /// declarations (see `tokenize_cpp`), don't throw off the structure.
    #[test]
    fn test_cpp_macros_and_conditionals() {
        let source = "namespace mozilla {\n\
                      class Foo final : public nsIRunnable {\n \
                       public:\n  \
                        NS_DECL_ISUPPORTS\n  \
                        NS_DECL_NSIRUNNABLE\n\
                      };\n\
                      Foo::Foo()\n    \
                          : mA(1)\n\
                      #ifdef DEBUG\n      \
                            ,\n      \
                            mB(2)\n\
                      #endif\n\
                      {\n\
                      }\n\
                      #ifdef XP_WIN\n\
                      void Bar() { Win(); }\n\
                      #else\n\
                      void Bar() { Posix(); }\n\
                      #endif  // XP_WIN\n\
                      nsIPrincipal* GetPrincipal() { return nullptr; }\n\
                      class WorkerPrivate::EventTarget final {};\n\
                      /*\n\
                      #if 0\n\
                      */\n\
                      }  // namespace mozilla\n";
        assert_eq!(
            structure("a.cpp", source),
            vec![
                "namespace:mozilla",
                "class:mozilla::Foo",
                "method:mozilla::Foo::Foo",
                "method:mozilla::Bar",
                "method:mozilla::Bar",
                "method:mozilla::GetPrincipal",
                "class:mozilla::WorkerPrivate::EventTarget",
            ]
        );
        let tokenized = hypertokenize_source_file("a.cpp", source).unwrap();
        // The rows have their conditionals.
        assert_eq!(
            tokenized
                .structure
                .iter()
                .map(|row| row.pp.join(" | "))
                .collect::<Vec<_>>(),
            vec!["", "", "", "defined(XP_WIN)", "!defined(XP_WIN)", "", ""]
        );
        for expected in [
            "mozilla::Foo i NS_DECL_NSIRUNNABLE",
            // (The directives in the constructor's initializers are its.)
            "mozilla::Foo::Foo k #ifdef",
            "mozilla::Foo::Foo i DEBUG",
            "mozilla::Foo::Foo i mB",
            "mozilla::Foo::Foo k #endif",
            "mozilla k #else",
            "mozilla::Bar i Posix",
            "mozilla c XP_WIN",
            // (A directive in a block comment is the comment's.)
            "mozilla c #if",
        ] {
            assert!(
                tokenized.tokenized.iter().any(|line| line == expected),
                "{} in {:#?}",
                expected,
                tokenized.tokenized
            );
        }
        // The tokens are in order, with their offsets.
        let mut previous = 0;
        for (line, offset) in tokenized.tokenized.iter().zip(&tokenized.offsets) {
            let token = split_token_line(line).token;
            let offset = offset.unwrap() as usize;
            assert_eq!(source.get(offset..offset + token.len()), Some(token));
            assert!(offset >= previous, "{:?}", line);
            previous = offset + token.len();
        }
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
                "c:///",
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

    /// Rust's plain comments' words, items in macros (see
    /// `rust_item_macro_body`), and functions in `extern` blocks.
    #[test]
    fn test_rust_comments_and_macro_items() {
        let source = "mod m {\n\
                      // Plain words\n\
                      feature! {\n    \
                          #![feature = \"fs\"]\n    \
                          pub fn open() -> u32 { 1 }\n\
                      }\n\
                      macro_rules! m { ($x:expr) => { $x }; }\n\
                      extern \"C\" { fn malloc(size: usize) -> *mut u8; }\n\
                      }\n";
        assert_eq!(
            structure("a.rs", source),
            vec!["namespace:m", "method:m::open", "method:m::malloc"]
        );
        let classes = classes("a.rs", source);
        for expected in [
            "c://",
            "c:Plain",
            "c:words",
            "i:feature",
            "o:!",
            "o:{",
            "k:fn",
            "i:open",
            "o:}",
        ] {
            assert!(
                classes.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                classes
            );
        }
    }

    /// JS's private methods are containers, and its strings are single
    /// tokens (but not template strings, whose substitutions are code).
    #[test]
    fn test_js_private_methods_and_strings() {
        let source = "class C {\n  #restore() { return 'a b' + `c ${d}`; }\n}\n";
        assert_eq!(
            structure("a.js", source),
            vec!["class:C", "method:C::#restore"]
        );
        let classes = classes("a.js", source);
        assert!(classes.iter().any(|c| c == "s:'a b'"), "{:?}", classes);
        assert!(classes.iter().any(|c| c == "i:d"), "{:?}", classes);
    }

    /// XPIDL's interfaces and members are containers, its code blocks are
    /// tokenized as C++, and historical XPIDL's preprocessor lines are
    /// comments' words.
    #[test]
    fn test_xpidl() {
        let source = "#ifndef nsIFoo_h__\n\
                      #include \"nsISupports.idl\"\n\
                      %{C++\n\
                      inline bool IsFoo(int aX) { return aX > 1; }\n\
                      %}\n\
                      [scriptable, uuid(a88e5a60-205a-4bb1-94e1-2628daf51eae)]\n\
                      interface nsIFoo : nsISupports {\n  \
                        const unsigned long FLAG = 1 << 2;\n  \
                        readonly attribute AString name;\n  \
                        void go(in long aCount);\n\
                      };\n\
                      [ref] native StdFunction(std::function<void(int)>);\n";
        assert_eq!(
            structure("a.idl", source),
            vec![
                "method:IsFoo",
                "class:nsIFoo",
                "field:nsIFoo::FLAG",
                "field:nsIFoo::name",
                "method:nsIFoo::go",
                "typedef:StdFunction",
            ]
        );
        let classes = classes("a.idl", source);
        for expected in [
            "s:\"nsISupports.idl\"",
            "k:inline",
            "i:IsFoo",
            "k:interface",
            "i:nsIFoo",
            "k:readonly",
            "k:attribute",
            "t:std::function<void(int)>",
            "c:#ifndef",
            "c:nsIFoo_h__",
        ] {
            assert!(
                classes.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                classes
            );
        }
    }

    /// Objective-C++'s Objective-C sections are parsed with tree-sitter-objc
    /// and the rest as C++, and Objective-C's both with tree-sitter-objc.
    #[test]
    fn test_objc() {
        let source = "#import <Cocoa/Cocoa.h>\n\
                      namespace mozilla {\n\
                      void Foo(const nsTArray<int>& aArray) { Bar(); }\n\
                      }  // namespace mozilla\n\
                      @interface PixelHostingView : NSView\n\
                      - (id)initWithFrame:(NSRect)inFrame geckoChild:(int)inChild;\n\
                      @end\n\
                      @implementation PixelHostingView\n\
                      - (id)initWithFrame:(NSRect)inFrame geckoChild:(int)inChild {\n  \
                        return [super initWithFrame:inFrame];\n\
                      }\n\
                      @end\n";
        assert_eq!(
            structure("a.mm", source),
            vec![
                "namespace:mozilla",
                "method:mozilla::Foo",
                "class:PixelHostingView",
                "method:PixelHostingView::initWithFrame",
                "class:PixelHostingView",
                "method:PixelHostingView::initWithFrame",
            ]
        );
        let c_source = "static int Helper(int a) { return [obj count] + a; }\n\
                        @implementation Foo\n\
                        + (void)reset { NSLog(@\"hi\"); }\n\
                        @end\n";
        assert_eq!(
            structure("a.m", c_source),
            vec!["method:Helper", "class:Foo", "method:Foo::reset"]
        );
        let m_classes = classes("a.m", c_source);
        assert!(
            m_classes.iter().any(|c| c == "s:@\"hi\""),
            "{:?}",
            m_classes
        );
        // Objective-C's statements and expressions in C++ functions are the
        // C++'s, not sections of their own, so the `@try` and the
        // `@protocol(...)` don't end or start them.
        let statements = "void Foo(id aObj) {\n  \
                          @try {\n    \
                          [aObj bar];\n  \
                          } @catch (NSException* e) {\n  \
                          }\n  \
                          Use(@\"a\"\n      \
                          @\"b\",\n      \
                          @protocol(NSObject));\n\
                          }\n\
                          void Bar() { for (id x in Items()) { Use(x); } }\n";
        assert!(objc_sections(statements).is_empty());
        assert_eq!(
            objc_sections("@class Foo;\nvoid F();\n@protocol Bar\n- (void)x;\n@end\n"),
            vec![0..12, 22..52]
        );
        assert_eq!(
            structure("a.mm", statements),
            vec!["method:Foo", "method:Bar"]
        );
        let mm_classes = classes("a.mm", statements);
        assert!(
            mm_classes.iter().any(|c| c == "s:@\"b\""),
            "{:?}",
            mm_classes
        );
        // Objective-C++'s methods' bodies are C++, in the methods' contexts,
        // and methods in conditionals are parsed in their first branches.
        let methods = "@implementation Foo\n\
                       #ifdef ACCESSIBILITY\n\
                       - (id)accessible {\n  \
                         RefPtr<a11y::Acc> acc = a11y::Get(self);\n  \
                         return [acc native];\n\
                       }\n\
                       #endif\n\
                       - (void)bar {\n  \
                         gfx::IntPoint p = GetPoint(@selector(baz:qux:));\n\
                       }\n\
                       @end\n";
        assert_eq!(
            structure("a.mm", methods),
            vec!["class:Foo", "method:Foo::accessible", "method:Foo::bar"]
        );
        let contexts: Vec<String> = hypertokenize_source_file("a.mm", methods)
            .unwrap()
            .tokenized
            .iter()
            .map(|line| {
                let parsed = split_token_line(line);
                format!("{}@{}", parsed.token, parsed.context)
            })
            .collect();
        for expected in [
            "a11y@Foo::accessible",
            "native@Foo::accessible",
            "IntPoint@Foo::bar",
            "@selector(baz:qux:)@Foo::bar",
        ] {
            assert!(
                contexts.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                contexts
            );
        }

        // Headers with Objective-C declarations are tokenized like
        // Objective-C++: but not Doxygen's `@class` in a comment, and with
        // forward declarations of protocols through their `;`, and
        // libwebrtc's `RTC_OBJC_TYPE(...)`, whose tokens get the contexts at
        // them.
        let header = "#include \"foo.h\"\n\
                      /**\n @class Doc\n */\n\
                      @class NSView;\n\
                      @protocol RTC_OBJC_TYPE\n(RTCVideoRenderer);\n\
                      namespace webrtc {\n\
                      class Renderer {};\n\
                      }\n\
                      RTC_OBJC_EXPORT\n\
                      @protocol RTC_OBJC_TYPE\n(RTCVideoEncoder)<NSObject>\n\
                      - (void)setCallback:(int)callback;\n\
                      - (void)encode:(RTC_OBJC_TYPE(RTCVideoFrame) *)frame;\n\
                      @end\n";
        assert_eq!(
            structure("a.h", header),
            vec![
                "namespace:webrtc",
                "class:webrtc::Renderer",
                "class:RTCVideoEncoder",
                "method:RTCVideoEncoder::setCallback",
                "method:RTCVideoEncoder::encode",
            ]
        );
        let tokens: Vec<String> = hypertokenize_source_file("a.h", header)
            .unwrap()
            .tokenized
            .iter()
            .map(|line| {
                let parsed = split_token_line(line);
                format!(
                    "{}:{}@{}",
                    parsed.class.as_char(),
                    parsed.token,
                    parsed.context
                )
            })
            .collect();
        for expected in [
            "c:@class@%",
            "i:RTC_OBJC_TYPE@RTCVideoEncoder",
            "i:RTCVideoEncoder@RTCVideoEncoder",
            "i:RTC_OBJC_TYPE@RTCVideoEncoder::encode",
            "i:RTCVideoFrame@RTCVideoEncoder::encode",
        ] {
            assert!(
                tokens.iter().any(|t| t == expected),
                "{} in {:?}",
                expected,
                tokens
            );
        }

        // C++ in Objective-C++'s declarations (instance variables' and
        // parameters' types) is tokenized as C++, but not in strings.
        let cpp_types = "@implementation Foo {\n  \
                         std::unique_ptr<Bar> mBar;\n\
                         }\n\
                         - (void)setBar:(std::unique_ptr<Bar>)bar {\n  \
                         mBar = std::move(bar);\n\
                         }\n\
                         - (void)log { NSLog(@\"RTC_OBJC_TYPE(Foo)\"); }\n\
                         @end\n";
        assert_eq!(
            structure("a.mm", cpp_types),
            vec!["class:Foo", "method:Foo::setBar", "method:Foo::log"]
        );
        let mm_classes = classes("a.mm", cpp_types);
        for expected in ["i:unique_ptr", "o:::", "s:@\"RTC_OBJC_TYPE(Foo)\""] {
            assert!(
                mm_classes.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                mm_classes
            );
        }
    }

    /// `SCOPE_MACROS` before blocks don't make the blocks compound
    /// literals: their statements are statements, and the macros are
    /// identifiers, but not in comments.
    #[test]
    fn test_scope_macros() {
        let source = "void F() {\n  \
                      ANGLE_MTL_OBJC_SCOPE\n  \
                      {\n    \
                      if (a) { G(); } else { H(); }\n  \
                      }\n  \
                      // ANGLE_MTL_OBJC_SCOPE {\n\
                      }\n";
        let classes = classes("a.mm", source);
        for expected in [
            "i:ANGLE_MTL_OBJC_SCOPE",
            "k:if",
            "k:else",
            "c:ANGLE_MTL_OBJC_SCOPE",
        ] {
            assert!(
                classes.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                classes
            );
        }
    }

    /// Dead branches (`#if 0`'s, and `#if 1`'s `#else`), with any
    /// conditionals in them, are comments' words, in the contexts at them,
    /// and their conditionals' own directives are directives.
    #[test]
    fn test_dead_branches() {
        let source = "namespace ns {\n\
                      void F() {\n\
                      #if 0\n  \
                      Old(a, \"b\");\n\
                      #  ifdef X\n  \
                      Older();\n\
                      #  endif\n\
                      #else\n  \
                      New();\n\
                      #endif\n\
                      }\n\
                      #if 1\n\
                      void G() {}\n\
                      #else\n\
                      void H() {}\n\
                      #endif\n\
                      }\n";
        assert_eq!(
            structure("a.cpp", source),
            vec!["namespace:ns", "method:ns::F", "method:ns::G"]
        );
        let tokens: Vec<String> = hypertokenize_source_file("a.cpp", source)
            .unwrap()
            .tokenized
            .iter()
            .map(|line| {
                let parsed = split_token_line(line);
                format!(
                    "{}:{}@{}",
                    parsed.class.as_char(),
                    parsed.token,
                    parsed.context
                )
            })
            .collect();
        for expected in [
            "k:#if@ns::F",
            "c:Old(a,@ns::F",
            "c:ifdef@ns::F",
            "k:#else@ns::F",
            "i:New@ns::F",
            "c:H()@ns",
        ] {
            assert!(
                tokens.iter().any(|t| t == expected),
                "{} in {:?}",
                expected,
                tokens
            );
        }
    }

    /// C++'s conversion operators are containers named by their types,
    /// without their parameters and qualifiers.
    #[test]
    fn test_conversion_operators() {
        let source = "class Foo {\n  \
                      explicit operator bool() const { return mX; }\n  \
                      operator const char*() const;\n  \
                      operator Error&() { return mError; }\n\
                      };\n\
                      Foo::operator bool() const { return true; }\n\
                      template <typename T> Foo::operator T*() { return nullptr; }\n";
        assert_eq!(
            structure("a.cpp", source),
            vec![
                "class:Foo",
                "method:Foo::operator%20bool",
                "field:Foo::operator%20const%20char*",
                "method:Foo::operator%20Error&",
                "method:Foo::operator%20bool",
                "method:Foo::operator%20T*",
            ]
        );
    }

    /// JS and TS class fields and Rust's named fields are containers, with
    /// their comments, attributes, and decorators (and trailing comments after
    /// their separators).
    #[test]
    fn test_js_rust_fields() {
        let js = "class Foo {\n  \
                  // The count.\n  \
                  count = 0; // (Zero.)\n  \
                  handleClick = () => { this.count++; };\n  \
                  static #secret = 1;\n  \
                  bar() {}\n\
                  }\n";
        assert_eq!(
            structure("a.js", js),
            vec![
                "class:Foo",
                "field:Foo::count",
                "field:Foo::handleClick",
                "field:Foo::#secret",
                "method:Foo::bar",
            ]
        );
        let contexts = |filename: &str, source: &str| -> Vec<String> {
            hypertokenize_source_file(filename, source)
                .unwrap()
                .tokenized
                .iter()
                .map(|line| {
                    let parsed = split_token_line(line);
                    format!("{}@{}", parsed.token, parsed.context)
                })
                .collect()
        };
        let js_contexts = contexts("a.js", js);
        for expected in [
            "The@Foo::count",
            "(Zero.)@Foo::count",
            "this@Foo::handleClick",
        ] {
            assert!(
                js_contexts.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                js_contexts
            );
        }
        assert_eq!(
            structure(
                "a.ts",
                "class Foo {\n  @observable\n  private name: string = \"x\";\n}\n"
            ),
            vec!["class:Foo", "field:Foo::name"]
        );
        let rust = "pub struct Foo {\n    \
                    /// The count.\n    \
                    #[serde(rename = \"c\")]\n    \
                    pub count: u32,\n    \
                    name: String, // its name\n\
                    }\n\
                    pub struct T(u32);\n";
        assert_eq!(
            structure("a.rs", rust),
            vec![
                "struct:Foo",
                "field:Foo::count",
                "field:Foo::name",
                "struct:T"
            ]
        );
        let rust_contexts = contexts("a.rs", rust);
        for expected in ["The@Foo::count", "serde@Foo::count", "its@Foo::name"] {
            assert!(
                rust_contexts.iter().any(|c| c == expected),
                "{} in {:?}",
                expected,
                rust_contexts
            );
        }
    }

    /// ERROR nodes which error recovery makes extras (like comments) aren't
    /// comments: their tokens are classed as they'd be otherwise.
    #[test]
    fn test_extra_errors_are_not_comments() {
        assert_eq!(
            classes("a.cpp", "mozilla::dom::Foo*"),
            vec!["i:mozilla", "o:::", "i:dom", "o:::", "i:Foo", "o:*"]
        );
    }

    /// Files' tokens cover their text (but whitespace): tokens that grammars
    /// leave out of their trees (ex: anonymous regexes, and external
    /// scanners' delimiters), and text that error recovery leaves out of
    /// them, would be lost.
    #[test]
    fn test_tokens_cover_text() {
        for (filename, source) in [
            // (tree-sitter-cpp's `= 0`'s `0`.)
            ("a.cpp", "class A {\n  virtual ~A() = 0;\n};\n"),
            // (tree-sitter-xpidl's code blocks' delimiters.)
            ("a.idl", "interface nsIFoo {\n%{C++\n  int x;\n%}\n};\n"),
            // (Strings' contents around escape sequences, and f-strings'
            // format specifiers.)
            ("a.py", "x = \"fail in a\\n future\"\ny = f\"{a:>10}\"\n"),
            // (tree-sitter-ipdl's comments' text.)
            ("a.ipdl", "// a comment\n/* MPL\n * x */\nprotocol P {};\n"),
            // (Characters' contents, and an ERROR node with only its `}`, and
            // tree-sitter-kotlin-ng's hidden tokens: `;`s, `!is`'s `!`, and a
            // `?` before a comment.)
            ("a.kt", "val a = it == '-'\n"),
            ("b.kt", "}\nprivate fun Intent.strip() {"),
            (
                "c.kt",
                "fun f() { a(); b() }\n\
                 val c = x !is Foo\n\
                 val d = y as? Bar? // e\n",
            ),
            // (Raw strings' delimiters, and macro repetitions' separators.)
            ("a.rs", "fn f() { let a = r#\"x y\"#; let b = r\"z\"; }\n"),
            (
                "b.rs",
                "macro_rules! m { ($($e:expr),+) => { $($e);* }; }\n",
            ),
        ] {
            let tokens: String = hypertokenize_source_file(filename, source)
                .unwrap()
                .tokenized
                .iter()
                .flat_map(|line| split_token_line(line).token.chars().collect::<Vec<_>>())
                .filter(|c| !c.is_whitespace())
                .collect();
            let text: String = source.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(tokens, text, "{}", filename);
        }
    }

    /// Comments (and Rust attributes) which belong to containers next to them
    /// have their contexts (see `trivia_container`).
    #[test]
    fn test_comment_contexts() {
        // Each comment's marker (its first word) and context.
        let comments = |filename: &str, source: &str| -> Vec<String> {
            hypertokenize_source_file(filename, source)
                .unwrap()
                .tokenized
                .iter()
                .map(|line| split_token_line(line))
                .filter(|parsed| {
                    matches!(parsed.class, TokenClass::Comment | TokenClass::Boilerplate)
                        && parsed.token.starts_with('/')
                })
                .map(|parsed| format!("{}@{}", parsed.token, parsed.context))
                .collect()
        };
        assert_eq!(
            comments(
                "a.cpp",
                "/* License. */
namespace mozilla {

// About Foo.
class Foo {
  \
                 // Accessors

  // The count.
  int mCount;
  int mX;  // The x.
  \
                 /**\n   * Does it.\n   */\n  void Do();\n};\n\n}  // namespace mozilla\n"
            ),
            vec![
                "/*@%",
                "//@mozilla::Foo",
                "//@mozilla::Foo",
                "//@mozilla::Foo::mCount",
                "//@mozilla::Foo::mX",
                "/**@mozilla::Foo::Do",
                "//@mozilla",
            ]
        );
        let tokenized = hypertokenize_source_file(
            "a.rs",
            "// License.\n\n/// The S.\n#[derive(Debug)]\nstruct S {}\n",
        )
        .unwrap();
        assert!(
            tokenized.tokenized.iter().any(|line| line == "S i derive"),
            "{:?}",
            tokenized.tokenized
        );
        assert!(
            tokenized.tokenized.iter().any(|line| line == "S c ///"),
            "{:?}",
            tokenized.tokenized
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
