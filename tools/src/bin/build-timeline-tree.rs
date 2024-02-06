// This binary consumes the "syntax" repo built by `build-syntax-token-tree.rs`
// to produce the "timeline" repo.

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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::from_utf8;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use git2::{Blob, DiffFindOptions, ObjectType, Oid, Patch, Repository, Sort};
use tools::blame::LineData;
use tools::file_format::config::{index_blame, index_timeline_history_by_source_rev, syntax_commit_to_meta, HistorySyntaxCommitMeta};
use tools::tree_sitter_support::cst_tokenizer::namespace_for_file;

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
fn start_fast_import(git_repo: &Repository) -> Child {
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
enum TimelineRepoCommit {
    Commit(git2::Oid),
    Mark(usize),
}

impl fmt::Display for TimelineRepoCommit {
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
    commit: &TimelineRepoCommit,
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
    commit: &TimelineRepoCommit,
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
fn sanitize(path: &Path) -> std::borrow::Cow<'_, str> {
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

/// Given a blob and its parent, derive the diff and process its hunks in order
/// to produce the set of unmodified token (line indices) as well as the
/// removals and additions so we can infer token moves in a subsequent pass once
/// we've run this logic across all patches.
///
/// This method could potentially be naively parallelized.
fn ingest_diff_accumulating_deltas(
    blob: &git2::Blob,
    parent_blob: &git2::Blob,
    path: &Path,
    delter: &mut DeltaMachine,
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

    let namespace = namespace_for_file(path);

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

            if let Some((context, token)) = from_utf8(line.content()).unwrap().split_once(' ') {
                delter.push_diff_token(&line, context, token);
            }

            match line.origin() {
                '+' => {
                    delta -= 1;
                }
                '-' => {
                    delta += 1;
                }
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

        delter.flush_hunk();
    }

    let linecount = count_lines(blob);
    for i in latest_line..linecount {
        unchanged.push((i, add_delta(i, delta)));
    }
    Ok(unchanged)
}

/// Consumes the aggregated output of all `ingest_diff_accumulating_deltas`
/// processing of the patches in order to detect token movement leveraging their
/// semantic binding.
///
/// This method could potentially be parallelized based naively based on a
/// language basis first since it's likely pointless to try and bother deriving
/// the history of a JS implementation of something being written into C++, it
/// just didn't work.  Then within the language we could dynamic heuristics to
/// partition.  We expect most larger patches with high amount of token churn
/// to be the result of automated scripts with high locality like a code
/// formatter or the conversion of test manifest .ini files to .toml files, it
/// likely would be fine to not bother applying more expensive heuristics in
/// cases where there are an overwhelming number of changes or where a simple
/// top-N histogram of impacted tokens does not satisfy simple rename
/// heuristics.
fn infer_token_moves_from_diff_deltas {

}

///
///
fn hyperblame_for_path(
    diff_data: &TimelineData,
    commit: &git2::Commit,
    blob: &git2::Blob,
    import_helper: &mut Child,
    blame_parents: &[TimelineRepoCommit],
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
            .unmodified_tokens
            .get(&(parent.id(), path.to_path_buf()))
        {
            Some(entry) => entry,
            _ => continue,
        };
        let parent_annotate_blob = match read_path_blob(import_helper, blame_parent, parent_path) {
            Some(blob) => blob,
            _ => continue,
        };
        let parent_blame = std::str::from_utf8(&parent_annotate_blob)
            .unwrap() // This will always be valid
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

// Helper that recursively walks the tree for the given commit, skipping over
// unmodified entries.  When modified blobs are encountered, the provided
// `handler` is invoked.
//
// XXX wip notes: I think because this traversal is really so close to
// `build_blame_tree` and this is a pattern we expect to repeat for "files-struct"
// beyond the default "files" ingestion,
//
//
// XXX so build_blame_tree more sanely is just passing trees around, except for
// the leaf nodes where it calls blame_for_path and that uses file_movement to
// find the right predecessor path for the history file and then the import
// helper to fish out its contents.
//
// Because we generally do need to deal with the predecessor files here, we do
// need to maintain access to at least the "partition" (files, files-struct, etc.)
// root for the given trees rather than going fully relative.  In order to avoid
// passing a crap-load more stuff, maybe it makes sense to just stick with this
// current path-heavy approach but passing the "partition" to handle that aspect.
fn process_tree_changes(
    partition: &'static str,
    file_movement: Option<&HashMap<Oid, PathBuf>>,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    mut path: PathBuf,
    handler: &mut dyn FnMut(&Blob, &Blob, &Path)
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
            if let Ok(parent_entry) = parent.tree()?.get_path(&path)
                && parent_entry.id() == entry.id()
            {
                path.pop();
                continue 'outer;
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

                    handler(&blob, &parent_blob, &path);
                }
            }
            Some(ObjectType::Tree) => {
                process_tree_changes(partition, file_movement, git_repo, commit, path.clone(), handler)?;
            }
            _ => (),
        };

        path.pop();
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_blame_tree(
    diff_data: &TimelineData,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    tree_at_path: &git2::Tree,
    parent_trees: &[Option<git2::Tree>],
    import_helper: &mut Child,
    blame_parents: &[TimelineRepoCommit],
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
            if let Some(parent_entry) = parent_tree.get_name(entry_name)
                && parent_entry.id() == entry.id()
            {
                // Item at `path` is the same in the tree for `commit` as in
                // `parent_trees[i]`, so the blame must be the same too
                let oid = read_path_oid(import_helper, &blame_parents[i], &path).unwrap();
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

        match entry.kind() {
            Some(ObjectType::Blob) => {
                let blame_text = hyperblame_for_path(
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
                // This is a submodule. We insert a corresponding submodule entry in the blame
                // repo. The oid that we use doesn't really matter here but for hash-compatibility
                // with the old (pre-fast-import) code, we use the same hash that the old code
                // used, which corresponds to an empty directory.
                // For the external ref data format documentation, refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Externaldataformat
                assert_eq!(entry.filemode(), 0o160000);
                writeln!(
                    import_helper.stdin.as_mut().unwrap(),
                    "M {:06o} 4b825dc642cb6eb9a060e54bf8d69288fbee4904 {}",
                    entry.filemode(),
                    sanitize(&path)
                )
                .unwrap();
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

struct TimelineData {
    /// The source tree commit for which this DiffData holds data.
    source_rev: git2::Oid,
    /// The source tree hg commit, if we have an associated hg tree.
    source_hg_rev: Option<String>,
    // The history syntax tree commit for which this DiffData holds data.
    syntax_rev: git2::Oid,

    // Map from file (blob) id in the child rev to the path that the file was
    // at in the parent revision, for files that got moved. Set to None if the
    // child rev has multiple parents.
    file_movement: Option<HashMap<Oid, PathBuf>>,
    // Map to find unmodified tokens for modified files in a revision (files that
    // are not modified don't have entries here). The key is of the map is a
    // tuple containing the parent commit id and path to the file (in the child
    // revision). The parent commit id is needed in the case of merge commits,
    // where a file that is modified may have different sets of unmodified tokens
    // (by line index) with respect to the different parent commits.
    // The value in the map is a vec of token mappings as produced by the
    // `unmodified_tokens` function.
    unmodified_tokens: HashMap<(git2::Oid, PathBuf), Vec<(usize, usize)>>,
}

/// Accumulates raw token statistics for a single revision.
///
///
struct TokenStatsMachine {
    /// Stats for each token across the whole revision.
    revision_token_deltas: BTreeMap<String, TokenDelta>,

    /// Map from file to token to deltas for that token in that file.
    file_token_deltas: BTreeMap<String, BTreeMap<String, TokenDelta>>,


}

/// Accumulates deltas from hunks as driven by `ingest_diff_accumulating_deltas`
/// performing immediate local inference, as well as deriving sufficient state
/// to allow for move and rename inference in subsequent passes once all diffs
/// have been ingested.
///
/// ## Heuristics
///
/// ### Token evolution inferred from isolated changes
///
/// If we have a paired token removal and addition with stable context on either
/// side, we can potentially infer that the token "evolved", especially if the
/// the removed and added tokens meet other similarity heuristics.  Similarity
/// heuristics for cases where we have a tree-sitter grammar available could
/// mean that the token is still known to be a type identifier or a variable
/// name identifier.  For cases without a grammar, looking like a word in both
/// cases might be a sufficient metric.
///
/// This can also potentially be expanded to runs of tokens, especially in cases
/// where we do have additional tree-sitter grammar context we can use to know
/// that a sequence of tokens remains in the same "hole" in the "comby.dev"
/// terminology.  That is, if an argument "Type argName" changes to "NewType
/// newArgName", we would ideally be able to model that as "Type" evolving to
/// "NewType" and "argName" evolving to "newArgName".  And it might still
/// potentially be the case for a composite series of tokens like "Type"
/// evolving to "SomeNamespace::NamespacedType" where we would be going from 1
/// token to 3 tokens but still occupying the same hole, so we could still
/// potentially say the 3 tokens all evolved from the same 1st token.
///
/// ### Token Run Move Inference
///
/// In the case of refactorings, we expect a meaningful amount of locality when
/// it comes to moved chunks of code.  The patch author will likely be
/// performing cut-and-paste in large sections, potentially followed by targeted
/// changes.  We do not expect them to move individual tokens piece by piece
/// like the author is assembling a ransom note.  So it makes little sense for
/// our approach to act like they do.  To this end, we favor longer runs
struct DeltaMachine {
    cur_namespace: &'static str,

    // The current run's context; we automatically flush whenever the context
    // changes.  None if we're not in a run.
    cur_context: Option<String>,

    // New tokens and the 1-based line number they are being added on.
    added_in_run: Vec<(u32, String)>,
    // Removed tokens and the 1-base line number they are being removed from
    // (so their line number in the preceding revision).
    removed_in_run: Vec<(u32, String)>,

    path_pairs: Vec<(String, String)>,
    namespaces: HashMap<&'static str, DeltaNamespace>,
}

struct DeltaNamespace {
    /// Keyed by Context
    context_clusters: HashMap<String, DeltaContextCluster>,
}

struct DeltaContextCluster {
    // runs, each is associated with a single path-pair
    runs: Vec<DeltaRun>,

    // XXX NEXT: question of how to best represent the within-file moves, the
    // between-file moves, and the evolutions.  In general we want these keyed
    // by the old-path and new-path... I guess that does suggest that at the
    // end of the inference phase we want to render all of these into some kind
    // of new structure
    evolutions: Vec<()>,
    moved_out: Vec<()>,
    moved_in: Vec<()>,
}

impl DeltaContextCluster {
    // TODO: general idea here is:
    // - process all the tokens, generating utf8 identifiers for them as we go
    //   so that we can build a suffix array of all the removed tokens.  we use
    //   0 as the lowest sentinel that's required / to delimit the removed runs.
    // - we sort the addition runs by the number of additions so that we can
    //   try and match and consume longer runs first.
    // - we process the additions by doing a longest prefix search for the
    //   additions against the removals as we process forward through the run.
    //   - we have a position in the run, and we move forward each time we find
    //     a suitable match; see other notes but the general idea is that we
    //     do require some alphanum alignment to reuse.
    //   - because of consumption, we potentially maybe do the binary search
    //     greedy lcp and as long as that finds us an un-consumed run of tokens,
    //     we just use that.  But if we've already consumed the tokens, perhaps
    //     we slide around the adjacent indexes running a fitness func that does
    //     the locality thing, etc.  (The fitness func wouldn't be appropriate
    //     for binary search because it would be multi-dimensional for our
    //     locality needs.)  I think there's more thoughts in the notes too.
    fn infer_moves(&mut self) {

    }
}

struct DeltaRun {
    pub path_pair_index: u32,

    // (was this token consumed into an evolution, 1-based line number, token)
    // We retain the raw added/removed tokens for now marked as consumed as
    // potentially useful for backout processing and/or invariant checks.
    added: Vec<(bool, u32, String)>,
    removed: Vec<(bool, u32, String)>,

    // (old line number, old token string, new line number, new token string)
    evolved: Vec<(u32, String, u32, String)>,
}

/// Post-move/evolution-summary-inference providing per-token-line information
/// for a file and its predecessor as derived in parallel processing.  All token
/// references here are unresolved, non-canonical references that need to look
/// at the existing "annotated" state of the preceding revision in sequential
/// processing on the "main" thread to be correctly resolved.
struct DeltaFileSummary {

}

impl DeltaMachine {
    fn new() {
        Self {
            cur_namespace: "",
            cur_context: None,

            added_in_run: vec![],
            removed_in_run: vec![],

            namespaces: HashMap::default(),
        }
    }

    fn set_namespace(&mut self, namespace: &'static str) {
        self.cur_namespace = namespace;
    }

    /// Report the addition or removal of a symbol with the given pretty symbol
    /// context in the given semantic namespace, with "%" currently representing
    /// having no symbol context.  The origin must be either '+' or '-'
    /// indicating addition or removal.
    fn push_diff_token(&mut self, line: &DiffLine, context: &str, token: &str) {
        match line.origin() {
            '+' => {
                if self.cur_context.is_none() {
                    self.cur_context = Some(context.to_owned());
                } else if context != self.cur_context {
                    self.flush_run(false);
                    self.cur_context = Some(context.to_owned());
                }

                self.added_in_run.push((line.new_lineno().unwrap(), token.to_owned()));
            },
            '-' => {
                if self.cur_context.is_none() {
                    self.cur_context = Some(context.to_owned());
                } else if context != self.cur_context {
                    self.flush_run(false);
                    self.cur_context = Some(context.to_owned());
                }

                self.removed_in_run.push((line.old_lineno().unwrap(), token.to_owned()));
            },
            ' ' => {
                self.flush_run(true);
            },
            _ => return,
        };
    }

    fn flush_run(&mut self, infer_single_evolution: bool) {
        // (we expect to be called like this a lot currently)
        if self.added_in_run.len() == 0 && self.removed_in_run() == 0 {
            self.cur_context = None;
            return;
        }

        // If we have an added and removed single tokens and this is a case
        // where we think it's okay to infer single-token evolution (like
        // specifically because we hit a ' ' context, as opposed to having our
        // token context change), then infer it.
        if self.added_in_run.len() == 1 && self.removed_in_run.len() == 1 {

        }

        // If this is only removals (we know at least one of added/removed is
        // non-zero from our check at the top), we want to emit a marker removal
        // associated with the first token being removed.
        if self.added_in_run.len() == 0 {

        }
    }

    /// Called when we're at the end of the current hunk.
    fn flush_hunk(&mut self) {
        self.flush_run(true);
    }

    fn perform_inference_passes(&mut self) {
        // TODO: implement a phase where we look at overall token metrics and
        // clusters to infer evolved tokens and also apply this to the context
        // clusters.

        // XXX For now we just run inference within the clusters as they exist.
        for ns in self.namespaces.values_mut() {
            for cluster in ns.context_clusters.values_mut() {
                cluster.infer_moves();
            }
        }
    }
}

// Does the CPU-intensive work required for pre-computation of a given revision
// that can depend on any source tree revision and syntax token tree revisions
// (which will already have been generated through the current revision), but
// cannot depend on any revisions timeline revisions because those will not have
// been created yet.  (Anything that depends on timeline revisions needs to
// happen in our main thread logic.)
fn thread_preprocess_revision(
    git_repo: &git2::Repository,
    rev_meta: &HistorySyntaxCommitMeta,
) -> Result<TimelineData, git2::Error> {
    let commit = git_repo.find_commit(*rev_meta.syntax_rev).unwrap();

    // ## Infer file movement from the "files" tree
    //
    // We only need to determine movement once, and using the token rep is our
    // best option right now.  Note that as we teach the system to understand
    // renames and other refactorings, it becomes possible for us to use those
    // heuristics instead, although if we can follow the evolution of tokens
    // through time it's not clear that the file movement is as important.
    let file_movement = if commit.parent_count() == 1 {
        let parent_root = commit.parent(0).unwrap().tree().unwrap();
        let parent_files = parent_root
            .get_name("files")
            .unwrap()
            .to_object(git_repo)
            .unwrap()
            .peel_to_tree()
            .unwrap();
        let cur_root = commit.tree().unwrap();
        let cur_files = cur_root
            .get_name("files")
            .unwrap()
            .to_object(git_repo)
            .unwrap()
            .peel_to_tree()
            .unwrap();

        let mut movement = HashMap::new();
        let mut diff = git_repo
            .diff_tree_to_tree(Some(&parent_files), Some(&cur_files), None)
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

    // ## Process the "files" token-centric mapping
    //
    // This provides us with the unmodified_tokens mapping as well as
    // accumulated add/removed tokens so we can try and infer moves in the
    // next passes.
    let mut unmodified_tokens = HashMap::new();

    let mut delter = DeltaMachine::new();
    process_tree_changes(
        "files",
        file_movement.as_ref(),
        git_repo,
        &commit,
        PathBuf::new(),
        &mut |blob: &Blob, parent_blob: &Blob, path: &Path| {
            ingest_diff_accumulating_deltas(blob, parent_blob, path, delter);
        }
    )?;

    // ## Process token movement inference
    //
    // We have ingested all the diff hunks which means we can now perform our
    // various inference phases for token movement and evolutions.
    delter.infer_moves();

    // ## Process the "file-struct" symbol rep


    Ok(TimelineData {
        source_rev: *git_oid,
        // XXX this should get filled in downstream
        source_hg_rev: None,
        syntax_rev:

        file_movement,
        unmodified_tokens,
    })
}

struct ComputeThread {
    query_tx: Sender<HistorySyntaxCommitMeta>,
    response_rx: Receiver<TimelineData>,
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

    fn compute(&self, rev_meta: &HistorySyntaxCommitMeta) {
        self.query_tx.send(*rev_meta).unwrap();
    }

    fn read_result(&self) -> TimelineData {
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
    query_rx: Receiver<HistorySyntaxCommitMeta>,
    response_tx: Sender<TimelineData>,
    git_repo_path: String,
) {
    let git_repo = Repository::open(git_repo_path).unwrap();
    while let Ok(rev) = query_rx.recv() {
        let result = thread_preprocess_revision(&git_repo, &rev).unwrap();
        response_tx.send(result).unwrap();
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<_> = env::args().collect();
    let source_repo_path = args[1].to_string();
    let source_repo = Repository::open(&source_repo_path).unwrap();
    let syntax_repo_path = args[2].to_string();
    let syntax_repo = Repository::open(&syntax_repo_path).unwrap();
    let timeline_repo = Repository::open(&args[3]).unwrap();
    let rev_summary_root = &args[4];

    // Note that don't do anything with hg or cinnabar in this program; we depend
    // on build-syntax-token-tree to have included an "hg HGREV" line in the
    // commit messages it emitted into the syntax_repo.

    let blame_ref = env::var("BLAME_REF").ok().unwrap_or("HEAD".to_string());
    let commit_limit = env::var("COMMIT_LIMIT")
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);

    info!("Reading existing blame map of timeline repo ref {}...", blame_ref);
    /// Maps syntax repo revision to timeline repo commit
    let mut timeline_map = if let Ok(oid) = timeline_repo.refname_to_id(&blame_ref) {
        let timeline_map = index_timeline_history_by_source_rev(&timeline_repo, Some(oid));
        timeline_map
            .into_iter()
            .map(|(k, v)| (k, TimelineRepoCommit::Commit(v.timeline_rev)))
            .collect::<HashMap<git2::Oid, TimelineRepoCommit>>()
    } else {
        HashMap::new()
    };

    // We are primarily processing the "syntax" repo which is derived from the
    // "source" repo.  So start a walk in the syntax repo from the provided
    // BLAME_REF.
    let mut walk = syntax_repo.revwalk().unwrap();
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    walk.push(syntax_repo.refname_to_id(&blame_ref).unwrap())
        .unwrap();
    let mut revs_to_process = walk
        .map(|r| r.unwrap()) // walk produces Result<git2::Oid> so we unwrap to just the Oid
        // We don't need to process revisions we already have a timeline revision for.
        .filter(|git_oid| !timeline_map.contains_key(git_oid))
        // Read the commit so we can have all the relevant revision identifiers.
        .map(|syntax_oid| {
            let commit = syntax_repo.find_commit(syntax_oid).unwrap();
            syntax_commit_to_meta(&commit)
        })
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
        compute_threads.push(ComputeThread::new(&syntax_repo_path));
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

    let mut import_helper = start_fast_import(&syntax_repo);

    // Tracks completion count and serves as the basis for the mark <idnum>
    // assigned to each commit.
    let mut rev_done = 0;

    for rev_meta in revs_to_process.iter() {
        // Read a result. Since we hand out compute requests in round-robin order
        // and each thread processes them in FIFO order we know exactly which
        // thread is going to give us our result.
        // We assert to make sure it's the right one.
        let thread = &compute_threads[rev_done % num_threads];
        let diff_data = thread.read_result();
        assert!(diff_data.revision == *rev_meta.syntax_rev);

        // If there are more revisions that we haven't requested yet, request
        // another one from this thread.
        if compute_index < rev_count {
            thread.compute(&revs_to_process[compute_index]);
            compute_index += 1;
        }

        rev_done += 1;

        info!(
            "Transforming {} (hg {:?}) progress {}/{}",
            rev_meta.syntax_rev, rev_meta.source_hg_rev, rev_done, rev_count
        );
        let commit = source_repo.find_commit(*rev_meta.syntax_rev).unwrap();
        let parent_trees = commit
            .parents()
            .map(|parent_commit| Some(parent_commit.tree().unwrap()))
            .collect::<Vec<_>>();
        let blame_parents = commit
            .parent_ids()
            .map(|pid| timeline_map[&pid])
            .collect::<Vec<_>>();

        // Scope the import_helper borrow
        {
            // Here we write out the metadata for a new commit to the blame repo.
            // For details on the data format, refer to the documentation at
            // https://git-scm.com/docs/git-fast-import#_commit
            // https://git-scm.com/docs/git-fast-import#_mark
            let mut import_stream = BufWriter::new(import_helper.stdin.as_mut().unwrap());
            writeln!(import_stream, "commit {}", blame_ref).unwrap();
            writeln!(import_stream, "mark :{}", rev_done).unwrap();
            timeline_map.insert(*rev_meta.syntax_rev, TimelineRepoCommit::Mark(rev_done));

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

            let commit_msg = if let Some(hg_rev) = rev_meta.source_hg_rev {
                format!("git {}\nsyntax {}\nhg {}\n", rev_meta.source_rev, rev_meta.syntax_rev, hg_rev)
            } else {
                format!("git {}\nsyntax {}\n", rev_meta.source_rev, rev_meta.syntax_rev)
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
            // In a change from "build-blame.rs", we don't use "deleteall" because we
            // want to retain the existing contents of the "tokens" subdir.  However,
            // we do want the semantics of starting from deletion for all other subdirs,
            // so we do explicitly delete those subdirectories.
            writeln!(import_stream, "D annotated").unwrap();
            writeln!(import_stream, "D future").unwrap();
            writeln!(import_stream, "D files-delta").unwrap();
            import_stream.flush().unwrap();
        }

        build_blame_tree(
            &diff_data,
            &source_repo,
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
}
