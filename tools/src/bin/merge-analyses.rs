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
use std::collections::HashSet;

extern crate rustc_serialize;
use self::rustc_serialize::json::Object;

extern crate tools;
use tools::file_format::analysis::{AnalysisSource, parse_location, read_analyses, read_source, read_target, WithLocation};

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();

    if args.len() == 0 {
        eprintln!("Usage: merge-analyses <filename> [<filename> [...]]");
        eprintln!("  This tool will merge the analysis data from the given files");
        eprintln!("  and print it to stdout; each line will be in a normalized format.");
        std::process::exit(1);
    }

    let mut unique_targets = HashSet::new();

    let src_data = read_analyses(
        &args.iter().map(AsRef::as_ref).collect::<Vec<&str>>(),
        &mut |obj: &Object| {
            // return source objects for so that they come out of `read_analyses` for
            // additional processing below.
            if let Some(src) = read_source(obj) {
                return Some(src);
            }
            // for target objects, just print them back out, but use the `unique_targets`
            // hashset to deduplicate them.
            if let Some(tgt) = read_target(obj) {
                let loc = parse_location(obj.get("loc").unwrap().as_string().unwrap());
                let target_str = format!("{}", WithLocation { data: tgt, loc });
                if !unique_targets.contains(&target_str) {
                    println!("{}", target_str);
                    unique_targets.insert(target_str);
                }
            }
            None
        });

    // For each bucket of source data at a given location, sort the source data by
    // the `pretty` field. This allows us to walk through the bucket and operate
    // with the assumption that entries with the same (location, pretty) tuple are
    // adjacent. If we do run into such entries we merge them to union the tokens
    // in the `syntax` and `sym` fields.
    for mut loc_data in src_data {
        loc_data.data.sort_by(|s1, s2| {
            s1.pretty.cmp(&s2.pretty)
        });
        let mut last_entry : Option<AnalysisSource> = None;
        for analysis_entry in std::mem::replace(&mut loc_data.data, Vec::new()) {
            match last_entry {
                Some(mut e) => {
                    if e.pretty == analysis_entry.pretty {
                        // the (loc, pretty) tuple on `analysis_entry` matches that
                        // on `last_entry` so we merge them
                        e.merge(analysis_entry);
                        last_entry = Some(e);
                    } else {
                        loc_data.data.push(e);
                        last_entry = Some(analysis_entry);
                    }
                }
                None => {
                    last_entry = Some(analysis_entry);
                }
            }
        }
        if let Some(e) = last_entry {
            loc_data.data.push(e);
        }
        print!("{}", loc_data);
    }
}
