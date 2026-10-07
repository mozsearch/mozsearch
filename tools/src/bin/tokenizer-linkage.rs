//! How a tree's definitions would link to their history contexts (see
//! `hyperblame::recency::link_definitions`) with the current tokenizer, for
//! checking tokenizer changes before reprocessing the history: for each file,
//! tokenize its source, and link the index's analysis to the tokens'
//! contexts, as crossref links it to the history's.  With `--report N`, the
//! files are the first N of crossref's `diags/crossref/recency-linkage.tsv`
//! (the history's worst), with its counts for comparison; otherwise they're
//! the paths on stdin.
//!
//! Prints a row per file (path, the history's unlinked definitions (or "-"),
//! the tokenizer's unlinked definitions, at the top level, in namespaces, and
//! in other contexts, linked definitions, and examples), and totals.

use std::fs;
use std::io::BufRead;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use ustr::UstrMap;

use tools::file_format::analysis::{read_analysis, read_structured, read_target};
use tools::hyperblame::recency::{CONTEXT_KINDS, FileDigests, FileLinkage, link_definitions};
use tools::tree_sitter_support::cst_tokenizer::hypertokenize_source_file;

#[derive(Parser)]
struct Cli {
    /// The tree's index directory (with `analysis/` and `diags/`).
    index: String,
    /// The tree's source files directory.
    files: String,
    /// Check the first N files of crossref's report, rather than stdin's.
    #[arg(long)]
    report: Option<usize>,
    /// How many threads to use.
    #[arg(long, default_value_t = 16)]
    threads: usize,
}

/// A file to check, with the history's count of its definitions without their
/// own contexts, if crossref reported it.
struct Target {
    path: String,
    history_unlinked: Option<u32>,
}

/// How a file's definitions link to the tokenizer's contexts, or why they
/// can't.
enum Checked {
    Linked(FileLinkage),
    Failed(&'static str),
}

fn check(cli: &Cli, path: &str) -> Checked {
    let Ok(source) = fs::read_to_string(format!("{}/{}", cli.files, path)) else {
        return Checked::Failed("no source");
    };
    let Ok(tokenized) = hypertokenize_source_file(path, &source) else {
        return Checked::Failed("not tokenized");
    };
    if tokenized.profile.lang == "none" {
        return Checked::Failed("no grammar");
    }
    let mut files = tokenized.tokenized.join("\n");
    files.push('\n');
    let Some(digests) = FileDigests::from_tokens(&source, &files, &tokenized.structure, true)
    else {
        return Checked::Failed("not aligned");
    };
    let analysis_fname = format!("{}/analysis/{}.gz", cli.index, path);
    let analysis = read_analysis(&analysis_fname, &mut read_target);
    let mut context_syms: UstrMap<bool> = UstrMap::default();
    for datum in read_analysis(&analysis_fname, &mut read_structured) {
        for piece in datum.data {
            if CONTEXT_KINDS.contains(&piece.kind.as_str()) {
                context_syms.insert(piece.sym, matches!(&*piece.kind, "function" | "method"));
            }
        }
    }
    Checked::Linked(link_definitions(
        &digests,
        &source,
        &analysis,
        &context_syms,
        |_, _, _, _| {},
    ))
}

fn main() {
    let cli = Cli::parse();
    let targets: Vec<Target> = match cli.report {
        Some(count) => {
            let report =
                fs::read_to_string(format!("{}/diags/crossref/recency-linkage.tsv", cli.index))
                    .expect("crossref's report");
            report
                .lines()
                .filter(|line| !line.starts_with('#'))
                .take(count)
                .map(|line| {
                    let mut columns = line.split('\t');
                    Target {
                        path: columns.next().unwrap_or_default().to_string(),
                        history_unlinked: columns.next().and_then(|n| n.parse().ok()),
                    }
                })
                .collect()
        }
        None => std::io::stdin()
            .lock()
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.is_empty())
            .map(|path| Target {
                path,
                history_unlinked: None,
            })
            .collect(),
    };

    let results: Mutex<Vec<Option<Checked>>> =
        Mutex::new((0..targets.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..cli.threads.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(target) = targets.get(i) else {
                        break;
                    };
                    let checked = check(&cli, &target.path);
                    results.lock().unwrap()[i] = Some(checked);
                }
            });
        }
    });

    println!(
        "# path\thistory unlinked\tunlinked\ttop level\tnamespaces\tother contexts\tlinked\texamples (pretty in context at line)"
    );
    let (mut history_total, mut total, mut linked_total) = (0, 0, 0);
    let mut unlinked_totals = [0; 3];
    let mut failed = 0;
    for (target, checked) in targets.iter().zip(results.into_inner().unwrap()) {
        let history = target
            .history_unlinked
            .map_or("-".to_string(), |n| n.to_string());
        match checked.unwrap() {
            Checked::Linked(linkage) => {
                history_total += target.history_unlinked.unwrap_or(0);
                total += linkage.unlinked_total();
                linked_total += linkage.linked;
                for (sum, n) in unlinked_totals.iter_mut().zip(linkage.unlinked) {
                    *sum += n;
                }
                let examples: Vec<String> = linkage
                    .examples
                    .iter()
                    .map(|(pretty, context, line)| format!("{} in {} at {}", pretty, context, line))
                    .collect();
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    target.path,
                    history,
                    linkage.unlinked_total(),
                    linkage.unlinked[0],
                    linkage.unlinked[1],
                    linkage.unlinked[2],
                    linkage.linked,
                    examples.join("; ")
                );
            }
            Checked::Failed(why) => {
                failed += 1;
                println!("{}\t{}\t{}", target.path, history, why);
            }
        }
    }
    println!(
        "# {} files ({} failed): {} definitions unlinked ({} at the top level, {} in namespaces, {} in other contexts; the history had {}), {} linked",
        targets.len(),
        failed,
        total,
        unlinked_totals[0],
        unlinked_totals[1],
        unlinked_totals[2],
        history_total,
        linked_total
    );
}
