//! Write the token indexes (see `file_format::history::rev_token_index`) of
//! a history's revisions whose rev-summaries have totals rather than file
//! deltas, for histories built before build-timeline-tree wrote them:
//!
//! ```text
//! index-mass-revisions HISTORY SOURCE_REPO [--branch BRANCH] [--revs FILE] [--force] [--check]
//! ```
//!
//! A revision's files and the tokens they changed are as its summary would
//! have listed them: its added, modified and moved files' from their
//! files-delta journals' records of it (at its timeline commit); and from
//! their tokenized versions before it (in the syntax repo, at its parent's
//! revision), every token of which it removed, those of its deleted files
//! (unless moved elsewhere, but not copies' sources), of its files rewritten
//! in place or replaced by another moved onto them (whose journals have their
//! additions), and of its files whose new versions aren't tracked (whose
//! journals are gone), by the keys the journals track them by (see
//! `tracked_token_keys`).  (In no namespace, which leaves out none of a
//! namespace's value words, which only adds them to the index: a lookup never
//! misses a file.)  A history window's first revision (whose parent isn't in
//! the history) adds every file.
//!
//! The history's branch is its repos' HEAD's, or `--branch`'s.  Without
//! `--revs` (a file of revisions, one a line), it finds the revisions whose
//! summaries have totals itself, by reading the start of every summary, unless
//! a run over all of them has (see `COMPLETE_STAMP`), and it skips revisions
//! which have an index, unless `--force`.  With `--check`, the revisions are
//! ones whose summaries have their file deltas, and rather than writing
//! indexes, it compares what it finds with those, to check that it finds
//! every file and token they have, and no other file.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use git2::{Delta, Oid, Repository};
use rayon::prelude::*;
use serde::Deserialize;

use tools::file_format::config::ThreadLocalRepository;
use tools::file_format::history::rev_summaries::{RevSummaryRecord, rev_summary_path};
use tools::file_format::history::rev_token_index::{
    self, IndexedFile, indexed_files, rev_token_index_path,
};
use tools::file_format::history::syntax_files::{TokenLine, split_token_line, token_file_lines};
use tools::file_format::history::timeline_common::{ChangeKind, JournalVersionRef};
use tools::file_format::history::timeline_files_delta::FileDeltaRecord;
use tools::file_format::history::timeline_tokens::tracked_token_keys;
use tools::hyperblame::journals::{JournalKind, JournalReader};
use tools::hyperblame::token_blame::TreeHistory;

/// The file in the rev-summaries which says that every revision whose summary
/// has totals has an index (written after a run over all of them), after
/// which build-timeline-tree writes new ones' indexes.
const COMPLETE_STAMP: &str = "token-indexes-complete";

/// How much of a summary is read to tell whether it has file deltas (which
/// come after its message).
const SUMMARY_PREFIX_BYTES: usize = 64 * 1024;

/// What of a summary is needed of a revision's parent.
#[derive(Deserialize)]
struct SyntaxRev {
    syntax_rev: String,
}

fn read_summary(rev_summaries: &Path, rev: &str) -> Option<RevSummaryRecord> {
    serde_json::from_slice(&fs::read(rev_summaries.join(rev_summary_path(rev))).ok()?).ok()
}

/// Whether the summary at `path` has totals rather than file deltas.
fn has_totals(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut prefix = vec![0; SUMMARY_PREFIX_BYTES];
    let mut len = 0;
    while len < prefix.len() {
        match file.read(&mut prefix[len..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => len += n,
        }
    }
    let prefix = &prefix[..len];
    // (serde writes an empty map as `{}`; a merge's are empty too, without
    // totals.)
    if !prefix
        .windows(17)
        .any(|w| w == br#""file_deltas":{},"#.as_slice())
    {
        return false;
    }
    fs::read(path).is_ok_and(|contents| {
        serde_json::from_slice::<RevSummaryRecord>(&contents)
            .is_ok_and(|summary| summary.file_totals.is_some())
    })
}

/// The revisions whose summaries have totals, under `rev_summaries`.
fn mass_revisions(rev_summaries: &Path) -> Vec<String> {
    let mut dirs: Vec<PathBuf> = vec![];
    for top in fs::read_dir(rev_summaries.join("by-source-rev"))
        .into_iter()
        .flatten()
        .flatten()
    {
        dirs.extend(
            fs::read_dir(top.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path()),
        );
    }
    let scanned = AtomicUsize::new(0);
    let started = Instant::now();
    let mut revs: Vec<String> = dirs
        .par_iter()
        .flat_map_iter(|dir| {
            let found: Vec<String> = fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let rev = name.strip_suffix(".json")?.to_string();
                    has_totals(&entry.path()).then_some(rev)
                })
                .collect();
            let n = scanned.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 4096 == 0 {
                eprintln!(
                    "Scanned {} of {} directories ({:.0?})",
                    n,
                    dirs.len(),
                    started.elapsed()
                );
            }
            found
        })
        .collect();
    revs.sort();
    revs
}

/// The files of `rev` and the tokens they changed, as its summary would list
/// them (see the module's comment), and how many files' tokens came from the
/// syntax repo.
fn revision_files(
    history: &TreeHistory,
    source: &Repository,
    rev_summaries: &Path,
    rev: &str,
) -> Result<(Vec<IndexedFile>, usize), String> {
    let oid = Oid::from_str(rev).map_err(|e| e.to_string())?;
    let commit = source.find_commit(oid).map_err(|e| e.to_string())?;
    let timeline_rev = history
        .timeline_commit(oid)
        .ok_or("no timeline commit")?
        .id()
        .to_string();
    // (Its parent in the history: the first parent, if it has one, and if the
    // history has it (a history window's first revision adds every file).)
    let parent_syntax: Option<SyntaxRev> = commit.parent_id(0).ok().and_then(|parent| {
        fs::read(rev_summaries.join(rev_summary_path(&parent.to_string())))
            .ok()
            .and_then(|contents| serde_json::from_slice(&contents).ok())
    });
    let parent_tree = match parent_syntax {
        Some(_) => Some(
            commit
                .parent(0)
                .and_then(|parent| parent.tree())
                .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let diff = source
        .diff_tree_to_tree(
            parent_tree.as_ref(),
            Some(&commit.tree().map_err(|e| e.to_string())?),
            None,
        )
        .map_err(|e| e.to_string())?;
    let mut changed = vec![];
    let mut deleted = vec![];
    for delta in diff.deltas() {
        let path = |file: git2::DiffFile| file.path().map(|p| p.to_string_lossy().into_owned());
        match delta.status() {
            Delta::Deleted => deleted.extend(path(delta.old_file())),
            status => {
                changed.extend(path(delta.new_file()).map(|path| (path, status == Delta::Modified)))
            }
        }
    }

    // (A file's tokenized version before the revision, by the keys the
    // journals track them by, as build-timeline-tree's `compute_revision_stats`
    // has them, but in no namespace, which leaves out none of its value
    // words.)
    let syntax_tree = parent_syntax
        .and_then(|p| Oid::from_str(&p.syntax_rev).ok())
        .and_then(|oid| history.syntax.find_commit(oid).ok())
        .and_then(|commit| commit.tree().ok());
    let old_keys = |path: &str| -> Result<Option<Vec<String>>, String> {
        let Some(tree) = &syntax_tree else {
            return Err(format!("no syntax revision for {}'s parent", rev));
        };
        let Some(blob) = tree
            .get_path(Path::new(&format!("files/{}", path)))
            .ok()
            .and_then(|entry| history.syntax.find_blob(entry.id()).ok())
        else {
            return Ok(None);
        };
        let contents = String::from_utf8_lossy(blob.content());
        let lines: Vec<TokenLine> = token_file_lines(&contents)
            .into_iter()
            .map(split_token_line)
            .collect();
        let mut keys: Vec<String> = vec![];
        for (at, line) in lines.iter().enumerate() {
            let prev = at.checked_sub(1).map(|prev| &lines[prev]);
            tracked_token_keys("", line, prev, &mut |key| keys.push(key.into_owned()));
        }
        Ok(Some(keys))
    };

    let mut files = vec![];
    let mut from_syntax = 0;
    let mut moved_from = HashSet::new();
    let mut reader = JournalReader::new(&history.timeline);
    for (path, modified) in changed {
        let version = JournalVersionRef {
            timeline_rev: timeline_rev.clone(),
            path: JournalKind::FilesDelta.journal_path(&path),
        };
        let records = reader
            .records::<FileDeltaRecord>(&version)
            .unwrap_or_default();
        if records.is_empty() {
            // (A modified file without a journal is one whose new version
            // isn't tracked (ex: it became too big): a removal of its old
            // version's tokens.)
            if modified && let Some(keys) = old_keys(&path)? {
                files.push(IndexedFile::new(
                    path,
                    None,
                    keys.iter().map(String::as_str),
                ));
                from_syntax += 1;
            }
            continue;
        }
        // (Its newest record, at its timeline commit, is its, if it has one.)
        let Some(FileDeltaRecord::Detail(detail)) = records.into_iter().next() else {
            continue;
        };
        if detail.desc.source_rev != rev {
            continue;
        }
        // (A deleted file which was moved here isn't a removal, but one
        // copied here is, as build-timeline-tree has them.)
        if let Some(from) = &detail.delta.moved_from
            && !detail.delta.copied
        {
            moved_from.insert(from.clone());
        }
        // (A file rewritten in place, or replaced by another moved there, is
        // its removal and an addition, whose journal has the addition: its
        // removal's tokens are its version's before.)
        let rewritten = if modified && detail.delta.change != ChangeKind::Changed {
            from_syntax += 1;
            old_keys(&path)?.unwrap_or_default()
        } else {
            vec![]
        };
        let tokens = detail
            .delta
            .symbol_group
            .symbol_deltas
            .values()
            .flat_map(|symbol| symbol.token_changes.keys().map(String::as_str));
        files.push(IndexedFile::new(
            path,
            detail.delta.moved_from.clone(),
            tokens.chain(rewritten.iter().map(String::as_str)),
        ));
    }

    // (Deleted files, but those moved elsewhere: every token of their
    // versions before the revision.)
    for path in deleted {
        if moved_from.contains(&path) {
            continue;
        }
        let Some(keys) = old_keys(&path)? else {
            continue;
        };
        files.push(IndexedFile::new(
            path,
            None,
            keys.iter().map(String::as_str),
        ));
        from_syntax += 1;
    }
    Ok((files, from_syntax))
}

fn main() {
    env_logger::init();
    git2::opts::strict_hash_verification(false);
    let args: Vec<String> = std::env::args().collect();
    let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let option = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|at| args.get(at + 1))
    };
    let revs_file = option("--revs");
    let branch = option("--branch");
    let positional: Vec<&String> = positional
        .into_iter()
        .filter(|a| Some(*a) != revs_file && Some(*a) != branch)
        .collect();
    let [history_dir, source_dir] = positional[..] else {
        eprintln!(
            "Usage: index-mass-revisions HISTORY SOURCE_REPO [--branch BRANCH] [--revs FILE] [--force] [--check]"
        );
        std::process::exit(1);
    };
    let (force, check) = (flag("--force"), flag("--check"));
    let history = TreeHistory::open(history_dir, branch.map(String::as_str)).expect("the history");
    let source: ThreadLocalRepository = Repository::open(source_dir)
        .expect("the source repo")
        .into();
    let rev_summaries = Path::new(history_dir).join("rev-summaries");

    let stamp = rev_summaries.join(COMPLETE_STAMP);
    if revs_file.is_none() && !check && !force && stamp.exists() {
        println!(
            "Every revision whose summary has totals has an index (see {:?}).",
            stamp
        );
        return;
    }
    let started = Instant::now();
    let mut revs: Vec<String> = match revs_file {
        Some(file) => fs::read_to_string(file)
            .expect("the revisions")
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect(),
        None => mass_revisions(&rev_summaries),
    };
    if !check && !force {
        revs.retain(|rev| !rev_summaries.join(rev_token_index_path(rev)).exists());
    }
    eprintln!(
        "{} revisions to {} ({:.0?} to find them)",
        revs.len(),
        if check { "check" } else { "index" },
        started.elapsed()
    );

    let started = Instant::now();
    let done = AtomicUsize::new(0);
    let totals = std::sync::Mutex::new(BTreeMap::<&str, u64>::new());
    let add = |name: &'static str, n: u64| *totals.lock().unwrap().entry(name).or_default() += n;
    revs.par_iter().for_each(|rev| {
        let result = revision_files(&history, &source, &rev_summaries, rev);
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        let (files, from_syntax) = match result {
            Ok(found) => found,
            Err(e) => {
                eprintln!("{}: {}", rev, e);
                add("failed", 1);
                return;
            }
        };
        let summary = read_summary(&rev_summaries, rev);
        if check {
            // (Every file and token of the summary's, found.)
            let Some(summary) = summary else {
                add("failed", 1);
                return;
            };
            // (One with totals has nothing to check against.)
            if summary.file_totals.is_some() {
                add("with totals", 1);
                return;
            }
            let found: BTreeMap<&str, &IndexedFile> = files
                .iter()
                .map(|file| (file.path.as_str(), file))
                .collect();
            let expected = indexed_files(&summary.file_deltas);
            let (mut missing_files, mut missing_hashes, mut extra_files) = (0, 0, 0);
            for file in &expected {
                match found.get(file.path.as_str()) {
                    None => {
                        missing_files += 1;
                        eprintln!("{}: missing file {}", rev, file.path);
                    }
                    Some(found) => {
                        let found: BTreeSet<u32> = found.hashes.iter().copied().collect();
                        let missing = file.hashes.iter().filter(|h| !found.contains(h)).count();
                        if missing > 0 {
                            eprintln!("{}: {} tokens missing of {}", rev, missing, file.path);
                        }
                        missing_hashes += missing;
                    }
                }
            }
            let expected: BTreeSet<&str> = expected.iter().map(|f| f.path.as_str()).collect();
            extra_files += found
                .keys()
                .filter(|path| !expected.contains(*path))
                .count();
            add("files", expected.len() as u64);
            add("missing files", missing_files);
            add("missing tokens", missing_hashes as u64);
            add("extra files", extra_files as u64);
            add("from syntax", from_syntax as u64);
        } else {
            let expected = summary
                .as_ref()
                .and_then(|s| s.file_totals.as_ref())
                .map(|totals| totals.files.values().map(|n| *n as u64).sum::<u64>());
            if expected.is_some_and(|expected| expected != files.len() as u64) {
                add("file counts off", 1);
                log::info!(
                    "{}: {} files, its totals say {:?}",
                    rev,
                    files.len(),
                    expected
                );
            }
            add("files", files.len() as u64);
            add("from syntax", from_syntax as u64);
            let index = rev_token_index::encode(files);
            add("bytes", index.len() as u64);
            let path = rev_summaries.join(rev_token_index_path(rev));
            let tmp = path.with_extension("tokens.tmp");
            fs::write(&tmp, &index).expect("writing the index");
            fs::rename(&tmp, &path).expect("renaming the index");
        }
        if n % 100 == 0 || n == revs.len() {
            eprintln!(
                "{} of {} revisions ({:.0?}): {:?}",
                n,
                revs.len(),
                started.elapsed(),
                totals.lock().unwrap()
            );
        }
    });
    let totals = totals.into_inner().unwrap();
    let summary = format!(
        "{} revisions in {:.0?}: {:?}",
        revs.len(),
        started.elapsed(),
        totals
    );
    println!("{}", summary);
    // (Once, after a run over all of them: those which failed (ex: a
    // revision whose source commit is gone) would fail again.)
    if revs_file.is_none() && !check {
        fs::write(&stamp, format!("{}\n", summary)).expect("writing the stamp");
    }
}
