//! The token-name index: the names of a tree's history's token journals (see
//! `timeline_tokens`), for finding the tokens which start with, end with, or
//! contain a query (case- and accent-insensitively), ex: for the broader search
//! of a removed token's note.  Built at index time from the history's head
//! (`build-token-names`), since journals are only ever added (a token's
//! journal stays once it appears), rather than kept in the history: a sorted
//! list rewritten as tokens appear would be history state through time for
//! little benefit, and listing the journals takes seconds (firefox-shared's
//! 6.5M, in 3.5 s; see token-history-search.md's "Token-name index").
//!
//! Two files in the index's `token-names/` directory, of the names, a line
//! each:
//! - `names`, sorted by their folded forms (see `fold`), for prefixes (by
//!   bisection), and for substrings (by scanning, case-insensitively, ~20-50
//!   ms for firefox's 117 MB, but only with the query's accents: the names
//!   aren't folded in the file);
//! - `reversed`, sorted by their folded forms reversed, for suffixes.
//!
//! Folding lowercases, and folds Latin letters' accents (ex: "Größe" and
//! "GROSSE" both fold to "grosse"), but not other scripts' combining marks,
//! which are vowel signs and the like (dropping them all folded 333 of
//! firefox-shared's names, Indic words, to nothing).

use std::cmp::Ordering;
use std::fs;
use std::io::Write;
use std::path::Path;

use memmap::Mmap;
use regex::bytes::RegexBuilder;

/// The base letters of the Latin-1 Supplement's and Latin Extended-A's
/// letters with diacritics (U+00C0-U+017F), lowercase, by code point; '\0'
/// for those without one (ex: `×`, `÷`).  "ß" folds to "ss", "æ" to "ae",
/// "œ" to "oe", "ĳ" to "ij" (see `fold`).
const LATIN_BASES: &str = concat!(
    // U+00C0-U+00FF
    "aaaaaaaceeeeiiii",
    "dnooooo\0ouuuuyts",
    "aaaaaaaceeeeiiii",
    "dnooooo\0ouuuuyty",
    // U+0100-U+017F
    "aaaaaaccccccccdd",
    "ddeeeeeeeeeegggg",
    "gggghhhhiiiiiiii",
    "iijjjjkkklllllll",
    "lllnnnnnnnnnoooo",
    "oooorrrrrrssssss",
    "ssttttttuuuuuuuu",
    "uuuuwwyyyzzzzzzs",
);

/// A token's folded form: lowercase, with Latin letters' accents folded.
pub fn fold(token: &str) -> String {
    let mut folded = String::with_capacity(token.len());
    for c in token.chars() {
        for c in c.to_lowercase() {
            match c {
                'ß' => folded.push_str("ss"),
                'æ' => folded.push_str("ae"),
                'œ' => folded.push_str("oe"),
                'ĳ' => folded.push_str("ij"),
                '\u{c0}'..='\u{17f}' => match LATIN_BASES.as_bytes()[c as usize - 0xc0] {
                    0 => folded.push(c),
                    base => folded.push(base as char),
                },
                c => folded.push(c),
            }
        }
    }
    folded
}

/// A folded form reversed (by characters), for suffixes.
fn reversed(folded: &str) -> String {
    folded.chars().rev().collect()
}

/// Write the index for `names` (journals' tokens) into `dir`, replacing it.
pub fn write_index(dir: &Path, mut names: Vec<String>) -> std::io::Result<()> {
    names.sort();
    names.dedup();
    let tmp = dir.with_extension("tmp");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp)?;
    let mut keyed: Vec<(String, &String)> = names.iter().map(|name| (fold(name), name)).collect();
    let write = |file: &str, keyed: &[(String, &String)]| -> std::io::Result<()> {
        let mut out = std::io::BufWriter::new(fs::File::create(tmp.join(file))?);
        for (_, name) in keyed {
            out.write_all(name.as_bytes())?;
            out.write_all(b"\n")?;
        }
        out.flush()
    };
    keyed.sort();
    write("names", &keyed)?;
    for (key, _) in keyed.iter_mut() {
        *key = reversed(key);
    }
    keyed.sort();
    write("reversed", &keyed)?;
    let _ = fs::remove_dir_all(dir);
    fs::rename(&tmp, dir)
}

/// Names a query matched, and whether there were more than the limit.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenNameMatches {
    pub names: Vec<String>,
    pub more: bool,
}

/// How a query matches names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NameMatch {
    Prefix,
    Suffix,
    Contains,
}

/// A tree's token-name index, mapped (an empty file, which can't be mapped,
/// as None).
#[derive(Debug)]
pub struct TokenNames {
    names: Option<Mmap>,
    reversed: Option<Mmap>,
}

fn bytes(map: &Option<Mmap>) -> &[u8] {
    map.as_deref().unwrap_or(&[])
}

/// The line of `data` containing offset `at`: its start and end.
fn line_at(data: &[u8], at: usize) -> (usize, usize) {
    let start = data[..at]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let end = data[at..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(data.len(), |i| at + i);
    (start, end)
}

/// The first line of `data` (sorted by `key`) whose key isn't less than
/// `target`, by bisection: its start.
fn lower_bound(data: &[u8], target: &str, key: &dyn Fn(&str) -> String) -> usize {
    let (mut lo, mut hi) = (0, data.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let (start, end) = line_at(data, mid);
        let line = std::str::from_utf8(&data[start..end]).unwrap_or("");
        match key(line).as_str().cmp(target) {
            Ordering::Less => lo = end + 1,
            _ => hi = start,
        }
    }
    lo.min(data.len())
}

impl TokenNames {
    /// The index in `dir` (an index's `token-names/`), if it's there.
    pub fn open(dir: &Path) -> Option<TokenNames> {
        let map = |file: &str| -> Option<Option<Mmap>> {
            let file = fs::File::open(dir.join(file)).ok()?;
            if file.metadata().ok()?.len() == 0 {
                return Some(None);
            }
            unsafe { Mmap::map(&file).ok().map(Some) }
        };
        Some(TokenNames {
            names: map("names")?,
            reversed: map("reversed")?,
        })
    }

    /// The names matching `query` as `how` says (folded), up to `limit`, in
    /// their files' order (for substrings, the names'), and whether there were
    /// more.
    pub fn find(&self, query: &str, how: NameMatch, limit: usize) -> (Vec<String>, bool) {
        let folded = fold(query);
        let mut found = vec![];
        let mut push = |name: &str| -> bool {
            if found.len() >= limit {
                return false;
            }
            found.push(name.to_string());
            true
        };
        let mut more = false;
        match how {
            NameMatch::Prefix | NameMatch::Suffix => {
                let reversed_key = |name: &str| reversed(&fold(name));
                let (data, target, key): (&[u8], String, &dyn Fn(&str) -> String) = match how {
                    NameMatch::Prefix => (bytes(&self.names), folded, &fold),
                    _ => (bytes(&self.reversed), reversed(&folded), &reversed_key),
                };
                let mut at = lower_bound(data, &target, key);
                while at < data.len() {
                    let (_, end) = line_at(data, at);
                    let line = std::str::from_utf8(&data[at..end]).unwrap_or("");
                    if !key(line).starts_with(&target) {
                        break;
                    }
                    if !push(line) {
                        more = true;
                        break;
                    }
                    at = end + 1;
                }
            }
            NameMatch::Contains => {
                // (The names aren't folded: a case-insensitive search of the
                // query, and of its accents' folds, by checking each match's
                // name folded.)
                // (ASCII queries case-insensitively in ASCII: Unicode's adds
                // `ſ` to `s` and `K` (the Kelvin sign) to `k`, which keeps
                // the regex from searching for a literal: 1.1 s rather than
                // ~20 ms for "structopt" in 6.5M names.)
                let Ok(re) = RegexBuilder::new(&regex::escape(query))
                    .case_insensitive(true)
                    .unicode(!query.is_ascii())
                    .build()
                else {
                    return (vec![], false);
                };
                let data = bytes(&self.names);
                let mut at = 0;
                while let Some(m) = re.find_at(data, at) {
                    let (start, end) = line_at(data, m.start());
                    let line = std::str::from_utf8(&data[start..end]).unwrap_or("");
                    if fold(line).contains(&folded) && !push(line) {
                        more = true;
                        break;
                    }
                    at = end + 1;
                    if at >= data.len() {
                        break;
                    }
                }
            }
        }
        (found, more)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fold() {
        assert_eq!(fold("BackstagePass"), "backstagepass");
        assert_eq!(fold("Größe"), "grosse");
        assert_eq!(fold("GRÖSSE"), "grösse".replace('ö', "o"));
        assert_eq!(fold("Élève"), "eleve");
        assert_eq!(fold("Œuvre"), "oeuvre");
        assert_eq!(fold("Ž"), "z");
        // (Other scripts keep their marks.)
        assert_eq!(fold("हिंदी"), "हिंदी");
        assert_eq!(fold("中文"), "中文");
    }

    #[test]
    fn test_find() {
        let dir = std::env::temp_dir().join(format!("token-names-{}", std::process::id()));
        let names: Vec<String> = [
            "BackstagePass",
            "backstagePass",
            "nsBackstagePass",
            "BackstagePassWrapper",
            "SystemGlobal",
            "Größe",
            "Grosse",
            "zebra",
            "abc",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        write_index(&dir, names).unwrap();
        let index = TokenNames::open(&dir).unwrap();
        let find = |query: &str, how: NameMatch, limit: usize| index.find(query, how, limit);

        let (prefix, more) = find("backstagep", NameMatch::Prefix, 10);
        assert_eq!(
            prefix,
            vec!["BackstagePass", "backstagePass", "BackstagePassWrapper"]
        );
        assert!(!more);
        let (suffix, _) = find("PASS", NameMatch::Suffix, 10);
        assert_eq!(
            suffix,
            vec!["BackstagePass", "backstagePass", "nsBackstagePass"]
        );
        let (contains, _) = find("stagepass", NameMatch::Contains, 10);
        assert_eq!(
            contains,
            vec![
                "BackstagePass",
                "backstagePass",
                "BackstagePassWrapper",
                "nsBackstagePass"
            ]
        );
        // (Accents fold both ways.)
        assert_eq!(
            find("grösse", NameMatch::Prefix, 10).0,
            vec!["Grosse", "Größe"]
        );
        assert_eq!(find("GROSSE", NameMatch::Contains, 10).0, vec!["Grosse"]);
        // (Limits.)
        assert_eq!(
            find("b", NameMatch::Prefix, 2),
            (
                vec!["BackstagePass".to_string(), "backstagePass".to_string()],
                true
            )
        );
        assert_eq!(
            find("nothing", NameMatch::Contains, 10).0,
            Vec::<String>::new()
        );
        assert_eq!(find("", NameMatch::Prefix, 100).0.len(), 9);
        fs::remove_dir_all(dir).unwrap();
    }
}
