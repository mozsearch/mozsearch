//! Which of libgit2's file pairings (renames and copies; see
//! `renames::find_similar`) a revision's history keeps, by how much of the
//! files' content the inference finds shared (see
//! `inference::PairingSupport`): for linear revisions, and for merges against
//! their first parents (which only pair their renamed and copied files'
//! tokens for this; see build-timeline-tree's `preprocess_merge`).
//!
//! - A deleted file which libgit2 reports as the source of several renames
//!   (ex: a file split in two) is continued by one of them (see
//!   `continuations`); the others are copies.
//! - A rename whose files share less than `MIN_RENAME_CONTENT_SIMILARITY` is a
//!   deletion and an addition (see `keeps_rename`).
//! - A copy most of whose content wasn't copied is an addition (see
//!   `keeps_copy`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::inference::{FileChangeKind, PairingSupport};

/// git pairs renamed files by similarity, which small unrelated files can
/// reach just by sharing a license header and some syntax.  We only keep a
/// rename if the content similarity of the files (see `PairingSupport`) is at
/// least this, otherwise we treat it as a deletion and an addition.
///
/// This is deliberately lower than git's threshold (30%).  In the mozsearch
/// history and a replay of vendored puppeteer syncs, the renames below 0.2
/// were all unrelated files, but 0.2 to 0.45 also has real renames of
/// rewritten files (ex: a 0.24 move of ProductLauncher.ts into a new package, a
/// 0.42 .js to .ts conversion).  Wrongly splitting a rename loses more than
/// wrongly keeping one: shared content is only found as moved if it's in long
/// enough runs.
pub const MIN_RENAME_CONTENT_SIMILARITY: f64 = 0.2;

/// A file change of a revision, for deciding its pairing: its kind, paths, and
/// the support for its pairing (renames' and copies').
pub struct Pairing<'a> {
    pub kind: FileChangeKind,
    pub old_path: Option<&'a str>,
    pub new_path: Option<&'a str>,
    pub support: Option<&'a PairingSupport>,
}

struct Candidate {
    idx: usize,
    similarity: f64,
    supported: u32,
    same_name: bool,
}

/// Whether `challenger` continues the old file rather than `rename`: it has at
/// least a quarter (and 2 tokens) more of the old file's content, since
/// similarity favors smaller files (there, 0.65 vs 0.63), but small
/// differences in shared content are often generic (ex: ffvpx's deleted
/// x86/pixelutils.h has 16 to 19 content tokens in each of 3 unrelated new
/// headers).
fn continues_better(challenger: &Candidate, rename: &Candidate) -> bool {
    challenger.supported >= rename.supported + (rename.supported / 4).max(2)
}

/// For each renamed file's old path, the index of the change which continues
/// it: libgit2 can report a deleted file as the source of several renames
/// (ex: when a file is split in two), which would give the old file's tokens
/// fates from each of them.  Only the most similar one is a rename, unless
/// another of them, or of the copies from the deleted file, has clearly more
/// of its content (see `continues_better`): ex: puppeteer's
/// firefox-data.spec.ts was "renamed" to chromedriver/chromedriver-data.spec.ts
/// (63 of its 108 content tokens) but "copied" to firefox/firefox-data.spec.ts
/// (86).  The others are copies (which `keeps_copy` may make additions).
pub fn continuations(pairings: &[Pairing]) -> HashMap<String, usize> {
    let renamed_from: HashSet<&str> = pairings
        .iter()
        .filter(|p| p.kind == FileChangeKind::Renamed)
        .filter_map(|p| p.old_path)
        .collect();
    let mut candidates: HashMap<&str, (Vec<Candidate>, Vec<Candidate>)> = HashMap::new();
    for (idx, p) in pairings.iter().enumerate() {
        let (Some(support), Some(old_path), Some(new_path)) = (p.support, p.old_path, p.new_path)
        else {
            continue;
        };
        if !renamed_from.contains(old_path) {
            continue;
        }
        let candidate = Candidate {
            idx,
            similarity: support.similarity().unwrap_or(0.0),
            supported: support.supported,
            same_name: Path::new(old_path).file_name() == Path::new(new_path).file_name(),
        };
        let (renames, copies) = candidates.entry(old_path).or_default();
        if p.kind == FileChangeKind::Renamed {
            renames.push(candidate);
        } else {
            copies.push(candidate);
        }
    }
    let mut best_renames = HashMap::new();
    for (old_path, (renames, copies)) in &candidates {
        // (The first of the most similar, and of the most content.)
        let Some(most_similar) = renames
            .iter()
            .reduce(|a, b| if b.similarity > a.similarity { b } else { a })
        else {
            continue;
        };
        let challenger = renames
            .iter()
            .chain(copies)
            .filter(|c| c.idx != most_similar.idx)
            .reduce(|a, b| {
                if (b.supported, b.same_name) > (a.supported, a.same_name) {
                    b
                } else {
                    a
                }
            });
        let best = match challenger {
            Some(challenger) if continues_better(challenger, most_similar) => challenger,
            _ => most_similar,
        };
        best_renames.insert(old_path.to_string(), best.idx);
    }
    best_renames
}

/// The kind a change's pairing becomes given the `continuations` (see there):
/// a rename which doesn't continue its old file is a copy, and a copy which
/// does is the rename.
pub fn continued_kind(
    idx: usize,
    pairing: &Pairing,
    continuations: &HashMap<String, usize>,
) -> FileChangeKind {
    let best = pairing.old_path.and_then(|old| continuations.get(old));
    match (pairing.kind, best) {
        (FileChangeKind::Renamed, Some(&best)) if best != idx => FileChangeKind::Copied,
        (FileChangeKind::Copied, Some(&best)) if best == idx => FileChangeKind::Renamed,
        (kind, _) => kind,
    }
}

/// Whether a rename's files share enough content to keep it (see
/// `MIN_RENAME_CONTENT_SIMILARITY`); files without content keep git's word.
pub fn keeps_rename(support: &PairingSupport) -> bool {
    support
        .similarity()
        .is_none_or(|s| s >= MIN_RENAME_CONTENT_SIMILARITY)
}

/// Whether a copy is one: git's copy detection is similarity based, so a new
/// file which was split out of an existing file (and which the inference
/// determined was mainly moved tokens) or which just shares boilerplate with
/// an existing file can be detected as a copy.  For the purposes of the
/// file-level history (sentinel, files-delta, symbol changes) we only want to
/// treat the file as a copy if the majority of its content was actually
/// copied, otherwise it's a new file.
pub fn keeps_copy(support: &PairingSupport) -> bool {
    support.new_content == 0 || support.supported * 2 >= support.new_content
}

#[cfg(test)]
mod tests {
    use super::*;

    fn support(old: u32, new: u32, supported: u32) -> PairingSupport {
        PairingSupport {
            old_content: old,
            new_content: new,
            supported,
        }
    }

    #[test]
    fn test_pairing_rules() {
        // (puppeteer's firefox-data.spec.ts: "renamed" to the chromedriver
        // file, with less of its content than the copy.)
        let (rename, copy) = (support(108, 100, 63), support(108, 110, 86));
        let pairings = [
            Pairing {
                kind: FileChangeKind::Renamed,
                old_path: Some("firefox-data.spec.ts"),
                new_path: Some("chromedriver/chromedriver-data.spec.ts"),
                support: Some(&rename),
            },
            Pairing {
                kind: FileChangeKind::Copied,
                old_path: Some("firefox-data.spec.ts"),
                new_path: Some("firefox/firefox-data.spec.ts"),
                support: Some(&copy),
            },
        ];
        let continuations = continuations(&pairings);
        assert_eq!(continuations["firefox-data.spec.ts"], 1);
        assert_eq!(
            continued_kind(0, &pairings[0], &continuations),
            FileChangeKind::Copied
        );
        assert_eq!(
            continued_kind(1, &pairings[1], &continuations),
            FileChangeKind::Renamed
        );

        assert!(keeps_rename(&support(100, 100, 20)));
        assert!(!keeps_rename(&support(100, 100, 19)));
        assert!(keeps_rename(&support(0, 0, 0)));
        assert!(keeps_copy(&support(100, 100, 50)));
        assert!(!keeps_copy(&support(100, 100, 49)));
    }
}
