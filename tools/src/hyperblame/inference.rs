//! Token movement and evolution inference for a single revision.
//!
//! Given the old and new token-per-line representations of all of the files
//! changed in a revision, we determine the origin of every token in the new
//! versions of the files and the fate of every removed token.  The results are
//! consumed by `build-timeline-tree` to propagate token history in the
//! "annotated" files and to generate the "future" journal and statistics.
//!
//! ## Data we have
//!
//! We diff the token-per-line files, where each line is "{context} {class}
//! {token}" (see `file_format::history::syntax_files`).
//! We get the byproduct of an insert/delete diff which is trying to perform
//! minimal edits and where we absolutely expect re-orderings to result in
//! paired insertions and deletions.  Additionally, because the context is part
//! of each line, any change to the context of a token (ex: renaming the method
//! it lives in) shows up as a removal and an addition of the token.  So we
//! explicitly compare tokens without their context when trying to pair up
//! removals and additions, using the context only as a tie-breaker.
//!
//! ## Passes
//!
//! 1. Per-file diffs.  We diff each file with the patience algorithm which
//!    anchors on unique lines and so is less prone to rogue synchronization on
//!    extremely common tokens like `}` and `;`.  Unchanged tokens are mapped
//!    directly.  Each run of changes becomes a "block" of removed and added
//!    tokens.
//! 2. "Big rocks" move inference.  In the case of refactorings, we expect a
//!    meaningful amount of locality when it comes to moved chunks of code.  The
//!    patch author will likely be performing cut-and-paste in large sections,
//!    potentially followed by targeted changes.  We do not expect them to move
//!    individual tokens piece by piece like the author is assembling a ransom
//!    note.  So we build a suffix array over all of the removed tokens in each
//!    namespace (language) across all files with sentinels between the removed
//!    runs and then process the added runs from longest to shortest, walking
//!    each run and finding the longest match for the remainder of the run.
//!    This looks a lot like Xdelta in that the suffix array fundamentally deals
//!    with the phase issue for us.  If we find a sufficiently long or good
//!    match we consume it, otherwise we move to the next token and repeat.
//!    - Long/good means that we require that the match contains alphanumeric
//!      tokens.  We don't want to throw away braces and such and forbid them
//!      from having their detection transplanted, but we do require that they
//!      are part of some actual content-bearing tokens.
//!    - When there are multiple candidate locations for a match of a given
//!      length, we apply a fitness function which favors (in order): directly
//!      continuing the previous match from this added run, runs we've matched
//!      from recently in this added run (a stack, so that if "A B C D E F G"
//!      became "A B C 1 2 3 D E F G" we'll resume the A-G run after the new
//!      tokens), the same diff block, the same file, the same context.
//! 3. Local re-contexting.  For the tokens left over in each block, we re-diff
//!    the removed and added tokens without their contexts.  This catches short
//!    runs of tokens whose context changed which were too short to satisfy the
//!    requirements of the previous pass.
//! 4. Evolution inference.  If we have a paired token removal and addition with
//!    stable context on either side, we infer that the token "evolved", ex: a
//!    type or variable was renamed or `>` became `>=`.  Specifically, within
//!    each block we use the tokens matched in the prior passes as anchors, and
//!    for each gap between anchors where the removed and added sides of the
//!    gap consist entirely of the same number of unmatched tokens, we pair them
//!    up positionally if they have the same `TokenClass`.  Keywords are only
//!    paired when the gap is a single token on each side (ex: `const` to `mut`
//!    or `var` to `let`) because a keyword in a larger gap is more likely to be
//!    part of a rewrite than an evolution.  That is, if an
//!    argument "Type argName" changes to "NewType newArgName", we model that as
//!    "Type" evolving to "NewType" and "argName" evolving to "newArgName".
//!    - Requiring that the gaps consist entirely of the paired tokens on both
//!      sides is what "stable context on either side" means.  If the removed
//!      side of a gap also contains tokens which were moved elsewhere, the
//!      removed and added tokens were not in the same "slot".  (In a
//!      refactoring the removed side of a gap can span hundreds of tokens that
//!      moved into newly extracted functions with a stray leftover token.)
//!
//! ## Future work
//!
//! - Runs of tokens where we would ideally be able to model "Type" evolving to
//!   "SomeNamespace::NamespacedType" where we would be going from 1 token to 3
//!   tokens but still occupying the same "hole" in comby.dev terminology.
//! - Accumulating inferred renames across the revision to detect patterns like
//!   "Remote" becoming "Shared" with "Args" appended so that we can use the
//!   renames to re-synchronize sequences that lack sufficient context.
//! - Looking at overall token metrics to skip inference for high-churn
//!   automated changes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};

use super::suffix_array::SuffixArray;
use crate::file_format::history::syntax_files::{TokenClass, TokenLine, split_token_line};

/// Does this token contain content-bearing (alphanumeric) characters?
pub fn is_word_token(token: &str) -> bool {
    token.chars().any(|c| c.is_alphanumeric())
}

/// Diff old and new token lines.
pub fn diff_token_lines(old: &[&str], new: &[&str], timeout: Duration) -> Vec<DiffOp> {
    capture_diff_slices_deadline(
        Algorithm::Patience,
        old,
        new,
        Some(Instant::now() + timeout),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    /// The new file is a copy of the old file which continues to exist; the
    /// old file will have its own `Modified` change if it was modified.  The
    /// tokens which the diff says were "removed" from the old file were not
    /// actually removed, so they don't get fates and can't be move sources.
    ///
    /// Because git's copy detection is based on similarity, a new file that
    /// was split out of an existing file will frequently be detected as a copy.
    /// So we first run move inference over the entire new file and only fall
    /// back to treating tokens as copied (`TokenOrigin::Unchanged` relative to
    /// the old file) if they were not moved from somewhere else.  This means
    /// that tokens which were removed from the original file are treated as
    /// moved, and tokens which still exist in the original file (ex: shared
    /// boilerplate or a true copy) are treated as copied.
    Copied,
}

pub struct FileChangeInput<'a> {
    pub kind: FileChangeKind,
    /// Tokens can only move between files in the same namespace.  See
    /// `LanguageProfile::namespace`.
    pub namespace: &'a str,
    /// The old token lines; empty for `Added`.
    pub old_lines: Vec<&'a str>,
    /// The new token lines; empty for `Deleted`.
    pub new_lines: Vec<&'a str>,
}

/// Where did a token in the new version of a file come from?  Line numbers are
/// 1-based.  `from_file` values are indices into the `FileChangeInput` slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenOrigin {
    /// The diff says this token is unchanged relative to the old version of
    /// this same file.
    Unchanged { old_lineno: u32 },
    /// The token was removed from the old version of `from_file` (which can be
    /// this same file) and added here, potentially with a different context.
    Moved { from_file: u32, old_lineno: u32 },
    /// The token replaced the given removed token which we believe it evolved
    /// from.
    Evolved { from_file: u32, old_lineno: u32 },
    /// The token is new.
    Added,
}

/// What happened to a token removed from the old version of a file?  Line
/// numbers are 1-based.  `to_file` values are indices into the
/// `FileChangeInput` slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovedFate {
    Extinguished,
    MovedTo { to_file: u32, new_lineno: u32 },
    EvolvedInto { to_file: u32, new_lineno: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemovedToken {
    pub old_lineno: u32,
    pub fate: RemovedFate,
}

/// A run of removed tokens without any added tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemovalRun {
    /// The (1-based) line number of the token in the new version of the file
    /// which precedes the removed tokens and will host the removal marker, or
    /// 0 if the removal happened at the start of the file, in which case the
    /// file sentinel hosts the marker.
    pub host_new_lineno: u32,
    /// The (1-based) line number of the first removed token in the old file.
    pub first_old_lineno: u32,
    pub num_removed: u32,
    /// How many of the removed tokens were moved elsewhere.
    pub num_moved: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInference {
    /// The origin of each token in the new version of the file; index 0 is
    /// token line 1.
    pub origins: Vec<TokenOrigin>,
    /// The fates of removed tokens, ordered by `old_lineno`.
    pub removed: Vec<RemovedToken>,
    pub removal_runs: Vec<RemovalRun>,
}

/// How much of the content of a file git paired with an old file (as a rename
/// or copy) actually came from the old file according to the inference.
///
/// git's pairing is based on the similarity of the token files, which counts
/// every token.  Small unrelated files can be similar enough to be paired just
/// because they share a license header and some syntax, so here only "content"
/// tokens count: identifiers, strings, comments, and text, but not keywords,
/// operators, numbers, or boilerplate (see
/// `tree_sitter_support::boilerplate`).  In particular, ordinary comments,
/// including large block comments at the top of a file, do count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairingSupport {
    pub old_content: u32,
    pub new_content: u32,
    /// Content tokens in the new file which are unchanged from, or moved
    /// within, the old file.  Evolutions don't count because they're a
    /// consequence of the pairing rather than evidence for it: when unrelated
    /// files are paired, the tokens between their shared syntax get paired up
    /// as evolutions.
    pub supported: u32,
}

fn is_content(line: &str) -> bool {
    matches!(
        split_token_line(line).effective_class(),
        TokenClass::Identifier | TokenClass::String | TokenClass::Comment | TokenClass::Text
    )
}

impl PairingSupport {
    /// `file` is the index of `input` in the inputs passed to `infer_revision`
    /// and `inference` its result.
    pub fn compute(file: usize, input: &FileChangeInput, inference: &FileInference) -> Self {
        let file = file as u32;
        let mut support = PairingSupport {
            old_content: input.old_lines.iter().filter(|l| is_content(l)).count() as u32,
            ..Default::default()
        };
        for (line, origin) in input.new_lines.iter().zip(&inference.origins) {
            if !is_content(line) {
                continue;
            }
            support.new_content += 1;
            let from_old = match *origin {
                TokenOrigin::Unchanged { .. } => true,
                TokenOrigin::Moved { from_file, .. } => from_file == file,
                TokenOrigin::Evolved { .. } | TokenOrigin::Added => false,
            };
            if from_old {
                support.supported += 1;
            }
        }
        support
    }

    /// The content similarity of the files as a Dice coefficient like git's
    /// similarity score, or None if neither file has content, in which case
    /// we can't second-guess git.
    pub fn similarity(&self) -> Option<f64> {
        match self.old_content + self.new_content {
            0 => None,
            total => Some(2.0 * self.supported as f64 / total as f64),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InferenceConfig {
    /// Maximum number of suffix array candidates to consider for a match at a
    /// given length.  Candidates adjacent to the previous match and in the same
    /// diff block are always considered.
    pub max_candidates: usize,
    /// If a namespace has more removed tokens than this in a single revision,
    /// we skip the suffix-array move inference for that namespace.  This is a
    /// safeguard against giant automated rewrites where the inference is both
    /// expensive and unlikely to be useful.
    pub max_removed_tokens_for_moves: usize,
    /// Timeout for each individual diff.
    pub diff_timeout: Duration,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        InferenceConfig {
            max_candidates: 256,
            max_removed_tokens_for_moves: 4_000_000,
            diff_timeout: Duration::from_secs(10),
        }
    }
}

/// A run of changes in a single file as reported by the per-file diff.
struct Block {
    file: u32,
    /// 0-based index into the file's old lines.
    old_start: u32,
    old_len: u32,
    /// 0-based index into the file's new lines.
    new_start: u32,
    new_len: u32,
    /// False for copies, where the diff's removals are not real removals.
    removals_real: bool,
}

impl Block {
    fn old_range(&self) -> std::ops::Range<usize> {
        self.old_start as usize..(self.old_start + self.old_len) as usize
    }

    fn new_range(&self) -> std::ops::Range<usize> {
        self.new_start as usize..(self.new_start + self.new_len) as usize
    }

    fn contains_old(&self, old_idx: usize) -> bool {
        self.old_range().contains(&old_idx)
    }
}

struct InferenceState<'a, 'b> {
    inputs: &'b [FileChangeInput<'a>],
    old_tokens: Vec<Vec<TokenLine<'a>>>,
    new_tokens: Vec<Vec<TokenLine<'a>>>,
    /// Per-file origin of each new token.
    origins: Vec<Vec<TokenOrigin>>,
    /// Per-file fate of each old token; None for tokens that were not removed.
    fates: Vec<Vec<Option<RemovedFate>>>,
    /// For `Copied` files, the 1-based line number of the token in the old
    /// file that the diff says each new token is equal to, which we fall back
    /// to if the token wasn't otherwise explained.
    copy_equal: Vec<Vec<Option<u32>>>,
    blocks: Vec<Block>,
}

impl<'a, 'b> InferenceState<'a, 'b> {
    fn record_move(&mut self, from_file: usize, old_idx: usize, to_file: usize, new_idx: usize) {
        debug_assert_eq!(self.origins[to_file][new_idx], TokenOrigin::Added);
        debug_assert_eq!(
            self.fates[from_file][old_idx],
            Some(RemovedFate::Extinguished)
        );
        self.origins[to_file][new_idx] = TokenOrigin::Moved {
            from_file: from_file as u32,
            old_lineno: old_idx as u32 + 1,
        };
        self.fates[from_file][old_idx] = Some(RemovedFate::MovedTo {
            to_file: to_file as u32,
            new_lineno: new_idx as u32 + 1,
        });
    }

    fn record_evolution(&mut self, file: usize, old_idx: usize, new_idx: usize) {
        self.origins[file][new_idx] = TokenOrigin::Evolved {
            from_file: file as u32,
            old_lineno: old_idx as u32 + 1,
        };
        self.fates[file][old_idx] = Some(RemovedFate::EvolvedInto {
            to_file: file as u32,
            new_lineno: new_idx as u32 + 1,
        });
    }

    fn is_unmatched_old(&self, file: usize, old_idx: usize) -> bool {
        self.fates[file][old_idx] == Some(RemovedFate::Extinguished)
    }

    fn is_unmatched_new(&self, file: usize, new_idx: usize) -> bool {
        self.origins[file][new_idx] == TokenOrigin::Added
    }
}

/// Run all inference passes for a revision.  The returned vec is parallel to
/// `inputs`.
pub fn infer_revision(inputs: &[FileChangeInput], config: &InferenceConfig) -> Vec<FileInference> {
    let mut state = InferenceState {
        inputs,
        old_tokens: inputs
            .iter()
            .map(|i| i.old_lines.iter().map(|l| split_token_line(l)).collect())
            .collect(),
        new_tokens: inputs
            .iter()
            .map(|i| i.new_lines.iter().map(|l| split_token_line(l)).collect())
            .collect(),
        origins: inputs
            .iter()
            .map(|i| vec![TokenOrigin::Added; i.new_lines.len()])
            .collect(),
        fates: inputs
            .iter()
            .map(|i| vec![None; i.old_lines.len()])
            .collect(),
        copy_equal: inputs.iter().map(|_| vec![]).collect(),
        blocks: vec![],
    };

    diff_files(&mut state, config);

    let mut namespaces: Vec<&str> = inputs.iter().map(|i| i.namespace).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    for namespace in namespaces {
        infer_big_rock_moves(&mut state, namespace, config);
    }

    infer_local_recontexting(&mut state, config);
    infer_evolutions(&mut state);

    finalize(state)
}

/// Pass 1: Diff each file to establish the unchanged tokens and blocks.
fn diff_files(state: &mut InferenceState, config: &InferenceConfig) {
    for (file, input) in state.inputs.iter().enumerate() {
        let ops = diff_token_lines(&input.old_lines, &input.new_lines, config.diff_timeout);

        if input.kind == FileChangeKind::Copied {
            // See the `FileChangeKind::Copied` docs; the whole new file is an
            // added run with the diff's equal tokens as a fallback.
            let mut copy_equal = vec![None; input.new_lines.len()];
            for op in ops {
                if let DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } = op
                {
                    for j in 0..len {
                        copy_equal[new_index + j] = Some((old_index + j) as u32 + 1);
                    }
                }
            }
            state.copy_equal[file] = copy_equal;
            if !input.new_lines.is_empty() {
                state.blocks.push(Block {
                    file: file as u32,
                    old_start: 0,
                    old_len: 0,
                    new_start: 0,
                    new_len: input.new_lines.len() as u32,
                    removals_real: false,
                });
            }
            continue;
        }

        let removals_real = true;
        for op in ops {
            let (old_start, old_len, new_start, new_len) = match op {
                DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => {
                    for j in 0..len {
                        state.origins[file][new_index + j] = TokenOrigin::Unchanged {
                            old_lineno: (old_index + j) as u32 + 1,
                        };
                    }
                    continue;
                }
                DiffOp::Delete {
                    old_index,
                    old_len,
                    new_index,
                } => (old_index, old_len, new_index, 0),
                DiffOp::Insert {
                    old_index,
                    new_index,
                    new_len,
                } => (old_index, 0, new_index, new_len),
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => (old_index, old_len, new_index, new_len),
            };
            if removals_real {
                for old_idx in old_start..old_start + old_len {
                    state.fates[file][old_idx] = Some(RemovedFate::Extinguished);
                }
            }
            state.blocks.push(Block {
                file: file as u32,
                old_start: old_start as u32,
                old_len: old_len as u32,
                new_start: new_start as u32,
                new_len: new_len as u32,
                removals_real,
            });
        }
    }
}

/// Where a position in the suffix array text came from.
#[derive(Clone, Copy)]
struct TextSource {
    file: u32,
    old_idx: u32,
    block: u32,
}

const SENTINEL_SOURCE: TextSource = TextSource {
    file: u32::MAX,
    old_idx: u32::MAX,
    block: u32::MAX,
};

/// Pass 2: Suffix-array based move inference within a namespace.
fn infer_big_rock_moves(state: &mut InferenceState, namespace: &str, config: &InferenceConfig) {
    let in_namespace = |file: u32| state.inputs[file as usize].namespace == namespace;

    // ## Intern tokens and build the text of all removed tokens.
    let mut interned: HashMap<&str, u32> = HashMap::new();
    // Index 0 is the sentinel.
    let mut id_is_word: Vec<bool> = vec![false];

    let mut text: Vec<u32> = vec![];
    let mut sources: Vec<TextSource> = vec![];
    for (block_idx, block) in state.blocks.iter().enumerate() {
        if !block.removals_real || block.old_len == 0 || !in_namespace(block.file) {
            continue;
        }
        let file = block.file as usize;
        for old_idx in block.old_range() {
            let token = state.old_tokens[file][old_idx].token;
            let next_id = id_is_word.len() as u32;
            let id = *interned.entry(token).or_insert_with(|| next_id);
            if id == next_id {
                id_is_word.push(is_word_token(token));
            }
            text.push(id);
            sources.push(TextSource {
                file: block.file,
                old_idx: old_idx as u32,
                block: block_idx as u32,
            });
        }
        text.push(0);
        sources.push(SENTINEL_SOURCE);
    }

    if text.is_empty() || text.len() > config.max_removed_tokens_for_moves {
        return;
    }

    // ## Gather the added runs, biggest first.
    let mut added_runs: Vec<usize> = state
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.new_len > 0 && in_namespace(b.file))
        .map(|(i, _)| i)
        .collect();
    if added_runs.is_empty() {
        return;
    }
    added_runs.sort_by_key(|&i| std::cmp::Reverse(state.blocks[i].new_len));

    let sa = SuffixArray::new(&text);
    let mut consumed = vec![false; text.len()];

    for block_idx in added_runs {
        let (file, new_range) = {
            let b = &state.blocks[block_idx];
            (b.file as usize, b.new_range())
        };
        // Tokens that don't exist in the removed text can't match, so they get
        // an id that will never be found.
        let query: Vec<u32> = new_range
            .clone()
            .map(|new_idx| {
                interned
                    .get(state.new_tokens[file][new_idx].token)
                    .copied()
                    .unwrap_or(u32::MAX)
            })
            .collect();
        // word_prefix[i] is the number of word tokens in query[..i].
        let mut word_prefix = vec![0u32; query.len() + 1];
        for (i, &id) in query.iter().enumerate() {
            let is_word = id != u32::MAX && id_is_word[id as usize];
            word_prefix[i + 1] = word_prefix[i] + is_word as u32;
        }

        // The end position in the text of the last match and the block it was in.
        let mut last_match: Option<(usize, u32)> = None;
        // Recently matched blocks, most recent last.
        let mut recent_blocks: Vec<u32> = vec![];

        let mut i = 0;
        while i < query.len() {
            let ranges = sa.match_ranges(&query[i..], query.len() - i);
            let mut accepted: Option<(usize, usize)> = None;
            for k in (2..=ranges.len()).rev() {
                let words = word_prefix[i + k] - word_prefix[i];
                if !(words >= 2 || (words >= 1 && k >= 3)) {
                    continue;
                }
                let (lo, hi) = ranges[k - 1];
                let is_usable = |p: usize| -> bool {
                    p + k <= text.len()
                        && text[p..p + k] == query[i..i + k]
                        && !consumed[p..p + k].iter().any(|c| *c)
                };

                let mut candidates: Vec<usize> = (lo..hi.min(lo + config.max_candidates))
                    .map(|r| sa.suffix(r))
                    .filter(|&p| is_usable(p))
                    .collect();
                if hi - lo > config.max_candidates {
                    // Make sure we consider the positions our fitness function
                    // most favors even if they were not in the capped range.
                    if let Some((end, _)) = last_match {
                        for p in end..(end + 16).min(text.len()) {
                            if is_usable(p) {
                                candidates.push(p);
                            }
                        }
                    }
                    // Same-block candidates.
                    let mut scanned = 0;
                    for (p, source) in sources.iter().enumerate() {
                        if source.block as usize == block_idx && is_usable(p) {
                            candidates.push(p);
                        }
                        scanned += 1;
                        if scanned > 1_000_000 {
                            break;
                        }
                    }
                }
                if candidates.is_empty() {
                    continue;
                }

                let fitness = |p: usize| {
                    let source = sources[p];
                    let (continuation, distance) = match last_match {
                        Some((end, last_block)) if source.block == last_block && p >= end => {
                            (0, p - end)
                        }
                        _ => match recent_blocks.iter().rev().position(|b| *b == source.block) {
                            Some(depth) => (1, depth),
                            None => (2, 0),
                        },
                    };
                    let same_block = (source.block as usize != block_idx) as u8;
                    let same_file = (source.file as usize != file) as u8;
                    let same_context =
                        (state.old_tokens[source.file as usize][source.old_idx as usize].context
                            != state.new_tokens[file][new_range.start + i].context)
                            as u8;
                    (
                        continuation,
                        distance,
                        same_block,
                        same_file,
                        same_context,
                        p,
                    )
                };
                let best = candidates.into_iter().min_by_key(|&p| fitness(p)).unwrap();
                accepted = Some((best, k));
                break;
            }

            match accepted {
                Some((p, k)) => {
                    for j in 0..k {
                        let source = sources[p + j];
                        consumed[p + j] = true;
                        state.record_move(
                            source.file as usize,
                            source.old_idx as usize,
                            file,
                            new_range.start + i + j,
                        );
                    }
                    let matched_block = sources[p].block;
                    last_match = Some((p + k, matched_block));
                    recent_blocks.retain(|b| *b != matched_block);
                    recent_blocks.push(matched_block);
                    i += k;
                }
                None => {
                    i += 1;
                }
            }
        }
    }
}

/// Pass 3: Re-diff the leftover tokens in each block without their contexts.
fn infer_local_recontexting(state: &mut InferenceState, config: &InferenceConfig) {
    for block_idx in 0..state.blocks.len() {
        let block = &state.blocks[block_idx];
        if !block.removals_real || block.old_len == 0 || block.new_len == 0 {
            continue;
        }
        let file = block.file as usize;
        let old_idxs: Vec<usize> = block
            .old_range()
            .filter(|&o| state.is_unmatched_old(file, o))
            .collect();
        let new_idxs: Vec<usize> = block
            .new_range()
            .filter(|&n| state.is_unmatched_new(file, n))
            .collect();
        if old_idxs.is_empty() || new_idxs.is_empty() {
            continue;
        }
        let old_texts: Vec<&str> = old_idxs
            .iter()
            .map(|&o| state.old_tokens[file][o].token)
            .collect();
        let new_texts: Vec<&str> = new_idxs
            .iter()
            .map(|&n| state.new_tokens[file][n].token)
            .collect();
        let ops = capture_diff_slices_deadline(
            Algorithm::Myers,
            &old_texts,
            &new_texts,
            Some(Instant::now() + config.diff_timeout),
        );
        for op in ops {
            if let DiffOp::Equal {
                old_index,
                new_index,
                len,
            } = op
            {
                for j in 0..len {
                    state.record_move(file, old_idxs[old_index + j], file, new_idxs[new_index + j]);
                }
            }
        }
    }
}

/// Pass 4: Pair up leftover tokens between anchors as evolutions.
fn infer_evolutions(state: &mut InferenceState) {
    for block_idx in 0..state.blocks.len() {
        let block = &state.blocks[block_idx];
        if !block.removals_real || block.old_len == 0 || block.new_len == 0 {
            continue;
        }
        let file = block.file as usize;
        let (old_start, new_start) = (block.old_start as i64, block.new_start as i64);

        // ## Find the monotonic anchors within the block.
        let mut anchors: Vec<(i64, i64)> = vec![(new_start - 1, old_start - 1)];
        for new_idx in block.new_range() {
            if let TokenOrigin::Moved {
                from_file,
                old_lineno,
            } = state.origins[file][new_idx]
            {
                let old_idx = old_lineno as usize - 1;
                if from_file as usize == file
                    && block.contains_old(old_idx)
                    && old_idx as i64 > anchors.last().unwrap().1
                {
                    anchors.push((new_idx as i64, old_idx as i64));
                }
            }
        }
        anchors.push((
            new_start + block.new_len as i64,
            old_start + block.old_len as i64,
        ));

        // ## Pair up equal-sized gaps which occupy the same slot.
        let mut pairs = vec![];
        for window in anchors.windows(2) {
            let ((n0, o0), (n1, o1)) = (window[0], window[1]);
            let new_gap = (n0 + 1) as usize..n1 as usize;
            let old_gap = (o0 + 1) as usize..o1 as usize;
            if new_gap.is_empty() || new_gap.len() != old_gap.len() {
                continue;
            }
            // Both sides of the gap must consist entirely of unmatched tokens;
            // see the module docs.
            if !new_gap.clone().all(|n| state.is_unmatched_new(file, n))
                || !old_gap.clone().all(|o| state.is_unmatched_old(file, o))
            {
                continue;
            }
            let isolated = new_gap.len() == 1;
            for (n, o) in new_gap.zip(old_gap) {
                let class = state.new_tokens[file][n].effective_class();
                if class != state.old_tokens[file][o].effective_class()
                    || (class == TokenClass::Keyword && !isolated)
                {
                    continue;
                }
                pairs.push((o, n));
            }
        }
        for (o, n) in pairs {
            state.record_evolution(file, o, n);
        }
    }
}

fn finalize(mut state: InferenceState) -> Vec<FileInference> {
    // Fall back to the copy relationship for copied tokens not otherwise
    // explained.
    for (file, copy_equal) in state.copy_equal.iter().enumerate() {
        for (new_idx, old_lineno) in copy_equal.iter().enumerate() {
            if let Some(old_lineno) = old_lineno
                && state.origins[file][new_idx] == TokenOrigin::Added
            {
                state.origins[file][new_idx] = TokenOrigin::Unchanged {
                    old_lineno: *old_lineno,
                };
            }
        }
    }

    let mut results: Vec<FileInference> = state
        .origins
        .into_iter()
        .zip(state.fates.iter())
        .map(|(origins, fates)| FileInference {
            origins,
            removed: fates
                .iter()
                .enumerate()
                .filter_map(|(old_idx, fate)| {
                    fate.map(|fate| RemovedToken {
                        old_lineno: old_idx as u32 + 1,
                        fate,
                    })
                })
                .collect(),
            removal_runs: vec![],
        })
        .collect();

    for block in &state.blocks {
        let file = block.file as usize;
        // Removal markers only make sense when there's a new version of the
        // file for the marker to live in.
        if !block.removals_real
            || block.new_len != 0
            || block.old_len == 0
            || state.inputs[file].kind == FileChangeKind::Deleted
        {
            continue;
        }
        let num_moved = block
            .old_range()
            .filter(|&o| matches!(state.fates[file][o], Some(RemovedFate::MovedTo { .. })))
            .count() as u32;
        results[file].removal_runs.push(RemovalRun {
            // The token preceding the removal is at 0-based index new_start - 1
            // which is 1-based line number new_start; 0 is the sentinel.
            host_new_lineno: block.new_start,
            first_old_lineno: block.old_start + 1,
            num_removed: block.old_len,
            num_moved,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::syntax_files::format_token_line;

    /// Keywords for the purposes of our test helper; everything else gets
    /// `TokenClass::guess`.
    const TEST_KEYWORDS: &[&str] = &[
        "class", "const", "continue", "else", "fn", "for", "if", "let", "mut", "return", "while",
    ];

    /// Helper to build token lines from a context and a space-separated list of
    /// tokens.
    fn toks(context: &str, tokens: &str) -> Vec<String> {
        tokens
            .split_whitespace()
            .map(|t| {
                let class = if TEST_KEYWORDS.contains(&t) {
                    TokenClass::Keyword
                } else {
                    TokenClass::guess(t)
                };
                format_token_line(context, class, t)
            })
            .collect()
    }

    fn input<'a>(
        kind: FileChangeKind,
        old: &'a [String],
        new: &'a [String],
    ) -> FileChangeInput<'a> {
        FileChangeInput {
            kind,
            namespace: "cpp",
            old_lines: old.iter().map(|s| s.as_str()).collect(),
            new_lines: new.iter().map(|s| s.as_str()).collect(),
        }
    }

    fn summarize(inference: &FileInference) -> String {
        inference
            .origins
            .iter()
            .map(|o| match o {
                TokenOrigin::Unchanged { .. } => "U",
                TokenOrigin::Moved { .. } => "M",
                TokenOrigin::Evolved { .. } => "E",
                TokenOrigin::Added => "A",
            })
            .collect()
    }

    #[test]
    fn test_single_token_evolution() {
        let mut old = toks("%", "int");
        old.extend(toks("Foo", "void Foo ( ) { return x > 0 ; }"));
        let mut new = toks("%", "int");
        new.extend(toks("Foo", "void Foo ( ) { return x >= 0 ; }"));
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUUUUUUEUUU");
        assert_eq!(
            results[0].origins[8],
            TokenOrigin::Evolved {
                from_file: 0,
                old_lineno: 9
            }
        );
        assert_eq!(
            results[0].removed,
            vec![RemovedToken {
                old_lineno: 9,
                fate: RemovedFate::EvolvedInto {
                    to_file: 0,
                    new_lineno: 9
                }
            }]
        );
        assert!(results[0].removal_runs.is_empty());
    }

    #[test]
    fn test_method_rename_recontexts_body() {
        // Renaming a method changes the context of every token in it, so the
        // diff sees the whole method removed and re-added.  We should see the
        // body tokens as moved and the name as evolved.
        let mut old = toks("%", "class Foo {");
        old.extend(toks("Foo::bar", "void bar ( ) { mCount = mCount + 1 ; }"));
        old.extend(toks("%", "} ;"));
        let mut new = toks("%", "class Foo {");
        new.extend(toks("Foo::baz", "void baz ( ) { mCount = mCount + 1 ; }"));
        new.extend(toks("%", "} ;"));
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUMEMMMMMMMMMMUU");
        assert!(
            results[0]
                .removed
                .iter()
                .all(|r| r.fate != RemovedFate::Extinguished)
        );
    }

    #[test]
    fn test_evolution_requires_same_slot() {
        // From a real refactoring: most of the removed tokens in the block were
        // moved into a new function in another file, leaving a stray `continue`
        // which is not in the same slot as the added `for`.
        let old = toks(
            "main",
            "fn main ( ) { let v = compute ( alpha , beta ) ; continue }",
        );
        let new = toks("main", "fn main ( ) { for }");
        let empty: Vec<String> = vec![];
        let helper = toks("helper", "let v = compute ( alpha , beta ) ;");
        let inputs = vec![
            input(FileChangeKind::Modified, &old, &new),
            input(FileChangeKind::Added, &empty, &helper),
        ];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[1]), "M".repeat(helper.len()));
        assert_eq!(summarize(&results[0]), "UUUUUAU");
        let continue_fate = results[0]
            .removed
            .iter()
            .find(|r| r.old_lineno == 16)
            .unwrap()
            .fate;
        assert_eq!(continue_fate, RemovedFate::Extinguished);
    }

    #[test]
    fn test_evolution_requires_compatible_class() {
        // A string replaced by a number or an identifier isn't an evolution...
        let old = toks("f", "log ( \"loc\" ) ; x = \"name\" ;");
        let new = toks("f", "log ( 42 ) ; x = null ;");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUAUUUUAU");

        // ...but number to number and punctuation to punctuation are.
        let old = toks("f", "if ( a > 0 ) return 1 ;");
        let new = toks("f", "if ( a >= 0 ) return 2 ;");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUEUUUEU");
    }

    #[test]
    fn test_keyword_evolution_policy() {
        // An isolated keyword to keyword replacement is an evolution.
        let old = toks("f", "let p : * const u8 ;");
        let new = toks("f", "let p : * mut u8 ;");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUUEUU");

        // A keyword to identifier replacement in the same slot is not.
        let old = toks("f", "data { let ( piece ) }");
        let new = toks("f", "data { process_analysis_target ( piece ) }");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUAUUUU");

        // Keywords in a multi-token gap don't pair, but identifiers do.
        let old = toks("f", "a ( if x ) ;");
        let new = toks("f", "a ( while y ) ;");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUAEUU");
    }

    #[test]
    fn test_argument_evolution() {
        // "Type argName" => "NewType newArgName"
        let old = toks("f", "void f ( Type argName ) ;");
        let new = toks("f", "void f ( NewType newArgName ) ;");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUEEUU");
    }

    #[test]
    fn test_pure_deletion_removal_run() {
        let old = toks("%", "a b c d e f g");
        let new = toks("%", "a b f g");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUU");
        assert_eq!(
            results[0].removal_runs,
            vec![RemovalRun {
                host_new_lineno: 2,
                first_old_lineno: 3,
                num_removed: 3,
                num_moved: 0,
            }]
        );
        assert_eq!(results[0].removed.len(), 3);

        // Removal at the very start is hosted by the sentinel.
        let new = toks("%", "d e f g");
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(results[0].removal_runs[0].host_new_lineno, 0);
    }

    #[test]
    fn test_move_between_files() {
        // A function is moved from a.cpp to b.cpp (which is new) and a second
        // function stays behind.
        let mut a_old = toks("stay", "void stay ( ) { return ; }");
        a_old.extend(toks(
            "moveMe",
            "int moveMe ( int x ) { return x * 2 + gOffset ; }",
        ));
        let a_new = toks("stay", "void stay ( ) { return ; }");
        let b_new = toks(
            "moveMe",
            "int moveMe ( int x ) { return x * 2 + gOffset ; }",
        );
        let empty: Vec<String> = vec![];
        let inputs = vec![
            input(FileChangeKind::Modified, &a_old, &a_new),
            input(FileChangeKind::Added, &empty, &b_new),
        ];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUUUUUUU");
        assert_eq!(summarize(&results[1]), "M".repeat(b_new.len()));
        assert_eq!(
            results[1].origins[0],
            TokenOrigin::Moved {
                from_file: 0,
                old_lineno: 9
            }
        );
        // a.cpp gets a removal marker noting all the removed tokens moved.
        assert_eq!(
            results[0].removal_runs,
            vec![RemovalRun {
                host_new_lineno: 8,
                first_old_lineno: 9,
                num_removed: b_new.len() as u32,
                num_moved: b_new.len() as u32,
            }]
        );
    }

    #[test]
    fn test_move_resumes_after_insertion() {
        // "A B C D E F G" moved from one file to another and became
        // "A B C 1 2 3 D E F G".  A decoy "D E F G" was also removed elsewhere.
        let a_old = toks("x", "alpha beta gamma delta epsilon zeta eta");
        let c_old = toks("y", "delta epsilon zeta eta");
        let b_new = toks("x", "alpha beta gamma one two three delta epsilon zeta eta");
        let empty: Vec<String> = vec![];
        let inputs = vec![
            input(FileChangeKind::Deleted, &a_old, &empty),
            input(FileChangeKind::Deleted, &c_old, &empty),
            input(FileChangeKind::Added, &empty, &b_new),
        ];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[2]), "MMMAAAMMMM");
        // The resumed run should come from a.cpp (file 0), not the decoy.
        for new_idx in 6..10 {
            match results[2].origins[new_idx] {
                TokenOrigin::Moved { from_file, .. } => assert_eq!(from_file, 0),
                o => panic!("unexpected origin {:?}", o),
            }
        }
        // The decoy's tokens were extinguished.
        assert!(
            results[1]
                .removed
                .iter()
                .all(|r| r.fate == RemovedFate::Extinguished)
        );
    }

    #[test]
    fn test_block_reorder_within_file() {
        let mut old = toks("first", "void first ( ) { doFirstThing ( ) ; }");
        old.extend(toks("second", "void second ( ) { doSecondThing ( ) ; }"));
        let mut new = toks("second", "void second ( ) { doSecondThing ( ) ; }");
        new.extend(toks("first", "void first ( ) { doFirstThing ( ) ; }"));
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        // Whichever function the diff decided to keep is unchanged and the other
        // is moved; nothing should be added.
        let summary = summarize(&results[0]);
        assert!(!summary.contains('A'), "{}", summary);
        assert!(!summary.contains('E'), "{}", summary);
        assert!(summary.contains('M'), "{}", summary);
    }

    #[test]
    fn test_copy_does_not_remove() {
        let old = toks("%", "a b c d");
        let new = toks("%", "a b x");
        let inputs = vec![input(FileChangeKind::Copied, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUA");
        assert!(results[0].removed.is_empty());
        assert!(results[0].removal_runs.is_empty());
    }

    #[test]
    fn test_copy_that_is_really_a_move() {
        // git decided b.cpp is a copy of a.cpp, but the function was actually
        // moved out of a.cpp, with only the include being shared.
        let mut a_old = toks("%", "#include \"a.h\"");
        a_old.extend(toks("stay", "void stay ( ) { return ; }"));
        a_old.extend(toks("go", "void go ( ) { doSomething ( ) ; }"));
        let mut a_new = toks("%", "#include \"a.h\"");
        a_new.extend(toks("stay", "void stay ( ) { return ; }"));
        let mut b_new = toks("%", "#include \"a.h\"");
        b_new.extend(toks("go", "void go ( ) { doSomething ( ) ; }"));
        let inputs = vec![
            input(FileChangeKind::Modified, &a_old, &a_new),
            input(FileChangeKind::Copied, &a_old, &b_new),
        ];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        // The include is copied, the function is moved.
        assert_eq!(summarize(&results[1]), "UUMMMMMMMMMM");
        assert_eq!(results[0].removal_runs[0].num_moved, 10);
        assert!(
            results[0]
                .removed
                .iter()
                .all(|r| matches!(r.fate, RemovedFate::MovedTo { to_file: 1, .. }))
        );
    }

    fn classed(class: TokenClass, tokens: &str) -> Vec<String> {
        tokens
            .split_whitespace()
            .map(|t| format_token_line("%", class, t))
            .collect()
    }

    #[test]
    fn test_pairing_support() {
        let license = classed(
            TokenClass::Boilerplate,
            "/* This Source Code Form is subject to the terms of the Mozilla Public License */",
        );

        // Unrelated small files which git paired because they share a license
        // header and some syntax.
        let mut old = license.clone();
        old.extend(toks("%", "const fs = require ( \"fs\" ) ;"));
        let mut new = license.clone();
        new.extend(toks("%", "const symbol = Symbol ( \"handle\" ) ;"));
        let inputs = vec![input(FileChangeKind::Renamed, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        let support = PairingSupport::compute(0, &inputs[0], &results[0]);
        assert_eq!(
            support,
            PairingSupport {
                old_content: 3,
                new_content: 3,
                supported: 0
            }
        );
        assert_eq!(support.similarity(), Some(0.0));

        // A real rename where the code was rewritten but the big block comment
        // at the top survived.
        let comment = classed(
            TokenClass::Comment,
            "/* This module coordinates the widgets and must be kept in sync with the \
             frobnicator because of the reasons described at length here */",
        );
        let mut old = license.clone();
        old.extend(comment.clone());
        old.extend(toks("go", "void go ( ) { doSomething ( ) ; }"));
        let mut new = license.clone();
        new.extend(comment.clone());
        new.extend(toks("go", "fn go ( ) { other_thing ( ) ; }"));
        let inputs = vec![input(FileChangeKind::Renamed, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        let support = PairingSupport::compute(0, &inputs[0], &results[0]);
        // The 24 comment tokens plus `go`.
        assert_eq!(support.supported, 25);
        assert!(support.similarity().unwrap() > 0.9, "{:?}", support);

        // Nothing to judge by.
        let inputs = vec![input(FileChangeKind::Renamed, &license, &license)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        let support = PairingSupport::compute(0, &inputs[0], &results[0]);
        assert_eq!(support.similarity(), None);
    }

    #[test]
    fn test_namespaces_isolated() {
        let a_old = toks("x", "function frobnicate ( widget ) { }");
        let empty: Vec<String> = vec![];
        let b_new = toks("x", "function frobnicate ( widget ) { }");
        let mut inputs = vec![
            input(FileChangeKind::Deleted, &a_old, &empty),
            input(FileChangeKind::Added, &empty, &b_new),
        ];
        inputs[0].namespace = "js";
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[1]), "A".repeat(b_new.len()));
    }
}
