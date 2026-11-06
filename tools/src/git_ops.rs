use git2::{Commit, Object, Repository, TreeEntry};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::file_format::{
    code_coverage_report::{self, EXACT},
    config::GitData,
    coverage::{InterpolatedCoverage, interpolate_coverage},
};

/// A git command to run git fast-import with: the git named by MOZSEARCH_GIT at
/// run time, or else when the tools were built, or else the git on the PATH.
/// The nix package builds the tools with our git, whose fast-import scales to
/// repos with millions of distinct file names (see nix/mozsearch/git.nix),
/// which makes the history tools several times faster on firefox-main.  (It's
/// also `mozsearch-git` on the PATH, for scripts working on those repos.  The
/// tools' other git commands are the system's `git`, which detects SHA-1
/// collisions, since they work on the source repos.)
pub fn fast_import_git() -> std::process::Command {
    let git = std::env::var_os("MOZSEARCH_GIT")
        .or_else(|| option_env!("MOZSEARCH_GIT").map(Into::into))
        .unwrap_or_else(|| "git".into());
    // (So that the logs say which git wrote the repo.)
    log::info!("Running git fast-import from {}", Path::new(&git).display());
    std::process::Command::new(git)
}

/// The most compute threads the history tools use by default.  More threads
/// contend in libgit2 (which has process-wide locks, ex: for its pack windows):
/// on 1500 revisions of a firefox window, build-timeline-tree took 158s with
/// 15 compute threads, 198s with 31, and 334s with 63 (with a quarter of its
/// time in the kernel), and about the same with 7 as with 15.
const MAX_HISTORY_COMPUTE_THREADS: usize = 15;

/// How many compute threads a history tool uses: its own variable if set (ex:
/// SYNTAX_COMPUTE_THREADS), or else COMPUTE_THREADS if set, or else one fewer
/// than the CPUs (leaving one for the main thread), up to
/// `MAX_HISTORY_COMPUTE_THREADS`.  (Ex: the wubspecs tree limits only
/// build-syntax-token-tree's threads, since each keeps 10 revisions in flight,
/// which for the HTML Standard's early revisions take ~0.8 GB each, while
/// build-timeline-tree's are what its history's speed is limited by.)
pub fn history_compute_threads(tool_var: &str) -> usize {
    compute_threads(
        std::env::var(tool_var).ok().as_deref(),
        std::env::var("COMPUTE_THREADS").ok().as_deref(),
        num_cpus::get(),
    )
}

fn compute_threads(tool: Option<&str>, common: Option<&str>, cpus: usize) -> usize {
    tool.and_then(|v| v.parse().ok())
        .or_else(|| common.and_then(|v| v.parse().ok()))
        .unwrap_or_else(|| cpus.saturating_sub(1).min(MAX_HISTORY_COMPUTE_THREADS))
        .max(1)
}

// Helpers to do things with git2

fn latin1_to_string(bytes: Vec<u8>) -> String {
    bytes.iter().map(|&c| c as char).collect()
}

/// Whether a revision from a request (ex: `/{tree}/rev/{rev}/...`) is a
/// (maybe abbreviated) hex commit id, which is what the pages for a revision
/// take (web-server redirects a ref name to its commit's id; see
/// `resolve_ref_name`).
pub fn is_hex_rev(rev: &str) -> bool {
    (4..=40).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether a revision from a request is a ref name (ex: a branch's, `main`, a
/// tag's, or one with slashes, `release/esr128`), in git's rules for them but
/// only of letters, digits, `.`, `_`, `-` and `/`, so not any of git's other
/// revision syntax (ex: `main~1`, `main@{yesterday}`, `main^{/message}`),
/// whose history walks can be of any length.
pub fn is_ref_name(name: &str) -> bool {
    name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.contains("..")
        && name.split('/').all(|component| {
            !component.is_empty() && !component.starts_with('.') && !component.ends_with(".lock")
        })
}

/// A ref name's (see `is_ref_name`) commit's full id, if it's one of the
/// repository's refs, looked up as git looks up a short ref name (as is,
/// then `refs/tags/NAME`, `refs/heads/NAME`, `refs/remotes/NAME`, ...).
pub fn resolve_ref_name(repo: &Repository, name: &str) -> Option<String> {
    if !is_ref_name(name) {
        return None;
    }
    let reference = repo.resolve_reference_from_short_name(name).ok()?;
    Some(reference.peel_to_commit().ok()?.id().to_string())
}

/// A revision from a request (see `is_hex_rev`) as its commit's full id, which
/// is what the pages and git commands for it use.
pub fn canonical_rev(repo: &Repository, rev: &str) -> Result<String, &'static str> {
    if !is_hex_rev(rev) {
        return Err("Bad revision");
    }
    let commit = repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    Ok(commit.id().to_string())
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

#[test]
fn test_is_ref_name() {
    for name in [
        "main",
        "HEAD",
        "FIREFOX_NIGHTLY_120_END",
        "release/esr128",
        "v1.2-rc",
    ] {
        assert!(is_ref_name(name), "{}", name);
    }
    for name in [
        "",
        "main~1",
        "main^",
        "main@{yesterday}",
        "main^{/message}",
        "a..b",
        "-output",
        "release/",
        "a//b",
        ".hidden",
        "a.lock",
        "main:path",
    ] {
        assert!(!is_ref_name(name), "{}", name);
    }
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

#[derive(Deserialize, Serialize)]
pub struct FileCoverage {
    pub exact: bool,
    pub lines: Vec<InterpolatedCoverage>,
}

pub fn get_coverage(
    git_data: Option<&GitData>,
    coverage_commit: Option<&Commit>,
    path: impl AsRef<Path>,
) -> Option<FileCoverage> {
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
                    FileCoverage {
                        exact: coverage_commit
                            .message()
                            .unwrap_or_default()
                            .contains(EXACT),
                        lines: interpolate_coverage(raw),
                    }
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
    let coverage_rev = coverage_rev_for_head(git_data)?;
    coverage_summary(git_data, &coverage_rev, path)
}

/// The coverage repo's revision of the head's coverage (see
/// `coverage_summary`), if the tree has a coverage repo: once for many paths,
/// rather than reading the repo's HEAD for each (see
/// `RepoIngestion::ingest_file_list_and_apply_heuristics`).
pub fn coverage_rev_for_head(git_data: Option<&GitData>) -> Option<String> {
    let git_data = git_data?;
    git_data.coverage_repo.as_ref()?;
    let head_oid = git_data.repo.head().ok()?.peel_to_commit().ok()?.id();
    Some(format!("refs/tags/reverse/all/all/{}", head_oid))
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

            let main_repo_oid = coverage_commit
                .message()
                .ok()?
                .lines()
                .next()?
                .trim()
                .to_owned();

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

#[cfg(test)]
mod tests {
    use super::compute_threads;

    #[test]
    fn test_compute_threads() {
        // A tool's own variable, then COMPUTE_THREADS, then the CPUs (but
        // one, up to 15).
        assert_eq!(compute_threads(Some("4"), Some("8"), 32), 4);
        assert_eq!(compute_threads(None, Some("8"), 32), 8);
        assert_eq!(compute_threads(Some("bad"), Some("8"), 32), 8);
        assert_eq!(compute_threads(None, None, 32), 15);
        assert_eq!(compute_threads(None, None, 4), 3);
        assert_eq!(compute_threads(None, None, 1), 1);
        assert_eq!(compute_threads(Some("0"), None, 32), 1);
    }
}
