//! A tree of specs' summaries of its spec repositories' open pull requests
//! (`/{tree}/spec-prs`, and a spec repository's at `/{tree}/spec-prs/{spec}`;
//! see `format::format_spec_prs`): their metadata (titles, authors, review
//! decisions, drafts, labels), and what each changes relative to the spec's
//! trunk (files, hunks, sections), for knowing which pull requests there are
//! and what they touch, and filtering them (ex: only approved ones, or those
//! authored by people who can review), without importing them (see
//! `try_import`, which their pages link to).  Not a history: only the pull
//! requests' current heads.
//!
//! The summaries are written to `HISTORY_ROOT/spec-prs/{spec}.json` by the
//! tree's `sync-spec-prs` hook (in its config repo directory, like
//! `import-try`; it runs mozsearch's scripts/spec-prs/sync.py, see its
//! README.md), which the web server runs when asked (a POST to
//! `/{tree}/spec-prs/sync`, for all of the configured repositories, or with
//! `?spec=`, one), one sync at a time, at most once every `MIN_INTERVAL` for
//! the same repositories, with its output in `HISTORY_ROOT/spec-prs/sync.log`.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::ops::{Range, RangeInclusive};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cmd_pipeline::facets::{FacetValueView, FacetView};
use crate::file_format::config::Config;

/// How long a sync can take before it's killed.
pub const TIME_LIMIT: Duration = Duration::from_secs(30 * 60);
/// How long after a sync of the same repositories another can be asked for.
pub const MIN_INTERVAL: Duration = Duration::from_secs(2 * 60);
/// How many syncs can wait (of different repositories, or trees).
const MAX_QUEUED: usize = 8;

/// The syncs of the server, which `run` performs.
pub static SYNCS: LazyLock<SpecPrSyncs> = LazyLock::new(SpecPrSyncs::default);

/// The hook which syncs a tree's pull request summaries, if it can.
pub fn hook(cfg: &Config, tree_name: &str) -> Option<PathBuf> {
    let tree_config = cfg.trees.get(tree_name)?;
    tree_config.paths.history_path.as_ref()?;
    let hook = Path::new(&cfg.config_repo_path)
        .join(tree_name)
        .join("sync-spec-prs");
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&hook)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    executable.then_some(hook)
}

/// Where a tree's summaries are, if it has a history.
fn summaries_dir(cfg: &Config, tree_name: &str) -> Option<PathBuf> {
    let history = cfg.trees.get(tree_name)?.paths.history_path.as_ref()?;
    Some(Path::new(history).join("spec-prs"))
}

/// A spec repository's summary, as sync.py writes it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RepoSummary {
    /// Its directory in the forest.
    pub spec: String,
    /// Its GitHub repository (OWNER/NAME), or its directory.
    pub repo: String,
    pub url: String,
    /// When the summary was written, and the metadata's snapshot taken.
    pub synced: String,
    #[serde(default)]
    pub snapshot: Option<String>,
    pub trunk: String,
    pub prs: Vec<PrSummary>,
}

/// An open pull request's summary.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrSummary {
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// The author's association with the repository, as GitHub says it (ex:
    /// "MEMBER", "CONTRIBUTOR").
    #[serde(default)]
    pub association: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Its review decision ("APPROVED", "CHANGES_REQUESTED",
    /// "REVIEW_REQUIRED"), if known.
    #[serde(default)]
    pub review: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub files: Vec<FileSummary>,
    /// The specs whose directories it changes (for repositories of several).
    #[serde(default)]
    pub specs: Vec<String>,
    /// Why it has no summary of its changes, if it doesn't.
    #[serde(default)]
    pub error: Option<String>,
}

/// What a pull request changes in a file: lines added and removed, its hunks
/// (`[base start, base lines, head start, head lines]`, in the merge base's
/// and the head's versions), and the sections they're in (in the merge
/// base's version).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileSummary {
    pub path: String,
    /// The spec whose source it is, if it's one's (whose rendering its
    /// sections' ids are anchors of).
    #[serde(default)]
    pub spec: Option<String>,
    pub added: u32,
    pub removed: u32,
    #[serde(default)]
    pub hunks: Vec<[u32; 4]>,
    #[serde(default)]
    pub sections: Vec<SectionRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectionRef {
    pub line: u32,
    pub id: Option<String>,
    pub title: String,
}

/// Whether `spec` could be a spec repository's directory in the forest (ex:
/// "dom", "csswg-drafts").
pub fn valid_spec(spec: &str) -> bool {
    !spec.is_empty()
        && !spec.starts_with('.')
        && spec
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A spec repository's summary, if it has one.
pub fn repo_summary(cfg: &Config, tree_name: &str, spec: &str) -> Option<RepoSummary> {
    if !valid_spec(spec) {
        return None;
    }
    let text =
        fs::read_to_string(summaries_dir(cfg, tree_name)?.join(format!("{}.json", spec))).ok()?;
    serde_json::from_str(&text).ok()
}

/// The tree's spec repositories' summaries, by their directories.
pub fn repo_summaries(cfg: &Config, tree_name: &str) -> Vec<RepoSummary> {
    let Some(dir) = summaries_dir(cfg, tree_name) else {
        return vec![];
    };
    let mut summaries: Vec<RepoSummary> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let spec = name.strip_suffix(".json")?;
            repo_summary(cfg, tree_name, spec)
        })
        .collect();
    summaries.sort_by(|a, b| a.spec.cmp(&b.spec));
    summaries
}

/// Whether a pull request's author can review the repository's pull requests,
/// as their association says (its owners, its organization's members, and its
/// collaborators).
pub fn can_review(association: Option<&str>) -> bool {
    matches!(association, Some("OWNER" | "MEMBER" | "COLLABORATOR"))
}

/// How the pages name a review decision.
pub fn review_name(review: Option<&str>) -> &'static str {
    match review {
        Some("APPROVED") => "Approved",
        Some("CHANGES_REQUESTED") => "Changes requested",
        Some("REVIEW_REQUIRED") => "Review required",
        _ => "No review decision",
    }
}

/// A pull request's values of the facets of a repository's pull requests
/// (see `pr_facets`), by facet.
pub fn pr_facet_values(pr: &PrSummary) -> BTreeMap<&'static str, Vec<String>> {
    let mut values = BTreeMap::new();
    values.insert(
        "review",
        vec![pr.review.clone().unwrap_or_else(|| "NONE".to_string())],
    );
    values.insert(
        "author",
        vec![
            if can_review(pr.association.as_deref()) {
                "reviewer"
            } else {
                "contributor"
            }
            .to_string(),
        ],
    );
    values.insert(
        "draft",
        vec![if pr.draft { "draft" } else { "ready" }.to_string()],
    );
    values.insert("label", pr.labels.clone());
    values.insert("spec", pr.specs.clone());
    let sections: BTreeSet<String> = pr
        .files
        .iter()
        .flat_map(|file| file.sections.iter())
        .map(|section| section.title.clone())
        .collect();
    values.insert("section", sections.into_iter().collect());
    values
}

/// The facets of a repository's pull requests: their review decisions,
/// whether their authors can review, whether they're drafts, their labels,
/// the specs they change (for repositories of several), and the sections they
/// change (see `pr_facet_values`).
pub fn pr_facets(prs: &[PrSummary]) -> Vec<FacetView> {
    let mut counts: HashMap<&'static str, BTreeMap<String, u32>> = HashMap::new();
    for pr in prs {
        for (facet, values) in pr_facet_values(pr) {
            for value in values {
                *counts.entry(facet).or_default().entry(value).or_default() += 1;
            }
        }
    }
    let named = |facet: &str, value: &str| -> (String, String) {
        match (facet, value) {
            ("review", _) => (
                review_name(Some(value)).to_string(),
                "Its review decision, as GitHub says it".to_string(),
            ),
            ("author", "reviewer") => (
                "Can review".to_string(),
                "Authored by people who can review the repository's pull requests (its owners, members and collaborators)".to_string(),
            ),
            ("author", _) => (
                "Other contributors".to_string(),
                "Authored by other contributors".to_string(),
            ),
            ("draft", "draft") => ("Drafts".to_string(), "Draft pull requests".to_string()),
            ("draft", _) => (
                "Ready for review".to_string(),
                "Pull requests which aren't drafts".to_string(),
            ),
            _ => (value.to_string(), value.to_string()),
        }
    };
    let order: &[(&'static str, &str)] = &[
        ("review", "Review"),
        ("author", "Author"),
        ("draft", "Draft"),
        ("label", "Label"),
        ("spec", "Spec"),
        ("section", "Section"),
    ];
    let mut facets = vec![];
    for (key, label) in order {
        let Some(values) = counts.remove(key) else {
            continue;
        };
        // (A facet with one value filters nothing.)
        if values.len() < 2 {
            continue;
        }
        let mut views: Vec<FacetValueView> = values
            .into_iter()
            .map(|(value, count)| {
                let (name, title) = named(key, &value);
                FacetValueView {
                    value,
                    name,
                    title,
                    count,
                    ..Default::default()
                }
            })
            .collect();
        views.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        facets.push(FacetView::new(key, label.to_string(), views));
    }
    facets
}

/// A change of a file by a pull request, from the diff of the file's trunk
/// version and its merged version (the trunk with the pull request's changes
/// merged; see `format::spec_pr_blocks_json`): the (0-based) lines it replaces
/// in the trunk's, and those replacing them in the merged version's (either
/// may be empty).
#[derive(Clone, Debug, PartialEq)]
pub struct LineHunk {
    pub trunk: Range<usize>,
    pub merged: Range<usize>,
}

/// The changes of the merged version of a file from its trunk version (see
/// `LineHunk`), as git diffs them.
pub fn line_hunks(trunk: &str, merged: &str) -> Vec<LineHunk> {
    let mut options = git2::DiffOptions::new();
    options.context_lines(0);
    let Ok(patch) = git2::Patch::from_buffers(
        trunk.as_bytes(),
        None,
        merged.as_bytes(),
        None,
        Some(&mut options),
    ) else {
        return vec![];
    };
    // (A hunk's start is 1-based, or for an empty side, the line before.)
    let range = |start: u32, lines: u32| match lines {
        0 => start as usize..start as usize,
        lines => start as usize - 1..(start + lines) as usize - 1,
    };
    (0..patch.num_hunks())
        .filter_map(|i| patch.hunk(i).ok())
        .map(|(hunk, _)| LineHunk {
            trunk: range(hunk.old_start(), hunk.old_lines()),
            merged: range(hunk.new_start(), hunk.new_lines()),
        })
        .collect()
}

/// A pull request's change of a rendered page (see `block_changes`).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockChange {
    /// The trunk's (0-based) line beside whose block the change goes: the
    /// first it replaces, or the one it adds lines after.
    pub anchor: usize,
    /// The lines it replaces in the trunk's version, and those replacing
    /// them in the merged version's (see `LineHunk`).
    pub trunk: Range<usize>,
    pub merged: Range<usize>,
    /// The merged version's page's blocks with the lines replacing them (none
    /// if it only removes lines).
    pub blocks: Vec<usize>,
}

/// The changes of a merged version's rendered page from the trunk's: for each
/// of the file's changes (see `line_hunks`), the page's blocks with its merged
/// lines (`block_lines`, the merged version's blocks' lines; see
/// `rendered_docs::block_lines`), if any (else it changes nothing the page
/// shows, ex: a comment, or another page), or if it only removes lines and
/// they were where the page's lines are (`page_lines`), none; changes with
/// blocks in common are one.  Each goes beside the trunk's block with the
/// first line it replaces, or the line it adds lines after (at most the last
/// of the trunk's `trunk_lines`).
pub fn block_changes(
    hunks: &[LineHunk],
    block_lines: &[Option<RangeInclusive<usize>>],
    page_lines: &Range<usize>,
    trunk_lines: usize,
) -> Vec<BlockChange> {
    let mut changes: Vec<BlockChange> = vec![];
    for hunk in hunks {
        let blocks: Vec<usize> = block_lines
            .iter()
            .enumerate()
            .filter(|(_, lines)| {
                lines.as_ref().is_some_and(|lines| {
                    !hunk.merged.is_empty()
                        && *lines.start() < hunk.merged.end
                        && *lines.end() >= hunk.merged.start
                })
            })
            .map(|(i, _)| i)
            .collect();
        if hunk.merged.is_empty() {
            if !page_lines.contains(&hunk.merged.start) {
                continue;
            }
        } else if blocks.is_empty() {
            continue;
        }
        match changes.last_mut() {
            Some(last) if blocks.iter().any(|block| last.blocks.contains(block)) => {
                last.trunk.end = last.trunk.end.max(hunk.trunk.end);
                last.merged.end = last.merged.end.max(hunk.merged.end);
                for block in blocks {
                    if !last.blocks.contains(&block) {
                        last.blocks.push(block);
                    }
                }
            }
            _ => changes.push(BlockChange {
                anchor: anchor_line(&hunk.trunk, trunk_lines),
                trunk: hunk.trunk.clone(),
                merged: hunk.merged.clone(),
                blocks,
            }),
        }
    }
    changes
}

/// The trunk's (0-based) line beside whose block a change replacing its lines
/// `trunk` goes: the first it replaces, or if it only adds lines, the one it
/// adds them after (at most the last of the trunk's `trunk_lines`).
fn anchor_line(trunk: &Range<usize>, trunk_lines: usize) -> usize {
    match trunk.is_empty() {
        true => trunk.start.saturating_sub(1),
        false => trunk.start,
    }
    .min(trunk_lines.saturating_sub(1))
}

/// The most lines of each side of a `SourceChange`.
pub const SOURCE_CHANGE_LINES: usize = 40;

/// A pull request's change of a spec's source, for its rendered page's view
/// when it has no rendering of the pull request's version (see
/// `source_changes`).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceChange {
    /// The trunk's line it goes beside, and the trunk's lines it replaces
    /// (see `BlockChange`).
    pub anchor: usize,
    pub trunk: Range<usize>,
    /// The lines it removes and adds (at most `SOURCE_CHANGE_LINES` of each),
    /// and how many more there are.
    pub removed: Vec<String>,
    pub added: Vec<String>,
    pub more_removed: usize,
    pub more_added: usize,
}

/// The changes of a file's source from `old` to `new` (see `line_hunks`), for
/// its trunk version's rendered page, whose lines (`trunk_lines` of them)
/// `old`'s lines are at as `trunk_of` maps them: either the trunk's own (ex:
/// the trunk with a pull request's changes merged, a Bikeshed spec's before
/// its import renders them), or its pull request's base's (when its changes
/// conflict with the trunk's; see `map_lines`).
pub fn source_changes(
    old: &str,
    new: &str,
    trunk_of: impl Fn(&Range<usize>) -> Range<usize>,
    trunk_lines: usize,
) -> Vec<SourceChange> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let some = |lines: &[&str], range: &Range<usize>| -> (Vec<String>, usize) {
        let lines = lines.get(range.clone()).unwrap_or_default();
        let shown = lines.len().min(SOURCE_CHANGE_LINES);
        (
            lines[..shown].iter().map(|line| line.to_string()).collect(),
            lines.len() - shown,
        )
    };
    line_hunks(old, new)
        .into_iter()
        .map(|hunk| {
            let trunk = trunk_of(&hunk.trunk);
            let (removed, more_removed) = some(&old_lines, &hunk.trunk);
            let (added, more_added) = some(&new_lines, &hunk.merged);
            SourceChange {
                anchor: anchor_line(&trunk, trunk_lines),
                trunk,
                removed,
                added,
                more_removed,
                more_added,
            }
        })
        .collect()
}

/// Where a file's (0-based) lines `lines` are in its later version, from the
/// changes between them (`hunks`, as `line_hunks` gives them, the earlier
/// version's lines as `trunk` and the later's as `merged`): the lines they
/// are, with those changes made to them, or for none, where they were, before
/// the lines it added there.
pub fn map_lines(hunks: &[LineHunk], lines: &Range<usize>) -> Range<usize> {
    // (A position between lines, as the start of lines or their end: lines
    // added there are before the start and after the end, and one in lines
    // it changed is the start or end of their change.)
    let map = |position: usize, end: bool| {
        let mut offset = 0isize;
        for hunk in hunks {
            if position < hunk.trunk.start
                || (position == hunk.trunk.start && (end || !hunk.trunk.is_empty()))
            {
                break;
            }
            if position < hunk.trunk.end {
                return if end {
                    hunk.merged.end
                } else {
                    hunk.merged.start
                };
            }
            offset = hunk.merged.end as isize - hunk.trunk.end as isize;
        }
        position.saturating_add_signed(offset)
    };
    let start = map(lines.start, lines.is_empty());
    start..map(lines.end, true).max(start)
}

/// What's become of a tree's syncs, for its pages and their polling.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SyncStatus {
    /// The tree can't sync its pull requests' summaries.
    Unavailable,
    /// Not syncing, and the last sync's outcome, if any: how many seconds
    /// ago it finished, and its error, if it failed.
    Idle {
        finished: Option<u64>,
        error: Option<String>,
    },
    Queued {
        ahead: usize,
    },
    Running {
        seconds: u64,
        step: String,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Job {
    tree: String,
    spec: Option<String>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    running: Option<(Job, Instant, String)>,
    /// The syncs' last outcomes, by tree and spec.
    finished: HashMap<Job, (Option<String>, Instant)>,
}

#[derive(Default)]
pub struct SpecPrSyncs {
    state: Mutex<State>,
    queued: Condvar,
}

impl SpecPrSyncs {
    /// The status of the tree's syncs (or of one spec repository's).
    pub fn status(&self, cfg: &Config, tree_name: &str, spec: Option<&str>) -> SyncStatus {
        if hook(cfg, tree_name).is_none() {
            return SyncStatus::Unavailable;
        }
        let job = Job {
            tree: tree_name.to_string(),
            spec: spec.map(str::to_string),
        };
        let state = self.state.lock().unwrap();
        if let Some((running, started, step)) = &state.running
            && running.tree == job.tree
        {
            return SyncStatus::Running {
                seconds: started.elapsed().as_secs(),
                step: step.clone(),
            };
        }
        if let Some(ahead) = state
            .queue
            .iter()
            .position(|queued| queued.tree == job.tree)
        {
            return SyncStatus::Queued {
                ahead: ahead + state.running.is_some() as usize,
            };
        }
        let last = state.finished.get(&job).or_else(|| {
            state.finished.get(&Job {
                spec: None,
                ..job.clone()
            })
        });
        SyncStatus::Idle {
            finished: last.map(|(_, at)| at.elapsed().as_secs()),
            error: last.and_then(|(error, _)| error.clone()),
        }
    }

    /// Ask for a sync of the tree's pull requests' summaries (or of one spec
    /// repository's), unless the same is waiting or running, or finished
    /// less than `MIN_INTERVAL` ago; and its status.
    pub fn request(
        &self,
        cfg: &Config,
        tree_name: &str,
        spec: Option<&str>,
    ) -> Result<SyncStatus, &'static str> {
        if hook(cfg, tree_name).is_none() {
            return Err("This tree can't sync its pull requests");
        }
        let job = Job {
            tree: tree_name.to_string(),
            spec: spec.map(str::to_string),
        };
        {
            let mut state = self.state.lock().unwrap();
            let busy = state.running.as_ref().is_some_and(|(j, _, _)| *j == job)
                || state.queue.contains(&job);
            let recent = state
                .finished
                .get(&job)
                .is_some_and(|(_, at)| at.elapsed() < MIN_INTERVAL);
            if !busy && !recent {
                if state.queue.len() >= MAX_QUEUED {
                    return Err("Too many syncs are waiting; try again later");
                }
                state.queue.push_back(job);
                self.queued.notify_one();
            }
        }
        Ok(self.status(cfg, tree_name, spec))
    }

    /// Perform the syncs as they're asked for, forever.
    pub fn run(&self, cfg: &Config) {
        loop {
            let job = {
                let mut state = self.state.lock().unwrap();
                let job = loop {
                    if let Some(job) = state.queue.pop_front() {
                        break job;
                    }
                    state = self.queued.wait(state).unwrap();
                };
                state.running = Some((job.clone(), Instant::now(), String::new()));
                job
            };
            let result = self.sync(cfg, &job);
            let mut state = self.state.lock().unwrap();
            state.running = None;
            if let Err(message) = &result {
                eprintln!("Syncing {}'s pull requests failed: {}", job.tree, message);
            }
            state.finished.insert(job, (result.err(), Instant::now()));
        }
    }

    fn sync(&self, cfg: &Config, job: &Job) -> Result<(), String> {
        let tree_config = cfg.trees.get(&job.tree).ok_or("No such tree")?;
        let hook = hook(cfg, &job.tree).ok_or("The tree can't sync its pull requests")?;
        let dir = summaries_dir(cfg, &job.tree).ok_or("The tree has no history")?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let envs: Vec<(&str, &str)> = vec![
            ("MOZSEARCH_PATH", &cfg.mozsearch_path),
            ("CONFIG_REPO", &cfg.config_repo_path),
            ("TREE_NAME", &job.tree),
            (
                "GIT_ROOT",
                tree_config.paths.git_path.as_deref().unwrap_or_default(),
            ),
            (
                "HISTORY_ROOT",
                tree_config
                    .paths
                    .history_path
                    .as_deref()
                    .unwrap_or_default(),
            ),
            ("INDEX_ROOT", &tree_config.paths.index_path),
        ];
        let args: Vec<&str> = job.spec.as_deref().into_iter().collect();
        crate::try_import::run_hook(
            &hook,
            &args,
            &envs,
            &dir.join("sync.log"),
            TIME_LIMIT,
            &mut |step| {
                if let Some((_, _, running)) = &mut self.state.lock().unwrap().running {
                    *running = step;
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abstract_server::{
        AbstractServer, CommitInfo, HtmlFileRoot, Result, SearchfoxIndexRoot, ServerError, TreeInfo,
    };
    use crate::file_format::crossref::CrossrefData;
    use crate::file_format::jumpref::JumprefData;
    use async_trait::async_trait;
    use serde_json::Value;

    /// A tree of specs with `gizmo`'s pull requests' summaries (see `pr`).
    #[derive(Clone)]
    struct Fake(SyncStatus);

    #[async_trait]
    impl AbstractServer for Fake {
        fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync> {
            Box::new(self.clone())
        }
        fn tree_info(&self) -> Result<TreeInfo> {
            Ok(TreeInfo {
                name: "specs".to_string(),
            })
        }
        fn commit_info(&self) -> Result<Option<CommitInfo>> {
            Ok(None)
        }
        fn translate_path(&self, _root: SearchfoxIndexRoot, _path: &str) -> Result<String> {
            Err(ServerError::Unsupported)
        }
        async fn coverage_history(
            &self,
            _path: &str,
        ) -> Result<Option<Vec<crate::git_ops::RevisionCoverage>>> {
            Err(ServerError::Unsupported)
        }
        async fn fetch_raw_source(&self, _path: &str) -> Result<String> {
            Err(ServerError::Unsupported)
        }
        async fn fetch_formatted_lines(&self, _path: &str) -> Result<(Vec<String>, String)> {
            Err(ServerError::Unsupported)
        }
        async fn fetch_html(&self, _root: HtmlFileRoot, _path: &str) -> Result<String> {
            Err(ServerError::Unsupported)
        }
        async fn crossref_lookup(&self, _symbol: &str) -> Result<Option<CrossrefData>> {
            Ok(None)
        }
        async fn jumpref_lookup(&self, _symbol: &str) -> Result<Option<JumprefData>> {
            Ok(None)
        }
        async fn coverage_summary(
            &self,
            _path: &str,
        ) -> Result<Option<crate::file_format::code_coverage_report::NodeMetadata>> {
            Err(ServerError::Unsupported)
        }
        async fn fetch_raw_analysis<'a>(
            &self,
            _path: &str,
        ) -> Result<futures_core::stream::BoxStream<'a, Value>> {
            Err(ServerError::Unsupported)
        }
        async fn search_files(
            &self,
            _pathre: &str,
            _not_pathre: &str,
            _include_dirs: bool,
            _limit: usize,
        ) -> Result<crate::abstract_server::FileMatches> {
            Err(ServerError::Unsupported)
        }
        async fn search_identifiers(
            &self,
            _needle: &str,
            _exact: bool,
            _ignore_case: bool,
            _limit: usize,
        ) -> Result<Vec<(ustr::Ustr, ustr::Ustr)>> {
            Err(ServerError::Unsupported)
        }
        async fn search_text(
            &self,
            _pattern: &str,
            _fold_case: bool,
            _path: &str,
            _not_path: &str,
            _limit: usize,
        ) -> Result<crate::abstract_server::TextMatches> {
            Err(ServerError::Unsupported)
        }
        async fn perform_query(&self, _q: &str) -> Result<Value> {
            Err(ServerError::Unsupported)
        }
        async fn fetch_history_journal(
            &self,
            _kind: crate::hyperblame::journals::JournalKind,
            _target: &str,
            _source_rev: Option<&str>,
            _expand: bool,
        ) -> Result<Vec<Value>> {
            Err(ServerError::Unsupported)
        }
        async fn spec_prs(&self, spec: Option<&str>) -> Result<Vec<RepoSummary>> {
            if spec.is_some_and(|spec| spec != "gizmo") {
                return Ok(vec![]);
            }
            Ok(vec![RepoSummary {
                spec: "gizmo".to_string(),
                repo: "example/gizmo".to_string(),
                url: "https://github.com/example/gizmo".to_string(),
                synced: "2026-10-26T14:03:07Z".to_string(),
                snapshot: None,
                trunk: "0".repeat(40),
                prs: vec![
                    pr(2, Some("CHANGES_REQUESTED"), "CONTRIBUTOR", true),
                    pr(1, None, "MEMBER", false),
                ],
            }])
        }
        async fn spec_prs_sync_status(&self, _spec: Option<&str>) -> Result<SyncStatus> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn test_format_spec_prs() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let render = |status: SyncStatus, spec: Option<&str>| {
            let mut writer = Vec::new();
            crate::format::format_spec_prs(&Fake(status), "specs", spec, None, &mut writer)
                .map(|()| String::from_utf8(writer).unwrap())
        };
        let idle = SyncStatus::Idle {
            finished: None,
            error: None,
        };
        let page = render(idle.clone(), Some("gizmo")).unwrap();
        assert!(page.contains("Pull requests of example/gizmo"), "{}", page);
        assert!(page.contains(r#"href="/specs/try/gizmo/2""#), "{}", page);
        assert!(
            page.contains(r#"href="/specs/rendered/dom/dom.bs#s1""#),
            "{}",
            page
        );
        assert!(
            page.contains(r#"href="/specs/rendered-prs/dom/dom.bs""#),
            "{}",
            page
        );
        assert!(page.contains(r#"data-nouns="pull requests""#), "{}", page);
        assert!(page.contains("Synced 2026-10-26 14:03 UTC."), "{}", page);
        let page = render(idle, None).unwrap();
        assert!(page.contains(r#"href="/specs/spec-prs/gizmo""#), "{}", page);
        let page = render(
            SyncStatus::Running {
                seconds: 3,
                step: "gizmo: fetching".to_string(),
            },
            None,
        )
        .unwrap();
        assert!(page.contains("Syncing (3s so far)"), "{}", page);
        let page = render(
            SyncStatus::Idle {
                finished: Some(5),
                error: Some("No snapshot".to_string()),
            },
            Some("gizmo"),
        )
        .unwrap();
        assert!(page.contains("The last sync failed"), "{}", page);
        assert!(render(SyncStatus::Unavailable, None).is_err());
        // (A spec repository without a summary yet.)
        let mut writer = Vec::new();
        crate::format::format_spec_prs(
            &Fake(SyncStatus::Idle {
                finished: None,
                error: None,
            }),
            "specs",
            Some("other"),
            None,
            &mut writer,
        )
        .unwrap();
        let page = String::from_utf8(writer).unwrap();
        assert!(page.contains("haven't been synced yet"), "{}", page);
    }

    fn pr(number: u32, review: Option<&str>, association: &str, draft: bool) -> PrSummary {
        PrSummary {
            number,
            title: format!("PR {}", number),
            url: None,
            author: Some("someone".to_string()),
            association: Some(association.to_string()),
            draft,
            labels: vec![],
            review: review.map(str::to_string),
            head: None,
            updated: None,
            files: vec![FileSummary {
                path: "dom/dom.bs".to_string(),
                spec: Some("dom".to_string()),
                added: 1,
                removed: 0,
                hunks: vec![[10, 0, 10, 1]],
                sections: vec![SectionRef {
                    line: 5,
                    id: Some(format!("s{}", number % 2)),
                    title: format!("Section {}", number % 2),
                }],
            }],
            specs: vec!["dom".to_string()],
            error: None,
        }
    }

    #[test]
    fn test_block_changes() {
        let trunk = "a\nb\nc\nd\ne\nf\n";
        // (Changes c, adds after e, removes f.)
        let merged = "a\nb\nC\nd\ne\nnew\nnewer\n";
        let hunks = line_hunks(trunk, merged);
        assert_eq!(
            hunks,
            [
                LineHunk {
                    trunk: 2..3,
                    merged: 2..3
                },
                LineHunk {
                    trunk: 5..6,
                    merged: 5..7
                },
            ]
        );
        let hunks = vec![
            LineHunk {
                trunk: 2..3,
                merged: 2..3,
            },
            // (Changes the same block as the first.)
            LineHunk {
                trunk: 3..3,
                merged: 4..5,
            },
            // (Only removes lines.)
            LineHunk {
                trunk: 6..8,
                merged: 7..7,
            },
            // (Adds lines no block has.)
            LineHunk {
                trunk: 9..9,
                merged: 8..9,
            },
        ];
        let blocks = vec![Some(0..=1), Some(2..=4), None, Some(5..=7)];
        let changes = block_changes(&hunks, &blocks, &(0..10), 10);
        assert_eq!(
            changes,
            [
                BlockChange {
                    anchor: 2,
                    trunk: 2..3,
                    merged: 2..5,
                    blocks: vec![1],
                },
                BlockChange {
                    anchor: 6,
                    trunk: 6..8,
                    merged: 7..7,
                    blocks: vec![],
                },
            ]
        );
        // (A removal of another page's lines.)
        assert_eq!(block_changes(&hunks[2..3], &blocks, &(0..5), 10), []);
        // (Lines added go after the line before them.)
        let added = [LineHunk {
            trunk: 5..5,
            merged: 5..7,
        }];
        assert_eq!(block_changes(&added, &blocks, &(0..10), 10)[0].anchor, 4);
    }

    #[test]
    fn test_source_changes() {
        let trunk = "a\nb\nc\nd\ne\nf\n";
        let merged = "a\nb\nC\nd\ne\nnew\nnewer\n";
        assert_eq!(
            source_changes(trunk, merged, |lines| lines.clone(), 6),
            [
                SourceChange {
                    anchor: 2,
                    trunk: 2..3,
                    removed: vec!["c".to_string()],
                    added: vec!["C".to_string()],
                    more_removed: 0,
                    more_added: 0,
                },
                SourceChange {
                    anchor: 5,
                    trunk: 5..6,
                    removed: vec!["f".to_string()],
                    added: vec!["new".to_string(), "newer".to_string()],
                    more_removed: 0,
                    more_added: 0,
                },
            ]
        );
        // (At most `SOURCE_CHANGE_LINES` of each side.)
        let many = "x\n".repeat(SOURCE_CHANGE_LINES + 5);
        let changes = source_changes("a\n", &many, |lines| lines.clone(), 1);
        assert_eq!(changes[0].added.len(), SOURCE_CHANGE_LINES);
        assert_eq!(changes[0].more_added, 5);
        assert_eq!(changes[0].more_removed, 0);
    }

    #[test]
    fn test_map_lines() {
        // (The later version adds two lines after line 1, changes line 3 into
        // two, and removes line 5.)
        let earlier = "0\n1\n2\n3\n4\n5\n6\n";
        let later = "0\n1\nx\ny\n2\n3a\n3b\n4\n6\n";
        let hunks = line_hunks(earlier, later);
        assert_eq!(map_lines(&hunks, &(0..2)), 0..2);
        assert_eq!(map_lines(&hunks, &(2..3)), 4..5);
        assert_eq!(map_lines(&hunks, &(3..4)), 5..7);
        assert_eq!(map_lines(&hunks, &(4..5)), 7..8);
        // (Removed lines are where they were.)
        assert_eq!(map_lines(&hunks, &(5..6)), 8..8);
        assert_eq!(map_lines(&hunks, &(5..7)), 8..9);
        // (Lines added after line 1 are before the lines it added there.)
        assert_eq!(map_lines(&hunks, &(2..2)), 2..2);
    }

    #[test]
    fn test_pr_facets() {
        let prs = vec![
            pr(1, Some("APPROVED"), "MEMBER", false),
            pr(2, Some("CHANGES_REQUESTED"), "CONTRIBUTOR", false),
            pr(3, None, "NONE", true),
        ];
        let facets = pr_facets(&prs);
        let keys: Vec<&str> = facets.iter().map(|f| f.key).collect();
        // (One spec, so no spec facet.)
        assert_eq!(keys, ["review", "author", "draft", "section"]);
        let author: Vec<(String, u32)> = facets[1]
            .values
            .iter()
            .map(|v| (v.name.clone(), v.count))
            .collect();
        assert_eq!(
            author,
            [
                ("Other contributors".to_string(), 2),
                ("Can review".to_string(), 1)
            ]
        );
        assert_eq!(pr_facet_values(&prs[2])["review"], ["NONE"]);
        assert!(can_review(Some("COLLABORATOR")) && !can_review(Some("FIRST_TIME_CONTRIBUTOR")));
    }
}
