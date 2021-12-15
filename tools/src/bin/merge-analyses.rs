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
//! Note that as this code uses the analysis.rs code for parsing and printing,
//! the emitted output should always be in a consistent/normalized format.

use std::env;
use std::io::stdout;

extern crate regex;
use regex::Regex;

extern crate env_logger;

extern crate tools;
use tools::file_format::merger::merge_files;

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().skip(1).collect();

    if args.len() == 0 {
        eprintln!("Usage: merge-analyses <filename> [<filename> [...]]");
        eprintln!("  This tool will merge the analysis data from the given files");
        eprintln!("  and print it to stdout; each line will be in a normalized format.");
        std::process::exit(1);
    }

    // The paths are relative, so don't look for a leading slash, but instead anchor at the front.
    let re_platform = Regex::new(r"^analysis-([^/]+)/").unwrap();

    // Build a list of platforms that parallels the list of files in `args`.
    let platforms: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, fname)| {
            re_platform
                .captures(fname)
                .and_then(|c| c.get(1))
                .map_or(format!("platform-{}", i), |m| m.as_str().to_string())
        })
        .collect();

    merge_files(&args, &platforms, &mut stdout());
}
