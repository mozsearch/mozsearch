// Build or update the "forest" repository of a tree of web specs (see
// `tools::spec_forest`): one repository whose top-level directories are the
// specs' upstream repositories, with a commit for each of their first-parent
// commits.
//
// Usage:
//   build-spec-forest [--fetch] SPECS_TOML FOREST_REPO
//
// SPECS_TOML lists the specs (`[[spec]]`s with `name`, `url`, and `branch`)
// and the forest's root `.gitattributes` (`attributes`).  FOREST_REPO is
// created if it doesn't exist.  With --fetch, the specs' branches are fetched
// first (into `refs/upstream/<spec>`); otherwise the previously fetched ones
// are used.  If FOREST_REPO has a working tree, it's reset to the forest's
// branch afterwards.

use std::path::Path;
use std::process::{Command, exit};

use git2::Repository;
use tools::spec_forest::{FOREST_REF, ForestConfig, fetch_upstreams, update_forest};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fetch = args.first().is_some_and(|arg| arg == "--fetch");
    if fetch {
        args.remove(0);
    }
    if args.len() != 2 {
        eprintln!("Usage: build-spec-forest [--fetch] SPECS_TOML FOREST_REPO");
        exit(1);
    }
    let (config_path, forest) = (Path::new(&args[0]), Path::new(&args[1]));
    let fail = |e: String| -> ! {
        eprintln!("build-spec-forest: {}", e);
        exit(1);
    };
    let config = ForestConfig::load(config_path).unwrap_or_else(|e| fail(e));
    let repo = match Repository::open(forest) {
        Ok(repo) => repo,
        Err(_) => {
            let repo = Repository::init(forest).unwrap_or_else(|e| fail(e.to_string()));
            repo.set_head(FOREST_REF)
                .unwrap_or_else(|e| fail(e.to_string()));
            repo
        }
    };
    if fetch {
        fetch_upstreams(forest, &config).unwrap_or_else(|e| fail(e));
    }
    let update = update_forest(forest, &config).unwrap_or_else(|e| fail(e));
    for (name, count) in &update.appended {
        println!("{}: {} new commits", name, count);
    }
    if let Some(tip) = update.tip {
        println!("Forest tip: {}", tip);
        if let Some(workdir) = repo.workdir() {
            let status = Command::new("git")
                .arg("-C")
                .arg(workdir)
                .args(["reset", "--quiet", "--hard", FOREST_REF])
                .status();
            if !status.is_ok_and(|status| status.success()) {
                fail("Couldn't reset the forest's working tree".to_string());
            }
        }
    }
}
