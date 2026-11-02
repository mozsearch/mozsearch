//! Crash sets: the crash reports a crash-stats search found, fetched once and
//! stored, and their analysis, for the `crash-set:` view (see
//! `render_crash_set`): what each crash's threads were busy with (see
//! `digest`), and the set's families (by which components' code its threads
//! other than the crashing one were busy in, and shutdown hangs' pending
//! blockers) and their variants (by which functions).  See
//! asuth-gecko-notes' tooling/searchfox/crash-sets-plan.md for the design:
//! nothing a user writes is stored (a set is a search's results, its ID its
//! manifest's hash, and a search finding nothing stores nothing), and the
//! next phases (the web server's ingestion with a user's token, lifetimes,
//! the carry-over across re-indexes).
//!
//! The store is a directory (`crash_sets_path` in the config, else
//! `crash-sets/` beside the trees' index directories; see
//! `AbstractServer::crash_sets`):
//! - `sets/ID.json`: a set's manifest (see `CrashSetManifest`), whose ID is
//!   its hash (see `set_id`);
//! - `crashes/AB/CRASH-ID.json.gz` (or `.json`, ex: tests' fixtures): each
//!   crash's whitelisted fields (see `whitelist`), once however many sets
//!   have it, which the crash view reads too (see `LocalIndex::crash_report`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::crash_stack::{classify_threads, is_crash_id, parse_report, url_encode_query};
use crate::crash_threads::{FrameRole, ThreadRules, ThreadState, rule_name, slug};

/// The manifests' format's version.
pub const MANIFEST_VERSION: u32 = 1;
/// How many crashes a set has at most.
pub const MAX_CRASHES: usize = 500;
/// How many days a search's dates span at most.
pub const MAX_DAYS: i64 = 30;
/// How many of a busy thread's own frames its digest has (see `BusyThread`).
const TOP_FRAMES: usize = 3;
/// How many of the crashing thread's own frames its digest has.
const CRASHING_FRAMES: usize = 16;
/// crash-stats' API.
pub const CRASH_STATS_API: &str = "https://crash-stats.mozilla.org/api";

/// A crash-stats search (SuperSearch's), of the parameters searchfox allows:
/// a signature (exactly) or a piece of proto signatures (which contain it:
/// the frames further up the stack than the signature's), the product, and
/// optionally release channels, versions, platforms (ex: "Windows NT") and
/// process types (ex: "parent"), the dates (`date_from` up to `date_to`,
/// `YYYY-MM-DD`), and how many crashes (the latest).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CrashSearch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proto_signature: Option<String>,
    pub product: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub release_channels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub process_types: Vec<String>,
    pub date_from: String,
    pub date_to: String,
    pub limit: usize,
}

impl CrashSearch {
    /// Whether it's a search searchfox fetches (see `CrashSearch`): with a
    /// signature or a proto signature's piece, the product, dates at most
    /// `MAX_DAYS` apart, and at most `MAX_CRASHES` crashes.
    pub fn check(&self) -> Result<(), String> {
        let text = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
        if text(&self.signature) == text(&self.proto_signature) {
            return Err(
                "A crash set's search is for a signature or a proto signature's piece".to_string(),
            );
        }
        if self.product.trim().is_empty() {
            return Err("A crash set's search needs its product".to_string());
        }
        let date = |date: &str| {
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| format!("{:?} isn't a date (YYYY-MM-DD)", date))
        };
        let (from, to) = (date(&self.date_from)?, date(&self.date_to)?);
        if to <= from || (to - from).num_days() > MAX_DAYS {
            return Err(format!(
                "A crash set's dates are from one day up to another at most {} days later",
                MAX_DAYS
            ));
        }
        if self.limit == 0 || self.limit > MAX_CRASHES {
            return Err(format!("A crash set has from 1 to {} crashes", MAX_CRASHES));
        }
        Ok(())
    }

    /// SuperSearch's parameters for it (but what to return).
    pub fn params(&self) -> Vec<(&'static str, String)> {
        let mut params = vec![("product", self.product.clone())];
        if let Some(signature) = &self.signature {
            params.push(("signature", format!("={}", signature)));
        }
        if let Some(piece) = &self.proto_signature {
            params.push(("proto_signature", format!("~{}", piece)));
        }
        for (name, values) in [
            ("release_channel", &self.release_channels),
            ("version", &self.versions),
            ("platform", &self.platforms),
            ("process_type", &self.process_types),
        ] {
            params.extend(values.iter().map(|value| (name, value.clone())));
        }
        params.push(("date", format!(">={}", self.date_from)));
        params.push(("date", format!("<{}", self.date_to)));
        params
    }

    /// crash-stats' search page of it, for people to check it.
    pub fn search_url(&self) -> String {
        let query: Vec<String> = self
            .params()
            .iter()
            .map(|(name, value)| format!("{}={}", name, url_encode_query(value)))
            .collect();
        format!(
            "https://crash-stats.mozilla.org/search/?{}",
            query.join("&")
        )
    }

    /// What it's for, in a few words (ex: "the signature `X`"), as HTML.
    pub fn about_html(&self) -> String {
        use crate::rendered_docs::escape;
        let mut about = match (&self.signature, &self.proto_signature) {
            (Some(signature), _) => format!("the signature <code>{}</code>", escape(signature)),
            (None, Some(piece)) => {
                format!("proto signatures containing <code>{}</code>", escape(piece))
            }
            (None, None) => "crashes".to_string(),
        };
        let mut filters = vec![escape(&self.product)];
        for values in [
            &self.release_channels,
            &self.versions,
            &self.platforms,
            &self.process_types,
        ] {
            if !values.is_empty() {
                filters.push(escape(&values.join(" or ")));
            }
        }
        filters.push(format!(
            "{} to {}",
            escape(&self.date_from),
            escape(&self.date_to)
        ));
        about.push_str(&format!(" ({})", filters.join(", ")));
        about
    }
}

/// A crash set's manifest: its manifest's format's version, its search, when
/// it was fetched, how many crashes crash-stats had for it then (`total`, of
/// which the set has the latest), and its crashes' IDs (sorted).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CrashSetManifest {
    pub version: u32,
    pub search: CrashSearch,
    pub fetched_at: String,
    pub total: u64,
    pub crashes: Vec<String>,
}

/// FNV-1a's 64-bit hash of `bytes`: stable across builds, unlike std's.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// A short stable hash of `text`, as hex (ex: a facet's value for a long
/// key).
fn short_hash(text: &str) -> String {
    format!("{:08x}", fnv1a(text.as_bytes()) as u32)
}

/// A set's ID: its manifest's hash, 16 hex digits, so that no one chooses
/// it.  (Not collision resistant, so a set never replaces another's
/// manifest; see `CrashSetStore::store_manifest`.)
pub fn set_id(manifest: &CrashSetManifest) -> String {
    let canonical = serde_json::to_string(manifest).unwrap_or_default();
    format!("{:016x}", fnv1a(canonical.as_bytes()))
}

/// Whether `id` is a set's ID (see `set_id`).
pub fn is_set_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The processed crash's fields the crash view and the analysis use (see
/// the module's documentation), and nothing else whatever crash-stats
/// returned (a whitelist, so that protected data is never stored, even from
/// a token which could see it).
pub fn whitelist(processed: &Value) -> Value {
    const FIELDS: &[&str] = &[
        "uuid",
        "signature",
        "proto_signature",
        "product",
        "version",
        "build",
        "release_channel",
        "os_name",
        "os_pretty_version",
        "cpu_arch",
        "process_type",
        "date_processed",
        "uptime",
        "moz_crash_reason",
        "async_shutdown_timeout",
        "shutdown_progress",
    ];
    const FRAME: &[&str] = &[
        "frame",
        "function",
        "file",
        "line",
        "module",
        "module_offset",
        "offset",
        "trust",
    ];
    const INLINE: &[&str] = &["function", "file", "line"];
    let pick = |value: &Value, keys: &[&str]| -> Value {
        Value::Object(
            keys.iter()
                .filter_map(|key| value.get(*key).map(|v| (key.to_string(), v.clone())))
                .collect(),
        )
    };
    let mut out = pick(processed, FIELDS);
    let dump = &processed["json_dump"];
    let threads: Vec<Value> = dump["threads"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|thread| {
            let frames: Vec<Value> = thread["frames"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|frame| {
                    let mut kept = pick(frame, FRAME);
                    if let Some(inlines) = frame["inlines"].as_array() {
                        kept["inlines"] =
                            inlines.iter().map(|inline| pick(inline, INLINE)).collect();
                    }
                    kept
                })
                .collect();
            let mut kept = pick(thread, &["thread_name"]);
            kept["frames"] = Value::Array(frames);
            kept
        })
        .collect();
    let modules: Vec<Value> = dump["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|module| pick(module, &["filename", "version", "debug_id"]))
        .collect();
    out["json_dump"] = json!({
        "crash_info": pick(&dump["crash_info"], &["type", "address", "crashing_thread"]),
        "threads": threads,
        "modules": modules,
    });
    out
}

/// A crash set store (see the module's documentation).
#[derive(Clone, Debug)]
pub struct CrashSetStore {
    root: PathBuf,
}

/// Writes `contents` to `path` (through a temporary file renamed into
/// place, so that readers never see a part of it).
fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("no directory")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {}", parent.display(), e))?;
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&temporary, contents).map_err(|e| format!("{}: {}", temporary.display(), e))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("{}: {}", path.display(), e))
}

impl CrashSetStore {
    pub fn new(root: impl Into<PathBuf>) -> CrashSetStore {
        CrashSetStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn crash_path(&self, id: &str, extension: &str) -> PathBuf {
        self.root
            .join("crashes")
            .join(&id[..2])
            .join(format!("{}.{}", id, extension))
    }

    /// The stored crash `id`, if the store has it.
    pub fn crash(&self, id: &str) -> Result<Option<Value>, String> {
        if !is_crash_id(id) {
            return Err(format!("{:?} isn't a crash report's ID", id));
        }
        let path = self.crash_path(id, "json.gz");
        let text = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut text = String::new();
                flate2::read::GzDecoder::new(file)
                    .read_to_string(&mut text)
                    .map_err(|e| format!("{}: {}", path.display(), e))?;
                text
            }
            Err(_) => match std::fs::read_to_string(self.crash_path(id, "json")) {
                Ok(text) => text,
                Err(_) => return Ok(None),
            },
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("crash {}: {}", id, e))
    }

    /// Whether the store has the crash `id`.
    pub fn has_crash(&self, id: &str) -> bool {
        is_crash_id(id)
            && (self.crash_path(id, "json.gz").exists() || self.crash_path(id, "json").exists())
    }

    /// Stores the processed crash `id`'s whitelisted fields (see
    /// `whitelist`).
    pub fn store_crash(&self, id: &str, processed: &Value) -> Result<(), String> {
        if !is_crash_id(id) {
            return Err(format!("{:?} isn't a crash report's ID", id));
        }
        let text = serde_json::to_vec(&whitelist(processed)).map_err(|e| e.to_string())?;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&text).map_err(|e| e.to_string())?;
        let bytes = gz.finish().map_err(|e| e.to_string())?;
        write_atomically(&self.crash_path(id, "json.gz"), &bytes)
    }

    /// The set `id`'s manifest, if the store has it.
    pub fn manifest(&self, id: &str) -> Result<Option<CrashSetManifest>, String> {
        if !is_set_id(id) {
            return Err(format!("{:?} isn't a crash set's ID", id));
        }
        match std::fs::read_to_string(self.root.join("sets").join(format!("{}.json", id))) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| format!("crash set {}: {}", id, e)),
            Err(_) => Ok(None),
        }
    }

    /// Stores `manifest`, and returns its ID (see `set_id`); one with the
    /// same ID but other contents is an error (a hash collision) rather than
    /// replaced.
    pub fn store_manifest(&self, manifest: &CrashSetManifest) -> Result<String, String> {
        let id = set_id(manifest);
        match self.manifest(&id)? {
            Some(existing) if existing == *manifest => return Ok(id),
            Some(_) => return Err(format!("crash set {} exists with other contents", id)),
            None => {}
        }
        let text = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
        write_atomically(&self.root.join("sets").join(format!("{}.json", id)), &text)?;
        Ok(id)
    }
}

/// Checks that `token` (a crash-stats API token) can't see protected data,
/// by asking for protected columns (crash-stats drops those a request may
/// not see, rather than refusing them): any in the hits means it can.
async fn check_unprivileged(
    client: &reqwest::Client,
    api: &str,
    token: &str,
) -> Result<(), String> {
    const PROTECTED: &[&str] = &["url", "email", "user_comments"];
    let since = (chrono::Utc::now() - chrono::Duration::days(2)).format("%Y-%m-%d");
    let mut url = format!(
        "{}/SuperSearch/?product=Firefox&_results_number=20&_facets_size=0&_columns=uuid&date=%3E%3D{}",
        api, since
    );
    for column in PROTECTED {
        url.push_str(&format!("&_columns={}", column));
    }
    let response = client
        .get(&url)
        .header("Auth-Token", token)
        .send()
        .await
        .map_err(|e| format!("Couldn't check the token: {}", e))?;
    match response.status() {
        status if status.is_success() => {}
        reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::FORBIDDEN => return Ok(()),
        status => return Err(format!("Couldn't check the token ({})", status)),
    }
    let hits: Value = response
        .json()
        .await
        .map_err(|e| format!("Couldn't check the token: {}", e))?;
    let privileged = hits["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|hit| PROTECTED.iter().any(|column| hit.get(*column).is_some()));
    match privileged {
        true => Err(
            "That token can see protected crash data: create one without any permissions"
                .to_string(),
        ),
        false => Ok(()),
    }
}

/// GETs crash-stats' `url` as JSON, waiting and trying again after its rate
/// limits' 429s (a few times).
async fn get_json(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<Value, String> {
    for attempt in 1..=5u64 {
        let mut request = client.get(url);
        if let Some(token) = token {
            request = request.header("Auth-Token", token);
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
            continue;
        }
        let response = response.error_for_status().map_err(|e| e.to_string())?;
        return response.json().await.map_err(|e| e.to_string());
    }
    Err(format!("crash-stats kept rate limiting {}", url))
}

/// Fetches the crashes `search` finds from crash-stats (at `api`, see
/// `CRASH_STATS_API`) into `store`, and stores their set's manifest,
/// returning its ID; nothing is stored if it finds none.  `token` (a
/// crash-stats API token without permissions, which this checks first) is
/// only for its rate limits: the search uses it, and the processed crashes'
/// fetches when they're rate limited without it (crash-stats then strips
/// protected data itself, and `whitelist` keeps what the store has to public
/// fields either way).  It's not kept.
pub async fn fetch_set(
    store: &CrashSetStore,
    search: &CrashSearch,
    api: &str,
    token: Option<&str>,
) -> Result<String, String> {
    search.check()?;
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "searchfox/",
            env!("CARGO_PKG_VERSION"),
            " (crash sets)"
        ))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    if let Some(token) = token {
        check_unprivileged(&client, api, token).await?;
    }
    let query: Vec<String> = search
        .params()
        .iter()
        .map(|(name, value)| format!("{}={}", name, url_encode_query(value)))
        .collect();
    let url = format!(
        "{}/SuperSearch/?{}&_columns=uuid&_sort=-date&_facets_size=0&_results_number={}",
        api,
        query.join("&"),
        search.limit
    );
    let found = get_json(&client, &url, token).await?;
    let mut crashes: Vec<String> = found["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|hit| hit["uuid"].as_str())
        .filter(|id| is_crash_id(id))
        .map(str::to_string)
        .collect();
    if crashes.is_empty() {
        return Err("crash-stats found no crashes for that search".to_string());
    }
    crashes.sort();
    crashes.dedup();
    for id in &crashes {
        if store.has_crash(id) {
            continue;
        }
        let url = format!("{}/ProcessedCrash/?crash_id={}", api, id);
        let processed = match get_json(&client, &url, None).await {
            Ok(processed) => processed,
            Err(_) if token.is_some() => get_json(&client, &url, token).await?,
            Err(e) => return Err(e),
        };
        store.store_crash(id, &processed)?;
    }
    let manifest = CrashSetManifest {
        version: MANIFEST_VERSION,
        search: search.clone(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        total: found["total"].as_u64().unwrap_or(crashes.len() as u64),
        crashes,
    };
    store.store_manifest(&manifest)
}

/// A frame of a busy thread (see `BusyThread`): its function (without its
/// arguments; see `rule_name`), and where its code is: its file's directory
/// in the repository (ex: "dom/cache"), or else its module (ex: "ntdll.dll").
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BusyFrame {
    pub function: String,
    pub place: String,
}

/// A thread of a crash which wasn't idle (or its crashing thread): which
/// (its index and label), what it is (its kind) and was doing (its state's
/// label), its own frames from the top (without its infrastructure's; see
/// `crash_threads::FrameRole`), a few of them (more for the crashing
/// thread's), and where its own code in the repository was busy (`place`:
/// the directory of its first own frame with a file in the repository which
/// isn't a library's, ex: "dom/cache" rather than SQLite's or mozStorage's
/// it called, see `ThreadRules::is_library`, else of its first with a file),
/// and its functions there (`code`: its first frame with a file, and its
/// place's frame and its two callers), if it has any.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BusyThread {
    pub index: usize,
    pub label: String,
    /// Its name without its number (ex: "IndexedDB IO"), if it has one.
    #[serde(default)]
    pub name: Option<String>,
    pub kind: String,
    pub state: String,
    pub crashing: bool,
    pub frames: Vec<BusyFrame>,
    pub place: Option<String>,
    pub code: Vec<String>,
}

/// A thing a crash's threads were doing (see `CrashDigest::features`): its
/// key (ex: "storage thread in dom/indexedDB", "blocked by IndexedDB"), and
/// its detail (its thread's functions there).
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub key: String,
    pub detail: String,
}

/// What a crash of a set was doing (see `digest`): its report's ID, date,
/// version, platform, process type and signature; a shutdown hang's pending
/// blockers (see `pending_blockers`), its busy threads, and (from its set;
/// see `set_view`) its family's and variant's keys, and its variant's label.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CrashDigest {
    pub id: String,
    pub date: String,
    pub version: String,
    pub platform: String,
    pub process_type: String,
    pub signature: String,
    pub blockers: Vec<String>,
    pub busy: Vec<BusyThread>,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub variant_label: String,
}

impl CrashDigest {
    /// What its threads other than the crashing one were busy with in the
    /// repository's code (their names, or their kinds, and places, ex:
    /// "QuotaManager IO in dom/cache", with their functions there), and its
    /// pending blockers.
    pub fn features(&self) -> Vec<Feature> {
        let mut features: Vec<Feature> = self
            .blockers
            .iter()
            .map(|blocker| Feature {
                key: format!("blocked by {}", blocker),
                detail: String::new(),
            })
            .collect();
        for thread in self.busy.iter().filter(|thread| !thread.crashing) {
            if let Some(place) = &thread.place {
                let who = thread.name.as_deref().unwrap_or(&thread.kind);
                features.push(Feature {
                    key: format!("{} in {}", who, place),
                    detail: format!("{}: {}", who, thread.code.join(" ‹ ")),
                });
            }
        }
        features
    }

    /// Where its crashing thread's code was called from (its first own frame
    /// in another place than its top one's, ex: "crashing thread called from
    /// dom/quota"), with its functions from there.
    pub fn crashing_caller(&self) -> Option<Feature> {
        let thread = self.busy.iter().find(|thread| thread.crashing)?;
        let top = &thread.frames.first()?.place;
        let caller = thread.frames.iter().position(|frame| &frame.place != top);
        let from = caller.unwrap_or(0);
        Some(Feature {
            key: match caller {
                Some(at) => format!("crashing thread called from {}", thread.frames[at].place),
                None => format!("crashing thread in {}", top),
            },
            detail: thread.frames[from..]
                .iter()
                .take(TOP_FRAMES)
                .map(|frame| frame.function.as_str())
                .collect::<Vec<_>>()
                .join(" ‹ "),
        })
    }
}

/// The names of the blockers a shutdown hang's AsyncShutdownTimeout
/// annotation (`async_shutdown_timeout`: its phase's conditions, each a name
/// and a state, which is nested conditions, or what it was at) has pending
/// (not "Complete"), innermost, at most 5.
fn pending_blockers(annotation: &str) -> Vec<String> {
    fn walk(condition: &Value, into: &mut Vec<String>) {
        let name = condition["name"].as_str().unwrap_or_default();
        match &condition["state"] {
            Value::Array(nested) => {
                let before = into.len();
                for nested in nested {
                    walk(nested, into);
                }
                // (A condition whose nested ones are all complete is pending
                // itself.)
                if into.len() == before && !name.is_empty() {
                    into.push(name.to_string());
                }
            }
            Value::String(state) if state == "Complete" => {}
            _ if !name.is_empty() => into.push(name.to_string()),
            _ => {}
        }
    }
    let Ok(timeout) = serde_json::from_str::<Value>(annotation) else {
        return vec![];
    };
    let mut blockers = vec![];
    for condition in timeout["conditions"].as_array().into_iter().flatten() {
        walk(condition, &mut blockers);
    }
    let mut seen = BTreeSet::new();
    blockers.retain(|name| seen.insert(name.clone()));
    blockers.truncate(5);
    blockers
}

/// What the crash `id` (stored, see `CrashSetStore::crash`) was doing, by its
/// threads' classes (see `ThreadRules::classify`): its busy threads (those
/// which weren't idle, and the crashing one) and its pending blockers.  (Its
/// family and variant are its set's to say; see `set_view`.)
pub fn digest(rules: &ThreadRules, id: &str, crash: &Value) -> CrashDigest {
    let (summary, threads) = parse_report(id, crash);
    let crashing = summary
        .crashing_thread
        .as_ref()
        .map_or(usize::MAX, |thread| thread.index);
    let classified = classify_threads(rules, &threads, crashing);
    let mut busy = vec![];
    for (thread, (thread_summary, roles)) in threads.iter().zip(classified) {
        let is_crashing = thread.thread.index == crashing;
        if !is_crashing
            && matches!(
                thread_summary.class.state,
                ThreadState::Idle | ThreadState::Unknown
            )
        {
            continue;
        }
        // (Its own frames, and whether each is in the repository.)
        let own: Vec<(BusyFrame, bool)> = thread
            .frames
            .iter()
            .zip(&roles)
            .filter(|(_, role)| **role == FrameRole::Own)
            .filter_map(|(frame, _)| {
                let function = frame.function.as_deref()?;
                if function == "?" || function.starts_with("<unknown") {
                    return None;
                }
                let (place, in_repository) = match (&frame.source.path, &frame.module) {
                    (Some(path), _) => (
                        path.rsplit_once('/')
                            .map_or_else(|| "(top)".to_string(), |(dir, _)| dir.to_string()),
                        true,
                    ),
                    (None, Some(module)) => (module.clone(), false),
                    (None, None) => ("?".to_string(), false),
                };
                Some((
                    BusyFrame {
                        function: rule_name(function),
                        place,
                    },
                    in_repository,
                ))
            })
            .collect();
        if own.is_empty() {
            continue;
        }
        let in_repository: Vec<usize> = (0..own.len()).filter(|i| own[*i].1).collect();
        let place_at = in_repository
            .iter()
            .copied()
            .find(|i| !rules.is_library(&own[*i].0.place))
            .or(in_repository.first().copied());
        let code: Vec<String> = match (in_repository.first(), place_at) {
            (Some(first), Some(at)) => {
                let mut code = vec![];
                if *first < at {
                    code.push(own[*first].0.function.clone());
                    if *first + 1 < at {
                        code.push("…".to_string());
                    }
                }
                code.extend(
                    own[at..]
                        .iter()
                        .take(TOP_FRAMES)
                        .map(|(frame, _)| frame.function.clone()),
                );
                code
            }
            _ => vec![],
        };
        busy.push(BusyThread {
            index: thread.thread.index,
            label: match &thread.thread.name {
                Some(name) if !name.is_empty() => format!("#{} ({})", thread.thread.index, name),
                _ => format!("#{}", thread.thread.index),
            },
            kind: thread_summary.class.kind.clone(),
            state: thread_summary.class.state.label(),
            crashing: is_crashing,
            place: place_at.map(|at| own[at].0.place.clone()),
            code,
            name: thread
                .thread
                .name
                .as_deref()
                .map(crate::crash_threads::rule_thread_name)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            frames: own
                .into_iter()
                .map(|(frame, _)| frame)
                .take(match is_crashing {
                    true => CRASHING_FRAMES,
                    false => TOP_FRAMES,
                })
                .collect(),
        });
    }
    let blockers = crash["async_shutdown_timeout"]
        .as_str()
        .map(pending_blockers)
        .unwrap_or_default();
    let text = |key: &str| crash[key].as_str().unwrap_or_default().to_string();
    CrashDigest {
        id: id.to_string(),
        date: text("date_processed")
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
        version: text("version"),
        platform: text("os_name"),
        process_type: crash["process_type"]
            .as_str()
            .unwrap_or("parent")
            .to_string(),
        signature: text("signature"),
        blockers,
        busy,
        family: String::new(),
        variant: String::new(),
        variant_label: String::new(),
    }
}

/// A variant of a family (see `Family`): its key, its label (its busy
/// threads' functions), its facet's value, and its crashes (their positions
/// in `CrashSetView::digests`, the latest first).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Variant {
    pub key: String,
    pub label: String,
    pub value: String,
    pub crashes: Vec<usize>,
}

/// A family of a set's crashes (see `set_view`): its key, its facet's value,
/// how many crashes it has, and its variants (the biggest first).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Family {
    pub key: String,
    pub value: String,
    pub crashes: usize,
    pub variants: Vec<Variant>,
}

/// A crash set's analysis: its ID and manifest, its crashes' digests (the
/// latest first; see `digest`), those the store doesn't have any more, what
/// most of its crashes' threads were busy with (`common`, with how many
/// crashes; see `set_view`), and its families (the biggest first).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CrashSetView {
    pub id: String,
    pub manifest: CrashSetManifest,
    pub digests: Vec<CrashDigest>,
    pub missing: Vec<String>,
    pub common: Vec<(String, usize)>,
    pub families: Vec<Family>,
}

/// Of a set's crashes, the share which a feature most of them have is in
/// (see `set_view`), at least.
const COMMON_SHARE: f64 = 0.75;

/// The analysis of the set `id` in `store`, by `rules` (see `CrashSetView`):
/// its crashes' families, by what their threads other than the crashing one
/// were busy with in the repository's code, and their pending blockers (see
/// `CrashDigest::features`), but what most of them (`COMMON_SHARE`) were
/// busy with (ex: a network monitor's thread, in every crash), which says
/// nothing of a crash; or else by where their crashing thread's code was
/// called from (see `CrashDigest::crashing_caller`).  Their variants are by
/// those features' details (their functions).
pub fn set_view(
    store: &CrashSetStore,
    rules: &ThreadRules,
    id: &str,
) -> Result<CrashSetView, String> {
    let manifest = store
        .manifest(id)?
        .ok_or_else(|| format!("There's no crash set {} here (it may have expired)", id))?;
    let mut digests = vec![];
    let mut missing = vec![];
    for crash_id in &manifest.crashes {
        match store.crash(crash_id)? {
            Some(crash) => digests.push(digest(rules, crash_id, &crash)),
            None => missing.push(crash_id.clone()),
        }
    }
    Ok(analyze(id, manifest, digests, missing))
}

/// The analysis of the set `id` (see `set_view`), of its crashes' `digests`
/// (and the IDs of those `missing` from the store).
pub fn analyze(
    id: &str,
    manifest: CrashSetManifest,
    mut digests: Vec<CrashDigest>,
    missing: Vec<String>,
) -> CrashSetView {
    digests.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.id.cmp(&b.id)));

    // What most crashes have (of sets of a few crashes at least).
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for digest in &digests {
        let keys: BTreeSet<String> = digest
            .features()
            .into_iter()
            .map(|feature| feature.key)
            .collect();
        for key in keys {
            *counts.entry(key).or_default() += 1;
        }
    }
    let threshold = ((digests.len() as f64) * COMMON_SHARE).ceil() as usize;
    let mut common: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, count)| digests.len() >= 4 && *count >= threshold.max(2))
        .collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let is_common = |key: &str| common.iter().any(|(common, _)| common == key);

    for digest in digests.iter_mut() {
        let distinctive: Vec<Feature> = digest
            .features()
            .into_iter()
            .filter(|feature| !is_common(&feature.key))
            .collect();
        let (family, label) = match (distinctive.is_empty(), digest.crashing_caller()) {
            (false, _) => {
                let keys: BTreeSet<&str> = distinctive.iter().map(|f| f.key.as_str()).collect();
                let details: BTreeSet<&str> = distinctive
                    .iter()
                    .map(|f| f.detail.as_str())
                    .filter(|detail| !detail.is_empty())
                    .collect();
                (
                    keys.into_iter().collect::<Vec<_>>().join(" + "),
                    details.into_iter().collect::<Vec<_>>().join("; "),
                )
            }
            (true, Some(caller)) => (
                format!("nothing else distinctive busy; {}", caller.key),
                caller.detail,
            ),
            (true, None) => ("nothing distinctive".to_string(), String::new()),
        };
        digest.variant = format!("{} / {}", family, label);
        digest.family = family;
        digest.variant_label = label;
    }

    let mut families: BTreeMap<String, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
    for (at, digest) in digests.iter().enumerate() {
        families
            .entry(digest.family.clone())
            .or_default()
            .entry(digest.variant.clone())
            .or_default()
            .push(at);
    }
    let mut families: Vec<Family> = families
        .into_iter()
        .map(|(key, variants)| {
            let value = format!("f{}", short_hash(&key));
            let mut variants: Vec<Variant> = variants
                .into_iter()
                .map(|(variant, crashes)| Variant {
                    label: digests[crashes[0]].variant_label.clone(),
                    value: format!("{}/v{}", value, short_hash(&variant)),
                    key: variant,
                    crashes,
                })
                .collect();
            variants.sort_by(|a, b| {
                b.crashes
                    .len()
                    .cmp(&a.crashes.len())
                    .then_with(|| a.key.cmp(&b.key))
            });
            Family {
                crashes: variants.iter().map(|variant| variant.crashes.len()).sum(),
                key,
                value,
                variants,
            }
        })
        .collect();
    families.sort_by(|a, b| b.crashes.cmp(&a.crashes).then_with(|| a.key.cmp(&b.key)));
    CrashSetView {
        id: id.to_string(),
        manifest,
        digests,
        missing,
        common,
        families,
    }
}

/// A crash's facets' values (see `set_facets`): its family's and variant's,
/// what its other threads were busy with (see `CrashDigest::features`), its
/// version, platform and process type.
fn crash_facet_values(
    view: &CrashSetView,
    at: usize,
) -> Vec<(&'static str, Vec<(String, String)>)> {
    let digest = &view.digests[at];
    let (family, variant) = view
        .families
        .iter()
        .find_map(|family| {
            family
                .variants
                .iter()
                .find(|variant| variant.crashes.contains(&at))
                .map(|variant| (family, variant))
        })
        .expect("every crash is in a family");
    let busy: BTreeSet<String> = digest
        .features()
        .into_iter()
        .map(|feature| feature.key)
        .collect();
    let one = |value: &str| match value.is_empty() {
        true => vec![("unknown".to_string(), "unknown".to_string())],
        false => vec![(slug(value), value.to_string())],
    };
    vec![
        (
            "family",
            vec![
                (family.value.clone(), family.key.clone()),
                (variant.value.clone(), variant.label.clone()),
            ],
        ),
        (
            "busy",
            match busy.is_empty() {
                true => vec![("none".to_string(), "nothing".to_string())],
                false => busy.iter().map(|key| (slug(key), key.clone())).collect(),
            },
        ),
        ("version", one(&digest.version)),
        ("platform", one(&digest.platform)),
        ("process", one(&digest.process_type)),
    ]
}

/// The navigation panel's facets of a set's view (see `TextFile::facets`),
/// whose items are its crashes' rows: their families (with their variants
/// nested), what their other threads were busy with (common or not), their
/// versions, platforms and process types, with how many crashes have each.
pub fn set_facets(view: &CrashSetView) -> Value {
    use crate::cmd_pipeline::facets::{FacetValueView, FacetView};
    #[derive(Default)]
    struct Level {
        name: String,
        count: u32,
        order: usize,
        nested: BTreeMap<String, Level>,
    }
    let mut roots: BTreeMap<&'static str, Level> = BTreeMap::new();
    for at in 0..view.digests.len() {
        for (key, values) in crash_facet_values(view, at) {
            let root = roots.entry(key).or_default();
            match key {
                // (A family's variants are nested in it.)
                "family" => {
                    let mut level = &mut *root;
                    for (order, (value, name)) in values.into_iter().enumerate() {
                        level = level.nested.entry(value).or_default();
                        level.name = name;
                        level.count += 1;
                        level.order = order;
                    }
                }
                _ => {
                    for (value, name) in values {
                        let level = root.nested.entry(value).or_default();
                        level.name = name;
                        level.count += 1;
                    }
                }
            }
        }
    }
    fn views(level: Level) -> Vec<FacetValueView> {
        let mut values: Vec<(String, Level)> = level.nested.into_iter().collect();
        values.sort_by(|(a_value, a), (b_value, b)| {
            b.count.cmp(&a.count).then_with(|| a_value.cmp(b_value))
        });
        values
            .into_iter()
            .map(|(value, level)| FacetValueView {
                value,
                name: level.name.clone(),
                title: level.name.clone(),
                count: level.count,
                values: views(level),
                ..Default::default()
            })
            .collect()
    }
    let mut facets = vec![];
    for (key, label) in [
        ("family", "Family"),
        ("busy", "Busy with"),
        ("version", "Version"),
        ("platform", "Platform"),
        ("process", "Process type"),
    ] {
        if let Some(level) = roots.remove(key) {
            facets.push(FacetView::new(key, label.to_string(), views(level)));
        }
    }
    json!({ "facets": facets, "noun": "crash", "nouns": "crashes" })
}

/// The crash view's query URL of the crash `id`, with every thread's.
fn crash_view_url(tree_name: &str, id: &str) -> String {
    format!(
        "/{}/query/default?q={}",
        tree_name,
        url_encode_query(&format!("crash-report:{} crash-threads:all", id))
    )
}

/// The `crash-set:` view of `view` (see the module's documentation), for the
/// query page: its notice (what the set is: its search, linked to crash-stats'
/// page of it, when it was fetched), its families (each with its variants,
/// how many crashes each has, and its latest crash's view), and its crashes'
/// table (a row each, which the navigation panel's facets filter; see
/// `set_facets`).
pub fn render_crash_set(tree_name: &str, view: &CrashSetView) -> String {
    use crate::rendered_docs::escape;
    let manifest = &view.manifest;
    let plural = |n: usize, one: &str, many: &str| match n {
        1 => format!("1 {}", one),
        n => format!("{} {}", n, many),
    };
    let mut html = String::from(r#"<div class="crash-set-view">"#);
    html.push_str(&format!(
        r#"<div class="crash-notice"><p>The crash set <code class="crash-set-id">{}</code>: {} from crash-stats' search for {} (<a class="crash-set-search" href="{}">the search</a>, which found {} then), fetched {}.  Grouped into {} by what their threads other than the crashing one were busy with in the tree's code (their directories) and shutdown hangs' pending blockers, but what most of them had (or else by where the crashing thread's code was called from), and within those, variants by their functions.{}</p>"#,
        escape(&view.id),
        plural(view.digests.len(), "crash report", "crash reports"),
        manifest.search.about_html(),
        escape(&manifest.search.search_url()),
        manifest.total,
        escape(&manifest.fetched_at),
        plural(view.families.len(), "family", "families"),
        match view.missing.len() {
            0 => String::new(),
            n => format!(
                "  {} the store doesn't have any more.",
                plural(n, "crash", "crashes")
            ),
        }
    ));
    if !view.common.is_empty() {
        html.push_str(&format!(
            r#"<p class="crash-set-common">What most of them were busy with, which says nothing of a crash: {}.</p>"#,
            view.common
                .iter()
                .map(|(key, count)| format!("{} ({})", escape(key), count))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    html.push_str("</div>");
    html.push_str(r#"<section class="crash-families">"#);
    for family in &view.families {
        html.push_str(&format!(
            r#"<details class="crash-family" open data-family="{}"><summary><span class="crash-family-key">{}</span> <span class="crash-family-count">{}</span></summary><ul class="crash-variants">"#,
            escape(&family.value),
            escape(&family.key),
            plural(family.crashes, "crash", "crashes")
        ));
        for variant in &family.variants {
            let latest = &view.digests[variant.crashes[0]];
            html.push_str(&format!(
                r#"<li class="crash-variant" data-variant="{}"><span class="crash-variant-label">{}</span> <span class="crash-variant-count">{}</span> <a class="crash-variant-latest" href="{}">its latest crash</a></li>"#,
                escape(&variant.value),
                escape(&variant.label),
                plural(variant.crashes.len(), "crash", "crashes"),
                escape(&crash_view_url(tree_name, &latest.id))
            ));
        }
        html.push_str("</ul></details>");
    }
    html.push_str("</section>");
    html.push_str(r#"<table class="crash-set-crashes"><thead><tr><th>Crash</th><th>Date</th><th>Version</th><th>Platform</th><th>Process</th><th>Family</th><th>Variant</th></tr></thead><tbody class="facet-items">"#);
    for (at, digest) in view.digests.iter().enumerate() {
        let facets: serde_json::Map<String, Value> = crash_facet_values(view, at)
            .into_iter()
            .map(|(key, values)| {
                (
                    key.to_string(),
                    values
                        .into_iter()
                        .map(|(value, _)| Value::String(value))
                        .collect(),
                )
            })
            .collect();
        html.push_str(&format!(
            r#"<tr class="crash-set-crash" data-path="{}" data-facets="{}"><td><a class="crash-set-crash-link" href="{}">{}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            escape(&digest.id),
            escape(&Value::Object(facets).to_string()),
            escape(&crash_view_url(tree_name, &digest.id)),
            escape(&digest.id),
            escape(&digest.date),
            escape(&digest.version),
            escape(&digest.platform),
            escape(&digest.process_type),
            escape(&digest.family),
            escape(&digest.variant_label)
        ));
    }
    html.push_str("</tbody></table></div>");
    html.push_str(FAMILIES_SCRIPT);
    html
}

/// The script of a set's view's families (see `render_crash_set`): as the
/// navigation panel's facets filter its crashes' rows (facets.js's
/// "facetschange"), its families and variants say how many of those shown
/// they have, those without any hide, and their latest crashes' links go to
/// the latest shown.
const FAMILIES_SCRIPT: &str = r#"<script>
(() => {
  const body = document.querySelector(".crash-set-crashes .facet-items");
  if (!body) {
    return;
  }
  const rows = [...body.querySelectorAll(".crash-set-crash")].map(row => ({
    row,
    values: JSON.parse(row.dataset.facets).family || [],
    href: row.querySelector(".crash-set-crash-link")?.getAttribute("href"),
  }));
  const plural = n => `${n} ${n == 1 ? "crash" : "crashes"}`;
  body.addEventListener("facetschange", () => {
    const shown = rows.filter(entry => !entry.row.hidden);
    const filtered = shown.length < rows.length;
    const show = (element, value, count, total) => {
      const n = shown.filter(entry => entry.values.includes(value)).length;
      element.hidden = n == 0;
      count.textContent = filtered ? `${n} of ${plural(total)}` : plural(total);
      return n;
    };
    for (const family of document.querySelectorAll(".crash-family")) {
      const count = family.querySelector(".crash-family-count");
      count.dataset.total ??= rows.filter(entry => entry.values.includes(family.dataset.family)).length;
      show(family, family.dataset.family, count, Number(count.dataset.total));
      for (const variant of family.querySelectorAll(".crash-variant")) {
        const value = variant.dataset.variant;
        const variantCount = variant.querySelector(".crash-variant-count");
        variantCount.dataset.total ??= rows.filter(entry => entry.values.includes(value)).length;
        if (show(variant, value, variantCount, Number(variantCount.dataset.total))) {
          // (Rows are the latest first.)
          const latest = shown.find(entry => entry.values.includes(value));
          variant.querySelector(".crash-variant-latest")?.setAttribute("href", latest.href);
        }
      }
    }
  });
})();
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn search() -> CrashSearch {
        CrashSearch {
            signature: Some(
                "AsyncShutdownTimeout | profile-before-change | QuotaManager".to_string(),
            ),
            product: "Firefox".to_string(),
            release_channels: vec!["nightly".to_string()],
            date_from: "2026-10-25".to_string(),
            date_to: "2026-11-01".to_string(),
            limit: 100,
            ..Default::default()
        }
    }

    #[test]
    fn test_search() {
        assert_eq!(search().check(), Ok(()));
        let mut both = search();
        both.proto_signature = Some("QuotaClient".to_string());
        assert!(both.check().is_err());
        let mut long = search();
        long.date_from = "2026-09-01".to_string();
        assert!(long.check().is_err());
        let mut many = search();
        many.limit = MAX_CRASHES + 1;
        assert!(many.check().is_err());
        assert_eq!(
            search().search_url(),
            "https://crash-stats.mozilla.org/search/?product=Firefox&signature=%3DAsyncShutdownTimeout%20%7C%20profile-before-change%20%7C%20QuotaManager&release_channel=nightly&date=%3E%3D2026-10-25&date=%3C2026-11-01"
        );
    }

    #[test]
    fn test_store() {
        let root = std::env::temp_dir().join(format!("crash-sets-test-{}", std::process::id()));
        let store = CrashSetStore::new(&root);
        let id = "5e4c0b1a-7e57-4c0d-8e5a-000000261101";
        let processed = json!({
            "uuid": id,
            "signature": "Foo",
            "user_comments": "protected",
            "url": "https://example.com/protected",
            "json_dump": {
                "crash_info": {"type": "SIGSEGV", "crashing_thread": 0},
                "threads": [{"thread_name": "Main", "frames": [{"frame": 0, "function": "f", "registers": {"rip": "0x1"}, "inlines": [{"function": "g", "file": "a.cpp", "line": 1}]}]}],
                "modules": [{"filename": "XUL", "version": "1", "debug_id": "D", "base_addr": "0x0"}],
                "mac_boot_args": "protected",
            },
        });
        store.store_crash(id, &processed).unwrap();
        let stored = store.crash(id).unwrap().unwrap();
        // (Only the whitelist's fields.)
        assert!(stored.get("user_comments").is_none() && stored.get("url").is_none());
        assert!(stored["json_dump"].get("mac_boot_args").is_none());
        assert!(
            stored["json_dump"]["threads"][0]["frames"][0]
                .get("registers")
                .is_none()
        );
        assert_eq!(
            stored["json_dump"]["threads"][0]["frames"][0]["inlines"][0]["function"],
            "g"
        );
        assert!(stored["json_dump"]["modules"][0].get("base_addr").is_none());
        assert!(store.crash("not-a-crash-id").is_err());

        let manifest = CrashSetManifest {
            version: MANIFEST_VERSION,
            search: search(),
            fetched_at: "2026-11-01T12:00:00Z".to_string(),
            total: 1,
            crashes: vec![id.to_string()],
        };
        let set = store.store_manifest(&manifest).unwrap();
        assert!(is_set_id(&set));
        assert_eq!(set, set_id(&manifest));
        assert_eq!(store.manifest(&set).unwrap(), Some(manifest.clone()));
        assert_eq!(store.store_manifest(&manifest).unwrap(), set);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A crash's digest whose other busy threads are `busy` (their names,
    /// places, and functions there).
    fn busy_digest(id: &str, busy: &[(&str, &str, &str)]) -> CrashDigest {
        let thread = |index: usize, name: &str, place: Option<&str>, code: &str, crashing: bool| {
            BusyThread {
                index,
                label: format!("#{} ({})", index, name),
                name: Some(name.to_string()),
                kind: "storage thread".to_string(),
                state: "active".to_string(),
                crashing,
                frames: vec![BusyFrame {
                    function: code.to_string(),
                    place: place.unwrap_or("ntdll.dll").to_string(),
                }],
                place: place.map(str::to_string),
                code: vec![code.to_string()],
            }
        };
        let mut threads = vec![BusyThread {
            frames: vec![
                BusyFrame {
                    function: "Crash".to_string(),
                    place: "mfbt".to_string(),
                },
                BusyFrame {
                    function: "nsTimerImpl::Fire".to_string(),
                    place: "xpcom/threads".to_string(),
                },
            ],
            ..thread(0, "IPDL Background", Some("mfbt"), "Crash", true)
        }];
        threads.extend(
            busy.iter()
                .enumerate()
                .map(|(at, (name, place, code))| thread(at + 1, name, Some(place), code, false)),
        );
        CrashDigest {
            id: id.to_string(),
            date: format!("2026-10-3{}", &id[id.len() - 1..]),
            version: "146.0".to_string(),
            platform: "Windows NT".to_string(),
            process_type: "parent".to_string(),
            signature: "Foo".to_string(),
            blockers: vec![],
            busy: threads,
            family: String::new(),
            variant: String::new(),
            variant_label: String::new(),
        }
    }

    #[test]
    fn test_analyze() {
        // (A thread every crash had busy says nothing; the others make the
        // families, their functions the variants, and a crash with only the
        // common ones is by where its crashing thread's code was called
        // from.)
        let common = (
            "Link Monitor",
            "netwerk/system/win32",
            "nsNotifyAddrListener::Run",
        );
        let digests = vec![
            busy_digest(
                "c1",
                &[common, ("QuotaManager IO", "dom/quota", "UnloadQuota")],
            ),
            busy_digest(
                "c2",
                &[common, ("QuotaManager IO", "dom/quota", "UnloadQuota")],
            ),
            busy_digest(
                "c3",
                &[
                    common,
                    ("QuotaManager IO", "dom/quota", "CreateDirectoryMetadata2"),
                ],
            ),
            busy_digest(
                "c4",
                &[
                    common,
                    ("DOMCacheThread", "dom/cache", "DeleteAllCacheEntries"),
                ],
            ),
            busy_digest(
                "c5",
                &[
                    common,
                    ("DOMCacheThread", "dom/cache", "DeleteAllCacheEntries"),
                ],
            ),
            busy_digest("c6", &[common]),
        ];
        let manifest = CrashSetManifest {
            version: MANIFEST_VERSION,
            search: search(),
            fetched_at: "2026-11-01T12:00:00Z".to_string(),
            total: 6,
            crashes: digests.iter().map(|digest| digest.id.clone()).collect(),
        };
        let view = analyze("0123456789abcdef", manifest, digests, vec![]);
        assert_eq!(
            view.common,
            vec![("Link Monitor in netwerk/system/win32".to_string(), 6)]
        );
        let families: Vec<(&str, usize, Vec<usize>)> = view
            .families
            .iter()
            .map(|family| {
                (
                    family.key.as_str(),
                    family.crashes,
                    family
                        .variants
                        .iter()
                        .map(|variant| variant.crashes.len())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            families,
            vec![
                ("QuotaManager IO in dom/quota", 3, vec![2, 1]),
                ("DOMCacheThread in dom/cache", 2, vec![2]),
                (
                    "nothing else distinctive busy; crashing thread called from xpcom/threads",
                    1,
                    vec![1]
                ),
            ]
        );
        assert_eq!(
            view.families[0].variants[0].label,
            "QuotaManager IO: UnloadQuota"
        );
        // (The latest first.)
        assert_eq!(view.digests[0].id, "c6");
        let facets = set_facets(&view);
        assert_eq!(facets["nouns"], "crashes");
        assert_eq!(facets["facets"][0]["key"], "family");
        assert_eq!(facets["facets"][0]["values"][0]["count"], 3);
        assert_eq!(facets["facets"][0]["values"][0]["values"][0]["count"], 2);
    }

    #[test]
    fn test_pending_blockers() {
        let annotation = r#"{"phase":"profile-before-change","conditions":[{"name":"QuotaManager: shutting down","state":[{"name":"Done","state":"Complete"},{"name":"IndexedDB client","state":"(none)"},{"name":"Cache client","state":{"pending":2}}]},{"name":"Other: done","state":"Complete"},{"name":"Nested: all done","state":[{"name":"x","state":"Complete"}]}]}"#;
        assert_eq!(
            pending_blockers(annotation),
            vec!["IndexedDB client", "Cache client", "Nested: all done"]
        );
        assert!(pending_blockers("not json").is_empty());
    }
}
