//! Sidecar files: data which a page's script loads after the page (ex: a
//! `/rev/` page's token-centric blame data, `/{tree}/rev-hyperblame/...`),
//! which the web-server writes under the tree's sidecar directory, by URL
//! (`/{tree}/{rest}` is `sidecar_dir(...)/{rest}`), when it computes them, for
//! nginx to serve (falling back to the web-server for those not written yet,
//! or evicted), rather than keeping them in memory, where they'd be evicted
//! before the page requested them under load.  The sidecar directories are
//! `sidecars/{tree}` beside the trees' index directories (ex:
//! `/index/sidecars/firefox-main`, beside `/index/nginx-cache` on the web
//! servers), and nginx doesn't cache the web-server's responses for them
//! (see nginx-setup.py), so nothing is cached twice.
//!
//! Files are written gzipped with an empty marker file, like output-file's
//! (nginx's `try_files` looks for the marker, and `gzip_static` serves the
//! `.gz`): the `.gz` is renamed into place before the marker is created, so
//! that nginx only finds complete files, and the janitor (see `janitor`)
//! removes the marker before the `.gz`.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::file_format::config::Config;

/// The janitor evicts files while less than this fraction of the filesystem
/// is free, until twice as much is.  (The SSD's free space is also its
/// over-provisioning, which keeps its writes fast.)
const MIN_FREE_FRACTION: f64 = 0.10;
/// How often the janitor checks.
const JANITOR_PERIOD: Duration = Duration::from_secs(5 * 60);

/// The sidecar directory of a tree whose index directory is `index_path`.
pub fn sidecar_dir(index_path: &str, tree_name: &str) -> PathBuf {
    sidecar_root(index_path).join(tree_name)
}

/// The directory of the trees' sidecar directories, beside their index
/// directories.
pub fn sidecar_root(index_path: &str) -> PathBuf {
    Path::new(index_path)
        .parent()
        .unwrap_or(Path::new("/"))
        .join("sidecars")
}

/// Write the sidecar file of `/{tree_name}/{rest}` (ex:
/// `rev-hyperblame/{rev}/{path}/lines-0.json`) for the tree whose index
/// directory is `index_path`, unless it would be outside the tree's sidecar
/// directory.  Failures only mean the web-server computes it again when it's
/// requested, so they're logged, not returned.
pub fn write(index_path: &str, tree_name: &str, rest: &str, contents: &str) {
    if rest.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..") {
        return;
    }
    let path = sidecar_dir(index_path, tree_name).join(rest);
    if let Err(e) = write_file(&path, contents) {
        log::warn!("Couldn't write the sidecar {}: {}", path.display(), e);
    }
}

fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if path.exists() {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(
        ".{}.{}.{}.tmp",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    {
        let mut writer = GzEncoder::new(File::create(&temp)?, Compression::default());
        writer.write_all(contents.as_bytes())?;
        writer.finish()?;
    }
    let mut gz = path.as_os_str().to_owned();
    gz.push(".gz");
    fs::rename(&temp, &gz)?;
    File::create(path)?;
    Ok(())
}

/// The free and total bytes of the filesystem with `path`, by `df`.
fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let output = std::process::Command::new("df")
        .args(["--output=avail,size", "-B1"])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut numbers = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .map(|n| n.parse::<u64>().ok());
    Some((numbers.next()??, numbers.next()??))
}

/// The sidecar files under `root` (their markers), with their last access
/// times (or modification times, if later) and their `.gz`s' sizes.
fn sidecar_files(root: &Path) -> Vec<(SystemTime, PathBuf, u64)> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy();
            if name.ends_with(".gz") || name.starts_with('.') {
                return None;
            }
            let mut gz = path.as_os_str().to_owned();
            gz.push(".gz");
            let meta = fs::metadata(&gz).ok()?;
            let used = meta.accessed().ok()?.max(meta.modified().ok()?);
            Some((used, path.to_path_buf(), meta.len()))
        })
        .collect()
}

/// Remove sidecar files under `root`, least recently used first, while less
/// than `MIN_FREE_FRACTION` of the filesystem is free, until twice as much
/// is, and return how many it removed.  (nginx's reads update the `.gz`s'
/// access times, at least daily with `relatime`.)
pub fn evict(root: &Path) -> usize {
    let Some((free, size)) = disk_space(root) else {
        return 0;
    };
    let min_free = (size as f64 * MIN_FREE_FRACTION) as u64;
    if free >= min_free {
        return 0;
    }
    let mut to_free = (2 * min_free).saturating_sub(free);
    let mut files = sidecar_files(root);
    files.sort();
    let mut removed = 0;
    for (_, marker, len) in files {
        if to_free == 0 {
            break;
        }
        // (The marker first, so that nginx doesn't find a marker without
        // its `.gz`.)
        let mut gz = marker.as_os_str().to_owned();
        gz.push(".gz");
        if fs::remove_file(&marker).is_ok() && fs::remove_file(&gz).is_ok() {
            to_free = to_free.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}

/// Remove the trees' sidecar directories (which may be of an earlier index,
/// ex: locally), and then evict sidecar files (see `evict`) periodically,
/// forever: for a web-server thread, when the web-server starts.
pub fn janitor(cfg: &Config) {
    for (tree_name, tree_config) in &cfg.trees {
        let dir = sidecar_dir(&tree_config.paths.index_path, tree_name);
        if dir.exists() {
            // (Renamed first, so that nginx stops finding them at once.)
            let old = dir.with_file_name(format!(".{}.old.{}", tree_name, std::process::id()));
            match fs::rename(&dir, &old) {
                Ok(()) => {
                    let _ = fs::remove_dir_all(&old);
                }
                Err(e) => log::warn!("Couldn't clear the sidecars {}: {}", dir.display(), e),
            }
        }
    }
    let mut roots: Vec<PathBuf> = cfg
        .trees
        .values()
        .map(|tree| sidecar_root(&tree.paths.index_path))
        .collect();
    roots.sort();
    roots.dedup();
    loop {
        for root in &roots {
            if root.exists() {
                let removed = evict(root);
                if removed > 0 {
                    log::info!("Evicted {} sidecar files from {}", removed, root.display());
                }
            }
        }
        std::thread::sleep(JANITOR_PERIOD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_file_and_files() {
        let root = std::env::temp_dir().join(format!("sidecars-test-{}", std::process::id()));
        let path = root.join("tree/rev-hyperblame/abc/a/b.cpp/lines-0.json");
        write_file(&path, "{\"x\": 1}").unwrap();
        assert!(path.exists(), "the marker");
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        let gz = root.join("tree/rev-hyperblame/abc/a/b.cpp/lines-0.json.gz");
        let mut decoder = flate2::read::GzDecoder::new(File::open(&gz).unwrap());
        let mut text = String::new();
        std::io::Read::read_to_string(&mut decoder, &mut text).unwrap();
        assert_eq!(text, "{\"x\": 1}");
        // (No temporary files are left.)
        let names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{:?}", names);
        let files = sidecar_files(&root);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, path);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_sidecar_dir() {
        assert_eq!(
            sidecar_dir("/index/firefox-main", "firefox-main"),
            PathBuf::from("/index/sidecars/firefox-main")
        );
    }
}
