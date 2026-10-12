//! Suggestions for citations of anchors their specs no longer define ("stale"
//! anchors, ex: Gecko's citations of the HTML Standard's
//! `#parsing-main-inselect`, which it removed), from the history of the specs'
//! tree (see `hyperblame`), in two steps:
//!
//! - With the history, when it's updated or regenerated (see the
//!   `stale-anchors` tool), for the anchors which the trees citing the specs
//!   cited (as of their last indexing, see `write_cited`), we `update` a
//!   `Cache` in the history's directory (and uploaded with it), where they
//!   aren't yet:
//!   - The newest revision of the spec's source's directory defining the
//!     anchor, found by analyzing old revisions as spec-analyze analyzes the
//!     tip (with Bikeshed for Bikeshed specs, whose ids are mostly generated):
//!     back from the tip by doubling steps until one defines it, then
//!     bisecting, so about 2 log2(N) analyses of N revisions (ex: ~28 of the
//!     HTML Standard's 12,477), some shared by the spec's anchors, in
//!     parallel.  The next revision removed it.  (An anchor defined only
//!     between two of the doubling steps' revisions isn't found, nor one
//!     defined before a rename of the directory.)
//!   - Where the definition's word tokens in that revision are as of the head
//!     (see `Follower::follow_through`), again whenever their files change.
//!   For Gecko's ~290 stale anchors, ~1,000 old revisions' analyses, about
//!   half of them find a definition.
//! - At indexing, spec-analyze looks the citing tree's stale anchors up in the
//!   cache (see `stale_anchors`): the definitions at the tip where most of
//!   their tokens are now are the suggestions (ex: a renamed `<dfn>`'s, for
//!   its text which stayed), "maybe"s, since a definition's text can go
//!   elsewhere.  It's quick, and the anchors it hasn't seen before get looked
//!   up by the next history update.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use git2::{Oid, Repository};
use serde::{Deserialize, Serialize};

use super::{
    Lines, SpecEntry, SpecFormat, SpecsConfig, bikeshed, ecmarkup, respec, spec_anchor, wattsi,
};
use crate::file_format::analysis::{
    AnalysisKind, Location, read_analysis, read_structured, read_target,
};
use crate::file_format::config::{ThreadLocalRepository, timeline_commit_to_meta};
use crate::file_format::history::syntax_files::TokenClass;
use crate::git_ops::history_compute_threads;
use crate::hyperblame::future::Follower;
use crate::hyperblame::token_blame::TreeHistory;
use crate::semantic_analyzer::Record;

/// The cache's file, in the specs' tree's history's directory.
pub const CACHE_FILE: &str = "stale-anchors.json";

/// The cache's format's version, which changing (ex: when the analyzers
/// change ids) discards the cache.
const CACHE_VERSION: u32 = 1;

/// The most tokens of a definition we follow.
const MAX_TOKENS: usize = 8;

/// The most suggestions for an anchor.
const MAX_SUGGESTIONS: usize = 3;

/// The most characters of a suggestion's pretty name.
const MAX_PRETTY: usize = 80;

/// A definition at the tip which a stale anchor's may be now.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Suggestion {
    pub sym: String,
    pub pretty: String,
    /// Its definition's file and line in the specs' tree, as "PATH#LINE".
    pub jump: String,
}

/// What we know about a stale anchor: the revision which removed it, and what
/// it may be now.
#[derive(Clone, Debug, PartialEq)]
pub struct Stale {
    pub removed: String,
    pub suggestions: Vec<Suggestion>,
}

/// The specs' tree, whose history (and tip's analysis) we look stale anchors
/// up in.
pub struct SpecTree<'a> {
    /// Its name, for links to it.
    pub name: &'a str,
    /// Its git repo (ex: the wubspecs tree's forest).
    pub git: &'a Path,
    /// Its history (see `TreeConfigPaths::history_path`), with the cache.
    pub history: &'a Path,
    /// Its analysis of the tip.
    pub analysis: &'a Path,
}

/// Where a token is as of a revision: its file and (1-based) index.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Place {
    pub path: String,
    pub token: u32,
}

/// The newest revision defining an anchor, and the definition's tokens there.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Definition {
    pub rev: String,
    /// The next revision of the spec's directory, which removed it.
    pub removed: String,
    /// The definition's file (ex: an included file) and word tokens.
    pub file: String,
    pub tokens: Vec<u32>,
}

/// Where a definition's tokens are as of a head (None for those removed, or
/// lost).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Followed {
    pub head: String,
    pub places: Vec<Option<Place>>,
}

/// What we know about a stale anchor.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entry {
    /// The head (a source revision) when we searched the spec's history.
    pub searched: String,
    /// Its definition, if a revision defined it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined: Option<Definition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed: Option<Followed>,
}

/// What we know about stale anchors, by their symbols, as a JSON file.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Cache {
    pub version: u32,
    #[serde(default)]
    pub anchors: BTreeMap<String, Entry>,
}

impl Cache {
    /// The cache of the history in `history`.
    pub fn path(history: &Path) -> PathBuf {
        history.join(CACHE_FILE)
    }

    /// The cache in `path`, or an empty one if there's none (or it's of
    /// another version).
    pub fn load(path: &Path) -> Cache {
        let cache = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Cache>(&text).ok())
            .filter(|cache| cache.version == CACHE_VERSION);
        cache.unwrap_or(Cache {
            version: CACHE_VERSION,
            anchors: BTreeMap::new(),
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap())?;
        std::fs::rename(&tmp, path)
    }
}

/// Write the symbols of the stale anchors a tree cites, as a JSON array, for
/// the next history update (see `read_cited`).
pub fn write_cited(path: &Path, syms: &[String]) -> std::io::Result<()> {
    let mut syms = syms.to_vec();
    syms.sort();
    syms.dedup();
    std::fs::write(path, serde_json::to_string_pretty(&syms).unwrap())
}

/// The symbols in the lists of stale anchors at `paths` (see `write_cited`),
/// sorted, without those which aren't there.
pub fn read_cited(paths: &[PathBuf]) -> Vec<String> {
    let mut syms: Vec<String> = paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .flatten()
        .collect();
    syms.sort();
    syms.dedup();
    syms
}

/// What an update did, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    /// The stale anchors.
    pub anchors: usize,
    /// Those whose specs' histories we searched (the others were cached).
    pub searched: usize,
    /// The old revisions we analyzed.
    pub analyzed: usize,
    /// Of the anchors, those a revision defined.
    pub found: usize,
    /// The tokens we followed.
    pub followed: usize,
}

/// Update `cache` (of the history at `history`, of the repo at `git`) for the
/// stale anchors `wanted` (by their specs, anchors, and symbols), keeping only
/// their entries: search the histories of the specs of those it hasn't seen,
/// and follow their definitions' tokens to the head where their files have
/// changed since it last did.
pub fn update(
    wanted: &[(&SpecEntry, String, String)],
    config: &SpecsConfig,
    git: &Path,
    history_path: &Path,
    cache: &mut Cache,
) -> Result<Stats, String> {
    let mut stats = Stats {
        anchors: wanted.len(),
        ..Stats::default()
    };
    let repo = Repository::open(git).map_err(|e| e.to_string())?;
    let history = TreeHistory::open(&history_path.to_string_lossy(), None)
        .map_err(|e| format!("{}: {}", history_path.display(), e))?;
    let head = history_head(&history)?;
    let exists = |rev: &str| Oid::from_str(rev).is_ok_and(|oid| repo.find_commit(oid).is_ok());

    // The cached entries (if their revisions are still there, ex: not after
    // regenerating the forest), and the anchors to search for.
    let mut entries: HashMap<String, Entry> = HashMap::new();
    let mut to_search: Vec<(&SpecEntry, &str, &str)> = vec![];
    for (spec, anchor, sym) in wanted {
        let cached = cache.anchors.remove(sym).filter(|entry| {
            exists(&entry.searched)
                && entry
                    .defined
                    .as_ref()
                    .is_none_or(|d| exists(&d.rev) && exists(&d.removed))
        });
        match cached {
            Some(entry) => {
                entries.insert(sym.clone(), entry);
            }
            None => to_search.push((spec, anchor, sym)),
        }
    }
    stats.searched = to_search.len();

    // Search the specs' histories for the others' definitions.
    let analyzed = AtomicUsize::new(0);
    let found = search_histories(&to_search, config, git, head, &analyzed);
    stats.analyzed = analyzed.into_inner();
    let mut ranges = TokenRanges::new(&history, &repo);
    // (By file and revision, since files' tokens are read one at a time.)
    let mut found: Vec<_> = to_search.iter().zip(found).collect();
    found.sort_by(|a, b| {
        let key = |f: &Option<(Oid, Oid, String, Location)>| {
            f.as_ref().map(|(rev, _, file, _)| (file.clone(), *rev))
        };
        key(&a.1).cmp(&key(&b.1))
    });
    for ((_, _, sym), found) in found {
        let defined = found.and_then(|(rev, removed, file, loc)| {
            let tokens = ranges.word_tokens(rev, &file, &loc)?;
            Some(Definition {
                rev: rev.to_string(),
                removed: removed.to_string(),
                file,
                tokens,
            })
        });
        entries.insert(
            sym.to_string(),
            Entry {
                searched: head.to_string(),
                defined,
                followed: None,
            },
        );
    }

    // Follow the definitions' tokens to the head, unless they were as of a
    // head since which their files haven't changed.
    let head_tree = repo
        .find_commit(head)
        .and_then(|c| c.tree())
        .map_err(|e| e.to_string())?;
    let changed_since = |rev: &str, paths: &[&str]| -> bool {
        let Some(tree) = Oid::from_str(rev)
            .ok()
            .and_then(|oid| repo.find_commit(oid).ok())
            .and_then(|c| c.tree().ok())
        else {
            return true;
        };
        let oid =
            |tree: &git2::Tree, path: &str| tree.get_path(Path::new(path)).ok().map(|e| e.id());
        paths
            .iter()
            .any(|path| oid(&tree, path) != oid(&head_tree, path))
    };
    let mut to_follow: Vec<(String, Definition)> = vec![];
    for (sym, entry) in entries.iter_mut() {
        let Some(definition) = &entry.defined else {
            continue;
        };
        if let Some(followed) = &mut entry.followed {
            let paths: Vec<&str> = followed
                .places
                .iter()
                .flatten()
                .map(|p| p.path.as_str())
                .collect();
            if followed.head == head.to_string() || !changed_since(&followed.head, &paths) {
                followed.head = head.to_string();
                continue;
            }
        }
        to_follow.push((sym.clone(), definition.clone()));
    }
    // (By file and revision, for the follower's caches.)
    to_follow.sort_by(|a, b| (&a.1.file, &a.1.rev).cmp(&(&b.1.file, &b.1.rev)));
    let follower = Follower::new(&history)?;
    for (sym, definition) in to_follow {
        let (rev, removed) = match (
            Oid::from_str(&definition.rev),
            Oid::from_str(&definition.removed),
        ) {
            (Ok(rev), Ok(removed)) => (rev, removed),
            _ => continue,
        };
        let places = definition
            .tokens
            .iter()
            .map(|&token| {
                stats.followed += 1;
                let future = follower
                    .follow_through(rev, &definition.file, token, removed)
                    .ok()?;
                match (future.outcome, future.path, future.token) {
                    ("now", Some(path), Some(token)) => Some(Place { path, token }),
                    _ => None,
                }
            })
            .collect();
        if let Some(entry) = entries.get_mut(&sym) {
            entry.followed = Some(Followed {
                head: head.to_string(),
                places,
            });
        }
    }

    for (sym, entry) in entries {
        stats.found += usize::from(entry.defined.is_some());
        cache.anchors.insert(sym, entry);
    }
    Ok(stats)
}

/// The source revision of a history's head.
fn history_head(history: &TreeHistory) -> Result<Oid, String> {
    let head = history
        .head_timeline_commit()
        .ok_or("the specs' history is empty")?;
    Ok(timeline_commit_to_meta(&head).source_rev)
}

/// The stale anchors among `wanted` (by their symbols) which a revision of
/// their specs defined, as `tree`'s history's cache has them (see `update`),
/// with what they may be now: the definitions at the tip (which `defined`, by
/// their symbols) where most of their definitions' tokens are, if the cache is
/// as of the history's head.  Also returns how many have suggestions.
pub fn stale_anchors(
    wanted: &[String],
    tree: &SpecTree,
    defined: &dyn Fn(&str) -> bool,
) -> Result<(HashMap<String, Stale>, usize), String> {
    let cache = Cache::load(&Cache::path(tree.history));
    let repo = Repository::open(tree.git).map_err(|e| e.to_string())?;
    let history = TreeHistory::open(&tree.history.to_string_lossy(), None)
        .map_err(|e| format!("{}: {}", tree.history.display(), e))?;
    let head = history_head(&history)?;
    let entries: Vec<(&String, &Entry)> = wanted
        .iter()
        .filter_map(|sym| Some((sym, cache.anchors.get(sym)?)))
        .filter(|(_, entry)| entry.defined.is_some())
        .collect();
    // The definitions at the tip where the tokens are (if the cache is as of
    // the history's head), by file, since files' tokens are read one at a
    // time.
    let mut places: Vec<&Place> = entries
        .iter()
        .filter_map(|(_, entry)| entry.followed.as_ref())
        .filter(|followed| followed.head == head.to_string())
        .flat_map(|followed| followed.places.iter().flatten())
        .collect();
    places.sort_by(|a, b| (&a.path, a.token).cmp(&(&b.path, b.token)));
    places.dedup();
    let mut ranges = TokenRanges::new(&history, &repo);
    let mut tip = Tip::new(tree.analysis);
    let at: HashMap<&Place, Suggestion> = places
        .into_iter()
        .filter_map(|place| Some((place, tip.definition(&mut ranges, head, place)?)))
        .collect();
    let (mut stale, mut suggested) = (HashMap::new(), 0);
    for (sym, entry) in entries {
        let Some(definition) = &entry.defined else {
            continue;
        };
        let mut votes: Vec<(Suggestion, usize)> = vec![];
        let places = entry
            .followed
            .iter()
            .filter(|followed| followed.head == head.to_string())
            .flat_map(|followed| followed.places.iter().flatten());
        for place in places {
            let Some(suggestion) = at.get(place).cloned() else {
                continue;
            };
            if !defined(&suggestion.sym) {
                continue;
            }
            match votes.iter_mut().find(|(s, _)| s.sym == suggestion.sym) {
                Some((_, count)) => *count += 1,
                None => votes.push((suggestion, 1)),
            }
        }
        let most = votes.iter().map(|(_, count)| *count).max().unwrap_or(0);
        let suggestions: Vec<Suggestion> = votes
            .into_iter()
            .filter(|(_, count)| *count == most)
            .map(|(suggestion, _)| suggestion)
            .take(MAX_SUGGESTIONS)
            .collect();
        suggested += usize::from(!suggestions.is_empty());
        stale.insert(
            sym.clone(),
            Stale {
                removed: definition.removed.clone(),
                suggestions,
            },
        );
    }
    Ok((stale, suggested))
}

/// Find, in `revs` (newest first, where the first doesn't define the
/// anchor), the newest index whose revision defines it, by `defined_at`
/// (which gives the definition), back from the first by doubling steps, then
/// bisecting.
fn search<T>(len: usize, defined_at: impl Fn(usize) -> Option<T>) -> Option<(usize, T)> {
    if len < 2 {
        return None;
    }
    // (`lo` doesn't define it, `hi` does.)
    let mut lo = 0;
    let mut step = 1;
    let (mut hi, mut found) = loop {
        let at = (lo + step).min(len - 1);
        if let Some(def) = defined_at(at) {
            break (at, def);
        }
        if at == len - 1 {
            return None;
        }
        lo = at;
        step *= 2;
    };
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        match defined_at(mid) {
            Some(def) => {
                hi = mid;
                found = def;
            }
            None => lo = mid,
        }
    }
    Some((hi, found))
}

/// The definitions of a spec's anchors in a revision, of those we're looking
/// for: their files and locations.
type Probe = HashMap<String, (String, Location)>;

/// A spec's history, and the definitions of the anchors we're looking for in
/// the revisions we've analyzed.
struct SpecHistory<'a> {
    spec: &'a SpecEntry,
    revs: Vec<Oid>,
    anchors: HashSet<String>,
    probes: Mutex<HashMap<usize, Arc<OnceLock<Option<Probe>>>>>,
}

/// For each of `search`'s anchors (by spec), the newest revision defining it,
/// the next revision (removing it), and the definition's file and location,
/// if a revision defined it, searching specs' histories in parallel.
fn search_histories(
    anchors: &[(&SpecEntry, &str, &str)],
    config: &SpecsConfig,
    git: &Path,
    head: Oid,
    analyzed: &AtomicUsize,
) -> Vec<Option<(Oid, Oid, String, Location)>> {
    let mut histories: Vec<SpecHistory> = vec![];
    for (spec, anchor, _) in anchors {
        match histories.iter_mut().find(|h| h.spec.name == spec.name) {
            Some(history) => {
                history.anchors.insert(anchor.to_string());
            }
            None => histories.push(SpecHistory {
                spec,
                revs: spec_revs(git, head, spec),
                anchors: HashSet::from([anchor.to_string()]),
                probes: Mutex::default(),
            }),
        }
    }
    let Ok(repo) = Repository::open(git) else {
        return vec![None; anchors.len()];
    };
    let repo: ThreadLocalRepository = repo.into();
    // (The specs with the most anchors first, since they take longest.)
    let mut order: Vec<usize> = (0..anchors.len()).collect();
    let count = |spec: &SpecEntry| {
        histories
            .iter()
            .find(|h| h.spec.name == spec.name)
            .map_or(0, |h| h.anchors.len())
    };
    order.sort_by_key(|&i| std::cmp::Reverse(count(anchors[i].0)));
    let results: Vec<Mutex<Option<(Oid, Oid, String, Location)>>> =
        anchors.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let threads = history_compute_threads("STALE_ANCHOR_THREADS");
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&index) = order.get(i) else {
                        break;
                    };
                    let (spec, anchor, _) = anchors[index];
                    let Some(history) = histories.iter().find(|h| h.spec.name == spec.name) else {
                        continue;
                    };
                    let defined_at = |at: usize| -> Option<(String, Location)> {
                        let cell = history
                            .probes
                            .lock()
                            .unwrap()
                            .entry(at)
                            .or_default()
                            .clone();
                        let probe = cell.get_or_init(|| {
                            analyzed.fetch_add(1, Ordering::Relaxed);
                            probe(
                                history.spec,
                                config,
                                &repo,
                                history.revs[at],
                                &history.anchors,
                            )
                        });
                        probe.as_ref()?.get(anchor).cloned()
                    };
                    if let Some((at, (file, loc))) = search(history.revs.len(), defined_at) {
                        *results[index].lock().unwrap() =
                            Some((history.revs[at], history.revs[at - 1], file, loc));
                    }
                }
            });
        }
    });
    results
        .into_iter()
        .map(|r| r.into_inner().unwrap())
        .collect()
}

/// The revisions of a spec's source's directory, newest first, up to `head`.
fn spec_revs(git: &Path, head: Oid, spec: &SpecEntry) -> Vec<Oid> {
    let Some(source) = spec.source_path() else {
        return vec![];
    };
    let dir = source
        .rsplit_once('/')
        .map_or(source.as_str(), |(dir, _)| dir);
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(git)
        .args(["rev-list", &head.to_string(), "--", dir])
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| Oid::from_str(line.trim()).ok())
            .collect(),
        _ => vec![],
    }
}

/// The definitions of `anchors` in revision `rev` of `spec`, by analyzing its
/// source as spec-analyze does, or None if we can't (ex: a Bikeshed spec with
/// a configured index, or Bikeshed failing).
fn probe(
    spec: &SpecEntry,
    config: &SpecsConfig,
    repo: &Repository,
    rev: Oid,
    anchors: &HashSet<String>,
) -> Option<Probe> {
    let (format, source_path) = (spec.format?, spec.source_path()?);
    let tree = repo.find_commit(rev).ok()?.tree().ok()?;
    let read = |file: &str| -> Option<String> {
        let entry = tree.get_path(Path::new(file)).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;
        Some(String::from_utf8_lossy(blob.content()).into_owned())
    };
    let shortname = spec.shortname();
    let files: Vec<(String, Vec<Record>)> = match format {
        SpecFormat::Wattsi => {
            let Some(source) = read(&source_path) else {
                return Some(Probe::new());
            };
            vec![(source_path, wattsi::analyze(&source, shortname, config).0)]
        }
        SpecFormat::Ecmarkup => {
            ecmarkup::analyze_with_imports(&source_path, &read, shortname, config, &[])
                .map_or(vec![], |(files, _, _)| files)
        }
        SpecFormat::Respec => {
            let Some(source) = read(&source_path) else {
                return Some(Probe::new());
            };
            let dir = Path::new(&source_path).parent().unwrap_or(Path::new(""));
            let scripts: Vec<String> = respec::local_scripts(&source)
                .iter()
                .filter_map(|script| read(&dir.join(script).to_string_lossy()))
                .filter(|text| text.contains("respecConfig"))
                .collect();
            let scripts = scripts.join("\n");
            if !respec::is_respec(&source, &scripts) {
                return Some(Probe::new());
            }
            respec::analyze_with_includes(
                &source_path,
                &read,
                &scripts,
                shortname,
                config,
                &super::xref::Terms::default(),
            )
            .map_or(vec![], |(files, _, _)| files)
        }
        SpecFormat::Bikeshed => {
            // (Bikeshed reads the spec's files, so they're written out.)
            if spec.index.is_some() {
                return None;
            }
            read(&source_path)?;
            let dir = Path::new(&source_path).parent().unwrap_or(Path::new(""));
            let out = Exported::new(repo, &tree, dir)?;
            let json = bikeshed::run_bikeshed(&out.root.join(&source_path), None).ok()?;
            let index = bikeshed::parse_index(&json).ok()?;
            bikeshed::analyze_with_includes(&source_path, &read, &index, shortname, config)
                .map_or(vec![], |(files, _)| files)
        }
    };
    let mut probe = Probe::new();
    for (file, records) in files {
        for record in records {
            let Record::Target(target) = record else {
                continue;
            };
            if target.data.kind != AnalysisKind::Def {
                continue;
            }
            let Some(anchor) = spec_anchor(shortname, &target.data.sym) else {
                continue;
            };
            if anchors.contains(&anchor) {
                probe.entry(anchor).or_insert((file.clone(), target.loc));
            }
        }
    }
    Some(probe)
}

/// A directory of a revision written out to a temporary directory, which is
/// removed when it's dropped.
struct Exported {
    root: PathBuf,
}

impl Exported {
    fn new(repo: &Repository, tree: &git2::Tree, dir: &Path) -> Option<Exported> {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "stale-anchors-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let exported = Exported { root };
        let subtree = if dir.as_os_str().is_empty() {
            tree.clone()
        } else {
            let entry = tree.get_path(dir).ok()?;
            repo.find_tree(entry.id()).ok()?
        };
        let mut ok = true;
        subtree
            .walk(git2::TreeWalkMode::PreOrder, |parent, entry| {
                if entry.kind() != Some(git2::ObjectType::Blob) {
                    return git2::TreeWalkResult::Ok;
                }
                let path = exported
                    .root
                    .join(dir)
                    .join(parent)
                    .join(entry.name().unwrap_or_default());
                let written = repo.find_blob(entry.id()).ok().and_then(|blob| {
                    std::fs::create_dir_all(path.parent()?).ok()?;
                    std::fs::write(&path, blob.content()).ok()
                });
                ok &= written.is_some();
                git2::TreeWalkResult::Ok
            })
            .ok()?;
        ok.then_some(exported)
    }
}

impl Drop for Exported {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The tokens of files in revisions (see `TreeHistory::token_ranges`), and
/// their sources' lines.
struct TokenRanges<'a> {
    history: &'a TreeHistory,
    repo: &'a Repository,
    cache: HashMap<(Oid, String), Option<(Vec<(Range<usize>, TokenClass)>, Lines)>>,
}

impl<'a> TokenRanges<'a> {
    fn new(history: &'a TreeHistory, repo: &'a Repository) -> Self {
        TokenRanges {
            history,
            repo,
            cache: HashMap::new(),
        }
    }

    fn get(&mut self, rev: Oid, path: &str) -> Option<&(Vec<(Range<usize>, TokenClass)>, Lines)> {
        let key = (rev, path.to_string());
        if !self.cache.contains_key(&key) {
            let ranges = (|| {
                let timeline_commit = self.history.timeline_commit(rev)?;
                let tree = self.repo.find_commit(rev).ok()?.tree().ok()?;
                let entry = tree.get_path(Path::new(path)).ok()?;
                let blob = self.repo.find_blob(entry.id()).ok()?;
                let source = std::str::from_utf8(blob.content()).ok()?;
                let ranges = self
                    .history
                    .token_ranges(&timeline_commit, path, source)
                    .ok()??;
                Some((ranges, Lines::new(source)))
            })();
            // (Only the last file's, since they can be large.)
            self.cache.clear();
            self.cache.insert(key.clone(), ranges);
        }
        self.cache.get(&key)?.as_ref()
    }

    /// The (1-based) indices of the word tokens (not keywords or operators,
    /// ex: a `<dfn>`'s text) of the definition at `loc` in `path` in `rev`.
    fn word_tokens(&mut self, rev: Oid, path: &str, loc: &Location) -> Option<Vec<u32>> {
        let (ranges, lines) = self.get(rev, path)?;
        let range = lines.range(loc)?;
        let tokens: Vec<u32> = ranges
            .iter()
            .enumerate()
            .filter(|(_, (r, class))| {
                r.start < range.end
                    && range.start < r.end
                    && !matches!(class, TokenClass::Keyword | TokenClass::Operator)
            })
            .map(|(i, _)| i as u32 + 1)
            .take(MAX_TOKENS)
            .collect();
        (!tokens.is_empty()).then_some(tokens)
    }

    /// The location of a token in `rev` (on one line).
    fn location(&mut self, rev: Oid, place: &Place) -> Option<Location> {
        let (ranges, lines) = self.get(rev, &place.path)?;
        let (range, _) = ranges.get((place.token as usize).checked_sub(1)?)?;
        Some(lines.location(range))
    }
}

/// The definitions in the tip's analysis, by file: those with structured
/// records (dfns, headings, ...), not ids of other elements (ex: Bikeshed's
/// links' `ref-for-` ids, Wattsi's cross-references'), which aren't what a
/// definition becomes.
struct Tip<'a> {
    analysis: &'a Path,
    defs: HashMap<String, Vec<(Location, String, String)>>,
}

impl<'a> Tip<'a> {
    fn new(analysis: &'a Path) -> Self {
        Tip {
            analysis,
            defs: HashMap::new(),
        }
    }

    /// The definition at the tip (`head`) that the token at `place` is in, if
    /// it's in one.
    fn definition(
        &mut self,
        ranges: &mut TokenRanges,
        head: Oid,
        place: &Place,
    ) -> Option<Suggestion> {
        let at = ranges.location(head, place)?;
        let analysis = self.analysis;
        let defs = self.defs.entry(place.path.clone()).or_insert_with(|| {
            // (Compressed, or not, as `read_analysis` reads them.)
            let file = analysis.join(format!("{}.gz", place.path));
            let file = file.to_string_lossy();
            let structured: HashSet<String> = read_analysis(&file, &mut read_structured)
                .into_iter()
                .flat_map(|s| s.data.into_iter().map(|s| s.sym.to_string()))
                .collect();
            read_analysis(&file, &mut read_target)
                .into_iter()
                .flat_map(|target| {
                    let loc = target.loc;
                    target
                        .data
                        .into_iter()
                        .filter(|t| t.kind == AnalysisKind::Def && t.sym.starts_with("SPEC_"))
                        .map(move |t| (loc, t.sym.to_string(), t.pretty.to_string()))
                })
                .filter(|(_, sym, _)| structured.contains(sym))
                .collect()
        });
        defs.iter()
            .find(|(loc, _, _)| {
                loc.lineno == at.lineno
                    && loc.col_start <= at.col_start
                    && at.col_start < loc.col_end.max(loc.col_start + 1)
            })
            .map(|(loc, sym, pretty)| Suggestion {
                sym: sym.clone(),
                pretty: shorten(pretty),
                jump: format!("{}#{}", place.path, loc.lineno),
            })
    }
}

/// A definition's pretty name, up to `MAX_PRETTY` characters (some are a
/// whole definition list's text, ex: css-color-4's system colors').
fn shorten(pretty: &str) -> String {
    match pretty.char_indices().nth(MAX_PRETTY) {
        Some((end, _)) => format!("{}…", pretty[..end].trim_end()),
        None => pretty.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search() {
        // (Revisions 0-2 don't define it, 3-9 do.)
        let calls = std::cell::RefCell::new(vec![]);
        let defined_at = |at: usize| {
            calls.borrow_mut().push(at);
            (at >= 3).then_some(at)
        };
        assert_eq!(search(10, defined_at), Some((3, 3)));
        // (Steps of 1 and 2 back, to 1 and 3, then bisecting between them.)
        assert_eq!(*calls.borrow(), vec![1, 3, 2]);

        // Only the oldest does.
        assert_eq!(search(10, |at| (at == 9).then_some(())), Some((9, ())));
        // None do, or there's no older revision.
        assert_eq!(search(10, |_| None::<()>), None);
        assert_eq!(search(1, |_| Some(())), None);
        // The one before the tip.
        assert_eq!(search(2, |at| (at >= 1).then_some(())), Some((1, ())));
    }

    #[test]
    fn test_cache() {
        let dir = std::env::temp_dir().join(format!("stale-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stale-anchors.json");
        assert!(Cache::load(&path).anchors.is_empty());
        let mut cache = Cache::load(&path);
        cache.anchors.insert(
            "SPEC_x_a".to_string(),
            Entry {
                searched: "h".to_string(),
                defined: Some(Definition {
                    rev: "r".to_string(),
                    removed: "s".to_string(),
                    file: "x/source".to_string(),
                    tokens: vec![3, 4],
                }),
                followed: Some(Followed {
                    head: "h".to_string(),
                    places: vec![
                        Some(Place {
                            path: "x/source".to_string(),
                            token: 5,
                        }),
                        None,
                    ],
                }),
            },
        );
        cache.anchors.insert(
            "SPEC_x_never".to_string(),
            Entry {
                searched: "h".to_string(),
                defined: None,
                followed: None,
            },
        );
        cache.save(&path).unwrap();
        let loaded = Cache::load(&path);
        assert_eq!(loaded.anchors, cache.anchors);
        // Another version's cache is discarded.
        std::fs::write(
            &path,
            r#"{"version": 0, "anchors": {"SPEC_x_a": {"searched": "h"}}}"#,
        )
        .unwrap();
        assert!(Cache::load(&path).anchors.is_empty());

        // The cited lists.
        let (a, b) = (dir.join("a.json"), dir.join("b.json"));
        write_cited(&a, &["SPEC_x_b".to_string(), "SPEC_x_a".to_string()]).unwrap();
        write_cited(&b, &["SPEC_x_a".to_string(), "SPEC_x_c".to_string()]).unwrap();
        assert_eq!(
            read_cited(&[a, b, dir.join("missing.json")]),
            vec!["SPEC_x_a", "SPEC_x_b", "SPEC_x_c"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
