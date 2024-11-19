/*
This file combines the synchronous gold-standards of
https://crates.io/crates/globset and https://crates.io/crates/walkdir (both
from the ripgrep author) to perform filtered tree enumeration and wraps them
with a call to tokio::block_in_place to avoid gumming up the scheduling works.

The only current desired consumer for this file is
the `test_check_insta`mechanism, but it could also make sense to be used for
other purposes.  It's also possible the crates ecosystem will gain a popular
crate we can just use directly.

Note that although the web UI has a mechanism to search known files, that
currently is based on shelling out to grep to filter `repo-files` and
`objdir-files`, not actually perform a filesystem traversal and we want to keep
that as filtering pre-canned data, not as a filesystem traversal.
*/

use std::path::Path;

use globset::Glob;
use tokio::task;
use walkdir::WalkDir;

/// Given a path to a root dir to traverse and a glob pattern, block in place
/// and return a sorted list of all files matching the glob relative to the root
/// dir.  Return value looks like ("relative/path/", "file.ext").
///
/// Note: This currently requires that there be no top-level files that match
/// the glob because of laziness about construction of the relative path.  We
/// will panic when this laziness eventually becomes a problem.
///
/// Everything operates in terms of strings because callers currently like to
/// use `format!` to build paths and we don't have to deal with adversarial
/// paths here.
pub fn block_in_place_glob_tree(root: &str, glob: &str) -> Vec<(String, String)> {
    task::block_in_place(|| {
        let mut paths = vec![];

        let glob = Glob::new(glob).unwrap().compile_matcher();

        let root_path = Path::new(root);
        for entry in WalkDir::new(root_path) {
            let entry = entry.unwrap();
            if glob.is_match(entry.path()) && entry.file_type().is_file() {
                let rel_path = entry.path().strip_prefix(root_path).unwrap();
                paths.push((
                    // We want a trailing slash for simplified string formatting.
                    format!("{}/", rel_path.parent().unwrap().to_str().unwrap()),
                    rel_path.file_name().unwrap().to_str().unwrap().to_string(),
                ));
            }
        }

        paths.sort();

        paths
    })
}
