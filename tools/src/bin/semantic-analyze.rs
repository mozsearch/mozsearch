// Analyze data formats with the tree-sitter mappings of
// `tools::semantic_analyzer` (ex: Fluent .ftl files).
//
// Usage:
//   semantic-analyze FILES_ROOT ANALYSIS_ROOT < paths
//
// Paths (relative to FILES_ROOT, one per line, ex: the repo-files list) which
// mappings apply to get their analysis (all of those mappings' records) written
// to ANALYSIS_ROOT/<path>, or added to its analysis there if it has some (ex:
// C++ files' uses of prefs; see `semantic_analyzer::merge_records`).

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::exit;

use tools::semantic_analyzer::{Record, analyze, builtin_mappings, merge_records};

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
        if !mappings.iter().any(|m| m.applies_to(path)) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(files_root.join(path)) else {
            continue;
        };
        let applying: Vec<_> = mappings
            .iter()
            .filter(|m| m.applies_to(path) && m.applies_to_source(&source))
            .collect();
        if applying.is_empty() {
            continue;
        }
        match analyze(&applying, path, &source) {
            Ok(records) => {
                let out_path = analysis_root.join(path);
                let lines = match std::fs::read_to_string(&out_path) {
                    Ok(existing) => merge_records(&existing, records),
                    Err(_) => records.iter().map(Record::to_json).collect(),
                };
                if let Some(dir) = out_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let mut out = std::io::BufWriter::new(std::fs::File::create(&out_path).unwrap());
                for line in &lines {
                    writeln!(out, "{}", line).unwrap();
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
