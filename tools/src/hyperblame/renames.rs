//! libgit2's rename and copy detection (`git_diff_find_similar`) for big
//! diffs, with the similarities it compares files by computed on threads
//! first.
//!
//! libgit2 compares the files it might pair one pair at a time, on one thread:
//! each target (added files, and modified ones different enough to break into
//! a deletion and an addition) against each source (deleted files, and with
//! copies, modified ones), by their signatures (its default metric: `hashsig`,
//! by default with smart whitespace handling and small files allowed).  A
//! revision with thousands of added and deleted files, ex: firefox's webrtc
//! update of 2018-07-20 (1,402 added, 1,235 deleted and 2,275 modified files),
//! takes minutes of that.  So for big diffs, `find_similar` computes the
//! signatures, and the comparisons libgit2 will make, on threads first, and
//! gives libgit2 a metric which looks them up (and computes any others as the
//! default metric does).  libgit2 still pairs the files itself, in its order,
//! with the same scores, so the results are the default metric's.

use std::collections::{HashMap, VecDeque};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use git2::{Binding, Blob, Delta, Diff, DiffFindOptions, Oid, Repository};
use libgit2_sys as raw;

/// libgit2's file signatures (its `sys/hashsig.h`, which libgit2-sys doesn't
/// bind).
#[repr(C)]
struct GitHashsig {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn git_hashsig_create(
        out: *mut *mut GitHashsig,
        buf: *const c_char,
        buflen: usize,
        opts: c_int,
    ) -> c_int;
    fn git_hashsig_create_fromfile(
        out: *mut *mut GitHashsig,
        path: *const c_char,
        opts: c_int,
    ) -> c_int;
    fn git_hashsig_compare(a: *const GitHashsig, b: *const GitHashsig) -> c_int;
    fn git_hashsig_free(sig: *mut GitHashsig);
}

/// The default metric's hashsig options for `flags` (see libgit2's
/// `normalize_find_opts`).
fn hashsig_options(flags: u32) -> c_int {
    let whitespace = if flags & raw::GIT_DIFF_FIND_IGNORE_WHITESPACE != 0 {
        1 // GIT_HASHSIG_IGNORE_WHITESPACE
    } else if flags & raw::GIT_DIFF_FIND_DONT_IGNORE_WHITESPACE != 0 {
        0 // GIT_HASHSIG_NORMAL
    } else {
        2 // GIT_HASHSIG_SMART_WHITESPACE
    };
    whitespace | 4 // GIT_HASHSIG_ALLOW_SMALL_FILES
}

/// libgit2's default break rewrite threshold, for options without one.
const DEFAULT_BREAK_REWRITE_THRESHOLD: u16 = 60;

/// Diffs with fewer possible comparisons than this are left to libgit2 alone
/// (a few seconds' worth at most).
const MIN_COMPARISONS: usize = 200_000;

/// Diffs with more possible comparisons than this are too (their scores would
/// take too much memory).
const MAX_COMPARISONS: usize = 400_000_000;

/// A blob's signature, or the error computing it gave, as libgit2's metric
/// callbacks give it.
struct Signature {
    sig: *mut GitHashsig,
    error: c_int,
    /// Its index among `Similarities::sigs` (usize::MAX for one computed
    /// while libgit2 compares, which it frees).
    index: usize,
    /// Its row as a source's old file and its column as a target's new file
    /// in `Similarities::scores`, if it's either.
    row: Option<usize>,
    col: Option<usize>,
}

/// The precomputed signatures and similarities of a diff's files.
struct Similarities {
    by_oid: HashMap<Oid, usize>,
    sigs: Vec<Signature>,
    /// Sources' old files' (rows) similarities to targets' new files
    /// (columns), or -1 if not computed.
    scores: Vec<Vec<i8>>,
    /// Modified files' similarities of their old and new files, by their
    /// signatures' indexes.
    selves: HashMap<(usize, usize), i8>,
    hashsig_options: c_int,
}

impl Similarities {
    fn score(&self, a: &Signature, b: &Signature) -> Option<c_int> {
        if a.index == usize::MAX || b.index == usize::MAX {
            return None;
        }
        if let (Some(row), Some(col)) = (a.row, b.col) {
            let score = self.scores[row][col];
            if score >= 0 {
                return Some(score as c_int);
            }
        }
        self.selves
            .get(&(a.index, b.index))
            .map(|&score| score as c_int)
    }
}

impl Drop for Similarities {
    fn drop(&mut self) {
        for sig in &self.sigs {
            if !sig.sig.is_null() {
                unsafe { git_hashsig_free(sig.sig) };
            }
        }
    }
}

/// Signatures, for sharing among threads, which only read them.
struct SharedSigs<'a>(&'a [Signature]);
unsafe impl Sync for SharedSigs<'_> {}

impl SharedSigs<'_> {
    // (So that closures capture the whole, which is Sync, not its slice.)
    fn sigs(&self) -> &[Signature] {
        self.0
    }
}

/// A signature, for sending from the thread which computed it.
struct SendSig(*mut GitHashsig, c_int);
unsafe impl Send for SendSig {}

/// `work(state, i)` for each i in 0..n, on up to `threads` threads, each with
/// its own `init()` state, in order.
fn parallel_map<S, T: Send>(
    n: usize,
    threads: usize,
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, usize) -> T + Sync,
) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<T>> = (0..n).map(|_| None).collect();
    thread::scope(|scope| {
        let handles: Vec<_> = (0..threads.min(n).max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut state = init();
                    let mut done = vec![];
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= n {
                            break;
                        }
                        done.push((i, work(&mut state, i)));
                    }
                    done
                })
            })
            .collect();
        for handle in handles {
            for (i, result) in handle.join().unwrap() {
                results[i] = Some(result);
            }
        }
    });
    results.into_iter().map(Option::unwrap).collect()
}

fn is_blob(mode: u32) -> bool {
    // (GIT_MODE_ISBLOB: regular files, executable or not, but not links.)
    mode & 0o170000 == 0o100000
}

/// Whether libgit2 doesn't compare files whose sizes are so different (see
/// `similarity_measure`).
fn sizes_too_different(a: usize, b: usize) -> bool {
    a > 127 && b > 127 && (a > b << 3 || b > a << 3)
}

/// A blob's signature for libgit2's default metric with `options` (see
/// `hashsig_options`), or the error computing it gave, and its size.
struct OwnedSig {
    sig: *mut GitHashsig,
    error: c_int,
    size: usize,
    options: c_int,
}

// (Signatures are only read once made, ex: by `git_hashsig_compare`.)
unsafe impl Send for OwnedSig {}
unsafe impl Sync for OwnedSig {}

impl OwnedSig {
    fn new(contents: &[u8], options: c_int) -> Self {
        let mut sig = ptr::null_mut();
        let error = unsafe {
            git_hashsig_create(
                &mut sig,
                contents.as_ptr() as *const c_char,
                contents.len(),
                options,
            )
        };
        OwnedSig {
            sig,
            error,
            size: contents.len(),
            options,
        }
    }
}

impl Drop for OwnedSig {
    fn drop(&mut self) {
        if !self.sig.is_null() {
            unsafe { git_hashsig_free(self.sig) };
        }
    }
}

/// Big blobs' contents and signatures, shared by threads with their own
/// handles on a repository (ex: build-timeline-tree's compute threads), for
/// `find_similar_shared` and for reading blobs (`with_blob`): a revision's old
/// version of a file is the new version of the revision before it, which
/// another thread is usually reading too, or just read.  libgit2 rebuilds a
/// blob from its pack's delta chain at each read (and doesn't keep delta bases
/// over 1 MiB), and the wubspecs reblame's compute threads mostly rebuilt the
/// HTML Standard's token files (tens of MB, 160 MB nowadays), four times a
/// revision (twice for rename detection, and twice to read them).  (A blob
/// being read by one thread is waited for by the others, rather than read
/// again.)
pub struct SharedBlobs {
    inner: Mutex<SharedBlobsInner>,
    min_blob_bytes: usize,
    max_blob_bytes: usize,
    /// How many big blobs were read, and how many times kept ones were used.
    reads: AtomicUsize,
    hits: AtomicUsize,
    /// How many diffs `find_similar_shared` left as they were.
    left: AtomicUsize,
}

/// A kept blob, and its signature once it's needed.
struct KeptBlob {
    contents: Arc<[u8]>,
    sig: OnceLock<Arc<OwnedSig>>,
}

/// The blob being read by a thread, which the others wait for: the blob if
/// it's to be kept, or None if not (ex: it's small), when they read it.
type LoadingBlob = Arc<OnceLock<Option<Arc<KeptBlob>>>>;

#[derive(Default)]
struct SharedBlobsInner {
    blobs: HashMap<Oid, Arc<KeptBlob>>,
    /// The blobs, oldest first, for dropping them when there are too many
    /// bytes.
    order: VecDeque<Oid>,
    bytes: usize,
    loading: HashMap<Oid, LoadingBlob>,
}

/// A blob, from `SharedBlobs`.
enum LoadedBlob<'r> {
    Kept(Arc<KeptBlob>),
    Read(Blob<'r>),
}

impl LoadedBlob<'_> {
    fn contents(&self) -> &[u8] {
        match self {
            LoadedBlob::Kept(kept) => &kept.contents,
            LoadedBlob::Read(blob) => blob.content(),
        }
    }
}

impl SharedBlobs {
    /// Keeping blobs of at least `min_blob_bytes`, up to `max_blob_bytes` in
    /// all.
    pub fn new(min_blob_bytes: usize, max_blob_bytes: usize) -> Self {
        SharedBlobs {
            inner: Mutex::new(SharedBlobsInner::default()),
            min_blob_bytes,
            max_blob_bytes,
            reads: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            left: AtomicUsize::new(0),
        }
    }

    /// How many big blobs were read, how many times kept ones were used, and
    /// how many diffs `find_similar_shared` left as they were.
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.reads.load(Ordering::Relaxed),
            self.hits.load(Ordering::Relaxed),
            self.left.load(Ordering::Relaxed),
        )
    }

    fn keeps(&self, size: usize) -> bool {
        size >= self.min_blob_bytes && size <= self.max_blob_bytes
    }

    /// The blob `oid` in `repo`, if it has it.
    fn load<'r>(&self, repo: &'r Repository, oid: Oid) -> Option<LoadedBlob<'r>> {
        let loading = {
            let mut inner = self.inner.lock().unwrap();
            if let Some(kept) = inner.blobs.get(&oid) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(LoadedBlob::Kept(kept.clone()));
            }
            inner.loading.entry(oid).or_default().clone()
        };
        let mut read = None;
        let kept = loading
            .get_or_init(|| {
                let blob = repo.find_blob(oid).ok()?;
                if !self.keeps(blob.content().len()) {
                    read = Some(blob);
                    return None;
                }
                self.reads.fetch_add(1, Ordering::Relaxed);
                Some(Arc::new(KeptBlob {
                    contents: Arc::from(blob.content()),
                    sig: OnceLock::new(),
                }))
            })
            .clone();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner
                .loading
                .get(&oid)
                .is_some_and(|other| Arc::ptr_eq(other, &loading))
            {
                inner.loading.remove(&oid);
                if let Some(kept) = &kept {
                    inner.bytes += kept.contents.len();
                    inner.blobs.insert(oid, kept.clone());
                    inner.order.push_back(oid);
                    while inner.bytes > self.max_blob_bytes {
                        let Some(oldest) = inner.order.pop_front() else {
                            break;
                        };
                        if let Some(blob) = inner.blobs.remove(&oldest) {
                            inner.bytes -= blob.contents.len();
                        }
                    }
                }
            }
        }
        match (kept, read) {
            (Some(kept), _) => Some(LoadedBlob::Kept(kept)),
            (None, Some(blob)) => Some(LoadedBlob::Read(blob)),
            // (Another thread read it, and didn't keep it.)
            (None, None) => Some(LoadedBlob::Read(repo.find_blob(oid).ok()?)),
        }
    }

    /// `f` of the contents of the blob `oid` in `repo`, if it has it.
    pub fn with_blob<R>(
        &self,
        repo: &Repository,
        oid: Oid,
        f: impl FnOnce(&[u8]) -> R,
    ) -> Option<R> {
        Some(f(self.load(repo, oid)?.contents()))
    }

    /// The signature of the blob `oid` in `repo`, with `options` (see
    /// `hashsig_options`), if it has the blob.
    fn signature(&self, repo: &Repository, oid: Oid, options: c_int) -> Option<Arc<OwnedSig>> {
        Some(match self.load(repo, oid)? {
            LoadedBlob::Kept(kept) => {
                let sig = kept
                    .sig
                    .get_or_init(|| Arc::new(OwnedSig::new(&kept.contents, options)));
                if sig.options == options {
                    sig.clone()
                } else {
                    Arc::new(OwnedSig::new(&kept.contents, options))
                }
            }
            LoadedBlob::Read(blob) => Arc::new(OwnedSig::new(blob.content(), options)),
        })
    }
}

/// libgit2's default rename threshold (and rename from rewrite threshold),
/// for options without one.
const DEFAULT_THRESHOLD: u16 = 50;

/// Whether libgit2's rename and copy detection would leave `diff` as it is:
/// for a diff of only modified (and deleted) files, if none of the modified
/// files is different enough from its old version to be broken into a
/// deletion and an addition (see libgit2's `is_rename_target`), there are no
/// targets, so nothing to pair.  The modified files' self-similarities are
/// computed as libgit2's default metric does, from `shared`'s signatures.
/// (Anything else, ex: an added file, a modified file whose sizes are too
/// different to compare, a signature libgit2 couldn't make, or options with
/// another metric, and this says no, and libgit2 decides.)
fn leaves_unchanged(
    repo: &Repository,
    diff: &Diff,
    opts: &mut DiffFindOptions,
    shared: &SharedBlobs,
) -> bool {
    let raw_opts = unsafe { ptr::read(opts.raw()) };
    let mut flags = raw_opts.flags;
    // (Which would be `diff.renames`' flags, or another metric's scores.)
    if flags & raw::GIT_DIFF_FIND_ALL == 0 || !raw_opts.metric.is_null() {
        return false;
    }
    // (See libgit2's `normalize_find_opts`.)
    if flags & raw::GIT_DIFF_FIND_EXACT_MATCH_ONLY != 0 {
        flags &= !(raw::GIT_DIFF_FIND_REWRITES
            | raw::GIT_DIFF_BREAK_REWRITES
            | raw::GIT_DIFF_FIND_RENAMES_FROM_REWRITES);
    }
    if flags & raw::GIT_DIFF_BREAK_REWRITES != 0 {
        flags |= raw::GIT_DIFF_FIND_REWRITES;
    }
    let threshold = |given: u16, default: u16| {
        if given == 0 || given > 100 {
            default
        } else {
            given
        }
    };
    let break_threshold = threshold(
        raw_opts.break_rewrite_threshold,
        DEFAULT_BREAK_REWRITE_THRESHOLD,
    ) as c_int;
    let from_rewrite_threshold =
        threshold(raw_opts.rename_from_rewrite_threshold, DEFAULT_THRESHOLD) as c_int;
    let breaks = flags & raw::GIT_DIFF_BREAK_REWRITES != 0;
    let from_rewrites = flags & raw::GIT_DIFF_FIND_RENAMES_FROM_REWRITES != 0;
    let rewrites = flags & raw::GIT_DIFF_FIND_REWRITES != 0 || from_rewrites;
    let options = hashsig_options(flags);
    // (All the deltas' kinds first, before computing any signatures.)
    if !diff
        .deltas()
        .all(|delta| matches!(delta.status(), Delta::Modified | Delta::Deleted))
    {
        return false;
    }
    if !rewrites {
        // (Then modified files aren't targets; see `is_rename_target`.)
        return true;
    }
    for delta in diff.deltas() {
        let (old, new) = (delta.old_file(), delta.new_file());
        if delta.status() != Delta::Modified || !is_blob(u32::from(new.mode())) {
            continue;
        }
        if !is_blob(u32::from(old.mode())) {
            return false;
        }
        let score = if old.id() == new.id() {
            100
        } else {
            let (Some(a), Some(b)) = (
                shared.signature(repo, old.id(), options),
                shared.signature(repo, new.id(), options),
            ) else {
                return false;
            };
            if a.error < 0 || b.error < 0 || sizes_too_different(a.size, b.size) {
                return false;
            }
            unsafe { git_hashsig_compare(a.sig, b.sig) }
        };
        if score < 0
            || (breaks && score < break_threshold)
            || (from_rewrites && score < from_rewrite_threshold)
        {
            return false;
        }
    }
    true
}

/// `find_similar`, but leaving alone diffs which libgit2 would leave as they
/// are (see `leaves_unchanged`), whose modified files' self-similarities are
/// computed from `shared`'s blobs and signatures rather than by libgit2, which
/// reads each of their old and new blobs.  (With HISTORY_CHECK_SIMILAR=1, for
/// tests, libgit2 checks them anyway.)
pub fn find_similar_shared(
    repo: &Repository,
    diff: &mut Diff,
    opts: &mut DiffFindOptions,
    threads: usize,
    shared: &SharedBlobs,
) -> Result<(), git2::Error> {
    if !leaves_unchanged(repo, diff, opts, shared) {
        return find_similar(repo, diff, opts, threads);
    }
    shared.left.fetch_add(1, Ordering::Relaxed);
    if std::env::var("HISTORY_CHECK_SIMILAR").is_ok_and(|v| v == "1") {
        let deltas = |diff: &Diff| -> Vec<(
            Delta,
            Option<std::path::PathBuf>,
            Oid,
            Option<std::path::PathBuf>,
            Oid,
        )> {
            diff.deltas()
                .map(|d| {
                    (
                        d.status(),
                        d.old_file().path().map(|p| p.to_path_buf()),
                        d.old_file().id(),
                        d.new_file().path().map(|p| p.to_path_buf()),
                        d.new_file().id(),
                    )
                })
                .collect()
        };
        let before = deltas(diff);
        find_similar(repo, diff, opts, threads)?;
        assert!(
            before == deltas(diff),
            "libgit2's rename detection changed a diff find_similar_shared would have left"
        );
    }
    Ok(())
}

/// libgit2's rename and copy detection on `diff` (of `repo`'s trees) with
/// `opts`, with the comparisons computed on up to `threads` threads first if
/// there are many (see the module's comment).
pub fn find_similar(
    repo: &Repository,
    diff: &mut Diff,
    opts: &mut DiffFindOptions,
    threads: usize,
) -> Result<(), git2::Error> {
    find_similar_above(repo, diff, opts, threads, MIN_COMPARISONS)
}

fn find_similar_above(
    repo: &Repository,
    diff: &mut Diff,
    opts: &mut DiffFindOptions,
    threads: usize,
    min_comparisons: usize,
) -> Result<(), git2::Error> {
    // (Plain data, which git2 owns, so copied.)
    let raw_opts = unsafe { ptr::read(opts.raw()) };
    // (The diff's files: (status, old oid and mode, new oid and mode).)
    let files: Vec<(Delta, Oid, u32, Oid, u32)> = diff
        .deltas()
        .map(|d| {
            let (old, new) = (d.old_file(), d.new_file());
            (
                d.status(),
                old.id(),
                u32::from(old.mode()),
                new.id(),
                u32::from(new.mode()),
            )
        })
        .collect();
    // (Deleted files, and with copies, modified ones; see libgit2's
    // `is_rename_source`.  Without copies, modified files which are broken
    // are sources too, which libgit2 then compares as it does by default.)
    let copies = raw_opts.flags & raw::GIT_DIFF_FIND_COPIES != 0;
    let is_source = |status: Delta, mode: u32| {
        is_blob(mode)
            && match status {
                Delta::Deleted | Delta::Typechange => true,
                Delta::Modified => copies,
                _ => false,
            }
    };
    let possible_targets = files
        .iter()
        .filter(|f| is_blob(f.4) && matches!(f.0, Delta::Added | Delta::Modified))
        .count();
    let possible_sources = files.iter().filter(|f| is_source(f.0, f.2)).count();
    let possible = possible_targets.saturating_mul(possible_sources);
    let breaks = raw_opts.flags & raw::GIT_DIFF_BREAK_REWRITES != 0;
    if possible < min_comparisons || possible > MAX_COMPARISONS || threads < 2 {
        return diff.find_similar(Some(opts));
    }

    // The signatures of the blobs libgit2 might compare.
    let hashsig_options = hashsig_options(raw_opts.flags);
    let mut by_oid: HashMap<Oid, usize> = HashMap::new();
    let mut oids: Vec<Oid> = vec![];
    let mut add = |oid: Oid| {
        *by_oid.entry(oid).or_insert_with(|| {
            oids.push(oid);
            oids.len() - 1
        })
    };
    for &(status, old, old_mode, new, new_mode) in &files {
        if is_source(status, old_mode) || (status == Delta::Modified && is_blob(old_mode)) {
            add(old);
        }
        if is_blob(new_mode) && matches!(status, Delta::Added | Delta::Modified) {
            add(new);
        }
    }
    let path = repo.path().to_path_buf();
    let computed = parallel_map(
        oids.len(),
        threads,
        || Repository::open(&path).ok(),
        |repo, i| {
            let blob = repo.as_ref().and_then(|repo| repo.find_blob(oids[i]).ok());
            let Some(blob) = blob else {
                return (SendSig(ptr::null_mut(), -1), 0);
            };
            let content = blob.content();
            let mut sig = ptr::null_mut();
            let error = unsafe {
                git_hashsig_create(
                    &mut sig,
                    content.as_ptr() as *const c_char,
                    content.len(),
                    hashsig_options,
                )
            };
            (SendSig(sig, error), content.len())
        },
    );
    let mut sizes = Vec::with_capacity(computed.len());
    let mut sigs: Vec<Signature> = computed
        .into_iter()
        .enumerate()
        .map(|(index, (SendSig(sig, error), size))| {
            sizes.push(size);
            Signature {
                sig,
                error,
                index,
                row: None,
                col: None,
            }
        })
        .collect();
    let compare = |sigs: &[Signature], a: usize, b: usize| -> i8 {
        if sigs[a].sig.is_null() || sigs[b].sig.is_null() || sizes_too_different(sizes[a], sizes[b])
        {
            return -1;
        }
        let score = unsafe { git_hashsig_compare(sigs[a].sig, sigs[b].sig) };
        if score < 0 { -1 } else { score as i8 }
    };

    // Modified files' self-similarities, which decide which of them are
    // broken into targets (see libgit2's `is_rename_target`).
    let modified: Vec<(usize, usize)> = files
        .iter()
        .filter(|f| f.0 == Delta::Modified && is_blob(f.2) && is_blob(f.4) && f.1 != f.3)
        .map(|f| (by_oid[&f.1], by_oid[&f.3]))
        .collect();
    let shared = SharedSigs(&sigs);
    let self_scores = parallel_map(
        modified.len(),
        threads,
        || (),
        |_, i| compare(shared.sigs(), modified[i].0, modified[i].1),
    );
    let selves: HashMap<(usize, usize), i8> = modified
        .iter()
        .copied()
        .zip(self_scores)
        .filter(|&(_, score)| score >= 0)
        .collect();
    let threshold = match raw_opts.break_rewrite_threshold {
        0 | 101.. => DEFAULT_BREAK_REWRITE_THRESHOLD,
        threshold => threshold,
    };

    // The targets' and sources' signatures, and the comparisons of each
    // target with each source.
    let mut rows = vec![];
    let mut cols = vec![];
    for &(status, old, old_mode, new, new_mode) in &files {
        if is_source(status, old_mode) {
            let i = by_oid[&old];
            if sigs[i].row.is_none() {
                sigs[i].row = Some(rows.len());
                rows.push(i);
            }
        }
        let target = is_blob(new_mode)
            && match status {
                Delta::Added => true,
                Delta::Modified => {
                    breaks
                        && match (by_oid.get(&old), by_oid.get(&new)) {
                            (Some(&old), Some(&new)) => selves
                                .get(&(old, new))
                                .is_none_or(|&score| (score as u16) < threshold),
                            _ => true,
                        }
                }
                _ => false,
            };
        if target {
            let i = by_oid[&new];
            if sigs[i].col.is_none() {
                sigs[i].col = Some(cols.len());
                cols.push(i);
            }
        }
    }
    let shared = SharedSigs(&sigs);
    let scores = parallel_map(
        rows.len(),
        threads,
        || (),
        |_, row| {
            cols.iter()
                .map(|&col| {
                    if rows[row] == col {
                        -1
                    } else {
                        compare(shared.sigs(), rows[row], col)
                    }
                })
                .collect::<Vec<i8>>()
        },
    );

    let similarities = Similarities {
        by_oid,
        sigs,
        scores,
        selves,
        hashsig_options,
    };
    let mut metric = raw::git_diff_similarity_metric {
        file_signature: Some(file_signature),
        buffer_signature: Some(buffer_signature),
        free_signature: Some(free_signature),
        similarity: Some(similarity),
        payload: &similarities as *const Similarities as *mut c_void,
    };
    let mut find_opts = unsafe { ptr::read(opts.raw()) };
    find_opts.metric = &mut metric;
    let result = unsafe { raw::git_diff_find_similar(diff.raw(), &find_opts) };
    if result < 0 {
        return Err(git2::Error::last_error(result));
    }
    Ok(())
}

/// A signature libgit2 asks for while comparing that isn't precomputed (or a
/// working directory file's, which tree diffs don't have).
fn computed_signature(sig: *mut GitHashsig, error: c_int, out: *mut *mut c_void) -> c_int {
    if error < 0 {
        return error;
    }
    let signature = Box::new(Signature {
        sig,
        error,
        index: usize::MAX,
        row: None,
        col: None,
    });
    unsafe { *out = Box::into_raw(signature) as *mut c_void };
    0
}

extern "C" fn file_signature(
    out: *mut *mut c_void,
    _file: *const raw::git_diff_file,
    path: *const c_char,
    payload: *mut c_void,
) -> c_int {
    let similarities = unsafe { &*(payload as *const Similarities) };
    let mut sig = ptr::null_mut();
    let error =
        unsafe { git_hashsig_create_fromfile(&mut sig, path, similarities.hashsig_options) };
    computed_signature(sig, error, out)
}

extern "C" fn buffer_signature(
    out: *mut *mut c_void,
    file: *const raw::git_diff_file,
    buf: *const c_char,
    len: usize,
    payload: *mut c_void,
) -> c_int {
    let similarities = unsafe { &*(payload as *const Similarities) };
    let oid = unsafe { Oid::from_raw(&(*file).id) };
    if let Some(&i) = similarities.by_oid.get(&oid) {
        let signature = &similarities.sigs[i];
        if signature.error < 0 {
            return signature.error;
        }
        if !signature.sig.is_null() {
            unsafe { *out = signature as *const Signature as *mut c_void };
            return 0;
        }
    }
    let mut sig = ptr::null_mut();
    let error = unsafe { git_hashsig_create(&mut sig, buf, len, similarities.hashsig_options) };
    computed_signature(sig, error, out)
}

extern "C" fn free_signature(sig: *mut c_void, _payload: *mut c_void) {
    let signature = sig as *mut Signature;
    unsafe {
        if (*signature).index == usize::MAX {
            let signature = Box::from_raw(signature);
            if !signature.sig.is_null() {
                git_hashsig_free(signature.sig);
            }
        }
    }
}

extern "C" fn similarity(
    score: *mut c_int,
    a: *mut c_void,
    b: *mut c_void,
    payload: *mut c_void,
) -> c_int {
    let similarities = unsafe { &*(payload as *const Similarities) };
    let (a, b) = unsafe { (&*(a as *const Signature), &*(b as *const Signature)) };
    if let Some(computed) = similarities.score(a, b) {
        unsafe { *score = computed };
        return 0;
    }
    let result = unsafe { git_hashsig_compare(a.sig, b.sig) };
    if result < 0 {
        return result;
    }
    unsafe { *score = result };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree of the files with these names and contents.
    fn tree<'r>(repo: &'r Repository, files: &[(String, String)]) -> git2::Tree<'r> {
        let mut builder = repo.treebuilder(None).unwrap();
        for (name, content) in files {
            let oid = repo.blob(content.as_bytes()).unwrap();
            builder.insert(name, oid, 0o100644).unwrap();
        }
        repo.find_tree(builder.write().unwrap()).unwrap()
    }

    fn lines(seed: usize, count: usize) -> String {
        (0..count)
            .map(|i| {
                format!(
                    "line {} of file {}: {}\n",
                    i,
                    seed,
                    (i * 7919 + seed * 104729) % 1000003
                )
            })
            .collect()
    }

    #[test]
    fn test_same_as_libgit2s() {
        let dir =
            std::env::temp_dir().join(format!("mozsearch-similar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repository::init_bare(&dir).unwrap();
        // Renames (exact and edited), copies, rewrites (broken), small files,
        // files too different in size, deletions, and additions.
        let mut old = vec![];
        let mut new = vec![];
        for i in 0..40 {
            let content = lines(i, 20 + i * 3);
            old.push((format!("f{:02}", i), content.clone()));
            match i % 8 {
                0 => new.push((format!("renamed{:02}", i), content)),
                1 => new.push((
                    format!("edited{:02}", i),
                    content.replace("line 3 ", "LINE 3 "),
                )),
                // (With copies, libgit2 copies from modified files, not
                // unmodified ones.)
                2 => {
                    new.push((format!("f{:02}", i), content.replace("line 2 ", "LINE 2 ")));
                    new.push((
                        format!("copy{:02}", i),
                        content.replace("line 5 ", "LINE 5 "),
                    ));
                }
                3 => new.push((format!("f{:02}", i), lines(i + 1000, 30))),
                4 => new.push((format!("f{:02}", i), content.replace("line 1 ", "LINE 1 "))),
                5 => new.push((format!("small{:02}", i), format!("tiny {}\n", i))),
                6 => new.push((format!("big{:02}", i), content.repeat(20))),
                _ => {}
            }
        }
        for i in 0..10 {
            new.push((format!("added{:02}", i), lines(i + 2000, 25)));
        }
        old.sort();
        new.sort();
        let (old, new) = (tree(&repo, &old), tree(&repo, &new));
        let options = || {
            let mut opts = DiffFindOptions::new();
            opts.copies(true)
                .copy_threshold(30)
                .renames(true)
                .rename_threshold(30)
                .rename_limit(1000000)
                .break_rewrites(true)
                .break_rewrites_for_renames_only(true);
            opts
        };
        let summary = |diff: &Diff| -> Vec<String> {
            diff.deltas()
                .map(|d| {
                    format!(
                        "{:?} {:?} -> {:?}",
                        d.status(),
                        d.old_file().path(),
                        d.new_file().path()
                    )
                })
                .collect()
        };
        let mut expected = repo
            .diff_tree_to_tree(Some(&old), Some(&new), None)
            .unwrap();
        expected.find_similar(Some(&mut options())).unwrap();
        let mut actual = repo
            .diff_tree_to_tree(Some(&old), Some(&new), None)
            .unwrap();
        find_similar_above(&repo, &mut actual, &mut options(), 4, 0).unwrap();
        let expected = summary(&expected);
        assert_eq!(summary(&actual), expected);
        // (It found renames and copies, and broke rewrites.)
        for kind in ["Renamed", "Copied", "Added", "Deleted", "Modified"] {
            assert!(
                expected.iter().any(|d| d.starts_with(kind)),
                "{}: {:?}",
                kind,
                expected
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn options() -> DiffFindOptions {
        let mut opts = DiffFindOptions::new();
        opts.copies(true)
            .copy_threshold(30)
            .renames(true)
            .rename_threshold(30)
            .rename_limit(1000000)
            .break_rewrites(true)
            .break_rewrites_for_renames_only(true);
        opts
    }

    fn deltas(diff: &Diff) -> Vec<String> {
        diff.deltas()
            .map(|d| {
                format!(
                    "{:?} {:?} {} -> {:?} {}",
                    d.status(),
                    d.old_file().path(),
                    d.old_file().id(),
                    d.new_file().path(),
                    d.new_file().id()
                )
            })
            .collect()
    }

    #[test]
    fn test_shared_leaves_diffs_libgit2_leaves() {
        let dir = std::env::temp_dir().join(format!(
            "mozsearch-similar-shared-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repository::init_bare(&dir).unwrap();
        let mut state: u64 = 0x2545F4914F6CDD1D;
        let mut next = |n: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        // (Kept blobs, and blobs read each time.)
        let shared = [
            SharedBlobs::new(0, 1 << 30),
            SharedBlobs::new(1 << 30, 1 << 30),
        ];
        let (mut left, mut changed, mut added) = (0, 0, 0);
        for round in 0..200 {
            let mut old = vec![];
            let mut new = vec![];
            for i in 0..(1 + next(4)) {
                let name = format!("f{}", i);
                let content = lines(round * 10 + i, next(60));
                old.push((name.clone(), content.clone()));
                let edited = match next(11) {
                    // (Deleted.)
                    0 => continue,
                    // (Edited a little, or a lot.)
                    1..=3 => content.replacen("line 1", "LINE 1", 1),
                    4 => content.replace("line", "LINE"),
                    5 => lines(round * 10 + i + 5000, next(60)),
                    // (Sizes too different to compare.)
                    6 => content.repeat(10),
                    // (Small, or empty.)
                    7 => format!("tiny {}\n", next(3)),
                    8 => String::new(),
                    // (The first file's, edited: a copy or a rename, if it's
                    // rewritten or deleted.)
                    _ => old[0].1.replacen("line 2", "LINE 2", 1),
                };
                new.push((name, edited));
            }
            if next(8) == 0 {
                new.push(("added".to_string(), lines(round + 9000, 30)));
            }
            let (old, new) = (tree(&repo, &old), tree(&repo, &new));
            let expected = {
                let mut diff = repo
                    .diff_tree_to_tree(Some(&old), Some(&new), None)
                    .unwrap();
                let before = deltas(&diff);
                diff.find_similar(Some(&mut options())).unwrap();
                let after = deltas(&diff);
                let leaves = shared.each_ref().map(|shared| {
                    let diff = repo
                        .diff_tree_to_tree(Some(&old), Some(&new), None)
                        .unwrap();
                    leaves_unchanged(&repo, &diff, &mut options(), shared)
                });
                assert_eq!(leaves[0], leaves[1]);
                if leaves[0] {
                    assert_eq!(after, before, "round {}", round);
                    left += 1;
                } else if after != before {
                    changed += 1;
                }
                if before.iter().any(|d| d.starts_with("Added")) {
                    assert!(!leaves[0]);
                    added += 1;
                }
                after
            };
            for shared in &shared {
                let mut diff = repo
                    .diff_tree_to_tree(Some(&old), Some(&new), None)
                    .unwrap();
                find_similar_shared(&repo, &mut diff, &mut options(), 2, shared).unwrap();
                assert_eq!(deltas(&diff), expected, "round {}", round);
            }
        }
        // (Both kinds of diffs, and blobs kept and read again.)
        assert!(
            left > 20 && changed > 10 && added > 10,
            "{} {} {}",
            left,
            changed,
            added
        );
        let (reads, hits, _) = shared[0].stats();
        assert!(reads > 0 && hits > 0);
        assert_eq!(shared[1].stats().0, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_shared_blobs_read_once() {
        let dir = std::env::temp_dir().join(format!(
            "mozsearch-shared-blobs-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repository::init_bare(&dir).unwrap();
        let blobs: Vec<(Oid, String)> = (0..20)
            .map(|i| {
                // (Big ones, and small ones, which aren't kept.)
                let content = lines(i, if i % 4 == 0 { 10 } else { 3000 });
                (repo.blob(content.as_bytes()).unwrap(), content)
            })
            .collect();
        let shared = SharedBlobs::new(10_000, 1 << 30);
        thread::scope(|scope| {
            for t in 0..8 {
                let (dir, blobs, shared) = (&dir, &blobs, &shared);
                scope.spawn(move || {
                    let repo = Repository::open_bare(dir).unwrap();
                    for k in 0..(3 * blobs.len()) {
                        let (oid, content) = &blobs[(k * (t + 1)) % blobs.len()];
                        let read = shared
                            .with_blob(&repo, *oid, |c| String::from_utf8(c.to_vec()).unwrap());
                        assert_eq!(read.as_ref(), Some(content));
                        assert!(shared.signature(&repo, *oid, hashsig_options(0)).is_some());
                    }
                });
            }
        });
        // (Each big one once, by one of the threads.)
        let (reads, hits, _) = shared.stats();
        assert_eq!(reads, 15);
        assert!(hits > 0);
        // (And dropped, oldest first, past the limit.)
        let limited = SharedBlobs::new(10_000, 100_000);
        for (oid, _) in &blobs {
            limited.with_blob(&repo, *oid, |_| ()).unwrap();
        }
        let inner = limited.inner.lock().unwrap();
        assert!(inner.bytes <= 100_000 && !inner.blobs.is_empty());
        assert!(inner.blobs.contains_key(&blobs[19].0) && !inner.blobs.contains_key(&blobs[1].0));
        assert!(inner.loading.is_empty());
        drop(inner);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
