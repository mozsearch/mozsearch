//! Token-centric blame as presented on source listing pages: what the blame
//! strip shows for each line, the `BLAME_INFO` data the page's JS uses to
//! colorize the strip, describe lines in the blame popup, and map `#tokens=`
//! hashes to lines, and the per-token data for the popup, which the page loads
//! separately in chunks of lines (the "hyperblame" data files).  See "Token
//! blame UI plan" in the hyperblame notes.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::ops::RangeInclusive;

use serde::Serialize;

use super::token_blame::FileTokenBlame;
use crate::file_format::history::timeline_annotated::RemovalMarker;

/// What the page needs to know about a commit.
pub struct CommitMeta {
    /// The commit time, in seconds since the epoch.
    pub time: i64,
    pub author: String,
}

#[derive(Serialize)]
pub struct BlameInfo {
    /// `[rev, commit time, author index]` for each commit, indexed by the
    /// commit indices in the strip's classes and data attributes.
    pub commits: Vec<(String, i64, usize)>,
    pub authors: Vec<String>,
    /// The paths the strip's data attributes refer to; index 0 is the file
    /// itself.
    pub paths: Vec<String>,
    /// The number of tokens on each line, for mapping `#tokens=` hashes, which
    /// name tokens by their (1-based) index in the file, to lines.
    #[serde(rename = "tokenCounts")]
    pub token_counts: Vec<u32>,
    /// The (0-based) first line of each chunk of the popup's data; see
    /// `LinesChunk`.
    pub chunks: Vec<usize>,
    /// The URL of the directory with the page's hyperblame data files
    /// (`commits.json` and `lines-K.json` for each chunk K), which depends on
    /// whether the page is for the tip or a revision.
    #[serde(rename = "dataUrl", skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
}

/// About how many tokens each chunk of the popup's data has.  Chunks end at
/// line boundaries.
pub const CHUNK_TOKENS: usize = 16384;

/// The popup's data for a chunk of lines, the `lines-K.json` hyperblame file
/// for chunk K.
#[derive(Debug, PartialEq, Serialize)]
pub struct LinesChunk {
    /// The (0-based) index of the chunk's first line.
    #[serde(rename = "firstLine")]
    pub first_line: usize,
    /// For each line, its tokens as `[GAP, LENGTH, COMMIT, LINENO]` or
    /// `[GAP, LENGTH, COMMIT, LINENO, PATH]`, where:
    /// - GAP is the number of UTF-16 code units between the end of the
    ///   previous token on the line (or the start of the line) and the token,
    ///   and LENGTH is the token's length in them.
    /// - COMMIT and PATH (0, the file itself, if omitted) are the commit which
    ///   introduced the token and the token's path there, as indices into
    ///   `BlameInfo::commits` and `BlameInfo::paths`.
    /// - LINENO is the token's (1-based) index in the file in that commit, as
    ///   a delta from the previous token in the chunk with the same commit and
    ///   path.
    pub lines: Vec<Vec<Vec<i64>>>,
    /// The predecessors of the tokens that evolved from other tokens, by the
    /// (1-based) index of the token in the file, as `[COMMIT, PATH, LINENO]`.
    pub preds: BTreeMap<u32, (usize, usize, u32)>,
}

fn utf16_len(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.chars().map(char::len_utf16).sum()
    }
}

/// The tokens on a line from one commit (and path).
#[derive(Debug, PartialEq)]
pub struct StripEntry {
    pub commit: usize,
    pub path: usize,
    /// The ranges of the tokens' indices in the file in that commit.
    pub tokens: Vec<RangeInclusive<u32>>,
}

/// A removal of a run of tokens; see `RemovalMarker`.
#[derive(Debug, PartialEq)]
pub struct StripRemoval {
    pub commit: usize,
    /// The path of the file in the commit's parent.
    pub path: usize,
    /// The index of the first removed token in the file in the commit's parent.
    pub first_token: u32,
    pub num_removed: u32,
    pub num_moved: u32,
}

/// Which neighbor a line without tokens got its commit from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interpolation {
    /// The lines on both sides, which have the same commit.
    Between,
    /// The line above, because the lines on either side have different
    /// commits or there are no lines with tokens below.
    Above,
    /// The line below, because there are no lines with tokens above.
    Below,
}

/// What the blame strip shows for a line.
#[derive(Debug, Default, PartialEq)]
pub struct StripLine {
    /// The commit whose color the line gets.
    pub commit: Option<usize>,
    /// Set if the line has no tokens and got its commit from its neighbors.
    pub interpolated: Option<Interpolation>,
    /// The line's tokens by commit, newest first.
    pub entries: Vec<StripEntry>,
    /// A removal before the first token of the file; only for the first line.
    pub removal_above: Option<StripRemoval>,
    /// A removal between this line and the next line with tokens.
    pub removal_below: Option<StripRemoval>,
    /// Removals between the tokens of this line, newest first.
    pub removals_within: Vec<StripRemoval>,
}

pub struct PageBlame {
    pub lines: Vec<StripLine>,
    pub info: BlameInfo,
    pub chunks: Vec<LinesChunk>,
}

struct Indexer {
    info: BlameInfo,
    commit_indices: HashMap<String, usize>,
    author_indices: HashMap<String, usize>,
    path_indices: HashMap<String, usize>,
}

impl Indexer {
    fn commit(&mut self, rev: &str, meta: &mut impl FnMut(&str) -> CommitMeta) -> usize {
        if let Some(&index) = self.commit_indices.get(rev) {
            return index;
        }
        let CommitMeta { time, author } = meta(rev);
        let author_index = match self.author_indices.get(&author) {
            Some(&index) => index,
            None => {
                let index = self.info.authors.len();
                self.info.authors.push(author.clone());
                self.author_indices.insert(author, index);
                index
            }
        };
        let index = self.info.commits.len();
        self.info
            .commits
            .push((rev.to_string(), time, author_index));
        self.commit_indices.insert(rev.to_string(), index);
        index
    }

    fn path(&mut self, path: &str) -> usize {
        if let Some(&index) = self.path_indices.get(path) {
            return index;
        }
        let index = self.info.paths.len();
        self.info.paths.push(path.to_string());
        self.path_indices.insert(path.to_string(), index);
        index
    }

    fn removal(
        &mut self,
        marker: &RemovalMarker,
        meta: &mut impl FnMut(&str) -> CommitMeta,
    ) -> StripRemoval {
        StripRemoval {
            commit: self.commit(&marker.source_rev, meta),
            path: self.path(&marker.path),
            first_token: marker.lineno,
            num_removed: marker.num_removed,
            num_moved: marker.num_moved,
        }
    }
}

/// Coalesce sorted token indices into ranges.
fn token_ranges(mut indices: Vec<u32>) -> Vec<RangeInclusive<u32>> {
    indices.sort_unstable();
    indices.dedup();
    let mut ranges: Vec<RangeInclusive<u32>> = vec![];
    for index in indices {
        match ranges.last_mut() {
            Some(range) if *range.end() + 1 == index => *range = *range.start()..=index,
            _ => ranges.push(index..=index),
        }
    }
    ranges
}

/// Build the page's token blame for the file at `path` whose contents are
/// `source`.  `meta` provides the metadata of the commits the blame refers to.
pub fn page_blame(
    blame: &FileTokenBlame,
    source: &str,
    path: &str,
    meta: impl FnMut(&str) -> CommitMeta,
) -> PageBlame {
    page_blame_with_chunk_tokens(blame, source, path, meta, CHUNK_TOKENS)
}

fn page_blame_with_chunk_tokens(
    blame: &FileTokenBlame,
    source: &str,
    path: &str,
    mut meta: impl FnMut(&str) -> CommitMeta,
    chunk_size: usize,
) -> PageBlame {
    let mut indexer = Indexer {
        info: BlameInfo {
            commits: vec![],
            authors: vec![],
            paths: vec![],
            token_counts: vec![],
            chunks: vec![],
            data_url: None,
        },
        commit_indices: HashMap::new(),
        author_indices: HashMap::new(),
        path_indices: HashMap::new(),
    };
    indexer.path(path);

    // `FileTokenBlame::lines` orders revisions by commit time, so learn the
    // times as part of assigning commit indices.
    let line_blames = blame.lines(|rev| {
        let index = indexer.commit(rev, &mut meta);
        indexer.info.commits[index].1
    });

    let mut lines = Vec::with_capacity(line_blames.len());
    let mut chunks: Vec<LinesChunk> = vec![];
    let mut chunk_tokens = 0;
    // The last LINENO in the chunk for each commit and path, for deltas.
    let mut last_linenos: HashMap<(usize, usize), u32> = HashMap::new();
    for (line_index, line_blame) in line_blames.iter().enumerate() {
        let tokens = &blame.tokens[line_blame.tokens.clone()];
        indexer.info.token_counts.push(tokens.len() as u32);

        if chunks.is_empty() || chunk_tokens >= chunk_size {
            indexer.info.chunks.push(line_index);
            chunks.push(LinesChunk {
                first_line: line_index,
                lines: vec![],
                preds: BTreeMap::new(),
            });
            chunk_tokens = 0;
            last_linenos.clear();
        }
        let chunk = chunks.last_mut().unwrap();
        chunk_tokens += tokens.len();

        // Group the line's tokens by commit and path for the strip, and
        // describe them for the popup.
        let mut groups: Vec<(usize, usize, Vec<u32>)> = vec![];
        let mut chunk_line = Vec::with_capacity(tokens.len());
        let mut prev_end = blame.line_starts[line_index];
        for (offset, token) in tokens.iter().enumerate() {
            let introduced = &token.data.introduced;
            let commit = indexer.commit(&introduced.source_rev, &mut meta);
            let path = indexer.path(&introduced.path);
            match groups.iter_mut().find(|g| g.0 == commit && g.1 == path) {
                Some(group) => group.2.push(introduced.lineno),
                None => groups.push((commit, path, vec![introduced.lineno])),
            }

            let last_lineno = last_linenos
                .insert((commit, path), introduced.lineno)
                .unwrap_or(0);
            let mut desc = vec![
                utf16_len(&source[prev_end..token.range.start]) as i64,
                utf16_len(&source[token.range.clone()]) as i64,
                commit as i64,
                introduced.lineno as i64 - last_lineno as i64,
            ];
            if path != 0 {
                desc.push(path as i64);
            }
            chunk_line.push(desc);
            prev_end = token.range.end;

            if let Some(predecessor) = &token.data.predecessor {
                let token_index = (line_blame.tokens.start + offset + 1) as u32;
                chunk.preds.insert(
                    token_index,
                    (
                        indexer.commit(&predecessor.source_rev, &mut meta),
                        indexer.path(&predecessor.path),
                        predecessor.lineno,
                    ),
                );
            }
        }
        chunk.lines.push(chunk_line);
        let mut entries: Vec<StripEntry> = groups
            .into_iter()
            .map(|(commit, path, indices)| StripEntry {
                commit,
                path,
                tokens: token_ranges(indices),
            })
            .collect();
        let mut removals_within: Vec<StripRemoval> = line_blame
            .removals_within
            .iter()
            .map(|marker| indexer.removal(marker, &mut meta))
            .collect();
        let commits = &indexer.info.commits;
        let newest_first =
            |a: usize, b: usize| (commits[b].1, &commits[b].0).cmp(&(commits[a].1, &commits[a].0));
        entries.sort_by(|a, b| newest_first(a.commit, b.commit));
        removals_within.sort_by(|a, b| newest_first(a.commit, b.commit));

        lines.push(StripLine {
            commit: line_blame.rev.map(|rev| indexer.commit(rev, &mut meta)),
            interpolated: None,
            entries,
            removal_above: None,
            removal_below: line_blame
                .removal_below
                .map(|marker| indexer.removal(marker, &mut meta)),
            removals_within,
        });
    }
    if let (Some(first), Some(marker)) = (lines.first_mut(), blame.removal_above()) {
        first.removal_above = Some(indexer.removal(marker, &mut meta));
    }
    interpolate_blank_lines(&mut lines);

    PageBlame {
        lines,
        info: indexer.info,
        chunks,
    }
}

/// Give runs of lines without tokens (blank lines) a commit so that they don't
/// look like they're significant: the commit of the lines on both sides of
/// them if those are the same, otherwise the line above's, which puts the
/// color change at the start of the next line with tokens.
fn interpolate_blank_lines(lines: &mut [StripLine]) {
    let interpolate_run = |run: &mut [StripLine], above: Option<usize>, below: Option<usize>| {
        let (commit, how) = match (above, below) {
            (Some(above), Some(below)) if above == below => (above, Interpolation::Between),
            (Some(above), _) => (above, Interpolation::Above),
            (None, Some(below)) => (below, Interpolation::Below),
            (None, None) => return,
        };
        for line in run {
            line.commit = Some(commit);
            line.interpolated = Some(how);
        }
    };
    let mut above = None;
    let mut run_start = None;
    for i in 0..lines.len() {
        if lines[i].entries.is_empty() {
            run_start.get_or_insert(i);
            continue;
        }
        let commit = lines[i].commit;
        if let Some(start) = run_start.take() {
            interpolate_run(&mut lines[start..i], above, commit);
        }
        above = commit;
    }
    if let Some(start) = run_start {
        interpolate_run(&mut lines[start..], above, None);
    }
}

impl StripEntry {
    /// "COMMIT:PATH:TOKENS" where TOKENS is comma-separated ranges like
    /// "5-7,9".
    fn write_attr(&self, out: &mut String) {
        write!(out, "{}:{}:", self.commit, self.path).unwrap();
        for (i, range) in self.tokens.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if range.start() == range.end() {
                write!(out, "{}", range.start()).unwrap();
            } else {
                write!(out, "{}-{}", range.start(), range.end()).unwrap();
            }
        }
    }
}

impl StripRemoval {
    /// "COMMIT:PATH:FIRST_TOKEN:NUM_REMOVED:NUM_MOVED".
    fn write_attr(&self, out: &mut String) {
        write!(
            out,
            "{}:{}:{}:{}:{}",
            self.commit, self.path, self.first_token, self.num_removed, self.num_moved
        )
        .unwrap();
    }
}

impl StripLine {
    /// The classes and data attributes for the line's `.blame-strip` element,
    /// other than the alternating color class and accessibility attributes.
    ///
    /// Classes: `bc-N` names the line's commit, `scar-above`, `scar-below`, and
    /// `scar-within` mark removals (with `sa-N`, `sb-N`, and `sw-N` naming the
    /// newest removal's commit), and `blame-interpolated` marks lines colored
    /// by their neighbors.
    ///
    /// Data attributes: `data-hyperblame` has the line's entries separated by
    /// ";" (see `StripEntry::write_attr`), `data-rm-above`, `data-rm-below`,
    /// and `data-rm-within` have removals separated by ";" (see
    /// `StripRemoval::write_attr`), and `data-interp` is "above" or "below" for
    /// lines colored by only one of their neighbors.
    pub fn strip_attrs(&self) -> (String, String) {
        let mut classes = String::new();
        let mut data = String::new();
        if let Some(commit) = self.commit {
            write!(classes, " bc-{}", commit).unwrap();
        }
        if self.interpolated.is_some() {
            classes.push_str(" blame-interpolated");
        }
        let removal_classes = [
            ("sa", "scar-above", self.removal_above.as_ref()),
            ("sb", "scar-below", self.removal_below.as_ref()),
            ("sw", "scar-within", self.removals_within.first()),
        ];
        for (prefix, class, removal) in removal_classes {
            if let Some(removal) = removal {
                write!(classes, " {} {}-{}", class, prefix, removal.commit).unwrap();
            }
        }

        data.push_str(r#" data-hyperblame=""#);
        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                data.push(';');
            }
            entry.write_attr(&mut data);
        }
        data.push('"');
        match self.interpolated {
            Some(Interpolation::Above) => data.push_str(r#" data-interp="above""#),
            Some(Interpolation::Below) => data.push_str(r#" data-interp="below""#),
            _ => {}
        }
        let removal_attrs = [
            ("above", self.removal_above.iter().collect::<Vec<_>>()),
            ("below", self.removal_below.iter().collect()),
            ("within", self.removals_within.iter().collect()),
        ];
        for (name, removals) in removal_attrs {
            if removals.is_empty() {
                continue;
            }
            write!(data, r#" data-rm-{}=""#, name).unwrap();
            for (i, removal) in removals.iter().enumerate() {
                if i > 0 {
                    data.push(';');
                }
                removal.write_attr(&mut data);
            }
            data.push('"');
        }
        (classes, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunks() {
        use crate::file_format::history::timeline_annotated::{HyperLineData, HyperTokenRef};
        use crate::hyperblame::token_blame::{FileHistory, blame_tokens};
        use crate::tree_sitter_support::cst_tokenizer::hypertokenize_source_file;

        // "é" is 2 bytes of UTF-8 but 1 UTF-16 code unit.
        let source = "int x;\n\n  s = \"é\" + x;\n";
        let tokenized = hypertokenize_source_file("a.cpp", source).unwrap();
        // int x ; | s = "é" + x ;
        let revs = ["1", "1", "1", "2", "2", "2", "1", "2", "2"];
        assert_eq!(tokenized.tokenized.len(), revs.len());
        let mut annotated = vec![HyperLineData::new_introduced("0", 0).serialize()];
        for (i, rev) in revs.iter().enumerate() {
            let mut data = HyperLineData::new_introduced(rev, i as u32 + 1);
            if i == 3 {
                data.predecessor = Some(HyperTokenRef {
                    source_rev: "1".into(),
                    path: "old.cpp".into(),
                    lineno: 9,
                });
            }
            annotated.push(data.serialize());
        }
        let history = FileHistory {
            path: "a.cpp".to_string(),
            source_rev: git2::Oid::ZERO_SHA1,
            lang: "cpp".to_string(),
            syntax: tokenized.tokenized.join("\n"),
            annotated: annotated.join("\n"),
        };
        let blame = blame_tokens(source, &history).unwrap();
        let meta = |rev: &str| CommitMeta {
            time: rev.parse().unwrap(),
            author: format!("author {}", rev),
        };

        let page = page_blame(&blame, source, "a.cpp", meta);
        assert_eq!(page.info.chunks, vec![0]);
        let commit = |rev: &str| page.info.commits.iter().position(|c| c.0 == rev).unwrap() as i64;
        let (c1, c2) = (commit("1"), commit("2"));
        assert_eq!(
            page.chunks,
            vec![LinesChunk {
                first_line: 0,
                lines: vec![
                    vec![vec![0, 3, c1, 1], vec![1, 1, c1, 1], vec![0, 1, c1, 1]],
                    vec![],
                    vec![
                        vec![2, 1, c2, 4],
                        vec![1, 1, c2, 1],
                        vec![1, 3, c2, 1],
                        vec![1, 1, c1, 4],
                        vec![1, 1, c2, 2],
                        vec![0, 1, c2, 1],
                    ],
                ],
                preds: BTreeMap::from([(4, (c1 as usize, 1, 9))]),
            }]
        );
        assert_eq!(page.info.paths, vec!["a.cpp", "old.cpp"]);

        // With tiny chunks, each chunk starts at a line boundary once the
        // previous one has enough tokens, and deltas start over.
        let page = page_blame_with_chunk_tokens(&blame, source, "a.cpp", meta, 3);
        assert_eq!(page.info.chunks, vec![0, 1]);
        assert_eq!(page.chunks[1].first_line, 1);
        assert_eq!(page.chunks[1].lines[1][0], vec![2, 1, c2, 4]);
        assert_eq!(page.chunks[1].lines[1][3], vec![1, 1, c1, 7]);
    }

    #[test]
    fn test_token_ranges() {
        assert_eq!(token_ranges(vec![]), vec![]);
        assert_eq!(token_ranges(vec![7, 5, 6, 9, 9]), vec![5..=7, 9..=9]);
    }

    fn line(commit: Option<usize>) -> StripLine {
        StripLine {
            commit,
            entries: commit
                .map(|commit| StripEntry {
                    commit,
                    path: 0,
                    tokens: vec![1..=1],
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_interpolate_blank_lines() {
        let mut lines: Vec<StripLine> = [None, Some(1), None, None, Some(1), None, Some(2), None]
            .into_iter()
            .map(line)
            .collect();
        interpolate_blank_lines(&mut lines);
        use Interpolation::*;
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.commit, l.interpolated))
                .collect::<Vec<_>>(),
            vec![
                (Some(1), Some(Below)),
                (Some(1), None),
                (Some(1), Some(Between)),
                (Some(1), Some(Between)),
                (Some(1), None),
                (Some(1), Some(Above)),
                (Some(2), None),
                (Some(2), Some(Above)),
            ]
        );

        // A file without any tokens has nothing to interpolate from.
        let mut lines: Vec<StripLine> = [None, None].into_iter().map(line).collect();
        interpolate_blank_lines(&mut lines);
        assert!(
            lines
                .iter()
                .all(|l| l.commit.is_none() && l.interpolated.is_none())
        );
    }

    #[test]
    fn test_strip_attrs() {
        let removal = |commit| StripRemoval {
            commit,
            path: 0,
            first_token: 10,
            num_removed: 3,
            num_moved: 1,
        };
        let line = StripLine {
            commit: Some(2),
            interpolated: None,
            entries: vec![
                StripEntry {
                    commit: 2,
                    path: 0,
                    tokens: vec![5..=7, 9..=9],
                },
                StripEntry {
                    commit: 0,
                    path: 1,
                    tokens: vec![3..=3],
                },
            ],
            removal_above: None,
            removal_below: Some(removal(4)),
            removals_within: vec![removal(3), removal(1)],
        };
        assert_eq!(
            line.strip_attrs(),
            (
                " bc-2 scar-below sb-4 scar-within sw-3".to_string(),
                concat!(
                    r#" data-hyperblame="2:0:5-7,9;0:1:3""#,
                    r#" data-rm-below="4:0:10:3:1""#,
                    r#" data-rm-within="3:0:10:3:1;1:0:10:3:1""#
                )
                .to_string()
            )
        );
    }
}
