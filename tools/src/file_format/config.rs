use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::BufReader;
use std::io::Read;
use std::str;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use git2::{Oid, Repository};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeCaching {
    Everything,
    Codesearch,
    Nothing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeErrorHandling {
    /// Keep going, don't stop the indexing process.
    Continue,
    /// Generate an error and stop the indexing process.
    Halt,
}

/// Schema for the config.json files for loading; used to derive the actual
/// `Config` instance which also ends up including things like git info.
#[derive(Clone, Debug, Deserialize)]
pub struct ConfigJson {
    pub mozsearch_path: String,
    pub config_repo: String,
    /// What tree is the default for purposes of choosing which tree gets
    /// searched when viewing the root index page (which is derived from
    /// help.html).
    pub default_tree: Option<String>,
    /// What type of EC2 instance type to use for the web-server when it's spun
    /// up.
    pub instance_type: Option<String>,
    pub trees: BTreeMap<String, TreeConfigPaths>,

    #[serde(default)]
    pub allow_webtest: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeConfigPaths {
    /// Tree priority; higher numbers mean more important.  Used to control the
    /// order in which we apply the `cache` directive.
    pub priority: u32,
    pub on_error: TreeErrorHandling,
    pub cache: TreeCaching,
    /// Absolute path to the root of the tree's index, INDEX_ROOT.
    pub index_path: String,
    /// Absolute path to the root of the checked out source tree which should be
    /// a sub-directory of the `index_path`.
    pub files_path: String,
    /// Absolute path to the root of the shared directory for firefox trees like
    /// "firefox-main" and "firefox-beta" which share downloaded resources in
    /// "firefox-shared".  Exposed as SHARED_ROOT for scripting and to minimize
    /// path hard-coding.
    pub shared_path: Option<String>,
    /// Absolute path to where the `.git` sub-directory can be located; this
    /// should certainly be the same as `files_path`, and this will be a thing
    /// even if the canonical revision control system is mercurial.
    pub git_path: Option<String>,
    /// The git branch this tree is associated with, defaults to "HEAD" if not
    /// specified.  For trees like "firefox-main" that rely on a shared bare
    /// checkout for blame, an actual branch must be specified.
    pub git_branch: Option<String>,
    /// If this tree is replacing a previous tree, the name of that previous
    /// tree so that we can set up the redirects automatically in the web server
    /// config.  For example, for the "firefox-main" tree, the value would be
    /// "mozilla-central" which it replaces.
    pub oldtree_name: Option<String>,
    /// Absolute path to the "old" git checkout; currently this exists to
    /// support the mozilla hg conversion and this should be the old "gecko"
    /// git-cinnabar repo using the original gecko-dev hashes and where the
    /// mapping is via the shared hg revision associated with the revisions.
    pub oldgit_path: Option<String>,
    /// Absolute path to where the blame repo is which should be a sub-directory
    /// of the `index_path`.
    pub git_blame_path: Option<String>,
    /// Absolute path to where the history sub-tree lives; this should be a
    /// sub-directory of the `index_path`.
    pub history_path: Option<String>,
    /// Absolute path to where generated files can be found, and which will then
    /// be mapped into `"__GENERATED__"`.  This will usually be a sub-directory
    /// of the `index_path` but exceptions could be possible.
    pub objdir_path: String,
    /// List of the path prefixes where files may be missing at the point of
    /// gathering metadata.
    #[serde(default)]
    pub ignore_missing_path_prefixes: Vec<String>,
    /// If this is actually a mercurial repo, the URL of the hg server, no
    /// trailing `/`.
    pub hg_root: Option<String>,
    /// Coverage server URL.
    pub ccov_root: Option<String>,
    /// Relative path within the source tree that's really a WPT root.
    pub wpt_root: Option<String>,
    /// If this is actually a git repo hosted on github, its URL.  If the repo
    /// isn't github, we'll need to learn other URL mapping support.
    pub github_repo: Option<String>,
    /// For the oldgit repo, it it's a git repo hosted on github, its URL.  Note
    /// that for firefox-* because gecko-dev has stopped updating, some of the
    /// revisions will simply not exist there but we will know what they would
    /// be if they exist.
    pub oldgithub_repo: Option<String>,
    /// Absolute path to where we store the livegrep index.
    pub codesearch_path: String,
    /// Manually allocated port number to host the livegrep server on, starting
    /// from 8081 why not.
    pub codesearch_port: u32,
    /// Definitions of SCIP-based indexes to ingest.  Currently it's expected
    /// that the build script will handle downloading or generating the indexes.
    #[serde(default)]
    pub scip_subtrees: BTreeMap<String, ScipSubtreeConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScipSubtreeConfig {
    /// The path the SCIP index can be found at.
    pub scip_index_path: String,
    /// The tree-relative path where the files referenced by the index can be
    /// found.  For example, if there's a JS subtree that lives at
    /// "components/foo" and that's the root where the build script runs
    /// scip-typescript, then that's the value to put here, because the index
    /// will have paths relative to that directory.  (And while the index will
    /// have a "ProjectRoot", we want to handle the case where the SCIP index
    /// was generated on another machine.)
    ///
    /// Leave this empty if the subtree is actually at the root of the tree.
    pub subtree_root: String,
}

pub struct GitData {
    pub repo: Repository,
    pub blame_repo: Option<Repository>,

    pub blame_map: HashMap<Oid, Oid>, // Maps repo OID to blame_repo OID.
    // Maps repo OID to Hg rev.  This comes from our blame commits, but cinnabar
    // can also tell us this bidirectionally via `git2hg` and `hg2git`.
    pub hg_map: HashMap<Oid, String>,
    pub old_map: HashMap<Oid, Oid>, // Maps oldgit OID to repo OID.

    pub mailmap: Mailmap,
    /// Revs that we want to skip over during blame computation
    pub blame_ignore: BlameIgnoreList,
}

pub struct TreeConfig {
    pub paths: TreeConfigPaths,
    pub git: Option<GitData>,
}

impl TreeConfig {
    pub fn get_git(&self) -> Result<&GitData, &'static str> {
        match self.git {
            Some(ref git) => Ok(git),
            None => Err("History data unavailable"),
        }
    }

    pub fn get_git_path(&self) -> Result<&str, &'static str> {
        match self.paths.git_path {
            Some(ref git_path) => Ok(git_path),
            None => Err("History data unavailable"),
        }
    }

    pub fn find_source_file(&self, path: &str) -> String {
        if path.starts_with("__GENERATED__") {
            return path.replace("__GENERATED__", &self.paths.objdir_path);
        }
        format!("{}/{}", &self.paths.files_path, path)
    }

    pub fn should_ignore_missing_file(&self, path: &str) -> bool {
        for prefix in &self.paths.ignore_missing_path_prefixes {
            if path.starts_with(prefix) {
                return true;
            }
        }
        false
    }
}

pub struct Config {
    pub trees: BTreeMap<String, TreeConfig>,
    pub mozsearch_path: String,
    pub config_repo_path: String,
    // FIXME: Move these to TreeConfig.
    pub url_map_path: Option<String>,
    pub doc_trees_path: Option<String>,
}

impl Config {
    /// Synchronously read the contents of a file in the given tree's config
    /// directory, falling back to `MOZSEARCH/config_defaults/FILENAME` if
    /// available.
    pub fn read_tree_config_file_with_default(
        &self,
        filename: &str,
    ) -> Result<String, &'static str> {
        let repo_specific_path = format!("{}/{}", self.config_repo_path, filename);
        if let Ok(data_str) = std::fs::read_to_string(repo_specific_path) {
            return Ok(data_str);
        }
        let default_path = format!("{}/config_defaults/{}", self.mozsearch_path, filename);
        if let Ok(data_str) = std::fs::read_to_string(default_path) {
            return Ok(data_str);
        }
        Err("Unable to read the requested file")
    }

    /// Synchronously attempt to locate and read the contents of the given file
    /// at the given root using the given tree as context.  Documentation on the
    /// roots can be found on `SourceDescriptor`.
    pub fn maybe_read_file_from_given_root(
        &self,
        tree: &str,
        root: &str,
        file: &str,
    ) -> Result<Option<String>, &'static str> {
        let tree = self.trees.get(tree).unwrap();

        let path_root = match root {
            "config_repo" => &self.config_repo_path,
            "files" => &tree.paths.files_path,
            "index" => &tree.paths.index_path,
            "mozsearch" => &self.mozsearch_path,
            "objdir" => &tree.paths.objdir_path,
            _ => {
                return Err("invalid root specified");
            }
        };

        let full_path = format!("{}/{}", path_root, file);
        match fs::metadata(&full_path) {
            Ok(_) => match fs::read_to_string(full_path) {
                Ok(str) => Ok(Some(str)),
                // We should maybe convert to our server Result error or at least
                // dynamic strings, but for these static strings, let's have fun
                // with how useless this is!
                _ => Err("some kind of read error I guess"),
            },
            _ => Ok(None),
        }
    }
}

/// Ingest the provided blame_repo's provided ref (or HEAD if None), and
/// returning 3 maps:
/// 1. Map from source git rev to blame repo git rev
/// 2. Map from source git rev to hg repo git rev
/// 3. Map from old git rev to source git rev
///
/// If changing what we encode in the blame commit, you also need to change
/// the `extract_info_from_blame_commit` helper below.
pub fn index_blame(
    blame_repo: &Repository,
    head_ref: Option<Oid>,
) -> (HashMap<Oid, Oid>, HashMap<Oid, String>, HashMap<Oid, Oid>) {
    let mut walk = blame_repo.revwalk().unwrap();

    let mut blame_map = HashMap::new();
    let mut hg_map = HashMap::new();
    let mut oldrev_map = HashMap::new();

    if let Some(oid) = head_ref {
        // XXX This is speculative based on an attempt to generate the initial
        // firefox-main blame in an empty blame repo using the default branch
        // name ("main") when there were no commits yet.  I lost the log, but
        // I believe we had ended up inside this method, but letting us just
        // use HEAD had worked out okay, so I'm presuming this was the case.
        if let Err(_) = walk.push(oid) {
            return (blame_map, hg_map, oldrev_map);
        }
    } else {
        walk.push_head().unwrap();
    }

    for r in walk {
        let oid = r.unwrap();
        let commit = blame_repo.find_commit(oid).unwrap();

        let msg = commit.message().unwrap();
        let pieces = msg.split_whitespace().collect::<Vec<_>>();

        // "git <OID>" is always a given
        let orig_oid = Oid::from_str(pieces[1]).unwrap();
        blame_map.insert(orig_oid, commit.id());

        // "hg <REV>" may or may not be present.
        // "oldrevs <OID,OID,OID,...>" may or may not be present; this should
        // only be present if "hg" was already present but we can handle it not
        // being there.
        for (key, val) in pieces.iter().skip(2).tuples() {
            match *key {
                "hg" => {
                    let hg_id = val.to_string();
                    hg_map.insert(orig_oid, hg_id);
                }
                "oldrevs" => {
                    for oldrev in val.split(',') {
                        let oldrev_oid = Oid::from_str(oldrev).unwrap();
                        oldrev_map.insert(oldrev_oid, orig_oid);
                    }
                }
                _ => {}
            }
        }
    }

    (blame_map, hg_map, oldrev_map)
}

pub struct BlameCommitInfo {
    pub sourcerev: Oid,
    // we're horribly inconsistent about whether this is a string or an oid
    pub hgrev: Option<String>,
    pub oldrevs: Option<String>,
}

pub fn extract_info_from_blame_commit(commit: &git2::Commit) -> BlameCommitInfo {
    let msg = commit.message().unwrap();
    let pieces = msg.split_whitespace().collect::<Vec<_>>();

    // "git <OID>" is always a given
    let orig_oid = Oid::from_str(pieces[1]).unwrap();
    let mut hgrev = None;
    let mut oldrevs = None;

    // "hg <REV>" may or may not be present.
    // "oldrevs <OID,OID,OID,...>" may or may not be present; this should
    // only be present if "hg" was already present but we can handle it not
    // being there.
    for (key, val) in pieces.iter().skip(2).tuples() {
        match *key {
            "hg" => {
                hgrev = Some(val.to_string());
            }
            "oldrevs" => {
                oldrevs = Some(val.to_string());
            }
            _ => {}
        }
    }

    BlameCommitInfo {
        sourcerev: orig_oid,
        hgrev,
        oldrevs,
    }
}

pub fn load(
    config_path: &str,
    need_indexes: bool,
    only_tree: Option<&str>,
    url_map_path: Option<String>,
    doc_trees_path: Option<String>,
) -> Config {
    let config_file = File::open(config_path).unwrap();
    let mut reader = BufReader::new(&config_file);
    let mut input = String::new();
    reader.read_to_string(&mut input).unwrap();
    let config: ConfigJson = serde_json::from_str(&input).unwrap();

    let mut trees = BTreeMap::new();
    for (tree_name, paths) in config.trees {
        if let Some(only_tree_name) = only_tree {
            if tree_name != only_tree_name {
                continue;
            }
        }

        let git = match (&paths.git_path, &paths.git_blame_path) {
            (Some(git_path), Some(git_blame_path)) => {
                let repo = Repository::open(git_path).unwrap();
                let mailmap = Mailmap::load(&repo);
                let blame_ignore = BlameIgnoreList::load(&repo);

                let blame_repo = Repository::open(git_blame_path).unwrap();
                // The call to index_blame below explicitly knows to just use the head
                // if we pass None, which is why we aren't doing anything to default
                // git_branch to the literal string "HEAD".
                let blame_ref = match &paths.git_branch {
                    Some(branch_name) => Some(blame_repo.refname_to_id(&format!("refs/heads/{}", branch_name)).unwrap()),
                    None => None,
                };
                let (blame_map, hg_map, old_map) = if need_indexes {
                    index_blame(&blame_repo, blame_ref)
                } else {
                    (HashMap::new(), HashMap::new(), HashMap::new())
                };

                Some(GitData {
                    repo,
                    blame_repo: Some(blame_repo),
                    blame_map,
                    hg_map,
                    old_map,
                    mailmap,
                    blame_ignore,
                })
            }
            (Some(git_path), &None) => {
                let repo = Repository::open(git_path).unwrap();
                let mailmap = Mailmap::load(&repo);
                let blame_ignore = BlameIgnoreList::load(&repo);

                Some(GitData {
                    repo,
                    blame_repo: None,
                    blame_map: HashMap::new(),
                    hg_map: HashMap::new(),
                    old_map: HashMap::new(),
                    mailmap,
                    blame_ignore,
                })
            }
            _ => None,
        };

        trees.insert(tree_name, TreeConfig { paths, git });
    }

    Config {
        trees,
        mozsearch_path: config.mozsearch_path,
        config_repo_path: config.config_repo,
        url_map_path,
        doc_trees_path,
    }
}

#[derive(Hash, Eq, PartialEq, Debug)]
pub struct MailmapKey(Option<String>, Option<String>);

/// Mapping from names and emails to replace to the real names and emails for
/// these authors.
pub struct Mailmap {
    /// Map from old name and email to real name and email
    pub entries: HashMap<MailmapKey, MailmapKey>,
}

impl Mailmap {
    // Look up an entry in the mailmap, and return the real name and email.
    pub fn lookup<'a>(&'a self, name: &'a str, email: &'a str) -> (&'a str, &'a str) {
        // Unfortunately, we need to actually own our key strings due to type
        // matching & the keys being tuple-structs. I doubt this will have any
        // meaningful perf impact.

        // Try to look up with both name & email.
        let mut key = MailmapKey(Some(name.to_owned()), Some(email.to_owned()));
        if let Some(MailmapKey(new_name, new_email)) = self.entries.get(&key) {
            return (
                new_name.as_ref().map(String::as_str).unwrap_or(name),
                new_email.as_ref().map(String::as_str).unwrap_or(email),
            );
        }

        // Try looking up only by email.
        key.0 = None;
        if let Some(MailmapKey(new_name, new_email)) = self.entries.get(&key) {
            return (
                new_name.as_ref().map(String::as_str).unwrap_or(name),
                new_email.as_ref().map(String::as_str).unwrap_or(email),
            );
        }

        // Not in the mailmap, return it as-is.
        (name, email)
    }

    /// Load the Mailmap for the given repository.
    pub fn load(repo: &Repository) -> Self {
        // Repo may not have a mailmap file, in which case we can just generate
        // an empty one.
        Mailmap::try_load(repo).unwrap_or_else(|| Mailmap {
            entries: HashMap::new(),
        })
    }

    fn parse_line(mut line: &str) -> Option<(MailmapKey, MailmapKey)> {
        fn nonempty(s: &str) -> Option<String> {
            if s.is_empty() {
                None
            } else {
                Some(s.to_owned())
            }
        }

        // Remove text after a '#' comment from the line.
        line = line.split('#').next().unwrap();

        // name_a is the optional string before the first email.
        let idx = line.find('<')?;
        let name_a = nonempty(line[..idx].trim());
        line = &line[idx + 1..];

        // email_a is the required string until the end of the email block.
        let idx = line.find('>')?;
        let email_a = line[..idx].trim().to_owned();
        line = &line[idx + 1..];

        // name_b and email_b are optional. name_b requires email_b.
        let (name_b, email_b) = if let Some(idx) = line.find('<') {
            let name_b = nonempty(line[..idx].trim());
            line = &line[idx + 1..];

            let idx = line.find('>')?;
            let email_b = line[..idx].trim().to_owned();
            line = &line[idx + 1..];

            (name_b, Some(email_b))
        } else {
            (None, None)
        };

        // If we have junk at the end of the line - ignore it.
        if !line.trim().is_empty() {
            return None;
        }

        // Determine which format was being used, and build up our old and new
        // mailmap keys.
        let old;
        let new;
        if let Some(email_b) = email_b {
            new = MailmapKey(name_a, Some(email_a));
            old = MailmapKey(name_b, Some(email_b));
        } else {
            new = MailmapKey(name_a, None);
            old = MailmapKey(None, Some(email_a));
        }

        Some((old, new))
    }

    fn try_load(repo: &Repository) -> Option<Self> {
        // Get current mailmap from the repository.
        let obj = repo.revparse_single("HEAD:.mailmap").ok()?;
        let blob = obj.peel_to_blob().ok()?;
        let data = str::from_utf8(blob.content()).ok()?;

        // Parse each entry in turn
        let mut entries = HashMap::new();
        for line in data.lines() {
            if let Some((old, new)) = Mailmap::parse_line(line) {
                entries.insert(old, new);
            }
        }

        Some(Mailmap { entries })
    }
}

#[derive(Default)]
pub struct BlameIgnoreList {
    pub entries: HashSet<String>,
}

impl BlameIgnoreList {
    pub fn load(repo: &Repository) -> Self {
        // Produce an empty list if we fail to load anything
        BlameIgnoreList::try_load(repo).unwrap_or_default()
    }

    fn try_load(repo: &Repository) -> Option<Self> {
        let obj = repo.revparse_single("HEAD:.git-blame-ignore-revs").ok()?;
        let blob = obj.peel_to_blob().ok()?;
        let data = str::from_utf8(blob.content()).ok()?;

        let mut entries = HashSet::new();
        for line in data.lines() {
            let trimmed = line.split('#').next().unwrap().trim();
            // I guess we could also verify these are actually revisions but
            // that will eat CPU cycles for not much real benefit
            if !trimmed.is_empty() {
                entries.insert(trimmed.to_owned());
            }
        }

        Some(BlameIgnoreList { entries })
    }

    pub fn should_ignore(&self, rev: &str) -> bool {
        self.entries.contains(rev)
    }
}

impl GitData {
    pub fn should_ignore_for_blame(&self, rev: &str) -> bool {
        // TODO: we might want to pull the commit message and check for
        // special annotations like "#skip-blame" or backouts as well.
        // For now just check the list.
        self.blame_ignore.should_ignore(rev)
    }
}
