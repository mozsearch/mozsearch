//! This tool merges the analysis data in the files provided as arguments,
//! and prints the merged analysis data to stdout. The "target" data lines
//! from the input files are emitted to the output, but normalized and
//! deduplicated. The "source" data lines are merged such that the `syntax`
//! and `sym` properties are unioned across all input lines that have a
//! matching (loc, pretty) tuple.
//! This ensures that for a given identifier, only a single context menu
//! item will be displayed for a given "pretty" representation, and that
//! context menu will link to all the symbols from all the input files that
//! match that.
//!
//! With `--batch <out-dir>`, it instead merges, for each tree-relative path
//! read from stdin (one per line), that path's analysis in each of the
//! current directory's `analysis-<platform>` directories, into
//! `<out-dir>/<path>`: what process-gecko-analysis.sh did with a
//! merge-analyses process per file (~250k for firefox), which spent most of
//! its time starting them.  (Callers give each process a chunk of the paths,
//! ex: `parallel --pipe -N 500`, as the analyses' interned strings are never
//! freed.)
//!
//! Note that as this code uses the analysis.rs code for parsing and printing,
//! the emitted output should always be in a consistent/normalized format.

use std::env;
use std::fs::File;
use std::io::{BufRead, BufWriter, Write, stdin, stdout};
use std::path::Path;

extern crate regex;
use regex::Regex;

extern crate env_logger;

extern crate tools;
use tools::file_format::merger::merge_files;

/// The platform of an analysis file's path (`analysis-<platform>/...`), else
/// a name of its position.
fn platform_of(re_platform: &Regex, i: usize, fname: &str) -> String {
    re_platform
        .captures(fname)
        .and_then(|c| c.get(1))
        .map_or(format!("platform-{}", i), |m| m.as_str().to_string())
}

/// Merge each path read from stdin's analyses in the current directory's
/// `analysis-*` directories (in their names' order, as the shell's
/// `analysis-*/<path>` glob had them) into `out_dir`.
fn batch(out_dir: &str, re_platform: &Regex) -> std::io::Result<()> {
    let mut dirs: Vec<String> = std::fs::read_dir(".")?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("analysis-"))
        .collect();
    dirs.sort();
    for line in stdin().lock().lines() {
        let line = line?;
        let path = line.trim_end_matches('\r');
        if path.is_empty() {
            continue;
        }
        let files: Vec<String> = dirs
            .iter()
            .map(|dir| format!("{}/{}", dir, path))
            .filter(|file| Path::new(file).is_file())
            .collect();
        if files.is_empty() {
            continue;
        }
        let platforms: Vec<String> = files
            .iter()
            .enumerate()
            .map(|(i, file)| platform_of(re_platform, i, file))
            .collect();
        let out_path = Path::new(out_dir).join(path);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut writer = BufWriter::new(File::create(&out_path)?);
        merge_files(&files, &platforms, &mut writer);
        writer.flush()?;
    }
    Ok(())
}

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().skip(1).collect();

    if args.is_empty() || (args[0] == "--batch" && args.len() != 2) {
        eprintln!("Usage: merge-analyses <filename> [<filename> [...]]");
        eprintln!("  This tool will merge the analysis data from the given files");
        eprintln!("  and print it to stdout; each line will be in a normalized format.");
        eprintln!("   or: merge-analyses --batch <out-dir> < <paths>");
        eprintln!("  merges each path's analysis-*/<path> files into <out-dir>/<path>.");
        std::process::exit(1);
    }

    // The paths are relative, so don't look for a leading slash, but instead anchor at the front.
    let re_platform = Regex::new(r"^analysis-([^/]+)/").unwrap();

    if args[0] == "--batch" {
        if let Err(e) = batch(&args[1], &re_platform) {
            eprintln!("merge-analyses --batch: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // Build a list of platforms that parallels the list of files in `args`.
    let platforms: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, fname)| platform_of(&re_platform, i, fname))
        .collect();

    merge_files(&args, &platforms, &mut stdout());
}
