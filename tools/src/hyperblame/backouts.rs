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
//!   These are resolved by asking git-cinnabar for the source repo's git
//!   revision, which it can do for abbreviated hg revisions.
//!
//! Git revisions are then mapped to syntax commits with the syntax repo's
//! source mapping notes (see `hyperblame::source_mapping`).
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
use crate::hyperblame::source_mapping::SourceMapping;

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
        }
    }

    /// The syntax commit a target refers to, if its revision has been
    /// processed.
    pub fn resolve(&self, syntax_repo: &Repository, target: &BackoutTargetRef) -> Option<Oid> {
        self.syntax_mapping
            .lookup(syntax_repo, self.source_rev(target)?)
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
    for target in parse_backout_targets(message) {
        let Some(oid) = resolver.resolve(syntax_repo, &target) else {
            continue;
        };
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
    use crate::hyperblame::source_mapping::{NotesRefs, default_notes_ref};
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
}
