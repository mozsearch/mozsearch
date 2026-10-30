//! The web server's imports of try pushes into the token-centric history and
//! commit index of the trees which can import them, for reviewing them
//! (`/{tree}/try/hg/{rev}`, the URL naming the kind of push; see `url_push`
//! and `format::format_try`), whose views then have
//! them like the branch's revisions (see `TreeHistory`'s try notes, and the
//! commit index's other branches).  What's imported is what the history
//! derives (the token-centric blame, and the explore page's sparklines), and,
//! if the tree's hook does it, the push's semantic analysis (see `try_delta`),
//! whose steps the hook logs as "Performing semantic::STEP step" lines (like
//! the indexing's other steps), so that the push's page can say why it has no
//! semantic analysis if it failed (see `semantic_failure`).
//!
//! A tree can import try pushes if it has a history and its directory in the
//! config repo has an executable `import-try` hook (like `upload-after-index`
//! and others).  The hook gets the push's name as its argument (see
//! `normalize_push`): its hg revision on try (`/{tree}/try/hg/{rev}`), or in a
//! tree of specs (see `spec_repos`), a spec repository's pull request,
//! `SPEC/NUMBER` (`/{tree}/try/{spec}/{number}`, which the hook makes into
//! forest commits with build-spec-pr, and which can be imported again for its
//! newer commits), and these
//! environment variables: MOZSEARCH_PATH, CONFIG_REPO, TREE_NAME, GIT_ROOT (the
//! tree's `git_path`), HISTORY_ROOT (its `history_path`), GIT_BRANCH (its
//! `git_branch`, or HEAD), and MAX_REVISIONS.  It fetches the push into the
//! tree's source repo (ex: with git-cinnabar, from hg's try repository) and
//! runs mozsearch's scripts/import-try-push.sh, which processes it into the
//! history's and the commit index's `try` branches (see `source_mapping`'s
//! "Multiple heads"), writes the push's own commits to
//! `HISTORY_ROOT/try-pushes/REV`, and last sets `refs/try/REV` in the source
//! repo (REV being the hook's argument), so that a push is imported once that
//! ref exists and the history has its head (see `imported_head`).
//!
//! Imports start in the order they were asked for, on `WORKERS` threads, each
//! running the hook at a lower priority than the server's (`nice`) and in its
//! own process group, so that it and what it runs can be killed after
//! `TIME_LIMIT`, with its output in `HISTORY_ROOT/try-imports/REV.log`.  A
//! try push's import runs alone (the history repos and the commit index take
//! one writer at a time, and pushes based on the same new revisions would
//! process them twice at once), but up to `WORKERS` spec pull requests'
//! imports run at a time, mostly rendering and analyzing their specs, whose
//! hooks take turns for their history steps by locking the file
//! `TRY_IMPORT_LOCK` (ex: with flock(1); see
//! scripts/import-try-push-batched.sh, which imports the histories of those
//! waiting at once), and for their other steps writing to the source repo
//! with locks of their own, ex: a spec pull request's forest commits (which
//! write only its own refs, which the history steps don't; see the wubspecs
//! config's import-try hook), so that they can make them while the history's
//! is held, and wait for it together.  The
//! queue only lives in memory: a restart of the server loses the waiting and
//! running imports, but asking again resumes them (the history tools skip the
//! revisions they've processed), and finished imports are in the repos.  At
//! most `MAX_QUEUED` try pushes' imports wait at a time (and
//! `MAX_QUEUED_PULL_REQUESTS` spec pull requests', whose summaries' pages ask
//! for a spec's at once; see `TryImports::request_all`), and a failed import
//! can only be asked for again after `RETRY_AFTER`.
//!
//! Imports are on the web server's index volume, which the next indexing
//! replaces, but the indexer asks the web server which spec pull requests it
//! imported (see infrastructure/aws/carry-over-imports.py), and the next web
//! server imports them again, those still open (see
//! `spec_prs::start`).

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use git2::Oid;
use serde::{Deserialize, Serialize};

use crate::file_format::config::Config;

/// The step of an import's status while its pages' data is precomputed after
/// its hook (see `TryImports::run`).
pub const PRECOMPUTING: &str = "Precomputing its canvas and dataflow";

/// The most try pushes' imports which can be waiting at a time.
pub const MAX_QUEUED: usize = 8;
/// The most spec pull requests' imports which can be waiting at a time.
pub const MAX_QUEUED_PULL_REQUESTS: usize = 500;
/// The most spec pull requests which can be asked for at once (see
/// `TryImports::request_all`): all of a big spec's, ex: the HTML Standard's 315
/// open pull requests (2026-10-30).
pub const MAX_BATCH: usize = 500;
/// How many imports can run at a time (only spec pull requests' run beside
/// others; see the module's comment).  Spec pull requests' imports hold the
/// history's lock for their history steps, which import the histories of
/// those waiting at once, and then render and analyze unlocked.  On
/// 2026-10-30, with 6 workers on wubspecs' web server, whose hooks still made
/// their forest commits holding the history's lock (so that few could wait
/// for it together):
/// - 78 restored imports of smaller specs' pull requests took 207 s: 2.6 s of
///   history alone (1.2-1.6 s each in batches of 4 or 5), then about 11 s
///   unlocked, using half a core or so and at most 4 GB.
/// - The HTML Standard's 314 took 48 minutes, the lock busy 99% of the time:
///   11.9 s of history alone, 7-8 s each in batches of 2-4 (its 1.5 million
///   tokens, and many pull requests' many revisions), then about 25 s
///   unlocked, using a core or so, 3.3 GB at the median and 15 GB at the
///   99th percentile (19 GB at most).
///
/// With their forest commits beside the history steps, 10 workers don't wait
/// for the lock for smaller specs.  The HTML Standard's, which the lock bounds
/// whatever the workers, make batches of those waiting (while one batch's
/// history runs, the others render, analyze and wait for the next), at about
/// 7.5 s each, so about 39 minutes rather than 48 for those 314; and since
/// they finish their history steps a batch at a time, a batch's worth (not
/// all 10) render and analyze at once.
pub const WORKERS: usize = 10;
/// The most revisions an import processes (see import-try-push.sh), which
/// includes those of its base which are newer than the tree's revision.
pub const MAX_REVISIONS: usize = 2000;
/// How long an import can take before it's killed.
pub const TIME_LIMIT: Duration = Duration::from_secs(60 * 60);
/// How long after an import failed it can be asked for again.
pub const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The imports of the server, which `run` performs.
pub static IMPORTS: LazyLock<TryImports> = LazyLock::new(TryImports::default);

/// The hook which imports a tree's try pushes, if it can.
pub fn hook(cfg: &Config, tree_name: &str) -> Option<PathBuf> {
    let tree_config = cfg.trees.get(tree_name)?;
    tree_config.paths.history_path.as_ref()?;
    let hook = Path::new(&cfg.config_repo_path)
        .join(tree_name)
        .join("import-try");
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&hook)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    executable.then_some(hook)
}

/// The tree which imports try pushes for `tree_name`, if not itself: one
/// sharing its history which has a hook while it doesn't (ex: firefox-try,
/// firefox-disco's sibling with only the platform pushes are compiled for),
/// to which its pages for try pushes send people.
pub fn importing_tree(cfg: &Config, tree_name: &str) -> Option<String> {
    if hook(cfg, tree_name).is_some() {
        return None;
    }
    let history = cfg.trees.get(tree_name)?.paths.history_path.as_ref()?;
    cfg.trees
        .iter()
        .find(|(name, tree)| {
            name.as_str() != tree_name
                && tree.paths.history_path.as_ref() == Some(history)
                && hook(cfg, name).is_some()
        })
        .map(|(name, _)| name.clone())
}

/// A try push's revision, as the hook takes it: an abbreviated or full
/// revision (12 to 40 hex digits, lowercased), if it's one.
pub fn normalize_rev(rev: &str) -> Option<String> {
    let rev = rev.trim();
    ((12..=40).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| rev.to_ascii_lowercase())
}

/// A try push's name, as the hook takes it and as its import's refs and files
/// are named (`refs/try/NAME`, `try-pushes/NAME`; see the module's comment),
/// if `name` is one's (which keeps those paths in their directories): an hg
/// revision on try (see `normalize_rev`), or a spec's pull request,
/// `SPEC/NUMBER` (see `url_push`).
pub fn normalize_push(name: &str) -> Option<String> {
    if let Some(rev) = normalize_rev(name) {
        return Some(rev);
    }
    let (spec, number) = name.split_once('/')?;
    let spec_ok = !spec.is_empty()
        && spec.len() <= 100
        && spec != "hg"
        && !spec.starts_with('.')
        && spec
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let number_ok = (1..=9).contains(&number.len())
        && !number.starts_with('0')
        && number.bytes().all(|b| b.is_ascii_digit());
    (spec_ok && number_ok).then(|| name.to_string())
}

/// Whether a try push (see `normalize_push`) is a spec's pull request, which
/// can be imported again (for its newer commits), rather than an hg push.
pub fn is_pull_request(name: &str) -> bool {
    name.contains('/')
}

/// The spec repositories of a tree of specs (its index's specs.toml's specs
/// with upstreams; see `spec_forest`), whose pull requests it imports, or
/// none for other trees.
pub fn spec_repos(
    cfg: &Config,
    tree_name: &str,
) -> std::sync::Arc<Vec<crate::spec_forest::SpecConfig>> {
    static REPOS: LazyLock<
        Mutex<HashMap<String, std::sync::Arc<Vec<crate::spec_forest::SpecConfig>>>>,
    > = LazyLock::new(Default::default);
    let mut repos = REPOS.lock().unwrap();
    repos
        .entry(tree_name.to_string())
        .or_insert_with(|| {
            let specs = cfg
                .trees
                .get(tree_name)
                .map(|tree| Path::new(&tree.paths.index_path).join("specs.toml"))
                .filter(|path| path.exists())
                .and_then(|path| crate::spec_forest::ForestConfig::load(&path).ok())
                .map_or_else(Vec::new, |config| config.specs);
            std::sync::Arc::new(specs)
        })
        .clone()
}

/// The try push (as the hook takes it; see `normalize_push`) of a try push's
/// page's URL's kind and id (`/{tree}/try/{kind}/{id}`; see `push_url_path`),
/// if they're one's: in a tree of specs, a spec repository's pull request
/// (`try/SPEC/NUMBER`, ex: `try/dom/1512`), and in other trees, an hg revision
/// on try (`try/hg/REV`).
pub fn url_push(cfg: &Config, tree_name: &str, kind: &str, id: &str) -> Option<String> {
    let repos = spec_repos(cfg, tree_name);
    if repos.is_empty() {
        return (kind == "hg").then(|| normalize_rev(id)).flatten();
    }
    if !repos.iter().any(|spec| spec.name == kind) {
        return None;
    }
    normalize_push(&format!("{}/{}", kind, id)).filter(|name| is_pull_request(name))
}

/// The try push of a stack context, the path of its page in the tree (ex:
/// "try/hg/abcdef012345" or "try/dom/1512"; see `url_push`), if it's a try
/// push's (by its form; see `normalize_push`).
pub fn stack_push(stack: &str) -> Option<String> {
    let (kind, id) = stack.strip_prefix("try/")?.split_once('/')?;
    match kind {
        "hg" => normalize_rev(id),
        _ => normalize_push(&format!("{}/{}", kind, id)).filter(|name| is_pull_request(name)),
    }
}

/// The path of a try push's page in its tree (`/{tree}/{path}`), which is also
/// its stack context (see `stack_push`), ex: "try/hg/abcdef012345" or
/// "try/dom/1512".
pub fn push_url_path(push: &str) -> String {
    if is_pull_request(push) {
        format!("try/{}", push)
    } else {
        format!("try/hg/{}", push)
    }
}

/// The try push whose stack rebased onto the indexed revision (see
/// `try_delta`) has its head at `head` (`refs/try-rebased/{push}`), if any.
pub fn rebased_head_push(repo: &git2::Repository, head: &str) -> Option<String> {
    let oid = git2::Oid::from_str(head).ok()?;
    // (Spec pull requests' are `refs/try-rebased/{spec}/{number}`.)
    for glob in ["refs/try-rebased/*", "refs/try-rebased/*/*"] {
        for reference in repo.references_glob(glob).ok()?.flatten() {
            if reference.target() != Some(oid) {
                continue;
            }
            if let Some(push) = reference
                .name()
                .ok()
                .and_then(|name| name.strip_prefix("refs/try-rebased/"))
            {
                return Some(push.to_string());
            }
        }
    }
    None
}

/// How a try push's pages name it: their title (ex: "Try push abcdef012345",
/// or "Pull request #1512 of whatwg/dom"), and its upstream page, if any (ex:
/// the pull request's on GitHub).
pub fn push_title(cfg: &Config, tree_name: &str, push: &str) -> (String, Option<String>) {
    let Some((spec, number)) = push
        .split_once('/')
        .and_then(|(spec, number)| Some((spec, number.parse::<u32>().ok()?)))
    else {
        return (format!("Try push {}", push), None);
    };
    let repos = spec_repos(cfg, tree_name);
    let config = repos.iter().find(|config| config.name == spec);
    let link = config.and_then(|config| config.pull_request_url(number));
    let repo = link
        .as_deref()
        .and_then(|link| link.strip_prefix("https://github.com/"))
        .and_then(|rest| rest.rsplit_once("/pull/"))
        .map_or(spec, |(repo, _)| repo);
    (format!("Pull request #{} of {}", number, repo), link)
}

/// The head of the try push `rev` (see `normalize_rev`), if the tree has
/// imported it.
pub fn imported_head(cfg: &Config, tree_name: &str, rev: &str) -> Option<Oid> {
    let git = cfg.trees.get(tree_name)?.git.as_ref()?;
    let head = git
        .repo
        .find_reference(&format!("refs/try/{}", rev))
        .ok()?
        .peel_to_commit()
        .ok()?
        .id();
    git.history.as_ref()?.timeline_commit(head)?;
    Some(head)
}

/// The spec pull requests imported into the tree (`SPEC/NUMBER`; see
/// `imported_head`), for importing them again into the next index's (see
/// `spec_prs::start`).
pub fn imported_pull_requests(cfg: &Config, tree_name: &str) -> Vec<String> {
    let Some(git) = cfg.trees.get(tree_name).and_then(|tree| tree.git.as_ref()) else {
        return vec![];
    };
    let Ok(references) = git.repo.references() else {
        return vec![];
    };
    let mut pushes: Vec<String> = references
        .flatten()
        .filter_map(|reference| {
            let push = reference
                .name()
                .ok()?
                .strip_prefix("refs/try/")?
                .to_string();
            (is_pull_request(&push) && normalize_push(&push).as_deref() == Some(push.as_str()))
                .then_some(push)
        })
        .filter(|push| imported_head(cfg, tree_name, push).is_some())
        .collect();
    pushes.sort();
    pushes
}

/// The stack of the imported try push `rev`, oldest first, which the import
/// listed (see import-try-push.sh), or else its head; and the number of its
/// commits which only select try's tasks, which aren't listed.
pub fn push_commits(cfg: &Config, tree_name: &str, rev: &str) -> (Vec<Oid>, usize) {
    let Some(head) = imported_head(cfg, tree_name, rev) else {
        return (vec![], 0);
    };
    let list = cfg.trees[tree_name]
        .paths
        .history_path
        .as_ref()
        .and_then(|history| {
            fs::read_to_string(Path::new(history).join("try-pushes").join(rev)).ok()
        })
        .unwrap_or_default();
    let mut stack = vec![];
    let mut task_selections = 0;
    for line in list.lines().map(str::trim) {
        match line.strip_prefix('-') {
            Some(_) => task_selections += 1,
            None => stack.extend(Oid::from_str(line).ok()),
        }
    }
    if stack.is_empty() && task_selections == 0 {
        stack.push(head);
    }
    (stack, task_selections)
}

/// The imported try push whose stack has the commit `rev` (see
/// `push_commits`), if any: its name (ex: an hg revision, or a spec's pull
/// request's SPEC/NUMBER).
pub fn commit_push(cfg: &Config, tree_name: &str, rev: &str) -> Option<String> {
    let history = cfg.trees.get(tree_name)?.paths.history_path.as_ref()?;
    let pushes = Path::new(history).join("try-pushes");
    // (A spec's pull request's list is in its spec's directory.)
    let mut lists = vec![];
    for entry in fs::read_dir(&pushes).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            lists.extend(
                fs::read_dir(&path)
                    .ok()?
                    .flatten()
                    .map(|entry| entry.path()),
            );
        } else {
            lists.push(path);
        }
    }
    lists.into_iter().find_map(|list| {
        fs::read_to_string(&list)
            .ok()?
            .lines()
            .any(|line| line.trim() == rev)
            .then(|| {
                list.strip_prefix(&pushes)
                    .ok()?
                    .to_str()
                    .map(str::to_string)
            })?
    })
}

/// An imported try push, for its page (see `format::format_try` and
/// `AbstractServer::try_push`).
#[derive(Debug, Deserialize, Serialize)]
pub struct TryPush {
    /// Its stack, oldest first, with backouts marked (see
    /// `explore::mark_history_backouts`), at most `explore::MAX_COMMITS` of
    /// it.
    pub stack: Vec<crate::commit_index::CommitRef>,
    /// Whether its stack has more commits than `stack`.
    pub truncated: bool,
    /// How many of its commits only select try's tasks (see `push_commits`).
    pub task_selections: usize,
    /// The head of its stack rebased onto the indexed revision, if that has a
    /// semantic delta (see `try_delta`).
    pub semantic_head: Option<String>,
    /// And if it has none, why, if its import's analysis failed.
    pub semantic_failure: Option<SemanticFailure>,
    /// Its commits which have landed since it diverged from the trunk, which
    /// its semantic analysis left out (see scripts/rebase-try-push.sh).
    #[serde(default)]
    pub landed: Vec<LandedCommit>,
}

/// A try push's commit which has landed, by its import's log (see
/// scripts/rebase-try-push.sh's "Landed: " lines).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct LandedCommit {
    /// The push's commit.
    pub rev: String,
    /// The commit it landed as, in the tree's history.
    pub landed: String,
    /// Whether its changes are the landed commit's, rather than it being the
    /// same Phabricator revision (which may have changed before it landed).
    pub same_changes: bool,
    /// The landed commit's summary (its message's first line).
    #[serde(default)]
    pub summary: String,
    /// The push's commit's summary.
    #[serde(default)]
    pub rev_summary: String,
}

/// The head of the try push `push`'s stack rebased onto the indexed revision
/// (`refs/try-rebased/{push}`), if it has a semantic analysis (see
/// `try_delta`).
pub fn semantic_head(cfg: &Config, tree_name: &str, push: &str) -> Option<String> {
    let tree_config = cfg.trees.get(tree_name)?;
    let git = tree_config.git.as_ref()?;
    git.repo
        .refname_to_id(&format!("refs/try-rebased/{}", push))
        .ok()
        .map(|oid| oid.to_string())
        .filter(|head| {
            crate::try_delta::delta_dir(&tree_config.paths.index_path, head)
                .join("meta.json")
                .exists()
        })
}

/// The imported try push `rev` (see `normalize_rev`), if the tree imported
/// it.
pub fn try_push(cfg: &Config, tree_name: &str, rev: &str) -> Option<TryPush> {
    let tree_config = cfg.trees.get(tree_name)?;
    let git = tree_config.git.as_ref()?;
    imported_head(cfg, tree_name, rev)?;
    let (stack, task_selections) = push_commits(cfg, tree_name, rev);
    let mut stack: Vec<crate::commit_index::CommitRef> = stack
        .into_iter()
        .filter_map(|oid| git.repo.find_commit(oid).ok())
        .map(|commit| crate::commit_index::commit_ref(&commit))
        .collect();
    let truncated = stack.len() > crate::hyperblame::explore::MAX_COMMITS;
    stack.truncate(crate::hyperblame::explore::MAX_COMMITS);
    crate::hyperblame::explore::mark_history_backouts(
        git.history.as_ref().map(|h| Path::new(&h.path)),
        &mut stack,
    );
    let semantic_head = semantic_head(cfg, tree_name, rev);
    let semantic_failure = match semantic_head {
        Some(_) => None,
        None => semantic_failure(cfg, tree_name, rev),
    };
    let landed = import_log(cfg, tree_name, rev)
        .map(|log| parse_landed(&log))
        .unwrap_or_default()
        .into_iter()
        .map(|mut landed| {
            let summary = |rev: &str| {
                Oid::from_str(rev)
                    .ok()
                    .and_then(|oid| git.repo.find_commit(oid).ok())
                    .map(|commit| {
                        commit
                            .summary()
                            .ok()
                            .flatten()
                            .unwrap_or_default()
                            .to_string()
                    })
                    .unwrap_or_default()
            };
            landed.summary = summary(&landed.landed);
            landed.rev_summary = summary(&landed.rev);
            landed
        })
        .collect();
    Some(TryPush {
        stack,
        truncated,
        task_selections,
        semantic_head,
        semantic_failure,
        landed,
    })
}

/// What's become of an import, for the try pages and their polling.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ImportStatus {
    /// The tree can't import try pushes.
    Unavailable,
    NotImported,
    /// Waiting, after `ahead` others.
    Queued {
        ahead: usize,
    },
    /// Running for `seconds`, its output's last line being `step`.
    Running {
        seconds: u64,
        step: String,
    },
    Imported {
        head: String,
    },
    /// It failed `seconds` ago, with its output's last lines as `message`; it
    /// can be asked for again after `RETRY_AFTER`.
    Failed {
        message: String,
        seconds: u64,
        retry: bool,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Job {
    tree: String,
    rev: String,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    /// The running jobs, when each started, and its step.
    running: Vec<(Job, Instant, String)>,
    failed: HashMap<Job, (String, Instant)>,
}

/// Whether the import of `rev` can start beside the imports of `running`: a
/// spec pull request's beside fewer than `WORKERS` others, all spec pull
/// requests', and a try push's alone.
fn can_start(rev: &str, running: &[&str]) -> bool {
    match is_pull_request(rev) {
        true => running.len() < WORKERS && running.iter().all(|rev| is_pull_request(rev)),
        false => running.is_empty(),
    }
}

#[derive(Default)]
pub struct TryImports {
    state: Mutex<State>,
    queued: Condvar,
}

impl TryImports {
    /// The status of the import of the try push `rev` (see `normalize_rev`)
    /// into the tree.
    pub fn status(&self, cfg: &Config, tree_name: &str, rev: &str) -> ImportStatus {
        if hook(cfg, tree_name).is_none() {
            return ImportStatus::Unavailable;
        }
        let job = Job {
            tree: tree_name.to_string(),
            rev: rev.to_string(),
        };
        {
            let state = self.state.lock().unwrap();
            if let Some((_, started, step)) =
                state.running.iter().find(|(running, _, _)| *running == job)
            {
                return ImportStatus::Running {
                    seconds: started.elapsed().as_secs(),
                    step: step.clone(),
                };
            }
            if let Some(ahead) = state.queue.iter().position(|queued| *queued == job) {
                return ImportStatus::Queued {
                    ahead: ahead + state.running.len(),
                };
            }
            if let Some((message, at)) = state.failed.get(&job) {
                return ImportStatus::Failed {
                    message: message.clone(),
                    seconds: at.elapsed().as_secs(),
                    retry: at.elapsed() >= RETRY_AFTER,
                };
            }
        }
        match imported_head(cfg, tree_name, rev) {
            Some(head) => ImportStatus::Imported {
                head: head.to_string(),
            },
            None => ImportStatus::NotImported,
        }
    }

    /// Ask for the try push `rev` (see `normalize_push`) to be imported into
    /// the tree, unless it is (but for spec pull requests, which are imported
    /// again, for their newer commits) or will be, or it failed less than
    /// `RETRY_AFTER` ago, or too many imports are waiting; and its status.
    pub fn request(
        &self,
        cfg: &Config,
        tree_name: &str,
        rev: &str,
    ) -> Result<ImportStatus, &'static str> {
        let status = self.status(cfg, tree_name, rev);
        match status {
            ImportStatus::NotImported | ImportStatus::Failed { retry: true, .. } => {}
            ImportStatus::Imported { .. } if is_pull_request(rev) => {}
            ImportStatus::Unavailable => return Err("This tree can't import try pushes"),
            _ => return Ok(status),
        }
        self.enqueue(tree_name, &[rev.to_string()])?;
        Ok(self.status(cfg, tree_name, rev))
    }

    /// Ask for the spec pull requests `revs` (`SPEC/NUMBER`: those of a spec's
    /// summaries which aren't imported at their heads; see
    /// `spec_prs::pr_imports`) to be imported into the tree, all of them, as
    /// `request` would each, or none: if there are more than `MAX_BATCH`, or
    /// not enough room for them in the queue.  Returns how many it asked for (not
    /// those waiting or running already).
    pub fn request_all(
        &self,
        cfg: &Config,
        tree_name: &str,
        revs: &[String],
    ) -> Result<usize, &'static str> {
        if hook(cfg, tree_name).is_none() {
            return Err("This tree can't import try pushes");
        }
        if revs.iter().any(|rev| !is_pull_request(rev)) {
            return Err("Only spec pull requests are imported together");
        }
        if revs.len() > MAX_BATCH {
            return Err("Too many pull requests to import at once; import them from their pages");
        }
        let revs: Vec<String> = revs
            .iter()
            .filter(|rev| {
                matches!(
                    self.status(cfg, tree_name, rev),
                    ImportStatus::NotImported
                        | ImportStatus::Failed { retry: true, .. }
                        | ImportStatus::Imported { .. }
                )
            })
            .cloned()
            .collect();
        self.enqueue(tree_name, &revs)
    }

    /// Queue the imports of the spec pull requests `revs` (`SPEC/NUMBER`)
    /// which the previous index's web server had imported (see
    /// `spec_prs::start`), but those imported, waiting or running,
    /// or which failed less than `RETRY_AFTER` ago, however many there are;
    /// how many it queued.
    pub fn restore(&self, cfg: &Config, tree_name: &str, revs: &[String]) -> usize {
        if hook(cfg, tree_name).is_none() {
            return 0;
        }
        let revs: Vec<String> = revs
            .iter()
            .filter(|rev| {
                is_pull_request(rev)
                    && matches!(
                        self.status(cfg, tree_name, rev),
                        ImportStatus::NotImported | ImportStatus::Failed { retry: true, .. }
                    )
            })
            .cloned()
            .collect();
        self.enqueue_jobs(tree_name, &revs, false).unwrap_or(0)
    }

    /// Queue the imports of `revs` into the tree, but those waiting or
    /// running, all of them or (if there isn't room for them) none; how many
    /// it queued.
    fn enqueue(&self, tree_name: &str, revs: &[String]) -> Result<usize, &'static str> {
        self.enqueue_jobs(tree_name, revs, true)
    }

    /// As `enqueue`, or without the queue's limits (unless `limited`).
    fn enqueue_jobs(
        &self,
        tree_name: &str,
        revs: &[String],
        limited: bool,
    ) -> Result<usize, &'static str> {
        let jobs = {
            let mut state = self.state.lock().unwrap();
            let jobs: Vec<Job> = revs
                .iter()
                .map(|rev| Job {
                    tree: tree_name.to_string(),
                    rev: rev.to_string(),
                })
                .filter(|job| {
                    !state.queue.contains(job)
                        && !state.running.iter().any(|(running, _, _)| running == job)
                })
                .collect();
            for pull_requests in [false, true] {
                let queued = state
                    .queue
                    .iter()
                    .filter(|job| is_pull_request(&job.rev) == pull_requests)
                    .count();
                let asked = jobs
                    .iter()
                    .filter(|job| is_pull_request(&job.rev) == pull_requests)
                    .count();
                match pull_requests {
                    _ if !limited => {}
                    false if asked > 0 && queued + asked > MAX_QUEUED => {
                        return Err(
                            "Too many try pushes are waiting to be imported; try again later",
                        );
                    }
                    true if asked > 0 && queued + asked > MAX_QUEUED_PULL_REQUESTS => {
                        return Err(
                            "Too many pull requests are waiting to be imported; try again later",
                        );
                    }
                    _ => {}
                }
            }
            for job in &jobs {
                state.failed.remove(job);
                state.queue.push_back(job.clone());
            }
            jobs.len()
        };
        self.queued.notify_all();
        Ok(jobs)
    }

    /// Set the step of the running `job`.
    fn set_step(&self, job: &Job, step: String) {
        if let Some((_, _, running)) = self
            .state
            .lock()
            .unwrap()
            .running
            .iter_mut()
            .find(|(running, _, _)| running == job)
        {
            *running = step;
        }
    }

    /// Perform the imports as they're asked for, forever, as one of the
    /// `WORKERS` threads running this, starting each when it can (see
    /// `can_start`), and after each which succeeds, `precompute` (tree, push)
    /// what its pages show which takes long to compute (ex: its canvas and
    /// its slicing; see the web-server's `precompute_try_push`), before its
    /// status says it's imported (as `Running` until then, with the step
    /// `PRECOMPUTING`).  Failing to precompute only means its pages compute
    /// it when asked.
    pub fn run(&self, cfg: &Config, precompute: &dyn Fn(&str, &str)) {
        loop {
            let job = {
                let mut state = self.state.lock().unwrap();
                loop {
                    // (In the order they were asked for: a try push's import
                    // waits for the running ones, and those after it for it.)
                    let startable = state.queue.front().is_some_and(|job| {
                        let running: Vec<&str> = state
                            .running
                            .iter()
                            .map(|(job, _, _)| job.rev.as_str())
                            .collect();
                        can_start(&job.rev, &running)
                    });
                    if startable && let Some(job) = state.queue.pop_front() {
                        state
                            .running
                            .push((job.clone(), Instant::now(), String::new()));
                        break job;
                    }
                    state = self.queued.wait(state).unwrap();
                }
            };
            let result = self.import(cfg, &job);
            if result.is_ok() {
                self.set_step(&job, PRECOMPUTING.to_string());
                let started = Instant::now();
                precompute(&job.tree, &job.rev);
                if let Some(log_path) = import_log_path(cfg, &job.tree, &job.rev) {
                    append_to_log(
                        &log_path,
                        &format!(
                            "try-import: precomputed its pages' canvas and dataflow in {:.1} s",
                            started.elapsed().as_secs_f64()
                        ),
                    );
                }
            }
            {
                let mut state = self.state.lock().unwrap();
                state.running.retain(|(running, _, _)| *running != job);
                if let Err(message) = result {
                    eprintln!(
                        "Importing try push {} into {} failed: {}",
                        job.rev, job.tree, message
                    );
                    state.failed.insert(job, (message, Instant::now()));
                }
            }
            // (The next ones may start now.)
            self.queued.notify_all();
        }
    }

    /// Run the tree's hook for the job, until it's done or `TIME_LIMIT`.
    /// (`run` precomputes its pages' data after it.)
    fn import(&self, cfg: &Config, job: &Job) -> Result<(), String> {
        let tree_config = cfg.trees.get(&job.tree).ok_or("No such tree")?;
        let hook = hook(cfg, &job.tree).ok_or("The tree can't import try pushes")?;
        let history_root = tree_config
            .paths
            .history_path
            .as_ref()
            .ok_or("The tree has no history")?;
        // (A spec's pull request's, `SPEC/NUMBER`, in its spec's directory.)
        let log_path = Path::new(history_root)
            .join("try-imports")
            .join(format!("{}.log", job.rev));
        fs::create_dir_all(log_path.parent().unwrap()).map_err(|e| e.to_string())?;
        let max_revisions = MAX_REVISIONS.to_string();
        let lock_path = Path::new(history_root)
            .join("try-imports")
            .join("history.lock");
        let envs: Vec<(&str, &str)> = vec![
            ("MOZSEARCH_PATH", &cfg.mozsearch_path),
            ("CONFIG_REPO", &cfg.config_repo_path),
            ("TREE_NAME", &job.tree),
            (
                "GIT_ROOT",
                tree_config.paths.git_path.as_deref().unwrap_or_default(),
            ),
            ("HISTORY_ROOT", history_root),
            (
                "GIT_BRANCH",
                tree_config.paths.git_branch.as_deref().unwrap_or("HEAD"),
            ),
            // (For the semantic analysis of the push; see `try_delta`.)
            ("CONFIG_FILE", &cfg.config_file),
            ("INDEX_ROOT", &tree_config.paths.index_path),
            ("FILES_ROOT", &tree_config.paths.files_path),
            ("OBJDIR", &tree_config.paths.objdir_path),
            ("MAX_REVISIONS", &max_revisions),
            // (Which hooks of imports which can run beside others lock for
            // their steps which can't; see the module's comment.)
            ("TRY_IMPORT_LOCK", lock_path.to_str().unwrap_or_default()),
        ];
        run_hook(
            &hook,
            &[&job.rev],
            &envs,
            &log_path,
            TIME_LIMIT,
            &mut |step| self.set_step(job, step),
        )?;
        if imported_head(cfg, &job.tree, &job.rev).is_none() {
            return Err("The import finished, but the history doesn't have the push".to_string());
        }
        Ok(())
    }
}

/// Run a tree's hook (ex: `import-try`) with `args` and `envs`, at a lower
/// priority than the server's (`nice`) and in its own process group, with
/// its output in `log_path`, until it's done or `time_limit` (when it and
/// what it runs are killed), telling `step` its output's last line every
/// second.  Its error is its output's last lines, if it fails.  Its log ends
/// with how long it took and what it used (see `HookUsage`).
pub fn run_hook(
    hook: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    log_path: &Path,
    time_limit: Duration,
    step: &mut dyn FnMut(String),
) -> Result<(), String> {
    let log = fs::File::create(log_path).map_err(|e| e.to_string())?;
    let mut command = Command::new("nice");
    command.arg("-n10").arg(hook).args(args);
    for (name, value) in envs {
        command.env(name, value);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Couldn't run {}: {}", hook.display(), e))?;
    let started = Instant::now();
    let (status, usage) = loop {
        if let Some(exited) = try_wait_with_usage(&child).map_err(|e| e.to_string())? {
            break exited;
        }
        if started.elapsed() > time_limit {
            // (The hook's process group, with what it runs.)
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.wait();
            return Err(format!(
                "It took longer than {} minutes",
                time_limit.as_secs() / 60
            ));
        }
        step(last_lines(log_path, 1));
        std::thread::sleep(Duration::from_secs(1));
    };
    // (Its error is from before the usage line.)
    let result = match status.success() {
        true => Ok(()),
        false => Err(last_lines(log_path, 12)),
    };
    append_to_log(log_path, &usage.describe(started.elapsed()));
    result
}

/// What a hook's processes used (from wait4, which counts the processes it
/// waited for, and theirs, and so on).
#[derive(Debug, PartialEq)]
struct HookUsage {
    user_seconds: f64,
    system_seconds: f64,
    /// The biggest of its processes' peak resident memory.
    max_rss_bytes: u64,
}

impl HookUsage {
    fn describe(&self, wall: Duration) -> String {
        let wall = wall.as_secs_f64();
        let cpu = self.user_seconds + self.system_seconds;
        format!(
            "try-import: the hook took {:.0} s, using {:.0} CPU seconds ({:.0} user, {:.0} \
             system; {:.1} cores on average), and its biggest process peaked at {:.1} GB",
            wall,
            cpu,
            self.user_seconds,
            self.system_seconds,
            if wall > 0.0 { cpu / wall } else { 0.0 },
            self.max_rss_bytes as f64 / 1e9
        )
    }
}

/// The child's exit status and usage, if it has exited (reaping it, so it
/// mustn't be waited for again).
fn try_wait_with_usage(
    child: &std::process::Child,
) -> std::io::Result<Option<(std::process::ExitStatus, HookUsage)>> {
    let mut status = 0;
    // SAFETY: rusage is plain data, which wait4 fills in.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let pid = unsafe { libc::wait4(child.id() as i32, &mut status, libc::WNOHANG, &mut usage) };
    if pid < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if pid == 0 {
        return Ok(None);
    }
    let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
    Ok(Some((
        std::process::ExitStatus::from_raw(status),
        HookUsage {
            user_seconds: seconds(usage.ru_utime),
            system_seconds: seconds(usage.ru_stime),
            // (In kilobytes, on Linux.)
            max_rss_bytes: usage.ru_maxrss.max(0) as u64 * 1024,
        },
    )))
}

/// Add a line to an import's log (after its hook is done).
fn append_to_log(log_path: &Path, line: &str) {
    if let Ok(mut log) = fs::OpenOptions::new().append(true).open(log_path) {
        let _ = writeln!(log, "{}", line);
    }
}

/// Why the imported try push `rev` has no semantic analysis, if its import's
/// failed (see the module's comment).
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct SemanticFailure {
    /// The step it failed in (the last one it started).
    pub step: String,
    /// Its error: its last "ERROR: " line after the step started (the hook's
    /// own messages), or else its last error line from what the step ran (ex:
    /// a Python exception's, or git's "error: " or "fatal: " lines).
    pub error: Option<String>,
    /// The files whose changes conflicted in the step, if it rebased the push
    /// (see scripts/rebase-try-push.sh).
    pub conflicts: Vec<String>,
    /// The commit whose changes conflicted, as its hash and summary, if the
    /// rebase said (it applies the push's commits one at a time).
    #[serde(default)]
    pub conflicting_commit: Option<String>,
    /// Whether the rebase found that all of the push's commits have landed
    /// (but those selecting try's tasks), so there's nothing to analyze.
    #[serde(default)]
    pub all_landed: bool,
}

/// Why the imported try push `rev` has no semantic analysis, from its
/// import's log, if its import tried one (see `SemanticFailure`).  (The caller
/// knows whether it has one; see `try_delta`.)
pub fn semantic_failure(cfg: &Config, tree_name: &str, rev: &str) -> Option<SemanticFailure> {
    parse_semantic_failure(&import_log(cfg, tree_name, rev)?)
}

/// Where the try push `rev`'s import's log is (see `TryImports::import`).
fn import_log_path(cfg: &Config, tree_name: &str, rev: &str) -> Option<PathBuf> {
    let history = cfg.trees.get(tree_name)?.paths.history_path.as_ref()?;
    Some(
        Path::new(history)
            .join("try-imports")
            .join(format!("{}.log", rev)),
    )
}

/// The imported try push `rev`'s import's log, if it's there.
fn import_log(cfg: &Config, tree_name: &str, rev: &str) -> Option<String> {
    let log = fs::read(import_log_path(cfg, tree_name, rev)?).ok()?;
    Some(String::from_utf8_lossy(&log).into_owned())
}

/// The commits an import's log says landed (see `LandedCommit`), without
/// their summaries.
fn parse_landed(log: &str) -> Vec<LandedCommit> {
    let mut landed: Vec<LandedCommit> = vec![];
    for line in log.lines() {
        let Some(rest) = line.strip_prefix("Landed: ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(rev), Some("as"), Some(as_rev)) = (words.next(), words.next(), words.next())
        else {
            continue;
        };
        if landed.iter().any(|known| known.rev == rev) {
            continue;
        }
        landed.push(LandedCommit {
            rev: rev.to_string(),
            landed: as_rev.to_string(),
            same_changes: rest.ends_with("(the same changes)"),
            summary: String::new(),
            rev_summary: String::new(),
        });
    }
    landed
}

/// `semantic_failure` for an import's log.
fn parse_semantic_failure(log: &str) -> Option<SemanticFailure> {
    const STEP: &str = "Performing semantic::";
    // (Without the shell's traces of the commands, ex: of the step's echo.)
    let lines: Vec<&str> = log.lines().filter(|line| !line.starts_with('+')).collect();
    let start = lines.iter().rposition(|line| line.starts_with(STEP))?;
    let step = lines[start][STEP.len()..]
        .split(" step")
        .next()
        .unwrap_or_default()
        .to_string();
    let after = &lines[start + 1..];
    let is_exception = |line: &str| {
        line.split_once(": ").is_some_and(|(name, _)| {
            !name.contains(char::is_whitespace)
                && (name.ends_with("Error") || name.ends_with("Exception"))
        })
    };
    let error = after
        .iter()
        .rev()
        .find_map(|line| line.strip_prefix("ERROR: "))
        .or_else(|| {
            after.iter().rev().copied().find(|line| {
                is_exception(line) || line.starts_with("error: ") || line.starts_with("fatal: ")
            })
        })
        .map(|error| error.trim().to_string());
    let mut conflicts: Vec<String> = vec![];
    let conflicting_commit = after
        .iter()
        .rev()
        .find_map(|line| line.strip_prefix("Conflicting commit: "))
        .map(|commit| commit.trim().to_string());
    let all_landed = after
        .iter()
        .any(|line| line.trim() == "All of the push's commits have landed.");
    for line in after {
        // (git apply's, for an hg push's commits, or a spec's pull request's
        // hook's, from build-spec-pr; see `spec_forest::pull_request_commits`.)
        let file = line
            .strip_prefix("Applied patch to '")
            .and_then(|rest| rest.strip_suffix("' with conflicts."))
            .or_else(|| {
                line.strip_prefix("error: ")
                    .and_then(|rest| rest.strip_suffix(": patch does not apply"))
            })
            .or_else(|| line.strip_prefix("Conflicting file: "));
        if let Some(file) = file
            && !conflicts.iter().any(|known| known == file)
        {
            conflicts.push(file.to_string());
        }
    }
    Some(SemanticFailure {
        step,
        error,
        conflicts,
        conflicting_commit,
        all_landed,
    })
}

/// The last `n` non-empty lines of a log (of its last 8 KiB).
fn last_lines(path: &Path, n: usize) -> String {
    let mut text = String::new();
    if let Ok(mut file) = fs::File::open(path) {
        let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        let _ = file.seek(SeekFrom::Start(len.saturating_sub(8192)));
        let mut bytes = vec![];
        let _ = file.read_to_end(&mut bytes);
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hook's log ends with what its processes used, theirs included (ex:
    /// awk's, run by the shell the hook runs), after its error's lines.
    #[test]
    fn test_run_hook_usage() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("try-import-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let hook = dir.join("hook");
        let log = dir.join("log");
        fs::write(
            &hook,
            "#!/bin/sh\n\
             echo started\n\
             sh -c 'awk \"BEGIN { s = sprintf(\\\"%80000000s\\\", \\\"\\\"); \
             for (i = 0; i < 2000000; i++) n += i }\"'\n\
             echo \"ERROR: it failed\"\n\
             exit \"$1\"\n",
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        for code in ["0", "1"] {
            let result = run_hook(&hook, &[code], &[], &log, TIME_LIMIT, &mut |_| {});
            let text = fs::read_to_string(&log).unwrap();
            let last = text.lines().last().unwrap();
            assert!(last.starts_with("try-import: the hook took "), "{}", last);
            let gb: f64 = last
                .rsplit(' ')
                .nth(1)
                .and_then(|gb| gb.parse().ok())
                .unwrap();
            assert!(gb >= 0.08, "awk's 80 MB string counts: {}", last);
            let cpu: f64 = last
                .split("using ")
                .nth(1)
                .and_then(|rest| rest.split(' ').next())
                .and_then(|cpu| cpu.parse().ok())
                .unwrap();
            assert!(cpu >= 0.0);
            match code {
                "0" => assert_eq!(result, Ok(())),
                _ => {
                    let error = result.unwrap_err();
                    assert!(error.ends_with("ERROR: it failed"), "{}", error);
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_can_start() {
        // (Spec pull requests' imports run beside each other, up to `WORKERS`.)
        assert!(can_start("dom/1", &[]));
        assert!(can_start("dom/1", &["dom/2", "html/3"]));
        let full: Vec<String> = (0..WORKERS).map(|i| format!("dom/{}", i + 2)).collect();
        let full: Vec<&str> = full.iter().map(String::as_str).collect();
        assert!(!can_start("dom/1", &full));
        // (Try pushes' imports run alone.)
        assert!(can_start("0123456789ab", &[]));
        assert!(!can_start("0123456789ab", &["dom/2"]));
        assert!(!can_start("dom/1", &["0123456789ab"]));
    }

    #[test]
    fn test_push_urls() {
        assert_eq!(push_url_path("abcdef012345"), "try/hg/abcdef012345");
        assert_eq!(push_url_path("dom/1512"), "try/dom/1512");
        assert_eq!(
            stack_push("try/hg/ABCDEF012345"),
            Some("abcdef012345".to_string())
        );
        assert_eq!(stack_push("try/dom/1512"), Some("dom/1512".to_string()));
        assert_eq!(stack_push("try/abcdef012345"), None);
        assert_eq!(stack_push("try/dom/01512"), None);
        assert_eq!(stack_push("try/hg/1512"), None);
        assert_eq!(stack_push("explore/bug/123"), None);
    }

    #[test]
    fn test_normalize_push() {
        assert_eq!(
            normalize_push("ABCDEF012345"),
            Some("abcdef012345".to_string())
        );
        assert_eq!(
            normalize_push("csswg-drafts/1234"),
            Some("csswg-drafts/1234".to_string())
        );
        assert!(is_pull_request("csswg-drafts/1234"));
        assert!(!is_pull_request("abcdef012345"));
        for bad in [
            "dom/0",
            "dom/1a",
            "dom/",
            "/12",
            "../12",
            "dom/../12",
            "hg/12",
            ".git/1",
            "dom/1/2",
            "dom/1234567890",
        ] {
            assert_eq!(normalize_push(bad), None, "{}", bad);
        }
    }

    #[test]
    fn test_normalize_rev() {
        assert_eq!(
            normalize_rev("ABCDEF012345"),
            Some("abcdef012345".to_string())
        );
        assert_eq!(normalize_rev(&"a".repeat(40)), Some("a".repeat(40)));
        assert_eq!(normalize_rev("abcdef01234"), None);
        assert_eq!(normalize_rev(&"a".repeat(41)), None);
        assert_eq!(normalize_rev("abcdef01234g"), None);
        assert_eq!(normalize_rev("../../etc/pw"), None);
    }

    #[test]
    fn test_importing_tree() {
        use std::os::unix::fs::PermissionsExt;
        // Trees a and b share a history, which b imports try pushes into (it
        // has a hook), and c has another, with no hook.
        let root = std::env::temp_dir().join(format!("try-import-trees-{}", std::process::id()));
        let repo = root.join("config");
        fs::create_dir_all(repo.join("b")).unwrap();
        let hook = repo.join("b").join("import-try");
        fs::write(&hook, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let tree = |name: &str, history: &str| {
            serde_json::json!({
                "priority": 1, "on_error": "halt", "cache": "nothing",
                "index_path": root.join(name).to_string_lossy(),
                "files_path": "/files", "objdir_path": "/objdir",
                "codesearch_path": "/livegrep.idx", "codesearch_port": 8081,
                "history_path": root.join(history).to_string_lossy(),
            })
        };
        let config = serde_json::json!({
            "mozsearch_path": "/mozsearch",
            "config_repo": repo.to_string_lossy(),
            "trees": {
                "a": tree("a", "shared-history"),
                "b": tree("b", "shared-history"),
                "c": tree("c", "c-history"),
            },
        });
        let config_path = root.join("config.json");
        fs::write(&config_path, config.to_string()).unwrap();
        let cfg = crate::file_format::config::load(
            config_path.to_str().unwrap(),
            false,
            None,
            None,
            None,
        );
        assert_eq!(importing_tree(&cfg, "a"), Some("b".to_string()));
        assert_eq!(importing_tree(&cfg, "b"), None);
        assert_eq!(importing_tree(&cfg, "c"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_parse_semantic_failure() {
        // (A tree's import without semantic analysis.)
        assert_eq!(parse_semantic_failure("Performing import step\nok\n"), None);
        // (A Python script's exception, after the traces of the step's echo
        // and of later commands.)
        let log = "+ echo 'Performing semantic::rebase step for x'\n\
                   Performing semantic::rebase step for x\n\
                   Performing semantic::build-analysis step for x : 2026\n\
                   + python3 select_tus.py\n\
                   Traceback (most recent call last):\n  \
                   File \"select_tus.py\", line 36, in <module>\n\
                   FileNotFoundError: [Errno 2] No such file or directory: 'cc.json'\n\
                   + echo 'WARNING: failed.'\n\
                   WARNING: failed.\n";
        assert_eq!(
            parse_semantic_failure(log),
            Some(SemanticFailure {
                step: "build-analysis".to_string(),
                error: Some(
                    "FileNotFoundError: [Errno 2] No such file or directory: 'cc.json'".to_string()
                ),
                conflicts: vec![],
                conflicting_commit: None,
                all_landed: false,
            })
        );
        // (A rebase's conflicts, with the hook's own message.)
        let log = "Performing semantic::rebase step for x\n\
                   Performing three-way merge...\n\
                   Applied patch to 'a.h' with conflicts.\n\
                   Applied patch to 'b.cpp' cleanly.\n\
                   error: c.py: patch does not apply\n\
                   ERROR: The push's changes conflict.\n";
        assert_eq!(
            parse_semantic_failure(log),
            Some(SemanticFailure {
                step: "rebase".to_string(),
                error: Some("The push's changes conflict.".to_string()),
                conflicts: vec!["a.h".to_string(), "c.py".to_string()],
                conflicting_commit: None,
                all_landed: false,
            })
        );
        // (A rebase leaving out landed commits, conflicting in another, and
        // one finding that all of them landed.)
        let log = "Performing semantic::rebase step for x\n\
                   Landed: aaa as bbb (the same changes)\n\
                   Landed: ccc as ddd (its Differential Revision)\n\
                   Applied patch to 'a.h' with conflicts.\n\
                   Conflicting commit: eee Bug 1 - Do a thing\n\
                   ERROR: The push's changes conflict.\n";
        let failure = parse_semantic_failure(log).unwrap();
        assert_eq!(failure.conflicts, vec!["a.h".to_string()]);
        assert_eq!(
            failure.conflicting_commit.as_deref(),
            Some("eee Bug 1 - Do a thing")
        );
        assert!(!failure.all_landed);
        let landed = parse_landed(log);
        assert_eq!(
            landed
                .iter()
                .map(|l| (l.rev.as_str(), l.landed.as_str(), l.same_changes))
                .collect::<Vec<_>>(),
            vec![("aaa", "bbb", true), ("ccc", "ddd", false)]
        );
        let log = "Performing semantic::rebase step for x\n\
                   Landed: aaa as bbb (the same changes)\n\
                   All of the push's commits have landed.\n\
                   ERROR: The push's changes conflict.\n";
        assert!(parse_semantic_failure(log).unwrap().all_landed);
    }

    #[test]
    fn test_last_lines() {
        let path = std::env::temp_dir().join(format!("try-import-log-{}", std::process::id()));
        fs::write(&path, "one\n\ntwo\nthree\n\n").unwrap();
        assert_eq!(last_lines(&path, 1), "three");
        assert_eq!(last_lines(&path, 2), "two\nthree");
        assert_eq!(last_lines(&path, 9), "one\ntwo\nthree");
        let _ = fs::remove_file(&path);
    }
}
