//! The "forest" repository of a tree of web specs (wubspecs): its top-level
//! directories are the specs' upstream repositories (ex: `html/` is
//! whatwg/html's tree, and `fetch/` is whatwg/fetch's), so that the history
//! tools, which assume one repository per tree, see one coherent history of
//! all of them (see `build-spec-forest`).
//!
//! - Each upstream's branch is fetched into the forest repository as
//!   `refs/upstream/<spec>`, so the forest has all of the objects itself.
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
    pub url: String,
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_branch() -> String {
    "main".to_string()
}

impl SpecConfig {
    /// The web page of an upstream commit, for GitHub repositories.
    pub fn commit_url(&self, commit: Oid) -> Option<String> {
        let repo = self.url.strip_prefix("https://github.com/")?;
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
}

impl ForestConfig {
    pub fn load(path: &Path) -> Result<ForestConfig, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Couldn't read {}: {}", path.display(), e))?;
        let config: ForestConfig =
            toml::from_str(&text).map_err(|e| format!("Bad {}: {}", path.display(), e))?;
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

/// Fetch each spec's upstream branch into `refs/upstream/<spec>`.
pub fn fetch_upstreams(forest: &Path, config: &ForestConfig) -> Result<(), String> {
    for spec in &config.specs {
        let refspec = format!("+refs/heads/{}:{}", spec.branch, upstream_ref(&spec.name));
        let status = Command::new("git")
            .arg("-C")
            .arg(forest)
            .args(["fetch", "--quiet", "--no-tags", &spec.url, &refspec])
            .status()
            .map_err(|e| format!("Couldn't run git fetch: {}", e))?;
        if !status.success() {
            return Err(format!("Couldn't fetch {} from {}", spec.name, spec.url));
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
        (repo, ids)
    }

    fn config(dir: &Path, names: &[&str], attributes: &str) -> ForestConfig {
        ForestConfig {
            attributes: attributes.to_string(),
            specs: names
                .iter()
                .map(|name| SpecConfig {
                    name: name.to_string(),
                    url: dir.join(name).to_str().unwrap().to_string(),
                    branch: "main".to_string(),
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
    fn test_trailer() {
        let spec = SpecConfig {
            name: "fetch".to_string(),
            url: "https://github.com/whatwg/fetch.git".to_string(),
            branch: "main".to_string(),
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
