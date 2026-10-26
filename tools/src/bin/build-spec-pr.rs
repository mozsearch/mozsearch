// Make the forest commits of a spec's pull request (see
// `tools::spec_forest::pull_request_commits`), for importing it as a try push
// (see `tools::try_import`; the tree's import-try hook runs this, and then
// scripts/import-try-push.sh with what it prints).
//
// Usage:
//   build-spec-pr [--fetch] SPECS_TOML FOREST_REPO SPEC/NUMBER
//
// With --fetch, the pull request's head (GitHub's `refs/pull/NUMBER/head`) is
// fetched from the spec's repository first.  Prints:
//   base: the forest commit of the trunk commit it's based on
//   stack: its commits' forest commits, oldest first
//   head: its head's forest commit
//   rebased: its head rebased onto the forest's tip (also set as
//     `refs/try-rebased/SPEC/NUMBER`), or "conflicts: FILE..." instead

use std::path::Path;
use std::process::exit;

use git2::Repository;
use tools::spec_forest::{ForestConfig, fetch_pull_request, pull_request_commits};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fetch = args.first().is_some_and(|arg| arg == "--fetch");
    if fetch {
        args.remove(0);
    }
    if args.len() != 3 {
        eprintln!("Usage: build-spec-pr [--fetch] SPECS_TOML FOREST_REPO SPEC/NUMBER");
        exit(1);
    }
    let fail = |e: String| -> ! {
        println!("ERROR: {}", e);
        exit(1);
    };
    let (config_path, forest) = (Path::new(&args[0]), Path::new(&args[1]));
    let (name, number) = args[2]
        .split_once('/')
        .and_then(|(name, number)| Some((name, number.parse::<u32>().ok()?)))
        .unwrap_or_else(|| fail(format!("{} isn't SPEC/NUMBER", args[2])));
    let config = ForestConfig::load(config_path).unwrap_or_else(|e| fail(e));
    let spec = config
        .specs
        .iter()
        .find(|spec| spec.name == name)
        .unwrap_or_else(|| fail(format!("{} isn't a spec repository of the tree", name)));
    if fetch {
        println!("Fetching pull request {} of {}", number, name);
        fetch_pull_request(forest, spec, number).unwrap_or_else(|e| fail(e));
    }
    println!("Making the forest commits of pull request {} of {}", number, name);
    let commits = pull_request_commits(forest, spec, number).unwrap_or_else(|e| fail(e));
    let repo = Repository::open(forest).unwrap_or_else(|e| fail(e.to_string()));
    let rebased_ref = format!("refs/try-rebased/{}/{}", name, number);
    println!("base: {}", commits.base);
    println!(
        "stack: {}",
        commits
            .stack
            .iter()
            .map(|oid| oid.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("head: {}", commits.stack.last().unwrap());
    match &commits.rebased {
        Ok(rebased) => {
            repo.reference(&rebased_ref, *rebased, true, "build-spec-pr")
                .unwrap_or_else(|e| fail(e.to_string()));
            println!("rebased: {}", rebased);
        }
        Err(conflicts) => {
            if let Ok(mut reference) = repo.find_reference(&rebased_ref) {
                let _ = reference.delete();
            }
            println!("conflicts: {}", conflicts.join(" "));
        }
    }
}
