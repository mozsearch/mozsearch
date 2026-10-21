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
//
// We read what the commit being written has (ex: its first parent's symdex
// files) from what we wrote and read, or from a commit on disk, rather than
// through git fast-import where we can (see `ActiveCache`);
// `SYNTAX_CACHE_CHECK=1` checks those against git fast-import's answers.

extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate num_cpus;
extern crate tools;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::env;
use std::fmt;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use clap::Parser;
use git2::{ObjectType, Oid, Repository, Sort, TreeWalkMode, TreeWalkResult};
use tools::cinnabar::{
    CinnabarBatch, OldRevisions, old_revision_notes_writer, parse_oldrevs, write_old_revision_notes,
};
use tools::fast_import::{FastImportInput, FastImportOutput};
use tools::file_format::config::{HistorySyntaxCommitMeta, index_blame, syntax_commit_to_meta};
use tools::file_format::history::io_helpers::{
    read_record_file_contents, record_file_contents_to_string,
};
use tools::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use tools::file_format::history::syntax_symdex::{SymdexHeader, SymdexRecord};
use tools::git_notes::{NotesReader, NotesWriter};
use tools::git_ops::{fast_import_git, history_compute_threads};
use tools::history_stop::{STOPPED_EXIT_CODE, stop_requested};
use tools::hyperblame::history_config::{
    AttributeRules, AttributeSet, EffectiveAttributes, FileOptions, HistoryConfig, LangSource,
    ResolvedLanguage, parse_repo_gitattributes, resolve_language,
};
use tools::source_mapping::{
    NOTES_BATCH_SIZE, NotesRefs, SourceMapping, notes_writer, point_branch_at,
    require_notes_for_existing_branch,
};
use tools::tree_sitter_support::cst_tokenizer::{
    ContextMode, HyperTokenized, LanguageProfile, TOKENIZER_VERSION, hypertokenize_with_options,
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

/// git fast-import writing the syntax repo, through threads which write what we
/// give it and read its answers (see `tools::fast_import`), so that it takes in
/// what we wrote while we go on, and the marks of the commits whose ids we've
/// asked for.  (Writing to its pipe ourselves, and waiting for each commit's
/// id, the main thread and git fast-import took turns: both were busy only a
/// fifth of the time.)
struct SyntaxImport {
    child: Child,
    input: FastImportInput,
    output: FastImportOutput,
    /// The marks we've asked for the commit ids of (see `request_mark`) whose
    /// answers we haven't read yet, in order.
    unread_marks: VecDeque<usize>,
    /// The hex ids of the commits this run wrote whose answers we've read.
    mark_revs: HashMap<usize, String>,
    /// What we know of the commit being written's paths without asking git
    /// fast-import.
    cache: ActiveCache,
    /// The syntax repo on disk, for reading `cache.base`'s paths.
    disk: Repository,
    /// Whether to check what the cache gives against git fast-import's answers
    /// (`SYNTAX_CACHE_CHECK=1`).
    check_cache: bool,
}

impl SyntaxImport {
    fn start(git_repo: &Repository) -> Self {
        let mut child = start_fast_import(git_repo);
        let input = FastImportInput::new(child.stdin.take().unwrap());
        let output = FastImportOutput::new(child.stdout.take().unwrap());
        SyntaxImport {
            child,
            input,
            output,
            unread_marks: VecDeque::new(),
            mark_revs: HashMap::new(),
            cache: ActiveCache::default(),
            disk: Repository::open(git_repo.path()).unwrap(),
            check_cache: env::var("SYNTAX_CACHE_CHECK").is_ok_and(|v| v == "1"),
        }
    }

    /// Start writing a commit whose first parent is `parent` (None for a
    /// root): what the cache knows of the paths carries over if that's the
    /// commit we just wrote, and otherwise it starts over from `parent`, which
    /// is on disk once we checkpoint if we wrote it.
    fn start_commit(&mut self, parent: Option<SyntaxRepoCommit>) {
        if self.cache.parent == parent && parent.is_some() {
            return;
        }
        let base = match parent {
            Some(SyntaxRepoCommit::Commit(oid)) => Some(oid),
            Some(SyntaxRepoCommit::Mark(mark)) => self.checkpoint(mark),
            None => None,
        };
        self.cache.rebase(parent, base, &self.disk);
    }

    /// Checkpoint, so that the commit with `mark` (which was requested) is on
    /// disk, and give its id.
    fn checkpoint(&mut self, mark: usize) -> Option<Oid> {
        writeln!(self.input, "checkpoint").unwrap();
        self.cache.checkpoints += 1;
        // (Asking for the commit's id again, whose answer comes once git
        // fast-import has done the checkpoint, unlike the answer to the
        // request when the commit was written.)
        self.request_mark(mark);
        self.output();
        let rev = self.mark_rev(mark, true)?;
        Oid::from_str(&rev).ok()
    }

    /// Finish writing the commit with `mark` (whose id has been requested).
    fn finish_commit(&mut self, mark: usize) {
        self.cache.finish_commit(SyntaxRepoCommit::Mark(mark));
        if self.cache.bytes > CACHE_BYTES {
            // (Starting over from the commit we just wrote, on disk.)
            let base = self.checkpoint(mark);
            self.cache.clears += 1;
            self.cache
                .rebase(Some(SyntaxRepoCommit::Mark(mark)), base, &self.disk);
        }
    }

    /// Write `contents` to `path` in the commit being written, with `mode`.
    fn write_inline(&mut self, mode: i32, path: &Path, contents: &[u8]) {
        writeln!(self.input, "M {:06o} inline {}", mode, sanitize(path)).unwrap();
        writeln!(self.input, "data {}", contents.len()).unwrap();
        self.input.write_all(contents).unwrap();
        // (We skip the optional trailing LF character, since in practice it
        // wasn't useful for debugging, and the blobs we write always have
        // one anyway.)
        let change = if is_read_back(path) {
            Change::Write(contents.into())
        } else {
            Change::Unknown
        };
        self.cache.change(path, change);
    }

    /// Write the object `oid` to `path` in the commit being written, with
    /// `mode` (a tree, for a directory).
    fn write_existing(&mut self, mode: i32, oid: &str, path: &Path) {
        writeln!(self.input, "M {:06o} {} {}", mode, oid, sanitize(path)).unwrap();
        let change = if mode == 0o040000 {
            Change::UnknownTree
        } else {
            Change::Unknown
        };
        self.cache.change(path, change);
    }

    /// Delete `path` from the commit being written: a file, or if `tree`, a
    /// directory (or whatever it is).
    fn delete(&mut self, path: &Path, tree: bool) {
        writeln!(self.input, "D {}", sanitize(path)).unwrap();
        let change = if tree {
            Change::DeleteTree
        } else {
            Change::Delete
        };
        self.cache.change(path, change);
    }

    /// git fast-import's output, for reading its answers to the requests we've
    /// written, which this sends on to it first, after any answers to
    /// `request_mark` before them.
    fn output(&mut self) -> &mut FastImportOutput {
        self.input.flush().unwrap();
        while let Some(mark) = self.unread_marks.pop_front() {
            let mut line = String::new();
            self.output.read_line(&mut line).unwrap();
            self.mark_revs.insert(mark, line.trim().to_string());
        }
        &mut self.output
    }

    /// Ask for the id of the commit with `mark`, which must have been
    /// terminated, without waiting for git fast-import to write it, which it
    /// does only once it has processed everything before.
    fn request_mark(&mut self, mark: usize) {
        writeln!(self.input, "get-mark :{}", mark).unwrap();
        // (So that it can answer while we go on.)
        self.input.flush().unwrap();
        self.unread_marks.push_back(mark);
    }

    /// The hex id of the commit with `mark`, which must have been requested,
    /// if we have it without waiting for git fast-import (unless `wait`).
    fn mark_rev(&mut self, mark: usize, wait: bool) -> Option<String> {
        if !self.mark_revs.contains_key(&mark) {
            if wait {
                self.output();
            } else {
                self.output.take_available();
                while !self.unread_marks.is_empty() && self.output.has_line() {
                    let mut line = String::new();
                    self.output.read_line(&mut line).unwrap();
                    let answered = self.unread_marks.pop_front().unwrap();
                    self.mark_revs.insert(answered, line.trim().to_string());
                }
            }
        }
        self.mark_revs.get(&mark).cloned()
    }

    /// Close git fast-import's input once it has everything we wrote, and wait
    /// for it to exit.
    fn finish(&mut self) -> ExitStatus {
        self.input.finish().unwrap();
        self.child.wait().unwrap()
    }
}

/// The most bytes of contents `ActiveCache` keeps before starting over.
const CACHE_BYTES: usize = 4 << 30;

/// Whether we read `path` back from the commit being written (its
/// "files-struct" entries and symdex files, but not its "files" entries),
/// so that the cache should keep its contents.
fn is_read_back(path: &Path) -> bool {
    path.starts_with("files-struct") || path.starts_with("symdex")
}

/// A change to the commit being written.
#[derive(Clone)]
enum Change {
    /// A blob with these contents was written to the path.
    Write(Rc<[u8]>),
    /// An existing blob was written to the path.
    Unknown,
    /// An existing tree was written to the path, replacing what was under it.
    UnknownTree,
    /// The file at the path was deleted.
    Delete,
    /// The path, and anything under it, was deleted.
    DeleteTree,
}

impl Change {
    fn is_tree(&self) -> bool {
        matches!(self, Change::UnknownTree | Change::DeleteTree)
    }
}

/// What a path of the commit being written has, as far as `ActiveCache` knows.
enum Known {
    /// These contents, or nothing.
    Contents(Option<Rc<[u8]>>),
    Unknown,
}

/// A path's bytes, which `ActiveCache` keys paths by: our paths are joined
/// with "/" (and have no "." or ".." components), so comparing bytes is
/// comparing paths, and a directory's descendants are the keys between "dir/"
/// and "dir0" ("/" + 1), which comparing `Path`s' components took most of the
/// main thread's time doing.
type PathKey = Box<[u8]>;

fn path_key(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

/// The path itself and its ancestor directories, innermost first.
fn ancestor_keys(key: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::once(key).chain(
        key.iter()
            .enumerate()
            .rev()
            .filter(|&(_, &b)| b == b'/')
            .map(move |(i, _)| &key[..i]),
    )
}

/// What we know of the paths of the commit being written without asking git
/// fast-import, which answers each read only after it has processed everything
/// we sent before it, making us and it take turns.  (Reading each changed
/// file's "files-struct" entry, and each changed symbol's symdex file, through
/// it, the main thread spent over a quarter of its time waiting on it.)  That's
/// the first parent's paths which we read or wrote, carried over from each
/// commit to its child when that's the next commit we write, as it almost
/// always is, and the commit's changes; and for the paths not changed since
/// it, `base`, a commit on disk which the first parent descends from through
/// commits we wrote, whose versions of them the first parent has.
#[derive(Default)]
struct ActiveCache {
    /// The commit being written's first parent, which `entries` are of.
    parent: Option<SyntaxRepoCommit>,
    /// Paths' contents (None if missing) in `parent`.
    entries: BTreeMap<PathKey, Option<Rc<[u8]>>>,
    /// The bytes of contents in `entries`.
    bytes: usize,
    /// The commit's changes, in order, and the latest change of each path and
    /// of each tree (by their indexes).
    changes: Vec<(PathKey, Change)>,
    path_changes: HashMap<PathKey, usize>,
    tree_changes: HashMap<PathKey, usize>,
    /// The commit on disk, and its tree, whose versions `parent` has of the
    /// paths which neither `changed` nor `changed_trees` (with their
    /// ancestors) have.
    base: Option<Oid>,
    base_tree: Option<Oid>,
    changed: HashSet<PathKey>,
    changed_trees: HashSet<PathKey>,
    hits: usize,
    disk_reads: usize,
    fast_import_reads: usize,
    checkpoints: usize,
    clears: usize,
}

impl ActiveCache {
    /// Start over from `parent`, whose paths are `base`'s.
    fn rebase(&mut self, parent: Option<SyntaxRepoCommit>, base: Option<Oid>, disk: &Repository) {
        self.parent = parent;
        self.entries.clear();
        self.bytes = 0;
        self.base = base;
        self.base_tree = base.and_then(|oid| Some(disk.find_commit(oid).ok()?.tree_id()));
        self.changed.clear();
        self.changed_trees.clear();
    }

    fn change(&mut self, path: &Path, change: Change) {
        let index = self.changes.len();
        let key: PathKey = path_key(path).into();
        if change.is_tree() {
            self.tree_changes.insert(key.clone(), index);
        } else {
            self.path_changes.insert(key.clone(), index);
        }
        self.changes.push((key, change));
    }

    /// The latest change of `path` in the commit being written, if any.
    fn latest_change(&self, path: &Path) -> Option<&Change> {
        let key = path_key(path);
        let exact = self.path_changes.get(key).copied();
        let tree = if self.tree_changes.is_empty() {
            None
        } else {
            ancestor_keys(key)
                .filter_map(|a| self.tree_changes.get(a).copied())
                .max()
        };
        let latest = exact.max(tree)?;
        Some(&self.changes[latest].1)
    }

    /// Whether `path` may differ from `base`'s version in `parent`.
    fn changed_since_base(&self, key: &[u8]) -> bool {
        self.changed.contains(key)
            || (!self.changed_trees.is_empty()
                && ancestor_keys(key).any(|a| self.changed_trees.contains(a)))
    }

    /// What `path` has in the commit being written, as far as we know.
    fn known(&mut self, path: &Path, disk: &Repository) -> Known {
        match self.latest_change(path) {
            Some(Change::Write(contents)) => return Known::Contents(Some(contents.clone())),
            Some(Change::Delete | Change::DeleteTree) => return Known::Contents(None),
            Some(Change::Unknown | Change::UnknownTree) => return Known::Unknown,
            None => {}
        }
        let key = path_key(path);
        if let Some(contents) = self.entries.get(key) {
            self.hits += 1;
            return Known::Contents(contents.clone());
        }
        if let Some(tree) = self.base_tree
            && !self.changed_since_base(key)
            && let Some(contents) = read_tree_path(disk, tree, path)
        {
            self.disk_reads += 1;
            self.remember(path, contents.clone());
            return Known::Contents(contents);
        }
        Known::Unknown
    }

    /// Remember `contents` as the parent's version of `path`, which the
    /// commit being written hasn't changed.
    fn remember(&mut self, path: &Path, contents: Option<Rc<[u8]>>) {
        self.remember_key(path_key(path).into(), contents);
    }

    fn remember_key(&mut self, key: PathKey, contents: Option<Rc<[u8]>>) {
        self.bytes += contents.as_ref().map_or(0, |c| c.len());
        if let Some(Some(old)) = self.entries.insert(key, contents) {
            self.bytes -= old.len();
        }
    }

    fn forget(&mut self, key: &[u8]) {
        if let Some(Some(old)) = self.entries.remove(key) {
            self.bytes -= old.len();
        }
    }

    /// The commit being written, `commit`, is finished: it's the parent of the
    /// next one, if that's the next one we write.
    fn finish_commit(&mut self, commit: SyntaxRepoCommit) {
        for (key, change) in std::mem::take(&mut self.changes) {
            match change {
                Change::Write(contents) => {
                    self.changed.insert(key.clone());
                    self.remember_key(key, Some(contents));
                }
                Change::Delete => {
                    self.changed.insert(key.clone());
                    self.remember_key(key, None);
                }
                Change::Unknown => {
                    self.forget(&key);
                    self.changed.insert(key);
                }
                Change::UnknownTree | Change::DeleteTree => {
                    // (The path, and the keys from "path/" to "path0".)
                    self.forget(&key);
                    let mut start = key.to_vec();
                    start.push(b'/');
                    let mut end = key.to_vec();
                    end.push(b'0');
                    let under: Vec<PathKey> = self
                        .entries
                        .range::<[u8], _>((
                            std::ops::Bound::Included(&start[..]),
                            std::ops::Bound::Excluded(&end[..]),
                        ))
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in under {
                        self.forget(&k);
                    }
                    self.changed_trees.insert(key);
                }
            }
        }
        self.path_changes.clear();
        self.tree_changes.clear();
        self.parent = Some(commit);
    }
}

/// The contents of the blob at `path` in the tree `tree` on disk (None if
/// there's none), unless we can't tell (ex: it's a directory).
fn read_tree_path(disk: &Repository, tree: Oid, path: &Path) -> Option<Option<Rc<[u8]>>> {
    let tree = disk.find_tree(tree).ok()?;
    match tree.get_path(path) {
        Ok(entry) if entry.kind() == Some(ObjectType::Blob) => {
            let blob = disk.find_blob(entry.id()).ok()?;
            Some(Some(blob.content().into()))
        }
        Ok(_) => None,
        Err(e) if e.code() == git2::ErrorCode::NotFound => Some(None),
        Err(_) => None,
    }
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
        // Objects aren't looked up in the repo's packs before they're stored;
        // see build-timeline-tree.
        .arg("-c")
        .arg("fastimport.lookupPacks=false")
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
fn read_path_oid(import_helper: &mut SyntaxImport, from: ReadFrom, path: &Path) -> Option<String> {
    let stdin = &mut import_helper.input;
    match from {
        ReadFrom::Active => writeln!(stdin, "ls {}", quote_path(path)),
        ReadFrom::Commit(commit) => writeln!(stdin, "ls {} {}", commit, sanitize(path)),
    }
    .unwrap();
    let reader = import_helper.output();
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
    import_helper: &mut SyntaxImport,
    from: ReadFrom,
    path: &Path,
) -> Option<Vec<u8>> {
    let ReadFrom::Active = from else {
        return read_fast_import_blob(import_helper, from, path);
    };
    let known = import_helper.cache.known(path, &import_helper.disk);
    if let Known::Contents(contents) = known {
        let contents = contents.map(|c| c.to_vec());
        if import_helper.check_cache {
            assert_eq!(
                read_fast_import_blob(import_helper, from, path),
                contents,
                "the cache's {}",
                path.display()
            );
        }
        return contents;
    }
    import_helper.cache.fast_import_reads += 1;
    let contents = read_fast_import_blob(import_helper, from, path);
    // (The parent's version, if the commit being written hasn't changed it.)
    if import_helper.cache.latest_change(path).is_none() {
        import_helper
            .cache
            .remember(path, contents.as_deref().map(Rc::from));
    }
    contents
}

/// Read the contents of the object at `path` in `from` through git
/// fast-import.
fn read_fast_import_blob(
    import_helper: &mut SyntaxImport,
    from: ReadFrom,
    path: &Path,
) -> Option<Vec<u8>> {
    let oid = read_path_oid(import_helper, from, path)?;
    writeln!(import_helper.input, "cat-blob {}", oid).unwrap();
    let reader = import_helper.output();
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
    options: FileOptions,
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
    to_tokenize: Vec<(PathBuf, Oid, LanguageProfile, LangSource, FileOptions)>,
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
    to_tokenize: Vec<(PathBuf, Oid, LanguageProfile, LangSource, FileOptions)>,
) -> Result<HashMap<PathBuf, TokenizedFile>, git2::Error> {
    let mut inputs = Vec::with_capacity(to_tokenize.len());
    for (path, oid, profile, lang_source, options) in to_tokenize {
        let content = git_repo.find_blob(oid)?.content().to_vec();
        inputs.push((path, content, profile, lang_source, options));
    }
    #[allow(clippy::type_complexity)]
    let tokenize = |(path, content, profile, lang_source, options): (
        PathBuf,
        Vec<u8>,
        LanguageProfile,
        LangSource,
        FileOptions,
    )| {
        let text = std::str::from_utf8(&content).ok()?;
        let hypertokenized = hypertokenize_with_options(profile, options.tokenize, text).ok()?;
        Some((
            path,
            TokenizedFile {
                hypertokenized,
                lang_source,
                options,
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
                    ResolvedLanguage::Tokenize(profile, lang_source, options) => {
                        ctx.to_tokenize.push((
                            path.clone(),
                            entry.id(),
                            profile,
                            lang_source,
                            options,
                        ));
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
    /// the "mozilla" namespace is in most C++ files).  (The symdex files of
    /// merges start out as their first parent's too.)
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
            pp: vec![],
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

#[test]
fn test_symdex_rows_cap() {
    let row = |i: usize| FileStructureRow {
        pretty: format!("key{}", i),
        is_def: true,
        kind: "field".to_string(),
        pp: vec![],
    };
    let mut symdex: HashMap<String, HashMap<String, SymbolNotes>> = HashMap::new();
    let rows: Vec<FileStructureRow> = (0..=MAX_SYMDEX_ROWS).map(row).collect();
    add_symdex_records(&mut symdex, "json", &rows, Path::new("big.json"));
    assert!(symdex.get("json").is_none_or(|symbols| symbols.is_empty()));
    add_symdex_records(&mut symdex, "json", &rows[..2], Path::new("small.json"));
    assert_eq!(symdex["json"].len(), 2);
}

/// Where to read the syntax repo data corresponding to `parent_idx`'s tree.
fn parent_source(syntax_parents: &[SyntaxRepoCommit], parent_idx: usize) -> ReadFrom<'_> {
    if parent_idx == 0 {
        ReadFrom::Active
    } else {
        ReadFrom::Commit(&syntax_parents[parent_idx])
    }
}

/// The most structure rows a file can have and get symdex records for its
/// symbols.  A file with more is data (ex: TC39 Temporal's polyfill's test
/// snapshots, JSON files with up to ~600k keys), whose symdex files (one per
/// symbol, each read and written through git-fast-import) took the syntax over
/// an hour per revision, and whose symbols few would look up by name.  It
/// keeps its tokens and its structure, just not symdex records.  (Firefox's
/// largest structures are ~50k rows, ex: WebGPU CTS expectations' and
/// perfetto.h's.)
const MAX_SYMDEX_ROWS: usize = 100_000;

/// Note the symdex records of the first parent's version of the source file at
/// `path` (from its "files-struct" entry), so that they're filtered out of
/// their symbols' symdex files, which start out as the first parent's, and so
/// that `SymbolNotes::drop_unchanged_files` can compare them with the new
/// records.  This reads from the commit being written, which gives the first
/// parent's version as long as the entry hasn't been modified yet.
fn note_old_records(
    import_helper: &mut SyntaxImport,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    path: &Path,
) {
    let struct_path = PathBuf::from("files-struct").join(path);
    let Some(blob) = read_path_blob(import_helper, ReadFrom::Active, &struct_path) else {
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
    // (It has no symdex records to filter out.)
    if records.len() > MAX_SYMDEX_ROWS {
        return;
    }
    let by_lang = symdex.entry(namespace.to_string()).or_default();
    let source_path = path.to_str().unwrap();
    for record in records {
        // Records are on their parent symbol too; see `process_source_tree_changes`.
        let parent = record.pretty.rsplit_once("::").map(|(p, _)| p);
        for pretty in std::iter::once(record.pretty.as_str()).chain(parent) {
            let sym_notes = by_lang.entry(pretty.to_string()).or_default();
            sym_notes.files_to_filter.insert(path.to_path_buf());
            sym_notes.old_records.push(SymdexRecord {
                file_row: record.clone(),
                path: source_path.to_string(),
            });
        }
    }
}

/// Add the symdex records of the rows of the file at `path` (whose language's
/// namespace is `namespace`), for writing into their symbols' symdex files.
fn add_symdex_records(
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    namespace: &str,
    rows: &[FileStructureRow],
    path: &Path,
) {
    if rows.is_empty() || rows.len() > MAX_SYMDEX_ROWS {
        return;
    }
    let by_lang = symdex.entry(namespace.to_string()).or_default();
    let source_path = path.to_str().unwrap();
    for record in rows {
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

/// For a merge whose entry at `path` (a file or a directory) is another
/// parent's (`from`), whose derived entries it gets, note the first parent's
/// versions' symdex records for filtering, and add the other parent's
/// versions' records, of the files which differ from the first parent's:
/// the merge's symdex starts out as its first parent's.  This must happen
/// before the entry is written, while reading from the commit being written
/// gives the first parent's versions.
fn note_propagated_records(
    import_helper: &mut SyntaxImport,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    git_repo: &git2::Repository,
    first_entry: Option<&git2::TreeEntry>,
    entry: &git2::TreeEntry,
    from: ReadFrom,
    path: &Path,
) {
    // The other parent's version of a file: the first parent's records go,
    // and its come.
    fn propagate_file(
        import_helper: &mut SyntaxImport,
        symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
        from: ReadFrom,
        file: &Path,
    ) {
        note_old_records(import_helper, symdex, file);
        let struct_path = PathBuf::from("files-struct").join(file);
        let Some(blob) = read_path_blob(import_helper, from, &struct_path) else {
            return;
        };
        let parsed: Option<(FileStructureHeader, Vec<FileStructureRow>)> =
            read_record_file_contents(&blob);
        if let Some((header, rows)) = parsed
            && let Some(namespace) = header.effective_namespace()
        {
            add_symdex_records(symdex, namespace, &rows, file);
        }
    }
    match entry.kind() {
        Some(ObjectType::Blob) => {
            // (A directory which is now a file.)
            if let Some(first) = first_entry
                && first.kind() == Some(ObjectType::Tree)
            {
                note_removed_records(import_helper, symdex, git_repo, first, path);
            }
            propagate_file(import_helper, symdex, from, path);
        }
        Some(ObjectType::Tree) => {
            let Ok(new_tree) = entry.to_object(git_repo).and_then(|o| o.peel_to_tree()) else {
                return;
            };
            let old_tree = first_entry
                .filter(|first| first.kind() == Some(ObjectType::Tree))
                .and_then(|first| first.to_object(git_repo).ok())
                .and_then(|o| o.into_tree().ok());
            // (A file which is now a directory.)
            if first_entry.is_some_and(|first| first.kind() == Some(ObjectType::Blob)) {
                note_old_records(import_helper, symdex, path);
            }
            let Ok(diff) = git_repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)
            else {
                return;
            };
            for delta in diff.deltas() {
                match (delta.old_file().path(), delta.new_file().path()) {
                    (_, Some(new)) if !delta.new_file().id().is_zero() => {
                        propagate_file(import_helper, symdex, from, &path.join(new));
                    }
                    (Some(old), _) => note_old_records(import_helper, symdex, &path.join(old)),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Note the symdex records of the first parent's versions of the files at
/// `path` in its source tree (a file or a directory), which are being
/// removed, so that they're filtered out of their symbols' symdex files.  This
/// must happen before the derived entries are deleted from the commit being
/// written.
fn note_removed_records(
    import_helper: &mut SyntaxImport,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    git_repo: &git2::Repository,
    entry: &git2::TreeEntry,
    path: &Path,
) {
    match entry.kind() {
        Some(ObjectType::Blob) => {
            note_old_records(import_helper, symdex, path);
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
                note_old_records(import_helper, symdex, &file);
            }
        }
        _ => {}
    }
}

/// Delete the derived entries of a source entry, which `blob` says is a file
/// (or else a directory, or whatever it is).
fn delete_syntax_path(
    import_helper: &mut SyntaxImport,
    tokenize_path: &Path,
    struct_path: &Path,
    blob: bool,
) {
    import_helper.delete(tokenize_path, !blob);
    import_helper.delete(struct_path, !blob);
}

/// Update the "files" and "files-struct" subtrees of the commit being written,
/// which start out as its first parent's (or empty for a root), for the
/// entries of the source tree at `path` which differ from the first parent's:
/// entries identical to another parent's (for merges) get that parent's
/// derived entries, changed files get written, changed directories get
/// recursed into, and removed entries get deleted.  `parent_trees` are the
/// parents' source trees at `path`.  The symdex records of the changes
/// accumulate in `symdex` for `process_symdex_tree`.
#[allow(clippy::too_many_arguments)]
fn process_source_tree_changes(
    syntax_data: &SyntaxTreeData,
    symdex: &mut HashMap<String, HashMap<String, SymbolNotes>>,
    git_repo: &git2::Repository,
    commit: &git2::Commit,
    tree_at_path: &git2::Tree,
    parent_trees: &[Option<git2::Tree>],
    import_helper: &mut SyntaxImport,
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
                    parent_entry.kind() == Some(ObjectType::Blob),
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
                // (A merge's symdex starts out as its first parent's.)
                if i > 0 {
                    note_propagated_records(
                        import_helper,
                        symdex,
                        git_repo,
                        first_entry.as_ref(),
                        &entry,
                        from,
                        &path,
                    );
                }
                match read_path_oid(import_helper, from, &tokenize_path) {
                    Some(oid) => {
                        let struct_oid = read_path_oid(import_helper, from, &struct_path).unwrap();
                        let mode = entry.filemode();
                        import_helper.write_existing(mode, &oid, &tokenize_path);
                        import_helper.write_existing(mode, &struct_oid, &struct_path);
                    }
                    // That parent has no history for the entry, so neither
                    // do we.
                    None => delete_syntax_path(
                        import_helper,
                        &tokenize_path,
                        &struct_path,
                        entry.kind() == Some(ObjectType::Blob),
                    ),
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
            delete_syntax_path(
                import_helper,
                &tokenize_path,
                &struct_path,
                first_kind == Some(ObjectType::Blob),
            );
        }

        match entry.kind() {
            Some(ObjectType::Blob) => {
                // ## Load any old "files-struct" entry to populate SymbolNotes::files_to_filter
                note_old_records(import_helper, symdex, &path);

                // ## Process the new hypertokenized data, if any

                // For the inline data format documentation, refer to
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Inlinedataformat
                // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Exactbytecountformat
                if let Some(TokenizedFile {
                    hypertokenized,
                    lang_source,
                    options,
                }) = syntax_data.hypertokenized_files.get(&path)
                {
                    info!(
                        "  Writing out {} and {} (tokens: {} structure: {})",
                        tokenize_path.display(),
                        struct_path.display(),
                        hypertokenized.tokenized.len(),
                        hypertokenized.structure.len(),
                    );

                    // ## Write the tokenized file contents
                    let tokenized_text = hypertokenized.tokenized.join("\n");
                    import_helper.write_inline(
                        entry.filemode(),
                        &tokenize_path,
                        tokenized_text.as_bytes(),
                    );

                    // ## Write the files-struct contents
                    let struct_text = record_file_contents_to_string(
                        &FileStructureHeader {
                            lang: Some(hypertokenized.profile.lang.to_string()),
                            namespace: Some(hypertokenized.profile.namespace.to_string()),
                            tokenizer: Some(TOKENIZER_VERSION),
                            lang_source: Some(lang_source.as_str().to_string()),
                            contexts: (options.tokenize.contexts != ContextMode::Default)
                                .then(|| options.tokenize.contexts.as_str().to_string()),
                            handler_moves: options.handler_moves,
                        },
                        &hypertokenized.structure,
                    );
                    import_helper.write_inline(
                        entry.filemode(),
                        &struct_path,
                        struct_text.as_bytes(),
                    );

                    // ## Accumulate the symdex data.
                    add_symdex_records(
                        symdex,
                        hypertokenized.profile.namespace,
                        &hypertokenized.structure,
                        &path,
                    );
                } else {
                    if !syntax_data.skipped.contains(&path) {
                        warn!(
                            "  Did not find hypertokenized version of {}",
                            path.display()
                        );
                    }
                    // Any derived entries from the first parent are stale.
                    // (Of a file, since a directory's were deleted above.)
                    if first_entry.is_some() {
                        delete_syntax_path(import_helper, &tokenize_path, &struct_path, true);
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

/// The most bytes a path component of a symdex file can have (before its last
/// one gets ".ndjson"); see `symdex_path_for_pretty`.
const MAX_SYMDEX_SEGMENT_BYTES: usize = 128;

/// Convert a pretty identifier into a relative path by turning each "::"
/// delimited segment into a path component.  Segments are escaped so they
/// can't introduce additional path components or be "." or "..", which is
/// possible for things like INI section names.  Segments longer than
/// `MAX_SYMDEX_SEGMENT_BYTES` are shortened to a prefix and a hash of the whole
/// segment (the records have the whole pretty identifier): file systems limit
/// names to 255 bytes, and firefox has many longer ones (ex: the section names
/// of web-platform-tests metadata for WebGPU's conformance tests, which are
/// test URLs with long queries).
fn symdex_path_for_pretty(pretty: &str) -> String {
    pretty
        .split("::")
        .map(|segment| {
            let escaped = segment.replace('%', "%25").replace('/', "%2F");
            let escaped = match escaped.as_str() {
                "" | "." | ".." => escaped.replace('.', "%2E") + "%",
                _ => escaped,
            };
            if escaped.len() <= MAX_SYMDEX_SEGMENT_BYTES {
                return escaped;
            }
            let hash = Oid::hash_object(git2::ObjectType::Blob, segment.as_bytes())
                .unwrap()
                .to_string();
            let mut end = MAX_SYMDEX_SEGMENT_BYTES - 17;
            while !escaped.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}~{}", &escaped[..end], &hash[..16])
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
    // Long segments are shortened, differently for different segments.
    let long = format!("x.html?q={}", "é".repeat(100));
    let other = format!("{}é", long);
    let path = symdex_path_for_pretty(&format!("{}::b", long));
    let (first, rest) = path.split_once('/').unwrap();
    assert_eq!(rest, "b");
    assert!(first.len() <= MAX_SYMDEX_SEGMENT_BYTES && first.starts_with("x.html?q=é"));
    assert_ne!(first, symdex_path_for_pretty(&other));
    assert_eq!(path, symdex_path_for_pretty(&format!("{}::b", long)));
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
    import_helper: &mut SyntaxImport,
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
            notes.drop_unchanged_files();
            if notes.files_to_filter.is_empty() && notes.symdex_records.is_empty() {
                continue;
            }
            let sym_path = PathBuf::from(format!(
                "symdex/{}/{}/{}.ndjson",
                lang,
                symdex_prefix_dir(&pretty),
                symdex_path_for_pretty(&pretty)
            ));
            // The symdex starts out as the first parent's, even for merges,
            // whose other parents' versions of files are in `notes` (see
            // `note_propagated_records`).
            let mut records: Vec<SymdexRecord> = vec![];
            if !syntax_parents.is_empty()
                && let Some(parent_symdex_blob) =
                    read_path_blob(import_helper, ReadFrom::Active, &sym_path)
            {
                let parsed_file: Option<(SymdexHeader, Vec<SymdexRecord>)> =
                    read_record_file_contents(&parent_symdex_blob);
                if let Some((_header, parent_records)) = parsed_file {
                    records = parent_records
                        .into_iter()
                        .filter(|rec| !notes.files_to_filter.contains(&PathBuf::from(&rec.path)))
                        .collect();
                }
            }

            records.append(&mut notes.symdex_records);
            records.sort();

            let header = SymdexHeader {};

            // Delete the file if we no longer have any records for the file.
            if records.is_empty() {
                info!("  Deleting moot symdex file {}", sym_path.display());
                import_helper.delete(&sym_path, false);
            } else {
                let symdex_text = record_file_contents_to_string(&header, &records);
                let symdex_bytes = symdex_text.as_bytes();

                info!(
                    "  Writing symdex file {} with {} entries.",
                    sym_path.display(),
                    records.len()
                );
                import_helper.write_inline(0o100644, &sym_path, symdex_bytes);
            }
        }
    }
    info!("Done processing symdex.");

    Ok(())
}

/// A processed revision whose syntax commit's id we don't have yet: the
/// revision, its syntax commit's mark, its committer time, and whether a batch
/// of notes ends with its note.
type UnnotedRevision = (Oid, usize, i64, bool);

/// Note the revisions whose syntax commits' ids we have (or all of them, if
/// `wait`), in order, writing each batch of notes once it's complete, so that
/// the batches are each NOTES_BATCH_SIZE revisions, as they were when we
/// waited for each revision's id.
fn note_revisions(
    import_helper: &mut SyntaxImport,
    notes: &mut NotesWriter,
    unnoted: &mut VecDeque<UnnotedRevision>,
    wait: bool,
) {
    while let Some(&(rev, mark, time, ends_batch)) = unnoted.front() {
        let Some(syntax_rev) = import_helper.mark_rev(mark, wait) else {
            break;
        };
        unnoted.pop_front();
        import_helper.mark_revs.remove(&mark);
        notes.add(rev, &syntax_rev, time);
        if ends_batch {
            notes.flush(&mut import_helper.input).unwrap();
        }
    }
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
    // Cache big directories' trees too (see build-timeline-tree).
    unsafe { git2::opts::set_cache_object_limit(git2::ObjectType::Tree, 16 << 20) }.unwrap();
    unsafe { git2::opts::set_cache_max_size(4 << 30) }.unwrap();

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

    // We also maintain notes mapping the old revisions of the revisions we
    // process to them, as build-blame does in the blame repo, for the web
    // server's permalinks to old revisions (see `tools::cinnabar`).  Histories
    // from before we wrote them get them for the revisions already processed
    // from a walk of the branch, if we're given old revisions.
    let mut old_notes = old_revision_notes_writer(&blame_repo, &blame_ref);
    let existing_old_notes = NotesReader::open(&blame_repo, [old_notes.notes_ref()]);
    let head_time = blame_repo
        .refname_to_id(&blame_ref)
        .and_then(|oid| blame_repo.find_commit(oid))
        .map_or(0, |commit| commit.committer().when().seconds());
    let mut seed_old_to_new = HashMap::new();
    if existing_old_notes.is_empty()
        && (cli.old_cinnabar_repo_path.is_some() || cli.old_revision_map.is_some())
        && let Ok(oid) = blame_repo.refname_to_id(&blame_ref)
    {
        info!("Seeding {} from {}...", old_notes.notes_ref(), blame_ref);
        (_, _, seed_old_to_new) = index_blame(&blame_repo, Some(oid), true);
    }

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

    let num_threads = history_compute_threads("SYNTAX_COMPUTE_THREADS");
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

    let mut import_helper = SyntaxImport::start(&blame_repo);
    let mut notes = notes_writer(&blame_repo, &notes_refs);
    if !seed_old_to_new.is_empty() {
        let num_old_notes = write_old_revision_notes(
            &blame_repo,
            &mut old_notes,
            &seed_old_to_new,
            head_time,
            &mut import_helper.input,
        )
        .unwrap();
        info!(
            "Seeded {} of {} old revision notes.",
            num_old_notes,
            seed_old_to_new.len()
        );
    }
    // The old revisions we've written notes for in this run.
    let mut old_revs_noted = HashSet::new();

    // Tracks completion count and serves as the basis for the mark <idnum>
    // assigned to each commit.
    let mut rev_done = 0;
    // The revisions processed whose syntax commits' ids we haven't had yet,
    // for their notes, in order, and how many since the last batch of notes.
    let mut unnoted: VecDeque<UnnotedRevision> = VecDeque::new();
    let mut since_batch = 0;

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

        import_helper.start_commit(blame_parents.first().copied());
        // Scope the import_helper borrow
        {
            // Here we write out the metadata for a new commit to the blame repo.
            // For details on the data format, refer to the documentation at
            // https://git-scm.com/docs/git-fast-import#_commit
            // https://git-scm.com/docs/git-fast-import#_mark
            let import_stream = &mut import_helper.input;
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
                // Like `index_blame`, the earliest revision with an old
                // revision wins in the unlikely event of duplicates.
                for old_rev in parse_oldrevs(oldrevs) {
                    if old_revs_noted.insert(old_rev)
                        && existing_old_notes.lookup(&blame_repo, old_rev).is_none()
                    {
                        old_notes.add(
                            old_rev,
                            &git_oid.to_string(),
                            commit.committer().when().seconds(),
                        );
                    }
                }
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

        // Terminate the commit, and ask for its id, for its note, without
        // waiting for git fast-import to get to it.  Its note, and each batch
        // of notes, are written once we have the ids (see `note_revisions`).
        writeln!(&mut import_helper.input).unwrap();
        import_helper.request_mark(rev_done);
        import_helper.finish_commit(rev_done);
        since_batch += 1;
        let ends_batch = since_batch >= NOTES_BATCH_SIZE;
        unnoted.push_back((
            *git_oid,
            rev_done,
            commit.committer().when().seconds(),
            ends_batch,
        ));
        if ends_batch {
            since_batch = 0;
            // The old revision notes go first so that a crash can't leave a
            // revision recorded as processed without its old revision notes.
            old_notes.flush(&mut import_helper.input).unwrap();
        }
        note_revisions(&mut import_helper, &mut notes, &mut unnoted, false);

        if rev_done % 100000 == 0 {
            info!("Completed 100,000 commits, issuing checkpoint...");
            writeln!(&mut import_helper.input, "checkpoint").unwrap();
        }
    }

    note_revisions(&mut import_helper, &mut notes, &mut unnoted, true);
    old_notes.flush(&mut import_helper.input).unwrap();
    notes.flush(&mut import_helper.input).unwrap();
    drop(hg_helper);

    let cache = &import_helper.cache;
    info!(
        "Reads from the commits written: {} from what we wrote and read, {} from disk, {} through git fast-import; {} checkpoints, {} clears",
        cache.hits, cache.disk_reads, cache.fast_import_reads, cache.checkpoints, cache.clears
    );
    info!("Shutting down fast-import...");
    let exitcode = import_helper.finish();
    if exitcode.success() {
        info!("Done!");
    } else {
        info!("Fast-import exited with {:?}", exitcode.code());
    }
    if stopped {
        std::process::exit(STOPPED_EXIT_CODE);
    }
}
