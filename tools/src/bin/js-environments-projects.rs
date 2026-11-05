// js-environments-projects: scip-typescript projects of a tree's JS
// environments (INDEX_ROOT/js-environments.json; see js-environments, and
// tools::js_environments::projects), written into OUT_DIR, in batches by
// environment kind (OUT_DIR/batches/<kind>.<n>.lst, each listing projects),
// for scripts/js-environments-analyze.sh: the environments of the flavors
// FLAVORS names (comma-separated; default: RULES's manifest flavors), but the
// files COVERED lists (ex: those TypeScript projects' analysis covers).
//
// Usage: js-environments-projects FILES_ROOT INDEX_ROOT RULES OUT_DIR
//          [--covered COVERED] [--flavors FLAVORS] [--batch-size N]

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;

use tools::js_environments::discover::Discovery;
use tools::js_environments::projects::{ProjectsOptions, write_projects};
use tools::js_environments::rules::Rules;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "Usage: js-environments-projects FILES_ROOT INDEX_ROOT RULES OUT_DIR [--covered COVERED] [--flavors FLAVORS] [--batch-size N]";
    if args.len() < 5 {
        eprintln!("{}", usage);
        std::process::exit(1);
    }
    let files_root = Path::new(&args[1]);
    let index_root = Path::new(&args[2]);
    let rules = Rules::parse(&fs::read_to_string(&args[3]).expect("Couldn't read RULES"))
        .unwrap_or_else(|e| panic!("{}: {}", args[3], e));
    let out_dir = Path::new(&args[4]);
    let (mut covered, mut flavors, mut batch_size) = (HashSet::new(), None, 100);
    let mut rest = args[5..].iter();
    while let Some(arg) = rest.next() {
        let value = rest.next().unwrap_or_else(|| panic!("{}", usage));
        match arg.as_str() {
            "--covered" => {
                covered = fs::read_to_string(value)
                    .expect("Couldn't read COVERED")
                    .lines()
                    .map(str::to_string)
                    .collect()
            }
            "--flavors" => {
                flavors = Some(
                    value
                        .split(',')
                        .map(str::to_string)
                        .collect::<BTreeSet<_>>(),
                )
            }
            "--batch-size" => batch_size = value.parse().expect("Not a number"),
            _ => panic!("{}", usage),
        }
    }
    let discovery: Discovery = serde_json::from_str(
        &fs::read_to_string(index_root.join("js-environments.json"))
            .expect("Couldn't read js-environments.json"),
    )
    .expect("Couldn't parse js-environments.json");
    let read = |path: &str| fs::read_to_string(files_root.join(path)).ok();
    let summary = write_projects(
        &discovery,
        &rules,
        &read,
        &ProjectsOptions {
            files_root,
            out_dir,
            flavors,
            covered: &covered,
            batch_size,
        },
    )
    .expect("Couldn't write the projects");
    println!(
        "js-environments-projects: {} projects in {} batches:",
        summary.projects.values().sum::<usize>(),
        summary.batches
    );
    for (kind, n) in &summary.projects {
        println!(
            "  {:>7} {} projects, documenting {} files",
            n, kind, summary.documents[kind]
        );
    }
}
