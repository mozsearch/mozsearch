// Analyze data formats with the tree-sitter mappings of
// `tools::semantic_analyzer` (ex: Fluent .ftl files), and documents (Markdown
// and reStructuredText) for their Sphinx labels and references to labels and
// documents (see `tools::rendered_docs::analysis`).
//
// Usage:
//   semantic-analyze FILES_ROOT ANALYSIS_ROOT [DOC_TREES_JSON] < paths
//
// Paths (relative to FILES_ROOT, one per line, ex: the repo-files list) which
// mappings apply to, and documents, get their analysis (all of those mappings'
// records) written to ANALYSIS_ROOT/<path>, or added to its analysis there if
// it has some (ex: C++ files' uses of prefs; see
// `semantic_analyzer::merge_records`).  Files with uses which name
// definitions or constants in any file (see `semantic_analyzer::Resolve`) are
// written once all of the files are analyzed.  DOC_TREES_JSON is the map of the
// documentation's staged layout (see `file_format::doc_trees`), if the tree
// has one.

use std::io::BufRead;
use std::path::Path;
use std::process::exit;

use rayon::prelude::*;

use tools::file_format::doc_trees::read_doc_trees;
use tools::rendered_docs::analysis::{TreeFiles, doc_records};
use tools::rendered_docs::is_doc_path;
use tools::semantic_analyzer::{
    Analysis, CompiledMapping, Record, Resolvables, UnresolvedUse, analyze_resolving,
    builtin_mappings, file_def_record, sort_records, write_records,
};

/// How many files' analyses are in memory at once (see `main`).  (Smaller
/// batches were slower, without saving memory: each thread's state is most
/// of it.)
const ANALYSIS_BATCH: usize = 4096;
/// The most threads analyzing: on firefox's files, 16 threads took 18 s and
/// 1.3 GB, and 32 threads 17 s and 1.7 GB.
const MAX_THREADS: usize = 16;

/// A file's analysis, if mappings apply to it or it's a document (else None).
fn analyze_file(
    mappings: &[CompiledMapping],
    files_root: &Path,
    path: &str,
    files: &TreeFiles,
) -> Option<Result<Analysis, String>> {
    let is_doc = is_doc_path(path);
    if !is_doc && !mappings.iter().any(|m| m.applies_to(path)) {
        return None;
    }
    let source = std::fs::read_to_string(files_root.join(path)).ok()?;
    let applying: Vec<_> = mappings
        .iter()
        .filter(|m| m.applies_to(path) && m.applies_to_source(&source))
        .collect();
    if applying.is_empty() && !is_doc {
        return None;
    }
    let analysis = if applying.is_empty() {
        Ok(Analysis {
            records: vec![file_def_record(path)],
            constants: vec![],
            unresolved: vec![],
        })
    } else {
        analyze_resolving(&applying, path, &source)
    };
    Some(analysis.and_then(|mut analysis| {
        if is_doc {
            // (A document that rendering can't handle shouldn't stop the
            // rest.)
            let docs = std::panic::catch_unwind(|| doc_records(path, &source, files))
                .map_err(|_| "Couldn't render the document".to_string())?;
            analysis.records.extend(docs);
            sort_records(&mut analysis.records);
        }
        Ok(analysis)
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 && args.len() != 4 {
        eprintln!(
            "Usage: {} FILES_ROOT ANALYSIS_ROOT [DOC_TREES_JSON] < paths",
            args[0]
        );
        exit(1);
    }
    let (files_root, analysis_root) = (Path::new(&args[1]), Path::new(&args[2]));
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(
            std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(MAX_THREADS),
        )
        .build_global();
    let mappings = builtin_mappings().unwrap_or_else(|e| {
        eprintln!("Bad mapping: {}", e);
        exit(1);
    });
    // (Documents link to the tree's other files, so we need them all first.)
    let paths: Vec<String> = std::io::stdin()
        .lock()
        .lines()
        .map(|line| line.unwrap().trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    let doc_trees = args
        .get(3)
        .filter(|path| Path::new(path).exists())
        .map(read_doc_trees);
    let files = TreeFiles::new(paths.iter().cloned(), doc_trees);
    // What uses in any file can name (see `semantic_analyzer::Resolve`), and
    // the files with such uses, which wait for all of the files.
    let mut resolvables = Resolvables::new(&mappings);
    let mut resolving: Vec<(&str, Vec<Record>, Vec<UnresolvedUse>)> = vec![];
    let (mut analyzed, mut failed) = (0, 0);
    // The files are analyzed in parallel (rayon), a batch at a time, so that
    // only a batch's analyses are in memory, and then taken in their order
    // (what their uses can name, errors, which wait), and those which don't
    // wait are written in parallel.  (It was single-threaded: ~2 minutes for
    // firefox-disco's.)
    for batch in paths.chunks(ANALYSIS_BATCH) {
        let analyses: Vec<Option<Result<Analysis, String>>> = batch
            .par_iter()
            .map(|path| analyze_file(&mappings, files_root, path, &files))
            .collect();
        let mut writing: Vec<(&str, Vec<Record>)> = vec![];
        for (path, analysis) in batch.iter().zip(analyses) {
            let path = path.as_str();
            match analysis {
                None => {}
                Some(Ok(analysis)) => {
                    resolvables.add(path, &analysis);
                    let Analysis {
                        records,
                        unresolved,
                        ..
                    } = analysis;
                    if unresolved.is_empty() {
                        writing.push((path, records));
                    } else {
                        resolving.push((path, records, unresolved));
                    }
                    analyzed += 1;
                }
                Some(Err(e)) => {
                    eprintln!("Couldn't analyze {}: {}", path, e);
                    failed += 1;
                }
            }
        }
        writing
            .into_par_iter()
            .for_each(|(path, records)| write_records(analysis_root, path, records));
    }
    let unresolved_count: usize = resolving.iter().map(|(_, _, u)| u.len()).sum();
    let mut resolved_count = 0;
    for (path, mut records, unresolved) in resolving {
        match resolvables.resolve(path, &unresolved) {
            Ok(resolved) => {
                // (A source and a target record for each.)
                resolved_count += resolved.len() / 2;
                records.extend(resolved);
            }
            Err(e) => eprintln!("Couldn't resolve {}'s uses: {}", path, e),
        }
        sort_records(&mut records);
        write_records(analysis_root, path, records);
    }
    println!(
        "Analyzed {} files ({} failed); resolved {} of {} uses of definitions and constants in any file",
        analyzed, failed, resolved_count, unresolved_count
    );
}
