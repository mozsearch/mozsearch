// Compare the journals of two timeline repos built by build-timeline-tree from
// the same syntax repo, typically one with history consolidation and one
// without, by expanding their summary records (see `hyperblame::journals`).
// Expanded journals must have identical detail records, which verifies that
// consolidation doesn't lose or change anything, and every summary record must
// equal the aggregate of its expanded details (see
// `hyperblame::consolidation::verify_summaries`).
//
// Usage:
//   compare-timeline-journals TIMELINE_A TIMELINE_B [--all-revisions [--every N]]
//
// This compares every journal at the branch heads (`BLAME_REF`, defaulting to
// HEAD, like the history tools), which must be for the same source revision.
// With `--all-revisions`, it also compares the journals each revision changed
// in either repo, as of that revision (or only every Nth revision, plus all
// merges, with `--every N`).  Exits with an error status if anything differs.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::process::ExitCode;

use clap::Parser;
use git2::{Commit, DiffOptions, ObjectType, Oid, Repository, TreeWalkMode, TreeWalkResult};
use serde_json::Value;
use tools::file_format::config::timeline_commit_to_meta;
use tools::file_format::history::timeline_common::JournalVersionRef;
use tools::file_format::history::timeline_files_delta::FileDeltaRecord;
use tools::file_format::history::timeline_future::FutureRecord;
use tools::file_format::history::timeline_tokens::TokenDeltaRecord;
use tools::hyperblame::consolidation::verify_summaries;
use tools::hyperblame::journals::{JournalKind, JournalReader};

#[derive(Parser)]
struct Cli {
    /// The first timeline repo, typically built with consolidation.
    #[clap(value_parser)]
    timeline_a: String,

    /// The second timeline repo, typically built without consolidation.
    #[clap(value_parser)]
    timeline_b: String,

    /// Also compare the journals changed by each revision, as of that
    /// revision.
    #[clap(long, value_parser)]
    all_revisions: bool,

    /// With `--all-revisions`, only compare every Nth revision (in an arbitrary
    /// but stable order) and merges.
    #[clap(long, value_parser, default_value = "1")]
    every: usize,
}

/// How many differences to describe in detail.
const MAX_REPORTED: usize = 20;

#[derive(Default)]
struct Comparison {
    compared: usize,
    differing: usize,
    reported: usize,
    summaries: usize,
}

impl Comparison {
    fn report(&mut self, what: String) {
        self.differing += 1;
        if self.reported < MAX_REPORTED {
            self.reported += 1;
            println!("DIFFERENT: {}", what);
        }
    }
}

/// The expanded detail records of a journal version by source revision.
fn expanded_by_rev(
    reader: &mut JournalReader,
    kind: JournalKind,
    version: &JournalVersionRef,
) -> Result<BTreeMap<String, Value>, String> {
    let mut by_rev = BTreeMap::new();
    for record in reader.records_json(kind, version, true)? {
        let rev = record
            .get("source_rev")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("expanded record without a source_rev: {}", record))?
            .to_string();
        if by_rev.insert(rev.clone(), record).is_some() {
            return Err(format!("{} is listed twice", rev));
        }
    }
    Ok(by_rev)
}

/// Check the summaries of a journal version; see `verify_summaries`.
fn check_summaries(
    reader: &mut JournalReader,
    kind: JournalKind,
    version: &JournalVersionRef,
    comparison: &mut Comparison,
) {
    let result = match kind {
        JournalKind::Future => verify_summaries::<FutureRecord>(reader, version),
        JournalKind::FilesDelta => verify_summaries::<FileDeltaRecord>(reader, version),
        JournalKind::Tokens => verify_summaries::<TokenDeltaRecord>(reader, version),
    };
    match result {
        Ok((checked, problems)) => {
            comparison.summaries += checked;
            for problem in problems {
                comparison.report(problem);
            }
        }
        Err(e) => comparison.report(format!("{}:{}: {}", version.timeline_rev, version.path, e)),
    }
}

/// Compare the journal at `path` in the commits `a` and `b` of the two repos.
fn compare_journal(
    readers: &mut (JournalReader, JournalReader),
    (a, b): (Oid, Oid),
    path: &str,
    comparison: &mut Comparison,
) {
    let Some(kind) = JournalKind::for_path(path) else {
        return;
    };
    comparison.compared += 1;
    let version = |rev: Oid| JournalVersionRef {
        timeline_rev: rev.to_string(),
        path: path.to_string(),
    };
    check_summaries(&mut readers.0, kind, &version(a), comparison);
    check_summaries(&mut readers.1, kind, &version(b), comparison);
    let (records_a, records_b) = match (
        expanded_by_rev(&mut readers.0, kind, &version(a)),
        expanded_by_rev(&mut readers.1, kind, &version(b)),
    ) {
        (Ok(records_a), Ok(records_b)) => (records_a, records_b),
        (Err(e), _) => return comparison.report(format!("{} in A {}: {}", path, a, e)),
        (_, Err(e)) => return comparison.report(format!("{} in B {}: {}", path, b, e)),
    };
    if records_a == records_b {
        return;
    }
    let only_a: Vec<&String> = records_a
        .keys()
        .filter(|rev| !records_b.contains_key(*rev))
        .collect();
    let only_b: Vec<&String> = records_b
        .keys()
        .filter(|rev| !records_a.contains_key(*rev))
        .collect();
    let changed: Vec<&String> = records_a
        .iter()
        .filter(|(rev, record)| records_b.get(*rev).is_some_and(|other| other != *record))
        .map(|(rev, _)| rev)
        .collect();
    comparison.report(format!(
        "{} (A {}, B {}): only in A {:?}, only in B {:?}, different {:?}",
        path, a, b, only_a, only_b, changed
    ));
}

/// All the journal paths in a commit's tree.
fn journal_paths(repo: &Repository, commit: &Commit) -> Result<BTreeSet<String>, git2::Error> {
    let mut paths = BTreeSet::new();
    let tree = commit.tree()?;
    for kind in JournalKind::ALL {
        let Ok(entry) = tree.get_path(std::path::Path::new(kind.dir())) else {
            continue;
        };
        let subtree = repo.find_tree(entry.id())?;
        subtree.walk(TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(ObjectType::Blob)
                && let Ok(name) = entry.name()
            {
                paths.insert(format!("{}/{}{}", kind.dir(), dir, name));
            }
            TreeWalkResult::Ok
        })?;
    }
    Ok(paths)
}

/// The journal paths a commit changed relative to its first parent.
fn changed_journal_paths(
    repo: &Repository,
    commit: &Commit,
) -> Result<BTreeSet<String>, git2::Error> {
    let tree = commit.tree()?;
    let parent_tree = match commit.parents().next() {
        Some(parent) => Some(parent.tree()?),
        None => None,
    };
    let mut opts = DiffOptions::new();
    for kind in JournalKind::ALL {
        opts.pathspec(kind.dir());
    }
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
    let mut paths = BTreeSet::new();
    for delta in diff.deltas() {
        for file in [delta.old_file(), delta.new_file()] {
            if let Some(path) = file.path() {
                paths.insert(path.to_string_lossy().into_owned());
            }
        }
    }
    Ok(paths)
}

/// Map source revisions to the timeline commits for them on the branch.
fn commits_by_source_rev(repo: &Repository, head: Oid) -> Result<HashMap<Oid, Oid>, git2::Error> {
    let mut walk = repo.revwalk()?;
    walk.push(head)?;
    let mut by_source_rev = HashMap::new();
    for oid in walk {
        let oid = oid?;
        let meta = timeline_commit_to_meta(&repo.find_commit(oid)?);
        by_source_rev.insert(meta.source_rev, oid);
    }
    Ok(by_source_rev)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let blame_ref = env::var("BLAME_REF").unwrap_or("HEAD".to_string());
    let repo_a = Repository::open(&cli.timeline_a).unwrap();
    let repo_b = Repository::open(&cli.timeline_b).unwrap();
    let head_a = repo_a.refname_to_id(&blame_ref).unwrap();
    let head_b = repo_b.refname_to_id(&blame_ref).unwrap();
    let commit_a = repo_a.find_commit(head_a).unwrap();
    let commit_b = repo_b.find_commit(head_b).unwrap();
    let (source_a, source_b) = (
        timeline_commit_to_meta(&commit_a).source_rev,
        timeline_commit_to_meta(&commit_b).source_rev,
    );
    if source_a != source_b {
        eprintln!(
            "The heads are for different source revisions: {} and {}",
            source_a, source_b
        );
        return ExitCode::FAILURE;
    }

    // ## The journals at the heads.
    let mut paths = journal_paths(&repo_a, &commit_a).unwrap();
    paths.extend(journal_paths(&repo_b, &commit_b).unwrap());
    let mut readers = (JournalReader::new(&repo_a), JournalReader::new(&repo_b));
    let mut heads = Comparison::default();
    for path in &paths {
        compare_journal(&mut readers, (head_a, head_b), path, &mut heads);
    }
    println!(
        "Heads (source revision {}): {} of {} journals differ ({} summaries checked)",
        source_a, heads.differing, heads.compared, heads.summaries
    );
    let mut differing = heads.differing;

    // ## The journals each revision changed.
    if cli.all_revisions {
        let by_source_b = commits_by_source_rev(&repo_b, head_b).unwrap();
        let mut revisions = Comparison::default();
        let mut num_revisions = 0;
        let mut by_source_a: Vec<(Oid, Oid)> = commits_by_source_rev(&repo_a, head_a)
            .unwrap()
            .into_iter()
            .collect();
        by_source_a.sort();
        for (idx, (source_rev, rev_a)) in by_source_a.into_iter().enumerate() {
            let Some(&rev_b) = by_source_b.get(&source_rev) else {
                revisions.report(format!("{} is only in A", source_rev));
                continue;
            };
            let commit_a = repo_a.find_commit(rev_a).unwrap();
            if idx % cli.every.max(1) != 0 && commit_a.parent_count() < 2 {
                continue;
            }
            num_revisions += 1;
            let mut paths = changed_journal_paths(&repo_a, &commit_a).unwrap();
            paths.extend(
                changed_journal_paths(&repo_b, &repo_b.find_commit(rev_b).unwrap()).unwrap(),
            );
            // A reader per revision keeps the cache from growing without bound.
            let mut readers = (JournalReader::new(&repo_a), JournalReader::new(&repo_b));
            for path in &paths {
                compare_journal(&mut readers, (rev_a, rev_b), path, &mut revisions);
            }
        }
        println!(
            "All {} revisions: {} of {} changed journal versions differ ({} summaries checked)",
            num_revisions, revisions.differing, revisions.compared, revisions.summaries
        );
        differing += revisions.differing;
    }

    if differing == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
