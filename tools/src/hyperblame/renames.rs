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

use std::collections::HashMap;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use git2::{Binding, Delta, Diff, DiffFindOptions, Oid, Repository};
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
}
