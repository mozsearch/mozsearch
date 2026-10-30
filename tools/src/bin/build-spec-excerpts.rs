// Write the excerpts of a spec tree's files' rendered views, for search
// results' lines in them (see `format::spec_excerpt`): for each file with a
// rendered view (its specs' sources, and the files its Bikeshed specs include
// and its ecmarkup specs import; see `format::rendered_view_files`), its
// blocks on all of its view's pages, their lines and HTML (see
// `format::spec_excerpt_blocks`), in INDEX_ROOT/spec-excerpts/PATH.tsv (see
// `format::spec_excerpts_path`).  spec-analyze.sh runs this after
// spec-analyze, whose Bikeshed renderings the Bikeshed specs' views are.  (A
// paged spec's view renders its whole source for each page: ex: the HTML
// Standard's 61 pages take about 25 s.)
//
// Usage: build-spec-excerpts CONFIG_FILE TREE_NAME

use std::fs;
use std::io::Write;
use std::time::Instant;

use rayon::prelude::*;
use tools::file_format::config;
use tools::format::{excerpt_line, rendered_view_files, spec_excerpt_blocks, spec_excerpts_path};

/// How many files are rendered at a time (a big spec's render holds its
/// whole source's parse).
const THREADS: usize = 8;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [config_file, tree_name] = args.as_slice() else {
        eprintln!("Usage: build-spec-excerpts CONFIG_FILE TREE_NAME");
        std::process::exit(1);
    };
    let cfg = config::load(config_file, false, Some(tree_name), None, None);
    let Some(tree_config) = cfg.trees.get(tree_name) else {
        eprintln!("No tree {}", tree_name);
        std::process::exit(1);
    };
    let files = rendered_view_files(tree_name, tree_config);
    let root = std::path::Path::new(&tree_config.paths.index_path).join("spec-excerpts");
    let _ = fs::remove_dir_all(&root);
    let start = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .unwrap();
    let results: Vec<(String, Result<usize, String>)> = pool.install(|| {
        files
            .par_iter()
            .map(|path| {
                let started = Instant::now();
                let result = spec_excerpt_blocks(tree_name, tree_config, path)
                    .map_err(str::to_string)
                    .and_then(|blocks| {
                        let out = spec_excerpts_path(tree_config, path);
                        if let Some(dir) = out.parent() {
                            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                        }
                        let mut writer = std::io::BufWriter::new(
                            fs::File::create(&out).map_err(|e| e.to_string())?,
                        );
                        for block in &blocks {
                            writeln!(writer, "{}", excerpt_line(block))
                                .map_err(|e| e.to_string())?;
                        }
                        writer.flush().map_err(|e| e.to_string())?;
                        Ok(blocks.len())
                    });
                println!(
                    "{}: {} ({:.1} s)",
                    path,
                    match &result {
                        Ok(n) => format!("{} blocks", n),
                        Err(e) => format!("no excerpts: {}", e),
                    },
                    started.elapsed().as_secs_f64()
                );
                (path.clone(), result)
            })
            .collect()
    });
    let written = results.iter().filter(|(_, result)| result.is_ok()).count();
    println!(
        "Wrote the excerpts of {} of {} files in {:.1} s",
        written,
        results.len(),
        start.elapsed().as_secs_f64()
    );
}
