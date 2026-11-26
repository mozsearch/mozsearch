//! GitHub's API's pull requests of the repository of a tree importing them
//! (see `try_import::github_pull_requests`): a pull request's head, title,
//! state and base branch, for its review's URL (`/{tree}/review/github/N`;
//! see the web-server's `review_page`), which imports it again when it has
//! newer commits.  Unauthenticated (60 requests an hour), so each answer is
//! kept a while (`CACHE_TTL`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

/// Where GitHub's API is, if the config doesn't say (`github_api_url`).
pub const GITHUB_API_URL: &str = "https://api.github.com";

/// A pull request, as GitHub's API has it: its head commit, its title, its
/// state ("open" or "closed"), whether it's merged, and the branch it's to.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub head: String,
    pub title: String,
    pub state: String,
    pub merged: bool,
    pub base: String,
}

/// How long `cached_pull_request` keeps an answer, and how many.
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_ENTRIES: usize = 10_000;

/// Pull request `number` of the repository `repo` (`OWNER/REPO`), by the API
/// at `base` (or, for a `file://` base, its `pulls-NUMBER.json`, ex: tests'
/// fixtures), as it was in the last `CACHE_TTL`, if it has one.
pub async fn cached_pull_request(base: &str, repo: &str, number: u64) -> Option<PullRequest> {
    static CACHE: Mutex<Option<HashMap<(String, String, u64), (Instant, Option<PullRequest>)>>> =
        Mutex::new(None);
    let key = (base.to_string(), repo.to_string(), number);
    if let Some(cache) = CACHE.lock().unwrap().as_ref()
        && let Some((at, answer)) = cache.get(&key)
        && at.elapsed() < CACHE_TTL
    {
        return answer.clone();
    }
    let answer = parse_pull_request(&fetch(base, repo, number).await?);
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() >= CACHE_ENTRIES {
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        if cache.len() >= CACHE_ENTRIES {
            cache.clear();
        }
    }
    cache.insert(key, (Instant::now(), answer.clone()));
    answer
}

/// The API's pull request (or a `file://` base's `pulls-NUMBER.json`).
async fn fetch(base: &str, repo: &str, number: u64) -> Option<Value> {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let text = match base.strip_prefix("file://") {
        Some(dir) => {
            let path = format!("{}/pulls-{}.json", dir.trim_end_matches('/'), number);
            std::fs::read_to_string(path).ok()?
        }
        None => {
            let client = CLIENT.get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(concat!("searchfox/", env!("CARGO_PKG_VERSION")))
                    .timeout(Duration::from_secs(10))
                    .build()
                    .unwrap_or_default()
            });
            let url = format!(
                "{}/repos/{}/pulls/{}",
                base.trim_end_matches('/'),
                repo,
                number
            );
            client
                .get(url)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .text()
                .await
                .ok()?
        }
    };
    serde_json::from_str(&text).ok()
}

/// A pull request, from the API's answer (see `PullRequest`).
pub fn parse_pull_request(value: &Value) -> Option<PullRequest> {
    let head = value["head"]["sha"].as_str()?;
    if head.len() != 40 || !head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(PullRequest {
        number: value["number"].as_u64()?,
        head: head.to_ascii_lowercase(),
        title: value["title"].as_str().unwrap_or_default().to_string(),
        state: value["state"].as_str().unwrap_or_default().to_string(),
        merged: value["merged"].as_bool().unwrap_or(false) || value["merged_at"].as_str().is_some(),
        base: value["base"]["ref"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_pull_request() {
        let value = json!({
            "number": 1154,
            "title": "CSP All The Things",
            "state": "open",
            "merged_at": null,
            "head": {"sha": "ABCDEF0123456789ABCDEF0123456789ABCDEF01", "ref": "csp"},
            "base": {"ref": "master"},
        });
        assert_eq!(
            parse_pull_request(&value),
            Some(PullRequest {
                number: 1154,
                head: "abcdef0123456789abcdef0123456789abcdef01".to_string(),
                title: "CSP All The Things".to_string(),
                state: "open".to_string(),
                merged: false,
                base: "master".to_string(),
            })
        );
        assert_eq!(
            parse_pull_request(&json!({"number": 1, "head": {"sha": "nope"}})),
            None,
            "(not a commit)"
        );
    }
}
