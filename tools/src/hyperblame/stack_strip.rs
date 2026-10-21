//! The patch stack strip (see "Patch stack strip (M4)" in the hyperblame
//! notes): for a file at a revision viewed in the explicit context of a stack
//! of commits (an explore page's, ex: a try push's, or a bug's commits), which
//! of the stack's commits touched the tokens on each line, as a slot per
//! commit beside the blame strip (see stack-strip.js), like the explore page's
//! sparklines per line.
//!
//! A token was touched by a stack commit if the commit introduced it in its
//! current form, or introduced any of the forms it evolved from, so the
//! file's states at each of the stack's commits count, not only the viewed
//! one's: its annotated record (see `timeline_annotated`) says which revision
//! introduced its current form and which token it evolved from (one step back),
//! whose record in that revision says what it evolved from, and so on.  We
//! follow those back until a revision from before the stack (by commit time,
//! older than its oldest commit), ex: through other commits between a bug's
//! commits.  Runs of tokens a stack commit removed show as scars on the line
//! which their removal marker is on (only the newest marker of a token is
//! kept, so a later removal hides an earlier one).  Cross-file moves aren't
//! followed specially (a moved token keeps its identity in the other file).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use git2::{Oid, Repository};
use serde::Serialize;

use super::token_blame::{TreeHistory, blame_tokens};
use crate::file_format::history::timeline_annotated::{
    HyperLineData, HyperTokenRef, annotated_lines,
};

/// The most steps we follow a token's evolution back.
const MAX_EVOLUTIONS: usize = 64;

/// The stack strip of a file, as the page's script takes it.
#[derive(Debug, PartialEq, Serialize)]
pub struct StackStrip {
    /// The stack's commits, oldest first: `[rev, summary]`, the slots' order.
    pub commits: Vec<(String, String)>,
    /// The lines (0-based) whose tokens stack commits touched, with the slots
    /// of those commits: `[line, [slot...]]`.
    pub lines: Vec<(usize, Vec<usize>)>,
    /// The lines with removals by stack commits: `[line, [slot...]]`.
    pub scars: Vec<(usize, Vec<usize>)>,
}

/// The stack strip of the file at `path` in `rev`, whose contents are
/// `source`, for the stack of `commits` (oldest first).
pub fn stack_strip(
    history: &TreeHistory,
    repo: &Repository,
    commits: &[Oid],
    rev: Oid,
    path: &str,
    source: &str,
) -> Result<StackStrip, String> {
    let timeline_commit = history
        .timeline_commit(rev)
        .ok_or("The history doesn't have the revision")?;
    let file = history
        .file_history(&timeline_commit, path)
        .map_err(|e| e.to_string())?
        .ok_or("The history doesn't have the file")?;
    let blame = blame_tokens(source, &file).map_err(|e| e.to_string())?;

    let slots: HashMap<String, usize> = commits
        .iter()
        .enumerate()
        .map(|(slot, oid)| (oid.to_string(), slot))
        .collect();
    let commit_time = |rev: &str| -> Option<i64> {
        let commit = repo.find_commit(Oid::from_str(rev).ok()?).ok()?;
        Some(commit.time().seconds())
    };
    let start = commits
        .iter()
        .filter_map(|oid| commit_time(&oid.to_string()))
        .min()
        .ok_or("The stack has no commits")?;
    let mut times: HashMap<String, Option<i64>> = HashMap::new();
    let mut annotated: HashMap<(String, String), Option<Vec<String>>> = HashMap::new();
    // The record of a token (canonical ref) in its revision's annotated file.
    let mut record = |token: &HyperTokenRef| -> Option<HyperLineData<'static>> {
        let lines = annotated
            .entry((token.source_rev.to_string(), token.path.to_string()))
            .or_insert_with(|| {
                let text =
                    history.annotated(Oid::from_str(&token.source_rev).ok()?, &token.path)?;
                annotated_lines(&text).ok()
            })
            .as_ref()?;
        let mut data = HyperLineData::parse(lines.get(token.lineno as usize)?);
        data.resolve_path(&token.path);
        Some(owned(data))
    };

    let mut lines: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut scars: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for token in &blame.tokens {
        let mut touched = BTreeSet::new();
        if let Some(&slot) = slots.get(&*token.data.introduced.source_rev) {
            touched.insert(slot);
        }
        let mut evolved_from = token.data.predecessor.clone().map(|p| p.into_owned());
        for _ in 0..MAX_EVOLUTIONS {
            let Some(from) = evolved_from.take() else {
                break;
            };
            match slots.get(&*from.source_rev) {
                Some(&slot) => {
                    touched.insert(slot);
                }
                None => {
                    // (From before the stack, or a revision we don't know.)
                    let time = *times
                        .entry(from.source_rev.to_string())
                        .or_insert_with(|| commit_time(&from.source_rev));
                    if time.is_none_or(|time| time < start) {
                        break;
                    }
                }
            }
            evolved_from = record(&from).and_then(|data| data.predecessor);
        }
        if !touched.is_empty() {
            lines.entry(token.line).or_default().extend(touched);
        }
        if let Some(marker) = &token.data.removal_marker
            && let Some(&slot) = slots.get(&*marker.source_rev)
        {
            scars.entry(token.line).or_default().insert(slot);
        }
    }
    if let Some(marker) = blame.removal_above()
        && let Some(&slot) = slots.get(&*marker.source_rev)
    {
        scars.entry(0).or_default().insert(slot);
    }

    let summary = |oid: &Oid| -> String {
        repo.find_commit(*oid)
            .ok()
            .and_then(|commit| {
                commit
                    .summary_bytes()
                    .map(|summary| String::from_utf8_lossy(summary).into_owned())
            })
            .unwrap_or_default()
    };
    let collect = |map: BTreeMap<usize, BTreeSet<usize>>| -> Vec<(usize, Vec<usize>)> {
        map.into_iter()
            .map(|(line, slots)| (line, slots.into_iter().collect()))
            .collect()
    };
    Ok(StackStrip {
        commits: commits
            .iter()
            .map(|oid| (oid.to_string(), summary(oid)))
            .collect(),
        lines: collect(lines),
        scars: collect(scars),
    })
}

/// A record which doesn't borrow the annotated file it was read from.
fn owned(data: HyperLineData) -> HyperLineData<'static> {
    HyperLineData {
        introduced: data.introduced.into_owned(),
        predecessor: data.predecessor.map(HyperTokenRef::into_owned),
        removal_marker: None,
    }
}
