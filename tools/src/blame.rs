use crate::config;
use crate::links;

use git2;
use rustc_serialize::json::Json;
use std::borrow::Cow;
use std::collections::BTreeMap;

use chrono::datetime::DateTime;
use chrono::naive::datetime::NaiveDateTime;
use chrono::offset::fixed::FixedOffset;

pub fn commit_header(commit: &git2::Commit) -> Result<(String, String), &'static str> {
    fn entity_replace(s: &str) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    let msg = commit.message().ok_or("Invalid message")?;
    let mut iter = msg.split('\n');
    let header = iter.next().unwrap();
    let remainder = iter.collect::<Vec<_>>().join("\n");
    let header = links::linkify_commit_header(&entity_replace(header));
    Ok((header, entity_replace(&remainder)))
}

pub fn get_commit_info(
    cfg: &config::Config,
    tree_name: &str,
    revs: &str,
) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = config::get_git(tree_config)?;
    let mut infos = vec![];
    for rev in revs.split(',') {
        let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
        let commit = commit_obj.as_commit().ok_or("Bad revision")?;
        let (msg, _) = commit_header(&commit)?;

        let naive_t = NaiveDateTime::from_timestamp(commit.time().seconds(), 0);
        let tz = FixedOffset::east(commit.time().offset_minutes() * 60);
        let t: DateTime<FixedOffset> = DateTime::from_utc(naive_t, tz);
        let t = t.to_rfc2822();

        let sig = commit.author();
        let (name, email) = git
            .mailmap
            .lookup(sig.name().unwrap(), sig.email().unwrap());

        let msg = format!("{}\n<br><i>{} &lt;{}>, {}</i>", msg, name, email, t);

        let mut obj = BTreeMap::new();

        obj.insert("header".to_owned(), Json::String(msg));

        let parents = commit.parent_ids().collect::<Vec<_>>();
        if parents.len() == 1 {
            obj.insert("parent".to_owned(), Json::String(parents[0].to_string()));
        }

        obj.insert("date".to_owned(), Json::String(t));

        match (&tree_config.paths.hg_root, git.hg_map.get(&commit_obj.id())) {
            (Some(hg_path), Some(hg_id)) => {
                obj.insert(
                    "fulldiff".to_owned(),
                    Json::String(format!("{}/rev/{}", hg_path, hg_id)),
                );
            }
            _ => (),
        };

        infos.push(Json::Object(obj));
    }

    let json = Json::Array(infos);

    Ok(json.to_string())
}

#[derive(Debug)]
pub struct LineData<'a> {
    pub rev: Cow<'a, str>,
    pub path: Cow<'a, str>,
    pub lineno: Cow<'a, str>,
    pub author: Cow<'a, str>,
}

impl<'a> LineData<'a> {
    pub fn deserialize(line: &'a str) -> Self {
        let mut pieces = line.splitn(4, ':');
        let rev = pieces.next().unwrap();
        let path = pieces.next().unwrap();
        let lineno = pieces.next().unwrap();
        let author = pieces.next().unwrap();
        LineData {
            rev: Cow::Borrowed(rev),
            path: Cow::Borrowed(path),
            lineno: Cow::Borrowed(lineno),
            author: Cow::Borrowed(author),
        }
    }

    pub fn path_unchanged() -> Cow<'a, str> {
        Cow::Owned(String::from("%"))
    }

    pub fn is_path_unchanged(&self) -> bool {
        self.path == "%"
    }

    pub fn serialize(&self) -> String {
        format!("{}:{}:{}:{}", self.rev, self.path, self.lineno, self.author)
    }
}
