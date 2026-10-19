//! A disk cache of pipeline steps' results (ex: `slice-rev`'s slicing; see
//! `cmd_dataflow`), for the pipeline-server, which initializes it (see
//! `init`); without it (ex: searchfox-tool, the checks), steps compute their
//! results every time.
//!
//! A step's result is cached by its server's identity (see
//! `AbstractServer::cache_identity`: the index's, and a try push's delta's),
//! its name, and its arguments (all of those that change it, canonically), as
//! deterministic over those, so that queries differing in what comes after the
//! step (ex: which cluster, which format) share it, where nginx's cache only
//! shares identical URLs' responses.  Files are under `step-cache/`, beside the
//! trees' index directories (as the web-server's sidecars are; see
//! `sidecars`), gzipped, written to a temporary file and renamed into place,
//! with their keys (checked when read, for the hash's collisions).
//!
//! - Startup: the directory is removed if it's another binary's (by its
//!   `.stamp`, as the sidecars' are), since its steps' results could differ,
//!   but kept across a crash's restart.  (So rebuilding the pipeline-server
//!   clears it, for development.)
//! - Space: at most `CAP_BYTES` (its share of the space index.sh reserves for
//!   nginx's cache; see nginx-setup.py), least recently used first (by their
//!   modification times, which hits update), checked periodically.
//! - Concurrency: a step's result is computed once at a time (others asking
//!   for it then wait for it), and at most `MAX_COMPUTING` cached steps are
//!   computed at once (ex: slicings peak at a few hundred MB).

use std::collections::HashMap;
use std::fs::{self, File};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::Value;

/// The cache's space (2 GB; asuth, 2026-10-17).
pub const CAP_BYTES: u64 = 2_000_000_000;
/// How many cached steps are computed at once at most.
const MAX_COMPUTING: usize = 2;
/// How often the janitor checks the cache's size.
const JANITOR_PERIOD: Duration = Duration::from_secs(5 * 60);

pub struct StepCache {
    root: PathBuf,
    /// The keys being computed, which others wait for.
    computing: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    permits: tokio::sync::Semaphore,
}

static CACHE: OnceLock<StepCache> = OnceLock::new();

/// Start caching in `root` (removing it if it's another binary's), with a
/// janitor keeping it within `cap` bytes: for the pipeline-server, once.
pub fn init(root: &Path, cap: u64) {
    let stamp = binary_stamp();
    let stamp_path = root.join(".stamp");
    if fs::read_to_string(&stamp_path).ok().as_deref() != Some(stamp.as_str()) && root.exists() {
        let old = root.with_file_name(format!(".step-cache.old.{}", std::process::id()));
        if fs::rename(root, &old).is_ok() {
            std::thread::spawn(move || fs::remove_dir_all(&old));
        }
    }
    if fs::create_dir_all(root).is_ok() {
        let _ = fs::write(&stamp_path, &stamp);
    }
    let cache = StepCache {
        root: root.to_path_buf(),
        computing: Mutex::new(HashMap::new()),
        permits: tokio::sync::Semaphore::new(MAX_COMPUTING),
    };
    if CACHE.set(cache).is_ok() {
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            loop {
                evict(&root, cap);
                std::thread::sleep(JANITOR_PERIOD);
            }
        });
    }
}

/// What the binary's results could depend on: the binary itself.
fn binary_stamp() -> String {
    use std::os::unix::fs::MetadataExt;
    let exe = std::env::current_exe().unwrap_or_default();
    match fs::metadata(&exe) {
        Ok(meta) => format!("binary {} {} {}\n", meta.ino(), meta.mtime(), meta.len()),
        Err(_) => "binary -\n".to_string(),
    }
}

/// A step's result: from the cache, if there's one (see `init`) and `identity`
/// (the server's; see `AbstractServer::cache_identity`) and it has it, else
/// computed (and then cached, unless it failed).
pub async fn cached<E, F, Fut>(
    identity: Option<String>,
    step: &str,
    args: &str,
    compute: F,
) -> Result<Value, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value, E>>,
{
    let (Some(cache), Some(identity)) = (CACHE.get(), identity) else {
        return compute().await;
    };
    cache.get_or_compute(&identity, step, args, compute).await
}

/// Whether the cache has a step's result (by its file, without reading it;
/// for estimates, see `PipelineCommand::estimate`).
pub fn is_cached(identity: Option<String>, step: &str, args: &str) -> bool {
    let (Some(cache), Some(identity)) = (CACHE.get(), identity) else {
        return false;
    };
    cache.has(&identity, step, args)
}

impl StepCache {
    fn key(identity: &str, step: &str, args: &str) -> String {
        format!("{}\n{}\n{}", identity, step, args)
    }

    fn has(&self, identity: &str, step: &str, args: &str) -> bool {
        self.path(&Self::key(identity, step, args)).is_file()
    }

    fn path(&self, key: &str) -> PathBuf {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = format!("{:016x}", hasher.finish());
        self.root.join(&hash[..2]).join(format!("{}.json.gz", hash))
    }

    /// The cached result, if it's there (and of this key).
    fn read(&self, key: &str) -> Option<Value> {
        let path = self.path(key);
        let mut text = String::new();
        GzDecoder::new(File::open(&path).ok()?)
            .read_to_string(&mut text)
            .ok()?;
        let (stored_key, value) = text.split_once("\n\n")?;
        if stored_key != key {
            return None;
        }
        // (A use: the janitor evicts the least recently used.)
        if let Ok(file) = File::options().append(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        serde_json::from_str(value).ok()
    }

    fn write(&self, key: &str, value: &Value) -> std::io::Result<()> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = self.path(key);
        let dir = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir)?;
        let temp = dir.join(format!(
            ".{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        {
            let mut writer = GzEncoder::new(File::create(&temp)?, Compression::default());
            writer.write_all(key.as_bytes())?;
            writer.write_all(b"\n\n")?;
            serde_json::to_writer(&mut writer, value)?;
            writer.finish()?;
        }
        fs::rename(&temp, &path)
    }

    async fn get_or_compute<E, F, Fut>(
        &self,
        identity: &str,
        step: &str,
        args: &str,
        compute: F,
    ) -> Result<Value, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, E>>,
    {
        let key = Self::key(identity, step, args);
        if let Some(value) = self.read(&key) {
            return Ok(value);
        }
        // (One computation of a key at a time: the others wait for its
        // result.)
        let lock = self
            .computing
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        let _computing = lock.lock().await;
        if let Some(value) = self.read(&key) {
            return Ok(value);
        }
        let result = {
            let _permit = self.permits.acquire().await;
            compute().await
        };
        if let Ok(value) = &result
            && let Err(e) = self.write(&key, value)
        {
            log::warn!("Couldn't cache {}'s result: {}", step, e);
        }
        self.computing.lock().unwrap().remove(&key);
        result
    }
}

/// Remove cached results under `root`, least recently used first, while they
/// take more than `cap` bytes, until they take at most 90% of it, and return
/// how many it removed.
pub fn evict(root: &Path, cap: u64) -> usize {
    let mut files: Vec<(SystemTime, PathBuf, u64)> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file() && entry.file_name().to_string_lossy().ends_with(".json.gz")
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((
                meta.modified().ok()?,
                entry.path().to_path_buf(),
                meta.len(),
            ))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, _, len)| len).sum();
    if total <= cap {
        return 0;
    }
    files.sort();
    let target = cap / 10 * 9;
    let mut removed = 0;
    for (_, path, len) in files {
        if total <= target {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cache(root: &Path) -> StepCache {
        StepCache {
            root: root.to_path_buf(),
            computing: Mutex::new(HashMap::new()),
            permits: tokio::sync::Semaphore::new(MAX_COMPUTING),
        }
    }

    #[tokio::test]
    async fn test_cached_once() {
        let dir = std::env::temp_dir().join(format!("step-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = cache(&dir);
        let computed = AtomicU64::new(0);
        let compute = || async {
            computed.fetch_add(1, Ordering::Relaxed);
            Ok::<Value, ()>(json!({"clusters": [[0, 1]]}))
        };
        let first = cache
            .get_or_compute("tree 1", "slice", "a..b", compute)
            .await;
        let second = cache
            .get_or_compute("tree 1", "slice", "a..b", compute)
            .await;
        assert_eq!(first, second);
        assert_eq!(computed.load(Ordering::Relaxed), 1);
        assert!(cache.has("tree 1", "slice", "a..b"));
        assert!(!cache.has("tree 2", "slice", "a..b"));
        // (Another server's, or other arguments, are another result.)
        cache
            .get_or_compute("tree 2", "slice", "a..b", compute)
            .await
            .unwrap();
        cache
            .get_or_compute("tree 1", "slice", "a..c", compute)
            .await
            .unwrap();
        assert_eq!(computed.load(Ordering::Relaxed), 3);
        // (Failures aren't cached.)
        let failed = cache
            .get_or_compute("tree 1", "slice", "bad", || async {
                Err::<Value, &str>("no")
            })
            .await;
        assert_eq!(failed, Err("no"));
        assert!(
            cache
                .read(&StepCache::key("tree 1", "slice", "bad"))
                .is_none()
        );
        // (Eviction, least recently used first.)
        assert_eq!(evict(&dir, 1), 3);
        fs::remove_dir_all(&dir).unwrap();
    }
}
