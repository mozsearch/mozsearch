//! The commit index: which commits mention a bug or a Phabricator revision,
//! for the `/explore/` endpoints (see "Commit index" in the hyperblame notes).
//!
//! It's derived from the commit messages by `build-commit-index` in the setup
//! phase, and kept beside the history (in `commit-index/` under the tree's
//! `history_path`) so that updates only need to process new commits.  Each
//! branch has its own index, in `commit-index/<branch>/` (see `branch_dir`),
//! like the history repos' branches, since trees like firefox-main and
//! firefox-beta share a history: one index for all of them couldn't tell
//! which commits are on a tree's branch quickly (checking that a commit is an
//! ancestor of a tree's head took 1-7s on firefox-main without a
//! commit-graph), and each copy is small (~80 MB for firefox-main).  In each:
//! - `state.json` has the schema version and the heads processed so far.  A
//!   different schema version means rebuilding from scratch, which is cheap
//!   (a revwalk and parsing the messages), as does a processed head which isn't
//!   an ancestor of the new head (ex: a rebased branch in development), whose
//!   commits may no longer be in the history, unless the index accumulates
//!   heads (see `UpdateOptions`).
//! - `by-bug` has `BUG<TAB>REV<TAB>ISO_DATE<TAB>FLAGS` lines and `by-phab` has
//!   `DNNN<TAB>REV<TAB>ISO_DATE<TAB>FLAGS` lines, sorted, so lookups can
//!   bisect.  FLAGS is "b" for backouts (by their summary lines) or "-".
//! - `by-rev` has `REV<TAB>LANDED<TAB>POSITION` lines, sorted, for each
//!   commit's place in the history (see `CommitPosition`).  Commit dates
//!   aren't when commits landed, nor in their order: Mercurial had one date,
//!   the author's, which git-cinnabar's commits have as their committer
//!   dates, and which a patch kept when it landed weeks later or was relanded
//!   after a backout (in firefox-main, 30% of 2016-2017's commits are dated
//!   more than a day before one of their ancestors, and 8% more than 14
//!   days).  A commit can't have landed before its parents, so LANDED is the
//!   latest of its and its ancestors' commit dates, and POSITION numbers the
//!   commits parents first, so that ordering by both is topological, with
//!   dates which only go forward.
//!
//! Trees like try (or review), which process many unrelated heads based on
//! other branches' revisions, accumulate their heads in one index, which
//! reads through to the indexes of the branches they're based on (ex: "main")
//! rather than having their commits too, like `source_mapping`'s NOTES_REF and
//! READ_NOTES_REFS.  And explore pages look for a bug's commits on other
//! branches (ex: uplifts) in their indexes (see `other_branches`).
//!
//! This doesn't come from the history processing (which has the messages in
//! its rev-summaries), because this covers more: all of the repo's commits,
//! not just the history's (ex: a window's), including try and review heads,
//! for 14s of processing for firefox-main from scratch.  What the history
//! knows besides, which commits backed out which, is in the rev-summaries
//! (`backs_out` and `backed_out_by`), which the explore pages read (see
//! `hyperblame::explore::mark_history_backouts`).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use git2::{Oid, Repository, Sort};
use lazy_static::lazy_static;
use memmap::Mmap;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Bump this when the files' contents change so existing indexes get rebuilt.
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    schema: u32,
    /// The heads whose history has been processed.
    heads: Vec<String>,
    /// See `UpdateOptions`.
    #[serde(default)]
    accumulate: bool,
    #[serde(default)]
    read: Vec<String>,
    /// How many commits `by-rev` has numbered (the next one's POSITION).
    #[serde(default)]
    revs: u64,
}

fn read_state(dir: &Path) -> State {
    fs::read_to_string(dir.join("state.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// How a branch's index is updated (see `update_branch`).
#[derive(Clone, Debug, Default)]
pub struct UpdateOptions {
    /// Keep every head's history, for trees like try (or review) which process
    /// many unrelated heads: a head which doesn't descend from the processed
    /// ones is added to them, rather than replacing them (and rebuilding the
    /// index).
    pub accumulate: bool,
    /// The branches (under the same `commit-index/`) whose indexes have the
    /// history this branch's heads are based on (ex: "main" for try), whose
    /// processed commits aren't processed again here, and which lookups read
    /// through to.
    pub read: Vec<String>,
}

/// A commit which mentions a bug or Phabricator revision.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct CommitRef {
    pub rev: String,
    #[serde(rename = "isoDate")]
    pub iso_date: String,
    /// Whether the commit is a backout.
    pub backout: bool,
}

/// A commit's ref, as the index has it (its date in UTC, and whether it's a
/// backout by its summary line), for commits which aren't from the index (ex:
/// a try push's; see `try_import`).
pub fn commit_ref(commit: &git2::Commit) -> CommitRef {
    let message = String::from_utf8_lossy(commit.message_bytes());
    CommitRef {
        rev: commit.id().to_string(),
        iso_date: iso_date(commit),
        backout: is_backout(&message),
    }
}

/// A commit's date as the index has it, in UTC.
fn iso_date(commit: &git2::Commit) -> String {
    iso_seconds(commit.time().seconds())
}

fn iso_seconds(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|date| date.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

fn iso_to_seconds(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|date| date.timestamp())
}

/// Where a commit is in the history (see `by-rev` in the module's comment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitPosition {
    /// When it landed, at the earliest: the latest of its and its ancestors'
    /// commit dates (ISO 8601, in UTC).
    pub landed: String,
    /// Whether it's from an index this one reads through to (see
    /// `UpdateOptions::read`), whose commits come before this one's.
    pub base: bool,
    /// Its place among its index's commits, parents first.
    pub position: u64,
}

impl CommitPosition {
    /// A key ordering commits topologically (parents first), by when they
    /// landed.
    pub fn order_key(&self) -> (&str, bool, u64) {
        (&self.landed, !self.base, self.position)
    }
}

/// The bug numbers a commit message's summary line mentions, like
/// `links::linkify_bug_numbers`.
pub fn message_bugs(message: &str) -> Vec<String> {
    lazy_static! {
        static ref BUG_REGEX: Regex =
            Regex::new(r"\b(?i)bug\s*(?P<bugno>[1-9][0-9]{2,6})\b").unwrap();
    }
    let summary = message.lines().next().unwrap_or("");
    let mut bugs: Vec<String> = BUG_REGEX
        .captures_iter(summary)
        .map(|c| c["bugno"].to_string())
        .collect();
    bugs.dedup();
    bugs
}

/// The Phabricator revisions (ex: "D12345") a commit message links to with a
/// "Differential Revision:" line.
pub fn message_phab_revs(message: &str) -> Vec<String> {
    lazy_static! {
        static ref PHAB_REGEX: Regex =
            Regex::new(r"(?m)^Differential Revision: \S*/(?P<rev>D[0-9]+)\s*$").unwrap();
    }
    PHAB_REGEX
        .captures_iter(message)
        .map(|c| c["rev"].to_string())
        .collect()
}

fn is_backout(message: &str) -> bool {
    lazy_static! {
        static ref BACKOUT_REGEX: Regex = Regex::new(r"^(?i)(back(ed|ing)? ?out|revert)").unwrap();
    }
    BACKOUT_REGEX.is_match(message.lines().next().unwrap_or(""))
}

/// The directory of `branch`'s index under a history's `commit-index/`, ex:
/// "commit-index/main", or "commit-index/HEAD" for trees without a
/// `git_branch`.
pub fn branch_dir(index_root: &Path, branch: &str) -> PathBuf {
    index_root.join(branch.replace('%', "%25").replace('/', "%2F"))
}

/// The branch of a git ref, for `branch_dir`: "refs/heads/beta" -> "beta",
/// and other refs (ex: "HEAD") as they are.
pub fn ref_branch(git_ref: &str) -> &str {
    git_ref.strip_prefix("refs/heads/").unwrap_or(git_ref)
}

/// The files of the index from before branches had their own, directly in
/// `commit-index/`.
const UNBRANCHED_FILES: [&str; 3] = ["state.json", "by-bug", "by-phab"];

/// A sorted file of `KEY<TAB>FIELDS...` lines, mapped, for bisecting.
struct SortedLines {
    mmap: Option<Mmap>,
}

impl SortedLines {
    fn open(path: &Path) -> SortedLines {
        let mmap = fs::File::open(path)
            .ok()
            .and_then(|file| unsafe { Mmap::map(&file) }.ok());
        SortedLines { mmap }
    }

    /// The fields after the key of the lines for `key`.
    fn find(&self, key: &str) -> Vec<Vec<String>> {
        let Some(mmap) = &self.mmap else {
            return vec![];
        };
        let data = &mmap[..];
        let line_end = |start: usize| {
            data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(data.len(), |i| start + i)
        };
        // Bisect for the first line starting with the key.  Every line
        // starting before `low` sorts before it, and every line starting at or
        // after `high` doesn't.
        let prefix = format!("{}\t", key);
        let prefix = prefix.as_bytes();
        let mut low = 0;
        let mut high = data.len();
        while low < high {
            let mid = low + (high - low) / 2;
            let start = data[low..mid]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(low, |i| low + i + 1);
            let end = line_end(start);
            if &data[start..end] < prefix {
                low = end + 1;
            } else {
                high = start;
            }
        }
        let mut found = vec![];
        let mut start = low;
        while start < data.len() && data[start..].starts_with(prefix) {
            let end = line_end(start);
            let line = String::from_utf8_lossy(&data[start + prefix.len()..end]);
            found.push(line.split('\t').map(str::to_string).collect());
            start = end + 1;
        }
        found
    }
}

/// The `by-rev` fields of a line (see `SortedLines::find`).
fn parse_position(fields: &[String], base: bool) -> Option<CommitPosition> {
    match fields {
        [landed, position, ..] => Some(CommitPosition {
            landed: landed.clone(),
            base,
            position: position.parse().ok()?,
        }),
        _ => None,
    }
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Write the lines sorted and without duplicates.
fn write_lines(path: &Path, mut lines: Vec<String>) -> std::io::Result<()> {
    lines.sort_unstable();
    lines.dedup();
    let tmp = path.with_extension("tmp");
    let mut file = std::io::BufWriter::new(fs::File::create(&tmp)?);
    for line in &lines {
        writeln!(file, "{}", line)?;
    }
    file.flush()?;
    drop(file);
    fs::rename(tmp, path)
}

/// Update the commit index in `dir` with the history of `head` in `repo`,
/// returning how many commits we processed.
pub fn update(repo: &Repository, head: Oid, dir: &Path) -> Result<usize, String> {
    update_with(repo, head, dir, &UpdateOptions::default(), &[])
}

/// `update` with `options`, hiding the processed heads of `bases` (the indexes
/// of the branches `options.read` names), which have their commits.
fn update_with(
    repo: &Repository,
    head: Oid,
    dir: &Path,
    options: &UpdateOptions,
    bases: &[CommitIndex],
) -> Result<usize, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let state_path = dir.join("state.json");
    let mut state = read_state(dir);
    let rewritten = !options.accumulate
        && state
            .heads
            .iter()
            .any(|processed| match Oid::from_str(processed) {
                Ok(oid) => oid != head && !repo.graph_descendant_of(head, oid).unwrap_or(false),
                Err(_) => true,
            });
    let rebuild = state.schema != SCHEMA_VERSION
        || rewritten
        || state.accumulate != options.accumulate
        || state.read != options.read;
    let (mut by_bug, mut by_phab, mut by_rev) = if rebuild {
        state = State {
            schema: SCHEMA_VERSION,
            heads: vec![],
            accumulate: options.accumulate,
            read: options.read.clone(),
            revs: 0,
        };
        (vec![], vec![], vec![])
    } else {
        (
            read_lines(&dir.join("by-bug")),
            read_lines(&dir.join("by-phab")),
            read_lines(&dir.join("by-rev")),
        )
    };
    // (The processed commits' lines, sorted as written, for their landing
    // dates.)
    let processed_revs = by_rev.len();

    let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
    // (Parents first, for their landing dates and positions.)
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)
        .map_err(|e| e.to_string())?;
    walk.push(head).map_err(|e| e.to_string())?;
    for processed in &state.heads {
        let oid = Oid::from_str(processed).map_err(|e| e.to_string())?;
        // (An accumulated head may be gone, ex: a try push's.)
        if walk.hide(oid).is_err() && !options.accumulate {
            return Err(format!("Couldn't hide processed head {}", oid));
        }
    }
    for base in bases {
        for processed in read_state(&base.dir).heads {
            if let Ok(oid) = Oid::from_str(&processed) {
                let _ = walk.hide(oid);
            }
        }
    }
    let base_revs: Vec<SortedLines> = bases
        .iter()
        .map(|base| SortedLines::open(&base.dir.join("by-rev")))
        .collect();
    let mut landed: HashMap<Oid, i64> = HashMap::new();
    let mut count = 0;
    for oid in walk {
        let oid = oid.map_err(|e| e.to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
        let message = String::from_utf8_lossy(commit.message_bytes());
        let date = iso_date(&commit);
        let flags = if is_backout(&message) { "b" } else { "-" };
        for bug in message_bugs(&message) {
            by_bug.push(format!("{}\t{}\t{}\t{}", bug, oid, date, flags));
        }
        for phab in message_phab_revs(&message) {
            by_phab.push(format!("{}\t{}\t{}\t{}", phab, oid, date, flags));
        }
        // (A parent is this walk's, or processed before, here or in a base
        // index, or else, ex: a gone try push's, has its own date.)
        let mut when = commit.time().seconds();
        for parent in commit.parents() {
            let rev = parent.id().to_string();
            let parent_landed = landed
                .get(&parent.id())
                .copied()
                .or_else(|| {
                    let lines = &by_rev[..processed_revs];
                    let i = lines.partition_point(|line| line.as_str() < rev.as_str());
                    lines
                        .get(i)
                        .and_then(|line| line.strip_prefix(&format!("{}\t", rev)))
                        .and_then(|fields| iso_to_seconds(fields.split('\t').next()?))
                })
                .or_else(|| {
                    base_revs.iter().find_map(|lines| {
                        let fields = lines.find(&rev).into_iter().next()?;
                        iso_to_seconds(&parse_position(&fields, true)?.landed)
                    })
                })
                .unwrap_or_else(|| parent.time().seconds());
            when = when.max(parent_landed);
        }
        landed.insert(oid, when);
        by_rev.push(format!("{}\t{}\t{}", oid, iso_seconds(when), state.revs));
        state.revs += 1;
        count += 1;
    }

    write_lines(&dir.join("by-bug"), by_bug).map_err(|e| e.to_string())?;
    write_lines(&dir.join("by-phab"), by_phab).map_err(|e| e.to_string())?;
    write_lines(&dir.join("by-rev"), by_rev).map_err(|e| e.to_string())?;
    // Only the new head matters from now on, since it includes the others it
    // was built on, unless we accumulate heads.
    if !options.accumulate {
        state.heads.clear();
    }
    if !state.heads.contains(&head.to_string()) {
        state.heads.push(head.to_string());
    }
    fs::write(&state_path, serde_json::to_string(&state).unwrap()).map_err(|e| e.to_string())?;
    Ok(count)
}

/// Update the index of `branch` (see `branch_dir`) under `index_root` with the
/// history of `head` (see `update`, and `UpdateOptions`), removing an index
/// from before branches had their own.
pub fn update_branch(
    repo: &Repository,
    head: Oid,
    index_root: &Path,
    branch: &str,
    options: &UpdateOptions,
) -> Result<usize, String> {
    for file in UNBRANCHED_FILES {
        let path = index_root.join(file);
        if path.is_file() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    let bases: Vec<CommitIndex> = options
        .read
        .iter()
        .map(|base| CommitIndex {
            dir: branch_dir(index_root, base),
            bases: vec![],
        })
        .collect();
    update_with(repo, head, &branch_dir(index_root, branch), options, &bases)
}

/// The branch whose index is in `dir` under a `commit-index/` (see
/// `branch_dir`).
fn dir_branch(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    Some(name.replace("%2F", "/").replace("%25", "%"))
}

/// Read access to a commit index.
pub struct CommitIndex {
    dir: PathBuf,
    /// The indexes it reads through to (see `UpdateOptions::read`).
    bases: Vec<CommitIndex>,
}

impl CommitIndex {
    pub fn open(dir: &Path) -> Option<CommitIndex> {
        dir.join("by-bug").exists().then(|| CommitIndex {
            dir: dir.to_path_buf(),
            bases: vec![],
        })
    }

    /// Open `branch`'s index under `index_root` (see `branch_dir`), with the
    /// indexes it reads through to.
    pub fn open_branch(index_root: &Path, branch: &str) -> Option<CommitIndex> {
        let mut index = Self::open(&branch_dir(index_root, branch))?;
        index.bases = read_state(&index.dir)
            .read
            .iter()
            .filter(|base| *base != branch)
            .filter_map(|base| Self::open(&branch_dir(index_root, base)))
            .collect();
        Some(index)
    }

    /// The branch of the index, if it's a branch's (see `open_branch`).
    pub fn branch(&self) -> Option<String> {
        dir_branch(&self.dir)
    }

    /// The other branches' indexes under the same `commit-index/` as this
    /// branch's (not the ones it reads through to), by branch name, for
    /// finding a bug's commits on other branches (ex: uplifts).
    pub fn other_branches(&self) -> Vec<(String, CommitIndex)> {
        let Some(root) = self.dir.parent() else {
            return vec![];
        };
        let skip: Vec<&Path> = std::iter::once(self.dir.as_path())
            .chain(self.bases.iter().map(|base| base.dir.as_path()))
            .collect();
        let Ok(entries) = fs::read_dir(root) else {
            return vec![];
        };
        let mut branches: Vec<(String, CommitIndex)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|dir| !skip.contains(&dir.as_path()))
            .filter_map(|dir| Some((dir_branch(&dir)?, Self::open(&dir)?)))
            .collect();
        branches.sort_by(|a, b| a.0.cmp(&b.0));
        branches
    }

    /// The refs for `key` in `file`, here and in the indexes this reads
    /// through to, oldest first.
    fn lookup_all(&self, file: &str, key: &str) -> Vec<CommitRef> {
        let mut refs = self.lookup(file, key);
        for base in &self.bases {
            refs.extend(base.lookup(file, key));
        }
        refs.sort_by(|a, b| a.iso_date.cmp(&b.iso_date).then(a.rev.cmp(&b.rev)));
        refs.dedup_by(|a, b| a.rev == b.rev);
        refs
    }

    fn lookup(&self, file: &str, key: &str) -> Vec<CommitRef> {
        let mut refs: Vec<CommitRef> = SortedLines::open(&self.dir.join(file))
            .find(key)
            .into_iter()
            .filter_map(|fields| match fields.as_slice() {
                [rev, iso_date, flags, ..] => Some(CommitRef {
                    rev: rev.clone(),
                    iso_date: iso_date.clone(),
                    backout: flags == "b",
                }),
                _ => None,
            })
            .collect();
        refs.sort_by(|a, b| a.iso_date.cmp(&b.iso_date).then(a.rev.cmp(&b.rev)));
        refs
    }

    /// Where each of `revs` is in the history (see `CommitPosition`), of
    /// those the index has: from the indexes this reads through to, whose
    /// commits come first, or else from this one.
    pub fn positions<'a>(
        &self,
        revs: impl IntoIterator<Item = &'a str>,
    ) -> HashMap<String, CommitPosition> {
        let own = SortedLines::open(&self.dir.join("by-rev"));
        let bases: Vec<SortedLines> = self
            .bases
            .iter()
            .map(|base| SortedLines::open(&base.dir.join("by-rev")))
            .collect();
        let mut positions = HashMap::new();
        for rev in revs {
            let found = bases
                .iter()
                .find_map(|lines| parse_position(lines.find(rev).first()?, true))
                .or_else(|| parse_position(own.find(rev).first()?, false));
            if let Some(position) = found {
                positions.insert(rev.to_string(), position);
            }
        }
        positions
    }

    /// The commits mentioning a bug, oldest first.
    pub fn bug_commits(&self, bug: &str) -> Vec<CommitRef> {
        self.lookup_all("by-bug", bug)
    }

    /// The commits of a Phabricator revision (ex: "D12345"), oldest first.
    pub fn phab_commits(&self, phab_rev: &str) -> Vec<CommitRef> {
        self.lookup_all("by-phab", phab_rev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_and_lookup() {
        let dir = std::env::temp_dir().join(format!("commit-index-test-{}", std::process::id()));
        let repo = Repository::init(dir.join("repo")).unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let mut parents: Vec<Oid> = vec![];
        let mut commit = |message: &str, seconds: i64| -> Oid {
            let sig =
                git2::Signature::new("A", "a@example.com", &git2::Time::new(seconds, 0)).unwrap();
            let parent_commits: Vec<git2::Commit> = parents
                .iter()
                .map(|p| repo.find_commit(*p).unwrap())
                .collect();
            let parent_refs: Vec<&git2::Commit> = parent_commits.iter().collect();
            let oid = repo
                .commit(None, &sig, &sig, message, &tree, &parent_refs)
                .unwrap();
            parents = vec![oid];
            oid
        };
        let a = commit("Bug 100 - Part 1: a", 1_700_000_000);
        let b = commit(
            "Bug 100 - Part 2: b\n\nDifferential Revision: https://phabricator.example.com/D5\n",
            1_700_000_100,
        );
        let c = commit(
            "Backed out changeset abc (bug 100) for failures",
            1_700_000_200,
        );

        let index_dir = dir.join("index");
        assert_eq!(update(&repo, c, &index_dir).unwrap(), 3);
        let index = CommitIndex::open(&index_dir).unwrap();
        let bug = index.bug_commits("100");
        assert_eq!(
            bug.iter().map(|r| r.rev.clone()).collect::<Vec<_>>(),
            vec![a.to_string(), b.to_string(), c.to_string()]
        );
        assert_eq!(
            bug.iter().map(|r| r.backout).collect::<Vec<_>>(),
            vec![false, false, true]
        );
        assert_eq!(bug[0].iso_date, "2023-11-14T22:13:20Z");
        assert_eq!(index.phab_commits("D5").len(), 1);
        assert!(index.bug_commits("1000").is_empty());

        // Only new commits are processed.
        assert_eq!(update(&repo, c, &index_dir).unwrap(), 0);
        let d = commit("Bug 1000 - Something else", 1_700_000_300);
        assert_eq!(update(&repo, d, &index_dir).unwrap(), 1);
        assert_eq!(index.bug_commits("1000").len(), 1);
        assert_eq!(index.bug_commits("100").len(), 3);

        // Rewriting the last commit (ex: amending it) replaces it.
        let sig =
            git2::Signature::new("A", "a@example.com", &git2::Time::new(1_700_000_400, 0)).unwrap();
        let amended = repo
            .commit(
                None,
                &sig,
                &sig,
                "Bug 1001 - Something else, amended",
                &tree,
                &[&repo.find_commit(c).unwrap()],
            )
            .unwrap();
        assert_eq!(update(&repo, amended, &index_dir).unwrap(), 4);
        assert!(index.bug_commits("1000").is_empty());
        assert_eq!(index.bug_commits("1001").len(), 1);
        assert_eq!(index.bug_commits("100").len(), 3);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_branches() {
        let dir =
            std::env::temp_dir().join(format!("commit-index-branches-{}", std::process::id()));
        let repo = Repository::init(dir.join("repo")).unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let commit = |message: &str, parent: Option<Oid>, seconds: i64| -> Oid {
            let sig =
                git2::Signature::new("A", "a@example.com", &git2::Time::new(seconds, 0)).unwrap();
            let parents: Vec<git2::Commit> = parent
                .iter()
                .map(|p| repo.find_commit(*p).unwrap())
                .collect();
            let parents: Vec<&git2::Commit> = parents.iter().collect();
            repo.commit(None, &sig, &sig, message, &tree, &parents)
                .unwrap()
        };
        let base = commit("Bug 101 - Landed before the cut", None, 1_700_000_000);
        let main = commit("Bug 102 - Landed after the cut", Some(base), 1_700_000_100);
        let beta = commit("Bug 200 - Uplift", Some(base), 1_700_000_200);

        let root = dir.join("commit-index");
        let plain = UpdateOptions::default();
        // (An index from before branches had their own goes.)
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("by-bug"), "999\tabc\t2020-01-01T00:00:00Z\t-\n").unwrap();
        assert_eq!(
            update_branch(&repo, main, &root, "main", &plain).unwrap(),
            2
        );
        assert_eq!(
            update_branch(&repo, beta, &root, ref_branch("refs/heads/beta"), &plain).unwrap(),
            2
        );
        assert!(!root.join("by-bug").exists());
        // Updating one branch doesn't disturb the other, or redo its commits.
        assert_eq!(
            update_branch(&repo, main, &root, "main", &plain).unwrap(),
            0
        );
        let bugs = |branch: &str| -> Vec<String> {
            let index = CommitIndex::open_branch(&root, branch).unwrap();
            ["101", "102", "200"]
                .into_iter()
                .filter(|bug| !index.bug_commits(bug).is_empty())
                .map(str::to_string)
                .collect()
        };
        assert_eq!(bugs("main"), vec!["101", "102"]);
        assert_eq!(bugs("beta"), vec!["101", "200"]);
        assert!(CommitIndex::open_branch(&root, "release").is_none());
        assert_eq!(branch_dir(&root, "a/b%c"), root.join("a%2Fb%25c"));

        // A try index accumulates unrelated pushes based on main, without
        // main's commits, which it reads through to.
        let try_options = UpdateOptions {
            accumulate: true,
            read: vec!["main".to_string()],
        };
        let push1 = commit(
            "Bug 300 - Try this\n\nDifferential Revision: https://phab/D30",
            Some(main),
            1_700_000_300,
        );
        let push2 = commit("Bug 400 - Try that", Some(base), 1_700_000_400);
        assert_eq!(
            update_branch(&repo, push1, &root, "try", &try_options).unwrap(),
            1
        );
        assert_eq!(
            update_branch(&repo, push2, &root, "try", &try_options).unwrap(),
            1
        );
        // (Again, nothing.)
        assert_eq!(
            update_branch(&repo, push1, &root, "try", &try_options).unwrap(),
            0
        );
        let try_index = CommitIndex::open_branch(&root, "try").unwrap();
        assert_eq!(try_index.bug_commits("300")[0].rev, push1.to_string());
        assert_eq!(try_index.bug_commits("400")[0].rev, push2.to_string());
        assert_eq!(try_index.phab_commits("D30").len(), 1);
        assert_eq!(try_index.bug_commits("102")[0].rev, main.to_string());
        assert_eq!(bugs("try"), vec!["101", "102"]);
        // Its other branches (not main, which it reads through to).
        let others: Vec<String> = try_index
            .other_branches()
            .into_iter()
            .map(|(b, _)| b)
            .collect();
        assert_eq!(others, vec!["beta"]);
        let main_others: Vec<String> = CommitIndex::open_branch(&root, "main")
            .unwrap()
            .other_branches()
            .into_iter()
            .map(|(b, _)| b)
            .collect();
        assert_eq!(main_others, vec!["beta", "try"]);
        assert_eq!(try_index.branch().as_deref(), Some("try"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_positions() {
        let dir =
            std::env::temp_dir().join(format!("commit-index-positions-{}", std::process::id()));
        let repo = Repository::init(dir.join("repo")).unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let commit = |message: &str, parents: &[Oid], seconds: i64| -> Oid {
            let sig =
                git2::Signature::new("A", "a@example.com", &git2::Time::new(seconds, 0)).unwrap();
            let parents: Vec<git2::Commit> = parents
                .iter()
                .map(|p| repo.find_commit(*p).unwrap())
                .collect();
            let parents: Vec<&git2::Commit> = parents.iter().collect();
            repo.commit(None, &sig, &sig, message, &tree, &parents)
                .unwrap()
        };
        // (A patch dated before it landed, as hg's were, backed out and
        // relanded with its date; and a branch merged in.)
        let a = commit("Bug 1 - a", &[], 1_000);
        let b = commit("Bug 2 - b", &[a], 2_000);
        let land = commit("Bug 3 - land", &[b], 500);
        let out = commit("Backed out changeset 0123456789ab (bug 3)", &[land], 3_000);
        let side = commit("Bug 4 - side", &[b], 2_500);
        let reland = commit("Bug 3 - land", &[out], 500);
        let merge = commit("Merge", &[reland, side], 3_100);

        let root = dir.join("commit-index");
        let plain = UpdateOptions::default();
        update_branch(&repo, reland, &root, "main", &plain).unwrap();
        let index = CommitIndex::open_branch(&root, "main").unwrap();
        let revs = [a, b, land, out, reland].map(|oid| oid.to_string());
        let positions = index.positions(revs.iter().map(String::as_str));
        let landed: Vec<&str> = revs
            .iter()
            .map(|rev| positions[rev].landed.as_str())
            .collect();
        assert_eq!(
            landed,
            vec![
                "1970-01-01T00:16:40Z",
                "1970-01-01T00:33:20Z",
                "1970-01-01T00:33:20Z",
                "1970-01-01T00:50:00Z",
                "1970-01-01T00:50:00Z"
            ]
        );
        let mut by_order = revs.to_vec();
        by_order.reverse();
        by_order.sort_by(|x, y| positions[x].order_key().cmp(&positions[y].order_key()));
        assert_eq!(by_order, revs.to_vec());

        // (An update numbers its commits after the processed ones, with
        // landing dates from their processed parents.)
        update_branch(&repo, merge, &root, "main", &plain).unwrap();
        let positions = index.positions(
            [side, merge, reland]
                .map(|oid| oid.to_string())
                .iter()
                .map(String::as_str),
        );
        assert!(positions[&merge.to_string()].position > positions[&reland.to_string()].position);
        assert_eq!(positions[&merge.to_string()].landed, "1970-01-01T00:51:40Z");
        assert_eq!(positions[&side.to_string()].landed, "1970-01-01T00:41:40Z");

        // (A try index's commits come after its base's, which it reads
        // through to, and land after their parents there.)
        let try_options = UpdateOptions {
            accumulate: true,
            read: vec!["main".to_string()],
        };
        let push = commit("Bug 5 - try", &[merge], 100);
        update_branch(&repo, push, &root, "try", &try_options).unwrap();
        let try_index = CommitIndex::open_branch(&root, "try").unwrap();
        let positions = try_index.positions(
            [push, merge]
                .map(|oid| oid.to_string())
                .iter()
                .map(String::as_str),
        );
        let (push_at, merge_at) = (
            &positions[&push.to_string()],
            &positions[&merge.to_string()],
        );
        assert!(!push_at.base && merge_at.base);
        assert_eq!(push_at.landed, merge_at.landed);
        assert!(push_at.order_key() > merge_at.order_key());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_message_parsing() {
        assert_eq!(
            message_bugs("Bug 1517978 - Part 1: Stuff."),
            vec!["1517978"]
        );
        assert_eq!(
            message_bugs("Backed out 2 changesets (bug 123456, bug 654321) for failures"),
            vec!["123456", "654321"]
        );
        assert_eq!(
            message_bugs("No bug: tidy things\n\nFixes bug 999999."),
            Vec::<String>::new()
        );
        assert!(is_backout("Backed out changeset abc (bug 123456)"));
        assert!(is_backout("Revert \"Bug 1 - Thing\""));
        assert!(!is_backout("Bug 123456 - Back out the old approach"));
        assert_eq!(
            message_phab_revs(
                "Bug 1 - Thing\n\nDifferential Revision: https://phabricator.services.mozilla.com/D12345\n"
            ),
            vec!["D12345"]
        );
    }
}
