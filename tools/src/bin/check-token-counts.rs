// Check that token journals' records sum to their tokens' tracked instances
// (see `hyperblame::token_counts`): at a timeline commit (the head by
// default), each token journal's sum (its head and segments) against the
// instances in its syntax commit's files.  The journals only sum exactly if
// every revision's records are exact, including merges' (whose journals union
// their parents' records; see build-timeline-tree's "Merges"), which is what
// `token-timeline:`'s absolute counts need.
//
// Usage: check-token-counts HISTORY_DIR [--timeline-rev REV] [--show N]
//        [TOKEN...]
//
// Prints the tokens whose sums differ, the biggest differences first, and
// totals; exits 1 if any differ.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use git2::{Oid, Repository, TreeWalkMode, TreeWalkResult};
use tools::file_format::config::timeline_commit_to_meta;
use tools::hyperblame::segments;
use tools::hyperblame::token_counts::{journal_sum, tree_counts};

#[derive(Parser)]
struct Args {
    /// The history directory (with the syntax and timeline repos).
    history: String,
    /// The timeline commit to check (default: HEAD).
    #[clap(long)]
    timeline_rev: Option<String>,
    /// How many differing tokens to print.
    #[clap(long, default_value = "40")]
    show: usize,
    /// Instead, walk this many commits back along the first parents, and
    /// check each one's changes: its journals' sums' changes against its
    /// syntax files' counts' (to find the revisions whose records are off).
    #[clap(long)]
    walk: Option<usize>,
    /// Only these tokens (default: all).
    tokens: Vec<String>,
}

/// A token journal's sum at `tree`, if it has the journal.
fn journal_sum_at(repo: &Repository, tree: &git2::Tree, path: &Path) -> Option<i64> {
    let head = blob_text(repo, tree, path)?;
    let years = segments::segment_years(head.lines().next().unwrap_or(""));
    let texts: Vec<String> = years
        .iter()
        .filter_map(|year| blob_text(repo, tree, &segments::segment_path(path, *year)))
        .collect();
    Some(journal_sum(&segments::join(
        &head,
        texts.iter().map(String::as_str),
    )))
}

/// The changes of the tracked counts between two syntax trees' files.
fn count_changes(
    repo: &Repository,
    old: &git2::Tree,
    new: &git2::Tree,
) -> Result<HashMap<String, i64>, git2::Error> {
    let mut changes = HashMap::new();
    let files = |tree: &git2::Tree| {
        tree.get_path(Path::new("files"))
            .ok()
            .and_then(|entry| repo.find_tree(entry.id()).ok())
    };
    let (old_files, new_files) = (files(old), files(new));
    let diff = repo.diff_tree_to_tree(old_files.as_ref(), new_files.as_ref(), None)?;
    for delta in diff.deltas() {
        for (file, root, sign) in [(delta.old_file(), old, -1), (delta.new_file(), new, 1)] {
            if file.id().is_zero() {
                continue;
            }
            let path = file.path().unwrap().to_string_lossy().into_owned();
            let blob = repo.find_blob(file.id())?;
            if let Ok(contents) = std::str::from_utf8(blob.content()) {
                let namespace = tools::hyperblame::token_counts::file_namespace(repo, root, &path);
                tools::hyperblame::token_counts::add_tracked_counts(
                    &namespace,
                    contents,
                    sign,
                    &mut changes,
                );
            }
        }
    }
    changes.retain(|_, n| *n != 0);
    Ok(changes)
}

/// `--walk`: each commit's journals' changes against its counts' changes.
fn walk(args: &Args, timeline: &Repository, syntax: &Repository, start: git2::Commit) -> ExitCode {
    let only: BTreeSet<&String> = args.tokens.iter().collect();
    let mut commit = start;
    let (mut bad_merges, mut bad_linear, mut merges, mut linear) = (0, 0, 0, 0);
    let mut off_by_kind: HashMap<&str, i64> = HashMap::new();
    for _ in 0..args.walk.unwrap() {
        let Ok(parent) = commit.parent(0) else {
            break;
        };
        let meta = timeline_commit_to_meta(&commit);
        let parent_meta = timeline_commit_to_meta(&parent);
        let syntax_new = syntax.find_commit(meta.syntax_rev).unwrap().tree().unwrap();
        let syntax_old = syntax
            .find_commit(parent_meta.syntax_rev)
            .unwrap()
            .tree()
            .unwrap();
        let counts = count_changes(syntax, &syntax_old, &syntax_new).unwrap();
        let (new_tree, old_tree) = (commit.tree().unwrap(), parent.tree().unwrap());
        let diff = timeline
            .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)
            .unwrap();
        let mut tokens: BTreeSet<String> = counts.keys().cloned().collect();
        for delta in diff.deltas() {
            for file in [delta.old_file(), delta.new_file()] {
                let path = file.path().unwrap().to_string_lossy();
                let Some(rest) = path.strip_prefix("tokens/") else {
                    continue;
                };
                // (A head, or a segment: tokens/AB/CD/TOKEN.ndjson[.d/YEAR.ndjson].)
                let name = rest.splitn(3, '/').nth(2).unwrap_or("");
                let token = name.split(".ndjson").next().unwrap_or("");
                if !token.is_empty() {
                    tokens.insert(token.to_string());
                }
            }
        }
        let is_merge = commit.parent_count() > 1;
        let mut off = vec![];
        for token in tokens {
            if !only.is_empty() && !only.contains(&token) {
                continue;
            }
            let path = tools::file_format::history::timeline_tokens::token_timeline_path(&token);
            let journal = journal_sum_at(timeline, &new_tree, &path).unwrap_or(0)
                - journal_sum_at(timeline, &old_tree, &path).unwrap_or(0);
            let count = counts.get(&token).copied().unwrap_or(0);
            if journal != count {
                off.push((token, count, journal));
            }
        }
        if is_merge {
            merges += 1;
        } else {
            linear += 1;
        }
        if !off.is_empty() {
            let kind = if is_merge { "merge" } else { "linear" };
            *off_by_kind.entry(kind).or_default() +=
                off.iter().map(|(_, c, j)| (c - j).abs()).sum::<i64>();
            if is_merge {
                bad_merges += 1;
            } else {
                bad_linear += 1;
            }
            off.sort_by_key(|(_, c, j)| std::cmp::Reverse((c - j).abs()));
            println!(
                "{} {} ({}): {} tokens off: {}",
                kind,
                meta.source_rev,
                commit.time().seconds(),
                off.len(),
                off.iter()
                    .take(6)
                    .map(|(t, c, j)| format!("{} {:+} counted, {:+} journaled", t, c, j))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        commit = parent;
    }
    println!(
        "{} of {} merges and {} of {} linear revisions have journals off (by {:?})",
        bad_merges, merges, bad_linear, linear, off_by_kind
    );
    if bad_merges + bad_linear == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn blob_text(repo: &Repository, tree: &git2::Tree, path: &Path) -> Option<String> {
    let entry = tree.get_path(path).ok()?;
    String::from_utf8(repo.find_blob(entry.id()).ok()?.content().to_vec()).ok()
}

fn main() -> ExitCode {
    let args = Args::parse();
    let timeline = Repository::open(Path::new(&args.history).join("timeline")).unwrap();
    let syntax = Repository::open(Path::new(&args.history).join("syntax")).unwrap();
    let commit = match &args.timeline_rev {
        Some(rev) => timeline.find_commit(Oid::from_str(rev).unwrap()).unwrap(),
        None => timeline.head().unwrap().peel_to_commit().unwrap(),
    };
    if args.walk.is_some() {
        return walk(&args, &timeline, &syntax, commit);
    }
    let meta = timeline_commit_to_meta(&commit);
    let syntax_tree = syntax.find_commit(meta.syntax_rev).unwrap().tree().unwrap();
    let counts = tree_counts(&syntax, &syntax_tree).unwrap();

    // The journals' sums.
    let tree = commit.tree().unwrap();
    let mut journals = vec![];
    if let Ok(tokens) = tree.get_path(Path::new("tokens")) {
        let tokens = timeline.find_tree(tokens.id()).unwrap();
        tokens
            .walk(TreeWalkMode::PreOrder, |dir, entry| {
                let name = entry.name().unwrap_or("");
                if entry.kind() == Some(git2::ObjectType::Blob)
                    && !dir.contains(".ndjson.d/")
                    && let Some(token) = name.strip_suffix(".ndjson")
                {
                    journals.push((token.to_string(), format!("tokens/{}{}", dir, name)));
                }
                TreeWalkResult::Ok
            })
            .unwrap();
    }
    let only: BTreeSet<&String> = args.tokens.iter().collect();
    let mut sums: HashMap<String, i64> = HashMap::new();
    // (Journals whose headers' counts aren't their sums, and those without.)
    let mut header_off = vec![];
    let mut uncounted = 0;
    for (token, path) in journals {
        if !only.is_empty() && !only.contains(&token) {
            continue;
        }
        let path = Path::new(&path);
        let Some(head) = blob_text(&timeline, &tree, path) else {
            continue;
        };
        let years = segments::segment_years(head.lines().next().unwrap_or(""));
        let texts: Vec<String> = years
            .iter()
            .filter_map(|year| blob_text(&timeline, &tree, &segments::segment_path(path, *year)))
            .collect();
        let sum = journal_sum(&segments::join(&head, texts.iter().map(String::as_str)));
        match segments::header_count(head.lines().next().unwrap_or("")) {
            Some(count) if count != sum => header_off.push((token.clone(), count, sum)),
            Some(_) => {}
            None => uncounted += 1,
        }
        sums.insert(token, sum);
    }
    for (token, count, sum) in header_off.iter().take(args.show) {
        println!(
            "{}: its header counts {}, its records sum to {}",
            token, count, sum
        );
    }

    // The differences, with the tokens without journals (but with instances).
    let mut tokens: BTreeSet<&String> = sums.keys().collect();
    if only.is_empty() {
        tokens.extend(counts.iter().filter(|(_, n)| **n != 0).map(|(t, _)| t));
    } else {
        tokens.extend(only.iter().copied());
    }
    let mut differing: Vec<(i64, &String, i64, i64)> = tokens
        .into_iter()
        .map(|token| {
            let count = counts.get(token).copied().unwrap_or(0);
            let sum = sums.get(token).copied().unwrap_or(0);
            ((sum - count).abs(), token, count, sum)
        })
        .filter(|(diff, ..)| *diff != 0)
        .collect();
    differing.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    for (diff, token, count, sum) in differing.iter().take(args.show) {
        println!(
            "{}: {} instances, journal sums to {} (off by {})",
            token, count, sum, diff
        );
    }
    let total_off: i64 = differing.iter().map(|(diff, ..)| diff).sum();
    println!(
        "{} of {} journals and counted tokens differ (by {} instances in all); timeline {} (source {})",
        differing.len(),
        sums.len().max(counts.len()),
        total_off,
        commit.id(),
        meta.source_rev
    );
    println!(
        "{} journals' headers' counts aren't their sums; {} journals have no count",
        header_off.len(),
        uncounted
    );
    if differing.is_empty() && header_off.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
