//! Helpers for asking git-cinnabar about the relationship between git and hg
//! revisions, and for determining the "old" revisions ("oldrevs") of a
//! revision.
//!
//! For the firefox-* trees, the source repository is the new canonical Firefox
//! git repository with git-cinnabar metadata, so it can tell us each
//! revision's hg revision (`git cinnabar git2hg`) and vice versa (`git cinnabar
//! hg2git`).  We also have an "old" git-cinnabar repository using the original
//! gecko-dev revisions, and by asking it for the git revision of the hg
//! revision we get the gecko-dev revision, which lets us support permalinks
//! from the old mozilla-* trees.  This is the "double cinnabar" approach from
//! bug 1890435.  Some CVS-era revisions no longer exist in the new repository,
//! so a mapping file provides the old revisions for those, taking precedence.
//! build-blame records the old revisions in its commit messages as
//! `oldrevs OID,OID,...` and so do the history tools.

use std::collections::HashMap;
use std::fs::read_to_string;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use git2::{Oid, Repository};

/// A `git cinnabar git2hg --batch` or `git cinnabar hg2git --batch` process.
pub struct CinnabarBatch {
    child: Child,
    stdout: BufReader<ChildStdout>,
}

impl CinnabarBatch {
    fn start(repo: &Repository, command: &str) -> CinnabarBatch {
        let mut child = Command::new("git")
            .arg("cinnabar")
            .arg(command)
            .arg("--batch")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .current_dir(repo.path())
            .spawn()
            .unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        CinnabarBatch { child, stdout }
    }

    /// For looking up the hg revisions of git revisions.
    pub fn git2hg(repo: &Repository) -> CinnabarBatch {
        CinnabarBatch::start(repo, "git2hg")
    }

    /// For looking up the git revisions of hg revisions.
    pub fn hg2git(repo: &Repository) -> CinnabarBatch {
        CinnabarBatch::start(repo, "hg2git")
    }

    /// Look up the corresponding revision, returning None if cinnabar doesn't
    /// know about it (which it expresses as all zeroes, including for
    /// repositories without any cinnabar metadata).
    pub fn lookup(&mut self, rev: &str) -> Option<String> {
        writeln!(self.child.stdin.as_mut().unwrap(), "{}", rev).unwrap();
        let mut result = String::new();
        self.stdout.read_line(&mut result).unwrap();
        let result = result.trim();
        if result.is_empty() || result.chars().all(|c| c == '0') {
            None
        } else {
            Some(result.to_string())
        }
    }
}

impl Drop for CinnabarBatch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Determines the old revisions of revisions; see the module docs.
#[derive(Default)]
pub struct OldRevisions {
    old_hg2git: Option<CinnabarBatch>,
    /// Maps new revisions to comma-separated old revisions.
    map: HashMap<Oid, String>,
}

impl OldRevisions {
    /// `old_cinnabar_repo` is the old git-cinnabar repository and
    /// `old_revision_map` a file of `NEWREV OLDREV1,OLDREV2,...` lines, both
    /// optional.
    pub fn new(
        old_cinnabar_repo: Option<&Path>,
        old_revision_map: Option<&Path>,
    ) -> Result<OldRevisions, String> {
        let old_hg2git = match old_cinnabar_repo {
            Some(path) => {
                let repo = Repository::open(path)
                    .map_err(|e| format!("Unable to open {}: {}", path.display(), e))?;
                Some(CinnabarBatch::hg2git(&repo))
            }
            None => None,
        };
        let map = match old_revision_map {
            Some(path) => parse_old_revision_map(
                &read_to_string(path)
                    .map_err(|e| format!("Unable to read {}: {}", path.display(), e))?,
            )?,
            None => HashMap::new(),
        };
        Ok(OldRevisions { old_hg2git, map })
    }

    /// The comma-separated old revisions of the (new) git revision `rev` whose
    /// hg revision is `hg_rev`, if known.
    pub fn lookup(&mut self, rev: Oid, hg_rev: Option<&str>) -> Option<String> {
        if let Some(oldrevs) = self.map.get(&rev) {
            return Some(oldrevs.clone());
        }
        match (hg_rev, &mut self.old_hg2git) {
            (Some(hg_rev), Some(old_hg2git)) => old_hg2git.lookup(hg_rev),
            _ => None,
        }
    }
}

fn parse_old_revision_map(contents: &str) -> Result<HashMap<Oid, String>, String> {
    let mut map = HashMap::new();
    for line in contents.lines() {
        if let Some((newrev, oldrevs)) = line.split_once(' ') {
            let newrev = Oid::from_str(newrev)
                .map_err(|e| format!("Bad revision in old revision map line {:?}: {}", line, e))?;
            map.insert(newrev, oldrevs.trim().to_owned());
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_old_revision_map() {
        let new1 = "1111111111111111111111111111111111111111";
        let new2 = "2222222222222222222222222222222222222222";
        let map = parse_old_revision_map(&format!(
            "{} aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
             {} bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,cccccccccccccccccccccccccccccccccccccccc\n",
            new1, new2
        ))
        .unwrap();
        let mut old_revisions = OldRevisions {
            old_hg2git: None,
            map,
        };
        assert_eq!(
            old_revisions.lookup(Oid::from_str(new2).unwrap(), Some("ffff")),
            Some(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,cccccccccccccccccccccccccccccccccccccccc"
                    .to_string()
            )
        );
        // Without the old cinnabar repository, revisions not in the map don't
        // have old revisions.
        assert_eq!(
            old_revisions.lookup(Oid::from_bytes(&[3; 20]).unwrap(), Some("ffff")),
            None
        );
        assert!(parse_old_revision_map("nothex aaaa\n").is_err());
    }
}
