//! `re:`'s fuller engine, for patterns livegrep (RE2, a line at a time) can't
//! run: lookaround, backreferences, matches across lines (a `\n`, `(?s)`'s
//! `.`), or patterns wider than it takes.  livegrep finds the files with the
//! literals every match contains (see `regex_plan`; one search per conjunct,
//! the files in all of them), and fancy-regex runs the pattern on those files'
//! text: line by line, as livegrep would, unless the pattern spans lines, when
//! on the whole text (with `^` and `$` at lines' ends).  Without a literal to
//! narrow the files, a `path:`'s files are searched, if few enough; otherwise
//! the search is refused as too expensive (see `query_limits`).
//!
//! What it may read is the request's tier's `grep_files` and `grep_bytes`;
//! past them, and past the match limit, the results say a limit was hit, as
//! livegrep's do.

use std::collections::{BTreeMap, BTreeSet};

use futures_util::StreamExt;
use ustr::Ustr;

use crate::abstract_server::{
    AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError, TextBounds, TextHit,
    TextMatchInFile, TextMatches, TextMatchesByFile,
};
use crate::query_limits::{QueryLimits, Tier, TooExpensive};
use crate::regex_plan::{RegexPlan, livegrep_alternation};

/// How many lines livegrep may give for each literal's search (its files are
/// the candidates; past this, the candidates are incomplete).
const PREFILTER_MATCHES: usize = 50_000;
/// How many of the plan's conjuncts are searched for (the most selective
/// guesses: those with the longest literals).
const MAX_CONJUNCTS: usize = 3;
/// How many backtracking steps fancy-regex may take on a file (past this, the
/// file's other matches are skipped, and a limit was hit).
const BACKTRACK_LIMIT: usize = 1_000_000;
/// How many files are read at once.
const CONCURRENT_READS: usize = 8;
/// How many matches by default (as livegrep's).
const DEFAULT_MATCHES: usize = 4000;

/// What to search for.
pub struct FullGrep<'a> {
    pub pattern: &'a str,
    pub fold_case: bool,
    pub pathre: &'a str,
    pub not_pathre: &'a str,
    /// How many matches (0 for the default).
    pub limit: usize,
}

/// Compile `pattern` for the fuller engine.
pub fn compile(pattern: &str, fold_case: bool) -> Result<fancy_regex::Regex> {
    fancy_regex::RegexBuilder::new(pattern)
        .case_insensitive(fold_case)
        .multi_line(true)
        .backtrack_limit(BACKTRACK_LIMIT)
        .build()
        .map_err(|e| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!("Bad regular expression: {}", e),
            })
        })
}

/// The capture groups' names, after the whole match's (None for unnamed
/// ones).
pub fn capture_names(re: &fancy_regex::Regex) -> Vec<Option<String>> {
    re.capture_names()
        .skip(1)
        .map(|name| name.map(str::to_string))
        .collect()
}

/// The files livegrep finds with the plan's literals, with their path kinds:
/// the files in all of the conjuncts' complete searches (or the smallest
/// incomplete one's, if none is complete), and whether that's incomplete.
async fn candidate_files(
    server: &(dyn AbstractServer + Send + Sync),
    plan: &RegexPlan,
    search: &FullGrep<'_>,
    limits: &QueryLimits,
    tier: Tier,
) -> Result<(BTreeMap<Ustr, Ustr>, bool, bool)> {
    let mut conjuncts = plan.query.conjuncts();
    conjuncts.sort_by_key(|conjunct| {
        std::cmp::Reverse(
            conjunct
                .iter()
                .filter_map(|literal| literal.longest_piece())
                .map(|piece| piece.chars().count())
                .min()
                .unwrap_or(0),
        )
    });
    let alternations: Vec<(String, bool)> = conjuncts
        .iter()
        .filter_map(|conjunct| livegrep_alternation(conjunct))
        .take(MAX_CONJUNCTS)
        .collect();

    if alternations.is_empty() {
        // (Without a literal, a path's files, if it's narrow enough.)
        let mut refused = TooExpensive {
            what: "regular expression search".to_string(),
            exceeded: vec![],
            details: vec![format!(
                "{:?} has no literal text that every match contains, so every file would need to be searched",
                search.pattern
            )],
            hints: vec![
                "add literal text that every match contains (ex: a name)".to_string(),
                "or a narrow path: for the files to search".to_string(),
            ],
            tier,
            signed_in_allows: false,
        };
        if search.pathre.is_empty() {
            return Err(ServerError::TooExpensive(Box::new(refused)));
        }
        let limit = limits.grep_files.min(usize::MAX as u64 - 1) as usize;
        let files = server
            .search_files(search.pathre, search.not_pathre, false, limit + 1)
            .await?;
        if files.limit_hit || files.file_matches.len() > limit {
            refused.exceeded.push(crate::query_limits::Exceeded {
                limit: "grep_files".to_string(),
                estimate: files.file_matches.len() as u64,
                allowed: limits.grep_files,
            });
            return Err(ServerError::TooExpensive(Box::new(refused)));
        }
        let candidates = files
            .file_matches
            .into_iter()
            .map(|file| (file.path, file.concise.path_kind))
            .collect();
        return Ok((candidates, false, false));
    }

    let searches =
        futures_util::future::join_all(alternations.iter().map(|(alternation, fold_case)| {
            server.search_text(
                alternation,
                *fold_case,
                search.pathre,
                search.not_pathre,
                PREFILTER_MATCHES,
            )
        }))
        .await;
    let mut complete: Option<BTreeMap<Ustr, Ustr>> = None;
    let mut smallest_incomplete: Option<BTreeMap<Ustr, Ustr>> = None;
    let mut timed_out = false;
    for search in searches {
        let matches = search?;
        timed_out |= matches.timed_out;
        let files: BTreeMap<Ustr, Ustr> = matches
            .by_file
            .into_iter()
            .map(|file| (file.file, file.path_kind))
            .collect();
        if matches.limit_hit || matches.timed_out {
            if smallest_incomplete
                .as_ref()
                .is_none_or(|smallest| files.len() < smallest.len())
            {
                smallest_incomplete = Some(files);
            }
        } else {
            complete = Some(match complete {
                Some(mut so_far) => {
                    so_far.retain(|path, _| files.contains_key(path));
                    so_far
                }
                None => files,
            });
        }
    }
    Ok(match (complete, smallest_incomplete) {
        (Some(files), _) => (files, false, timed_out),
        (None, Some(files)) => (files, true, timed_out),
        (None, None) => (BTreeMap::new(), false, timed_out),
    })
}

/// The matches of `re` in `text`, by the line they start on (line by line,
/// unless `spans_lines`), up to `limit`; and whether the backtracking limit
/// stopped it.
pub fn file_matches(
    re: &fancy_regex::Regex,
    text: &str,
    spans_lines: bool,
    limit: usize,
) -> (Vec<TextMatchInFile>, bool) {
    // (Each line's start, for byte offsets' lines.)
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_text = |line: usize| -> &str {
        let start = line_starts[line];
        let end = line_starts
            .get(line + 1)
            .map_or(text.len(), |next| next - 1);
        text[start..end].trim_end_matches('\r')
    };
    let position = |offset: usize| -> (usize, usize) {
        let line = line_starts.partition_point(|&start| start <= offset) - 1;
        (line, offset - line_starts[line])
    };

    let mut by_line: BTreeMap<usize, Vec<TextHit>> = BTreeMap::new();
    let mut count = 0;
    let mut backtracked_out = false;
    let mut add = |captures: &fancy_regex::Captures<str>, base: usize| -> bool {
        let Some(found) = captures.get(0) else {
            return true;
        };
        if found.start() == found.end() {
            return true;
        }
        let (start_line, start) = position(base + found.start());
        let (end_line, end) = position(base + found.end());
        // (A match ending at a line's start ends at the previous line's end.)
        let (end_line, end) = if end == 0 && end_line > start_line {
            (end_line - 1, line_text(end_line - 1).len())
        } else {
            (end_line, end)
        };
        by_line.entry(start_line).or_default().push(TextHit {
            start_line: start_line as u32 + 1,
            start: start as u32,
            end_line: end_line as u32 + 1,
            end: end as u32,
            captures: (1..captures.len())
                .map(|i| captures.get(i).map(|m| m.as_str().to_string()))
                .collect(),
        });
        count += 1;
        count < limit
    };
    if spans_lines {
        for captures in re.captures_iter(text) {
            match captures {
                Ok(captures) => {
                    if !add(&captures, 0) {
                        break;
                    }
                }
                Err(_) => {
                    backtracked_out = true;
                    break;
                }
            }
        }
    } else {
        'lines: for line in 0..line_starts.len() {
            let current = line_text(line);
            for captures in re.captures_iter(current) {
                match captures {
                    Ok(captures) => {
                        if !add(&captures, line_starts[line]) {
                            break 'lines;
                        }
                    }
                    Err(_) => {
                        backtracked_out = true;
                        break 'lines;
                    }
                }
            }
        }
    }

    let matches = by_line
        .into_iter()
        .map(|(line, hits)| {
            let line_str = line_text(line).to_string();
            let first = &hits[0];
            let end = if first.end_line == first.start_line {
                first.end
            } else {
                line_str.len() as u32
            };
            // (Lines are 1-based in the hits.)
            let last = hits
                .iter()
                .map(|hit| hit.end_line as usize - 1)
                .max()
                .unwrap_or(line);
            TextMatchInFile {
                line_num: line as u32 + 1,
                bounds: TextBounds {
                    start: first.start as i32,
                    end_exclusive: end as i32,
                },
                line_str,
                hits,
                more_lines: (line + 1..=last)
                    .map(|l| line_text(l).to_string())
                    .collect(),
            }
        })
        .collect();
    (matches, backtracked_out)
}

/// Search for `search`'s pattern, planned as `plan`, within `limits`.
pub async fn full_grep(
    server: &(dyn AbstractServer + Send + Sync),
    search: &FullGrep<'_>,
    plan: &RegexPlan,
    limits: &QueryLimits,
    tier: Tier,
) -> Result<TextMatches> {
    let re = compile(search.pattern, search.fold_case)?;
    let (candidates, mut limit_hit, timed_out) =
        candidate_files(server, plan, search, limits, tier).await?;

    let mut paths: Vec<(Ustr, Ustr)> = candidates.into_iter().collect();
    if paths.len() as u64 > limits.grep_files {
        paths.truncate(limits.grep_files as usize);
        limit_hit = true;
    }
    let limit = if search.limit == 0 {
        DEFAULT_MATCHES
    } else {
        search.limit
    };

    let reading = paths.into_iter().map(|(path, path_kind)| async move {
        let text = server.fetch_raw_source(&path).await;
        (path, path_kind, text)
    });
    let mut reads = futures_util::stream::iter(reading).buffered(CONCURRENT_READS);
    let mut by_file = vec![];
    let mut bytes: u64 = 0;
    let mut count = 0;
    let mut seen = BTreeSet::new();
    while let Some((path, path_kind, text)) = reads.next().await {
        // (A file that's gone, or isn't text, has no matches.)
        let Ok(text) = text else {
            continue;
        };
        bytes += text.len() as u64;
        if bytes > limits.grep_bytes {
            limit_hit = true;
            break;
        }
        let (matches, backtracked_out) = file_matches(&re, &text, plan.spans_lines, limit - count);
        limit_hit |= backtracked_out;
        if matches.is_empty() || !seen.insert(path) {
            continue;
        }
        count += matches.iter().map(|m| m.hits.len()).sum::<usize>();
        by_file.push(TextMatchesByFile {
            file: path,
            path_kind,
            matches,
        });
        if count >= limit {
            limit_hit = true;
            break;
        }
    }
    Ok(TextMatches {
        by_file,
        limit_hit,
        timed_out,
        pattern: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_matches() {
        let text = "fn a() {\n    foo(1);\n    bar(foo(2));\n}\nfoo(\n  3);\n";
        let re = compile(r"foo\((\d)\)", false).unwrap();
        let (matches, _) = file_matches(&re, text, false, 100);
        assert_eq!(
            matches.iter().map(|m| m.line_num).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(matches[1].hits[0].start, 8);
        assert_eq!(matches[1].hits[0].captures, vec![Some("2".to_string())]);
        // (Across lines: the match starting on line 5 ends on line 6.)
        let re = compile(r"foo\(\n\s*(\d)\)", false).unwrap();
        let (matches, _) = file_matches(&re, text, true, 100);
        assert_eq!(matches.len(), 1);
        let hit = &matches[0].hits[0];
        assert_eq!(
            (hit.start_line, hit.start, hit.end_line, hit.end),
            (5, 0, 6, 4)
        );
        assert_eq!(hit.captures, vec![Some("3".to_string())]);
        assert_eq!(matches[0].line_str, "foo(");
        // (`^` is at lines' starts, and lookaround works.)
        let re = compile(r"^\s+(?=bar)\w+", false).unwrap();
        let (matches, _) = file_matches(&re, text, true, 100);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_num, 3);
        // (Backreferences, and the limit.)
        let re = compile(r"(\w)\1", false).unwrap();
        let (matches, _) = file_matches(&re, "aa bb cc\ndd", false, 2);
        assert_eq!(matches.iter().map(|m| m.hits.len()).sum::<usize>(), 2);
        assert_eq!(
            capture_names(&compile(r"(?<name>a)(b)", false).unwrap()),
            vec![Some("name".to_string()), None]
        );
    }
}
