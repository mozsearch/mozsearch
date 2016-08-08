use config;

use std::collections::BTreeMap;
use rustc_serialize::json::Json;
use regex::Regex;
use git2;

pub fn linkify(s: &str) -> String {
    let re = Regex::new(r"\b(?P<bugno>[1-9][0-9]{4,9})\b").unwrap();
    re.replace_all(s, "<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=$bugno\">$bugno</a>")
}

pub fn commit_header(commit: &git2::Commit) -> Result<(String, String), &'static str> {
    fn entity_replace(s: &str) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    let msg = try!(commit.message().ok_or("Invalid message"));
    let mut iter = msg.split('\n');
    let header = iter.next().unwrap();
    let remainder = iter.collect::<Vec<_>>().join("\n");
    let header = linkify(&entity_replace(header));
    Ok((header, entity_replace(&remainder)))
}

pub fn get_commit_info(cfg: &config::Config, tree_name: &str, rev: &str) -> Result<String, &'static str> {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));
    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));
    let (msg, _) = try!(commit_header(&commit));

    let sig = commit.author();
    let msg = format!("{}\n<br><i>{} &lt;{}></i>", msg, sig.name().unwrap(), sig.email().unwrap());

    let mut obj = BTreeMap::new();

    obj.insert("header".to_owned(), Json::String(msg));

    let parents = commit.parent_ids().collect::<Vec<_>>();
    if parents.len() == 1 {
        obj.insert("parent".to_owned(), Json::String(parents[0].to_string()));
    }

    let json = Json::Object(obj);

    Ok(json.to_string())
}
