use crate::file_format::config::Config;
use crate::links;

use serde_json::{json, to_string, Map};
use std::borrow::Cow;
use std::str::Split;

use chrono::datetime::DateTime;
use chrono::naive::datetime::NaiveDateTime;
use chrono::offset::fixed::FixedOffset;

fn find_phab_rev(iter: Split<char>) -> Option<String> {
    const PREFIX: &str = "Differential Revision: ";

    for line in iter {
        if let Some(stripped) = line.strip_prefix(PREFIX) {
            return Some(stripped.to_string());
        }
    }

    None
}

enum CommitHeaderResultKind {
    HeaderOnly,
    HeaderRemainder,
    HeaderPatch,
}

fn commit_header_impl(
    commit: &git2::Commit,
    kind: CommitHeaderResultKind,
) -> Result<(String, Option<String>, Option<String>), &'static str> {
    fn entity_replace(s: &str) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    let msg = commit.message().ok_or("Invalid message")?;
    let mut iter = msg.split('\n');
    let header = iter.next().unwrap();
    let phab_rev = match kind {
        CommitHeaderResultKind::HeaderPatch => find_phab_rev(iter.clone()),
        _ => None,
    };
    let remainder = match kind {
        CommitHeaderResultKind::HeaderRemainder => {
            let raw = iter.collect::<Vec<_>>().join("\n");
            Some(entity_replace(&raw))
        }
        _ => None,
    };
    let header = links::linkify_commit_header(&entity_replace(header));
    Ok((header, remainder, phab_rev))
}

pub fn commit_header(commit: &git2::Commit) -> Result<String, &'static str> {
    commit_header_impl(commit, CommitHeaderResultKind::HeaderOnly).map(|x| x.0)
}

pub fn commit_header_remainder(commit: &git2::Commit) -> Result<(String, String), &'static str> {
    commit_header_impl(commit, CommitHeaderResultKind::HeaderRemainder).map(|x| (x.0, x.1.unwrap()))
}

pub fn commit_header_patch(
    commit: &git2::Commit,
) -> Result<(String, Option<String>), &'static str> {
    commit_header_impl(commit, CommitHeaderResultKind::HeaderPatch).map(|x| (x.0, x.2))
}

pub fn get_commit_info(cfg: &Config, tree_name: &str, revs: &str) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let mut infos = vec![];
    for rev in revs.split(',') {
        let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
        let commit = commit_obj.as_commit().ok_or("Bad revision")?;
        let (msg, phab_rev) = commit_header_patch(commit)?;

        let naive_t = NaiveDateTime::from_timestamp(commit.time().seconds(), 0);
        let tz = FixedOffset::east(commit.time().offset_minutes() * 60);
        let t: DateTime<FixedOffset> = DateTime::from_utc(naive_t, tz);
        let t = t.to_rfc2822();

        let sig = commit.author();
        let (name, email) = git
            .mailmap
            .lookup(sig.name().unwrap(), sig.email().unwrap());

        let msg = format!("{}\n<br><i>{} &lt;{}>, {}</i>", msg, name, email, t);

        let mut obj = Map::new();

        obj.insert("header".to_owned(), json!(msg));

        let parents = commit.parent_ids().collect::<Vec<_>>();
        if parents.len() == 1 {
            obj.insert("parent".to_owned(), json!(parents[0].to_string()));
        }

        obj.insert("date".to_owned(), json!(t));

        if let (Some(hg_path), Some(hg_id)) =
            (&tree_config.paths.hg_root, git.hg_map.get(&commit_obj.id()))
        {
            obj.insert(
                "fulldiff".to_owned(),
                json!(format!("{}/rev/{}", hg_path, hg_id)),
            );
        };

        if let Some(rev) = phab_rev {
            obj.insert("phab".to_owned(), json!(rev));
        }

        infos.push(json!(obj));
    }

    Ok(to_string(&json!(infos)).unwrap())
}

#[derive(Debug)]
pub struct LineData<'a> {
    pub rev: Cow<'a, str>,
    pub path: Cow<'a, str>,
    pub lineno: Cow<'a, str>,
}

impl<'a> LineData<'a> {
    pub fn deserialize(line: &'a str) -> Self {
        let mut pieces = line.splitn(4, ':');
        let rev = pieces.next().unwrap();
        let path = pieces.next().unwrap();
        let lineno = pieces.next().unwrap();
        LineData {
            rev: Cow::Borrowed(rev),
            path: Cow::Borrowed(path),
            lineno: Cow::Borrowed(lineno),
        }
    }

    pub fn path_unchanged() -> Cow<'a, str> {
        Cow::Owned(String::from("%"))
    }

    pub fn is_path_unchanged(&self) -> bool {
        self.path == "%"
    }

    pub fn serialize(&self) -> String {
        // The trailing colon delimits an empty "author" field
        // that was never used.
        format!("{}:{}:{}:", self.rev, self.path, self.lineno)
    }
}
