//! Write a tree's analysis files as their readers read them, gzipped: for
//! each path (relative to the current directory, the tree's `analysis/`)
//! read from stdin, one per line, read its (uncompressed) analysis records,
//! escape their symbols' commas and process its locals' symbols (see
//! `analysis::read_analysis_values` and `local_symbols`), write them to
//! `<path>.gz`, and leave in its place a zero-length marker with the gzipped
//! file's times, for nginx's `try_files` (as compress-outputs.sh's
//! gzip-with-markers.sh does for the tree's other outputs).
//!
//! Readers still process what they read (which changes nothing, as
//! processing is idempotent; see `local_symbols`'s tests), but the data
//! API's `analysis` route can serve the files as they are, gzipped, rather
//! than parsing, processing and serializing each whole file in memory (see
//! `LocalIndex::normalized_analysis_file`).  compress-outputs.sh runs this,
//! and then writes the tree's `analysis-normalized` stamp, which says so.
//!
//! (A path is read uncompressed even if it ends in `.gz`: a source file of
//! the tree can be named so, ex: `simple.css.gz`, whose analysis is
//! `simple.css.gz`, written to `simple.css.gz.gz`.  compress-outputs.sh
//! passes those first, so that a file `simple.css`'s `simple.css.gz` then
//! replaces its marker.)
//!
//! The files are normalized on a thread per CPU, which take the biggest
//! files first, under a budget of the analysis being normalized at once (see
//! `tools::budget_queue` and `analysis_budget`): a file's records take ~7.5
//! times its size (358 MB for a 48 MB analysis).

use std::fs::{File, FileTimes};
use std::io::{BufRead, BufReader, BufWriter, Write, stdin};
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::Compression;
use flate2::write::GzEncoder;

use tools::budget_queue::{BudgetQueue, release_freed_memory};
use tools::file_format::analysis::analysis_values_from;
use tools::memory_budget::mem_total;

fn normalize(path: &str) -> std::io::Result<()> {
    let records = analysis_values_from(BufReader::new(File::open(path)?), path);
    let gz_path = format!("{}.gz", path);
    // (Buffered before the encoder too: serde_json writes in fragments, each
    // of which the encoder's write would zero its output buffer for, which
    // was most of the time.)
    let mut writer = BufWriter::with_capacity(
        1 << 20,
        GzEncoder::new(
            BufWriter::new(File::create(&gz_path)?),
            Compression::default(),
        ),
    );
    for record in &records {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    writer
        .into_inner()
        .map_err(|e| e.into_error())?
        .finish()?
        .flush()?;
    let gz = std::fs::metadata(&gz_path)?;
    let marker = File::create(path)?;
    marker.set_times(
        FileTimes::new()
            .set_accessed(gz.accessed()?)
            .set_modified(gz.modified()?),
    )?;
    Ok(())
}

/// How many bytes of analysis the files being normalized at once may have:
/// a 64th of the machine's memory, so their records (~7.5 times that) take
/// about an eighth of it (ex: 2 GB of analysis, ~15 GB, on config8's 128 GB
/// indexers, where compress-outputs peaked at ~26 GB with `parallel`'s
/// chunks of 500 paths on 2026-10-27).  (The threads start with the biggest
/// files, so they take all of it at first.)
fn analysis_budget() -> u64 {
    mem_total().unwrap_or(64 << 30) / 64
}

/// Files with at least this much analysis give back the memory their records
/// freed when they're done (see `release_freed_memory`), as
/// dataflow-summarize's do.
const RELEASE_AFTER_BYTES: u64 = 32 << 20;

fn main() {
    env_logger::init();
    let paths: Vec<String> = stdin()
        .lock()
        .lines()
        .map(|line| line.expect("Couldn't read the paths"))
        .map(|line| line.trim_end_matches('\r').to_string())
        .filter(|path| !path.is_empty())
        .collect();
    // (A file which can't be read takes no budget, and fails below.)
    let sizes = paths
        .iter()
        .map(|path| std::fs::metadata(path).map_or(0, |meta| meta.len()));
    let queue = BudgetQueue::new(sizes.enumerate(), analysis_budget());
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(paths.len().max(1));
    let failed = AtomicBool::new(false);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                while let Some((i, taken)) = queue.take() {
                    let path = &paths[i];
                    if let Err(e) = normalize(path) {
                        eprintln!("normalize-analyses: {}: {}", path, e);
                        failed.store(true, Ordering::Relaxed);
                    }
                    if taken >= RELEASE_AFTER_BYTES {
                        release_freed_memory();
                    }
                    queue.release(taken);
                }
            });
        }
    });
    if failed.into_inner() {
        std::process::exit(1);
    }
}
