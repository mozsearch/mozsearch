//! The Rust files whose analysis a try push can change (see
//! scripts/try-analyze-rust.sh), one path per line: the `.rs` files it
//! changes (those of CHANGED at FILES_ROOT, its head's checkout), and the
//! tree's `.rs` files at its head which use the symbols they define (by the
//! tree's analysis of them and its crossref), whose records a change to the
//! definitions can change (ex: a renamed or removed item's uses), up to
//! `--max-users` of them, those using the most of the symbols first.  It says
//! how many it found on stderr.
//!
//! Usage: try-rust-files CONFIG TREE FILES_ROOT CHANGED [--max-users N]
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use clap::Parser;

use tools::file_format::analysis::read_analysis_values;
use tools::file_format::bisectable_mmap::BisectableMmap;
use tools::file_format::config;
use tools::file_format::crossref::CrossrefData;

#[derive(Parser)]
struct Cli {
    /// The config file.
    config_file: String,
    /// The tree.
    tree_name: String,
    /// The push's head's checkout.
    files_root: String,
    /// The files the push changes, one path per line.
    changed: String,
    /// The most files using the changed files' symbols to list.
    #[clap(long, default_value_t = 1000)]
    max_users: usize,
}

/// The symbols a file's analysis (gzipped or not) defines or declares, but
/// its locals'.
fn defined_symbols(analysis: &Path) -> BTreeSet<String> {
    let gz = format!("{}.gz", analysis.display());
    let path = if Path::new(&gz).exists() {
        gz
    } else {
        analysis.display().to_string()
    };
    read_analysis_values(&path)
        .unwrap_or_default()
        .iter()
        .filter(|value| value.get("target").is_some())
        .filter(|value| matches!(value["kind"].as_str(), Some("def" | "decl")))
        .filter_map(|value| value["sym"].as_str())
        .filter(|sym| !sym.is_empty() && !sym.starts_with("V_"))
        .map(String::from)
        .collect()
}

fn main() {
    let cli = Cli::parse();
    let cfg = config::load(&cli.config_file, false, Some(&cli.tree_name), None, None);
    let tree_config = cfg.trees.get(&cli.tree_name).expect("No such tree");
    let index_path = &tree_config.paths.index_path;
    let files_root = Path::new(&cli.files_root);

    let changed: Vec<String> = std::fs::read_to_string(&cli.changed)
        .expect("Couldn't read the changed files")
        .lines()
        .filter(|path| path.ends_with(".rs") && !path.starts_with("__GENERATED__/"))
        .map(String::from)
        .collect();
    let mut files: BTreeSet<String> = changed
        .iter()
        .filter(|path| files_root.join(path).is_file())
        .cloned()
        .collect();

    // The tree's files using the symbols the changed files define, by how
    // many of them they use.
    let mut users: BTreeMap<String, usize> = BTreeMap::new();
    // (`BisectableMmap::new` can't map a missing or empty file.)
    let (inline, extra) = (
        format!("{}/crossref", index_path),
        format!("{}/crossref-extra", index_path),
    );
    let crossref = std::fs::metadata(&inline)
        .is_ok_and(|meta| meta.len() > 0 && Path::new(&extra).exists())
        .then(|| BisectableMmap::<CrossrefData>::new(&inline, &extra))
        .flatten();
    let mut symbols = 0;
    if let Some(crossref) = &crossref {
        for path in &changed {
            for sym in defined_symbols(&Path::new(index_path).join("analysis").join(path)) {
                symbols += 1;
                let Ok(Some(data)) = crossref.lookup(&sym) else {
                    continue;
                };
                let used_in: BTreeSet<&str> = [&data.uses, &data.assignments]
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|result| result.tree.is_none())
                    .map(|result| result.path.as_str())
                    .filter(|path| path.ends_with(".rs") && !path.starts_with("__GENERATED__/"))
                    .collect();
                for path in used_in {
                    *users.entry(path.to_string()).or_default() += 1;
                }
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = users
        .into_iter()
        .filter(|(path, _)| !files.contains(path) && files_root.join(path).is_file())
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let found = ranked.len();
    files.extend(ranked.into_iter().take(cli.max_users).map(|(path, _)| path));
    eprintln!(
        "try-rust-files: {} changed files, defining {} symbols, used in {} other files ({} listed)",
        changed.len(),
        symbols,
        found,
        found.min(cli.max_users)
    );
    for path in files {
        println!("{}", path);
    }
}
