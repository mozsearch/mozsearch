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
//! See `git_notes` for how the notes are stored.  Notes are written in batches
//! of `NOTES_BATCH_SIZE` revisions, and only for revisions whose commits were
//! completely written.  Since the history commits are a deterministic function
//! of their inputs, a crash just means the revisions since the last batch get
//! processed again with identical results, and a commit left incomplete by a
//! crash (git fast-import commits whatever it has at the end of its input) is
//! replaced rather than being mistaken for a processed revision.

use std::env;

use git2::{Oid, Repository};

use crate::git_notes::{NotesReader, NotesWriter, notes_ref_for_branch};

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

    fn all(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.write.as_str()).chain(self.read.iter().map(String::as_str))
    }
}

/// `refs/notes/mozsearch-source-mapping-<BRANCH>`; see `notes_ref_for_branch`.
pub fn default_notes_ref(repo: &Repository, blame_ref: &str) -> String {
    notes_ref_for_branch(repo, NOTES_REF_PREFIX, blame_ref)
}

/// Read access to a history repo's notes as of when they were opened.
#[derive(Clone)]
pub struct SourceMapping(NotesReader);

impl SourceMapping {
    pub fn open(repo: &Repository, refs: &NotesRefs) -> SourceMapping {
        SourceMapping(NotesReader::open(repo, refs.all()))
    }

    /// True if none of the notes refs exist.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The history commit id recorded for `source_rev`, if any.
    pub fn lookup(&self, repo: &Repository, source_rev: Oid) -> Option<Oid> {
        self.0.lookup(repo, source_rev)
    }
}

/// A writer for the notes ref of `refs`.
pub fn notes_writer(repo: &Repository, refs: &NotesRefs) -> NotesWriter {
    NotesWriter::new(repo, &refs.write, "source revisions to history commits")
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
            refs.all().collect::<Vec<_>>().join(" or ")
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_notes::test_support::write_notes;

    #[test]
    fn test_source_mapping() {
        let dir = std::env::temp_dir().join(format!("hb-source-mapping-{}", std::process::id()));
        let repo = Repository::init(&dir).unwrap();
        let refs = NotesRefs {
            write: default_notes_ref(&repo, "refs/heads/try"),
            read: vec![default_notes_ref(&repo, "refs/heads/main")],
        };
        assert_eq!(refs.write, "refs/notes/mozsearch-source-mapping-try");
        let rev = |n: u8| Oid::from_bytes(&[n; 20]).unwrap();
        let commit = |n: u8| Oid::from_bytes(&[n + 100; 20]).unwrap();
        assert!(SourceMapping::open(&repo, &refs).is_empty());

        // The notes ref we write to is consulted before the others.
        write_notes(
            &repo,
            &refs.read[0],
            &[(rev(1), commit(9)), (rev(2), commit(2))],
        );
        write_notes(
            &repo,
            &refs.write,
            &[(rev(1), commit(1)), (rev(3), commit(3))],
        );
        let mapping = SourceMapping::open(&repo, &refs);
        for n in 1..=3 {
            assert_eq!(mapping.lookup(&repo, rev(n)), Some(commit(n)));
        }
        assert_eq!(mapping.lookup(&repo, rev(4)), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
