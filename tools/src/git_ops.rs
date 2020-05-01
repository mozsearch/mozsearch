use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::{
    Commit, Diff, DiffDelta, DiffFindOptions, DiffOptions, Error, Oid, Patch, Repository, TreeEntry,
};

use crate::config::GitData;

// Helpers to do things with git2

fn latin1_to_string(bytes: Vec<u8>) -> String {
    bytes.iter().map(|&c| c as char).collect()
}

pub fn decode_bytes(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => latin1_to_string(bytes),
    }
}

pub fn read_blob_entry(repo: &Repository, entry: &TreeEntry) -> String {
    let blob_obj = entry.to_object(repo).unwrap();
    let blob = blob_obj.as_blob().unwrap();
    let mut content = Vec::new();
    content.extend(blob.content());
    decode_bytes(content)
}

pub fn get_blame_lines(
    git_data: Option<&GitData>,
    blame_commit: &Option<Commit>,
    path: &str,
) -> Option<Vec<String>> {
    match (git_data, blame_commit) {
        (
            Some(&GitData {
                blame_repo: Some(ref blame_repo),
                ..
            }),
            &Some(ref blame_commit),
        ) => {
            let blame_tree = blame_commit.tree().ok()?;

            match blame_tree.get_path(Path::new(path)) {
                Ok(blame_entry) => {
                    let blame_data = read_blob_entry(blame_repo, &blame_entry);
                    Some(blame_data.lines().map(str::to_string).collect::<Vec<_>>())
                }
                Err(_) => None,
            }
        }
        _ => None,
    }
}

#[derive(Debug)]
pub struct LineMap {
    // Stores pairs (a, b) sorted so that `a` is always increasing.
    // `a` is a line number, and `b` is the amount of adjustment needed
    // for line numbers from `a` onwards. Subsequent entries effectively
    // override previous ones, rather than being cumulative.
    adjustments: Vec<(u32, i32)>,
}

impl LineMap {
    pub fn map_line(&self, lineno: u32) -> u32 {
        // Find the entry (a, b) in `adjustments` with the largest
        // a <= lineno. This will give us the adjustment needed for
        // lineno
        let mut line_shift = 0;
        for (ref start, ref shift) in &self.adjustments {
            if *start <= lineno {
                line_shift = *shift;
            } else {
                break;
            }
        }
        assert!(lineno as i32 + line_shift >= 0);
        (lineno as i32 + line_shift) as u32
    }
}

/// Given two blob ids, corresponding to the "old" and "new" version of
/// a file, this function produces a LineMap that can be used to map line
/// numbers in the "new" file to approximately equivalent line numbers in
/// "old" file. It does this by walking through the diff hunks between the
/// two files and tracking a cumulative "shift" that needs to be applied
/// to find the equivalent line in the old file.
/// This is largely a reimplementation of the core bit of the implementation at
/// https://chromium.googlesource.com/chromium/tools/depot_tools.git/+/master/git_hyper_blame.py
fn compute_line_map(repo: &Repository, new_oid: Oid, old_oid: Oid) -> Result<LineMap, Error> {
    let new_blob = repo.find_blob(new_oid)?;
    let old_blob = repo.find_blob(old_oid)?;
    if old_blob.content().len() == 0 {
        // If the old file had no lines, we can't really generate a line map to it, so abort
        return Err(Error::from_str("Can't generate a linemap to an empty file"));
    }

    let patch = Patch::from_blobs(
        &old_blob,
        None,
        &new_blob,
        None,
        Some(DiffOptions::new().context_lines(0)),
    )?;

    let mut adjustments = Vec::new();
    let mut shift: i32 = 0;
    for hunk_index in 0..patch.num_hunks() {
        let (hunk, _) = patch.hunk(hunk_index)?;
        if hunk.new_lines() == hunk.old_lines() {
            // The hunk didn't add or remove lines, so the amount to
            // shift remains the same as before
            continue;
        }
        if hunk.new_lines() > hunk.old_lines() {
            // Lines were added in the hunk, let's map all the new
            // lines to the last line of the hunk in the old file.
            let extra_lines = hunk.new_lines() - hunk.old_lines();
            for i in 0..extra_lines {
                shift -= 1;
                adjustments.push((hunk.new_start() + hunk.old_lines() + i, shift))
            }
        }
        if hunk.new_lines() < hunk.old_lines() {
            // Lines were removed in the hunk, so we have a discontinuity
            // where lines after the hunk in the new file have a bigger
            // shift than lines before/inside the hunk.
            shift = shift + hunk.old_lines() as i32 - hunk.new_lines() as i32;
            adjustments.push((hunk.new_start() + hunk.new_lines(), shift));
        }
    }
    Ok(LineMap { adjustments })
}

/// Represents the inputs to a map_to_previous_version call, which maps a file
/// in a given revision to its ancestor version.
#[derive(Hash, PartialEq, Eq)]
pub struct PrevBlameInput {
    /// The revision in which the file was modified.
    rev: String,
    /// The path to the file we are interested in mapping to its older version.
    path: PathBuf,
}

/// Represents the result of a map_to_previous_version call, which maps a file
/// in a given revision to its ancestor version.
pub struct PreviousVersionMap {
    /// The parent revision.
    parent_rev: Oid,
    /// The path of the file in the parent revision.
    old_path: PathBuf,
    /// An object that allows mapping lines from the new version of the file to
    /// to lines in the old version fo the file.
    line_map: LineMap,
}

/// Caches the previous blame information.
pub type PrevBlameCache = HashMap<PrevBlameInput, PreviousVersionMap>;

#[derive(Clone, Debug)]
pub struct FileDiffEntry {
    pub old_path: Option<PathBuf>,
    pub old_id: Oid,
    pub new_id: Oid,
}

impl FileDiffEntry {
    fn from(delta: &DiffDelta) -> Self {
        FileDiffEntry {
            old_path: delta.old_file().path().map(|p| p.to_path_buf()),
            old_id: delta.old_file().id(),
            new_id: delta.new_file().id(),
        }
    }
}

/// Caches the results of a tree diff. When we do a tree diff, we compare a
/// revision to its parent revision, and get a list of all the files that changed.
/// The output-file binary processes one file at a time, so if we repeat this
/// whole tree diff for each affected file that's a lot of wasted work. Instead,
/// the first time we do a tree diff on a particular revision, we store all the
/// resulting data here, keyed by (revision, path). The value in the cache gives
/// the blob ids for the old and new versions of the file, as well as the old
/// pathname if there was one.
pub type TreeDiffCache = HashMap<(String, PathBuf), FileDiffEntry>;

fn diff_trees<'a>(
    git_data: &'a GitData,
    commit: &Commit,
    parent_commit: &Commit,
) -> Result<Diff<'a>, Error> {
    let older_tree = parent_commit.tree()?;
    let newer_tree = commit.tree()?;

    let mut diff = git_data
        .repo
        .diff_tree_to_tree(Some(&older_tree), Some(&newer_tree), None)?;
    diff.find_similar(Some(DiffFindOptions::new().renames(true)))?;
    Ok(diff)
}

/// Given a GitData, commit revision, and target file, this function returns information
/// to track lines in that file backwards by one commit. Specifically, it returns
/// the parent commit, the corresponding path of the equivalent source file in that commit
/// (in case it was moved or copied), and a LineMap that allows mapping individual lines
/// backwards from the target file to the source file. Note that this only works with
/// non-merge revisions (i.e. there has to be a unique parent).
/// If the target_file was not modified in the indicated commit, this function will return
/// an error.
fn map_to_previous_version(
    git_data: &GitData,
    rev: &str,
    target_file: &Path,
    cache: Option<&mut TreeDiffCache>,
) -> Result<PreviousVersionMap, Error> {
    let commit_obj = git_data.repo.revparse_single(rev)?;
    let commit = commit_obj
        .as_commit()
        .ok_or_else(|| Error::from_str("Commit_obj error"))?;
    if commit.parent_ids().len() != 1 {
        // If the commit didn't have a unique parent, let's abort
        return Err(Error::from_str(
            "No unique parent, don't know where to look for prev blame",
        ));
    }

    let parent_commit = commit.parent(0)?;

    let delta = {
        if let Some(cache) = cache {
            let key = (String::from(rev), PathBuf::from(target_file));
            if !cache.contains_key(&key) {
                let diff = diff_trees(git_data, &commit, &parent_commit)?;
                for delta in diff.deltas() {
                    if let Some(file) = delta.new_file().path() {
                        cache.insert(
                            (String::from(rev), PathBuf::from(file)),
                            FileDiffEntry::from(&delta),
                        );
                    }
                }
            }

            cache
                .get(&key)
                .ok_or_else(|| {
                    Error::from_str(&format!(
                        "No delta for target {:?} in rev {}",
                        target_file, rev
                    ))
                })?
                .clone()
        } else {
            let diff = diff_trees(git_data, &commit, &parent_commit)?;
            let delta = diff
                .deltas()
                .find(|delta| delta.new_file().path() == Some(target_file))
                .ok_or_else(|| {
                    Error::from_str(&format!(
                        "No delta for target {:?} in rev {}",
                        target_file, rev
                    ))
                })?;
            FileDiffEntry::from(&delta)
        }
    };

    if delta.old_id.is_zero() {
        return Err(Error::from_str(&format!(
            "Target {:?} added in rev {} with no ancestor",
            target_file, rev
        )));
    }

    Ok(PreviousVersionMap {
        parent_rev: parent_commit.id(),
        old_path: delta
            .old_path
            .ok_or_else(|| Error::from_str("Couldn't get old path"))?,
        line_map: compute_line_map(&git_data.repo, delta.new_id, delta.old_id)?,
    })
}

/// Given a GitData, commit revision, target file, and line number in that file, this
/// function tries to find the equivalent line in the parent revision, and provide the
/// blame information for that line, along with the filename at that parent revision.
/// The filename is necessary since the blame information may contain the special string
/// "%" to indicate "the current file".
pub fn find_prev_blame(
    git_data: &GitData,
    rev: &str,
    target_file: &Path,
    lineno: u32,
    cache: &mut PrevBlameCache,
    diff_cache: Option<&mut TreeDiffCache>,
) -> Result<(String, PathBuf), Error> {
    let entry = cache.entry(PrevBlameInput {
        rev: String::from(rev),
        path: PathBuf::from(target_file),
    });
    // Can't use or_insert_with because of the try-wrapper around map_to_previous_version
    let PreviousVersionMap {
        parent_rev: parent_commit,
        old_path,
        line_map,
    } = match entry {
        Entry::Occupied(hit) => hit.into_mut(),
        Entry::Vacant(miss) => miss.insert(map_to_previous_version(
            git_data,
            rev,
            target_file,
            diff_cache,
        )?),
    };

    let old_lineno = line_map.map_line(lineno);
    let parent_blame_oid = git_data.blame_map.get(&parent_commit).ok_or_else(|| {
        Error::from_str(&format!("Couldn't get blame rev for {:?}", parent_commit))
    })?;
    let parent_blame = git_data
        .blame_repo
        .as_ref()
        .unwrap()
        .find_commit(*parent_blame_oid)?;

    match get_blame_lines(
        Some(git_data),
        &Some(parent_blame),
        target_file.to_str().unwrap(),
    ) {
        Some(blame_lines) => {
            // line numbers are 1-based, array indexing is 0-based. But we might get old_lineno
            // as 0 if the hunk that's getting blame-skipped was just adding a bunch of lines to
            // the top of a file, as the "previous line" would be the last line of the "old" side
            // of the hunk, which would just point to line 0 (start of file). This is fine, but
            // we need to avoid looking up index -1 in the array.
            let old_lineno_ix = i32::max(0, (old_lineno as i32) - 1);
            Ok((
                blame_lines[old_lineno_ix as usize].clone(),
                old_path.to_path_buf(),
            ))
        }
        None => Err(Error::from_str(
            "Unable to get blame lines for parent commit",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{index_blame, BlameIgnoreList, Mailmap};
    use std::env;

    fn build_git_data() -> Option<GitData> {
        let repo = Repository::open(env::var("GIT_ROOT").ok()?).unwrap();
        let blame_repo = env::var("BLAME_ROOT")
            .ok()
            .map(|s| Repository::open(s).unwrap());
        let (blame_map, hg_map) = match &blame_repo {
            Some(ref blame_repo) => index_blame(&blame_repo, None),
            None => (HashMap::new(), HashMap::new()),
        };
        let mailmap = Mailmap::load(&repo);
        let blame_ignore = BlameIgnoreList::load(&repo);
        Some(GitData {
            repo,
            blame_repo,
            blame_map,
            hg_map,
            mailmap,
            blame_ignore,
        })
    }

    // This not really a test but a debugging tool to run some part of the
    // code above in relative isolation. Run with e.g.
    //  GIT_ROOT=$HOME/webrender GIT_REV=d477ecc5978bb353c1d6e93a3387e9a4eb197572 TEST_PATH=.taskcluster.yml cargo test --release print_prev_data -- --nocapture
    #[test]
    fn print_prev_data() {
        let git_data = match build_git_data() {
            Some(x) => x,
            None => return, // prevent cargo test from panicking if run without the env args
        };
        let PreviousVersionMap {
            parent_rev: parent,
            old_path,
            line_map,
        } = map_to_previous_version(
            &git_data,
            &env::var("GIT_REV").unwrap_or("HEAD".to_string()),
            Path::new(&env::var("TEST_PATH").unwrap()),
            None,
        )
        .unwrap();
        println!("parent commit: {:?}", parent);
        println!("path in parent commit: {:?}", old_path);
        println!("line mapping: {:?}", line_map);
    }

    // This not really a test but a debugging tool to run some part of the
    // code above in relative isolation. Run with e.g.
    //  GIT_ROOT=$HOME/webrender BLAME_ROOT=$HOME/wr-blame GIT_REV=d477ecc5978bb353c1d6e93a3387e9a4eb197572 TEST_PATH=.taskcluster.yml TEST_LINE=160 cargo test --release print_prev_blame -- --nocapture
    #[test]
    fn print_prev_blame() {
        let git_data = match build_git_data() {
            Some(x) => x,
            None => return, // prevent cargo test from panicking if run without the env args
        };
        let mut cache = PrevBlameCache::new();
        let (blame_data, old_path) = find_prev_blame(
            &git_data,
            &env::var("GIT_REV").unwrap_or("HEAD".to_string()),
            Path::new(&env::var("TEST_PATH").unwrap()),
            env::var("TEST_LINE").unwrap().parse().unwrap(),
            &mut cache,
            None,
        )
        .unwrap();
        println!("prev blame data: {:?}", blame_data);
        println!("path in parent commit: {:?}", old_path);
    }
}
