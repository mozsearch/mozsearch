//! The carry-over of a tree's imported try pushes (but spec pull requests',
//! see `spec_prs::start`) from its web server to the next index's, so that a
//! review in progress needn't wait for its push's import again (see the
//! review continuity plan's phase 1).  Imports are on the web server's index
//! volume, which the next index replaces, so:
//!
//! - The web server notes when each push was last asked for (a person's
//!   request, ex: a review dashboard's or the Import button's; see
//!   `TryImports::request`) and looked at (its review page, at most once per
//!   nginx's cache of it), in `HISTORY_ROOT/try-imports/TREE.activity.json`.
//! - Its inventory (`/{tree}/try-imports/inventory`, only for requests from
//!   the indexer; see the web-server) lists its imported pushes, each with
//!   its commits (their summaries, bugs and Phabricator revisions), whether
//!   they all landed, its semantic analysis, when it was imported and how
//!   long that took, and its activity (see `CarriedPush`); and the pushes
//!   carried over to it which it didn't import again, kept a while (see
//!   `kept`), for their pages to say what they were (phase 2's tombstones).
//! - The indexer writes the old web server's inventory to the new index's
//!   `HISTORY_ROOT/try-imports/TREE.carried-over.json` (see
//!   infrastructure/aws/carry-over-imports.py), and the new web server, once
//!   it starts, imports again its live set (see `live_set`: the pushes which
//!   haven't all landed, asked for or looked at lately, the most recent
//!   first), after people's requests (see `start` and `restore`).
//! - Until the live set is imported (or a deadline), the indexer keeps the
//!   old web server serving, posting its inventory again every few minutes
//!   for the new one to import what's new (`/{tree}/try-imports/restore`),
//!   and polling the new one's progress (`/{tree}/try-imports/restore-status`;
//!   see `RestoreStatus` and infrastructure/aws/cutover-gate.py).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::Oid;
use serde::{Deserialize, Serialize};

use crate::file_format::config::Config;
use crate::try_import::{self, IMPORTS, ImportStatus};

/// How recently a push must have been asked for or looked at, in days, to be
/// in the live set, if its tree doesn't say (see
/// `TreeConfigPaths::try_restore_days`).
pub const DEFAULT_RESTORE_DAYS: u64 = 14;
/// How long the inventory keeps a push carried over but not imported again,
/// in days since it was last asked for, looked at or imported: a landed
/// push's, and another's.
pub const LANDED_KEEP_DAYS: u64 = 30;
pub const KEEP_DAYS: u64 = 60;
/// How much later than the one noted a push's request or look is noted again,
/// in seconds (so that its activity's file isn't written for each).
const ACTIVITY_GRAIN: u64 = 60;
const DAY: u64 = 24 * 60 * 60;

/// The time, in seconds since the epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// When a push was last asked for and looked at, in seconds since the epoch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Activity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewed: Option<u64>,
}

impl Activity {
    /// When it was last asked for or looked at.
    pub fn last(&self) -> Option<u64> {
        self.requested.max(self.viewed)
    }

    fn merge(&mut self, other: &Activity) {
        self.requested = self.requested.max(other.requested);
        self.viewed = self.viewed.max(other.viewed);
    }
}

/// A commit of a carried-over push's stack.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CarriedCommit {
    pub rev: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bugs: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phab: Option<u64>,
}

/// An imported push, as the inventory has it (see the module's comment).
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarriedPush {
    /// Its name (an hg revision; see `try_import::normalize_push`).
    pub push: String,
    /// Its head, as imported.
    pub head: String,
    /// When it was imported (its import's log's time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported: Option<u64>,
    /// How long its import took, in seconds (the hook's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_seconds: Option<f64>,
    #[serde(flatten)]
    pub activity: Activity,
    /// Whether all of its stack's commits landed (by its import's rebase,
    /// or the index's commit index's Phabricator revisions).
    pub landed: bool,
    /// Its stack, oldest first.
    pub commits: Vec<CarriedCommit>,
    /// Its semantic analysis: "analyzed", "failed" or "none".
    pub semantic: String,
    /// The indexed revision it was imported over, and its date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_date: Option<String>,
}

impl CarriedPush {
    /// When it was last asked for, looked at, or else imported.
    fn last(&self) -> Option<u64> {
        self.activity.last().or(self.imported)
    }
}

/// The inventory, and the carried-over pushes, as JSON.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CarriedPushes {
    pub pushes: Vec<CarriedPush>,
}

fn try_imports_dir(cfg: &Config, tree_name: &str) -> Option<PathBuf> {
    let history = cfg.trees.get(tree_name)?.paths.history_path.as_ref()?;
    Some(Path::new(history).join("try-imports"))
}

/// Where the tree's pushes' activity is (see `Activity`).
fn activity_path(cfg: &Config, tree_name: &str) -> Option<PathBuf> {
    Some(try_imports_dir(cfg, tree_name)?.join(format!("{}.activity.json", tree_name)))
}

/// Where the indexer carried the previous index's web server's inventory to
/// (see infrastructure/aws/carry-over-imports.py), with what was posted to
/// `restore` since.
fn carried_path(cfg: &Config, tree_name: &str) -> Option<PathBuf> {
    Some(try_imports_dir(cfg, tree_name)?.join(format!("{}.carried-over.json", tree_name)))
}

/// Write `value` as JSON to `path`, at once (through a temporary file).
fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(value)?)?;
    fs::rename(&temporary, path)
}

/// The trees' pushes' activity, loaded from their files when first used.
static ACTIVITY: LazyLock<Mutex<HashMap<String, BTreeMap<String, Activity>>>> =
    LazyLock::new(Default::default);

/// Run `f` on the tree's pushes' activity, writing it if `f` says it changed
/// it.
fn with_activity<R>(
    cfg: &Config,
    tree_name: &str,
    f: impl FnOnce(&mut BTreeMap<String, Activity>) -> (R, bool),
) -> R {
    let path = activity_path(cfg, tree_name);
    let mut all = ACTIVITY.lock().unwrap();
    let pushes = all.entry(tree_name.to_string()).or_insert_with(|| {
        path.as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    });
    let (result, changed) = f(pushes);
    if changed && let Some(path) = path {
        if let Err(err) = write_json(&path, pushes) {
            eprintln!("{}: couldn't write {}: {}", tree_name, path.display(), err);
        }
    }
    result
}

/// Note that a person asked for the try push `push` (see
/// `TryImports::request`); not for spec pull requests, which are carried
/// over if they're open (see `spec_prs::start`).
pub fn note_requested(cfg: &Config, tree_name: &str, push: &str) {
    note(cfg, tree_name, push, |activity| &mut activity.requested);
}

/// Note that the imported try push `push`'s review page was looked at.
pub fn note_viewed(cfg: &Config, tree_name: &str, push: &str) {
    note(cfg, tree_name, push, |activity| &mut activity.viewed);
}

fn note(cfg: &Config, tree_name: &str, push: &str, field: fn(&mut Activity) -> &mut Option<u64>) {
    if try_import::is_pull_request(push) || try_import::hook(cfg, tree_name).is_none() {
        return;
    }
    let now = now();
    with_activity(cfg, tree_name, |pushes| {
        let at = field(pushes.entry(push.to_string()).or_default());
        match *at {
            Some(then) if now < then + ACTIVITY_GRAIN => ((), false),
            _ => {
                *at = Some(now);
                ((), true)
            }
        }
    });
}

/// The push's activity, as noted.
pub fn activity(cfg: &Config, tree_name: &str, push: &str) -> Activity {
    with_activity(cfg, tree_name, |pushes| {
        (pushes.get(push).copied().unwrap_or_default(), false)
    })
}

/// Merge the carried-over pushes' activity into the tree's.
fn merge_activity(cfg: &Config, tree_name: &str, carried: &[CarriedPush]) {
    with_activity(cfg, tree_name, |pushes| {
        let mut changed = false;
        for push in carried {
            let activity = pushes.entry(push.push.clone()).or_default();
            let before = *activity;
            activity.merge(&push.activity);
            changed |= *activity != before;
        }
        ((), changed)
    });
}

/// The pushes carried over to the tree's web server (see `carried_path`).
pub fn carried(cfg: &Config, tree_name: &str) -> Vec<CarriedPush> {
    carried_path(cfg, tree_name)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<CarriedPushes>(&text).ok())
        .map(|carried| carried.pushes)
        .unwrap_or_default()
}

/// Whether all of `commits` landed, by their Phabricator revisions' commits
/// on the tree's branch (as its commit index has them): not if any has
/// none.
fn landed_in_index(cfg: &Config, tree_name: &str, commits: &[CarriedCommit]) -> bool {
    let Some(index) = cfg
        .trees
        .get(tree_name)
        .and_then(|tree| tree.git.as_ref())
        .and_then(|git| git.history.as_ref())
        .and_then(|history| history.commit_index())
    else {
        return false;
    };
    !commits.is_empty()
        && commits.iter().all(|commit| {
            commit
                .phab
                .is_some_and(|phab| !index.phab_commits(&format!("D{}", phab)).is_empty())
        })
}

/// The revision the tree indexed, and its date.
fn indexed_rev(cfg: &Config, tree_name: &str) -> Option<(String, String)> {
    let tree = cfg.trees.get(tree_name)?;
    let git = tree.git.as_ref()?;
    let commit = git
        .repo
        .revparse_single(tree.paths.git_branch.as_deref().unwrap_or("HEAD"))
        .ok()?
        .peel_to_commit()
        .ok()?;
    let reference = crate::commit_index::commit_ref(&commit);
    Some((reference.rev, reference.iso_date))
}

/// How long the hook took, by its import's log's last lines (see
/// `try_import::HookUsage::describe`).
fn import_seconds(log: &str) -> Option<f64> {
    log.lines().rev().find_map(|line| {
        line.strip_prefix("try-import: the hook took ")?
            .split_once(" s")?
            .0
            .parse()
            .ok()
    })
}

/// The imported push `push`, as the inventory has it.
pub fn carried_push(cfg: &Config, tree_name: &str, push: &str) -> Option<CarriedPush> {
    let git = cfg.trees.get(tree_name)?.git.as_ref()?;
    let head = try_import::imported_head(cfg, tree_name, push)?;
    let imported = try_import::try_push(cfg, tree_name, push)?;
    let commits: Vec<CarriedCommit> = imported
        .stack
        .iter()
        .filter_map(|reference| {
            let commit = git
                .repo
                .find_commit(Oid::from_str(&reference.rev).ok()?)
                .ok()?;
            let message = String::from_utf8_lossy(commit.message_bytes());
            Some(CarriedCommit {
                rev: reference.rev.clone(),
                summary: commit
                    .summary()
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string(),
                bugs: crate::commit_index::message_bugs(&message)
                    .iter()
                    .filter_map(|bug| bug.parse().ok())
                    .collect(),
                phab: reference.phabricator_revision,
            })
        })
        .collect();
    let landed = (!commits.is_empty()
        && commits.iter().all(|commit| {
            imported
                .landed
                .iter()
                .any(|landed| landed.rev == commit.rev)
        }))
        || landed_in_index(cfg, tree_name, &commits);
    let log_path = try_imports_dir(cfg, tree_name)?.join(format!("{}.log", push));
    let imported_at = fs::metadata(&log_path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs());
    let import_seconds = fs::read_to_string(&log_path)
        .ok()
        .and_then(|log| import_seconds(&log));
    let semantic = match (&imported.semantic_head, &imported.semantic_failure) {
        (Some(_), _) => "analyzed",
        (None, Some(_)) => "failed",
        (None, None) => "none",
    };
    let (base, base_date) = indexed_rev(cfg, tree_name).unzip();
    Some(CarriedPush {
        push: push.to_string(),
        head: head.to_string(),
        imported: imported_at,
        import_seconds,
        activity: activity(cfg, tree_name, push),
        landed,
        commits,
        semantic: semantic.to_string(),
        base,
        base_date,
    })
}

/// Whether the inventory keeps a push carried over but not imported again
/// (see `LANDED_KEEP_DAYS` and `KEEP_DAYS`).
fn kept(push: &CarriedPush, now: u64) -> bool {
    let days = match push.landed {
        true => LANDED_KEEP_DAYS,
        false => KEEP_DAYS,
    };
    push.last()
        .is_some_and(|last| now.saturating_sub(last) <= days * DAY)
}

/// The tree's inventory (see the module's comment): its imported pushes, and
/// those carried over to it which it keeps, the most recently asked for or
/// looked at first.
pub fn inventory(cfg: &Config, tree_name: &str) -> Vec<CarriedPush> {
    let mut pushes: BTreeMap<String, CarriedPush> = try_import::imported_try_pushes(cfg, tree_name)
        .into_iter()
        .filter_map(|push| Some((push.clone(), carried_push(cfg, tree_name, &push)?)))
        .collect();
    let now = now();
    for mut push in carried(cfg, tree_name) {
        if !pushes.contains_key(&push.push) && kept(&push, now) {
            push.activity.merge(&activity(cfg, tree_name, &push.push));
            pushes.insert(push.push.clone(), push);
        }
    }
    let mut pushes: Vec<CarriedPush> = pushes.into_values().collect();
    pushes.sort_by(|a, b| b.last().cmp(&a.last()).then_with(|| a.push.cmp(&b.push)));
    pushes
}

/// The tree's `try_restore_days` (see `DEFAULT_RESTORE_DAYS`).
fn restore_days(cfg: &Config, tree_name: &str) -> u64 {
    cfg.trees
        .get(tree_name)
        .and_then(|tree| tree.paths.try_restore_days)
        .unwrap_or(DEFAULT_RESTORE_DAYS)
}

/// The live set of `pushes` (see the module's comment): those which haven't
/// all landed, and were asked for or looked at (or, if neither was noted,
/// imported) in the last `days` days, the most recent first.
pub fn live_set(pushes: &[CarriedPush], now: u64, days: u64) -> Vec<String> {
    let mut live: Vec<(u64, &str)> = pushes
        .iter()
        .filter(|push| !push.landed)
        .filter_map(|push| {
            let last = push.last()?;
            (now.saturating_sub(last) <= days * DAY).then_some((last, push.push.as_str()))
        })
        .collect();
    live.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    let mut seen = std::collections::HashSet::new();
    live.into_iter()
        .filter(|(_, push)| seen.insert(*push))
        .map(|(_, push)| push.to_string())
        .collect()
}

/// Merge `newer` into `pushes`, by their names: a push's metadata is the
/// newer's, and its activity both's.
fn merge_pushes(pushes: &mut Vec<CarriedPush>, newer: Vec<CarriedPush>) {
    for push in newer {
        match pushes.iter_mut().find(|old| old.push == push.push) {
            Some(old) => {
                let mut activity = old.activity;
                activity.merge(&push.activity);
                *old = push;
                old.activity = activity;
            }
            None => pushes.push(push),
        }
    }
}

/// The trees' live sets being imported again, in order (see `restore`).
static RESTORING: LazyLock<Mutex<HashMap<String, Vec<String>>>> = LazyLock::new(Default::default);

/// Import again the live set (see `live_set`) of the pushes carried over to
/// the tree's web server and of `newer` (the previous index's web server's
/// inventory, posted since; see the module's comment), after people's
/// requests, the most recent first (see `TryImports::restore`): those which
/// haven't all landed (by the index's commit index too), and aren't imported,
/// waiting or running.  `newer` is kept with the carried-over pushes (see
/// `carried_path`), and their activity with the tree's.  Its status.
pub fn restore(cfg: &Config, tree_name: &str, newer: Vec<CarriedPush>) -> RestoreStatus {
    if try_import::hook(cfg, tree_name).is_none() {
        return RestoreStatus::default();
    }
    let newer: Vec<CarriedPush> = newer
        .into_iter()
        .filter(|push| {
            !try_import::is_pull_request(&push.push)
                && try_import::normalize_push(&push.push).as_deref() == Some(push.push.as_str())
        })
        .collect();
    let mut pushes = carried(cfg, tree_name);
    if !newer.is_empty() {
        merge_pushes(&mut pushes, newer);
        if let Some(path) = carried_path(cfg, tree_name) {
            let carried = CarriedPushes {
                pushes: pushes.clone(),
            };
            if let Err(err) = write_json(&path, &carried) {
                eprintln!("{}: couldn't write {}: {}", tree_name, path.display(), err);
            }
        }
    }
    merge_activity(cfg, tree_name, &pushes);
    for push in &mut pushes {
        push.activity.merge(&activity(cfg, tree_name, &push.push));
        if !push.landed && landed_in_index(cfg, tree_name, &push.commits) {
            push.landed = true;
        }
    }
    let live = live_set(&pushes, now(), restore_days(cfg, tree_name));
    {
        let mut restoring = RESTORING.lock().unwrap();
        let list = restoring.entry(tree_name.to_string()).or_default();
        for push in &live {
            if !list.contains(push) {
                list.push(push.clone());
            }
        }
    }
    let queued = IMPORTS.restore(cfg, tree_name, &live);
    if queued > 0 {
        eprintln!(
            "{}: importing again {} of the previous index's imported try pushes ({} in its live set)",
            tree_name,
            queued,
            live.len()
        );
    }
    status(cfg, tree_name)
}

/// What a web server does for a tree once it starts: import again the live
/// set of the try pushes the indexer carried over to it (see `restore`), if
/// any.  (Again at each start, for those whose imports a restart lost.)
pub fn start(cfg: &Config, tree_name: &str) {
    if try_import::hook(cfg, tree_name).is_none() || carried(cfg, tree_name).is_empty() {
        return;
    }
    restore(cfg, tree_name, vec![]);
}

/// A push's state, of a live set being imported again.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct RestoreState {
    pub push: String,
    pub state: String,
}

/// The progress of the tree's live set's imports (see `restore`): how many
/// pushes are in it, imported (`done`), failed, running and waiting, and
/// those not imported, waiting or running (ex: if a restart lost them, until
/// the next `restore`), and each push's state.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreStatus {
    pub live: usize,
    pub done: usize,
    pub failed: usize,
    pub running: usize,
    pub queued: usize,
    pub not_imported: usize,
    pub pushes: Vec<RestoreState>,
}

/// The tree's live set's progress (see `RestoreStatus`).
pub fn status(cfg: &Config, tree_name: &str) -> RestoreStatus {
    let live = RESTORING
        .lock()
        .unwrap()
        .get(tree_name)
        .cloned()
        .unwrap_or_default();
    let mut status = RestoreStatus {
        live: live.len(),
        ..Default::default()
    };
    for push in live {
        let state = match IMPORTS.status(cfg, tree_name, &push) {
            ImportStatus::Imported { .. } => {
                status.done += 1;
                "imported"
            }
            ImportStatus::Failed { .. } => {
                status.failed += 1;
                "failed"
            }
            ImportStatus::Running { .. } => {
                status.running += 1;
                "running"
            }
            ImportStatus::Queued { .. } => {
                status.queued += 1;
                "queued"
            }
            ImportStatus::NotImported | ImportStatus::Unavailable => {
                status.not_imported += 1;
                "notImported"
            }
        };
        status.pushes.push(RestoreState {
            push,
            state: state.to_string(),
        });
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(name: &str, last: Option<u64>, imported: Option<u64>, landed: bool) -> CarriedPush {
        CarriedPush {
            push: name.to_string(),
            imported,
            activity: Activity {
                requested: last,
                viewed: None,
            },
            landed,
            ..Default::default()
        }
    }

    /// The live set: the pushes which haven't all landed, asked for or looked
    /// at in the last days (or else imported then), the most recent first.
    #[test]
    fn test_live_set() {
        let now = 100 * DAY;
        let pushes = vec![
            push("old", Some(now - 20 * DAY), None, false),
            push("recent", Some(now - DAY), None, false),
            push("landed", Some(now), None, true),
            push("latest", Some(now - 60), None, false),
            push("unnoted", None, Some(now - 2 * DAY), false),
            push("never", None, None, false),
        ];
        assert_eq!(
            live_set(&pushes, now, 14),
            vec!["latest", "recent", "unnoted"]
        );
        assert_eq!(live_set(&pushes, now, 30).len(), 4, "with old");
    }

    /// A newer inventory's push replaces the carried-over one's metadata,
    /// with both's activity.
    #[test]
    fn test_merge_pushes() {
        let mut pushes = vec![push("a", Some(10), Some(5), false)];
        pushes[0].activity.viewed = Some(30);
        let mut newer = push("a", Some(20), Some(15), true);
        newer.semantic = "analyzed".to_string();
        merge_pushes(&mut pushes, vec![newer, push("b", None, Some(1), false)]);
        assert_eq!(pushes.len(), 2);
        assert_eq!(
            pushes[0].activity,
            Activity {
                requested: Some(20),
                viewed: Some(30)
            }
        );
        assert!(pushes[0].landed && pushes[0].semantic == "analyzed");
        assert_eq!(pushes[1].push, "b");
    }

    /// Kept for a while after they were last wanted: landed pushes less long.
    #[test]
    fn test_kept() {
        let now = 100 * DAY;
        assert!(kept(&push("a", Some(now - 40 * DAY), None, false), now));
        assert!(!kept(&push("a", Some(now - 40 * DAY), None, true), now));
        assert!(!kept(&push("a", Some(now - 61 * DAY), None, false), now));
        assert!(!kept(&push("a", None, None, false), now));
    }

    #[test]
    fn test_import_seconds() {
        let log = "Performing semantic::delta step\n\
                   try-import: the hook took 312 s, using 900 CPU seconds (800 user, 100 \
                   system; 2.9 cores on average), and its biggest process peaked at 4.0 GB\n\
                   try-import: precomputed its pages' canvas and dataflow in 40.1 s\n";
        assert_eq!(import_seconds(log), Some(312.0));
        assert_eq!(import_seconds("nothing"), None);
    }

    /// The inventory's JSON: camelCase, its activity flattened.
    #[test]
    fn test_carried_push_json() {
        let mut carried = push("abc", Some(7), Some(5), false);
        carried.import_seconds = Some(1.5);
        carried.commits.push(CarriedCommit {
            rev: "r".to_string(),
            summary: "Bug 1 - x".to_string(),
            bugs: vec![1],
            phab: Some(2),
        });
        let json = serde_json::to_value(&carried).unwrap();
        assert_eq!(json["requested"], 7);
        assert_eq!(json["importSeconds"], 1.5);
        assert_eq!(json["commits"][0]["phab"], 2);
        let back: CarriedPush = serde_json::from_value(json).unwrap();
        assert_eq!(back, carried);
    }
}
