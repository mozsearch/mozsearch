//! Mapping source revisions to the history repos' commits with git notes (bug
//! 1983433).
//!
//! The syntax and timeline repos each have a notes ref, by default
//! `refs/notes/mozsearch-source-mapping-<BRANCH>`, whose notes are keyed by
//! source revision and contain the hex id of the syntax/timeline commit derived
//! from it.  Like git-cinnabar's hg2git notes, the annotated objects (source
//! revisions) don't exist in the repos, which git notes allow.
//!
//! This lets the history tools find the revisions they've already processed
//! without walking the whole history branch to build in-memory maps: they walk
//! the source (or syntax) history with a revwalk hide callback which hides
//! processed revisions, so only the new revisions and the processed parents of
//! the oldest ones are visited.  The notes also cover processed revisions which
//! aren't reachable from the branch, like the first history of a merge of long
//! unrelated histories when a run stopped partway through the second (which is
//! what build-blame's marks file is for).  The web server will be able to map
//! source revisions to history commits the same way.
//!
//! ## Multiple heads (try and review)
//!
//! For trees like try, where we'll process many unrelated heads which are based
//! on revisions from another branch, the `NOTES_REF` environment variable can
//! name a single notes ref which accumulates all of the processed revisions
//! (with the branch just pointing at the most recently processed head), and
//! `READ_NOTES_REFS` can name (space-separated) notes refs to also consult,
//! like the main branch's, so the base revisions aren't processed again.
//! Nothing is ever removed from the notes and we assume the repos are never
//! garbage collected, since a try revision's commits may only be reachable
//! from the notes (which don't keep them alive); a server just accumulates
//! revisions until it's replaced by a fresh one.
//!
//! ## Writing
//!
//! git fast-import's `N` command requires the annotated object to exist, so we
//! write the notes as regular files in the notes commits, using the 2 level
//! fanout (`ab/cd/<36 hex digits>`) git would use for between 65536 and 16M
//! notes.  (Reads also accept the other fanouts in case git ever rewrites the
//! notes.)  Notes are written in batches of `NOTES_BATCH_SIZE` revisions, and
//! only for revisions whose commits were completely written.  Since the
//! history commits are a deterministic function of their inputs, a crash just
//! means the revisions since the last batch get processed again with identical
//! results, and a commit left incomplete by a crash (git fast-import commits
//! whatever it has at the end of its input) is replaced rather than being
//! mistaken for a processed revision.

use std::env;
use std::io::{self, Write};
use std::path::Path;

use git2::{Oid, Repository};

pub const NOTES_REF_PREFIX: &str = "refs/notes/mozsearch-source-mapping-";

/// How many revisions we process between writing notes commits.
pub const NOTES_BATCH_SIZE: usize = 100;

/// The notes refs a history tool writes and consults for a repo.
pub struct NotesRefs {
    /// The notes ref we write to, which is also consulted first.
    pub write: String,
    /// Additional notes refs to consult, in order.
    pub read: Vec<String>,
}

impl NotesRefs {
    /// The notes refs for `repo` when processing `blame_ref`, which are
    /// `default_notes_ref` unless overridden by the `NOTES_REF` and
    /// `READ_NOTES_REFS` environment variables; see the module docs.
    pub fn from_env(repo: &Repository, blame_ref: &str) -> NotesRefs {
        NotesRefs {
            write: env::var("NOTES_REF").unwrap_or_else(|_| default_notes_ref(repo, blame_ref)),
            read: env::var("READ_NOTES_REFS")
                .map(|refs| refs.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }

    fn all(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.write).chain(&self.read)
    }
}

/// `refs/notes/mozsearch-source-mapping-<BRANCH>` for a `blame_ref` of
/// `refs/heads/<BRANCH>`, or for `HEAD`, the branch the repo's HEAD refers to
/// (so that both ways of naming the branch share notes).
pub fn default_notes_ref(repo: &Repository, blame_ref: &str) -> String {
    let head_target = if blame_ref == "HEAD" {
        repo.find_reference("HEAD")
            .ok()
            .and_then(|head| head.symbolic_target().ok().flatten().map(str::to_string))
    } else {
        None
    };
    let target = head_target.as_deref().unwrap_or(blame_ref);
    let name = target
        .strip_prefix("refs/heads/")
        .or_else(|| target.strip_prefix("refs/"))
        .unwrap_or(target);
    format!("{}{}", NOTES_REF_PREFIX, name)
}

/// The path of the note for `rev` in a notes tree with our 2 level fanout.
pub fn note_path(rev: Oid) -> String {
    let hex = rev.to_string();
    format!("{}/{}/{}", &hex[..2], &hex[2..4], &hex[4..])
}

/// The paths a note for `rev` could have, for all the fanouts git uses in
/// practice, ours first.
fn note_paths(rev: Oid) -> [String; 3] {
    let hex = rev.to_string();
    [note_path(rev), format!("{}/{}", &hex[..2], &hex[2..]), hex]
}

/// Read access to the notes of a repo as of when they were opened.  This only
/// holds object ids so it can be shared between threads, each of which passes
/// its own `Repository` to `lookup`.
#[derive(Clone)]
pub struct SourceMapping {
    /// The trees of the notes refs which exist, in lookup order.
    trees: Vec<Oid>,
}

impl SourceMapping {
    pub fn open(repo: &Repository, refs: &NotesRefs) -> SourceMapping {
        SourceMapping {
            trees: refs
                .all()
                .filter_map(|name| repo.find_reference(name).ok()?.peel_to_tree().ok())
                .map(|tree| tree.id())
                .collect(),
        }
    }

    /// True if none of the notes refs exist.
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// The history commit id recorded for `source_rev`, if any.
    pub fn lookup(&self, repo: &Repository, source_rev: Oid) -> Option<Oid> {
        let paths = note_paths(source_rev);
        for tree_id in &self.trees {
            let Ok(tree) = repo.find_tree(*tree_id) else {
                continue;
            };
            for path in &paths {
                if let Ok(entry) = tree.get_path(Path::new(path)) {
                    let blob = repo.find_blob(entry.id()).ok()?;
                    return std::str::from_utf8(blob.content())
                        .ok()
                        .and_then(|contents| Oid::from_str(contents.trim()).ok());
                }
            }
        }
        None
    }
}

/// Exit with an error if `repo` has a `blame_ref` branch but none of the notes
/// refs exist, which means that it was built by a version of the history tools
/// which didn't write notes and needs to be regenerated.  (Otherwise we would
/// quietly process all of its history again.)
pub fn require_notes_for_existing_branch(
    repo: &Repository,
    blame_ref: &str,
    refs: &NotesRefs,
    mapping: &SourceMapping,
) {
    if mapping.is_empty() && repo.refname_to_id(blame_ref).is_ok() {
        log::error!(
            "{} has {} but no {}, so it was built without source mapping notes and needs \
             to be regenerated.",
            repo.path().display(),
            blame_ref,
            refs.all().cloned().collect::<Vec<_>>().join(" or ")
        );
        std::process::exit(1);
    }
}

/// Accumulates notes and writes them as notes commits in a git fast-import
/// stream.
pub struct NotesWriter {
    notes_ref: String,
    /// The existing notes commit, which our first notes commit builds on.
    /// Subsequent notes commits build on the previous one, which fast-import
    /// does by default.
    from: Option<Oid>,
    wrote_commit: bool,
    pending: Vec<(Oid, String)>,
    /// The commit time of the most recently added revision, which we use for
    /// the notes commits so that they're deterministic.
    time: i64,
}

impl NotesWriter {
    pub fn new(repo: &Repository, notes_ref: &str) -> NotesWriter {
        NotesWriter {
            notes_ref: notes_ref.to_string(),
            from: repo.refname_to_id(notes_ref).ok(),
            wrote_commit: false,
            pending: vec![],
            time: 0,
        }
    }

    /// Record that `source_rev` (committed at `time`) was processed into
    /// the commit with hex id `commit`.
    pub fn add(&mut self, source_rev: Oid, commit: &str, time: i64) {
        self.pending.push((source_rev, commit.to_string()));
        self.time = time;
    }

    pub fn num_pending(&self) -> usize {
        self.pending.len()
    }

    /// Write a notes commit with the pending notes, if any.  This must not be
    /// called in the middle of writing another commit.
    pub fn flush(&mut self, stream: &mut impl Write) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        writeln!(stream, "commit {}", self.notes_ref)?;
        writeln!(stream, "committer mozsearch <> {} +0000", self.time)?;
        let message = format!(
            "Map {} source revisions to history commits\n",
            self.pending.len()
        );
        write!(stream, "data {}\n{}", message.len(), message)?;
        if !self.wrote_commit
            && let Some(from) = self.from
        {
            writeln!(stream, "from {}", from)?;
        }
        for (source_rev, commit) in self.pending.drain(..) {
            let contents = format!("{}\n", commit);
            writeln!(stream, "M 100644 inline {}", note_path(source_rev))?;
            write!(stream, "data {}\n{}", contents.len(), contents)?;
        }
        writeln!(stream)?;
        self.wrote_commit = true;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::process::{ChildStdin, Command, Stdio};

    pub fn fast_import(repo: &Repository, write: impl FnOnce(&mut ChildStdin)) {
        let mut child = Command::new("git")
            .arg("fast-import")
            .arg("--quiet")
            .stdin(Stdio::piped())
            .current_dir(repo.path())
            .spawn()
            .unwrap();
        write(child.stdin.as_mut().unwrap());
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
    }

    /// Write a notes commit mapping source revisions to history commits.
    pub fn write_notes(repo: &Repository, notes_ref: &str, notes: &[(Oid, Oid)]) {
        fast_import(repo, |stream| {
            let mut writer = NotesWriter::new(repo, notes_ref);
            for (source_rev, commit) in notes {
                writer.add(*source_rev, &commit.to_string(), 0);
            }
            writer.flush(stream).unwrap();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::fast_import;
    use super::*;
    use std::process::Command;

    #[test]
    fn test_note_paths() {
        let rev = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        assert_eq!(
            note_paths(rev),
            [
                "01/23/456789abcdef0123456789abcdef01234567".to_string(),
                "01/23456789abcdef0123456789abcdef01234567".to_string(),
                "0123456789abcdef0123456789abcdef01234567".to_string(),
            ]
        );
    }

    #[test]
    fn test_write_and_lookup() {
        let dir = std::env::temp_dir().join(format!("hb-source-mapping-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let refs = NotesRefs {
            write: format!("{}main", NOTES_REF_PREFIX),
            read: vec![format!("{}other", NOTES_REF_PREFIX)],
        };
        let rev = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let commit = |n: u8| Oid::from_bytes(&[n + 100; 20]).unwrap();
        assert_eq!(default_notes_ref(&repo, "refs/heads/main"), refs.write);
        assert!(SourceMapping::open(&repo, &refs).is_empty());

        // Two batches in one stream, then another run which builds on them.
        fast_import(&repo, |stream| {
            let mut writer = NotesWriter::new(&repo, &refs.write);
            writer.add(rev(1), &commit(1).to_string(), 1000);
            writer.add(rev(2), &commit(2).to_string(), 1001);
            writer.flush(stream).unwrap();
            writer.add(rev(3), &commit(3).to_string(), 1002);
            writer.flush(stream).unwrap();
            // Flushing without pending notes doesn't write a commit.
            writer.flush(stream).unwrap();
        });
        fast_import(&repo, |stream| {
            let mut writer = NotesWriter::new(&repo, &refs.write);
            writer.add(rev(4), &commit(4).to_string(), 1003);
            writer.flush(stream).unwrap();
        });
        // A read-only ref consulted after the one we write, whose note for
        // rev(1) is shadowed.
        fast_import(&repo, |stream| {
            let mut writer = NotesWriter::new(&repo, &refs.read[0]);
            writer.add(rev(1), &commit(9).to_string(), 1000);
            writer.add(rev(5), &commit(5).to_string(), 1000);
            writer.flush(stream).unwrap();
        });

        let mapping = SourceMapping::open(&repo, &refs);
        for n in 1..=5 {
            assert_eq!(mapping.lookup(&repo, rev(n)), Some(commit(n)));
        }
        assert_eq!(mapping.lookup(&repo, rev(6)), None);
        let notes_commit = repo
            .find_reference(&refs.write)
            .unwrap()
            .peel_to_commit()
            .unwrap();
        assert_eq!(notes_commit.parent_count(), 1);
        assert_eq!(notes_commit.time().seconds(), 1003);

        // git itself can read the notes, even though the annotated objects
        // don't exist.
        let output = Command::new("git")
            .args(["notes", "--ref", &refs.write, "show", &rev(2).to_string()])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            commit(2).to_string()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
