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

use std::fs::{File, FileTimes};
use std::io::{BufRead, BufReader, BufWriter, Write, stdin};

use flate2::Compression;
use flate2::write::GzEncoder;

use tools::file_format::analysis::analysis_values_from;

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

fn main() {
    env_logger::init();
    let mut failed = false;
    for line in stdin().lock().lines() {
        let line = line.expect("Couldn't read the paths");
        let path = line.trim_end_matches('\r');
        if path.is_empty() {
            continue;
        }
        if let Err(e) = normalize(path) {
            eprintln!("normalize-analyses: {}: {}", path, e);
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}
