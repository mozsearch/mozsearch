use git2::{Commit, Object, Repository, TreeEntry};
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
