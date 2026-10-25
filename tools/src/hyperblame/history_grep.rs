//! History greps (`text-history:`, `re-history:`; see `cmd_history_grep`): the
//! revisions where the number of a pattern's matches in a file changed, as
//! `git log -S --pickaxe-regex` finds them per file, or, for a pattern with
//! capture groups, where the values its matches captured there changed.
//!
//! Rather than checking every revision, the token journals say which ones
//! may have: every match contains the tokens `regex_plan` finds, whole, and a
//! revision adding or removing a match adds or removes one of them there,
//! which the token's journal records (and the revision's summary, by file).
//! The planner picks the cheapest of them by their journals' sizes (an
//! `And`'s rarest, all of an `Or`'s), and each candidate revision's files
//! where those tokens changed are checked by matching the pattern in the
//! file's versions before and after the revision.  With `path:`, the files'
//! every revision can be checked instead, from their `files-delta`
//! journals, which also sees what the token journals don't (see below), and
//! is how a pattern without a token to narrow it by is searched.
//!
//! What the token journals can't see (and `git log -S` would):
//! - merges (skipped, as `git log -S` does without `-m`);
//! - code moved between files (moves aren't the journals' changes);
//! - matches in what the journals don't track: prose in strings, URLs and
//!   paths;
//! - a match appearing without one of its tokens being added or removed in
//!   that file (ex: existing tokens rearranged around it, or a number changed
//!   next to it).
//!
//! The plan's costs (journal bytes, candidate revisions) are checked against
//! the request's limits before anything expensive is read, and refused if
//! they're over them (see `query_limits`).  A page checks up to the limits'
//! (revision, file) pairs and bytes of their versions, newest revisions
//! first, and says where the next one starts.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use chrono::{NaiveDate, Weekday};
use git2::{Oid, Repository, Tree};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::abstract_server::{ErrorDetails, ErrorLayer, ServerError};
use crate::cmd_pipeline::full_grep::{capture_names, compile, file_matches};
use crate::commit_index::CommitPosition;
use crate::file_format::history::rev_summaries::RevSummaryRecord;
use crate::file_format::history::timeline_common::JournalVersionRef;
use crate::file_format::history::timeline_files_delta::FileDeltaRecord;
use crate::file_format::history::timeline_tokens::{TokenDeltaRecord, token_timeline_path};
use crate::file_format::token_names::NameMatch;
use crate::hyperblame::journals::JournalReader;
use crate::hyperblame::token_blame::TreeHistory;
use crate::hyperblame::token_history::{
    case_variants, header_html, read_journal, read_rev_summary,
};
use crate::query_limits::{QueryLimits, Tier, TooExpensive};
use crate::regex_plan::{self, RegexPlan, TokenTerm};

/// How many token names a piece of a token (a literal's cut-off end) may
/// stand for; more, and it isn't used.
const MAX_EXPANSION: usize = 64;
/// How many files a `path:` may name for their every revision to be checked.
pub const MAX_PATH_FILES: usize = 50;
/// How many lines and values each row shows of those added and removed.
const MAX_SHOWN: usize = 5;

/// What to look for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryGrepArgs {
    /// The regular expression (a `text-history:`'s text, escaped).
    pub pattern: String,
    /// A `text-history:`'s text, as given.
    pub text: Option<String>,
    pub case_sensitive: bool,
    pub pathre: String,
    pub not_pathre: String,
    /// The files `path:` names (see `MAX_PATH_FILES`), whose every revision
    /// can be checked, if it names few enough.
    pub files: Vec<String>,
    /// Only revisions from `since` until `until` (YYYY-MM-DD).
    pub since: Option<String>,
    pub until: Option<String>,
    /// Where the page starts in the candidate revisions (newest first).
    pub page: usize,
    /// Prefer checking `files`' every revision when there are any (which sees
    /// value changes; see the module's comment) to the token journals.
    pub prefer_files: bool,
    /// Chart the matches over time (`text-timeline:`, `re-timeline:`; see
    /// `Timeline`): all of the candidates at once, rather than a page.
    pub timeline: bool,
    /// Only matches in this scope (a class's, a function's, ... pretty
    /// identifier, as the history's contexts have them) of `files`: the
    /// revisions changing it, and the pattern on its lines (see
    /// `TreeHistory::container_lines`).
    pub scope: Option<String>,
    /// Show the revisions which were backed out, and their backouts, rather
    /// than leaving out those whose backout undid them (see
    /// `hide_backout_pairs`).
    pub show_backouts: bool,
    /// What the query had that a history grep doesn't take (ex: a cut-off
    /// `until:`), which its page says it ignored.
    pub ignored: Option<String>,
}

/// What it found.  (Every field is serialized, for the template.)
#[derive(Clone, Debug, Default, Serialize)]
pub struct HistoryGrep {
    pub pattern: String,
    pub text: Option<String>,
    pub case_sensitive: bool,
    pub capture_names: Vec<Option<String>>,
    /// The tokens whose journals gave the candidate revisions.
    pub tokens: Vec<String>,
    /// Or the files whose every revision was a candidate.
    pub files: Vec<String>,
    /// How many candidate revisions there are.
    pub candidates: usize,
    /// The candidates this page checked, [page_start, page_end).
    pub page_start: usize,
    pub page_end: usize,
    /// Where the next page starts, if there are more candidates.
    pub next_page: Option<usize>,
    /// How many (revision, file) pairs it checked, and bytes of their
    /// versions it read.
    pub checked: usize,
    pub bytes: u64,
    pub rows: Vec<HistoryGrepRow>,
    /// Merges it skipped, and candidates without summaries.
    pub merges_skipped: usize,
    pub unsummarized: usize,
    /// Whether the tree has no history (see `cmd_history_grep`).
    pub unsupported: bool,
    /// The matches over time, for `text-timeline:` and `re-timeline:`.
    pub timeline: Option<Timeline>,
    /// The scope it was limited to, if any, how many of its versions' lines
    /// it found (a parse each), and whether its revisions were narrowed to
    /// those changing the pattern's names in it (when they were too many;
    /// see `history_grep`).
    pub scope: Option<String>,
    pub parses: usize,
    pub pruned: bool,
    /// How many backed out revisions' changes, with their backouts', it left
    /// out (see `hide_backout_pairs`).
    pub backouts_hidden: usize,
    /// What the query had that it ignored (see `HistoryGrepArgs::ignored`).
    pub ignored: Option<String>,
    /// Whether the page stopped at its time limit (see `QueryLimits::
    /// history_page_seconds`).
    pub out_of_time: bool,
    /// Whether the candidates are ordered by when they landed, parents first
    /// (see `tools::commit_index`), rather than in the journals' order (see
    /// `merge_sequences`), for a tree whose commit index doesn't have them.
    pub landing_order: bool,
}

/// A history grep's matches over time (`text-timeline:`, `re-timeline:`): a
/// line of their count over the files; or, for a pattern with capture groups,
/// of the value each file's match captured, if they're numbers and each file
/// has one; or else of how many distinct values the matches captured (ex: a
/// switch's `case`s), with what each change added and removed (sets, so that
/// reorderings aren't changes).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Timeline {
    /// "matches", "value" or "values" (see above).
    pub measure: String,
    pub series: Vec<TimelineSeries>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TimelineSeries {
    /// The file whose values it is, or "" for all of them.
    pub label: String,
    /// Its value before its first change, and after each change, oldest
    /// first.
    pub start: f64,
    pub points: Vec<TimelinePoint>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TimelinePoint {
    pub rev: String,
    /// See `HistoryGrepRow::date`.
    pub date: String,
    pub committed: Option<String>,
    /// The commit message's first line, as HTML.
    pub header: String,
    pub value: f64,
    /// What it added and removed: values (a value's groups joined by ", "),
    /// or for counts, lines.
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// How many timeline lines of files' values at most (more, and it's the
/// number of distinct values).
const MAX_VALUE_SERIES: usize = 8;

/// A revision's change to a file's matches.
#[derive(Clone, Debug, Default, Serialize)]
pub struct HistoryGrepRow {
    pub rev: String,
    /// When it landed (YYYY-MM-DD), at the earliest (see `tools::
    /// commit_index`), or, without the commit index, its commit date.
    pub date: String,
    /// Its commit date, if it was another day (ex: a patch landed weeks after
    /// it was written, or relanded after a backout, in hg's history).
    pub committed: Option<String>,
    /// The commit message's first line, as HTML (see `header_html`).
    pub header: String,
    /// Whether it backs out other revisions.
    pub backout: bool,
    pub path: String,
    /// The file's path before the revision, if it moved it.
    pub old_path: Option<String>,
    /// How many matches the file had before and after.
    pub before: usize,
    pub after: usize,
    /// The matching lines it added and removed (the first few).
    pub added_lines: Vec<String>,
    pub removed_lines: Vec<String>,
    /// For a pattern with capture groups, the values it added and removed
    /// (the first few).
    pub added_values: Vec<Vec<Option<String>>>,
    pub removed_values: Vec<Vec<Option<String>>>,
}

fn bad_input(message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::BadInput,
        message,
    })
}

fn data_problem(message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::DataLayer,
        message,
    })
}

/// The size of a token's journal, with its segments, from their objects'
/// headers (without reading them), if it has one.
fn journal_bytes(repo: &Repository, tree: &Tree, token: &str) -> Option<u64> {
    path_journal_bytes(repo, tree, &token_timeline_path(token))
}

/// The size of the journal at `path`, with its segments (see
/// `journal_bytes`).
fn path_journal_bytes(repo: &Repository, tree: &Tree, path: &Path) -> Option<u64> {
    let path = path.to_path_buf();
    let entry = tree.get_path(&path).ok()?;
    let odb = repo.odb().ok()?;
    let mut bytes = odb.read_header(entry.id()).ok()?.0 as u64;
    let mut segments = path.into_os_string();
    segments.push(".d");
    if let Ok(dir) = tree.get_path(Path::new(&segments))
        && let Ok(dir) = repo.find_tree(dir.id())
    {
        for entry in dir.iter() {
            bytes += odb
                .read_header(entry.id())
                .map_or(0, |(size, _)| size as u64);
        }
    }
    Some(bytes)
}

/// The tokens with journals a term stands for (its case variants, if
/// `fold_case`; the names a piece is of), if it's usable: None if a piece is
/// of too many names (or there's no names index).
fn term_tokens(
    term: &TokenTerm,
    fold_case: bool,
    repo: &Repository,
    tree: &Tree,
    names: &dyn Fn(&str, NameMatch, usize) -> Option<(Vec<String>, bool)>,
) -> Option<Vec<String>> {
    let with_journal = |token: &str| tree.get_path(&token_timeline_path(token)).is_ok();
    let (how, piece) = match term {
        TokenTerm::Whole(token) => {
            return Some(if fold_case {
                case_variants(repo, tree, token)
            } else if with_journal(token) {
                vec![token.clone()]
            } else {
                vec![]
            });
        }
        TokenTerm::Prefix(piece) => (NameMatch::Prefix, piece),
        TokenTerm::Suffix(piece) => (NameMatch::Suffix, piece),
        TokenTerm::Contains(piece) => (NameMatch::Contains, piece),
    };
    let (found, more) = names(piece, how, MAX_EXPANSION)?;
    if more {
        return None;
    }
    // (The index matches case- and accent-insensitively.)
    let fits = |name: &str| match term {
        _ if fold_case => true,
        TokenTerm::Prefix(_) => name.starts_with(piece.as_str()),
        TokenTerm::Suffix(_) => name.ends_with(piece.as_str()),
        _ => name.contains(piece.as_str()),
    };
    Some(found.into_iter().filter(|name| fits(name)).collect())
}

/// A journal record's date (a summary's week's last day), and its revisions.
fn token_record_revs(record: &TokenDeltaRecord) -> (Option<NaiveDate>, Vec<&String>) {
    match record {
        TokenDeltaRecord::Detail(detail) => (
            parse_date(&detail.desc.iso_date),
            vec![&detail.desc.source_rev],
        ),
        TokenDeltaRecord::Summary(summary) => (
            week_end(summary.desc.iso_week_range),
            summary.desc.source_revs.iter().collect(),
        ),
    }
}

fn file_record_revs(record: &FileDeltaRecord) -> (Option<NaiveDate>, Vec<&String>) {
    match record {
        FileDeltaRecord::Detail(detail) => (
            parse_date(&detail.desc.iso_date),
            vec![&detail.desc.source_rev],
        ),
        FileDeltaRecord::Summary(summary) => (
            week_end(summary.desc.iso_week_range),
            summary.desc.source_revs.iter().collect(),
        ),
    }
}

fn parse_date(iso: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(iso.get(..10)?, "%Y-%m-%d").ok()
}

fn week_end((year, week, _): (u16, u8, u8)) -> Option<NaiveDate> {
    NaiveDate::from_isoywd_opt(year as i32, week as u32, Weekday::Sun)
}

/// What a file's version has of the pattern's matches: how many, their
/// lines (trimmed), and their captured values.
struct Found {
    count: usize,
    lines: BTreeMap<String, usize>,
    values: BTreeMap<Vec<Option<String>>, usize>,
}

fn found(re: &fancy_regex::Regex, text: &str, spans_lines: bool) -> Found {
    let (matches, _) = file_matches(re, text, spans_lines, usize::MAX);
    let mut result = Found {
        count: 0,
        lines: BTreeMap::new(),
        values: BTreeMap::new(),
    };
    for line in matches {
        *result
            .lines
            .entry(line.line_str.trim().to_string())
            .or_default() += line.hits.len();
        for hit in line.hits {
            result.count += 1;
            if !hit.captures.is_empty() {
                *result.values.entry(hit.captures).or_default() += 1;
            }
        }
    }
    result
}

/// What `a` has more of than `b` (the first `MAX_SHOWN`).
fn more_of<T: Ord + Clone>(a: &BTreeMap<T, usize>, b: &BTreeMap<T, usize>) -> Vec<T> {
    a.iter()
        .flat_map(|(item, &n)| {
            let extra = n.saturating_sub(b.get(item).copied().unwrap_or(0));
            std::iter::repeat_n(item.clone(), extra)
        })
        .take(MAX_SHOWN)
        .collect()
}

/// A blob's size at `path` in `tree` (from its header), and its id.
fn blob_at(repo: &Repository, tree: &Tree, path: &str) -> Option<(Oid, u64)> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    let size = repo.odb().ok()?.read_header(entry.id()).ok()?.0 as u64;
    Some((entry.id(), size))
}

fn blob_text(repo: &Repository, oid: Oid) -> String {
    repo.find_blob(oid)
        .map(|blob| String::from_utf8_lossy(blob.content()).into_owned())
        .unwrap_or_default()
}

/// Search `history` (of the source repo `source`) for `args`, within
/// `limits` (of `tier`), with `names` looking up the token names index (see
/// `TokenNames::find`).
pub fn history_grep(
    history: &TreeHistory,
    source: &Repository,
    args: &HistoryGrepArgs,
    limits: &QueryLimits,
    tier: Tier,
    names: &dyn Fn(&str, NameMatch, usize) -> Option<(Vec<String>, bool)>,
) -> Result<HistoryGrep, ServerError> {
    let started = std::time::Instant::now();
    let what = match (args.text.is_some(), args.timeline) {
        (true, false) => "text-history",
        (false, false) => "re-history",
        (true, true) => "text-timeline",
        (false, true) => "re-timeline",
    };
    let refuse = |estimates: &[(&str, u64)], details: Vec<String>, hints: Vec<String>| {
        TooExpensive::check(what, estimates, limits, tier).map(|mut refused| {
            refused.details = details;
            refused.hints = hints;
            ServerError::TooExpensive(Box::new(refused))
        })
    };
    if let Some(refused) = refuse(&[("history_grep", 1)], vec![], vec![]) {
        return Err(refused);
    }

    let fold_case = !args.case_sensitive;
    let plan: RegexPlan = regex_plan::plan(&args.pattern, fold_case)
        .map_err(|e| bad_input(format!("Bad regular expression: {}", e)))?;
    let re = compile(&args.pattern, fold_case)?;
    let path_regex = |re: &str| -> Result<Option<Regex>, ServerError> {
        match re {
            "" => Ok(None),
            re => Regex::new(re)
                .map(Some)
                .map_err(|e| bad_input(format!("Bad path regular expression: {}", e))),
        }
    };
    let (pathre, not_pathre) = (path_regex(&args.pathre)?, path_regex(&args.not_pathre)?);
    let path_ok = |path: &str| {
        pathre.as_ref().is_none_or(|re| re.is_match(path))
            && !not_pathre.as_ref().is_some_and(|re| re.is_match(path))
    };
    let date_arg = |date: &Option<String>, name: &str| -> Result<Option<NaiveDate>, ServerError> {
        match date {
            None => Ok(None),
            Some(date) => parse_date(date).map(Some).ok_or_else(|| {
                bad_input(format!("{}: isn't a date (YYYY-MM-DD): {:?}", name, date))
            }),
        }
    };
    let (since, until) = (
        date_arg(&args.since, "since")?,
        date_arg(&args.until, "until")?,
    );

    let timeline: &Repository = &history.timeline;
    let head = history
        .head_timeline_commit()
        .ok_or_else(|| data_problem("The history has no timeline".to_string()))?;
    let head_rev = head.id().to_string();
    let tree = head.tree().map_err(|e| data_problem(e.to_string()))?;

    let mut result = HistoryGrep {
        pattern: args.pattern.clone(),
        text: args.text.clone(),
        case_sensitive: args.case_sensitive,
        capture_names: capture_names(&re),
        page_start: args.page,
        ignored: args.ignored.clone(),
        ..Default::default()
    };

    // ## The plan: the token journals, or the files' journals.
    let tokens = plan.query.tokens();
    let mut costs: HashMap<(TokenTerm, bool), Option<(u64, Vec<String>)>> = HashMap::new();
    let mut cost = |term: &TokenTerm, fold: bool| -> Option<u64> {
        costs
            .entry((term.clone(), fold))
            .or_insert_with(|| {
                let tokens = term_tokens(term, fold, timeline, &tree, names)?;
                let bytes = tokens
                    .iter()
                    .filter_map(|token| journal_bytes(timeline, &tree, token))
                    .sum();
                Some((bytes, tokens))
            })
            .as_ref()
            .map(|(bytes, _)| *bytes)
    };
    let cheapest = tokens.cheapest(&mut cost);
    // (The files' journals when they're cheaper, or the token journals can't
    // see what's asked: values, which change without a token being added or
    // removed, ex: a version number.)
    let journal_path = |file: &str| format!("files-delta/{}.ndjson", file);
    let files_bytes: u64 = args
        .files
        .iter()
        .filter_map(|file| path_journal_bytes(timeline, &tree, Path::new(&journal_path(file))))
        .sum();
    let has_values = !result.capture_names.is_empty();
    let use_files = !args.files.is_empty()
        && (args.prefer_files
            || args.scope.is_some()
            || has_values
            || cheapest
                .as_ref()
                .is_none_or(|(token_bytes, _)| files_bytes <= *token_bytes));
    let mut candidates: BTreeMap<String, Option<NaiveDate>> = BTreeMap::new();
    // (Each journal's candidates in its order, newest first: the order the
    // history processed them, which is topological; see `merge_sequences`.)
    let mut sequences: Vec<Vec<(String, Option<NaiveDate>)>> = vec![];
    let in_window = |date: Option<NaiveDate>| {
        date.is_none_or(|date| since.is_none_or(|s| date >= s) && until.is_none_or(|u| date <= u))
    };
    let mut chosen_tokens: BTreeSet<String> = BTreeSet::new();
    // (With the files' journals: each candidate's files, as they were named
    // then, with the names before, if it moved them.)
    let mut rev_files: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    // (For a scope: its candidates changing the pattern's whole names, which
    // every match contains; see below.)
    let mut pruned: BTreeMap<String, Option<NaiveDate>> = BTreeMap::new();
    let whole_tokens: BTreeSet<String> = whole_terms(&tokens);
    if args.scope.is_some() && args.files.is_empty() {
        return Err(bad_input(format!(
            "No file has the scope {:?} (a path: can say which)",
            args.scope.as_deref().unwrap_or_default()
        )));
    }
    result.scope = args.scope.clone();

    if use_files {
        result.files = args.files.clone();
        let journal_bytes = files_bytes;
        if let Some(refused) = refuse(
            &[("history_journal_bytes", journal_bytes)],
            vec![format!(
                "Checking every revision of {} files would need {} KiB of history to be read",
                args.files.len(),
                journal_bytes / 1024
            )],
            vec!["a narrower path:".to_string()],
        ) {
            return Err(refused);
        }
        let mut reader = JournalReader::new(timeline);
        for file in &args.files {
            let version = JournalVersionRef {
                timeline_rev: head_rev.clone(),
                path: journal_path(file),
            };
            let records: Vec<FileDeltaRecord> = reader.records(&version).map_err(data_problem)?;
            // (The journal follows its file through renames: older records
            // are of the path it was renamed or copied from.)
            let mut path = file.clone();
            let mut sequence = vec![];
            for record in &records {
                let (date, revs) = file_record_revs(record);
                let moved_from = match record {
                    FileDeltaRecord::Detail(detail) => detail.delta.moved_from.clone(),
                    FileDeltaRecord::Summary(_) => None,
                };
                let at = (path.clone(), moved_from.clone());
                if let Some(from) = moved_from {
                    path = from;
                }
                if !in_window(date) {
                    continue;
                }
                // (With a scope, those changing it, and those changing its
                // names the pattern's matches contain, in case those are
                // fewer and needed.)
                if let Some(scope) = &args.scope {
                    let group = file_record_group(record);
                    if !touches_scope(group, scope, None) {
                        continue;
                    }
                    if touches_scope(group, scope, Some(&whole_tokens)) {
                        for rev in &revs {
                            pruned.entry((*rev).clone()).or_insert(date);
                        }
                    }
                }
                for rev in revs {
                    candidates.entry(rev.clone()).or_insert(date);
                    rev_files.entry(rev.clone()).or_default().push(at.clone());
                    sequence.push((rev.clone(), date));
                }
            }
            sequences.push(sequence);
        }
        // (Too many of the scope's revisions: those changing the pattern's
        // names in it, if it has any, which can miss changes to the rest of
        // its matches.)
        let too_many = |n: usize| {
            n as u64 > limits.history_revisions
                || args.timeline && n as u64 > limits.history_candidates
        };
        if args.scope.is_some() && too_many(candidates.len()) && !whole_tokens.is_empty() {
            candidates = pruned;
            result.pruned = true;
        }
    } else {
        let Some((bytes, terms)) = cheapest else {
            let hints = vec![
                "text that every match contains, a name (not a keyword or a number)".to_string(),
                format!(
                    "or a path: naming at most {} files, whose every revision is checked",
                    MAX_PATH_FILES
                ),
            ];
            let mut refused = TooExpensive {
                what: what.to_string(),
                exceeded: vec![],
                details: vec![format!(
                    "{:?} has no name every match contains whose history the server keeps, so every revision would need to be checked",
                    args.text.as_deref().unwrap_or(&args.pattern)
                )],
                hints,
                tier,
                signed_in_allows: false,
            };
            // (Whatever the limits: it's every revision.)
            refused.exceeded.clear();
            return Err(ServerError::TooExpensive(Box::new(refused)));
        };
        for (term, fold) in &terms {
            if let Some(Some((_, tokens))) = costs.get(&((*term).clone(), *fold)) {
                chosen_tokens.extend(tokens.iter().cloned());
            }
        }
        if let Some(refused) = refuse(
            &[("history_journal_bytes", bytes)],
            vec![format!(
                "Its rarest name{} {} would need {} KiB of history to be read",
                if chosen_tokens.len() == 1 { "" } else { "s" },
                chosen_tokens
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                bytes / 1024
            )],
            vec![
                "a rarer name in the text".to_string(),
                "since:YYYY-MM-DD, for the recent history".to_string(),
            ],
        ) {
            return Err(refused);
        }
        for token in &chosen_tokens {
            let Some(journal) =
                read_journal(timeline, &tree, token, usize::MAX).map_err(data_problem)?
            else {
                continue;
            };
            let mut sequence = vec![];
            for record in &journal.records {
                let (date, revs) = token_record_revs(record);
                if in_window(date) {
                    for rev in revs {
                        candidates.entry(rev.clone()).or_insert(date);
                        sequence.push((rev.clone(), date));
                    }
                }
            }
            sequences.push(sequence);
        }
        result.tokens = chosen_tokens.iter().cloned().collect();
    }

    // (Newest first: by when they landed, parents first, from the commit
    // index, rather than by their commit dates (see `tools::commit_index`);
    // or, without it, in the journals' order.)
    let mut ordered = merge_sequences(&sequences, &|rev| candidates.contains_key(rev));
    let positions = history
        .commit_index()
        .map(|index| index.positions(ordered.iter().map(|(rev, _)| rev.as_str())))
        .unwrap_or_default();
    result.landing_order = landing_order(&mut ordered, &positions);
    result.candidates = ordered.len();
    if let Some(refused) = refuse(
        &[("history_revisions", ordered.len() as u64)],
        vec![format!(
            "{} revisions would need to be checked ({})",
            ordered.len(),
            if use_files {
                format!("every revision of {} files", args.files.len())
            } else {
                format!("those changing {}", result.tokens.join(", "))
            }
        )],
        vec![
            "a rarer name in the text".to_string(),
            "a narrower path:".to_string(),
            "since:YYYY-MM-DD, for the recent history".to_string(),
        ],
    ) {
        return Err(refused);
    }

    // (A timeline checks all of its candidates at once, so it's refused now
    // if what that takes is over the limits: the files' journals say how
    // many file versions; the files' sizes now estimate theirs; and a scope
    // takes a parse of each version.)
    if args.timeline {
        let pairs: u64 = if use_files {
            ordered
                .iter()
                .map(|(rev, _)| rev_files.get(rev).map_or(0, |files| files.len() as u64))
                .sum()
        } else {
            ordered.len() as u64
        };
        let mut bytes_estimate = 0;
        if use_files && let Ok(head_tree) = source.head().and_then(|head| head.peel_to_tree()) {
            let sizes: Vec<u64> = args
                .files
                .iter()
                .filter_map(|file| blob_at(source, &head_tree, file).map(|(_, size)| size))
                .collect();
            if !sizes.is_empty() {
                bytes_estimate = 2 * pairs * (sizes.iter().sum::<u64>() / sizes.len() as u64);
            }
        }
        let parses = if args.scope.is_some() { 2 * pairs } else { 0 };
        if let Some(refused) = refuse(
            &[
                ("history_candidates", pairs),
                ("history_blob_bytes", bytes_estimate),
                ("history_parses", parses),
            ],
            vec![format!(
                "A timeline checks all of its {} revisions at once ({} file versions{})",
                ordered.len(),
                pairs,
                if args.scope.is_some() {
                    ", finding the scope's lines in each"
                } else {
                    ""
                }
            )],
            vec![
                "since:YYYY-MM-DD and until:YYYY-MM-DD, for a shorter time".to_string(),
                "a narrower path:".to_string(),
            ],
        ) {
            return Err(refused);
        }
    }

    // ## The page: each candidate revision's files, checked.
    let mut changes: Vec<Change> = vec![];
    let mut index = args.page;
    while index < ordered.len() {
        // (A page stops at its time limit; a timeline, which can't, is
        // refused.)
        let elapsed = started.elapsed().as_secs();
        if result.checked > 0 && elapsed >= limits.history_page_seconds {
            if args.timeline
                && let Some(refused) = refuse(
                    &[("history_page_seconds", elapsed + 1)],
                    vec![format!(
                        "A timeline checks all of its {} revisions at once, and {} of them took {} s",
                        ordered.len(),
                        index - args.page,
                        elapsed
                    )],
                    vec!["since:YYYY-MM-DD and until:YYYY-MM-DD, for a shorter time".to_string()],
                )
            {
                return Err(refused);
            }
            result.out_of_time = true;
            break;
        }
        let rev = &ordered[index].0;
        let Some(summary) = read_rev_summary(&history.path, rev) else {
            result.unsummarized += 1;
            index += 1;
            continue;
        };
        let Some(commit) = Oid::from_str(rev)
            .ok()
            .and_then(|oid| source.find_commit(oid).ok())
        else {
            result.unsummarized += 1;
            index += 1;
            continue;
        };
        // (Merges are skipped, as `git log -S` does, but for a timeline of
        // values: where a side branch's commits lacked what the merge's other
        // side had (ex: a name another branch's backout restored), only the
        // merge removes it, so its change from its first parent counts,
        // which applying again what the side branch's commits applied doesn't
        // change, since values are sets; see `build_timeline`.)
        let merge = commit.parent_count() > 1;
        if merge && !(args.timeline && !result.capture_names.is_empty() && use_files) {
            result.merges_skipped += 1;
            index += 1;
            continue;
        }
        let files = match rev_files.get(rev) {
            // (Only the files named, as they were named then, which changed
            // the scope if there is one; a summarized week's revisions are
            // checked by their own summaries, if they list the file.)
            Some(files) if use_files => files
                .iter()
                .filter(|(path, _)| {
                    args.scope.as_deref().is_none_or(|scope| {
                        summary
                            .file_deltas
                            .get(path)
                            .is_none_or(|file| touches_scope(&file.delta.symbol_group, scope, None))
                    })
                })
                .cloned()
                .collect(),
            _ => revision_files(
                source,
                &commit,
                &summary,
                &chosen_tokens,
                use_files,
                &path_ok,
                args.scope.as_deref(),
            ),
        };
        let (Ok(new_tree), old_tree) = (
            commit.tree(),
            commit.parent(0).ok().and_then(|parent| parent.tree().ok()),
        ) else {
            index += 1;
            continue;
        };
        // (Each file's versions' blobs and sizes, to check against the
        // page's limits before reading them.)
        let pairs: Vec<(
            String,
            Option<String>,
            Option<(Oid, u64)>,
            Option<(Oid, u64)>,
        )> = files
            .into_iter()
            .map(|(path, old_path)| {
                let new = blob_at(source, &new_tree, &path);
                let old = old_tree.as_ref().and_then(|old_tree| {
                    blob_at(source, old_tree, old_path.as_deref().unwrap_or(&path))
                });
                (path, old_path, new, old)
            })
            .collect();
        let bytes: u64 = pairs
            .iter()
            .map(|(_, _, new, old)| new.map_or(0, |b| b.1) + old.map_or(0, |b| b.1))
            .sum();
        if result.checked > 0
            && (result.checked + pairs.len() > limits.history_candidates as usize
                || result.bytes + bytes > limits.history_blob_bytes)
        {
            if args.timeline
                && let Some(refused) = refuse(
                    &[("history_blob_bytes", result.bytes + bytes)],
                    vec![format!(
                        "A timeline checks all of its {} revisions at once, and they're big",
                        ordered.len()
                    )],
                    vec!["since:YYYY-MM-DD, for the recent history".to_string()],
                )
            {
                return Err(refused);
            }
            break;
        }
        // (A scope's lines take a parse of each version.)
        if args.scope.is_some()
            && result.parses > 0
            && (result.parses + 2 * pairs.len()) as u64 > limits.history_parses
        {
            if args.timeline
                && let Some(refused) = refuse(
                    &[("history_parses", (result.parses + 2 * pairs.len()) as u64)],
                    vec![format!(
                        "A timeline checks all of its {} revisions at once, finding the scope's lines in each",
                        ordered.len()
                    )],
                    vec!["since:YYYY-MM-DD, for the recent history".to_string()],
                )
            {
                return Err(refused);
            }
            break;
        }
        result.checked += pairs.len();
        result.bytes += bytes;
        let parent = commit.parent_id(0).ok();
        // (When it landed, if the commit index knows, with its commit date if
        // that's another day.)
        // (A history's dates are landing dates since Part 541, with the
        // commit's own, if it's another, in `commit_date`; older histories'
        // are commit dates.)
        let summary_date: String = summary.iso_date.chars().take(10).collect();
        let date = positions.get(rev).map_or(summary_date.clone(), |position| {
            position.landed.chars().take(10).collect()
        });
        let committed: String = summary
            .commit_date
            .as_deref()
            .map_or(summary_date, |commit_date| {
                commit_date.chars().take(10).collect()
            });
        let committed = (committed != date).then_some(committed);
        for (path, old_path, new, old) in pairs {
            let mut new_text = new.map(|b| blob_text(source, b.0)).unwrap_or_default();
            let mut old_text = old.map(|b| blob_text(source, b.0)).unwrap_or_default();
            if let Some(scope) = &args.scope {
                new_text = scope_text(history, commit.id(), &path, &new_text, scope);
                old_text = match parent {
                    Some(parent) => scope_text(
                        history,
                        parent,
                        old_path.as_deref().unwrap_or(&path),
                        &old_text,
                        scope,
                    ),
                    None => String::new(),
                };
                result.parses += 2;
            }
            let new_found = found(&re, &new_text, plan.spans_lines);
            let old_found = found(&re, &old_text, plan.spans_lines);
            if new_found.count == old_found.count && new_found.values == old_found.values {
                continue;
            }
            changes.push(Change {
                seq: index,
                merge,
                rev: rev.clone(),
                date: date.clone(),
                committed: committed.clone(),
                header: header_html(&summary.message),
                backout: !summary.backs_out.is_empty(),
                backs_out: summary.backs_out.clone(),
                path,
                old_path,
                old: old_found,
                new: new_found,
            });
        }
        index += 1;
    }
    result.page_end = index;
    result.next_page = (index < ordered.len()).then_some(index);
    if !args.show_backouts {
        result.backouts_hidden = hide_backout_pairs(&mut changes);
    }
    result.rows = changes
        .iter()
        .filter(|change| !change.merge)
        .map(|change| HistoryGrepRow {
            rev: change.rev.clone(),
            date: change.date.clone(),
            committed: change.committed.clone(),
            header: change.header.clone(),
            backout: change.backout,
            path: change.path.clone(),
            old_path: change.old_path.clone(),
            before: change.old.count,
            after: change.new.count,
            added_lines: more_of(&change.new.lines, &change.old.lines),
            removed_lines: more_of(&change.old.lines, &change.new.lines),
            added_values: more_of(&change.new.values, &change.old.values),
            removed_values: more_of(&change.old.values, &change.new.values),
        })
        .collect();
    if args.timeline {
        result.timeline = Some(build_timeline(&changes, !result.capture_names.is_empty()));
    }
    Ok(result)
}

/// A revision's change to a file's matches.
struct Change {
    /// Its place among the candidates, newest first (see `merge_sequences`).
    seq: usize,
    /// Whether it's a merge's (from its first parent; only for timelines of
    /// values, which only show it if it changed them).
    merge: bool,
    rev: String,
    /// When it landed (YYYY-MM-DD; see `HistoryGrepRow::date`), and when it
    /// was committed, if that was another day.
    date: String,
    committed: Option<String>,
    header: String,
    backout: bool,
    /// The revisions it backs out, if it's a backout.
    backs_out: Vec<String>,
    path: String,
    old_path: Option<String>,
    old: Found,
    new: Found,
}

/// Leave out the changes of revisions which were backed out, with their
/// backouts', where the backout undid them exactly in the file (the file's
/// matches after the backout are those before the backed out revision, and
/// those before it those after the revision): ex: a landing, its backout and
/// its relanding, of which only the relanding is interesting.  (The history
/// knows backouts by their messages; see `hyperblame::backouts`.)  A backout
/// which didn't undo its revision exactly in the file (ex: with other changes
/// between them) is left, as is one of a backout (a relanding).  How many
/// backed out revisions' changes were left out.
fn hide_backout_pairs(changes: &mut Vec<Change>) -> usize {
    let index: HashMap<(String, String), usize> = changes
        .iter()
        .enumerate()
        .map(|(i, change)| ((change.rev.clone(), change.path.clone()), i))
        .collect();
    let mut hidden = vec![false; changes.len()];
    for (i, backout) in changes.iter().enumerate() {
        if backout.merge {
            continue;
        }
        for rev in &backout.backs_out {
            let Some(&j) = index.get(&(rev.clone(), backout.path.clone())) else {
                continue;
            };
            let landing = &changes[j];
            let same = |a: &Found, b: &Found| {
                a.count == b.count && a.values == b.values && a.lines == b.lines
            };
            if !hidden[i]
                && !hidden[j]
                && same(&backout.new, &landing.old)
                && same(&backout.old, &landing.new)
            {
                hidden[i] = true;
                hidden[j] = true;
            }
        }
    }
    let mut i = 0;
    changes.retain(|_| {
        i += 1;
        !hidden[i - 1]
    });
    hidden.iter().filter(|h| **h).count() / 2
}

/// Order `candidates` (revisions, with their journals' dates) newest first by
/// when they landed, parents first (by their `positions`; see `tools::
/// commit_index`), if every one has a position: whether it did.
fn landing_order(
    candidates: &mut [(String, Option<NaiveDate>)],
    positions: &HashMap<String, CommitPosition>,
) -> bool {
    if !candidates
        .iter()
        .all(|(rev, _)| positions.contains_key(rev))
    {
        return false;
    }
    candidates.sort_by(|(a, _), (b, _)| positions[b].order_key().cmp(&positions[a].order_key()));
    true
}

/// The revisions of several journals' sequences, each newest first, merged
/// newest first: each journal's order kept, and different journals'
/// interleaved by their dates.  Only those `keep` takes, once each.  (For
/// when the commit index can't order them: the journals' order is the
/// history's, mostly, but weekly summaries group and list revisions by their
/// commit dates, which for hg's commits aren't when they landed; see
/// `tools::commit_index`.)
fn merge_sequences(
    sequences: &[Vec<(String, Option<NaiveDate>)>],
    keep: &dyn Fn(&str) -> bool,
) -> Vec<(String, Option<NaiveDate>)> {
    let mut at = vec![0; sequences.len()];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut merged = vec![];
    loop {
        for (i, sequence) in sequences.iter().enumerate() {
            while at[i] < sequence.len()
                && (seen.contains(&sequence[at[i]].0) || !keep(&sequence[at[i]].0))
            {
                at[i] += 1;
            }
        }
        // (The newest head; of equal ones, the first journal's.)
        let Some(i) = (0..sequences.len())
            .filter(|&i| at[i] < sequences[i].len())
            .max_by(|&a, &b| {
                sequences[a][at[a]]
                    .1
                    .cmp(&sequences[b][at[b]].1)
                    .then(b.cmp(&a))
            })
        else {
            break;
        };
        let (rev, date) = sequences[i][at[i]].clone();
        seen.insert(rev.clone());
        merged.push((rev, date));
        at[i] += 1;
    }
    merged
}

/// A value's label: its groups' texts, joined.
fn value_label(value: &[Option<String>]) -> String {
    value
        .iter()
        .map(|part| part.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The timeline of `changes` (any order; see `Timeline`), of values if
/// `values`.
fn build_timeline(changes: &[Change], values: bool) -> Timeline {
    // (Oldest first, in the candidates' order: by when they landed, parents
    // first, not by their commit dates; see `history_grep`.)
    let mut changes: Vec<&Change> = changes
        .iter()
        .filter(|change| values || !change.merge)
        .collect();
    changes.sort_by_key(|change| std::cmp::Reverse(change.seq));
    // (Revisions in order, each with its changes.)
    let mut revisions: Vec<(&Change, Vec<&Change>)> = vec![];
    for change in changes {
        match revisions.last_mut() {
            Some((first, group)) if first.rev == change.rev => group.push(change),
            _ => revisions.push((change, vec![change])),
        }
    }
    let point =
        |first: &Change, value: f64, added: Vec<String>, removed: Vec<String>| TimelinePoint {
            rev: first.rev.clone(),
            date: first.date.clone(),
            committed: first.committed.clone(),
            header: first.header.clone(),
            value,
            added,
            removed,
        };
    let number = |value: &Vec<Option<String>>| -> Option<f64> {
        match value.as_slice() {
            [Some(text)] => text.trim().parse::<f64>().ok(),
            _ => None,
        }
    };

    // One numeric value per file, for a few files: their values.
    let files: BTreeSet<&str> = revisions
        .iter()
        .flat_map(|(_, group)| group.iter().map(|c| c.path.as_str()))
        .collect();
    let single_numbers = values
        && files.len() <= MAX_VALUE_SERIES
        && revisions.iter().all(|(_, group)| {
            group.iter().all(|c| {
                [&c.old, &c.new].iter().all(|found| {
                    found.values.len() <= 1 && found.values.keys().all(|v| number(v).is_some())
                })
            })
        });
    if single_numbers {
        let mut series = vec![];
        for file in files {
            let mut line = TimelineSeries {
                label: file.to_string(),
                ..Default::default()
            };
            for (first, group) in &revisions {
                let Some(change) = group.iter().find(|c| c.path == file) else {
                    continue;
                };
                let value_of = |found: &Found| found.values.keys().next().and_then(number);
                // (A merge's point only if it changed the value.)
                if change.merge
                    && line.points.last().is_some_and(|p: &TimelinePoint| {
                        p.value.to_bits() == value_of(&change.new).unwrap_or(f64::NAN).to_bits()
                    })
                {
                    continue;
                }
                if line.points.is_empty() {
                    line.start = value_of(&change.old).unwrap_or(f64::NAN);
                }
                line.points.push(point(
                    first,
                    value_of(&change.new).unwrap_or(f64::NAN),
                    change.new.values.keys().map(|v| value_label(v)).collect(),
                    change.old.values.keys().map(|v| value_label(v)).collect(),
                ));
            }
            series.push(line);
        }
        return Timeline {
            measure: "value".to_string(),
            series,
        };
    }

    // Otherwise one line, over the files: their matches, or their distinct
    // values.  Each change applies what it added and removed to its file's
    // state, rather than replacing it, so that changes on parallel branches
    // (whose `old`s are their own branch's) don't undo each other; values as
    // sets (the values it introduced, and those it left none of), so that a
    // change seen twice (ex: cherry-picked to another branch) isn't applied
    // twice.
    struct FileState {
        count: i64,
        values: BTreeMap<Vec<Option<String>>, i64>,
    }
    let mut state: HashMap<&str, FileState> = HashMap::new();
    let distinct = |state: &HashMap<&str, FileState>| -> BTreeSet<Vec<Option<String>>> {
        state
            .values()
            .flat_map(|file| {
                file.values
                    .iter()
                    .filter(|(_, n)| **n > 0)
                    .map(|(value, _)| value.clone())
            })
            .collect()
    };
    let count = |state: &HashMap<&str, FileState>| -> f64 {
        state.values().map(|file| file.count.max(0) as f64).sum()
    };
    let mut line = TimelineSeries::default();
    for (index, (first, group)) in revisions.iter().enumerate() {
        // (A file's state before its first change is its `old`.)
        for change in group {
            state
                .entry(change.path.as_str())
                .or_insert_with(|| FileState {
                    count: change.old.count as i64,
                    values: change
                        .old
                        .values
                        .iter()
                        .map(|(value, n)| (value.clone(), *n as i64))
                        .collect(),
                });
        }
        let before_values = distinct(&state);
        let before_count = count(&state);
        if index == 0 {
            line.start = if values {
                before_values.len() as f64
            } else {
                before_count
            };
        }
        for change in group {
            let file = state.get_mut(change.path.as_str()).unwrap();
            file.count += change.new.count as i64 - change.old.count as i64;
            for value in change.new.values.keys() {
                if !change.old.values.contains_key(value) {
                    file.values.insert(value.clone(), 1);
                }
            }
            for value in change.old.values.keys() {
                if !change.new.values.contains_key(value) {
                    file.values.insert(value.clone(), 0);
                }
            }
        }
        let after_values = distinct(&state);
        // (Merges only where they changed the values.)
        if values && group.iter().all(|c| c.merge) && after_values == before_values {
            continue;
        }
        let (value, added, removed) = if values {
            (
                after_values.len() as f64,
                after_values
                    .difference(&before_values)
                    .map(|v| value_label(v))
                    .collect(),
                before_values
                    .difference(&after_values)
                    .map(|v| value_label(v))
                    .collect(),
            )
        } else {
            (
                count(&state),
                group
                    .iter()
                    .flat_map(|c| more_of(&c.new.lines, &c.old.lines))
                    .take(MAX_SHOWN)
                    .collect(),
                group
                    .iter()
                    .flat_map(|c| more_of(&c.old.lines, &c.new.lines))
                    .take(MAX_SHOWN)
                    .collect(),
            )
        };
        line.points.push(point(first, value, added, removed));
    }
    Timeline {
        measure: if values { "values" } else { "matches" }.to_string(),
        series: vec![line],
    }
}

/// The files of `commit` (summarized by `summary`) to check, with their
/// paths before it if it moved them: with the token journals, those where
/// one of `tokens` changed; with the files' journals, those `path_ok` takes.
/// (A revision changing too many files for its summary to list them has its
/// changed files from git.)
fn revision_files(
    source: &Repository,
    commit: &git2::Commit,
    summary: &RevSummaryRecord,
    tokens: &BTreeSet<String>,
    use_files: bool,
    path_ok: &dyn Fn(&str) -> bool,
    scope: Option<&str>,
) -> Vec<(String, Option<String>)> {
    if summary.file_deltas.is_empty() {
        if summary.file_totals.is_none() {
            return vec![];
        }
        let (Ok(new_tree), Some(old_tree)) = (
            commit.tree(),
            commit.parent(0).ok().and_then(|parent| parent.tree().ok()),
        ) else {
            return vec![];
        };
        let Ok(diff) = source.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None) else {
            return vec![];
        };
        return diff
            .deltas()
            .filter_map(|delta| {
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path())?
                    .to_string_lossy()
                    .into_owned();
                path_ok(&path).then_some((path, None))
            })
            .collect();
    }
    summary
        .file_deltas
        .iter()
        .filter(|(path, _)| path_ok(path))
        .filter(|(_, file)| {
            scope.is_none_or(|scope| touches_scope(&file.delta.symbol_group, scope, None))
        })
        .filter(|(_, file)| {
            use_files
                || file
                    .delta
                    .symbol_group
                    .symbol_deltas
                    .values()
                    .any(|symbol| {
                        symbol
                            .token_changes
                            .keys()
                            .any(|token| tokens.contains(token))
                    })
        })
        .map(|(path, file)| (path.clone(), file.delta.moved_from.clone()))
        .collect()
}

/// Whether `context` (a history context's pretty identifier) is `scope` or
/// in it (as `format::history_data` decides).
fn in_scope(context: &str, scope: &str) -> bool {
    context == scope
        || context
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with("::"))
}

/// Whether a files-delta group changed `scope`'s tokens (any of them, or if
/// `tokens`, one of those).
fn touches_scope(
    group: &crate::file_format::history::timeline_common::SymbolSyntaxDeltaGroup,
    scope: &str,
    tokens: Option<&BTreeSet<String>>,
) -> bool {
    group.symbol_deltas.iter().any(|(context, delta)| {
        in_scope(context, scope)
            && match tokens {
                None => {
                    let totals = &delta.token_totals;
                    totals.added + totals.removed + totals.evolved_from + totals.evolved_into > 0
                }
                Some(tokens) => delta
                    .token_changes
                    .keys()
                    .any(|token| tokens.contains(token)),
            }
    })
}

fn file_record_group(
    record: &FileDeltaRecord,
) -> &crate::file_format::history::timeline_common::SymbolSyntaxDeltaGroup {
    match record {
        FileDeltaRecord::Detail(detail) => &detail.delta.symbol_group,
        FileDeltaRecord::Summary(summary) => &summary.symbol_group,
    }
}

/// The whole tokens of a token query's terms (any of them; for narrowing a
/// scope's revisions).
fn whole_terms(query: &regex_plan::TokenQuery) -> BTreeSet<String> {
    use regex_plan::TokenQuery;
    match query {
        TokenQuery::All => BTreeSet::new(),
        TokenQuery::Term {
            term: TokenTerm::Whole(token),
            ..
        } => BTreeSet::from([token.clone()]),
        TokenQuery::Term { .. } => BTreeSet::new(),
        TokenQuery::And(parts) | TokenQuery::Or(parts) => {
            parts.iter().flat_map(whole_terms).collect()
        }
    }
}

/// `scope`'s lines in `text`, the version of `path` in the source revision
/// `rev` (by the history's contexts; see `TreeHistory::container_lines`), or
/// nothing if it isn't there.
fn scope_text(history: &TreeHistory, rev: Oid, path: &str, text: &str, scope: &str) -> String {
    let Some(timeline_commit) = history.timeline_commit(rev) else {
        return String::new();
    };
    let Ok(Some(lines)) = history.container_lines(&timeline_commit, path, text) else {
        return String::new();
    };
    let Some(&(first, last)) = lines.get(scope) else {
        return String::new();
    };
    text.lines()
        .skip(first.saturating_sub(1) as usize)
        .take((last + 1).saturating_sub(first) as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_found_and_differences() {
        let re = compile(r"kMax\s*=\s*(\d+)", false).unwrap();
        let old = found(&re, "int kMax = 3;\nint kMax = 4;\n", false);
        let new = found(&re, "int kMax = 3;\nint kMax = 5;\n", false);
        assert_eq!((old.count, new.count), (2, 2));
        assert_eq!(
            more_of(&new.values, &old.values),
            vec![vec![Some("5".to_string())]]
        );
        assert_eq!(
            more_of(&old.values, &new.values),
            vec![vec![Some("4".to_string())]]
        );
        assert_eq!(
            more_of(&new.lines, &old.lines),
            vec!["int kMax = 5;".to_string()]
        );
        // (A reordering is no change.)
        let swapped = found(&re, "int kMax = 4;\nint kMax = 3;\n", false);
        assert!(more_of(&swapped.values, &old.values).is_empty());
        assert_eq!(
            week_end((2026, 43, 43)),
            NaiveDate::from_ymd_opt(2026, 10, 25)
        );
    }

    fn change(rev: &str, date: &str, path: &str, re: &str, old: &str, new: &str) -> Change {
        let re = compile(re, false).unwrap();
        Change {
            // (Newer dates first, as the candidates are.)
            seq: 99_999_999 - date.replace('-', "").parse::<usize>().unwrap(),
            merge: false,
            rev: rev.to_string(),
            date: date.to_string(),
            committed: None,
            header: format!("Commit {}", rev),
            backout: false,
            backs_out: vec![],
            path: path.to_string(),
            old_path: None,
            old: found(&re, old, false),
            new: found(&re, new, false),
        }
    }

    #[test]
    fn test_merge_sequences() {
        let date = |d: &str| parse_date(d);
        // (One journal's order is kept, though its dates disagree: the
        // relanding keeps its landing's date.)
        let a = vec![
            ("reland".to_string(), date("2014-02-12")),
            ("backout".to_string(), date("2014-02-12")),
            ("land".to_string(), date("2014-02-12")),
            ("older".to_string(), date("2014-01-01")),
        ];
        let b = vec![
            ("b2".to_string(), date("2014-03-01")),
            ("backout".to_string(), date("2014-02-12")),
            ("b1".to_string(), date("2014-01-15")),
        ];
        let merged: Vec<String> = merge_sequences(&[a, b], &|rev| rev != "older")
            .into_iter()
            .map(|(rev, _)| rev)
            .collect();
        assert_eq!(merged, vec!["b2", "reland", "backout", "land", "b1"]);
    }

    #[test]
    fn test_landing_order() {
        // (A patch landed thrice and backed out twice, all landings with one
        // date: by date, the backouts would come last.)
        let re = r"MACRO\((\w+)\)";
        let (p, d) = ("MACRO(POPNV)", "MACRO(DUPAT)");
        let mut changes = vec![
            change("land1", "2014-02-12", "o.h", re, p, d),
            change("out1", "2014-02-12", "o.h", re, d, p),
            change("land2", "2014-02-12", "o.h", re, p, d),
            change("out2", "2014-02-12", "o.h", re, d, p),
            change("land3", "2014-02-12", "o.h", re, p, d),
        ];
        let date = parse_date("2014-02-12");
        let mut candidates: Vec<(String, Option<NaiveDate>)> =
            ["out2", "out1", "land3", "land2", "land1"]
                .iter()
                .map(|rev| (rev.to_string(), date))
                .collect();
        // (Each landed when its backout did, at the earliest.)
        let landed = ["17:46", "18:10", "18:10", "19:30", "19:30"];
        let mut positions: HashMap<String, CommitPosition> = changes
            .iter()
            .zip(landed)
            .enumerate()
            .map(|(i, (change, time))| {
                let position = CommitPosition {
                    landed: format!("2014-02-12T{}:00Z", time),
                    base: false,
                    position: i as u64,
                };
                (change.rev.clone(), position)
            })
            .collect();
        assert!(landing_order(&mut candidates, &positions));
        let order: Vec<&str> = candidates.iter().map(|(rev, _)| rev.as_str()).collect();
        assert_eq!(order, vec!["land3", "out2", "land2", "out1", "land1"]);
        // (So the values end with the last landing's.)
        for change in &mut changes {
            change.seq = order.iter().position(|rev| *rev == change.rev).unwrap();
        }
        let timeline = build_timeline(&changes, true);
        let last = timeline.series[0].points.last().unwrap();
        assert_eq!(last.rev, "land3");
        assert_eq!(last.added, vec!["DUPAT"]);
        // (Without every candidate's position, the order stays.)
        positions.remove("out1");
        let mut unordered = candidates.clone();
        unordered.reverse();
        assert!(!landing_order(&mut unordered, &positions));
        assert_eq!(unordered[0].0, "land1");
    }

    #[test]
    fn test_hide_backout_pairs() {
        let re = r"MACRO\((\w+)\)";
        let backout = |rev: &str, date: &str, of: &str, old: &str, new: &str| {
            let mut change = change(rev, date, "o.h", re, old, new);
            change.backout = true;
            change.backs_out = vec![of.to_string()];
            change
        };
        let mut changes = vec![
            change(
                "land",
                "2020-01-14",
                "o.h",
                re,
                "MACRO(A)",
                "MACRO(A) MACRO(B)",
            ),
            backout("out", "2020-01-14", "land", "MACRO(A) MACRO(B)", "MACRO(A)"),
            change(
                "reland",
                "2020-01-15",
                "o.h",
                re,
                "MACRO(A)",
                "MACRO(A) MACRO(B)",
            ),
            // (A backout which didn't undo its revision exactly stays.)
            change(
                "land2",
                "2020-02-01",
                "o.h",
                re,
                "MACRO(A) MACRO(B)",
                "MACRO(A) MACRO(B) MACRO(C)",
            ),
            backout(
                "out2",
                "2020-02-02",
                "land2",
                "MACRO(A) MACRO(B) MACRO(C)",
                "MACRO(A)",
            ),
        ];
        assert_eq!(hide_backout_pairs(&mut changes), 1);
        assert_eq!(
            changes.iter().map(|c| c.rev.as_str()).collect::<Vec<_>>(),
            vec!["reland", "land2", "out2"]
        );
        // (The timeline is the same without the pair.)
        let timeline = build_timeline(&changes, true);
        assert_eq!(timeline.series[0].points[0].added, vec!["B"]);
    }

    #[test]
    fn test_timelines() {
        // (A constant's value, from changes in any order.)
        let re = r"kMax = (\d+)";
        let changes = vec![
            change("b", "2021-02-01", "a.h", re, "kMax = 3;", "kMax = 5;"),
            change("a", "2020-01-01", "a.h", re, "", "kMax = 3;"),
        ];
        let timeline = build_timeline(&changes, true);
        assert_eq!(timeline.measure, "value");
        let line = &timeline.series[0];
        assert_eq!(line.label, "a.h");
        assert!(line.start.is_nan());
        assert_eq!(
            line.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![3.0, 5.0]
        );
        assert_eq!(line.points[1].added, vec!["5"]);
        assert_eq!(line.points[1].removed, vec!["3"]);

        // (A switch's cases: how many distinct labels, with those added and
        // removed; reordering isn't a change, and a file's other cases count
        // from before its first change.)
        let re = r"case (\w+):";
        let changes = vec![
            change(
                "a",
                "2020-01-01",
                "i.cpp",
                re,
                "case A: case B:",
                "case A: case B: case C:",
            ),
            change(
                "b",
                "2020-02-01",
                "i.cpp",
                re,
                "case A: case B: case C:",
                "case C: case B: case A:",
            ),
            change(
                "c",
                "2020-03-01",
                "i.cpp",
                re,
                "case C: case B: case A:",
                "case C: case D:",
            ),
        ];
        let timeline = build_timeline(&changes, true);
        assert_eq!(timeline.measure, "values");
        let line = &timeline.series[0];
        assert_eq!(line.start, 2.0);
        assert_eq!(
            line.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![3.0, 3.0, 2.0]
        );
        assert_eq!(line.points[0].added, vec!["C"]);
        assert!(line.points[1].added.is_empty() && line.points[1].removed.is_empty());
        assert_eq!(line.points[2].added, vec!["D"]);
        assert_eq!(line.points[2].removed, vec!["A", "B"]);

        // (Counts, over files.)
        let re = r"structopt";
        let changes = vec![
            change("a", "2020-01-01", "x.rs", re, "", "structopt structopt"),
            change("a", "2020-01-01", "y.rs", re, "", "structopt"),
            change("b", "2021-01-01", "x.rs", re, "structopt structopt", ""),
        ];
        let timeline = build_timeline(&changes, false);
        assert_eq!(timeline.measure, "matches");
        let line = &timeline.series[0];
        assert_eq!(
            line.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![3.0, 1.0]
        );
    }
}
