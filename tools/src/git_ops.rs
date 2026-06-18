use git2::{Commit, Object, Repository, TreeEntry};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::file_format::{
    code_coverage_report,
    config::GitData,
    coverage::{InterpolatedCoverage, interpolate_coverage},
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

pub fn git_time_to_chrono(git_time: git2::Time) -> chrono::DateTime<chrono::FixedOffset> {
    let tz = chrono::FixedOffset::east_opt(git_time.offset_minutes() * 60).unwrap();
    chrono::DateTime::from_timestamp(git_time.seconds(), 0)
        .unwrap()
        .with_timezone(&tz)
}

#[test]
fn test_git_time_to_chrono() {
    let source = git2::Time::new(1773370074, 2 * 60); // Fri Mar 13 04:47:54 2026 +0200
    let expected = chrono::DateTime::parse_from_rfc3339("2026-03-13T04:47:54+02:00").unwrap();
    let actual = git_time_to_chrono(source);
    assert_eq!(actual, expected);
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

    let coverage_summary_path = if path == "" {
        PathBuf::from("index.json")
    } else {
        let covered = tree.get_path(path).ok()?;

        match covered.kind()? {
            git2::ObjectType::Tree => path.join("index.json"),
            git2::ObjectType::Blob => path.with_added_extension("summary.json"),
            _ => return None,
        }
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

#[derive(Serialize, Debug)]
pub struct RevisionCoverage {
    pub rev: String,
    pub date: liquid::model::DateTime,
    #[serde(flatten)]
    pub data: code_coverage_report::NodeMetadata,
}

pub fn coverage_history(
    git_data: Option<&GitData>,
    path: impl AsRef<Path>,
) -> Option<Vec<RevisionCoverage>> {
    let path = path.as_ref();

    let coverage_repo = git_data.as_ref()?.coverage_repo.as_ref()?;

    let mut revwalk = coverage_repo.revwalk().ok()?;
    revwalk
        .set_sorting(git2::Sort::TIME | git2::Sort::REVERSE)
        .ok()?;
    revwalk.push_head().ok()?;

    let history: Vec<_> = revwalk
        .flat_map(|commit_oid| {
            let commit_oid = commit_oid.ok()?;

            let coverage_commit = coverage_repo.find_commit(commit_oid).ok()?;

            let main_repo_oid = coverage_commit.message().ok()?.trim().to_owned();

            let commit_rev = commit_oid.to_string();
            let data = coverage_summary(git_data, &commit_rev, path)?;

            let date = git_time_to_chrono(coverage_commit.committer().when());
            let date = date.format("%F %T %z").to_string();
            let date = liquid::model::DateTime::from_str(&date)?;

            Some(RevisionCoverage {
                rev: main_repo_oid,
                date,
                data,
            })
        })
        .collect();

    if history.is_empty() {
        None
    } else {
        Some(history)
    }
}
