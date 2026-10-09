// Update the commit index (see `tools::commit_index`), which maps bugs and
// Phabricator revisions to the commits which mention them, for the history of
// a branch of a git repo.  Only commits which weren't processed before are
// processed, unless the index's schema changed.
//
// Usage:
//   build-commit-index GIT_REPO INDEX_DIR [REF]
//
// REF defaults to HEAD.  Trees keep their indexes in `commit-index/` under
// their `history_path` (INDEX_DIR), a directory per branch, which REF names
// (ex: "refs/heads/beta" is "commit-index/beta"; see
// `commit_index::branch_dir`), matching the tree's `git_branch` (or "HEAD").

extern crate env_logger;
extern crate git2;
extern crate tools;

use std::env;
use std::path::Path;
use std::process::exit;
use std::time::Instant;

use git2::Repository;

use tools::commit_index;

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();
    if !(3..=4).contains(&args.len()) {
        eprintln!("Usage: {} GIT_REPO INDEX_DIR [REF]", args[0]);
        exit(1);
    }
    let repo = Repository::open(&args[1]).unwrap_or_else(|e| {
        eprintln!("Couldn't open {}: {}", args[1], e);
        exit(1);
    });
    let refname = args.get(3).map_or("HEAD", String::as_str);
    let head = repo
        .revparse_single(refname)
        .and_then(|object| object.peel_to_commit())
        .unwrap_or_else(|e| {
            eprintln!("Bad ref {}: {}", refname, e);
            exit(1);
        })
        .id();

    let start = Instant::now();
    let branch = commit_index::ref_branch(refname);
    match commit_index::update_branch(&repo, head, Path::new(&args[2]), branch) {
        Ok(count) => println!(
            "Processed {} commits of {} up to {} in {:.1}s",
            count,
            branch,
            head,
            start.elapsed().as_secs_f64()
        ),
        Err(e) => {
            eprintln!("Couldn't update the commit index: {}", e);
            exit(1);
        }
    }
}
