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
//!      are part of some actual content-bearing tokens.  And a match from
//!      another file must be long, or have rare identifiers or several of
//!      them (see `InferenceConfig::cross_file_min_len`), since short runs of
//!      common tokens (ex: `: true ,`) match other files' removals by
//!      coincidence.
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
//!    - Gaps whose sides differ in size (a "hole" in comby.dev terminology,
//!      ex: `DtlsIdentity::DEFAULT_HASH_ALGORITHM` becoming
//!      `DEFAULT_DTLS_HASH_ALGORITHM`) only pair identifiers and strings whose
//!      subwords are similar (see `hole_pairs`), since they're often rewrites
//!      (ex: `EventUtils.sendMouseEvent` becoming `clickOnRequestRow`).
//!    - Then identifiers' renames inferred in the revision apply wherever else
//!      both names are left unmatched in a block (see
//!      `infer_revision_renames`), if they were seen twice or are similar.
//! 5. Renames' patterns.  The ways the revision's renames changed their names'
//!    subwords, ex: "Remote" becoming "Shared" with "Args" appended, pair other
//!    unmatched identifiers whose names differ in only those ways, in a block,
//!    and across blocks (ex: to another file), where each pair anchors the
//!    identical unmatched tokens around it, which re-synchronizes sequences
//!    with too little context to match (see `infer_rename_patterns`).
//! 6. Inline event handlers, for files with the `searchfox-handler-moves`
//!    history attribute: the code of a handler removed from an element with an
//!    id is followed to the code after a string of that id which the revision
//!    added (ex: `case "allTabsMenu-searchTabs":`; see `infer_handler_moves`).
//!
//! ## Future work
//!
//! - Looking at overall token metrics to skip inference for high-churn
//!   automated changes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::Hash;

use imara_diff::{Algorithm, Diff, Interner, Token};
use similar::DiffOp;

use super::suffix_array::SuffixArray;
use crate::file_format::history::syntax_files::{TokenClass, TokenLine, split_token_line};

/// Does this token contain content-bearing (alphanumeric) characters?
pub fn is_word_token(token: &str) -> bool {
    token.chars().any(|c| c.is_alphanumeric())
}

/// Diff old and new token lines with imara-diff's histogram diff, a
/// refinement of patience diff: it anchors on the lines which occur least, so
/// it doesn't synchronize on lines like `}` and `;`.  It's fast enough not to
/// need a deadline (ex: 3ms for a 44k-token file most of whose lines changed,
/// which took `similar`'s patience diff 9s), and so it's deterministic, unlike
/// a diff which gives up when time runs out and so depends on how busy the
/// machine is.
pub fn diff_token_lines(old: &[&str], new: &[&str]) -> Vec<DiffOp> {
    diff_slices(Algorithm::Histogram, old, new)
}

/// Diff two sequences with imara-diff, as `similar` ops: equal runs between
/// its hunks, and a hunk with both removals and additions as a replacement
/// (as `similar` reports them).
fn diff_slices<T: Hash + Eq + Copy>(algorithm: Algorithm, old: &[T], new: &[T]) -> Vec<DiffOp> {
    let mut interner = Interner::new(old.len() + new.len());
    let before: Vec<Token> = old.iter().map(|&t| interner.intern(t)).collect();
    let after: Vec<Token> = new.iter().map(|&t| interner.intern(t)).collect();
    let mut diff = Diff::default();
    diff.compute_with(algorithm, &before, &after, interner.num_tokens());

    let mut ops = vec![];
    let (mut old_index, mut new_index) = (0, 0);
    for hunk in diff.hunks() {
        let (old_start, old_end) = (hunk.before.start as usize, hunk.before.end as usize);
        let (new_start, new_end) = (hunk.after.start as usize, hunk.after.end as usize);
        if old_start > old_index {
            ops.push(DiffOp::Equal {
                old_index,
                new_index,
                len: old_start - old_index,
            });
        }
        ops.push(match (old_end - old_start, new_end - new_start) {
            (old_len, 0) => DiffOp::Delete {
                old_index: old_start,
                old_len,
                new_index: new_start,
            },
            (0, new_len) => DiffOp::Insert {
                old_index: old_start,
                new_index: new_start,
                new_len,
            },
            (old_len, new_len) => DiffOp::Replace {
                old_index: old_start,
                old_len,
                new_index: new_start,
                new_len,
            },
        });
        (old_index, new_index) = (old_end, new_end);
    }
    if old_index < old.len() {
        ops.push(DiffOp::Equal {
            old_index,
            new_index,
            len: old.len() - old_index,
        });
    }
    ops
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

#[derive(Clone)]
pub struct FileChangeInput<'a> {
    pub kind: FileChangeKind,
    /// Tokens can only move between files in the same namespace.  See
    /// `LanguageProfile::namespace`.
    pub namespace: &'a str,
    /// The old token lines; empty for `Added`.
    pub old_lines: Vec<&'a str>,
    /// The new token lines; empty for `Deleted`.
    pub new_lines: Vec<&'a str>,
    /// Whether the `searchfox-handler-moves` heuristic applies to the file's
    /// old version (see `infer_handler_moves`).
    pub handler_moves: bool,
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
    /// A match from another file must be at least this many tokens long, or
    /// rare enough (see `cross_file_min_rarity`), or have enough identifiers
    /// (see `cross_file_min_identifiers`): short runs of common tokens (ex: `:
    /// true ,`, `) ; if ( !`, `for the`) match removals in other files by
    /// coincidence, which would blame them on those files' histories.
    pub cross_file_min_len: usize,
    /// How rare a shorter match from another file must be: the sum, over its
    /// identifiers and strings (not keywords, numbers, or comments' words), of
    /// 1 / how many times each occurs in the old and new versions of the
    /// namespace's changed files.  Ex: 0.5 for an `#include`'s path that's
    /// only where it moved from and to, or more for a declaration with
    /// several identifiers that occur a few times each, but ~0 for `: true ,`
    /// or `return NS_OK ;`.
    pub cross_file_min_rarity: f64,
    /// How many distinct identifiers and strings a shorter match from another
    /// file can have instead, ex: a moved declaration's `PRFileDesc *
    /// fileDesc ; nsresult getRv = ...`, whose identifiers occur often in the
    /// files, but not `if ( rv != SECSuccess ) { return`.
    pub cross_file_min_identifiers: usize,
    /// If a namespace has more removed tokens than this in a single revision,
    /// we skip the suffix-array move inference for that namespace.  This is a
    /// safeguard against giant automated rewrites where the inference is both
    /// expensive and unlikely to be useful.
    pub max_removed_tokens_for_moves: usize,
    /// Whether renames' patterns pair other identifiers (see
    /// `infer_rename_patterns`); off only to measure what they add.
    pub rename_patterns: bool,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        InferenceConfig {
            max_candidates: 256,
            max_removed_tokens_for_moves: 4_000_000,
            cross_file_min_len: 16,
            cross_file_min_rarity: 0.25,
            cross_file_min_identifiers: 4,
            rename_patterns: true,
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
        self.record_evolution_between(file, old_idx, file, new_idx);
    }

    /// An evolution from another file's token (see `infer_rename_patterns`).
    fn record_evolution_between(
        &mut self,
        from_file: usize,
        old_idx: usize,
        to_file: usize,
        new_idx: usize,
    ) {
        self.origins[to_file][new_idx] = TokenOrigin::Evolved {
            from_file: from_file as u32,
            old_lineno: old_idx as u32 + 1,
        };
        self.fates[from_file][old_idx] = Some(RemovedFate::EvolvedInto {
            to_file: to_file as u32,
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

    diff_files(&mut state);

    let mut namespaces: Vec<&str> = inputs.iter().map(|i| i.namespace).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    for namespace in namespaces {
        infer_big_rock_moves(&mut state, namespace, config);
    }

    infer_local_recontexting(&mut state);
    infer_evolutions(&mut state);
    if config.rename_patterns {
        infer_rename_patterns(&mut state);
    }
    infer_handler_moves(&mut state);

    finalize(state)
}

/// Pass 1: Diff each file to establish the unchanged tokens and blocks.
fn diff_files(state: &mut InferenceState) {
    for (file, input) in state.inputs.iter().enumerate() {
        let ops = diff_token_lines(&input.old_lines, &input.new_lines);

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

/// The positions of a text which moves have consumed, for finding whether a
/// run of positions has any without looking at each: a bitset of the
/// positions, with summaries (each bit of which says whether a word of the
/// level below has any bits set), so that finding the next consumed position
/// takes a few word operations.  (Runs in a BTreeMap took two searches of it,
/// which, done for each candidate position of each length of match tried,
/// were most of the time of some revisions with big moves.)
struct ConsumedRuns {
    /// The positions' bits, then the summaries of those, and of those.
    levels: [Vec<u64>; 3],
}

impl ConsumedRuns {
    /// For a text of `len` positions.
    fn new(len: usize) -> Self {
        let words = len.div_ceil(64).max(1);
        let summaries = words.div_ceil(64);
        ConsumedRuns {
            levels: [
                vec![0; words],
                vec![0; summaries],
                vec![0; summaries.div_ceil(64)],
            ],
        }
    }

    /// Consume `range`, none of which is consumed.
    fn consume(&mut self, range: std::ops::Range<usize>) {
        for p in range {
            let mut bit = p;
            for level in &mut self.levels {
                level[bit / 64] |= 1 << (bit % 64);
                bit /= 64;
            }
        }
    }

    /// The first consumed position at or after `p` (or `usize::MAX`), so that
    /// a run starting at `p` is unconsumed if it ends by it.
    fn unconsumed_from(&self, p: usize) -> usize {
        self.first_set(0, p).unwrap_or(usize::MAX)
    }

    /// The first set bit at or after `bit` in `level`.
    fn first_set(&self, level: usize, bit: usize) -> Option<usize> {
        let words = &self.levels[level];
        let word = bit / 64;
        let rest = words.get(word)? & (!0u64 << (bit % 64));
        if rest != 0 {
            return Some(word * 64 + rest.trailing_zeros() as usize);
        }
        // (The next word with any set, from the level above, or for the top
        // level, which is a few words, by looking.)
        let next = if level + 1 < self.levels.len() {
            self.first_set(level + 1, word + 1)?
        } else {
            (word + 1..words.len()).find(|&w| words[w] != 0)?
        };
        Some(next * 64 + words[next].trailing_zeros() as usize)
    }
}

/// Same-block move candidates are only considered in this many positions at
/// the start of the removed text.
const SAME_BLOCK_SCAN_LIMIT: usize = 1_000_001;

/// Pass 2: Suffix-array based move inference within a namespace.
fn infer_big_rock_moves(state: &mut InferenceState, namespace: &str, config: &InferenceConfig) {
    let in_namespace = |file: u32| state.inputs[file as usize].namespace == namespace;

    // ## Intern tokens and build the text of all removed tokens.
    let mut interned: HashMap<&str, u32> = HashMap::new();
    // Index 0 is the sentinel.
    let mut id_is_word: Vec<bool> = vec![false];

    let mut text: Vec<u32> = vec![];
    let mut sources: Vec<TextSource> = vec![];
    // The positions in the text of each block's removed tokens, which are
    // contiguous.
    let mut block_text: HashMap<u32, (usize, usize)> = HashMap::new();
    for (block_idx, block) in state.blocks.iter().enumerate() {
        if !block.removals_real || block.old_len == 0 || !in_namespace(block.file) {
            continue;
        }
        let file = block.file as usize;
        let start = text.len();
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
        block_text.insert(block_idx as u32, (start, text.len()));
        text.push(0);
        sources.push(SENTINEL_SOURCE);
    }

    if text.is_empty() || text.len() > config.max_removed_tokens_for_moves {
        return;
    }
    // How many times each removed token occurs in the old and new versions of
    // the namespace's files, for whether matches from other files are rare
    // enough (see `InferenceConfig::cross_file_min_rarity`).
    let mut file_count = vec![0u32; id_is_word.len()];
    for (file, input) in state.inputs.iter().enumerate() {
        if input.namespace != namespace {
            continue;
        }
        for line in state.old_tokens[file].iter().chain(&state.new_tokens[file]) {
            if let Some(&id) = interned.get(line.token) {
                file_count[id as usize] += 1;
            }
        }
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
    let mut consumed = ConsumedRuns::new(text.len());

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
        // word_prefix[i] is the number of word tokens in query[..i], and
        // rarity_prefix[i] the rarity of query[..i] (see
        // `InferenceConfig::cross_file_min_rarity`).
        let mut word_prefix = vec![0u32; query.len() + 1];
        let mut rarity_prefix = vec![0f64; query.len() + 1];
        // (Whether each token is an identifier or a string.)
        let mut is_identifier = vec![false; query.len()];
        for (i, &id) in query.iter().enumerate() {
            let is_word = id != u32::MAX && id_is_word[id as usize];
            word_prefix[i + 1] = word_prefix[i] + is_word as u32;
            is_identifier[i] = is_word
                && matches!(
                    state.new_tokens[file][new_range.start + i].effective_class(),
                    TokenClass::Identifier | TokenClass::String
                );
            rarity_prefix[i + 1] = rarity_prefix[i]
                + if is_identifier[i] {
                    1.0 / file_count[id as usize] as f64
                } else {
                    0.0
                };
        }
        let distinct_identifiers = |range: std::ops::Range<usize>| {
            let mut ids: Vec<u32> = range
                .filter(|&j| is_identifier[j])
                .map(|j| query[j])
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids.len()
        };

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
                // (A match from another file has to be long or have a rare
                // word; see `InferenceConfig::cross_file_min_len`.)
                let distinctive = k >= config.cross_file_min_len
                    || rarity_prefix[i + k] - rarity_prefix[i] >= config.cross_file_min_rarity
                    || distinct_identifiers(i..i + k) >= config.cross_file_min_identifiers;
                // (Whether the text at `p`, which starts with `query[i..i +
                // k]`, can be the match.  Checking that each candidate's text
                // matches, and that none of it is consumed, token by token,
                // was quadratic in the matches' lengths, which in the HTML
                // Standard's revisions which edited all of its `source` and
                // the `index` generated from it, which repeat each other, are
                // hundreds of thousands of tokens: a revision took 10 minutes.)
                let is_usable_match = |p: usize| -> bool {
                    consumed.unconsumed_from(p) >= p + k
                        && (distinctive || sources[p].file as usize == file)
                };
                // (For positions which aren't from the suffix array's range
                // of matches.)
                let is_usable = |p: usize| -> bool {
                    p + k <= text.len() && text[p..p + k] == query[i..i + k] && is_usable_match(p)
                };

                let mut candidates: Vec<usize> = (lo..hi.min(lo + config.max_candidates))
                    .map(|r| sa.suffix(r))
                    .filter(|&p| is_usable_match(p))
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
                    // Same-block candidates, from whichever is smaller: the
                    // block's positions or the positions which match.  (Only
                    // within the first SAME_BLOCK_SCAN_LIMIT positions of the
                    // text, which is how far this used to scan.)
                    if let Some(&(start, end)) = block_text.get(&(block_idx as u32)) {
                        let end = end.min(SAME_BLOCK_SCAN_LIMIT);
                        if end.saturating_sub(start) <= hi - lo {
                            candidates.extend((start..end).filter(|&p| is_usable(p)));
                        } else {
                            candidates.extend(
                                (lo..hi)
                                    .map(|r| sa.suffix(r))
                                    .filter(|&p| p >= start && p < end && is_usable_match(p)),
                            );
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
                    consumed.consume(p..p + k);
                    for j in 0..k {
                        let source = sources[p + j];
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
fn infer_local_recontexting(state: &mut InferenceState) {
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
        // Myers diff matches as many tokens as it can, which is what we want
        // here; imara-diff's non-minimal variant bounds the cost of blocks
        // which barely match (deterministically, unlike a deadline).
        let ops = diff_slices(Algorithm::Myers, &old_texts, &new_texts);
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
/// Unequal gaps (see `infer_evolutions`) with up to this many tokens on each
/// side are aligned by their tokens' similarity.
const MAX_HOLE_TOKENS: usize = 16;

/// How similar (see `subword_similarity`) an identifier or string in a gap
/// must be to one on the other side to evolve from it when the gap's sides
/// differ in size.
const MIN_HOLE_SIMILARITY: f64 = 0.5;

/// The lowercase subwords of an identifier or string, sorted and
/// deduplicated (see `subword_seq`).
fn subwords(token: &str) -> Vec<String> {
    let mut words = subword_seq(token);
    words.sort();
    words.dedup();
    words
}

/// The lowercase subwords of an identifier or string in order, split at non-
/// alphanumeric characters, and at camelCase, ex: `["url", "parser", "for",
/// "x"]` for `URLParser_forX`.
fn subword_seq(token: &str) -> Vec<String> {
    let mut words = vec![];
    let mut word = String::new();
    let chars: Vec<char> = token.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        // (A new word at a lowercase-to-uppercase change, or at an uppercase
        // letter followed by a lowercase one after uppercase ones.)
        let boundary = c.is_uppercase()
            && i > 0
            && (chars[i - 1].is_lowercase()
                || chars[i - 1].is_ascii_digit()
                || (chars[i - 1].is_uppercase()
                    && chars.get(i + 1).is_some_and(|next| next.is_lowercase())));
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// The Dice coefficient of two tokens' subwords (see `subwords`) of at least
/// 3 characters (ex: not `ns` and `h` in `"nsIFile.h"` and
/// `"nsShellService.h"`).
fn subword_similarity(a: &str, b: &str) -> f64 {
    let long = |token: &str| -> Vec<String> {
        subwords(token)
            .into_iter()
            .filter(|word| word.chars().count() >= 3)
            .collect()
    };
    let (a, b) = (long(a), long(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let common = a.iter().filter(|word| b.contains(word)).count();
    2.0 * common as f64 / (a.len() + b.len()) as f64
}

/// The pairs of an unequal gap's tokens which evolved: identifiers and
/// strings similar enough to the other side's (see `MIN_HOLE_SIMILARITY`), as
/// the alignment (in order) with the greatest total similarity.  Ex:
/// `DtlsIdentity :: DEFAULT_HASH_ALGORITHM` becoming
/// `DEFAULT_DTLS_HASH_ALGORITHM`, but not `EventUtils . sendMouseEvent`
/// becoming `clickOnRequestRow`, which was rewritten.
fn hole_pairs(
    state: &InferenceState,
    file: usize,
    old_gap: std::ops::Range<usize>,
    new_gap: std::ops::Range<usize>,
) -> Vec<(usize, usize)> {
    let (olds, news): (Vec<usize>, Vec<usize>) = (old_gap.collect(), new_gap.collect());
    let score = |o: usize, n: usize| -> f64 {
        let (old, new) = (&state.old_tokens[file][o], &state.new_tokens[file][n]);
        let class = new.effective_class();
        if class != old.effective_class()
            || !matches!(class, TokenClass::Identifier | TokenClass::String)
        {
            return 0.0;
        }
        let similarity = subword_similarity(old.token, new.token);
        if similarity >= MIN_HOLE_SIMILARITY {
            similarity
        } else {
            0.0
        }
    };
    // best[i][j]: the best total for olds[i..] and news[j..].
    let (m, n) = (olds.len(), news.len());
    let mut best = vec![vec![0.0f64; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let paired = score(olds[i], news[j]);
            let with = if paired > 0.0 {
                paired + best[i + 1][j + 1]
            } else {
                0.0
            };
            best[i][j] = with.max(best[i + 1][j]).max(best[i][j + 1]);
        }
    }
    let mut pairs = vec![];
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        let paired = score(olds[i], news[j]);
        if paired > 0.0 && best[i][j] == paired + best[i + 1][j + 1] {
            pairs.push((olds[i], news[j]));
            i += 1;
            j += 1;
        } else if best[i][j] == best[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

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
            if new_gap.is_empty() || old_gap.is_empty() {
                continue;
            }
            // Both sides of the gap must consist entirely of unmatched tokens;
            // see the module docs.
            if !new_gap.clone().all(|n| state.is_unmatched_new(file, n))
                || !old_gap.clone().all(|o| state.is_unmatched_old(file, o))
            {
                continue;
            }
            // Unequal sides (ex: a qualified name becoming another name) only
            // pair similar tokens; see `hole_pairs`.
            if new_gap.len() != old_gap.len() {
                if new_gap.len() <= MAX_HOLE_TOKENS && old_gap.len() <= MAX_HOLE_TOKENS {
                    pairs.extend(hole_pairs(state, file, old_gap, new_gap));
                }
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
    infer_revision_renames(state);
}

/// A rename's old and new tokens' indices in a block (see
/// `infer_revision_renames`).
type RenameTokens = (Vec<usize>, Vec<usize>);

/// Identifiers which evolved somewhere in the revision (ex: a renamed type's
/// uses in the slots `infer_evolutions` pairs) evolve the same way wherever
/// else both are left unmatched in a block, in order, since renames are
/// usually consistent across a revision: ex: a renamed function's call in a
/// line that was otherwise rewritten.  (If the rename was seen at least
/// twice, or is between similar names.)
fn infer_revision_renames(state: &mut InferenceState) {
    // The renames, old text -> (new text, how many times).
    let mut renames: HashMap<&str, (&str, usize)> = HashMap::new();
    let mut ambiguous: HashSet<&str> = HashSet::new();
    for (file, origins) in state.origins.iter().enumerate() {
        for (new_idx, origin) in origins.iter().enumerate() {
            let TokenOrigin::Evolved {
                from_file,
                old_lineno,
            } = *origin
            else {
                continue;
            };
            let new = &state.new_tokens[file][new_idx];
            let old = &state.old_tokens[from_file as usize][old_lineno as usize - 1];
            if new.effective_class() != TokenClass::Identifier
                || old.effective_class() != TokenClass::Identifier
            {
                continue;
            }
            // (An old name which became several different names isn't a
            // rename to apply elsewhere.)
            match renames.get_mut(old.token) {
                Some((existing, _)) if *existing != new.token => {
                    ambiguous.insert(old.token);
                }
                Some((_, count)) => *count += 1,
                None => {
                    renames.insert(old.token, (new.token, 1));
                }
            }
        }
    }
    // (Positional evolutions can be wrong, ex: `queryString` to `isClass` in a
    // rewritten line, so only renames seen at least twice, or between similar
    // names, are applied elsewhere.)
    let renames: HashMap<&str, &str> = renames
        .into_iter()
        .filter(|(old, (new, count))| {
            !ambiguous.contains(old)
                && (*count >= 2 || subword_similarity(old, new) >= MIN_HOLE_SIMILARITY)
        })
        .map(|(old, (new, _))| (old, new))
        .collect();
    if renames.is_empty() {
        return;
    }
    let mut pairs = vec![];
    for block in &state.blocks {
        if !block.removals_real || block.old_len == 0 || block.new_len == 0 {
            continue;
        }
        let file = block.file as usize;
        // Each rename's unmatched old and new tokens in the block, in order.
        // (In a stable order, since renames to the same new name compete for
        // its tokens.)
        let mut candidates: BTreeMap<(&str, &str), RenameTokens> = BTreeMap::new();
        for old_idx in block.old_range() {
            let token = &state.old_tokens[file][old_idx];
            if state.is_unmatched_old(file, old_idx)
                && token.effective_class() == TokenClass::Identifier
                && let Some(new) = renames.get(token.token)
            {
                candidates
                    .entry((token.token, new))
                    .or_default()
                    .0
                    .push(old_idx);
            }
        }
        for new_idx in block.new_range() {
            let token = &state.new_tokens[file][new_idx];
            if !state.is_unmatched_new(file, new_idx)
                || token.effective_class() != TokenClass::Identifier
            {
                continue;
            }
            for ((_, new), (_, news)) in candidates.iter_mut() {
                if *new == token.token {
                    news.push(new_idx);
                }
            }
        }
        for (olds, news) in candidates.into_values() {
            pairs.extend(olds.into_iter().zip(news).map(|(o, n)| (file, o, n)));
        }
    }
    for (file, o, n) in pairs {
        if state.is_unmatched_old(file, o) && state.is_unmatched_new(file, n) {
            state.record_evolution(file, o, n);
        }
    }
}

/// How a rename changed its name's subwords (see `subword_seq`), ex:
/// `RemoteWorkerSuspendOp` becoming `SharedWorkerSuspendOpArgs` is
/// `Replace(["remote"], ["shared"])` and `Append(["args"])`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum RenameRule {
    Replace(Vec<String>, Vec<String>),
    Prepend(Vec<String>),
    Append(Vec<String>),
    /// Subwords added between others.
    Insert(Vec<String>),
    DropStart(Vec<String>),
    DropEnd(Vec<String>),
    /// Subwords dropped from between others.
    Drop(Vec<String>),
}

/// Where `words` first occurs in `seq`, from `from` and ending before `end`.
fn find_words(seq: &[String], words: &[String], from: usize, end: usize) -> Option<usize> {
    if words.is_empty() || words.len() > end.saturating_sub(from) {
        return None;
    }
    (from..=end - words.len()).find(|&i| seq[i..i + words.len()] == *words)
}

/// Where `words` first occurs in `seq` between other subwords.
fn find_interior(seq: &[String], words: &[String]) -> Option<usize> {
    find_words(seq, words, 1, seq.len().saturating_sub(1))
}

/// `seq` with `len` subwords at `at` replaced by `with`.
fn splice(seq: &[String], at: usize, len: usize, with: &[String]) -> Vec<String> {
    let mut spliced = seq[..at].to_vec();
    spliced.extend_from_slice(with);
    spliced.extend_from_slice(&seq[at + len..]);
    spliced
}

impl RenameRule {
    /// The name `seq` becomes by this rule, if it applies (not `Insert`,
    /// whose position isn't known).
    fn apply(&self, seq: &[String]) -> Option<Vec<String>> {
        match self {
            RenameRule::Replace(from, to) => {
                find_words(seq, from, 0, seq.len()).map(|i| splice(seq, i, from.len(), to))
            }
            RenameRule::Prepend(words) => Some(splice(seq, 0, 0, words)),
            RenameRule::Append(words) => Some(splice(seq, seq.len(), 0, words)),
            RenameRule::Insert(_) => None,
            RenameRule::DropStart(words) => (seq.len() > words.len() && seq.starts_with(words))
                .then(|| seq[words.len()..].to_vec()),
            RenameRule::DropEnd(words) => (seq.len() > words.len() && seq.ends_with(words))
                .then(|| seq[..seq.len() - words.len()].to_vec()),
            RenameRule::Drop(words) => {
                find_interior(seq, words).map(|i| splice(seq, i, words.len(), &[]))
            }
        }
    }

    /// The name `seq` was before this rule, if it applies (not `Drop`, whose
    /// position isn't known).
    fn unapply(&self, seq: &[String]) -> Option<Vec<String>> {
        match self {
            RenameRule::Replace(from, to) => {
                find_words(seq, to, 0, seq.len()).map(|i| splice(seq, i, to.len(), from))
            }
            RenameRule::Prepend(words) => (seq.len() > words.len() && seq.starts_with(words))
                .then(|| seq[words.len()..].to_vec()),
            RenameRule::Append(words) => (seq.len() > words.len() && seq.ends_with(words))
                .then(|| seq[..seq.len() - words.len()].to_vec()),
            RenameRule::Insert(words) => {
                find_interior(seq, words).map(|i| splice(seq, i, words.len(), &[]))
            }
            RenameRule::DropStart(words) => Some(splice(seq, 0, 0, words)),
            RenameRule::DropEnd(words) => Some(splice(seq, seq.len(), 0, words)),
            RenameRule::Drop(_) => None,
        }
    }
}

/// A rename's rules (see `RenameRule`), from the alignment of its names'
/// subwords, and whether the names share any.
fn rename_rules(old: &[String], new: &[String]) -> (Vec<RenameRule>, bool) {
    let old_words: Vec<&str> = old.iter().map(String::as_str).collect();
    let new_words: Vec<&str> = new.iter().map(String::as_str).collect();
    let mut rules = vec![];
    let mut shared = false;
    for op in diff_slices(Algorithm::Myers, &old_words, &new_words) {
        let rule = match op {
            DiffOp::Equal { .. } => {
                shared = true;
                continue;
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => RenameRule::Replace(
                old[old_index..old_index + old_len].to_vec(),
                new[new_index..new_index + new_len].to_vec(),
            ),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => {
                let words = new[new_index..new_index + new_len].to_vec();
                if old_index == 0 {
                    RenameRule::Prepend(words)
                } else if old_index == old.len() {
                    RenameRule::Append(words)
                } else {
                    RenameRule::Insert(words)
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                let words = old[old_index..old_index + old_len].to_vec();
                if old_index == 0 {
                    RenameRule::DropStart(words)
                } else if old_index + old_len == old.len() {
                    RenameRule::DropEnd(words)
                } else {
                    RenameRule::Drop(words)
                }
            }
        };
        rules.push(rule);
    }
    rules.sort();
    rules.dedup();
    (rules, shared)
}

/// A rule must be seen in this many of a revision's renames (distinct pairs
/// of names sharing a subword) to pair other names.
const MIN_RULE_RENAMES: usize = 2;

/// At most this many of a revision's rules (the most seen) pair other names,
/// which bounds the work of big rewrites, which have many.
const MAX_RULES: usize = 64;

/// A revision's rules (see `infer_rename_patterns`).
struct RenameRules {
    list: Vec<RenameRule>,
    set: HashSet<RenameRule>,
}

impl RenameRules {
    /// How many rules `old` became `new` by, if it could have by only the
    /// revision's.
    fn rename(&self, old: &str, new: &str) -> Option<usize> {
        if old == new {
            return None;
        }
        let (rules, _) = rename_rules(&subword_seq(old), &subword_seq(new));
        (!rules.is_empty() && rules.iter().all(|rule| self.set.contains(rule)))
            .then_some(rules.len())
    }

    /// For each of `olds`, the one of `news` it could have become by the
    /// fewest rules (then the first by name), of those `allowed`, and whether
    /// no other could have.  (Meeting in the middle: each old name after a
    /// rule, and each new name before one, so renames by up to two rules
    /// meet.)
    fn candidates<'t>(
        &self,
        olds: &BTreeSet<&'t str>,
        news: &BTreeSet<&'t str>,
        allowed: impl Fn(&str, &str) -> bool,
    ) -> BTreeMap<&'t str, (&'t str, bool)> {
        let key = |seq: &[String]| seq.join("\u{1f}");
        let mut by_key: HashMap<String, Vec<&'t str>> = HashMap::new();
        for &new in news {
            let seq = subword_seq(new);
            let mut keys: Vec<String> = self
                .list
                .iter()
                .filter_map(|rule| rule.unapply(&seq))
                .map(|seq| key(&seq))
                .collect();
            keys.push(key(&seq));
            keys.sort();
            keys.dedup();
            for k in keys {
                by_key.entry(k).or_default().push(new);
            }
        }
        let mut best = BTreeMap::new();
        for &old in olds {
            let seq = subword_seq(old);
            let mut keys: Vec<String> = self
                .list
                .iter()
                .filter_map(|rule| rule.apply(&seq))
                .map(|seq| key(&seq))
                .collect();
            keys.push(key(&seq));
            let mut found: BTreeSet<(usize, &'t str)> = BTreeSet::new();
            for k in keys {
                for &new in by_key.get(&k).into_iter().flatten() {
                    if let Some(rules) = self.rename(old, new)
                        && allowed(old, new)
                    {
                        found.insert((rules, new));
                    }
                }
            }
            if let Some(&(_, new)) = found.first() {
                let unique = found.iter().all(|&(_, other)| other == new);
                best.insert(old, (new, unique));
            }
        }
        best
    }
}

/// The names in a block or namespace of unmatched identifiers (old or new),
/// with their tokens' positions (file and index) in order.
type NamePositions<'a> = BTreeMap<&'a str, Vec<(usize, usize)>>;

/// Pass 5: Renames' patterns.  A revision which renames things usually
/// renames them alike: ex: D198005 (the PRemoteWorker refactoring) renamed
/// `RemoteWorkerOp` to `SharedWorkerOpArgs`, `RemoteWorkerSuspendOp` to
/// `SharedWorkerSuspendOpArgs`, and so on, "Remote" becoming "Shared" with
/// "Args" appended, which slots pair where only the names changed (in
/// RemoteWorkerController.cpp), but not where the structs they name moved to
/// another file with too little else to match (PRemoteWorker.ipdl's to
/// SharedWorkerOpArgs.ipdlh).  So the revision's identifier evolutions whose
/// names share a subword give rules (see `RenameRule`), and those seen in at
/// least `MIN_RULE_RENAMES` of them pair other unmatched identifiers whose
/// names differ by only such rules (and at most two of them; see
/// `RenameRules::candidates`): in each block, as `infer_revision_renames`
/// does, if the names share a subword or the revision renamed the name (the
/// old name isn't in any of its files' new versions, nor the new name in
/// their old versions), then across each namespace's blocks (ex: other
/// files'), if the revision renamed the name, the names pair uniquely, and
/// each name's unmatched tokens are in one file, where each pair anchors the identical unmatched tokens around it (ex: the
/// structs' `struct` and `{ } ;`) as moved with it (see `extend_anchor`).
/// (Rules are learned across namespaces, as above, where the renames were in
/// C++ and the structs in IPDL.)  On mozsearch's history, without those
/// requirements, rules paired `info` (the `info!` macro) with `config` (from
/// `tree_info` becoming `tree_config`) in a block, and across files,
/// `serde_json` with `serde` (from `fields_json` becoming `fields`), and
/// `data` with `data_str`; and in a firefox window, `aCertDer` in 4 files
/// with `aCertId` in 1, in the order of the files.  With them, on
/// mozsearch's history, 51 tokens' origins change in 15 of 2,751 revisions
/// (ex: `applyDiagramOptions` to `applyOptions`, `getter_name` to
/// `cpp_getter_name`), and in a firefox window (2026-09-13 to 27), 780 in 35
/// of 4,061 (ex: uniffi's generated `logins` names becoming `db_crypto`
/// ones, and `generator` becoming `mGenerator`), 11 across files, and 185
/// moves anchored, for ~3% more inference time.
fn infer_rename_patterns(state: &mut InferenceState) {
    // ## The revision's rules.
    let mut renames: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (file, origins) in state.origins.iter().enumerate() {
        for (new_idx, origin) in origins.iter().enumerate() {
            let TokenOrigin::Evolved {
                from_file,
                old_lineno,
            } = *origin
            else {
                continue;
            };
            let new = &state.new_tokens[file][new_idx];
            let old = &state.old_tokens[from_file as usize][old_lineno as usize - 1];
            if new.effective_class() == TokenClass::Identifier
                && old.effective_class() == TokenClass::Identifier
                && old.token != new.token
            {
                renames.insert((old.token, new.token));
            }
        }
    }
    let mut seen: HashMap<RenameRule, usize> = HashMap::new();
    for (old, new) in renames {
        let (rules, shared) = rename_rules(&subword_seq(old), &subword_seq(new));
        if shared {
            for rule in rules {
                *seen.entry(rule).or_default() += 1;
            }
        }
    }
    let mut seen: Vec<(RenameRule, usize)> = seen
        .into_iter()
        .filter(|(_, count)| *count >= MIN_RULE_RENAMES)
        .collect();
    if seen.is_empty() {
        return;
    }
    seen.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    seen.truncate(MAX_RULES);
    let list: Vec<RenameRule> = seen.into_iter().map(|(rule, _)| rule).collect();
    let rules = RenameRules {
        set: list.iter().cloned().collect(),
        list,
    };
    let is_identifier = |line: &TokenLine| line.effective_class() == TokenClass::Identifier;
    // (Whether the revision renamed a name: see above.)
    fn names<'a>(tokens: &[Vec<TokenLine<'a>>]) -> HashSet<&'a str> {
        tokens
            .iter()
            .flatten()
            .filter(|line| line.effective_class() == TokenClass::Identifier)
            .map(|line| line.token)
            .collect()
    }
    let (old_names, new_names) = (names(&state.old_tokens), names(&state.new_tokens));
    let renamed = |old: &str, new: &str| !new_names.contains(old) && !old_names.contains(new);
    let share_subword = |old: &str, new: &str| {
        let (old, new) = (subword_seq(old), subword_seq(new));
        old.iter().any(|word| new.contains(word))
    };

    // ## In each block.
    let mut pairs = vec![];
    for block in &state.blocks {
        if !block.removals_real || block.old_len == 0 || block.new_len == 0 {
            continue;
        }
        let file = block.file as usize;
        let mut olds: NamePositions = BTreeMap::new();
        for old_idx in block.old_range() {
            let line = &state.old_tokens[file][old_idx];
            if state.is_unmatched_old(file, old_idx) && is_identifier(line) {
                olds.entry(line.token).or_default().push((file, old_idx));
            }
        }
        let mut news: NamePositions = BTreeMap::new();
        for new_idx in block.new_range() {
            let line = &state.new_tokens[file][new_idx];
            if state.is_unmatched_new(file, new_idx) && is_identifier(line) {
                news.entry(line.token).or_default().push((file, new_idx));
            }
        }
        if olds.is_empty() || news.is_empty() {
            continue;
        }
        let candidates = rules.candidates(
            &olds.keys().copied().collect(),
            &news.keys().copied().collect(),
            |old, new| share_subword(old, new) || renamed(old, new),
        );
        // (Renames to the same name compete for its tokens, in the names'
        // order, as in `infer_revision_renames`.)
        for (old, (new, _)) in candidates {
            pairs.extend(olds[old].iter().zip(&news[new]).map(|(&o, &n)| (o, n)));
        }
    }
    for ((file, o), (_, n)) in pairs {
        if state.is_unmatched_old(file, o) && state.is_unmatched_new(file, n) {
            state.record_evolution(file, o, n);
        }
    }

    // ## Across each namespace's blocks, for names which pair uniquely.
    let mut namespaces: Vec<&str> = state.inputs.iter().map(|i| i.namespace).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    for namespace in namespaces {
        let mut olds: NamePositions = BTreeMap::new();
        let mut news: NamePositions = BTreeMap::new();
        for block in &state.blocks {
            let file = block.file as usize;
            // (Not copies' tokens, which fall back to their copy's.)
            if !block.removals_real || state.inputs[file].namespace != namespace {
                continue;
            }
            for old_idx in block.old_range() {
                let line = &state.old_tokens[file][old_idx];
                if state.is_unmatched_old(file, old_idx) && is_identifier(line) {
                    olds.entry(line.token).or_default().push((file, old_idx));
                }
            }
            for new_idx in block.new_range() {
                let line = &state.new_tokens[file][new_idx];
                if state.is_unmatched_new(file, new_idx) && is_identifier(line) {
                    news.entry(line.token).or_default().push((file, new_idx));
                }
            }
        }
        if olds.is_empty() || news.is_empty() {
            continue;
        }
        let candidates = rules.candidates(
            &olds.keys().copied().collect(),
            &news.keys().copied().collect(),
            renamed,
        );
        let mut claims: HashMap<&str, usize> = HashMap::new();
        for (new, _) in candidates.values() {
            *claims.entry(new).or_default() += 1;
        }
        // (And whose tokens are in a file on each side: pairing a name's
        // tokens in several files with another's would be arbitrary.)
        let one_file = |positions: &[(usize, usize)]| {
            positions.iter().all(|&(file, _)| file == positions[0].0)
        };
        let mut anchors = vec![];
        for (old, (new, unique)) in &candidates {
            if !unique || claims[new] != 1 || !one_file(&olds[old]) || !one_file(&news[new]) {
                continue;
            }
            for (&(from_file, o), &(to_file, n)) in olds[old].iter().zip(&news[new]) {
                if state.is_unmatched_old(from_file, o) && state.is_unmatched_new(to_file, n) {
                    state.record_evolution_between(from_file, o, to_file, n);
                    anchors.push((from_file, o, to_file, n));
                }
            }
        }
        for (from_file, o, to_file, n) in anchors {
            extend_anchor(state, from_file, o, to_file, n);
        }
    }
}

/// Record the identical unmatched tokens around a pair of renamed
/// identifiers from different blocks (see `infer_rename_patterns`) as moved
/// with it, outward on each side until a token differs or was matched.
fn extend_anchor(
    state: &mut InferenceState,
    from_file: usize,
    old_idx: usize,
    to_file: usize,
    new_idx: usize,
) {
    for step in [-1isize, 1] {
        let (mut o, mut n) = (old_idx, new_idx);
        loop {
            let (Some(next_o), Some(next_n)) =
                (o.checked_add_signed(step), n.checked_add_signed(step))
            else {
                break;
            };
            (o, n) = (next_o, next_n);
            let (Some(old), Some(new)) = (
                state.old_tokens[from_file].get(o),
                state.new_tokens[to_file].get(n),
            ) else {
                break;
            };
            if old.token != new.token
                || old.effective_class() != new.effective_class()
                || !state.is_unmatched_old(from_file, o)
                || !state.is_unmatched_new(to_file, n)
            {
                break;
            }
            state.record_move(from_file, o, to_file, n);
        }
    }
}

/// The element id and event of a markup event handler's code, from its
/// context (see `markup_tokenizer`): ex: `allTabsMenu-searchTabs` and
/// `oncommand` for `#allTabsMenu::#allTabsMenu-searchTabs@oncommand`, and
/// the context up to the handler, which its code's tokens share.
fn handler_of(context: &str) -> Option<(&str, &str, &str)> {
    let mut start = 0;
    for segment in context.split("::") {
        let end = start + segment.len();
        if let Some((element, event)) = segment.split_once('@')
            && event.starts_with("on")
        {
            let id = element.strip_prefix('#')?;
            return (!id.is_empty()).then_some((id, event, &context[..end]));
        }
        start = end + 2;
    }
    None
}

/// A string token's text without its quotes, if it's a quoted string.
fn string_text(token: &str) -> Option<&str> {
    let quote = token.chars().next()?;
    (matches!(quote, '"' | '\'' | '`') && token.len() >= 2 && token.ends_with(quote))
        .then(|| &token[1..token.len() - 1])
}

/// Pass 6: Inline event handlers, for files with the `searchfox-handler-moves`
/// history attribute (see `history_config`), which asuth wanted for the
/// directories where Firefox's UI moved its XUL (and later XHTML) documents'
/// handlers into JS, for CSP, since it's a heuristic.  Handlers were mostly
/// rewritten as they moved, ex: Bug 1895490's `oncommand=
/// "gTabsPanel.searchTabs();"` on `#allTabsMenu-searchTabs` became `case
/// "allTabsMenu-searchTabs": this.searchTabs(); break;` in a listener's
/// `switch (target.id)`, so big rocks only found verbatim pieces (22% of the
/// handlers' tokens on 10 such commits).  The element's id is what connects
/// them: a handler's unmatched code (in its element's context; see
/// `handler_of`) is aligned with the unmatched tokens after a string of its
/// element's id which the revision added in a file of the same namespace (if
/// exactly one), up to a few times the handler's length: identical tokens
/// moved, and the words between them, if as many on each side and of the
/// same classes, evolved (ex: `gTabsPanel` into `this`).
fn infer_handler_moves(state: &mut InferenceState) {
    if !state.inputs.iter().any(|input| input.handler_moves) {
        return;
    }
    // The strings the revision added, by their text: (file, index).
    let mut anchors: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for (file, tokens) in state.new_tokens.iter().enumerate() {
        for (idx, token) in tokens.iter().enumerate() {
            if state.is_unmatched_new(file, idx)
                && token.effective_class() == TokenClass::String
                && let Some(text) = string_text(token.token)
                && !text.is_empty()
            {
                anchors.entry(text).or_default().push((file, idx));
            }
        }
    }
    if anchors.is_empty() {
        return;
    }
    let mut runs = vec![];
    for (file, input) in state.inputs.iter().enumerate() {
        if !input.handler_moves {
            continue;
        }
        let tokens = &state.old_tokens[file];
        let mut i = 0;
        while i < tokens.len() {
            let handler = state
                .is_unmatched_old(file, i)
                .then(|| handler_of(tokens[i].context))
                .flatten();
            let Some((id, _, prefix)) = handler else {
                i += 1;
                continue;
            };
            let start = i;
            while i < tokens.len()
                && state.is_unmatched_old(file, i)
                && handler_of(tokens[i].context).is_some_and(|(_, _, p)| p == prefix)
            {
                i += 1;
            }
            if let Some([(to_file, at)]) = anchors.get(id).map(|a| a.as_slice())
                && state.inputs[*to_file].namespace == input.namespace
            {
                runs.push((file, start..i, *to_file, *at));
            }
        }
    }
    for (file, run, to_file, at) in runs {
        let window: Vec<usize> = (at + 1..state.new_tokens[to_file].len())
            .filter(|&n| state.is_unmatched_new(to_file, n))
            .take(run.len() * 3 + 16)
            .collect();
        let olds: Vec<&str> = run
            .clone()
            .map(|o| state.old_tokens[file][o].token)
            .collect();
        let news: Vec<&str> = window
            .iter()
            .map(|&n| state.new_tokens[to_file][n].token)
            .collect();
        let mut moves = vec![];
        let mut evolutions = vec![];
        for op in diff_slices(Algorithm::Myers, &olds, &news) {
            match op {
                DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => {
                    for j in 0..len {
                        moves.push((run.start + old_index + j, window[new_index + j]));
                    }
                }
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    // (The words between them, if as many on each side, ex:
                    // `gTabsPanel` and `: this`'s `this`.)
                    let olds: Vec<usize> = (run.start + old_index..run.start + old_index + old_len)
                        .filter(|&o| is_word_token(state.old_tokens[file][o].token))
                        .collect();
                    let news: Vec<usize> = window[new_index..new_index + new_len]
                        .iter()
                        .copied()
                        .filter(|&n| is_word_token(state.new_tokens[to_file][n].token))
                        .collect();
                    if olds.len() == news.len() {
                        for (o, n) in olds.into_iter().zip(news) {
                            if state.old_tokens[file][o].effective_class()
                                == state.new_tokens[to_file][n].effective_class()
                            {
                                evolutions.push((o, n));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // (Only where the alignment found the handler's code: a few
        // identical tokens, not just punctuation by chance.)
        let words = moves
            .iter()
            .filter(|&&(o, _)| is_word_token(state.old_tokens[file][o].token))
            .count();
        if words == 0 {
            continue;
        }
        for (o, n) in moves {
            if state.is_unmatched_old(file, o) && state.is_unmatched_new(to_file, n) {
                state.record_move(file, o, to_file, n);
            }
        }
        for (o, n) in evolutions {
            if state.is_unmatched_old(file, o) && state.is_unmatched_new(to_file, n) {
                state.record_evolution_between(file, o, to_file, n);
            }
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
    #[test]
    fn test_consumed_runs() {
        let mut consumed = ConsumedRuns::new(100);
        assert_eq!(consumed.unconsumed_from(5), usize::MAX);
        consumed.consume(10..20);
        consumed.consume(30..31);
        // Before a run, the run's start; in it, the position itself; after
        // the last, none.
        assert_eq!(consumed.unconsumed_from(0), 10);
        assert_eq!(consumed.unconsumed_from(10), 10);
        assert_eq!(consumed.unconsumed_from(19), 19);
        assert_eq!(consumed.unconsumed_from(20), 30);
        assert_eq!(consumed.unconsumed_from(31), usize::MAX);
        // (So a run of 10 from 0 is unconsumed, but not one of 11.)
        assert!(consumed.unconsumed_from(0) >= 10);
        assert!(consumed.unconsumed_from(0) < 11);
        consumed.consume(5..5);
        assert_eq!(consumed.unconsumed_from(0), 10);
    }

    #[test]
    fn test_consumed_runs_like_btreemap() {
        // (Against runs in a BTreeMap, as before, over a text big enough
        // for every level, with runs sparse enough that finding the next one
        // skips whole summary words.)
        let len = 1_000_000;
        let mut consumed = ConsumedRuns::new(len);
        let mut runs: std::collections::BTreeMap<usize, usize> = Default::default();
        let reference = |runs: &std::collections::BTreeMap<usize, usize>, p: usize| {
            if let Some((_, &end)) = runs.range(..=p).next_back()
                && end > p
            {
                return p;
            }
            runs.range(p + 1..)
                .next()
                .map_or(usize::MAX, |(&start, _)| start)
        };
        let mut seed: u64 = 1;
        let mut random = |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        for round in 0..300 {
            let start = random(len);
            let end = (start + 1 + random(if round % 3 == 0 { 5000 } else { 50 })).min(len);
            if consumed.unconsumed_from(start) >= end {
                consumed.consume(start..end);
                runs.insert(start, end);
            }
            for _ in 0..200 {
                let p = random(len);
                assert_eq!(consumed.unconsumed_from(p), reference(&runs, p), "at {}", p);
            }
        }
        assert_eq!(consumed.unconsumed_from(len - 1), reference(&runs, len - 1));
    }

    use super::*;
    use crate::file_format::history::syntax_files::format_token_line;

    /// Keywords for the purposes of our test helper; everything else gets
    /// `TokenClass::guess`.
    const TEST_KEYWORDS: &[&str] = &[
        "class", "const", "continue", "else", "fn", "for", "if", "let", "mut", "protocol",
        "return", "struct", "while",
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
            handler_moves: false,
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
    fn test_diff_ops() {
        // The ops tile both sides, equal runs are equal, and a removal next
        // to an addition is a replacement.
        let old: Vec<&str> = "a b } x y } c d".split(' ').collect();
        let new: Vec<&str> = "a b } z } c d e".split(' ').collect();
        let ops = diff_token_lines(&old, &new);
        let (mut old_pos, mut new_pos) = (0, 0);
        for op in &ops {
            assert_eq!(op.old_range().start, old_pos, "{:?}", ops);
            assert_eq!(op.new_range().start, new_pos, "{:?}", ops);
            if let DiffOp::Equal { .. } = op {
                assert_eq!(old[op.old_range()], new[op.new_range()]);
            }
            (old_pos, new_pos) = (op.old_range().end, op.new_range().end);
        }
        assert_eq!((old_pos, new_pos), (old.len(), new.len()));
        assert!(matches!(
            ops[1],
            DiffOp::Replace {
                old_index: 3,
                old_len: 2,
                new_index: 3,
                new_len: 1,
            }
        ));
        assert!(diff_token_lines(&[], &[]).is_empty());
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
    fn test_hole_evolution() {
        // A qualified name becoming a similar unqualified one evolves (but its
        // scope is removed), and a rewritten call doesn't.
        let old = toks(
            "f",
            "x = DtlsIdentity :: DEFAULT_HASH_ALGORITHM ; EventUtils . sendMouseEvent ( a ) ;",
        );
        let new = toks(
            "f",
            "x = DEFAULT_DTLS_HASH_ALGORITHM ; clickOnRequestRow ( a ) ;",
        );
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[0]), "UUEUAUUUU");
        assert_eq!(
            results[0].origins[2],
            TokenOrigin::Evolved {
                from_file: 0,
                old_lineno: 5
            }
        );
        assert_eq!(
            subwords("URLParser_forX"),
            vec!["for", "parser", "url", "x"]
        );
        assert!(subword_similarity("startsWith", "startsWithNoEquals") >= MIN_HOLE_SIMILARITY);
        assert_eq!(
            subword_similarity("sendMouseEvent", "clickOnRequestRow"),
            0.0
        );
    }

    #[test]
    fn test_revision_renames() {
        // `copiedText` became `clipboard` in two slots, so it did in a line
        // which was otherwise rewritten too (where its slot is unequal and the
        // names aren't similar).  But a lone dissimilar rename isn't applied
        // elsewhere.
        let old = toks(
            "f",
            "use ( copiedText ) ; use ( copiedText ) ; if ( copiedText ) { go ( ) ; } use ( a ) ; check ( a , b ) ;",
        );
        let new = toks(
            "f",
            "use ( clipboard ) ; use ( clipboard ) ; while ( ready && clipboard ) { go ( ) ; } use ( z ) ; check ( b , z , c ) ;",
        );
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        let evolved: Vec<&str> = results[0]
            .origins
            .iter()
            .enumerate()
            .filter(|(_, o)| matches!(o, TokenOrigin::Evolved { .. }))
            .map(|(i, _)| split_token_line(&new[i]).token)
            .collect();
        // (`if` to `while` is an isolated keyword's evolution, and `a` to `z` a
        // slot's.)
        assert_eq!(
            evolved,
            vec!["clipboard", "clipboard", "while", "clipboard", "z"]
        );
    }

    #[test]
    fn test_revision_renames_to_the_same_name() {
        // `alpha` and `beta` both became `item` (twice each), and a rewritten
        // line has both but only one `item`, which consistently goes to the
        // first rename by name, not by the hashing of the run.
        let old = toks(
            "f",
            "use ( alpha ) ; use ( alpha ) ; take ( beta ) ; take ( beta ) ; if ( alpha || beta ) { }",
        );
        let new = toks(
            "f",
            "use ( item ) ; use ( item ) ; take ( item ) ; take ( item ) ; while ( item ) { }",
        );
        let inputs = vec![input(FileChangeKind::Modified, &old, &new)];
        let last_item = new
            .iter()
            .rposition(|l| split_token_line(l).token == "item")
            .unwrap();
        let last_alpha = old
            .iter()
            .rposition(|l| split_token_line(l).token == "alpha")
            .unwrap();
        for _ in 0..8 {
            let results = infer_revision(&inputs, &InferenceConfig::default());
            assert_eq!(
                results[0].origins[last_item],
                TokenOrigin::Evolved {
                    from_file: 0,
                    old_lineno: last_alpha as u32 + 1
                }
            );
        }
    }

    fn words(words: &str) -> Vec<String> {
        words.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn test_rename_rules() {
        let (rules, shared) = rename_rules(
            &subword_seq("RemoteWorkerSuspendOp"),
            &subword_seq("SharedWorkerSuspendOpArgs"),
        );
        assert!(shared);
        assert_eq!(
            rules,
            vec![
                RenameRule::Replace(words("remote"), words("shared")),
                RenameRule::Append(words("args")),
            ]
        );
        // (Meeting in the middle: the old name with one rule, the new name
        // without the other.)
        assert_eq!(
            rules[0].apply(&subword_seq("RemoteWorkerThawOp")),
            Some(words("shared worker thaw op"))
        );
        assert_eq!(
            rules[1].unapply(&subword_seq("SharedWorkerThawOpArgs")),
            Some(words("shared worker thaw op"))
        );
        assert_eq!(rules[1].unapply(&subword_seq("Args")), None);
        let (rules, _) = rename_rules(&subword_seq("mFooBar"), &subword_seq("mBar"));
        assert_eq!(rules, vec![RenameRule::Drop(words("foo"))]);
        let (rules, shared) = rename_rules(&subword_seq("queryString"), &subword_seq("isClass"));
        assert!(!shared);
        assert_eq!(
            rules,
            vec![RenameRule::Replace(
                words("query string"),
                words("is class")
            )]
        );
    }

    fn input_in<'a>(
        namespace: &'a str,
        kind: FileChangeKind,
        old: &'a [String],
        new: &'a [String],
    ) -> FileChangeInput<'a> {
        FileChangeInput {
            namespace,
            ..input(kind, old, new)
        }
    }

    #[test]
    fn test_rename_patterns() {
        // (D198005, the PRemoteWorker refactoring, in miniature.)  The C++
        // renames `RemoteWorkerSuspendOp` and `RemoteWorkerResumeOp` in slots,
        // so "Remote" becomes "Shared" and "Args" is appended in its other
        // names too: `RemoteFreezeOp` in a rewritten line (whose names aren't
        // similar enough for its hole; see `hole_pairs`), and the
        // structs which moved from PRemoteWorker.ipdl to a new
        // SharedWorkerOpArgs.ipdlh, with their `struct` and `{ } ;`, though
        // `RemoteWorkerThawOp` wasn't renamed anywhere else.
        let cpp_old = toks(
            "f",
            "send ( RemoteWorkerSuspendOp ( ) ) ; send ( RemoteWorkerResumeOp ( ) ) ; if ( ready ) { queue ( RemoteFreezeOp ) ; }",
        );
        let cpp_new = toks(
            "f",
            "send ( SharedWorkerSuspendOpArgs ( ) ) ; send ( SharedWorkerResumeOpArgs ( ) ) ; while ( a && b ) { dispatch ( c , d , SharedFreezeOpArgs , e ) ; }",
        );
        let ipdl_old = toks(
            "PRemoteWorker",
            "struct RemoteWorkerSuspendOp { } ; struct RemoteWorkerThawOp { } ; protocol PRemoteWorker { }",
        );
        let ipdl_new = toks("PRemoteWorker", "protocol PRemoteWorker { }");
        let ipdlh_new = toks(
            "SharedWorkerOpArgs",
            "struct SharedWorkerSuspendOpArgs { } ; struct SharedWorkerThawOpArgs { } ;",
        );
        let empty: Vec<String> = vec![];
        let inputs = vec![
            input_in("cpp", FileChangeKind::Modified, &cpp_old, &cpp_new),
            input_in("ipdl", FileChangeKind::Modified, &ipdl_old, &ipdl_new),
            input_in("ipdl", FileChangeKind::Added, &empty, &ipdlh_new),
        ];
        let without = InferenceConfig {
            rename_patterns: false,
            ..InferenceConfig::default()
        };
        let before = infer_revision(&inputs, &without);
        let results = infer_revision(&inputs, &InferenceConfig::default());
        let freeze = cpp_new
            .iter()
            .position(|l| split_token_line(l).token == "SharedFreezeOpArgs")
            .unwrap();
        assert_eq!(before[0].origins[freeze], TokenOrigin::Added);
        assert_eq!(
            results[0].origins[freeze],
            TokenOrigin::Evolved {
                from_file: 0,
                old_lineno: 22
            }
        );
        assert_eq!(summarize(&before[2]), "AAAAAAAAAA");
        assert_eq!(summarize(&results[2]), "MEMMMMEMMM");
        // (From the old file's structs.)
        assert_eq!(
            results[2].origins[6],
            TokenOrigin::Evolved {
                from_file: 1,
                old_lineno: 7
            }
        );
        // (A rule seen once doesn't pair other names: without the second
        // rename, nothing else does.)
        let cpp_old_once = toks("f", "send ( RemoteWorkerSuspendOp ( ) ) ;");
        let cpp_new_once = toks("f", "send ( SharedWorkerSuspendOpArgs ( ) ) ;");
        let inputs = vec![
            input_in(
                "cpp",
                FileChangeKind::Modified,
                &cpp_old_once,
                &cpp_new_once,
            ),
            input_in("ipdl", FileChangeKind::Modified, &ipdl_old, &ipdl_new),
            input_in("ipdl", FileChangeKind::Added, &empty, &ipdlh_new),
        ];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[2]), "AAAAAAAAAA");
    }

    #[test]
    fn test_handler_moves() {
        // (Bug 1895490 in miniature: a XUL button's `oncommand` became a case
        // of a listener's `switch (target.id)`, rewritten.)
        let line = |context: &str, class: TokenClass, token: &str| {
            crate::file_format::history::syntax_files::format_token_line(context, class, token)
        };
        let handler = "#allTabsMenu::#allTabsMenu-searchTabs@oncommand";
        let markup_old = vec![
            line("#allTabsMenu", TokenClass::Operator, "<toolbarbutton"),
            line("#allTabsMenu", TokenClass::Operator, "oncommand="),
            line(handler, TokenClass::Identifier, "gTabsPanel"),
            line(handler, TokenClass::Operator, "."),
            line(handler, TokenClass::Identifier, "searchTabs"),
            line(handler, TokenClass::Operator, "("),
            line(handler, TokenClass::Operator, ")"),
            line(handler, TokenClass::Operator, ";"),
            line("#allTabsMenu", TokenClass::Operator, "/>"),
        ];
        let markup_new = vec![
            line("#allTabsMenu", TokenClass::Operator, "<toolbarbutton"),
            line("#allTabsMenu", TokenClass::Operator, "/>"),
        ];
        // (`searchTabs` is common in the files, so big rocks don't find the
        // handler's code; see `InferenceConfig::cross_file_min_rarity`.)
        let js_old = toks("init", "searchTabs ( ) ; searchTabs ( ) ; searchTabs ( ) ;");
        let js_new = toks(
            "init",
            "searchTabs ( ) ; searchTabs ( ) ; searchTabs ( ) ; switch ( target . id ) { case \"allTabsMenu-searchTabs\" : this . searchTabs ( ) ; break ; }",
        );
        let mut markup = input_in("web", FileChangeKind::Modified, &markup_old, &markup_new);
        let js = input_in("web", FileChangeKind::Modified, &js_old, &js_new);
        let searchtabs = js_new
            .iter()
            .rposition(|l| split_token_line(l).token == "searchTabs")
            .unwrap();
        let this = js_new
            .iter()
            .position(|l| split_token_line(l).token == "this")
            .unwrap();
        // (Without the attribute, the handler's code is gone.)
        let results = infer_revision(&[markup.clone(), js.clone()], &InferenceConfig::default());
        assert_eq!(results[1].origins[searchtabs], TokenOrigin::Added);
        assert_eq!(results[1].origins[this], TokenOrigin::Added);
        markup.handler_moves = true;
        let results = infer_revision(&[markup, js], &InferenceConfig::default());
        assert_eq!(
            results[1].origins[searchtabs],
            TokenOrigin::Moved {
                from_file: 0,
                old_lineno: 5
            }
        );
        // (`gTabsPanel` became `this`.)
        assert_eq!(
            results[1].origins[this],
            TokenOrigin::Evolved {
                from_file: 0,
                old_lineno: 3
            }
        );
        assert_eq!(
            handler_of(handler),
            Some(("allTabsMenu-searchTabs", "oncommand", handler))
        );
        assert_eq!(handler_of("#a::@onclick"), None);
        assert_eq!(handler_of("#a::#b"), None);
    }

    #[test]
    fn test_cross_file_moves_need_distinctive_matches() {
        // A test loses an `enabled : true ,` (it has other `true`s), and
        // another file gains a `shown : true ,`, which isn't a move from the
        // test (that would blame it on the test's history).  But an
        // `#include` whose path is only where it moved from and to is one.
        let a_old = toks(
            "test",
            "check ( { enabled : true , other : true } ) ; ok ( true ) ; #include \"mozilla/Foo.h\"",
        );
        let a_new = toks("test", "check ( { other : true } ) ; ok ( true ) ;");
        let b_old = toks("State", "let state = { open : false } ;");
        let b_new = toks(
            "State",
            "let state = { open : false , shown : true , } ; #include \"mozilla/Foo.h\"",
        );
        let inputs = vec![
            input(FileChangeKind::Modified, &a_old, &a_new),
            input(FileChangeKind::Modified, &b_old, &b_new),
        ];
        let results = infer_revision(&inputs, &InferenceConfig::default());
        assert_eq!(summarize(&results[1]), "UUUUUUUAAAAAUUMM");
        // (Without the requirement, the `: true ,` was a move.)
        let config = InferenceConfig {
            cross_file_min_len: 0,
            ..InferenceConfig::default()
        };
        let results = infer_revision(&inputs, &config);
        assert_eq!(summarize(&results[1]), "UUUUUUUAAMMMUUMM");
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
