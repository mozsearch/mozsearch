// Render a spec pull request's head's Bikeshed specs, for its import as a try
// push (see `tools::spec_forest`'s pull requests; the tree's import-try hook
// runs this after build-spec-pr, in a sandbox, since it runs Bikeshed on the
// pull request's sources): the Bikeshed specs in the pull request's spec
// repository's directory SPEC whose sources' directories it changes from BASE
// (its base's forest commit) to HEAD (its head's), each from HEAD's files into
// OUT_DIR/<source path>.html (`bikeshed index --html`; see
// `spec_analyze::bikeshed::run_bikeshed`), with `bikeshed index`'s JSON as
// OUT_DIR/<source path>.json, for HEAD's rendered views (`/{tree}/rev-rendered/
// HEAD/<source path>`; see `format::bikeshed_rendering_path`).  The other
// formats' specs are rendered by the web server, at any revision.
//
// Usage: render-spec-pr SPECS_TOML FOREST_REPO SPEC BASE HEAD OUT_DIR

use std::fs;
use std::path::Path;
use std::process::exit;

use git2::{ObjectType, Oid, Repository, Tree};
use tools::spec_analyze::bikeshed::run_bikeshed;
use tools::spec_analyze::{SpecFormat, SpecsConfig};

/// Write `tree`'s files under `dir` (but symlinks and submodules).
fn export(repo: &Repository, tree: &Tree, dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for entry in tree.iter() {
        let Ok(name) = entry.name() else {
            continue;
        };
        let path = dir.join(name);
        match entry.kind() {
            Some(ObjectType::Tree) => {
                let subtree = repo.find_tree(entry.id()).map_err(|e| e.to_string())?;
                export(repo, &subtree, &path)?;
            }
            Some(ObjectType::Blob) if entry.filemode() != 0o120000 => {
                let blob = repo.find_blob(entry.id()).map_err(|e| e.to_string())?;
                fs::write(&path, blob.content()).map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        eprintln!("Usage: render-spec-pr SPECS_TOML FOREST_REPO SPEC BASE HEAD OUT_DIR");
        exit(1);
    }
    let fail = |e: String| -> ! {
        println!("ERROR: {}", e);
        exit(1);
    };
    let config = SpecsConfig::load(Path::new(&args[0])).unwrap_or_else(|e| fail(e));
    let repo = Repository::open(&args[1]).unwrap_or_else(|e| fail(e.to_string()));
    let spec = &args[2];
    let tree_of = |rev: &str| -> Tree<'_> {
        Oid::from_str(rev)
            .and_then(|oid| repo.find_commit(oid))
            .and_then(|commit| commit.tree())
            .unwrap_or_else(|e| fail(format!("{}: {}", rev, e)))
    };
    let (base, head) = (tree_of(&args[3]), tree_of(&args[4]));
    // (Absolute, since Bikeshed runs in its source's directory.)
    fs::create_dir_all(&args[5]).unwrap_or_else(|e| fail(e.to_string()));
    let out_dir = fs::canonicalize(&args[5]).unwrap_or_else(|e| fail(e.to_string()));

    // The files the pull request changes in its spec repository.
    let mut options = git2::DiffOptions::new();
    options.pathspec(format!("{}/", spec));
    let diff = repo
        .diff_tree_to_tree(Some(&base), Some(&head), Some(&mut options))
        .unwrap_or_else(|e| fail(e.to_string()));
    let changed: Vec<String> = diff
        .deltas()
        .flat_map(|delta| [delta.old_file().path(), delta.new_file().path()])
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    // Its Bikeshed specs whose sources' directories it changes.
    let sources: Vec<String> = config
        .specs
        .iter()
        .filter(|entry| entry.format == Some(SpecFormat::Bikeshed))
        .filter_map(|entry| entry.source_path())
        .filter(|source| source.starts_with(&format!("{}/", spec)))
        .filter(|source| {
            let dir = Path::new(source).parent().unwrap().to_string_lossy().into_owned();
            changed.iter().any(|path| path.starts_with(&format!("{}/", dir)))
        })
        .collect();
    if sources.is_empty() {
        println!("The pull request changes none of {}'s Bikeshed specs", spec);
        return;
    }

    // Its head's files, for Bikeshed.
    let checkout = std::env::temp_dir().join(format!("render-spec-pr-{}", std::process::id()));
    let _ = fs::remove_dir_all(&checkout);
    let spec_tree = head
        .get_name(spec)
        .and_then(|entry| repo.find_tree(entry.id()).ok())
        .unwrap_or_else(|| fail(format!("{} isn't in {}", spec, args[4])));
    export(&repo, &spec_tree, &checkout.join(spec)).unwrap_or_else(|e| fail(e));
    let mut failed = 0;
    for source in &sources {
        println!("Rendering {} with Bikeshed", source);
        let html = out_dir.join(format!("{}.html", source));
        fs::create_dir_all(html.parent().unwrap()).unwrap_or_else(|e| fail(e.to_string()));
        match run_bikeshed(&checkout.join(source), Some(&html)) {
            Ok(index) => {
                fs::write(out_dir.join(format!("{}.json", source)), index)
                    .unwrap_or_else(|e| fail(e.to_string()));
            }
            Err(e) => {
                // (Its other specs are still rendered.)
                println!("ERROR: rendering {}: {}", source, e);
                failed += 1;
            }
        }
    }
    let _ = fs::remove_dir_all(&checkout);
    println!(
        "Rendered {} of {} Bikeshed specs",
        sources.len() - failed,
        sources.len()
    );
}
