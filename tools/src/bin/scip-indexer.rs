extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate protobuf;
extern crate scip;
extern crate tools;

use clap::Parser;
use lazy_static::lazy_static;
use regex::Regex;
use scip::types::descriptor::Suffix;
use serde_json::Map;
use std::collections::{HashMap, BTreeSet};
use std::fs::{self, File};
use std::io;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tools::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisStructured, AnalysisTarget, LineRange, Location,
    SourceRange, SourceTag, StructuredFieldInfo, StructuredMethodInfo, StructuredOverrideInfo,
    StructuredSuperInfo, StructuredTag, TargetTag, WithLocation,
};
use tools::file_format::config;
use ustr::{ustr, Ustr, UstrMap};

/// Normalize illegal symbol characters into underscores.
fn sanitize_symbol(sym: &str) -> String {
    // Downstream processing of the symbol doesn't deal well with
    // these characters, so replace them with underscores.
    fn is_special_char(c: char) -> bool {
        matches!(c, ',' | ' ' | '\n')
    }
    sym.replace(is_special_char, "_").trim_matches('_').into()
}

fn create_output_dir(output_file: &Path) -> io::Result<()> {
    let mut output_dir = output_file.to_owned();
    output_dir.pop();
    fs::create_dir_all(output_dir)
}

#[derive(Parser)]
struct ScipIndexerCli {
    /// Path to the variable-expanded config file
    #[clap(value_parser)]
    config_file: String,

    /// The tree in the config file we're cross-referencing
    #[clap(value_parser)]
    tree_name: String,

    #[arg(long, value_parser)]
    subtree_name: Option<String>,

    /// Relative path from the root of the searchfox source tree to the SCIP
    /// index's contents.  If the SCIP index is from the root then this should
    /// be ".", otherwise it should be the relative path like "js-subtree/".
    #[arg(long, value_parser)]
    subtree_root: String,

    /// Platform name if this is per-platform.
    #[arg(long, value_parser)]
    platform: Option<String>,

    /// rustc analysis directories or scip inputs
    #[arg(value_parser)]
    inputs: Vec<PathBuf>,
}

// https://docs.rs/scip/latest/scip/types/struct.Occurrence.html#structfield.range
fn scip_range_to_searchfox_location(range: &[i32]) -> Location {
    // Searchfox uses 1-indexed lines, 0-indexed columns.
    let line_start = range[0] as u32 + 1;
    let col_start = range[1] as u32;
    let line_end = if range.len() == 3 {
        line_start
    } else {
        range[2] as u32 + 1
    };
    let col_end = *range.last().unwrap() as u32;
    // Rust spans are multi-line... So we just use the start column as
    // the end column if it spans multiple rows, searchfox has fallback
    // code to handle this.
    let col_end = if line_start != line_end {
        col_start
    } else {
        col_end
    };
    Location {
        lineno: line_start,
        col_start,
        col_end,
    }
}

fn node_range_to_searchfox_location(range: tree_sitter::Range) -> Location {
    // Searchfox uses 1-indexed lines, 0-indexed columns while tree-sitter uses
    // 0-based for both.
    Location {
        lineno: range.start_point.row as u32 + 1,
        col_start: range.start_point.column as u32,
        col_end: range.end_point.column as u32,
    }
}
fn node_range_to_searchfox_range(range: tree_sitter::Range) -> SourceRange {
    // Searchfox uses 1-indexed lines, 0-indexed columns while tree-sitter uses
    // 0-based for both.
    SourceRange {
        start_lineno: range.start_point.row as u32 + 1,
        start_col: range.start_point.column as u32,
        end_lineno: range.end_point.row as u32 + 1,
        end_col: range.end_point.column as u32,
    }
}

fn write_line(mut file: &mut File, data: &impl serde::Serialize) {
    use std::io::Write;
    serde_json::to_writer(&mut file, data).unwrap();
    file.write_all(b"\n").unwrap();
}

fn scip_roles_to_searchfox_analysis_kind(roles: i32) -> AnalysisKind {
    macro_rules! map_to_searchfox {
        ($scip:ident, $sfox:ident) => {
            if roles & scip::types::SymbolRole::$scip as i32 != 0 {
                return AnalysisKind::$sfox;
            }
        };
    }

    map_to_searchfox!(Definition, Def);
    map_to_searchfox!(Import, Use);
    // Read/Write would be pretty neat to have, but neither rust-analyzer or
    // scip-typescript generates these values.
    map_to_searchfox!(WriteAccess, Use);
    map_to_searchfox!(ReadAccess, Use);
    map_to_searchfox!(Generated, Use);
    // This would be very interesting if it works for rust-analyzer, as our
    // current file-level granularity for determining the pathkind for grouping
    // obviously will be wrong for tests in the source file, which is definitely
    // a rust idiom.   (Also a python idiom too!)
    map_to_searchfox!(Test, Use);
    map_to_searchfox!(Import, Use);

    return AnalysisKind::Use;
}

/// Our specifically handled languages for conditional logic.  We currently
/// require tree-sitter support for all supported languages.
enum ScipLang {
    Python,
    Rust,
    Typescript,
    Jvm,
}

enum PrettyAction {
    /// Don't include this in the pretty, but keep using what we've built.
    Omit,
    /// Append this to the current list of pieces.
    Append,
    /// Reset the list and use this as the first entry of the new list.
    ResetAndUse,
    /// The symbol name is known to be useless and some other means of inferring
    /// a pretty name (Ex: from the documentation) must be used.  Reset the list
    /// and use the alternate name source.  Note that it's expected this will be
    /// the last directive expected; re-evaluate if you need to use this rule
    /// more than once.
    UseAlternateSource,
}

/// Helper structure that we use to populate our tree-sitter queries.  See the
/// block comment in `analyze_using_scip` for context.
///
/// This structure captures the relevant data for queries that help us know
/// which SCIP occurrence should be the definition that starts a nesting range
/// both for position:sticky purposes and for context/contextsym purposes.
/// In general this just means knowing the node name for a given construct plus
/// the field names for the name and body.  Frequently the field name is "name",
/// but sometimes it can be something like "type".  Usually the body is "body".
///
/// Note that the query syntax is quite powerful and the names of captures could
/// potentially be used to encode metadata, so if making any changes to the type
/// here or adding a whole bunch of additional instances below, it's probably
/// worth considering moving to using externally stored ".scm" files which
/// have semantic capture groups instead of this current approach.  In
/// particular, alternations could be invaluable.  Also, we should avoid doing
/// anything that resembles reinventing `tree-sitter-highlight`.
///
/// The current approach has been chosen for prototyping expediency and for
/// ease of adding sidecar data, but as noted above and after having done
/// additional research, especially around conventions (and limited rust binding
/// support) for `#`-prefixed predicates, it's clear this is not the path
/// forward without a very good reason.  (Readability could be a good reason;
/// the s-expr syntax is powerful but probably unfamiliar to most people.  That
/// said, its use around tree-sitter is so common that it seems like a
/// reasonable thing to understand if touching tree-sitter related code.)
///
/// A reasonable hybrid approach, depending on how easy it is to add custom
/// predicates to the rust bindings, is to embed full s-exprs here but leave
/// our sidecar structure so that we can just use rust logic for what would
/// otherwise be potentially complex predicates.
struct SitterNesting {
    root_node_type: Vec<&'static str>,
    name_field: &'static str,
    body_field: &'static str,
}

lazy_static! {
    // our list is manually derived from the tags.scm file:
    // https://github.com/tree-sitter/tree-sitter-python/blob/master/queries/tags.scm
    static ref PYTHON_NESTING: Vec<SitterNesting> = vec![
        SitterNesting {
            root_node_type: vec!["class_definition"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["function_definition"],
            name_field: "name",
            body_field: "body",
        },
    ];
    // our list is manually derived from the tags.scm file:
    // https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm
    static ref RUST_NESTING: Vec<SitterNesting> = vec![
        SitterNesting {
            root_node_type: vec!["struct_item"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["enum_item"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["union_item"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["function_item"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["trait_item"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["mod_item"],
            name_field: "name",
            body_field: "body",
        },
        // TODO macro_definition lacks a body so the body needs to be the parent
        // node maybe?
        //
        // impl can also have a "trait" field, but symbol-wise I think the
        // important symbol for context is the struct type not the trait type.
        SitterNesting {
            root_node_type: vec!["impl_item"],
            name_field: "type",
            body_field: "body",
        },
    ];
    // tree-sitter support for typescript is a little weird because the
    // typescript languages (typescript and tsx) extend the javascript
    // language.
    //
    // for now we manually derive these from both base queries tags.scm files:
    // https://github.com/tree-sitter/tree-sitter-javascript/blob/master/queries/tags.scm
    // https://github.com/tree-sitter/tree-sitter-typescript/blob/master/queries/tags.scm
    static ref JS_NESTING: Vec<SitterNesting> = vec![
        // ### from the JS tags
        SitterNesting {
            root_node_type: vec!["method_definition"],
            name_field: "name",
            body_field: "body",
        },
        // There's an alt over class and class_declaration; class_declaration
        // becomes "class" if we add a "let blah = " ahead of it (and it stops
        // being a declaration).
        SitterNesting {
            root_node_type: vec!["class", "class_declaration"],
            name_field: "name",
            body_field: "body",
        },
        // There's also an alt over function/generators
        SitterNesting {
            root_node_type: vec![
                "function",
                "function_declaration",
                "generator_function",
                "generator_function_declaration"
            ],
            name_field: "name",
            body_field: "body",
        },
        // TODO: tags.scm has logic for lexical binds on arrow functions and
        // this is worth considering, although arguably this might also resemble
        // the lambda case.  But this is also beyond our current approach with
        // generated syntax.  Also, conceptually, the arrow functions should
        // already exist within a nesting scope, which raises the question of
        // whether it's actually desirable to treat them like C++ lambdas which
        // we also currently fold in.
        // TODO: There's also property-defined arrow functions.
        //
        // ### from the TS tags
        // XXX skipping function_signature because it lacks a directly available
        // body.
        // XXX skipping (abstract_)method_signature because it lacks a directly
        // available body.
        SitterNesting {
            root_node_type: vec!["abstract_class_declaration"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["module"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["interface_declaration"],
            name_field: "name",
            body_field: "body",
        },
    ];
    // https://github.com/tree-sitter/tree-sitter-java/blob/master/queries/tags.scm
    static ref JAVA_NESTING: Vec<SitterNesting> = vec![
        SitterNesting {
            root_node_type: vec!["class_declaration"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["method_declaration"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["interface_declaration"],
            name_field: "name",
            body_field: "body",
        },
    ];
    // no tags.scm in tree-sitter-kotlin
    static ref KOTLIN_NESTING: Vec<SitterNesting> = vec![
        SitterNesting {
            root_node_type: vec!["class_declaration"],
            name_field: "name",
            body_field: "body",
        },
        SitterNesting {
            root_node_type: vec!["function_declaration"],
            name_field: "name",
            body_field: "body",
        },
    ];
}

struct NestedSymbol {
    /// The symbol covering this nested range that should be used for
    /// contextsym.
    sym: Ustr,
    /// The pretty identifier of that symbol.
    pretty: Ustr,
    /// The range it covers; we really only need the last line, but include this
    /// for debugging.
    nesting_range: SourceRange,
}

fn compile_nesting_queries(
    lang: tree_sitter::Language,
    nesting: &Vec<SitterNesting>,
) -> tree_sitter::Query {
    let query_pats: Vec<String> = nesting
        .iter()
        .map(|ndef| {
            let mut parts = vec![];
            let mut indent = "";
            if ndef.root_node_type.len() > 1 {
                parts.push("[".to_string());
                indent = "  ";
            }
            for node_type in &ndef.root_node_type {
                parts.push(format!(
                    "{}({} {}: (_) @name {}: (_) @body)",
                    indent, node_type, ndef.name_field, ndef.body_field
                ));
            }
            if ndef.root_node_type.len() > 1 {
                parts.push("]".to_string());
            }
            parts.join("\n")
        })
        .collect();
    tree_sitter::Query::new(lang, &query_pats.join("\n")).unwrap()
}

// ### Process SCIP Descriptors
//
// Map the descriptor individually to build up canonical
// mozsearch symbol name.  In general, we maintain the SCIP
// descriptor syntax because there's no reason not to.  The main
// exception is we don't re-wrap backtick-enclosed values.
// (Interestingly, the scip lib's format_symbol_with method also
// does not re-wrap descriptors, but this is probably an
// oversight.)
//
// Note that this implementation inherently looks a lot like
// part of the scip crate's format_symbol_with impl out of
// necessity (we're reversing an explicit spec).  We don't call
// that method with a format option to just request the
// descriptors because that method is destructive and we both
// want to ensure stability and control of this mapping, like
// not emitting backticks.  This is very much a one-way mapping
// with policy decisions made here.
struct SymbolAnalysis {
    kind: Option<&'static str>,
    pretty: Ustr,
    norm_sym: Ustr,
    parent_sym: Option<Ustr>,
    contributes_to_parent: bool,
}

fn symbol_name(lang_name: &str, subtree_name: Option<&str>, scip_symbol: &str) -> Ustr {
    if let Some(subtree_name) = subtree_name {
        ustr(&format!(
            "S_{}_{}_{}",
            lang_name,
            subtree_name,
            scip_symbol
        ))
    } else {
        ustr(&format!(
            "S_{}_{}",
            lang_name,
            scip_symbol
        ))
    }
}

fn analyse_symbol(
    symbol: &scip::types::Symbol,
    lang: &ScipLang,
    lang_name: &str,
    subtree_name: Option<&str>,
    relative_path: &str,
    doc_name: Option<&str>,
    doc_namespace: Option<&str>)
-> SymbolAnalysis {
    let mut pretty_pieces = vec![];
    let mut sym_pieces = vec![];
    let mut last_kind = None;
    let mut last_contributes_to_parent = false;
    let mut prev_kind = None;

    for descriptor in &symbol.descriptors {
        // Ignore descriptor enums from the future, skipping them.
        let suffix = match descriptor.suffix.enum_value() {
            // UnspecifiedSuffix is weird because it's an in-domain
            // value (it's explicitly part of the protobuf schema
            // for suffix), but since currently the suffix is only
            // built by parsing a string encoding of the enum, it
            // logically is similar in nature to the enum_value
            // being an Err.  Regardless, we skip it.
            Ok(Suffix::UnspecifiedSuffix) => {
                warn!("Experienced unspecified suffix on {}", symbol);
                continue;
            }
            Ok(v) => v,
            Err(_) => {
                warn!("Experienced weird suffix error on {}", symbol);
                continue;
            }
        };
        let escaped = sanitize_symbol(&descriptor.name);

        let (sym_piece, pretty_action, maybe_kind, contributes_to_parent) = match suffix
        {
            // Confusingly, package is deprecated in favor of
            // namespace, but right now the SCIP crate parses '/'
            // as Package, not Namespace.
            Suffix::Package | Suffix::Namespace => {
                // Pretty: For JS/TS the namespace includes the file path which
                // ends up way too verbose and now how humans would describe
                // things.
                //
                // TODO: Handle scip-typescript emitting symbols for file names
                // since our explicit heuristic above ends up leaving them
                // entirely with an empty pretty, and in that case we do want
                // to emit the path as a pretty, but we also want to emit a
                // "FILE_" symbol instead of a scip "S_" symbol.
                //
                // TODO: it would also be good to emit a "namespace" kind and/or
                // symbol for rust modules as we go? (For C++ we don't
                // emit a structured record but do emit an "NS_"-prefixed sym.)
                (
                    format!("{}/", escaped),
                    match lang {
                        ScipLang::Typescript => PrettyAction::Omit,
                        _ => PrettyAction::Append,
                    },
                    None,
                    false,
                )
            }
            Suffix::Type => (
                format!("{}#", escaped),
                PrettyAction::Append,
                Some("class"),
                false,
            ),
            Suffix::Term => (
                format!("{}.", escaped),
                PrettyAction::Append,
                Some("field"),
                true,
            ),
            Suffix::Method => (
                format!(
                    "{}({}).",
                    escaped,
                    sanitize_symbol(&descriptor.disambiguator)
                ),
                PrettyAction::Append,
                Some("method"),
                true,
            ),
            Suffix::TypeParameter => {
                // Not sure what cases this is used in...
                (
                    format!("[{}]", escaped),
                    PrettyAction::ResetAndUse,
                    None,
                    false,
                )
            }
            Suffix::Parameter => {
                // For now, at least, arguments don't get tracked by the parent.
                (
                    format!("({})", escaped),
                    PrettyAction::ResetAndUse,
                    Some("arg"),
                    false,
                )
            }
            Suffix::Macro => (
                format!("{}!", escaped),
                PrettyAction::Append,
                Some("macro"),
                false,
            ),
            Suffix::Meta => {
                // We see this used for fields in JS, at least when
                // preceded by a `Type#`.
                (
                    format!("{}:", escaped),
                    PrettyAction::Append,
                    Some("field"),
                    true,
                )
            }
            // Local is special because the symbol's "scheme" is
            // "local", so the suffix is interesting as a marker,
            // but doesn't actually exist on the descriptor as
            // an actual string suffix.
            Suffix::Local => {
                // We prefix the local with the relative path of the
                // doc, which should be the same as scip-typescript.
                // Conceptually, this is similar to what we do with
                // C++ where we hash over the filename/line and the
                // variable name.
                //
                // We also put a "#" on there to try and do a little
                // extra name-spacing.
                (
                    format!("{}/#{}", sanitize_symbol(relative_path), escaped),
                    PrettyAction::UseAlternateSource,
                    None,
                    false,
                )
            }
            // Suffix::UnspecifiedSuffix is not possible because we
            // excluded it above, but rust doesn't know that.
            Suffix::UnspecifiedSuffix => {
                ("".to_owned(), PrettyAction::Omit, None, false)
            }
        };
        prev_kind = last_kind;
        last_kind = maybe_kind;
        last_contributes_to_parent = contributes_to_parent;

        sym_pieces.push(sym_piece);

        match pretty_action {
            PrettyAction::Omit => {}
            PrettyAction::Append => {
                pretty_pieces.push(descriptor.name.clone());
            }
            PrettyAction::ResetAndUse => {
                pretty_pieces.clear();
                pretty_pieces.push(descriptor.name.clone());
            }
            PrettyAction::UseAlternateSource => {
                pretty_pieces.clear();
                if let Some(name) = doc_name {
                    pretty_pieces.push(name.to_string());
                } else {
                    pretty_pieces.push("unknown".to_string());
                }
            }
        }
    }

    // If we have an explicit doc namespace that provides context
    // the descriptors do not provide, then use that for all
    // pieces except the last piece we get from the descriptor.
    if let Some(namespace) = doc_namespace {
        if let Some(last_piece) = pretty_pieces.pop() {
            pretty_pieces = namespace.split("::").map(|s| s.to_string()).collect();
            pretty_pieces.push(last_piece);
        }
    }

    // We've standardized on "::" as the delimiter here even though
    // one might argue on a convention of using ".".  But crossref
    // currently requires "::" and this seems like a reasonable
    // convention.  Especially as the introduction of private JS
    // symbols prefixed with "#" has mooted the hacky syntax
    // previously used by mozsearch and used as a convention on MDN
    // URLs.
    let pretty = ustr(&pretty_pieces.join("::"));
    let norm_sym = symbol_name(lang_name, subtree_name, &sym_pieces.join(""));

    // Infer a parent sym if it seems to be a slice
    let parent_sym = if prev_kind == Some("class") && sym_pieces.len() >= 2 {
        Some(symbol_name(lang_name, subtree_name, &sym_pieces[..sym_pieces.len() - 1].join("")))
    } else {
        None
    };

    SymbolAnalysis {
        kind: last_kind,
        pretty,
        norm_sym,
        parent_sym,
        contributes_to_parent: last_contributes_to_parent,
    }
}

fn analyze_using_scip(
    tree_config: &config::TreeConfig,
    subtree_name: Option<&str>,
    subtree_root: &str,
    platform: &Option<String>,
    scip_file: PathBuf,
) {
    use protobuf::Message;
    use scip::types::*;

    let file = File::open(&scip_file).expect("Can't open scip file");
    let byte_count = file.metadata().expect("Failed to get file metadata").len();
    let mut file = BufReader::new(file);
    let mut file = protobuf::CodedInputStream::from_buf_read(&mut file);
    let index = Index::parse_from(&mut file).expect("Failed to read scip index");

    let mut scip_symbol_to_structured: HashMap<String, AnalysisStructured> = HashMap::new();
    let mut our_symbol_to_scip_sym: UstrMap<String> = UstrMap::default();

    let (lang_name, lang) = match index.metadata.tool_info.name.as_str() {
        "rust-analyzer" => ("rs", ScipLang::Rust),
        "scip-python" => ("py", ScipLang::Python),
        "scip-typescript" => ("js", ScipLang::Typescript),
        "scip-java" => ("jvm", ScipLang::Jvm),
        _ => {
            warn!("Unsupported language; we need tree-sitter support.");
            return;
        }
    };

    // ## First Pass: Process Symbol Definitions
    //
    // It's necessary to process all of the symbol definitions first because
    // occurrences can reference symbols defined in other documents.  (The
    // exception is locals inherently are file-local.)
    //
    // This also provides us a good opportunity to normalize the symbol names we
    // see and perform structural inference from the descriptors.  Structural
    // inference inherently involves mutating symbols as we go, so we do not
    // write anything out during this pass.  This is just as well as we can make
    // sure to only emit structured information at the point of definition
    // (which we do not know until we look at the occurrences).
    for doc in &index.documents {
        info!(
            "Processing symbols/definitions for '{}'",
            &doc.relative_path,
        );

        // XXX next steps
        // finish up the loop before to populate the structured things:
        // - was thinking we just re-derive the descriptor string ourselves as
        //   we go and we hold onto the previous state of the derived string so
        //   that we can do the parent lookup for fields/methods.
        // - the previous descriptor isn't particularly useful because it's
        //   always going to be a type.
        // - extra things:
        //   - we do want to handle the relationship
        //   - it could be good to have a basic approach for the documentation
        //     - for rust that does give us size and alignment which is nice
        //     - for JS we get the inferred type as well, plus the extracted
        //       comment as a second string after the type info.

        for scip_sym_info in &doc.symbols {
            // Process each symbol to:
            // - Derive a canonical mozsearch symbol name and map it.
            // - Derive structured analysis information that we will emit in the
            //   next pass if/when we see the definition.
            if let Ok(scip_sym) = scip::symbol::parse_symbol(&scip_sym_info.symbol) {
                // ### Extract Metadata from Documentation Markdown
                //
                // The documentation is what VS Code shows in tooltips for
                // symbols, which means there's dense information in there that
                // can be quite useful, but that it's intended for human
                // consumption, which means we need to regex it out.

                // XXX these should be made Option<u32>, but StructuredFieldInfo
                // needs to have its signature updated.  Right now this is hacky.
                let size_bytes = 0;
                //let mut align_bytes = 0;
                let offset_bytes = 0;

                // Until https://github.com/rust-lang/rust-analyzer/pull/16559
                // landed in rust-analyzer, we tried to use the doc string
                // containing the tunneled hover information to additionally
                // namespace the pretty identifier in an attempt to match our
                // original rust-analysis behavior.  This ended up only working
                // for fields (where we also would populate size_bytes and
                // offset_bytes above).  Bug 1881645 provides some more context
                // but the general situation is that this specific logic can
                // likely be removed as part of a nice clean-up, but it's also
                // worth revisiting the symbol and pretty mappings with more
                // intent.
                //
                // That said, there may be other SCIP languages where this could
                // still be a useful hack.
                let doc_namespace: Option<String> = None;

                // High confidence identifier name of what's being defined for
                // fallback use in the case of locals as extracted from the doc
                // strings.
                //
                // Because the doc strings are intended to be human-readable
                // rather than machine-readable, this may not always be
                // something we can reliably parse.  In particular, rust-analyzer
                // likes to excerpt the declaration, and our whole point in
                // using SCIP is to not be writing our own rust parser, although
                // we can probably evolve good-enough regexps, etc.
                //
                // TODO: Consider allowing for a fix-up pass when processing the
                // occurrences when we can potentially have the underlying token
                // available and/or a full tree-sitter parse.
                let mut fallback_kind = None;
                let mut doc_name = None;
                let mut type_pretty = None;

                lazy_static! {
                    // used for fields, methods, arguments/parameters
                    static ref RE_TS_TYPED: Regex =
                        Regex::new(r"^```ts\n([^ ]+) (.+): ([^\n]+)\n```$").unwrap();
                    // used for modules, classes
                    static ref RE_TS_UNTYPED: Regex =
                        Regex::new(r"^```ts\n([^ ]+) (.+)\n```$").unwrap();
                    // used for modules, classes
                    static ref RE_KT_FUNCTION: Regex =
                        Regex::new(r"^```kt\n([^ ]+) ([^ ]+) fun ([^ ]+)\(.*\)(?:: (.*))?\n```$").unwrap();
                }

                // TODO: Consider trying to do something where the documentation
                // is actually a real docstring.
                for (i, doc) in scip_sym_info.documentation.iter().enumerate() {
                    if i == 0 {
                        match &lang {
                            ScipLang::Python => {
                                // TODO: try and extract some info from here;
                                // It looks like this could be very descriptor-dependent.
                            }
                            ScipLang::Rust => {
                                // We no longer do anything for rust.  See the
                                // doc_namespace comments above.
                            }
                            ScipLang::Typescript => {
                                if let Some(caps) = RE_TS_TYPED.captures(doc) {
                                    if let Some(s) = caps.get(1) {
                                        fallback_kind =
                                            Some(s.as_str().trim_matches(|c| c == '(' || c == ')'));
                                    }
                                    if let Some(s) = caps.get(2) {
                                        doc_name = Some(s.as_str().to_string());
                                    }
                                    if let Some(s) = caps.get(3) {
                                        type_pretty = Some(ustr(s.as_str()));
                                    }
                                } else if let Some(caps) = RE_TS_UNTYPED.captures(doc) {
                                    if let Some(s) = caps.get(2) {
                                        doc_name = Some(s.as_str().to_string());
                                    }
                                }
                            }
                            ScipLang::Jvm => {
                                if let Some(caps) = RE_KT_FUNCTION.captures(doc) {
                                    fallback_kind = Some("method");
                                    if let Some(s) = caps.get(3) {
                                        doc_name = Some(s.as_str().to_string());
                                    }
                                }
                            }
                        }
                    }
                    // Otherwise this is an extracted docstring, which we can't
                    //use yet.
                }

                // Signature documentation is new and a more reliable source of
                // type information.
                if let Some(doc) = scip_sym_info.signature_documentation.as_ref() {
                    if !doc.text.is_empty() {
                        type_pretty = Some(ustr(&doc.text));
                    }
                }

                let mut symbol_info = analyse_symbol(
                    &scip_sym,
                    &lang,
                    &lang_name,
                    subtree_name,
                    &doc.relative_path,
                    doc_name.as_deref(),
                    doc_namespace.as_deref()
                );

                let mut supers = vec![];
                let mut overrides = vec![];

                // SCIP provides the full transitive closure of relationships,
                // but our current model favors only having the immediate links.
                // TODO: filter out indirect ancestors
                for rel in &scip_sym_info.relationships {
                    let Ok(rel_scip_sym) = scip::symbol::parse_symbol(&rel.symbol) else {
                        info!("bad relationship symbol: {}", rel.symbol);
                        continue;
                    };
                    let parent_symbol_info = analyse_symbol(
                        &rel_scip_sym,
                        &lang,
                        &lang_name,
                        subtree_name,
                        &doc.relative_path,
                        None,
                        None
                    );

                    // If our symbol is a local (or if we failed to unwrap its kind for any reason),
                    // fallback to the kind of the symbol it is related to
                    if symbol_info.kind.is_none() {
                        symbol_info.kind = parent_symbol_info.kind;
                    }
                    match symbol_info.kind {
                        Some("class") => {
                            supers.push(StructuredSuperInfo {
                                sym: ustr(&parent_symbol_info.norm_sym),
                                offset_bytes: 0,
                                props: vec![],
                            });
                        }
                        Some("method") => {
                            overrides.push(StructuredOverrideInfo {
                                sym: ustr(&parent_symbol_info.norm_sym),
                            });
                        }
                        _ => {}
                    }
                }

                // Ensure that supers and overrides are sorted to avoid flaky tests
                supers.sort_unstable_by_key(|r| r.sym);
                overrides.sort_unstable_by_key(|r| r.sym);

                let structured = AnalysisStructured {
                    structured: StructuredTag::Structured,
                    pretty: symbol_info.pretty,
                    sym: symbol_info.norm_sym,
                    type_pretty,
                    kind: ustr(symbol_info.kind.or(fallback_kind).unwrap_or("")),
                    subsystem: None,
                    parent_sym: symbol_info.parent_sym,
                    slot_owner: None,
                    impl_kind: ustr("impl"),
                    size_bytes: if size_bytes > 0 {
                        Some(size_bytes)
                    } else {
                        None
                    },
                    own_vf_ptr_bytes: None,
                    binding_slots: vec![],
                    ontology_slots: vec![],
                    supers,
                    methods: vec![],
                    fields: vec![],
                    overrides,
                    props: vec![],
                    labels: BTreeSet::default(),

                    idl_sym: None,
                    subclass_syms: vec![],
                    overridden_by_syms: vec![],
                    variants: vec![],
                    extra: Map::default(),
                };
                // for local symbols we use our own symbol because the SCIP symbol is not
                // actually unique; we also need to update our helper mapping.
                if scip_sym_info.symbol.starts_with("local ") {
                    scip_symbol_to_structured.insert(symbol_info.norm_sym.to_string(), structured);
                    our_symbol_to_scip_sym.insert(symbol_info.norm_sym, symbol_info.norm_sym.to_string());
                } else {
                    scip_symbol_to_structured.insert(scip_sym_info.symbol.clone(), structured);
                    our_symbol_to_scip_sym.insert(symbol_info.norm_sym, scip_sym_info.symbol.clone());
                }

                if symbol_info.contributes_to_parent {
                    if let Some(psym) = symbol_info.parent_sym {
                        if let Some(scip_psym) = our_symbol_to_scip_sym.get(&psym) {
                            if let Some(pstruct) = scip_symbol_to_structured.get_mut(scip_psym) {
                                match &symbol_info.kind {
                                    Some("method") => {
                                        pstruct.methods.push(StructuredMethodInfo {
                                            pretty: symbol_info.pretty,
                                            sym: symbol_info.norm_sym,
                                            props: vec![],
                                            labels: BTreeSet::default(),
                                            // TODO: see about trying to extract args from markdown?
                                            args: vec![],
                                        });
                                    }
                                    Some("field") => {
                                        pstruct.fields.push(StructuredFieldInfo {
                                            line_range: ustr(""),
                                            pretty: symbol_info.pretty,
                                            sym: symbol_info.norm_sym,
                                            type_pretty: type_pretty.unwrap_or_else(|| ustr("")),
                                            type_sym: ustr(""),
                                            offset_bytes,
                                            bit_positions: None,
                                            size_bytes: if size_bytes > 0 {
                                                Some(size_bytes)
                                            } else {
                                                None
                                            },
                                            labels: BTreeSet::default(),
                                            pointer_info: vec![],
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            } else {
                warn!(
                    "Unable to parse SCIP symbol: {}\n:{:?}",
                    scip_sym_info.symbol,
                    scip::symbol::parse_symbol(&scip_sym_info.symbol)
                );
            }
        }
    }

    let analysis_root = Path::new(&tree_config.paths.index_path).join(match platform {
        None => "analysis".to_string(),
        Some(platform) => format!("analysis-{}", platform),
    });

    for doc in &index.documents {
        let searchfox_path = Path::new(&doc.relative_path).to_owned();
        let searchfox_path = Path::new(&subtree_root).to_owned().join(&searchfox_path);

        let output_file = analysis_root.join(&searchfox_path);
        if let Err(err) = create_output_dir(&output_file) {
            error!(
                "Couldn't create dir for: {}, {:?}",
                output_file.display(),
                err
            );
            continue;
        }
        let mut file = match File::create(&output_file) {
            Ok(f) => f,
            Err(err) => {
                error!(
                    "Couldn't open output file: {}, {:?}",
                    output_file.display(),
                    err
                );
                continue;
            }
        };

        let source_fname =
            tree_config.find_source_file(&format!("{}/{}", subtree_root, &doc.relative_path));
        let source_contents = match std::fs::read(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                error!("Unable to open source file {}", source_fname);
                continue;
            }
        };

        // XXX Because typescript has different languages for typescript and tsx,
        // we currently need to create the parser and the queries on a per-file
        // basis.  We should revisit this if profiling shows this is a big problem.
        // Same thing with Java and Kotlin
        let mut parser = tree_sitter::Parser::new();
        let ts_query = match &lang {
            ScipLang::Python => {
                parser
                    .set_language(tree_sitter_python::language())
                    .expect("Error loading Python grammar");
                compile_nesting_queries(tree_sitter_python::language(), &PYTHON_NESTING)
            }
            ScipLang::Rust => {
                parser
                    .set_language(tree_sitter_rust::language())
                    .expect("Error loading Rust grammar");
                compile_nesting_queries(tree_sitter_rust::language(), &RUST_NESTING)
            }
            ScipLang::Typescript => {
                if doc.relative_path.ends_with(".tsx") || doc.relative_path.ends_with(".jsx") {
                    parser
                        .set_language(tree_sitter_typescript::language_tsx())
                        .expect("Error loading TSX grammar");
                    compile_nesting_queries(tree_sitter_typescript::language_tsx(), &JS_NESTING)
                } else {
                    parser
                        .set_language(tree_sitter_typescript::language_typescript())
                        .expect("Error loading Typescript grammar");
                    compile_nesting_queries(
                        tree_sitter_typescript::language_typescript(),
                        &JS_NESTING,
                    )
                }
            }
            ScipLang::Jvm => {
                if doc.relative_path.ends_with(".kt") {
                    parser
                        .set_language(tree_sitter_kotlin::language())
                        .expect("Error loading Kotlin grammar");
                    compile_nesting_queries(tree_sitter_kotlin::language(), &KOTLIN_NESTING)
                } else {
                    parser
                        .set_language(tree_sitter_java::language())
                        .expect("Error loading Java grammar");
                    compile_nesting_queries(tree_sitter_java::language(), &JAVA_NESTING)
                }
            }
        };
        let name_capture_ix = ts_query.capture_index_for_name("name").unwrap();
        let body_capture_ix = ts_query.capture_index_for_name("body").unwrap();

        let parse_tree = match parser.parse(&source_contents[..], None) {
            Some(t) => t,
            _ => {
                warn!("tree-sitter parse failed, skipping file.");
                continue;
            }
        };

        let mut query_cursor = tree_sitter::QueryCursor::new();
        let mut query_matches =
            query_cursor.matches(&ts_query, parse_tree.root_node(), &source_contents[..]);
        let mut next_parse_match;
        let mut next_parse_loc = Location::default();
        let mut next_parse_nesting = SourceRange::default();

        let mut nesting_stack: Vec<NestedSymbol> = vec![];

        // Be chatty about the files we're outputting so that it's easier to follow
        // the path of rust analysis generation.
        info!(
            "Processing occurrences for '{}' to '{}'",
            searchfox_path.display(),
            output_file.display()
        );

        for occurrence in &doc.occurrences {
            // We need to normalize locals to include the file path, consistent
            // with how we handled them in the prior pass.  Note that this is
            // not actually an official SCIP symbol, but because the local symbols
            // are not namespaced, we just store it using our normalized version.
            let (is_local, norm_scip_sym) = if occurrence.symbol.starts_with("local ") {
                (
                    true,
                    symbol_name(
                        lang_name,
                        subtree_name,
                        &format!("{}/#{}", sanitize_symbol(&doc.relative_path), &occurrence.symbol[6..])
                    ),
                )
            } else {
                (false, ustr(&occurrence.symbol))
            };

            let sinfo = match scip_symbol_to_structured.get(norm_scip_sym.as_str()) {
                Some(s) => s,
                None => {
                    // For occurences that don't match any symbol, we create a new structured fake,
                    // save it, and return it.

                    let symbol = match scip::symbol::parse_symbol(&occurrence.symbol) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("{:?}", e);
                            continue;
                        }
                    };

                    let symbol_info = analyse_symbol(&symbol, &lang, lang_name, subtree_name, &doc.relative_path, None, None);

                    let fake = AnalysisStructured {
                        structured: StructuredTag::Structured,
                        pretty: symbol_info.pretty,
                        sym: symbol_info.norm_sym,
                        type_pretty: None,
                        kind: ustr(symbol_info.kind.unwrap_or("")),
                        subsystem: None,
                        parent_sym: symbol_info.parent_sym,
                        slot_owner: None,
                        impl_kind: ustr("impl"),
                        size_bytes: None,
                        own_vf_ptr_bytes: None,
                        binding_slots: vec![],
                        ontology_slots: vec![],
                        supers: vec![],
                        methods: vec![],
                        fields: vec![],
                        overrides: vec![],
                        props: vec![],
                        labels: BTreeSet::default(),

                        idl_sym: None,
                        subclass_syms: vec![],
                        overridden_by_syms: vec![],
                        variants: vec![],
                        extra: Map::default(),
                    };
                    scip_symbol_to_structured.insert(norm_scip_sym.to_owned(), fake);
                    our_symbol_to_scip_sym.insert(symbol_info.norm_sym, norm_scip_sym.to_owned());
                    scip_symbol_to_structured.get(norm_scip_sym.as_str()).unwrap()
                }
            };
            let loc = scip_range_to_searchfox_location(&occurrence.range);
            let kind = scip_roles_to_searchfox_analysis_kind(occurrence.symbol_roles);

            // ## Tree-Sitter Nesting Magic
            //
            // Goals:
            // 1. Provide `nesting_range` information for source records for
            //    relevant namespaces.
            // 2. Contribute context/contextsym information to target records.
            //    These should usually line up with the nesting information we
            //    want from the above.
            //
            // There are broadly 2 implementation approaches:
            // 1. Detailed AST traversal: We walk the AST ourselves and keep our
            //    cursor close to the occurrences.
            // 2. Use tree-sitter's query mechanism to have it give us a
            //    filtered set of the AST nodes that should correspond to the
            //    relevant nesting and namespace nesting levels.
            //
            // We're going with the 2nd approach because anything we do is going
            // to need to involve language-specific configuration data, and it
            // seems very silly to re-invent a worse version of the query
            // mechanism.
            //
            // A very nice thing about the query mechanism is it can tell you
            // which pattern index matched, which means we can define a little
            // data structure that we use to derive the query and also have
            // structured rust data that we can consult without any hacks.
            // (NB: As documented on the SitterNesting type, we probably should
            // be using a different approach based on `.scm` files.)

            // Pop off any nested symbols which don't include the current line.
            while let Some(nested) = nesting_stack.last() {
                if loc.lineno > nested.nesting_range.end_lineno {
                    nesting_stack.pop();
                } else {
                    break;
                }
            }

            // Check if this symbol starts a nesting range or if we need to skip.
            // We defer pushing any nesting we start to simplify the logic below
            // that's nesting aware; a symbol shouldn't be its own contextsym!

            // Skip any matched symbols that are earlier than our current symbol.
            while next_parse_loc < loc {
                next_parse_match = query_matches.next();
                (next_parse_loc, next_parse_nesting) = if let Some(pm) = &next_parse_match {
                    (
                        node_range_to_searchfox_location(
                            pm.nodes_for_capture_index(name_capture_ix)
                                .next()
                                .unwrap()
                                .range(),
                        ),
                        node_range_to_searchfox_range(
                            pm.nodes_for_capture_index(body_capture_ix)
                                .next()
                                .unwrap()
                                .range(),
                        ),
                    )
                } else {
                    (
                        Location {
                            lineno: u32::MAX,
                            col_start: 0,
                            col_end: 0,
                        },
                        SourceRange {
                            start_lineno: u32::MAX,
                            start_col: 0,
                            end_lineno: u32::MAX,
                            end_col: 0,
                        },
                    )
                }
            }

            // If we match the name, then push this.
            let starts_nest = if next_parse_loc == loc {
                // Note that it's possible this current approach could result
                // with us defining the start of 2+ nesting ranges on the same
                // line.  format.rs handles this and we similarly require any
                // other consumers to handle this.
                Some(NestedSymbol {
                    sym: sinfo.sym.clone(),
                    pretty: sinfo.pretty.clone(),
                    nesting_range: next_parse_nesting.clone(),
                })
            } else {
                None
            };

            let no_crossref = is_local && sinfo.supers.is_empty() && sinfo.overrides.is_empty();

            {
                let mut syntax = vec![kind.to_ustr()];
                if !sinfo.kind.is_empty() {
                    syntax.push(sinfo.kind.clone());
                }
                let source_data = WithLocation {
                    data: AnalysisSource {
                        source: SourceTag::Source,
                        syntax,
                        pretty: ustr(&format!("{} {}", sinfo.kind, sinfo.pretty)),
                        sym: vec![sinfo.sym.clone()],
                        no_crossref,
                        nesting_range: if let Some(nest) = &starts_nest {
                            nest.nesting_range.clone()
                        } else {
                            SourceRange::default()
                        },
                        // TODO: Expose type information for fields/etc.
                        type_pretty: sinfo.type_pretty.clone(),
                        type_sym: None,
                        arg_ranges: vec![],
                        expansion_info: None,
                    },
                    loc,
                };
                write_line(&mut file, &source_data);
            }

            // If this was the definition point, then write out the structured record.
            if kind == AnalysisKind::Def {
                write_line(
                    &mut file,
                    &WithLocation {
                        data: sinfo,
                        loc,
                    },
                );
            }

            // TODO: Contextual info.

            if !no_crossref {
                let (contextsym, context) = if let Some(nested) = nesting_stack.last() {
                    (nested.sym.clone(), nested.pretty.clone())
                } else {
                    (ustr(""), ustr(""))
                };

                let target_data = WithLocation {
                    data: AnalysisTarget {
                        target: TargetTag::Target,
                        kind,
                        pretty: sinfo.pretty.clone(),
                        sym: sinfo.sym.clone(),
                        context,
                        contextsym,
                        peek_range: LineRange {
                            start_lineno: 0,
                            end_lineno: 0,
                        },
                        arg_ranges: vec![],
                    },
                    loc: loc.clone(),
                };
                write_line(&mut file, &target_data);
            }

            // Push the nesting now that we've finished processing the symbol
            // and this avoids any consultations of the stack above.
            if let Some(nested) = starts_nest {
                nesting_stack.push(nested);
            }
        }
    }

    assert_eq!(file.pos(), byte_count, "Should've processed the whole file");
}

fn main() {
    env_logger::init();

    let cli = ScipIndexerCli::parse();

    let tree_name = &cli.tree_name;
    let cfg = config::load(&cli.config_file, false, Some(&tree_name));
    let tree_config = cfg.trees.get(tree_name).unwrap();

    for file in cli.inputs {
        analyze_using_scip(&tree_config, cli.subtree_name.as_deref(), &cli.subtree_root, &cli.platform, file);
    }
}
