//! Token-centric blame for presenting a revision of a file: pairs the file's
//! tokens with the timeline repo's "annotated" records for them and summarizes
//! them per source line for the blame strip.
//!
//! The history doesn't record where tokens are in the source (that way a
//! file's syntax and annotated representations only change when its tokens
//! do), so we re-tokenize the source to recover the tokens' byte offsets.  We
//! check that the tokens match the syntax repo's "files" representation, which
//! catches tokenizer changes on histories that weren't regenerated, and callers
//! should fall back to classic blame for any error.
//!
//! See "Token blame UI plan" in the hyperblame notes for how this is used.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::ops::Range;
use std::path::Path;

use git2::{Commit, Oid, Repository};

use crate::file_format::config::{ThreadLocalRepository, timeline_commit_to_meta};
use crate::file_format::history::syntax_files::{split_token_line, token_file_lines};
use crate::file_format::history::syntax_files_struct::FileStructureHeader;
use crate::file_format::history::timeline_annotated::{HyperLineData, RemovalMarker};
use crate::source_mapping::{NotesRefs, SourceMapping, default_notes_ref};
use crate::tree_sitter_support::cst_tokenizer::{hypertokenize_with_profile, profile_for_lang};

/// A tree's history repos (see `TreeConfigPaths::history_path`) for its branch.
pub struct TreeHistory {
    /// The directory with the history's repos and rev-summaries.
    pub path: String,
    pub syntax: ThreadLocalRepository,
    pub timeline: ThreadLocalRepository,
    branch_ref: String,
    mapping: SourceMapping,
}

impl TreeHistory {
    /// Open the history at `history_path` for `git_branch`, or HEAD if None.
    pub fn open(history_path: &str, git_branch: Option<&str>) -> Result<TreeHistory, git2::Error> {
        let syntax = Repository::open(format!("{}/syntax", history_path))?;
        let timeline = Repository::open(format!("{}/timeline", history_path))?;
        let branch_ref = git_branch.map_or("HEAD".to_string(), |branch| {
            format!("refs/heads/{}", branch)
        });
        let notes_refs = NotesRefs {
            write: default_notes_ref(&timeline, &branch_ref),
            read: vec![],
        };
        let mapping = SourceMapping::open(&timeline, &notes_refs);
        Ok(TreeHistory {
            path: history_path.to_string(),
            syntax: syntax.into(),
            timeline: timeline.into(),
            branch_ref,
            mapping,
        })
    }

    /// The timeline commit derived from `source_rev`, if the history has it.
    pub fn timeline_commit(&self, source_rev: Oid) -> Option<Commit<'_>> {
        let timeline_rev = self.mapping.lookup(&self.timeline, source_rev)?;
        self.timeline.find_commit(timeline_rev).ok()
    }

    /// The timeline commit at the head of the branch.
    pub fn head_timeline_commit(&self) -> Option<Commit<'_>> {
        let timeline_rev = self.timeline.refname_to_id(&self.branch_ref).ok()?;
        self.timeline.find_commit(timeline_rev).ok()
    }

    /// The history representation of `path` as of `timeline_commit`, or None
    /// if the history doesn't have the file (ex: binary files, or files the
    /// history configuration excludes).
    pub fn file_history(
        &self,
        timeline_commit: &Commit,
        path: &str,
    ) -> Result<Option<FileHistory>, TokenBlameError> {
        let meta = timeline_commit_to_meta(timeline_commit);
        let syntax_tree = self.syntax.find_commit(meta.syntax_rev)?.tree()?;
        let timeline_tree = timeline_commit.tree()?;

        let Some(files_struct) = read_path(
            &self.syntax,
            &syntax_tree,
            &format!("files-struct/{}", path),
        )?
        else {
            return Ok(None);
        };
        let header_line = files_struct.lines().next().unwrap_or_default();
        let header: FileStructureHeader = serde_json::from_str(header_line)
            .map_err(|e| TokenBlameError::BadHistory(format!("files-struct header: {}", e)))?;
        let lang = header
            .lang
            .ok_or_else(|| TokenBlameError::BadHistory("files-struct has no lang".to_string()))?;

        let syntax = read_path(&self.syntax, &syntax_tree, &format!("files/{}", path))?;
        let annotated = read_path(
            &self.timeline,
            &timeline_tree,
            &format!("annotated/{}", path),
        )?;
        match (syntax, annotated) {
            (Some(syntax), Some(annotated)) => Ok(Some(FileHistory {
                path: path.to_string(),
                source_rev: meta.source_rev,
                lang,
                syntax,
                annotated,
            })),
            _ => Err(TokenBlameError::BadHistory(format!(
                "files-struct/{} without its files or annotated file",
                path
            ))),
        }
    }
}

fn read_path(
    repo: &Repository,
    tree: &git2::Tree,
    path: &str,
) -> Result<Option<String>, TokenBlameError> {
    let entry = match tree.get_path(Path::new(path)) {
        Ok(entry) => entry,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let blob = repo.find_blob(entry.id())?;
    let text = std::str::from_utf8(blob.content())
        .map_err(|e| TokenBlameError::BadHistory(format!("{} isn't UTF-8: {}", path, e)))?;
    Ok(Some(text.to_string()))
}

/// The history representation of a file in one revision.
pub struct FileHistory {
    pub path: String,
    /// The source revision the history describes.
    pub source_rev: Oid,
    /// The language the file was tokenized as.
    pub lang: String,
    /// The `files/PATH` contents from the syntax repo.
    pub syntax: String,
    /// The `annotated/PATH` contents from the timeline repo.
    pub annotated: String,
}

#[derive(Debug)]
pub enum TokenBlameError {
    Git(git2::Error),
    /// The history's data isn't what we expect.
    BadHistory(String),
    /// The file's language has no tokenizer (anymore).
    UnknownLang(String),
    /// The tokenizer failed on the source.
    Tokenize(String),
    /// The source's tokens don't match the history's, as of the given token
    /// index (which is the token count if the counts differ).
    TokenMismatch(usize),
}

impl fmt::Display for TokenBlameError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TokenBlameError::Git(e) => write!(f, "git error: {}", e),
            TokenBlameError::BadHistory(msg) => write!(f, "bad history: {}", msg),
            TokenBlameError::UnknownLang(lang) => write!(f, "unknown language {:?}", lang),
            TokenBlameError::Tokenize(msg) => write!(f, "tokenizing failed: {}", msg),
            TokenBlameError::TokenMismatch(index) => {
                write!(
                    f,
                    "the tokens don't match the history as of token {}",
                    index
                )
            }
        }
    }
}

impl From<git2::Error> for TokenBlameError {
    fn from(e: git2::Error) -> Self {
        TokenBlameError::Git(e)
    }
}

/// A token of the source with its history.
#[derive(Debug)]
pub struct BlamedToken<'a> {
    /// The token's byte range in the source.
    pub range: Range<usize>,
    /// The (0-based) source line the token is on.  Tokens never span lines.
    pub line: usize,
    /// The token's history, with paths resolved (no "%").
    pub data: HyperLineData<'a>,
}

/// The tokens of a file with their history.
#[derive(Debug)]
pub struct FileTokenBlame<'a> {
    /// The history of the file sentinel, which hosts removals from the start
    /// of the file.
    pub sentinel: HyperLineData<'a>,
    pub tokens: Vec<BlamedToken<'a>>,
    /// The byte offset of the start of each source line.  Lines are separated
    /// by "\n" and a trailing "\n" doesn't start another line, matching
    /// `format_code`.
    pub line_starts: Vec<usize>,
}

/// Pair the tokens of `source`, which must be the contents of the file in the
/// revision `history` describes, with their history.
pub fn blame_tokens<'a>(
    source: &str,
    history: &'a FileHistory,
) -> Result<FileTokenBlame<'a>, TokenBlameError> {
    let profile = profile_for_lang(&history.lang)
        .ok_or_else(|| TokenBlameError::UnknownLang(history.lang.clone()))?;
    let tokenized =
        hypertokenize_with_profile(profile, source).map_err(TokenBlameError::Tokenize)?;

    let syntax_lines = token_file_lines(&history.syntax);
    if syntax_lines.len() != tokenized.tokenized.len() {
        return Err(TokenBlameError::TokenMismatch(
            syntax_lines.len().min(tokenized.tokenized.len()),
        ));
    }
    // The annotated file has a line for the sentinel and then one per token.
    let mut annotated_lines = token_file_lines(&history.annotated).into_iter();
    if annotated_lines.len() != syntax_lines.len() + 1 {
        return Err(TokenBlameError::BadHistory(format!(
            "{} tokens but {} annotated lines",
            syntax_lines.len(),
            annotated_lines.len()
        )));
    }
    let parse = |line: &'a str| -> Result<HyperLineData<'a>, TokenBlameError> {
        let mut data: HyperLineData = serde_json::from_str(line)
            .map_err(|e| TokenBlameError::BadHistory(format!("annotated line: {}", e)))?;
        data.introduced.resolve_path(&history.path);
        if let Some(predecessor) = &mut data.predecessor {
            predecessor.resolve_path(&history.path);
        }
        if let Some(marker) = &mut data.removal_marker {
            marker.resolve_path(&history.path);
        }
        Ok(data)
    };
    let sentinel = parse(annotated_lines.next().unwrap())?;

    let line_starts = line_starts(source);
    let mut tokens = Vec::with_capacity(syntax_lines.len());
    for (index, ((ours, theirs), annotated)) in tokenized
        .tokenized
        .iter()
        .zip(syntax_lines)
        .zip(annotated_lines)
        .enumerate()
    {
        let token = split_token_line(ours).token;
        if token != split_token_line(theirs).token {
            return Err(TokenBlameError::TokenMismatch(index));
        }
        let start = tokenized.offsets[index].ok_or(TokenBlameError::TokenMismatch(index))? as usize;
        tokens.push(BlamedToken {
            range: start..start + token.len(),
            line: line_starts.partition_point(|&line_start| line_start <= start) - 1,
            data: parse(annotated)?,
        });
    }

    Ok(FileTokenBlame {
        sentinel,
        tokens,
        line_starts,
    })
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    // A trailing newline (or an empty source) doesn't start a line.
    if starts.last() == Some(&source.len()) {
        starts.pop();
    }
    starts
}

/// What the blame strip shows for one source line.
#[derive(Debug, Default)]
pub struct LineBlame<'b> {
    /// The range of `FileTokenBlame::tokens` on the line.
    pub tokens: Range<usize>,
    /// The newest revision to have changed the line's tokens, or None if the
    /// line has no tokens.
    pub rev: Option<&'b str>,
    /// Whether more than one revision changed the line's tokens.
    pub mixed: bool,
    /// A removal between the end of this line and the next line with tokens
    /// (or the end of the file).
    pub removal_below: Option<&'b RemovalMarker<'b>>,
    /// Removals between the tokens of this line.
    pub removals_within: Vec<&'b RemovalMarker<'b>>,
}

impl<'a> FileTokenBlame<'a> {
    /// A removal at the start of the file, before its first token.
    pub fn removal_above(&self) -> Option<&RemovalMarker<'a>> {
        self.sentinel.removal_marker.as_ref()
    }

    /// Summarize the tokens per source line.  `commit_time` provides the
    /// commit time of a revision, for picking the newest revision of a line.
    pub fn lines<'b>(&'b self, mut commit_time: impl FnMut(&str) -> i64) -> Vec<LineBlame<'b>> {
        let mut lines: Vec<LineBlame> = (0..self.line_starts.len())
            .map(|_| LineBlame::default())
            .collect();
        let mut revs = HashSet::new();
        let mut newest: Option<(i64, &str)> = None;
        for (index, token) in self.tokens.iter().enumerate() {
            let line = &mut lines[token.line];
            if line.tokens.is_empty() {
                line.tokens = index..index;
                revs.clear();
                newest = None;
            }
            line.tokens.end = index + 1;

            let rev: &str = &token.data.introduced.source_rev;
            if revs.insert(rev) {
                let candidate = (commit_time(rev), rev);
                if newest.is_none_or(|newest| candidate.cmp(&newest) == Ordering::Greater) {
                    newest = Some(candidate);
                }
            }
            line.rev = newest.map(|(_, rev)| rev);
            line.mixed = revs.len() > 1;

            if let Some(marker) = &token.data.removal_marker {
                let next_line = self.tokens.get(index + 1).map(|next| next.line);
                if next_line == Some(token.line) {
                    line.removals_within.push(marker);
                } else {
                    line.removal_below = Some(marker);
                }
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_annotated::HyperTokenRef;
    use crate::tree_sitter_support::cst_tokenizer::hypertokenize_source_file;

    #[test]
    fn test_line_starts() {
        assert_eq!(line_starts(""), Vec::<usize>::new());
        assert_eq!(line_starts("a"), vec![0]);
        assert_eq!(line_starts("a\n"), vec![0]);
        assert_eq!(line_starts("a\n\nb"), vec![0, 2, 3]);
        assert_eq!(line_starts("a\n\n"), vec![0, 2]);
    }

    /// A history for `source` where token `i` was introduced by the revision
    /// `revs[i]` and `markers` maps token indices (0 for the sentinel, token
    /// `i` is `i + 1`) to (revision, number removed).
    fn history(source: &str, revs: &[&str], markers: &[(usize, &str, u32)]) -> FileHistory {
        let tokenized = hypertokenize_source_file("a.cpp", source).unwrap();
        assert_eq!(
            tokenized.tokenized.len(),
            revs.len(),
            "{:?}",
            tokenized.tokenized
        );
        let mut annotated = vec![];
        for (line, rev) in std::iter::once("0").chain(revs.iter().copied()).enumerate() {
            let mut data = HyperLineData::new_introduced(rev, line as u32);
            if let Some((_, rm_rev, num_removed)) = markers.iter().find(|m| m.0 == line) {
                data.removal_marker = Some(RemovalMarker {
                    source_rev: (*rm_rev).into(),
                    path: "%".into(),
                    lineno: 1,
                    first_removed: HyperTokenRef::new_unchanged_path("0", 1),
                    num_removed: *num_removed,
                    num_moved: 0,
                });
            }
            annotated.push(data.serialize());
        }
        FileHistory {
            path: "a.cpp".to_string(),
            source_rev: Oid::ZERO_SHA1,
            lang: "cpp".to_string(),
            syntax: tokenized.tokenized.join("\n"),
            annotated: annotated.join("\n"),
        }
    }

    #[test]
    fn test_blame_tokens() {
        let source = "int x;\n\nint y = f(x);\n";
        // int x ; | int y = f ( x ) ;
        let revs = ["1", "1", "1", "2", "3", "2", "2", "2", "3", "2", "2"];
        let history = history(source, &revs, &[(0, "4", 3), (3, "5", 2), (8, "6", 1)]);
        let blame = blame_tokens(source, &history).unwrap();
        assert_eq!(
            blame
                .tokens
                .iter()
                .map(|t| (&source[t.range.clone()], t.line))
                .collect::<Vec<_>>(),
            vec![
                ("int", 0),
                ("x", 0),
                (";", 0),
                ("int", 2),
                ("y", 2),
                ("=", 2),
                ("f", 2),
                ("(", 2),
                ("x", 2),
                (")", 2),
                (";", 2),
            ]
        );
        assert_eq!(blame.tokens[0].data.introduced.path, "a.cpp");
        assert_eq!(blame.removal_above().unwrap().source_rev, "4");

        // Revision N is newer than revision N-1.
        let lines = blame.lines(|rev| rev.parse().unwrap());
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].tokens, 0..3);
        assert_eq!(lines[0].rev, Some("1"));
        assert!(!lines[0].mixed);
        // The ";" hosts a removal before the next token, on a later line.
        assert_eq!(lines[0].removal_below.unwrap().source_rev, "5");
        assert!(lines[0].removals_within.is_empty());

        assert_eq!(lines[1].tokens, 0..0);
        assert_eq!(lines[1].rev, None);

        assert_eq!(lines[2].tokens, 3..11);
        assert_eq!(lines[2].rev, Some("3"));
        assert!(lines[2].mixed);
        assert!(lines[2].removal_below.is_none());
        assert_eq!(lines[2].removals_within.len(), 1);
        assert_eq!(lines[2].removals_within[0].source_rev, "6");
    }

    #[test]
    fn test_blame_tokens_mismatch() {
        let history = history("int x;", &["1", "1", "1"], &[]);
        assert!(matches!(
            blame_tokens("int y;", &history),
            Err(TokenBlameError::TokenMismatch(1))
        ));
        assert!(matches!(
            blame_tokens("int x; y", &history),
            Err(TokenBlameError::TokenMismatch(3))
        ));
        let bad_lang = FileHistory {
            lang: "cobol".to_string(),
            ..history
        };
        assert!(matches!(
            blame_tokens("int x;", &bad_lang),
            Err(TokenBlameError::UnknownLang(_))
        ));
    }
}
