use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, self};
use std::io::BufReader;
use std::io::Read;
use std::str;

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
    /// Absolute path to where the `.git` sub-directory can be located; this
    /// should certainly be the same as `files_path`, and this will be a thing
    /// even if the canonical revision control system is mercurial.
    pub git_path: Option<String>,
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
    pub hg_map: HashMap<Oid, String>, // Maps repo OID to Hg rev.

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
        match &self.git {
            &Some(ref git) => Ok(git),
            &None => Err("History data unavailable"),
        }
    }

    pub fn get_git_path(&self) -> Result<&str, &'static str> {
        match &self.paths.git_path {
            &Some(ref git_path) => Ok(git_path),
            &None => Err("History data unavailable"),
        }
    }

    pub fn find_source_file(&self, path: &str) -> String {
        if path.starts_with("__GENERATED__") {
            return path.replace("__GENERATED__", &self.paths.objdir_path);
        }
        format!("{}/{}", &self.paths.files_path, path)
    }

    pub fn should_ignore_missing_file(&self, path: &String) -> bool {
        for prefix in &self.paths.ignore_missing_path_prefixes {
            if path.starts_with(prefix) {
                return true;
            }
        }
        return false;
    }
}

pub struct Config {
    pub trees: BTreeMap<String, TreeConfig>,
    pub mozsearch_path: String,
    pub config_repo_path: String,
    pub url_map_path: Option<String>,
}

impl Config {
    /// Synchronously read the contents of a file in the given tree's config
    /// directory, falling back to `MOZSEARCH/config_defaults/FILENAME` if
    /// available.
    pub fn read_tree_config_file_with_default(&self, filename: &str) -> Result<String, &'static str> {
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
    pub fn maybe_read_file_from_given_root(&self, tree: &str, root: &str, file: &str) -> Result<Option<String>, &'static str> {
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
            }
            _ => Ok(None),
        }
    }
}

pub fn index_blame(
    blame_repo: &Repository,
    head_ref: Option<Oid>,
) -> (HashMap<Oid, Oid>, HashMap<Oid, String>) {
    let mut walk = blame_repo.revwalk().unwrap();
    if let Some(oid) = head_ref {
        walk.push(oid).unwrap();
    } else {
        walk.push_head().unwrap();
    }

    let mut blame_map = HashMap::new();
    let mut hg_map = HashMap::new();
    for r in walk {
        let oid = r.unwrap();
        let commit = blame_repo.find_commit(oid).unwrap();

        let msg = commit.message().unwrap();
        let pieces = msg.split_whitespace().collect::<Vec<_>>();

        let orig_oid = Oid::from_str(pieces[1]).unwrap();
        blame_map.insert(orig_oid, commit.id());

        if pieces.len() > 2 {
            let hg_id = pieces[3].to_owned();
            hg_map.insert(orig_oid, hg_id);
        }
    }

    (blame_map, hg_map)
}

pub fn load(config_path: &str, need_indexes: bool, only_tree: Option<&str>,
            url_map_path: Option<String>) -> Config {
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
            (&Some(ref git_path), &Some(ref git_blame_path)) => {
                let repo = Repository::open(&git_path).unwrap();
                let mailmap = Mailmap::load(&repo);
                let blame_ignore = BlameIgnoreList::load(&repo);

                let blame_repo = Repository::open(&git_blame_path).unwrap();
                let (blame_map, hg_map) = if need_indexes {
                    index_blame(&blame_repo, None)
                } else {
                    (HashMap::new(), HashMap::new())
                };

                Some(GitData {
                    repo: repo,
                    blame_repo: Some(blame_repo),
                    blame_map: blame_map,
                    hg_map: hg_map,
                    mailmap: mailmap,
                    blame_ignore: blame_ignore,
                })
            }
            (&Some(ref git_path), &None) => {
                let repo = Repository::open(&git_path).unwrap();
                let mailmap = Mailmap::load(&repo);
                let blame_ignore = BlameIgnoreList::load(&repo);

                Some(GitData {
                    repo: repo,
                    blame_repo: None,
                    blame_map: HashMap::new(),
                    hg_map: HashMap::new(),
                    mailmap: mailmap,
                    blame_ignore: blame_ignore,
                })
            }
            _ => None,
        };

        trees.insert(
            tree_name,
            TreeConfig {
                paths: paths,
                git: git,
            },
        );
    }

    Config {
        trees,
        mozsearch_path: config.mozsearch_path,
        config_repo_path: config.config_repo,
        url_map_path: url_map_path,
    }
}

#[derive(Hash, Eq, PartialEq, Debug)]
struct MailmapKey(Option<String>, Option<String>);

/// Mapping from names and emails to replace to the real names and emails for
/// these authors.
pub struct Mailmap {
    /// Map from old name and email to real name and email
    entries: HashMap<MailmapKey, MailmapKey>,
}

impl Mailmap {
    // Look up an entry in the mailmap, and return the real name and email.
    pub fn lookup<'a>(&'a self, name: &'a str, email: &'a str) -> (&'a str, &'a str) {
        // Unfortunately, we need to actually own our key strings due to type
        // matching & the keys being tuple-structs. I doubt this will have any
        // meaningful perf impact.

        // Try to look up with both name & email.
        let mut key = MailmapKey(Some(name.to_owned()), Some(email.to_owned()));
        if let Some(&MailmapKey(ref new_name, ref new_email)) = self.entries.get(&key) {
            return (
                new_name.as_ref().map(String::as_str).unwrap_or(name),
                new_email.as_ref().map(String::as_str).unwrap_or(email),
            );
        }

        // Try looking up only by email.
        key.0 = None;
        if let Some(&MailmapKey(ref new_name, ref new_email)) = self.entries.get(&key) {
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
    entries: HashSet<String>,
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
