// Analyze data formats with the tree-sitter mappings of
// `tools::semantic_analyzer` (ex: Fluent .ftl files).
//
// Usage:
//   semantic-analyze FILES_ROOT ANALYSIS_ROOT < paths
//
// Paths (relative to FILES_ROOT, one per line, ex: the repo-files list) which
// mappings apply to get their analysis (all of those mappings' records) written
// to ANALYSIS_ROOT/<path>.

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::exit;

use tools::semantic_analyzer::{analyze, builtin_mappings};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} FILES_ROOT ANALYSIS_ROOT < paths", args[0]);
        exit(1);
    }
    let (files_root, analysis_root) = (Path::new(&args[1]), Path::new(&args[2]));
    let mappings = builtin_mappings().unwrap_or_else(|e| {
        eprintln!("Bad mapping: {}", e);
        exit(1);
    });
    let (mut analyzed, mut failed) = (0, 0);
    for line in std::io::stdin().lock().lines() {
        let path = line.unwrap();
        let path = path.trim();
        let applying: Vec<_> = mappings.iter().filter(|m| m.applies_to(path)).collect();
        if applying.is_empty() {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(files_root.join(path)) else {
            continue;
        };
        match analyze(&applying, path, &source) {
            Ok(records) => {
                let out_path = analysis_root.join(path);
                if let Some(dir) = out_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let mut out = std::io::BufWriter::new(std::fs::File::create(&out_path).unwrap());
                for record in &records {
                    writeln!(out, "{}", record.to_json()).unwrap();
                }
                analyzed += 1;
            }
            Err(e) => {
                eprintln!("Couldn't analyze {}: {}", path, e);
                failed += 1;
            }
        }
    }
    println!("Analyzed {} files ({} failed)", analyzed, failed);
}
