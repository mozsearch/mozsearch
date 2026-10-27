//! Precompute a tree's dataflow summaries (see `tools::dataflow::summary`):
//! each function's flows between its parameters, fields, globals, returns
//! and calls, from the dataflow frontends' IR of each listed file, and the
//! edges between fields those flows are once composed through calls.
//! crossref rolls the edges up into classes' and subsystems' data roll-ups.
//! The `.ipdl` and `.ipdlh` files' structured records are read first, so
//! that summaries see through the code IPDL generates to the struct fields
//! and message parameters it carries (see `IpdlSlots`).
//!
//! Writes, into the tree's index directory:
//! - `dataflow-summaries`: a summary per line (JSON), by function symbol;
//! - `dataflow-fields`: an edge between fields per line (JSON);
//! - `diags/dataflow-summarize/files.tsv`: each file's analysis bytes, how
//!   long it took, and its functions' count, slowest first.
//!
//! Scheduling: a thread that's free takes the biggest file left whose
//! analysis fits in what's left of the budget (see `BudgetQueue`), so that
//! the big files start first, and small ones fill the threads while they run
//! (rather than files in the list's order, where a thread that took a big
//! file waited for the budget, idle).
//!
//! Memory: summarizing a file takes ~10 times its analysis's size at its peak
//! (ex: ~2 GB for firefox's `mfbt/Assertions.h`, whose analysis is 212 MB, and
//! ~5 GB for each platform's generated `rlbox.wasm.c`), so:
//! - the files being summarized at once have at most `analysis_budget` bytes
//!   of analysis (a bigger file is summarized alone);
//! - the memory summarizing a file of at least `RELEASE_AFTER_BYTES` freed is
//!   given back (see `release_freed_memory`), and the threads keep their
//!   summaries serialized (in one buffer each), so that they don't pin the
//!   pages it freed: otherwise each thread's heap keeps the size of the
//!   biggest files it summarized, which killed firefox-disco's first run (57
//!   GiB of 62 with 16 threads, 6 minutes in).  On 703 big firefox files with
//!   16 threads, one big file at a time peaked at 844 MB without these, and at
//!   300 MB (as one thread does) with them.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use clap::Parser;

use tools::budget_queue::{BudgetQueue, release_freed_memory};
use tools::dataflow::frontend::{build_file_ir, lang_for_path};
use tools::dataflow::records::FileRecords;
use tools::dataflow::summary::{
    IpdlSlots, Summary, field_edges, structured_overrides, summarize_in,
};
use tools::file_format::analysis::read_analysis_values;
use tools::file_format::config;

#[derive(Parser)]
struct DataflowSummarizeCli {
    /// Path to the config.json file.
    #[clap(value_parser)]
    config_file: String,

    /// The tree to summarize.
    #[clap(value_parser)]
    tree_name: String,

    /// A file listing the tree-relative paths of the analysis files to
    /// summarize (those of languages with dataflow frontends).
    #[clap(value_parser)]
    analysis_files_list: String,

    #[clap(value_parser, default_value_t = 8)]
    thread_count: usize,

    /// How many MB of analysis the files being summarized at once may have
    /// (see `analysis_budget`, the default).
    #[clap(long, value_parser)]
    analysis_budget_mb: Option<u64>,
}

/// A source file's text: the tree's, or, for generated files, the objdir's.
fn read_source(files_path: &str, objdir_path: &str, path: &str) -> Option<String> {
    let full = match path.strip_prefix("__GENERATED__/") {
        Some(rest) => format!("{}/{}", objdir_path, rest),
        None => format!("{}/{}", files_path, path),
    };
    std::fs::read_to_string(full).ok()
}

/// The path of a file's analysis (gzipped or not), and its size, uncompressed
/// (estimated for gzipped ones), if it has one.  (An empty file is the
/// marker compress-outputs leaves beside the gzipped one, in an index that's
/// done.)
fn analysis_file(index_path: &str, path: &str) -> Option<(String, u64)> {
    let plain = format!("{}/analysis/{}", index_path, path);
    if let Ok(meta) = std::fs::metadata(&plain)
        && meta.len() > 0
    {
        return Some((plain, meta.len()));
    }
    // (Gzipped analysis is 10 to 15 times smaller.)
    let gz = format!("{}.gz", plain);
    let meta = std::fs::metadata(&gz).ok()?;
    Some((gz, meta.len() * 10))
}

/// How many bytes of analysis the files being summarized at once may have:
/// a 32nd of the machine's memory, so their summaries' peaks (~10 times
/// that) take about a third of it (ex: 2 GB on a 64 GB indexer).
fn analysis_budget() -> u64 {
    let total = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|meminfo| {
            let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
            let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            Some(kb * 1024)
        })
        .unwrap_or(64 << 30);
    total / 32
}

/// Files with at least this much analysis give back the memory their
/// summaries freed (see `release_freed_memory`) when they're done.  (Not
/// smaller ones: with 16 threads, trimming after each of 1,600 1-50 MB files
/// took a third of the time, and the arenas keep at most ~10 times this for
/// each thread.)
const RELEASE_AFTER_BYTES: u64 = 32 << 20;

/// The summaries of a file's functions with bodies, or why it has none.
fn summarize_file(
    files_path: &str,
    objdir_path: &str,
    path: &str,
    analysis: &str,
    ipdl: &IpdlSlots,
) -> Result<Vec<Summary>, String> {
    let source = read_source(files_path, objdir_path, path).ok_or("no source")?;
    let values = read_analysis_values(analysis).ok_or("no analysis")?;
    let recs = FileRecords::from_values(values);
    let ir = build_file_ir(path, &source, &recs)?;
    Ok(ir
        .funcs
        .iter()
        .filter(|f| f.has_body)
        .map(|f| {
            let overrides = recs
                .structured(&f.sym)
                .map(structured_overrides)
                .unwrap_or_default();
            summarize_in(f, ipdl, overrides)
        })
        .collect())
}

/// What the code IPDL generates is (see `IpdlSlots`), from the structured
/// records of the tree's `.ipdl` and `.ipdlh` files' analysis.
fn read_ipdl_slots(index_path: &str, files: &[String]) -> IpdlSlots {
    let mut records = vec![];
    for path in files {
        if !(path.ends_with(".ipdl") || path.ends_with(".ipdlh")) {
            continue;
        }
        let Some((analysis, _)) = analysis_file(index_path, path) else {
            continue;
        };
        records.extend(
            read_analysis_values(&analysis)
                .into_iter()
                .flatten()
                .filter(|value| value.get("structured").is_some()),
        );
    }
    IpdlSlots::from_structured(&records)
}

/// How long summarizing a file took (see `diags/dataflow-summarize`).
struct FileTiming {
    /// Its index in the files.
    path: usize,
    bytes: u64,
    elapsed: std::time::Duration,
    /// Its functions' summaries' count, if it had analysis to summarize.
    functions: Option<usize>,
}

/// Write the files' timings, slowest first, as `files.tsv` in the index's
/// `diags/dataflow-summarize`.
fn write_timings(index_path: &str, files: &[String], timings: &mut [FileTiming]) {
    let dir = Path::new(index_path).join("diags/dataflow-summarize");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(file) = File::create(dir.join("files.tsv")) else {
        return;
    };
    timings.sort_by(|a, b| b.elapsed.cmp(&a.elapsed));
    let mut out = BufWriter::new(file);
    let _ = writeln!(
        out,
        "# path\tanalysis bytes\tseconds\tfunctions (- if it failed)"
    );
    for timing in timings.iter() {
        let _ = writeln!(
            out,
            "{}\t{}\t{:.3}\t{}",
            files[timing.path],
            timing.bytes,
            timing.elapsed.as_secs_f64(),
            timing
                .functions
                .map_or_else(|| "-".to_string(), |n| n.to_string())
        );
    }
    let _ = out.flush();
}

fn main() {
    env_logger::init();
    let cli = DataflowSummarizeCli::parse();
    let cfg = config::load(&cli.config_file, false, Some(&cli.tree_name), None, None);
    let tree_config = cfg.trees.get(&cli.tree_name).unwrap();
    let paths = &tree_config.paths;

    let list = File::open(&cli.analysis_files_list).unwrap();
    let listed: Vec<String> = BufReader::new(list).lines().map_while(|l| l.ok()).collect();
    let start = Instant::now();
    let ipdl = read_ipdl_slots(&paths.index_path, &listed);
    let files: Vec<String> = listed
        .into_iter()
        // (TypeScript's declaration files have no functions' bodies.)
        .filter(|path| lang_for_path(path).is_some() && !path.ends_with(".d.ts"))
        .collect();

    let analyses: Vec<Option<(String, u64)>> = files
        .iter()
        .map(|path| analysis_file(&paths.index_path, path))
        .collect();
    let thread_count = cli.thread_count.max(1);
    let queue = BudgetQueue::new(
        analyses
            .iter()
            .enumerate()
            .filter_map(|(i, analysis)| Some((i, analysis.as_ref()?.1))),
        cli.analysis_budget_mb
            .map_or_else(analysis_budget, |mb| mb << 20),
    );
    let mut summaries: Vec<Summary> = vec![];
    let mut failed = analyses
        .iter()
        .filter(|analysis| analysis.is_none())
        .count();
    // (Each file's time, and the threads' times waiting for the budget.)
    let mut timings: Vec<FileTiming> = vec![];
    let mut waited = std::time::Duration::ZERO;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let (paths, files, analyses, queue, ipdl) =
                    (&paths, &files, &analyses, &queue, &ipdl);
                s.spawn(move || {
                    // (Serialized, a line each, so that what summarizing a
                    // file allocates is freed in whole pages, which
                    // `release_freed_memory` can give back, rather than
                    // pinned by the summaries kept between them.)
                    let mut out: Vec<u8> = vec![];
                    let mut failed = 0;
                    let mut timings = vec![];
                    let mut waited = std::time::Duration::ZERO;
                    loop {
                        let wait_start = Instant::now();
                        let Some((i, taken)) = queue.take() else {
                            break;
                        };
                        waited += wait_start.elapsed();
                        let path = &files[i];
                        let (analysis, size) = analyses[i].as_ref().unwrap();
                        let file_start = Instant::now();
                        let functions = match summarize_file(
                            &paths.files_path,
                            &paths.objdir_path,
                            path,
                            analysis,
                            ipdl,
                        ) {
                            Ok(file_summaries) => {
                                for summary in &file_summaries {
                                    serde_json::to_writer(&mut out, summary).unwrap();
                                    out.push(b'\n');
                                }
                                Some(file_summaries.len())
                            }
                            Err(_) => {
                                failed += 1;
                                None
                            }
                        };
                        if *size >= RELEASE_AFTER_BYTES {
                            release_freed_memory();
                        }
                        queue.release(taken);
                        timings.push(FileTiming {
                            path: i,
                            bytes: *size,
                            elapsed: file_start.elapsed(),
                            functions,
                        });
                    }
                    (out, failed, timings, waited)
                })
            })
            .collect();
        for handle in handles {
            let (out, f, t, w) = handle.join().unwrap();
            summaries.extend(
                out.split(|b| *b == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| serde_json::from_slice::<Summary>(line).unwrap()),
            );
            failed += f;
            timings.extend(t);
            waited += w;
        }
    });
    // (Several files can define a function, ex: a header's inline one in each
    // platform's generated copy; the first, by path, wins.)
    summaries.sort_by(|a, b| a.sym.cmp(&b.sym).then(a.path.cmp(&b.path)));
    summaries.dedup_by(|a, b| a.sym == b.sym);
    let summarized = start.elapsed();

    let edges = field_edges(&summaries, &ipdl);
    let composed = start.elapsed() - summarized;

    let index = Path::new(&paths.index_path);
    let mut out = BufWriter::new(File::create(index.join("dataflow-summaries")).unwrap());
    for summary in &summaries {
        serde_json::to_writer(&mut out, summary).unwrap();
        out.write_all(b"\n").unwrap();
    }
    out.flush().unwrap();
    let mut out = BufWriter::new(File::create(index.join("dataflow-fields")).unwrap());
    for edge in &edges {
        serde_json::to_writer(&mut out, edge).unwrap();
        out.write_all(b"\n").unwrap();
    }
    out.flush().unwrap();

    // (How much of the threads' time summarizing took, and waiting for the
    // budget.)
    let busy: std::time::Duration = timings.iter().map(|timing| timing.elapsed).sum();
    let threads_time = summarized.as_secs_f64() * thread_count as f64;
    write_timings(&paths.index_path, &files, &mut timings);
    println!(
        "dataflow-summarize {}: {} files ({} without source or analysis), {} functions, {} field edges; summarized in {:.1?} (at most {} MB of analysis at once; {} threads {:.0}% busy, {:.0}% waiting for the budget; the slowest file {:.1?}), composed in {:.1?}",
        cli.tree_name,
        files.len(),
        failed,
        summaries.len(),
        edges.len(),
        summarized,
        queue.cap() >> 20,
        thread_count,
        100.0 * busy.as_secs_f64() / threads_time,
        100.0 * waited.as_secs_f64() / threads_time,
        timings
            .first()
            .map(|timing| timing.elapsed)
            .unwrap_or_default(),
        composed
    );
}
