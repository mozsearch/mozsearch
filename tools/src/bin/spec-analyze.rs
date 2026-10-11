// Analyze a tree's web specs' sources for the terms they define and their
// references to them (see `tools::spec_analyze`).
//
// Usage:
//   spec-analyze FILES_ROOT ANALYSIS_ROOT SPECS_TOML
//
// SPECS_TOML lists the specs (`[[spec]]`s, see `spec_analyze::SpecsConfig`),
// and each spec whose format we analyze gets the analysis of its source
// written to ANALYSIS_ROOT/<path>, or added to its analysis there if it has
// some (see `semantic_analyzer::merge_records`).  Bikeshed specs are analyzed
// with `bikeshed index --json` (mozsearch's fork's), from the PATH or
// $BIKESHED, unless they have an index configured.  ecmarkup specs are
// analyzed with the biblios their package.json's scripts load (see
// `ecmarkup::Biblio`), so the specs those are of (ECMA-262, and ECMA-402,
// which loads ECMA-262's) are analyzed first.

use std::collections::HashMap;
use std::path::Path;
use std::process::exit;
use std::time::Instant;

use tools::semantic_analyzer::write_records;
use tools::spec_analyze::{SpecFormat, SpecsConfig, bikeshed, ecmarkup, wattsi};

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
    // (The specs whose biblios others load first, in the order they load
    // each other's.)
    let biblio_rank = |spec: &tools::spec_analyze::SpecEntry| {
        let rendered = spec.rendered.as_deref().unwrap_or_default();
        ecmarkup::biblio_urls()
            .position(|url| url == rendered)
            .unwrap_or(usize::MAX)
    };
    let mut specs: Vec<_> = config.specs.iter().collect();
    specs.sort_by_key(|spec| biblio_rank(spec));
    // The biblios of the specs whose biblios others load, by their URLs.
    let mut biblios: HashMap<&str, ecmarkup::Biblio> = HashMap::new();
    for spec in specs {
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
                // (With the files it imports, and the biblios its
                // package.json, in its directory or a parent, loads.)
                let read = |file: &str| std::fs::read_to_string(files_root.join(file)).ok();
                let package_json = Path::new(&path)
                    .ancestors()
                    .skip(1)
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .find_map(|dir| read(&dir.join("package.json").to_string_lossy()))
                    .unwrap_or_default();
                let loaded: Vec<&ecmarkup::Biblio> = ecmarkup::loaded_biblios(&package_json)
                    .into_iter()
                    .filter_map(|url| biblios.get(url))
                    .collect();
                let Some((files, stats, biblio)) = ecmarkup::analyze_with_imports(
                    &path,
                    &read,
                    spec.shortname(),
                    &config,
                    &loaded,
                ) else {
                    continue;
                };
                let loaded_count = loaded.len();
                if let Some(url) = ecmarkup::biblio_urls().find(|url| spec.rendered.as_deref() == Some(*url)) {
                    biblios.insert(url, biblio);
                }
                let file_count = files.len();
                for (file, records) in files {
                    write_records(analysis_root, &file, records);
                }
                println!(
                    "{} (and {} imported files, with {} biblios): {} clauses, {} abstract operations, {} terms, {} productions, {} autolinks, {} xrefs, {} nonterminals ({} unresolved), {} references to the biblios, in {:.2}s",
                    path,
                    file_count - 1,
                    loaded_count,
                    stats.clauses,
                    stats.operations,
                    stats.terms,
                    stats.productions,
                    stats.autolinks,
                    stats.xrefs,
                    stats.nonterminals,
                    stats.unresolved,
                    stats.biblio_references,
                    started.elapsed().as_secs_f64()
                );
            }
            SpecFormat::Bikeshed => {
                // (Bikeshed's index of the spec, unless it's configured.)
                let json = match &spec.index {
                    Some(index) => std::fs::read_to_string(config.dir.join(index))
                        .map_err(|e| format!("{}: {}", index, e)),
                    None => bikeshed::run_bikeshed(&files_root.join(&path)),
                };
                let index = match json.and_then(|json| bikeshed::parse_index(&json)) {
                    Ok(index) => index,
                    Err(e) => {
                        eprintln!("spec-analyze: {}: {}", path, e);
                        continue;
                    }
                };
                // (With the files it includes.)
                let read = |file: &str| std::fs::read_to_string(files_root.join(file)).ok();
                let Some((files, stats)) =
                    bikeshed::analyze_with_includes(&path, &read, &index, spec.shortname(), &config)
                else {
                    continue;
                };
                let file_count = files.len();
                for (file, records) in files {
                    write_records(analysis_root, &file, records);
                }
                println!(
                    "{} (and {} included files): {} dfns, {} headings, {} links ({} to specs which aren't the tree's), {} without positions, in {:.2}s",
                    path,
                    file_count - 1,
                    stats.dfns,
                    stats.headings,
                    stats.links,
                    stats.external,
                    stats.unplaced,
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }
}
