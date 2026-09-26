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
//!   variants like `Back out`, `Backout`, and `Backed out changesets X, Y`.
//!   These are resolved via the "hg" lines git-cinnabar lets
//!   `build-syntax-token-tree` put in the syntax commits.
//!
//! Only commits within `BACKOUT_HORIZON_SECS` before the backout count.  Older
//! reverts are usually deliberate decisions to remove something rather than
//! backouts, and are left as normal history.  In 2026 firefox-main history,
//! 378 of 385 reverts were of commits which had landed less than 7 days
//! before.

use std::collections::{HashMap, HashSet};

use git2::{Commit, Oid, Repository};
use lazy_static::lazy_static;
use regex::Regex;

use crate::file_format::config::syntax_commit_to_meta;

/// How long after landing a revert of a commit is considered a backout.
pub const BACKOUT_HORIZON_SECS: i64 = 14 * 24 * 60 * 60;

/// A reference to a backed out commit from a backout's commit message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BackoutTargetRef {
    /// A full, lowercase git revision.
    Git(String),
    /// A lowercase hg revision or prefix of one (at least 12 hex digits).
    Hg(String),
}

lazy_static! {
    static ref GIT_REVERT: Regex = Regex::new(r"(?m)^\s*This reverts commit ([0-9a-fA-F]{40})\b")
        .unwrap();
    /// A backout line and the list of revisions after "Backed out changeset(s)".
    static ref HG_BACKOUT: Regex = Regex::new(
        r"(?im)^\s*back(?:ed|ing)?[ -]?out\s+(?:(?:changesets?|csets?)\s+)?((?:[0-9a-f]{12,40}(?:\s*(?:,|&|\+|and)\s*)?)+)\b"
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
        for rev in HEX_REV.find_iter(&caps[1]) {
            targets.push(BackoutTargetRef::Hg(rev.as_str().to_lowercase()));
        }
    }
    let mut seen = HashSet::new();
    targets.retain(|t| seen.insert(t.clone()));
    targets
}

/// The number of hex digits of hg revisions we index by, which is the length
/// of the abbreviated revisions used in backout messages.
const HG_PREFIX_LEN: usize = 12;

/// Maps the revisions backout messages can reference to syntax commits.
#[derive(Default)]
pub struct BackoutTargetIndex {
    source_to_syntax: HashMap<Oid, Oid>,
    /// Maps the first `HG_PREFIX_LEN` hex digits of hg revisions to the full
    /// hg revisions and their syntax commits.
    hg_by_prefix: HashMap<String, Vec<(String, Oid)>>,
}

impl BackoutTargetIndex {
    /// Index all the commits in the syntax repo reachable from `head`.
    pub fn build(syntax_repo: &Repository, head: Oid) -> Self {
        let mut index = BackoutTargetIndex::default();
        let mut walk = syntax_repo.revwalk().unwrap();
        walk.push(head).unwrap();
        for oid in walk {
            let commit = syntax_repo.find_commit(oid.unwrap()).unwrap();
            let meta = syntax_commit_to_meta(&commit);
            index.add(
                meta.source_rev,
                meta.syntax_rev,
                meta.source_hg_rev.as_deref(),
            );
        }
        index
    }

    pub fn add(&mut self, source_rev: Oid, syntax_rev: Oid, hg_rev: Option<&str>) {
        self.source_to_syntax.insert(source_rev, syntax_rev);
        if let Some(hg_rev) = hg_rev
            && let Some(prefix) = hg_rev.get(..HG_PREFIX_LEN)
        {
            self.hg_by_prefix
                .entry(prefix.to_lowercase())
                .or_default()
                .push((hg_rev.to_lowercase(), syntax_rev));
        }
    }

    /// The syntax commits a target could refer to.
    pub fn resolve(&self, target: &BackoutTargetRef) -> Vec<Oid> {
        let git = |rev: &str| {
            Oid::from_str(rev)
                .ok()
                .and_then(|oid| self.source_to_syntax.get(&oid).copied())
        };
        match target {
            BackoutTargetRef::Git(rev) => git(rev).into_iter().collect(),
            BackoutTargetRef::Hg(rev) => {
                let matches: Vec<Oid> = rev
                    .get(..HG_PREFIX_LEN)
                    .and_then(|prefix| self.hg_by_prefix.get(prefix))
                    .into_iter()
                    .flatten()
                    .filter(|(hg_rev, _)| hg_rev.starts_with(rev.as_str()))
                    .map(|(_, syntax_rev)| *syntax_rev)
                    .collect();
                if matches.is_empty() && rev.len() == 40 {
                    // Some "Backed out changeset" messages may use git
                    // revisions.
                    git(rev).into_iter().collect()
                } else {
                    matches
                }
            }
        }
    }
}

/// Find the syntax commits that `backout` (a syntax commit whose source commit
/// has the given message) backs out: ancestors referenced by the message which
/// landed within `BACKOUT_HORIZON_SECS` before it.  They're ordered earliest
/// first by ancestry because commits landed together (ex: a stack of patches
/// in one push) can have the same timestamp.
pub fn find_backed_out(
    syntax_repo: &Repository,
    index: &BackoutTargetIndex,
    backout: &Commit,
    message: &str,
) -> Vec<Oid> {
    let backout_time = backout.committer().when().seconds();
    let mut found: Vec<(Oid, i64)> = vec![];
    for target in parse_backout_targets(message) {
        for oid in index.resolve(&target) {
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
        ] {
            let expected: Vec<BackoutTargetRef> =
                revs.into_iter().map(|r| Hg(r.to_string())).collect();
            assert_eq!(parse_backout_targets(message), expected, "{}", message);
        }
    }

    #[test]
    fn test_resolve() {
        let source = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let syntax = |n: u8| Oid::from_bytes(&[n + 100; 20]).unwrap();
        let mut index = BackoutTargetIndex::default();
        index.add(
            source(1),
            syntax(1),
            Some("f16bd0803a60aaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
        index.add(
            source(2),
            syntax(2),
            Some("f16bd0803a60bbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        );
        index.add(source(3), syntax(3), None);
        // An ambiguous prefix resolves to all candidates; `find_backed_out`
        // then only keeps ancestors within the horizon.
        assert_eq!(
            index.resolve(&Hg("f16bd0803a60".to_string())),
            vec![syntax(1), syntax(2)]
        );
        assert_eq!(
            index.resolve(&Hg("f16bd0803a60bb".to_string())),
            vec![syntax(2)]
        );
        assert_eq!(index.resolve(&Git(source(3).to_string())), vec![syntax(3)]);
        assert_eq!(index.resolve(&Hg(source(3).to_string())), vec![syntax(3)]);
        assert_eq!(index.resolve(&Git(source(4).to_string())), vec![]);
    }
}
