//! Recognizing backouts: commits which undo ("back out" or "revert") commits
//! that landed shortly before them, usually because those commits broke the
//! tree.  A backout and the commits it backs out aren't interesting history,
//! so `build-timeline-tree` restores the pre-landing blame for the tokens a
//! backout restores and marks the backout's journal records with `backs_out`.
//!
//! Backouts are recognized from their commit messages:
//! - git reverts (`git revert`, used by Lando and GitHub), one line per
//!   reverted commit: `This reverts commit <40 hex digit git revision>.`
//! - Mercurial-era backouts (`hg backout`/`hg oops`, see
//!   https://wiki.mozilla.org/Sheriffing/How_To/Backouts), one line per
//!   backed out changeset, identified by an (usually 12 hex digit) hg
//!   revision: `Backed out changeset 1a2b3c4d5e6f (bug 1234567) for ...`, or
//!   variants like `Back out`, `Backout of`, `Backed out changesets X, Y`,
//!   `Back out rev 12345:X`, `Revert X`, ranges like `Back out X to Y` (or
//!   hg's `X:Y`), and lines starting with `Bug N - `.  These are resolved by
//!   asking git-cinnabar for the source repo's git revision, which it can do
//!   for abbreviated hg revisions.
//!
//! Merges don't need examining: in firefox-main, each of the 80 merges with
//! backout lines merges in the linear backout commit (hg's backout then merge
//! workflow), which carries the same lines.  Backouts which only name bugs
//! (ex: `Back out bug 1234567`, and all of the CVS era's) aren't recognized.
//!
//! Git revisions are then mapped to syntax commits with the syntax repo's
//! source mapping notes (see `source_mapping`).
//!
//! Only commits within `BACKOUT_HORIZON_SECS` before the backout count.  Older
//! reverts are usually deliberate decisions to remove something rather than
//! backouts, and are left as normal history.  In 2026 firefox-main history,
//! 378 of 385 reverts were of commits which had landed less than 7 days
//! before.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use git2::{Commit, Oid, Repository};
use lazy_static::lazy_static;
use regex::Regex;

use crate::cinnabar::CinnabarBatch;
use crate::source_mapping::SourceMapping;

/// How long after landing a revert of a commit is considered a backout.
pub const BACKOUT_HORIZON_SECS: i64 = 14 * 24 * 60 * 60;

/// The most commits a backed out range (ex: "Back out X to Y") can have; more
/// is more likely a misreading than a backout.
const MAX_RANGE_COMMITS: usize = 100;

/// A reference to a backed out commit from a backout's commit message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BackoutTargetRef {
    /// A full, lowercase git revision.
    Git(String),
    /// A lowercase hg revision or prefix of one (at least 12 hex digits).
    Hg(String),
    /// An inclusive range of hg revisions like `Hg`'s (ex: "Back out X to Y"),
    /// in either order.
    HgRange(String, String),
}

lazy_static! {
    static ref GIT_REVERT: Regex = Regex::new(r"(?m)^\s*This reverts commit ([0-9a-fA-F]{40})\b")
        .unwrap();
    /// A backout (or revert) line and the list of revisions after "Backed out
    /// changeset(s)", optionally after a bug number (ex: "Bug 1234567 -
    /// Backed out changeset X").  Revisions can have hg's local revision
    /// numbers (ex: "12345:1a2b3c4d5e6f").
    static ref HG_BACKOUT: Regex = Regex::new(
        r"(?im)^[ \t]*(?:(?:bug|b=)\s*\d+\s*[-:.,]*\s*)?(?:back(?:ed|ing)?[ -]?out(?:\s+of)?|(?:this\s+)?revert(?:s|ed|ing)?)\s+(?:(?:changesets?|csets?|revs?|revisions?)\s*:?\s*)?((?:(?:\d+:)?[0-9a-f]{12,40}(?:\s*(?:,|&|\+|and)\s*)?)+)\b"
    )
    .unwrap();
    /// What follows the first revision of a range (ex: "X to Y"), with the
    /// last revision.
    static ref HG_RANGE: Regex = Regex::new(
        r"(?i)^\s*(?:to|through|thru|-|–|\.\.\.?|:)\s*(?:(?:changesets?|csets?|revs?|revisions?)\s+)?(?:\d+:)?([0-9a-f]{12,40})\b"
    )
    .unwrap();
    static ref HEX_REV: Regex = Regex::new(r"[0-9a-fA-F]{12,40}").unwrap();
}

/// Parse the commits a commit message says it backs out, in message order.
pub fn parse_backout_targets(message: &str) -> Vec<BackoutTargetRef> {
    let mut targets = vec![];
    for caps in GIT_REVERT.captures_iter(message) {
        targets.push(BackoutTargetRef::Git(caps[1].to_lowercase()));
    }
    for caps in HG_BACKOUT.captures_iter(message) {
        let mut revs: Vec<String> = HEX_REV
            .find_iter(&caps[1])
            .map(|rev| rev.as_str().to_lowercase())
            .collect();
        let range = HG_RANGE.captures(&message[caps.get(0).unwrap().end()..]);
        // (The last revision of the list starts the range.)
        let range_start = range.as_ref().and_then(|_| revs.pop());
        targets.extend(revs.into_iter().map(BackoutTargetRef::Hg));
        if let (Some(range), Some(start)) = (range, range_start) {
            targets.push(BackoutTargetRef::HgRange(start, range[1].to_lowercase()));
        }
    }
    let mut seen = HashSet::new();
    targets.retain(|t| seen.insert(t.clone()));
    targets
}

/// Resolves the revisions backout messages reference to syntax commits.  This
/// is shared by the compute threads.
pub struct BackoutTargetResolver {
    syntax_mapping: SourceMapping,
    source_repo_path: PathBuf,
    /// A `git cinnabar hg2git` process for the source repo, started when first
    /// needed, or None if we aren't using git-cinnabar.  Backouts are rare
    /// enough that there's no point in having one per thread.
    hg2git: Option<Mutex<Option<CinnabarBatch>>>,
}

impl BackoutTargetResolver {
    pub fn new(
        syntax_mapping: SourceMapping,
        source_repo_path: &Path,
        use_cinnabar: bool,
    ) -> BackoutTargetResolver {
        BackoutTargetResolver {
            syntax_mapping,
            source_repo_path: source_repo_path.to_path_buf(),
            hg2git: use_cinnabar.then(|| Mutex::new(None)),
        }
    }

    /// The source repo revision a target refers to, if known.
    fn source_rev(&self, target: &BackoutTargetRef) -> Option<Oid> {
        match target {
            BackoutTargetRef::Git(rev) => Oid::from_str(rev).ok(),
            BackoutTargetRef::Hg(rev) => {
                let from_hg = self.hg2git.as_ref().and_then(|hg2git| {
                    let mut hg2git = hg2git.lock().unwrap();
                    hg2git
                        .get_or_insert_with(|| {
                            CinnabarBatch::hg2git(
                                &Repository::open(&self.source_repo_path).unwrap(),
                            )
                        })
                        .lookup(rev)
                });
                match from_hg {
                    Some(git_rev) => Oid::from_str(&git_rev).ok(),
                    // Some "Backed out changeset" messages may use git
                    // revisions.
                    None if rev.len() == 40 => Oid::from_str(rev).ok(),
                    None => None,
                }
            }
            BackoutTargetRef::HgRange(..) => None,
        }
    }

    /// The syntax commit a target refers to, if its revision has been
    /// processed (None for ranges; see `resolve_all`).
    pub fn resolve(&self, syntax_repo: &Repository, target: &BackoutTargetRef) -> Option<Oid> {
        self.syntax_mapping
            .lookup(syntax_repo, self.source_rev(target)?)
    }

    /// The syntax commits a target refers to: for a range, the later end and
    /// its ancestors which descend from the earlier end, or none if the ends
    /// aren't both known, neither descends from the other, or there are more
    /// than `MAX_RANGE_COMMITS`.
    pub fn resolve_all(&self, syntax_repo: &Repository, target: &BackoutTargetRef) -> Vec<Oid> {
        let BackoutTargetRef::HgRange(a, b) = target else {
            return self.resolve(syntax_repo, target).into_iter().collect();
        };
        let resolve_hg =
            |rev: &str| self.resolve(syntax_repo, &BackoutTargetRef::Hg(rev.to_string()));
        let (Some(a), Some(b)) = (resolve_hg(a), resolve_hg(b)) else {
            return vec![];
        };
        let descends =
            |x: Oid, y: Oid| x == y || syntax_repo.graph_descendant_of(x, y).unwrap_or(false);
        let (first, last) = if descends(b, a) {
            (a, b)
        } else if descends(a, b) {
            (b, a)
        } else {
            return vec![];
        };
        let Ok(mut walk) = syntax_repo.revwalk() else {
            return vec![];
        };
        if walk.push(last).is_err() {
            return vec![];
        }
        if let Ok(first_commit) = syntax_repo.find_commit(first) {
            for parent in first_commit.parent_ids() {
                let _ = walk.hide(parent);
            }
        }
        // (Not branches merged in between, which don't descend from the first.)
        let commits: Vec<Oid> = walk
            .filter_map(Result::ok)
            .filter(|&oid| descends(oid, first))
            .take(MAX_RANGE_COMMITS + 1)
            .collect();
        if commits.len() > MAX_RANGE_COMMITS {
            return vec![];
        }
        commits
    }
}

/// Find the syntax commits that `backout` (a syntax commit whose source commit
/// has the given message) backs out: ancestors referenced by the message which
/// landed within `BACKOUT_HORIZON_SECS` before it.  They're ordered earliest
/// first by ancestry because commits landed together (ex: a stack of patches
/// in one push) can have the same timestamp.
pub fn find_backed_out(
    syntax_repo: &Repository,
    resolver: &BackoutTargetResolver,
    backout: &Commit,
    message: &str,
) -> Vec<Oid> {
    let backout_time = backout.committer().when().seconds();
    let mut found: Vec<(Oid, i64)> = vec![];
    let targets = parse_backout_targets(message);
    for oid in targets
        .iter()
        .flat_map(|target| resolver.resolve_all(syntax_repo, target))
    {
        if oid == backout.id() || found.iter().any(|(o, _)| *o == oid) {
            continue;
        }
        let Ok(commit) = syntax_repo.find_commit(oid) else {
            continue;
        };
        let time = commit.committer().when().seconds();
        if backout_time - time > BACKOUT_HORIZON_SECS
            || !syntax_repo
                .graph_descendant_of(backout.id(), oid)
                .unwrap_or(false)
        {
            continue;
        }
        found.push((oid, time));
    }

    // Order by how many of the other targets descend from each target (a total
    // order even if the targets aren't all on one line of history), then time.
    let num_descendants: Vec<usize> = found
        .iter()
        .map(|(a, _)| {
            found
                .iter()
                .filter(|(b, _)| a != b && syntax_repo.graph_descendant_of(*b, *a).unwrap_or(false))
                .count()
        })
        .collect();
    let mut order: Vec<usize> = (0..found.len()).collect();
    order.sort_by_key(|&i| {
        (
            std::cmp::Reverse(num_descendants[i]),
            found[i].1,
            found[i].0,
        )
    });
    order.into_iter().map(|i| found[i].0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_notes::test_support::write_notes;
    use crate::source_mapping::{NotesRefs, default_notes_ref};
    use BackoutTargetRef::*;

    #[test]
    fn test_parse_git_reverts() {
        assert_eq!(
            parse_backout_targets(
                "Revert \"Bug 2053925 - Part 2: Render NL browser actions\" for causing bc failures\n\
                 \n\
                 This reverts commit 5b7cc1a12a3fb035d9ecab4d4b36ec5386789a7d.\n\
                 \n\
                 Revert \"Bug 2053925 - Part 1: Add the AI action confirmation component\"\n\
                 \n\
                 This reverts commit EE47CD8A7889F2DDD50A8E10D206D9F60C4BBB5C.\n"
            ),
            vec![
                Git("5b7cc1a12a3fb035d9ecab4d4b36ec5386789a7d".to_string()),
                Git("ee47cd8a7889f2ddd50a8e10d206d9f60c4bbb5c".to_string()),
            ]
        );
        // Mentioning a revert in prose doesn't count.
        assert_eq!(
            parse_backout_targets(
                "Bug 1 - Fix things.\n\nNote that this reverts commit \
                 5b7cc1a12a3fb035d9ecab4d4b36ec5386789a7d's approach."
            ),
            vec![]
        );
    }

    #[test]
    fn test_parse_hg_backouts() {
        assert_eq!(
            parse_backout_targets(
                "Backed out 2 changesets (bug 1957792, bug 1957787) as requested by dmeehan. \
                 a=backout\n\
                 \n\
                 Backed out changeset f16bd0803a60 (bug 1957792)\n\
                 Backed out changeset d6743fb3dd89 (bug 1957787)\n"
            ),
            vec![
                Hg("f16bd0803a60".to_string()),
                Hg("d6743fb3dd89".to_string())
            ]
        );
        for (message, revs) in [
            (
                "Backed out changeset 1a2b3c4d5e6f (bug 123) for causing failures in \
                 test_0123456789abcdef.html",
                vec!["1a2b3c4d5e6f"],
            ),
            (
                "Backout 1a2b3c4d5e6f (bug 123) for bustage",
                vec!["1a2b3c4d5e6f"],
            ),
            ("back out changeset 1A2B3C4D5E6F", vec!["1a2b3c4d5e6f"]),
            ("Backing out 1a2b3c4d5e6f", vec!["1a2b3c4d5e6f"]),
            (
                "Backed out changesets 1a2b3c4d5e6f, 2b3c4d5e6f7a and 3c4d5e6f7a8b (bug 123)",
                vec!["1a2b3c4d5e6f", "2b3c4d5e6f7a", "3c4d5e6f7a8b"],
            ),
            ("Backed out 3 changesets (bug 123) for bustage", vec![]),
            ("Bug 123 - Don't back out things.", vec![]),
            (
                "Bug 123 - Backed out changeset 1a2b3c4d5e6f for failures",
                vec!["1a2b3c4d5e6f"],
            ),
            (
                "Bug 123: backout 1a2b3c4d5e6f, 2b3c4d5e6f7a",
                vec!["1a2b3c4d5e6f", "2b3c4d5e6f7a"],
            ),
            (
                "b=123 revert 1a2b3c4d5e6f and 2b3c4d5e6f7a",
                vec!["1a2b3c4d5e6f", "2b3c4d5e6f7a"],
            ),
            ("Backout of changeset 1a2b3c4d5e6f", vec!["1a2b3c4d5e6f"]),
            ("Backed out changeset: 1a2b3c4d5e6f", vec!["1a2b3c4d5e6f"]),
            ("Back out rev 1a2b3c4d5e6f (bug 123)", vec!["1a2b3c4d5e6f"]),
            (
                "Backout 87134:1a2b3c4d5e6f and 87135:2b3c4d5e6f7a",
                vec!["1a2b3c4d5e6f", "2b3c4d5e6f7a"],
            ),
            ("Revert 1a2b3c4d5e6f for M2 failures", vec!["1a2b3c4d5e6f"]),
            (
                "This reverts 1a2b3c4d5e6f (bug 123), which didn't link.",
                vec!["1a2b3c4d5e6f"],
            ),
            // A revert to a revision isn't a backout of it, and bug numbers
            // aren't revisions.
            ("Revert to 1a2b3c4d5e6f which was fine.", vec![]),
            ("Back out bug 123 (1a2b3c4d5e6f - 2b3c4d5e6f7a)", vec![]),
            ("Back out bug 1234567 for bustage", vec![]),
        ] {
            let expected: Vec<BackoutTargetRef> =
                revs.into_iter().map(|r| Hg(r.to_string())).collect();
            assert_eq!(parse_backout_targets(message), expected, "{}", message);
        }
        let range = |a: &str, b: &str| HgRange(a.to_string(), b.to_string());
        for (message, expected) in [
            (
                "Backout rev 1a2b3c4d5e6f to 2b3c4d5e6f7a (bug 123)",
                vec![range("1a2b3c4d5e6f", "2b3c4d5e6f7a")],
            ),
            (
                "Back out changesets 1a2b3c4d5e6f through 456:2b3c4d5e6f7a",
                vec![range("1a2b3c4d5e6f", "2b3c4d5e6f7a")],
            ),
            (
                "back out 1a2b3c4d5e6f..2b3c4d5e6f7a for test failures",
                vec![range("1a2b3c4d5e6f", "2b3c4d5e6f7a")],
            ),
            // (hg's own range notation.)
            (
                "Back out 1a2b3c4d5e6f:2b3c4d5e6f7a (bug 123) for b2g bustage",
                vec![range("1a2b3c4d5e6f", "2b3c4d5e6f7a")],
            ),
            (
                "Back out 0a2b3c4d5e6f, 1a2b3c4d5e6f - 2b3c4d5e6f7a (bug 1)",
                vec![
                    Hg("0a2b3c4d5e6f".to_string()),
                    range("1a2b3c4d5e6f", "2b3c4d5e6f7a"),
                ],
            ),
        ] {
            assert_eq!(parse_backout_targets(message), expected, "{}", message);
        }
    }

    #[test]
    fn test_resolve() {
        let dir = std::env::temp_dir().join(format!("hb-backout-resolve-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let source = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let syntax = |n: u8| Oid::from_bytes(&[n + 100; 20]).unwrap();
        let refs = NotesRefs {
            write: default_notes_ref(&repo, "refs/heads/main"),
            read: vec![],
        };
        write_notes(&repo, &refs.write, &[(source(1), syntax(1))]);
        // Without git-cinnabar, only git revisions resolve, including 40 digit
        // "hg" revisions.
        let resolver = BackoutTargetResolver::new(SourceMapping::open(&repo, &refs), &dir, false);
        assert_eq!(
            resolver.resolve(&repo, &Git(source(1).to_string())),
            Some(syntax(1))
        );
        assert_eq!(
            resolver.resolve(&repo, &Hg(source(1).to_string())),
            Some(syntax(1))
        );
        assert_eq!(
            resolver.resolve(&repo, &Hg("010101010101".to_string())),
            None
        );
        assert_eq!(resolver.resolve(&repo, &Git(source(2).to_string())), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_range() {
        let dir = std::env::temp_dir().join(format!("hb-backout-range-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let sig = git2::Signature::now("a", "a@example.com").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let commit = |message: &str, parents: &[Oid]| {
            let parents: Vec<Commit> = parents
                .iter()
                .map(|p| repo.find_commit(*p).unwrap())
                .collect();
            let parents: Vec<&Commit> = parents.iter().collect();
            repo.commit(None, &sig, &sig, message, &tree, &parents)
                .unwrap()
        };
        // a <- b <- c <- merge <- d, with side (from a) merged in between.
        let a = commit("a", &[]);
        let b = commit("b", &[a]);
        let c = commit("c", &[b]);
        let side = commit("side", &[a]);
        let merge = commit("merge", &[c, side]);
        let d = commit("d", &[merge]);
        let unrelated = commit("unrelated", &[]);
        // Without git-cinnabar, 40 digit "hg" revisions are git revisions,
        // which the notes map to syntax commits (here, themselves).
        let refs = NotesRefs {
            write: default_notes_ref(&repo, "refs/heads/main"),
            read: vec![],
        };
        let commits = [a, b, c, side, merge, d, unrelated];
        let notes: Vec<(Oid, Oid)> = commits.iter().map(|&c| (c, c)).collect();
        write_notes(&repo, &refs.write, &notes);
        let resolver = BackoutTargetResolver::new(SourceMapping::open(&repo, &refs), &dir, false);
        let resolve = |x: Oid, y: Oid| {
            let mut oids = resolver.resolve_all(&repo, &HgRange(x.to_string(), y.to_string()));
            oids.sort();
            oids
        };
        let mut expected = vec![b, c, merge, d];
        expected.sort();
        assert_eq!(resolve(b, d), expected);
        // Either order.
        assert_eq!(resolve(d, b), expected);
        assert_eq!(resolve(c, c), vec![c]);
        assert_eq!(resolve(b, unrelated), vec![]);
        assert_eq!(resolver.resolve_all(&repo, &Hg(c.to_string())), vec![c]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
