extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate num_cpus;
extern crate tools;
extern crate unicode_normalization;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

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
    diff_data: &DiffData,
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
        let parent_path = diff_data
            .file_movement
            .as_ref()
            .and_then(|m| m.get(&blob.id()))
            .map(|p| p.borrow())
            .unwrap_or(path);
        let unmodified_lines = match diff_data
            .unmodified_lines
            .get(&(parent.id(), path.to_path_buf()))
        {
            Some(entry) => entry,
            _ => continue,
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
        for (lineno, parent_lineno) in unmodified_lines {
            if path_unchanged {
                blame[*lineno] = String::from(parent_blame[*parent_lineno]);
                continue;
            }
            let mut pieces = parent_blame[*parent_lineno].splitn(4, ':');
            let p_rev = pieces.next().unwrap();
            let mut p_fname = pieces.next().unwrap();
            let p_lineno = pieces.next().unwrap();
            let p_author = pieces.next().unwrap();
            if p_fname == "%" {
                p_fname = parent_path.to_str().unwrap();
            }
            blame[*lineno] = format!("{}:{}:{}:{}", p_rev, p_fname, p_lineno, p_author);
        }
    }
    // Extra entry so the `join` call after adds a trailing newline
    blame.push(String::new());
    Ok(blame.join("\n"))
}

// This recursively walks the tree for the given commit, skipping over unmodified
// entries, exactly like build_blame_tree does. However, instead of building the
// blame tree, this simply computes the unmodified_lines for each blob that was
// modified in `commit`, relative to all the parents. The results are populated
// into the `results` HashMap.
fn find_unmodified_lines(
    file_movement: Option<&HashMap<Oid, PathBuf>>,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    mut path: PathBuf,
    results: &mut HashMap<(git2::Oid, PathBuf), Vec<(usize, usize)>>,
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
        for parent in commit.parents() {
            if let Ok(parent_entry) = parent.tree()?.get_path(&path) {
                if parent_entry.id() == entry.id() {
                    path.pop();
                    continue 'outer;
                }
            }
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                let blob = entry.to_object(git_repo)?.peel_to_blob()?;
                for parent in commit.parents() {
                    let parent_path = file_movement
                        .and_then(|m| m.get(&blob.id()))
                        .map(|p| p.borrow())
                        .unwrap_or(&path);
                    let parent_blob = match parent.tree()?.get_path(parent_path) {
                        Ok(t) if t.kind() == Some(ObjectType::Blob) => {
                            t.to_object(git_repo)?.peel_to_blob()?
                        }
                        _ => continue,
                    };

                    results.insert(
                        (parent.id(), path.clone()),
                        unmodified_lines(&blob, &parent_blob)?,
                    );
                }
            }
            Some(ObjectType::Tree) => {
                find_unmodified_lines(file_movement, git_repo, commit, path.clone(), results)?;
            }
            _ => (),
        };

        path.pop();
    }

    Ok(())
}

fn build_blame_tree(
    builder: &mut git2::TreeBuilder,
    diff_data: &DiffData,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    tree_at_path: &git2::Tree,
    blame_repo: &git2::Repository,
    blame_parents: &[git2::Commit],
    mut path: PathBuf,
) -> Result<(), git2::Error> {
    'outer: for entry in tree_at_path.iter() {
        let entry_name = entry.name().unwrap();
        path.push(entry_name);
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
                    diff_data,
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
                    diff_data,
                    git_repo,
                    commit,
                    &entry.to_object(git_repo)?.peel_to_tree()?,
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

struct DiffData {
    // The commit for which this DiffData holds data.
    revision: git2::Oid,
    // Map from file (blob) id in the child rev to the path that the file was
    // at in the parent revision, for files that got moved. Set to None if the
    // child rev has multiple parents.
    file_movement: Option<HashMap<Oid, PathBuf>>,
    // Map to find unmodified lines for modified files in a revision (files that
    // are not modified don't have entries here). The key is of the map is a
    // tuple containing the parent commit id and path to the file (in the child
    // revision). The parent commit id is needed in the case of merge commits,
    // where a file that is modified may have different sets of unmodified lines
    // with respect to the different parent commits.
    // The value in the map is a vec of line mappings as produced by the
    // `unmodified_lines` function.
    unmodified_lines: HashMap<(git2::Oid, PathBuf), Vec<(usize, usize)>>,
}

// Does the CPU-intensive work required for blame computation of a given revision.
// This does not mutate anything in `git_repo` and has no other dependencies, so
// it can be parallelized.
fn compute_diff_data(
    git_repo: &git2::Repository,
    git_oid: &git2::Oid,
) -> Result<DiffData, git2::Error> {
    let commit = git_repo.find_commit(*git_oid).unwrap();
    let file_movement = if commit.parent_count() == 1 {
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
        Some(movement)
    } else {
        None
    };

    let mut unmodified_lines = HashMap::new();
    find_unmodified_lines(
        file_movement.as_ref(),
        git_repo,
        &commit,
        PathBuf::new(),
        &mut unmodified_lines,
    )?;

    Ok(DiffData {
        revision: *git_oid,
        file_movement,
        unmodified_lines,
    })
}

struct ComputeThread {
    query_tx: Sender<git2::Oid>,
    response_rx: Receiver<DiffData>,
}

impl ComputeThread {
    fn new(git_repo_path: &str) -> Self {
        let (query_tx, query_rx) = channel();
        let (response_tx, response_rx) = channel();
        let git_repo_path = git_repo_path.to_string();
        thread::spawn(move || {
            compute_thread_main(query_rx, response_tx, git_repo_path);
        });

        ComputeThread {
            query_tx,
            response_rx,
        }
    }

    fn compute(&self, rev: &git2::Oid) {
        self.query_tx.send(*rev).unwrap();
    }

    fn read_result(&self) -> DiffData {
        match self.response_rx.try_recv() {
            Ok(result) => result,
            Err(_) => {
                info!("Waiting on compute, work on optimizing that...");
                self.response_rx.recv().unwrap()
            }
        }
    }
}

fn compute_thread_main(
    query_rx: Receiver<git2::Oid>,
    response_tx: Sender<DiffData>,
    git_repo_path: String,
) {
    let git_repo = Repository::open(git_repo_path).unwrap();
    while let Ok(rev) = query_rx.recv() {
        let result = compute_diff_data(&git_repo, &rev).unwrap();
        response_tx.send(result).unwrap();
    }
}

fn main() {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<_> = env::args().collect();
    let git_repo_path = args[1].to_string();
    let git_repo = Repository::open(&git_repo_path).unwrap();
    let blame_repo = Repository::open(&args[2]).unwrap();
    let use_cinnabar = env::var("CINNABAR").map_or(true, |v| v != "0");
    let mut hg_helper = if use_cinnabar {
        Some(start_cinnabar_helper(&git_repo))
    } else {
        None
    };
    let blame_ref = env::var("BLAME_REF").ok().unwrap_or("HEAD".to_string());
    let commit_limit = env::var("COMMIT_LIMIT")
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);

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
    let mut revs_to_process = walk
        .map(|r| r.unwrap()) // walk produces Result<git2::Oid> so we unwrap to just the Oid
        .filter(|git_oid| !blame_map.contains_key(git_oid))
        .collect::<Vec<_>>();
    if commit_limit > 0 && commit_limit < revs_to_process.len() {
        info!(
            "Truncating list of commits from {} to specified limit {}",
            revs_to_process.len(),
            commit_limit
        );
        revs_to_process.truncate(commit_limit);
    }
    let rev_count = revs_to_process.len();

    let num_threads: usize = num_cpus::get() - 1; // 1 for the main thread
    const COMPUTE_BUFFER_SIZE: usize = 10;

    info!("Starting {} compute threads...", num_threads);
    let mut compute_threads = Vec::with_capacity(num_threads);
    for _ in 0..num_threads {
        compute_threads.push(ComputeThread::new(&git_repo_path));
    }

    // This tracks the index of the next revision in revs_to_process for which
    // we want to request a compute. All revs at indices less than this index
    // have already been requested.
    let mut compute_index = 0;

    info!("Filling compute buffer...");
    let initial_request_count = rev_count.min(COMPUTE_BUFFER_SIZE * num_threads);
    while compute_index < initial_request_count {
        let thread = &compute_threads[compute_index % num_threads];
        thread.compute(&revs_to_process[compute_index]);
        compute_index += 1;
    }

    // We should have sent an equal number of requests to each thread, except
    // if we ran out of requests because there were so few.
    assert!((compute_index % num_threads == 0) || compute_index == rev_count);

    // Tracks completion count
    let mut rev_done = 0;

    for git_oid in revs_to_process.iter() {
        // Read a result. Since we hand out compute requests in round-robin order
        // and each thread processes them in FIFO order we know exactly which
        // thread is going to give us our result.
        // We assert to make sure it's the right one.
        let thread = &compute_threads[rev_done % num_threads];
        let diff_data = thread.read_result();
        assert!(diff_data.revision == *git_oid);

        // If there are more revisions that we haven't requested yet, request
        // another one from this thread.
        if compute_index < rev_count {
            thread.compute(&revs_to_process[compute_index]);
            compute_index += 1;
        }

        rev_done += 1;

        let hg_rev = match hg_helper {
            Some(ref mut helper) => get_hg_rev(helper, &git_oid),
            None => None, // we don't support mapfiles any more.
        };

        info!(
            "Transforming {} (hg {:?}) progress {}/{}",
            git_oid, hg_rev, rev_done, rev_count
        );
        let commit = git_repo.find_commit(*git_oid).unwrap();
        let blame_parents = commit
            .parent_ids()
            .map(|pid| blame_repo.find_commit(blame_map[&pid]).unwrap())
            .collect::<Vec<_>>();

        let mut builder = blame_repo.treebuilder(None).unwrap();
        build_blame_tree(
            &mut builder,
            &diff_data,
            &git_repo,
            &commit,
            &commit.tree().unwrap(),
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

        blame_map.insert(*git_oid, blame_oid);
        info!("  -> {}", blame_oid);
    }

    if let Some(mut helper) = hg_helper {
        helper.kill().unwrap();
    }
}
