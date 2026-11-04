//! Crash sets: the crash reports a crash-stats search found, fetched once and
//! stored, and their analysis, for the `crash-set:` view (see
//! `render_crash_set`): what each crash's threads were busy with (see
//! `digest`), and the set's families (by which components' code its threads
//! other than the crashing one were busy in, and shutdown hangs' pending
//! blockers) and their variants (by which functions).  See
//! asuth-gecko-notes' tooling/searchfox/crash-sets-plan.md for the design:
//! nothing a user writes is stored (a set is a search's results, its ID its
//! manifest's hash, and a search finding nothing stores nothing).  The web
//! server's form asks for sets' ingestions (see `request_ingestion`), with a
//! user's token for crash-stats' rate limits (which must have no
//! permissions, and is forgotten once the fetch is done).  Sets are kept for
//! the web server's life, and ingestions which would go over the store's cap
//! are refused (see `CrashSetStore::cap`); the next index carries some of
//! them over, the most recently used, within 60% of its cap (see
//! `carry_over`).
//!
//! The store is a directory (`crash_sets_path` in the config, else
//! `crash-sets/` beside the trees' index directories; see
//! `AbstractServer::crash_sets`):
//! - `sets/ID.json`: a set's manifest (see `CrashSetManifest`), whose ID is
//!   its hash (see `set_id`);
//! - `crashes/AB/CRASH-ID.json.gz` (or `.json`, ex: tests' fixtures): each
//!   crash's whitelisted fields (see `whitelist`), once however many sets
//!   have it, which the crash view reads too (see `LocalIndex::crash_report`);
//! - `touched/ID`: when the set was last used (its time; see
//!   `CrashSetStore::touch`);
//! - `cap`: how many bytes its sets and crashes may take (see
//!   `CrashSetStore::cap`).

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
/// How many frames of a busy thread's stack its set's diagrams have (more
/// join what different crashes' threads were busy with through their shared
/// callers, into big diagrams).
const GRAPH_FRAMES: usize = 4;
/// crash-stats' API.
pub const CRASH_STATS_API: &str = "https://crash-stats.mozilla.org/api";
/// How many days back a set's search may start (see `CrashSearch::check_recent`).
pub const MAX_DAYS_BACK: i64 = 30;
/// How many crashes a set has at most without a token, under crash-stats'
/// rate limits for requests without one.
pub const MAX_CRASHES_WITHOUT_TOKEN: usize = 100;
/// How many bytes a store's sets and crashes may take at most, unless its
/// `cap` file or the config says (see `CrashSetStore::cap`): 5 GB.
pub const DEFAULT_CAP_BYTES: u64 = 5_000_000_000;
/// How often a set's use is noted at most (see `CrashSetStore::touch`).
pub const TOUCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// How big a crash's file is taken to be, for the estimates of ingestions'
/// sizes, without the store's own average (see `request_ingestion`).
const CRASH_BYTES_ESTIMATE: u64 = 30_000;

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

    /// Whether its dates are recent enough (see `MAX_DAYS_BACK`), as of
    /// `today`, and it has few enough crashes for whether it has a token.
    pub fn check_recent(&self, today: chrono::NaiveDate, with_token: bool) -> Result<(), String> {
        let date = |date: &str| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok();
        if date(&self.date_from).is_none_or(|from| (today - from).num_days() > MAX_DAYS_BACK)
            || date(&self.date_to).is_none_or(|to| (to - today).num_days() > 1)
        {
            return Err(format!(
                "A crash set's dates are within the last {} days",
                MAX_DAYS_BACK
            ));
        }
        if !with_token && self.limit > MAX_CRASHES_WITHOUT_TOKEN {
            return Err(format!(
                "A crash set without a token has at most {} crashes",
                MAX_CRASHES_WITHOUT_TOKEN
            ));
        }
        Ok(())
    }

    /// The search a form's `fields` say (see crash_set_form.liquid; a
    /// field can repeat, ex: checkboxes): `signature` or `proto_signature`,
    /// `product`, `release_channel`s, `versions` (comma-separated),
    /// `platform`s, `process_type`s, the days `from` and `through` (so its
    /// `date_to` is the next day), and `limit`.
    pub fn from_form(fields: &[(String, String)]) -> Result<CrashSearch, String> {
        let one = |name: &str| {
            fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let all = |name: &str| -> Vec<String> {
            fields
                .iter()
                .filter(|(field, _)| field == name)
                .flat_map(|(_, value)| value.split(','))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        };
        let through = one("through").unwrap_or_default();
        let date_to = chrono::NaiveDate::parse_from_str(&through, "%Y-%m-%d")
            .map_err(|_| format!("{:?} isn't a date (YYYY-MM-DD)", through))?
            + chrono::Duration::days(1);
        let search = CrashSearch {
            signature: one("signature"),
            proto_signature: one("proto_signature"),
            product: one("product").unwrap_or_else(|| "Firefox".to_string()),
            release_channels: all("release_channel"),
            versions: all("versions"),
            platforms: all("platform"),
            process_types: all("process_type"),
            date_from: one("from").unwrap_or_default(),
            date_to: date_to.format("%Y-%m-%d").to_string(),
            limit: one("limit")
                .unwrap_or_default()
                .parse()
                .map_err(|_| "A crash set's number of crashes is a number".to_string())?,
        };
        search.check()?;
        Ok(search)
    }

    /// The last day of its dates (the day before `date_to`), for its form.
    pub fn through(&self) -> String {
        chrono::NaiveDate::parse_from_str(&self.date_to, "%Y-%m-%d")
            .map(|to| {
                (to - chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default()
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

/// A crash set store (see the module's documentation): its directory, which
/// it writes, and optionally a seed, another store which it only reads,
/// after its own (ex: tests' fixtures, beside what the tests write).
#[derive(Clone, Debug)]
pub struct CrashSetStore {
    root: PathBuf,
    seed: Option<PathBuf>,
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

/// The path of the crash `id`'s file in the store at `root`.
fn crash_path(root: &Path, id: &str, extension: &str) -> PathBuf {
    root.join("crashes")
        .join(&id[..2])
        .join(format!("{}.{}", id, extension))
}

impl CrashSetStore {
    pub fn new(root: impl Into<PathBuf>) -> CrashSetStore {
        CrashSetStore {
            root: root.into(),
            seed: None,
        }
    }

    /// The store, reading `seed` after its own (see `CrashSetStore`).
    pub fn with_seed(mut self, seed: Option<PathBuf>) -> CrashSetStore {
        self.seed = seed;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Its directories to read, its own first.
    fn roots(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.root).chain(self.seed.as_ref())
    }

    /// The stored crash `id`, if the store has it.
    pub fn crash(&self, id: &str) -> Result<Option<Value>, String> {
        if !is_crash_id(id) {
            return Err(format!("{:?} isn't a crash report's ID", id));
        }
        for root in self.roots() {
            let path = crash_path(root, id, "json.gz");
            let text = match std::fs::File::open(&path) {
                Ok(file) => {
                    let mut text = String::new();
                    flate2::read::GzDecoder::new(file)
                        .read_to_string(&mut text)
                        .map_err(|e| format!("{}: {}", path.display(), e))?;
                    text
                }
                Err(_) => match std::fs::read_to_string(crash_path(root, id, "json")) {
                    Ok(text) => text,
                    Err(_) => continue,
                },
            };
            return serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| format!("crash {}: {}", id, e));
        }
        Ok(None)
    }

    /// Whether the store has the crash `id`.
    pub fn has_crash(&self, id: &str) -> bool {
        is_crash_id(id)
            && self.roots().any(|root| {
                crash_path(root, id, "json.gz").exists() || crash_path(root, id, "json").exists()
            })
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
        write_atomically(&crash_path(&self.root, id, "json.gz"), &bytes)
    }

    /// The set `id`'s manifest, if the store has it.
    pub fn manifest(&self, id: &str) -> Result<Option<CrashSetManifest>, String> {
        if !is_set_id(id) {
            return Err(format!("{:?} isn't a crash set's ID", id));
        }
        for root in self.roots() {
            if let Ok(text) =
                std::fs::read_to_string(root.join("sets").join(format!("{}.json", id)))
            {
                return serde_json::from_str(&text)
                    .map(Some)
                    .map_err(|e| format!("crash set {}: {}", id, e));
            }
        }
        Ok(None)
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
        // (Its creation is its first use.)
        self.touch(&id)?;
        Ok(id)
    }

    /// Notes a use of the set `id` (see `touched`), unless it was used in
    /// the last `TOUCH_INTERVAL` (so that a busy set's page doesn't write it
    /// every time), if the store has it.
    pub fn touch(&self, id: &str) -> Result<(), String> {
        if !is_set_id(id) {
            return Err(format!("{:?} isn't a crash set's ID", id));
        }
        let path = self.root.join("touched").join(id);
        if let Some(touched) = self.touched(id)
            && touched
                .elapsed()
                .is_ok_and(|elapsed| elapsed < TOUCH_INTERVAL)
            && path.exists()
        {
            return Ok(());
        }
        if self.manifest(id)?.is_none() {
            return Err(format!("There's no crash set {} here", id));
        }
        write_atomically(&path, b"")
    }

    /// When the set `id` was last used (its `touched/ID`'s time; see
    /// `touch`), or else stored.
    pub fn touched(&self, id: &str) -> Option<std::time::SystemTime> {
        let modified = |path: PathBuf| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        };
        self.roots().find_map(|root| {
            modified(root.join("touched").join(id))
                .or_else(|| modified(root.join("sets").join(format!("{}.json", id))))
        })
    }

    /// The sets the store has (its own, not its seed's).
    pub fn set_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(self.root.join("sets"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()?
                    .strip_suffix(".json")
                    .filter(|id| is_set_id(id))
                    .map(str::to_string)
            })
            .collect();
        ids.sort();
        ids
    }

    /// How big the crash `id`'s file is, if the store has it (its own).
    fn crash_bytes(&self, id: &str) -> Option<(String, u64)> {
        ["json.gz", "json"].iter().find_map(|extension| {
            let path = crash_path(&self.root, id, extension);
            let bytes = std::fs::metadata(&path).ok()?.len();
            Some((format!("crashes/{}/{}.{}", &id[..2], id, extension), bytes))
        })
    }

    /// How many bytes its sets and crashes take (its own), and how many
    /// crashes it has.
    pub fn size(&self) -> (u64, usize) {
        let mut bytes = 0;
        let mut crashes = 0;
        for dir in ["sets", "crashes"] {
            for entry in walkdir::WalkDir::new(self.root.join(dir))
                .into_iter()
                .flatten()
                .filter(|entry| entry.file_type().is_file())
            {
                bytes += entry.metadata().map_or(0, |meta| meta.len());
                if dir == "crashes" {
                    crashes += 1;
                }
            }
        }
        (bytes, crashes)
    }

    /// How many bytes its sets and crashes may take at most: its `cap` file's
    /// (which the indexer writes, by the index's volume's free space; see
    /// infrastructure/aws/carry-over-crash-sets.py), else
    /// `DEFAULT_CAP_BYTES`.  Sets are kept for the web server's life, and
    /// ingestions which would go over it are refused (see
    /// `request_ingestion`), until the next index, which only carries some of
    /// them over (see `carry_over`).
    pub fn cap(&self) -> u64 {
        std::fs::read_to_string(self.root.join("cap"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(DEFAULT_CAP_BYTES)
    }

    /// Removes the crash `id`'s file (its own; ex: one a failed fetch
    /// stored, which no set has).
    fn remove_crash(&self, id: &str) {
        for extension in ["json.gz", "json"] {
            let _ = std::fs::remove_file(crash_path(&self.root, id, extension));
        }
    }
}

/// What the next index's store carries over of a store (see `carry_over`):
/// its sets, the most recently used first, but penalized for their sizes,
/// within a budget; their files (relative to the store's directory, a crash's
/// once, with the first set which has it); and how many bytes those take.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct CarryOver {
    pub sets: Vec<CarriedSet>,
    pub files: Vec<String>,
    pub bytes: u64,
}

/// A set carried over (see `CarryOver`): its ID, how many bytes its files
/// take (its own, but those of crashes a set before it has), and when it was
/// last used (see `CrashSetStore::touched`), in seconds since the epoch.
#[derive(Debug, PartialEq, Serialize)]
pub struct CarriedSet {
    pub id: String,
    pub bytes: u64,
    pub touched: u64,
}

/// How many bytes make a set count as a day older than it is, when choosing
/// the sets to carry over (see `carry_over`): 10 MB, so that one of the most
/// crashes (`MAX_CRASHES`, about 12 MB) counts as about twice as old as a
/// small one.
const CARRY_OVER_BYTES_PER_DAY: f64 = 10_000_000.0;

/// The sets of `store` to carry over to the next index's store, within
/// `budget` bytes (see infrastructure/aws/carry-over-crash-sets.py: 60% of
/// its cap): by how long ago they were last used, in days, plus one, times
/// one plus their sizes in `CARRY_OVER_BYTES_PER_DAY`s, the least first, as
/// long as they fit (a crash several of them have counting once).
pub fn carry_over(store: &CrashSetStore, budget: u64, now: std::time::SystemTime) -> CarryOver {
    struct Candidate {
        id: String,
        files: Vec<(String, u64, Option<String>)>,
        touched: std::time::SystemTime,
        score: f64,
    }
    let mut candidates: Vec<Candidate> = store
        .set_ids()
        .into_iter()
        .filter_map(|id| {
            let manifest = store.manifest(&id).ok()??;
            let manifest_bytes =
                std::fs::metadata(store.root.join("sets").join(format!("{}.json", id)))
                    .ok()?
                    .len();
            let mut files = vec![(format!("sets/{}.json", id), manifest_bytes, None)];
            files.extend(manifest.crashes.iter().filter_map(|crash| {
                let (path, bytes) = store.crash_bytes(crash)?;
                Some((path, bytes, Some(crash.clone())))
            }));
            let touched = store.touched(&id)?;
            let days = now
                .duration_since(touched)
                .unwrap_or_default()
                .as_secs_f64()
                / 86400.0;
            let bytes: u64 = files.iter().map(|(_, bytes, _)| bytes).sum();
            let score = (days + 1.0) * (1.0 + bytes as f64 / CARRY_OVER_BYTES_PER_DAY);
            Some(Candidate {
                id,
                files,
                touched,
                score,
            })
        })
        .collect();
    candidates.sort_by(|a, b| a.score.total_cmp(&b.score).then_with(|| a.id.cmp(&b.id)));
    let mut carried = CarryOver::default();
    let mut crashes: BTreeSet<String> = BTreeSet::new();
    for candidate in candidates {
        let new_files: Vec<&(String, u64, Option<String>)> = candidate
            .files
            .iter()
            .filter(|(_, _, crash)| crash.as_ref().is_none_or(|crash| !crashes.contains(crash)))
            .collect();
        let bytes: u64 = new_files.iter().map(|(_, bytes, _)| bytes).sum();
        if carried.bytes + bytes > budget {
            continue;
        }
        carried.bytes += bytes;
        for (path, _, crash) in new_files {
            carried.files.push(path.clone());
            crashes.extend(crash.clone());
        }
        carried.sets.push(CarriedSet {
            id: candidate.id,
            bytes,
            touched: candidate
                .touched
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
    }
    carried
}

/// The contents of the store's file `path` (see `CarryOver::files`), if it's
/// one of its sets' or crashes' files, which it has (its own).
pub fn carried_file(store: &CrashSetStore, path: &str) -> Option<Vec<u8>> {
    let valid = match path.split('/').collect::<Vec<_>>()[..] {
        ["sets", file] => file.strip_suffix(".json").is_some_and(is_set_id),
        ["crashes", dir, file] => file
            .strip_suffix(".json.gz")
            .or_else(|| file.strip_suffix(".json"))
            .is_some_and(|id| is_crash_id(id) && id[..2] == *dir),
        _ => false,
    };
    match valid {
        true => std::fs::read(store.root.join(path)).ok(),
        false => None,
    }
}

/// `bytes` in base64 (RFC 4648's, padded), for the carried-over files' export
/// (see web-server.rs's `crash-sets/files`).
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | (*byte as u32) << (16 - 8 * i));
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// A failure of a request of crash-stats' API (see `Api::get`): its HTTP
/// status, if it had one, and what went wrong.
struct ApiError {
    status: Option<u16>,
    message: String,
}

/// crash-stats' API at `base` (see `CRASH_STATS_API`), or a directory standing
/// in for it (`file://DIR`, ex: tests' fixtures): `DIR/SuperSearch.json`
/// answers any search (or `DIR/SuperSearch.TOKEN.json`, a token's, ex: one
/// which can see protected data), and `DIR/ProcessedCrash/ID.json` is the
/// processed crash ID.  (A directory's has no HTTP client, which needs CA
/// certificates, which nix's sandbox, where its tests run, doesn't have.)
struct Api<'a> {
    base: &'a str,
    client: Option<reqwest::Client>,
}

impl Api<'_> {
    /// `endpoint`'s JSON for `query` (ex: "crash_id=..."), with `token` if
    /// given, waiting and trying again after crash-stats' rate limits' 429s
    /// (a few times).
    async fn get(
        &self,
        endpoint: &str,
        query: &str,
        token: Option<&str>,
    ) -> Result<Value, ApiError> {
        let failed = |status: Option<u16>, message: String| ApiError { status, message };
        if let Some(dir) = self.base.strip_prefix("file://") {
            let path = match endpoint {
                "SuperSearch" => token
                    .map(|token| format!("{}/SuperSearch.{}.json", dir, token))
                    .filter(|path| Path::new(path).exists())
                    .unwrap_or_else(|| format!("{}/SuperSearch.json", dir)),
                "ProcessedCrash" => {
                    let id = query
                        .split('&')
                        .find_map(|param| param.strip_prefix("crash_id="))
                        .filter(|id| is_crash_id(id))
                        .ok_or_else(|| failed(Some(400), "no crash ID".to_string()))?;
                    format!("{}/ProcessedCrash/{}.json", dir, id)
                }
                _ => return Err(failed(Some(404), endpoint.to_string())),
            };
            let text = std::fs::read_to_string(&path)
                .map_err(|_| failed(Some(404), format!("{} {}", endpoint, query)))?;
            return serde_json::from_str(&text).map_err(|e| failed(None, e.to_string()));
        }
        let url = format!("{}/{}/?{}", self.base, endpoint, query);
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| failed(None, "no HTTP client".to_string()))?;
        for attempt in 1..=5u64 {
            let mut request = client.get(&url);
            if let Some(token) = token {
                request = request.header("Auth-Token", token);
            }
            let response = request
                .send()
                .await
                .map_err(|e| failed(None, e.to_string()))?;
            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
                continue;
            }
            if !status.is_success() {
                return Err(failed(
                    Some(status.as_u16()),
                    format!("crash-stats' {} answered {}", endpoint, status),
                ));
            }
            return response
                .json()
                .await
                .map_err(|e| failed(None, e.to_string()));
        }
        Err(failed(
            Some(429),
            format!("crash-stats kept rate limiting its {}", endpoint),
        ))
    }
}

/// Checks that `token` (a crash-stats API token) can't see protected data,
/// by asking for protected columns (crash-stats drops those a request may
/// not see, rather than refusing them): any in the hits means it can.
async fn check_unprivileged(api: &Api<'_>, token: &str) -> Result<(), String> {
    const PROTECTED: &[&str] = &["url", "email", "user_comments"];
    let since = (chrono::Utc::now() - chrono::Duration::days(2)).format("%Y-%m-%d");
    let mut query = format!(
        "product=Firefox&_results_number=20&_facets_size=0&_columns=uuid&date=%3E%3D{}",
        since
    );
    for column in PROTECTED {
        query.push_str(&format!("&_columns={}", column));
    }
    let hits = match api.get("SuperSearch", &query, Some(token)).await {
        Ok(hits) => hits,
        // (Refused, which crash-stats could do too.)
        Err(ApiError {
            status: Some(400 | 403),
            ..
        }) => return Ok(()),
        Err(ApiError {
            status: Some(401), ..
        }) => return Err("crash-stats doesn't know that token".to_string()),
        Err(e) => return Err(format!("Couldn't check the token: {}", e.message)),
    };
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

/// Fetches the crashes `search` finds from crash-stats (at `api`, see `Api`)
/// into `store`, and stores their set's manifest, returning its ID; nothing
/// is stored if it finds none.  `token` (a crash-stats API token without
/// permissions, which this checks first) is only for its rate limits: the
/// search uses it, and the processed crashes' fetches when they're rate
/// limited without it (crash-stats then strips protected data itself, and
/// `whitelist` keeps what the store has to public fields either way).  It's
/// not kept.  `progress` hears how many of the crashes it has, of how many,
/// as it fetches them.
pub async fn fetch_set(
    store: &CrashSetStore,
    search: &CrashSearch,
    api: &str,
    token: Option<&str>,
    progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<String, String> {
    search.check()?;
    let api = Api {
        base: api,
        client: match api.starts_with("file://") {
            true => None,
            false => Some(
                reqwest::Client::builder()
                    .user_agent(concat!(
                        "searchfox/",
                        env!("CARGO_PKG_VERSION"),
                        " (crash sets)"
                    ))
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .map_err(|e| e.to_string())?,
            ),
        },
    };
    if let Some(token) = token {
        check_unprivileged(&api, token).await?;
    }
    let query: Vec<String> = search
        .params()
        .iter()
        .map(|(name, value)| format!("{}={}", name, url_encode_query(value)))
        .collect();
    let query = format!(
        "{}&_columns=uuid&_sort=-date&_facets_size=0&_results_number={}",
        query.join("&"),
        search.limit
    );
    let found = api
        .get("SuperSearch", &query, token)
        .await
        .map_err(|e| e.message)?;
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
    crashes.truncate(search.limit);
    // (Stopping if the store would go over its cap, and removing what it
    // stored if it fails.)
    let cap = store.cap();
    let (mut bytes, _) = store.size();
    let mut stored: Vec<String> = Vec::new();
    let fetched = async {
        for (fetched, id) in crashes.iter().enumerate() {
            progress(fetched, crashes.len());
            if store.has_crash(id) {
                continue;
            }
            let query = format!("crash_id={}", id);
            let processed = match api.get("ProcessedCrash", &query, None).await {
                Ok(processed) => processed,
                Err(_) if token.is_some() => api
                    .get("ProcessedCrash", &query, token)
                    .await
                    .map_err(|e| e.message)?,
                Err(e) => return Err(e.message),
            };
            store.store_crash(id, &processed)?;
            stored.push(id.clone());
            bytes += store.crash_bytes(id).map_or(0, |(_, bytes)| bytes);
            if bytes > cap {
                return Err(FULL.to_string());
            }
        }
        Ok(())
    }
    .await;
    if let Err(message) = fetched {
        for id in &stored {
            store.remove_crash(id);
        }
        return Err(message);
    }
    progress(crashes.len(), crashes.len());
    let manifest = CrashSetManifest {
        version: MANIFEST_VERSION,
        search: search.clone(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        total: found["total"].as_u64().unwrap_or(crashes.len() as u64),
        crashes,
    };
    store.store_manifest(&manifest)
}

/// Why an ingestion which would go over the store's cap is refused (see
/// `CrashSetStore::cap`).
const FULL: &str = "There's no room for that many more crash reports until searchfox's next re-index: ask for fewer, or try again after it";

/// How many crash sets' ingestions can be waiting or running at once (see
/// `request_ingestion`), of a client's at most `MAX_CLIENT_INGESTIONS`.
pub const MAX_INGESTIONS: usize = 8;
pub const MAX_CLIENT_INGESTIONS: usize = 1;
/// How many ingestions' statuses are kept (the latest).
const KEPT_INGESTIONS: usize = 200;

/// A crash set's ingestion's status (see `request_ingestion`): waiting, after
/// `ahead` others; running, its crashes fetched so far, of how many, and
/// how long it's been running; done, the set's ID; or failed, why.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum IngestionStatus {
    Queued {
        ahead: usize,
    },
    Running {
        fetched: usize,
        total: usize,
        seconds: u64,
    },
    Done {
        set: String,
    },
    Failed {
        message: String,
    },
}

/// An ingestion's work: where to store the set, crash-stats' API, and the
/// token, which only lives here (in memory) until the ingestion runs, and
/// then until it's done.
struct IngestionWork {
    store: CrashSetStore,
    api: String,
    token: Option<String>,
}

/// An ingestion: its search, its client (see `admission::client_key`), its
/// status, and when it started running.
struct Ingestion {
    search: CrashSearch,
    client: Option<String>,
    status: IngestionStatus,
    started: Option<std::time::Instant>,
}

#[derive(Default)]
struct Ingestions {
    /// By their IDs, in the order they were asked for (their IDs' order
    /// isn't; see `order`).
    jobs: std::collections::HashMap<String, Ingestion>,
    order: std::collections::VecDeque<String>,
    queue: std::collections::VecDeque<(String, IngestionWork)>,
}

static INGESTIONS: std::sync::LazyLock<(std::sync::Mutex<Ingestions>, std::sync::Condvar)> =
    std::sync::LazyLock::new(Default::default);

/// Asks for the crash set `search` finds to be fetched into `store` from
/// crash-stats' `api` (see `fetch_set`), with `token` (a crash-stats API
/// token, which must have no permissions; kept in memory only until the
/// ingestion's done), for `client`: the web server's crash sets' form's
/// (see `format::format_crash_set_form`).  Returns the ingestion's ID (see
/// `ingestion_status`), or why it can't be asked for: its search's dates
/// aren't recent (see `CrashSearch::check_recent`), or `MAX_INGESTIONS` are
/// waiting or running, or `MAX_CLIENT_INGESTIONS` of the client's, or the
/// store would go over its cap (see `CrashSetStore::cap`) with its crashes
/// and those of the others waiting or running (as many as their limits, at
/// the store's crashes' average size).  They run one at a time, in the order
/// they were asked for, on their own thread.
pub fn request_ingestion(
    store: CrashSetStore,
    api: &str,
    search: CrashSearch,
    token: Option<String>,
    client: Option<String>,
) -> Result<String, String> {
    search.check()?;
    search.check_recent(chrono::Utc::now().date_naive(), token.is_some())?;
    let (bytes, crashes) = store.size();
    let per_crash = match crashes {
        0 => CRASH_BYTES_ESTIMATE,
        crashes => bytes / crashes as u64,
    };
    let cap = store.cap();
    static WORKER: std::sync::Once = std::sync::Once::new();
    WORKER.call_once(|| {
        std::thread::spawn(run_ingestions);
    });
    let (lock, queued) = &*INGESTIONS;
    let mut ingestions = lock.lock().unwrap();
    let active: Vec<&Ingestion> = ingestions
        .jobs
        .values()
        .filter(|job| {
            matches!(
                job.status,
                IngestionStatus::Queued { .. } | IngestionStatus::Running { .. }
            )
        })
        .collect();
    if active.len() >= MAX_INGESTIONS {
        return Err(
            "Too many crash sets are being fetched now; try again in a few minutes".to_string(),
        );
    }
    if client.is_some()
        && active.iter().filter(|job| job.client == client).count() >= MAX_CLIENT_INGESTIONS
    {
        return Err("You're fetching a crash set already; wait for it to finish".to_string());
    }
    let pending: usize = active.iter().map(|job| job.search.limit).sum();
    if bytes + (pending + search.limit) as u64 * per_crash > cap {
        return Err(FULL.to_string());
    }
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = format!(
        "{:016x}",
        fnv1a(
            format!(
                "{:?} {} {}",
                std::time::SystemTime::now(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                serde_json::to_string(&search).unwrap_or_default()
            )
            .as_bytes()
        )
    );
    let ahead = ingestions.queue.len();
    ingestions.jobs.insert(
        id.clone(),
        Ingestion {
            search,
            client,
            status: IngestionStatus::Queued { ahead },
            started: None,
        },
    );
    ingestions.order.push_back(id.clone());
    ingestions.queue.push_back((
        id.clone(),
        IngestionWork {
            store,
            api: api.to_string(),
            token,
        },
    ));
    // (Forgetting the oldest finished ones.)
    while ingestions.order.len() > KEPT_INGESTIONS {
        let Some(oldest) = ingestions.order.pop_front() else {
            break;
        };
        ingestions.jobs.remove(&oldest);
    }
    queued.notify_one();
    Ok(id)
}

/// The ingestion `id`'s search and status, if it's one of the latest (see
/// `KEPT_INGESTIONS`).
pub fn ingestion_status(id: &str) -> Option<(CrashSearch, IngestionStatus)> {
    let ingestions = INGESTIONS.0.lock().unwrap();
    let job = ingestions.jobs.get(id)?;
    let status = match (&job.status, job.started) {
        (IngestionStatus::Running { fetched, total, .. }, Some(started)) => {
            IngestionStatus::Running {
                fetched: *fetched,
                total: *total,
                seconds: started.elapsed().as_secs(),
            }
        }
        (IngestionStatus::Queued { .. }, _) => IngestionStatus::Queued {
            ahead: ingestions
                .queue
                .iter()
                .position(|(queued, _)| queued == id)
                .unwrap_or(0),
        },
        (status, _) => status.clone(),
    };
    Some((job.search.clone(), status))
}

/// The ingestions' thread: runs them one at a time (see `request_ingestion`).
fn run_ingestions() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for crash sets' ingestions");
    let (lock, queued) = &*INGESTIONS;
    loop {
        let (id, work, search) = {
            let mut ingestions = lock.lock().unwrap();
            while ingestions.queue.is_empty() {
                ingestions = queued.wait(ingestions).unwrap();
            }
            let (id, work) = ingestions.queue.pop_front().unwrap();
            let Some(job) = ingestions.jobs.get_mut(&id) else {
                continue;
            };
            job.status = IngestionStatus::Running {
                fetched: 0,
                total: 0,
                seconds: 0,
            };
            job.started = Some(std::time::Instant::now());
            (id, work, job.search.clone())
        };
        let progress = |fetched: usize, total: usize| {
            if let Some(job) = lock.lock().unwrap().jobs.get_mut(&id) {
                job.status = IngestionStatus::Running {
                    fetched,
                    total,
                    seconds: 0,
                };
            }
        };
        let IngestionWork { store, api, token } = work;
        let result = runtime.block_on(fetch_set(
            &store,
            &search,
            &api,
            token.as_deref(),
            &progress,
        ));
        // (The token's gone with this.)
        drop(token);
        if let Some(job) = lock.lock().unwrap().jobs.get_mut(&id) {
            job.status = match result {
                Ok(set) => IngestionStatus::Done { set },
                Err(message) => IngestionStatus::Failed { message },
            };
        }
    }
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
/// it called, see `ThreadRules::is_library`, nor generic application code's,
/// see `ThreadRules::is_generic`, else of its first with a file),
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
    /// Its stack in the repository's code from its place down (its callers),
    /// but libraries' and generic application code's frames (ex: event
    /// loops', dispatchers'), a few of them, for its set's diagrams (see
    /// `StackGraph`).
    #[serde(default)]
    pub stack: Vec<BusyFrame>,
    /// Its own frames' files in the repository (but libraries'), for its
    /// set's facets of directories and subsystems.
    #[serde(default)]
    pub paths: Vec<String>,
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
        // (Its own frames, and each's file in the repository, if it is.)
        let own: Vec<(BusyFrame, Option<String>)> = thread
            .frames
            .iter()
            .zip(&roles)
            .filter(|(_, role)| **role == FrameRole::Own)
            .filter_map(|(frame, _)| {
                let function = frame.function.as_deref()?;
                if function == "?" || function.starts_with("<unknown") {
                    return None;
                }
                let place = match (&frame.source.path, &frame.module) {
                    (Some(path), _) => path
                        .rsplit_once('/')
                        .map_or_else(|| "(top)".to_string(), |(dir, _)| dir.to_string()),
                    (None, Some(module)) => module.clone(),
                    (None, None) => "?".to_string(),
                };
                Some((
                    BusyFrame {
                        function: rule_name(function),
                        place,
                    },
                    frame.source.path.clone(),
                ))
            })
            .collect();
        if own.is_empty() {
            continue;
        }
        let in_repository: Vec<usize> = (0..own.len()).filter(|i| own[*i].1.is_some()).collect();
        let place_at = in_repository
            .iter()
            .copied()
            .find(|i| !rules.is_library(&own[*i].0.place) && !rules.is_generic(&own[*i].0.function))
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
        // (Its stack in the repository's code, from its place down, but
        // libraries' frames, for the set's diagrams; and its files.)
        let stack: Vec<BusyFrame> = place_at.map_or_else(Vec::new, |at| {
            own[at..]
                .iter()
                .filter(|(frame, path)| {
                    path.is_some()
                        && !rules.is_library(&frame.place)
                        && !rules.is_generic(&frame.function)
                })
                .take(GRAPH_FRAMES)
                .map(|(frame, _)| frame.clone())
                .collect()
        });
        let paths: Vec<String> = own
            .iter()
            .filter(|(frame, _)| !rules.is_library(&frame.place))
            .filter_map(|(_, path)| path.clone())
            .fold(vec![], |mut paths, path| {
                if !paths.contains(&path) {
                    paths.push(path);
                }
                paths
            });
        busy.push(BusyThread {
            index: thread.thread.index,
            stack,
            paths,
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

/// A function in a set's diagram (see `StackGraph`): its name (without its
/// arguments), where its code is (its directory), and how many of the
/// diagram's stacks have it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StackNode {
    pub function: String,
    pub place: String,
    pub count: usize,
}

/// A call in a set's diagram: its caller's and its callee's positions in
/// its nodes, and how many of its stacks have it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StackEdge {
    pub from: usize,
    pub to: usize,
    pub count: usize,
}

/// A busy thread's stack in a set's diagram: its crash's ID, the thread's
/// label, and its nodes' and edges' positions (so that the view can count
/// those of the crashes its facets show).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphStack {
    pub crash: String,
    pub thread: String,
    pub nodes: Vec<usize>,
    pub edges: Vec<usize>,
}

/// A connected part of the graph of what a set's crashes' threads other than
/// the crashing one were busy with (see `stack_graphs`): their stacks'
/// functions in the repository's code (but libraries'), callers over
/// callees, and the stacks.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StackGraph {
    pub nodes: Vec<StackNode>,
    pub edges: Vec<StackEdge>,
    pub stacks: Vec<GraphStack>,
}

/// How many diagrams a set's view has at most (the biggest).
const MAX_GRAPHS: usize = 40;

/// The diagrams of what a set's crashes' threads other than the crashing
/// one were busy with (see `StackGraph`): the graph of their stacks (see
/// `BusyThread::stack`; but those of what most of them were busy with,
/// `common`, which says nothing), each function a node, each call an edge,
/// in its connected parts, the biggest (by their stacks) first, at most
/// `MAX_GRAPHS`.
fn stack_graphs(digests: &[CrashDigest], is_common: &dyn Fn(&str) -> bool) -> Vec<StackGraph> {
    // (A function is a node by its name and place.)
    let mut nodes: Vec<(String, String)> = vec![];
    let mut node_of: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut stacks: Vec<(String, String, Vec<usize>)> = vec![];
    for digest in digests {
        for thread in digest.busy.iter().filter(|thread| !thread.crashing) {
            let (Some(place), false) = (&thread.place, thread.stack.is_empty()) else {
                continue;
            };
            let who = thread.name.as_deref().unwrap_or(&thread.kind);
            if is_common(&format!("{} in {}", who, place)) {
                continue;
            }
            let mut at: Vec<usize> = vec![];
            for frame in &thread.stack {
                let key = (frame.function.clone(), frame.place.clone());
                let id = *node_of.entry(key.clone()).or_insert_with(|| {
                    nodes.push(key);
                    nodes.len() - 1
                });
                // (Not a recursion's repeats.)
                if !at.contains(&id) {
                    at.push(id);
                }
            }
            stacks.push((digest.id.clone(), thread.label.clone(), at));
        }
    }
    // The connected parts (union-find over the calls).
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    fn root(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for (_, _, at) in &stacks {
        for pair in at.windows(2) {
            let (a, b) = (root(&mut parent, pair[0]), root(&mut parent, pair[1]));
            if a != b {
                parent[a] = b;
            }
        }
    }
    let mut parts: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (stack, (_, _, at)) in stacks.iter().enumerate() {
        let part = root(&mut parent, at[0]);
        parts.entry(part).or_default().push(stack);
    }
    let mut graphs: Vec<StackGraph> = parts
        .into_values()
        .map(|part_stacks| {
            let mut graph = StackGraph {
                nodes: vec![],
                edges: vec![],
                stacks: vec![],
            };
            let mut local: BTreeMap<usize, usize> = BTreeMap::new();
            let mut edge_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
            for stack in part_stacks {
                let (crash, thread, at) = &stacks[stack];
                let ids: Vec<usize> = at
                    .iter()
                    .map(|node| {
                        *local.entry(*node).or_insert_with(|| {
                            let (function, place) = nodes[*node].clone();
                            graph.nodes.push(StackNode {
                                function,
                                place,
                                count: 0,
                            });
                            graph.nodes.len() - 1
                        })
                    })
                    .collect();
                for id in &ids {
                    graph.nodes[*id].count += 1;
                }
                // (Callers over callees: the stack is innermost first.)
                let mut edges = vec![];
                for pair in ids.windows(2) {
                    let (caller, callee) = (pair[1], pair[0]);
                    let edge = *edge_of.entry((caller, callee)).or_insert_with(|| {
                        graph.edges.push(StackEdge {
                            from: caller,
                            to: callee,
                            count: 0,
                        });
                        graph.edges.len() - 1
                    });
                    if !edges.contains(&edge) {
                        graph.edges[edge].count += 1;
                        edges.push(edge);
                    }
                }
                graph.stacks.push(GraphStack {
                    crash: crash.clone(),
                    thread: thread.clone(),
                    nodes: ids,
                    edges,
                });
            }
            graph
        })
        .collect();
    graphs.sort_by(|a, b| {
        b.stacks
            .len()
            .cmp(&a.stacks.len())
            .then_with(|| a.nodes[0].function.cmp(&b.nodes[0].function))
    });
    graphs.truncate(MAX_GRAPHS);
    graphs
}

/// A crash set's analysis: its ID and manifest, its crashes' digests (the
/// latest first; see `digest`), those the store doesn't have any more, what
/// most of its crashes' threads were busy with (`common`, with how many
/// crashes; see `set_view`), its families (the biggest first), the diagrams
/// of what its crashes' threads were busy with (see `stack_graphs`), and its
/// crashes' files' subsystems (the tree's, filled by `crash-set`, see
/// `add_subsystems`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CrashSetView {
    pub id: String,
    pub manifest: CrashSetManifest,
    pub digests: Vec<CrashDigest>,
    pub missing: Vec<String>,
    pub common: Vec<(String, usize)>,
    pub families: Vec<Family>,
    #[serde(default)]
    pub graphs: Vec<StackGraph>,
    #[serde(default)]
    pub subsystems: BTreeMap<String, String>,
}

impl CrashSetView {
    /// Its crashes' files' subsystems, by `subsystem_of` (ex: the tree's
    /// per-file information's; see `AbstractServer::file_facet_info`).
    pub fn add_subsystems(&mut self, subsystem_of: &dyn Fn(&str) -> Option<String>) {
        let paths: BTreeSet<&String> = self
            .digests
            .iter()
            .flat_map(|digest| digest.busy.iter().flat_map(|thread| &thread.paths))
            .collect();
        for path in paths {
            if let Some(subsystem) = subsystem_of(path) {
                self.subsystems.insert(path.clone(), subsystem);
            }
        }
    }
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
    let graphs = stack_graphs(&digests, &is_common);
    CrashSetView {
        id: id.to_string(),
        manifest,
        digests,
        missing,
        common,
        families,
        graphs,
        subsystems: BTreeMap::new(),
    }
}

/// A facet's value's path from its facet's top (ex: a family and its
/// variant), each value with its name.
type ValuePath = Vec<(String, String)>;

/// A crash's facets' values (see `set_facets`), as paths from their facets'
/// tops: its family's and variant's; what its threads other than the
/// crashing one were busy with, by thread (ex: "QuotaManager IO" › "dom/quota";
/// or its pending blockers; see `CrashDigest::features`); the subsystems and
/// the directories of its distinctive busy threads' files (theirs and their
/// parents'; see `CrashSetView::subsystems`); its version, platform and
/// process type.
fn crash_facet_values(view: &CrashSetView, at: usize) -> Vec<(&'static str, Vec<ValuePath>)> {
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
    // (What its other threads were busy with, under their names.)
    let mut busy: Vec<ValuePath> = vec![];
    for blocker in &digest.blockers {
        busy.push(vec![
            ("blocked".to_string(), "pending blockers".to_string()),
            (format!("blocked/{}", slug(blocker)), blocker.clone()),
        ]);
    }
    for thread in digest.busy.iter().filter(|thread| !thread.crashing) {
        let Some(place) = &thread.place else {
            continue;
        };
        let who = thread.name.as_deref().unwrap_or(&thread.kind);
        let top = format!("t-{}", slug(who));
        busy.push(vec![
            (top.clone(), who.to_string()),
            (format!("{}/{}", top, slug(place)), place.clone()),
        ]);
    }
    if busy.is_empty() {
        busy.push(vec![("none".to_string(), "nothing".to_string())]);
    }
    // (Its distinctive busy threads' files' subsystems and directories,
    // nested: not the crashing thread's, nor what most of the set's crashes
    // had busy, which say nothing of it.)
    let is_common = |thread: &BusyThread| {
        thread.place.as_ref().is_some_and(|place| {
            let who = thread.name.as_deref().unwrap_or(&thread.kind);
            let key = format!("{} in {}", who, place);
            view.common.iter().any(|(common, _)| *common == key)
        })
    };
    let paths: BTreeSet<&String> = digest
        .busy
        .iter()
        .filter(|thread| !thread.crashing && !is_common(thread))
        .flat_map(|thread| &thread.paths)
        .collect();
    let nested = |text: &str| -> ValuePath {
        let mut path = vec![];
        let mut so_far = String::new();
        for segment in text.split('/').filter(|segment| !segment.is_empty()) {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(segment);
            path.push((so_far.clone(), segment.to_string()));
        }
        path
    };
    let subsystems: Vec<ValuePath> = paths
        .iter()
        .filter_map(|path| view.subsystems.get(*path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|subsystem| nested(subsystem))
        .collect();
    let directories: Vec<ValuePath> = paths
        .iter()
        .filter_map(|path| path.rsplit_once('/').map(|(dir, _)| dir))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(nested)
        .collect();
    let one = |value: &str| match value.is_empty() {
        true => vec![vec![("unknown".to_string(), "unknown".to_string())]],
        false => vec![vec![(slug(value), value.to_string())]],
    };
    vec![
        (
            "family",
            vec![vec![
                (family.value.clone(), family.key.clone()),
                (variant.value.clone(), variant.label.clone()),
            ]],
        ),
        ("busy", busy),
        ("subsystem", subsystems),
        ("directory", directories),
        ("version", one(&digest.version)),
        ("platform", one(&digest.platform)),
        ("process", one(&digest.process_type)),
    ]
}

/// The values of a crash's facets (see `crash_facet_values`), as its row's
/// `data-facets` has them: each value along each of its paths, once.
fn flat_facet_values(paths: &[ValuePath]) -> Vec<String> {
    paths
        .iter()
        .flatten()
        .map(|(value, _)| value.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The navigation panel's facets of a set's view (see `TextFile::facets`),
/// whose items are its crashes' rows (see `crash_facet_values`): their
/// families (with their variants nested), what their other threads were
/// busy with (by thread, then where), the subsystems and directories
/// involved (nested), their versions, platforms and process types, with how
/// many crashes have each.
pub fn set_facets(view: &CrashSetView) -> Value {
    use crate::cmd_pipeline::facets::{FacetValueView, FacetView};
    #[derive(Default)]
    struct Level {
        name: String,
        count: u32,
        nested: BTreeMap<String, Level>,
    }
    let mut roots: BTreeMap<&'static str, Level> = BTreeMap::new();
    for at in 0..view.digests.len() {
        for (key, paths) in crash_facet_values(view, at) {
            let root = roots.entry(key).or_default();
            // (A crash counts once for each value, whatever its paths through
            // it.)
            let mut counted: BTreeSet<String> = BTreeSet::new();
            for path in paths {
                let mut level = &mut *root;
                for (value, name) in path {
                    level = level.nested.entry(value.clone()).or_default();
                    level.name = name;
                    if counted.insert(value) {
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
        ("subsystem", "Subsystems involved"),
        ("directory", "Directories involved"),
        ("version", "Version"),
        ("platform", "Platform"),
        ("process", "Process type"),
    ] {
        if let Some(level) = roots.remove(key)
            && !level.nested.is_empty()
        {
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
/// page of it, when it was fetched), the diagrams of what they were busy with
/// (see `stack_graphs`; static/js/crash-set.js draws them), its families
/// (each with its variants, how many crashes each has, and its latest crash's
/// view; the biggest few open), and its crashes' table (a row each, which the
/// navigation panel's facets filter; see `set_facets`).
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
    // The diagrams of what they were busy with (see `stack_graphs`), which
    // crash-set.js draws.
    if !view.graphs.is_empty() {
        html.push_str(&format!(
            r#"<section class="crash-set-graphs"><h3 class="crash-set-heading">What they were busy with</h3><p class="crash-set-about">Their threads' stacks (but the crashing one's, and what most of them had), in the tree's code but libraries', callers over callees, a diagram for each connected part, the biggest first: each function's color says how many of the stacks have it, and its number how many.</p><div class="crash-graphs"></div><script type="application/json" id="crash-set-graphs">{}</script></section>"#,
            serde_json::to_string(&view.graphs)
                .unwrap_or_else(|_| "[]".to_string())
                .replace('<', "\\u003c")
        ));
    }
    html.push_str(
        r#"<section class="crash-families"><h3 class="crash-set-heading">Their families</h3>"#,
    );
    for (position, family) in view.families.iter().enumerate() {
        html.push_str(&format!(
            r#"<details class="crash-family"{} data-family="{}"><summary><span class="crash-family-key">{}</span> <span class="crash-family-count">{}</span></summary><ul class="crash-variants">"#,
            // (The biggest few open.)
            match position < 3 {
                true => " open",
                false => "",
            },
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
    html.push_str(r#"<h3 class="crash-set-heading">Their crashes</h3><table class="crash-set-crashes"><thead><tr><th>Crash</th><th>Date</th><th>Version</th><th>Platform</th><th>Process</th><th>Family</th><th>Variant</th></tr></thead><tbody class="facet-items">"#);
    for (at, digest) in view.digests.iter().enumerate() {
        let facets: serde_json::Map<String, Value> = crash_facet_values(view, at)
            .into_iter()
            .map(|(key, paths)| {
                (
                    key.to_string(),
                    flat_facet_values(&paths)
                        .into_iter()
                        .map(Value::String)
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
    // (Its use, for the next index's carry-over; see `CrashSetStore::touch`.)
    html.push_str(&format!(
        r#"<script>navigator.sendBeacon?.("/{}/crash-sets/touch/{}");</script>"#,
        escape(tree_name),
        escape(&view.id)
    ));
    // (graph.js's layout, for the diagrams.)
    html.push_str(&format!(
        r#"<script defer src="/{tree}/static/js/graph.js"></script><script defer src="/{tree}/static/js/graph-records.js"></script><script defer src="/{tree}/static/js/crash-set.js"></script>"#,
        tree = escape(tree_name)
    ));
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
                // (Its function, called by a dispatcher of its place's.)
                stack: vec![
                    BusyFrame {
                        function: code.to_string(),
                        place: place.unwrap_or("?").to_string(),
                    },
                    BusyFrame {
                        function: format!("Dispatch{}", place.unwrap_or("?").len()),
                        place: place.unwrap_or("?").to_string(),
                    },
                ],
                paths: place.map_or_else(Vec::new, |place| vec![format!("{}/File.cpp", place)]),
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
        // (What they were busy with, by thread, then where.)
        let busy = &facets["facets"][1];
        assert_eq!(busy["key"], "busy");
        assert_eq!(busy["values"][0]["name"], "Link Monitor");
        assert_eq!(busy["values"][0]["count"], 6);
        assert_eq!(
            busy["values"][0]["values"][0]["name"],
            "netwerk/system/win32"
        );
        assert_eq!(busy["values"][1]["name"], "QuotaManager IO");
        assert_eq!(
            busy["values"][1]["values"][0]["value"],
            "t-quotamanager-io/dom-quota"
        );
        // (The directories involved, but the crashing thread's and the common
        // ones', nested.)
        let directories = &facets["facets"][2];
        assert_eq!(directories["key"], "directory");
        let names: Vec<&str> = directories["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["dom"]);
        assert_eq!(directories["values"][0]["count"], 5);
        assert_eq!(directories["values"][0]["values"][0]["value"], "dom/quota");

        // The diagrams: of the stacks but those of the common feature, in
        // their connected parts, the biggest first, each function in it once
        // (in the order of the latest stacks') with how many stacks have it,
        // callers over callees.
        let graphs: Vec<(Vec<(&str, usize)>, usize)> = view
            .graphs
            .iter()
            .map(|graph| {
                (
                    graph
                        .nodes
                        .iter()
                        .map(|node| (node.function.as_str(), node.count))
                        .collect(),
                    graph.stacks.len(),
                )
            })
            .collect();
        assert_eq!(
            graphs,
            vec![
                (
                    vec![
                        ("CreateDirectoryMetadata2", 1),
                        ("Dispatch9", 3),
                        ("UnloadQuota", 2)
                    ],
                    3
                ),
                (vec![("DeleteAllCacheEntries", 2), ("Dispatch9", 2)], 2),
            ]
        );
        assert_eq!(
            view.graphs[0].edges[0],
            StackEdge {
                from: 1,
                to: 0,
                count: 1
            }
        );
    }

    #[test]
    fn test_from_form() {
        let fields = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect()
        };
        let search = CrashSearch::from_form(&fields(&[
            ("signature", " Foo | Bar "),
            ("proto_signature", ""),
            ("release_channel", "nightly"),
            ("release_channel", "beta"),
            ("versions", "146.0a1, 145.0"),
            ("platform", "Windows NT"),
            ("from", "2026-10-25"),
            ("through", "2026-10-31"),
            ("limit", "50"),
            ("token", "secret"),
        ]))
        .unwrap();
        assert_eq!(search.signature.as_deref(), Some("Foo | Bar"));
        assert_eq!(search.proto_signature, None);
        assert_eq!(search.product, "Firefox");
        assert_eq!(search.release_channels, vec!["nightly", "beta"]);
        assert_eq!(search.versions, vec!["146.0a1", "145.0"]);
        assert_eq!(search.date_to, "2026-11-01");
        assert_eq!(search.through(), "2026-10-31");
        // (Not the token, which isn't the search's.)
        assert!(!serde_json::to_string(&search).unwrap().contains("secret"));
        assert!(
            CrashSearch::from_form(&fields(&[
                ("from", "2026-10-25"),
                ("through", "2026-10-31"),
                ("limit", "50")
            ]))
            .is_err()
        );
        assert!(
            CrashSearch::from_form(&fields(&[
                ("signature", "Foo"),
                ("from", "2026-10-25"),
                ("through", "soon"),
                ("limit", "50")
            ]))
            .is_err()
        );

        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert_eq!(search.check_recent(today, false), Ok(()));
        let mut old = search.clone();
        old.date_from = "2026-09-01".to_string();
        old.date_to = "2026-09-10".to_string();
        assert!(old.check_recent(today, true).is_err());
        let mut many = search.clone();
        many.limit = MAX_CRASHES_WITHOUT_TOKEN + 1;
        assert!(many.check_recent(today, false).is_err());
        assert_eq!(many.check_recent(today, true), Ok(()));
    }

    /// A crash-stats stand-in (see `Api`) with `crashes`, which a token
    /// "privileged" sees protected data of, and a store, in a temporary
    /// directory, which it removes when dropped.
    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str, crashes: &[&str]) -> Fixture {
            let dir =
                std::env::temp_dir().join(format!("crash-sets-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("api/ProcessedCrash")).unwrap();
            let hits: Vec<Value> = crashes.iter().map(|id| json!({ "uuid": id })).collect();
            std::fs::write(
                dir.join("api/SuperSearch.json"),
                json!({ "hits": hits, "total": 7 }).to_string(),
            )
            .unwrap();
            std::fs::write(
                dir.join("api/SuperSearch.privileged.json"),
                json!({ "hits": [{ "uuid": crashes[0], "url": "https://example.com/" }] })
                    .to_string(),
            )
            .unwrap();
            for id in crashes {
                let crash = json!({
                    "uuid": id,
                    "signature": "Foo",
                    "user_comments": "protected",
                    "json_dump": { "crash_info": { "crashing_thread": 0 }, "threads": [] },
                });
                std::fs::write(
                    dir.join(format!("api/ProcessedCrash/{}.json", id)),
                    crash.to_string(),
                )
                .unwrap();
            }
            Fixture { dir }
        }

        fn api(&self) -> String {
            format!("file://{}", self.dir.join("api").display())
        }

        fn store(&self) -> CrashSetStore {
            CrashSetStore::new(self.dir.join("store"))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn test_fetch_set() {
        let crashes = [
            "5e4c0b1a-7e57-4c0d-8e5a-0000002611b1",
            "5e4c0b1a-7e57-4c0d-8e5a-0000002611b2",
        ];
        let fixture = Fixture::new("fetch", &crashes);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let progress = std::sync::Mutex::new(vec![]);
        let fetch = |token: Option<&str>| {
            runtime.block_on(fetch_set(
                &fixture.store(),
                &search(),
                &fixture.api(),
                token,
                &|fetched, total| progress.lock().unwrap().push((fetched, total)),
            ))
        };
        // (A token which can see protected data is refused.)
        assert!(fetch(Some("privileged")).unwrap_err().contains("protected"));
        assert!(fixture.store().crash(crashes[0]).unwrap().is_none());
        let set = fetch(Some("unprivileged")).unwrap();
        let manifest = fixture.store().manifest(&set).unwrap().unwrap();
        assert_eq!(manifest.crashes, crashes);
        assert_eq!(manifest.total, 7);
        assert_eq!(*progress.lock().unwrap(), vec![(0, 2), (1, 2), (2, 2)]);
        // (Whitelisted.)
        let stored = fixture.store().crash(crashes[1]).unwrap().unwrap();
        assert_eq!(stored["signature"], "Foo");
        assert!(stored.get("user_comments").is_none());
        // (Without a token too.)
        assert!(fetch(None).is_ok());

        // A store seeded with another reads it after its own, and writes its
        // own.
        let seeded = CrashSetStore::new(fixture.dir.join("other"))
            .with_seed(Some(fixture.dir.join("store")));
        assert_eq!(seeded.manifest(&set).unwrap(), Some(manifest));
        assert!(seeded.has_crash(crashes[0]));
        assert!(!fixture.dir.join("other").exists());
    }

    #[test]
    fn test_ingestion() {
        let crashes = ["5e4c0b1a-7e57-4c0d-8e5a-0000002611c1"];
        let fixture = Fixture::new("ingest", &crashes);
        let mut recent = search();
        let today = chrono::Utc::now().date_naive();
        recent.date_from = (today - chrono::Duration::days(3))
            .format("%Y-%m-%d")
            .to_string();
        recent.date_to = (today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let job = request_ingestion(
            fixture.store(),
            &fixture.api(),
            recent.clone(),
            None,
            Some("client".to_string()),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            let (searched, status) = ingestion_status(&job).unwrap();
            assert_eq!(searched, recent);
            if matches!(
                status,
                IngestionStatus::Done { .. } | IngestionStatus::Failed { .. }
            ) {
                break status;
            }
            assert!(std::time::Instant::now() < deadline, "{:?}", status);
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let IngestionStatus::Done { set } = status else {
            panic!("{:?}", status);
        };
        assert_eq!(
            fixture.store().manifest(&set).unwrap().unwrap().crashes,
            crashes
        );
        assert!(ingestion_status("0000000000000000").is_none());
        // (Old dates are refused before anything's queued.)
        let mut old = search();
        old.date_from = "2020-01-01".to_string();
        old.date_to = "2020-01-08".to_string();
        assert!(request_ingestion(fixture.store(), &fixture.api(), old, None, None).is_err());
    }

    #[test]
    fn test_cap() {
        let crashes = [
            "5e4c0b1a-7e57-4c0d-8e5a-0000002611d1",
            "5e4c0b1a-7e57-4c0d-8e5a-0000002611d2",
        ];
        let fixture = Fixture::new("cap", &crashes);
        let store = fixture.store();
        assert_eq!(store.cap(), DEFAULT_CAP_BYTES);
        std::fs::create_dir_all(store.root()).unwrap();
        std::fs::write(store.root().join("cap"), "1000\n").unwrap();
        assert_eq!(store.cap(), 1000);
        // (An ingestion which would go over it is refused: 100 crashes, at
        // `CRASH_BYTES_ESTIMATE` each in an empty store.)
        let mut recent = search();
        let today = chrono::Utc::now().date_naive();
        recent.date_from = (today - chrono::Duration::days(3))
            .format("%Y-%m-%d")
            .to_string();
        recent.date_to = (today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(
            request_ingestion(
                fixture.store(),
                &fixture.api(),
                recent,
                None,
                Some("cap client".to_string())
            ),
            Err(FULL.to_string())
        );
        // (A fetch which goes over it fails, and removes what it stored.)
        std::fs::write(store.root().join("cap"), "1").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fetch = || {
            runtime.block_on(fetch_set(
                &store,
                &search(),
                &fixture.api(),
                None,
                &|_, _| {},
            ))
        };
        assert_eq!(fetch(), Err(FULL.to_string()));
        assert!(!store.has_crash(crashes[0]));
        assert!(store.set_ids().is_empty());
        std::fs::remove_file(store.root().join("cap")).unwrap();
        let set = fetch().unwrap();
        assert_eq!(store.set_ids(), vec![set]);
        let (bytes, stored) = store.size();
        assert_eq!(stored, 2);
        assert!(bytes > 0);
    }

    #[test]
    fn test_carry_over() {
        let fixture = Fixture::new("carry", &["5e4c0b1a-7e57-4c0d-8e5a-0000002611e1"]);
        let store = fixture.store();
        let now = std::time::SystemTime::now();
        let day = std::time::Duration::from_secs(86400);
        let crash = |n: u32| format!("5e4c0b1a-7e57-4c0d-8e5a-0000002611{:02x}", 0xe0 + n);
        let set_time = |path: &Path, time: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        // (A set of `crashes`, last used `age` ago.)
        let add_set = |n: u32, crashes: &[u32], age: std::time::Duration| {
            for crash in crashes.iter().map(|n| crash(*n)) {
                if !store.has_crash(&crash) {
                    store
                        .store_crash(&crash, &json!({ "uuid": crash, "signature": "Foo" }))
                        .unwrap();
                }
            }
            let id = store
                .store_manifest(&CrashSetManifest {
                    version: MANIFEST_VERSION,
                    search: search(),
                    fetched_at: format!("2026-11-01T00:00:0{}Z", n),
                    total: crashes.len() as u64,
                    crashes: crashes.iter().map(|n| crash(*n)).collect(),
                })
                .unwrap();
            set_time(&store.root().join("touched").join(&id), now - age);
            id
        };
        // A, used now; B, 2 days ago, which has A's crash too; C, half a day
        // ago, but big (15 MB, as much as a day and a half); D, 5 days ago.
        let a = add_set(1, &[1], std::time::Duration::ZERO);
        let b = add_set(2, &[1, 2], 2 * day);
        std::fs::write(
            crash_path(store.root(), &crash(3), "json.gz"),
            vec![0u8; 15_000_000],
        )
        .unwrap();
        let c = add_set(3, &[3], day / 2);
        let d = add_set(4, &[4], 5 * day);
        let ids = |carried: &CarryOver| -> Vec<String> {
            carried.sets.iter().map(|set| set.id.clone()).collect()
        };
        let all = carry_over(&store, u64::MAX, now);
        assert_eq!(ids(&all), vec![a.clone(), b.clone(), c.clone(), d.clone()]);
        // (A crash once, with the first set which has it.)
        assert_eq!(
            all.files
                .iter()
                .filter(|path| path.contains(&crash(1)))
                .count(),
            1
        );
        assert_eq!(all.files.len(), 4 + 4);
        assert_eq!(all.bytes, all.sets.iter().map(|set| set.bytes).sum::<u64>());
        assert_eq!(
            all.sets[1].touched,
            (now - 2 * day)
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        // (Within a budget, those which fit: not C.)
        let small = carry_over(&store, 1_000_000, now);
        assert_eq!(ids(&small), vec![a.clone(), b.clone(), d.clone()]);
        assert!(small.bytes < 1_000_000);
        assert!(carry_over(&store, 0, now).sets.is_empty());
        // (A use makes it the first.)
        store.touch(&d).unwrap();
        assert_eq!(ids(&carry_over(&store, u64::MAX, now + day))[0], d);
        // (But at most once in `TOUCH_INTERVAL`.)
        let touched = store.touched(&d).unwrap();
        set_time(
            &store.root().join("touched").join(&d),
            touched - TOUCH_INTERVAL / 2,
        );
        store.touch(&d).unwrap();
        assert_eq!(store.touched(&d), Some(touched - TOUCH_INTERVAL / 2));
        assert!(store.touch("0000000000000000").is_err());
        assert!(store.touch("../x").is_err());

        // Their files, but no others.
        assert_eq!(
            carried_file(&store, &format!("sets/{}.json", a)),
            std::fs::read(store.root().join("sets").join(format!("{}.json", a))).ok()
        );
        let crash_file = format!("crashes/5e/{}.json.gz", crash(1));
        assert!(carried_file(&store, &crash_file).is_some());
        for path in [
            "cap".to_string(),
            format!("touched/{}", a),
            format!("crashes/5f/{}.json.gz", crash(1)),
            format!("crashes/5e/../5e/{}.json.gz", crash(1)),
            format!("sets/../sets/{}.json", a),
            "/etc/passwd".to_string(),
        ] {
            assert_eq!(carried_file(&store, &path), None, "{}", path);
        }
    }

    #[test]
    fn test_base64() {
        for (bytes, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(bytes.as_bytes()), encoded);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
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
