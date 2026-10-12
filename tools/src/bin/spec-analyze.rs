// Analyze a tree's web specs' sources for the terms they define and their
// references to them (see `tools::spec_analyze`).
//
// Usage:
//   spec-analyze [--citations-only] [--spec-files SPEC_FILES_ROOT] FILES_ROOT ANALYSIS_ROOT SPECS_TOML [CITING_PATHS]
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
//
// The files listed in CITING_PATHS (paths relative to FILES_ROOT, one per
// line, ex: the repo-files list), but for the specs' sources (and the files
// they include), get the records of their citations of the specs (see
// `spec_analyze::citations`), added to their analysis.  With
// --citations-only, the specs' sources aren't analyzed, since SPECS_TOML is
// another tree's (ex: firefox-disco's citations of wubspecs' specs), but its
// ecmarkup specs are if they're in SPEC_FILES_ROOT (that tree's files), for
// their clauses' names, which comments can cite their sections by (see
// `citations::Titles`).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::exit;
use std::time::Instant;

use tools::semantic_analyzer::{file_def_record, write_records};
use tools::spec_analyze::{SpecFormat, SpecsConfig, bikeshed, citations, ecmarkup, wattsi};

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    let mut citations_only = false;
    let mut spec_files_root: Option<String> = None;
    while args.len() > 1 && args[1].starts_with("--") {
        match args.remove(1).as_str() {
            "--citations-only" => citations_only = true,
            "--spec-files" if args.len() > 1 => spec_files_root = Some(args.remove(1)),
            _ => args.clear(),
        }
    }
    if args.len() != 4 && args.len() != 5 {
        eprintln!(
            "Usage: spec-analyze [--citations-only] [--spec-files SPEC_FILES_ROOT] FILES_ROOT ANALYSIS_ROOT SPECS_TOML [CITING_PATHS]"
        );
        exit(1);
    }
    let (files_root, analysis_root) = (Path::new(&args[1]), Path::new(&args[2]));
    let config = SpecsConfig::load(Path::new(&args[3])).unwrap_or_else(|e| {
        eprintln!("spec-analyze: {}", e);
        exit(1);
    });
    // The files the specs' analysis wrote, which aren't cited from.
    let mut spec_files: HashSet<String> = HashSet::new();
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
    // The ecmarkup specs' clauses' names, for citations of their sections
    // (ECMA-262's first, and ECMA-402's for comments mentioning Intl).
    let mut titles = citations::Titles::default();
    let mut add_titles = |spec: &tools::spec_analyze::SpecEntry, biblio: &ecmarkup::Biblio| {
        titles.add_spec(spec.shortname(), biblio.clause_names());
        if spec.rendered.as_deref() == ecmarkup::biblio_urls().nth(1) {
            titles.set_intl(spec.shortname());
        }
    };
    // (Only their titles, from the linked tree's files, with --citations-only.)
    if let Some(root) = spec_files_root.as_deref().filter(|_| citations_only) {
        let root = Path::new(root);
        for spec in specs.iter().filter(|spec| spec.format == Some(SpecFormat::Ecmarkup)) {
            let Some(path) = spec.source_path() else {
                continue;
            };
            let read = |file: &str| std::fs::read_to_string(root.join(file)).ok();
            // (Their own clauses, so without the biblios they load.)
            let Some((_, _, biblio)) =
                ecmarkup::analyze_with_imports(&path, &read, spec.shortname(), &config, &[])
            else {
                continue;
            };
            add_titles(spec, &biblio);
        }
    }
    for spec in specs.into_iter().filter(|_| !citations_only) {
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
                spec_files.insert(path.clone());
                println!(
                    "{}: {} headings, {} dfns, {} anchors, {} cross-references ({} unresolved), {} links, in {:.2}s",
                    path,
                    stats.headings,
                    stats.dfns,
                    stats.anchors,
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
                add_titles(spec, &biblio);
                if let Some(url) = ecmarkup::biblio_urls().find(|url| spec.rendered.as_deref() == Some(*url)) {
                    biblios.insert(url, biblio);
                }
                let file_count = files.len();
                for (file, records) in files {
                    write_records(analysis_root, &file, records);
                    spec_files.insert(file);
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
                    spec_files.insert(file);
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

    // Citations of the specs, in the tree's other files.
    let Some(citing_paths) = args.get(4) else {
        return;
    };
    let paths = std::fs::read_to_string(citing_paths).unwrap_or_else(|e| {
        eprintln!("spec-analyze: {}: {}", citing_paths, e);
        exit(1);
    });
    let started = Instant::now();
    let (mut files, mut cited, mut steps, mut sections) = (0, 0, 0, 0);
    for path in paths.lines().map(str::trim).filter(|p| !p.is_empty()) {
        if spec_files.contains(path) {
            continue;
        }
        // (Only text files, and quickly past those without URLs.)
        let Ok(source) = std::fs::read_to_string(files_root.join(path)) else {
            continue;
        };
        if !source.contains("://") {
            continue;
        }
        let analysis_path = analysis_root.join(path);
        let defs = citations::analysis_defs(&analysis_path);
        let (mut records, stats) =
            citations::citation_records(&source, &defs, &config, Some(&titles));
        if records.is_empty() {
            continue;
        }
        // (A file without other analysis gets its file's definition, as
        // semantic-analyze's do.)
        if !analysis_path.exists() {
            records.insert(0, file_def_record(path));
        }
        write_records(analysis_root, path, records);
        files += 1;
        cited += stats.citations;
        steps += stats.steps;
        sections += stats.sections;
    }
    println!(
        "Citations: {} of the specs ({} of sections by their titles) and {} of their steps, in {} files, in {:.2}s",
        cited,
        sections,
        steps,
        files,
        started.elapsed().as_secs_f64()
    );
}
