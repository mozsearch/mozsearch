//! Derive token and symbol level statistics from the results of
//! `inference::infer_revision`.  These are the basis of the
//! "history/timeline/files-delta" and "history/timeline/tokens" journals and
//! the rev-summaries.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::file_format::history::syntax_files::TokenLine;
use crate::file_format::history::timeline_common::{
    ChangeKind, SymbolSyntaxDelta, SymbolSyntaxDeltaGroup, TokenDeltaDetails,
};
use crate::file_format::history::timeline_tokens::tracked_token_key;

use super::inference::{FileChangeInput, FileInference, RemovedFate, TokenOrigin};
use crate::file_format::history::syntax_files::split_token_line;

/// The context used by the tokenizer when a token is not inside any structure.
pub const NO_CONTEXT: &str = "%";

pub struct RevisionStats {
    /// Parallel to the inference inputs.
    pub file_groups: Vec<SymbolSyntaxDeltaGroup>,
    /// Totals for trackable tokens across all files in the revision.
    pub token_totals: BTreeMap<String, TokenDeltaDetails>,
}

#[derive(Clone, Copy)]
enum Delta {
    Added,
    Moved,
    EvolvedFrom,
    EvolvedInto,
    Removed,
}

fn bump(details: &mut TokenDeltaDetails, delta: Delta) {
    match delta {
        Delta::Added => details.added += 1,
        Delta::Moved => details.moved += 1,
        Delta::EvolvedFrom => details.evolved_from += 1,
        Delta::EvolvedInto => details.evolved_into += 1,
        Delta::Removed => details.removed += 1,
    }
}

/// The token preceding the given 0-based index, for bug number detection.
fn prev_line<'a>(lines: &[&'a str], idx: usize) -> Option<TokenLine<'a>> {
    idx.checked_sub(1).map(|i| split_token_line(lines[i]))
}

/// Compute per-file symbol deltas and per-token totals.
///
/// `old_symbols` and `new_symbols` are parallel to `inputs` and provide the set
/// of "pretty" symbol identifiers from the "files-struct" records for the old
/// and new versions of each file.  These let us tell whether a symbol was added
/// or removed versus just changed.
pub fn compute_revision_stats(
    inputs: &[FileChangeInput],
    inferences: &[FileInference],
    old_symbols: &[BTreeSet<String>],
    new_symbols: &[BTreeSet<String>],
) -> RevisionStats {
    let mut token_totals: BTreeMap<String, TokenDeltaDetails> = BTreeMap::new();
    let mut groups: Vec<BTreeMap<String, SymbolSyntaxDelta>> = vec![BTreeMap::new(); inputs.len()];
    // For each (file, new context), the number of moved/evolved tokens that came
    // from each (source file, old context).
    let mut correspondences: Vec<HashMap<String, HashMap<(u32, String), u32>>> =
        vec![HashMap::new(); inputs.len()];
    // For each (file, new context), the number of tokens in that context.
    let mut new_context_sizes: Vec<HashMap<String, u32>> = vec![HashMap::new(); inputs.len()];

    let mut record = |groups: &mut Vec<BTreeMap<String, SymbolSyntaxDelta>>,
                      file: usize,
                      namespace: &str,
                      line: &TokenLine,
                      prev: Option<&TokenLine>,
                      delta: Delta| {
        let sym_delta = groups[file]
            .entry(line.context.to_string())
            .or_insert_with(|| SymbolSyntaxDelta::new(ChangeKind::Changed));
        bump(&mut sym_delta.token_totals, delta);
        if let Some(key) = tracked_token_key(namespace, line, prev) {
            bump(
                sym_delta.token_changes.entry(key.to_string()).or_default(),
                delta,
            );
            bump(token_totals.entry(key.into_owned()).or_default(), delta);
        }
    };

    for (file, (input, inference)) in inputs.iter().zip(inferences.iter()).enumerate() {
        // ## New side
        for (new_idx, origin) in inference.origins.iter().enumerate() {
            let line = split_token_line(input.new_lines[new_idx]);
            *new_context_sizes[file]
                .entry(line.context.to_string())
                .or_default() += 1;
            let (delta, source) = match *origin {
                TokenOrigin::Unchanged { .. } => continue,
                TokenOrigin::Added => (Delta::Added, None),
                TokenOrigin::Moved {
                    from_file,
                    old_lineno,
                } => (Delta::Moved, Some((from_file, old_lineno))),
                TokenOrigin::Evolved {
                    from_file,
                    old_lineno,
                } => (Delta::EvolvedFrom, Some((from_file, old_lineno))),
            };
            let prev = prev_line(&input.new_lines, new_idx);
            record(
                &mut groups,
                file,
                input.namespace,
                &line,
                prev.as_ref(),
                delta,
            );
            if let Some((from_file, old_lineno)) = source {
                let old_line =
                    split_token_line(inputs[from_file as usize].old_lines[old_lineno as usize - 1]);
                *correspondences[file]
                    .entry(line.context.to_string())
                    .or_default()
                    .entry((from_file, old_line.context.to_string()))
                    .or_default() += 1;
            }
        }

        // ## Old side
        for removed in &inference.removed {
            let old_idx = removed.old_lineno as usize - 1;
            let line = split_token_line(input.old_lines[old_idx]);
            let delta = match removed.fate {
                RemovedFate::Extinguished => Delta::Removed,
                RemovedFate::EvolvedInto { .. } => Delta::EvolvedInto,
                // Moves are counted on the new side.
                RemovedFate::MovedTo { .. } => continue,
            };
            let prev = prev_line(&input.old_lines, old_idx);
            record(
                &mut groups,
                file,
                input.namespace,
                &line,
                prev.as_ref(),
                delta,
            );
        }
    }

    // ## Determine symbol change kinds
    for (file, group) in groups.iter_mut().enumerate() {
        let (old, new) = (&old_symbols[file], &new_symbols[file]);
        for pretty in old.symmetric_difference(new) {
            group
                .entry(pretty.clone())
                .or_insert_with(|| SymbolSyntaxDelta::new(ChangeKind::Changed));
        }
        for (pretty, sym_delta) in group.iter_mut() {
            sym_delta.change = if pretty == NO_CONTEXT {
                ChangeKind::Changed
            } else if new.contains(pretty) && !old.contains(pretty) {
                ChangeKind::Added
            } else if old.contains(pretty) && !new.contains(pretty) {
                ChangeKind::Removed
            } else {
                ChangeKind::Changed
            };
        }
    }

    // ## Detect evolved (renamed) symbols
    //
    // An added symbol evolved from a removed symbol if the majority of its
    // tokens were moved/evolved from that symbol.  We process candidates with
    // the most corresponding tokens first so that each removed symbol is only
    // claimed once.
    let mut candidates: Vec<(u32, usize, String, u32, String)> = vec![];
    for (file, group) in groups.iter().enumerate() {
        for (pretty, sym_delta) in group {
            if sym_delta.change != ChangeKind::Added {
                continue;
            }
            let Some(sources) = correspondences[file].get(pretty) else {
                continue;
            };
            let size = new_context_sizes[file].get(pretty).copied().unwrap_or(0);
            for ((from_file, old_pretty), count) in sources {
                let from = *from_file as usize;
                let removed_from_source = old_symbols[from].contains(old_pretty)
                    && !new_symbols[from].contains(old_pretty);
                if old_pretty != pretty && removed_from_source && *count * 2 >= size {
                    candidates.push((*count, file, pretty.clone(), *from_file, old_pretty.clone()));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.cmp(a));
    let mut claimed_old: BTreeSet<(u32, String)> = BTreeSet::new();
    let mut claimed_new: BTreeSet<(usize, String)> = BTreeSet::new();
    for (_, file, pretty, from_file, old_pretty) in candidates {
        if claimed_new.contains(&(file, pretty.clone()))
            || claimed_old.contains(&(from_file, old_pretty.clone()))
        {
            continue;
        }
        claimed_new.insert((file, pretty.clone()));
        claimed_old.insert((from_file, old_pretty.clone()));
        if let Some(sym_delta) = groups[file].get_mut(&pretty) {
            sym_delta.change = ChangeKind::Evolved;
            sym_delta.evolved_from = Some(old_pretty.clone());
        }
        if let Some(sym_delta) = groups[from_file as usize].get_mut(&old_pretty) {
            sym_delta.evolved_into = Some(pretty);
        }
    }

    RevisionStats {
        file_groups: groups
            .into_iter()
            .map(|symbol_deltas| SymbolSyntaxDeltaGroup { symbol_deltas })
            .collect(),
        token_totals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyperblame::inference::{FileChangeKind, InferenceConfig, infer_revision};

    fn toks(context: &str, tokens: &str) -> Vec<String> {
        tokens
            .split_whitespace()
            .map(|t| format!("{} {}", context, t))
            .collect()
    }

    fn syms(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_tracking_filter() {
        use crate::file_format::history::syntax_files::{TokenClass, format_token_line};
        let line = |class, token| format_token_line("Foo::bar", class, token);
        let old = vec![
            line(TokenClass::Keyword, "return"),
            line(TokenClass::Operator, ";"),
        ];
        let new = vec![
            line(TokenClass::Keyword, "if"),
            line(TokenClass::Identifier, "this"),
            line(TokenClass::Identifier, "mReady"),
            line(TokenClass::Comment, "//"),
            line(TokenClass::Comment, "See"),
            line(TokenClass::Comment, "Bug"),
            line(TokenClass::Comment, "1620052."),
            line(TokenClass::Keyword, "return"),
            line(TokenClass::Operator, ";"),
        ];
        let inputs = vec![FileChangeInput {
            kind: FileChangeKind::Modified,
            namespace: "cpp",
            old_lines: old.iter().map(|s| s.as_str()).collect(),
            new_lines: new.iter().map(|s| s.as_str()).collect(),
        }];
        let inferences = infer_revision(&inputs, &InferenceConfig::default());
        let stats = compute_revision_stats(
            &inputs,
            &inferences,
            &[syms(&["Foo::bar"])],
            &[syms(&["Foo::bar"])],
        );
        let tracked: Vec<&str> = stats.token_totals.keys().map(|k| k.as_str()).collect();
        assert_eq!(tracked, vec!["bug-1620052", "mReady"]);
        let sym = &stats.file_groups[0].symbol_deltas["Foo::bar"];
        // Everything still counts towards the totals.
        assert_eq!(sym.token_totals.added, 7);
        assert_eq!(sym.token_changes.len(), 2);
    }

    #[test]
    fn test_method_rename_is_evolution() {
        let mut old = toks("Foo", "class Foo {");
        old.extend(toks("Foo::bar", "void bar ( ) { mCount = mCount + 1 ; }"));
        old.extend(toks("Foo", "} ;"));
        let mut new = toks("Foo", "class Foo {");
        new.extend(toks("Foo::baz", "void baz ( ) { mCount = mCount + 1 ; }"));
        new.extend(toks("Foo", "} ;"));
        let inputs = vec![FileChangeInput {
            kind: FileChangeKind::Modified,
            namespace: "cpp",
            old_lines: old.iter().map(|s| s.as_str()).collect(),
            new_lines: new.iter().map(|s| s.as_str()).collect(),
        }];
        let inferences = infer_revision(&inputs, &InferenceConfig::default());
        let stats = compute_revision_stats(
            &inputs,
            &inferences,
            &[syms(&["Foo", "Foo::bar"])],
            &[syms(&["Foo", "Foo::baz"])],
        );
        let group = &stats.file_groups[0].symbol_deltas;
        let baz = &group["Foo::baz"];
        assert_eq!(baz.change, ChangeKind::Evolved);
        assert_eq!(baz.evolved_from.as_deref(), Some("Foo::bar"));
        assert_eq!(baz.token_changes["baz"].evolved_from, 1);
        assert_eq!(baz.token_changes["mCount"].moved, 2);
        let bar = &group["Foo::bar"];
        assert_eq!(bar.change, ChangeKind::Removed);
        assert_eq!(bar.evolved_into.as_deref(), Some("Foo::baz"));
        assert_eq!(bar.token_changes["bar"].evolved_into, 1);
        assert!(!group.contains_key("Foo"));

        // Only the evolution is a non-move change for the token totals.
        assert!(stats.token_totals["baz"].has_non_move_changes());
        assert!(stats.token_totals["bar"].has_non_move_changes());
        assert!(!stats.token_totals["mCount"].has_non_move_changes());
    }
}
