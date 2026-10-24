//! Admission of the web-server's and the pipeline-server's requests, for
//! fairness under load (ex: AI crawlers): each request is from a class (see
//! `RequestClass`, by its user agent) and a client (its address's /24, or /48
//! for IPv6, from the forwarding headers), and each client has a token bucket
//! per class, which its requests take tokens from (pages more than their data
//! files): a client out of tokens waits for them, up to its class's
//! `max_wait`, or is turned away (with a 429 and `Retry-After`).  A client
//! whose requests have been sustained without a gap for long (as crawlers'
//! are, unlike people's and their tools' sessions, which pause) refills at
//! half the rate, and counts as a bot for the classes' shares of the server's
//! concurrency (see `concurrency_limit`).  Requests without forwarding headers
//! (ex: from the web server itself, or tests) aren't limited.  Each server
//! has its own buckets (see `admit_request`).
//!
//! The data API's requests (see `data_api`) have their own buckets (see
//! `Pool`), so that a tool's bursts of them don't use up its person's pages',
//! and take a small cost up front and then their processing time (see
//! `Admitted::charge`): what they cost the server is its CPU and memory, which
//! vary (a crossref lookup is cheap, a big analysis file isn't), and which
//! are easier to measure than to predict.  A client can go into debt (up to
//! its bucket's capacity) for its last request, which its next ones wait for.
//!
//! Clients are by `Fastly-Client-IP` (which Fastly sets, unless the request
//! has one already, which its VCL can prevent) or else `X-Forwarded-For`
//! (whose first address the client can choose), which nginx passes on: so a
//! client can evade its bucket by lying about its address, as one with many
//! addresses can, but those are the WAF's to stop; this is for fairness
//! between the others.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use hyper::http::HeaderMap;
use tokio::sync::{Semaphore, SemaphorePermit};

/// Who a request is from, by its user agent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestClass {
    /// A browser (or anything not identifying itself as one of the others).
    Browser,
    /// A tool acting for a person which identifies itself, ex: searchfox-cli
    /// or an AI assistant's fetches.
    Agent,
    /// A crawler, or a script (ex: curl).
    Bot,
}

/// Substrings (lowercased) of the user agents of agents and bots.  (Agents
/// first: some agents' user agents mention "bot".)
const AGENTS: &[&str] = &[
    "searchfox-cli",
    "searchfox-remote",
    "claude-user",
    "claude-code",
    "chatgpt-user",
    "perplexity-user",
    "mistralai-user",
];
const BOTS: &[&str] = &[
    "bot",
    "crawl",
    "spider",
    "slurp",
    "curl/",
    "wget/",
    "python-",
    "httpx",
    "go-http-client",
    "java/",
    "okhttp",
    "scrapy",
    "libwww",
    "headless",
];

impl RequestClass {
    pub fn of(user_agent: &str) -> RequestClass {
        let user_agent = user_agent.to_ascii_lowercase();
        if AGENTS.iter().any(|agent| user_agent.contains(agent)) {
            RequestClass::Agent
        } else if user_agent.is_empty() || BOTS.iter().any(|bot| user_agent.contains(bot)) {
            RequestClass::Bot
        } else {
            RequestClass::Browser
        }
    }

    /// The bucket's capacity, refill rate (tokens per second), and how long a
    /// request can wait for tokens.
    fn bucket(self) -> (f64, f64, Duration) {
        match self {
            RequestClass::Browser => (60.0, 2.0, Duration::from_secs(10)),
            RequestClass::Agent => (30.0, 1.0, Duration::from_secs(20)),
            RequestClass::Bot => (10.0, 0.2, Duration::from_secs(5)),
        }
    }

    /// How many of the server's concurrent requests (of the 4 each server
    /// processes at once) the class can have.
    pub const fn concurrency_limit(self) -> usize {
        match self {
            RequestClass::Browser => 4,
            RequestClass::Agent => 2,
            RequestClass::Bot => 1,
        }
    }
}

/// Which of a client's buckets a request takes its tokens from, by its path.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Pool {
    /// Pages and their data files (see `cost`), and queries.
    Pages,
    /// The data API's requests (`/{tree}/api/...`; see `data_api`).
    Api,
}

impl Pool {
    pub fn of(path: &str) -> Pool {
        if path.split('/').nth(2) == Some("api") {
            Pool::Api
        } else {
            Pool::Pages
        }
    }

    /// How many tokens a second of a request's processing takes (see
    /// `Admitted::charge`).
    fn tokens_per_second(self) -> f64 {
        match self {
            Pool::Pages => 0.0,
            Pool::Api => 1.0,
        }
    }
}

/// A gap this long between a client's requests ends a sustained run.
const GAP: Duration = Duration::from_secs(60);
/// A run of requests this long without a gap is sustained (see the module's
/// comment).
const SUSTAINED: Duration = Duration::from_secs(15 * 60);
/// Clients idle this long are forgotten (when there are many).
const FORGET_AFTER: Duration = Duration::from_secs(60 * 60);
const MANY_CLIENTS: usize = 100_000;

struct Client {
    tokens: f64,
    refilled: Instant,
    last_request: Instant,
    run_start: Instant,
    /// How many of its requests have been turned away since it was last
    /// admitted.
    rejections: u64,
}

/// The decision about a request.
#[derive(Debug, PartialEq)]
pub enum Admission {
    /// Go ahead, as this class (bots' for a sustained client's).
    Admit(RequestClass),
    /// Wait this long, then ask again.
    Wait(Duration),
    /// Turn it away; the client may retry after this long.  (With how many
    /// of its requests have been turned away in a row, this one included.)
    Reject(Duration, u64),
}

#[derive(Default)]
pub struct Admissions {
    clients: Mutex<HashMap<(String, RequestClass, Pool), Client>>,
}

pub static ADMISSIONS: LazyLock<Admissions> = LazyLock::new(Admissions::default);

/// The client a request is from, by its forwarding headers' first address
/// (Fastly's client IP, else X-Forwarded-For's first): its /24 (or /48), or
/// None if it has none (ex: a local request).
pub fn client_key(fastly_client_ip: Option<&str>, forwarded_for: Option<&str>) -> Option<String> {
    let address = fastly_client_ip
        .or_else(|| forwarded_for.and_then(|header| header.split(',').next()))?
        .trim();
    if let Ok(ip) = address.parse::<std::net::Ipv4Addr>() {
        let [a, b, c, _] = ip.octets();
        return Some(format!("{}.{}.{}.0/24", a, b, c));
    }
    if let Ok(ip) = address.parse::<std::net::Ipv6Addr>() {
        let s = ip.segments();
        return Some(format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2]));
    }
    None
}

impl Admissions {
    /// Whether a request costing `cost` tokens, from `client` (see
    /// `client_key`) of `class`, for `pool`, may go ahead now (taking its
    /// tokens), after waiting `waited` so far.
    pub fn admit(
        &self,
        client: &str,
        class: RequestClass,
        pool: Pool,
        cost: f64,
        waited: Duration,
        now: Instant,
    ) -> Admission {
        let (capacity, rate, max_wait) = class.bucket();
        let mut clients = self.clients.lock().unwrap();
        if clients.len() > MANY_CLIENTS {
            clients.retain(|_, c| now.duration_since(c.last_request) < FORGET_AFTER);
        }
        let state = clients
            .entry((client.to_string(), class, pool))
            .or_insert_with(|| Client {
                tokens: capacity,
                refilled: now,
                last_request: now,
                run_start: now,
                rejections: 0,
            });
        // (A request after a gap starts a new run; but waiting isn't a gap.)
        if waited.is_zero() && now.duration_since(state.last_request) >= GAP {
            state.run_start = now;
        }
        let sustained = now.duration_since(state.run_start) >= SUSTAINED;
        let rate = if sustained { rate / 2.0 } else { rate };
        let elapsed = now.duration_since(state.refilled).as_secs_f64();
        state.tokens = (state.tokens + elapsed * rate).min(capacity);
        state.refilled = now;
        if state.tokens >= cost {
            state.tokens -= cost;
            state.last_request = now;
            state.rejections = 0;
            return Admission::Admit(if sustained { RequestClass::Bot } else { class });
        }
        let needed = Duration::from_secs_f64((cost - state.tokens) / rate);
        if waited + needed > max_wait {
            state.rejections += 1;
            Admission::Reject(needed, state.rejections)
        } else {
            Admission::Wait(needed)
        }
    }

    /// Take `tokens` more from the bucket of `client`'s requests of `class`
    /// for `pool` (after its request; see `Admitted::charge`), down to minus
    /// its capacity.
    pub fn charge(&self, client: &str, class: RequestClass, pool: Pool, tokens: f64) {
        let (capacity, _, _) = class.bucket();
        let mut clients = self.clients.lock().unwrap();
        if let Some(state) = clients.get_mut(&(client.to_string(), class, pool)) {
            state.tokens = (state.tokens - tokens).max(-capacity);
        }
    }
}

/// How long a request took to process (inside the server's concurrency
/// limit, so not counting its waiting), which the pipeline-server's responses
/// carry as an extension for `Admitted::charge`.
#[derive(Clone, Copy, Debug)]
pub struct Processing(pub Duration);

/// How many tokens more to take from the request's bucket, which an
/// expensive query's response carries as an extension (its time's worth, at
/// `EXPENSIVE_TOKENS_PER_SECOND`; see `PipelineCommand::set_limits`), so that
/// a client's expensive queries (ex: history greps) slow its next ones.
#[derive(Clone, Copy, Debug)]
pub struct Charge(pub f64);

/// How many tokens a second of an expensive query's processing takes (as the
/// data API's requests' do; see `Pool::tokens_per_second`).
pub const EXPENSIVE_TOKENS_PER_SECOND: f64 = 1.0;

/// An admitted request (see `admit_request`): its class, with the permit to
/// hold while it's processed, and its bucket, for charging it afterwards.
pub struct Admitted {
    pub class: RequestClass,
    pub permit: Option<SemaphorePermit<'static>>,
    /// Its client and the class of its bucket (not a sustained client's bot
    /// class), and the pool, if it's from outside.
    bucket: Option<(String, RequestClass, Pool)>,
}

impl Admitted {
    /// Whether it's from outside (see `client_key`), rather than from the web
    /// server itself or tests.
    pub fn from_outside(&self) -> bool {
        self.bucket.is_some()
    }

    /// Charge the request's bucket for its processing time, if its pool
    /// charges for that (see `Pool::tokens_per_second`).
    pub fn charge(&self, processing: Duration) {
        self.charge_tokens(processing.as_secs_f64() * self.bucket_pool_rate());
    }

    fn bucket_pool_rate(&self) -> f64 {
        self.bucket
            .as_ref()
            .map_or(0.0, |(_, _, pool)| pool.tokens_per_second())
    }

    /// Take `tokens` more from the request's bucket (see `Charge`).
    pub fn charge_tokens(&self, tokens: f64) {
        if let Some((client, class, pool)) = &self.bucket
            && tokens > 0.0
        {
            ADMISSIONS.charge(client, *class, *pool, tokens);
        }
    }
}

/// Agents' and bots' shares of the server's concurrent requests (see
/// `RequestClass::concurrency_limit`).
static AGENT_SEMAPHORE: Semaphore = Semaphore::const_new(RequestClass::Agent.concurrency_limit());
static BOT_SEMAPHORE: Semaphore = Semaphore::const_new(RequestClass::Bot.concurrency_limit());

/// Admit a request for `path` with `headers`, after waiting for its client's
/// tokens (see `Admissions::admit`), if it's from outside (see `client_key`):
/// its class, with a permit for its class's share of the server's
/// concurrency (for agents and bots), to hold while it's processed (and
/// taken before the server's own); or, if it's turned away, how long until
/// it may retry (for a 429's `Retry-After`).
pub async fn admit_request(headers: &HeaderMap, path: &str) -> Result<Admitted, Duration> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let mut class = RequestClass::of(header("user-agent").unwrap_or_default());
    let mut bucket = None;
    if let Some(client) = client_key(header("fastly-client-ip"), header("x-forwarded-for")) {
        let pool = Pool::of(path);
        let cost = cost(path);
        let start = Instant::now();
        loop {
            match ADMISSIONS.admit(&client, class, pool, cost, start.elapsed(), Instant::now()) {
                Admission::Admit(admitted) => {
                    bucket = Some((client.clone(), class, pool));
                    class = admitted;
                    break;
                }
                Admission::Wait(wait) => tokio::time::sleep(wait).await,
                Admission::Reject(retry, rejections) => {
                    // (Only the 1st, 2nd, 4th, 8th, ... in a row, so that a
                    // crawler's don't fill the log.)
                    if rejections.is_power_of_two() {
                        println!(
                            "Turned away {:?} {} for {} ({} in a row)",
                            class, client, path, rejections
                        );
                    }
                    return Err(Duration::from_secs(retry.as_secs().max(1)));
                }
            }
        }
    }
    let permit = match class {
        RequestClass::Browser => None,
        RequestClass::Agent => Some(AGENT_SEMAPHORE.acquire().await.unwrap()),
        RequestClass::Bot => Some(BOT_SEMAPHORE.acquire().await.unwrap()),
    };
    Ok(Admitted {
        class,
        permit,
        bucket,
    })
}

/// How many tokens a request for `path` takes up front: pages 1, their data
/// files (which nginx usually serves from sidecar files) less, and the data
/// API's requests a little (and then their processing time; see `Pool`).
pub fn cost(path: &str) -> f64 {
    match path.split('/').nth(2) {
        Some("rev-hyperblame") => 0.2,
        Some("api") => 0.1,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classes() {
        let firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:150.0) Gecko/20100101 Firefox/150.0";
        assert_eq!(RequestClass::of(firefox), RequestClass::Browser);
        assert_eq!(RequestClass::of("searchfox-cli/0.3"), RequestClass::Agent);
        assert_eq!(
            RequestClass::of("Mozilla/5.0 (compatible; GPTBot/1.2)"),
            RequestClass::Bot
        );
        assert_eq!(RequestClass::of("curl/8.5.0"), RequestClass::Bot);
        assert_eq!(RequestClass::of(""), RequestClass::Bot);
    }

    #[test]
    fn test_client_key() {
        assert_eq!(client_key(None, None), None);
        assert_eq!(
            client_key(None, Some("203.0.113.7, 10.0.0.1")),
            Some("203.0.113.0/24".to_string())
        );
        assert_eq!(
            client_key(Some("2001:db8:1:2::5"), Some("203.0.113.7")),
            Some("2001:db8:1::/48".to_string())
        );
    }

    #[test]
    fn test_buckets() {
        let admissions = Admissions::default();
        let start = Instant::now();
        let bot = RequestClass::Bot;
        // A bot's 10 tokens, then waits (5 s for a token at 0.2/s), which it
        // can't, so it's turned away.
        for _ in 0..10 {
            assert_eq!(
                admissions.admit("a", bot, Pool::Pages, 1.0, Duration::ZERO, start),
                Admission::Admit(bot)
            );
        }
        assert_eq!(
            admissions.admit("a", bot, Pool::Pages, 1.0, Duration::ZERO, start),
            Admission::Wait(Duration::from_secs(5))
        );
        assert!(matches!(
            admissions.admit("a", bot, Pool::Pages, 1.0, Duration::from_secs(1), start),
            Admission::Reject(_, 1)
        ));
        assert!(matches!(
            admissions.admit("a", bot, Pool::Pages, 1.0, Duration::from_secs(1), start),
            Admission::Reject(_, 2)
        ));
        // After 5 s, it has one again.
        let later = start + Duration::from_secs(5);
        assert_eq!(
            admissions.admit("a", bot, Pool::Pages, 1.0, Duration::ZERO, later),
            Admission::Admit(bot)
        );
        // Other clients have their own buckets.
        assert_eq!(
            admissions.admit("b", bot, Pool::Pages, 1.0, Duration::ZERO, start),
            Admission::Admit(bot)
        );
    }

    #[test]
    fn test_api_pool() {
        let admissions = Admissions::default();
        let start = Instant::now();
        let agent = RequestClass::Agent;
        assert_eq!(Pool::of("/tests/api/v1/crossref"), Pool::Api);
        assert_eq!(Pool::of("/tests/source/a.cpp"), Pool::Pages);
        // An agent's 30 pages leave its API requests' bucket full.
        for _ in 0..30 {
            admissions.admit("d", agent, Pool::Pages, 1.0, Duration::ZERO, start);
        }
        assert_eq!(
            admissions.admit("d", agent, Pool::Api, 0.1, Duration::ZERO, start),
            Admission::Admit(agent)
        );
        // A long request puts it in debt (down to minus its capacity), which
        // its next request waits for, or is turned away.
        admissions.charge("d", agent, Pool::Api, 1000.0);
        assert!(matches!(
            admissions.admit("d", agent, Pool::Api, 0.1, Duration::ZERO, start),
            Admission::Reject(_, 1)
        ));
        // (30 s to pay its debt of 30 tokens at 1 a second.)
        let later = start + Duration::from_secs(31);
        assert_eq!(
            admissions.admit("d", agent, Pool::Api, 0.1, Duration::ZERO, later),
            Admission::Admit(agent)
        );
    }

    #[test]
    fn test_sustained() {
        let admissions = Admissions::default();
        let start = Instant::now();
        let browser = RequestClass::Browser;
        // A request every 10 s for 15 minutes is sustained: it counts as a
        // bot.
        let mut now = start;
        let mut last = None;
        while now <= start + SUSTAINED {
            last = Some(admissions.admit("c", browser, Pool::Pages, 1.0, Duration::ZERO, now));
            now += Duration::from_secs(10);
        }
        assert_eq!(last, Some(Admission::Admit(RequestClass::Bot)));
        // After a gap, it's a browser's again.
        now += GAP;
        assert_eq!(
            admissions.admit("c", browser, Pool::Pages, 1.0, Duration::ZERO, now),
            Admission::Admit(browser)
        );
    }
}
