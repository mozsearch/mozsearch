use config;

use std::collections::BTreeMap;
use rustc_serialize::json::Json;
use regex::Regex;

pub fn linkify(s: &str) -> String {
    let re = Regex::new(r"\b(?P<bugno>[1-9][0-9]{4,9})\b").unwrap();
    re.replace_all(s, "<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=$bugno\">$bugno</a>")
}

pub fn get_commit_info(cfg: &config::Config, tree_name: &str, rev: &str) -> Result<String, &'static str> {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));
    let commit_obj = try!(tree_config.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));
    let msg = try!(commit.message().ok_or("Invalid message"));
    let msg = msg.split('\n').next().unwrap();
    let msg = linkify(msg);

    let sig = commit.author();
    let msg = format!("{}\n<br><i>{} &lt;{}></i>", msg, sig.name().unwrap(), sig.email().unwrap());

    let mut obj = BTreeMap::new();
    obj.insert("header".to_owned(), Json::String(msg));
    let json = Json::Object(obj);

    Ok(json.to_string())
}
