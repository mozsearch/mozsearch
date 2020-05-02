extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate tools;
extern crate unicode_normalization;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use git2::{DiffFindOptions, ObjectType, Oid, Patch, Repository, Sort};
use tools::config::index_blame;
use unicode_normalization::UnicodeNormalization;

fn get_hg_rev(helper: &mut Child, git_oid: &Oid) -> Option<String> {
    write!(helper.stdin.as_mut().unwrap(), "{}\n", git_oid).unwrap();
    let mut reader = BufReader::new(helper.stdout.as_mut().unwrap());
    let mut result = String::new();
    reader.read_line(&mut result).unwrap();
    let hgrev = result.trim();
    if hgrev.chars().all(|c| c == '0') {
        return None;
    }
    return Some(hgrev.to_string());
}

fn start_cinnabar_helper(git_repo: &Repository) -> Child {
    Command::new("git")
        .arg("cinnabar")
        .arg("git2hg")
        .arg("--batch")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(git_repo.path())
        .spawn()
        .unwrap()
}

fn count_lines(blob: &git2::Blob) -> usize {
    let data = blob.content();
    if data.is_empty() {
        return 0;
    }
    let mut linecount = 0;
    for b in data {
        if *b == b'\n' {
            linecount += 1;
        }
    }
    if data[data.len() - 1] != b'\n' {
        linecount += 1;
    }
    linecount
}

fn unmodified_lines(
    blob: &git2::Blob,
    parent_blob: &git2::Blob,
) -> Result<Vec<(usize, usize)>, git2::Error> {
    let mut unchanged = Vec::new();

    let patch = Patch::from_blobs(parent_blob, None, blob, None, None)?;

    if patch.delta().flags().is_binary() {
        return Ok(unchanged);
    }

    fn add_delta(lineno: usize, delta: i32) -> usize {
        ((lineno as i32) + delta) as usize
    }

    let mut latest_line: usize = 0;
    let mut delta: i32 = 0;

    for hunk_index in 0..patch.num_hunks() {
        for line_index in 0..patch.num_lines_in_hunk(hunk_index)? {
            let line = patch.line_in_hunk(hunk_index, line_index)?;

            if let Some(lineno) = line.new_lineno() {
                let lineno = lineno as usize;
                for i in latest_line..lineno - 1 {
                    unchanged.push((i, add_delta(i, delta)));
                }
                latest_line = (lineno - 1) + 1;
            }

            match line.origin() {
                '+' => delta -= 1,
                '-' => delta += 1,
                ' ' => {
                    assert_eq!(
                        line.old_lineno().unwrap() as usize,
                        add_delta(line.new_lineno().unwrap() as usize, delta)
                    );
                    unchanged.push((
                        (line.new_lineno().unwrap() - 1) as usize,
                        (line.old_lineno().unwrap() - 1) as usize,
                    ));
                }
                _ => (),
            };
        }
    }

    let linecount = count_lines(blob);
    for i in latest_line..linecount {
        unchanged.push((i, add_delta(i, delta)));
    }
    Ok(unchanged)
}

fn blame_for_path(
    file_movement: &HashMap<Oid, HashMap<Oid, PathBuf>>,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    blame_repo: &git2::Repository,
    blame_parents: &[git2::Commit],
    path: &Path,
) -> Result<String, git2::Error> {
    let blob = commit
        .tree()?
        .get_path(path)?
        .to_object(git_repo)?
        .peel_to_blob()?;
    let linecount = count_lines(&blob);
    // TODO: drop this author field entirely, I don't think it gets consumed by anything
    let commit_id = commit.id();
    let author = commit
        .author()
        .name()
        .map(|n| n.nfkd().filter(|c| c.is_ascii()).collect::<String>())
        .unwrap_or_default();
    let mut blame = Vec::with_capacity(linecount);
    for line in 1..=linecount {
        blame.push(format!("{}:%:{}:{}", commit_id, line, author));
    }

    for (parent, blame_parent) in commit.parents().zip(blame_parents.iter()).rev() {
        let parent_path = file_movement
            .get(&parent.id())
            .and_then(|m| m.get(&blob.id()))
            .map(|p| p.borrow())
            .unwrap_or(path);
        let parent_blob = match parent.tree()?.get_path(parent_path) {
            Ok(t) if t.kind() == Some(ObjectType::Blob) => t.to_object(git_repo)?.peel_to_blob()?,
            _ => {
                continue;
            }
        };
        let parent_blame_blob = match blame_parent.tree()?.get_path(parent_path) {
            Ok(entry) => entry.to_object(blame_repo)?.peel_to_blob()?,
            _ => continue,
        };
        let parent_blame = std::str::from_utf8(parent_blame_blob.content())
            .unwrap() // We only ever put ascii in the blame blob (for now)
            .lines()
            .collect::<Vec<&str>>();

        let path_unchanged = path == parent_path;
        for (lineno, parent_lineno) in unmodified_lines(&blob, &parent_blob)? {
            if path_unchanged {
                blame[lineno] = String::from(parent_blame[parent_lineno]);
                continue;
            }
            let mut pieces = parent_blame[parent_lineno].splitn(4, ':');
            let p_rev = pieces.next().unwrap();
            let mut p_fname = pieces.next().unwrap();
            let p_lineno = pieces.next().unwrap();
            let p_author = pieces.next().unwrap();
            if p_fname == "%" {
                p_fname = parent_path.to_str().unwrap();
            }
            blame[lineno] = format!("{}:{}:{}:{}", p_rev, p_fname, p_lineno, p_author);
        }
    }
    // Extra entry so the `join` call after adds a trailing newline
    blame.push(String::new());
    Ok(blame.join("\n"))
}

fn build_blame_tree(
    builder: &mut git2::TreeBuilder,
    file_movement: &HashMap<Oid, HashMap<Oid, PathBuf>>,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    blame_repo: &git2::Repository,
    blame_parents: &[git2::Commit],
    mut path: PathBuf,
) -> Result<(), git2::Error> {
    let tree_at_path = if path == PathBuf::new() {
        commit.tree()?
    } else {
        commit
            .tree()?
            .get_path(&path)?
            .to_object(git_repo)?
            .peel_to_tree()?
    };
    'outer: for entry in tree_at_path.iter() {
        path.push(entry.name().unwrap());
        for (i, parent) in commit.parents().enumerate() {
            if let Ok(parent_entry) = parent.tree()?.get_path(&path) {
                if parent_entry.id() == entry.id() {
                    // Item at `path` is the same in the tree for `commit` as in the tree
                    // for `parent`, so the blame must be the same too
                    let blame_parent_entry = blame_parents[i].tree()?.get_path(&path)?;
                    builder.insert(
                        entry.name().unwrap(),
                        blame_parent_entry.id(),
                        entry.filemode(),
                    )?;
                    path.pop();
                    continue 'outer;
                }
            }
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                let blame_text = blame_for_path(
                    file_movement,
                    git_repo,
                    commit,
                    blame_repo,
                    blame_parents,
                    &path,
                )?;
                builder.insert(
                    entry.name().unwrap(),
                    blame_repo.blob(&blame_text.as_bytes())?,
                    entry.filemode(),
                )?;
            }
            Some(ObjectType::Commit) => {
                // This is a submodule, just treat it as an empty dir. We could
                // probably also skip over it entirely.
                let entry_builder = blame_repo.treebuilder(None)?;
                builder.insert(
                    entry.name().unwrap(),
                    entry_builder.write()?,
                    entry.filemode(),
                )?;
            }
            Some(ObjectType::Tree) => {
                let mut entry_builder = blame_repo.treebuilder(None)?;
                build_blame_tree(
                    &mut entry_builder,
                    file_movement,
                    git_repo,
                    commit,
                    blame_repo,
                    blame_parents,
                    path.clone(),
                )?;
                builder.insert(
                    entry.name().unwrap(),
                    entry_builder.write()?,
                    entry.filemode(),
                )?;
            }
            _ => {
                panic!(
                    "Unexpected entry kind {:?} found in tree for commit {:?} at path {:?}",
                    entry.kind(),
                    commit.id(),
                    path
                );
            }
        };

        path.pop();
    }

    Ok(())
}

fn main() {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<_> = env::args().collect();
    let git_repo = Repository::open(&args[1]).unwrap();
    let blame_repo = Repository::open(&args[2]).unwrap();
    let use_cinnabar = env::var("CINNABAR").map_or(true, |v| v != "0");
    let mut hg_helper = if use_cinnabar {
        Some(start_cinnabar_helper(&git_repo))
    } else {
        None
    };
    let blame_ref = env::var("BLAME_REF").ok().unwrap_or("HEAD".to_string());

    info!("Reading existing blame map of ref {}...", blame_ref);
    let mut blame_map = if let Ok(oid) = blame_repo.refname_to_id(&blame_ref) {
        let (blame_map, _) = index_blame(&blame_repo, Some(oid));
        blame_map
    } else {
        HashMap::new()
    };

    let mut refobj = blame_repo
        .find_reference(&blame_ref)
        .ok()
        .and_then(|r| r.resolve().ok());

    let mut walk = git_repo.revwalk().unwrap();
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    walk.push(git_repo.refname_to_id(&blame_ref).unwrap())
        .unwrap();
    let revs_to_process = walk
        .map(|r| r.unwrap()) // walk produces Result<git2::Oid> so we unwrap to just the Oid
        .filter(|git_oid| !blame_map.contains_key(git_oid))
        .collect::<Vec<_>>();
    let rev_count = revs_to_process.len();

    info!("Transforming new commits...");

    let mut rev_done = 0;
    for git_oid in revs_to_process {
        rev_done += 1;

        let hg_rev = match hg_helper {
            Some(ref mut helper) => get_hg_rev(helper, &git_oid),
            None => None, // we don't support mapfiles any more.
        };

        info!(
            "Transforming {} (hg {:?}) progress {}/{}",
            git_oid, hg_rev, rev_done, rev_count
        );
        let commit = git_repo.find_commit(git_oid).unwrap();
        let blame_parents = commit
            .parent_ids()
            .map(|pid| blame_repo.find_commit(blame_map[&pid]).unwrap())
            .collect::<Vec<_>>();

        let mut file_movement = HashMap::new();
        if commit.parent_count() == 1 {
            let mut movement = HashMap::new();
            let mut diff = git_repo
                .diff_tree_to_tree(
                    Some(&commit.parent(0).unwrap().tree().unwrap()),
                    Some(&commit.tree().unwrap()),
                    None,
                )
                .unwrap();
            diff.find_similar(Some(
                DiffFindOptions::new().copies(true).rename_limit(1000000),
            ))
            .unwrap();
            for delta in diff.deltas() {
                if !delta.old_file().id().is_zero()
                    && !delta.new_file().id().is_zero()
                    && delta.old_file().path() != delta.new_file().path()
                {
                    movement.insert(
                        delta.new_file().id(),
                        delta.old_file().path().unwrap().to_path_buf(),
                    );
                }
            }
            file_movement.insert(commit.parent_id(0).unwrap(), movement);
        }

        let mut builder = blame_repo.treebuilder(None).unwrap();
        build_blame_tree(
            &mut builder,
            &file_movement,
            &git_repo,
            &commit,
            &blame_repo,
            &blame_parents,
            PathBuf::new(),
        )
        .unwrap();
        let tree_oid = builder.write().unwrap();

        let commit_msg = if let Some(hg_rev) = hg_rev {
            format!("git {}\nhg {}\n", git_oid, hg_rev)
        } else {
            format!("git {}\n", git_oid)
        };

        let commit_ref = if refobj.is_some() {
            None
        } else {
            // This should only happen on the first commit, if the blame_ref
            // doesn't exist yet in the destination repo
            Some(blame_ref.as_str())
        };
        let blame_oid = blame_repo
            .commit(
                commit_ref,
                &commit.author(),
                &commit.committer(),
                &commit_msg,
                &blame_repo.find_tree(tree_oid).unwrap(),
                &blame_parents.iter().collect::<Vec<_>>(),
            )
            .unwrap();

        if let Some(ref mut refobj) = refobj {
            *refobj = refobj.set_target(blame_oid, "").unwrap();
        } else {
            refobj = blame_repo
                .find_reference(&blame_ref)
                .ok()
                .and_then(|r| r.resolve().ok());
            assert!(refobj.is_some());
        }

        blame_map.insert(git_oid, blame_oid);
        info!("  -> {}", blame_oid);
    }

    if let Some(mut helper) = hg_helper {
        helper.kill().unwrap();
    }
}
