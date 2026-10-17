//! Precompute a tree's dataflow summaries (see `tools::dataflow::summary`):
//! each function's flows between its parameters, fields, globals, returns
//! and calls, from the dataflow frontends' IR of each listed file, and the
//! edges between fields those flows are once composed through calls.
//! crossref rolls the edges up into classes' and subsystems' data roll-ups.
//!
//! Writes, into the tree's index directory:
//! - `dataflow-summaries`: a summary per line (JSON), by function symbol;
//! - `dataflow-fields`: an edge between fields per line (JSON).

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
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
}

/// A source file's text: the tree's, or, for generated files, the objdir's.
fn read_source(files_path: &str, objdir_path: &str, path: &str) -> Option<String> {
    let full = match path.strip_prefix("__GENERATED__/") {
        Some(rest) => format!("{}/{}", objdir_path, rest),
        None => format!("{}/{}", files_path, path),
    };
    std::fs::read_to_string(full).ok()
}

/// The summaries of a file's functions with bodies, or why it has none.
fn summarize_file(
    index_path: &str,
    files_path: &str,
    objdir_path: &str,
    path: &str,
) -> Result<Vec<Summary>, String> {
    let source = read_source(files_path, objdir_path, path).ok_or("no source")?;
    let analysis = format!("{}/analysis/{}", index_path, path);
    let values = read_analysis_values(&analysis)
        .or_else(|| read_analysis_values(&format!("{}.gz", analysis)))
        .ok_or("no analysis")?;
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
    let mut summaries: Vec<Summary> = vec![];
    let mut failed = 0;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let (paths, files, next) = (&paths, &files, &next);
                s.spawn(move || {
                    let mut out = vec![];
                    let mut failed = 0;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = files.get(i) else {
                            break;
                        };
                        match summarize_file(
                            &paths.index_path,
                            &paths.files_path,
                            &paths.objdir_path,
                            path,
                        ) {
                            Ok(file_summaries) => out.extend(file_summaries),
                            Err(_) => failed += 1,
                        }
                    }
                    (out, failed)
                })
            })
            .collect();
        for handle in handles {
            let (out, f) = handle.join().unwrap();
            summaries.extend(out);
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
        "dataflow-summarize {}: {} files ({} without source or analysis), {} functions, {} field edges; summarized in {:.1?}, composed in {:.1?}",
        cli.tree_name,
        files.len(),
        failed,
        summaries.len(),
        edges.len(),
        summarized,
        composed
    );
}
