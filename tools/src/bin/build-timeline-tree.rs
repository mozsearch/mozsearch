// This binary consumes the "syntax" repo built by `build-syntax-token-tree.rs`
// to produce the "timeline" repo and the (non-git) "rev-summaries" directory.
//
// Usage:
//   build-timeline-tree SOURCE_REPO SYNTAX_REPO TIMELINE_REPO REV_SUMMARIES_DIR
//
// The environment variables `BLAME_REF`, `COMMIT_LIMIT`, and `CINNABAR` (set to 0
// to not ask git-cinnabar about the hg revisions in backout messages) are handled
// the same as by `build-syntax-token-tree`.  `CONSOLIDATE=0` disables the
// consolidation of journals into weekly summaries (see below), which is useful
// for checking consolidation.  Like it, we record the revisions
// we've processed in git notes (in the timeline repo), and we find the syntax
// commits of source revisions via the syntax repo's notes; see
// `source_mapping`.  `MAX_CHECKPOINTS` and `MAX_WRITTEN_BYTES` end the run
// early after that many git fast-import checkpoints or bytes of blobs (see
// `main`).
//
// ## Timeline repo contents
//
// - `annotated/PATH`: The token-centric blame for each file in the syntax
//   repo's `files/` subtree.  See `timeline_annotated.rs`.
// - `future/PATH.ndjson`: Physical-path journal of what happened to tokens in
//   and the file at PATH.  See `timeline_future.rs`.
// - `files-delta/PATH.ndjson`: Logical-path journal of per-symbol changes to
//   the file at PATH.  See `timeline_files_delta.rs`.
// - `tokens/AB/CD/TOKEN.ndjson`: Journal of changes involving TOKEN, where ABCD
//   starts a hash of TOKEN.  See `timeline_tokens.rs`.
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
// does get a rev-summary.  Merges read their parents from the repo on disk
// rather than through git-fast-import; see `MergeParents`.
//
// To determine which journals may differ between parents, we look at the files
// which differ between the first parent and each other parent in the syntax
// repo, and the tokens that differ in those files, and at the journals which
// differ between the parents' timeline commits, which the syntax repo's
// differences miss when a branch's changes cancelled out (ex: a token added
// and then removed on the branch, whose journal has the branch's records).
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
//
// ### Consolidation
//
// When a revision appends to a journal, older weeks' records get consolidated
// into weekly summary records whose details can be recovered from the journal
// versions they reference, and merges union summaries as well as details; see
// `hyperblame::consolidation`.

extern crate env_logger;
extern crate git2;
#[macro_use]
extern crate log;
extern crate num_cpus;
extern crate tools;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use chrono::{SecondsFormat, Utc};
use git2::{Delta, DiffFindOptions, ObjectType, Oid, Repository, Sort};
use serde::Serialize;
use serde::de::DeserializeOwned;

use tools::file_format::config::{
    HistorySyntaxCommitMeta, syntax_commit_to_meta, timeline_commit_to_meta,
};
use tools::file_format::history::io_helpers::{
    read_record_file_contents, record_file_contents_to_string,
};
use tools::file_format::history::rev_summaries::{
    RevFileSummaryRecord, RevSummaryRecord, file_deltas_or_totals, rev_summary_path,
};
use tools::file_format::history::syntax_files::{split_token_line, token_file_lines};
use tools::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use tools::file_format::history::timeline_annotated::{
    HyperLineData, PATH_UNCHANGED, RemovalMarker,
};
use tools::file_format::history::timeline_common::{
    ChangeKind, DetailRecordRef, FileSyntaxDelta, JournalVersionRef, TokenDeltaDetails,
    token_ref_set_insert,
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
use tools::git_notes::NotesWriter;
use tools::git_ops::{fast_import_git, git_time_to_chrono, history_compute_threads};
use tools::history_stop::{ENDED_EARLY_EXIT_CODE, STOPPED_EXIT_CODE, stop_requested};
use tools::hyperblame::backouts::{BackoutTargetResolver, find_backed_out};
use tools::hyperblame::consolidation::{
    Summarize, consolidate_appended, fill_in_pred_revs, iso_week, merge_journal_texts,
    merge_journal_versions, prepend_unconsolidated,
};
use tools::hyperblame::inference::{
    FileChangeInput, FileChangeKind, FileInference, InferenceConfig, PairingSupport, RemovedFate,
    TokenOrigin, diff_token_lines, infer_revision,
};
use tools::hyperblame::segments::{self, SPLIT_BYTES};
use tools::hyperblame::stats::compute_revision_stats;
use tools::source_mapping::{
    NOTES_BATCH_SIZE, NotesRefs, SourceMapping, notes_writer, point_branch_at,
    require_notes_for_existing_branch,
};
use tools::tree_sitter_support::cst_tokenizer::namespace_for_file;

/// A journal to write once we have the hex id of the commit's first parent: its
/// path, the first year its head keeps if it's a token journal (see
/// `write_journal`), and its contents given that id; see `FastImport`'s
/// `deferred`.
type DeferredJournal = (PathBuf, Option<i32>, Box<dyn FnOnce(&str) -> String>);

/// git-fast-import writing the timeline repo, and what we know of the tree of
/// the commit we're writing through it.
struct FastImport {
    child: Child,
    input: FastImportInput,
    output: FastImportOutput,
    /// The marks we've asked for the commit ids of (see `request_mark`) whose
    /// answers we haven't read yet, in order.
    unread_marks: VecDeque<usize>,
    /// The hex ids of the commits this run wrote whose answers we've read.
    mark_revs: HashMap<usize, String>,
    cache: TreeCache,
    /// Journals to write once we have the hex id of the commit's first parent,
    /// given it (and if they're token journals, the first year their heads
    /// keep; see `write_journal`); see `prepend_journal_record`.
    deferred: Vec<DeferredJournal>,
    readers: DiskReaders,
    /// The bytes of blobs we've given it (see MAX_WRITTEN_BYTES in `main`).
    written: u64,
}

impl FastImport {
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
    /// does only once it has processed everything before.  `mark_rev` reads the
    /// answer.
    fn request_mark(&mut self, mark: usize) {
        writeln!(self.input, "get-mark :{}", mark).unwrap();
        // (So that it can answer while we go on.)
        self.input.flush().unwrap();
        self.unread_marks.push_back(mark);
    }

    /// The hex id of the commit with `mark`, which must have been requested.
    fn mark_rev(&mut self, mark: usize) -> String {
        if !self.mark_revs.contains_key(&mark) {
            self.output();
        }
        self.mark_revs[&mark].clone()
    }

    /// The hex id of `commit`, which if it's a mark must have been requested.
    fn commit_rev(&mut self, commit: &TimelineRepoCommit) -> String {
        match commit {
            TimelineRepoCommit::Commit(oid) => oid.to_string(),
            TimelineRepoCommit::Mark(mark) => self.mark_rev(*mark),
        }
    }

    /// The hex id of `commit` if we have it without waiting for git
    /// fast-import.
    fn known_commit_rev(&mut self, commit: &TimelineRepoCommit) -> Option<String> {
        let TimelineRepoCommit::Mark(mark) = commit else {
            return Some(commit.to_string());
        };
        if !self.mark_revs.contains_key(mark) {
            self.output.take_available();
            while !self.unread_marks.is_empty() && self.output.has_line() {
                let mut line = String::new();
                self.output.read_line(&mut line).unwrap();
                let answered = self.unread_marks.pop_front().unwrap();
                self.mark_revs.insert(answered, line.trim().to_string());
            }
        }
        self.mark_revs.get(mark).cloned()
    }

    /// Check that `path` isn't one of the journals we've put off writing, which
    /// we'd otherwise read or write out of order.
    fn check_not_deferred(&self, path: &Path) {
        assert!(
            !self
                .deferred
                .iter()
                .any(|(deferred, _, _)| deferred == path),
            "{} was used after its write was deferred",
            path.display()
        );
    }

    /// Close git fast-import's input once it has everything we wrote, and wait
    /// for it to exit.
    fn finish(&mut self) -> std::process::ExitStatus {
        self.input.finish().unwrap();
        self.child.wait().unwrap()
    }
}

/// How many bytes we let `FastImportInput` queue for git fast-import.
const MAX_QUEUED_BYTES: usize = 256 << 20;
/// `FastImportInput` sends writes smaller than this together.
const SMALL_WRITES_BYTES: usize = 64 << 10;

/// Our end of git fast-import's input, which hands what we write to a thread
/// which writes it to the pipe, so that we can go on preparing the next
/// journals while git fast-import takes in the last ones.  (Writing to its
/// 64 KiB pipe ourselves, in the full firefox reblame we spent a fifth to a
/// quarter of our time waiting for room in it, and git fast-import a quarter
/// to a third of its time waiting for input.)  What we've written gets to git
/// fast-import before any requests we write later, and `FastImport::output`
/// sends everything on before we wait for an answer.
struct FastImportInput {
    /// Small writes, to send together.
    buffer: Vec<u8>,
    sender: Option<Sender<Vec<u8>>>,
    /// The bytes sent which the thread hasn't written yet, and whether it
    /// failed to write.
    queued: Arc<(Mutex<(usize, bool)>, Condvar)>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl FastImportInput {
    fn new(mut stdin: ChildStdin) -> Self {
        let (sender, receiver) = channel::<Vec<u8>>();
        let queued = Arc::new((Mutex::new((0, false)), Condvar::new()));
        let thread_queued = queued.clone();
        let thread = thread::spawn(move || -> io::Result<()> {
            let (lock, condvar) = &*thread_queued;
            for chunk in receiver {
                let result = stdin.write_all(&chunk);
                let mut state = lock.lock().unwrap();
                state.0 -= chunk.len();
                state.1 |= result.is_err();
                condvar.notify_all();
                result?;
            }
            // Dropping `stdin` closes git fast-import's input.
            Ok(())
        });
        FastImportInput {
            buffer: vec![],
            sender: Some(sender),
            queued,
            thread: Some(thread),
        }
    }

    fn send(&mut self, chunk: Vec<u8>) {
        if chunk.is_empty() {
            return;
        }
        let (lock, condvar) = &*self.queued;
        let mut state = lock.lock().unwrap();
        while state.0 > 0 && state.0 + chunk.len() > MAX_QUEUED_BYTES && !state.1 {
            state = condvar.wait(state).unwrap();
        }
        state.0 += chunk.len();
        drop(state);
        // (This fails if the thread stopped because it failed to write.)
        self.sender
            .as_ref()
            .unwrap()
            .send(chunk)
            .expect("failed to write to git fast-import");
    }

    fn send_buffer(&mut self) {
        let chunk = std::mem::take(&mut self.buffer);
        self.send(chunk);
    }

    /// Write everything sent, and close git fast-import's input.
    fn finish(&mut self) -> io::Result<()> {
        self.send_buffer();
        drop(self.sender.take());
        match self.thread.take() {
            Some(thread) => thread.join().unwrap(),
            None => Ok(()),
        }
    }
}

impl Write for FastImportInput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() >= SMALL_WRITES_BYTES {
            self.send_buffer();
            self.send(buf.to_vec());
        } else {
            self.buffer.extend_from_slice(buf);
            if self.buffer.len() >= SMALL_WRITES_BYTES {
                self.send_buffer();
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_buffer();
        Ok(())
    }
}

/// Our end of git fast-import's output, which a thread reads as it comes, so
/// that we can see whether it has answered without waiting for it (see
/// `FastImport::known_commit_rev`).
struct FastImportOutput {
    chunks: Receiver<Vec<u8>>,
    /// What we've taken from the thread and not read.
    buffer: Vec<u8>,
    pos: usize,
}

impl FastImportOutput {
    fn new(mut stdout: ChildStdout) -> Self {
        let (sender, chunks) = channel();
        thread::spawn(move || {
            let mut chunk = vec![0; 64 << 10];
            // (Until git fast-import exits, or we stop reading.)
            while let Ok(len @ 1..) = stdout.read(&mut chunk) {
                if sender.send(chunk[..len].to_vec()).is_err() {
                    break;
                }
            }
        });
        FastImportOutput {
            chunks,
            buffer: vec![],
            pos: 0,
        }
    }

    /// Take what the thread has read so far, without waiting for more.
    fn take_available(&mut self) {
        while let Ok(chunk) = self.chunks.try_recv() {
            self.buffer.drain(..self.pos);
            self.pos = 0;
            self.buffer.extend_from_slice(&chunk);
        }
    }

    /// Whether what we've taken has a whole line we haven't read.
    fn has_line(&self) -> bool {
        self.buffer[self.pos..].contains(&b'\n')
    }
}

impl Read for FastImportOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let len = available.len().min(buf.len());
        buf[..len].copy_from_slice(&available[..len]);
        self.consume(len);
        Ok(len)
    }
}

impl BufRead for FastImportOutput {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos == self.buffer.len() {
            // (Nothing more once git fast-import exits.)
            if let Ok(chunk) = self.chunks.recv() {
                self.buffer = chunk;
                self.pos = 0;
            }
        }
        Ok(&self.buffer[self.pos..])
    }

    fn consume(&mut self, amount: usize) {
        self.pos += amount;
    }
}

/// The contents of paths in the first parent of the commit being written, from
/// what we wrote to them and read from them, carried over from each commit to
/// its child when that's the next commit we write, as it almost always is.  So
/// a revision rarely needs to read its parent's annotated files and journals
/// back through git fast-import, which answers each request only after it has
/// processed everything we sent before it, making us and it take turns: when
/// building 2 years of firefox history, the main thread spent 80% of its time
/// waiting on it, and it spent over 40% of its time on our reads.
///
/// What the entries don't have is mostly paths the run hasn't touched yet (ex:
/// the journals of tokens it hasn't seen), which the parent has the same
/// version of as `base`, a commit on disk which the cache started from, and
/// which `DiskReaders` read from it without waiting on git fast-import.
///
/// All changes to the commit being written must go through `write_inline_blob`,
/// `write_existing_blob` and `delete_path`, which record them in `pending`.
#[derive(Default)]
struct TreeCache {
    /// The first parent of the commit being written, which `entries` describe.
    parent: Option<TimelineRepoCommit>,
    entries: HashMap<PathBuf, CachedPath>,
    /// The total size of the blobs in `entries`.
    bytes: usize,
    /// The paths the commit being written has changed, with their new contents
    /// if we know them.
    pending: HashMap<PathBuf, Option<CachedPath>>,
    /// A commit on disk which `parent` descends from through commits we wrote
    /// (or is), if we know of one, and the paths they changed, so that the
    /// parent has the same version of every other path.
    base: Option<Oid>,
    changed: HashSet<PathBuf>,
    hits: usize,
    misses: usize,
    clears: usize,
    disk_reads: usize,
    fast_import_reads: usize,
}

#[derive(Clone)]
enum CachedPath {
    Blob(Rc<[u8]>),
    Missing,
}

/// How much blob data `TreeCache` holds before we clear it (which is simpler
/// than tracking use, and it quickly fills with the hot paths again).
const TREE_CACHE_MAX_BYTES: usize = 8 << 30;

impl TreeCache {
    /// Whether the entries describe `parent`.
    fn describes(&self, parent: Option<&TimelineRepoCommit>) -> bool {
        parent.is_some() && self.parent.as_ref() == parent
    }

    /// Start writing a commit whose first parent is `parent`, whose id is
    /// `on_disk` if it's on disk (which only matters if the entries don't
    /// describe it).
    fn begin(&mut self, parent: Option<&TimelineRepoCommit>, on_disk: Option<Oid>) {
        assert!(self.pending.is_empty());
        if !self.describes(parent) {
            self.entries.clear();
            self.bytes = 0;
            self.base = on_disk;
            self.changed.clear();
        }
        self.parent = parent.copied();
    }

    /// Finish writing the commit with mark `mark`, which becomes the parent the
    /// entries describe.  Returns whether that cleared the entries, after which
    /// the caller should `rebase` (once the commit is on disk), since the
    /// entries were the only versions of `changed` we could read without asking
    /// git fast-import: in the full firefox reblame, a chunk read 633,255 paths
    /// through it after a clear, and took 36 minutes rather than ~6.
    fn end(&mut self, mark: usize) -> bool {
        for (path, change) in self.pending.drain() {
            if self.base.is_some() {
                self.changed.insert(path.clone());
            }
            let old = match change {
                Some(contents) => {
                    if let CachedPath::Blob(blob) = &contents {
                        self.bytes += blob.len();
                    }
                    self.entries.insert(path, contents)
                }
                None => self.entries.remove(&path),
            };
            if let Some(CachedPath::Blob(blob)) = old {
                self.bytes -= blob.len();
            }
        }
        self.parent = Some(TimelineRepoCommit::Mark(mark));
        if self.bytes > TREE_CACHE_MAX_BYTES {
            self.entries.clear();
            self.bytes = 0;
            self.clears += 1;
            return true;
        }
        false
    }

    /// Make `base`, the parent (which must now be on disk), the base.
    fn rebase(&mut self, base: Oid) {
        self.base = Some(base);
        self.changed.clear();
    }

    /// Whether a read from `from` sees the entries (as opposed to a change the
    /// commit being written made, or another commit).
    fn reads_parent(&self, from: ReadFrom, path: &Path) -> bool {
        match from {
            ReadFrom::Active => !self.pending.contains_key(path),
            ReadFrom::Commit(commit) => self.parent.as_ref() == Some(commit),
        }
    }

    /// The commit on disk with the same version of `path` as `from`, if any.
    fn on_disk(&self, from: ReadFrom, path: &Path) -> Option<Oid> {
        self.base
            .filter(|_| self.reads_parent(from, path) && !self.changed.contains(path))
    }

    /// The contents of `path` read from `from`, if we know them: `Some(None)`
    /// means the path doesn't exist.
    fn get(&mut self, from: ReadFrom, path: &Path) -> Option<Option<Rc<[u8]>>> {
        let found = match from {
            ReadFrom::Active => match self.pending.get(path) {
                Some(change) => change.as_ref(),
                None => self.entries.get(path),
            },
            ReadFrom::Commit(_) if self.reads_parent(from, path) => self.entries.get(path),
            ReadFrom::Commit(_) => None,
        };
        let found = found.map(|contents| match contents {
            CachedPath::Blob(blob) => Some(blob.clone()),
            CachedPath::Missing => None,
        });
        if found.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        found
    }

    /// Record the contents of `path` read from `from`.
    fn note_read(&mut self, from: ReadFrom, path: &Path, blob: Option<&Rc<[u8]>>) {
        if !self.reads_parent(from, path) {
            return;
        }
        let contents = match blob {
            Some(blob) => {
                self.bytes += blob.len();
                CachedPath::Blob(blob.clone())
            }
            None => CachedPath::Missing,
        };
        if let Some(CachedPath::Blob(old)) = self.entries.insert(path.to_path_buf(), contents) {
            self.bytes -= old.len();
        }
    }
}

/// How many `DiskReaders` threads read at once.
const DISK_READER_THREADS: usize = 8;

/// A disk reader thread's requests (a commit and paths in it) and answers (the
/// paths' blobs, if any); see `DiskReaders`.
type DiskReaderThread = (Sender<(Oid, Vec<PathBuf>)>, Receiver<Vec<Option<Vec<u8>>>>);

/// Threads which read paths from commits on disk with git2, for `TreeCache`.
/// Reading through git fast-import took as long, but meant waiting for it to
/// catch up, and it was usually the busier of us.
struct DiskReaders {
    /// Each thread's requests and answers.
    threads: Vec<DiskReaderThread>,
}

impl DiskReaders {
    fn new(repo_path: &Path) -> Self {
        let threads = (0..DISK_READER_THREADS)
            .map(|_| {
                let (request_sender, requests) = channel::<(Oid, Vec<PathBuf>)>();
                let (answer_sender, answers) = channel();
                let repo_path = repo_path.to_path_buf();
                thread::spawn(move || {
                    let repo = Repository::open(&repo_path).unwrap();
                    for (commit, paths) in requests {
                        let tree = repo.find_commit(commit).unwrap().tree().unwrap();
                        let blobs = paths
                            .iter()
                            .map(|path| match tree.get_path(path) {
                                Ok(entry) => {
                                    Some(repo.find_blob(entry.id()).unwrap().content().to_vec())
                                }
                                Err(e) if e.code() == git2::ErrorCode::NotFound => None,
                                Err(e) => panic!("failed to read {}: {}", path.display(), e),
                            })
                            .collect();
                        if answer_sender.send(blobs).is_err() {
                            break;
                        }
                    }
                });
                (request_sender, answers)
            })
            .collect();
        DiskReaders { threads }
    }

    /// Start reading `paths` from `commit`, returning how many threads are
    /// reading them, for `finish`.
    fn start(&self, commit: Oid, paths: &[PathBuf]) -> usize {
        let per_thread = paths.len().div_ceil(self.threads.len()).max(1);
        let mut started = 0;
        for (chunk, (requests, _)) in paths.chunks(per_thread).zip(&self.threads) {
            requests.send((commit, chunk.to_vec())).unwrap();
            started += 1;
        }
        started
    }

    /// The contents of the paths of the last `start`, in order, `None` for
    /// those which don't exist.
    fn finish(&self, started: usize) -> Vec<Option<Vec<u8>>> {
        self.threads[..started]
            .iter()
            .flat_map(|(_, answers)| answers.recv().expect("a disk reader thread failed"))
            .collect()
    }
}

/// Starts the git-fast-import subcommand, to which data
/// is fed for adding to the blame repo. Refer to
/// https://git-scm.com/docs/git-fast-import for detailed
/// documentation on git-fast-import.
fn start_fast_import(git_repo: &Repository) -> FastImport {
    // Note that we use the `--force` flag here, because there
    // are cases where the blame repo branch we're building was
    // initialized from some other branch (e.g. gecko-dev beta
    // being initialized from gecko-dev master) just to take
    // advantage of work already done (the commits shared between
    // beta and master). After writing the new blame information
    // (for beta) the new branch head (beta) is not going to be a
    // a descendant of the original (master), and we need `--force`
    // to make git-fast-import allow that.
    let mut child = fast_import_git()
        // We rewrite big files (ex: journals) a lot, and compressing them was
        // half of git fast-import's time in the full firefox reblame, but even
        // the fastest zlib level only made them a quarter smaller.  So it just
        // stores them, which made 4,000 recent firefox revisions 45% faster,
        // and scripts/build-history.py's repacks compress them.
        .arg("-c")
        .arg("core.compression=0")
        // git fast-import tries to delta each blob against the previous blob it
        // was given, which for us is almost always a different file, so that's
        // mostly wasted time, and reading the blobs back means resolving the
        // deltas which do work out.  So blobs over 1k are stored whole (the
        // option is `core.bigFileThreshold`, since fast-import's
        // --big-file-threshold is ignored as of git 2.55), and repacking deltas
        // them: on a firefox window, this made build-timeline-tree 22% faster,
        // and what it wrote twice as big until it's repacked.
        .arg("-c")
        .arg("core.bigFileThreshold=1k")
        .arg("fast-import")
        .arg("--force")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(git_repo.path())
        .spawn()
        .unwrap();
    let input = FastImportInput::new(child.stdin.take().unwrap());
    let output = FastImportOutput::new(child.stdout.take().unwrap());
    FastImport {
        child,
        input,
        output,
        unread_marks: VecDeque::new(),
        mark_revs: HashMap::new(),
        cache: TreeCache::default(),
        deferred: vec![],
        readers: DiskReaders::new(git_repo.path()),
        written: 0,
    }
}

/// When writing to a git-fast-import stream, we can insert temporary
/// names (called "marks") for commits as we create them. This allows
/// us to refer to them later in the stream without knowing the final
/// oid for that commit. This enum abstracts over that, so bits of code
/// can refer to a specific commit that is either pre-existing in the
/// blame repo (and for which we have an oid) or that was written
/// earlier in the stream (and has a mark).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Where to read timeline repo data from.  Reading from the commit being
/// written (`Active`) gives the first parent's version of any path not yet
/// modified in it, since it starts out with the first parent's tree, and is
/// much cheaper than reading from the first parent itself, which makes
/// git-fast-import load the first parent's trees along the path again.
#[derive(Clone, Copy)]
enum ReadFrom<'a> {
    Active,
    Commit(&'a TimelineRepoCommit),
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

/// Read the oid of the object at the given path in the given
/// commit. Returns None if there is no such object.
/// Documentation for the fast-import command used is at
/// https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Readingfromanamedtree
fn read_path_oid(import_helper: &mut FastImport, from: ReadFrom, path: &Path) -> Option<String> {
    let input = &mut import_helper.input;
    match from {
        ReadFrom::Active => writeln!(input, "ls {}", quote_path(path)),
        ReadFrom::Commit(commit) => writeln!(input, "ls {} {}", commit, sanitize(path)),
    }
    .unwrap();
    read_ls_response(import_helper.output())
}

/// Read git fast-import's answer to an `ls`.
fn read_ls_response(reader: &mut impl BufRead) -> Option<String> {
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
fn read_blob(import_helper: &mut FastImport, oid: &str) -> Vec<u8> {
    writeln!(import_helper.input, "cat-blob {}", oid).unwrap();
    read_cat_blob_response(import_helper.output())
}

/// Read git fast-import's answer to a `cat-blob`.
fn read_cat_blob_response(reader: &mut impl BufRead) -> Vec<u8> {
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
fn read_path_blob(import_helper: &mut FastImport, from: ReadFrom, path: &Path) -> Option<Rc<[u8]>> {
    if matches!(from, ReadFrom::Active) {
        import_helper.check_not_deferred(path);
    }
    if let Some(found) = import_helper.cache.get(from, path) {
        return found;
    }
    let blob = match import_helper.cache.on_disk(from, path) {
        Some(commit) => {
            import_helper.cache.disk_reads += 1;
            let readers = &import_helper.readers;
            let started = readers.start(commit, &[path.to_path_buf()]);
            readers.finish(started).pop().unwrap().map(Rc::from)
        }
        None => {
            import_helper.cache.fast_import_reads += 1;
            read_path_oid(import_helper, from, path)
                .map(|oid| Rc::from(read_blob(import_helper, &oid)))
        }
    };
    import_helper.cache.note_read(from, path, blob.as_ref());
    blob
}

/// Read the first parent's version of each of `paths` that the cache doesn't
/// have: from disk where we can (see `TreeCache`), and otherwise asking git
/// fast-import about many at once, so that we wait for it to catch up once per
/// batch rather than once per path.  This must be done before the commit being
/// written changes anything.
fn prefetch_parent_paths(import_helper: &mut FastImport, paths: Vec<PathBuf>) {
    // (The batches keep how many answers git fast-import has to write before
    // we read them small.)
    const MAX_BATCH_PATHS: usize = 64;
    const MAX_BATCH_BYTES: usize = 16 * 1024;

    assert!(import_helper.cache.pending.is_empty());
    let mut missing: Vec<PathBuf> = paths
        .into_iter()
        .filter(|path| !import_helper.cache.entries.contains_key(path))
        .collect();
    missing.sort();
    missing.dedup();
    let cache = &mut import_helper.cache;
    let (on_disk, missing): (Vec<PathBuf>, Vec<PathBuf>) = missing
        .into_iter()
        .partition(|path| cache.on_disk(ReadFrom::Active, path).is_some());
    cache.disk_reads += on_disk.len();
    cache.fast_import_reads += missing.len();
    // (The threads read from disk while we wait on git fast-import.)
    let started = match cache.base {
        Some(base) => import_helper.readers.start(base, &on_disk),
        None => 0,
    };

    let mut rest = &missing[..];
    while !rest.is_empty() {
        let mut len = 0;
        let mut bytes = 0;
        while len < rest.len() && len < MAX_BATCH_PATHS && bytes < MAX_BATCH_BYTES {
            bytes += rest[len].as_os_str().len();
            len += 1;
        }
        let (batch, remaining) = rest.split_at(len);
        rest = remaining;

        let requests: String = batch
            .iter()
            .map(|path| format!("ls {}\n", quote_path(path)))
            .collect();
        import_helper.input.write_all(requests.as_bytes()).unwrap();
        let reader = import_helper.output();
        let oids: Vec<Option<String>> = batch.iter().map(|_| read_ls_response(reader)).collect();
        let requests: String = oids
            .iter()
            .flatten()
            .map(|oid| format!("cat-blob {}\n", oid))
            .collect();
        import_helper.input.write_all(requests.as_bytes()).unwrap();
        let reader = import_helper.output();
        let blobs: Vec<Option<Rc<[u8]>>> = oids
            .iter()
            .map(|oid| {
                oid.as_ref()
                    .map(|_| Rc::from(read_cat_blob_response(reader)))
            })
            .collect();
        for (path, blob) in batch.iter().zip(blobs) {
            import_helper
                .cache
                .note_read(ReadFrom::Active, path, blob.as_ref());
        }
    }

    let blobs = import_helper.readers.finish(started);
    for (path, blob) in on_disk.iter().zip(blobs) {
        let blob: Option<Rc<[u8]>> = blob.map(Rc::from);
        import_helper
            .cache
            .note_read(ReadFrom::Active, path, blob.as_ref());
    }
}

fn write_inline_blob(import_helper: &mut FastImport, path: &Path, contents: &[u8]) {
    // For the inline data format documentation, refer to
    // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Inlinedataformat
    // https://git-scm.com/docs/git-fast-import#Documentation/git-fast-import.txt-Exactbytecountformat
    import_helper.check_not_deferred(path);
    let import_stream = &mut import_helper.input;
    writeln!(import_stream, "M 100644 inline {}", sanitize(path)).unwrap();
    writeln!(import_stream, "data {}", contents.len()).unwrap();
    import_stream.write_all(contents).unwrap();
    import_helper.written += contents.len() as u64;
    // We skip the optional trailing LF character here since in practice it
    // wasn't particularly useful for debugging.
    import_helper.cache.pending.insert(
        path.to_path_buf(),
        Some(CachedPath::Blob(Rc::from(contents))),
    );
}

fn write_existing_blob(import_helper: &mut FastImport, path: &Path, oid: &str) {
    import_helper.check_not_deferred(path);
    writeln!(import_helper.input, "M 100644 {} {}", oid, sanitize(path)).unwrap();
    import_helper.cache.pending.insert(path.to_path_buf(), None);
}

fn delete_path(import_helper: &mut FastImport, path: &Path) {
    import_helper.check_not_deferred(path);
    writeln!(import_helper.input, "D {}", sanitize(path)).unwrap();
    import_helper
        .cache
        .pending
        .insert(path.to_path_buf(), Some(CachedPath::Missing));
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
fn unchanged_mapping(old_lines: &[&str], new_lines: &[&str]) -> Vec<Option<u32>> {
    let mut unchanged = vec![None; new_lines.len()];
    for op in diff_token_lines(old_lines, new_lines) {
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
                    let unchanged = unchanged_mapping(&old_lines, &new_lines);
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
            for op in diff_token_lines(&old_lines, &new_lines) {
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
        preprocess_merge(syntax_repo, &commit)?
    };

    let mut backed_out = vec![];
    if let RevisionChanges::Linear { files, .. } = &mut changes {
        let targets = find_backed_out(syntax_repo, backout_resolver, &commit, &message);
        if !targets.is_empty() {
            add_restore_bases(syntax_repo, &commit, &targets, files)?;
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

/// The compute threads, which take the revisions to preprocess (by their index
/// in the revisions to process) from a shared queue, so that a revision which
/// takes a while (ex: a merge whose parents differ a lot) only delays itself,
/// rather than the revisions queued behind it on its thread, which is how it
/// was when each thread had its own queue.  (On a firefox window, the main
/// thread spent half of its time waiting on such revisions.)  Results arrive in
/// any order, and `result` returns them in order.
struct ComputePool {
    query_tx: Sender<(usize, HistorySyntaxCommitMeta)>,
    response_rx: Receiver<(usize, TimelineData)>,
    /// Results which arrived before the ones before them.
    early: HashMap<usize, TimelineData>,
}

impl ComputePool {
    fn new(
        num_threads: usize,
        syntax_repo_path: &str,
        source_repo_path: &str,
        backout_resolver: Arc<BackoutTargetResolver>,
    ) -> Self {
        let (query_tx, query_rx) = channel();
        let query_rx = Arc::new(Mutex::new(query_rx));
        let (response_tx, response_rx) = channel();
        for _ in 0..num_threads {
            let query_rx = query_rx.clone();
            let response_tx = response_tx.clone();
            let syntax_repo_path = syntax_repo_path.to_string();
            let source_repo_path = source_repo_path.to_string();
            let backout_resolver = backout_resolver.clone();
            thread::spawn(move || {
                compute_thread_main(
                    query_rx,
                    response_tx,
                    syntax_repo_path,
                    source_repo_path,
                    backout_resolver,
                );
            });
        }
        ComputePool {
            query_tx,
            response_rx,
            early: HashMap::new(),
        }
    }

    fn compute(&self, index: usize, rev_meta: &HistorySyntaxCommitMeta) {
        self.query_tx.send((index, rev_meta.clone())).unwrap();
    }

    /// The result for the revision with the given index.
    fn result(&mut self, index: usize) -> TimelineData {
        if let Some(result) = self.early.remove(&index) {
            return result;
        }
        let mut waited = false;
        loop {
            let (i, result) = match self.response_rx.try_recv() {
                Ok(response) => response,
                Err(_) => {
                    if !waited {
                        info!("Waiting on compute, work on optimizing that...");
                        waited = true;
                    }
                    self.response_rx.recv().unwrap()
                }
            };
            if i == index {
                return result;
            }
            self.early.insert(i, result);
        }
    }
}

fn compute_thread_main(
    query_rx: Arc<Mutex<Receiver<(usize, HistorySyntaxCommitMeta)>>>,
    response_tx: Sender<(usize, TimelineData)>,
    syntax_repo_path: String,
    source_repo_path: String,
    backout_resolver: Arc<BackoutTargetResolver>,
) {
    let syntax_repo = Repository::open(syntax_repo_path).unwrap();
    let source_repo = Repository::open(source_repo_path).unwrap();
    let config = InferenceConfig::default();
    loop {
        // (The lock is only held while waiting for the next revision.)
        let Ok((index, rev)) = query_rx.lock().unwrap().recv() else {
            break;
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            thread_preprocess_revision(&syntax_repo, &source_repo, &backout_resolver, &rev, &config)
                .unwrap()
        }));
        match result {
            Ok(result) => response_tx.send((index, result)).unwrap(),
            // The other threads keep the response channel open, so the main
            // thread would wait for this result forever.
            Err(_) => {
                error!("Preprocessing {} failed", rev.source_rev);
                std::process::exit(101);
            }
        }
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

/// Load the records of a journal version through git fast-import, which (unlike
/// the repo on disk) has the commits written earlier in this run.
fn load_journal_version<H: DeserializeOwned + Default, R: DeserializeOwned>(
    import_helper: &mut FastImport,
    version: &JournalVersionRef,
) -> Result<Vec<R>, String> {
    let oid = Oid::from_str(&version.timeline_rev).map_err(|e| e.to_string())?;
    let commit = TimelineRepoCommit::Commit(oid);
    let path = Path::new(&version.path);
    let Some(head) = read_path_blob(import_helper, ReadFrom::Commit(&commit), path) else {
        return Ok(vec![]);
    };
    let head = std::str::from_utf8(&head).unwrap();
    let mut segment_texts = vec![];
    for year in segments::segment_years(head.lines().next().unwrap_or("")) {
        let segment = segments::segment_path(path, year);
        if let Some(text) = read_path_blob(import_helper, ReadFrom::Commit(&commit), &segment) {
            segment_texts.push(String::from_utf8(text.to_vec()).unwrap());
        }
    }
    let text = segments::join(head, segment_texts.iter().map(String::as_str));
    Ok(read_record_file_contents::<H, R>(text.as_bytes())
        .map(|(_, records): (H, Vec<R>)| records)
        .unwrap_or_default())
}

/// `segments::SPLIT_BYTES`, or HISTORY_SPLIT_BYTES (for tests, whose journals
/// are small).
fn split_bytes() -> usize {
    static SPLIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SPLIT.get_or_init(|| {
        env::var("HISTORY_SPLIT_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(SPLIT_BYTES)
    })
}

/// The first ISO year the heads of the token journals a revision dated
/// `iso_date` writes keep; see `hyperblame::segments`.
fn keep_from(iso_date: &str) -> Option<i32> {
    iso_week(iso_date).map(|(year, _)| segments::keep_from(year))
}

/// Write a journal's new head, given the first year it keeps if it's a token
/// journal: then once it's bigger than `split_bytes` (or has segments), the
/// records of earlier years go into their segments; see `hyperblame::segments`.
fn write_journal(
    import_helper: &mut FastImport,
    path: &Path,
    contents: &str,
    keep_from: Option<i32>,
) {
    let header = contents
        .split_once('\n')
        .map_or(contents, |(header, _)| header);
    let split =
        keep_from.filter(|_| contents.len() > split_bytes() || header.contains("\"segments\""));
    let Some((head, old)) =
        split.and_then(|keep_from| segments::take_old_years(contents, keep_from))
    else {
        write_inline_blob(import_helper, path, contents.as_bytes());
        return;
    };
    let mut years = segments::segment_years(header);
    for (year, lines) in old {
        let segment = segments::segment_path(path, year);
        let existing = if years.contains(&year) {
            read_path_blob(import_helper, ReadFrom::Active, &segment)
        } else {
            years.push(year);
            None
        };
        let existing = existing
            .as_deref()
            .map(|text| std::str::from_utf8(text).unwrap());
        let text = segments::add_to_segment(existing, &lines);
        write_inline_blob(import_helper, &segment, text.as_bytes());
    }
    years.sort_unstable_by(|a, b| b.cmp(a));
    let header = segments::with_segment_years(header, &years);
    let head = match head.split_once('\n') {
        Some((_, records)) => format!("{}\n{}", header, records),
        None => header,
    };
    write_inline_blob(import_helper, path, head.as_bytes());
}

/// What a linear revision needs to consolidate the journals it appends to.
struct Consolidation<'a> {
    /// The revision's date.
    iso_date: &'a str,
}

/// Prepend a record to the journal at `from_path` in the parent timeline commit
/// and write it to `to_path` in the new commit, consolidating it if requested.
/// `touched` has the paths already deleted or written in the new commit, which
/// we can't read the parent's version of from it.  Returns the number of
/// summary records written.
///
/// Summaries reference the parent's version of the journal, so when git
/// fast-import hasn't given us the parent's id yet, a journal with new ones is
/// written once it has, or at the end of the revision (see
/// `write_deferred_journals`), rather than waiting for it to catch up in the
/// middle of the revision.
#[allow(clippy::too_many_arguments)]
fn prepend_journal_record<
    H: DeserializeOwned + Default + Serialize + 'static,
    R: Summarize + 'static,
>(
    import_helper: &mut FastImport,
    parent: Option<&TimelineRepoCommit>,
    from_path: &Path,
    to_path: &Path,
    record: R,
    consolidation: Option<&Consolidation>,
    keep_from: Option<i32>,
    touched: &mut HashSet<PathBuf>,
) -> usize {
    let blob = parent.and_then(|parent| {
        let from = if touched.contains(from_path) {
            ReadFrom::Commit(parent)
        } else {
            ReadFrom::Active
        };
        read_path_blob(import_helper, from, from_path)
    });
    touched.insert(to_path.to_path_buf());
    // Almost always, no weeks get summarized, and the journal's other records
    // stay as they are.
    if let Some(blob) = &blob {
        let now = consolidation.map(|consolidation| consolidation.iso_date);
        if let Some(contents) =
            prepend_unconsolidated(std::str::from_utf8(blob).unwrap(), &record, now)
        {
            write_journal(import_helper, to_path, &contents, keep_from);
            return 0;
        }
    }
    let (header, mut records): (H, Vec<R>) = blob
        .and_then(|blob| read_record_file_contents(&blob))
        .unwrap_or_else(|| (H::default(), vec![]));
    records.insert(0, record);
    let mut num_summaries = 0;
    if let (Some(consolidation), Some(_)) = (consolidation, parent) {
        // (Without the parent's id; see `fill_in_pred_revs`.)
        let pred = JournalVersionRef {
            timeline_rev: String::new(),
            path: from_path.to_string_lossy().into_owned(),
        };
        num_summaries = consolidate_appended(
            &mut records,
            consolidation.iso_date,
            &pred,
            &mut |version| load_journal_version::<H, R>(import_helper, version),
        )
        .unwrap();
    }
    if num_summaries > 0 {
        let parent = parent.unwrap();
        match import_helper.known_commit_rev(parent) {
            Some(parent_rev) => {
                write_deferred_journals(import_helper, parent);
                fill_in_pred_revs(&mut records, &parent_rev);
            }
            None => {
                let contents = move |parent_rev: &str| {
                    fill_in_pred_revs(&mut records, parent_rev);
                    record_file_contents_to_string(&header, &records)
                };
                import_helper
                    .deferred
                    .push((to_path.to_path_buf(), keep_from, Box::new(contents)));
                return num_summaries;
            }
        }
    }
    let contents = record_file_contents_to_string(&header, &records);
    write_journal(import_helper, to_path, &contents, keep_from);
    num_summaries
}

/// Write the journals `prepend_journal_record` deferred, given the commit's
/// first parent, waiting for its id if need be.
fn write_deferred_journals(import_helper: &mut FastImport, parent: &TimelineRepoCommit) {
    let deferred = std::mem::take(&mut import_helper.deferred);
    if deferred.is_empty() {
        return;
    }
    let parent_rev = import_helper.commit_rev(parent);
    for (path, keep_from, contents) in deferred {
        write_journal(import_helper, &path, &contents(&parent_rev), keep_from);
    }
}

/// The parent commits of a merge, which we read from the timeline repo on disk
/// with git2 rather than through git fast-import (which requires a checkpoint
/// if they were written by this run).  Unlike linear revisions, which can read
/// most things from the commit being written (see `ReadFrom`), merges read
/// from other commits, and git fast-import reloads a commit's trees along the
/// path for every such read.  Loading a tree looks up the name of every entry in
/// a hash table which has a fixed number of buckets (4451, as of git 2.53) and
/// holds every name git fast-import has seen, which gets very slow with
/// millions of names: ex: over 10s per journal for all of firefox-main.
struct MergeParents<'r> {
    repo: &'r Repository,
    trees: Vec<git2::Tree<'r>>,
}

impl<'r> MergeParents<'r> {
    /// The parents with the given hex ids.
    fn new(repo: &'r Repository, parent_revs: &[String]) -> Self {
        let trees = parent_revs
            .iter()
            .map(|rev| {
                repo.find_commit(Oid::from_str(rev).unwrap())
                    .and_then(|commit| commit.tree())
                    .unwrap()
            })
            .collect();
        MergeParents { repo, trees }
    }

    /// The oid of the object at the given path in the `i`th parent, if any.
    fn path_oid(&self, i: usize, path: &Path) -> Option<Oid> {
        self.trees[i].get_path(path).ok().map(|entry| entry.id())
    }

    fn blob(&self, oid: Oid) -> Vec<u8> {
        self.repo.find_blob(oid).unwrap().content().to_vec()
    }

    fn path_blob(&self, i: usize, path: &Path) -> Option<Vec<u8>> {
        self.path_oid(i, path).map(|oid| self.blob(oid))
    }

    /// The text of the journal at `path` in the `i`th parent, with its
    /// segments (see `hyperblame::segments`), if any.
    fn journal_text(&self, i: usize, path: &Path) -> Option<String> {
        journal_text_in(self.repo, &self.trees[i], path)
    }

    /// Load the records of a journal version, which must be in a commit on disk
    /// (as the parents' journals and their predecessors are).
    fn load_journal_version<H: DeserializeOwned + Default, R: DeserializeOwned>(
        &self,
        version: &JournalVersionRef,
    ) -> Result<Vec<R>, String> {
        let oid = Oid::from_str(&version.timeline_rev).map_err(|e| e.to_string())?;
        let tree = self
            .repo
            .find_commit(oid)
            .and_then(|commit| commit.tree())
            .map_err(|e| e.to_string())?;
        Ok(journal_text_in(self.repo, &tree, Path::new(&version.path))
            .and_then(|text| read_record_file_contents::<H, R>(text.as_bytes()))
            .map(|(_, records)| records)
            .unwrap_or_default())
    }
}

/// The text of the journal at `path` in `tree`, with its segments (see
/// `hyperblame::segments`), if any.
fn journal_text_in(repo: &Repository, tree: &git2::Tree, path: &Path) -> Option<String> {
    let read = |path: &Path| {
        let entry = tree.get_path(path).ok()?;
        Some(String::from_utf8(repo.find_blob(entry.id()).unwrap().content().to_vec()).unwrap())
    };
    let head = read(path)?;
    let years = segments::segment_years(head.lines().next().unwrap_or(""));
    if years.is_empty() {
        return Some(head);
    }
    let texts: Vec<String> = years
        .iter()
        .filter_map(|year| read(&segments::segment_path(path, *year)))
        .collect();
    Some(segments::join(&head, texts.iter().map(String::as_str)))
}

/// Make git fast-import write out its pack and refs so that we can read the
/// commits it wrote with git2, and wait for that by asking it for the oid of
/// `mark`.
fn checkpoint(import_helper: &mut FastImport, mark: usize) {
    writeln!(import_helper.input, "checkpoint").unwrap();
    import_helper.request_mark(mark);
    import_helper.output();
}

/// The union of the journal at `path` across all parents (whose hex ids are
/// `parent_revs`), if the parents' versions differ.
fn union_journal<H: DeserializeOwned + Default + Serialize, R: Summarize>(
    parents: &MergeParents,
    parent_revs: &[String],
    path: &Path,
    keep_from: Option<i32>,
) -> Vec<(PathBuf, String)> {
    // (With their segments, if any.)
    let mut dir = path.as_os_str().to_owned();
    dir.push(".d");
    let dir = PathBuf::from(dir);
    let oids: Vec<(Option<Oid>, Option<Oid>)> = (0..parent_revs.len())
        .map(|i| (parents.path_oid(i, path), parents.path_oid(i, &dir)))
        .collect();
    if oids.iter().all(|oid| *oid == oids[0]) {
        // The tree already has the first parent's version.
        return vec![];
    }
    let segmented = oids.iter().any(|(_, dir)| dir.is_some());
    let blobs: Vec<(JournalVersionRef, Vec<u8>)> = (0..parent_revs.len())
        .zip(parent_revs)
        .filter_map(|(i, parent_rev)| {
            let version = JournalVersionRef {
                timeline_rev: parent_rev.clone(),
                path: path.to_string_lossy().into_owned(),
            };
            Some((version, parents.journal_text(i, path)?.into_bytes()))
        })
        .collect();
    let texts: Vec<(JournalVersionRef, &str)> = blobs
        .iter()
        .map(|(version, blob)| (version.clone(), std::str::from_utf8(blob).unwrap()))
        .collect();
    let contents = merge_journal_texts(&texts).unwrap_or_else(|| {
        let mut header: Option<H> = None;
        let mut versions = vec![];
        for (version, blob) in &blobs {
            if let Some((h, records)) = read_record_file_contents::<H, R>(blob) {
                header.get_or_insert(h);
                versions.push((version.clone(), records));
            }
        }
        // A version is often the pred of several of the summaries.
        let mut loaded: HashMap<JournalVersionRef, Vec<R>> = HashMap::new();
        let records = merge_journal_versions(versions, &mut |version| {
            if let Some(records) = loaded.get(version) {
                return Ok(records.clone());
            }
            let records = parents.load_journal_version::<H, R>(version)?;
            loaded.insert(version.clone(), records.clone());
            Ok(records)
        })
        .unwrap();
        record_file_contents_to_string(&header.unwrap_or_default(), &records)
    });
    let Some(keep_from) = keep_from.filter(|_| segmented || contents.len() > split_bytes()) else {
        return vec![(path.to_path_buf(), contents)];
    };
    // Only the files which differ from the first parent's.
    let split = segments::split(&contents, keep_from);
    std::iter::once((path.to_path_buf(), split.head))
        .chain(
            split
                .segments
                .into_iter()
                .map(|(year, text)| (segments::segment_path(path, year), text)),
        )
        .filter(|(path, text)| {
            parents.path_oid(0, path)
                != Some(Oid::hash_object(git2::ObjectType::Blob, text.as_bytes()).unwrap())
        })
        .collect()
}

/// A journal which a merge unions across its parents or deletes.
enum MergeJournal {
    Future(PathBuf),
    FilesDelta(PathBuf),
    Tokens(PathBuf),
    Delete(PathBuf),
}

impl MergeJournal {
    fn path(&self) -> &Path {
        match self {
            MergeJournal::Future(path)
            | MergeJournal::FilesDelta(path)
            | MergeJournal::Tokens(path)
            | MergeJournal::Delete(path) => path,
        }
    }

    /// The files to write for the journal (its new contents and, for a token
    /// journal, segments; see `hyperblame::segments`), given the first year a
    /// token journal's head keeps.
    fn merged(
        &self,
        parents: &MergeParents,
        parent_revs: &[String],
        keep_from: Option<i32>,
    ) -> Vec<(PathBuf, String)> {
        match self {
            MergeJournal::Future(path) => {
                union_journal::<FutureHeader, FutureRecord>(parents, parent_revs, path, None)
            }
            MergeJournal::FilesDelta(path) => {
                union_journal::<FileDeltaHeader, FileDeltaRecord>(parents, parent_revs, path, None)
            }
            MergeJournal::Tokens(path) => union_journal::<TokenHeader, TokenDeltaRecord>(
                parents,
                parent_revs,
                path,
                keep_from,
            ),
            MergeJournal::Delete(_) => vec![],
        }
    }

    fn write(&self, import_helper: &mut FastImport, files: Vec<(PathBuf, String)>) {
        if let MergeJournal::Delete(path) = self {
            delete_path(import_helper, path);
        }
        for (path, contents) in files {
            write_inline_blob(import_helper, &path, contents.as_bytes());
        }
    }
}

/// How many journals a merge needs to have for us to union them on threads.
const MIN_JOURNALS_FOR_THREADS: usize = 16;

/// Union or delete the journals of a merge, in order.  Firefox's merges can have
/// thousands of journals, whose parents' versions all need to be read and
/// parsed, so the unions are done on threads, each with its own handle on the
/// timeline repo (since git2's can't be shared between threads).
fn merge_journals(
    import_helper: &mut FastImport,
    parents: &MergeParents,
    parent_revs: &[String],
    journals: &[MergeJournal],
    keep_from: Option<i32>,
) {
    let num_threads = history_compute_threads().min(journals.len());
    if journals.len() < MIN_JOURNALS_FOR_THREADS || num_threads < 2 {
        for journal in journals {
            journal.write(
                import_helper,
                journal.merged(parents, parent_revs, keep_from),
            );
        }
        return;
    }
    let repo_path = parents.repo.path();
    let (job_tx, job_rx) = channel::<usize>();
    let job_rx = Mutex::new(job_rx);
    let (result_tx, result_rx) = channel::<(usize, Vec<(PathBuf, String)>)>();
    thread::scope(|scope| {
        for _ in 0..num_threads {
            let job_rx = &job_rx;
            let result_tx = result_tx.clone();
            scope.spawn(move || {
                let repo = Repository::open(repo_path).unwrap();
                let parents = MergeParents::new(&repo, parent_revs);
                loop {
                    let Ok(idx) = job_rx.lock().unwrap().recv() else {
                        break;
                    };
                    let journal = &journals[idx];
                    match catch_unwind(AssertUnwindSafe(|| {
                        journal.merged(&parents, parent_revs, keep_from)
                    })) {
                        Ok(contents) => result_tx.send((idx, contents)).unwrap(),
                        // The main thread would wait for this result forever.
                        Err(_) => {
                            error!("Merging {} failed", journal.path().display());
                            std::process::exit(101);
                        }
                    }
                }
            });
        }
        drop(result_tx);
        // Only hand out journals up to a window ahead of the next one to write,
        // so that the merged journals waiting to be written stay few.
        let window = 4 * num_threads;
        let mut sent = 0;
        let mut merged: HashMap<usize, Vec<(PathBuf, String)>> = HashMap::new();
        for (idx, journal) in journals.iter().enumerate() {
            while sent < journals.len() && sent < idx + window {
                job_tx.send(sent).unwrap();
                sent += 1;
            }
            let contents = loop {
                if let Some(contents) = merged.remove(&idx) {
                    break contents;
                }
                let (done, contents) = result_rx.recv().unwrap();
                merged.insert(done, contents);
            };
            journal.write(import_helper, contents);
        }
        drop(job_tx);
    });
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

#[allow(clippy::too_many_arguments)]
fn process_linear_revision(
    import_helper: &mut FastImport,
    data: &TimelineData,
    files: &[FileChange],
    token_totals: &BTreeMap<String, TokenDeltaDetails>,
    timeline_parents: &[TimelineRepoCommit],
    timeline_commits: &TimelineCommits,
    consolidation: Option<&Consolidation>,
    num_summaries: &mut usize,
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
    if parent.is_some() {
        // Along with the other files we read from the parent, below.
        let mut paths = vec![];
        for change in files {
            if let Some(old_path) = &change.old_path {
                paths.push(annotated_path(old_path));
                paths.push(future_path(old_path));
            }
            if let Some(new_path) = &change.new_path {
                paths.push(future_path(new_path));
                let from_path = match change.kind {
                    FileChangeKind::Renamed | FileChangeKind::Copied => {
                        change.old_path.as_ref().unwrap()
                    }
                    _ => new_path,
                };
                paths.push(files_delta_path(from_path));
            }
        }
        for (token, delta) in token_totals {
            if delta.has_non_move_changes() {
                paths.push(token_timeline_path(token));
            }
        }
        prefetch_parent_paths(import_helper, paths);

        for change in files {
            if let Some(old_path) = &change.old_path
                && !parent_annotated.contains_key(old_path)
            {
                // (Nothing has been modified in the new commit yet.)
                let lines =
                    read_path_blob(import_helper, ReadFrom::Active, &annotated_path(old_path))
                        .map(|blob| annotated_lines(&blob))
                        .unwrap_or_default();
                parent_annotated.insert(old_path.clone(), lines);
            }
        }
    }

    // ## Deletions (before modifications so renames/swaps work out)
    // The journal paths deleted or written so far; see `prepend_journal_record`.
    let mut touched: HashSet<PathBuf> = HashSet::new();
    for change in files {
        if matches!(
            change.kind,
            FileChangeKind::Deleted | FileChangeKind::Renamed
        ) {
            let old_path = change.old_path.as_deref().unwrap();
            delete_path(import_helper, &annotated_path(old_path));
            delete_path(import_helper, &files_delta_path(old_path));
            touched.insert(files_delta_path(old_path));
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
                let blob = read_path_blob(
                    import_helper,
                    ReadFrom::Commit(&base_commit),
                    &annotated_path(new_path),
                )?;
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
        *num_summaries += prepend_journal_record::<FutureHeader, FutureRecord>(
            import_helper,
            parent,
            &journal,
            &journal,
            FutureRecord::Detail(record),
            consolidation,
            None,
            &mut touched,
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
        *num_summaries += prepend_journal_record::<FileDeltaHeader, FileDeltaRecord>(
            import_helper,
            parent,
            &files_delta_path(from_path),
            &files_delta_path(new_path),
            FileDeltaRecord::Detail(FileDeltaDetailRecord {
                desc: desc.clone(),
                delta: change.delta.clone(),
            }),
            consolidation,
            None,
            &mut touched,
        );
    }

    // ## Token journals
    let token_keep_from = keep_from(&data.iso_date);
    for (token, delta) in token_totals {
        if !delta.has_non_move_changes() {
            continue;
        }
        let journal = token_timeline_path(token);
        *num_summaries += prepend_journal_record::<TokenHeader, TokenDeltaRecord>(
            import_helper,
            parent,
            &journal,
            &journal,
            TokenDeltaRecord::Detail(TokenDeltaDetailRecord {
                desc: desc.clone(),
                delta: delta.clone(),
            }),
            consolidation,
            token_keep_from,
            &mut touched,
        );
    }

    summaries
}

fn process_merge_revision(
    import_helper: &mut FastImport,
    data: &TimelineData,
    merge: &MergeChanges,
    parents: &MergeParents,
    parent_revs: &[String],
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
            && let Some(oid) = parents.path_oid(i, &annotated_path(path))
        {
            write_existing_blob(import_helper, &new_annotated, &oid.to_string());
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
            let Some(blob) = parents.path_blob(i, &annotated_path(path)) else {
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
    let mut journals = vec![];
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for (path, exists) in &merge.candidate_paths {
        journals.push(MergeJournal::Future(future_path(path)));
        if *exists {
            journals.push(MergeJournal::FilesDelta(files_delta_path(path)));
        } else {
            journals.push(MergeJournal::Delete(files_delta_path(path)));
        }
    }
    for token in &merge.candidate_tokens {
        journals.push(MergeJournal::Tokens(token_timeline_path(token)));
    }
    seen.extend(journals.iter().map(|journal| journal.path().to_path_buf()));
    // (And the journals which differ between the parents' timeline commits but
    // weren't candidates, whose files exist in the merge if the first parent
    // has them and the merge doesn't remove them, or the merge adds them.)
    let removed: HashSet<&str> = merge
        .files
        .iter()
        .filter_map(|change| change.removed_path.as_deref())
        .collect();
    let added: HashSet<&str> = merge
        .files
        .iter()
        .filter_map(|change| change.new_path.as_deref())
        .collect();
    for path in differing_journals(parents) {
        if seen.contains(&path) {
            continue;
        }
        let path_str = path.to_string_lossy();
        let journal = if path_str.starts_with("tokens/") {
            MergeJournal::Tokens(path.clone())
        } else if path_str.starts_with("future/") {
            MergeJournal::Future(path.clone())
        } else {
            let source = path_str
                .strip_prefix("files-delta/")
                .and_then(|rest| rest.strip_suffix(".ndjson"))
                .unwrap_or_default();
            let exists = added.contains(source)
                || (!removed.contains(source)
                    && parents.path_oid(0, &annotated_path(source)).is_some());
            if exists {
                MergeJournal::FilesDelta(path.clone())
            } else {
                MergeJournal::Delete(path.clone())
            }
        };
        seen.insert(path);
        journals.push(journal);
    }
    merge_journals(
        import_helper,
        parents,
        parent_revs,
        &journals,
        keep_from(&data.iso_date),
    );
}

/// The journals (`tokens/`, `future/`, and `files-delta/` paths, with
/// segments' paths as their journals') which differ between a merge's first
/// parent and its other parents.
fn differing_journals(parents: &MergeParents) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    for other in &parents.trees[1..] {
        for prefix in ["tokens", "future", "files-delta"] {
            let first = subtree(parents.repo, &parents.trees[0], prefix);
            let other = subtree(parents.repo, other, prefix);
            let mut blobs = vec![];
            differing_blobs(
                parents.repo,
                first.as_ref(),
                other.as_ref(),
                Path::new(prefix),
                &mut blobs,
            );
            for path in blobs {
                // (A segment, `PATH.d/YEAR.ndjson`, is its journal's.)
                let text = path.to_string_lossy();
                paths.insert(match text.find(".ndjson.d/") {
                    Some(at) => PathBuf::from(&text[..at + ".ndjson".len()]),
                    None => path,
                });
            }
        }
    }
    paths
}

/// The paths (under `prefix`) of the blobs which differ between two trees,
/// recursing only into subtrees whose ids differ.  (libgit2's tree diff takes
/// ~8 times as long, ex: 1.5s for a merge on the firefox history window.)
fn differing_blobs(
    repo: &Repository,
    a: Option<&git2::Tree>,
    b: Option<&git2::Tree>,
    prefix: &Path,
    out: &mut Vec<PathBuf>,
) {
    if a.map(|t| t.id()) == b.map(|t| t.id()) {
        return;
    }
    // Each name's (id, whether it's a tree) in each tree.
    type Side = Option<(Oid, bool)>;
    let mut entries: BTreeMap<Vec<u8>, (Side, Side)> = BTreeMap::new();
    for (tree, first) in [(a, true), (b, false)] {
        for entry in tree.into_iter().flat_map(|tree| tree.iter()) {
            let side = Some((entry.id(), entry.kind() == Some(ObjectType::Tree)));
            let slot = entries.entry(entry.name_bytes().to_vec()).or_default();
            if first {
                slot.0 = side;
            } else {
                slot.1 = side;
            }
        }
    }
    for (name, (x, y)) in entries {
        if x.map(|(id, _)| id) == y.map(|(id, _)| id) {
            continue;
        }
        let path = prefix.join(String::from_utf8_lossy(&name).as_ref());
        let tree_of = |side: Side| {
            side.filter(|(_, is_tree)| *is_tree)
                .and_then(|(id, _)| repo.find_tree(id).ok())
        };
        let (x_tree, y_tree) = (tree_of(x), tree_of(y));
        if x_tree.is_some() || y_tree.is_some() {
            differing_blobs(repo, x_tree.as_ref(), y_tree.as_ref(), &path, out);
        }
        if x.is_some_and(|(_, is_tree)| !is_tree) || y.is_some_and(|(_, is_tree)| !is_tree) {
            out.push(path);
        }
    }
}

fn write_rev_summary(rev_summary_root: &Path, summary: &RevSummaryRecord) {
    let path = rev_summary_root.join(rev_summary_path(&summary.source_rev));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_string(summary).unwrap()).unwrap();
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

/// A revision whose timeline commit we've written, but not the rev-summary and
/// note which need its id.
struct UnfinishedRevision {
    mark: usize,
    /// The rev-summary, but for `timeline_rev`.
    summary: RevSummaryRecord,
    source_rev: Oid,
    /// The commit time, for the note.
    time: i64,
}

impl UnfinishedRevision {
    fn finish(
        mut self,
        import_helper: &mut FastImport,
        notes: &mut NotesWriter,
        rev_summary_root: &Path,
    ) {
        self.summary.timeline_rev = import_helper.mark_rev(self.mark);
        write_rev_summary(rev_summary_root, &self.summary);
        for backed_out_rev in &self.summary.backs_out {
            mark_rev_summary_backed_out(rev_summary_root, backed_out_rev, &self.summary.source_rev);
        }

        // Only record the revision as processed once its rev-summary exists.
        notes.add(self.source_rev, &self.summary.timeline_rev, self.time);
        if notes.num_pending() >= NOTES_BATCH_SIZE {
            notes.flush(&mut import_helper.input).unwrap();
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // The history repos are ours and git fast-import already hashed what it
    // wrote, so libgit2 needn't hash every object it reads to check it (the
    // collision-detecting SHA-1 was over a tenth of the compute threads' time).
    git2::opts::strict_hash_verification(false);
    // libgit2 maps windows of pack files (shared by all of our Repository
    // handles) under a global lock, and unmaps the least recently used ones
    // when more than a limit (8 GB by default) is mapped, which with the full
    // firefox history's 100 GB of timeline packs had the merge threads taking
    // turns unmapping and mapping windows.  Mapping only takes address space.
    unsafe { git2::opts::set_mwindow_mapped_limit(1 << 40) }.unwrap();
    // libgit2 only caches trees of up to 4 KB, and 256 MB of objects for all
    // of the process's repositories (each compute thread has its own).  In the
    // 4-year firefox reblame, that was full, the compute threads' diffs of the
    // syntax repo's trees had them mostly waiting for each other on libgit2's
    // pack locks, and each chunk's rate fell from about 1,000 revisions a
    // minute to 330.
    unsafe { git2::opts::set_cache_object_limit(git2::ObjectType::Tree, 16 << 20) }.unwrap();
    unsafe { git2::opts::set_cache_max_size(4 << 30) }.unwrap();

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
    // git fast-import keeps every pack it writes (with a .keep file) until it
    // exits, so that repacking can't delete them from under it, and each
    // checkpoint writes one (as does each run).  We checkpoint before most
    // merges (see `MergeParents`), so over a history with many merges, a run's
    // packs pile up (ex: ~735 in 10,000 revisions of firefox's 2008 history),
    // slowing every object lookup, and nothing can combine them.  So with
    // MAX_CHECKPOINTS, we end the run after that many checkpoints, exiting with
    // ENDED_EARLY_EXIT_CODE, and scripts/build-history.py repacks them before
    // it runs us again.
    let max_checkpoints = env::var("MAX_CHECKPOINTS")
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .filter(|&max| max > 0);
    // Likewise, with MAX_WRITTEN_BYTES, we end the run once we've given git
    // fast-import that many bytes of blobs, which bounds how much there is for
    // scripts/build-history.py to repack after a run, since we rewrite whole
    // journals: ex: ~350 GiB in 10,000 revisions of firefox's 2020 history.
    let max_written_bytes = env::var("MAX_WRITTEN_BYTES")
        .ok()
        .and_then(|x| x.parse::<u64>().ok())
        .filter(|&max| max > 0);

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
    let mut mapping = SourceMapping::open(&timeline_repo, &notes_refs);
    require_notes_for_existing_branch(&timeline_repo, &blame_ref, &notes_refs, &mapping);
    info!("Using source mapping notes {}", notes_refs.write);
    // The walk below looks up every unprocessed revision in the notes, which is
    // all of them for each chunk of a reblame, which took minutes with the
    // full firefox history's notes.
    mapping.preload(&timeline_repo).unwrap();

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
    let syntax_head = syntax_repo.refname_to_id(&blame_ref).unwrap();
    let mut walk = syntax_repo.revwalk().unwrap();
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    walk.push(syntax_head).unwrap();
    let mut syntax_revs = walk
        .with_hide_callback(&mut hide_processed)
        .unwrap()
        .map(|r| r.unwrap()) // walk produces Result<git2::Oid> so we unwrap to just the Oid
        .collect::<Vec<_>>();
    info!(
        "{} revisions to process, building on {} processed revisions",
        syntax_revs.len(),
        processed.len()
    );
    if syntax_revs.is_empty()
        && let Some(TimelineRepoCommit::Commit(timeline_rev)) = processed.get(&syntax_head)
    {
        point_branch_at(&timeline_repo, &blame_ref, *timeline_rev);
    }
    let mut timeline_commits = TimelineCommits {
        syntax_repo: &syntax_repo,
        timeline_repo: &timeline_repo,
        mapping: mapping.clone(),
        known: processed,
    };
    if commit_limit > 0 && commit_limit < syntax_revs.len() {
        info!(
            "Truncating list of commits from {} to specified limit {}",
            syntax_revs.len(),
            commit_limit
        );
        syntax_revs.truncate(commit_limit);
    }
    // Read the commits so we can have all the relevant revision identifiers
    // (only after truncating, since build-history.py processes a chunk of all of
    // the unprocessed revisions at a time).
    let revs_to_process = syntax_revs
        .into_iter()
        .map(|syntax_oid| {
            let commit = syntax_repo.find_commit(syntax_oid).unwrap();
            syntax_commit_to_meta(&commit)
        })
        .collect::<Vec<_>>();
    let rev_count = revs_to_process.len();

    let backout_resolver = Arc::new(BackoutTargetResolver::new(
        syntax_mapping,
        Path::new(&source_repo_path),
        use_cinnabar,
    ));

    let num_threads = history_compute_threads();
    const COMPUTE_BUFFER_SIZE: usize = 10;

    info!("Starting {} compute threads...", num_threads);
    let mut compute_pool = ComputePool::new(
        num_threads,
        &syntax_repo_path,
        &source_repo_path,
        backout_resolver.clone(),
    );

    // This tracks the index of the next revision in revs_to_process for which
    // we want to request a compute. All revs at indices less than this index
    // have already been requested.
    let mut compute_index = 0;

    info!("Filling compute buffer...");
    let initial_request_count = rev_count.min(COMPUTE_BUFFER_SIZE * num_threads);
    while compute_index < initial_request_count {
        compute_pool.compute(compute_index, &revs_to_process[compute_index]);
        compute_index += 1;
    }

    let mut import_helper = start_fast_import(&timeline_repo);
    let mut notes = notes_writer(&timeline_repo, &notes_refs);
    // Journal consolidation (see `hyperblame::consolidation`).
    let consolidate = env::var("CONSOLIDATE").map_or(true, |v| v != "0");
    let mut num_summaries = 0;
    // The last revision written, until we finish it with the next.
    let mut unfinished: Option<UnfinishedRevision> = None;
    // The last mark whose commit git fast-import has written to disk.
    let mut checkpointed_mark = 0;
    let mut checkpoints = 0;

    // Tracks completion count and serves as the basis for the mark <idnum>
    // assigned to each commit.
    let mut rev_done = 0;

    // The exit status if we stop before the last revision.
    let mut stopped_with = None;
    for rev_meta in revs_to_process.iter() {
        if stop_requested() {
            info!("Stopping after {} revisions, as requested", rev_done);
            stopped_with = Some(STOPPED_EXIT_CODE);
            break;
        }
        if max_checkpoints.is_some_and(|max| checkpoints >= max) {
            info!(
                "Ending the run after {} revisions and {} checkpoints (MAX_CHECKPOINTS)",
                rev_done, checkpoints
            );
            stopped_with = Some(ENDED_EARLY_EXIT_CODE);
            break;
        }
        if max_written_bytes.is_some_and(|max| import_helper.written >= max) {
            info!(
                "Ending the run after {} revisions and {} bytes of blobs (MAX_WRITTEN_BYTES)",
                rev_done, import_helper.written
            );
            stopped_with = Some(ENDED_EARLY_EXIT_CODE);
            break;
        }
        let data = compute_pool.result(rev_done);
        assert!(data.meta.syntax_rev == rev_meta.syntax_rev);

        // If there are more revisions that we haven't requested yet, request
        // another one.
        if compute_index < rev_count {
            compute_pool.compute(compute_index, &revs_to_process[compute_index]);
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

        // Merges read their parents from disk; see `MergeParents`.
        if matches!(data.changes, RevisionChanges::Merge(_))
            && timeline_parents.iter().any(
                |parent| matches!(parent, TimelineRepoCommit::Mark(mark) if *mark > checkpointed_mark),
            )
        {
            checkpoint(&mut import_helper, rev_done - 1);
            checkpointed_mark = rev_done - 1;
            checkpoints += 1;
        }

        let first_parent = timeline_parents.first();
        let on_disk = match first_parent {
            _ if import_helper.cache.describes(first_parent) => None,
            Some(TimelineRepoCommit::Commit(oid)) => Some(*oid),
            Some(TimelineRepoCommit::Mark(mark)) if *mark <= checkpointed_mark => {
                Some(Oid::from_str(&import_helper.mark_rev(*mark)).unwrap())
            }
            _ => None,
        };
        import_helper.cache.begin(first_parent, on_disk);
        // Scope the import_helper borrow
        {
            // Here we write out the metadata for a new commit to the timeline
            // repo.  For details on the data format, refer to the documentation at
            // https://git-scm.com/docs/git-fast-import#_commit
            // https://git-scm.com/docs/git-fast-import#_mark
            let import_stream = &mut import_helper.input;
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
        }

        let file_deltas = match &data.changes {
            RevisionChanges::Linear {
                files,
                token_totals,
            } => {
                let consolidation = Consolidation {
                    iso_date: &data.iso_date,
                };
                process_linear_revision(
                    &mut import_helper,
                    &data,
                    files,
                    token_totals,
                    &timeline_parents,
                    &timeline_commits,
                    consolidate.then_some(&consolidation),
                    &mut num_summaries,
                )
            }
            RevisionChanges::Merge(merge) => {
                // The parents' hex ids, for journal versions referenced by
                // summaries.
                let parent_revs: Vec<String> = timeline_parents
                    .iter()
                    .map(|parent| import_helper.commit_rev(parent))
                    .collect();
                process_merge_revision(
                    &mut import_helper,
                    &data,
                    merge,
                    &MergeParents::new(&timeline_repo, &parent_revs),
                    &parent_revs,
                );
                BTreeMap::new()
            }
        };

        if let Some(parent) = first_parent {
            write_deferred_journals(&mut import_helper, parent);
        }
        // Terminate the commit, and ask for its id for the rev-summary, which we
        // write once we've written the next revision, by when git fast-import
        // has usually answered, rather than waiting for it to catch up now.
        writeln!(import_helper.input).unwrap();
        let cleared = import_helper.cache.end(rev_done);
        if let Some(previous) = unfinished.take() {
            previous.finish(&mut import_helper, &mut notes, &rev_summary_root);
        }
        import_helper.request_mark(rev_done);
        let (file_deltas, file_totals) = file_deltas_or_totals(file_deltas);
        unfinished = Some(UnfinishedRevision {
            mark: rev_done,
            summary: RevSummaryRecord {
                source_rev: rev_meta.source_rev.to_string(),
                hg_rev: rev_meta.source_hg_rev.clone(),
                old_revs: rev_meta
                    .oldrevs
                    .as_deref()
                    .map(|revs| revs.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
                syntax_rev: rev_meta.syntax_rev.to_string(),
                timeline_rev: String::new(),
                message: data.message.clone(),
                iso_date: data.iso_date.clone(),
                unmapped_author: data.unmapped_author.clone(),
                file_deltas,
                file_totals,
                backs_out: data.backed_out.clone(),
                backed_out_by: vec![],
            },
            source_rev: rev_meta.source_rev,
            time: syntax_commit.committer().when().seconds(),
        });

        if rev_done % 100000 == 0 || cleared {
            if cleared {
                info!("Cleared the tree cache, issuing checkpoint to rebase it...");
            } else {
                info!("Completed 100,000 commits, issuing checkpoint...");
            }
            checkpoint(&mut import_helper, rev_done);
            checkpointed_mark = rev_done;
            checkpoints += 1;
        }
        if cleared {
            let base = Oid::from_str(&import_helper.mark_rev(rev_done)).unwrap();
            import_helper.cache.rebase(base);
        }
    }

    if let Some(last) = unfinished.take() {
        last.finish(&mut import_helper, &mut notes, &rev_summary_root);
    }
    notes.flush(&mut import_helper.input).unwrap();
    info!("Wrote {} journal summary records.", num_summaries);
    info!(
        "Gave git fast-import {:.1} GiB of blobs.",
        import_helper.written as f64 / (1u64 << 30) as f64
    );
    let cache = &import_helper.cache;
    info!(
        "Tree cache: {} hits, {} misses, cleared {} times; read {} paths from disk and {} through git fast-import",
        cache.hits, cache.misses, cache.clears, cache.disk_reads, cache.fast_import_reads
    );

    info!("Shutting down fast-import...");
    let exitcode = import_helper.finish();
    if exitcode.success() {
        info!("Done!");
    } else {
        info!("Fast-import exited with {:?}", exitcode.code());
    }
    if let Some(code) = stopped_with {
        std::process::exit(code);
    }
}
