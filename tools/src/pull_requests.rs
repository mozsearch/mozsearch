//! The GitHub pull requests of a forest's commits (see `spec_forest`), and
//! the issues they close, for the blame popups' links (see
//! `blame::commit_info_json`): from the merged pull requests' snapshot (see
//! scripts/spec-prs/README.md), which `PullRequests::load` reads from the
//! tree's history directory, and from the commits' messages, whose subjects
//! name their pull requests as GitHub's squash merges do ("Fix it (#123)", or
//! "Merge pull request #123 from ..."), and whose closing keywords name the
//! issues they close ("Fixes #45.", as WHATWG's commits say).

use std::collections::HashMap;

use git2::Oid;
use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;

/// The file of a tree's merged pull requests, in its history directory.
pub const PULL_REQUESTS_FILE: &str = "pull-requests.json";

/// A spec repository's merged pull requests, in the snapshot: the forest's
/// specs whose repository it is, and its pull requests, each its number, its
/// merge commit (for a squash merge, the squashed commit), and the issues it
/// closes ("#N" in the repository, or "OWNER/NAME#N").
#[derive(Default, Deserialize)]
struct RepoPullRequests {
    #[serde(default)]
    specs: Vec<String>,
    #[serde(default)]
    prs: Vec<(u32, Option<String>, Vec<String>)>,
}

#[derive(Default, Deserialize)]
struct Snapshot {
    #[serde(default)]
    repos: HashMap<String, RepoPullRequests>,
}

/// A tree's merged pull requests, by their merge commits, and its specs'
/// repositories.
#[derive(Default)]
pub struct PullRequests {
    repos: Vec<String>,
    by_commit: HashMap<Oid, (usize, u32, Vec<String>)>,
    spec_repos: HashMap<String, usize>,
}

impl PullRequests {
    /// The merged pull requests of `PULL_REQUESTS_FILE` in `history_path`,
    /// or none if there isn't one (or it's not one).
    pub fn load(history_path: &str) -> PullRequests {
        let path = format!("{}/{}", history_path, PULL_REQUESTS_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return PullRequests::default();
        };
        match serde_json::from_str::<Snapshot>(&text) {
            Ok(snapshot) => PullRequests::from_snapshot(snapshot),
            Err(e) => {
                log::warn!("Not using {}: {}", path, e);
                PullRequests::default()
            }
        }
    }

    fn from_snapshot(snapshot: Snapshot) -> PullRequests {
        let mut links = PullRequests::default();
        let mut repos: Vec<_> = snapshot.repos.into_iter().collect();
        repos.sort_by(|a, b| a.0.cmp(&b.0));
        for (repo, found) in repos {
            let index = links.repos.len();
            links.repos.push(repo);
            for spec in found.specs {
                links.spec_repos.insert(spec, index);
            }
            for (number, merge, closes) in found.prs {
                if let Some(oid) = merge.and_then(|merge| Oid::from_str(&merge).ok()) {
                    links.by_commit.insert(oid, (index, number, closes));
                }
            }
        }
        links
    }

    /// The pull request whose merge commit is `upstream` (its repository and
    /// number) and the issues it closes.
    fn merged_by(&self, upstream: Oid) -> Option<(&str, u32, &[String])> {
        let (repo, number, closes) = self.by_commit.get(&upstream)?;
        Some((&self.repos[*repo], *number, closes))
    }

    /// The GitHub repository (OWNER/NAME) of a forest's spec.
    fn spec_repo(&self, spec: &str) -> Option<&str> {
        self.spec_repos
            .get(spec)
            .map(|index| self.repos[*index].as_str())
    }
}

/// A commit's pull request's and closed issues' pages.
#[derive(Debug, Default, PartialEq)]
pub struct CommitLinks {
    pub pull_request: Option<String>,
    pub issues: Vec<String>,
}

/// The pull request and closed issues of a forest's commit (with `message`,
/// whose `Upstream:` trailer says which upstream commit it is): the merged
/// pull request whose merge commit it is, if the snapshot has it, or the one
/// its subject names, and the issues that pull request closes and those the
/// message closes.  None if it's not a forest's commit, or its repository
/// isn't known (from its trailer's GitHub URL, or the snapshot).  (The
/// snapshot is only asked for for a forest's commit.)
pub fn commit_links<'a>(
    message: &str,
    pull_requests: impl FnOnce() -> &'a PullRequests,
) -> Option<CommitLinks> {
    let (spec, upstream) = crate::spec_forest::parse_upstream_trailer(message)?;
    let pull_requests = pull_requests();
    let trailer_repo = trailer_repo(message);
    let merged = pull_requests.merged_by(upstream);
    let repo = merged
        .map(|(repo, _, _)| repo)
        .or(trailer_repo.as_deref())
        .or_else(|| pull_requests.spec_repo(spec))?;
    let number = merged
        .map(|(_, number, _)| number)
        .or_else(|| subject_pull_request(message));
    let mut issues: Vec<String> = vec![];
    let refs = merged
        .map(|(_, _, closes)| closes.to_vec())
        .unwrap_or_default()
        .into_iter()
        .chain(closed_issues(message));
    for reference in refs {
        let Some(url) = issue_url(repo, &reference) else {
            continue;
        };
        // (Not the pull request itself, which a message can say it closes.)
        let own = number.map(|number| format!("https://github.com/{}/issues/{}", repo, number));
        if Some(&url) != own.as_ref() && !issues.contains(&url) {
            issues.push(url);
        }
    }
    let pull_request = number.map(|number| format!("https://github.com/{}/pull/{}", repo, number));
    if pull_request.is_none() && issues.is_empty() {
        return None;
    }
    Some(CommitLinks {
        pull_request,
        issues,
    })
}

/// The GitHub repository (OWNER/NAME) of a forest commit's `Upstream:`
/// trailer's URL, if it's a GitHub commit's page.
fn trailer_repo(message: &str) -> Option<String> {
    let line = message
        .lines()
        .rev()
        .find(|line| line.starts_with("Upstream: "))?;
    let url = line.split_whitespace().nth(2)?;
    let rest = url.strip_prefix("https://github.com/")?;
    let (repo, _) = rest.split_once("/commit/")?;
    (repo.split('/').count() == 2).then(|| repo.to_string())
}

lazy_static! {
    // GitHub's squash merges' subjects end with their pull requests' numbers,
    // and its merge commits' subjects start with them.
    static ref SQUASHED: Regex = Regex::new(r"\(#([0-9]+)\)\s*$").unwrap();
    static ref MERGED: Regex = Regex::new(r"^Merge pull request #([0-9]+) ").unwrap();
    // GitHub's closing keywords, each with issues (ex: "Fixes #1", "closes:
    // whatwg/dom#2", "Fixes https://github.com/whatwg/html/issues/3", "Fixes
    // #4, #5 and #6").
    static ref CLOSING: Regex = Regex::new(
        r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\b:?\s+((?:(?:,\s*|\s+and\s+)?(?:https://github\.com/[\w.-]+/[\w.-]+/issues/[0-9]+|(?:[\w.-]+/[\w.-]+)?#[0-9]+)\b)+)"
    )
    .unwrap();
    static ref ISSUE: Regex = Regex::new(
        r"https://github\.com/([\w.-]+/[\w.-]+)/issues/([0-9]+)|([\w.-]+/[\w.-]+)?#([0-9]+)"
    )
    .unwrap();
}

/// The pull request a commit's subject names, as GitHub's merges' subjects do.
fn subject_pull_request(message: &str) -> Option<u32> {
    let subject = message.lines().next()?;
    let captures = SQUASHED
        .captures(subject)
        .or_else(|| MERGED.captures(subject))?;
    captures[1].parse().ok()
}

/// The issues a commit's message closes, with GitHub's closing keywords, as
/// "#N" (in its repository) or "OWNER/NAME#N".
fn closed_issues(message: &str) -> Vec<String> {
    let mut issues = vec![];
    for closing in CLOSING.captures_iter(message) {
        for issue in ISSUE.captures_iter(&closing[1]) {
            let reference = match (issue.get(1), issue.get(2), issue.get(3), issue.get(4)) {
                (Some(repo), Some(number), _, _) => {
                    format!("{}#{}", repo.as_str(), number.as_str())
                }
                (_, _, Some(repo), Some(number)) => {
                    format!("{}#{}", repo.as_str(), number.as_str())
                }
                (_, _, None, Some(number)) => format!("#{}", number.as_str()),
                _ => continue,
            };
            issues.push(reference);
        }
    }
    issues
}

/// The page of an issue ("#N" in `repo`, or "OWNER/NAME#N").
fn issue_url(repo: &str, reference: &str) -> Option<String> {
    let (owner_repo, number) = reference.split_once('#')?;
    let number: u32 = number.parse().ok()?;
    let owner_repo = if owner_repo.is_empty() {
        repo
    } else {
        owner_repo
    };
    Some(format!(
        "https://github.com/{}/issues/{}",
        owner_repo, number
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPSTREAM: &str = "57535533d504b7148f72a3209d083122292321a0";

    fn snapshot(json: &str) -> PullRequests {
        PullRequests::from_snapshot(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn test_messages() {
        let none = PullRequests::default();
        // A squash merge's subject, and closing keywords.
        let message = format!(
            "Expose it to worklets (#13006)\n\nThis lets w3c/hr-time#169 expose it.\n\nFixes #12951, fixes whatwg/dom#7 and closes https://github.com/w3c/hr-time/issues/170.\n\nUpstream: html https://github.com/whatwg/html/commit/{}\n",
            UPSTREAM
        );
        assert_eq!(
            commit_links(&message, || &none),
            Some(CommitLinks {
                pull_request: Some("https://github.com/whatwg/html/pull/13006".to_string()),
                issues: vec![
                    "https://github.com/whatwg/html/issues/12951".to_string(),
                    "https://github.com/whatwg/dom/issues/7".to_string(),
                    "https://github.com/w3c/hr-time/issues/170".to_string(),
                ],
            })
        );
        // A merge commit's subject; several issues after one keyword.
        let message = format!(
            "Merge pull request #12 from someone/branch\n\nFixes #3, #4 and #5.\n\nUpstream: dom https://github.com/whatwg/dom/commit/{}\n",
            UPSTREAM
        );
        let links = commit_links(&message, || &none).unwrap();
        assert_eq!(
            links.pull_request.as_deref(),
            Some("https://github.com/whatwg/dom/pull/12")
        );
        assert_eq!(links.issues.len(), 3);
        // Only issues (WHATWG's subjects don't name their pull requests).
        let message = format!(
            "Fix hiding popovers\n\nFixes #9161.\n\nUpstream: html https://github.com/whatwg/html/commit/{}\n",
            UPSTREAM
        );
        assert_eq!(
            commit_links(&message, || &none),
            Some(CommitLinks {
                pull_request: None,
                issues: vec!["https://github.com/whatwg/html/issues/9161".to_string()],
            })
        );
        // Neither; and references which aren't closing ones.
        let message = format!(
            "Goes with w3c/trusted-types#606 (see #12)\n\nUpstream: html https://github.com/whatwg/html/commit/{}\n",
            UPSTREAM
        );
        assert_eq!(commit_links(&message, || &none), None);
        // Not a forest's commit.
        assert_eq!(commit_links("Fix it (#1)\n", || &none), None);
        // A trailer without a URL: its repository isn't known.
        let message = format!("Fix it (#1)\n\nUpstream: html {}\n", UPSTREAM);
        assert_eq!(commit_links(&message, || &none), None);
        // (Words ending with the keywords aren't them.)
        assert!(closed_issues("Prefixes #1, suffixed #2").is_empty());
    }

    #[test]
    fn test_snapshot() {
        let links = snapshot(&format!(
            r##"{{"taken": "2026-10-29T00:00:00Z", "repos": {{
                "whatwg/html": {{"specs": ["html"], "newest": "2026-10-29T00:00:00Z", "prs": [
                    [13006, "{}", ["#12951", "w3c/hr-time#169"]],
                    [13007, null, []]
                ]}}
            }}}}"##,
            UPSTREAM
        ));
        // The snapshot's pull request, though the subject doesn't say, and
        // its closed issues, with the message's (not twice).
        let message = format!(
            "Expose it\n\nFixes #12951. Closes #1.\n\nUpstream: html {}\n",
            UPSTREAM
        );
        assert_eq!(
            commit_links(&message, || &links),
            Some(CommitLinks {
                pull_request: Some("https://github.com/whatwg/html/pull/13006".to_string()),
                issues: vec![
                    "https://github.com/whatwg/html/issues/12951".to_string(),
                    "https://github.com/w3c/hr-time/issues/169".to_string(),
                    "https://github.com/whatwg/html/issues/1".to_string(),
                ],
            })
        );
        // Another commit of the spec: its subject's pull request, in the
        // snapshot's repository of its spec (its trailer has no URL).
        let message = "Fix it (#77)\n\nUpstream: html 1111111111111111111111111111111111111111\n";
        assert_eq!(
            commit_links(message, || &links)
                .unwrap()
                .pull_request
                .as_deref(),
            Some("https://github.com/whatwg/html/pull/77")
        );
        // A message saying it closes its own pull request.
        let message = format!(
            "Fix it (#5)\n\nFixes #5.\n\nUpstream: html https://github.com/whatwg/html/commit/{}\n",
            "2222222222222222222222222222222222222222"
        );
        assert_eq!(
            commit_links(&message, || &links).unwrap().issues,
            Vec::<String>::new()
        );
    }
}
