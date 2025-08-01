// This file is the historical line-centric blame implementation and has been
// forked to:
// - build-syntax-token-tree.rs - This converts the source tree into the
//   tree-sitter alternate representation of the tree state at any given point
//   in time.  This is not a blame tree.
// - build-timeline-tree.rs - This converts the syntax token tree into the
//   token-centric blame tree and other derived history data.

extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate num_cpus;
extern crate tools;

use std::borrow::{Borrow, Cow};
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use git2::{DiffFindOptions, ObjectType, Oid, Patch, Repository, Sort};
use tools::blame::LineData;
use tools::file_format::config::index_blame;

fn get_hg_rev(helper: &mut Child, git_oid: &Oid) -> Option<String> {
    writeln!(helper.stdin.as_mut().unwrap(), "{}", git_oid).unwrap();
    let mut reader = BufReader::new(helper.stdout.as_mut().unwrap());
    let mut result = String::new();
    reader.read_line(&mut result).unwrap();
    let hgrev = result.trim();
    if hgrev.chars().all(|c| c == '0') {
        return None;
    }
    Some(hgrev.to_string())
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

/// Starts the git-fast-import subcommand, to which data
/// is fed for adding to the blame repo. Refer to
/// https://git-scm.com/docs/git-fast-import for detailed
/// documentation on git-fast-import.
fn start_fast_import(git_repo: &Repository, marks_file: &str) -> Child {
    // Note that we use the `--force` flag here, because there
    // are cases where the blame repo branch we're building was
    // initialized from some other branch (e.g. gecko-dev beta
    // being initialized from gecko-dev master) just to take
    // advantage of work already done (the commits shared between
    // beta and master). After writing the new blame information
    // (for beta) the new branch head (beta) is not going to be a
    // a descendant of the original (master), and we need `--force`
    // to make git-fast-import allow that.
    Command::new("git")
        .arg("fast-import")
        .arg("--force")
        .arg("--quiet")
        .arg(format!("--import-marks-if-exists={}", marks_file))
        .arg(format!("--export-marks={}", marks_file))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(git_repo.path())
        .spawn()
        .unwrap()
}

/// When writing to a git-fast-import stream, we can insert temporary
/// names (called "marks") for commits as we create them. This allows
/// us to refer to them later in the stream without knowing the final
/// oid for that commit. This enum abstracts over that, so bits of code
/// can refer to a specific commit that is either pre-existing in the
/// blame repo (and for which we have an oid) or that was written
/// earlier in the stream (and has a mark).
#[derive(Clone, Copy, Debug)]
enum BlameRepoCommit {
    Commit(git2::Oid),
    Mark(usize),
}

/// Helper to parse the contents of a marks file written by git-fast-import's
/// `--export-marks=` argument.
///
/// This is used by our blame/history preprocessing logic in order to be able to
///  handle restarting processing when a (large) merge wasn't fully processing
/// in a single run.  The problem that can happen in those cases is that
/// `index_blame`/similar methods only revwalk a single branch of our
/// transformed blame and until the merge commit is ingested, we will lose track
/// of the first parent of the merge if we've begun processing the second parent
/// of the merge (modulo any commits they may share).
///
/// As the most dramatic example, our revwalk of the blink repository's history
/// up until the point of the merge of the "chromium" and "link trees" looks
/// like: [(289k commits of chromium), (182k commits of blink), (merge commit)].
/// As long as our import process doesn't fail, we're fine, but if we crash or
/// stop because of our "limit" mechanism once we've moved onto the blink
/// commits, then they won't be referenced by the single branch tag we're using
/// in our processing.
///
/// To this end, we load up the marks file to augment the information from
/// index_blame and make sure to pass `--import-marks-if-exists=` when we kick
/// off git-fast-import.  Note that we currently don't actually reference the
/// marker identifiers we get from the file since we do have the hashes
/// available and it makes sense to just use that, so the main reason we want
/// to tell git-fast-import to import the marks file is so that the contents of
/// the file are maintained through successive runs rather than only containing
/// revisions processed in the last run.
///
/// Note that another alternative would have been some kind of complex branch
/// management.  See bug 1782285 for more context.
fn ingest_git_fast_import_marks_file(
    blame_repo: &Repository,
    blame_map: &mut HashMap<Oid, BlameRepoCommit>,
    marks_file: &str,
) -> (u32, u32) {
    // The number of revisions the marker file told us about.
    let mut marker_revs_exist: u32 = 0;
    // The number of marker revisions we used to fill in the blame_map.  If this
    // is zero it means that `index_blame`
    let mut marker_revs_needed: u32 = 0;

    for line in marks_file.lines() {
        let line_pieces = line.split_whitespace().collect::<Vec<_>>();
        let marker_oid = Oid::from_str(line_pieces[1]).unwrap();

        marker_revs_exist += 1;

        let commit = blame_repo.find_commit(marker_oid).unwrap();

        let msg = commit.message().unwrap();
        // (This must match what's in `index_blame`.)
        let pieces = msg.split_whitespace().collect::<Vec<_>>();

        let orig_oid = Oid::from_str(pieces[1]).unwrap();
        blame_map.entry(orig_oid).or_insert_with(|| {
            marker_revs_needed += 1;
            BlameRepoCommit::Commit(marker_oid)
        });

        // (we don't care about pieces[3] which is the hg commit if it's there)
    }

    (marker_revs_exist, marker_revs_needed)
}

impl fmt::Display for BlameRepoCommit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Commit(oid) => write!(f, "{}", oid),
            // Mark-type commit references take the form :<idnum>
            Self::Mark(id) => write!(f, ":{}", id),
        }
    }
}

/// Read the oid of the object at the given path in the given
/// commit. Returns None if there is no such object.
/// Documentation for the fast-import command used is at
/// https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Readingfromanamedtree
fn read_path_oid(
    import_helper: &mut Child,
    commit: &BlameRepoCommit,
    path: &Path,
) -> Option<String> {
    writeln!(
        import_helper.stdin.as_mut().unwrap(),
        "ls {} {}",
        commit,
        sanitize(path)
    )
    .unwrap();
    let mut reader = BufReader::new(import_helper.stdout.as_mut().unwrap());
    let mut result = String::new();
    reader.read_line(&mut result).unwrap();
    // result will be of format
    //   <mode> SP ('blob' | 'tree' | 'commit') SP <dataref> HT <path> LF
    // where SP is a single space, HT is a tab character, and LF is the end of line.
    // We just want to extract the <dataref> piece which is the git oid of the
    // object we care about.
    // If the path doesn't exist, the response will instead be
    //   'missing' SP <path> LF
    // and in that case we return None
    let mut tokens = result.split_ascii_whitespace();
    if tokens.next()? == "missing" {
        return None;
    }
    tokens.nth(1).map(str::to_string)
}

/// Return the contents of the object at the given path in the
/// given commit. Returns None if there is no such object.
/// Documentation for the fast-import command used is at
/// https://git-scm.com/docs/git-fast-import#_cat_blob
fn read_path_blob(
    import_helper: &mut Child,
    commit: &BlameRepoCommit,
    path: &Path,
) -> Option<Vec<u8>> {
    let oid = read_path_oid(import_helper, commit, path)?;
    writeln!(import_helper.stdin.as_mut().unwrap(), "cat-blob {}", oid).unwrap();
    let mut reader = BufReader::new(import_helper.stdout.as_mut().unwrap());
    let mut description = String::new();
    reader.read_line(&mut description).unwrap();
    // description will be of the format:
    //   <sha1> SP 'blob' SP <size> LF
    let size: usize = description
        .split_ascii_whitespace()
        .nth(2)
        .unwrap()
        .parse()
        .unwrap();
    // The stream will now have <size> bytes of content followed
    // by a LF character that we want to discard. So we read size+1
    // bytes and then trim off the LF
    let mut blob = Vec::with_capacity(size + 1);
    reader
        .take((size + 1) as u64)
        .read_to_end(&mut blob)
        .unwrap();
    blob.truncate(size);
    Some(blob)
}

/// Sanitizes a path into a format that git-fast-import wants.
fn sanitize(path: &Path) -> std::borrow::Cow<str> {
    // Technically, I'm not sure what git-fast-import expects to happen with
    // non-unicode sequences in the path; the documentation is a bit unclear.
    // But in practice that hasn't come up yet.
    let mut result = path.to_string_lossy();
    if result.starts_with('"') || result.contains('\n') {
        // From git-fast-import documentation:
        // A path can use C-style string quoting; this is accepted
        // in all cases and mandatory if the filename starts with
        // double quote or contains LF. In C-style quoting, the complete
        // name should be surrounded with double quotes, and any LF,
        // backslash, or double quote characters must be escaped by
        // preceding them with a backslash.
        let escaped = result
            .replace("\\", "\\\\")
            .replace("\n", "\\\n")
            .replace("\"", "\\\"");
        result = std::borrow::Cow::Owned(format!(r#""{}""#, escaped));
    }
    result
}

#[test]
fn test_sanitize() {
    let p1 = PathBuf::from("first/second/third");
    assert_eq!(sanitize(&p1), "first/second/third");
    let p2 = PathBuf::from(r#""starts/with/quote"#);
    assert_eq!(sanitize(&p2), r#""\"starts/with/quote""#);
    let p3 = PathBuf::from(r#"internal/quote/"/is/ok"#);
    assert_eq!(sanitize(&p3), r#"internal/quote/"/is/ok"#);
    let p4 = PathBuf::from("internal/lf/\n/needs/escaping");
    assert_eq!(sanitize(&p4), "\"internal/lf/\\\n/needs/escaping\"");
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
    commit: &git2::Commit,
    blob: &git2::Blob,
    import_helper: &mut Child,
    blame_parents: &[BlameRepoCommit],
    path: &Path,
) -> Result<String, git2::Error> {
    let linecount = count_lines(blob);
    let mut line_data = LineData {
        rev: Cow::Owned(commit.id().to_string()),
        path: LineData::path_unchanged(),
        lineno: Cow::Owned(String::new()),
    };
    let mut blame = Vec::with_capacity(linecount);
    for line in 1..=linecount {
        line_data.lineno = Cow::Owned(line.to_string());
        blame.push(line_data.serialize());
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
        let parent_blame_blob = match read_path_blob(import_helper, blame_parent, parent_path) {
            Some(blob) => blob,
            _ => continue,
        };
        let parent_blame = std::str::from_utf8(&parent_blame_blob)
            .unwrap() // We only ever put ascii in the blame blob (for now)
            .lines()
            .collect::<Vec<&str>>();

        let path_unchanged = path == parent_path;
        for (lineno, parent_lineno) in unmodified_lines {
            if path_unchanged {
                blame[*lineno] = String::from(parent_blame[*parent_lineno]);
                continue;
            }
            let mut line_data = LineData::deserialize(parent_blame[*parent_lineno]);
            if line_data.is_path_unchanged() {
                line_data.path = Cow::Borrowed(parent_path.to_str().unwrap());
            }
            blame[*lineno] = line_data.serialize();
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

// To deal with a bad "blink" tree state we need the hard-coded empty blob hash
// of e69de29bb2d1d6434b8b29ae775ad8c2e48c5391.  We could also use a
// lazy_static and Oid::from_str here.
static EMPTY_SHA1_BLOB_OID: [u8; 20] = [
    0xe6, 0x9d, 0xe2, 0x9b, 0xb2, 0xd1, 0xd6, 0x43, 0x4b, 0x8b, 0x29, 0xae, 0x77, 0x5a, 0xd8, 0xc2,
    0xe4, 0x8c, 0x53, 0x91,
];

#[allow(clippy::too_many_arguments)]
fn build_blame_tree(
    diff_data: &DiffData,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    tree_at_path: &git2::Tree,
    parent_trees: &[Option<git2::Tree>],
    import_helper: &mut Child,
    blame_parents: &[BlameRepoCommit],
    mut path: PathBuf,
) -> Result<(), git2::Error> {
    'outer: for entry in tree_at_path.iter() {
        let entry_name = entry.name().unwrap();
        path.push(entry_name);
        for (i, parent_tree) in parent_trees.iter().enumerate() {
            let parent_tree = match parent_tree {
                None => continue, // This parent doesn't even have a tree at this path
                Some(p) => p,
            };
            if let Some(parent_entry) = parent_tree.get_name(entry_name) {
                if parent_entry.id() == entry.id() {
                    // Item at `path` is the same in the tree for `commit` as in
                    // `parent_trees[i]`, so the blame should be the same too.
                    //
                    // The exception to this situation is that empty sub-trees
                    let oid = match read_path_oid(import_helper, &blame_parents[i], &path) {
                        Some(oid) => oid,
                        None => match entry.kind() {
                            Some(ObjectType::Tree) => {
                                info!("Probably found an empty tree hierarchy: Unable to find blame for {:?} oid {} on parent src oid {} for parent blame oid {:?}", path, entry.id(), parent_tree.id(), blame_parents[i]);
                                path.pop();
                                continue 'outer;
                            }
                            Some(ObjectType::Commit) => {
                                if entry.id().as_bytes() == EMPTY_SHA1_BLOB_OID {
                                    info!("Missing commit (submodule) blame for empty blob OID for {:?} oid {}, this is expected.", path, entry.id());
                                } else {
                                    panic!("Missing commit (submodule) blame, unexpected: Unable to find blame for {:?} oid {} on parent src oid {} for parent blame oid {:?}", path, entry.id(), parent_tree.id(), blame_parents[i]);
                                }
                                path.pop();
                                continue 'outer;
                            }
                            _ => {
                                panic!("Unable to find blame for {:?} oid {} on parent src oid {} for parent blame oid {:?}", path, entry.id(), parent_tree.id(), blame_parents[i]);
                            }
                        },
                    };
                    writeln!(
                        import_helper.stdin.as_mut().unwrap(),
                        "M {:06o} {} {}",
                        entry.filemode(),
                        oid,
                        sanitize(&path)
                    )
                    .unwrap();
                    path.pop();
                    continue 'outer;
                }
            }
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                let blame_text = blame_for_path(
                    diff_data,
                    commit,
                    &entry.to_object(git_repo)?.peel_to_blob()?,
                    import_helper,
                    blame_parents,
                    &path,
                )?;
                // For the inline data format documentation, refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Inlinedataformat
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Exactbytecountformat
                let blame_bytes = blame_text.as_bytes();
                let import_stream = import_helper.stdin.as_mut().unwrap();
                writeln!(
                    import_stream,
                    "M {:06o} inline {}",
                    entry.filemode(),
                    sanitize(&path)
                )
                .unwrap();
                writeln!(import_stream, "data {}", blame_bytes.len()).unwrap();
                import_stream.write_all(blame_bytes).unwrap();
                // We skip the optional trailing LF character here since in practice it
                // wasn't particularly useful for debugging. Also the blame blobs we write
                // here always have a trailing LF anyway.
            }
            Some(ObjectType::Commit) => {
                // This is a submodule.  We pass through the existing OID
                // because we know it's a valid OBJ_COMMIT.  See
                // https://bugzilla.mozilla.org/show_bug.cgi?id=1782285#c5 for
                // more context about this rationale.
                //
                // For the external ref data format documentation, refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Externaldataformat
                assert_eq!(entry.filemode(), 0o160000);
                // Unfortunately the "blink" tree commit c1201a71c07d1e3268ebbfe5380aa6c5832c6edb
                // has a bad git tree where the "clank" submodule commit uses the empty
                // blob OID and git-fast-import will get upset since that exists in
                // the blink tree.  So whenever we see an empty blob OID, we just
                // omit the entry in that case.
                if entry.id().as_bytes() != EMPTY_SHA1_BLOB_OID {
                    writeln!(
                        import_helper.stdin.as_mut().unwrap(),
                        "M {:06o} {} {}",
                        entry.filemode(),
                        entry.id(),
                        sanitize(&path)
                    )
                    .unwrap();
                }
            }
            Some(ObjectType::Tree) => {
                let mut parent_subtrees = Vec::with_capacity(parent_trees.len());
                // Note that we require the elements in parent_trees to
                // correspond to elements in blame_parents, so we need to keep
                // the None elements in the vec rather than discarding them.
                for parent_tree in parent_trees {
                    let parent_subtree = match parent_tree {
                        None => None,
                        Some(tree) => tree
                            .get_name(entry_name)
                            // In the case where a git submodule has been removed
                            // and replaced by a regular file/directory in the
                            // same commit, we expect to_object to fail, and in
                            // that case we just want to treat it as None, so
                            // we use ok() instead of unwrap() which we
                            // previously used.
                            .and_then(|e| e.to_object(git_repo).ok())
                            .and_then(|o| o.into_tree().ok()),
                    };
                    parent_subtrees.push(parent_subtree);
                }
                build_blame_tree(
                    diff_data,
                    git_repo,
                    commit,
                    &entry.to_object(git_repo)?.peel_to_tree()?,
                    &parent_subtrees,
                    import_helper,
                    blame_parents,
                    path.clone(),
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
            DiffFindOptions::new()
                .copies(true)
                .copy_threshold(30)
                .renames(true)
                .rename_threshold(30)
                .rename_limit(1000000)
                .break_rewrites(true)
                .break_rewrites_for_renames_only(true),
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
            // When processing the blink repo we experienced a failure and it
            // turns out that normal handling only gets us the poison panic on
            // the channel, so we make sure to handle the panic here and dump it
            // unconditionally.  We then let the panic propagate like normal so
            // that the app crashes rather than hanging.
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                compute_thread_main(query_rx, response_tx, git_repo_path);
            }));
            if let Err(err) = result {
                eprintln!("ComputeThread panicked: {:?}", err);
                let bt = std::backtrace::Backtrace::force_capture();
                eprintln!("{bt}");
                panic::resume_unwind(err);
            }
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

    // force backtraces on in this file in general
    std::env::set_var("RUST_BACKTRACE", "full");

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
    let marks_file = env::var("MARKS_FILE")
        .ok()
        .unwrap_or("/tmp/mozsearch-fast-import.marks".to_string());
    let blame_ref = env::var("BLAME_REF").ok().unwrap_or("HEAD".to_string());
    let commit_limit = env::var("COMMIT_LIMIT")
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);

    info!("Reading existing blame map of ref {}...", blame_ref);
    let mut blame_map = if let Ok(oid) = blame_repo.refname_to_id(&blame_ref) {
        let (blame_map, _) = index_blame(&blame_repo, Some(oid));
        blame_map
            .into_iter()
            .map(|(k, v)| (k, BlameRepoCommit::Commit(v)))
            .collect::<HashMap<git2::Oid, BlameRepoCommit>>()
    } else {
        HashMap::new()
    };

    let mut prev_revs_done: usize = 0;
    if let Ok(contents) = std::fs::read_to_string(marks_file.clone()) {
        let (revs_found, revs_needed) =
            ingest_git_fast_import_marks_file(&blame_repo, &mut blame_map, &contents);
        prev_revs_done = revs_found as usize;
        info!("Marks file at {} (MARKS_FILE env overrides) had {} revs, {} were needed to augment branch data.",
              marks_file, revs_found, revs_needed);
    } else {
        info!(
            "No pre-existing marks file at {} (MARKS_FILE env overrides).",
            marks_file
        );
        if commit_limit > 0 {
            info!("The marks file will be retained because you specified a LIMIT.");
        } else {
            info!("The marks file will be deleted on success but left around on error.");
        }
    };

    let mut walk = git_repo.revwalk().unwrap();
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    walk.push(git_repo.refname_to_id(&blame_ref).unwrap())
        .unwrap();
    info!("Existing blame map has {} commits.", blame_map.len());
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

    let mut import_helper = start_fast_import(&blame_repo, &marks_file);

    // Tracks completion count and serves as the basis for the mark <idnum>
    // assigned to each commit.
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
            Some(ref mut helper) => get_hg_rev(helper, git_oid),
            None => None, // we don't support mapfiles any more.
        };

        info!(
            "Transforming {} (hg {:?}) progress {}/{}",
            git_oid, hg_rev, rev_done, rev_count
        );
        let commit = git_repo.find_commit(*git_oid).unwrap();
        let parent_trees = commit
            .parents()
            .map(|parent_commit| Some(parent_commit.tree().unwrap()))
            .collect::<Vec<_>>();
        let blame_parents = commit
            .parent_ids()
            .map(|pid| blame_map[&pid])
            .collect::<Vec<_>>();

        // Scope the import_helper borrow
        {
            // Here we write out the metadata for a new commit to the blame repo.
            // For details on the data format, refer to the documentation at
            // https://git-scm.com/docs/git-fast-import#_commit
            // https://git-scm.com/docs/git-fast-import#_mark
            let mut import_stream = BufWriter::new(import_helper.stdin.as_mut().unwrap());
            writeln!(import_stream, "commit {}", blame_ref).unwrap();
            // Because we maintain the markers through multiple consecutive runs,
            // we need to add the count of imported marks so we don't clobber any
            // existing ones.
            let mark_id = rev_done + prev_revs_done;
            writeln!(import_stream, "mark :{}", mark_id).unwrap();
            blame_map.insert(*git_oid, BlameRepoCommit::Mark(mark_id));

            let mut write_role = |role: &str, sig: &git2::Signature| {
                write!(import_stream, "{} ", role).unwrap();
                import_stream.write_all(sig.name_bytes()).unwrap();
                write!(import_stream, " <").unwrap();
                import_stream.write_all(sig.email_bytes()).unwrap();
                write!(import_stream, "> ").unwrap();
                // git-fast-import can take a few different date formats, but the
                // default "raw" format is the easiest for us to write. Refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-coderawcode
                let when = sig.when();
                writeln!(
                    import_stream,
                    "{} {}{:02}{:02}",
                    when.seconds(),
                    when.sign(),
                    when.offset_minutes().abs() / 60,
                    when.offset_minutes().abs() % 60,
                )
                .unwrap();
            };
            write_role("author", &commit.author());
            write_role("committer", &commit.committer());

            let commit_msg = if let Some(hg_rev) = hg_rev {
                format!("git {}\nhg {}\n", git_oid, hg_rev)
            } else {
                format!("git {}\n", git_oid)
            };

            write!(import_stream, "data {}\n{}\n", commit_msg.len(), commit_msg).unwrap();
            if let Some(first_parent) = blame_parents.first() {
                writeln!(import_stream, "from {}", first_parent).unwrap();
            } else {
                // This is a new root commit, so we need to use a special null
                // parent commit identifier for git-fast-import to know that.
                writeln!(
                    import_stream,
                    "from 0000000000000000000000000000000000000000"
                )
                .unwrap();
            }
            for additional_parent in blame_parents.iter().skip(1) {
                writeln!(import_stream, "merge {}", additional_parent).unwrap();
            }
            // For each commit, we start with a clean slate (all files deleted), and then
            // the build_blame_tree call below will add new files or link pre-existing
            // unmodified files/folders from older commits into the new commit's tree.
            // This is the recommended approach by the git-fast-import documentation at
            // https://git-scm.com/docs/git-fast-import#_filedeleteall and works
            // well for us, particularly in the case of merge commits where we might
            // need to pull some entries from one parent and other entries from the other
            // parent.
            writeln!(import_stream, "deleteall").unwrap();
            import_stream.flush().unwrap();
        }

        build_blame_tree(
            &diff_data,
            &git_repo,
            &commit,
            &commit.tree().unwrap(),
            &parent_trees,
            &mut import_helper,
            &blame_parents,
            PathBuf::new(),
        )
        .unwrap();

        if rev_done % 100000 == 0 {
            info!("Completed 100,000 commits, issuing checkpoint...");
            writeln!(import_helper.stdin.as_mut().unwrap(), "checkpoint").unwrap();
        }
    }

    if let Some(mut helper) = hg_helper {
        helper.kill().unwrap();
    }

    info!("Shutting down fast-import...");
    let exitcode = import_helper.wait().unwrap();
    if exitcode.success() {
        info!("Done!");
    } else {
        info!("Fast-import exited with {:?}", exitcode.code());
    }

    // The marks file is intended to be retained only in cases where the env var
    // LIMIT is specified and we need it for subsequent runs or we crash and
    // it's useful for investigation and then being able to restart without
    // data-loss.  Also, if we leave it around when it's not expected, it will
    // cause problems for other trees being indexed on the same machine.
    if commit_limit == 0 {
        info!(
            "Removing marks file {} after successful run because no limit was specified.",
            marks_file
        );
        std::fs::remove_file(marks_file).unwrap();
    }
}
