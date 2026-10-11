// Analyze a tree's web specs' sources for the terms they define and their
// references to them (see `tools::spec_analyze`).
//
// Usage:
//   spec-analyze FILES_ROOT ANALYSIS_ROOT SPECS_TOML
//
// SPECS_TOML lists the specs (`[[spec]]`s, see `spec_analyze::SpecsConfig`),
// and each spec whose format we analyze gets the analysis of its source
// written to ANALYSIS_ROOT/<path>, or added to its analysis there if it has
// some (see `semantic_analyzer::merge_records`).

use std::path::Path;
use std::process::exit;
use std::time::Instant;

use tools::semantic_analyzer::write_records;
use tools::spec_analyze::{SpecFormat, SpecsConfig, ecmarkup, wattsi};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: {} FILES_ROOT ANALYSIS_ROOT SPECS_TOML", args[0]);
        exit(1);
    }
    let (files_root, analysis_root) = (Path::new(&args[1]), Path::new(&args[2]));
    let config = SpecsConfig::load(Path::new(&args[3])).unwrap_or_else(|e| {
        eprintln!("spec-analyze: {}", e);
        exit(1);
    });
    for spec in &config.specs {
        let (Some(format), Some(path)) = (spec.format, spec.source_path()) else {
            continue;
        };
        let Ok(source) = std::fs::read_to_string(files_root.join(&path)) else {
            eprintln!("spec-analyze: {}: no source at {}", spec.name, path);
            continue;
        };
        let started = Instant::now();
        match format {
            SpecFormat::Wattsi => {
                let (records, stats) = wattsi::analyze(&source, spec.shortname(), &config);
                write_records(analysis_root, &path, records);
                println!(
                    "{}: {} headings, {} dfns, {} cross-references ({} unresolved), {} links, in {:.2}s",
                    path,
                    stats.headings,
                    stats.dfns,
                    stats.references,
                    stats.unresolved,
                    stats.links,
                    started.elapsed().as_secs_f64()
                );
            }
            SpecFormat::Ecmarkup => {
                let (records, stats) = ecmarkup::analyze(&source, spec.shortname(), &config);
                write_records(analysis_root, &path, records);
                println!(
                    "{}: {} clauses, {} abstract operations, {} terms, {} productions, {} autolinks, {} xrefs, {} nonterminals ({} unresolved), in {:.2}s",
                    path,
                    stats.clauses,
                    stats.operations,
                    stats.terms,
                    stats.productions,
                    stats.autolinks,
                    stats.xrefs,
                    stats.nonterminals,
                    stats.unresolved,
                    started.elapsed().as_secs_f64()
                );
            }
            SpecFormat::Bikeshed => {}
        }
    }
}
