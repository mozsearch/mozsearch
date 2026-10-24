//! What a query may cost the server, by who it's from: limits per tier
//! (browsers, agents, bots, signed-in people, and local requests), from the
//! config's `query_limits` (since the config says what the server is: its
//! instance and its trees), and SEARCHFOX_QUERY_LIMITS for operations,
//! over each tier's defaults (`QueryLimits::default_for`):
//!
//! ```json
//! "query_limits": {
//!   "browser": { "history_candidates": 4000 },
//!   "bot": { "history_grep": true, "history_candidates": 200 }
//! }
//! ```
//!
//! Expensive commands (ex: the history greps, `re:`'s fuller engine) plan
//! what they'd read first, from cheap sizes, and refuse with `TooExpensive`
//! (a 422, `query-too-expensive`) rather than start what the tier's limits
//! don't allow; see `PipelineCommand::set_limits`, which gives the commands
//! the request's limits.  The admission control's token buckets and
//! concurrency (see `admission`) apply as for any request.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::admission::RequestClass;

/// Who a request is from, for its limits.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Browser,
    Agent,
    Bot,
    /// A signed-in person (once the server has sign-in, ex: LDAP through the
    /// load balancer's OIDC; nothing maps to this yet).
    Authenticated,
    /// The server itself, tests, and searchfox-tool: requests without
    /// forwarding headers (see `admission::client_key`), which aren't
    /// limited.
    Local,
}

impl Tier {
    /// The tier of a request of `class` (see `admission::admit_request`), or
    /// a local one's (None).
    pub fn of(class: Option<RequestClass>) -> Tier {
        match class {
            None => Tier::Local,
            Some(RequestClass::Browser) => Tier::Browser,
            Some(RequestClass::Agent) => Tier::Agent,
            Some(RequestClass::Bot) => Tier::Bot,
        }
    }
}

/// A tier's limits.  (Byte counts are of what's read: files' contents and
/// journals.)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryLimits {
    /// Whether the history greps (`text-history:`, `re-history:` and the
    /// timelines) run at all.
    pub history_grep: bool,
    /// How many candidate revisions a history grep may have in all (over its
    /// pages): more, and it's refused, as not much cheaper than checking the
    /// history locally.
    pub history_revisions: u64,
    /// How many (revision, file) pairs a history grep's page may check.
    pub history_candidates: u64,
    /// How many bytes of those files' versions it may read.
    pub history_blob_bytes: u64,
    /// How many scopes' lines it may find (a parse of a file's version each;
    /// see `scope:`).
    pub history_parses: u64,
    /// How many bytes of token journals (and other history files) it may read
    /// to plan.
    pub history_journal_bytes: u64,
    /// How many files `re:`'s fuller engine (for patterns livegrep can't run;
    /// see `search-text`) may run the pattern on.
    pub grep_files: u64,
    /// How many bytes of those files it may read.
    pub grep_bytes: u64,
}

const MIB: u64 = 1024 * 1024;

impl QueryLimits {
    /// A tier's limits without configuration.  (Starting points, to be
    /// scaled from a real instance's timings.)
    pub fn default_for(tier: Tier) -> QueryLimits {
        let browser = QueryLimits {
            history_grep: true,
            history_revisions: 20_000,
            history_candidates: 2_000,
            history_blob_bytes: 256 * MIB,
            history_parses: 500,
            history_journal_bytes: 64 * MIB,
            grep_files: 5_000,
            grep_bytes: 512 * MIB,
        };
        match tier {
            Tier::Browser => browser,
            Tier::Agent => QueryLimits {
                history_grep: true,
                history_revisions: browser.history_revisions / 2,
                history_candidates: browser.history_candidates / 2,
                history_blob_bytes: browser.history_blob_bytes / 2,
                history_parses: browser.history_parses / 2,
                history_journal_bytes: browser.history_journal_bytes / 2,
                grep_files: browser.grep_files / 2,
                grep_bytes: browser.grep_bytes / 2,
            },
            Tier::Bot => QueryLimits {
                history_grep: false,
                history_revisions: 0,
                history_candidates: 0,
                history_blob_bytes: 0,
                history_parses: 0,
                history_journal_bytes: 0,
                grep_files: browser.grep_files / 5,
                grep_bytes: browser.grep_bytes / 8,
            },
            Tier::Authenticated => QueryLimits {
                history_grep: true,
                history_revisions: browser.history_revisions * 10,
                history_candidates: browser.history_candidates * 10,
                history_blob_bytes: browser.history_blob_bytes * 8,
                history_parses: browser.history_parses * 10,
                history_journal_bytes: browser.history_journal_bytes * 8,
                grep_files: browser.grep_files * 4,
                grep_bytes: browser.grep_bytes * 4,
            },
            Tier::Local => QueryLimits::unlimited(),
        }
    }

    pub fn unlimited() -> QueryLimits {
        QueryLimits {
            history_grep: true,
            history_revisions: u64::MAX,
            history_candidates: u64::MAX,
            history_blob_bytes: u64::MAX,
            history_parses: u64::MAX,
            history_journal_bytes: u64::MAX,
            grep_files: u64::MAX,
            grep_bytes: u64::MAX,
        }
    }

    /// The limit named `name` (a field's name), as a number (`history_grep`
    /// as 0 or 1).
    pub fn get(&self, name: &str) -> Option<u64> {
        match serde_json::to_value(self).ok()?.get(name)? {
            Value::Bool(b) => Some(*b as u64),
            value => value.as_u64(),
        }
    }
}

/// The config's `query_limits`: each tier's overrides of its defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueryLimitsConfig(pub BTreeMap<Tier, Map<String, Value>>);

impl QueryLimitsConfig {
    /// `tier`'s limits: its defaults, with this config's overrides, and
    /// then `overrides`' (ex: SEARCHFOX_QUERY_LIMITS's).  Unknown names are
    /// warned about and left out.
    pub fn for_tier(&self, tier: Tier, overrides: Option<&QueryLimitsConfig>) -> QueryLimits {
        let defaults = QueryLimits::default_for(tier);
        let mut value = serde_json::to_value(&defaults).unwrap();
        let fields = value.as_object_mut().unwrap();
        for config in std::iter::once(self).chain(overrides) {
            for (name, limit) in config.0.get(&tier).into_iter().flatten() {
                if fields.contains_key(name) {
                    fields.insert(name.clone(), limit.clone());
                } else {
                    eprintln!("query_limits: no limit named {:?} (for {:?})", name, tier);
                }
            }
        }
        serde_json::from_value(value).unwrap_or_else(|e| {
            eprintln!(
                "query_limits: bad limits for {:?} ({}); using the defaults",
                tier, e
            );
            defaults
        })
    }
}

/// The server's configured limits (see `init`), and SEARCHFOX_QUERY_LIMITS.
static CONFIGURED: OnceLock<(QueryLimitsConfig, Option<QueryLimitsConfig>)> = OnceLock::new();

/// Set the server's limits from its config's `query_limits` (once; later
/// calls are ignored).
pub fn init(config: QueryLimitsConfig) {
    let env = std::env::var("SEARCHFOX_QUERY_LIMITS")
        .ok()
        .and_then(|value| match serde_json::from_str(&value) {
            Ok(env) => Some(env),
            Err(e) => {
                eprintln!("SEARCHFOX_QUERY_LIMITS isn't query limits: {}", e);
                None
            }
        });
    let _ = CONFIGURED.set((config, env));
}

/// `tier`'s limits on this server (see `init`).
pub fn limits_for(tier: Tier) -> QueryLimits {
    match CONFIGURED.get() {
        Some((config, env)) => config.for_tier(tier, env.as_ref()),
        None => QueryLimitsConfig::default().for_tier(tier, None),
    }
}

/// A limit a plan's estimate went over.
#[derive(Clone, Debug, Serialize)]
pub struct Exceeded {
    /// The limit's name (see `QueryLimits`).
    pub limit: String,
    pub estimate: u64,
    pub allowed: u64,
}

/// A query refused because its plan would cost more than its tier's limits
/// allow (see `ServerError::TooExpensive`).
#[derive(Clone, Debug, Serialize)]
pub struct TooExpensive {
    /// What was refused, ex: "re-history".
    pub what: String,
    /// The limits its estimates went over.
    pub exceeded: Vec<Exceeded>,
    /// What the plan found (ex: the tokens it considered), for the
    /// explanation.
    pub details: Vec<String>,
    /// How the query could cost less.
    pub hints: Vec<String>,
    pub tier: Tier,
    /// Whether a signed-in person's limits would allow it.
    pub signed_in_allows: bool,
}

impl TooExpensive {
    /// Check `estimates` (limits' names and estimates) against `limits`, of
    /// `tier`: a `TooExpensive` for those over them, if any.
    pub fn check(
        what: &str,
        estimates: &[(&str, u64)],
        limits: &QueryLimits,
        tier: Tier,
    ) -> Option<TooExpensive> {
        let over = |limits: &QueryLimits| -> Vec<Exceeded> {
            estimates
                .iter()
                .filter_map(|&(name, estimate)| {
                    let allowed = limits.get(name).unwrap_or(0);
                    (estimate > allowed).then(|| Exceeded {
                        limit: name.to_string(),
                        estimate,
                        allowed,
                    })
                })
                .collect()
        };
        let exceeded = over(limits);
        if exceeded.is_empty() {
            return None;
        }
        let signed_in_allows = !matches!(tier, Tier::Authenticated | Tier::Local)
            && over(&limits_for(Tier::Authenticated)).is_empty();
        Some(TooExpensive {
            what: what.to_string(),
            exceeded,
            details: vec![],
            hints: vec![],
            tier,
            signed_in_allows,
        })
    }

    /// A one-paragraph explanation: the limits it went over, or, if it's
    /// too expensive whatever the limits, its details.
    pub fn message(&self) -> String {
        if self.exceeded.is_empty() {
            return format!(
                "This {} would be too expensive for the server: {}.",
                self.what,
                self.details.join("; ")
            );
        }
        let over: Vec<String> = self
            .exceeded
            .iter()
            .map(|e| {
                if e.allowed == 0 {
                    format!("{} isn't allowed for {:?} requests", e.limit, self.tier)
                } else {
                    format!(
                        "{} would be {}, over the limit of {}",
                        e.limit, e.estimate, e.allowed
                    )
                }
            })
            .collect();
        format!(
            "This {} would be too expensive for the server: {}.",
            self.what,
            over.join("; ")
        )
    }

    /// The 422 response's JSON body.
    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "error": "query-too-expensive",
            "message": self.message(),
        });
        if let (Some(fields), Ok(Value::Object(details))) =
            (value.as_object_mut(), serde_json::to_value(self))
        {
            fields.extend(details);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiers_and_overrides() {
        assert_eq!(Tier::of(None), Tier::Local);
        assert_eq!(Tier::of(Some(RequestClass::Agent)), Tier::Agent);
        let config: QueryLimitsConfig = serde_json::from_value(json!({
            "browser": { "history_candidates": 4000, "nonsense": 1 },
            "bot": { "history_grep": true },
        }))
        .unwrap();
        let browser = config.for_tier(Tier::Browser, None);
        assert_eq!(browser.history_candidates, 4000);
        // (The rest are the browser's defaults.)
        assert_eq!(
            browser.history_parses,
            QueryLimits::default_for(Tier::Browser).history_parses
        );
        // (An agent's are its own defaults.)
        assert_eq!(
            config.for_tier(Tier::Agent, None),
            QueryLimits::default_for(Tier::Agent)
        );
        assert!(config.for_tier(Tier::Bot, None).history_grep);
        // (The environment's overrides win.)
        let env: QueryLimitsConfig =
            serde_json::from_value(json!({ "browser": { "history_candidates": 10 } })).unwrap();
        assert_eq!(
            config
                .for_tier(Tier::Browser, Some(&env))
                .history_candidates,
            10
        );
        assert_eq!(QueryLimits::unlimited().get("history_grep"), Some(1));
    }

    #[test]
    fn test_too_expensive() {
        let limits = QueryLimits::default_for(Tier::Browser);
        assert!(
            TooExpensive::check(
                "re-history",
                &[("history_candidates", 10)],
                &limits,
                Tier::Browser
            )
            .is_none()
        );
        let refused = TooExpensive::check(
            "re-history",
            &[("history_candidates", 3_000), ("history_blob_bytes", 1)],
            &limits,
            Tier::Browser,
        )
        .unwrap();
        assert_eq!(refused.exceeded.len(), 1);
        assert!(refused.signed_in_allows);
        let json = refused.to_json();
        assert_eq!(json["error"], "query-too-expensive");
        assert_eq!(json["exceeded"][0]["limit"], "history_candidates");
        assert!(
            json["message"]
                .as_str()
                .unwrap()
                .contains("over the limit of 2000")
        );
        // (Bots' history greps aren't allowed at all.)
        let bot = TooExpensive::check(
            "re-history",
            &[("history_grep", 1)],
            &QueryLimits::default_for(Tier::Bot),
            Tier::Bot,
        )
        .unwrap();
        assert!(bot.message().contains("isn't allowed for Bot requests"));
    }
}
