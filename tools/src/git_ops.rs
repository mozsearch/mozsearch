use git2::{Commit, Object, Repository, TreeEntry};
use itertools::Itertools;
use serde::Serialize;
use std::{path::Path, str::FromStr};

use crate::file_format::{
    code_coverage_report,
    config::GitData,
    coverage::{interpolate_coverage, InterpolatedCoverage},
};

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

pub fn read_blob_object(object: &Object) -> String {
    let blob = object.as_blob().unwrap();
    let mut content = Vec::new();
    content.extend(blob.content());
    decode_bytes(content)
}

pub fn read_blob_entry(repo: &Repository, entry: &TreeEntry) -> String {
    let blob_obj = entry.to_object(repo).unwrap();
    read_blob_object(&blob_obj)
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
            Some(blame_commit),
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

pub fn get_coverage(
    git_data: Option<&GitData>,
    coverage_commit: Option<&Commit>,
    path: impl AsRef<Path>,
) -> Option<Vec<InterpolatedCoverage>> {
    git_data
        .and_then(|git_data| git_data.coverage_repo.as_ref())
        .zip(coverage_commit)
        .and_then(|(coverage_repo, coverage_commit)| {
            let coverage_tree = coverage_commit.tree().ok()?;
            coverage_tree
                .get_path(path.as_ref())
                .ok()
                .map(|coverage_entry| {
                    let coverage_data = read_blob_entry(coverage_repo, &coverage_entry);
                    let raw = coverage_data.lines().map(FromStr::from_str).map(Result::ok);
                    interpolate_coverage(raw)
                })
        })
}

/// Returns the coverage summary for the given path, if available.
/// Note: coverage_rev must be a reference in the coverage repository, not in the main repository.
pub fn coverage_summary(
    git_data: Option<&GitData>,
    coverage_rev: &str,
    path: impl AsRef<Path>,
) -> Option<code_coverage_report::NodeMetadata> {
    let path = path.as_ref();

    let coverage_repo = git_data.as_ref()?.coverage_repo.as_ref()?;

    let coverage_commit = coverage_repo
        .revparse_single(coverage_rev)
        .ok()?
        .peel_to_commit()
        .ok()?;

    let tree = coverage_commit.tree().ok()?;

    let covered = tree.get_path(path).ok()?;

    let coverage_summary_path = match covered.kind()? {
        git2::ObjectType::Tree => path.join("index.json"),
        git2::ObjectType::Blob => path.with_added_extension("summary.json"),
        _ => return None,
    };

    let coverage_summary_object = tree
        .get_path(&coverage_summary_path)
        .ok()?
        .to_object(coverage_repo)
        .ok()?;

    serde_json::from_slice(coverage_summary_object.as_blob()?.content()).ok()
}

/// Returns the coverage summary for the given path for the HEAD revision of the main repository, if available.
pub fn coverage_summary_for_head(
    git_data: Option<&GitData>,
    path: impl AsRef<Path>,
) -> Option<code_coverage_report::NodeMetadata> {
    let head_oid = git_data?.repo.head().ok()?.peel_to_commit().ok()?.id();

    let coverage_rev = format!("refs/tags/reverse/all/all/{}", head_oid);

    coverage_summary(git_data, &coverage_rev, path)
}

#[derive(Serialize, Default, Debug)]
pub struct CoverageNavigation {
    pub previous: Option<String>,
    pub next: Option<String>,
    pub latest: Option<String>,
}

/// Returns the previous, next and latest commit ids (in the main repository) for which coverage data is available for the given path.
pub fn coverage_navigation(
    git_data: Option<&GitData>,
    path: impl AsRef<Path>,
    rev: git2::Oid,
) -> Option<CoverageNavigation> {
    let path = path.as_ref();
    let git = git_data?;

    let main_repo = &git.repo;
    let coverage_repo = git.coverage_repo.as_ref()?;
    let main_commit = main_repo.find_commit(rev).ok()?;

    let mut revwalk = coverage_repo.revwalk().ok()?;
    revwalk.set_sorting(git2::Sort::TIME).ok()?;
    revwalk.push_head().ok()?;

    let mut revwalk = revwalk
        .flatten()
        .flat_map(|oid| coverage_repo.find_commit(oid).ok())
        .filter(|commit| {
            path == ""
                || commit
                    .tree()
                    .ok()
                    .is_some_and(|tree| tree.get_path(path).is_ok())
        })
        .peekable();
    // We want the later_coverage and previous_coverage iterators below to advance the same underlying iterator.
    let revwalk = revwalk.by_ref();

    let mut later_coverage = revwalk.peeking_take_while(|coverage_commit| {
        coverage_commit.committer().when() > main_commit.committer().when()
    });
    let latest_coverage = later_coverage.next();
    let next_coverage = later_coverage.last();

    let mut earlier_coverage = revwalk.skip_while(|coverage_commit| {
        coverage_commit.committer().when() >= main_commit.committer().when()
    });
    let previous_coverage = earlier_coverage.next();

    let main_repo_oid = |coverage_commit: Option<git2::Commit>| {
        coverage_commit.and_then(|coverage_commit| coverage_commit.message().map(ToOwned::to_owned))
    };

    Some(CoverageNavigation {
        previous: main_repo_oid(previous_coverage),
        next: main_repo_oid(next_coverage),
        latest: main_repo_oid(latest_coverage),
    })
}

#[cfg(test)]
mod tests {
    use crate::utils::TempDir;

    use super::*;
    use git2::*;

    #[test]
    fn test_coverage_navigation() {
        let tmpdir = TempDir::new("test_coverage_navigation");

        const MAIN: &str = "refs/heads/main";
        let main_repo = Repository::init_opts(
            tmpdir.join("main_repo"),
            RepositoryInitOptions::new().bare(true).initial_head(MAIN),
        )
        .unwrap();

        let make_tree = |repo: &Repository| {
            let blob = repo.blob(b"test\n").unwrap();
            let mut tree_builder = repo.treebuilder(None).unwrap();
            tree_builder
                .insert("test", blob, FileMode::Blob.into())
                .unwrap();
            tree_builder.write().unwrap()
        };

        // Build an empty Tree with a dummy test Blob
        let main_tree_oid = make_tree(&main_repo);

        // commits need to have distinct timestamps
        let mut timestamps = std::iter::successors(Some(0), |t| Some(t + 60));

        let mut commit_on_main_repo = |message: String| {
            let signature = Signature::new(
                "searchfox",
                "searchfox@localhost",
                &Time::new(timestamps.next().unwrap(), 0),
            )
            .unwrap();

            let tree = main_repo.find_tree(main_tree_oid).unwrap();

            let parent = main_repo
                .revparse_single(MAIN)
                .ok()
                .and_then(|oid| oid.peel_to_commit().ok());
            let parents = if let Some(parent) = parent.as_ref() {
                &[parent][..]
            } else {
                &[]
            };

            let oid = main_repo
                .commit(Some(MAIN), &signature, &signature, &message, &tree, parents)
                .unwrap();

            oid
        };

        let main_commits: Vec<_> = (0..=10)
            .map(|i| commit_on_main_repo(i.to_string()))
            .collect();

        const ALL_ALL: &str = "refs/heads/all/all";
        let coverage_repo = Repository::init_opts(
            tmpdir.join("coverage_repo"),
            RepositoryInitOptions::new()
                .bare(true)
                .initial_head(ALL_ALL),
        )
        .unwrap();

        // Build an empty Tree with a dummy test Blob
        let coverage_tree_oid = make_tree(&coverage_repo);

        let commit_on_coverage_repo = |for_main_commit_oid: Oid| {
            let main_commit = main_repo.find_commit(for_main_commit_oid).unwrap();
            let message = format!("{for_main_commit_oid}");
            let signature = main_commit.committer();

            let tree = coverage_repo.find_tree(coverage_tree_oid).unwrap();

            let parent = coverage_repo
                .revparse_single(ALL_ALL)
                .ok()
                .and_then(|oid| oid.peel_to_commit().ok());
            let parents = if let Some(parent) = parent.as_ref() {
                &[parent][..]
            } else {
                &[]
            };

            let oid = coverage_repo
                .commit(
                    Some(ALL_ALL),
                    &signature,
                    &signature,
                    &message,
                    &tree,
                    parents,
                )
                .unwrap();

            oid
        };

        commit_on_coverage_repo(main_commits[2]);
        commit_on_coverage_repo(main_commits[5]);
        commit_on_coverage_repo(main_commits[9]);

        let git_data = GitData {
            repo: main_repo,
            coverage_repo: Some(coverage_repo),
            blame_repo: None,
            blame_map: Default::default(),
            hg_map: Default::default(),
            old_map: Default::default(),
            mailmap: crate::file_format::config::Mailmap {
                entries: Default::default(),
            },
            blame_ignore: Default::default(),
        };

        let check = |main_repo_commit,
                     previous_commit_with_coverage: Option<usize>,
                     next_commit_with_coverage: Option<usize>,
                     latest_commit_with_coverage: Option<usize>| {
            let navigation =
                coverage_navigation(Some(&git_data), "", main_commits[main_repo_commit]).unwrap();
            assert_eq!(
                navigation.previous,
                previous_commit_with_coverage.map(|index| main_commits[index].to_string())
            );
            assert_eq!(
                navigation.next,
                next_commit_with_coverage.map(|index| main_commits[index].to_string())
            );
            assert_eq!(
                navigation.latest,
                latest_commit_with_coverage.map(|index| main_commits[index].to_string())
            );
        };

        check(0, None, Some(2), Some(9));
        check(1, None, Some(2), Some(9));
        check(2, None, Some(5), Some(9));
        check(3, Some(2), Some(5), Some(9));
        check(4, Some(2), Some(5), Some(9));
        check(5, Some(2), None, Some(9));
        check(6, Some(5), None, Some(9));
        check(7, Some(5), None, Some(9));
        check(8, Some(5), None, Some(9));
        check(9, Some(5), None, None);
        check(10, Some(9), None, None);
    }
}
