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
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tools::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisStructured, AnalysisTarget, LineRange, Location,
    SourceRange, SourceTag, StructuredFieldInfo, StructuredMethodInfo, StructuredOverrideInfo,
    StructuredSuperInfo, StructuredTag, TargetTag, WithLocation,
};
use ustr::{ustr, UstrMap};

/// Local filesystem path mappings and metadata which exist for the following
/// purposes:
/// 1. Know where to output the analysis files.
///   - There is only ever one analysis output directory.
/// 2. Know how to locate rust source files in order to hackily extract strings
///    that should have been in the save-analysis files.
///    - After config scripts run and normalize things there are 2 source
///      directories: revision controlled source (cross-platform) and the
///      (per-platform) generated files directory.
#[derive(Debug)]
struct TreeInfo<'a> {
    /// Local filesystem path root for the analysis dir where rust-indexer.rs
    /// should write its output.
    out_analysis_dir: &'a Path,
    /// Local filesystem path root for the source tree.  In the searchfox path
    /// space presented to users, this means all paths not prefixed with
    /// `__GENERATED__`.
    srcdir: &'a Path,
    /// Local filesystem path root for the per-platform generated source tree.
    /// In the searchfox path space presented to users, this means paths
    /// prefixed with `__GENERATED__`.
    generated: &'a Path,
    /// The searchfox path space prefix for generated.
    generated_friendly: &'a Path,
}

/// SCIP symbols

fn sanitize_symbol(sym: &str) -> String {
    // Downstream processing of the symbol doesn't deal well with
    // these characters, so replace them with underscores.
    fn is_special_char(c: char) -> bool {
        matches!(c, ',' | ' ' | '\n')
    }
    sym.replace(is_special_char, "_").trim_matches('_').into()
}

fn pretty_symbol(sym: &str) -> Cow<str> {
    if let Ok(sym) = scip::symbol::parse_symbol(&sym) {
        return Cow::Owned(scip::symbol::format_symbol(sym));
    }
    Cow::Borrowed(sym)
}

fn create_output_dir(output_file: &Path) -> io::Result<()> {
    let mut output_dir = output_file.to_owned();
    output_dir.pop();
    fs::create_dir_all(output_dir)
}

#[derive(Parser)]
struct RustIndexerCli {
    /// Points to the source root (FILES_ROOT)
    #[arg(value_parser)]
    src: PathBuf,

    /// Points to the directory where searchfox metadata should go (ANALYSIS_ROOT)
    #[arg(value_parser)]
    output: PathBuf,

    /// Points to the generated source files root (GENERATED)
    #[arg(value_parser)]
    generated: PathBuf,

    #[arg(long, value_parser)]
    subtree_name: String,

    /// Relative path from the root of the searchfox source tree to the SCIP
    /// index's contents.  If the SCIP index is from the root then this should
    /// be ".", otherwise it should be the relative path like "js-subtree/".
    #[arg(long, value_parser)]
    subtree_root: PathBuf,

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

/// Our specifically handled languages for conditional logic.
enum ScipLang {
    Rust,
    Typescript,
    Other,
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

fn analyze_using_scip(
    tree_info: &TreeInfo,
    subtree_name: &str,
    subtree_root: &PathBuf,
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
        "scip-typescript" => ("js", ScipLang::Typescript),
        _ => ("eh", ScipLang::Other),
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
                let mut size_bytes = 0;
                //let mut align_bytes = 0;
                let mut offset_bytes = 0;

                // For cases like rust-analyzer, we are provided with the
                // namespace of the identifier.  For example, for "Loader::new()"
                // defined in "simple.rs", this will be "simple::Loader".  For
                // the "Loader" type itself, this will just be "simple".
                //
                // For now we assume this will always include the type name,
                // but this and the `PrettyAction` type may need to evolve.
                let mut doc_namespace = None;

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
                let mut doc_name = None;
                let mut type_pretty = None;

                lazy_static! {
                    // This is specifically for picking out size, align, and the
                    // optionally present offset.  It will not match in all
                    // cases.
                    static ref RE_RUST_INFO: Regex =
                        Regex::new(r"^\n?```rust\n([^\n]+)\n```\n\n```rust\n(.+)(?: // size = (\d+), align = (\d+)(?:, offset = (\d+))?)?\n```$").unwrap();
                    // used for fields, methods, arguments/parameters
                    static ref RE_TS_TYPED: Regex =
                        Regex::new(r"^```ts\n([^ ])+ (.+): ([^\n]+)\n```$").unwrap();
                    // used for modules, classes
                    static ref RE_TS_UNTYPED: Regex =
                        Regex::new(r"^```ts\n([^ ])+ (.+)\n```$").unwrap();
                }

                for (i, doc) in scip_sym_info.documentation.iter().enumerate() {
                    if i == 0 {
                        match &lang {
                            ScipLang::Rust => {
                                if let Some(caps) = RE_RUST_INFO.captures(doc) {
                                    // XXX this gives us the full type signature absent the
                                    // comment piece, which is not going to intern well at all
                                    // for functions.  I think for functions the return value
                                    // (which we would get by splitting on the "->") might be
                                    // better at least from an interning perspective.
                                    //
                                    // In general we haven't defined what this is particularly
                                    // well.
                                    //
                                    // XXX also we only use this for fields right now.
                                    if let Some(s) = caps.get(1) {
                                        doc_namespace = Some(s.as_str().to_string());
                                    }
                                    if let Some(s) = caps.get(2) {
                                        type_pretty = Some(ustr(s.as_str()));
                                    }
                                    if let Some(s) = caps.get(3) {
                                        if let Ok(num) = s.as_str().parse::<u32>() {
                                            size_bytes = num;
                                        }
                                    }
                                    /*
                                    if let Some(s) = caps.get(4) {
                                        if let Ok(num) = s.as_str().parse::<u32>() {
                                            align_bytes = num;
                                        }
                                    }
                                    */
                                    if let Some(s) = caps.get(5) {
                                        if let Ok(num) = s.as_str().parse::<u32>() {
                                            offset_bytes = num;
                                        }
                                    }
                                }
                            }
                            ScipLang::Typescript => {
                                if let Some(caps) = RE_TS_TYPED.captures(doc) {
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
                            _ => {}
                        }
                    }
                    // Otherwise this is an extracted docstring, which we can't
                    //use yet.
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
                let mut pretty_pieces = vec![];
                let mut sym_pieces = vec![];
                let mut last_kind = None;
                let mut last_contributes_to_parent = false;
                let mut prev_kind = None;
                for desc_piece in &scip_sym.descriptors {
                    // Ignore descriptor enums from the future, skipping them.
                    let suffix = match desc_piece.suffix.enum_value() {
                        // UnspecifiedSuffix is weird because it's an in-domain
                        // value (it's explicitly part of the protobuf schema
                        // for suffix), but since currently the suffix is only
                        // built by parsing a string encoding of the enum, it
                        // logically is similar in nature to the enum_value
                        // being an Err.  Regardless, we skip it.
                        Ok(Suffix::UnspecifiedSuffix) => continue,
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let escaped = sanitize_symbol(&desc_piece.name);

                    let (sym_piece, pretty_action, maybe_kind, contributes_to_parent) = match suffix
                    {
                        // Confusingly, package is deprecated in favor of
                        // namespace, but right now the SCIP crate parses '/'
                        // as Package, not Namespace.
                        Suffix::Package | Suffix::Namespace => {
                            // Pretty: For JS/TS the namespace includes the file path which
                            // ends up way too verbose and now how humans would describe
                            // things.
                            (
                                format!("{}/", escaped),
                                match &lang {
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
                                sanitize_symbol(&desc_piece.disambiguator)
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
                                format!("{}/#{}", sanitize_symbol(&doc.relative_path), escaped),
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
                            pretty_pieces.push(desc_piece.name.clone());
                        }
                        PrettyAction::ResetAndUse => {
                            pretty_pieces.clear();
                            pretty_pieces.push(desc_piece.name.clone());
                        }
                        PrettyAction::UseAlternateSource => {
                            pretty_pieces.clear();
                            if let Some(name) = &doc_name {
                                pretty_pieces.push(name.clone());
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

                let pretty = ustr(&pretty_pieces.join(match &lang {
                    ScipLang::Rust => "::",
                    ScipLang::Typescript => ".",
                    ScipLang::Other => "::",
                }));
                let norm_sym = ustr(&format!(
                    "S_{}_{}_{}",
                    lang_name,
                    subtree_name,
                    sym_pieces.join("")
                ));

                // Infer a parent sym if it seems to be a slice
                let parent_sym = if prev_kind == Some("class") && sym_pieces.len() >= 2 {
                    Some(ustr(&format!(
                        "S_{}_{}_{}",
                        lang_name,
                        subtree_name,
                        sym_pieces[..sym_pieces.len() - 1].join("")
                    )))
                } else {
                    None
                };

                let mut supers = vec![];
                let mut overrides = vec![];

                // SCIP provides the full transitive closure of relationships,
                // but our current model favors only having the immediate links,
                // so only process the first element.
                if let Some(rel) = scip_sym_info.relationships.first() {
                    if let Some(rel_sinfo) = scip_symbol_to_structured.get(&rel.symbol) {
                        match last_kind {
                            Some("class") => {
                                supers.push(StructuredSuperInfo {
                                    pretty: rel_sinfo.pretty.clone(),
                                    sym: rel_sinfo.sym.clone(),
                                    props: vec![],
                                });
                            }
                            Some("method") => {
                                overrides.push(StructuredOverrideInfo {
                                    pretty: rel_sinfo.pretty.clone(),
                                    sym: rel_sinfo.sym.clone(),
                                });
                            }
                            _ => {}
                        }
                    }
                }

                let structured = AnalysisStructured {
                    structured: StructuredTag::Structured,
                    pretty,
                    sym: norm_sym,
                    type_pretty,
                    kind: ustr(last_kind.unwrap_or("")),
                    parent_sym,
                    slot_owner: None,
                    impl_kind: ustr("impl"),
                    size_bytes: if size_bytes > 0 {
                        Some(size_bytes)
                    } else {
                        None
                    },
                    binding_slots: vec![],
                    supers,
                    methods: vec![],
                    fields: vec![],
                    overrides,
                    props: vec![],

                    idl_sym: None,
                    subclass_syms: vec![],
                    overridden_by_syms: vec![],
                    extra: Map::default(),
                };
                scip_symbol_to_structured.insert(scip_sym_info.symbol.clone(), structured);
                our_symbol_to_scip_sym.insert(norm_sym, scip_sym_info.symbol.clone());

                if last_contributes_to_parent {
                    if let Some(psym) = parent_sym {
                        if let Some(scip_psym) = our_symbol_to_scip_sym.get(&psym) {
                            if let Some(pstruct) = scip_symbol_to_structured.get_mut(scip_psym) {
                                match &last_kind {
                                    Some("method") => {
                                        pstruct.methods.push(StructuredMethodInfo {
                                            pretty,
                                            sym: norm_sym,
                                            props: vec![],
                                        });
                                    }
                                    Some("field") => {
                                        pstruct.fields.push(StructuredFieldInfo {
                                            pretty,
                                            sym: norm_sym,
                                            type_pretty: type_pretty.unwrap_or_else(|| ustr("")),
                                            type_sym: ustr(""),
                                            offset_bytes,
                                            bit_positions: None,
                                            size_bytes: if size_bytes > 0 {
                                                Some(size_bytes)
                                            } else {
                                                None
                                            },
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

    for doc in &index.documents {
        let searchfox_path = Path::new(&doc.relative_path).to_owned();
        let searchfox_path = subtree_root.to_owned().join(&searchfox_path);

        let output_file = tree_info.out_analysis_dir.join(&searchfox_path);
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

        // Be chatty about the files we're outputting so that it's easier to follow
        // the path of rust analysis generation.
        info!(
            "Processing occurrences for '{}' to '{}'",
            searchfox_path.display(),
            output_file.display()
        );

        for occurrence in &doc.occurrences {
            let sinfo = match scip_symbol_to_structured.get(&occurrence.symbol) {
                Some(s) => s,
                None => {
                    warn!(
                        "Unable to find structured data for symbol: {}",
                        occurrence.symbol
                    );
                    continue;
                }
            };
            let loc = scip_range_to_searchfox_location(&occurrence.range);
            let kind = scip_roles_to_searchfox_analysis_kind(occurrence.symbol_roles);

            let is_local = occurrence.symbol.starts_with("local ");

            {
                let source_data = WithLocation {
                    data: AnalysisSource {
                        source: SourceTag::Source,
                        syntax: vec![kind.to_ustr(), sinfo.kind.clone()],
                        pretty: ustr(&format!("{} {}", sinfo.kind, sinfo.pretty)),
                        sym: vec![sinfo.sym.clone()],
                        no_crossref: is_local,
                        // TODO(bug 1796870): Nesting.
                        nesting_range: SourceRange::default(),
                        // TODO: Expose type information for fields/etc.
                        type_pretty: sinfo.type_pretty.clone(),
                        type_sym: None,
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
                        data: sinfo.clone(),
                        loc,
                    },
                );
            }

            // TODO: Contextual info.

            if !is_local {
                let target_data = WithLocation {
                    data: AnalysisTarget {
                        target: TargetTag::Target,
                        kind,
                        pretty: sinfo.pretty.clone(),
                        sym: sinfo.sym.clone(),
                        context: ustr(""),
                        contextsym: ustr(""),
                        peek_range: LineRange {
                            start_lineno: 0,
                            end_lineno: 0,
                        },
                    },
                    loc: loc.clone(),
                };
                write_line(&mut file, &target_data);
            }
        }
    }

    assert_eq!(file.pos(), byte_count, "Should've processed the whole file");
}

fn main() {
    env_logger::init();

    let cli = RustIndexerCli::parse();

    let tree_info = TreeInfo {
        srcdir: &cli.src,
        out_analysis_dir: &cli.output,
        generated: &cli.generated,
        generated_friendly: &PathBuf::from("__GENERATED__"),
    };

    info!("Tree info: {:?}", tree_info);

    for file in cli.inputs {
        analyze_using_scip(&tree_info, &cli.subtree_name, &cli.subtree_root, file);
    }
}
