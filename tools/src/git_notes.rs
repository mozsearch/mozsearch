//! Maps from revisions to other revisions stored as git notes and written with
//! git fast-import (bug 1983433).  The keys are revisions which usually don't
//! exist in the repo holding the notes (which git notes allow, and which is also
//! how git-cinnabar's hg2git notes work), and each note contains the hex id of
//! the revision the key maps to.  Lookups only read the few objects they need,
//! so these can replace in-memory maps built by walking a whole branch.
//!
//! Uses:
//! - `source_mapping`: source revisions to blame and history repo commits.
//! - `cinnabar::OldRevisionMap`: old revisions (ex: gecko-dev revisions) to new
//!   source revisions, in the blame repo.
//!
//! git fast-import's `N` command requires the annotated object to exist, so we
//! write the notes as regular files in notes commits, using the 2 level fanout
//! (`ab/cd/<36 hex digits>`) git would use for between 65536 and 16M notes.
//! (Reads also accept the other fanouts in case git ever rewrites the notes.)
//! git itself can read them, ex: `git notes --ref <REF> show <KEY>`.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

use git2::{ObjectType, Oid, Repository, TreeWalkMode, TreeWalkResult};

/// `<PREFIX><BRANCH>` for a `blame_ref` of `refs/heads/<BRANCH>`, or for
/// `HEAD`, the branch the repo's HEAD refers to (so that both ways of naming
/// the branch share notes).
pub fn notes_ref_for_branch(repo: &Repository, prefix: &str, blame_ref: &str) -> String {
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
    format!("{}{}", prefix, name)
}

/// The path of the note for `key` in a notes tree with our 2 level fanout.
pub fn note_path(key: Oid) -> String {
    let hex = key.to_string();
    format!("{}/{}/{}", &hex[..2], &hex[2..4], &hex[4..])
}

/// The contents of a note mapping to the revision with hex id `value`.
fn note_contents(value: &str) -> String {
    format!("{}\n", value)
}

/// The blob id of a note mapping to `value`, for comparing with
/// `NotesReader::note_blob_ids`.
pub fn note_blob_id(value: Oid) -> Oid {
    Oid::hash_object(
        ObjectType::Blob,
        note_contents(&value.to_string()).as_bytes(),
    )
    .unwrap()
}

/// The paths a note for `key` could have, for all the fanouts git uses in
/// practice, ours first.
fn note_paths(key: Oid) -> [String; 3] {
    let hex = key.to_string();
    [note_path(key), format!("{}/{}", &hex[..2], &hex[2..]), hex]
}

/// Read access to one or more notes refs as of when they were opened.  This
/// only holds object ids so it can be shared between threads, each of which
/// passes its own `Repository` to `lookup`.
#[derive(Clone)]
pub struct NotesReader {
    /// The trees of the notes refs which exist, in lookup order.
    trees: Vec<Oid>,
}

impl NotesReader {
    /// Open the given notes refs, which are consulted in order.  Refs which
    /// don't exist are ignored.
    pub fn open<'a>(repo: &Repository, refs: impl IntoIterator<Item = &'a str>) -> NotesReader {
        NotesReader {
            trees: refs
                .into_iter()
                .filter_map(|name| repo.find_reference(name).ok()?.peel_to_tree().ok())
                .map(|tree| tree.id())
                .collect(),
        }
    }

    /// True if none of the notes refs exist.
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// The blob ids of all of the notes by key, which lets us check many notes
    /// against what they should contain (see `note_blob_id`) by reading just
    /// the notes trees.  (Lookups read 3 trees each, and libgit2 doesn't cache
    /// fanout trees because they're bigger than its size limit for caching,
    /// so a million lookups take minutes.)
    pub fn note_blob_ids(&self, repo: &Repository) -> Result<HashMap<Oid, Oid>, git2::Error> {
        let mut blob_ids = HashMap::new();
        for tree_id in &self.trees {
            repo.find_tree(*tree_id)?
                .walk(TreeWalkMode::PreOrder, |dir, entry| {
                    if entry.kind() == Some(ObjectType::Blob)
                        && let Ok(name) = entry.name()
                    {
                        let hex = format!("{}{}", dir.replace('/', ""), name);
                        if let (40 | 64, Ok(key)) = (hex.len(), Oid::from_str(&hex)) {
                            // Earlier refs take precedence.
                            blob_ids.entry(key).or_insert(entry.id());
                        }
                    }
                    TreeWalkResult::Ok
                })?;
        }
        Ok(blob_ids)
    }

    /// The revision `key` maps to, if any.
    pub fn lookup(&self, repo: &Repository, key: Oid) -> Option<Oid> {
        let paths = note_paths(key);
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

/// Accumulates notes and writes them as notes commits in a git fast-import
/// stream.
pub struct NotesWriter {
    notes_ref: String,
    /// What the notes map, for the notes commit messages, ex: "old revisions
    /// to new revisions".
    description: String,
    /// The existing notes commit, which our first notes commit builds on.
    /// Subsequent notes commits build on the previous one, which fast-import
    /// does by default.
    from: Option<Oid>,
    wrote_commit: bool,
    pending: Vec<(Oid, String)>,
    /// The time of the most recently added note, which we use for the notes
    /// commits so that they're deterministic.
    time: i64,
}

impl NotesWriter {
    pub fn new(repo: &Repository, notes_ref: &str, description: &str) -> NotesWriter {
        NotesWriter {
            notes_ref: notes_ref.to_string(),
            description: description.to_string(),
            from: repo.refname_to_id(notes_ref).ok(),
            wrote_commit: false,
            pending: vec![],
            time: 0,
        }
    }

    /// Record that `key` maps to the revision with hex id `value`, where `time`
    /// is the commit time of the revision responsible for the note.
    pub fn add(&mut self, key: Oid, value: &str, time: i64) {
        self.pending.push((key, value.to_string()));
        self.time = time;
    }

    pub fn num_pending(&self) -> usize {
        self.pending.len()
    }

    pub fn notes_ref(&self) -> &str {
        &self.notes_ref
    }

    /// Write a notes commit with the pending notes, if any.  This must not be
    /// called in the middle of writing another commit.
    pub fn flush(&mut self, stream: &mut impl Write) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        writeln!(stream, "commit {}", self.notes_ref)?;
        writeln!(stream, "committer mozsearch <> {} +0000", self.time)?;
        let message = format!("Map {} {}\n", self.pending.len(), self.description);
        write!(stream, "data {}\n{}", message.len(), message)?;
        if !self.wrote_commit
            && let Some(from) = self.from
        {
            writeln!(stream, "from {}", from)?;
        }
        for (key, value) in self.pending.drain(..) {
            let contents = note_contents(&value);
            writeln!(stream, "M 100644 inline {}", note_path(key))?;
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
    use std::process::{ChildStdin, Stdio};

    pub fn fast_import(repo: &Repository, write: impl FnOnce(&mut ChildStdin)) {
        let mut child = crate::git_ops::fast_import_git()
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

    /// Write a notes commit with the given notes.
    pub fn write_notes(repo: &Repository, notes_ref: &str, notes: &[(Oid, Oid)]) {
        fast_import(repo, |stream| {
            let mut writer = NotesWriter::new(repo, notes_ref, "test revisions");
            for (key, value) in notes {
                writer.add(*key, &value.to_string(), 0);
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
        let dir = std::env::temp_dir().join(format!("git-notes-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let (main_ref, other_ref) = ("refs/notes/test-main", "refs/notes/test-other");
        let key = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let value = |n: u8| Oid::from_bytes(&[n + 100; 20]).unwrap();
        assert_eq!(
            notes_ref_for_branch(&repo, "refs/notes/test-", "refs/heads/main"),
            main_ref
        );
        assert!(NotesReader::open(&repo, [main_ref, other_ref]).is_empty());

        // Two batches in one stream, then another run which builds on them.
        fast_import(&repo, |stream| {
            let mut writer = NotesWriter::new(&repo, main_ref, "test revisions");
            writer.add(key(1), &value(1).to_string(), 1000);
            writer.add(key(2), &value(2).to_string(), 1001);
            writer.flush(stream).unwrap();
            writer.add(key(3), &value(3).to_string(), 1002);
            writer.flush(stream).unwrap();
            // Flushing without pending notes doesn't write a commit.
            writer.flush(stream).unwrap();
        });
        fast_import(&repo, |stream| {
            let mut writer = NotesWriter::new(&repo, main_ref, "test revisions");
            writer.add(key(4), &value(4).to_string(), 1003);
            writer.flush(stream).unwrap();
        });
        // A ref consulted after the first one, whose note for key(1) is
        // shadowed.
        fast_import(&repo, |stream| {
            let mut writer = NotesWriter::new(&repo, other_ref, "test revisions");
            writer.add(key(1), &value(9).to_string(), 1000);
            writer.add(key(5), &value(5).to_string(), 1000);
            writer.flush(stream).unwrap();
        });

        let reader = NotesReader::open(&repo, [main_ref, other_ref]);
        for n in 1..=5 {
            assert_eq!(reader.lookup(&repo, key(n)), Some(value(n)));
        }
        assert_eq!(reader.lookup(&repo, key(6)), None);
        let blob_ids = reader.note_blob_ids(&repo).unwrap();
        assert_eq!(blob_ids.len(), 5);
        for n in 1..=5 {
            assert_eq!(blob_ids[&key(n)], note_blob_id(value(n)));
        }
        let notes_commit = repo
            .find_reference(main_ref)
            .unwrap()
            .peel_to_commit()
            .unwrap();
        assert_eq!(notes_commit.parent_count(), 1);
        assert_eq!(notes_commit.time().seconds(), 1003);
        assert_eq!(notes_commit.message().unwrap(), "Map 1 test revisions\n");

        // git itself can read the notes, even though the annotated objects
        // don't exist.
        let output = Command::new("git")
            .args(["notes", "--ref", main_ref, "show", &key(2).to_string()])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            value(2).to_string()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
