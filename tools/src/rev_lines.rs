//! The lines a revision changes, for slicing it (see `dataflow::slice`): a
//! commit's changes from its first parent, or those of a range (`a..b`), in
//! the coordinates of another revision's versions of the files (the one whose
//! analysis slicing uses: the indexed revision, or a try push's head), with
//! the commits making them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use git2::{BlameOptions, Commit, Delta, DiffOptions, Patch, Repository, Sort};

use crate::dataflow::slice::{ChangedLines, FileChange};

/// The lines (1-based) of `new` which differ from `old` (all of them if
/// there's no `old`): the ones it adds or changes, and with those, the ones
/// before which it removes lines.
pub fn changed_lines(old: Option<&[u8]>, new: &[u8]) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let Some(old) = old else {
        let all: BTreeSet<u32> = (1..=line_count(new)).collect();
        return (all.clone(), all);
    };
    let mut added = BTreeSet::new();
    let mut touched = BTreeSet::new();
    let mut options = DiffOptions::new();
    options.context_lines(0);
    let Ok(patch) = Patch::from_buffers(old, None, new, None, Some(&mut options)) else {
        return (added, touched);
    };
    for h in 0..patch.num_hunks() {
        let Ok((hunk, count)) = patch.hunk(h) else {
            continue;
        };
        if hunk.new_lines() == 0 {
            // (A removal, before the line after it.)
            touched.insert(hunk.new_start().max(1) + 1);
            touched.insert(hunk.new_start().max(1));
        }
        for l in 0..count {
            if let Ok(line) = patch.line_in_hunk(h, l)
                && line.origin() == '+'
                && let Some(lineno) = line.new_lineno()
            {
                added.insert(lineno);
                touched.insert(lineno);
            }
        }
    }
    (added, touched)
}

/// How many lines a file's contents have.
fn line_count(text: &[u8]) -> u32 {
    let newlines = text.iter().filter(|b| **b == b'\n').count();
    (newlines + usize::from(!text.is_empty() && !text.ends_with(b"\n"))) as u32
}

/// For each line of `old` (line `l` at `l - 1`), its line in `new`, if it's
/// there unchanged.
pub fn line_map(old: &[u8], new: &[u8]) -> Vec<Option<u32>> {
    let count = line_count(old) as usize;
    let mut options = DiffOptions::new();
    options.context_lines(0);
    let Ok(patch) = Patch::from_buffers(old, None, new, None, Some(&mut options)) else {
        return vec![None; count];
    };
    let (mut removed, mut added) = (BTreeSet::new(), BTreeSet::new());
    for h in 0..patch.num_hunks() {
        let Ok((_, lines)) = patch.hunk(h) else {
            continue;
        };
        for l in 0..lines {
            let Ok(line) = patch.line_in_hunk(h, l) else {
                continue;
            };
            match line.origin() {
                '-' => removed.extend(line.old_lineno()),
                '+' => added.extend(line.new_lineno()),
                _ => {}
            }
        }
    }
    // (The lines neither side changes are in the same order on both.)
    let mut map = Vec::with_capacity(count);
    let mut next = 1;
    for line in 1..=count as u32 {
        if removed.contains(&line) {
            map.push(None);
            continue;
        }
        while added.contains(&next) {
            next += 1;
        }
        map.push(Some(next));
        next += 1;
    }
    map
}

fn blob_at(repo: &Repository, commit: &Commit, path: &str) -> Option<Vec<u8>> {
    let entry = commit.tree().ok()?.get_path(Path::new(path)).ok()?;
    Some(repo.find_blob(entry.id()).ok()?.content().to_vec())
}

/// The lines a revision changes (`<rev>`: a commit's changes from its first
/// parent; `<a>..<b>`: those from `a` to `b`), in the coordinates of
/// `context`'s versions of the files, with the commits making them (the
/// range's, by blame).  Lines changed again since aren't there, nor are files
/// `context` doesn't have (they're in the notes).
pub fn rev_changed_lines(
    repo: &Repository,
    context: &str,
    spec: &str,
) -> Result<ChangedLines, String> {
    let commit = |rev: &str| {
        repo.revparse_single(rev)
            .and_then(|object| object.peel_to_commit())
            .map_err(|err| format!("{}: {}", rev, err.message()))
    };
    let (from, to) = match spec.split_once("..") {
        Some((a, b)) => (Some(commit(a)?), commit(b)?),
        None => {
            let to = commit(spec)?;
            (to.parent(0).ok(), to)
        }
    };
    let context = commit(context)?;
    let git_error = |err: git2::Error| err.message().to_string();

    // The range's commits, oldest first.
    let mut walk = repo.revwalk().map_err(git_error)?;
    walk.push(to.id()).map_err(git_error)?;
    if let Some(from) = &from {
        walk.hide(from.id()).map_err(git_error)?;
    }
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)
        .map_err(git_error)?;
    let commits: Vec<String> = walk
        .filter_map(|oid| oid.ok())
        .map(|oid| oid.to_string())
        .collect();

    let old_tree = match &from {
        Some(from) => Some(from.tree().map_err(git_error)?),
        None => None,
    };
    let new_tree = to.tree().map_err(git_error)?;
    let diff = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)
        .map_err(git_error)?;
    let short = |commit: &Commit| commit.id().to_string()[..12].to_string();
    let mut files = BTreeMap::new();
    let mut notes = vec![];
    for delta in diff.deltas() {
        let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) else {
            continue;
        };
        if delta.status() == Delta::Deleted {
            notes.push(format!("{} is removed", path));
            continue;
        }
        let Ok(new) = repo.find_blob(delta.new_file().id()) else {
            continue;
        };
        let old = match delta.status() {
            Delta::Added => None,
            _ => repo.find_blob(delta.old_file().id()).ok(),
        };
        let (added, touched) = changed_lines(old.as_ref().map(|b| b.content()), new.content());
        // (In the context's versions of the files.)
        let map = if to.id() == context.id() {
            None
        } else {
            match blob_at(repo, &context, path) {
                Some(text) => Some(line_map(new.content(), &text)),
                None => {
                    notes.push(format!("{} isn't at {}", path, short(&context)));
                    continue;
                }
            }
        };
        let at = |line: u32| -> Option<u32> {
            match &map {
                None => Some(line),
                Some(map) => map.get(line as usize - 1).copied().flatten(),
            }
        };
        let mut line_commits: BTreeMap<u32, String> = BTreeMap::new();
        if let [only] = commits.as_slice() {
            line_commits.extend(added.iter().map(|line| (*line, only.clone())));
        } else if commits.len() > 1 {
            let mut options = BlameOptions::new();
            options.newest_commit(to.id());
            if let Some(from) = &from {
                options.oldest_commit(from.id());
            }
            if let Ok(blame) = repo.blame_file(Path::new(path), Some(&mut options)) {
                for line in &added {
                    if let Some(hunk) = blame.get_line(*line as usize) {
                        let id = hunk.final_commit_id().to_string();
                        if commits.contains(&id) {
                            line_commits.insert(*line, id);
                        }
                    }
                }
            }
        }
        let change = FileChange {
            added: added.iter().filter_map(|line| at(*line)).collect(),
            removed: touched
                .difference(&added)
                .filter_map(|line| at(*line))
                .collect(),
            commits: line_commits
                .into_iter()
                .filter_map(|(line, commit)| Some((at(line)?, commit)))
                .collect(),
        };
        if change.added.is_empty() && change.removed.is_empty() {
            notes.push(format!(
                "{}'s changes are changed again by {}",
                path,
                short(&context)
            ));
            continue;
        }
        files.insert(path.to_string(), change);
    }
    Ok(ChangedLines {
        commits,
        files,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_changed_lines() {
        let old = b"a\nb\nc\nd\n";
        let new = b"a\nB\nc\nx\ny\n";
        let (added, touched) = changed_lines(Some(old), new);
        assert_eq!(added.into_iter().collect::<Vec<_>>(), vec![2, 4, 5]);
        assert_eq!(touched.into_iter().collect::<Vec<_>>(), vec![2, 4, 5]);
        // (A removal touches the lines around it.)
        let (added, touched) = changed_lines(Some(b"a\nb\nc\n"), b"a\nc\n");
        assert!(added.is_empty());
        assert_eq!(touched.into_iter().collect::<Vec<_>>(), vec![1, 2]);
        let (added, _) = changed_lines(None, b"x\ny");
        assert_eq!(added.into_iter().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn test_line_map() {
        // (b changed, a line added before d, e removed.)
        let old = b"a\nb\nc\nd\ne\nf\n";
        let new = b"a\nB\nc\nnew\nd\nf\n";
        assert_eq!(
            line_map(old, new),
            vec![Some(1), None, Some(3), Some(5), None, Some(6)]
        );
        assert_eq!(line_map(b"x\ny", b"x\ny"), vec![Some(1), Some(2)]);
        assert_eq!(line_map(b"x\n", b""), vec![None]);
    }
}
