//! Counting tokens' tracked instances (see `timeline_tokens::tracked_token_keys`)
//! in a revision's syntax files, as token journals count them, for merges'
//! correction records (see build-timeline-tree's "Merges"), and for checking
//! that a token journal's records sum to the instances there are (the
//! `check-token-counts` tool), which `token_timeline` draws from.

use std::collections::HashMap;
use std::path::Path;

use git2::{Repository, Tree, TreeWalkMode, TreeWalkResult};

use crate::file_format::history::io_helpers::read_record_file_contents;
use crate::file_format::history::syntax_files::{split_token_line, token_file_lines};
use crate::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use crate::file_format::history::timeline_common::TokenDeltaDetails;
use crate::file_format::history::timeline_tokens::tracked_token_keys;
use crate::tree_sitter_support::cst_tokenizer::namespace_for_file;

/// The namespace of the syntax file of `path` (its files-struct header's, for
/// language overrides, else the path's default), as build-timeline-tree's
/// `struct_namespace` has it.
pub fn file_namespace(repo: &Repository, root: &Tree, path: &str) -> String {
    let header = root
        .get_path(&Path::new("files-struct").join(path))
        .ok()
        .and_then(|entry| repo.find_blob(entry.id()).ok())
        .and_then(|blob| {
            read_record_file_contents::<FileStructureHeader, FileStructureRow>(blob.content())
        })
        .map(|(header, _)| header);
    header
        .as_ref()
        .and_then(|header| header.effective_namespace().map(str::to_string))
        .unwrap_or_else(|| namespace_for_file(Path::new(path)).to_string())
}

/// Add `sign` times the tracked instances of each token in a syntax file's
/// `contents` (in `namespace`) to `counts`.
pub fn add_tracked_counts(
    namespace: &str,
    contents: &str,
    sign: i64,
    counts: &mut HashMap<String, i64>,
) {
    let lines = token_file_lines(contents);
    let mut prev = None;
    for line in &lines {
        let line = split_token_line(line);
        tracked_token_keys(namespace, &line, prev.as_ref(), &mut |key| {
            *counts.entry(key.into_owned()).or_default() += sign;
        });
        prev = Some(line);
    }
}

/// The tracked instances of each token in all of the files of a syntax repo
/// commit's tree.
pub fn tree_counts(repo: &Repository, root: &Tree) -> Result<HashMap<String, i64>, git2::Error> {
    let mut counts = HashMap::new();
    let Ok(files) = root.get_path(Path::new("files")) else {
        return Ok(counts);
    };
    let files = repo.find_tree(files.id())?;
    let mut paths = vec![];
    files.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            paths.push((format!("{}{}", dir, entry.name().unwrap_or("")), entry.id()));
        }
        TreeWalkResult::Ok
    })?;
    for (path, oid) in paths {
        let blob = repo.find_blob(oid)?;
        let Ok(contents) = std::str::from_utf8(blob.content()) else {
            continue;
        };
        let namespace = file_namespace(repo, root, &path);
        add_tracked_counts(&namespace, contents, 1, &mut counts);
    }
    Ok(counts)
}

/// A count field of a record line (ex: `"added":3`), or 0.  (Faster than
/// parsing the line, for merges' sums of big journals.)
fn line_field(line: &str, key: &str) -> i64 {
    let Some(at) = line.find(key) else {
        return 0;
    };
    let digits = &line[at + key.len()..];
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse().unwrap_or(0)
}

/// The change a token journal record line makes to its instances: those added
/// and evolved from other tokens, less those removed and evolved into others
/// (moves don't change them).
pub fn record_line_change(line: &str) -> i64 {
    line_field(line, "\"added\":") + line_field(line, "\"evolved_from\":")
        - line_field(line, "\"removed\":")
        - line_field(line, "\"evolved_into\":")
}

/// The sum of a token journal's records' changes (see `record_line_change`),
/// from its text (its head and segments, joined; see `segments::join`).
pub fn journal_sum(text: &str) -> i64 {
    text.lines().skip(1).map(record_line_change).sum()
}

/// A revision's file, for `exact_totals`: its old and new syntax contents,
/// with their namespaces, if they count (a copy's source, which stays, has no
/// old side).
pub struct CountedFile<'a> {
    pub old: Option<(&'a str, &'a str)>,
    pub new: Option<(&'a str, &'a str)>,
}

/// Make each token's changes in `totals` change its instances (see
/// `record_line_change`) by as much as `files`' old and new versions' counts
/// do, by putting the difference in `added` or `removed`: pairing removed and
/// added tokens (moves, evolutions) leaves the inference's net change off by a
/// token copied (ex: into a copied file, whose source stays) or whose key
/// depends on the token before it (a bug number), and token journals' sums
/// must be their tokens' instances (see `token_timeline`).
pub fn exact_totals(
    totals: &mut std::collections::BTreeMap<String, TokenDeltaDetails>,
    files: &[CountedFile],
) {
    let mut counts: HashMap<String, i64> = HashMap::new();
    for file in files {
        if let Some((namespace, contents)) = file.old {
            add_tracked_counts(namespace, contents, -1, &mut counts);
        }
        if let Some((namespace, contents)) = file.new {
            add_tracked_counts(namespace, contents, 1, &mut counts);
        }
    }
    for token in totals.keys() {
        counts.entry(token.clone()).or_default();
    }
    for (token, count) in counts {
        let delta = totals.entry(token).or_default();
        let net =
            (delta.added + delta.evolved_from) as i64 - (delta.removed + delta.evolved_into) as i64;
        match count - net {
            0 => {}
            more if more > 0 => delta.added += more as u32,
            fewer => delta.removed += (-fewer) as u32,
        }
    }
    totals.retain(|_, delta| !delta.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_tracked_counts() {
        // (Keywords, punctuation and numbers aren't tracked; a bug number is
        // by its preceding word.)
        let contents =
            "% i Widget\n% o ::\n% i Frob\n% k return\n% c Bug\n% c 123456\n% c Widget,\n";
        let mut counts = HashMap::new();
        add_tracked_counts("cpp", contents, 1, &mut counts);
        assert_eq!(counts.get("Widget"), Some(&2));
        assert_eq!(counts.get("Frob"), Some(&1));
        assert_eq!(counts.get("bug-123456"), Some(&1));
        assert_eq!(counts.get("return"), None);
        add_tracked_counts("cpp", "% i Widget\n", -1, &mut counts);
        assert_eq!(counts.get("Widget"), Some(&1));
    }

    #[test]
    fn test_exact_totals() {
        // (A token copied into a copied file, which the inference says was
        // moved from the source, which stays: its instances grow by one.)
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(
            "Widget".to_string(),
            TokenDeltaDetails {
                moved: 1,
                ..Default::default()
            },
        );
        totals.insert(
            "Gone".to_string(),
            TokenDeltaDetails {
                removed: 2,
                ..Default::default()
            },
        );
        let files = [
            CountedFile {
                old: None,
                new: Some(("cpp", "% i Widget\n")),
            },
            CountedFile {
                old: Some(("cpp", "% i Gone\n% i Gone\n% i Kept\n")),
                new: Some(("cpp", "% i Kept\n")),
            },
        ];
        exact_totals(&mut totals, &files);
        assert_eq!(
            totals["Widget"],
            TokenDeltaDetails {
                added: 1,
                moved: 1,
                ..Default::default()
            }
        );
        assert_eq!(totals["Gone"].removed, 2);
        assert!(!totals.contains_key("Kept"));
    }

    #[test]
    fn test_journal_sum() {
        let text = concat!(
            "{}\n",
            r#"{"type":"Detail","source_rev":"a","syntax_rev":"b","iso_date":"2020-01-01T00:00:00Z","added":3,"evolved_into":1}"#,
            "\n",
            r#"{"type":"Summary","source_revs":["c"],"preds":[],"iso_week_range":[2019,1,1],"removed":5,"evolved_from":7,"moved":4}"#,
        );
        assert_eq!(journal_sum(text), 3 - 1 - 5 + 7);
    }
}
