//! Landing dates: when revisions landed, at the earliest, which the history's
//! dates (its journals' and rev-summaries' `iso_date`s) are, rather than the
//! commits' dates.  Mercurial had one date, the author's, which git-cinnabar's
//! commits have as their committer dates too, and which a patch kept when it
//! landed weeks later or was relanded after a backout (in firefox-main, 30% of
//! 2016-2017's commits are dated more than a day before one of their
//! ancestors).  A revision can't have landed before its parents, so its
//! landing date is the latest of its and its ancestors' commit dates, and
//! landing dates only go forward along the history.  (The commit index's
//! `by-rev` has the same dates for every commit of a branch; see
//! `tools::commit_index`.)
//!
//! `build-timeline-tree` works them out for the revisions it processes, parents
//! first; the revisions processed before (ex: by an earlier chunk) have theirs
//! in their rev-summaries.
//!
//! For hg's revisions, hg.mozilla.org's pushlogs have when each was first
//! pushed (see `PushDates`), which is when it landed, and which is a revision's
//! own date rather than its commit date when a push dates file is given.
//! Revisions which landed together in a push have its date.  (A revision's
//! first push can't be before its parents', since a repo has a revision's
//! parents before it; the latest of its ancestors' is still taken, in case.)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat};
use git2::{Commit, Oid};
use serde::Deserialize;

use crate::commit_index::SortedLines;
use crate::file_format::history::rev_summaries::rev_summary_path;

/// When hg's revisions were first pushed to mozilla-central, an integration
/// repo (ex: autoland) or a release repo, from a push dates file of
/// `GIT_REV<TAB>HG_REV<TAB>PUSHED<TAB>REPO<TAB>PUSH_ID` lines sorted by git
/// revision (PUSHED is ISO 8601, in UTC), derived from an archive of
/// hg.mozilla.org's pushlogs (see `scripts/pushlog/`).
pub struct PushDates {
    lines: SortedLines,
}

impl PushDates {
    pub fn open(path: &Path) -> PushDates {
        PushDates {
            lines: SortedLines::open(path),
        }
    }

    /// When the git revision `rev` was first pushed, if it's hg's.
    pub fn get(&self, rev: Oid) -> Option<i64> {
        let fields = self.lines.find(&rev.to_string()).into_iter().next()?;
        parse_iso(fields.get(1)?)
    }
}

/// The landing dates (in seconds) of a run's revisions, and of those processed
/// before, from their rev-summaries.
#[derive(Default)]
pub struct LandingDates {
    by_syntax: HashMap<Oid, i64>,
    by_source: HashMap<Oid, i64>,
    /// The rev-summaries directory, if there is one.
    rev_summaries: Option<PathBuf>,
    /// When hg's revisions were pushed, if known.
    pushes: Option<PushDates>,
}

/// A commit's own date: its committer date (for hg's commits, its author's).
pub fn commit_seconds(commit: &Commit) -> i64 {
    commit.committer().when().seconds()
}

/// An ISO 8601 date in UTC, as the history writes them (ex:
/// "2014-02-12T17:46:24Z").
pub fn iso_seconds(seconds: i64) -> String {
    DateTime::from_timestamp(seconds, 0)
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn parse_iso(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|date| date.timestamp())
}

/// The source revision of a syntax commit, from its message's `git` line (see
/// `syntax_commit_to_meta`), if it has one.
fn syntax_source_rev(commit: &Commit) -> Option<Oid> {
    let message = commit.message().ok()?;
    let mut words = message.split_whitespace();
    while let Some(key) = words.next() {
        let value = words.next()?;
        if key == "git" {
            return Oid::from_str(value).ok();
        }
    }
    None
}

impl LandingDates {
    /// With the rev-summaries in `rev_summaries` for the revisions processed
    /// before.
    pub fn new(rev_summaries: &Path) -> LandingDates {
        LandingDates {
            rev_summaries: Some(rev_summaries.to_path_buf()),
            ..Default::default()
        }
    }

    /// With `pushes` for hg's revisions' dates.
    pub fn with_push_dates(mut self, pushes: PushDates) -> LandingDates {
        self.pushes = Some(pushes);
        self
    }

    /// A source revision's own date: when it was pushed, if known, or else
    /// `commit_seconds`.
    fn own(&self, source_rev: Oid, commit_seconds: i64) -> i64 {
        self.pushes
            .as_ref()
            .and_then(|pushes| pushes.get(source_rev))
            .unwrap_or(commit_seconds)
    }

    /// Add the syntax commit `commit` of the source revision `source_rev`,
    /// once its parents have been added (or were processed before), returning
    /// its landing date.
    pub fn add(&mut self, commit: &Commit, source_rev: Oid) -> i64 {
        let own = self.own(source_rev, commit_seconds(commit));
        let landed = commit
            .parents()
            .map(|parent| self.syntax(&parent))
            .fold(own, i64::max);
        self.by_syntax.insert(commit.id(), landed);
        self.by_source.insert(source_rev, landed);
        landed
    }

    /// Record a source revision's landing date (ex: from the commit index).
    pub fn insert_source(&mut self, source_rev: Oid, landed: i64) {
        self.by_source.insert(source_rev, landed);
    }

    /// A processed revision's landing date, from its rev-summary.
    fn summarized(&self, source_rev: Oid) -> Option<i64> {
        #[derive(Deserialize)]
        struct Dated {
            iso_date: String,
        }
        let path = self
            .rev_summaries
            .as_ref()?
            .join(rev_summary_path(&source_rev.to_string()));
        let file = std::fs::File::open(path).ok()?;
        let dated: Dated = serde_json::from_reader(std::io::BufReader::new(file)).ok()?;
        parse_iso(&dated.iso_date)
    }

    /// The landing date of a syntax commit: this run's, a processed
    /// revision's, or else its own date (see `own`).
    pub fn syntax(&self, commit: &Commit) -> i64 {
        if let Some(&landed) = self.by_syntax.get(&commit.id()) {
            return landed;
        }
        match syntax_source_rev(commit) {
            Some(source_rev) => self
                .by_source
                .get(&source_rev)
                .copied()
                .or_else(|| self.summarized(source_rev))
                .unwrap_or_else(|| self.own(source_rev, commit_seconds(commit))),
            None => commit_seconds(commit),
        }
    }

    /// The landing date of a source commit: this run's, a processed
    /// revision's, or else its own date (see `own`).
    pub fn source(&self, commit: &Commit) -> i64 {
        self.by_source
            .get(&commit.id())
            .copied()
            .or_else(|| self.summarized(commit.id()))
            .unwrap_or_else(|| self.own(commit.id(), commit_seconds(commit)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    #[test]
    fn test_landing_dates() {
        let dir = std::env::temp_dir().join(format!("hb-landing-{}", std::process::id()));
        let repo = Repository::init(dir.join("repo")).unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let commit = |message: &str, parents: &[Oid], seconds: i64| -> Oid {
            let sig =
                git2::Signature::new("A", "a@example.com", &git2::Time::new(seconds, 0)).unwrap();
            let parents: Vec<Commit> = parents
                .iter()
                .map(|p| repo.find_commit(*p).unwrap())
                .collect();
            let parents: Vec<&Commit> = parents.iter().collect();
            repo.commit(None, &sig, &sig, message, &tree, &parents)
                .unwrap()
        };
        // (Syntax commits name their source revisions.)
        let source = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let syntax = |n: u8, parents: &[Oid], seconds: i64| {
            commit(&format!("git {}\n", source(n)), parents, seconds)
        };
        let base = syntax(1, &[], 2_000);
        // (A patch dated before it landed, as hg's were.)
        let stale = syntax(2, &[base], 500);
        let later = syntax(3, &[stale], 3_000);

        // (The base was processed before: its rev-summary has its date.)
        let summaries = dir.join("rev-summaries");
        let path = summaries.join(rev_summary_path(&source(1).to_string()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!(r#"{{"iso_date":"{}"}}"#, iso_seconds(2_500))).unwrap();
        let mut landing = LandingDates::new(&summaries);
        let find = |oid: Oid| repo.find_commit(oid).unwrap();
        assert_eq!(landing.add(&find(stale), source(2)), 2_500);
        assert_eq!(landing.add(&find(later), source(3)), 3_000);
        assert_eq!(landing.syntax(&find(base)), 2_500);
        // (By source revision, for source commits; unknown ones have their
        // own dates.)
        let source_commit = commit("Bug 1 - Unknown", &[], 700);
        assert_eq!(landing.source(&find(source_commit)), 700);
        landing.insert_source(source_commit, 900);
        assert_eq!(landing.source(&find(source_commit)), 900);
        assert_eq!(iso_seconds(2_500), "1970-01-01T00:41:40Z");

        // (With push dates: the stale patch was pushed at 2,600, after its
        // base's landing, so it and its child land then.)
        let pushes_path = dir.join("pushes.tsv");
        let mut lines = vec![
            format!(
                "{}\t{}\t{}\tmozilla-central\t7",
                source(2),
                "a".repeat(40),
                iso_seconds(2_600)
            ),
            format!(
                "{}\t{}\t{}\tmozilla-central\t8",
                source(3),
                "b".repeat(40),
                iso_seconds(2_700)
            ),
        ];
        lines.sort();
        std::fs::write(&pushes_path, lines.join("\n") + "\n").unwrap();
        let mut landing =
            LandingDates::new(&summaries).with_push_dates(PushDates::open(&pushes_path));
        assert_eq!(landing.add(&find(stale), source(2)), 2_600);
        // (Its own commit date, 3,000, is later than its push: clocks.)
        assert_eq!(landing.add(&find(later), source(3)), 2_700);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
