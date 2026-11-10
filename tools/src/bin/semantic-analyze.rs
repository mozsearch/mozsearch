// Analyze data formats with the tree-sitter mappings of
// `tools::semantic_analyzer` (ex: Fluent .ftl files), and documents (Markdown
// and reStructuredText) for their Sphinx labels and references to labels and
// documents (see `tools::rendered_docs::analysis`).
//
// Usage:
//   semantic-analyze [--write-only LIST] [--save-resolvables FILE | --resolvables FILE]
//     FILES_ROOT ANALYSIS_ROOT [DOC_TREES_JSON] < paths
//
// Paths (relative to FILES_ROOT, one per line, ex: the repo-files list) which
// mappings apply to, and documents, get their analysis (all of those mappings'
// records) written to ANALYSIS_ROOT/<path>, or added to its analysis there if
// it has some (ex: C++ files' uses of prefs; see
// `semantic_analyzer::merge_records`).  Files with uses which name
// definitions or constants in any file (see `semantic_analyzer::Resolve`) are
// written once all of the files are analyzed.  DOC_TREES_JSON is the map of the
// documentation's staged layout (see `file_format::doc_trees`), if the tree
// has one.  With --write-only, only the paths LIST lists are written (ex: a
// try push's analyzed files and those it changes; see try-analyze-rest.sh),
// all of the paths still analyzed for what their uses can name, unless
// --resolvables names what the tree's files add to that (by file; see
// `semantic_analyzer::ResolvableParts`), which --save-resolvables saves
// (indexing does, in the index's semantic-resolvables.jsonl), in which case
// only the paths LIST lists are analyzed, and what they add replaces the
// tree's (as do the paths' which aren't there, ex: a try push's removed
// files): on firefox at config8's 2026-11-09 revision, a try push's analysis
// read 13,131 files in 25 s.

use std::collections::HashSet;
use std::io::BufRead;
use std::path::Path;
use std::process::exit;

use rayon::prelude::*;

use tools::file_format::doc_trees::read_doc_trees;
use tools::rendered_docs::analysis::{TreeFiles, doc_records};
use tools::rendered_docs::is_doc_path;
use tools::semantic_analyzer::{
    Analysis, CompiledMapping, Record, ResolvableParts, Resolvables, UnresolvedUse,
    analyze_resolving, builtin_mappings, file_def_record, sort_records, write_records,
};

/// What a file adds to `Resolvables`, as --save-resolvables saves it, one per
/// line.
#[derive(serde::Deserialize)]
struct SavedParts {
    path: String,
    #[serde(flatten)]
    parts: ResolvableParts,
}

/// The value of the option `name`, taken out of `args`, if it's there.
fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|arg| arg == name)?;
    if i + 1 >= args.len() {
        return None;
    }
    let value = args.remove(i + 1);
    args.remove(i);
    Some(value)
}

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
    let mut args: Vec<String> = std::env::args().collect();
    let read_file = |path: &str| {
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Couldn't read {}: {}", path, e);
            exit(1);
        })
    };
    let write_only: Option<HashSet<String>> = take_option(&mut args, "--write-only").map(|list| {
        read_file(&list)
            .lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect()
    });
    let save_resolvables = take_option(&mut args, "--save-resolvables");
    // (The tree's files' parts, by path, if they're given.)
    let tree_parts: Option<std::collections::HashMap<String, ResolvableParts>> =
        take_option(&mut args, "--resolvables").map(|file| {
            read_file(&file)
                .lines()
                .filter_map(|line| serde_json::from_str::<SavedParts>(line).ok())
                .map(|saved| (saved.path, saved.parts))
                .collect()
        });
    if (args.len() != 3 && args.len() != 4) || (tree_parts.is_some() && write_only.is_none()) {
        eprintln!(
            "Usage: {} [--write-only LIST] [--save-resolvables FILE | --resolvables FILE] FILES_ROOT ANALYSIS_ROOT [DOC_TREES_JSON] < paths\n(--resolvables needs --write-only)",
            args[0]
        );
        exit(1);
    }
    let writes = |path: &str| write_only.as_ref().is_none_or(|list| list.contains(path));
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
    // (What each file analyzed adds, for --save-resolvables, or, with the
    // tree's, to add in the paths' order once they're analyzed, as all of
    // the files' would be.)
    let mut parts: Vec<(&str, ResolvableParts)> = vec![];
    // (With the tree's, only the files written are analyzed.)
    let analyzing: Vec<&String> = paths
        .iter()
        .filter(|path| tree_parts.is_none() || writes(path))
        .collect();
    // The files are analyzed in parallel (rayon), a batch at a time, so that
    // only a batch's analyses are in memory, and then taken in their order
    // (what their uses can name, errors, which wait), and those which don't
    // wait are written in parallel.  (It was single-threaded: ~2 minutes for
    // firefox-disco's.)
    for batch in analyzing.chunks(ANALYSIS_BATCH) {
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
                    let added = resolvables.parts(&analysis);
                    if tree_parts.is_none() {
                        resolvables.add_parts(path, &added);
                    }
                    if !added.is_empty() && (tree_parts.is_some() || save_resolvables.is_some()) {
                        parts.push((path, added));
                    }
                    let Analysis {
                        records,
                        unresolved,
                        ..
                    } = analysis;
                    if writes(path) && unresolved.is_empty() {
                        writing.push((path, records));
                    } else if writes(path) {
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
    if let Some(tree_parts) = &tree_parts {
        // (The files analyzed's, and the tree's others', which the paths
        // still have, in the paths' order.)
        let analyzed_paths: HashSet<&str> = analyzing.iter().map(|path| path.as_str()).collect();
        let mut fresh: std::collections::HashMap<&str, ResolvableParts> =
            std::mem::take(&mut parts).into_iter().collect();
        for path in &paths {
            let path = path.as_str();
            if analyzed_paths.contains(path) {
                if let Some(added) = fresh.remove(path) {
                    resolvables.add_parts(path, &added);
                }
            } else if let Some(added) = tree_parts.get(path) {
                resolvables.add_parts(path, added);
            }
        }
    }
    if let Some(file) = &save_resolvables {
        #[derive(serde::Serialize)]
        struct Saving<'a> {
            path: &'a str,
            #[serde(flatten)]
            parts: &'a ResolvableParts,
        }
        let lines: String = parts
            .iter()
            .map(|(path, parts)| serde_json::to_string(&Saving { path, parts }).unwrap() + "\n")
            .collect();
        if let Err(e) = std::fs::write(file, lines) {
            eprintln!("Couldn't write {}: {}", file, e);
            exit(1);
        }
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
        "Analyzed {} files ({} failed){}; resolved {} of {} uses of definitions and constants in any file",
        analyzed,
        failed,
        match &tree_parts {
            Some(tree_parts) => format!(
                ", with the tree's other {} files' definitions",
                tree_parts.len()
            ),
            None => String::new(),
        },
        resolved_count,
        unresolved_count
    );
}
