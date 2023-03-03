extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate protobuf;
extern crate scip;
extern crate tools;

use clap::Parser;
use std::borrow::Cow;
use std::collections::{HashMap};
use std::fs::{self, File};
use std::io;
use std::io::{BufReader};
use std::path::{Path, PathBuf};
use tools::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisTarget, LineRange, Location, SourceRange, SourceTag,
    TargetTag, WithLocation,
};
use ustr::ustr;

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

fn sanitize_symbol(sym: &str) -> String {
    // Downstream processing of the symbol doesn't deal well with
    // these characters, so replace them with underscores.
    fn is_special_char(c: char) -> bool {
        matches!(c, ',' | ' ' | '.' | '(' | ')' | '\n' | '#' | '-' | '/')
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
    #[clap(value_parser)]
    src: PathBuf,

    /// Points to the directory where searchfox metadata should go (ANALYSIS_ROOT)
    #[clap(value_parser)]
    output: PathBuf,

    /// Points to the generated source files root (GENERATED)
    #[clap(value_parser)]
    generated: PathBuf,

    /// Common prefix to the scip files. If given e.g., the objdir, we can infer
    /// that a given scip file in objdir/tools/rust.scip refers to tools/ rather
    /// than top-level srcdir locations.
    #[clap(long, value_parser)]
    scip_prefix: Option<PathBuf>,

    /// rustc analysis directories or scip inputs
    #[clap(value_parser)]
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

fn scip_roles_to_searchfox_tags(roles: i32) -> Vec<AnalysisKind> {
    let mut values = vec![];

    macro_rules! map_to_searchfox {
        ($scip:ident, $sfox:ident) => {
            if roles & scip::types::SymbolRole::$scip as i32 != 0 {
                if values.last() != Some(&AnalysisKind::$sfox) {
                    values.push(AnalysisKind::$sfox);
                }
            }
        };
    }

    map_to_searchfox!(Definition, Def);
    map_to_searchfox!(Import, Use);
    map_to_searchfox!(WriteAccess, Use);
    map_to_searchfox!(ReadAccess, Use);
    map_to_searchfox!(Generated, Use);
    map_to_searchfox!(Test, Use);
    map_to_searchfox!(Import, Use);

    values
}

fn analyze_using_scip(tree_info: &TreeInfo, scip_prefix: Option<&PathBuf>, scip_file: PathBuf) {
    use protobuf::Message;
    use scip::types::*;

    let file = File::open(&scip_file).expect("Can't open scip file");
    let byte_count = file.metadata().expect("Failed to get file metadata").len();
    let mut file = BufReader::new(file);
    let mut file = protobuf::CodedInputStream::from_buf_read(&mut file);
    let index = Index::parse_from(&mut file).expect("Failed to read scip index");

    for doc in &index.documents {
        let searchfox_path = Path::new(&doc.relative_path).to_owned();
        let searchfox_path =
            match scip_prefix.and_then(|prefix| scip_file.strip_prefix(prefix).ok()) {
                Some(p) => {
                    let mut p = p.to_owned();
                    p.pop();
                    p.join(&searchfox_path)
                }
                None => searchfox_path,
            };

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
            "Writing analysis for '{}' to '{}'",
            searchfox_path.display(),
            output_file.display()
        );

        // A map from local symbol to the index in doc.symbols for this document.
        let mut doc_symbols_to_index = HashMap::new();
        for (index, symbol) in doc.symbols.iter().enumerate() {
            doc_symbols_to_index.insert(symbol.symbol.clone(), index);
        }

        let lookup_symbol = |s: &str| -> Cow<SymbolInformation> {
            match doc_symbols_to_index.get(s) {
                Some(i) => Cow::Borrowed(&doc.symbols[*i]),
                None => {
                    warn!("Didn't find symbol {:?} in local symbol table", s);
                    // Fake it till you make it? We have no info for this
                    // symbol, so...
                    Cow::Owned(SymbolInformation {
                        symbol: s.to_owned(),
                        documentation: vec![],
                        relationships: vec![],
                        special_fields: Default::default(),
                    })
                }
            }
        };

        for occurrence in &doc.occurrences {
            let loc = scip_range_to_searchfox_location(&occurrence.range);
            let symbol = lookup_symbol(&occurrence.symbol);
            {
                let global = sanitize_symbol(&symbol.symbol);
                let pretty = pretty_symbol(&symbol.symbol);
                let source_data = WithLocation {
                    data: AnalysisSource {
                        source: SourceTag::Source,
                        // TODO: Fill syntax.
                        syntax: vec![],
                        pretty: ustr(&pretty),
                        sym: vec![ustr(&global)],
                        no_crossref: false,
                        // TODO(bug 1796870): Nesting.
                        nesting_range: SourceRange::default(),
                        // TODO: Expose type information for fields/etc.
                        type_pretty: None,
                        type_sym: None,
                    },
                    loc,
                };
                write_line(&mut file, &source_data);
            }

            // TODO: Contextual info.
            let context = None;

            let get_target_data = |sym: &str, kind: AnalysisKind| -> WithLocation<AnalysisTarget> {
                let global = sanitize_symbol(sym);
                let pretty = pretty_symbol(sym);
                WithLocation {
                    data: AnalysisTarget {
                        target: TargetTag::Target,
                        kind,
                        pretty: ustr(&pretty),
                        sym: ustr(&global),
                        context: ustr(context.unwrap_or("")),
                        contextsym: ustr(context.unwrap_or("")),
                        peek_range: LineRange {
                            start_lineno: 0,
                            end_lineno: 0,
                        },
                    },
                    loc: loc.clone(),
                }
            };

            write_line(
                &mut file,
                &get_target_data(&symbol.symbol, AnalysisKind::Use),
            );

            for kind in scip_roles_to_searchfox_tags(occurrence.symbol_roles) {
                write_line(&mut file, &get_target_data(&symbol.symbol, kind));
            }

            for relationship in &symbol.relationships {
                let kind = if relationship.is_type_definition || relationship.is_implementation {
                    AnalysisKind::Def
                } else {
                    AnalysisKind::Use
                };
                write_line(&mut file, &get_target_data(&relationship.symbol, kind));
            }
        }

        println!("{}", doc.relative_path);
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
        analyze_using_scip(&tree_info, cli.scip_prefix.as_ref(), file);
    }
}
