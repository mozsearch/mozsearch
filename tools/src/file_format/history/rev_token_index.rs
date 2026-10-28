//! The files in which a revision changed each token, for the revisions whose
//! rev-summaries have totals rather than file deltas (those changing more than
//! `MAX_REV_SUMMARY_FILES` files; ex: a tree-wide refactor, a vendored
//! library's update, a test suite's import): written next to the summary, as
//! `by-source-rev/aa/bb/REV.tokens` (see `rev_token_index_path`).  Without
//! it, a history grep (see `hyperblame::history_grep`) whose candidates are
//! such a revision has to check all of its files, when the tokens it's for
//! changed in a few (ex: `notifyObservers` in 102 of the 39,474 files of
//! android-components' removal of its old API docs).
//!
//! It's keyed by the tokens' hashes (see `token_hash`), which are 4 bytes
//! rather than the tokens' text: a hash shared by two tokens has both of
//! their files, so a lookup may give a few files which didn't change the
//! token, but never misses one which did.
//!
//! ## Format
//!
//! Little-endian, laid out so that a lookup only reads what it needs of a big
//! one (ex: a history window's first revision, which adds every file), from a
//! memory map:
//!
//! - The header (`HEADER_LEN` bytes): `MAGIC`, then u32s of the number of
//!   files, of tokens, and of files between path restarts, then u64 offsets of
//!   the sections below, and the file's length.
//! - The token hashes, sorted, a u32 each.
//! - Each token's postings' end in the postings (a u32 each, in the hashes'
//!   order).
//! - The postings: each token's files' indices, ascending, as varints of
//!   their differences (the first from 0).
//! - The files' paths, sorted (those of the summary's `file_deltas`: their
//!   paths after the revision, or before it for deleted files), each as a
//!   varint of the bytes it shares with the previous path, a varint of the
//!   rest's length, and the rest; every `restart_interval`th path shares
//!   nothing, at an offset in the path restarts.
//! - The path restarts: u32 offsets in the paths of every `restart_interval`th
//!   path.
//! - The moves: a varint count, then for each moved file (by index,
//!   ascending), the varint of its index's difference from the previous one's,
//!   and its path before the revision (a varint length and the bytes).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::rev_summaries::RevFileSummaryRecord;

const MAGIC: &[u8; 8] = b"SFRTI1\0\0";
const HEADER_LEN: usize = 80;
/// How many paths a restart point starts.
const RESTART_INTERVAL: u32 = 64;

/// A token's hash: its UTF-8 bytes' 32-bit FNV-1a hash.
pub fn token_hash(token: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in token.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// The path of the index of `source_rev` relative to the rev-summaries root,
/// next to its summary (see `rev_summary_path`).
pub fn rev_token_index_path(source_rev: &str) -> PathBuf {
    let rev = source_rev.to_lowercase();
    let mut path = PathBuf::from("by-source-rev");
    path.push(&rev[0..2]);
    path.push(&rev[2..4]);
    path.push(format!("{}.tokens", rev));
    path
}

/// A changed file, for `encode`: its path (see the format), its path before
/// the revision if it moved, and the hashes of the tokens it changed (sorted,
/// distinct), rather than the tokens, which for a history window's first
/// revision would be hundreds of millions of strings.
pub struct IndexedFile {
    pub path: String,
    pub moved_from: Option<String>,
    pub hashes: Vec<u32>,
}

impl IndexedFile {
    pub fn new<'a>(
        path: String,
        moved_from: Option<String>,
        tokens: impl IntoIterator<Item = &'a str>,
    ) -> IndexedFile {
        let mut hashes: Vec<u32> = tokens.into_iter().map(token_hash).collect();
        hashes.sort_unstable();
        hashes.dedup();
        IndexedFile {
            path,
            moved_from,
            hashes,
        }
    }
}

/// The files of `file_deltas` (a revision's, as its summary would list them),
/// with the tokens their symbols' deltas changed.
pub fn indexed_files(file_deltas: &BTreeMap<String, RevFileSummaryRecord>) -> Vec<IndexedFile> {
    file_deltas
        .iter()
        .map(|(path, file)| {
            IndexedFile::new(
                path.clone(),
                file.delta.moved_from.clone(),
                file.delta
                    .symbol_group
                    .symbol_deltas
                    .values()
                    .flat_map(|symbol| symbol.token_changes.keys().map(String::as_str)),
            )
        })
        .collect()
}

fn write_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// The varint at `*pos` in `bytes`, moving `*pos` past it.
fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut n: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(*pos).ok_or("truncated varint")?;
        *pos += 1;
        n |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return Ok(n);
        }
        shift += 7;
        if shift > 63 {
            return Err("overlong varint".to_string());
        }
    }
}

/// The index of `files` (in any order).
pub fn encode(mut files: Vec<IndexedFile>) -> Vec<u8> {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    // (Each hash's files, ascending, so that tokens sharing a hash share an
    // entry.)
    let mut by_hash: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (index, file) in files.iter().enumerate() {
        for &hash in &file.hashes {
            by_hash.entry(hash).or_default().push(index as u32);
        }
    }
    let mut postings = vec![];
    let mut ends = vec![];
    for indices in by_hash.values() {
        let mut last = 0;
        for &index in indices {
            write_varint(&mut postings, (index - last) as u64);
            last = index;
        }
        ends.push(postings.len() as u32);
    }
    let mut paths = vec![];
    let mut restarts = vec![];
    let mut previous: &str = "";
    for (index, file) in files.iter().enumerate() {
        let shared = if index as u32 % RESTART_INTERVAL == 0 {
            restarts.push(paths.len() as u32);
            0
        } else {
            let mut shared = previous
                .bytes()
                .zip(file.path.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            while !file.path.is_char_boundary(shared) {
                shared -= 1;
            }
            shared
        };
        write_varint(&mut paths, shared as u64);
        write_varint(&mut paths, (file.path.len() - shared) as u64);
        paths.extend_from_slice(&file.path.as_bytes()[shared..]);
        previous = &file.path;
    }
    let mut moves = vec![];
    let moved: Vec<(usize, &String)> = files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| file.moved_from.as_ref().map(|from| (index, from)))
        .collect();
    write_varint(&mut moves, moved.len() as u64);
    let mut last = 0;
    for (index, from) in moved {
        write_varint(&mut moves, (index - last) as u64);
        last = index;
        write_varint(&mut moves, from.len() as u64);
        moves.extend_from_slice(from.as_bytes());
    }

    let hashes_at = HEADER_LEN;
    let ends_at = hashes_at + 4 * by_hash.len();
    let postings_at = ends_at + 4 * ends.len();
    let paths_at = postings_at + postings.len();
    let restarts_at = paths_at + paths.len();
    let moves_at = restarts_at + 4 * restarts.len();
    let len = moves_at + moves.len();
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(MAGIC);
    for n in [
        files.len() as u32,
        by_hash.len() as u32,
        RESTART_INTERVAL,
        0,
    ] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    for offset in [
        hashes_at,
        ends_at,
        postings_at,
        paths_at,
        restarts_at,
        moves_at,
        len,
    ] {
        out.extend_from_slice(&(offset as u64).to_le_bytes());
    }
    debug_assert_eq!(out.len(), HEADER_LEN);
    for hash in by_hash.keys() {
        out.extend_from_slice(&hash.to_le_bytes());
    }
    for end in ends {
        out.extend_from_slice(&end.to_le_bytes());
    }
    out.extend_from_slice(&postings);
    out.extend_from_slice(&paths);
    for restart in restarts {
        out.extend_from_slice(&restart.to_le_bytes());
    }
    out.extend_from_slice(&moves);
    out
}

/// An index's bytes (ex: a memory map of its file; see `RevTokenIndex::open`),
/// to look up.
pub struct RevTokenIndex<B: AsRef<[u8]>> {
    bytes: B,
    files: u32,
    tokens: u32,
    restart_interval: u32,
    hashes_at: usize,
    ends_at: usize,
    postings_at: usize,
    paths_at: usize,
    restarts_at: usize,
    moves_at: usize,
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| format!("truncated at {}", at))
}

impl RevTokenIndex<memmap::Mmap> {
    /// The index of `source_rev` in the rev-summaries at `root`, if it has
    /// one.
    pub fn open(root: &Path, source_rev: &str) -> Option<Self> {
        let file = std::fs::File::open(root.join(rev_token_index_path(source_rev))).ok()?;
        let map = unsafe { memmap::Mmap::map(&file) }.ok()?;
        RevTokenIndex::new(map).ok()
    }
}

impl<B: AsRef<[u8]>> RevTokenIndex<B> {
    pub fn new(bytes: B) -> Result<Self, String> {
        let b = bytes.as_ref();
        if b.len() < HEADER_LEN || &b[..8] != MAGIC {
            return Err("not a token index".to_string());
        }
        let offset =
            |i: usize| u64::from_le_bytes(b[24 + 8 * i..32 + 8 * i].try_into().unwrap()) as usize;
        let len = b.len();
        let length = offset(6);
        let index = RevTokenIndex {
            files: u32_at(b, 8)?,
            tokens: u32_at(b, 12)?,
            restart_interval: u32_at(b, 16)?.max(1),
            hashes_at: offset(0),
            ends_at: offset(1),
            postings_at: offset(2),
            paths_at: offset(3),
            restarts_at: offset(4),
            moves_at: offset(5),
            bytes,
        };
        if length != len
            || index.ends_at != index.hashes_at + 4 * index.tokens as usize
            || index.postings_at != index.ends_at + 4 * index.tokens as usize
            || index.moves_at < index.restarts_at
        {
            return Err("bad token index offsets".to_string());
        }
        Ok(index)
    }

    /// How many files the revision changed (with deltas).
    pub fn file_count(&self) -> usize {
        self.files as usize
    }

    /// How many distinct token hashes it has.
    pub fn token_count(&self) -> usize {
        self.tokens as usize
    }

    /// The indices of the files which changed a token with `hash`.
    fn postings(&self, hash: u32) -> Result<Vec<u32>, String> {
        let b = self.bytes.as_ref();
        let (mut low, mut high) = (0usize, self.tokens as usize);
        while low < high {
            let mid = (low + high) / 2;
            if u32_at(b, self.hashes_at + 4 * mid)? < hash {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if low == self.tokens as usize || u32_at(b, self.hashes_at + 4 * low)? != hash {
            return Ok(vec![]);
        }
        let start = if low == 0 {
            0
        } else {
            u32_at(b, self.ends_at + 4 * (low - 1))? as usize
        };
        let end = u32_at(b, self.ends_at + 4 * low)? as usize;
        let postings = b
            .get(self.postings_at + start..self.postings_at + end)
            .ok_or("bad postings")?;
        let (mut pos, mut last, mut out) = (0, 0u64, vec![]);
        while pos < postings.len() {
            last += read_varint(postings, &mut pos)?;
            out.push(last as u32);
        }
        Ok(out)
    }

    /// The paths of the files at `indices` (ascending), decoding each restart
    /// block they're in once.
    fn paths_of(&self, indices: &[u32]) -> Result<Vec<String>, String> {
        let b = self.bytes.as_ref();
        let paths = b.get(self.paths_at..self.restarts_at).ok_or("bad paths")?;
        let mut out = Vec::with_capacity(indices.len());
        let mut block: Option<(u32, Vec<String>)> = None;
        for &index in indices {
            if index >= self.files {
                return Err(format!("no file {}", index));
            }
            let restart = index / self.restart_interval;
            if block.as_ref().is_none_or(|(at, _)| *at != restart) {
                let mut pos = u32_at(b, self.restarts_at + 4 * restart as usize)? as usize;
                let count = self
                    .restart_interval
                    .min(self.files - restart * self.restart_interval);
                let mut decoded: Vec<String> = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let shared = read_varint(paths, &mut pos)? as usize;
                    let rest = read_varint(paths, &mut pos)? as usize;
                    let previous = decoded.last().map_or("", |p| p.as_str());
                    let mut path = previous
                        .get(..shared)
                        .ok_or("bad shared prefix")?
                        .to_string();
                    let bytes = paths.get(pos..pos + rest).ok_or("truncated path")?;
                    path.push_str(std::str::from_utf8(bytes).map_err(|e| e.to_string())?);
                    pos += rest;
                    decoded.push(path);
                }
                block = Some((restart, decoded));
            }
            let (_, decoded) = block.as_ref().unwrap();
            out.push(decoded[(index % self.restart_interval) as usize].clone());
        }
        Ok(out)
    }

    /// The moved files' paths before the revision, by their indices.
    fn moves(&self) -> Result<BTreeMap<u32, String>, String> {
        let b = self.bytes.as_ref();
        let moves = b.get(self.moves_at..).ok_or("bad moves")?;
        let mut pos = 0;
        let count = read_varint(moves, &mut pos)?;
        let (mut out, mut last) = (BTreeMap::new(), 0u64);
        for _ in 0..count {
            last += read_varint(moves, &mut pos)?;
            let len = read_varint(moves, &mut pos)? as usize;
            let bytes = moves.get(pos..pos + len).ok_or("truncated move")?;
            out.insert(
                last as u32,
                std::str::from_utf8(bytes)
                    .map_err(|e| e.to_string())?
                    .to_string(),
            );
            pos += len;
        }
        Ok(out)
    }

    /// The files (sorted, with their paths before the revision if it moved
    /// them) which changed any of `tokens`, and maybe a few others whose
    /// tokens share their hashes.
    pub fn files_changing<'a>(
        &self,
        tokens: impl IntoIterator<Item = &'a String>,
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let mut indices = BTreeSet::new();
        let hashes: BTreeSet<u32> = tokens.into_iter().map(|token| token_hash(token)).collect();
        for hash in hashes {
            indices.extend(self.postings(hash)?);
        }
        let indices: Vec<u32> = indices.into_iter().collect();
        self.with_moves(&indices)
    }

    /// All of the files (sorted, with their paths before the revision if it
    /// moved them).
    pub fn all_files(&self) -> Result<Vec<(String, Option<String>)>, String> {
        let indices: Vec<u32> = (0..self.files).collect();
        self.with_moves(&indices)
    }

    fn with_moves(&self, indices: &[u32]) -> Result<Vec<(String, Option<String>)>, String> {
        let paths = self.paths_of(indices)?;
        let mut moves = if indices.is_empty() {
            BTreeMap::new()
        } else {
            self.moves()?
        };
        Ok(indices
            .iter()
            .zip(paths)
            .map(|(index, path)| (path, moves.remove(index)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, moved_from: Option<&str>, tokens: &[&str]) -> IndexedFile {
        IndexedFile::new(
            path.to_string(),
            moved_from.map(str::to_string),
            tokens.iter().copied(),
        )
    }

    #[test]
    fn test_round_trip() {
        // (More files than a restart block, with shared prefixes, non-ASCII
        // paths, and a move.)
        let mut files: Vec<IndexedFile> = (0..150)
            .map(|i| {
                let tokens: &[&str] = match i % 3 {
                    0 => &["Foo", "common"],
                    1 => &["common"],
                    _ => &[],
                };
                file(&format!("dir/sub/fïle{:03}.cpp", i), None, tokens)
            })
            .collect();
        files.push(file(
            "a/moved.js",
            Some("old/place.js"),
            &["notifyObservers"],
        ));
        files.push(file("z/last.js", None, &["NotifyObservers", "Foo"]));
        let bytes = encode(files);
        let index = RevTokenIndex::new(bytes).unwrap();
        assert_eq!(index.file_count(), 152);
        assert_eq!(index.token_count(), 4);

        let tokens = ["notifyObservers".to_string(), "NotifyObservers".to_string()];
        assert_eq!(
            index.files_changing(&tokens).unwrap(),
            vec![
                ("a/moved.js".to_string(), Some("old/place.js".to_string())),
                ("z/last.js".to_string(), None),
            ]
        );
        let foo = index.files_changing(&["Foo".to_string()]).unwrap();
        assert_eq!(foo.len(), 51);
        assert_eq!(foo[0].0, "dir/sub/fïle000.cpp");
        assert_eq!(foo[49].0, "dir/sub/fïle147.cpp");
        assert_eq!(foo[50].0, "z/last.js");
        assert!(
            index
                .files_changing(&["absent".to_string()])
                .unwrap()
                .is_empty()
        );
        let all = index.all_files().unwrap();
        assert_eq!(all.len(), 152);
        assert!(all.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(
            all[0],
            ("a/moved.js".to_string(), Some("old/place.js".to_string()))
        );

        // (Nothing is an index too.)
        let empty = RevTokenIndex::new(encode(vec![])).unwrap();
        assert!(empty.all_files().unwrap().is_empty());
        assert!(RevTokenIndex::new(b"nope".to_vec()).is_err());
    }

    #[test]
    fn test_shared_hashes_never_miss() {
        // (Tokens whose hashes are the same have both of their files.)
        let (a, b) = ("costarring", "liquid");
        assert_eq!(token_hash(a), token_hash(b), "FNV-1a's known collision");
        let index = RevTokenIndex::new(encode(vec![
            file("a.txt", None, &[a]),
            file("b.txt", None, &[b]),
        ]))
        .unwrap();
        assert_eq!(index.files_changing(&[a.to_string()]).unwrap().len(), 2);
    }
}
