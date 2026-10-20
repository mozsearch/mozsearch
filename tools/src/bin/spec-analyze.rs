// Analyze a tree's web specs' sources for the terms they define and their
// references to them (see `tools::spec_analyze`).
//
// Usage:
//   spec-analyze [--citations-only] [--spec-files SPEC_FILES_ROOT] [--spec-index SPEC_INDEX_ROOT] [--rendered-specs RENDERED_ROOT] [--bcd BCD_JSON] [--stale-anchors SPEC_TREE SPEC_GIT SPEC_HISTORY CITED] FILES_ROOT ANALYSIS_ROOT SPECS_TOML [CITING_PATHS]
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
// `citations::Titles`), and the specs' definitions are those of
// SPEC_INDEX_ROOT's crossref (that tree's index), if it's given, for
// citations of anchors their specs don't define (see
// `SpecsConfig::fallbacks`).
//
// With --stale-anchors, citations of anchors their specs no longer define but
// once did are marked so, with the revision which removed them and what they
// may be now, as the cache in the history (SPEC_HISTORY) of the specs' tree
// (SPEC_TREE, whose repo is SPEC_GIT: this tree, or the linked tree whose index
// is SPEC_INDEX_ROOT) has them, and the stale anchors the tree cites are
// written to CITED, for the history's next update to look up (see
// `spec_analyze::stale` and the stale-anchors tool).
//
// With --bcd, MDN's browser-compat-data (BCD_JSON, its data.json) gives the
// specs' definitions (unless --citations-only) their MDN pages, and the
// symbols of the tree's WebIDL files (in CITING_PATHS) the specs' definitions
// which they reflect and their MDN pages, in supplement records (see
// `spec_analyze::bcd`).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::exit;
use std::time::Instant;

use tools::file_format::analysis::{AnalysisKind, Location};
use tools::semantic_analyzer::Record;
use tools::semantic_analyzer::{file_def_record, write_records};
use tools::spec_analyze::citations::Defined;
use tools::spec_analyze::{
    SpecEntry, SpecFormat, SpecsConfig, bcd, bikeshed, citations, ecmarkup, respec, stale, wattsi,
    xref,
};

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    let mut citations_only = false;
    let mut spec_files_root: Option<String> = None;
    let mut spec_index_root: Option<String> = None;
    let mut rendered_root: Option<String> = None;
    let mut bcd_path: Option<String> = None;
    // (The specs' tree's name, repo, and history, and where the stale anchors
    // cited go.)
    let mut stale_anchors: Option<(String, String, String, String)> = None;
    while args.len() > 1 && args[1].starts_with("--") {
        match args.remove(1).as_str() {
            "--citations-only" => citations_only = true,
            "--spec-files" if args.len() > 1 => spec_files_root = Some(args.remove(1)),
            "--spec-index" if args.len() > 1 => spec_index_root = Some(args.remove(1)),
            "--rendered-specs" if args.len() > 1 => rendered_root = Some(args.remove(1)),
            "--bcd" if args.len() > 1 => bcd_path = Some(args.remove(1)),
            "--stale-anchors" if args.len() > 4 => {
                let mut values = args.drain(1..5);
                stale_anchors = Some((
                    values.next().unwrap(),
                    values.next().unwrap(),
                    values.next().unwrap(),
                    values.next().unwrap(),
                ));
            }
            _ => args.clear(),
        }
    }
    if args.len() != 4 && args.len() != 5 {
        eprintln!(
            "Usage: spec-analyze [--citations-only] [--spec-files SPEC_FILES_ROOT] [--spec-index SPEC_INDEX_ROOT] [--rendered-specs RENDERED_ROOT] [--bcd BCD_JSON] [--stale-anchors SPEC_TREE SPEC_GIT SPEC_HISTORY CITED] FILES_ROOT ANALYSIS_ROOT SPECS_TOML [CITING_PATHS]"
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
    // The symbols the specs define: their analysis's, or with
    // --citations-only, their tree's crossref's.
    let mut defined = if citations_only {
        spec_index_root.as_deref().and_then(|root| {
            let defined = Defined::crossref(Path::new(root));
            if defined.is_none() {
                eprintln!("spec-analyze: {}: no crossref", root);
            }
            defined
        })
    } else {
        Some(Defined::Symbols(HashSet::new(), HashSet::new()))
    };
    // (And where, for the BCD's supplements of their definitions.)
    let mut definitions: HashMap<String, (String, Location)> = HashMap::new();
    let mut add_defined = |file: &str, records: &[Record]| {
        if let Some(defined) = defined.as_mut() {
            defined.add(records);
        }
        for record in records {
            if let Record::Target(target) = record
                && target.data.kind == AnalysisKind::Def
            {
                definitions
                    .entry(target.data.sym.clone())
                    .or_insert((file.to_string(), target.loc));
            }
        }
    };
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
        for spec in specs
            .iter()
            .filter(|spec| spec.format == Some(SpecFormat::Ecmarkup))
        {
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
    // The Bikeshed specs' analyses, each with a Bikeshed process (1-9 s:
    // wubspecs' 333 specs took 9.2 minutes one after another), on all of the
    // machine's threads, for the specs' loop below to use in their order.
    // (Bikeshed's `index` only reads its own data files and the spec's, and
    // writes its rendering, for --rendered-specs.)
    let mut bikeshed_analyses: HashMap<usize, BikeshedAnalysis> = if citations_only {
        HashMap::new()
    } else {
        analyze_bikeshed_specs(&specs, files_root, rendered_root.as_deref(), &config)
    };
    // The specs' exported definitions, which ReSpec specs' references to
    // other specs' terms are to (see `xref`); so ReSpec specs are analyzed
    // last.
    let mut terms = xref::Terms::default();
    let mut respec_specs: Vec<(&SpecEntry, String, String)> = vec![];
    for (spec_index, spec) in specs.into_iter().enumerate().filter(|_| !citations_only) {
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
                terms.add(spec, &wattsi::exports(&source));
                add_defined(&path, &records);
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
                if let Some(url) =
                    ecmarkup::biblio_urls().find(|url| spec.rendered.as_deref() == Some(*url))
                {
                    biblios.insert(url, biblio);
                }
                let file_count = files.len();
                for (file, records) in files {
                    add_defined(&file, &records);
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
            SpecFormat::Respec => respec_specs.push((spec, path, source)),
            SpecFormat::Bikeshed => {
                let Some(analysis) = bikeshed_analyses.remove(&spec_index) else {
                    continue;
                };
                let (exports, analyzed) = match analysis.result {
                    Ok(result) => result,
                    Err(e) => {
                        eprintln!("spec-analyze: {}: {}", path, e);
                        continue;
                    }
                };
                terms.add(spec, &exports);
                let Some((files, stats)) = analyzed else {
                    continue;
                };
                let file_count = files.len();
                for (file, records) in files {
                    add_defined(&file, &records);
                    write_records(analysis_root, &file, records);
                    spec_files.insert(file);
                }
                println!(
                    "{} (and {} included files): {} dfns, {} headings, {} anchors, {} links ({} to specs which aren't the tree's), {} without positions, in {:.2}s",
                    path,
                    file_count - 1,
                    stats.dfns,
                    stats.headings,
                    stats.anchors,
                    stats.links,
                    stats.external,
                    stats.unplaced,
                    analysis.seconds
                );
            }
        }
    }
    // (With the local scripts they load, for their configs.)
    let scripts = |path: &str, source: &str| -> String {
        let dir = Path::new(path).parent().unwrap_or(Path::new(""));
        respec::local_scripts(source)
            .iter()
            .filter_map(|script| std::fs::read_to_string(files_root.join(dir).join(script)).ok())
            .filter(|text| text.contains("respecConfig"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let respec_specs: Vec<_> = respec_specs
        .into_iter()
        .map(|(spec, path, source)| {
            let scripts = scripts(&path, &source);
            (spec, path, source, scripts)
        })
        .filter(|(spec, path, source, scripts)| {
            let is_respec = respec::is_respec(source, scripts);
            if !is_respec {
                eprintln!(
                    "spec-analyze: {}: {} isn't a ReSpec source",
                    spec.name, path
                );
            }
            is_respec
        })
        .collect();
    // (With the files they include.)
    let read = |file: &str| std::fs::read_to_string(files_root.join(file)).ok();
    for (spec, path, _, scripts) in &respec_specs {
        if let Some((_, _, exports)) = respec::analyze_with_includes(
            path,
            &read,
            scripts,
            spec.shortname(),
            &config,
            &xref::Terms::default(),
        ) {
            terms.add(spec, &exports);
        }
    }
    for (spec, path, _, scripts) in respec_specs {
        let started = Instant::now();
        let Some((files, stats, _)) = respec::analyze_with_includes(
            &path,
            &read,
            &scripts,
            spec.shortname(),
            &config,
            &terms,
        ) else {
            continue;
        };
        let file_count = files.len();
        for (file, records) in files {
            add_defined(&file, &records);
            write_records(analysis_root, &file, records);
            spec_files.insert(file);
        }
        println!(
            "{} (and {} included files): {} sections, {} dfns, {} IDL definitions without dfns, {} references ({} unresolved), {} to other specs' terms (with IDL's), {} to other specs' anchors, in {:.2}s",
            path,
            file_count - 1,
            stats.sections,
            stats.dfns,
            stats.idl,
            stats.references,
            stats.unresolved,
            stats.xrefs,
            stats.external,
            started.elapsed().as_secs_f64()
        );
    }

    // The BCD's MDN pages of the specs' definitions, and later the tree's
    // WebIDL symbols' spec definitions and pages (see `spec_analyze::bcd`).
    let bcd = bcd_path.as_deref().and_then(|path| {
        bcd::Bcd::load(Path::new(path))
            .map_err(|e| eprintln!("spec-analyze: BCD: {}", e))
            .ok()
    });
    if let Some(bcd) = &bcd
        && !citations_only
    {
        let has_meta = |sym: &str| defined.as_ref().is_some_and(|d| d.has_meta(sym));
        let (files, count) = bcd::spec_supplements(bcd, &config, &definitions, &has_meta);
        for (file, records) in files {
            write_records(analysis_root, &file, records);
        }
        println!(
            "BCD ({} features): MDN pages of {} of the specs' definitions",
            bcd.features.len(),
            count
        );
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
    let (mut files, mut cited, mut steps, mut sections, mut fallbacks, mut outdated) =
        (0, 0, 0, 0, 0, 0);
    // (Written once the citations of stale anchors are known.)
    let mut citing: Vec<(String, Vec<Record>)> = vec![];
    // (The files, on all of the machine's threads, in their order: this
    // was single-threaded, ~10 minutes of firefox's indexing.)
    let all_paths: Vec<&str> = paths
        .lines()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut found: Vec<(usize, Vec<Record>, citations::Stats)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut found = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(path) = all_paths.get(i).copied() else {
                            break;
                        };
                        if spec_files.contains(path) {
                            continue;
                        }
                        // (Only text files, and quickly past those without
                        // URLs.)
                        let Ok(source) = std::fs::read_to_string(files_root.join(path)) else {
                            continue;
                        };
                        if !source.contains("://") {
                            continue;
                        }
                        let analysis_path = analysis_root.join(path);
                        let defs = citations::analysis_defs(&analysis_path);
                        let (mut records, stats) = citations::citation_records(
                            &source,
                            &defs,
                            &config,
                            Some(&titles),
                            defined.as_ref(),
                        );
                        if records.is_empty() {
                            continue;
                        }
                        // (A file without other analysis gets its file's
                        // definition, as semantic-analyze's do.)
                        if !analysis_path.exists() {
                            records.insert(0, file_def_record(path));
                        }
                        found.push((i, records, stats));
                    }
                    found
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    found.sort_by_key(|(i, _, _)| *i);
    for (i, records, stats) in found {
        citing.push((all_paths[i].to_string(), records));
        files += 1;
        cited += stats.citations;
        steps += stats.steps;
        sections += stats.sections;
        fallbacks += stats.fallbacks;
        outdated += stats.outdated;
    }

    // Citations of anchors their specs no longer define (but once did).
    if let (Some((tree, git, history, cited_path)), Some(defined)) =
        (&stale_anchors, defined.as_ref())
    {
        let started = Instant::now();
        let mut cited: Vec<String> = vec![];
        let mut seen: HashSet<&str> = HashSet::new();
        for (_, records) in &citing {
            for record in records {
                let Record::Target(target) = record else {
                    continue;
                };
                let sym = target.data.sym.as_str();
                if seen.insert(sym) && !defined.contains(sym) && config.symbol_spec(sym).is_some() {
                    cited.push(sym.to_string());
                }
            }
        }
        if let Err(e) = stale::write_cited(Path::new(cited_path), &cited) {
            eprintln!("spec-analyze: {}: {}", cited_path, e);
        }
        // (The tip's analysis: this tree's, or the linked tree's index's.)
        let analysis = match (citations_only, &spec_index_root) {
            (true, Some(root)) => Path::new(root).join("analysis"),
            _ => analysis_root.to_path_buf(),
        };
        let spec_tree = stale::SpecTree {
            name: tree,
            git: Path::new(git),
            history: Path::new(history),
            analysis: &analysis,
        };
        match stale::stale_anchors(&cited, &spec_tree, &|sym| defined.contains(sym)) {
            Ok((found, suggested)) => {
                let marked: usize = citing
                    .iter_mut()
                    .map(|(_, records)| citations::mark_stale(records, &found, tree))
                    .sum();
                println!(
                    "Stale anchors: {} cited, {} defined in their specs' histories ({} with suggestions), {} citations and steps marked, in {:.2}s",
                    cited.len(),
                    found.len(),
                    suggested,
                    marked,
                    started.elapsed().as_secs_f64()
                );
            }
            Err(e) => eprintln!("spec-analyze: stale anchors: {}", e),
        }
    }
    for (path, records) in citing {
        write_records(analysis_root, &path, records);
    }
    if let Some(bcd) = &bcd {
        let webidl: Vec<(String, String)> = paths
            .lines()
            .map(str::trim)
            .filter(|path| path.ends_with(".webidl"))
            .filter_map(|path| {
                let source = std::fs::read_to_string(files_root.join(path)).ok()?;
                Some((path.to_string(), source))
            })
            .collect();
        let is_defined = |sym: &str| defined.as_ref().is_some_and(|d| d.has_meta(sym));
        let (files, stats) =
            bcd::webidl_supplements(bcd, &webidl, analysis_root, &config, &is_defined);
        for (path, records) in files {
            write_records(analysis_root, &path, records);
        }
        println!(
            "BCD: {} WebIDL symbols in {} files, {} in BCD, {} with their spec definitions, {} with MDN pages",
            stats.symbols,
            webidl.len(),
            stats.in_bcd,
            stats.spec,
            stats.mdn
        );
    }
    println!(
        "Citations: {} of the specs ({} of sections by their titles, {} of anchors their specs don't define, of their series' or successors', {} by outdated URLs) and {} of their steps, in {} files, in {:.2}s",
        cited,
        sections,
        fallbacks,
        outdated,
        steps,
        files,
        started.elapsed().as_secs_f64()
    );
}

/// A Bikeshed spec's analysis (see `analyze_bikeshed_specs`): its exported
/// definitions, and its source's and included files' records and its stats
/// (or None if it has none), or why it couldn't be indexed; and how long it
/// took.
struct BikeshedAnalysis {
    result: Result<(Vec<xref::Export>, Option<BikeshedRecords>), String>,
    seconds: f64,
}

/// A Bikeshed spec's source's and included files' records, and its stats.
type BikeshedRecords = (Vec<(String, Vec<Record>)>, bikeshed::Stats);

/// The Bikeshed specs' analyses, by their positions in `specs` (those with
/// sources), on all of the machine's threads: Bikeshed's index of each spec,
/// unless it's configured, and with `rendered_root`, its rendering, as
/// `rendered_root/<path>.html`, for its rendered view.
fn analyze_bikeshed_specs(
    specs: &[&SpecEntry],
    files_root: &Path,
    rendered_root: Option<&str>,
    config: &SpecsConfig,
) -> HashMap<usize, BikeshedAnalysis> {
    let jobs: Vec<(usize, &SpecEntry, String)> = specs
        .iter()
        .enumerate()
        .filter(|(_, spec)| spec.format == Some(SpecFormat::Bikeshed))
        .filter_map(|(i, spec)| Some((i, *spec, spec.source_path()?)))
        .filter(|(_, _, path)| files_root.join(path).is_file())
        .collect();
    let analyze = |spec: &SpecEntry, path: &str| -> BikeshedAnalysis {
        let started = Instant::now();
        let html = rendered_root.and_then(|root| {
            // (Absolute, since Bikeshed runs in the source's directory.)
            let html = std::path::absolute(Path::new(root).join(format!("{}.html", path))).ok()?;
            std::fs::create_dir_all(html.parent()?).ok()?;
            Some(html)
        });
        let json = match &spec.index {
            Some(index) => {
                if let (Some(output), Some(html)) = (&spec.output, &html)
                    && let Err(e) = std::fs::copy(config.dir.join(output), html)
                {
                    eprintln!("spec-analyze: {}: {}", output, e);
                }
                std::fs::read_to_string(config.dir.join(index))
                    .map_err(|e| format!("{}: {}", index, e))
            }
            None => bikeshed::run_bikeshed(&files_root.join(path), html.as_deref()),
        };
        let result = json
            .and_then(|json| bikeshed::parse_index(&json))
            .map(|index| {
                // (With the files it includes.)
                let read = |file: &str| std::fs::read_to_string(files_root.join(file)).ok();
                let analyzed =
                    bikeshed::analyze_with_includes(path, &read, &index, spec.shortname(), config);
                (bikeshed::exports(&index), analyzed)
            });
        BikeshedAnalysis {
            result,
            seconds: started.elapsed().as_secs_f64(),
        }
    };
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads.min(jobs.len().max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut analyses = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((spec_index, spec, path)) = jobs.get(i) else {
                            break;
                        };
                        analyses.push((*spec_index, analyze(spec, path)));
                    }
                    analyses
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    })
}
