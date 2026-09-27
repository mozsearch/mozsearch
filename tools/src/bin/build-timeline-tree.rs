// This binary consumes the "syntax" repo built by `build-syntax-token-tree.rs`
// to produce the "timeline" repo and the (non-git) "rev-summaries" directory.
//
// Usage:
//   build-timeline-tree SOURCE_REPO SYNTAX_REPO TIMELINE_REPO REV_SUMMARIES_DIR
//
// The environment variables `BLAME_REF`, `COMMIT_LIMIT`, and `CINNABAR` (set to 0
// to not ask git-cinnabar about the hg revisions in backout messages) are handled
// the same as by `build-syntax-token-tree`.  Like it, we record the revisions
// we've processed in git notes (in the timeline repo), and we find the syntax
// commits of source revisions via the syntax repo's notes; see
// `source_mapping`.
//
// ## Timeline repo contents
//
// - `annotated/PATH`: The token-centric blame for each file in the syntax
//   repo's `files/` subtree.  See `timeline_annotated.rs`.
// - `future/PATH.ndjson`: Physical-path journal of what happened to tokens in
//   and the file at PATH.  See `timeline_future.rs`.
// - `files-delta/PATH.ndjson`: Logical-path journal of per-symbol changes to
//   the file at PATH.  See `timeline_files_delta.rs`.
// - `tokens/AB/CD/TOKEN.ndjson`: Journal of changes involving TOKEN.  See
//   `timeline_tokens.rs`.
//
// Journals start with a header line and then have one record per revision
// ordered from newest to oldest.
//
// ## Processing model
//
// Like `build-blame.rs`, we have compute threads which do the CPU-intensive
// work required for each revision which can only depend on the syntax and
// source repos: they diff the token files, run the move/evolution inference in
// `hyperblame::inference`, and derive the statistics.  The main thread then
// processes the revisions in order, applying the results against the state of
// the parent timeline revision(s) which is the only way to resolve the
// canonical token refs, and writes out the new timeline revision via
// git-fast-import.
//
// Unlike `build-blame.rs` we don't use "deleteall" and re-propagate everything.
// Instead each new timeline commit starts from the tree of its first parent and
// we only write the paths that changed.
//
// ### Merges
//
// For merge commits we don't perform any inference.  For annotated files, like
// `build-blame.rs`, we propagate unchanged tokens from all parents with the
// first parent taking precedence, and any other tokens are treated as newly
// added by the merge.  For journals, we union the records from all of the
// parents so that the records from commits on the non-first-parent branches
// are not lost.  The merge commit itself doesn't get journal records, but it
// does get a rev-summary.
//
// To determine which journals may differ between parents, we look at the files
// which differ between the first parent and each other parent in the syntax
// repo, and the tokens that differ in those files.  This misses journal
// changes whose net effect cancelled out on a branch, like a token being added
// and then removed on the branch.  TODO: Improve this if it matters.
//
// ### Backouts
//
// For a revision which backs out earlier revisions (see `hyperblame::backouts`)
// we want blame to point at the history from before the backed out revisions
// landed rather than at the backout.  The compute thread aligns each file the
// backout writes with the file from before the earliest backed out revision
// which touched it (identical token lines, via the same diff used for merges).
// When building the annotated file, aligned tokens get the record from that
// earlier version unless their record has information from a revision which
// landed in between (see `restore_backed_out_records`).  So a backout that
// exactly undoes its revisions restores the earlier annotated file exactly, and
// one with other changes mixed in restores what lines up.  Aligning with the
// earlier version rather than following the diffs also puts the right records
// on duplicated tokens.
//
// The backout's journal records and rev-summary list the revisions it backs
// out in `backs_out`, and the backed out revisions' rev-summaries get
// `backed_out_by`.  Journal records are never rewritten, which keeps journal
// merging simple.

extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate num_cpus;
extern crate tools;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use chrono::{SecondsFormat, Utc};
use git2::{Delta, DiffFindOptions, Oid, Repository, Sort};
use serde::Serialize;
use serde::de::DeserializeOwned;

use tools::file_format::config::{
    HistorySyntaxCommitMeta, syntax_commit_to_meta, timeline_commit_to_meta,
};
use tools::file_format::history::io_helpers::{
    read_record_file_contents, record_file_contents_to_string,
};
use tools::file_format::history::rev_summaries::{
    RevFileSummaryRecord, RevSummaryRecord, rev_summary_path,
};
use tools::file_format::history::syntax_files::{split_token_line, token_file_lines};
use tools::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use tools::file_format::history::timeline_annotated::{
    HyperLineData, PATH_UNCHANGED, RemovalMarker,
};
use tools::file_format::history::timeline_common::{
    ChangeKind, DetailRecordRef, FileSyntaxDelta, TimelineRecord, TokenDeltaDetails,
    merge_journal_records, token_ref_set_insert,
};
use tools::file_format::history::timeline_files_delta::{
    FileDeltaDetailRecord, FileDeltaHeader, FileDeltaRecord,
};
use tools::file_format::history::timeline_future::{
    FutureDetailRecord, FutureHeader, FutureRecord,
};
use tools::file_format::history::timeline_tokens::{
    TokenDeltaDetailRecord, TokenDeltaRecord, TokenHeader, token_timeline_path, tracked_token_key,
};
use tools::git_ops::git_time_to_chrono;
use tools::hyperblame::backouts::{BackoutTargetResolver, find_backed_out};
use tools::hyperblame::inference::{
    FileChangeInput, FileChangeKind, FileInference, InferenceConfig, PairingSupport, RemovedFate,
    TokenOrigin, diff_token_lines, infer_revision,
};
use tools::hyperblame::stats::compute_revision_stats;
use tools::source_mapping::{
    NOTES_BATCH_SIZE, NotesRefs, SourceMapping, notes_writer, require_notes_for_existing_branch,
};
use tools::tree_sitter_support::cst_tokenizer::namespace_for_file;

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

/// The timeline commit a previous run derived from `syntax_rev`, if any.  The
/// notes are keyed by source revision, so we check that the timeline commit was
/// derived from this syntax commit rather than another syntax commit for the
/// same source revision (ex: if the syntax repo was regenerated, or had a
/// commit left incomplete by a crash which was since replaced).
fn processed_timeline_commit(
    syntax_repo: &Repository,
    timeline_repo: &Repository,
    mapping: &SourceMapping,
    syntax_rev: Oid,
) -> Option<Oid> {
    let source_rev = syntax_commit_to_meta(&syntax_repo.find_commit(syntax_rev).ok()?).source_rev;
    let timeline_rev = mapping.lookup(timeline_repo, source_rev)?;
    let meta = timeline_commit_to_meta(&timeline_repo.find_commit(timeline_rev).ok()?);
    (meta.syntax_rev == syntax_rev).then_some(timeline_rev)
}

/// The timeline commits for syntax commits: those written by this run and
/// those written by previous runs.
struct TimelineCommits<'a> {
    syntax_repo: &'a Repository,
    timeline_repo: &'a Repository,
    mapping: SourceMapping,
    /// The commits written by this run and the processed parents of the first
    /// revisions it processes.
    known: HashMap<Oid, TimelineRepoCommit>,
}

impl TimelineCommits<'_> {
    fn get(&self, syntax_rev: Oid) -> Option<TimelineRepoCommit> {
        self.known.get(&syntax_rev).copied().or_else(|| {
            processed_timeline_commit(
                self.syntax_repo,
                self.timeline_repo,
                &self.mapping,
                syntax_rev,
            )
            .map(TimelineRepoCommit::Commit)
        })
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

/// Return the contents of the object with the given oid.
/// Documentation for the fast-import command used is at
/// https://git-scm.com/docs/git-fast-import#_cat_blob
fn read_blob(import_helper: &mut Child, oid: &str) -> Vec<u8> {
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
    blob
}

/// Return the contents of the object at the given path in the
/// given commit. Returns None if there is no such object.
fn read_path_blob(
    import_helper: &mut Child,
    commit: &TimelineRepoCommit,
    path: &Path,
) -> Option<Vec<u8>> {
    let oid = read_path_oid(import_helper, commit, path)?;
    Some(read_blob(import_helper, &oid))
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

fn write_inline_blob(import_helper: &mut Child, path: &Path, contents: &[u8]) {
    // For the inline data format documentation, refer to
    // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Inlinedataformat
    // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Exactbytecountformat
    let import_stream = import_helper.stdin.as_mut().unwrap();
    writeln!(import_stream, "M 100644 inline {}", sanitize(path)).unwrap();
    writeln!(import_stream, "data {}", contents.len()).unwrap();
    import_stream.write_all(contents).unwrap();
    // We skip the optional trailing LF character here since in practice it
    // wasn't particularly useful for debugging.
}

fn write_existing_blob(import_helper: &mut Child, path: &Path, oid: &str) {
    writeln!(
        import_helper.stdin.as_mut().unwrap(),
        "M 100644 {} {}",
        oid,
        sanitize(path)
    )
    .unwrap();
}

fn delete_path(import_helper: &mut Child, path: &Path) {
    writeln!(
        import_helper.stdin.as_mut().unwrap(),
        "D {}",
        sanitize(path)
    )
    .unwrap();
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

fn annotated_path(path: &str) -> PathBuf {
    PathBuf::from(format!("annotated/{}", path))
}

fn future_path(path: &str) -> PathBuf {
    PathBuf::from(format!("future/{}.ndjson", path))
}

fn files_delta_path(path: &str) -> PathBuf {
    PathBuf::from(format!("files-delta/{}.ndjson", path))
}

// ## Compute thread logic

/// The results of processing a changed file in a non-merge revision.
struct FileChange {
    kind: FileChangeKind,
    /// The path of the file in the parent revision; None for added files except
    /// for copies which were downgraded to additions because most of their
    /// tokens were not copied, in which case tokens can still be `Unchanged`
    /// relative to this path.
    old_path: Option<String>,
    /// The path of the file in this revision; None for deleted files.
    new_path: Option<String>,
    inference: FileInference,
    delta: FileSyntaxDelta,
    /// For files written by a backout, the earlier version of the file to
    /// restore token records from.
    restore: Option<RestoreBase>,
}

/// The version of a file (at the same path) from before the revisions a backout
/// backs out.
struct RestoreBase {
    /// The parent of the earliest backed out revision which touched the file.
    syntax_rev: Oid,
    /// For each token in the backout's version of the file, the 1-based line
    /// number of the identical token in the earlier version, if any.
    aligned: Vec<Option<u32>>,
    /// The source revisions which landed between the earlier version and the
    /// backout, other than the backed out revisions.
    intervening: Arc<HashSet<String>>,
}

/// How a file in a merge commit relates to a specific parent.
enum ParentMapping {
    /// The file is identical to the file at `path` in the parent.
    Identical { path: String },
    /// For each token in the merge's version of the file, the 1-based line
    /// number of the unchanged token in the parent's file at `path`, if any.
    Diffed {
        path: String,
        unchanged: Vec<Option<u32>>,
    },
}

/// The results of processing a file that differs between a merge commit and
/// its first parent.
struct MergeFileChange {
    /// The path of the file in the merge commit; None if deleted relative to
    /// the first parent.
    new_path: Option<String>,
    /// The path of the file in the first parent if it was deleted or renamed.
    removed_path: Option<String>,
    /// The number of tokens in the file in the merge commit.
    num_tokens: u32,
    /// Parallel to the commit's parents.
    parents: Vec<Option<ParentMapping>>,
}

struct MergeChanges {
    files: Vec<MergeFileChange>,
    /// Paths whose physical/logical journals may differ between parents, and
    /// whether the path exists in the merge commit.
    candidate_paths: BTreeMap<String, bool>,
    /// Tokens whose journals may differ between parents.
    candidate_tokens: BTreeSet<String>,
}

enum RevisionChanges {
    Linear {
        files: Vec<FileChange>,
        token_totals: BTreeMap<String, TokenDeltaDetails>,
    },
    Merge(MergeChanges),
}

struct TimelineData {
    meta: HistorySyntaxCommitMeta,
    iso_date: String,
    unmapped_author: String,
    message: String,
    changes: RevisionChanges,
    /// The source revisions this revision backs out, earliest first.
    backed_out: Vec<String>,
}

fn subtree<'r>(repo: &'r Repository, root: &git2::Tree, name: &str) -> Option<git2::Tree<'r>> {
    root.get_path(Path::new(name))
        .ok()?
        .to_object(repo)
        .ok()?
        .peel_to_tree()
        .ok()
}

fn blob_string(repo: &Repository, oid: Oid) -> String {
    match repo.find_blob(oid) {
        Ok(blob) => String::from_utf8_lossy(blob.content()).into_owned(),
        Err(_) => String::new(),
    }
}

fn path_blob_string(repo: &Repository, tree: Option<&git2::Tree>, path: &str) -> Option<String> {
    let entry = tree?.get_path(Path::new(path)).ok()?;
    Some(blob_string(repo, entry.id()))
}

/// Load the set of symbol "pretty" identifiers from a "files-struct" file.
/// The parts of a "files-struct" file we care about.
#[derive(Default)]
struct FilesStruct {
    /// The namespace recorded in the header, if any.
    namespace: Option<String>,
    /// The set of symbol "pretty" identifiers.
    symbols: BTreeSet<String>,
}

fn load_files_struct(repo: &Repository, tree: Option<&git2::Tree>, path: &str) -> FilesStruct {
    let Some(contents) = path_blob_string(repo, tree, path) else {
        return FilesStruct::default();
    };
    let parsed: Option<(FileStructureHeader, Vec<FileStructureRow>)> =
        read_record_file_contents(contents.as_bytes());
    match parsed {
        Some((header, rows)) => FilesStruct {
            namespace: header.effective_namespace().map(str::to_string),
            symbols: rows.into_iter().map(|row| row.pretty).collect(),
        },
        None => FilesStruct::default(),
    }
}

/// The namespace for a file for inference purposes.  This comes from the
/// "files-struct" header when available because it accounts for language
/// overrides, falling back to the default for the path.
fn struct_namespace(path: &str, file_struct: &FilesStruct) -> String {
    file_struct
        .namespace
        .clone()
        .unwrap_or_else(|| namespace_for_file(Path::new(path)).to_string())
}

/// git pairs renamed files by similarity (see `diff_files_trees`), which small
/// unrelated files can reach just by sharing a license header and some syntax.
/// We only keep a rename if the content similarity of the files (see
/// `PairingSupport`) is at least this, otherwise we treat it as a deletion and
/// an addition.
///
/// This is deliberately lower than git's threshold (30%).  In the mozsearch
/// history and a replay of vendored puppeteer syncs, the renames below 0.2
/// were all unrelated files, but 0.2 to 0.45 also has real renames of
/// rewritten files (ex: a 0.24 move of ProductLauncher.ts into a new package, a
/// 0.42 .js to .ts conversion).  Wrongly splitting a rename loses more than
/// wrongly keeping one: shared content is only found as moved if it's in long
/// enough runs.
const MIN_RENAME_CONTENT_SIMILARITY: f64 = 0.2;

fn path_string(path: Option<&Path>) -> Option<String> {
    path.map(|p| p.to_string_lossy().into_owned())
}

fn diff_files_trees<'r>(
    repo: &'r Repository,
    old: Option<&git2::Tree>,
    new: Option<&git2::Tree>,
    find_similar: bool,
) -> Result<git2::Diff<'r>, git2::Error> {
    let mut diff = repo.diff_tree_to_tree(old, new, None)?;
    if find_similar {
        diff.find_similar(Some(
            DiffFindOptions::new()
                .copies(true)
                .copy_threshold(30)
                .renames(true)
                .rename_threshold(30)
                .rename_limit(1000000)
                .break_rewrites(true)
                .break_rewrites_for_renames_only(true),
        ))?;
    }
    Ok(diff)
}

fn delta_kind(status: Delta) -> Option<FileChangeKind> {
    match status {
        Delta::Added => Some(FileChangeKind::Added),
        Delta::Deleted => Some(FileChangeKind::Deleted),
        Delta::Modified | Delta::Typechange => Some(FileChangeKind::Modified),
        Delta::Renamed => Some(FileChangeKind::Renamed),
        Delta::Copied => Some(FileChangeKind::Copied),
        _ => None,
    }
}

/// Process a non-merge revision (including root revisions).
fn preprocess_linear(
    repo: &Repository,
    commit: &git2::Commit,
    config: &InferenceConfig,
) -> Result<RevisionChanges, git2::Error> {
    let cur_root = commit.tree()?;
    let cur_files = subtree(repo, &cur_root, "files");
    let cur_structs = subtree(repo, &cur_root, "files-struct");
    let parent_root = match commit.parents().next() {
        Some(parent) => Some(parent.tree()?),
        None => None,
    };
    let parent_files = parent_root.as_ref().and_then(|r| subtree(repo, r, "files"));
    let parent_structs = parent_root
        .as_ref()
        .and_then(|r| subtree(repo, r, "files-struct"));

    struct Pending {
        kind: FileChangeKind,
        /// The path in the parent revision.  Usually None for added files, but
        /// see the copy downgrade logic below.
        old_path: Option<String>,
        new_path: Option<String>,
        old_contents: String,
        new_contents: String,
        old_struct: FilesStruct,
        new_struct: FilesStruct,
        /// The namespace for inference purposes.  This comes from the
        /// "files-struct" header when available because it accounts for
        /// language overrides, falling back to the default for the path.
        namespace: String,
    }

    fn inputs_for(pending: &[Pending]) -> Vec<FileChangeInput<'_>> {
        pending
            .iter()
            .map(|p| FileChangeInput {
                kind: p.kind,
                namespace: &p.namespace,
                old_lines: token_file_lines(&p.old_contents),
                new_lines: token_file_lines(&p.new_contents),
            })
            .collect()
    }

    let diff = diff_files_trees(repo, parent_files.as_ref(), cur_files.as_ref(), true)?;
    let mut pending = vec![];
    for delta in diff.deltas() {
        let Some(kind) = delta_kind(delta.status()) else {
            continue;
        };
        let old_path = match kind {
            FileChangeKind::Added => None,
            _ => path_string(delta.old_file().path()),
        };
        let new_path = match kind {
            FileChangeKind::Deleted => None,
            _ => path_string(delta.new_file().path()),
        };
        let old_struct = match &old_path {
            Some(path) => load_files_struct(repo, parent_structs.as_ref(), path),
            None => FilesStruct::default(),
        };
        let new_struct = match &new_path {
            Some(path) => load_files_struct(repo, cur_structs.as_ref(), path),
            None => FilesStruct::default(),
        };
        let namespace = new_struct
            .namespace
            .clone()
            .or_else(|| old_struct.namespace.clone())
            .unwrap_or_else(|| {
                let path = new_path.as_deref().or(old_path.as_deref()).unwrap();
                namespace_for_file(Path::new(path)).to_string()
            });
        pending.push(Pending {
            kind,
            old_struct,
            new_struct,
            namespace,
            old_contents: match old_path {
                Some(_) => blob_string(repo, delta.old_file().id()),
                None => String::new(),
            },
            new_contents: match new_path {
                Some(_) => blob_string(repo, delta.new_file().id()),
                None => String::new(),
            },
            old_path,
            new_path,
        });
    }

    let mut inferences = infer_revision(&inputs_for(&pending), config);
    let supports: Vec<Option<PairingSupport>> = inputs_for(&pending)
        .iter()
        .zip(&inferences)
        .enumerate()
        .map(|(idx, (input, inference))| {
            (input.kind == FileChangeKind::Renamed)
                .then(|| PairingSupport::compute(idx, input, inference))
        })
        .collect();

    // libgit2 can report a deleted file as the source of several renames (ex:
    // when a file is split in two), which would give the old file's tokens
    // fates from each of them.  Only the most similar one is a rename; the
    // others are copies, which the copy check below may downgrade to additions
    // if their content was mainly moved rather than copied.
    // Maps old paths to the index, similarity, and new path of their best
    // rename.
    let mut best_renames: HashMap<String, (usize, f64, String)> = HashMap::new();
    for (idx, (p, support)) in pending.iter().zip(&supports).enumerate() {
        if let (Some(support), Some(old_path), Some(new_path)) = (support, &p.old_path, &p.new_path)
        {
            let similarity = support.similarity().unwrap_or(0.0);
            let best = best_renames
                .entry(old_path.clone())
                .or_insert_with(|| (idx, similarity, new_path.clone()));
            if similarity > best.1 {
                *best = (idx, similarity, new_path.clone());
            }
        }
    }
    let mut changed = false;
    for (idx, p) in pending.iter_mut().enumerate() {
        if let (FileChangeKind::Renamed, Some(old_path)) = (p.kind, &p.old_path) {
            let (best_idx, _, best_new_path) = &best_renames[old_path];
            if *best_idx != idx {
                info!(
                    "Treating rename {} -> {} as a copy because {} is more similar",
                    old_path,
                    p.new_path.as_deref().unwrap_or_default(),
                    best_new_path
                );
                p.kind = FileChangeKind::Copied;
                changed = true;
            }
        }
    }

    // Only keep renames whose content supports them (see
    // `MIN_RENAME_CONTENT_SIMILARITY`).  Splitting a rename into a deletion and
    // an addition means the old file's tokens become potential move sources for
    // every file, so we need to re-run the inference.  Any real shared content
    // (ex: a large comment) can still be found as moved.
    let mut split = vec![];
    for (p, support) in pending.iter_mut().zip(&supports) {
        let Some(support) = support else {
            continue;
        };
        if p.kind != FileChangeKind::Renamed {
            continue;
        }
        let similarity = support.similarity();
        let keep = similarity.is_none_or(|s| s >= MIN_RENAME_CONTENT_SIMILARITY);
        info!(
            "{} {} -> {}: {:?} similarity {:?}",
            if keep {
                "Keeping rename"
            } else {
                "Splitting rename"
            },
            p.old_path.as_deref().unwrap_or_default(),
            p.new_path.as_deref().unwrap_or_default(),
            support,
            similarity
        );
        if !keep {
            let old_path = p.old_path.take().unwrap();
            let old_struct = std::mem::take(&mut p.old_struct);
            split.push(Pending {
                kind: FileChangeKind::Deleted,
                namespace: struct_namespace(&old_path, &old_struct),
                old_path: Some(old_path),
                new_path: None,
                old_contents: std::mem::take(&mut p.old_contents),
                new_contents: String::new(),
                old_struct,
                new_struct: FilesStruct::default(),
            });
            p.kind = FileChangeKind::Added;
            p.namespace = struct_namespace(p.new_path.as_deref().unwrap(), &p.new_struct);
        }
    }
    if changed || !split.is_empty() {
        pending.extend(split);
        inferences = infer_revision(&inputs_for(&pending), config);
    }

    // git's copy detection is similarity based, so a new file which was split
    // out of an existing file (and which the inference determined was mainly
    // moved tokens) or which just shares boilerplate with an existing file can
    // be detected as a copy.  For the purposes of the file-level history
    // (sentinel, files-delta, symbol changes) we only want to treat the file as
    // a copy if the majority of its content (see `PairingSupport`) was actually
    // copied, otherwise it's a new file.  Note that the token origins still
    // reference the copy source for the tokens that were copied.
    let downgrade: Vec<bool> = inputs_for(&pending)
        .iter()
        .zip(&inferences)
        .enumerate()
        .map(|(idx, (input, inference))| {
            if input.kind != FileChangeKind::Copied {
                return false;
            }
            let support = PairingSupport::compute(idx, input, inference);
            let keep = support.new_content == 0 || support.supported * 2 >= support.new_content;
            info!(
                "{} {} -> {}: {:?}",
                if keep {
                    "Keeping copy"
                } else {
                    "Downgrading copy"
                },
                pending[idx].old_path.as_deref().unwrap_or_default(),
                pending[idx].new_path.as_deref().unwrap_or_default(),
                support
            );
            !keep
        })
        .collect();
    for (p, downgrade) in pending.iter_mut().zip(downgrade) {
        if downgrade {
            p.kind = FileChangeKind::Added;
        }
    }
    let inputs = inputs_for(&pending);

    let old_symbols: Vec<BTreeSet<String>> = pending
        .iter()
        .map(|p| match p.kind {
            // (Downgraded copies are `Added` but still have an `old_path`.)
            FileChangeKind::Added => BTreeSet::new(),
            _ => p.old_struct.symbols.clone(),
        })
        .collect();
    let new_symbols: Vec<BTreeSet<String>> = pending
        .iter()
        .map(|p| p.new_struct.symbols.clone())
        .collect();
    let stats = compute_revision_stats(&inputs, &inferences, &old_symbols, &new_symbols);
    drop(inputs);

    let files = pending
        .into_iter()
        .zip(inferences)
        .zip(stats.file_groups)
        .map(|((p, inference), symbol_group)| {
            let (change, moved_from) = match p.kind {
                FileChangeKind::Added => (ChangeKind::Added, None),
                FileChangeKind::Deleted => (ChangeKind::Removed, None),
                FileChangeKind::Modified => (ChangeKind::Changed, None),
                FileChangeKind::Renamed | FileChangeKind::Copied => {
                    (ChangeKind::Evolved, p.old_path.clone())
                }
            };
            FileChange {
                kind: p.kind,
                old_path: p.old_path,
                new_path: p.new_path,
                inference,
                delta: FileSyntaxDelta {
                    change,
                    moved_from,
                    copied: p.kind == FileChangeKind::Copied,
                    symbol_group,
                },
                restore: None,
            }
        })
        .collect();

    Ok(RevisionChanges::Linear {
        files,
        token_totals: stats.token_totals,
    })
}

/// For each of `new_lines`, the 1-based line number of the identical token in
/// `old_lines` per their diff, if any.
fn unchanged_mapping(
    old_lines: &[&str],
    new_lines: &[&str],
    config: &InferenceConfig,
) -> Vec<Option<u32>> {
    let mut unchanged = vec![None; new_lines.len()];
    for op in diff_token_lines(old_lines, new_lines, config.diff_timeout) {
        if let similar::DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for j in 0..len {
                unchanged[new_index + j] = Some((old_index + j) as u32 + 1);
            }
        }
    }
    unchanged
}

/// For each file written by a backout, find the version of the file from before
/// the earliest backed out revision (`backed_out`, earliest first) which touched
/// it, and align the backout's tokens with it.
fn add_restore_bases(
    repo: &Repository,
    commit: &git2::Commit,
    backed_out: &[Oid],
    files: &mut [FileChange],
    config: &InferenceConfig,
) -> Result<(), git2::Error> {
    let cur_files = subtree(repo, &commit.tree()?, "files");
    let mut intervening_by_base: HashMap<Oid, Arc<HashSet<String>>> = HashMap::new();
    let mut intervening_since = |base: Oid| -> Result<Arc<HashSet<String>>, git2::Error> {
        if let Some(revs) = intervening_by_base.get(&base) {
            return Ok(revs.clone());
        }
        let mut revs = HashSet::new();
        if let Some(parent) = commit.parent_ids().next() {
            let mut walk = repo.revwalk()?;
            walk.push(parent)?;
            walk.hide(base)?;
            for oid in walk {
                let oid = oid?;
                if !backed_out.contains(&oid) {
                    let meta = syntax_commit_to_meta(&repo.find_commit(oid)?);
                    revs.insert(meta.source_rev.to_string());
                }
            }
        }
        let revs = Arc::new(revs);
        intervening_by_base.insert(base, revs.clone());
        Ok(revs)
    };
    let mut targets = vec![];
    for oid in backed_out {
        let target = repo.find_commit(*oid)?;
        let target_files = subtree(repo, &target.tree()?, "files");
        let base = match target.parents().next() {
            Some(parent) => Some((parent.id(), subtree(repo, &parent.tree()?, "files"))),
            None => None,
        };
        targets.push((target_files, base));
    }
    let entry_id = |tree: &Option<git2::Tree>, path: &str| {
        tree.as_ref()
            .and_then(|t| t.get_path(Path::new(path)).ok())
            .map(|e| e.id())
    };
    for change in files.iter_mut() {
        let Some(new_path) = change.new_path.as_deref() else {
            continue;
        };
        let touched_by = targets.iter().find(|(target_files, base)| {
            let base_id = base.as_ref().and_then(|(_, tree)| entry_id(tree, new_path));
            entry_id(target_files, new_path) != base_id
        });
        let Some((_, Some((base_rev, base_files)))) = touched_by else {
            continue;
        };
        let (Some(base_contents), Some(new_contents)) = (
            path_blob_string(repo, base_files.as_ref(), new_path),
            path_blob_string(repo, cur_files.as_ref(), new_path),
        ) else {
            continue;
        };
        change.restore = Some(RestoreBase {
            syntax_rev: *base_rev,
            aligned: unchanged_mapping(
                &token_file_lines(&base_contents),
                &token_file_lines(&new_contents),
                config,
            ),
            intervening: intervening_since(*base_rev)?,
        });
    }
    Ok(())
}

/// Add the tracked tokens for `lines[range]` to `candidate_tokens`.  We don't
/// know the file's namespace here, so we use a namespace with no value words
/// which produces a superset of the tokens tracked in any namespace.  That's
/// fine because these are just candidates for journal merging.
fn add_tracked_tokens(
    lines: &[&str],
    range: std::ops::Range<usize>,
    candidate_tokens: &mut BTreeSet<String>,
) {
    for idx in range {
        let line = split_token_line(lines[idx]);
        let prev = idx.checked_sub(1).map(|i| split_token_line(lines[i]));
        if let Some(key) = tracked_token_key("", &line, prev.as_ref()) {
            candidate_tokens.insert(key.into_owned());
        }
    }
}

/// Process a merge revision.
fn preprocess_merge(
    repo: &Repository,
    commit: &git2::Commit,
    config: &InferenceConfig,
) -> Result<RevisionChanges, git2::Error> {
    let cur_root = commit.tree()?;
    let cur_files = subtree(repo, &cur_root, "files");
    let parent_files: Vec<Option<git2::Tree>> = commit
        .parents()
        .map(|p| p.tree().ok().and_then(|root| subtree(repo, &root, "files")))
        .collect();

    // ## Annotated changes relative to the first parent.
    let mut files = vec![];
    let diff = diff_files_trees(repo, parent_files[0].as_ref(), cur_files.as_ref(), true)?;
    for delta in diff.deltas() {
        let Some(kind) = delta_kind(delta.status()) else {
            continue;
        };
        let old_path = path_string(delta.old_file().path());
        if kind == FileChangeKind::Deleted {
            files.push(MergeFileChange {
                new_path: None,
                removed_path: old_path,
                num_tokens: 0,
                parents: vec![],
            });
            continue;
        }
        let new_path = path_string(delta.new_file().path()).unwrap();
        let new_oid = delta.new_file().id();
        let new_contents = blob_string(repo, new_oid);
        let new_lines = token_file_lines(&new_contents);

        let mut parents = vec![];
        for (i, parent_tree) in parent_files.iter().enumerate() {
            let parent_path = if i == 0 {
                match kind {
                    FileChangeKind::Added => None,
                    _ => old_path.clone(),
                }
            } else {
                Some(new_path.clone())
            };
            let entry = parent_path.as_ref().and_then(|path| {
                parent_tree
                    .as_ref()
                    .and_then(|t| t.get_path(Path::new(path)).ok())
            });
            let mapping = match (parent_path, entry) {
                (Some(path), Some(entry)) if entry.id() == new_oid => {
                    Some(ParentMapping::Identical { path })
                }
                (Some(path), Some(entry)) => {
                    let old_contents = blob_string(repo, entry.id());
                    let old_lines = token_file_lines(&old_contents);
                    let unchanged = unchanged_mapping(&old_lines, &new_lines, config);
                    Some(ParentMapping::Diffed { path, unchanged })
                }
                _ => None,
            };
            parents.push(mapping);
        }

        files.push(MergeFileChange {
            new_path: Some(new_path),
            removed_path: if kind == FileChangeKind::Renamed {
                old_path
            } else {
                None
            },
            num_tokens: new_lines.len() as u32,
            parents,
        });
    }

    // ## Journal candidates: differences between the first parent and others.
    let mut candidate_paths = BTreeMap::new();
    let mut candidate_tokens = BTreeSet::new();
    for other_files in parent_files.iter().skip(1) {
        let diff = diff_files_trees(repo, parent_files[0].as_ref(), other_files.as_ref(), false)?;
        for delta in diff.deltas() {
            let old_contents = blob_string(repo, delta.old_file().id());
            let new_contents = blob_string(repo, delta.new_file().id());
            let old_lines = token_file_lines(&old_contents);
            let new_lines = token_file_lines(&new_contents);
            for path in [delta.old_file().path(), delta.new_file().path()]
                .into_iter()
                .flatten()
            {
                let path = path.to_string_lossy().into_owned();
                let exists = cur_files
                    .as_ref()
                    .is_some_and(|t| t.get_path(Path::new(&path)).is_ok());
                candidate_paths.insert(path, exists);
            }
            for op in diff_token_lines(&old_lines, &new_lines, config.diff_timeout) {
                match op {
                    similar::DiffOp::Equal { .. } => {}
                    _ => {
                        add_tracked_tokens(&old_lines, op.old_range(), &mut candidate_tokens);
                        add_tracked_tokens(&new_lines, op.new_range(), &mut candidate_tokens);
                    }
                }
            }
        }
    }

    Ok(RevisionChanges::Merge(MergeChanges {
        files,
        candidate_paths,
        candidate_tokens,
    }))
}

// Does the CPU-intensive work required for pre-computation of a given revision
// that can depend on any source tree revision and syntax token tree revisions
// (which will already have been generated through the current revision), but
// cannot depend on any revisions timeline revisions because those will not have
// been created yet.  (Anything that depends on timeline revisions needs to
// happen in our main thread logic.)
fn thread_preprocess_revision(
    syntax_repo: &Repository,
    source_repo: &Repository,
    backout_resolver: &BackoutTargetResolver,
    rev_meta: &HistorySyntaxCommitMeta,
    config: &InferenceConfig,
) -> Result<TimelineData, git2::Error> {
    let commit = syntax_repo.find_commit(rev_meta.syntax_rev)?;

    let (iso_date, unmapped_author, message) = match source_repo.find_commit(rev_meta.source_rev) {
        Ok(source_commit) => {
            let author = source_commit.author();
            (
                git_time_to_chrono(source_commit.committer().when())
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
                format!(
                    "{} <{}>",
                    author.name().unwrap_or(""),
                    author.email().unwrap_or("")
                ),
                source_commit.message().unwrap_or("").to_string(),
            )
        }
        // It's possible to run against a syntax repo without the source repo
        // having all the revisions, in which case we fall back to the syntax
        // commit which has the same author/committer information.
        Err(_) => (
            git_time_to_chrono(commit.committer().when())
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
            String::new(),
            String::new(),
        ),
    };

    let mut changes = if commit.parent_count() <= 1 {
        preprocess_linear(syntax_repo, &commit, config)?
    } else {
        preprocess_merge(syntax_repo, &commit, config)?
    };

    let mut backed_out = vec![];
    if let RevisionChanges::Linear { files, .. } = &mut changes {
        let targets = find_backed_out(syntax_repo, backout_resolver, &commit, &message);
        if !targets.is_empty() {
            add_restore_bases(syntax_repo, &commit, &targets, files, config)?;
            for target in targets {
                let meta = syntax_commit_to_meta(&syntax_repo.find_commit(target)?);
                backed_out.push(meta.source_rev.to_string());
            }
        }
    }

    Ok(TimelineData {
        meta: rev_meta.clone(),
        iso_date,
        unmapped_author,
        message,
        changes,
        backed_out,
    })
}

struct ComputeThread {
    query_tx: Sender<HistorySyntaxCommitMeta>,
    response_rx: Receiver<TimelineData>,
}

impl ComputeThread {
    fn new(
        syntax_repo_path: &str,
        source_repo_path: &str,
        backout_resolver: Arc<BackoutTargetResolver>,
    ) -> Self {
        let (query_tx, query_rx) = channel();
        let (response_tx, response_rx) = channel();
        let syntax_repo_path = syntax_repo_path.to_string();
        let source_repo_path = source_repo_path.to_string();
        thread::spawn(move || {
            compute_thread_main(
                query_rx,
                response_tx,
                syntax_repo_path,
                source_repo_path,
                backout_resolver,
            );
        });

        ComputeThread {
            query_tx,
            response_rx,
        }
    }

    fn compute(&self, rev_meta: &HistorySyntaxCommitMeta) {
        self.query_tx.send(rev_meta.clone()).unwrap();
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
    syntax_repo_path: String,
    source_repo_path: String,
    backout_resolver: Arc<BackoutTargetResolver>,
) {
    let syntax_repo = Repository::open(syntax_repo_path).unwrap();
    let source_repo = Repository::open(source_repo_path).unwrap();
    let config = InferenceConfig::default();
    while let Ok(rev) = query_rx.recv() {
        let result = thread_preprocess_revision(
            &syntax_repo,
            &source_repo,
            &backout_resolver,
            &rev,
            &config,
        )
        .unwrap();
        response_tx.send(result).unwrap();
    }
}

// ## Main thread logic

/// Split an annotated file into its lines.
fn annotated_lines(blob: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(blob)
        .lines()
        .map(|l| l.to_string())
        .collect()
}

fn join_annotated(lines: Vec<String>) -> String {
    let mut contents = lines.join("\n");
    contents.push('\n');
    contents
}

fn read_journal<H: DeserializeOwned + Default, R: DeserializeOwned>(
    import_helper: &mut Child,
    commit: &TimelineRepoCommit,
    path: &Path,
) -> (H, Vec<R>) {
    read_path_blob(import_helper, commit, path)
        .and_then(|blob| read_record_file_contents(&blob))
        .unwrap_or_else(|| (H::default(), vec![]))
}

/// Prepend a record to the journal at `from_path` in the parent timeline commit
/// and write it to `to_path` in the new commit.
fn prepend_journal_record<
    H: DeserializeOwned + Default + Serialize,
    R: DeserializeOwned + Serialize,
>(
    import_helper: &mut Child,
    parent: Option<&TimelineRepoCommit>,
    from_path: &Path,
    to_path: &Path,
    record: R,
) {
    let (header, mut records): (H, Vec<R>) = match parent {
        Some(parent) => read_journal(import_helper, parent, from_path),
        None => (H::default(), vec![]),
    };
    records.insert(0, record);
    let contents = record_file_contents_to_string(&header, &records);
    write_inline_blob(import_helper, to_path, contents.as_bytes());
}

/// Union the journal at `path` across all parents, writing it if the parents'
/// versions differ.
fn union_journal<
    H: DeserializeOwned + Default + Serialize,
    R: DeserializeOwned + Serialize + TimelineRecord,
>(
    import_helper: &mut Child,
    parents: &[TimelineRepoCommit],
    path: &Path,
) {
    let oids: Vec<Option<String>> = parents
        .iter()
        .map(|p| read_path_oid(import_helper, p, path))
        .collect();
    if oids.iter().all(|oid| *oid == oids[0]) {
        // The tree already has the first parent's version.
        return;
    }
    let mut header: Option<H> = None;
    let mut versions = vec![];
    for oid in oids.iter().flatten() {
        let blob = read_blob(import_helper, oid);
        if let Some((h, records)) = read_record_file_contents::<H, R>(&blob) {
            header.get_or_insert(h);
            versions.push(records);
        }
    }
    let records = merge_journal_records(versions);
    let contents = record_file_contents_to_string(&header.unwrap_or_default(), &records);
    write_inline_blob(import_helper, path, contents.as_bytes());
}

/// The parent annotated files for the old paths of all changed files in a
/// linear revision.
type ParentAnnotated = HashMap<String, Vec<String>>;

fn parent_line<'a>(parents: &'a ParentAnnotated, path: &str, lineno: u32) -> Option<&'a str> {
    parents.get(path)?.get(lineno as usize).map(|s| s.as_str())
}

/// Build the annotated file lines for a changed file in a linear revision.
fn build_linear_annotated(
    rev: &str,
    change: &FileChange,
    changes: &[FileChange],
    parents: &ParentAnnotated,
) -> Vec<String> {
    let new_path = change.new_path.as_deref().unwrap();
    let old_path = change.old_path.as_deref();
    let origins = &change.inference.origins;
    let mut lines: Vec<String> = Vec::with_capacity(origins.len() + 1);

    // ## Sentinel
    let parent_sentinel = old_path.and_then(|p| parent_line(parents, p, 0));
    let sentinel = match (change.kind, old_path, parent_sentinel) {
        (FileChangeKind::Modified, _, Some(line)) => line.to_string(),
        (FileChangeKind::Renamed | FileChangeKind::Copied, Some(old_path), Some(line)) => {
            let mut pred = HyperLineData::parse(line).introduced;
            pred.resolve_path(old_path);
            pred.relativize_path(new_path);
            let mut data = HyperLineData::new_introduced(rev, 0);
            data.predecessor = Some(pred);
            data.serialize()
        }
        _ => HyperLineData::new_introduced(rev, 0).serialize(),
    };
    lines.push(sentinel);

    // ## Tokens
    let mut missing_history = 0;
    for (new_idx, origin) in origins.iter().enumerate() {
        let lineno = new_idx as u32 + 1;
        let line = match *origin {
            TokenOrigin::Unchanged { old_lineno } => {
                let old_path = old_path.unwrap();
                match parent_line(parents, old_path, old_lineno) {
                    Some(line) if old_path == new_path => Some(line.to_string()),
                    Some(line) => {
                        let mut data = HyperLineData::parse(line);
                        data.transplant(old_path, new_path);
                        Some(data.serialize())
                    }
                    None => None,
                }
            }
            TokenOrigin::Moved {
                from_file,
                old_lineno,
            } => {
                let from_path = changes[from_file as usize].old_path.as_deref().unwrap();
                parent_line(parents, from_path, old_lineno).map(|line| {
                    let mut data = HyperLineData::parse(line);
                    // The removal marker described removals after the token's
                    // old position, which is no longer meaningful.
                    data.removal_marker = None;
                    data.transplant(from_path, new_path);
                    data.serialize()
                })
            }
            TokenOrigin::Evolved {
                from_file,
                old_lineno,
            } => {
                let from_path = changes[from_file as usize].old_path.as_deref().unwrap();
                let mut data = HyperLineData::new_introduced(rev, lineno);
                if let Some(line) = parent_line(parents, from_path, old_lineno) {
                    let mut pred = HyperLineData::parse(line).introduced;
                    pred.resolve_path(from_path);
                    pred.relativize_path(new_path);
                    data.predecessor = Some(pred);
                }
                Some(data.serialize())
            }
            TokenOrigin::Added => Some(HyperLineData::new_introduced(rev, lineno).serialize()),
        };
        lines.push(line.unwrap_or_else(|| {
            missing_history += 1;
            HyperLineData::new_introduced(rev, lineno).serialize()
        }));
    }
    if missing_history > 0 {
        warn!(
            "  {} tokens in {} lacked parent history and were treated as added",
            missing_history, new_path
        );
    }

    // ## Removal markers
    if let Some(old_path) = old_path {
        for run in &change.inference.removal_runs {
            let Some(first) = parent_line(parents, old_path, run.first_old_lineno) else {
                continue;
            };
            let mut first_removed = HyperLineData::parse(first).introduced;
            first_removed.resolve_path(old_path);
            first_removed.relativize_path(new_path);
            let host = run.host_new_lineno as usize;
            let host_line = std::mem::take(&mut lines[host]);
            let mut host_data = HyperLineData::parse(&host_line);
            host_data.removal_marker = Some(RemovalMarker {
                source_rev: Cow::Borrowed(rev),
                path: Cow::Borrowed(if old_path == new_path {
                    PATH_UNCHANGED
                } else {
                    old_path
                }),
                lineno: run.first_old_lineno,
                first_removed,
                num_removed: run.num_removed,
                num_moved: run.num_moved,
            });
            lines[host] = host_data.serialize();
        }
    }

    lines
}

/// For a file written by a backout, give each token which is aligned with a
/// token in the file's earlier version (`base`, see `RestoreBase`) the earlier
/// token's record, unless its record has information from one of the
/// `intervening` revisions which landed in between, like a removal marker from
/// an unrelated commit.  Records which reference the backout or the backed out
/// revisions are replaced, but so are records which only reference older
/// revisions, because the backed out revisions' and the backout's diffs can
/// shuffle records between identical tokens (ex: punctuation), moves clear
/// removal markers, and a token moved to another file and back can get a
/// record from the other file.  Returns the line numbers of the restored tokens
/// and how many tokens are still introduced by one of `backout_revs` (the
/// backout and the backed out revisions), which happens when the backout
/// didn't exactly undo the backed out revisions.
fn restore_backed_out_records(
    lines: &mut [String],
    base: &[String],
    aligned: &[Option<u32>],
    backout_revs: &BTreeSet<&str>,
    intervening: &HashSet<String>,
) -> (BTreeSet<u32>, usize) {
    let has_new_info = |line: &str| {
        let data = HyperLineData::parse(line);
        intervening.contains(data.introduced.source_rev.as_ref())
            || data
                .predecessor
                .is_some_and(|pred| intervening.contains(pred.source_rev.as_ref()))
            || data
                .removal_marker
                .is_some_and(|marker| intervening.contains(marker.source_rev.as_ref()))
    };
    let mut restored = BTreeSet::new();
    // The sentinel (ex: when the backout restores a file that was deleted).
    if let (Some(sentinel), Some(base_sentinel)) = (lines.first(), base.first())
        && sentinel != base_sentinel
        && !has_new_info(sentinel)
    {
        lines[0] = base_sentinel.clone();
    }
    for (idx, base_lineno) in aligned.iter().enumerate() {
        let lineno = idx + 1;
        if let Some(base_line) = base_lineno.and_then(|l| base.get(l as usize))
            && lines
                .get(lineno)
                .is_some_and(|line| line != base_line && !has_new_info(line))
        {
            lines[lineno] = base_line.clone();
            restored.insert(lineno as u32);
        }
    }
    let unrestored = lines
        .iter()
        .skip(1)
        .filter(|line| {
            backout_revs.contains(HyperLineData::parse(line).introduced.source_rev.as_ref())
        })
        .count();
    (restored, unrestored)
}

/// Build the future records for all of the physical paths touched by a linear
/// revision.
///
/// `restored` has the line numbers of tokens whose records a backout restored
/// for each path (see `restore_backed_out_records`); they aren't newly added.
fn build_linear_future(
    desc: &DetailRecordRef,
    changes: &[FileChange],
    parents: &ParentAnnotated,
    restored: &HashMap<String, BTreeSet<u32>>,
) -> BTreeMap<String, FutureDetailRecord> {
    let mut records: BTreeMap<String, FutureDetailRecord> = BTreeMap::new();
    fn record_for<'r>(
        records: &'r mut BTreeMap<String, FutureDetailRecord>,
        desc: &DetailRecordRef,
        path: &str,
    ) -> &'r mut FutureDetailRecord {
        records
            .entry(path.to_string())
            .or_insert_with(|| FutureDetailRecord::new(desc.clone()))
    }

    for (idx, change) in changes.iter().enumerate() {
        let old_path = change.old_path.as_deref();
        let new_path = change.new_path.as_deref();

        // ## File-level changes
        match change.kind {
            FileChangeKind::Deleted => {
                record_for(&mut records, desc, old_path.unwrap())
                    .file_changes
                    .file_deleted = true;
            }
            FileChangeKind::Renamed => {
                record_for(&mut records, desc, old_path.unwrap())
                    .file_changes
                    .file_moved_to = new_path.map(str::to_string);
                record_for(&mut records, desc, new_path.unwrap())
                    .file_changes
                    .file_moved_from = old_path.map(str::to_string);
            }
            FileChangeKind::Copied => {
                let rec = record_for(&mut records, desc, new_path.unwrap());
                rec.file_changes.file_moved_from = old_path.map(str::to_string);
                rec.file_changes.file_copied = true;
            }
            FileChangeKind::Added | FileChangeKind::Modified => {}
        }

        // ## Removed tokens, recorded against the old physical path.
        if let Some(old_path) = old_path {
            for removed in &change.inference.removed {
                let Some(line) = parent_line(parents, old_path, removed.old_lineno) else {
                    continue;
                };
                // The ref's "%" means `old_path`, which is also what it means in
                // the future file for `old_path`, so no fixup is needed.
                let intro = HyperLineData::parse(line).introduced;
                let rec = record_for(&mut records, desc, old_path);
                let set = match removed.fate {
                    RemovedFate::Extinguished => &mut rec.extinguished_tokens,
                    RemovedFate::EvolvedInto { .. } => &mut rec.evolved_tokens,
                    RemovedFate::MovedTo { to_file, .. } if to_file as usize != idx => {
                        &mut rec.moved_out_tokens
                    }
                    // Moves within the same file don't need to be recorded.
                    RemovedFate::MovedTo { .. } => continue,
                };
                token_ref_set_insert(set, &intro.source_rev, &intro.path, intro.lineno);
            }
        }

        // ## Added and moved-in tokens, recorded against the new physical path.
        if let Some(new_path) = new_path {
            let restored = restored.get(new_path);
            for (new_idx, origin) in change.inference.origins.iter().enumerate() {
                if restored.is_some_and(|r| r.contains(&(new_idx as u32 + 1))) {
                    continue;
                }
                match *origin {
                    TokenOrigin::Added | TokenOrigin::Evolved { .. } => {
                        record_for(&mut records, desc, new_path)
                            .added_tokens
                            .insert(new_idx as u32 + 1);
                    }
                    TokenOrigin::Moved {
                        from_file,
                        old_lineno,
                    } if from_file as usize != idx => {
                        let from_path = changes[from_file as usize].old_path.as_deref().unwrap();
                        let Some(line) = parent_line(parents, from_path, old_lineno) else {
                            continue;
                        };
                        let mut intro = HyperLineData::parse(line).introduced;
                        intro.resolve_path(from_path);
                        intro.relativize_path(new_path);
                        token_ref_set_insert(
                            &mut record_for(&mut records, desc, new_path).moved_in_tokens,
                            &intro.source_rev,
                            &intro.path,
                            intro.lineno,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    records.retain(|_, rec| !rec.is_empty());
    records
}

fn process_linear_revision(
    import_helper: &mut Child,
    data: &TimelineData,
    files: &[FileChange],
    token_totals: &BTreeMap<String, TokenDeltaDetails>,
    timeline_parents: &[TimelineRepoCommit],
    timeline_commits: &TimelineCommits,
) -> BTreeMap<String, RevFileSummaryRecord> {
    let rev = data.meta.source_rev.to_string();
    let parent = timeline_parents.first();
    let desc = DetailRecordRef {
        source_rev: rev.clone(),
        syntax_rev: data.meta.syntax_rev.to_string(),
        iso_date: data.iso_date.clone(),
        backs_out: data.backed_out.clone(),
    };
    let backout_revs: BTreeSet<&str> = std::iter::once(rev.as_str())
        .chain(data.backed_out.iter().map(String::as_str))
        .collect();

    // ## Load the parent annotated files.
    let mut parent_annotated: ParentAnnotated = HashMap::new();
    if let Some(parent) = parent {
        for change in files {
            if let Some(old_path) = &change.old_path
                && !parent_annotated.contains_key(old_path)
            {
                let lines = read_path_blob(import_helper, parent, &annotated_path(old_path))
                    .map(|blob| annotated_lines(&blob))
                    .unwrap_or_default();
                parent_annotated.insert(old_path.clone(), lines);
            }
        }
    }

    // ## Deletions (before modifications so renames/swaps work out)
    for change in files {
        if matches!(
            change.kind,
            FileChangeKind::Deleted | FileChangeKind::Renamed
        ) {
            let old_path = change.old_path.as_deref().unwrap();
            delete_path(import_helper, &annotated_path(old_path));
            delete_path(import_helper, &files_delta_path(old_path));
        }
    }

    // ## Annotated files
    let mut restored_tokens: HashMap<String, BTreeSet<u32>> = HashMap::new();
    let (mut num_restored, mut num_unrestored) = (0, 0);
    for change in files {
        if let Some(new_path) = &change.new_path {
            debug!("  Writing annotated {}", new_path);
            let mut lines = build_linear_annotated(&rev, change, files, &parent_annotated);
            let base = change.restore.as_ref().and_then(|restore| {
                let base_commit = timeline_commits.get(restore.syntax_rev)?;
                let blob = read_path_blob(import_helper, &base_commit, &annotated_path(new_path))?;
                Some((annotated_lines(&blob), restore))
            });
            if let Some((base_lines, restore)) = base {
                let (restored, unrestored) = restore_backed_out_records(
                    &mut lines,
                    &base_lines,
                    &restore.aligned,
                    &backout_revs,
                    &restore.intervening,
                );
                num_restored += restored.len();
                num_unrestored += unrestored;
                restored_tokens.insert(new_path.clone(), restored);
            }
            write_inline_blob(
                import_helper,
                &annotated_path(new_path),
                join_annotated(lines).as_bytes(),
            );
        }
    }
    if !data.backed_out.is_empty() {
        info!(
            "  Backout of {}: restored {} token records; {} tokens are still attributed to the \
             backout or the backed out revisions",
            data.backed_out.join(", "),
            num_restored,
            num_unrestored
        );
    }

    // ## Future journals
    let future_records = build_linear_future(&desc, files, &parent_annotated, &restored_tokens);
    for (path, record) in future_records {
        let journal = future_path(&path);
        prepend_journal_record::<FutureHeader, FutureRecord>(
            import_helper,
            parent,
            &journal,
            &journal,
            FutureRecord::Detail(record),
        );
    }

    // ## Files-delta journals
    let mut summaries = BTreeMap::new();
    for change in files {
        let summary_path = change
            .new_path
            .as_ref()
            .or(change.old_path.as_ref())
            .unwrap();
        summaries.insert(
            summary_path.clone(),
            RevFileSummaryRecord {
                delta: change.delta.clone(),
            },
        );
        let Some(new_path) = &change.new_path else {
            continue;
        };
        let from_path = match change.kind {
            FileChangeKind::Renamed | FileChangeKind::Copied => change.old_path.as_ref().unwrap(),
            _ => new_path,
        };
        prepend_journal_record::<FileDeltaHeader, FileDeltaRecord>(
            import_helper,
            parent,
            &files_delta_path(from_path),
            &files_delta_path(new_path),
            FileDeltaRecord::Detail(FileDeltaDetailRecord {
                desc: desc.clone(),
                delta: change.delta.clone(),
            }),
        );
    }

    // ## Token journals
    for (token, delta) in token_totals {
        if !delta.has_non_move_changes() {
            continue;
        }
        let journal = token_timeline_path(token);
        prepend_journal_record::<TokenHeader, TokenDeltaRecord>(
            import_helper,
            parent,
            &journal,
            &journal,
            TokenDeltaRecord::Detail(TokenDeltaDetailRecord {
                desc: desc.clone(),
                delta: delta.clone(),
            }),
        );
    }

    summaries
}

fn process_merge_revision(
    import_helper: &mut Child,
    data: &TimelineData,
    merge: &MergeChanges,
    timeline_parents: &[TimelineRepoCommit],
) {
    let rev = data.meta.source_rev.to_string();

    // ## Annotated files
    for change in &merge.files {
        if let Some(removed_path) = &change.removed_path {
            delete_path(import_helper, &annotated_path(removed_path));
        }
    }
    for change in &merge.files {
        let Some(new_path) = &change.new_path else {
            continue;
        };
        let new_annotated = annotated_path(new_path);

        // Fast path: propagate an identical parent's annotated file.
        let identical = change
            .parents
            .iter()
            .enumerate()
            .find_map(|(i, m)| match m {
                Some(ParentMapping::Identical { path }) => Some((i, path)),
                _ => None,
            });
        if let Some((i, path)) = identical
            && path == new_path
            && let Some(oid) =
                read_path_oid(import_helper, &timeline_parents[i], &annotated_path(path))
        {
            write_existing_blob(import_helper, &new_annotated, &oid);
            continue;
        }

        let mut lines: Vec<Option<String>> = vec![None; change.num_tokens as usize + 1];
        // Process parents in reverse so the first parent takes precedence.
        for (i, mapping) in change.parents.iter().enumerate().rev() {
            let Some(mapping) = mapping else {
                continue;
            };
            let (path, unchanged) = match mapping {
                ParentMapping::Identical { path } => (path, None),
                ParentMapping::Diffed { path, unchanged } => (path, Some(unchanged)),
            };
            let Some(blob) =
                read_path_blob(import_helper, &timeline_parents[i], &annotated_path(path))
            else {
                continue;
            };
            let parent_lines = annotated_lines(&blob);
            let mut take = |new_lineno: usize, old_lineno: usize| {
                if let Some(line) = parent_lines.get(old_lineno) {
                    let mut data = HyperLineData::parse(line);
                    data.transplant(path, new_path);
                    lines[new_lineno] = Some(data.serialize());
                }
            };
            // The sentinel.
            take(0, 0);
            match unchanged {
                None => {
                    for lineno in 1..=change.num_tokens as usize {
                        take(lineno, lineno);
                    }
                }
                Some(unchanged) => {
                    for (new_idx, old_lineno) in unchanged.iter().enumerate() {
                        if let Some(old_lineno) = old_lineno {
                            take(new_idx + 1, *old_lineno as usize);
                        }
                    }
                }
            }
        }
        let lines: Vec<String> = lines
            .into_iter()
            .enumerate()
            .map(|(lineno, line)| {
                line.unwrap_or_else(|| {
                    HyperLineData::new_introduced(&rev, lineno as u32).serialize()
                })
            })
            .collect();
        write_inline_blob(
            import_helper,
            &new_annotated,
            join_annotated(lines).as_bytes(),
        );
    }

    // ## Journals
    for (path, exists) in &merge.candidate_paths {
        union_journal::<FutureHeader, FutureRecord>(
            import_helper,
            timeline_parents,
            &future_path(path),
        );
        if *exists {
            union_journal::<FileDeltaHeader, FileDeltaRecord>(
                import_helper,
                timeline_parents,
                &files_delta_path(path),
            );
        } else {
            delete_path(import_helper, &files_delta_path(path));
        }
    }
    for token in &merge.candidate_tokens {
        union_journal::<TokenHeader, TokenDeltaRecord>(
            import_helper,
            timeline_parents,
            &token_timeline_path(token),
        );
    }
}

fn write_rev_summary(rev_summary_root: &Path, summary: &RevSummaryRecord) {
    let path = rev_summary_root.join(rev_summary_path(&summary.source_rev));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_string_pretty(summary).unwrap()).unwrap();
}

/// Record in the rev-summary of `backed_out_rev` that `backout_rev` backed it
/// out.
fn mark_rev_summary_backed_out(rev_summary_root: &Path, backed_out_rev: &str, backout_rev: &str) {
    let path = rev_summary_root.join(rev_summary_path(backed_out_rev));
    let Some(mut summary) = fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str::<RevSummaryRecord>(&contents).ok())
    else {
        warn!(
            "No rev-summary for {} to mark as backed out by {}",
            backed_out_rev, backout_rev
        );
        return;
    };
    if !summary.backed_out_by.iter().any(|r| r == backout_rev) {
        summary.backed_out_by.push(backout_rev.to_string());
        write_rev_summary(rev_summary_root, &summary);
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<_> = env::args().collect();
    if args.len() != 5 {
        eprintln!(
            "Usage: {} SOURCE_REPO SYNTAX_REPO TIMELINE_REPO REV_SUMMARIES_DIR",
            args[0]
        );
        std::process::exit(1);
    }
    let source_repo_path = args[1].to_string();
    let syntax_repo_path = args[2].to_string();
    let syntax_repo = Repository::open(&syntax_repo_path).unwrap();
    let timeline_repo = Repository::open(&args[3]).unwrap();
    let rev_summary_root = PathBuf::from(&args[4]);

    let blame_ref = env::var("BLAME_REF").ok().unwrap_or("HEAD".to_string());
    let commit_limit = env::var("COMMIT_LIMIT")
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(0);
    let use_cinnabar = env::var("CINNABAR").map_or(true, |v| v != "0");

    // The syntax repo's notes map source revisions to the syntax commits, which
    // we need to resolve backouts, and the timeline repo's notes map them to the
    // timeline commits we've already written.
    let syntax_notes_refs = NotesRefs::from_env(&syntax_repo, &blame_ref);
    let syntax_mapping = SourceMapping::open(&syntax_repo, &syntax_notes_refs);
    require_notes_for_existing_branch(
        &syntax_repo,
        &blame_ref,
        &syntax_notes_refs,
        &syntax_mapping,
    );
    let notes_refs = NotesRefs::from_env(&timeline_repo, &blame_ref);
    let mapping = SourceMapping::open(&timeline_repo, &notes_refs);
    require_notes_for_existing_branch(&timeline_repo, &blame_ref, &notes_refs, &mapping);
    info!("Using source mapping notes {}", notes_refs.write);

    // We are primarily processing the "syntax" repo which is derived from the
    // "source" repo.  So start a walk in the syntax repo from the provided
    // BLAME_REF, hiding the syntax commits we've already processed (and so their
    // ancestors) and recording them as we go.  These are the processed parents
    // of the revisions we walk (plus the head if it has been processed).
    let mut processed = HashMap::new();
    let mut hide_processed = |syntax_rev: Oid| match processed_timeline_commit(
        &syntax_repo,
        &timeline_repo,
        &mapping,
        syntax_rev,
    ) {
        Some(timeline_rev) => {
            processed.insert(syntax_rev, TimelineRepoCommit::Commit(timeline_rev));
            true
        }
        None => false,
    };
    let mut walk = syntax_repo.revwalk().unwrap();
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    walk.push(syntax_repo.refname_to_id(&blame_ref).unwrap())
        .unwrap();
    let mut revs_to_process = walk
        .with_hide_callback(&mut hide_processed)
        .unwrap()
        .map(|r| r.unwrap()) // walk produces Result<git2::Oid> so we unwrap to just the Oid
        // Read the commit so we can have all the relevant revision identifiers.
        .map(|syntax_oid| {
            let commit = syntax_repo.find_commit(syntax_oid).unwrap();
            syntax_commit_to_meta(&commit)
        })
        .collect::<Vec<_>>();
    info!(
        "{} revisions to process, building on {} processed revisions",
        revs_to_process.len(),
        processed.len()
    );
    let mut timeline_commits = TimelineCommits {
        syntax_repo: &syntax_repo,
        timeline_repo: &timeline_repo,
        mapping: mapping.clone(),
        known: processed,
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

    let backout_resolver = Arc::new(BackoutTargetResolver::new(
        syntax_mapping,
        Path::new(&source_repo_path),
        use_cinnabar,
    ));

    let num_threads: usize = (num_cpus::get() - 1).max(1); // 1 for the main thread
    const COMPUTE_BUFFER_SIZE: usize = 10;

    info!("Starting {} compute threads...", num_threads);
    let mut compute_threads = Vec::with_capacity(num_threads);
    for _ in 0..num_threads {
        compute_threads.push(ComputeThread::new(
            &syntax_repo_path,
            &source_repo_path,
            backout_resolver.clone(),
        ));
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

    let mut import_helper = start_fast_import(&timeline_repo);
    let mut notes = notes_writer(&timeline_repo, &notes_refs);

    // Tracks completion count and serves as the basis for the mark <idnum>
    // assigned to each commit.
    let mut rev_done = 0;

    for rev_meta in revs_to_process.iter() {
        // Read a result. Since we hand out compute requests in round-robin order
        // and each thread processes them in FIFO order we know exactly which
        // thread is going to give us our result.
        // We assert to make sure it's the right one.
        let thread = &compute_threads[rev_done % num_threads];
        let data = thread.read_result();
        assert!(data.meta.syntax_rev == rev_meta.syntax_rev);

        // If there are more revisions that we haven't requested yet, request
        // another one from this thread.
        if compute_index < rev_count {
            thread.compute(&revs_to_process[compute_index]);
            compute_index += 1;
        }

        rev_done += 1;

        info!(
            "Transforming {} (syntax {}, hg {:?}) progress {}/{}",
            rev_meta.source_rev, rev_meta.syntax_rev, rev_meta.source_hg_rev, rev_done, rev_count
        );
        let syntax_commit = syntax_repo.find_commit(rev_meta.syntax_rev).unwrap();
        let timeline_parents = syntax_commit
            .parent_ids()
            .map(|pid| {
                timeline_commits
                    .get(pid)
                    .expect("parents are processed before their children")
            })
            .collect::<Vec<_>>();

        // Scope the import_helper borrow
        {
            // Here we write out the metadata for a new commit to the timeline
            // repo.  For details on the data format, refer to the documentation at
            // https://git-scm.com/docs/git-fast-import#_commit
            // https://git-scm.com/docs/git-fast-import#_mark
            let mut import_stream = BufWriter::new(import_helper.stdin.as_mut().unwrap());
            writeln!(import_stream, "commit {}", blame_ref).unwrap();
            writeln!(import_stream, "mark :{}", rev_done).unwrap();
            timeline_commits
                .known
                .insert(rev_meta.syntax_rev, TimelineRepoCommit::Mark(rev_done));

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
            // The syntax commit has the same author/committer as the source.
            write_role("author", &syntax_commit.author());
            write_role("committer", &syntax_commit.committer());

            let mut commit_msg = if let Some(hg_rev) = &rev_meta.source_hg_rev {
                format!(
                    "git {}\nsyntax {}\nhg {}\n",
                    rev_meta.source_rev, rev_meta.syntax_rev, hg_rev
                )
            } else {
                format!(
                    "git {}\nsyntax {}\n",
                    rev_meta.source_rev, rev_meta.syntax_rev
                )
            };
            if let Some(oldrevs) = &rev_meta.oldrevs {
                commit_msg.push_str(&format!("oldrevs {}\n", oldrevs));
            }

            write!(import_stream, "data {}\n{}\n", commit_msg.len(), commit_msg).unwrap();
            if let Some(first_parent) = timeline_parents.first() {
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
            for additional_parent in timeline_parents.iter().skip(1) {
                writeln!(import_stream, "merge {}", additional_parent).unwrap();
            }
            import_stream.flush().unwrap();
        }

        let file_deltas = match &data.changes {
            RevisionChanges::Linear {
                files,
                token_totals,
            } => process_linear_revision(
                &mut import_helper,
                &data,
                files,
                token_totals,
                &timeline_parents,
                &timeline_commits,
            ),
            RevisionChanges::Merge(merge) => {
                process_merge_revision(&mut import_helper, &data, merge, &timeline_parents);
                BTreeMap::new()
            }
        };

        // Terminate the commit so we can get its oid for the rev-summary.
        writeln!(import_helper.stdin.as_mut().unwrap()).unwrap();
        let timeline_rev = read_mark_oid(&mut import_helper, rev_done);

        write_rev_summary(
            &rev_summary_root,
            &RevSummaryRecord {
                source_rev: rev_meta.source_rev.to_string(),
                hg_rev: rev_meta.source_hg_rev.clone(),
                old_revs: rev_meta
                    .oldrevs
                    .as_deref()
                    .map(|revs| revs.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
                syntax_rev: rev_meta.syntax_rev.to_string(),
                timeline_rev: timeline_rev.clone(),
                message: data.message.clone(),
                iso_date: data.iso_date.clone(),
                unmapped_author: data.unmapped_author.clone(),
                file_deltas,
                backs_out: data.backed_out.clone(),
                backed_out_by: vec![],
            },
        );
        for backed_out_rev in &data.backed_out {
            mark_rev_summary_backed_out(
                &rev_summary_root,
                backed_out_rev,
                &rev_meta.source_rev.to_string(),
            );
        }

        // Only record the revision as processed once its rev-summary exists.
        notes.add(
            rev_meta.source_rev,
            &timeline_rev,
            syntax_commit.committer().when().seconds(),
        );
        if notes.num_pending() >= NOTES_BATCH_SIZE {
            notes.flush(import_helper.stdin.as_mut().unwrap()).unwrap();
        }

        if rev_done % 100000 == 0 {
            info!("Completed 100,000 commits, issuing checkpoint...");
            writeln!(import_helper.stdin.as_mut().unwrap(), "checkpoint").unwrap();
        }
    }

    notes.flush(import_helper.stdin.as_mut().unwrap()).unwrap();

    info!("Shutting down fast-import...");
    let exitcode = import_helper.wait().unwrap();
    if exitcode.success() {
        info!("Done!");
    } else {
        info!("Fast-import exited with {:?}", exitcode.code());
    }
}
