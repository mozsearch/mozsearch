use config;

use std::collections::BTreeMap;
use rustc_serialize::json::Json;
use regex::Regex;
use git2;

use chrono::naive::datetime::NaiveDateTime;
use chrono::offset::fixed::FixedOffset;
use chrono::datetime::DateTime;

fn linkify_commit_header(s: &str) -> String {
    lazy_static! {
        static ref BUG_NUMBER_REGEX: Regex = {
            Regex::new(r"\b(?P<bugno>[1-9][0-9]{4,9})\b").unwrap()
        };
        static ref SERVO_PR_REGEX: Regex = {
            Regex::new(r"#(?P<prno>[1-9][0-9]*)\b").unwrap()
        };
    }
    if s.starts_with("servo: ") {
        SERVO_PR_REGEX.replace_all(s, "#<a href=\"https://github.com/servo/servo/pull/$prno\">$prno</a>")
    } else {
        BUG_NUMBER_REGEX.replace_all(s, "<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=$bugno\">$bugno</a>")
    }
}

#[test]
fn test_linkify_servo_pr() {
    let linkified =
        linkify_commit_header("servo: Merge #1234 - stylo: Report a specific error for invalid CSS color values (from jdm:valueerr); r=heycam");
    assert!(linkified.contains("github.com"), "{:?}", linkified);
}

pub fn commit_header(commit: &git2::Commit) -> Result<(String, String), &'static str> {
    fn entity_replace(s: &str) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    let msg = try!(commit.message().ok_or("Invalid message"));
    let mut iter = msg.split('\n');
    let header = iter.next().unwrap();
    let remainder = iter.collect::<Vec<_>>().join("\n");
    let header = linkify_commit_header(&entity_replace(header));
    Ok((header, entity_replace(&remainder)))
}

pub fn get_commit_info(cfg: &config::Config, tree_name: &str, rev: &str) -> Result<String, &'static str> {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));
    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));
    let (msg, _) = try!(commit_header(&commit));

    let naive_t = NaiveDateTime::from_timestamp(commit.time().seconds(), 0);
    let tz = FixedOffset::east(commit.time().offset_minutes() * 60);
    let t : DateTime<FixedOffset> = DateTime::from_utc(naive_t, tz);
    let t = t.to_rfc2822();

    let sig = commit.author();

    let msg = format!("{}\n<br><i>{} &lt;{}>, {}</i>", msg, sig.name().unwrap(), sig.email().unwrap(), t);

    let mut obj = BTreeMap::new();

    obj.insert("header".to_owned(), Json::String(msg));

    let parents = commit.parent_ids().collect::<Vec<_>>();
    if parents.len() == 1 {
        obj.insert("parent".to_owned(), Json::String(parents[0].to_string()));
    }

    obj.insert("date".to_owned(), Json::String(t));

    let json = Json::Object(obj);

    Ok(json.to_string())
}
