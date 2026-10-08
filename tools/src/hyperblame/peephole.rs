//! The "peephole" history of a token: how the window of tokens around it (ex:
//! the enclosing `if (...)` condition or statement) changed over time, for the
//! blame popup's "follow this token into the past".  See "Peephole history" in
//! the hyperblame notes.
//!
//! Each step is the newest commit to have changed any of the window's tokens,
//! with the window as of that commit.  The next step starts from the commit's
//! (first) parent, anchored on the anchor token's predecessor if the commit
//! introduced the anchor, or else the nearest window token that already
//! existed.  The history ends when the commit introduced the whole window.
//!
//! Steps are meant to be cheap compared to rendering a revision of the file:
//! they read the file's syntax, annotated, and source blobs, find the window
//! with a heuristic over the syntax tokens rather than parsing the source, only
//! parse the window's annotated records, and find the anchor in the parent's
//! annotated file with a substring search.  Pages of steps are limited by the
//! number of steps and the bytes of blobs read.

use std::collections::HashMap;
use std::ops::Range;

use git2::{Oid, Repository};
use serde::Serialize;

use super::token_blame::{FileHistory, TreeHistory};
use crate::file_format::history::syntax_files::{
    TokenClass, TokenLine, split_token_line, token_file_lines,
};
use crate::file_format::history::timeline_annotated::{HyperLineData, HyperTokenRef};

/// The most tokens a window has.
pub const MAX_WINDOW_TOKENS: usize = 48;

/// Where a step starts: the anchor tokens, by their (1-based) indices in the
/// file at `path` in `rev`.  Following several tokens (ex: a line) follows
/// their identifiers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cursor {
    pub rev: String,
    pub path: String,
    pub tokens: Vec<u32>,
}

#[derive(Debug, Serialize)]
pub struct Step {
    /// The commit which changed the window.
    pub rev: String,
    /// The revision whose version of the window this is, which may be newer
    /// than `rev`.
    #[serde(rename = "stateRev")]
    pub state_rev: String,
    pub path: String,
    /// The (1-based) index of the window's first token in the file in
    /// `state_rev`, for `#tokens=` links.
    #[serde(rename = "firstToken")]
    pub first_token: u32,
    /// The source text of the window's lines (or its tokens separated by
    /// spaces, if we couldn't find them in the source).
    pub text: String,
    /// The window's tokens as `[START, END, CHANGED]`, where START and END are
    /// UTF-16 offsets in `text` and CHANGED is whether `rev` introduced it.
    pub tokens: Vec<(usize, usize, bool)>,
    /// The anchors' indices in `tokens`.
    pub anchors: Vec<usize>,
    /// How many tokens `rev` removed from within the window.
    pub removed: u32,
}

#[derive(Debug, Serialize)]
pub struct PeepholePage {
    pub steps: Vec<Step>,
    /// Where the next page starts, if the history continues.
    pub next: Option<Cursor>,
    /// Why the history ended, if it did: "introduced" (the last step's commit
    /// introduced the whole window), "root" (the commit has no parent),
    /// "lost" (we couldn't find the anchors in the parent), or "missing" (the
    /// history doesn't have the file in a revision).
    pub end: Option<&'static str>,
    /// The bytes of blobs we read.
    pub cost: usize,
}

/// A revision of the file.
struct State {
    history: FileHistory,
    source: String,
}

impl State {
    fn load(tree_history: &TreeHistory, repo: &Repository, rev: Oid, path: &str) -> Option<State> {
        let timeline_commit = tree_history.timeline_commit(rev)?;
        let history = tree_history.file_history(&timeline_commit, path).ok()??;
        let tree = repo.find_commit(rev).ok()?.tree().ok()?;
        let entry = tree.get_path(std::path::Path::new(path)).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;
        let source = String::from_utf8_lossy(blob.content()).into_owned();
        Some(State { history, source })
    }

    fn cost(&self) -> usize {
        self.history.syntax.len() + self.history.annotated.len() + self.source.len()
    }

    /// The (0-based) index of the token whose canonical ref is `identity`, by
    /// searching for the start of its annotated line.
    fn find_identity(&self, identity: &HyperTokenRef) -> Option<usize> {
        let mut relative = identity.clone();
        relative.relativize_path(&self.history.path);
        let needle = format!("{{\"i\":{}", serde_json::to_string(&relative).unwrap());
        let annotated = &self.history.annotated;
        let mut from = 0;
        while let Some(found) = annotated[from..].find(&needle) {
            let at = from + found;
            if at == 0 || annotated.as_bytes()[at - 1] == b'\n' {
                // Line 0 is the file sentinel, so line N is token N.
                let line = annotated.as_bytes()[..at]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count();
                return line.checked_sub(1);
            }
            from = at + 1;
        }
        None
    }
}

fn is_open(token: &TokenLine) -> bool {
    matches!(token.token, "(" | "[" | "{")
}

fn is_close(token: &TokenLine) -> bool {
    matches!(token.token, ")" | "]" | "}")
}

/// The window of tokens around `anchor`: the run of comment or text words
/// around it, or else the statement containing it (ex: `if (a && b) {` or
/// `foo(x, y);`).  If the statement is too long, it's narrowed to the innermost
/// bracketed group around the anchor, or the comma-separated item of that
/// group containing the anchor (ex: a parameter), or just the tokens around
/// the anchor.  Windows never cross a change of context (ex: into another
/// function).
pub fn window(tokens: &[TokenLine], anchor: usize) -> Range<usize> {
    let context = tokens[anchor].context;
    let is_prose = |i: usize| {
        matches!(
            tokens[i].effective_class(),
            TokenClass::Comment | TokenClass::Text | TokenClass::Boilerplate
        )
    };
    // Statements don't include comments.
    let same_context = |i: usize| tokens[i].context == context && !is_prose(i);
    let class = tokens[anchor].effective_class();
    if is_prose(anchor) {
        let prose = |i: usize| tokens[i].effective_class() == class && tokens[i].context == context;
        let mut start = anchor;
        while start > 0 && prose(start - 1) && anchor - start < MAX_WINDOW_TOKENS / 2 {
            start -= 1;
        }
        let mut end = anchor + 1;
        while end < tokens.len() && prose(end) && end - start < MAX_WINDOW_TOKENS {
            end += 1;
        }
        return start..end;
    }

    // Scan backward to the start of the statement.
    let mut start = anchor;
    let mut depth = 0;
    while start > 0 && same_context(start - 1) && anchor - start < MAX_WINDOW_TOKENS * 2 {
        let token = &tokens[start - 1];
        if is_close(token) {
            if depth == 0 && token.token == "}" {
                break;
            }
            depth += 1;
        } else if is_open(token) {
            if depth > 0 {
                depth -= 1;
            } else if token.token == "{" {
                break;
            }
        } else if depth == 0 && token.token == ";" {
            break;
        }
        start -= 1;
    }
    // Scan forward to the end of the statement.
    let mut end = anchor + 1;
    let mut depth = 0;
    while end < tokens.len() && same_context(end) && end - anchor < MAX_WINDOW_TOKENS * 2 {
        let token = &tokens[end];
        if is_open(token) {
            if depth == 0 && token.token == "{" {
                end += 1;
                break;
            }
            depth += 1;
        } else if is_close(token) {
            if depth > 0 {
                depth -= 1;
            } else if token.token == "}" {
                break;
            }
        } else if depth == 0 && token.token == ";" {
            end += 1;
            break;
        }
        end += 1;
    }
    if end - start <= MAX_WINDOW_TOKENS {
        return start..end;
    }

    // Narrow to the innermost bracketed group around the anchor, which may be
    // longer than the scans above went.
    let reach = MAX_WINDOW_TOKENS * 64;
    let group_start = anchor.saturating_sub(reach);
    let group_end = (anchor + reach).min(tokens.len());
    let mut open = None;
    let mut depth = 0;
    for i in (group_start..anchor).rev() {
        if !same_context(i) {
            break;
        }
        if is_close(&tokens[i]) {
            depth += 1;
        } else if is_open(&tokens[i]) {
            if depth == 0 {
                open = Some(i);
                break;
            }
            depth -= 1;
        }
    }
    if let Some(open) = open {
        let mut close = None;
        let mut depth = 0;
        for (i, token) in tokens.iter().enumerate().take(group_end).skip(open + 1) {
            if !same_context(i) {
                break;
            }
            if is_open(token) {
                depth += 1;
            } else if is_close(token) {
                if depth == 0 {
                    close = Some(i);
                    break;
                }
                depth -= 1;
            }
        }
        let close = close.unwrap_or(anchor + 1);
        if close + 1 - open <= MAX_WINDOW_TOKENS {
            return open..close + 1;
        }
        // The item of the group containing the anchor, including its comma.
        let at_depth_0 = |from: usize, to: usize| {
            let mut depth = 0;
            let mut commas = vec![];
            for (i, token) in tokens.iter().enumerate().take(to).skip(from) {
                if is_open(token) {
                    depth += 1;
                } else if is_close(token) {
                    depth -= 1;
                } else if depth == 0 && token.token == "," {
                    commas.push(i);
                }
            }
            commas
        };
        let commas = at_depth_0(open + 1, close);
        let item_start = commas
            .iter()
            .rev()
            .find(|&&c| c < anchor)
            .map_or(open + 1, |&c| c + 1);
        let item_end = commas
            .iter()
            .find(|&&c| c >= anchor)
            .map_or(close, |&c| c + 1);
        if item_end - item_start <= MAX_WINDOW_TOKENS && item_start <= anchor {
            return item_start..item_end;
        }
    }
    let start = anchor.saturating_sub(MAX_WINDOW_TOKENS / 2).max(start);
    start..(start + MAX_WINDOW_TOKENS).min(end)
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// Compute a page of the peephole history starting at `start`, with at most
/// `max_steps` steps, stopping once we've read `max_cost` bytes of blobs.
pub fn peephole_page(
    tree_history: &TreeHistory,
    repo: &Repository,
    start: Cursor,
    max_steps: usize,
    max_cost: usize,
) -> PeepholePage {
    let mut page = PeepholePage {
        steps: vec![],
        next: None,
        end: None,
        cost: 0,
    };
    let mut commit_times: HashMap<String, i64> = HashMap::new();
    let mut commit_time = |rev: &str| -> i64 {
        *commit_times.entry(rev.to_string()).or_insert_with(|| {
            Oid::from_str(rev)
                .and_then(|oid| repo.find_commit(oid))
                .map_or(0, |commit| commit.time().seconds())
        })
    };

    let mut cursor = start;
    let mut preloaded: Option<State> = None;
    let mut first = true;
    loop {
        if page.steps.len() >= max_steps || page.cost >= max_cost {
            page.next = Some(cursor);
            return page;
        }
        let Ok(rev) = Oid::from_str(&cursor.rev) else {
            page.end = Some("missing");
            return page;
        };
        let state = match preloaded.take() {
            Some(state) => state,
            None => match State::load(tree_history, repo, rev, &cursor.path) {
                Some(state) => {
                    page.cost += state.cost();
                    state
                }
                None => {
                    page.end = Some("missing");
                    return page;
                }
            },
        };

        let tokens: Vec<TokenLine> = token_file_lines(&state.history.syntax)
            .into_iter()
            .map(split_token_line)
            .collect();
        let annotated_lines = token_file_lines(&state.history.annotated);
        if cursor.tokens.is_empty()
            || cursor
                .tokens
                .iter()
                .any(|&t| t == 0 || t as usize > tokens.len())
            || annotated_lines.len() != tokens.len() + 1
        {
            page.end = Some("lost");
            return page;
        }
        let mut anchors: Vec<usize> = cursor.tokens.iter().map(|&t| t as usize - 1).collect();
        anchors.sort_unstable();
        anchors.dedup();
        // Following several tokens follows their identifiers, since
        // punctuation and keywords match loosely and aren't what anyone is
        // interested in.
        if first && anchors.len() > 1 {
            let identifiers: Vec<usize> = anchors
                .iter()
                .copied()
                .filter(|&a| tokens[a].effective_class() == TokenClass::Identifier)
                .collect();
            if !identifiers.is_empty() {
                anchors = identifiers;
            }
        }
        first = false;

        // The window covers all of the anchors' windows.
        let win = anchors
            .iter()
            .map(|&a| window(&tokens, a))
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .unwrap();
        let mut data: Vec<HyperLineData> = Vec::with_capacity(win.len());
        for i in win.clone() {
            let Ok(mut line_data) = serde_json::from_str::<HyperLineData>(annotated_lines[i + 1])
            else {
                page.end = Some("lost");
                return page;
            };
            line_data.introduced.resolve_path(&state.history.path);
            if let Some(predecessor) = &mut line_data.predecessor {
                predecessor.resolve_path(&state.history.path);
            }
            data.push(line_data);
        }

        // The newest commit to have introduced a window token or removed tokens
        // from within the window.
        let mut candidates: Vec<&str> = data
            .iter()
            .map(|d| d.introduced.source_rev.as_ref())
            .collect();
        for d in &data[..data.len() - 1] {
            if let Some(marker) = &d.removal_marker {
                candidates.push(&marker.source_rev);
            }
        }
        let changed_rev = candidates
            .into_iter()
            .max_by_key(|rev| (commit_time(rev), rev.to_string()))
            .unwrap()
            .to_string();

        page.steps.push(step(
            &state,
            &tokens,
            &data,
            &win,
            &anchors,
            &cursor,
            &changed_rev,
        ));

        // What the next step follows: each anchor if the commit didn't change
        // it, else its predecessor if it's the same kind of token (evolutions
        // between kinds are usually wrong), else nothing.  If that leaves
        // nothing, the nearest window token which already existed, but only if
        // enough of the window did.  (Common tokens like punctuation and short
        // words match loosely, so a few tokens which already existed don't mean
        // much.)
        let mut identities: Vec<(HyperTokenRef, Option<TokenClass>)> = vec![];
        for &anchor in &anchors {
            let anchor_data = &data[anchor - win.start];
            if anchor_data.introduced.source_rev != changed_rev {
                identities.push((anchor_data.introduced.clone(), None));
            } else if let Some(predecessor) = &anchor_data.predecessor {
                identities.push((predecessor.clone(), Some(tokens[anchor].effective_class())));
            }
        }
        let words: Vec<usize> = (0..data.len())
            .filter(|&j| {
                tokens[win.start + j]
                    .token
                    .chars()
                    .any(char::is_alphanumeric)
            })
            .collect();
        let old_words = words
            .iter()
            .filter(|&&j| data[j].introduced.source_rev != changed_rev)
            .count();
        let fallback = if old_words * 3 >= words.len() {
            let distance = |j: usize| {
                anchors
                    .iter()
                    .map(|&a| (j as isize - (a - win.start) as isize).abs())
                    .min()
                    .unwrap()
            };
            (0..data.len())
                .filter(|&j| data[j].introduced.source_rev != changed_rev)
                .min_by_key(|&j| distance(j))
                .map(|j| data[j].introduced.clone())
        } else {
            None
        };
        if identities.is_empty() && fallback.is_none() {
            page.end = Some("introduced");
            return page;
        }
        let parent = Oid::from_str(&changed_rev)
            .and_then(|oid| repo.find_commit(oid))
            .ok()
            .and_then(|commit| commit.parent_id(0).ok());
        let Some(parent) = parent else {
            page.end = Some("root");
            return page;
        };

        // Find what we're following in the parent.  It's usually in the same
        // file, but it could have been renamed.
        let mut parent_states: HashMap<String, Option<State>> = HashMap::new();
        let mut locate = |identity: &HyperTokenRef,
                          required_class: Option<TokenClass>,
                          page: &mut PeepholePage|
         -> Option<(String, usize)> {
            let mut paths = vec![cursor.path.clone()];
            if identity.path != cursor.path {
                paths.push(identity.path.to_string());
            }
            for path in paths {
                let parent_state = parent_states.entry(path.clone()).or_insert_with(|| {
                    let state = State::load(tree_history, repo, parent, &path);
                    page.cost += state.as_ref().map_or(0, State::cost);
                    state
                });
                let Some(parent_state) = parent_state else {
                    continue;
                };
                let Some(index) = parent_state.find_identity(identity) else {
                    continue;
                };
                if let Some(required_class) = required_class {
                    let class = token_file_lines(&parent_state.history.syntax)
                        .get(index)
                        .map(|line| split_token_line(line).effective_class());
                    if class != Some(required_class) {
                        continue;
                    }
                }
                return Some((path, index));
            }
            None
        };
        // Everything we follow needs to be in the same file: this one if any of
        // it is still here, else wherever most of it is.
        let located: Vec<(String, u32)> = identities
            .iter()
            .filter_map(|(identity, required_class)| {
                locate(identity, *required_class, &mut page)
                    .map(|(path, index)| (path, index as u32 + 1))
            })
            .collect();
        let mut next_path: Option<String> = if located.iter().any(|(path, _)| *path == cursor.path)
        {
            Some(cursor.path.clone())
        } else {
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for (path, _) in &located {
                *counts.entry(path).or_default() += 1;
            }
            counts
                .into_iter()
                .max_by_key(|&(path, count)| (count, std::cmp::Reverse(path)))
                .map(|(path, _)| path.to_string())
        };
        let mut next_tokens: Vec<u32> = located
            .iter()
            .filter(|(path, _)| Some(path) == next_path.as_ref())
            .map(|&(_, token)| token)
            .collect();
        if next_tokens.is_empty()
            && let Some(fallback) = &fallback
            && let Some((path, index)) = locate(fallback, None, &mut page)
        {
            next_path = Some(path);
            next_tokens.push(index as u32 + 1);
        }
        let Some(path) = next_path else {
            // If we only rejected predecessors, the window was new.
            let rejected_only =
                fallback.is_none() && identities.iter().all(|(_, class)| class.is_some());
            page.end = Some(if rejected_only { "introduced" } else { "lost" });
            return page;
        };
        let parent_state = parent_states.remove(&path).flatten();
        cursor = Cursor {
            rev: parent.to_string(),
            path,
            tokens: next_tokens,
        };
        preloaded = parent_state;
    }
}

/// The step for the window `win` (with annotated `data`) of `state`.
fn step(
    state: &State,
    tokens: &[TokenLine],
    data: &[HyperLineData],
    win: &Range<usize>,
    anchors: &[usize],
    cursor: &Cursor,
    changed_rev: &str,
) -> Step {
    // Tokens are slices of the source in order, with only whitespace between
    // them, so we can find them one after the other.
    let source = &state.source;
    let mut offsets = Vec::with_capacity(win.len());
    let mut pos = 0;
    for (i, token) in tokens[..win.end].iter().enumerate() {
        let Some(found) = source[pos..].find(token.token) else {
            offsets.clear();
            break;
        };
        let start = pos + found;
        pos = start + token.token.len();
        if i >= win.start {
            offsets.push(start..pos);
        }
    }

    let changed = |j: usize| data[j].introduced.source_rev == changed_rev;
    let (text, token_ranges) = if offsets.len() == win.len() {
        // Show whole lines.
        let text_start = source[..offsets[0].start].rfind('\n').map_or(0, |i| i + 1);
        let last_end = offsets[offsets.len() - 1].end;
        let text_end = source[last_end..]
            .find('\n')
            .map_or(source.len(), |i| last_end + i);
        let text = source[text_start..text_end].to_string();
        let ranges = offsets
            .iter()
            .enumerate()
            .map(|(j, range)| {
                let start = utf16_len(&source[text_start..range.start]);
                (start, start + utf16_len(&source[range.clone()]), changed(j))
            })
            .collect();
        (text, ranges)
    } else {
        let mut text = String::new();
        let mut ranges = vec![];
        for (j, token) in tokens[win.clone()].iter().enumerate() {
            if j > 0 {
                text.push(' ');
            }
            let start = utf16_len(&text);
            text.push_str(token.token);
            ranges.push((start, utf16_len(&text), changed(j)));
        }
        (text, ranges)
    };

    let removed = data[..data.len() - 1]
        .iter()
        .filter_map(|d| d.removal_marker.as_ref())
        .filter(|marker| marker.source_rev == changed_rev)
        .map(|marker| marker.num_removed)
        .sum();

    Step {
        rev: changed_rev.to_string(),
        state_rev: cursor.rev.clone(),
        path: cursor.path.clone(),
        first_token: win.start as u32 + 1,
        text,
        tokens: token_ranges,
        anchors: anchors.iter().map(|a| a - win.start).collect(),
        removed,
    }
}

/// Where the token at `cursor` (which should have one token) was just before
/// the commit which introduced it, for "show the latest version without this
/// token": in that commit's parent, the token it replaced if it evolved from
/// one (of the same kind), or else the nearest token around it which already
/// existed.  Returns the introducing commit too.
pub fn before(
    tree_history: &TreeHistory,
    repo: &Repository,
    cursor: &Cursor,
) -> Option<(String, Cursor)> {
    let rev = Oid::from_str(&cursor.rev).ok()?;
    let state = State::load(tree_history, repo, rev, &cursor.path)?;
    let tokens: Vec<TokenLine> = token_file_lines(&state.history.syntax)
        .into_iter()
        .map(split_token_line)
        .collect();
    let annotated_lines = token_file_lines(&state.history.annotated);
    let anchor = *cursor.tokens.first()? as usize;
    if anchor == 0 || anchor > tokens.len() || annotated_lines.len() != tokens.len() + 1 {
        return None;
    }
    let anchor = anchor - 1;
    let parse = |i: usize| -> Option<HyperLineData> {
        let mut data: HyperLineData = serde_json::from_str(annotated_lines[i + 1]).ok()?;
        data.introduced.resolve_path(&state.history.path);
        if let Some(predecessor) = &mut data.predecessor {
            predecessor.resolve_path(&state.history.path);
        }
        Some(data)
    };
    let anchor_data = parse(anchor)?;
    let introduced_rev = anchor_data.introduced.source_rev.to_string();
    let parent = repo
        .find_commit(Oid::from_str(&introduced_rev).ok()?)
        .ok()?
        .parent_id(0)
        .ok()?;

    // What to look for in the parent, best first: the predecessor, then the
    // nearest tokens which already existed, from the window outward.
    let mut candidates: Vec<(HyperTokenRef, Option<TokenClass>)> = vec![];
    if let Some(predecessor) = &anchor_data.predecessor {
        candidates.push((predecessor.clone(), Some(tokens[anchor].effective_class())));
    }
    let win = window(&tokens, anchor);
    let search_start = win.start.saturating_sub(MAX_WINDOW_TOKENS * 4);
    let search_end = (win.end + MAX_WINDOW_TOKENS * 4).min(tokens.len());
    let mut nearby: Vec<usize> = (search_start..search_end)
        .filter(|&i| i != anchor)
        .collect();
    nearby.sort_by_key(|&i| (!win.contains(&i), (i as isize - anchor as isize).abs()));
    for i in nearby {
        if let Some(data) = parse(i)
            && data.introduced.source_rev != introduced_rev
        {
            candidates.push((data.introduced, None));
            if candidates.len() >= 8 {
                break;
            }
        }
    }

    let mut paths = vec![cursor.path.clone()];
    if anchor_data.introduced.path != cursor.path {
        paths.push(anchor_data.introduced.path.to_string());
    }
    for path in paths {
        let Some(parent_state) = State::load(tree_history, repo, parent, &path) else {
            continue;
        };
        for (identity, required_class) in &candidates {
            let Some(index) = parent_state.find_identity(identity) else {
                continue;
            };
            if let Some(required_class) = required_class {
                let class = token_file_lines(&parent_state.history.syntax)
                    .get(index)
                    .map(|line| split_token_line(line).effective_class());
                if class != Some(*required_class) {
                    continue;
                }
            }
            return Some((
                introduced_rev,
                Cursor {
                    rev: parent.to_string(),
                    path,
                    tokens: vec![index as u32 + 1],
                },
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter_support::cst_tokenizer::hypertokenize_source_file;

    fn window_text(source: &str, anchor_text: &str, nth: usize) -> String {
        let tokenized = hypertokenize_source_file("a.cpp", source).unwrap();
        let tokens: Vec<TokenLine> = tokenized
            .tokenized
            .iter()
            .map(|l| split_token_line(l))
            .collect();
        let anchor = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| t.token == anchor_text)
            .nth(nth)
            .unwrap()
            .0;
        tokens[window(&tokens, anchor)]
            .iter()
            .map(|t| t.token)
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn test_window() {
        let source = "void f() {\n  int x = 1;\n  if (x > 1 && g(x)) {\n    // Some words here.\n    h(x, 2);\n  }\n}\n";
        // The if's condition, from inside a call in it.
        assert_eq!(window_text(source, "g", 0), "if ( x > 1 && g ( x ) ) {");
        // A statement.
        assert_eq!(window_text(source, "h", 0), "h ( x , 2 ) ;");
        assert_eq!(window_text(source, "1", 0), "int x = 1 ;");
        // Comment words.
        assert_eq!(window_text(source, "words", 0), "// Some words here.");
    }

    #[test]
    fn test_long_window() {
        // A statement longer than MAX_WINDOW_TOKENS narrows to the bracketed
        // group around the anchor...
        let args = (0..20)
            .map(|i| format!("a{}", i))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("void f() {{\n  g(1, 2, h({}), 3);\n}}\n", args);
        let text = window_text(&source, "a5", 0);
        assert!(text.starts_with("( a0 , a1"), "{}", text);
        assert!(text.ends_with("a19 )"), "{}", text);
        // ...or the item of the group containing the anchor...
        let args = (0..40)
            .map(|i| format!("a{} + 1", i))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("void f() {{\n  g(1, 2, h({}), 3);\n}}\n", args);
        assert_eq!(window_text(&source, "a30", 0), "a30 + 1 ,");
        assert_eq!(window_text(&source, "a39", 0), "a39 + 1");
        // ...or just the tokens around the anchor if that's too long too.
        let arg = (0..40)
            .map(|i| format!("a{}", i))
            .collect::<Vec<_>>()
            .join(" + ");
        let source = format!("void f() {{\n  g(1, 2, h({}), 3);\n}}\n", arg);
        let text = window_text(&source, "a30", 0);
        assert_eq!(text.split(' ').count(), MAX_WINDOW_TOKENS);
        assert!(text.contains("a30"), "{}", text);
    }
}
