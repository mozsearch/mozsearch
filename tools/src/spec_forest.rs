//! The "forest" repository of a tree of web specs (wubspecs): its top-level
//! directories are the specs' upstream repositories (ex: `html/` is
//! whatwg/html's tree, and `fetch/` is whatwg/fetch's), so that the history
//! tools, which assume one repository per tree, see one coherent history of
//! all of them (see `build-spec-forest`).
//!
//! - Each upstream's branch (or its default branch) is fetched into the forest
//!   repository as `refs/upstream/<spec>`, so the forest has all of the
//!   objects itself.  (The configuration's specs without upstreams are specs
//!   in other specs' repositories, ex: csswg-drafts' many specs, which
//!   spec-analyze analyzes; see `spec_analyze::SpecsConfig`.)
//! - The forest has a commit for each upstream first-parent commit, whose root
//!   tree's `<spec>` entry is the upstream commit's own tree (so nothing is
//!   rewritten), with the upstream commit's author, committer, and message,
//!   and an `Upstream: <spec> <commit>` trailer.  Its other entries are the
//!   other specs' trees as of their latest commits in the forest, and a
//!   `.gitattributes` from the configuration (ex: the specs' sources'
//!   `searchfox-lang`s, since the HTML Standard's `source` has no extension).
//! - New upstream commits are appended in the order of their commit times,
//!   but keeping each upstream's own order (its commit times aren't always
//!   increasing along its first parents), so the forest's order is exact
//!   within a spec and approximate across specs.  Commits older than the
//!   forest's tip (ex: all of a newly configured spec's) are appended at the
//!   tip.
//! - `refs/forest/<spec>` is the latest upstream commit in the forest, and the
//!   `refs/notes/upstream` notes map upstream commits to their forest commits
//!   (see `git_notes`).  Specs which are no longer configured are removed from
//!   the next commit's tree.

use std::collections::{BinaryHeap, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use git2::{Oid, Repository, Signature};
use serde::Deserialize;

use crate::git_notes::NotesWriter;
use crate::git_ops::fast_import_git;

/// The forest's branch.
pub const FOREST_REF: &str = "refs/heads/main";
/// The notes mapping upstream commits to their forest commits.
pub const UPSTREAM_NOTES_REF: &str = "refs/notes/upstream";

/// The specs of a forest, ex: from a `specs.toml`.
#[derive(Debug, Deserialize)]
pub struct ForestConfig {
    /// The forest's root `.gitattributes`, if any.
    #[serde(default)]
    pub attributes: String,
    #[serde(rename = "spec", default)]
    pub specs: Vec<SpecConfig>,
}

#[derive(Debug, Deserialize)]
pub struct SpecConfig {
    /// The spec's directory in the forest, ex: "html".
    pub name: String,
    /// The upstream repository, ex: "https://github.com/whatwg/html.git".
    /// (`ForestConfig::load` leaves out the specs without one.)
    #[serde(default)]
    pub url: Option<String>,
    /// The upstream branch (default: the repository's default branch).
    #[serde(default)]
    pub branch: Option<String>,
}

impl SpecConfig {
    fn url(&self) -> &str {
        self.url.as_deref().unwrap_or_default()
    }

    /// The web page of an upstream commit, for GitHub repositories.
    pub fn commit_url(&self, commit: Oid) -> Option<String> {
        let repo = self.url().strip_prefix("https://github.com/")?;
        let repo = repo.trim_end_matches('/').trim_end_matches(".git");
        Some(format!("https://github.com/{}/commit/{}", repo, commit))
    }

    /// The `Upstream:` trailer of a forest commit for an upstream commit.
    fn trailer(&self, commit: Oid) -> String {
        match self.commit_url(commit) {
            Some(url) => format!("Upstream: {} {}", self.name, url),
            None => format!("Upstream: {} {}", self.name, commit),
        }
    }

    /// The web page of a pull request, for GitHub repositories.
    pub fn pull_request_url(&self, number: u32) -> Option<String> {
        let repo = self.url().strip_prefix("https://github.com/")?;
        let repo = repo.trim_end_matches('/').trim_end_matches(".git");
        Some(format!("https://github.com/{}/pull/{}", repo, number))
    }

    /// The `Pull-Request:` trailer of a pull request's forest commits.
    fn pull_request_trailer(&self, number: u32) -> String {
        match self.pull_request_url(number) {
            Some(url) => format!("Pull-Request: {} {}", self.name, url),
            None => format!("Pull-Request: {} {}", self.name, number),
        }
    }
}

impl ForestConfig {
    pub fn load(path: &Path) -> Result<ForestConfig, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Couldn't read {}: {}", path.display(), e))?;
        let mut config: ForestConfig =
            toml::from_str(&text).map_err(|e| format!("Bad {}: {}", path.display(), e))?;
        config.specs.retain(|spec| spec.url.is_some());
        let mut names = std::collections::HashSet::new();
        for spec in &config.specs {
            let valid = !spec.name.is_empty()
                && spec.name != ".gitattributes"
                && !spec.name.starts_with('.')
                && spec
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                return Err(format!("Bad spec name {:?}", spec.name));
            }
            if !names.insert(spec.name.as_str()) {
                return Err(format!("Spec {} is configured twice", spec.name));
            }
        }
        Ok(config)
    }
}

pub fn upstream_ref(name: &str) -> String {
    format!("refs/upstream/{}", name)
}

pub fn included_ref(name: &str) -> String {
    format!("refs/forest/{}", name)
}

/// The upstream spec and commit of a forest commit, from its trailer.
pub fn parse_upstream_trailer(message: &str) -> Option<(&str, Oid)> {
    let line = message
        .lines()
        .rev()
        .find(|line| line.starts_with("Upstream: "))?;
    let mut parts = line["Upstream: ".len()..].split_whitespace();
    let name = parts.next()?;
    let commit = parts.next()?.rsplit('/').next()?;
    Some((name, Oid::from_str(commit).ok()?))
}

/// Fetch each spec's upstream branch (or default branch) into
/// `refs/upstream/<spec>`.
pub fn fetch_upstreams(forest: &Path, config: &ForestConfig) -> Result<(), String> {
    for spec in &config.specs {
        let branch = match &spec.branch {
            Some(branch) => format!("refs/heads/{}", branch),
            None => "HEAD".to_string(),
        };
        let refspec = format!("+{}:{}", branch, upstream_ref(&spec.name));
        let status = Command::new("git")
            .arg("-C")
            .arg(forest)
            .args(["fetch", "--quiet", "--no-tags", spec.url(), &refspec])
            .status()
            .map_err(|e| format!("Couldn't run git fetch: {}", e))?;
        if !status.success() {
            return Err(format!("Couldn't fetch {} from {}", spec.name, spec.url()));
        }
    }
    Ok(())
}

/// A spec's first-parent commits after the latest one in the forest, oldest
/// first, with their commit times.
fn new_commits(repo: &Repository, spec: &SpecConfig) -> Result<Vec<(i64, Oid)>, String> {
    let tip = repo
        .refname_to_id(&upstream_ref(&spec.name))
        .map_err(|_| format!("{} hasn't been fetched (see --fetch)", spec.name))?;
    let included = repo.refname_to_id(&included_ref(&spec.name)).ok();
    let mut commits = vec![];
    let mut next = Some(repo.find_commit(tip).map_err(|e| e.to_string())?);
    while let Some(commit) = next {
        if Some(commit.id()) == included {
            commits.reverse();
            return Ok(commits);
        }
        commits.push((commit.time().seconds(), commit.id()));
        next = commit.parents().next();
    }
    match included {
        // (Rewriting the history of a spec's branch would need its forest
        // commits to be rebuilt.)
        Some(included) => Err(format!(
            "{}'s latest commit in the forest, {}, isn't a first-parent ancestor of its branch",
            spec.name, included
        )),
        None => {
            commits.reverse();
            Ok(commits)
        }
    }
}

/// Interleave the specs' new commits (in their own orders) by commit time:
/// repeatedly take the oldest of the specs' next commits (and for ties, the
/// first configured spec's).
fn interleave(per_spec: &[Vec<(i64, Oid)>]) -> Vec<(usize, Oid)> {
    // (A max-heap of reversed keys.)
    let mut heap = BinaryHeap::new();
    for (spec, commits) in per_spec.iter().enumerate() {
        if let Some((time, _)) = commits.first() {
            heap.push(std::cmp::Reverse((*time, spec, 0)));
        }
    }
    let mut out = vec![];
    while let Some(std::cmp::Reverse((_, spec, i))) = heap.pop() {
        out.push((spec, per_spec[spec][i].1));
        if let Some((time, _)) = per_spec[spec].get(i + 1) {
            heap.push(std::cmp::Reverse((*time, spec, i + 1)));
        }
    }
    out
}

/// A fast-import identity, without the characters it can't contain.
fn ident(signature: &Signature) -> String {
    let clean = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .chars()
            .filter(|c| !matches!(c, '<' | '>' | '\n'))
            .collect::<String>()
            .trim()
            .to_string()
    };
    let when = signature.when();
    let offset = when.offset_minutes();
    format!(
        "{} <{}> {} {}{:02}{:02}",
        clean(signature.name_bytes()),
        clean(signature.email_bytes()),
        when.seconds(),
        if offset < 0 { '-' } else { '+' },
        offset.abs() / 60,
        offset.abs() % 60
    )
}

/// What `update_forest` appended.
#[derive(Debug, Default)]
pub struct ForestUpdate {
    /// How many commits each spec added.
    pub appended: Vec<(String, usize)>,
    pub tip: Option<Oid>,
}

/// Append the specs' new upstream commits (as of their `refs/upstream/`
/// refs) to the forest's branch, and map them in the upstream notes.
pub fn update_forest(forest: &Path, config: &ForestConfig) -> Result<ForestUpdate, String> {
    let repo = Repository::open(forest).map_err(|e| e.to_string())?;
    let mut per_spec = vec![];
    for spec in &config.specs {
        per_spec.push(new_commits(&repo, spec)?);
    }
    let order = interleave(&per_spec);
    let old_tip = repo.refname_to_id(FOREST_REF).ok();
    let mut update = ForestUpdate {
        appended: config
            .specs
            .iter()
            .zip(&per_spec)
            .map(|(spec, commits)| (spec.name.clone(), commits.len()))
            .collect(),
        tip: old_tip,
    };
    if order.is_empty() {
        return Ok(update);
    }

    // The first new commit's other changes: the attributes, if they changed,
    // and the specs which are no longer configured.
    let mut first_changes = vec![];
    let old_tree = match old_tip {
        Some(tip) => Some(
            repo.find_commit(tip)
                .and_then(|commit| commit.tree())
                .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let old_attributes = old_tree
        .as_ref()
        .and_then(|tree| tree.get_name(".gitattributes"))
        .and_then(|entry| repo.find_blob(entry.id()).ok())
        .map(|blob| blob.content().to_vec());
    if old_attributes.as_deref() != Some(config.attributes.as_bytes()) {
        if config.attributes.is_empty() {
            first_changes.push("D .gitattributes\n".to_string());
        } else {
            first_changes.push(format!(
                "M 100644 inline .gitattributes\ndata {}\n{}\n",
                config.attributes.len(),
                config.attributes
            ));
        }
    }
    if let Some(tree) = &old_tree {
        for entry in tree.iter() {
            let Ok(name) = entry.name() else {
                continue;
            };
            if name != ".gitattributes" && !config.specs.iter().any(|s| s.name == name) {
                first_changes.push(format!("D {}\n", name));
            }
        }
    }

    let mut child = fast_import_git()
        .args(["fast-import", "--quiet", "--done"])
        .stdin(Stdio::piped())
        .current_dir(repo.path())
        .spawn()
        .map_err(|e| format!("Couldn't run git fast-import: {}", e))?;
    {
        let stream = child.stdin.as_mut().unwrap();
        let mut write = || -> std::io::Result<()> {
            for (i, (spec_index, upstream)) in order.iter().enumerate() {
                let spec = &config.specs[*spec_index];
                let commit = repo.find_commit(*upstream).map_err(std::io::Error::other)?;
                let mut message = commit.message_bytes().to_vec();
                while message.last() == Some(&b'\n') {
                    message.pop();
                }
                message.extend_from_slice(format!("\n\n{}\n", spec.trailer(*upstream)).as_bytes());
                writeln!(stream, "commit {}", FOREST_REF)?;
                writeln!(stream, "mark :{}", i + 1)?;
                writeln!(stream, "author {}", ident(&commit.author()))?;
                writeln!(stream, "committer {}", ident(&commit.committer()))?;
                writeln!(stream, "data {}", message.len())?;
                stream.write_all(&message)?;
                writeln!(stream)?;
                match (i, old_tip) {
                    (0, Some(tip)) => writeln!(stream, "from {}", tip)?,
                    (0, None) => {}
                    _ => writeln!(stream, "from :{}", i)?,
                }
                if i == 0 {
                    for change in &first_changes {
                        write!(stream, "{}", change)?;
                    }
                }
                writeln!(stream, "M 040000 {} {}", commit.tree_id(), spec.name)?;
                writeln!(stream)?;
            }
            writeln!(stream, "done")
        };
        write().map_err(|e| format!("Couldn't write to git fast-import: {}", e))?;
    }
    drop(child.stdin.take());
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("git fast-import failed".to_string());
    }

    // Map the new forest commits' upstream commits, and move the specs'
    // `refs/forest/` refs to their latest.
    let repo = Repository::open(forest).map_err(|e| e.to_string())?;
    let tip = repo.refname_to_id(FOREST_REF).map_err(|e| e.to_string())?;
    let mut notes = NotesWriter::new(
        &repo,
        UPSTREAM_NOTES_REF,
        "upstream commits to forest commits",
    );
    let mut latest: HashMap<String, Oid> = HashMap::new();
    let mut next = Some(repo.find_commit(tip).map_err(|e| e.to_string())?);
    let mut mapped = vec![];
    while let Some(commit) = next {
        if Some(commit.id()) == old_tip {
            break;
        }
        let message = String::from_utf8_lossy(commit.message_bytes()).into_owned();
        if let Some((name, upstream)) = parse_upstream_trailer(&message) {
            latest.entry(name.to_string()).or_insert(upstream);
            mapped.push((upstream, commit.id(), commit.time().seconds()));
        }
        next = commit.parents().next();
    }
    mapped.reverse();
    let mut child = fast_import_git()
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .current_dir(repo.path())
        .spawn()
        .map_err(|e| format!("Couldn't run git fast-import: {}", e))?;
    {
        let stream = child.stdin.as_mut().unwrap();
        for (upstream, forest_commit, time) in mapped {
            notes.add(upstream, &forest_commit.to_string(), time);
        }
        notes
            .flush(stream)
            .map_err(|e| format!("Couldn't write notes: {}", e))?;
    }
    drop(child.stdin.take());
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err("git fast-import failed (notes)".to_string());
    }
    for (name, upstream) in latest {
        repo.reference(&included_ref(&name), upstream, true, "build-spec-forest")
            .map_err(|e| e.to_string())?;
    }
    update.tip = Some(tip);
    Ok(update)
}

/// The forest commit of an upstream commit, if it's in the forest.
pub fn forest_commit(repo: &Repository, upstream: Oid) -> Option<Oid> {
    crate::git_notes::NotesReader::open(repo, [UPSTREAM_NOTES_REF]).lookup(repo, upstream)
}

// ## Pull requests
//
// A spec's pull request (GitHub's `refs/pull/<number>/head`) is imported as a
// try push (see `try_import`, and build-spec-pr): forest commits for its
// commits, on top of the forest commit of the trunk commit it's based on, and
// its head "rebased" onto the forest's tip (the tip, with the spec's tree
// merged with the pull request's), as try pushes are onto the indexed
// revision (see `try_delta`).

/// The most commits of a pull request which are imported.
pub const MAX_PULL_REQUEST_COMMITS: usize = 250;
/// How far back along the spec's trunk a pull request's base is looked for.
const MAX_BASE_SEARCH: usize = 5000;

/// The ref a spec's pull request's head is fetched into.
pub fn pull_request_ref(name: &str, number: u32) -> String {
    format!("refs/upstream-pr/{}/{}", name, number)
}

/// Fetch a spec's pull request's head into `pull_request_ref`.  (GitHub's
/// `refs/pull/<number>/head`, through git rather than its API.)  Without
/// writing FETCH_HEAD or running git's automatic maintenance, so that imports
/// can fetch beside each other and beside an import's steps writing to the
/// forest (whose objects and refs git takes concurrent writers of).
pub fn fetch_pull_request(forest: &Path, spec: &SpecConfig, number: u32) -> Result<(), String> {
    let refspec = format!(
        "+refs/pull/{}/head:{}",
        number,
        pull_request_ref(&spec.name, number)
    );
    let status = Command::new("git")
        .arg("-C")
        .arg(forest)
        .args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-auto-maintenance",
            spec.url(),
            &refspec,
        ])
        .status()
        .map_err(|e| format!("Couldn't run git fetch: {}", e))?;
    if !status.success() {
        return Err(format!(
            "Couldn't fetch pull request {} of {} from {}",
            number,
            spec.name,
            spec.url()
        ));
    }
    Ok(())
}

/// The forest commits made for a pull request (see `import_pull_request`).
#[derive(Debug)]
pub struct PullRequestCommits {
    /// The forest commit of the trunk commit it's based on.
    pub base: Oid,
    /// Its commits' forest commits, oldest first, the last being its head's.
    pub stack: Vec<Oid>,
    /// Its head rebased onto the forest's tip, or why it couldn't be (the
    /// files whose changes conflict with the trunk's since its base).
    pub rebased: Result<Oid, Vec<String>>,
}

/// Make the forest commits of a spec's pull request, as fetched (see
/// `fetch_pull_request`):
/// - Its base is the latest of the spec's trunk commits in the forest which
///   its head has (ex: the commit it branched from, or the latest one merged
///   into it).
/// - Its stack is its head's first parents since then, each a forest commit
///   whose `<spec>` entry is the commit's tree, on top of the base's forest
///   commit, with the commit's author, committer and message, and `Upstream:`
///   and `Pull-Request:` trailers.  (Its merges of newer trunk commits are
///   commits of the stack too.)  The same pull request makes the same commits.
/// - Its rebased head is the forest's tip with the spec's tree merged with the
///   head's (`git merge-tree`), if they merge without conflicts.
pub fn pull_request_commits(
    forest: &Path,
    spec: &SpecConfig,
    number: u32,
) -> Result<PullRequestCommits, String> {
    let repo = Repository::open(forest).map_err(|e| e.to_string())?;
    let err = |e: git2::Error| e.to_string();
    let head = repo
        .refname_to_id(&pull_request_ref(&spec.name, number))
        .map_err(|_| {
            format!(
                "Pull request {} of {} hasn't been fetched",
                number, spec.name
            )
        })?;
    let included = repo
        .refname_to_id(&included_ref(&spec.name))
        .map_err(|_| format!("{} isn't in the forest", spec.name))?;

    // The base: usually the merge base, if it's a trunk commit in the forest;
    // else the latest trunk commit the head has.
    let has = |ancestor: Oid| {
        ancestor == head || repo.graph_descendant_of(head, ancestor).unwrap_or(false)
    };
    let merge_base = repo.merge_base(head, included).ok();
    let base = match merge_base.filter(|mb| forest_commit(&repo, *mb).is_some()) {
        Some(base) => base,
        None => {
            let mut next = Some(repo.find_commit(included).map_err(err)?);
            let mut found = None;
            for _ in 0..MAX_BASE_SEARCH {
                let Some(commit) = next else {
                    break;
                };
                if has(commit.id()) {
                    found = Some(commit.id());
                    break;
                }
                next = commit.parents().next();
            }
            found.ok_or_else(|| {
                format!(
                    "Pull request {} of {} isn't based on any of {}'s commits in the tree",
                    number, spec.name, spec.name
                )
            })?
        }
    };
    let base_forest = forest_commit(&repo, base)
        .ok_or_else(|| format!("{}'s commit {} isn't in the forest", spec.name, base))?;

    // The stack's upstream commits: the head's first parents which the base
    // doesn't have.
    let mut upstream = vec![];
    let mut next = Some(repo.find_commit(head).map_err(err)?);
    while let Some(commit) = next {
        let id = commit.id();
        if id == base || repo.graph_descendant_of(base, id).unwrap_or(false) {
            break;
        }
        if upstream.len() == MAX_PULL_REQUEST_COMMITS {
            return Err(format!(
                "Pull request {} of {} has more than {} commits",
                number, spec.name, MAX_PULL_REQUEST_COMMITS
            ));
        }
        upstream.push(commit);
        next = upstream.last().unwrap().parents().next();
    }
    if upstream.is_empty() {
        return Err(format!(
            "Pull request {} of {} has no commits which the tree doesn't have",
            number, spec.name
        ));
    }
    upstream.reverse();

    let with_spec_tree = |parent: &git2::Commit, tree: Oid| -> Result<Oid, String> {
        let parent_tree = parent.tree().map_err(err)?;
        let mut builder = repo.treebuilder(Some(&parent_tree)).map_err(err)?;
        builder.insert(&spec.name, tree, 0o040000).map_err(err)?;
        builder.write().map_err(err)
    };
    let mut parent = repo.find_commit(base_forest).map_err(err)?;
    let mut stack = vec![];
    for commit in &upstream {
        let tree = repo
            .find_tree(with_spec_tree(&parent, commit.tree_id())?)
            .map_err(err)?;
        let mut message = String::from_utf8_lossy(commit.message_bytes()).into_owned();
        let trimmed = message.trim_end().len();
        message.truncate(trimmed);
        message.push_str(&format!(
            "\n\n{}\n{}\n",
            spec.trailer(commit.id()),
            spec.pull_request_trailer(number)
        ));
        let id = repo
            .commit(
                None,
                &commit.author(),
                &commit.committer(),
                &message,
                &tree,
                &[&parent],
            )
            .map_err(err)?;
        parent = repo.find_commit(id).map_err(err)?;
        stack.push(id);
    }

    // The head rebased onto the forest's tip.
    let output = Command::new("git")
        .arg("-C")
        .arg(forest)
        .args(["merge-tree", "--write-tree", "--name-only", "--no-messages"])
        .arg(included.to_string())
        .arg(head.to_string())
        .output()
        .map_err(|e| format!("Couldn't run git merge-tree: {}", e))?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut lines = text.lines();
    let merged = lines
        .next()
        .and_then(|line| Oid::from_str(line.trim()).ok());
    let rebased = match (output.status.code(), merged) {
        (Some(0), Some(merged)) => {
            let tip = repo
                .find_commit(repo.refname_to_id(FOREST_REF).map_err(err)?)
                .map_err(err)?;
            let tree = repo.find_tree(with_spec_tree(&tip, merged)?).map_err(err)?;
            let head_commit = repo.find_commit(head).map_err(err)?;
            let message = format!(
                "Pull request {} of {}, rebased onto the tree's revision\n\n{}\n{}\n",
                number,
                spec.name,
                spec.trailer(head),
                spec.pull_request_trailer(number)
            );
            Ok(repo
                .commit(
                    None,
                    &head_commit.author(),
                    &head_commit.committer(),
                    &message,
                    &tree,
                    &[&tip],
                )
                .map_err(err)?)
        }
        (Some(1), _) => Err(lines
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|file| format!("{}/{}", spec.name, file))
            .collect()),
        _ => {
            return Err(format!(
                "git merge-tree failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    };
    Ok(PullRequestCommits {
        base: base_forest,
        stack,
        rebased,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository with first-parent commits of `files` at `times`, and a
    /// side branch merged into the second commit (whose own commit is older
    /// than everything else).
    fn upstream(dir: &Path, name: &str, times: &[i64]) -> (Repository, Vec<Oid>) {
        let repo = Repository::init(dir.join(name)).unwrap();
        let mut ids: Vec<Oid> = vec![];
        let mut parent: Option<Oid> = None;
        for (i, time) in times.iter().enumerate() {
            let blob = repo
                .blob(format!("{} revision {}\n", name, i).as_bytes())
                .unwrap();
            let mut builder = repo.treebuilder(None).unwrap();
            builder.insert("spec.bs", blob, 0o100644).unwrap();
            let tree = repo.find_tree(builder.write().unwrap()).unwrap();
            let sig = Signature::new("A", "a@example.com", &git2::Time::new(*time, 60)).unwrap();
            let mut parents = vec![];
            if let Some(p) = parent {
                parents.push(repo.find_commit(p).unwrap());
            }
            if i == 1 {
                // (A merged side branch, which isn't a first parent.)
                let side_sig =
                    Signature::new("S", "s@example.com", &git2::Time::new(1, 0)).unwrap();
                let side = repo
                    .commit(None, &side_sig, &side_sig, "Side", &tree, &[])
                    .unwrap();
                parents.push(repo.find_commit(side).unwrap());
            }
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            let id = repo
                .commit(
                    Some("refs/heads/main"),
                    &sig,
                    &sig,
                    &format!("{} change {}\n", name, i),
                    &tree,
                    &parent_refs,
                )
                .unwrap();
            ids.push(id);
            parent = Some(id);
        }
        // (Its default branch, which the forest fetches.)
        repo.set_head("refs/heads/main").unwrap();
        (repo, ids)
    }

    fn config(dir: &Path, names: &[&str], attributes: &str) -> ForestConfig {
        ForestConfig {
            attributes: attributes.to_string(),
            specs: names
                .iter()
                .map(|name| SpecConfig {
                    name: name.to_string(),
                    url: Some(dir.join(name).to_str().unwrap().to_string()),
                    branch: None,
                })
                .collect(),
        }
    }

    /// The forest's commits, oldest first: (upstream spec, upstream commit,
    /// root tree entries).
    fn forest_log(forest: &Repository) -> Vec<(String, Oid, Vec<String>)> {
        let mut out = vec![];
        let mut next = Some(
            forest
                .find_commit(forest.refname_to_id(FOREST_REF).unwrap())
                .unwrap(),
        );
        while let Some(commit) = next {
            let message = commit.message().unwrap().to_string();
            let (name, upstream) = parse_upstream_trailer(&message).unwrap();
            let entries = commit
                .tree()
                .unwrap()
                .iter()
                .map(|e| e.name().unwrap().to_string())
                .collect();
            out.push((name.to_string(), upstream, entries));
            next = commit.parents().next();
        }
        out.reverse();
        out
    }

    #[test]
    fn test_forest() {
        let dir = std::env::temp_dir().join(format!("spec-forest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // (dom's second commit is older than its first, as after a rebase.)
        let (_, html) = upstream(&dir, "html", &[100, 300, 500]);
        let (dom_repo, dom) = upstream(&dir, "dom", &[200, 150, 400]);
        let forest_dir = dir.join("forest");
        Repository::init(&forest_dir).unwrap();
        let config1 = config(
            &dir,
            &["html", "dom"],
            "html/source searchfox-lang=spec-html\n",
        );
        fetch_upstreams(&forest_dir, &config1).unwrap();
        let update = update_forest(&forest_dir, &config1).unwrap();
        assert_eq!(
            update.appended,
            [("html".to_string(), 3), ("dom".to_string(), 3)]
        );

        let forest = Repository::open(&forest_dir).unwrap();
        let log = forest_log(&forest);
        let order: Vec<(String, Oid)> = log.iter().map(|(n, u, _)| (n.clone(), *u)).collect();
        // By time, but dom in its own order (its 150 after its 200).
        assert_eq!(
            order,
            [
                ("html".to_string(), html[0]),
                ("dom".to_string(), dom[0]),
                ("dom".to_string(), dom[1]),
                ("html".to_string(), html[1]),
                ("dom".to_string(), dom[2]),
                ("html".to_string(), html[2]),
            ]
        );
        assert_eq!(log[0].2, [".gitattributes", "html"]);
        assert_eq!(log[5].2, [".gitattributes", "dom", "html"]);
        // The specs' trees are the upstream commits' own, and the commits
        // keep their authors and messages.
        let tip = forest.find_commit(update.tip.unwrap()).unwrap();
        let dom_tip = forest.find_commit(dom[2]).unwrap();
        assert_eq!(
            tip.tree().unwrap().get_name("dom").unwrap().id(),
            dom_tip.tree_id()
        );
        assert_eq!(tip.author().name().ok(), Some("A"));
        assert_eq!(tip.author().when().offset_minutes(), 60);
        assert!(
            tip.message()
                .unwrap()
                .starts_with("html change 2\n\nUpstream: html ")
        );
        // The notes map upstream commits to forest commits.
        assert_eq!(forest_commit(&forest, html[2]), Some(tip.id()));
        assert_eq!(
            forest_commit(&forest, dom[1]),
            Some(
                tip.parent(0)
                    .unwrap()
                    .parent(0)
                    .unwrap()
                    .parent(0)
                    .unwrap()
                    .id()
            )
        );

        // Updates append new commits, even older ones, and drop specs which
        // are no longer configured.
        let sig = Signature::new("A", "a@example.com", &git2::Time::new(50, 0)).unwrap();
        let parent = dom_repo.find_commit(dom[2]).unwrap();
        let tree = parent.tree().unwrap();
        let dom3 = dom_repo
            .commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "dom change 3",
                &tree,
                &[&parent],
            )
            .unwrap();
        let config2 = config(&dir, &["dom"], "");
        fetch_upstreams(&forest_dir, &config2).unwrap();
        let update = update_forest(&forest_dir, &config2).unwrap();
        assert_eq!(update.appended, [("dom".to_string(), 1)]);
        let log = forest_log(&forest);
        assert_eq!(log.len(), 7);
        assert_eq!(log[6], ("dom".to_string(), dom3, vec!["dom".to_string()]));
        assert_eq!(forest.refname_to_id(&included_ref("dom")).unwrap(), dom3);
        // (With nothing new, nothing changes.)
        let update = update_forest(&forest_dir, &config2).unwrap();
        assert_eq!(update.appended, [("dom".to_string(), 0)]);
        assert_eq!(forest_log(&forest).len(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_leaves_out_specs_in_other_repositories() {
        let path = std::env::temp_dir().join(format!("specs-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[spec]]\nname = \"csswg-drafts\"\nurl = \"https://github.com/w3c/csswg-drafts.git\"\n\n\
             [[spec]]\nname = \"css-grid-2\"\npath = \"csswg-drafts/css-grid-2\"\nformat = \"bikeshed\"\n",
        )
        .unwrap();
        let config = ForestConfig::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let names: Vec<&str> = config.specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["csswg-drafts"]);
    }

    /// A commit of `files` (name, contents) on top of `parents`, at `time`.
    fn commit_files(
        repo: &Repository,
        files: &[(&str, &str)],
        parents: &[Oid],
        time: i64,
        message: &str,
    ) -> Oid {
        let mut builder = repo.treebuilder(None).unwrap();
        for (name, contents) in files {
            let blob = repo.blob(contents.as_bytes()).unwrap();
            builder.insert(name, blob, 0o100644).unwrap();
        }
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = Signature::new("P", "p@example.com", &git2::Time::new(time, 0)).unwrap();
        let parents: Vec<git2::Commit> = parents
            .iter()
            .map(|p| repo.find_commit(*p).unwrap())
            .collect();
        let refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(None, &sig, &sig, message, &tree, &refs)
            .unwrap()
    }

    #[test]
    fn test_pull_requests() {
        let dir = std::env::temp_dir().join(format!("spec-forest-prs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The trunk: c0, then c1 changing a.bs; pull request 1 changes b.bs
        // from c0 in two commits (which merge cleanly with c1), and pull
        // request 2 changes a.bs from c0 (which conflicts with c1).
        let up = Repository::init(dir.join("spec")).unwrap();
        let c0 = commit_files(
            &up,
            &[("a.bs", "a0\n"), ("b.bs", "b0\n")],
            &[],
            100,
            "Start",
        );
        let c1 = commit_files(
            &up,
            &[("a.bs", "a1\n"), ("b.bs", "b0\n")],
            &[c0],
            200,
            "Change a",
        );
        up.reference("refs/heads/main", c1, true, "").unwrap();
        up.set_head("refs/heads/main").unwrap();
        let p1 = commit_files(
            &up,
            &[("a.bs", "a0\n"), ("b.bs", "b1\n")],
            &[c0],
            300,
            "Change b",
        );
        let p2 = commit_files(
            &up,
            &[("a.bs", "a0\n"), ("b.bs", "b2\n")],
            &[p1],
            400,
            "Change b again",
        );
        up.reference("refs/pull/1/head", p2, true, "").unwrap();
        let q1 = commit_files(
            &up,
            &[("a.bs", "aq\n"), ("b.bs", "b0\n")],
            &[c0],
            500,
            "Change a too",
        );
        up.reference("refs/pull/2/head", q1, true, "").unwrap();

        let forest_dir = dir.join("forest");
        Repository::init(&forest_dir).unwrap();
        let config = config(&dir, &["spec"], "");
        fetch_upstreams(&forest_dir, &config).unwrap();
        update_forest(&forest_dir, &config).unwrap();
        let forest = Repository::open(&forest_dir).unwrap();
        let spec = &config.specs[0];

        fetch_pull_request(&forest_dir, spec, 1).unwrap();
        let pr = pull_request_commits(&forest_dir, spec, 1).unwrap();
        assert_eq!(
            pr.base,
            forest_commit(&forest, c0).unwrap(),
            "Based on c0's forest commit"
        );
        assert_eq!(pr.stack.len(), 2);
        let head = forest.find_commit(pr.stack[1]).unwrap();
        assert_eq!(head.parent_id(0).unwrap(), pr.stack[0]);
        assert_eq!(
            forest
                .find_commit(pr.stack[0])
                .unwrap()
                .parent_id(0)
                .unwrap(),
            pr.base
        );
        let spec_tree =
            |commit: &git2::Commit| commit.tree().unwrap().get_name("spec").unwrap().id();
        assert_eq!(
            spec_tree(&head),
            up.find_commit(p2).unwrap().tree_id(),
            "with the commits' own trees"
        );
        let message = head.message().unwrap();
        assert!(
            message.starts_with("Change b again\n\nUpstream: spec "),
            "{}",
            message
        );
        assert!(message.contains("\nPull-Request: spec 1\n"), "{}", message);
        assert_eq!(parse_upstream_trailer(message), Some(("spec", p2)));
        assert_eq!(head.author().name().ok(), Some("P"));
        // (The same pull request makes the same commits.)
        assert_eq!(
            pull_request_commits(&forest_dir, spec, 1).unwrap().stack,
            pr.stack
        );
        // Rebased onto the tip: c1's a.bs, and the pull request's b.bs.
        let rebased = forest.find_commit(pr.rebased.unwrap()).unwrap();
        assert_eq!(
            rebased.parent_id(0).unwrap(),
            forest.refname_to_id(FOREST_REF).unwrap()
        );
        let tree = forest.find_tree(spec_tree(&rebased)).unwrap();
        let file = |name: &str| {
            String::from_utf8(
                forest
                    .find_blob(tree.get_name(name).unwrap().id())
                    .unwrap()
                    .content()
                    .to_vec(),
            )
            .unwrap()
        };
        assert_eq!(
            (file("a.bs").as_str(), file("b.bs").as_str()),
            ("a1\n", "b2\n")
        );

        // A conflicting one has no rebased head, but says which files.
        fetch_pull_request(&forest_dir, spec, 2).unwrap();
        let pr = pull_request_commits(&forest_dir, spec, 2).unwrap();
        assert_eq!(pr.stack.len(), 1);
        assert_eq!(pr.rebased, Err(vec!["spec/a.bs".to_string()]));

        // One based on a trunk commit newer than the forest's has it in its
        // stack, on top of the latest one the forest has.
        let c2 = commit_files(
            &up,
            &[("a.bs", "a1\n"), ("b.bs", "b0\n"), ("c.bs", "c\n")],
            &[c1],
            600,
            "Add c",
        );
        let r1 = commit_files(
            &up,
            &[("a.bs", "a1\n"), ("b.bs", "b3\n"), ("c.bs", "c\n")],
            &[c2],
            700,
            "Change b from c2",
        );
        up.reference("refs/pull/3/head", r1, true, "").unwrap();
        fetch_pull_request(&forest_dir, spec, 3).unwrap();
        let pr = pull_request_commits(&forest_dir, spec, 3).unwrap();
        assert_eq!(pr.base, forest_commit(&forest, c1).unwrap());
        assert_eq!(pr.stack.len(), 2);
        assert!(pr.rebased.is_ok());

        // One with nothing new is an error.
        up.reference("refs/pull/4/head", c0, true, "").unwrap();
        fetch_pull_request(&forest_dir, spec, 4).unwrap();
        assert!(pull_request_commits(&forest_dir, spec, 4).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_trailer() {
        let spec = SpecConfig {
            name: "fetch".to_string(),
            url: Some("https://github.com/whatwg/fetch.git".to_string()),
            branch: Some("main".to_string()),
        };
        let commit = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let trailer = spec.trailer(commit);
        assert_eq!(
            trailer,
            "Upstream: fetch https://github.com/whatwg/fetch/commit/0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(
            parse_upstream_trailer(&format!("Fix it.\n\n{}\n", trailer)),
            Some(("fetch", commit))
        );
    }
}
