// This binary derives both a token-centric (token per line) representation of
// the source files in the input source tree using tree-sitter as well as
// synthetic files using the same tree-sitter derived information.  It is
// intended to be subsequently processed by build-timeline-tree.rs.
//
// Usage:
//   build-syntax-token-tree SOURCE_REPO SYNTAX_REPO [HISTORY_CONFIG_DIR]
//     [--old-cinnabar-repo-path OLD_REPO] [--old-revision-map MAP_FILE]
//
// See `hyperblame::history_config` for the optional history configuration and
// `tools::cinnabar` for the old revision ("oldrevs") options, which match
// build-blame's.  The environment variables `BLAME_REF`, `COMMIT_LIMIT`, and
// `CINNABAR` (set to 0 to not ask git-cinnabar for hg revisions) are handled
// like build-blame.
//
// Rather than walking the syntax repo's branch to find the revisions we've
// already processed, we record them in git notes in the syntax repo, which also
// covers processed revisions that aren't reachable from the branch; see
// `source_mapping` for details, including the `NOTES_REF` and
// `READ_NOTES_REFS` environment variables.

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
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use clap::Parser;
use git2::{ObjectType, Oid, Repository, Sort, TreeWalkMode, TreeWalkResult};
use tools::cinnabar::{CinnabarBatch, OldRevisions};
use tools::file_format::config::{HistorySyntaxCommitMeta, syntax_commit_to_meta};
use tools::file_format::history::io_helpers::{
    read_record_file_contents, record_file_contents_to_string,
};
use tools::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use tools::file_format::history::syntax_symdex::{SymdexHeader, SymdexRecord};
use tools::git_ops::{fast_import_git, history_compute_threads};
use tools::history_stop::{STOPPED_EXIT_CODE, stop_requested};
use tools::hyperblame::history_config::{
    AttributeRules, AttributeSet, EffectiveAttributes, HistoryConfig, LangSource, ResolvedLanguage,
    parse_repo_gitattributes, resolve_language,
};
use tools::source_mapping::{
    NOTES_BATCH_SIZE, NotesRefs, SourceMapping, notes_writer, point_branch_at,
    require_notes_for_existing_branch,
};
use tools::tree_sitter_support::cst_tokenizer::{
    HyperTokenized, LanguageProfile, TOKENIZER_VERSION, hypertokenize_with_profile,
};

#[derive(Parser)]
struct Cli {
    /// Path to the source git repository.
    #[clap(value_parser)]
    git_repo_path: String,

    /// Path to the syntax git repository to populate.
    #[clap(value_parser)]
    syntax_repo_path: String,

    /// Optional history configuration directory; see
    /// `hyperblame::history_config`.
    #[clap(value_parser)]
    history_config_dir: Option<String>,

    /// The old git-cinnabar repository to map hg revisions to old revisions
    /// with; see `tools::cinnabar` and build-blame's option of the same name.
    #[clap(long, value_parser)]
    old_cinnabar_repo_path: Option<String>,

    /// A file mapping revisions to old revisions which takes precedence over
    /// the old cinnabar repository; see build-blame's option of the same name.
    #[clap(long, value_parser)]
    old_revision_map: Option<String>,
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
    fast_import_git()
        // We rewrite big files (ex: journals) a lot, and the fastest zlib level
        // saves time without making much difference in size.  (The packs get
        // repacked by maintenance anyway.)
        .arg("-c")
        .arg("core.compression=1")
        // Blobs over 1k are stored whole rather than as deltas against the
        // previous blob, which is almost always a different file; see
        // build-timeline-tree.
        .arg("-c")
        .arg("core.bigFileThreshold=1k")
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

/// Retrieve the commit oid for a mark via the `get-mark` command.  The commit
/// that defined the mark must have been terminated.
fn read_mark_oid(import_helper: &mut Child, mark: usize) -> String {
    writeln!(import_helper.stdin.as_mut().unwrap(), "get-mark :{}", mark).unwrap();
    let mut reader = BufReader::new(import_helper.stdout.as_mut().unwrap());
    let mut result = String::new();
    reader.read_line(&mut result).unwrap();
    result.trim().to_string()
}

/// Where to read syntax repo data from.  Reading from the commit being written
/// (`Active`) gives the first parent's version of any path not yet modified in
/// it, since it starts out with the first parent's tree, and is much cheaper
/// than reading from the first parent itself, which makes git-fast-import load
/// the first parent's trees along the path again (and the "symdex/js" tree can
/// have tens of thousands of entries).
#[derive(Clone, Copy)]
enum ReadFrom<'a> {
    Active,
    Commit(&'a SyntaxRepoCommit),
}

/// Quote a path with git-fast-import's C-style quoting, which is required when
/// reading from the active commit.
fn quote_path(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

#[test]
fn test_quote_path() {
    assert_eq!(quote_path(Path::new("files/a b.cpp")), "\"files/a b.cpp\"");
    assert_eq!(
        quote_path(Path::new("files/\"q\"\\x\ny")),
        r#""files/\"q\"\\x\ny""#
    );
}

/// Read the oid of the object at the given path in the given
/// commit. Returns None if there is no such object.
/// Documentation for the fast-import command used is at
/// https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Readingfromanamedtree
fn read_path_oid(import_helper: &mut Child, from: ReadFrom, path: &Path) -> Option<String> {
    let stdin = import_helper.stdin.as_mut().unwrap();
    match from {
        ReadFrom::Active => writeln!(stdin, "ls {}", quote_path(path)),
        ReadFrom::Commit(commit) => writeln!(stdin, "ls {} {}", commit, sanitize(path)),
    }
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
fn read_path_blob(import_helper: &mut Child, from: ReadFrom, path: &Path) -> Option<Vec<u8>> {
    let oid = read_path_oid(import_helper, from, path)?;
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
            // (LF must be written as `\n`; a backslash followed by an actual LF
            // would end the command line.)
            .replace("\n", "\\n")
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
    assert_eq!(sanitize(&p4), "\"internal/lf/\\n/needs/escaping\"");
}

/// A file tokenized for the syntax repo and why we picked its language.
struct TokenizedFile {
    hypertokenized: HyperTokenized,
    lang_source: LangSource,
}

/// The state needed by `process_modified_files`.
struct ModifiedFilesContext<'a> {
    /// The trees of the revision's parents, excluding any from before the
    /// history's start revision.
    parent_trees: &'a [git2::Tree<'a>],
    attrs: &'a EffectiveAttributes,
    /// Paths (and their ancestor directories) which must be processed even
    /// though they are unchanged because their resolved language changed.
    forced: &'a HashSet<PathBuf>,
    /// The files to tokenize; see `tokenize_files`.
    to_tokenize: Vec<(PathBuf, Oid, LanguageProfile, LangSource)>,
    /// Files we intentionally didn't tokenize.
    skipped: HashSet<PathBuf>,
}

/// How many files a revision needs to change for us to tokenize them in
/// parallel.  Revisions are processed in parallel anyway, but a big revision
/// (ex: the start of a history window, or a big merge) would otherwise hold up
/// everything after it.
const PARALLEL_TOKENIZE_FILES: usize = 64;

/// Tokenize the files collected by `process_modified_files`.
fn tokenize_files(
    git_repo: &git2::Repository,
    to_tokenize: Vec<(PathBuf, Oid, LanguageProfile, LangSource)>,
) -> Result<HashMap<PathBuf, TokenizedFile>, git2::Error> {
    let mut inputs = Vec::with_capacity(to_tokenize.len());
    for (path, oid, profile, lang_source) in to_tokenize {
        let content = git_repo.find_blob(oid)?.content().to_vec();
        inputs.push((path, content, profile, lang_source));
    }
    let tokenize =
        |(path, content, profile, lang_source): (PathBuf, Vec<u8>, LanguageProfile, LangSource)| {
            let text = std::str::from_utf8(&content).ok()?;
            let hypertokenized = hypertokenize_with_profile(profile, text).ok()?;
            Some((
                path,
                TokenizedFile {
                    hypertokenized,
                    lang_source,
                },
            ))
        };
    if inputs.len() < PARALLEL_TOKENIZE_FILES {
        return Ok(inputs.into_iter().filter_map(tokenize).collect());
    }
    // Deal the files out round robin so that each thread gets a mix.
    let num_workers = num_cpus::get().min(inputs.len());
    let mut chunks: Vec<Vec<_>> = (0..num_workers).map(|_| vec![]).collect();
    for (idx, input) in inputs.into_iter().enumerate() {
        chunks[idx % num_workers].push(input);
    }
    Ok(thread::scope(|scope| {
        let workers: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || chunk.into_iter().filter_map(tokenize).collect::<Vec<_>>())
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect()
    }))
}

fn process_modified_files(
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    mut path: PathBuf,
    ctx: &mut ModifiedFilesContext,
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
        if !ctx.forced.contains(&path) {
            for parent_tree in ctx.parent_trees {
                if let Ok(parent_entry) = parent_tree.get_path(&path)
                    && parent_entry.id() == entry.id()
                {
                    path.pop();
                    continue 'outer;
                }
            }
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                let path_str = path.as_os_str().to_string_lossy().into_owned();
                match resolve_language(&path_str, ctx.attrs) {
                    ResolvedLanguage::Skip => {
                        ctx.skipped.insert(path.clone());
                    }
                    ResolvedLanguage::Tokenize(profile, lang_source) => {
                        ctx.to_tokenize
                            .push((path.clone(), entry.id(), profile, lang_source));
                    }
                }
            }
            Some(ObjectType::Tree) => {
                process_modified_files(git_repo, commit, path.clone(), ctx)?;
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
    /// our new records before.
    files_to_filter: HashSet<PathBuf>,
    /// The records the first parent's versions of `files_to_filter` had for
    /// this symbol, which `drop_unchanged_files` compares to the new records.
    old_records: Vec<SymdexRecord>,
    /// The list of the records we want to insert into the given symdex file.
    symdex_records: Vec<SymdexRecord>,
}

impl SymbolNotes {
    /// Forget the files whose records for this symbol didn't change from the
    /// first parent's, whose records in the symdex file don't need rewriting.
    /// Records don't have line numbers, so most changes to a file don't change
    /// its records, and rewriting the symdex files of all of a file's symbols
    /// means reading them all from git-fast-import, including huge ones (ex:
    /// the "mozilla" namespace is in most C++ files).  Only for linear
    /// revisions, since the first parent is what the symdex file comes from.
    fn drop_unchanged_files(&mut self) {
        fn by_path(records: &[SymdexRecord]) -> HashMap<&str, Vec<&SymdexRecord>> {
            let mut by_path: HashMap<&str, Vec<&SymdexRecord>> = HashMap::new();
            for record in records {
                by_path
                    .entry(record.path.as_str())
                    .or_default()
                    .push(record);
            }
            for records in by_path.values_mut() {
                records.sort();
            }
            by_path
        }
        let old = by_path(&self.old_records);
        let new = by_path(&self.symdex_records);
        let unchanged: HashSet<PathBuf> = self
            .files_to_filter
            .iter()
            .filter(|path| {
                let path = path.to_str().unwrap_or_default();
                old.get(path) == new.get(path)
            })
            .cloned()
            .collect();
        if unchanged.is_empty() {
            return;
        }
        self.files_to_filter
            .retain(|path| !unchanged.contains(path));
        self.symdex_records
            .retain(|record| !unchanged.contains(Path::new(&record.path)));
    }
}

#[test]
fn test_drop_unchanged_files() {
    let record = |pretty: &str, path: &str| SymdexRecord {
        file_row: FileStructureRow {
            pretty: pretty.to_string(),
            is_def: true,
            kind: "method".to_string(),
        },
        path: path.to_string(),
    };
    let mut notes = SymbolNotes::default();
    notes.files_to_filter.insert(PathBuf::from("a.cpp"));
    notes.files_to_filter.insert(PathBuf::from("b.cpp"));
    notes.files_to_filter.insert(PathBuf::from("gone.cpp"));
    notes.old_records = vec![
        record("A::g", "a.cpp"),
        record("A::f", "a.cpp"),
        record("A::g", "b.cpp"),
        record("A::g", "gone.cpp"),
    ];
    notes.symdex_records = vec![
        record("A::f", "a.cpp"),
        record("A::g", "a.cpp"),
        record("A::h", "b.cpp"),
        record("A::i", "new.cpp"),
    ];
    notes.drop_unchanged_files();
    // a.cpp's records are the same (in another order), b.cpp's changed,
    // gone.cpp was removed, and new.cpp was added.
    assert_eq!(
        notes.files_to_filter,
        HashSet::from([PathBuf::from("b.cpp"), PathBuf::from("gone.cpp")])
    );
    assert_eq!(
        notes.symdex_records,
        vec![record("A::h", "b.cpp"), record("A::i", "new.cpp")]
    );
}

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
#[allow(clippy::too_many_arguments)]
/// Where to read the syntax repo data corresponding to `parent_idx`'s tree.
fn parent_source(syntax_parents: &[SyntaxRepoCommit], parent_idx: usize) -> ReadFrom<'_> {
    if parent_idx == 0 {
        ReadFrom::Active
    } else {
        ReadFrom::Commit(&syntax_parents[parent_idx])
    }
}

/// Note the symdex records of a parent's version of the source file at `path`
/// (from its "files-struct" entry), so that they're filtered out of their
/// symbols' symdex files, and for the first parent, so that
/// `SymbolNotes::drop_unchanged_files` can compare them with the new records.
/// Reading from the commit being written gives the first parent's version as
/// long as the entry hasn't been modified yet.
fn note_old_records(
    import_helper: &mut Child,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    from: ReadFrom,
    path: &Path,
    first_parent: bool,
) {
    let struct_path = PathBuf::from("files-struct").join(path);
    let Some(blob) = read_path_blob(import_helper, from, &struct_path) else {
        return;
    };
    let parsed_file: Option<(FileStructureHeader, Vec<FileStructureRow>)> =
        read_record_file_contents(&blob);
    let Some((header, records)) = parsed_file else {
        return;
    };
    let Some(namespace) = header.effective_namespace() else {
        return;
    };
    let by_lang = symdex.entry(namespace.to_string()).or_default();
    let source_path = path.to_str().unwrap();
    for record in records {
        // Records are on their parent symbol too; see `process_source_tree_changes`.
        let parent = record.pretty.rsplit_once("::").map(|(p, _)| p);
        for pretty in std::iter::once(record.pretty.as_str()).chain(parent) {
            let sym_notes = by_lang.entry(pretty.to_string()).or_default();
            sym_notes.files_to_filter.insert(path.to_path_buf());
            if first_parent {
                sym_notes.old_records.push(SymdexRecord {
                    file_row: record.clone(),
                    path: source_path.to_string(),
                });
            }
        }
    }
}

/// Note the symdex records of the first parent's versions of the files at
/// `path` in its source tree (a file or a directory), which are being
/// removed, so that they're filtered out of their symbols' symdex files.  This
/// must happen before the derived entries are deleted from the commit being
/// written.
fn note_removed_records(
    import_helper: &mut Child,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    git_repo: &git2::Repository,
    entry: &git2::TreeEntry,
    path: &Path,
) {
    match entry.kind() {
        Some(ObjectType::Blob) => {
            note_old_records(import_helper, symdex, ReadFrom::Active, path, true);
        }
        Some(ObjectType::Tree) => {
            let Ok(tree) = entry.to_object(git_repo).and_then(|o| o.peel_to_tree()) else {
                return;
            };
            let mut files = vec![];
            tree.walk(TreeWalkMode::PreOrder, |root, child| {
                if child.kind() == Some(ObjectType::Blob) {
                    files.push(path.join(root).join(child.name().unwrap_or_default()));
                }
                TreeWalkResult::Ok
            })
            .unwrap();
            for file in files {
                note_old_records(import_helper, symdex, ReadFrom::Active, &file, true);
            }
        }
        _ => {}
    }
}

fn delete_syntax_path(import_helper: &mut Child, tokenize_path: &Path, struct_path: &Path) {
    let stdin = import_helper.stdin.as_mut().unwrap();
    writeln!(stdin, "D {}", sanitize(tokenize_path)).unwrap();
    writeln!(stdin, "D {}", sanitize(struct_path)).unwrap();
}

/// Update the "files" and "files-struct" subtrees of the commit being written,
/// which start out as its first parent's (or empty for a root), for the
/// entries of the source tree at `path` which differ from the first parent's:
/// entries identical to another parent's (for merges) get that parent's
/// derived entries, changed files get written, changed directories get
/// recursed into, and removed entries get deleted.  `parent_trees` are the
/// parents' source trees at `path`.
fn process_source_tree_changes(
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
    let first_tree = parent_trees.first().and_then(|tree| tree.as_ref());

    // ## Entries the first parent had which are gone.
    if let Some(first_tree) = first_tree {
        for parent_entry in first_tree.iter() {
            let name = parent_entry.name().unwrap();
            if tree_at_path.get_name(name).is_none() {
                path.push(name);
                info!(" - Removing {}", path.display());
                note_removed_records(import_helper, symdex, git_repo, &parent_entry, &path);
                delete_syntax_path(
                    import_helper,
                    &files_root.join(&path),
                    &files_struct_root.join(&path),
                );
                path.pop();
            }
        }
    }

    'outer: for entry in tree_at_path.iter() {
        let entry_name = entry.name().unwrap();
        path.push(entry_name);
        let tokenize_path = files_root.join(&path);
        let struct_path = files_struct_root.join(&path);
        let forced = syntax_data.forced.contains(&path);
        let first_entry = first_tree.and_then(|tree| tree.get_name(entry_name));

        if !forced
            && first_entry
                .as_ref()
                .is_some_and(|e| e.id() == entry.id() && e.filemode() == entry.filemode())
        {
            // The commit already has the first parent's derived entries.
            path.pop();
            continue;
        }

        info!(" - Considering {}", path.display());
        if !forced {
            for (i, parent_tree) in parent_trees.iter().enumerate() {
                let Some(parent_tree) = parent_tree else {
                    continue;
                };
                if !parent_tree
                    .get_name(entry_name)
                    .is_some_and(|parent_entry| parent_entry.id() == entry.id())
                {
                    continue;
                }
                // Item at `path` is the same in the tree for `commit` as in
                // `parent_trees[i]` (apart from possibly its mode) so we can
                // propagate that parent's derived "files" and "files-struct"
                // entries.  This works for trees/blobs/everything.
                info!(
                    "  For {} with id {} propagating {} and {} from parent {}",
                    path.display(),
                    entry.id(),
                    tokenize_path.display(),
                    struct_path.display(),
                    i
                );
                let from = parent_source(syntax_parents, i);
                match read_path_oid(import_helper, from, &tokenize_path) {
                    Some(oid) => {
                        let struct_oid = read_path_oid(import_helper, from, &struct_path).unwrap();
                        let stdin = import_helper.stdin.as_mut().unwrap();
                        let mode = entry.filemode();
                        writeln!(stdin, "M {:06o} {} {}", mode, oid, sanitize(&tokenize_path))
                            .unwrap();
                        writeln!(
                            stdin,
                            "M {:06o} {} {}",
                            mode,
                            struct_oid,
                            sanitize(&struct_path)
                        )
                        .unwrap();
                    }
                    // That parent has no history for the entry, so neither
                    // do we.
                    None => delete_syntax_path(import_helper, &tokenize_path, &struct_path),
                }
                path.pop();
                continue 'outer;
            }
        }

        // An entry which was a different kind of thing in the first parent
        // (ex: a file which is now a directory) needs its old derived entries
        // removed before we write new ones.
        let first_kind = first_entry.as_ref().and_then(|e| e.kind());
        if let Some(first_entry) = &first_entry
            && first_kind != entry.kind()
        {
            note_removed_records(import_helper, symdex, git_repo, first_entry, &path);
            delete_syntax_path(import_helper, &tokenize_path, &struct_path);
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                // ## Load any old "files-struct" entries to populate SymbolNotes::files_to_filter
                for i in 0..syntax_parents.len() {
                    note_old_records(
                        import_helper,
                        symdex,
                        parent_source(syntax_parents, i),
                        &path,
                        i == 0,
                    );
                }

                // ## Process the new hypertokenized data, if any

                // For the inline data format documentation, refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Inlinedataformat
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Exactbytecountformat
                if let Some(TokenizedFile {
                    hypertokenized,
                    lang_source,
                }) = syntax_data.hypertokenized_files.get(&path)
                {
                    let import_stream = import_helper.stdin.as_mut().unwrap();
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
                            lang: Some(hypertokenized.profile.lang.to_string()),
                            namespace: Some(hypertokenized.profile.namespace.to_string()),
                            tokenizer: Some(TOKENIZER_VERSION),
                            lang_source: Some(lang_source.as_str().to_string()),
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
                        let by_lang = symdex
                            .entry(hypertokenized.profile.namespace.to_string())
                            .or_default();
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
                    if !syntax_data.skipped.contains(&path) {
                        warn!(
                            "  Did not find hypertokenized version of {}",
                            path.display()
                        );
                    }
                    // Any derived entries from the first parent are stale.
                    if first_entry.is_some() {
                        delete_syntax_path(import_helper, &tokenize_path, &struct_path);
                    }
                }
            }
            Some(ObjectType::Commit) => {
                // This is a submodule.  We don't create any entries for these
                // because we already won't have entries for things like binary
                // files.  This can be revisited in the future, but for now it
                // likely makes sense to not handle them and leave it up to the
                // normal boring "git log" functionality.  (Anything the first
                // parent had here was deleted above because its kind differed.)
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
                process_source_tree_changes(
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

/// The directories grouping a pretty identifier's symdex files: three levels
/// named by pairs of its first six characters (lowercased, and padded with "_"),
/// like `timeline_tokens::token_timeline_path`, so that a namespace's directory
/// doesn't get an entry for every top-level symbol, which git fast-import
/// handles slowly.
fn symdex_prefix_dir(pretty: &str) -> String {
    let mut prefix: Vec<char> = pretty
        .chars()
        .flat_map(char::to_lowercase)
        .take(6)
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '$' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    while prefix.len() < 6 {
        prefix.push('_');
    }
    prefix
        .chunks(2)
        .map(|pair| pair.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn test_symdex_prefix_dir() {
    assert_eq!(symdex_prefix_dir("mozilla::dom::Foo"), "mo/zi/ll");
    assert_eq!(symdex_prefix_dir("test_foo"), "te/st/_f");
    assert_eq!(symdex_prefix_dir("Ab"), "ab/__/__");
    assert_eq!(symdex_prefix_dir("../x.y"), "__/_x/_y");
}

/// Convert a pretty identifier into a relative path by turning each "::"
/// delimited segment into a path component.  Segments are escaped so they
/// can't introduce additional path components or be "." or "..", which is
/// possible for things like INI section names.
fn symdex_path_for_pretty(pretty: &str) -> String {
    pretty
        .split("::")
        .map(|segment| {
            let escaped = segment.replace('%', "%25").replace('/', "%2F");
            match escaped.as_str() {
                "" | "." | ".." => escaped.replace('.', "%2E") + "%",
                _ => escaped,
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn test_symdex_path_for_pretty() {
    assert_eq!(
        symdex_path_for_pretty("mozilla::dom::Foo"),
        "mozilla/dom/Foo"
    );
    assert_eq!(
        symdex_path_for_pretty("../x/test.html::a%20b"),
        "..%2Fx%2Ftest.html/a%2520b"
    );
    assert_eq!(symdex_path_for_pretty("a::..::b"), "a/%2E%2E%/b");
    assert_eq!(symdex_path_for_pretty("a::::b"), "a/%/b");
}

/// Process the symdex data populated by `process_source_tree_changes` by
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
/// changed.  So commits start out with their first parent's tree and we only
/// modify what changed (see `process_source_tree_changes` for "files" and
/// "files-struct"): we issue explicit "filemodify" commands for changed files
/// and "filedelete" commands for files which would have 0 records after the
/// header after being filtered.  (Although maybe we never actually want to
/// delete those?  Need to figure out how easy it is for the next stage to
/// explicitly notice the deletions.)
fn process_symdex_tree(
    symdex: HashMap<String, HashMap<String, SymbolNotes>>,
    import_helper: &mut Child,
    syntax_parents: &[SyntaxRepoCommit],
) -> Result<(), git2::Error> {
    info!("Processing symdex tree.");
    let linear = syntax_parents.len() <= 1;
    for (lang, lang_symbols) in symdex {
        info!(
            "Processing symdex lang {} with {} symbols.",
            lang,
            lang_symbols.len()
        );
        for (pretty, mut notes) in lang_symbols {
            if linear {
                notes.drop_unchanged_files();
                if notes.files_to_filter.is_empty() && notes.symdex_records.is_empty() {
                    continue;
                }
            }
            let sym_path = PathBuf::from(format!(
                "symdex/{}/{}/{}.ndjson",
                lang,
                symdex_prefix_dir(&pretty),
                symdex_path_for_pretty(&pretty)
            ));
            let mut records: Vec<SymdexRecord> = vec![];

            for i in 0..syntax_parents.len() {
                let parent_symdex_blob = match read_path_blob(
                    import_helper,
                    parent_source(syntax_parents, i),
                    &sym_path,
                ) {
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
    hypertokenized_files: HashMap<PathBuf, TokenizedFile>,

    /// Paths (and their ancestor directories) which must not be propagated
    /// from a parent even though they are unchanged because their resolved
    /// language changed.
    forced: HashSet<PathBuf>,

    /// Files we intentionally didn't tokenize.
    skipped: HashSet<PathBuf>,

    /// The id of the effective history configuration note attributes.
    hconfig: Option<Oid>,
}

/// A request for a compute thread to process a revision.
struct SyntaxJob {
    rev: Oid,
    /// The revision's parents, excluding any from before the history's start
    /// revision.
    parents: Vec<Oid>,
    /// The effective history configuration note attributes for the revision.
    note: Option<Arc<AttributeSet>>,
    /// The same for each of `parents`.
    parent_notes: Vec<Option<Arc<AttributeSet>>>,
}

/// Cache of parsed source repository `.gitattributes` by blob id.
type RepoAttributesCache = HashMap<Oid, Option<Arc<AttributeRules>>>;

fn effective_attributes(
    git_repo: &git2::Repository,
    tree: &git2::Tree,
    note: Option<Arc<AttributeSet>>,
    cache: &mut RepoAttributesCache,
) -> EffectiveAttributes {
    let repo = tree
        .get_name(".gitattributes")
        .filter(|entry| entry.kind() == Some(ObjectType::Blob))
        .and_then(|entry| {
            let id = entry.id();
            let rules = cache
                .entry(id)
                .or_insert_with(|| {
                    let blob = git_repo.find_blob(id).ok()?;
                    let rules = parse_repo_gitattributes(&String::from_utf8_lossy(blob.content()));
                    // Attributes without any `searchfox-*` rules don't matter to
                    // us, so we don't want them to be part of the identity.
                    (!rules.is_empty()).then(|| Arc::new(rules))
                })
                .clone()?;
            Some((id, rules))
        });
    EffectiveAttributes { repo, note }
}

/// If the effective attributes differ from any parent's, find the paths whose
/// resolved language changed, plus their ancestor directories.
fn find_forced_paths(
    tree: &git2::Tree,
    attrs: &EffectiveAttributes,
    parent_attrs: &[EffectiveAttributes],
) -> HashSet<PathBuf> {
    let mut forced = HashSet::new();
    let differing: Vec<&EffectiveAttributes> = parent_attrs
        .iter()
        .filter(|p| p.identity() != attrs.identity())
        .collect();
    if differing.is_empty() {
        return forced;
    }
    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() == Some(ObjectType::Blob) {
            let path = format!("{}{}", dir, entry.name().unwrap_or(""));
            let resolved = resolve_language(&path, attrs);
            if differing
                .iter()
                .any(|p| resolve_language(&path, p) != resolved)
            {
                for ancestor in Path::new(&path).ancestors() {
                    if ancestor.as_os_str().is_empty() {
                        break;
                    }
                    forced.insert(ancestor.to_path_buf());
                }
            }
        }
        TreeWalkResult::Ok
    })
    .unwrap();
    if !forced.is_empty() {
        info!(
            "  History attributes changed; re-tokenizing {} paths",
            forced.len()
        );
    }
    forced
}

// Does the CPU-intensive work required for blame computation of a given revision.
// This does not mutate anything in `git_repo` and has no other dependencies, so
// it can be parallelized.
fn compute_diff_data(
    git_repo: &git2::Repository,
    job: &SyntaxJob,
    cache: &mut RepoAttributesCache,
) -> Result<SyntaxTreeData, git2::Error> {
    let commit = git_repo.find_commit(job.rev).unwrap();
    let tree = commit.tree()?;

    let attrs = effective_attributes(git_repo, &tree, job.note.clone(), cache);
    let mut parent_trees = vec![];
    let mut parent_attrs = vec![];
    for (parent, note) in job.parents.iter().zip(job.parent_notes.iter()) {
        let parent_tree = git_repo.find_commit(*parent)?.tree()?;
        parent_attrs.push(effective_attributes(
            git_repo,
            &parent_tree,
            note.clone(),
            cache,
        ));
        parent_trees.push(parent_tree);
    }
    let forced = find_forced_paths(&tree, &attrs, &parent_attrs);

    let mut ctx = ModifiedFilesContext {
        parent_trees: &parent_trees,
        attrs: &attrs,
        forced: &forced,
        to_tokenize: vec![],
        skipped: HashSet::new(),
    };
    process_modified_files(git_repo, &commit, PathBuf::new(), &mut ctx)?;
    let hypertokenized_files = tokenize_files(git_repo, ctx.to_tokenize)?;
    let skipped = ctx.skipped;

    Ok(SyntaxTreeData {
        revision: job.rev,
        hypertokenized_files,
        forced,
        skipped,
        hconfig: job.note.as_ref().map(|n| n.id),
    })
}

struct ComputeThread {
    query_tx: Sender<SyntaxJob>,
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

    fn compute(&self, job: SyntaxJob) {
        self.query_tx.send(job).unwrap();
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
    query_rx: Receiver<SyntaxJob>,
    response_tx: Sender<SyntaxTreeData>,
    git_repo_path: String,
) {
    let git_repo = Repository::open(git_repo_path).unwrap();
    let mut cache = RepoAttributesCache::new();
    while let Ok(job) = query_rx.recv() {
        let result = compute_diff_data(&git_repo, &job, &mut cache).unwrap();
        response_tx.send(result).unwrap();
    }
}

/// The note attributes `rev` inherits from notes on its ancestors, which is to
/// say the attributes of the nearest ancestor with a note providing attributes.
/// This is used for the history's start revision (and so the other revisions
/// without parents), since its ancestors aren't processed.  Nearest means an ancestor note that none of the other ancestor
/// notes descend from, which approximates first-parent inheritance.
fn attributes_inherited_by(
    git_repo: &Repository,
    config: &HistoryConfig,
    rev: Oid,
) -> Option<Arc<AttributeSet>> {
    let mut nearest: Option<(Oid, Arc<AttributeSet>)> = None;
    for note_rev in config.note_revs() {
        let Some(attrs) = config.note(note_rev).and_then(|n| n.attributes.clone()) else {
            continue;
        };
        if note_rev == rev || !git_repo.graph_descendant_of(rev, note_rev).unwrap_or(false) {
            continue;
        }
        let is_nearer = match &nearest {
            None => true,
            Some((best, _)) => git_repo
                .graph_descendant_of(note_rev, *best)
                .unwrap_or(false),
        };
        if is_nearer {
            nearest = Some((note_rev, attrs));
        }
    }
    nearest.map(|(_, attrs)| attrs)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // The syntax repo is ours and git fast-import already hashed what it wrote,
    // so libgit2 needn't hash every object it reads to check it (see
    // build-timeline-tree).
    git2::opts::strict_hash_verification(false);
    // Don't make the compute threads take turns unmapping and mapping windows
    // of pack files (see build-timeline-tree).
    unsafe { git2::opts::set_mwindow_mapped_limit(1 << 40) }.unwrap();

    let cli = Cli::parse();
    let git_repo_path = cli.git_repo_path.clone();
    let git_repo = Repository::open(&git_repo_path).unwrap();
    let blame_repo = Repository::open(&cli.syntax_repo_path).unwrap();
    let history_config = match &cli.history_config_dir {
        Some(dir) => HistoryConfig::load(Path::new(dir)).unwrap_or_else(|e| {
            error!("Unable to load history configuration: {}", e);
            std::process::exit(1);
        }),
        None => HistoryConfig::empty(),
    };
    let use_cinnabar = env::var("CINNABAR").map_or(true, |v| v != "0");
    let mut hg_helper = use_cinnabar.then(|| CinnabarBatch::git2hg(&git_repo));
    let mut old_revisions = OldRevisions::new(
        cli.old_cinnabar_repo_path.as_deref().map(Path::new),
        cli.old_revision_map.as_deref().map(Path::new),
    )
    .unwrap_or_else(|e| {
        error!("Unable to set up old revision mapping: {}", e);
        std::process::exit(1);
    });
    let blame_ref = env::var("BLAME_REF").ok().unwrap_or("HEAD".to_string());
    let commit_limit = env::var("COMMIT_LIMIT")
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);

    // The syntax repo's notes map the source revisions we've already processed
    // to their syntax commits; see `source_mapping`.
    let notes_refs = NotesRefs::from_env(&blame_repo, &blame_ref);
    let mut mapping = SourceMapping::open(&blame_repo, &notes_refs);
    require_notes_for_existing_branch(&blame_repo, &blame_ref, &notes_refs, &mapping);
    info!("Using source mapping notes {}", notes_refs.write);
    // The walk below looks up every unprocessed revision in the notes, which is
    // all of them for each chunk of a reblame, which took minutes with the
    // full firefox history's notes.
    mapping.preload(&blame_repo).unwrap();

    let head = git_repo.refname_to_id(&blame_ref).unwrap();
    let mut walk = git_repo.revwalk().unwrap();
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    walk.push(head).unwrap();
    // If the history configuration has a start revision, we ignore its
    // ancestors.  Descendants of those ancestors which aren't descendants of the
    // start revision (ex: a merged branch which forked before the start) are
    // still processed, but without their parents from before the start; see
    // below.
    if let Some(start) = history_config.start {
        if !(start == head || git_repo.graph_descendant_of(head, start).unwrap_or(false)) {
            error!(
                "The history start revision {} isn't an ancestor of {}",
                start, blame_ref
            );
            std::process::exit(1);
        }
        info!("History starts at {}", start);
        for parent in git_repo.find_commit(start).unwrap().parent_ids() {
            walk.hide(parent).unwrap();
        }
    }
    // We also hide the revisions we've already processed (and so their
    // ancestors), recording them as we go.  These are the processed parents of
    // the revisions we walk (plus the head if it has been processed).
    let mut processed: HashMap<Oid, HistorySyntaxCommitMeta> = HashMap::new();
    let mut hide_processed = |rev: Oid| {
        let meta = mapping
            .lookup(&blame_repo, rev)
            .and_then(|syntax_rev| blame_repo.find_commit(syntax_rev).ok())
            .map(|commit| syntax_commit_to_meta(&commit));
        match meta {
            Some(meta) => {
                processed.insert(rev, meta);
                true
            }
            None => false,
        }
    };
    // The revisions to process (parents before children) and their parents
    // within the window, which have either been processed or are to be
    // processed.
    let mut walked: Vec<Oid> = walk
        .with_hide_callback(&mut hide_processed)
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // Revisions whose parents are all from before the start revision (ex: the
    // first revision of a branch which forked before the start) are derived
    // from the start revision instead, as if it were their parent, rather than
    // being roots which add every file.  That's much cheaper for the history
    // tools and attributes only their differences from the start to them.  So
    // the start revision goes first if it's to be processed (which keeps the
    // order topological, since it has no parents in the window), and otherwise
    // we need its syntax commit.
    if let Some(start) = history_config.start
        && let Some(pos) = walked.iter().position(|rev| *rev == start)
    {
        walked.remove(pos);
        walked.insert(0, start);
    }
    let to_process: HashSet<Oid> = walked.iter().copied().collect();
    let all_revs = walked
        .into_iter()
        .map(|oid| {
            let commit = git_repo.find_commit(oid).unwrap();
            let mut parents: Vec<Oid> = commit
                .parent_ids()
                .filter(|p| to_process.contains(p) || processed.contains_key(p))
                .collect();
            if let Some(start) = history_config.start
                && oid != start
                && parents.is_empty()
                && commit.parent_count() > 0
            {
                parents.push(start);
            }
            (oid, parents)
        })
        .collect::<Vec<_>>();
    if let Some(start) = history_config.start
        && !to_process.contains(&start)
        && !processed.contains_key(&start)
        && all_revs.iter().any(|(_, parents)| parents.contains(&start))
    {
        let meta = mapping
            .lookup(&blame_repo, start)
            .and_then(|syntax_rev| blame_repo.find_commit(syntax_rev).ok())
            .map(|commit| syntax_commit_to_meta(&commit))
            .expect("the start revision is processed before the revisions derived from it");
        processed.insert(start, meta);
    }
    info!(
        "{} revisions to process, building on {} processed revisions",
        all_revs.len(),
        processed.len()
    );
    if all_revs.is_empty()
        && let Some(meta) = processed.get(&head)
    {
        point_branch_at(&blame_repo, &blame_ref, meta.syntax_rev);
    }

    // ## Refuse to proceed if the history configuration changed for already
    // processed revisions, because then the history needs to be regenerated.
    // See "Determinism" in `hyperblame::history_config`.  This also determines
    // the effective note attributes of the processed parents.
    let mut processed_attributes = HashMap::new();
    for (oid, meta) in &processed {
        if meta.hstart != history_config.start {
            error!(
                "Already processed revision {} was derived with history start {:?}, but \
                 the start is now {:?}.  The syntax history needs to be regenerated.",
                oid, meta.hstart, history_config.start
            );
            std::process::exit(1);
        }
        let attributes = match meta.hconfig {
            Some(id) => match history_config.attributes_by_id(id) {
                Some(attributes) => Some(attributes),
                None => {
                    error!(
                        "Already processed revision {} was derived with history note \
                         attributes {} which are no longer in the configuration.  The syntax \
                         history needs to be regenerated from where they were introduced.",
                        oid, id
                    );
                    std::process::exit(1);
                }
            },
            None => None,
        };
        processed_attributes.insert(*oid, attributes);
    }
    for rev in history_config.note_revs() {
        match mapping.lookup(&blame_repo, rev) {
            Some(syntax_rev) => {
                let Some(attributes) = history_config.note(rev).unwrap().attributes.as_ref() else {
                    continue;
                };
                let recorded =
                    syntax_commit_to_meta(&blame_repo.find_commit(syntax_rev).unwrap()).hconfig;
                if recorded != Some(attributes.id) {
                    error!(
                        "The history configuration for already processed revision {} changed \
                         (recorded {:?}, now {:?}).  The syntax history needs to be regenerated \
                         from that revision.",
                        rev, recorded, attributes.id
                    );
                    std::process::exit(1);
                }
            }
            None => {
                // Notes on ancestors of the start revision can be inherited by it.
                let before_start = history_config
                    .start
                    .is_some_and(|start| git_repo.graph_descendant_of(start, rev).unwrap_or(false));
                if !to_process.contains(&rev) && !before_start {
                    warn!(
                        "History config note for {} which is not in the history",
                        rev
                    );
                }
            }
        }
    }

    // ## Determine the effective history configuration for every revision.
    let effective_notes = history_config.effective_attributes(
        &all_revs
            .iter()
            .map(|(oid, parents)| (*oid, parents.first().copied()))
            .collect::<Vec<_>>(),
        processed_attributes,
        // Revisions without parents (the start revision, and the roots of
        // unrelated histories in the window) get the attributes in effect at
        // the start revision, including from a note on the start revision
        // itself.
        history_config.start.and_then(|start| {
            history_config
                .note(start)
                .and_then(|note| note.attributes.clone())
                .or_else(|| attributes_inherited_by(&git_repo, &history_config, start))
        }),
    );
    let mut blame_map: HashMap<Oid, SyntaxRepoCommit> = processed
        .iter()
        .map(|(oid, meta)| (*oid, SyntaxRepoCommit::Commit(meta.syntax_rev)))
        .collect();

    let mut revs_to_process = all_revs.iter().map(|(oid, _)| *oid).collect::<Vec<_>>();
    let parents_of: HashMap<Oid, &Vec<Oid>> = all_revs.iter().map(|(o, p)| (*o, p)).collect();
    let make_job = |rev: Oid| SyntaxJob {
        rev,
        parents: parents_of[&rev].clone(),
        note: effective_notes[&rev].clone(),
        parent_notes: parents_of[&rev]
            .iter()
            .map(|p| effective_notes[p].clone())
            .collect(),
    };
    if commit_limit > 0 && commit_limit < revs_to_process.len() {
        info!(
            "Truncating list of commits from {} to specified limit {}",
            revs_to_process.len(),
            commit_limit
        );
        revs_to_process.truncate(commit_limit);
    }
    let rev_count = revs_to_process.len();

    let num_threads = history_compute_threads();
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
        thread.compute(make_job(revs_to_process[compute_index]));
        compute_index += 1;
    }

    // We should have sent an equal number of requests to each thread, except
    // if we ran out of requests because there were so few.
    assert!((compute_index % num_threads == 0) || compute_index == rev_count);

    let mut import_helper = start_fast_import(&blame_repo);
    let mut notes = notes_writer(&blame_repo, &notes_refs);

    // Tracks completion count and serves as the basis for the mark <idnum>
    // assigned to each commit.
    let mut rev_done = 0;

    let mut stopped = false;
    for git_oid in revs_to_process.iter() {
        if stop_requested() {
            info!("Stopping after {} revisions, as requested", rev_done);
            stopped = true;
            break;
        }
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
            thread.compute(make_job(revs_to_process[compute_index]));
            compute_index += 1;
        }

        rev_done += 1;

        let hg_rev = hg_helper
            .as_mut()
            .and_then(|helper| helper.lookup(&git_oid.to_string()));
        let oldrevs = old_revisions.lookup(*git_oid, hg_rev.as_deref());

        info!(
            "Transforming {} (hg {:?}) progress {}/{}",
            git_oid, hg_rev, rev_done, rev_count
        );
        let commit = git_repo.find_commit(*git_oid).unwrap();
        let parents = parents_of[git_oid];
        let parent_trees = parents
            .iter()
            .map(|pid| Some(git_repo.find_commit(*pid).unwrap().tree().unwrap()))
            .collect::<Vec<_>>();
        let blame_parents = parents.iter().map(|pid| blame_map[pid]).collect::<Vec<_>>();

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

            let mut commit_msg = if let Some(hg_rev) = hg_rev {
                format!("git {}\nhg {}\n", git_oid, hg_rev)
            } else {
                format!("git {}\n", git_oid)
            };
            if let Some(oldrevs) = &oldrevs {
                commit_msg.push_str(&format!("oldrevs {}\n", oldrevs));
            }
            if let Some(hconfig) = diff_data.hconfig {
                commit_msg.push_str(&format!("hconfig {}\n", hconfig));
            }
            if let Some(start) = history_config.start {
                commit_msg.push_str(&format!("hstart {}\n", start));
            }

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
            // In a change from "build-blame.rs", we don't use "deleteall": the
            // commit starts out with the first parent's tree and we only modify
            // what changed, which is much less work for git-fast-import.
            import_stream.flush().unwrap();
        }

        // Keying:
        // - namespace ("cpp", "rust", etc.) of the `LanguageProfile` used
        // - "pretty" symbol identifier
        let mut symdex: HashMap<String, HashMap<String, SymbolNotes>> = HashMap::new();

        process_source_tree_changes(
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

        // Terminate the commit so we can get its oid for the notes.
        writeln!(import_helper.stdin.as_mut().unwrap()).unwrap();
        let syntax_rev = read_mark_oid(&mut import_helper, rev_done);
        notes.add(*git_oid, &syntax_rev, commit.committer().when().seconds());
        if notes.num_pending() >= NOTES_BATCH_SIZE {
            notes.flush(import_helper.stdin.as_mut().unwrap()).unwrap();
        }

        if rev_done % 100000 == 0 {
            info!("Completed 100,000 commits, issuing checkpoint...");
            writeln!(import_helper.stdin.as_mut().unwrap(), "checkpoint").unwrap();
        }
    }

    notes.flush(import_helper.stdin.as_mut().unwrap()).unwrap();
    drop(hg_helper);

    info!("Shutting down fast-import...");
    let exitcode = import_helper.wait().unwrap();
    if exitcode.success() {
        info!("Done!");
    } else {
        info!("Fast-import exited with {:?}", exitcode.code());
    }
    if stopped {
        std::process::exit(STOPPED_EXIT_CODE);
    }
}
