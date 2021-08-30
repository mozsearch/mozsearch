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

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::hash::Hash;
use std::hash::Hasher;

extern crate regex;
use regex::Regex;
use serde_json::to_value;
use serde_json::{from_value, json, to_string, Value};
use tools::file_format::analysis::AnalysisUnion;

extern crate env_logger;

extern crate tools;
use tools::file_format::analysis::{
    read_analyses, AnalysisSource, AnalysisStructured, Location, WithLocation,
};

#[derive(Debug)]
pub struct HashedStructured {
    pub platforms: Vec<usize>,
    pub loc: Location,
    pub data: AnalysisStructured,
}

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

    let mut unique_targets = HashSet::new();
    // Maps from symbol name to a HashMap<u64 hash, HashedStructured>
    let mut structured_syms = BTreeMap::new();

    let src_data = read_analyses(&args, &mut |obj: Value, loc: &Location, i_file: usize| {
        if let Ok(unified) = from_value(obj) {
            match unified {
                AnalysisUnion::Source(src) => {
                    // return source objects for so that they come out of `read_analyses` for
                    // additional processing below.
                    return Some(src);
                }
                AnalysisUnion::Target(tgt) => {
                    // for target objects, just print them back out, but use the `unique_targets`
                    // hashset to deduplicate them.
                    let target_str = to_string(&WithLocation {
                        data: tgt,
                        loc: loc.clone(),
                    })
                    .unwrap();
                    if !unique_targets.contains(&target_str) {
                        println!("{}", target_str);
                        unique_targets.insert(target_str);
                    }
                }
                AnalysisUnion::Structured(structured) => {
                    // Structured objects may have different data for different platforms.  We
                    // detect this by building a map for each symbol from the hash of the string
                    // representation of their JSON encoding to the AnalysisStructured
                    // representation.  If, after processing the files we find there was a single
                    // hash, then we emit that record as we originally found it.  However, if there
                    // were multiple hashes, we pick the last.
                    //
                    // We used to have AnalysisStructured be hashable, but the `extra` Map was
                    // not currently hashable due to https://github.com/serde-rs/json/issues/747
                    // and in reality we just want to hash the JSON string, but it's already
                    // been parsed into a Value, which is why we're not using the string.
                    let variants = structured_syms
                        .entry(structured.sym.clone())
                        .or_insert(HashMap::new());
                    let json_str = to_string(&structured).unwrap();
                    let mut hasher = DefaultHasher::new();
                    json_str.hash(&mut hasher);
                    let hash_key = hasher.finish();
                    let hs = variants.entry(hash_key).or_insert(HashedStructured {
                        platforms: vec![],
                        loc: loc.clone(),
                        data: structured,
                    });
                    hs.platforms.push(i_file);
                }
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
        loc_data.data.sort_by(|s1, s2| s1.pretty.cmp(&s2.pretty));
        let mut last_entry: Option<AnalysisSource> = None;
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
        print!("{}", to_string(&loc_data).unwrap());
    }

    for (_id, mut hmap) in structured_syms {
        if hmap.len() == 1 {
            // There was only one variant of the structured info, so we can just use it as-is.
            let (_hash, hs) = hmap.drain().next().unwrap();
            println!(
                "{}",
                to_string(&WithLocation {
                    loc: hs.loc,
                    data: hs.data
                })
                .unwrap()
            );
        } else {
            // There are multiple variants, so we want to:
            // 1. Pick one of the variants as the canonical variant.  For now our heuristic is to
            //    pick the highest platform index.  This is because the platform list is currently
            //    accomplished via wildcard that puts "android-armv7" first and that's a 32-bit
            //    platform, and we'd rather our defaults be 64-bit.
            // 2. Using the `extras` field, populate a `platforms` value in
            //    the canonical variant as well a `variants` field.  This should
            //    allow round-tripping while also avoiding us actually doing
            //    anything with this surplus-ish info which we expect to only be
            //    consumed by front-end JS UI at this time for the purposes of
            //    showing differing memory layouts across platforms.
            //
            // Prior to the conversion to serde_json, the `extras` field was a
            // JSON-string `payload` field and we just did a lot of sketchy
            // gluing together of raw JSON string fragments.

            // Do a pass to pick the best hash.
            let mut best_hash = 0;
            let mut best_plat = 0;
            for (hash, hs) in hmap.iter() {
                let local_plat = hs.platforms.iter().max().unwrap();
                if local_plat >= &best_plat {
                    best_plat = *local_plat;
                    best_hash = *hash;
                }
            }

            let mut hs = hmap.remove(&best_hash).unwrap();
            hs.data.extra.insert(
                "platforms".to_string(),
                json!(hs
                    .platforms
                    .iter()
                    .map(|x| platforms[*x].clone())
                    .collect::<Vec<String>>()),
            );
            hs.data.extra.insert(
                "variants".to_string(),
                hmap.into_values()
                    .map(|mut variant| {
                        variant.data.extra.insert(
                            "platforms".to_string(),
                            json!(variant
                                .platforms
                                .iter()
                                .map(|x| platforms[*x].clone())
                                .collect::<Vec<String>>()),
                        );
                        to_value(&variant.data).unwrap()
                    })
                    .collect(),
            );

            println!(
                "{}",
                to_string(&WithLocation {
                    loc: hs.loc,
                    data: hs.data
                })
                .unwrap()
            );
        }
    }
}
