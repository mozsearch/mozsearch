// This binary derives both a token-centric (token per line) representation of
// the source files in the input source tree using tree-sitter as well as
// synthetic files using the same tree-sitter derived information.  It is
// intended to be subsequently processed by build-syntax-blame-tree.rs.

extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate num_cpus;
extern crate tools;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use git2::{ObjectType, Oid, Repository, Sort};
use tools::file_format::config::index_blame;
use tools::file_format::history::io_helpers::{
    read_record_file_contents, record_file_contents_to_string,
};
use tools::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use tools::file_format::history::syntax_symdex::{SymdexHeader, SymdexRecord};
use tools::tree_sitter_support::cst_tokenizer::{hypertokenize_source_file, HyperTokenized};

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
enum SyntaxRepoCommit {
    Commit(git2::Oid),
    Mark(usize),
}

impl fmt::Display for SyntaxRepoCommit {
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
    commit: &SyntaxRepoCommit,
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
    commit: &SyntaxRepoCommit,
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

fn process_modified_files(
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    mut path: PathBuf,
    results: &mut HashMap<PathBuf, HyperTokenized>,
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
                let path_str = path.as_os_str().to_string_lossy();
                if let Ok(blob_as_str) = std::str::from_utf8(blob.content()) {
                    if let Ok(hypertokenized) = hypertokenize_source_file(&path_str, blob_as_str) {
                        results.insert(path.clone(), hypertokenized);
                    }
                }
            }
            Some(ObjectType::Tree) => {
                process_modified_files(git_repo, commit, path.clone(), results)?;
            }
            _ => (),
        };

        path.pop();
    }

    Ok(())
}

/// Per-symdex symbol scratchpad for regenerating impacted symdex files.
#[derive(Default)]
struct SymbolNotes {
    /// The list of source files that referenced this symbol in their previous
    /// contents and that we need to filter out of the symdex file before adding
    /// our new records before.  This is all naive and we're not doing any
    /// diffing, so it's possible our changes end up as a net no-op.
    files_to_filter: HashSet<PathBuf>,
    /// The list of the records we want to insert into the given symdex file.
    symdex_records: Vec<SymdexRecord>,
}

/// Recursively process a subtree of the source tree, populating the derived
/// syntax repo "files" and "file-struct" subtrees as we go and accumulating
/// info in `symdex` for a post-pass once the root invocation of this method has
/// finished.
///
/// Broadly, we walk the contents of the current source tree subtree and for
/// each subtree (dir) or blob (file), we check if they've changed relative to
/// the parent revisions.  If they haven't changed, then we can just propagate
/// the existing syntax tree nodes.  A nice simplification here is that we don't
/// actually need to walk the syntax repo "files" and "files-struct" subtrees;
/// we can just look up their contents when we're propagating them.
fn recursively_process_source_tree(
    syntax_data: &SyntaxTreeData,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    tree_at_path: &git2::Tree,
    parent_trees: &[Option<git2::Tree>],
    import_helper: &mut Child,
    syntax_parents: &[SyntaxRepoCommit],
    mut path: PathBuf,
) -> Result<(), git2::Error> {
    let files_root = PathBuf::from("files");
    let files_struct_root = PathBuf::from("files-struct");

    'outer: for entry in tree_at_path.iter() {
        let entry_name = entry.name().unwrap();
        path.push(entry_name);
        info!(" - Considering {}", path.display());

        let mut tokenize_path = files_root.clone();
        tokenize_path.push(&path);

        let mut struct_path = files_struct_root.clone();
        struct_path.push(&path);

        for (i, parent_tree) in parent_trees.iter().enumerate() {
            let parent_tree = match parent_tree {
                None => continue, // This parent doesn't even have a tree at this path
                Some(p) => p,
            };
            if let Some(parent_entry) = parent_tree.get_name(entry_name) {
                if parent_entry.id() == entry.id() {
                    // Item at `path` is the same in the tree for `commit` as in
                    // `parent_trees[i]` so we can propagate our existing derived
                    // "files" and "files-struct" entries which will not have
                    // changed.  This works for trees/blobs/everything.

                    info!(
                        "  For {} with id {} trying to propagate {} and {}",
                        path.display(),
                        entry.id(),
                        tokenize_path.display(),
                        struct_path.display()
                    );

                    // "files" entry
                    let oid = match read_path_oid(import_helper, &syntax_parents[i], &tokenize_path)
                    {
                        Some(oid) => oid,
                        // If we lack existing history for this entry and nothing has changed in it,
                        // just skip the entry, because there's nothing we can do to make it have
                        // have history.
                        _ => {
                            path.pop();
                            continue 'outer;
                        }
                    };
                    writeln!(
                        import_helper.stdin.as_mut().unwrap(),
                        "M {:06o} {} {}",
                        entry.filemode(),
                        oid,
                        sanitize(&tokenize_path)
                    )
                    .unwrap();

                    // "files-struct" entry
                    let oid =
                        read_path_oid(import_helper, &syntax_parents[i], &struct_path).unwrap();
                    writeln!(
                        import_helper.stdin.as_mut().unwrap(),
                        "M {:06o} {} {}",
                        entry.filemode(),
                        oid,
                        sanitize(&struct_path)
                    )
                    .unwrap();

                    path.pop();
                    continue 'outer;
                }
            }
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                // ## Load any old "files-struct" entries to populate SymbolNotes::files_to_filter
                for syntax_parent in syntax_parents {
                    let parent_syntax_struct_blob =
                        match read_path_blob(import_helper, syntax_parent, &struct_path) {
                            Some(blob) => blob,
                            _ => continue,
                        };
                    let parsed_file: Option<(FileStructureHeader, Vec<FileStructureRow>)> =
                        read_record_file_contents(&parent_syntax_struct_blob);
                    if let Some((header, records)) = parsed_file {
                        let lang = match header.lang {
                            Some(lang) => lang,
                            _ => continue,
                        };
                        let by_lang = symdex.entry(lang.clone()).or_default();
                        for record in records {
                            let sym_notes = by_lang.entry(record.pretty.clone()).or_default();
                            sym_notes.files_to_filter.insert(path.clone());
                        }
                    }
                }

                // ## Process the new hypertokenized data, if any

                // For the inline data format documentation, refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Inlinedataformat
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Exactbytecountformat
                let import_stream = import_helper.stdin.as_mut().unwrap();

                if let Some(hypertokenized) = syntax_data.hypertokenized_files.get(&path) {
                    info!(
                        "  Writing out {} and {} (tokens: {} structure: {})",
                        tokenize_path.display(),
                        struct_path.display(),
                        hypertokenized.tokenized.len(),
                        hypertokenized.structure.len(),
                    );

                    // ## Write the tokenized file contents
                    let tokenized_text = hypertokenized.tokenized.join("\n");
                    let tokenized_bytes = tokenized_text.as_bytes();
                    writeln!(
                        import_stream,
                        "M {:06o} inline {}",
                        entry.filemode(),
                        sanitize(&tokenize_path)
                    )
                    .unwrap();
                    writeln!(import_stream, "data {}", tokenized_bytes.len()).unwrap();
                    import_stream.write_all(tokenized_bytes).unwrap();
                    // We skip the optional trailing LF character here since in practice it
                    // wasn't particularly useful for debugging. Also the blame blobs we write
                    // here always have a trailing LF anyway.

                    // ## Write the files-struct contents
                    let struct_text = record_file_contents_to_string(
                        &FileStructureHeader {
                            lang: Some(hypertokenized.lang.clone()),
                        },
                        &hypertokenized.structure,
                    );
                    let struct_bytes = struct_text.as_bytes();

                    writeln!(
                        import_stream,
                        "M {:06o} inline {}",
                        entry.filemode(),
                        sanitize(&struct_path)
                    )
                    .unwrap();
                    writeln!(import_stream, "data {}", struct_bytes.len()).unwrap();
                    import_stream.write_all(struct_bytes).unwrap();
                    // (skipping trailing LF again)

                    // ## Accumulate the symdex data.
                    if !hypertokenized.structure.is_empty() {
                        let by_lang = symdex.entry(hypertokenized.lang.clone()).or_default();
                        let source_path = path.to_str().unwrap();
                        for record in &hypertokenized.structure {
                            // Place the record on its parent if it has one too.
                            // XXX Currently we do this for all symbol types even the ones where
                            // maybe the parent doesn't really want the child present, but I think
                            // I came around to believing the linkage might be useful.
                            if let Some((parent, _)) = record.pretty.rsplit_once("::") {
                                let sym_notes = by_lang.entry(parent.to_string()).or_default();
                                sym_notes.symdex_records.push(SymdexRecord {
                                    file_row: record.clone(),
                                    path: source_path.to_string(),
                                });
                            }

                            // Add the entry for the symbol itself.
                            let sym_notes = by_lang.entry(record.pretty.clone()).or_default();
                            sym_notes.symdex_records.push(SymdexRecord {
                                file_row: record.clone(),
                                path: source_path.to_string(),
                            });
                        }
                    }
                } else {
                    warn!(
                        "  Did not find hypertokenized version of {}",
                        path.display()
                    );
                }
                // We skip the optional trailing LF character here since in practice it
                // wasn't particularly useful for debugging. Also the blame blobs we write
                // here always have a trailing LF anyway.
            }
            Some(ObjectType::Commit) => {
                // This is a submodule.  We don't create any entries for these
                // because we already won't have entries for things like binary
                // files.  This can be revisited in the future, but for now it
                // likely makes sense to not handle them and leave it up to the
                // normal boring "git log" functionality.
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
                recursively_process_source_tree(
                    syntax_data,
                    symdex,
                    git_repo,
                    commit,
                    &entry.to_object(git_repo)?.peel_to_tree()?,
                    &parent_subtrees,
                    import_helper,
                    syntax_parents,
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

/// Process the symdex data populated by `recursively_process_source_tree` by
/// modifying the contents of the "symdex" subtree of the syntax repo.
///
/// Because the files in the "symdex" subtree are aggregations of data from both
/// files that may have been modified as well as files that have not been
/// modified, our implementation approach in this file needs to deviate from the
/// more straightforward process used in the line-centric "build-blame.rs".
/// Specifically, in "build-blame" we use the "deleteall" command and then
/// propagate what exists in the tree, allowing deletions to be emergently
/// effected by deleted content not being propagated.
///
/// But for our symdex, we want to propagate everything that hasn't been
/// changed.  So our revised approach is to only issue deletion directives for
/// our "files" and "files-struct" subdirs and then re-propagate them
/// "build-blame" style.  This should net out the same for them, but this will
/// leave around our "symdex" subdir.  We can then issue explicit "filemodify"
/// commands for changed files and "filedelete" commands for files which would
/// have 0 records after the header after being filtered.  (Although maybe we
/// never actually want to delete those?  Need to figure out how easy it is for
/// the next stage to explicitly notice the deletions.)
fn process_symdex_tree(
    symdex: HashMap<String, HashMap<String, SymbolNotes>>,
    import_helper: &mut Child,
    syntax_parents: &[SyntaxRepoCommit],
) -> Result<(), git2::Error> {
    info!("Processing symdex tree.");
    for (lang, lang_symbols) in symdex {
        info!(
            "Processing symdex lang {} with {} symbols.",
            lang,
            lang_symbols.len()
        );
        for (pretty, mut notes) in lang_symbols {
            let sym_path = PathBuf::from(format!(
                "symdex/{}/{}.ndjson",
                lang,
                pretty.replace("::", "/")
            ));
            let mut records: Vec<SymdexRecord> = vec![];

            for syntax_parent in syntax_parents {
                let parent_symdex_blob =
                    match read_path_blob(import_helper, syntax_parent, &sym_path) {
                        Some(blob) => blob,
                        _ => continue,
                    };
                let parsed_file: Option<(SymdexHeader, Vec<SymdexRecord>)> =
                    read_record_file_contents(&parent_symdex_blob);
                records = if let Some((_header, mut records)) = parsed_file {
                    records
                        .drain(0..)
                        .filter(|rec| !notes.files_to_filter.contains(&PathBuf::from(&rec.path)))
                        .collect()
                } else {
                    records
                };

                break;
            }

            records.append(&mut notes.symdex_records);
            records.sort();

            let header = SymdexHeader {};

            // Delete the file if we no longer have any records for the file.
            if records.is_empty() {
                info!("  Deleting moot symdex file {}", sym_path.display());
                writeln!(
                    import_helper.stdin.as_mut().unwrap(),
                    "D {}",
                    sanitize(&sym_path)
                )
                .unwrap();
            } else {
                let symdex_text = record_file_contents_to_string(&header, &records);
                let symdex_bytes = symdex_text.as_bytes();

                info!(
                    "  Writing symdex file {} with {} entries.",
                    sym_path.display(),
                    records.len()
                );
                let import_stream = import_helper.stdin.as_mut().unwrap();

                writeln!(import_stream, "M 100644 inline {}", sanitize(&sym_path)).unwrap();
                writeln!(import_stream, "data {}", symdex_bytes.len()).unwrap();
                import_stream.write_all(symdex_bytes).unwrap();
                // (skipping trailing LF again)
            }
        }
    }
    info!("Done processing symdex.");

    Ok(())
}

struct SyntaxTreeData {
    /// The commit for which this DiffData holds data.
    revision: git2::Oid,

    /// The hypertokenized state for each modified source path.
    hypertokenized_files: HashMap<PathBuf, HyperTokenized>,
}

// Does the CPU-intensive work required for blame computation of a given revision.
// This does not mutate anything in `git_repo` and has no other dependencies, so
// it can be parallelized.
fn compute_diff_data(
    git_repo: &git2::Repository,
    git_oid: &git2::Oid,
) -> Result<SyntaxTreeData, git2::Error> {
    let commit = git_repo.find_commit(*git_oid).unwrap();

    let mut hypertokenized_files = HashMap::new();
    process_modified_files(git_repo, &commit, PathBuf::new(), &mut hypertokenized_files)?;

    Ok(SyntaxTreeData {
        revision: *git_oid,
        hypertokenized_files,
    })
}

struct ComputeThread {
    query_tx: Sender<git2::Oid>,
    response_rx: Receiver<SyntaxTreeData>,
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

    fn read_result(&self) -> SyntaxTreeData {
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
    response_tx: Sender<SyntaxTreeData>,
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
            .into_iter()
            .map(|(k, v)| (k, SyntaxRepoCommit::Commit(v)))
            .collect::<HashMap<git2::Oid, SyntaxRepoCommit>>()
    } else {
        HashMap::new()
    };
    info!("  Blame map has {} existing entries.", blame_map.len());

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

    let mut import_helper = start_fast_import(&blame_repo);

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
            writeln!(import_stream, "mark :{}", rev_done).unwrap();
            blame_map.insert(*git_oid, SyntaxRepoCommit::Mark(rev_done));

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
            // In a change from "build-blame.rs", we don't use "deleteall" because we
            // want to retain the existing contents of the "symdex" subdir.  However,
            // we do want the semantics of starting from deletion for "files" and
            // "files-struct", so we do explicitly delete those subdirectories.
            writeln!(import_stream, "D files").unwrap();
            writeln!(import_stream, "D files-struct").unwrap();
            import_stream.flush().unwrap();
        }

        // Keying:
        // - language ("cxx", "rust", etc.) as returned by `hypertokenize_source_file`
        // - "pretty" symbol identifier
        let mut symdex: HashMap<String, HashMap<String, SymbolNotes>> = HashMap::new();

        recursively_process_source_tree(
            &diff_data,
            &mut symdex,
            &git_repo,
            &commit,
            &commit.tree().unwrap(),
            &parent_trees,
            &mut import_helper,
            &blame_parents,
            PathBuf::new(),
        )
        .unwrap();

        process_symdex_tree(symdex, &mut import_helper, &blame_parents).unwrap();

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
