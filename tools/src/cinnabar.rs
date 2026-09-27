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
//!
//! ## Mapping old revisions to new revisions
//!
//! For the permalinks, web-server.rs needs the reverse mapping, from old
//! revisions to new revisions.  build-blame records it in git notes in the
//! blame repo (see `git_notes`), one notes ref per branch,
//! `refs/notes/mozsearch-old-revision-mapping-<BRANCH>`, keyed by old revision
//! and containing the new revision, so that the web server can look up old
//! revisions without walking the whole blame branch to build an in-memory map
//! (bug 1983433).  See `write_old_revision_notes` and `OldRevisionMap`.

use std::collections::HashMap;
use std::fs::read_to_string;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use git2::{Oid, Repository};

use crate::git_notes::{NotesReader, NotesWriter, note_blob_id, notes_ref_for_branch};

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

/// The revisions of an `oldrevs` value.
pub fn parse_oldrevs(oldrevs: &str) -> impl Iterator<Item = Oid> + '_ {
    oldrevs.split(',').filter_map(|rev| Oid::from_str(rev).ok())
}

pub const OLD_REVISION_NOTES_REF_PREFIX: &str = "refs/notes/mozsearch-old-revision-mapping-";

/// The blame repo's old revision notes ref for the branch `blame_ref`, which
/// is `refs/notes/mozsearch-old-revision-mapping-<BRANCH>`; see
/// `git_notes::notes_ref_for_branch`.
pub fn old_revision_notes_ref(blame_repo: &Repository, blame_ref: &str) -> String {
    notes_ref_for_branch(blame_repo, OLD_REVISION_NOTES_REF_PREFIX, blame_ref)
}

/// A writer for the blame repo's old revision notes for the branch
/// `blame_ref`.
pub fn old_revision_notes_writer(blame_repo: &Repository, blame_ref: &str) -> NotesWriter {
    NotesWriter::new(
        blame_repo,
        &old_revision_notes_ref(blame_repo, blame_ref),
        "old revisions to new revisions",
    )
}

/// Bring the old revision notes of `writer` up to date with `old_to_new`,
/// which should map all of the old revisions of the branch's revisions (as
/// found by `index_blame`) to their new revisions, by writing a notes commit
/// with the notes which are missing or differ to the git fast-import `stream`.
/// Returns how many notes that was.  build-blame uses this when it seeds a
/// blame branch's notes (see `source_mapping`); otherwise it writes notes for
/// the revisions it processes.
///
/// This compares the note blob ids with those of the expected contents rather
/// than looking up each note, which takes a few seconds for a million notes
/// rather than minutes.
pub fn write_old_revision_notes(
    blame_repo: &Repository,
    writer: &mut NotesWriter,
    old_to_new: &HashMap<Oid, Oid>,
    time: i64,
    stream: &mut impl Write,
) -> io::Result<usize> {
    let existing = NotesReader::open(blame_repo, [writer.notes_ref()])
        .note_blob_ids(blame_repo)
        .map_err(io::Error::other)?;
    let mut needed: Vec<(Oid, Oid)> = old_to_new
        .iter()
        .filter(|(old_rev, new_rev)| existing.get(old_rev) != Some(&note_blob_id(**new_rev)))
        .map(|(old_rev, new_rev)| (*old_rev, *new_rev))
        .collect();
    needed.sort();
    for (old_rev, new_rev) in &needed {
        writer.add(*old_rev, &new_rev.to_string(), time);
    }
    writer.flush(stream)?;
    Ok(needed.len())
}

/// How the web server maps a tree's old revisions to new revisions.
pub enum OldRevisionMap {
    /// The old revision notes for the tree's blame branch.
    Notes {
        notes_ref: String,
        reader: NotesReader,
    },
    /// A map built by walking the blame branch (`index_blame`), for blame repos
    /// from before build-blame wrote the notes.
    InMemory(HashMap<Oid, Oid>),
}

impl Default for OldRevisionMap {
    fn default() -> Self {
        OldRevisionMap::InMemory(HashMap::new())
    }
}

impl OldRevisionMap {
    /// The notes-based map for the blame repo's branch `blame_ref`, if the notes
    /// exist.
    pub fn from_notes(blame_repo: &Repository, blame_ref: &str) -> Option<OldRevisionMap> {
        let notes_ref = old_revision_notes_ref(blame_repo, blame_ref);
        let reader = NotesReader::open(blame_repo, [notes_ref.as_str()]);
        (!reader.is_empty()).then_some(OldRevisionMap::Notes { notes_ref, reader })
    }

    /// The new revision of `old_rev`, if known.  `blame_repo` is only needed for
    /// notes.
    pub fn get(&self, blame_repo: Option<&Repository>, old_rev: Oid) -> Option<Oid> {
        match self {
            OldRevisionMap::Notes { reader, .. } => reader.lookup(blame_repo?, old_rev),
            OldRevisionMap::InMemory(map) => map.get(&old_rev).copied(),
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
    use crate::git_notes::test_support::fast_import;

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

    #[test]
    fn test_old_revision_notes() {
        let dir = std::env::temp_dir().join(format!("old-revision-notes-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let old = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let new = |n: u8| Oid::from_bytes(&[n + 100; 20]).unwrap();
        assert_eq!(
            old_revision_notes_ref(&repo, "refs/heads/beta"),
            "refs/notes/mozsearch-old-revision-mapping-beta"
        );
        assert!(OldRevisionMap::from_notes(&repo, "refs/heads/beta").is_none());

        let write = |old_to_new: &HashMap<Oid, Oid>| {
            let mut num_written = 0;
            fast_import(&repo, |stream| {
                let mut writer = old_revision_notes_writer(&repo, "refs/heads/beta");
                num_written =
                    write_old_revision_notes(&repo, &mut writer, old_to_new, 1000, stream).unwrap();
            });
            num_written
        };
        // Seeding writes everything, then only missing or different notes are
        // written.
        let mut old_to_new = HashMap::from([(old(1), new(1)), (old(2), new(2))]);
        assert_eq!(write(&old_to_new), 2);
        assert_eq!(write(&old_to_new), 0);
        old_to_new.insert(old(3), new(3));
        old_to_new.insert(old(2), new(9));
        assert_eq!(write(&old_to_new), 2);

        let map = OldRevisionMap::from_notes(&repo, "refs/heads/beta").unwrap();
        assert_eq!(map.get(Some(&repo), old(1)), Some(new(1)));
        assert_eq!(map.get(Some(&repo), old(2)), Some(new(9)));
        assert_eq!(map.get(Some(&repo), old(3)), Some(new(3)));
        assert_eq!(map.get(Some(&repo), old(4)), None);
        assert_eq!(map.get(None, old(1)), None);
        let in_memory = OldRevisionMap::InMemory(old_to_new);
        assert_eq!(in_memory.get(None, old(3)), Some(new(3)));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
