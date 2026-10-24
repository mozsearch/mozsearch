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
    pub date: String,
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
    pub date: String,
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
    let what = if args.text.is_some() {
        "text-history"
    } else {
        "re-history"
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
    let in_window = |date: Option<NaiveDate>| {
        date.is_none_or(|date| since.is_none_or(|s| date >= s) && until.is_none_or(|u| date <= u))
    };
    let mut chosen_tokens: BTreeSet<String> = BTreeSet::new();
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
            for record in &records {
                let (date, revs) = file_record_revs(record);
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
                }
            }
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
            for record in &journal.records {
                let (date, revs) = token_record_revs(record);
                if in_window(date) {
                    for rev in revs {
                        candidates.entry(rev.clone()).or_insert(date);
                    }
                }
            }
        }
        result.tokens = chosen_tokens.iter().cloned().collect();
    }

    // (Newest first.)
    let mut ordered: Vec<(String, Option<NaiveDate>)> = candidates.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
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

    // (A timeline checks all of its candidates at once.)
    if args.timeline
        && let Some(refused) = refuse(
            &[("history_candidates", ordered.len() as u64)],
            vec![format!(
                "A timeline checks all of its {} revisions at once",
                ordered.len()
            )],
            vec![
                "since:YYYY-MM-DD, for the recent history".to_string(),
                "a narrower path:".to_string(),
            ],
        )
    {
        return Err(refused);
    }

    // ## The page: each candidate revision's files, checked.
    let mut changes: Vec<Change> = vec![];
    let mut index = args.page;
    while index < ordered.len() {
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
        if commit.parent_count() > 1 {
            result.merges_skipped += 1;
            index += 1;
            continue;
        }
        let files = revision_files(
            source,
            &commit,
            &summary,
            &chosen_tokens,
            use_files,
            &path_ok,
            args.scope.as_deref(),
        );
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
                rev: rev.clone(),
                date: summary.iso_date.clone(),
                header: header_html(&summary.message),
                backout: !summary.backs_out.is_empty(),
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
    result.rows = changes
        .iter()
        .map(|change| HistoryGrepRow {
            rev: change.rev.clone(),
            date: change.date.chars().take(10).collect(),
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
    rev: String,
    /// Its ISO date.
    date: String,
    header: String,
    backout: bool,
    path: String,
    old_path: Option<String>,
    old: Found,
    new: Found,
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
    let mut changes: Vec<&Change> = changes.iter().collect();
    changes.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.rev.cmp(&b.rev)));
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
            date: first.date.chars().take(10).collect(),
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
            rev: rev.to_string(),
            date: date.to_string(),
            header: format!("Commit {}", rev),
            backout: false,
            path: path.to_string(),
            old_path: None,
            old: found(&re, old, false),
            new: found(&re, new, false),
        }
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
