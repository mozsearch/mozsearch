//! The code review bot's API (code-review.moz.tools): the try pushes it makes
//! of Phabricator revisions' diffs (which their "CI (Treeherder) Jobs" links
//! are), for DWIM's `try D123456` (see `format::dwim_lookup`), and for review
//! dashboards (the data API's `phab-try`), which can't ask the API themselves,
//! as it has no CORS headers.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

/// Where the bot's API is, if the config doesn't say (`code_review_url`).
pub const CODE_REVIEW_URL: &str = "https://api.code-review.moz.tools";

/// The latest of a Phabricator revision's diffs which the bot pushed to try:
/// its try push (its revision on try, 40 hexadecimal digits), the diff's PHID
/// (Phabricator's, the bot's `provider_id`, so that a dashboard can tell
/// whether the push is of the revision's current diff; the bot's own ids of
/// diffs aren't Phabricator's), and when the bot made it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevisionTryPush {
    pub push: String,
    pub diff_phid: Option<String>,
    pub created: String,
}

/// The try push (its revision on try) of the latest of Phabricator revision
/// `number`'s diffs which the bot pushed to try, by the API at `base` (or,
/// for a `file://` base, its `diffs-NUMBER.json`, ex: tests' fixtures), if
/// any.
pub async fn try_push_of_revision(base: &str, number: u64) -> Option<String> {
    latest_try_diff(&diffs_of_revision(base, number).await?, number).map(|diff| diff.push)
}

/// How long `cached_try_push_of_revision` keeps an answer, and how many.
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_ENTRIES: usize = 10_000;

/// `latest_try_diff` of revision `number`, by the API at `base`, as it was in
/// the last `CACHE_TTL` (ex: for each of a dashboard's refreshes, by each of
/// its users), so that dashboards don't ask the bot's API for it each time.
pub async fn cached_try_push_of_revision(base: &str, number: u64) -> Option<RevisionTryPush> {
    static CACHE: Mutex<Option<HashMap<(String, u64), (Instant, Option<RevisionTryPush>)>>> =
        Mutex::new(None);
    let key = (base.to_string(), number);
    if let Some(cache) = CACHE.lock().unwrap().as_ref()
        && let Some((at, answer)) = cache.get(&key)
        && at.elapsed() < CACHE_TTL
    {
        return answer.clone();
    }
    let answer = latest_try_diff(&diffs_of_revision(base, number).await?, number);
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

/// The API's search of diffs for revision `number` (by the API at `base`, or
/// a `file://` base's `diffs-NUMBER.json`).
async fn diffs_of_revision(base: &str, number: u64) -> Option<Value> {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let text = match base.strip_prefix("file://") {
        Some(dir) => {
            let path = format!("{}/diffs-{}.json", dir.trim_end_matches('/'), number);
            std::fs::read_to_string(path).ok()?
        }
        None => {
            let client = CLIENT.get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(concat!("searchfox/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .unwrap_or_default()
            });
            // (Its search of the diffs' revisions' fields, which also finds
            // other revisions, ex: whose bug numbers have the number.)
            let url = format!(
                "{}/v1/diff/?search={}&limit=50",
                base.trim_end_matches('/'),
                number
            );
            client
                .get(url)
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

/// The try push of the latest diff of Phabricator revision `number` which was
/// pushed to try, in a response of the API's search of diffs.
pub fn latest_try_push(response: &Value, number: u64) -> Option<String> {
    latest_try_diff(response, number).map(|diff| diff.push)
}

/// The latest diff of Phabricator revision `number` which was pushed to try
/// (see `RevisionTryPush`), in a response of the API's search of diffs.
pub fn latest_try_diff(response: &Value, number: u64) -> Option<RevisionTryPush> {
    response["results"]
        .as_array()?
        .iter()
        .filter(|diff| {
            diff["revision"]["provider"] == "phabricator"
                && diff["revision"]["provider_id"].as_u64() == Some(number)
                && diff["repository"]["slug"] == "try"
        })
        .filter_map(|diff| {
            Some(RevisionTryPush {
                push: crate::try_import::normalize_rev(diff["mercurial_hash"].as_str()?)?,
                diff_phid: diff["provider_id"].as_str().map(str::to_string),
                created: diff["created"].as_str()?.to_string(),
            })
        })
        .max_by(|a, b| a.created.cmp(&b.created))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The latest diff's try push, of the revision's diffs only, as the API
    /// answered for D333833 (as in tests/code-review, the webtests' fixture,
    /// which nix's build of the tools doesn't have), whose search also found
    /// D325508's diff, which was pushed to comm-central's try.
    #[test]
    fn test_latest_try_push() {
        let diff = |number: u64, repository: &str, rev: &str, created: &str, phid: &str| {
            serde_json::json!({
                "id": 1358828,
                "provider_id": phid,
                "revision": { "provider": "phabricator", "provider_id": number },
                "repository": { "slug": repository },
                "mercurial_hash": rev,
                "created": created,
            })
        };
        let response = serde_json::json!({ "results": [
            diff(333833, "try", "6cecd3ef48df0709726721b2ff549a66bb242928", "2026-10-27T17:50:14.919973Z",
                 "PHID-DIFF-cnb25lbi3eznt5qpocdt"),
            diff(333833, "try", "eb82baaeebe8569e0a6c37fa9f12289a9fe7e850", "2026-11-03T21:55:52.781404Z",
                 "PHID-DIFF-pwvkmz6doimuhqweswq4"),
            diff(325508, "try-comm-central", "07d7ca6f3dab139c86baf28aa89121e50c3afd9a", "2026-09-11T08:00:00.000000Z",
                 "PHID-DIFF-x"),
        ]});
        assert_eq!(
            latest_try_push(&response, 333833).as_deref(),
            Some("eb82baaeebe8569e0a6c37fa9f12289a9fe7e850")
        );
        assert_eq!(latest_try_push(&response, 325508), None);
        assert_eq!(latest_try_push(&response, 1), None);
        // With its diff's PHID (Phabricator's, not the bot's id) and its date.
        assert_eq!(
            latest_try_diff(&response, 333833),
            Some(RevisionTryPush {
                push: "eb82baaeebe8569e0a6c37fa9f12289a9fe7e850".to_string(),
                diff_phid: Some("PHID-DIFF-pwvkmz6doimuhqweswq4".to_string()),
                created: "2026-11-03T21:55:52.781404Z".to_string(),
            })
        );
    }
}
