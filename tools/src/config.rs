use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str;

use rustc_serialize::json::{self, Json};
use rustc_serialize::Decodable;

use git2::{Oid, Repository};

#[derive(RustcDecodable, RustcEncodable)]
pub struct TreeConfigPaths {
    pub index_path: String,
    pub files_path: String,
    pub git_path: Option<String>,
    pub git_blame_path: Option<String>,
    pub objdir_path: String,
    pub hg_root: Option<String>,
    pub dxr_root: Option<String>,
    pub ccov_root: Option<String>,
    pub github_repo: Option<String>,
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

pub struct Config {
    pub trees: BTreeMap<String, TreeConfig>,
    pub mozsearch_path: String,
}

pub fn get_git(tree_config: &TreeConfig) -> Result<&GitData, &'static str> {
    match &tree_config.git {
        &Some(ref git) => Ok(git),
        &None => Err("History data unavailable"),
    }
}

pub fn get_git_path(tree_config: &TreeConfig) -> Result<&str, &'static str> {
    match &tree_config.paths.git_path {
        &Some(ref git_path) => Ok(git_path),
        &None => Err("History data unavailable"),
    }
}

pub fn index_blame(_repo: &Repository, blame_repo: &Repository) -> (HashMap<Oid, Oid>, HashMap<Oid, String>) {
    let mut walk = blame_repo.revwalk().unwrap();
    walk.push_head().unwrap();

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

pub fn load(config_path: &str, need_indexes: bool) -> Config {
    let config_file = File::open(config_path).unwrap();
    let mut reader = BufReader::new(&config_file);
    let mut input = String::new();
    reader.read_to_string(&mut input).unwrap();
    let config = Json::from_str(&input).unwrap();

    let mut obj = config.as_object().unwrap().clone();

    let mozsearch_json = obj.remove("mozsearch_path").unwrap();
    let mozsearch = mozsearch_json.as_string().unwrap();

    let trees_obj = obj.get("trees").unwrap().as_object().unwrap().clone();

    let mut trees = BTreeMap::new();
    for (tree_name, tree_config) in trees_obj {
        let mut decoder = json::Decoder::new(tree_config);
        let paths = TreeConfigPaths::decode(&mut decoder).unwrap();

        let git = match (&paths.git_path, &paths.git_blame_path) {
            (&Some(ref git_path), &Some(ref git_blame_path)) => {
                let repo = Repository::open(&git_path).unwrap();
                let mailmap = Mailmap::load(&repo);
                let blame_ignore = BlameIgnoreList::load(&repo);

                let blame_repo = Repository::open(&git_blame_path).unwrap();
                let (blame_map, hg_map) = if need_indexes {
                    index_blame(&repo, &blame_repo)
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
            },
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
            },
            _ => None,
        };

        trees.insert(tree_name, TreeConfig {
            paths: paths,
            git: git,
        });
    }

    Config { trees: trees, mozsearch_path: mozsearch.to_owned() }
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
            )
        }

        // Try looking up only by email.
        key.0 = None;
        if let Some(&MailmapKey(ref new_name, ref new_email)) = self.entries.get(&key) {
            return (
                new_name.as_ref().map(String::as_str).unwrap_or(name),
                new_email.as_ref().map(String::as_str).unwrap_or(email),
            )
        }

        // Not in the mailmap, return it as-is.
        (name, email)
    }

    /// Load the Mailmap for the given repository.
    pub fn load(repo: &Repository) -> Self {
        // Repo may not have a mailmap file, in which case we can just generate
        // an empty one.
        Mailmap::try_load(repo).unwrap_or_else(|| Mailmap { entries: HashMap::new() })
    }

    fn parse_line(mut line: &str) -> Option<(MailmapKey, MailmapKey)> {
        fn nonempty(s: &str) -> Option<String> {
            if s.is_empty() { None } else { Some(s.to_owned()) }
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
