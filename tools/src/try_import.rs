//! The web server's imports of try pushes into the token-centric history and
//! commit index of the trees which can import them, for reviewing them
//! (`/{tree}/try/{rev}`; see `format::format_try`), whose views then have
//! them like the branch's revisions (see `TreeHistory`'s try notes, and the
//! commit index's other branches).  What's imported is what the history
//! derives (the token-centric blame, and the explore page's sparklines), not
//! semantic analysis, which needs an indexing of the push.
//!
//! A tree can import try pushes if it has a history and its directory in the
//! config repo has an executable `import-try` hook (like `upload-after-index`
//! and others).  The hook gets the push's revision as the user gave it (see
//! `normalize_rev`; ex: its hg revision on try) as its argument, and these
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
//! Imports run one at a time (the history repos and the commit index take one
//! writer at a time, and pushes based on the same new revisions would process
//! them twice at once), in the order they were asked for, on one thread, which
//! runs the hook at a lower priority than the server's (`nice`) and in its own
//! process group, so that it and what it runs can be killed after
//! `TIME_LIMIT`, with its output in `HISTORY_ROOT/try-imports/REV.log`.  The
//! queue only lives in memory: a restart of the server loses the waiting and
//! running imports, but asking again resumes them (the history tools skip the
//! revisions they've processed), and finished imports are in the repos.  At
//! most `MAX_QUEUED` imports wait at a time, and a failed import can only be
//! asked for again after `RETRY_AFTER`.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use git2::Oid;
use serde::Serialize;

use crate::file_format::config::Config;

/// The most imports which can be waiting at a time.
pub const MAX_QUEUED: usize = 8;
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

/// A try push's revision, as the hook takes it: an abbreviated or full
/// revision (12 to 40 hex digits, lowercased), if it's one.
pub fn normalize_rev(rev: &str) -> Option<String> {
    let rev = rev.trim();
    ((12..=40).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| rev.to_ascii_lowercase())
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

/// What's become of an import, for the try pages and their polling.
#[derive(Clone, Debug, PartialEq, Serialize)]
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
    running: Option<(Job, Instant, String)>,
    failed: HashMap<Job, (String, Instant)>,
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
            if let Some((running, started, step)) = &state.running
                && *running == job
            {
                return ImportStatus::Running {
                    seconds: started.elapsed().as_secs(),
                    step: step.clone(),
                };
            }
            if let Some(ahead) = state.queue.iter().position(|queued| *queued == job) {
                return ImportStatus::Queued {
                    ahead: ahead + state.running.is_some() as usize,
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

    /// Ask for the try push `rev` (see `normalize_rev`) to be imported into
    /// the tree, unless it is or will be, or it failed less than
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
            ImportStatus::Unavailable => return Err("This tree can't import try pushes"),
            _ => return Ok(status),
        }
        let job = Job {
            tree: tree_name.to_string(),
            rev: rev.to_string(),
        };
        {
            let mut state = self.state.lock().unwrap();
            if state.queue.len() >= MAX_QUEUED {
                return Err("Too many try pushes are waiting to be imported; try again later");
            }
            state.failed.remove(&job);
            state.queue.push_back(job);
        }
        self.queued.notify_one();
        Ok(self.status(cfg, tree_name, rev))
    }

    /// Perform the imports as they're asked for, forever.
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
            let result = self.import(cfg, &job);
            let mut state = self.state.lock().unwrap();
            state.running = None;
            if let Err(message) = result {
                eprintln!(
                    "Importing try push {} into {} failed: {}",
                    job.rev, job.tree, message
                );
                state.failed.insert(job, (message, Instant::now()));
            }
        }
    }

    /// Run the tree's hook for the job, until it's done or `TIME_LIMIT`.
    fn import(&self, cfg: &Config, job: &Job) -> Result<(), String> {
        let tree_config = cfg.trees.get(&job.tree).ok_or("No such tree")?;
        let hook = hook(cfg, &job.tree).ok_or("The tree can't import try pushes")?;
        let history_root = tree_config
            .paths
            .history_path
            .as_ref()
            .ok_or("The tree has no history")?;
        let logs = Path::new(history_root).join("try-imports");
        fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log_path = logs.join(format!("{}.log", job.rev));
        let log = fs::File::create(&log_path).map_err(|e| e.to_string())?;
        let mut child = Command::new("nice")
            .arg("-n10")
            .arg(&hook)
            .arg(&job.rev)
            .env("MOZSEARCH_PATH", &cfg.mozsearch_path)
            .env("CONFIG_REPO", &cfg.config_repo_path)
            .env("TREE_NAME", &job.tree)
            .env(
                "GIT_ROOT",
                tree_config.paths.git_path.as_deref().unwrap_or_default(),
            )
            .env("HISTORY_ROOT", history_root)
            .env(
                "GIT_BRANCH",
                tree_config.paths.git_branch.as_deref().unwrap_or("HEAD"),
            )
            .env("MAX_REVISIONS", MAX_REVISIONS.to_string())
            // (For the semantic analysis of the push; see `try_delta`.)
            .env("CONFIG_FILE", &cfg.config_file)
            .env("INDEX_ROOT", &tree_config.paths.index_path)
            .env("FILES_ROOT", &tree_config.paths.files_path)
            .env("OBJDIR", &tree_config.paths.objdir_path)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .process_group(0)
            .spawn()
            .map_err(|e| format!("Couldn't run {}: {}", hook.display(), e))?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if started.elapsed() > TIME_LIMIT {
                // (The hook's process group, with what it runs.)
                let _ = Command::new("kill")
                    .args(["-TERM", "--", &format!("-{}", child.id())])
                    .status();
                let _ = child.wait();
                return Err(format!(
                    "It took longer than {} minutes",
                    TIME_LIMIT.as_secs() / 60
                ));
            }
            let step = last_lines(&log_path, 1);
            if let Some((_, _, running)) = &mut self.state.lock().unwrap().running {
                *running = step;
            }
            std::thread::sleep(Duration::from_secs(1));
        };
        if !status.success() {
            return Err(last_lines(&log_path, 12));
        }
        if imported_head(cfg, &job.tree, &job.rev).is_none() {
            return Err("The import finished, but the history doesn't have the push".to_string());
        }
        Ok(())
    }
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
    fn test_last_lines() {
        let path = std::env::temp_dir().join(format!("try-import-log-{}", std::process::id()));
        fs::write(&path, "one\n\ntwo\nthree\n\n").unwrap();
        assert_eq!(last_lines(&path, 1), "three");
        assert_eq!(last_lines(&path, 2), "two\nthree");
        assert_eq!(last_lines(&path, 9), "one\ntwo\nthree");
        let _ = fs::remove_file(&path);
    }
}
