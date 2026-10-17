//! Precompute a tree's dataflow summaries (see `tools::dataflow::summary`):
//! each function's flows between its parameters, fields, globals, returns
//! and calls, from the dataflow frontends' IR of each listed file, and the
//! edges between fields those flows are once composed through calls.
//! crossref rolls the edges up into classes' and subsystems' data roll-ups.
//!
//! Writes, into the tree's index directory:
//! - `dataflow-summaries`: a summary per line (JSON), by function symbol;
//! - `dataflow-fields`: an edge between fields per line (JSON).
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use clap::Parser;

use tools::dataflow::frontend::{build_file_ir, lang_for_path};
use tools::dataflow::records::FileRecords;
use tools::dataflow::summary::{Summary, field_edges, structured_overrides, summarize};
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
/// (estimated for gzipped ones), if it has one.
fn analysis_file(index_path: &str, path: &str) -> Option<(String, u64)> {
    let plain = format!("{}/analysis/{}", index_path, path);
    if let Ok(meta) = std::fs::metadata(&plain) {
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

/// The bytes of analysis being summarized, at most the budget's: a file
/// waits until its analysis fits (a file bigger than the budget takes all of
/// it, so it's summarized alone).
struct Budget {
    used: Mutex<u64>,
    freed: Condvar,
    cap: u64,
}

impl Budget {
    fn acquire(&self, bytes: u64) -> u64 {
        let bytes = bytes.min(self.cap);
        let mut used = self.used.lock().unwrap();
        while *used + bytes > self.cap {
            used = self.freed.wait(used).unwrap();
        }
        *used += bytes;
        bytes
    }

    fn release(&self, bytes: u64) {
        *self.used.lock().unwrap() -= bytes;
        self.freed.notify_all();
    }
}

/// Files with at least this much analysis give back the memory their
/// summaries freed (see `release_freed_memory`) when they're done.
const RELEASE_AFTER_BYTES: u64 = 1 << 20;

/// Give the memory freed in all of the threads' heaps back to the system
/// (glibc keeps it in each thread's arena otherwise).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release_freed_memory() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::os::raw::c_int;
    }
    // SAFETY: malloc_trim only releases free memory.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_freed_memory() {}

/// The summaries of a file's functions with bodies, or why it has none.
fn summarize_file(
    files_path: &str,
    objdir_path: &str,
    path: &str,
    analysis: &str,
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
            let mut summary = summarize(f);
            if let Some(structured) = recs.structured(&f.sym) {
                summary.overrides = structured_overrides(structured);
            }
            summary
        })
        .collect())
}

fn main() {
    env_logger::init();
    let cli = DataflowSummarizeCli::parse();
    let cfg = config::load(&cli.config_file, false, Some(&cli.tree_name), None, None);
    let tree_config = cfg.trees.get(&cli.tree_name).unwrap();
    let paths = &tree_config.paths;

    let list = File::open(&cli.analysis_files_list).unwrap();
    let files: Vec<String> = BufReader::new(list)
        .lines()
        .map_while(|l| l.ok())
        .filter(|path| lang_for_path(path).is_some())
        .collect();

    let start = Instant::now();
    // (Each thread takes the next file, as files' sizes vary a lot.)
    let thread_count = cli.thread_count.max(1);
    let next = AtomicUsize::new(0);
    let budget = Budget {
        used: Mutex::new(0),
        freed: Condvar::new(),
        cap: cli
            .analysis_budget_mb
            .map_or_else(analysis_budget, |mb| mb << 20),
    };
    let mut summaries: Vec<Summary> = vec![];
    let mut failed = 0;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let (paths, files, next, budget) = (&paths, &files, &next, &budget);
                s.spawn(move || {
                    // (Serialized, a line each, so that what summarizing a
                    // file allocates is freed in whole pages, which
                    // `release_freed_memory` can give back, rather than
                    // pinned by the summaries kept between them.)
                    let mut out: Vec<u8> = vec![];
                    let mut failed = 0;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = files.get(i) else {
                            break;
                        };
                        let Some((analysis, size)) = analysis_file(&paths.index_path, path) else {
                            failed += 1;
                            continue;
                        };
                        let taken = budget.acquire(size);
                        match summarize_file(&paths.files_path, &paths.objdir_path, path, &analysis)
                        {
                            Ok(file_summaries) => {
                                for summary in &file_summaries {
                                    serde_json::to_writer(&mut out, summary).unwrap();
                                    out.push(b'\n');
                                }
                            }
                            Err(_) => failed += 1,
                        }
                        if size >= RELEASE_AFTER_BYTES {
                            release_freed_memory();
                        }
                        budget.release(taken);
                    }
                    (out, failed)
                })
            })
            .collect();
        for handle in handles {
            let (out, f) = handle.join().unwrap();
            summaries.extend(
                out.split(|b| *b == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| serde_json::from_slice::<Summary>(line).unwrap()),
            );
            failed += f;
        }
    });
    // (Several files can define a function, ex: a header's inline one in each
    // platform's generated copy; the first, by path, wins.)
    summaries.sort_by(|a, b| a.sym.cmp(&b.sym).then(a.path.cmp(&b.path)));
    summaries.dedup_by(|a, b| a.sym == b.sym);
    let summarized = start.elapsed();

    let edges = field_edges(&summaries);
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

    println!(
        "dataflow-summarize {}: {} files ({} without source or analysis), {} functions, {} field edges; summarized in {:.1?} (at most {} MB of analysis at once), composed in {:.1?}",
        cli.tree_name,
        files.len(),
        failed,
        summaries.len(),
        edges.len(),
        summarized,
        budget.cap >> 20,
        composed
    );
}
