//! The code review bot's API (code-review.moz.tools): the try pushes it makes
//! of Phabricator revisions' diffs (which their "CI (Treeherder) Jobs" links
//! are), for DWIM's `try D123456` (see `format::dwim_lookup`).

use serde_json::Value;

/// Where the bot's API is, if the config doesn't say (`code_review_url`).
pub const CODE_REVIEW_URL: &str = "https://api.code-review.moz.tools";

/// The try push (its revision on try) of the latest of Phabricator revision
/// `number`'s diffs which the bot pushed to try, by the API at `base` (or,
/// for a `file://` base, its `diffs-NUMBER.json`, ex: tests' fixtures), if
/// any.
pub async fn try_push_of_revision(base: &str, number: u64) -> Option<String> {
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
    latest_try_push(&serde_json::from_str(&text).ok()?, number)
}

/// The try push of the latest diff of Phabricator revision `number` which was
/// pushed to try, in a response of the API's search of diffs.
pub fn latest_try_push(response: &Value, number: u64) -> Option<String> {
    response["results"]
        .as_array()?
        .iter()
        .filter(|diff| {
            diff["revision"]["provider"] == "phabricator"
                && diff["revision"]["provider_id"].as_u64() == Some(number)
                && diff["repository"]["slug"] == "try"
        })
        .filter_map(|diff| Some((diff["created"].as_str()?, diff["mercurial_hash"].as_str()?)))
        .max_by(|a, b| a.0.cmp(b.0))
        .map(|(_, rev)| rev.to_string())
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
        let diff = |number: u64, repository: &str, rev: &str, created: &str| {
            serde_json::json!({
                "revision": { "provider": "phabricator", "provider_id": number },
                "repository": { "slug": repository },
                "mercurial_hash": rev,
                "created": created,
            })
        };
        let response = serde_json::json!({ "results": [
            diff(333833, "try", "6cecd3ef48df0709726721b2ff549a66bb242928", "2026-10-27T17:50:14.919973Z"),
            diff(333833, "try", "eb82baaeebe8569e0a6c37fa9f12289a9fe7e850", "2026-11-03T21:55:52.781404Z"),
            diff(325508, "try-comm-central", "07d7ca6f3dab139c86baf28aa89121e50c3afd9a", "2026-09-11T08:00:00.000000Z"),
        ]});
        assert_eq!(
            latest_try_push(&response, 333833).as_deref(),
            Some("eb82baaeebe8569e0a6c37fa9f12289a9fe7e850")
        );
        assert_eq!(latest_try_push(&response, 325508), None);
        assert_eq!(latest_try_push(&response, 1), None);
    }
}
