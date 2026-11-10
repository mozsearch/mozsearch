//! A disk cache of files' IR (`frontend::build_file_ir`'s, before the tracer
//! canonicalizes its IPDL steps), for the pipeline-server's tracers (see
//! `Tracer::file`), which initializes it (see `init`): parsing the files a
//! slicing or trace enters is most of its time (on config8's web server, a
//! try push's slicing built 295 files' IR in 14.5 s, third_party/sqlite3's
//! sqlite3.c in 5.3 s), and the files are mostly the same from one query to
//! the next (a try push's unchanged files are its base's; see
//! `AbstractServer::file_cache_identity`).  Without it (ex: searchfox-tool, the
//! checks), files' IR is built every time.
//!
//! A file's IR is cached by its server's identity and its path, if building
//! it took at least `MIN_BUILD_TO_CACHE` (reading a small file's back takes
//! about as long as building it).  Files are under `file-ir-cache/`, beside
//! the step cache's directory (see `step_cache`), gzipped JSON, with their
//! keys (checked when read), written to a temporary file and renamed into
//! place; the directory is removed if it's another binary's (whose IR could
//! differ), and its least recently used files when they take more than
//! `CAP_BYTES`.  Each tracer logs how many files it read and built (see
//! `IrCacheStats`), for deciding whether to keep it.

use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use super::frontend::FileIr;

/// The cache's space (20 GB; the web servers' index disks have over a
/// terabyte free).
pub const CAP_BYTES: u64 = 20_000_000_000;
/// How long building a file's IR takes at least for it to be cached.
pub const MIN_BUILD_TO_CACHE: Duration = Duration::from_millis(20);
/// How often the janitor checks the cache's size.
const JANITOR_PERIOD: Duration = Duration::from_secs(10 * 60);

struct IrCache {
    root: PathBuf,
}

static CACHE: OnceLock<IrCache> = OnceLock::new();

/// Cache files' IR under `root` (removed first if another binary's), at most
/// `cap` bytes of it.
pub fn init(root: &Path, cap: u64) {
    let stamp = crate::step_cache::binary_stamp();
    let stamp_path = root.join(".stamp");
    if fs::read_to_string(&stamp_path).ok().as_deref() != Some(stamp.as_str()) && root.exists() {
        let old = root.with_file_name(format!(".file-ir-cache.old.{}", std::process::id()));
        if fs::rename(root, &old).is_ok() {
            std::thread::spawn(move || fs::remove_dir_all(&old));
        }
    }
    if fs::create_dir_all(root).is_ok() {
        let _ = fs::write(&stamp_path, &stamp);
    }
    if CACHE
        .set(IrCache {
            root: root.to_path_buf(),
        })
        .is_ok()
    {
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            loop {
                crate::step_cache::evict(&root, cap);
                std::thread::sleep(JANITOR_PERIOD);
            }
        });
    }
}

/// Whether there's a cache (see `init`).
pub fn enabled() -> bool {
    CACHE.get().is_some()
}

fn key(identity: &str, path: &str) -> String {
    format!("{}\n{}", identity, path)
}

impl IrCache {
    fn path(&self, key: &str) -> PathBuf {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = format!("{:016x}", hasher.finish());
        self.root.join(&hash[..2]).join(format!("{}.json.gz", hash))
    }
}

/// A file's cached IR (`identity`, its server's; see
/// `AbstractServer::file_cache_identity`), if it's there.
pub fn read(identity: &str, path: &str) -> Option<FileIr> {
    CACHE.get()?.read(identity, path)
}

/// Cache a file's IR, and return whether it was.
pub fn write(identity: &str, path: &str, ir: &FileIr) -> bool {
    CACHE
        .get()
        .is_some_and(|cache| cache.write(identity, path, ir))
}

impl IrCache {
    fn read(&self, identity: &str, path: &str) -> Option<FileIr> {
        let cache = self;
        let key = key(identity, path);
        let file_path = cache.path(&key);
        let mut reader = BufReader::new(GzDecoder::new(File::open(&file_path).ok()?));
        // (Its key's two lines, then its IR.)
        let mut stored = String::new();
        for _ in 0..2 {
            std::io::BufRead::read_line(&mut reader, &mut stored).ok()?;
        }
        if stored.trim_end_matches('\n') != key {
            return None;
        }
        let ir = serde_json::from_reader(reader).ok()?;
        // (A use: the janitor evicts the least recently used.)
        if let Ok(file) = File::options().append(true).open(&file_path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(ir)
    }

    fn write(&self, identity: &str, path: &str, ir: &FileIr) -> bool {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let cache = self;
        let key = key(identity, path);
        let file_path = cache.path(&key);
        let dir = file_path.parent().unwrap_or(&cache.root);
        let temp = dir.join(format!(
            ".{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| -> std::io::Result<()> {
            fs::create_dir_all(dir)?;
            let mut writer =
                BufWriter::new(GzEncoder::new(File::create(&temp)?, Compression::fast()));
            writer.write_all(key.as_bytes())?;
            writer.write_all(b"\n")?;
            serde_json::to_writer(&mut writer, ir)?;
            writer.into_inner().map_err(|e| e.into_error())?.finish()?;
            fs::rename(&temp, &file_path)
        })();
        if let Err(e) = &written {
            let _ = fs::remove_file(&temp);
            log::warn!("Couldn't cache {}'s IR: {}", path, e);
        }
        written.is_ok()
    }
}

/// A tracer's use of the cache, which it logs (see `Tracer`'s `Drop`).
#[derive(Default)]
pub struct IrCacheStats {
    /// The files whose IR was read from the cache, and how long that took.
    pub hits: usize,
    pub hit_time: Duration,
    /// The files whose IR was built, how long that took, and how many of
    /// them were cached.
    pub misses: usize,
    pub miss_time: Duration,
    pub written: usize,
}

impl IrCacheStats {
    pub fn add(&mut self, other: IrCacheStats) {
        self.hits += other.hits;
        self.hit_time += other.hit_time;
        self.misses += other.misses;
        self.miss_time += other.miss_time;
        self.written += other.written;
    }

    pub fn summary(&self) -> String {
        format!(
            "{} files read in {:.1} s; {} built in {:.1} s ({} cached)",
            self.hits,
            self.hit_time.as_secs_f64(),
            self.misses,
            self.miss_time.as_secs_f64(),
            self.written
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataflow::frontend::build_file_ir;
    use crate::dataflow::records::FileRecords;

    #[test]
    fn test_round_trip() {
        let root = std::env::temp_dir().join(format!("ir-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // (Its own, not the process's, which other tests' tracers would use.)
        let cache = IrCache { root: root.clone() };
        let source =
            "struct S { int f; };\nint g(S& s, int x) {\n  s.f = x;\n  return s.f + 1;\n}\n";
        let ir = build_file_ir("a.cpp", source, &FileRecords::default()).unwrap();
        assert!(cache.read("index 1", "a.cpp").is_none());
        assert!(cache.write("index 1", "a.cpp", &ir));
        let back = cache.read("index 1", "a.cpp").unwrap();
        // (As values, whose maps' orders don't matter.)
        assert_eq!(
            serde_json::to_value(&back).unwrap(),
            serde_json::to_value(&ir).unwrap()
        );
        // (By its server's identity and its path.)
        assert!(cache.read("index 2", "a.cpp").is_none());
        assert!(cache.read("index 1", "b.cpp").is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
