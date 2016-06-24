use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::collections::BTreeMap;
use std::collections::HashMap;

use rustc_serialize::json::{self, Json};
use rustc_serialize::Decodable;

use git2::{Oid, Repository};

#[derive(RustcDecodable, RustcEncodable)]
pub struct TreeConfigPaths {
    pub index_path: String,
    pub repo_path: String,
    pub blame_repo_path: String,
    pub objdir_path: String,
}

pub struct TreeConfig {
    pub paths: TreeConfigPaths,
    pub repo: Repository,
    pub blame_repo: Repository,

    pub blame_map: HashMap<Oid, Oid>, // Maps repo OID to blame_repo OID.
    pub hg_map: HashMap<Oid, String>, // Maps repo OID to HG rev.
}

pub struct Config {
    pub trees: BTreeMap<String, TreeConfig>,
    pub mozsearch_path: String,
}

fn index_blame(_repo: &Repository, blame_repo: &Repository) -> (HashMap<Oid, Oid>, HashMap<Oid, String>) {
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

    let repos = obj.get("repos").unwrap().as_object().unwrap().clone();
    
    let mut trees = BTreeMap::new();
    for (tree_name, tree_config) in repos {
        let mut decoder = json::Decoder::new(tree_config);
        let paths = TreeConfigPaths::decode(&mut decoder).unwrap();

        let repo = Repository::open(&paths.repo_path).unwrap();
        let blame_repo = Repository::open(&paths.blame_repo_path).unwrap();

        let (blame_map, hg_map) = if need_indexes {
            index_blame(&repo, &blame_repo)
        } else {
            (HashMap::new(), HashMap::new())
        };

        trees.insert(tree_name, TreeConfig {
            paths: paths,
            repo: repo,
            blame_repo: blame_repo,

            blame_map: blame_map,
            hg_map: hg_map,
        });
    }

    Config { trees: trees, mozsearch_path: mozsearch.to_owned() }
}
