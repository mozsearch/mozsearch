//! The lines a revision changes, for slicing it (see `dataflow::slice`): a
//! commit's changes from its first parent, or those of a range (`a..b`), in
//! the coordinates of another revision's versions of the files (the one whose
//! analysis slicing uses: the indexed revision, or a try push's head), with
//! the commits making them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use git2::{Commit, Delta, DiffOptions, Patch, Repository, Sort};

use crate::dataflow::slice::{ChangedLines, FileChange};
use crate::hyperblame::token_blame::{TreeHistory, blame_tokens};

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

/// The lines (1-based) of `old` which `new` removes (or changes).
pub fn removed_lines(old: &[u8], new: &[u8]) -> BTreeSet<u32> {
    let mut removed = BTreeSet::new();
    let mut options = DiffOptions::new();
    options.context_lines(0);
    let Ok(patch) = Patch::from_buffers(old, None, new, None, Some(&mut options)) else {
        return removed;
    };
    for h in 0..patch.num_hunks() {
        let Ok((_, count)) = patch.hunk(h) else {
            continue;
        };
        for l in 0..count {
            if let Ok(line) = patch.line_in_hunk(h, l)
                && line.origin() == '-'
            {
                removed.extend(line.old_lineno());
            }
        }
    }
    removed
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

/// Which of a range's `commits` (their ids, oldest first) made each line of
/// `path` at `to` (line `l` at `l - 1`; `source`, its contents), by the
/// tree's token-centric history (see `hyperblame::token_blame`): the newest of
/// them which introduced one of the line's tokens, or none (ex: a line the
/// range only moved, whose tokens are older).  None if the history doesn't
/// have `to` or the file.
fn history_line_commits(
    history: &TreeHistory,
    to: &Commit,
    path: &str,
    source: &[u8],
    commits: &[String],
) -> Option<Vec<Option<usize>>> {
    let timeline_commit = history.timeline_commit(to.id())?;
    let file_history = history.file_history(&timeline_commit, path).ok()??;
    let blame = blame_tokens(std::str::from_utf8(source).ok()?, &file_history).ok()?;
    let index: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| (commit.as_str(), i))
        .collect();
    let mut lines: Vec<Option<usize>> = vec![None; blame.line_starts.len()];
    for token in &blame.tokens {
        if let Some(&i) = index.get(token.data.introduced.source_rev.as_ref())
            && let Some(line) = lines.get_mut(token.line)
        {
            *line = Some(line.map_or(i, |j| j.max(i)));
        }
    }
    Some(lines)
}

/// The lines a revision changes (`<rev>`: a commit's changes from its first
/// parent; `<a>..<b>`: those from `a` to `b`), in the coordinates of
/// `context`'s versions of the files, with the commits making them: a
/// commit's, its; a range's, by the tree's token-centric history (see
/// `history_line_commits`), which slicing a range needs (a tree without it has
/// none).  Lines changed again since aren't there, nor are files `context`
/// doesn't have (they're in the notes).  And the lines it removes, in the old
/// side's coordinates (all of a removed file's).
pub fn rev_changed_lines(
    repo: &Repository,
    history: Option<&TreeHistory>,
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
    let mut removed_by_file: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for delta in diff.deltas() {
        if delta.status() == Delta::Deleted {
            if let Some(path) = delta.old_file().path().and_then(|p| p.to_str())
                && let Ok(old) = repo.find_blob(delta.old_file().id())
            {
                removed_by_file.insert(path.to_string(), (1..=line_count(old.content())).collect());
            }
            continue;
        }
        let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) else {
            continue;
        };
        let Ok(new) = repo.find_blob(delta.new_file().id()) else {
            continue;
        };
        let old = match delta.status() {
            Delta::Added => None,
            _ => repo.find_blob(delta.old_file().id()).ok(),
        };
        let (added, touched) = changed_lines(old.as_ref().map(|b| b.content()), new.content());
        if let Some(old) = &old {
            let removed = removed_lines(old.content(), new.content());
            if !removed.is_empty() {
                let old_path = delta
                    .old_file()
                    .path()
                    .and_then(|p| p.to_str())
                    .unwrap_or(path);
                removed_by_file.insert(old_path.to_string(), removed.into_iter().collect());
            }
        }
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
        } else if commits.len() > 1
            && let Some(history) = history
            && let Some(by_line) = history_line_commits(history, &to, path, new.content(), &commits)
        {
            for line in &added {
                if let Some(Some(index)) = by_line.get(*line as usize - 1) {
                    line_commits.insert(*line, commits[*index].clone());
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
        from: from.as_ref().map(|commit| commit.id().to_string()),
        removed_lines: removed_by_file,
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
    fn test_rev_changed_lines_commits() {
        let dir = std::env::temp_dir().join(format!("rev-lines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repository::init(&dir).unwrap();
        let sig = git2::Signature::new("t", "t@t", &git2::Time::new(0, 0)).unwrap();
        let commit = |text: &str, parents: &[git2::Oid]| -> git2::Oid {
            let mut builder = repo.treebuilder(None).unwrap();
            builder
                .insert("a", repo.blob(text.as_bytes()).unwrap(), 0o100644)
                .unwrap();
            let tree = repo.find_tree(builder.write().unwrap()).unwrap();
            let parents: Vec<Commit> = parents
                .iter()
                .map(|id| repo.find_commit(*id).unwrap())
                .collect();
            let parents: Vec<&Commit> = parents.iter().collect();
            repo.commit(None, &sig, &sig, "c", &tree, &parents).unwrap()
        };
        let c0 = commit("1\n2\n3\n", &[]);
        let c1 = commit("1\nX\n3\n", &[c0]);
        let c2 = commit("0\n1\nX\n3\n", &[c1]);
        // A commit's lines are its.
        let changes = rev_changed_lines(&repo, None, &c2.to_string(), &c2.to_string()).unwrap();
        let a = &changes.files["a"];
        assert_eq!(a.added.iter().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(a.commits[&1], c2.to_string());
        // A range's are by the history, which this tree doesn't have.
        let range = format!("{}..{}", c0, c2);
        let changes = rev_changed_lines(&repo, None, &c2.to_string(), &range).unwrap();
        let a = &changes.files["a"];
        assert_eq!(a.added.iter().copied().collect::<Vec<_>>(), vec![1, 3]);
        assert!(a.commits.is_empty());
        assert_eq!(changes.commits, vec![c1.to_string(), c2.to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(
            removed_lines(old, new).into_iter().collect::<Vec<_>>(),
            vec![2, 5]
        );
    }
}
