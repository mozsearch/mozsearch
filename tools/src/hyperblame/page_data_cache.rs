//! A short-lived in-memory cache of the hyperblame data files (see
//! `format::hyperblame_files`) of the web-server's `/rev/` pages.
//!
//! Rendering a `/rev/` page computes its token-centric blame (for the blame
//! strip), which includes everything the page's `/rev-hyperblame/` requests
//! for its popup data need, and the page makes those requests right after it
//! loads.  So rendering puts the data here, and the requests use it rather than
//! computing the blame again.  Requests for a page whose data isn't here (ex:
//! nginx cached the page) compute it, and concurrent requests for the same page
//! (ex: for several chunks) wait for one computation.
//!
//! (The tip's data files are static files written by output-file.)

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::page_blame::PageBlame;

/// The web-server's cache.
pub static PAGE_DATA_CACHE: LazyLock<PageDataCache> = LazyLock::new(PageDataCache::default);

/// How long we keep a page's data.
const TTL: Duration = Duration::from_secs(30);
/// The most pages whose data we keep.  A huge file's data is about 1 MB.
const MAX_ENTRIES: usize = 16;

/// The hyperblame data of a page.
pub struct PageData {
    /// The JSON of each chunk (`lines-K.json`).
    pub chunks: Vec<String>,
    /// The page's commits (`BlameInfo::commits`), for `commits.json`.
    pub revs: Vec<String>,
    /// `commits.json`, which we only generate if it's requested.
    pub commits_json: OnceLock<String>,
}

impl PageData {
    pub fn new(page: &PageBlame) -> PageData {
        PageData {
            chunks: page
                .chunks
                .iter()
                .map(|chunk| serde_json::to_string(chunk).unwrap())
                .collect(),
            revs: page
                .info
                .commits
                .iter()
                .map(|(rev, _, _)| rev.clone())
                .collect(),
            commits_json: OnceLock::new(),
        }
    }
}

/// (tree, revision, path)
type Key = (String, String, String);

struct Entry {
    created: Instant,
    /// None if the page has no token-centric blame.
    data: OnceLock<Option<Arc<PageData>>>,
}

#[derive(Default)]
pub struct PageDataCache {
    entries: Mutex<HashMap<Key, Arc<Entry>>>,
}

impl PageDataCache {
    /// The entry for `key`, creating it if necessary, after dropping expired
    /// entries and making room.
    fn entry(&self, key: Key) -> Arc<Entry> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| entry.created.elapsed() < TTL);
        if let Some(entry) = entries.get(&key) {
            return entry.clone();
        }
        while entries.len() >= MAX_ENTRIES {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(key, _)| key.clone())
                .unwrap();
            entries.remove(&oldest);
        }
        let entry = Arc::new(Entry {
            created: Instant::now(),
            data: OnceLock::new(),
        });
        entries.insert(key, entry.clone());
        entry
    }

    /// Keep the data of a page we rendered.
    pub fn insert(&self, key: Key, data: PageData) {
        // If a request already computed it, the data is the same.
        let _ = self.entry(key).data.set(Some(Arc::new(data)));
    }

    /// The data of a page, computing it with `compute` if we don't have it.
    pub fn get_or_compute(
        &self,
        key: Key,
        compute: impl FnOnce() -> Option<PageData>,
    ) -> Option<Arc<PageData>> {
        self.entry(key)
            .data
            .get_or_init(|| compute().map(Arc::new))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(chunk: &str) -> PageData {
        PageData {
            chunks: vec![chunk.to_string()],
            revs: vec![],
            commits_json: OnceLock::new(),
        }
    }

    fn key(n: usize) -> Key {
        ("tree".to_string(), "rev".to_string(), format!("path{}", n))
    }

    #[test]
    fn test_page_data_cache() {
        let cache = PageDataCache::default();
        cache.insert(key(0), data("inserted"));
        let got = cache.get_or_compute(key(0), || panic!("should be cached"));
        assert_eq!(got.unwrap().chunks, vec!["inserted"]);

        // Misses compute (once), including pages without token blame.
        let got = cache.get_or_compute(key(1), || Some(data("computed")));
        assert_eq!(got.unwrap().chunks, vec!["computed"]);
        assert!(cache.get_or_compute(key(2), || None).is_none());
        assert!(
            cache
                .get_or_compute(key(2), || panic!("should be cached"))
                .is_none()
        );

        // The oldest entries make room for new ones.
        for n in 3..3 + MAX_ENTRIES {
            cache.insert(key(n), data("filler"));
        }
        let got = cache.get_or_compute(key(0), || Some(data("recomputed")));
        assert_eq!(got.unwrap().chunks, vec!["recomputed"]);
        assert!(cache.entries.lock().unwrap().len() <= MAX_ENTRIES);
    }
}
