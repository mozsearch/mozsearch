// Analyze a spec's pull request's head rebased onto the tree's revision, for
// its semantic delta (see `tools::try_delta`; the tree's import-try hook runs
// this after build-spec-pr, in a sandbox, since it runs Bikeshed on the pull
// request's sources, and then build-try-delta with OUT_DIR/analysis): the
// specs in the pull request's spec repository's directory SPEC whose sources'
// directories REBASED (build-spec-pr's rebased head) changes from its parent
// (the tree's revision), with spec-analyze's `--only-specs`, over the tree's
// files (FILES_ROOT, as OUT_DIR/files' links to them) but SPEC's, which are
// REBASED's, and with the other specs' exported definitions as the index has
// them (its spec-terms.json; see spec-analyze's `--terms-out`), so that their
// records are as the tree's indexing would make them.  Their analysis goes in
// OUT_DIR/analysis, and their Bikeshed renderings in INDEX_ROOT/try-rendered/
// REBASED (for REBASED's rendered views; see render-spec-pr).
//
// Usage: analyze-spec-pr SPECS_TOML FOREST_REPO FILES_ROOT INDEX_ROOT SPEC REBASED OUT_DIR

use std::fs;
use std::path::Path;
use std::process::{Command, exit};

use git2::{ObjectType, Oid, Repository, Tree};
use tools::spec_analyze::SpecsConfig;

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
    if args.len() != 7 {
        eprintln!(
            "Usage: analyze-spec-pr SPECS_TOML FOREST_REPO FILES_ROOT INDEX_ROOT SPEC REBASED OUT_DIR"
        );
        exit(1);
    }
    let fail = |e: String| -> ! {
        println!("ERROR: {}", e);
        exit(1);
    };
    let (specs_toml, files_root, index_root) =
        (Path::new(&args[0]), Path::new(&args[2]), Path::new(&args[3]));
    let (spec, out_dir) = (&args[4], Path::new(&args[6]));
    let config = SpecsConfig::load(specs_toml).unwrap_or_else(|e| fail(e));
    let repo = Repository::open(&args[1]).unwrap_or_else(|e| fail(e.to_string()));
    let rebased = Oid::from_str(&args[5])
        .and_then(|oid| repo.find_commit(oid))
        .unwrap_or_else(|e| fail(format!("{}: {}", args[5], e)));
    let base = rebased.parent(0).unwrap_or_else(|e| fail(e.to_string()));
    let (base_tree, rebased_tree) = (
        base.tree().unwrap_or_else(|e| fail(e.to_string())),
        rebased.tree().unwrap_or_else(|e| fail(e.to_string())),
    );

    // The specs whose sources' directories it changes.
    let mut options = git2::DiffOptions::new();
    options.pathspec(format!("{}/", spec));
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&rebased_tree), Some(&mut options))
        .unwrap_or_else(|e| fail(e.to_string()));
    let changed: Vec<String> = diff
        .deltas()
        .flat_map(|delta| [delta.old_file().path(), delta.new_file().path()])
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let names: Vec<String> = config
        .specs
        .iter()
        .filter(|entry| {
            entry.source_path().is_some_and(|source| {
                let dir = Path::new(&source)
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                source.starts_with(&format!("{}/", spec))
                    && changed
                        .iter()
                        .any(|path| path.starts_with(&format!("{}/", dir)))
            })
        })
        .map(|entry| entry.name.clone())
        .collect();
    let analysis = out_dir.join("analysis");
    fs::create_dir_all(&analysis).unwrap_or_else(|e| fail(e.to_string()));
    if names.is_empty() {
        println!("The pull request changes none of {}'s specs", spec);
        return;
    }
    println!("Analyzing {}", names.join(", "));

    // The tree's files, but SPEC's, which are the rebased head's.
    let files = out_dir.join("files");
    let _ = fs::remove_dir_all(&files);
    fs::create_dir_all(&files).unwrap_or_else(|e| fail(e.to_string()));
    for entry in fs::read_dir(files_root).unwrap_or_else(|e| fail(e.to_string())).flatten() {
        let name = entry.file_name();
        if name == ".git" || name.to_string_lossy() == spec.as_str() {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), files.join(&name))
            .unwrap_or_else(|e| fail(e.to_string()));
    }
    let spec_tree = rebased_tree
        .get_name(spec)
        .and_then(|entry| repo.find_tree(entry.id()).ok())
        .unwrap_or_else(|| fail(format!("{} isn't in {}", spec, args[5])));
    export(&repo, &spec_tree, &files.join(spec)).unwrap_or_else(|e| fail(e));

    let mut command = Command::new("spec-analyze");
    command.arg("--only-specs").arg(names.join(","));
    command
        .arg("--terms")
        .arg(index_root.join("spec-terms.json"));
    let bcd = index_root.join("bcd/data.json");
    if bcd.exists() {
        command.arg("--bcd").arg(&bcd);
    }
    let status = command
        .arg("--rendered-specs")
        .arg(index_root.join("try-rendered").join(rebased.id().to_string()))
        .arg(&files)
        .arg(&analysis)
        .arg(specs_toml)
        .status()
        .unwrap_or_else(|e| fail(format!("Couldn't run spec-analyze: {}", e)));
    if !status.success() {
        fail("spec-analyze failed".to_string());
    }
    let _ = fs::remove_dir_all(&files);
    println!("analysis: {}", analysis.display());
}
