//! Corrections of typos in queries which find nothing (ex: `sericeworkerprivate.cpp`
//! for `ServiceWorkerPrivate.cpp`), from the words a tree's index knows: its
//! identifiers' names (their last components) and its files' and
//! directories' names, which crossref writes, lowercased, with their usual
//! case and how often they occur, in the index's `typo-words` (see
//! `write_words`).
//!
//! A term's corrections are the words within a small edit distance of it (1
//! for terms of up to 7 bytes, 2 for longer ones), found by scanning the
//! words, which is quick enough for a query which found nothing (~20 ms for
//! firefox's ~750,000 words) without an index of the words' edits (which at
//! distance 2 would be hundreds of entries a word).  They're ranked by a
//! distance in which transposed letters and letters next to each other on a
//! keyboard cost less (see `typo_cost`), and then by how often they occur.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use memmap::Mmap;

/// The index's file of words.
pub const WORDS_FILE: &str = "typo-words";

/// The most corrections of a term (and corrected queries).
pub const MAX_CORRECTIONS: usize = 3;

/// A word's last component of an identifier (ex: `mozilla::dom::Foo`'s `Foo`).
fn identifier_word(identifier: &str) -> &str {
    identifier.rsplit("::").next().unwrap_or(identifier)
}

/// Write the index's words (see the module's comment): `identifiers`' names,
/// with how many symbols each has, and the names of `files` (paths) and of
/// their directories, as lines "LOWERCASED\tUSUAL CASE\tCOUNT\n", sorted by
/// their lowercased forms' lengths and then the forms.  A word's usual case
/// is the case of its identifier with the most symbols (or else its first
/// file's or directory's).  (Only the lowercased forms are allocated: the
/// cases are borrowed, since firefox has over a million words.)
pub fn write_words<'a>(
    index_path: &str,
    identifiers: impl Iterator<Item = (&'a str, usize)>,
    files: impl Iterator<Item = &'a str>,
) -> std::io::Result<()> {
    // (By lowercased word, its usual case, that case's count, and its count.)
    let mut words: HashMap<String, (&'a str, usize, usize)> = HashMap::new();
    // (Files' cases only count for words which no identifier has.)
    let mut add = |word: &'a str, case_count: usize, count: usize| {
        if word.is_empty() || word.contains(['\t', '\n']) {
            return;
        }
        let entry = words.entry(word.to_lowercase()).or_insert((word, 0, 0));
        if case_count > entry.1 {
            (entry.0, entry.1) = (word, case_count);
        }
        entry.2 += count;
    };
    for (identifier, count) in identifiers {
        add(identifier_word(identifier), count, count);
    }
    for file in files {
        for component in file.split('/') {
            add(component, 0, 1);
        }
    }
    let mut lines: Vec<(String, &str, usize)> = words
        .into_iter()
        .map(|(lower, (usual, _, count))| (lower, usual, count))
        .collect();
    lines.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
    let mut out = BufWriter::new(File::create(Path::new(index_path).join(WORDS_FILE))?);
    for (lower, usual, count) in lines {
        writeln!(out, "{}\t{}\t{}", lower, usual, count)?;
    }
    out.flush()
}

/// An index's words (see `write_words`), mapped.
pub struct Words {
    mmap: Mmap,
}

/// The words of the indexes opened so far, by their paths (None for those
/// without them).
static OPENED: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<Words>>>>> =
    LazyLock::new(Default::default);

/// A possible correction of a term.
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    /// The word, in its usual case.
    pub word: String,
    /// Its distance from the term (see `typo_cost`).
    pub cost: f32,
    pub count: usize,
}

impl Words {
    /// The words of the index at `index_path`, if it has them.
    pub fn open(index_path: &str) -> Option<Arc<Words>> {
        let path = Path::new(index_path).join(WORDS_FILE);
        let mut opened = OPENED.lock().unwrap();
        opened
            .entry(path.clone())
            .or_insert_with(|| {
                let file = File::open(&path).ok()?;
                // (Safety: indexes' files don't change while they're served.)
                let mmap = unsafe { Mmap::map(&file) }.ok()?;
                Some(Arc::new(Words { mmap }))
            })
            .clone()
    }

    /// The words' lines, as (lowercased, usual case, count).
    fn lines(&self) -> impl Iterator<Item = (&[u8], &[u8], usize)> {
        self.mmap.split(|&b| b == b'\n').filter_map(|line| {
            let mut fields = line.splitn(3, |&b| b == b'\t');
            let lower = fields.next()?;
            let usual = fields.next()?;
            let count = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
            Some((lower, usual, count))
        })
    }

    /// The best corrections of `term` (at most `MAX_CORRECTIONS`), none if
    /// it's a word itself, or too short (under 4 bytes) to correct.
    pub fn corrections(&self, term: &str) -> Vec<Correction> {
        let term = term.to_lowercase();
        let term = term.as_bytes();
        if term.len() < 4 {
            return vec![];
        }
        let max = if term.len() <= 7 { 1 } else { 2 };
        let mut found: Vec<Correction> = vec![];
        for (lower, usual, count) in self.lines() {
            // (The words are sorted by length.)
            if lower.len() > term.len() + max {
                break;
            }
            match bounded_distance(term, lower, max) {
                Some(0) => return vec![],
                Some(_) => found.push(Correction {
                    word: String::from_utf8_lossy(usual).into_owned(),
                    cost: typo_cost(term, lower),
                    count,
                }),
                None => {}
            }
        }
        found.sort_by(|a, b| {
            a.cost
                .total_cmp(&b.cost)
                .then_with(|| b.count.cmp(&a.count))
                .then_with(|| a.word.cmp(&b.word))
        });
        found.truncate(MAX_CORRECTIONS);
        found
    }
}

/// The Levenshtein distance of `a` and `b`, if it's at most `max` (a band of
/// the table, stopping once a row's best is over `max`).
fn bounded_distance(a: &[u8], b: &[u8], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    const FAR: usize = usize::MAX / 2;
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![FAR; b.len() + 1];
    for i in 1..=a.len() {
        current.fill(FAR);
        current[0] = i;
        let from = i.saturating_sub(max).max(1);
        let to = (i + max).min(b.len());
        let mut best = if from == 1 { current[0] } else { FAR };
        for j in from..=to {
            let cost = (a[i - 1] != b[j - 1]) as usize;
            current[j] = (previous[j - 1] + cost)
                .min(previous[j] + 1)
                .min(current[j - 1] + 1);
            best = best.min(current[j]);
        }
        if best > max {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[b.len()] <= max).then_some(previous[b.len()])
}

/// QWERTY's rows, for which letters are next to each other.
const ROWS: [&[u8]; 4] = [
    b"1234567890-=",
    b"qwertyuiop[]",
    b"asdfghjkl;'",
    b"zxcvbnm,./",
];

/// Whether `a` and `b` are next to each other on a QWERTY keyboard (on a
/// row, or diagonally on the next).
fn adjacent(a: u8, b: u8) -> bool {
    let at = |key: u8| {
        ROWS.iter()
            .enumerate()
            .find_map(|(row, keys)| keys.iter().position(|&k| k == key).map(|col| (row, col)))
    };
    match (at(a), at(b)) {
        (Some((ra, ca)), Some((rb, cb))) => {
            ra.abs_diff(rb) <= 1 && ca.abs_diff(cb) <= 1 && (ra, ca) != (rb, cb)
        }
        _ => false,
    }
}

/// How likely a typo of `word` `term` is, as a distance: an optimal string
/// alignment distance (transposing adjacent letters is one edit), in which a
/// transposition, a letter next to the intended one on the keyboard, and a
/// doubled or undoubled letter cost less than other edits.
pub fn typo_cost(term: &[u8], word: &[u8]) -> f32 {
    let (n, m) = (term.len(), word.len());
    let mut d = vec![vec![0f32; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i as f32;
    }
    for j in 0..=m {
        d[0][j] = j as f32;
    }
    for i in 1..=n {
        for j in 1..=m {
            let (t, w) = (term[i - 1], word[j - 1]);
            let substitution = match () {
                _ if t == w => 0.0,
                _ if adjacent(t, w) => 0.6,
                _ => 1.0,
            };
            // (A letter left out or typed twice, next to the same letter,
            // is a likelier slip.)
            let deletion = if i >= 2 && term[i - 2] == t { 0.5 } else { 1.0 };
            let insertion = if j >= 2 && word[j - 2] == w { 0.5 } else { 1.0 };
            let mut best = (d[i - 1][j - 1] + substitution)
                .min(d[i - 1][j] + deletion)
                .min(d[i][j - 1] + insertion);
            if i >= 2 && j >= 2 && t == word[j - 2] && term[i - 2] == w {
                best = best.min(d[i - 2][j - 2] + 0.7);
            }
            d[i][j] = best;
        }
    }
    d[n][m]
}

/// The term of a query's word (`prefix:value`'s value, or the whole word),
/// if it's one to correct: a plain word or an identifier (its last
/// component, of a qualified name or a path), not a regular expression, a
/// quoted phrase or an option (ex: `case:sensitive`); as the range of the
/// word.
fn correctable(word: &str) -> Option<std::ops::Range<usize>> {
    const VALUED: &[&str] = &[
        "id",
        "idprefix",
        "sym",
        "symbol",
        "path",
        "file",
        "text",
        "calls-to",
        "calls-from",
        "calls-between",
        "class-diagram",
        "inheritance-diagram",
        "field-layout",
    ];
    let start = match word.split_once(':') {
        Some((prefix, value))
            if !value.is_empty()
                && !prefix.is_empty()
                && prefix.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-') =>
        {
            if !VALUED.contains(&prefix) {
                return None;
            }
            prefix.len() + 1
        }
        _ => 0,
    };
    let value = &word[start..];
    if value.bytes().any(|b| b"^$*+?()[]{}|\\\"'`".contains(&b)) {
        return None;
    }
    // (A qualified name's or a path's last component.)
    let last = value
        .rfind("::")
        .map(|at| at + 2)
        .or_else(|| value.rfind('/').map(|at| at + 1))
        .unwrap_or(0);
    let range = start + last..word.len();
    (range.len() >= 4).then_some(range)
}

/// The corrections of a query which found nothing (at most
/// `MAX_CORRECTIONS`, best first): its words whose terms aren't words of the
/// index (see `correctable`) replaced by their corrections, the first such
/// word's best ones, and the others' best.
pub fn correct_query(words: &Words, query: &str) -> Vec<String> {
    let parts: Vec<&str> = query.split(' ').collect();
    // (Each word to correct's index, range, and corrections.)
    let mut corrected: Vec<(usize, std::ops::Range<usize>, Vec<Correction>)> = vec![];
    for (i, part) in parts.iter().enumerate() {
        let Some(range) = correctable(part) else {
            continue;
        };
        let corrections = words.corrections(&part[range.clone()]);
        if !corrections.is_empty() {
            corrected.push((i, range, corrections));
        }
    }
    let Some((_, _, first)) = corrected.first() else {
        return vec![];
    };
    (0..first.len())
        .map(|k| {
            let mut parts: Vec<String> = parts.iter().map(|part| part.to_string()).collect();
            for (n, (i, range, corrections)) in corrected.iter().enumerate() {
                let word = &corrections[if n == 0 { k } else { 0 }].word;
                parts[*i].replace_range(range.clone(), word);
            }
            parts.join(" ")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(dir: &Path) -> Arc<Words> {
        write_words(
            dir.to_str().unwrap(),
            [
                ("mozilla::dom::ServiceWorkerPrivate", 3),
                ("mozilla::dom::ServiceWorkerPrivate::mWorkerRef", 1),
                ("WorkerRef", 2),
                ("workerRef", 1),
            ]
            .into_iter(),
            ["dom/serviceworkers/ServiceWorkerPrivate.cpp"].into_iter(),
        )
        .unwrap();
        Words::open(dir.to_str().unwrap()).unwrap()
    }

    #[test]
    fn test_distances() {
        assert_eq!(
            bounded_distance(b"sericeworker", b"serviceworker", 2),
            Some(1)
        );
        assert_eq!(bounded_distance(b"wokrer", b"worker", 2), Some(2));
        assert_eq!(bounded_distance(b"abcdef", b"uvwxyz", 2), None);
        assert_eq!(bounded_distance(b"abc", b"abcdef", 2), None);
        // (Transpositions and neighboring keys cost less.)
        assert!(typo_cost(b"wokrer", b"worker") < 1.0);
        assert!(typo_cost(b"wprker", b"worker") < typo_cost(b"wzrker", b"worker"));
        assert!(typo_cost(b"workker", b"worker") < 1.0);
    }

    #[test]
    fn test_corrections() {
        let dir = std::env::temp_dir().join(format!("typos-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let words = words(&dir);
        let best = words.corrections("sericeworkerprivate.cpp");
        assert_eq!(best[0].word, "ServiceWorkerPrivate.cpp");
        // (Its usual case is its most frequent one.)
        assert_eq!(words.corrections("WokerRef")[0].word, "WorkerRef");
        // (A word needs no correction.)
        assert!(words.corrections("serviceworkerprivate").is_empty());
        assert!(words.corrections("ServiceWorkers").is_empty());

        assert_eq!(
            correct_query(&words, "sericeworkerprivate.cpp"),
            ["ServiceWorkerPrivate.cpp"]
        );
        assert_eq!(
            correct_query(
                &words,
                "path:dom/sericeworkers/SericeWorkerPrivate.cpp mWokerRef"
            ),
            ["path:dom/sericeworkers/ServiceWorkerPrivate.cpp mWorkerRef"]
        );
        // (Options and regular expressions aren't corrected.)
        assert!(correct_query(&words, "case:sensitiv re:WokerRef.*").is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
