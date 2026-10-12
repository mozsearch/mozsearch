// Look up the anchors which trees' files cite but their specs no longer define
// (as of their last indexing, in the lists spec-analyze wrote, CITED, ex:
// firefox-disco's and the wubspecs tree's own) in the specs' tree's history,
// updating its cache (HISTORY/stale-anchors.json), which spec-analyze marks
// those citations with (see `tools::spec_analyze::stale`).  Run when the
// history is updated or regenerated, with Bikeshed (mozsearch's fork's) on
// the PATH or $BIKESHED, for the Bikeshed specs' old revisions.
//
// Usage: stale-anchors SPECS_TOML GIT HISTORY [CITED...]

use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::Instant;

use tools::spec_analyze::{SpecsConfig, stale};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: stale-anchors SPECS_TOML GIT HISTORY [CITED...]");
        exit(1);
    }
    let config = SpecsConfig::load(Path::new(&args[1])).unwrap_or_else(|e| {
        eprintln!("stale-anchors: {}", e);
        exit(1);
    });
    let (git, history) = (Path::new(&args[2]), Path::new(&args[3]));
    let cited: Vec<PathBuf> = args[4..].iter().map(PathBuf::from).collect();
    let syms = stale::read_cited(&cited);
    let wanted: Vec<_> = syms
        .iter()
        .filter_map(|sym| {
            let (spec, anchor) = config.symbol_spec(sym)?;
            Some((spec, anchor, sym.clone()))
        })
        .collect();

    let started = Instant::now();
    let cache_path = stale::Cache::path(history);
    let mut cache = stale::Cache::load(&cache_path);
    let stats = stale::update(&wanted, &config, git, history, &mut cache).unwrap_or_else(|e| {
        eprintln!("stale-anchors: {}", e);
        exit(1);
    });
    if let Err(e) = cache.save(&cache_path) {
        eprintln!("stale-anchors: {}: {}", cache_path.display(), e);
        exit(1);
    }
    println!(
        "Stale anchors: {} cited ({} looked up in their specs' histories, analyzing {} revisions, the others cached), {} defined in their histories, {} tokens followed, in {:.2}s",
        stats.anchors,
        stats.searched,
        stats.analyzed,
        stats.found,
        stats.followed,
        started.elapsed().as_secs_f64()
    );
}
