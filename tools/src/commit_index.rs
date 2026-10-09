//! The commit index: which commits mention a bug or a Phabricator revision,
//! for the `/explore/` endpoints (see "Commit index" in the hyperblame notes).
//!
//! It's derived from the commit messages by `build-commit-index` in the setup
//! phase, and kept beside the history (in `commit-index/` under the tree's
//! `history_path`) so that updates only need to process new commits:
//! - `state.json` has the schema version and the heads processed so far.  A
//!   different schema version means rebuilding from scratch, which is cheap
//!   (a revwalk and parsing the messages), as does a processed head which isn't
//!   an ancestor of the new head (ex: a rebased branch in development), whose
//!   commits may no longer be in the history.
//! - `by-bug` has `BUG<TAB>REV<TAB>ISO_DATE<TAB>FLAGS` lines and `by-phab` has
//!   `DNNN<TAB>REV<TAB>ISO_DATE<TAB>FLAGS` lines, sorted, so lookups can
//!   bisect.  FLAGS is "b" for backouts (by their summary lines) or "-".
//!
//! This doesn't come from the history processing (which has the messages in
//! its rev-summaries), because this covers more: all of the repo's commits,
//! not just the history's (ex: a window's), including try and review heads,
//! for 14s of processing for firefox-main from scratch.  What the history
//! knows besides, which commits backed out which, is in the rev-summaries
//! (`backs_out` and `backed_out_by`), which the explore pages read (see
//! `hyperblame::explore::mark_history_backouts`).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use git2::{Oid, Repository, Sort};
use lazy_static::lazy_static;
use memmap::Mmap;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Bump this when the files' contents change so existing indexes get rebuilt.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    schema: u32,
    /// The heads whose history has been processed.
    heads: Vec<String>,
}

/// A commit which mentions a bug or Phabricator revision.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CommitRef {
    pub rev: String,
    #[serde(rename = "isoDate")]
    pub iso_date: String,
    /// Whether the commit is a backout.
    pub backout: bool,
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
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let state_path = dir.join("state.json");
    let mut state: State = fs::read_to_string(&state_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let rewritten = state
        .heads
        .iter()
        .any(|processed| match Oid::from_str(processed) {
            Ok(oid) => oid != head && !repo.graph_descendant_of(head, oid).unwrap_or(false),
            Err(_) => true,
        });
    let rebuild = state.schema != SCHEMA_VERSION || rewritten;
    let (mut by_bug, mut by_phab) = if rebuild {
        state = State {
            schema: SCHEMA_VERSION,
            heads: vec![],
        };
        (vec![], vec![])
    } else {
        (
            read_lines(&dir.join("by-bug")),
            read_lines(&dir.join("by-phab")),
        )
    };

    let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
    walk.set_sorting(Sort::NONE).map_err(|e| e.to_string())?;
    walk.push(head).map_err(|e| e.to_string())?;
    for processed in &state.heads {
        let oid = Oid::from_str(processed).map_err(|e| e.to_string())?;
        walk.hide(oid).map_err(|e| e.to_string())?;
    }
    let mut count = 0;
    for oid in walk {
        let oid = oid.map_err(|e| e.to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
        let message = String::from_utf8_lossy(commit.message_bytes());
        let date = chrono::DateTime::from_timestamp(commit.time().seconds(), 0)
            .map(|date| date.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_default();
        let flags = if is_backout(&message) { "b" } else { "-" };
        for bug in message_bugs(&message) {
            by_bug.push(format!("{}\t{}\t{}\t{}", bug, oid, date, flags));
        }
        for phab in message_phab_revs(&message) {
            by_phab.push(format!("{}\t{}\t{}\t{}", phab, oid, date, flags));
        }
        count += 1;
    }

    write_lines(&dir.join("by-bug"), by_bug).map_err(|e| e.to_string())?;
    write_lines(&dir.join("by-phab"), by_phab).map_err(|e| e.to_string())?;
    // Only the new head matters from now on, since it includes the others it
    // was built on.
    state.heads = vec![head.to_string()];
    fs::write(&state_path, serde_json::to_string(&state).unwrap()).map_err(|e| e.to_string())?;
    Ok(count)
}

/// Read access to a commit index.
pub struct CommitIndex {
    dir: PathBuf,
}

impl CommitIndex {
    pub fn open(dir: &Path) -> Option<CommitIndex> {
        dir.join("by-bug").exists().then(|| CommitIndex {
            dir: dir.to_path_buf(),
        })
    }

    fn lookup(&self, file: &str, key: &str) -> Vec<CommitRef> {
        let Ok(file) = fs::File::open(self.dir.join(file)) else {
            return vec![];
        };
        let Ok(mmap) = (unsafe { Mmap::map(&file) }) else {
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
        let mut refs = vec![];
        let mut start = low;
        while start < data.len() && data[start..].starts_with(prefix) {
            let end = line_end(start);
            let line = String::from_utf8_lossy(&data[start..end]);
            let mut fields = line.split('\t').skip(1);
            if let (Some(rev), Some(iso_date), Some(flags)) =
                (fields.next(), fields.next(), fields.next())
            {
                refs.push(CommitRef {
                    rev: rev.to_string(),
                    iso_date: iso_date.to_string(),
                    backout: flags == "b",
                });
            }
            start = end + 1;
        }
        refs.sort_by(|a, b| a.iso_date.cmp(&b.iso_date).then(a.rev.cmp(&b.rev)));
        refs
    }

    /// The commits mentioning a bug, oldest first.
    pub fn bug_commits(&self, bug: &str) -> Vec<CommitRef> {
        self.lookup("by-bug", bug)
    }

    /// The commits of a Phabricator revision (ex: "D12345"), oldest first.
    pub fn phab_commits(&self, phab_rev: &str) -> Vec<CommitRef> {
        self.lookup("by-phab", phab_rev)
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
