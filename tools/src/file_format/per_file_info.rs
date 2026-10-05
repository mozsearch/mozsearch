use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use lexical_sort::natural_lexical_cmp;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, from_reader};
use ustr::{Ustr, UstrMap, existing_ustr};

use crate::abstract_server::{FileMatch, FileMatches, Result};

use super::config::Config;
use super::repo_data_ingestion::{
    ConcisePerFileInfo, DetailedPerFileInfo, PathKindConfig, RepoIngestionConfig,
    heuristic_path_kind,
};

/// Provides access to (concise) per-file info via a pre-loaded copy of
/// `concise-per-file-info.json` and any derived indices.  This exact same
/// information is also available inside the crossref database as
/// `FILE_`-prefixed symbols.
///
/// The reasons to favor using this implementation (or growing this
/// implementation):
/// - Searching for a subset of files in the tree, including using additional
///   constraints that can be pre-computed.
///   - The crate https://github.com/lun3x/multi_index_map has tentatively
///     been identified as a way to aid in precomputation.
/// - Up-front file I/O and object allocation versus crossref-lookup which
///   loads/allocates JSON each time.  This data is able to be shared immutably.
#[derive(Clone, Debug)]
pub struct FileLookupMap {
    // We are able to safely use a UstrMap here because we ensure that in cases
    // where we're dealing with non-Ustr values that we do not create new Ustrs
    // for paths that do not exist through use of `existing_ustr`.
    concise_per_file: Arc<UstrMap<ConcisePerFileInfo<Ustr>>>,
}

impl FileLookupMap {
    pub fn new(concise_file_path: &str) -> Self {
        let components_file = File::open(concise_file_path).unwrap();
        let mut reader = BufReader::new(&components_file);
        let map: UstrMap<ConcisePerFileInfo<Ustr>> = from_reader(&mut reader).unwrap();
        FileLookupMap {
            concise_per_file: Arc::new(map),
        }
    }

    /// File lookup for when you have an existing Ustr; under no circumstances
    /// should you mint a new Ustr for a potential path from content.  If that's
    /// what you have, use `lookup_file_from_str` if it's a one-off, or use
    /// `existing_ustr` if you will be using the path multiple times.
    ///
    /// The general concern is to avoid interning a bunch of incorrect query
    /// strings.
    pub fn lookup_file_from_ustr(&self, path_ustr: &Ustr) -> Option<&ConcisePerFileInfo<Ustr>> {
        self.concise_per_file.get(path_ustr)
    }

    /// File lookup when we don't have a Ustr already available; this is
    /// the appropriate call-site to use if you have a web-sourced potential
    /// path string which could be wrong (and therefore should not be interned).
    pub fn lookup_file_from_str(&self, path: &str) -> Option<&ConcisePerFileInfo<Ustr>> {
        if let Some(path_ustr) = existing_ustr(path) {
            self.concise_per_file.get(&path_ustr)
        } else {
            None
        }
    }

    /// Search the list of files by applying a regexp to the paths.
    pub fn search_files(
        &self,
        pathre: &str,
        include_dirs: bool,
        limit: usize,
    ) -> Result<FileMatches> {
        let re_path = Regex::new(pathre)?;
        let mut matches: Vec<FileMatch> = self
            .concise_per_file
            .iter()
            .filter(|v| {
                if !include_dirs && v.1.is_dir {
                    false
                } else {
                    re_path.is_match(v.0)
                }
            })
            .map(|v| FileMatch {
                path: *v.0,
                concise: v.1.clone(),
            })
            .take(limit)
            .collect();
        matches.sort_unstable_by(|a, b| natural_lexical_cmp(&a.path, &b.path));
        Ok(FileMatches {
            file_matches: matches,
        })
    }
}

/// What the `/explore/` pages and interdiff summaries facet files on (see
/// `format::explore_facets`): each file's path kind and subsystem as of the
/// tree's indexed revision, from `concise-per-file-info.json`, and the tree's
/// path kinds (from `per-file-info.toml`), whose heuristics guess the kinds of
/// the paths it doesn't have (ex: files deleted since).
///
/// This is a compact copy of the concise information (unlike `FileLookupMap`,
/// which the web server doesn't load), since firefox's is ~260 MB of JSON, most
/// of it descriptions, for ~500k files: the files' names by directory, with
/// indices of their path kinds and subsystems, which is ~33 MB for firefox
/// rather than the ~127 MB of interning their paths, and loads in ~0.4s.
pub struct PathFacetMap {
    /// Each directory's (ex: "dom/base", or "" at the top) files: their names
    /// (concatenated, sorted), and the end of each name with the indices of
    /// its path kind in `kinds` and its subsystem in `subsystems` (or
    /// `NO_SUBSYSTEM`).
    dirs: HashMap<Box<str>, DirFiles>,
    kinds: Vec<Ustr>,
    subsystems: Vec<Ustr>,
    path_kinds: BTreeMap<Ustr, PathKindConfig>,
    files: usize,
}

/// A directory's files in a `PathFacetMap`.
type DirFiles = (String, Vec<(u32, u16, u16)>);

const NO_SUBSYSTEM: u16 = u16::MAX;

#[derive(Deserialize)]
struct SlimConciseInfo {
    path_kind: String,
    is_dir: bool,
    #[serde(default)]
    subsystem: Option<String>,
}

/// Builds a `PathFacetMap` from the concise JSON's map of paths, one at a time
/// (rather than deserializing all of it first).
struct PathFacetMapBuilder<'b> {
    map: &'b mut PathFacetMap,
    kind_indices: HashMap<String, u16>,
    subsystem_indices: HashMap<String, u16>,
}

impl<'de> serde::de::Visitor<'de> for PathFacetMapBuilder<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a map of paths to their concise information")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(
        mut self,
        mut access: A,
    ) -> std::result::Result<(), A::Error> {
        fn index(values: &mut Vec<Ustr>, indices: &mut HashMap<String, u16>, value: String) -> u16 {
            *indices.entry(value).or_insert_with_key(|value| {
                values.push(ustr::ustr(value));
                (values.len() - 1) as u16
            })
        }
        while let Some((path, info)) = access.next_entry::<String, SlimConciseInfo>()? {
            if info.is_dir {
                continue;
            }
            let kind = index(&mut self.map.kinds, &mut self.kind_indices, info.path_kind);
            let subsystem = match info.subsystem {
                Some(subsystem) => index(
                    &mut self.map.subsystems,
                    &mut self.subsystem_indices,
                    subsystem,
                ),
                None => NO_SUBSYSTEM,
            };
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", &path));
            let (names, files) = match self.map.dirs.get_mut(dir) {
                Some(entry) => entry,
                None => self.map.dirs.entry(dir.into()).or_default(),
            };
            names.push_str(name);
            files.push((names.len() as u32, kind, subsystem));
            self.map.files += 1;
        }
        Ok(())
    }
}

impl PathFacetMap {
    pub fn load(
        concise_file_path: &str,
        per_file_info_toml: &str,
    ) -> std::result::Result<Self, String> {
        // (Parsing a slice is several times faster than `from_reader`, and a
        // map's pages are the page cache's rather than a copy's.)
        let file = File::open(concise_file_path).map_err(|e| e.to_string())?;
        let contents = unsafe { memmap::Mmap::map(&file) }.map_err(|e| e.to_string())?;
        Self::parse(&contents, per_file_info_toml)
    }

    /// The map of the JSON `concise_json` of `concise-per-file-info.json`.
    pub fn parse(
        concise_json: &[u8],
        per_file_info_toml: &str,
    ) -> std::result::Result<Self, String> {
        let config: RepoIngestionConfig =
            toml::from_str(per_file_info_toml).map_err(|e| e.to_string())?;
        if config.pathkind.is_empty() {
            return Err("per-file-info.toml has no path kinds".to_string());
        }
        let mut map = PathFacetMap {
            dirs: HashMap::new(),
            kinds: vec![],
            subsystems: vec![],
            path_kinds: config.pathkind,
            files: 0,
        };
        let mut deserializer = serde_json::Deserializer::from_slice(concise_json);
        serde::Deserializer::deserialize_map(
            &mut deserializer,
            PathFacetMapBuilder {
                map: &mut map,
                kind_indices: HashMap::new(),
                subsystem_indices: HashMap::new(),
            },
        )
        .map_err(|e| e.to_string())?;

        // Sort each directory's names (which the JSON's order probably already
        // did).
        for (names, files) in map.dirs.values_mut() {
            let mut entries: Vec<(&str, u16, u16)> = files
                .iter()
                .enumerate()
                .map(|(i, &(end, kind, subsystem))| {
                    let start = if i == 0 { 0 } else { files[i - 1].0 };
                    (&names[start as usize..end as usize], kind, subsystem)
                })
                .collect();
            if entries.is_sorted_by(|a, b| a.0 <= b.0) {
                continue;
            }
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            let mut sorted_names = String::with_capacity(names.len());
            let mut sorted_files = Vec::with_capacity(files.len());
            for (name, kind, subsystem) in entries {
                sorted_names.push_str(name);
                sorted_files.push((sorted_names.len() as u32, kind, subsystem));
            }
            *names = sorted_names;
            *files = sorted_files;
        }
        map.dirs.shrink_to_fit();
        for (names, files) in map.dirs.values_mut() {
            names.shrink_to_fit();
            files.shrink_to_fit();
        }
        Ok(map)
    }

    /// The indexed revision's path kind and subsystem indices of the file at
    /// `path`, if it has the file.
    fn lookup(&self, path: &str) -> Option<(u16, u16)> {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let (names, files) = self.dirs.get(dir)?;
        let name_at = |i: usize| {
            let start = if i == 0 { 0 } else { files[i - 1].0 as usize };
            &names[start..files[i].0 as usize]
        };
        // (A binary search over the indices, since the names aren't a slice.)
        let (mut low, mut high) = (0, files.len());
        while low < high {
            let mid = (low + high) / 2;
            match name_at(mid).cmp(name) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid,
                std::cmp::Ordering::Equal => return Some((files[mid].1, files[mid].2)),
            }
        }
        None
    }

    /// The path's kind, and whether it's the indexed revision's (rather than
    /// guessed from the path).
    pub fn path_kind(&self, path: &str) -> (Ustr, bool) {
        match self.lookup(path) {
            Some((kind, _)) => (self.kinds[kind as usize], true),
            None => (heuristic_path_kind(&self.path_kinds, path), false),
        }
    }

    /// The path's subsystem (ex: "Firefox/Sidebar"), if the indexed revision has
    /// the file and it has one.
    pub fn subsystem(&self, path: &str) -> Option<Ustr> {
        match self.lookup(path)? {
            (_, NO_SUBSYSTEM) => None,
            (_, subsystem) => Some(self.subsystems[subsystem as usize]),
        }
    }

    /// The path kinds' keys and names, in their display order.
    pub fn kinds(&self) -> Vec<(Ustr, Ustr)> {
        let mut kinds: Vec<_> = self.path_kinds.iter().collect();
        kinds.sort_by_key(|(_, kind)| kind.sort_order);
        kinds
            .into_iter()
            .map(|(key, kind)| (*key, kind.name))
            .collect()
    }
}

/// The trees' `PathFacetMap`s, loaded when first needed (see
/// `path_facet_map`), or None for trees whose files couldn't be loaded.
static PATH_FACET_MAPS: LazyLock<Mutex<HashMap<String, Option<Arc<PathFacetMap>>>>> =
    LazyLock::new(Default::default);

/// The tree's `PathFacetMap`, loading it if this is the first time (which
/// takes a few seconds for firefox, so the web server starts loading the
/// trees' at startup).
pub fn path_facet_map(cfg: &Config, tree_name: &str) -> Option<Arc<PathFacetMap>> {
    let mut maps = PATH_FACET_MAPS.lock().unwrap();
    if let Some(map) = maps.get(tree_name) {
        return map.clone();
    }
    let start = Instant::now();
    let map = cfg
        .trees
        .get(tree_name)
        .ok_or_else(|| "Invalid tree".to_string())
        .and_then(|tree_config| {
            let concise_path = format!(
                "{}/concise-per-file-info.json",
                tree_config.paths.index_path
            );
            let toml = cfg.read_tree_config_file_with_default("per-file-info.toml")?;
            PathFacetMap::load(&concise_path, &toml)
        });
    let map = match map {
        Ok(map) => {
            info!(
                "Loaded {}'s path kinds and subsystems ({} files) in {:?}",
                tree_name,
                map.files,
                start.elapsed()
            );
            Some(Arc::new(map))
        }
        Err(err) => {
            warn!(
                "Couldn't load {}'s path kinds and subsystems: {}",
                tree_name, err
            );
            None
        }
    };
    maps.insert(tree_name.to_string(), map.clone());
    map
}

pub fn get_concise_file_info<'a>(
    all_concise_info: &'a Value,
    path: &str,
) -> Option<&'a Map<String, Value>> {
    let mut cur_obj = all_concise_info.get("root")?.as_object()?;

    for path_component in path.split('/') {
        // The current node must be a directory, get its contents.
        let dir_obj = cur_obj.get("contents")?.as_object()?;
        // And now find the next node inside the components
        cur_obj = dir_obj.get(path_component)?.as_object()?;
    }

    Some(cur_obj)
}

pub fn read_detailed_file_info(path: &str, index_path: &str) -> Option<DetailedPerFileInfo> {
    let json_fname = format!("{}/detailed-per-file-info/{}", index_path, path);
    let json_file = File::open(json_fname).ok()?;
    let mut reader = BufReader::new(&json_file);
    from_reader(&mut reader).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const TOML: &str = r#"
[pathkind.normal]
name = "Core code"
default = true
decision_order = 0
sort_order = 0

[pathkind.test]
name = "Test files"
decision_order = 1
sort_order = 2

[pathkind.test.heuristics]
dir_names = ["tests"]

[pathkind.third_party]
name = "Third-party code"
decision_order = 2
sort_order = 1
"#;

    #[test]
    fn test_path_facet_map() {
        // (Out of order, which the JSON usually isn't.)
        let json = br#"{
            "dom": {"path_kind": "", "is_dir": true},
            "dom/b.cpp": {"path_kind": "normal", "is_dir": false, "subsystem": "Core/DOM: Core & HTML", "description": "skipped"},
            "dom/a.cpp": {"path_kind": "normal", "is_dir": false, "subsystem": null},
            "dom/tests/a.html": {"path_kind": "test", "is_dir": false, "subsystem": "Core/DOM: Core & HTML"},
            "vendored/x.c": {"path_kind": "third_party", "is_dir": false},
            "README": {"path_kind": "normal", "is_dir": false}
        }"#;
        let map = PathFacetMap::parse(json, TOML).unwrap();
        assert_eq!(map.files, 5);
        assert_eq!(map.path_kind("dom/b.cpp"), (ustr::ustr("normal"), true));
        assert_eq!(map.subsystem("dom/b.cpp").unwrap(), "Core/DOM: Core & HTML");
        assert_eq!(map.subsystem("dom/a.cpp"), None);
        assert_eq!(map.path_kind("README"), (ustr::ustr("normal"), true));
        // An explicit kind (ex: from a list of third-party paths) wins.
        assert_eq!(
            map.path_kind("vendored/x.c"),
            (ustr::ustr("third_party"), true)
        );
        // Paths the indexed revision doesn't have get the heuristics' kinds.
        assert_eq!(
            map.path_kind("gone/tests/b.js"),
            (ustr::ustr("test"), false)
        );
        assert_eq!(map.path_kind("dom/c.cpp"), (ustr::ustr("normal"), false));
        assert_eq!(map.subsystem("gone/tests/b.js"), None);
        // The display order is the sort order.
        let kinds: Vec<String> = map
            .kinds()
            .iter()
            .map(|(_, name)| name.to_string())
            .collect();
        assert_eq!(kinds, vec!["Core code", "Third-party code", "Test files"]);
    }
}
