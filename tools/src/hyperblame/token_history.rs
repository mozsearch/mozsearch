//! What a token's history says about its removals, for searches which find
//! few of its instances because it was removed or renamed (ex: firefox's
//! `BackstagePass`, which became `SystemGlobal`): its journal (see
//! `timeline_tokens`) has how many of its instances each commit added and
//! removed, from which come the commits which removed the most of them (its
//! last removals), and those commits' rev-summaries (see `rev_summaries`) say
//! which of its files and symbols they removed or renamed, and which tokens its
//! instances evolved into.  The query's default term notes it (see
//! `cmd_token_history` and `cmd_compile_results`), and `token-history:` shows
//! the journal.
//!
//! ## The last removals
//!
//! Going back from the newest record, the token had `relative` more instances
//! before each record than now: the removals so far minus the additions (the
//! journals count instances added and removed, not instances, which a history
//! window's start doesn't have).  Its last removals are the records back to the
//! first before which it had at least a fifth of the most it had (`peak`), so
//! that neither a long decline (ex: `nsIDOMNode`'s, from 2004 to 2018) nor a few
//! instances re-added and removed since (ex: `PRUnichar`'s, after its 2014
//! replacement by `char16_t`) make them something other than the removal which
//! made the token rare.
//!
//! ## What replaced it
//!
//! In the commits shown, the token's instances evolved into other tokens (their
//! symbols' `token_changes`: the token's `evolved_into` and the others'
//! `evolved_from`), symbols named by it evolved into others (their
//! `evolved_into`), its files were renamed (`moved_from`), and the biggest
//! removal's message names tokens whose instances evolved from others in the
//! commit (as their journals say), which is how commits too big for their
//! rev-summaries to list their files (see `MAX_REV_SUMMARY_FILES`; ex: `nsnull`
//! to `nullptr`) count.  Each
//! commit's votes count as much as the instances it removed.  The token was
//! renamed if its symbols or files were renamed to the top token and that has
//! at least half of the votes; otherwise the tokens with at least 15% of them
//! replaced it.
//!
//! ## Bounds
//!
//! The default term looks every token up, so this reads within bounds:
//! - The journal's head, and its segments, newest first, until `READ_BUDGET`
//!   bytes.  A removed token's recent segments are tiny, so its removal is
//!   reached soon (ex: `nsnull`'s 2012 removal of 25k instances, after 321 KB),
//!   and a common token's are 50-300 KB each, but it's alive, so the query
//!   doesn't show its history.
//! - Of the shown removals' summary records (see `consolidation`), the commit
//!   which removed the most, from their preds' heads.
//! - The rev-summaries of the shown commits (`MAX_SHOWN` and the last removal),
//!   up to `MAX_REV_SUMMARY_BYTES` each.
//! - The journals of `MAX_MESSAGE_WORDS` words of a message.
//!
//! On config8's firefox-shared history (2026-10-21), a prototype of this in
//! Python, over `git cat-file`, took 2-90 ms and read 10 KB to 2.4 MB of blobs
//! for tokens from `BackstagePass` to `nsIDOMWindow`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use chrono::{Datelike, NaiveDate, Weekday};
use git2::{Oid, Repository, Tree};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::segments;
use super::token_blame::TreeHistory;
use crate::file_format::history::rev_summaries::{RevSummaryRecord, rev_summary_path};
use crate::file_format::history::timeline_common::{ChangeKind, TokenDeltaDetails};
use crate::file_format::history::timeline_tokens::{
    TokenDeltaDetailRecord, TokenDeltaRecord, TokenDeltaSummaryRecord, is_trackable_token,
    token_timeline_path,
};

/// How many bytes of a token's journal to read for the query's note.
pub const READ_BUDGET: usize = 256 << 10;
/// How many bytes of a token's journal to read for its `token-history:` view.
pub const VIEW_READ_BUDGET: usize = 4 << 20;
/// The fewest instances removals need to have removed to be noted.
pub const MIN_REMOVED: u32 = 3;
/// How many of the biggest last removals are shown.
const MAX_SHOWN: usize = 3;
/// The biggest rev-summary read (big firefox commits' are a few MB; see
/// `MAX_REV_SUMMARY_FILES`).
const MAX_REV_SUMMARY_BYTES: u64 = 8 << 20;
/// The biggest journal head read to expand a summary record.
const MAX_PRED_BYTES: usize = 4 << 20;
/// How many words of a message are looked up, and how many with journals.
const MAX_MESSAGE_WORDS: usize = 8;
const MAX_MESSAGE_JOURNALS: usize = 4;
/// How many case variants of a token are looked at.
const MAX_VARIANTS: usize = 4;
/// How many of a commit's files and symbols are named.
const MAX_NAMES: usize = 5;
/// How many records of the view get their commits' headers.
const MAX_VIEW_HEADERS: usize = 200;
/// How much of a commit's header is shown.
const MAX_HEADER_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, Default)]
pub struct TokenHistoryOptions {
    /// Only the token's journal, not its case variants'.
    pub case_sensitive: bool,
    /// For the `token-history:` view: a bigger budget, and the records and
    /// the yearly totals.
    pub view: bool,
}

/// What a token's journal says about its removals; see the module docs.  (Its
/// fields all serialize, empty or not, since the templates compare them; see
/// token_history_summary.liquid.)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenHistory {
    /// The token whose journal this is: the one asked for, or the case
    /// variant of it whose last removals removed the most.
    pub token: String,
    /// The token's other case variants with journals.
    #[serde(default)]
    pub variants: Vec<String>,
    /// How many more instances the token had before its last removals than
    /// now, or 0 if it hasn't lost `MIN_REMOVED`.
    pub removed: u32,
    /// The most instances more than now it had, and the record (a date, or
    /// an ISO week) before which it had them.
    pub peak: u32,
    pub peak_when: String,
    /// The first of the last removals, and the last which removed at least a
    /// tenth of them.
    pub since: String,
    pub until: String,
    /// How many records (commits, and weeks of commits) of the last removals
    /// removed instances.
    pub removals: u32,
    /// The biggest of the last removals, and the last of them, newest first.
    #[serde(default)]
    pub commits: Vec<RemovalCommit>,
    /// The token it was renamed to, or the tokens which replaced it, by their
    /// share of its instances in the commits shown.
    #[serde(default)]
    pub renamed_to: Option<String>,
    #[serde(default)]
    pub replaced_by: Vec<Replacement>,
    /// The token's instances now, if its journal has them (see
    /// `TokenHeader::count`).
    #[serde(default)]
    pub count: Option<i64>,
    /// The newest record's date (or ISO week), and the oldest read's.
    pub latest: String,
    pub oldest: String,
    /// Whether the whole journal was read (else only back to `oldest`).
    pub read_all: bool,
    /// The view's: the instances added and removed each year, and the
    /// records read.
    #[serde(default)]
    pub years: Vec<YearTotals>,
    #[serde(default)]
    pub records: Vec<HistoryRecord>,
}

/// A commit of a token's last removals.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemovalCommit {
    pub rev: String,
    /// Its date (YYYY-MM-DD).
    pub date: String,
    /// The first line of its message (HTML, with its bugs linked), if its
    /// rev-summary was read.
    #[serde(default)]
    pub header: String,
    /// The token's instances it removed (including those which evolved into
    /// other tokens) and added (including those evolved from others).
    pub removed: u32,
    pub added: u32,
    /// If it's the commit of a summary record (the one which removed the
    /// most), the summarized week's.
    #[serde(default)]
    pub week: Option<WeekTotals>,
    /// Its files with the token in their names which it deleted or renamed.
    #[serde(default)]
    pub files: Vec<NameChange>,
    /// Its symbols named by the token which it removed or renamed (the first
    /// `MAX_NAMES`), and how many others.
    #[serde(default)]
    pub symbols: Vec<NameChange>,
    #[serde(default)]
    pub more_symbols: u32,
    /// Whether a later commit backed it out.
    #[serde(default)]
    pub backed_out: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeekTotals {
    /// The ISO week.
    pub week: String,
    pub commits: u32,
    pub removed: u32,
    pub added: u32,
}

/// A file or symbol removed (`to` None) or renamed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NameChange {
    pub name: String,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    pub token: String,
    pub percent: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct YearTotals {
    pub year: i32,
    pub added: u32,
    pub removed: u32,
    /// How many more instances than now the token had at the year's end.
    pub relative: i64,
}

/// A record of the view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    /// The commit's date, or the summarized ISO week.
    pub when: String,
    /// The commit, or the week's commits.
    pub revs: Vec<String>,
    /// The commit's header (HTML), for the first `MAX_VIEW_HEADERS`.
    #[serde(default)]
    pub header: String,
    /// The instances it added, and those evolved from other tokens (ex:
    /// renamed to this one), and removed, and those evolved into others.
    pub added: u32,
    pub evolved_from: u32,
    pub removed: u32,
    pub evolved_into: u32,
    /// How many more instances than now the token had after it.
    pub relative: i64,
}

pub(crate) fn delta(record: &TokenDeltaRecord) -> &TokenDeltaDetails {
    match record {
        TokenDeltaRecord::Detail(detail) => &detail.delta,
        TokenDeltaRecord::Summary(summary) => &summary.delta,
    }
}

/// The instances a delta removed (including those which evolved into other
/// tokens) and added.
pub(crate) fn gone(delta: &TokenDeltaDetails) -> u32 {
    delta.removed + delta.evolved_into
}

pub(crate) fn came(delta: &TokenDeltaDetails) -> u32 {
    delta.added + delta.evolved_from
}

pub(crate) fn week_label((year, week, _): (u16, u8, u8)) -> String {
    format!("{}-W{:02}", year, week)
}

/// A record's date, or its ISO week.
fn when(record: &TokenDeltaRecord) -> String {
    match record {
        TokenDeltaRecord::Detail(detail) => detail.desc.iso_date.chars().take(10).collect(),
        TokenDeltaRecord::Summary(summary) => week_label(summary.desc.iso_week_range),
    }
}

/// A record's date: a summary's week's last day.
fn date(record: &TokenDeltaRecord) -> Option<NaiveDate> {
    match record {
        TokenDeltaRecord::Detail(detail) => {
            NaiveDate::parse_from_str(detail.desc.iso_date.get(..10)?, "%Y-%m-%d").ok()
        }
        TokenDeltaRecord::Summary(summary) => {
            let (year, _, week) = summary.desc.iso_week_range;
            NaiveDate::from_isoywd_opt(year as i32, week as u32, Weekday::Sun)
        }
    }
}

/// The HTML of a commit message's first line (its first `MAX_HEADER_CHARS`,
/// for histories whose messages are one long line, ex: mozsearch's), with
/// its bugs linked.
pub fn header_html(message: &str) -> String {
    let header = message.lines().next().unwrap_or("");
    let header = match header.char_indices().nth(MAX_HEADER_CHARS) {
        Some((at, _)) => format!("{}…", &header[..at]),
        None => header.to_string(),
    };
    crate::links::linkify_commit_header(&header.replace('&', "&amp;").replace('<', "&lt;"))
}

pub(crate) fn read_blob(repo: &Repository, tree: &Tree, path: &str) -> Option<String> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    String::from_utf8(blob.content().to_vec()).ok()
}

fn parse_records(path: &str, text: &str) -> Result<Vec<TokenDeltaRecord>, String> {
    text.lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|e| format!("{}: bad record {:?}: {}", path, line, e))
        })
        .collect()
}

/// A token journal's records, newest first, as far as `budget` bytes go.
pub(crate) struct Journal {
    pub records: Vec<TokenDeltaRecord>,
    pub read_all: bool,
    /// The token's instances, if the journal's header has them (see
    /// `TokenHeader::count`).
    pub count: Option<i64>,
}

pub(crate) fn read_journal(
    repo: &Repository,
    tree: &Tree,
    token: &str,
    budget: usize,
) -> Result<Option<Journal>, String> {
    let path = token_timeline_path(token).to_string_lossy().into_owned();
    let Some(head) = read_blob(repo, tree, &path) else {
        return Ok(None);
    };
    let mut bytes = head.len();
    let mut records = parse_records(&path, &head)?;
    let mut read_all = true;
    for year in segments::segment_years(head.lines().next().unwrap_or("")) {
        if bytes >= budget {
            read_all = false;
            break;
        }
        let segment_path = segments::segment_path(Path::new(&path), year)
            .to_string_lossy()
            .into_owned();
        let Some(segment) = read_blob(repo, tree, &segment_path) else {
            continue;
        };
        bytes += segment.len();
        records.extend(parse_records(&segment_path, &segment)?);
    }
    let count = segments::header_count(head.lines().next().unwrap_or(""));
    Ok(Some(Journal {
        records,
        read_all,
        count,
    }))
}

/// How many more instances than now the token had before each record (see
/// the module docs).
fn relatives(records: &[TokenDeltaRecord]) -> Vec<i64> {
    let mut relative = 0i64;
    records
        .iter()
        .map(|record| {
            let delta = delta(record);
            relative += gone(delta) as i64 - came(delta) as i64;
            relative
        })
        .collect()
}

/// A token's last removals: the records up to `end` (inclusive).
#[derive(Debug, PartialEq)]
struct LastRemovals {
    end: usize,
    removed: u32,
    peak: u32,
    peak_index: usize,
}

fn last_removals(relative: &[i64]) -> Option<LastRemovals> {
    let (peak_index, peak) = relative.iter().enumerate().fold(
        None,
        |best: Option<(usize, i64)>, (i, &r)| match best {
            Some((_, b)) if b >= r => best,
            _ => Some((i, r)),
        },
    )?;
    if peak < MIN_REMOVED as i64 {
        return None;
    }
    let threshold = (MIN_REMOVED as i64).max((peak + 4) / 5);
    let end = relative.iter().position(|&r| r >= threshold)?;
    Some(LastRemovals {
        end,
        removed: relative[end] as u32,
        peak: peak as u32,
        peak_index,
    })
}

/// The records of the last removals to show: the `MAX_SHOWN` which removed the
/// most, and the last which removed at least a tenth of `removed`, newest
/// first; and the last.
fn shown_records(records: &[TokenDeltaRecord], removals: &LastRemovals) -> (Vec<usize>, usize) {
    let era = &records[..=removals.end];
    let mut biggest: Vec<usize> = (0..era.len())
        .filter(|&i| gone(delta(&era[i])) > 0)
        .collect();
    // (Stable, so the newer of equal removals first.)
    biggest.sort_by_key(|&i| std::cmp::Reverse(gone(delta(&era[i]))));
    biggest.truncate(MAX_SHOWN);
    let tenth = (removals.removed / 10).max(1);
    let last = (0..era.len())
        .find(|&i| gone(delta(&era[i])) >= tenth)
        .unwrap_or(removals.end);
    if !biggest.contains(&last) {
        biggest.push(last);
    }
    biggest.sort();
    (biggest, last)
}

/// The detail records of a summary's commits, from its preds' heads, which
/// `keep` keeps.
pub(crate) fn summary_details(
    repo: &Repository,
    summary: &TokenDeltaSummaryRecord,
    keep: &dyn Fn(&str) -> bool,
) -> Vec<TokenDeltaDetailRecord> {
    let wanted: Vec<&String> = summary
        .desc
        .source_revs
        .iter()
        .filter(|rev| keep(rev))
        .collect();
    let mut found: Vec<TokenDeltaDetailRecord> = vec![];
    for pred in &summary.desc.preds {
        let Some(tree) = Oid::from_str(&pred.timeline_rev)
            .ok()
            .and_then(|oid| repo.find_commit(oid).ok())
            .and_then(|commit| commit.tree().ok())
        else {
            continue;
        };
        let Some(text) = read_blob(repo, &tree, &pred.path) else {
            continue;
        };
        if text.len() > MAX_PRED_BYTES {
            continue;
        }
        for line in text.lines().skip(1) {
            if !wanted.iter().any(|rev| line.contains(rev.as_str())) {
                continue;
            }
            if let Ok(TokenDeltaRecord::Detail(detail)) = serde_json::from_str(line)
                && wanted.contains(&&detail.desc.source_rev)
                && !found
                    .iter()
                    .any(|f| f.desc.source_rev == detail.desc.source_rev)
            {
                found.push(detail);
            }
        }
    }
    found
}

pub(crate) fn read_rev_summary(history_path: &str, rev: &str) -> Option<RevSummaryRecord> {
    let path = Path::new(history_path)
        .join("rev-summaries")
        .join(rev_summary_path(rev));
    if fs::metadata(&path).ok()?.len() > MAX_REV_SUMMARY_BYTES {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// The parts of a symbol's pretty name.
fn name_parts(pretty: &str) -> Vec<&str> {
    pretty
        .split(['.', '#'])
        .flat_map(|p| p.split("::"))
        .collect()
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// What a file's renaming renamed `token` to: the new name's part in the
/// place of the token in the old name (ex: BackstagePass.h to SystemGlobal.h,
/// SystemGlobal).
fn renamed_part(token: &str, old: &str, new: &str) -> Option<String> {
    let (old, new) = (basename(old), basename(new));
    let at = old.to_ascii_lowercase().find(&token.to_ascii_lowercase())?;
    let (prefix, suffix) = (&old[..at], &old[at + token.len()..]);
    let part = new.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (part != token && is_trackable_token(part)).then(|| part.to_string())
}

/// What a commit says replaced `token`: votes by token, and the tokens its
/// symbols or files named by the token were renamed to.
fn commit_votes(
    token: &str,
    summary: &RevSummaryRecord,
) -> (HashMap<String, f64>, HashSet<String>) {
    let mut votes: HashMap<String, f64> = HashMap::new();
    let mut renamed = HashSet::new();
    for (path, file) in &summary.file_deltas {
        let file = &file.delta;
        if let Some(from) = &file.moved_from
            && !file.copied
            && let Some(part) = renamed_part(token, from, path)
        {
            *votes.entry(part.clone()).or_default() += 1.0;
            renamed.insert(part);
        }
        for (pretty, symbol) in &file.symbol_group.symbol_deltas {
            if let Some(into) = &symbol.evolved_into {
                let (old, new) = (name_parts(pretty), name_parts(into));
                if old.len() == new.len() {
                    for (o, n) in old.iter().zip(new.iter()) {
                        if *o == token && n != o && is_trackable_token(n) {
                            *votes.entry(n.to_string()).or_default() += 1.0;
                            renamed.insert(n.to_string());
                        }
                    }
                }
            }
            let Some(mine) = symbol.token_changes.get(token) else {
                continue;
            };
            if mine.evolved_into == 0 {
                continue;
            }
            for (other, delta) in &symbol.token_changes {
                if other != token && delta.evolved_from > 0 {
                    *votes.entry(other.clone()).or_default() +=
                        mine.evolved_into.min(delta.evolved_from) as f64;
                }
            }
        }
    }
    (votes, renamed)
}

lazy_static! {
    static ref WORD: Regex = Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*").unwrap();
}

/// Words of commit messages which aren't what replaced a token.
const STOP_WORDS: &[&str] = &[
    "and",
    "as",
    "backed",
    "bug",
    "by",
    "changeset",
    "convert",
    "converts",
    "for",
    "from",
    "into",
    "move",
    "moves",
    "out",
    "part",
    "patch",
    "remove",
    "removes",
    "rename",
    "renames",
    "replace",
    "replaces",
    "switch",
    "switches",
    "the",
    "to",
    "use",
    "uses",
    "using",
    "with",
];

/// The tokens which `message` names and whose journals say instances of them
/// evolved from other tokens in its commit `rev` (of `iso_date`), by how many.
/// (Not those only added: a commit adding code adds common words, ex: a
/// message's "code".)
fn message_votes(
    repo: &Repository,
    tree: &Tree,
    token: &str,
    rev: &str,
    message: &str,
    iso_date: &str,
) -> HashMap<String, f64> {
    let mut votes = HashMap::new();
    let header = message.lines().next().unwrap_or("");
    let year = NaiveDate::parse_from_str(iso_date.get(..10).unwrap_or(""), "%Y-%m-%d")
        .map(|date| date.iso_week().year())
        .ok();
    let mut seen = HashSet::new();
    let mut journals = 0;
    let words = WORD
        .find_iter(header)
        .map(|m| m.as_str())
        .filter(|word| {
            !word.eq_ignore_ascii_case(token)
                && word.len() >= 3
                && is_trackable_token(word)
                && !STOP_WORDS.contains(&word.to_ascii_lowercase().as_str())
        })
        .filter(|word| seen.insert(*word))
        .take(MAX_MESSAGE_WORDS);
    for word in words {
        if journals >= MAX_MESSAGE_JOURNALS {
            break;
        }
        let path = token_timeline_path(word).to_string_lossy().into_owned();
        let Some(head) = read_blob(repo, tree, &path) else {
            continue;
        };
        journals += 1;
        // The commit's record: in the head, or in the segment of its year.
        let mut texts = vec![head];
        if !texts[0].contains(rev)
            && let Some(year) = year
            && segments::segment_years(texts[0].lines().next().unwrap_or("")).contains(&year)
        {
            let segment_path = segments::segment_path(Path::new(&path), year);
            texts.extend(read_blob(repo, tree, &segment_path.to_string_lossy()));
        }
        let evolved = texts
            .iter()
            .flat_map(|text| text.lines().skip(1))
            .filter(|line| line.contains(rev))
            .filter_map(|line| serde_json::from_str::<TokenDeltaRecord>(line).ok())
            .map(|record| match record {
                TokenDeltaRecord::Detail(detail) => detail.delta.evolved_from,
                TokenDeltaRecord::Summary(summary) => {
                    summary_details(repo, &summary, &|r| r == rev)
                        .first()
                        .map_or(0, |detail| detail.delta.evolved_from)
                }
            })
            .sum::<u32>();
        if evolved > 0 {
            votes.insert(word.to_string(), evolved as f64);
        }
    }
    votes
}

/// The files and symbols named by `token` which a commit removed or renamed.
fn name_changes(token: &str, summary: &RevSummaryRecord) -> (Vec<NameChange>, Vec<NameChange>) {
    let lower = token.to_ascii_lowercase();
    let mut files = vec![];
    let mut symbols = vec![];
    for (path, file) in &summary.file_deltas {
        let file = &file.delta;
        match (&file.change, &file.moved_from) {
            (ChangeKind::Removed, _) if basename(path).to_ascii_lowercase().contains(&lower) => {
                files.push(NameChange {
                    name: path.clone(),
                    to: None,
                });
            }
            (ChangeKind::Evolved, Some(from))
                if !file.copied && basename(from).to_ascii_lowercase().contains(&lower) =>
            {
                files.push(NameChange {
                    name: from.clone(),
                    to: Some(path.clone()),
                });
            }
            _ => {}
        }
        for (pretty, symbol) in &file.symbol_group.symbol_deltas {
            if symbol.change == ChangeKind::Removed && name_parts(pretty).contains(&token) {
                symbols.push(NameChange {
                    name: pretty.clone(),
                    to: symbol.evolved_into.clone(),
                });
            }
        }
    }
    // (The shortest names first: a class before its members.)
    symbols.sort_by(|a, b| {
        (name_parts(&a.name).len(), &a.name).cmp(&(name_parts(&b.name).len(), &b.name))
    });
    symbols.dedup();
    (files, symbols)
}

/// The analysis of a journal, before the commits are looked up.
struct Analysis {
    journal: Journal,
    relative: Vec<i64>,
    removals: Option<LastRemovals>,
}

fn analyze(journal: Journal) -> Analysis {
    let relative = relatives(&journal.records);
    let removals = last_removals(&relative);
    Analysis {
        journal,
        relative,
        removals,
    }
}

/// The journals' tokens of `token`'s case variants, the token first, then by
/// their journals' sizes, biggest first.
pub(crate) fn case_variants(repo: &Repository, tree: &Tree, token: &str) -> Vec<String> {
    let path = token_timeline_path(token);
    let lower = token.to_lowercase();
    let mut variants: Vec<(usize, String)> = vec![];
    if let Some(dir) = path.parent()
        && let Ok(entry) = tree.get_path(dir)
        && let Ok(dir_tree) = repo.find_tree(entry.id())
    {
        let odb = repo.odb().ok();
        for entry in dir_tree.iter() {
            let Some(variant) = entry
                .name()
                .ok()
                .and_then(|name| name.strip_suffix(".ndjson"))
            else {
                continue;
            };
            if variant.to_lowercase() != lower {
                continue;
            }
            let size = odb
                .as_ref()
                .and_then(|odb| odb.read_header(entry.id()).ok())
                .map_or(0, |(size, _)| size);
            variants.push((
                if variant == token { usize::MAX } else { size },
                variant.to_string(),
            ));
        }
    }
    variants.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    variants.into_iter().map(|(_, variant)| variant).collect()
}

/// `token`'s history (see the module docs), or None if it has no journal (or
/// isn't a token the history tracks).  `header` gives the header (HTML) of a
/// commit, for the view's records.
pub fn token_history(
    history: &TreeHistory,
    token: &str,
    opts: TokenHistoryOptions,
    header: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<TokenHistory>, String> {
    if !is_trackable_token(token) {
        return Ok(None);
    }
    let repo: &Repository = &history.timeline;
    let Some(head) = history.head_timeline_commit() else {
        return Ok(None);
    };
    let tree = head.tree().map_err(|e| e.to_string())?;
    let budget = if opts.view {
        VIEW_READ_BUDGET
    } else {
        READ_BUDGET
    };

    // The token's journal, or its case variant's whose last removals removed
    // the most (the token's if it has removals).
    let variants = case_variants(repo, &tree, token);
    let mut best: Option<(String, Analysis)> = None;
    for variant in variants.iter().take(MAX_VARIANTS) {
        if opts.case_sensitive && variant != token {
            continue;
        }
        let Some(journal) = read_journal(repo, &tree, variant, budget)? else {
            continue;
        };
        let analysis = analyze(journal);
        let removed = |a: &Analysis| a.removals.as_ref().map_or(0, |r| r.removed);
        if best
            .as_ref()
            .is_none_or(|(_, b)| removed(&analysis) > removed(b))
        {
            best = Some((variant.clone(), analysis));
        }
        if variant == token && best.as_ref().is_some_and(|(_, b)| removed(b) > 0) {
            break;
        }
    }
    let Some((token, analysis)) = best else {
        return Ok(None);
    };
    let records = &analysis.journal.records;
    let mut result = TokenHistory {
        variants: variants
            .iter()
            .filter(|variant| **variant != token)
            .cloned()
            .collect(),
        latest: records.first().map(when).unwrap_or_default(),
        oldest: records.last().map(when).unwrap_or_default(),
        read_all: analysis.journal.read_all,
        count: analysis.journal.count,
        ..Default::default()
    };

    if let Some(removals) = &analysis.removals {
        result.removed = removals.removed;
        result.peak = removals.peak;
        result.peak_when = when(&records[removals.peak_index]);
        result.since = when(&records[removals.end]);
        result.removals = records[..=removals.end]
            .iter()
            .filter(|record| gone(delta(record)) > 0)
            .count() as u32;
        let (shown, last) = shown_records(records, removals);
        result.until = when(&records[last]);

        // The commits, and what they say replaced the token.
        let biggest = shown
            .iter()
            .copied()
            .max_by_key(|&i| (gone(delta(&records[i])), std::cmp::Reverse(i)));
        let mut votes: HashMap<String, f64> = HashMap::new();
        let mut renamed: HashSet<String> = HashSet::new();
        for &i in &shown {
            let (detail, week) = match &records[i] {
                TokenDeltaRecord::Detail(detail) => (Some(detail.clone()), None),
                TokenDeltaRecord::Summary(summary) => (
                    summary_details(repo, summary, &|_| true)
                        .into_iter()
                        .max_by_key(|detail| gone(&detail.delta)),
                    Some(WeekTotals {
                        week: week_label(summary.desc.iso_week_range),
                        commits: summary.desc.source_revs.len() as u32,
                        removed: gone(&summary.delta),
                        added: came(&summary.delta),
                    }),
                ),
            };
            let Some(detail) = detail else {
                continue;
            };
            let rev = &detail.desc.source_rev;
            let mut commit = RemovalCommit {
                rev: rev.clone(),
                date: detail.desc.iso_date.chars().take(10).collect(),
                removed: gone(&detail.delta),
                added: came(&detail.delta),
                week,
                ..Default::default()
            };
            if let Some(summary) = read_rev_summary(&history.path, rev) {
                commit.header = header_html(&summary.message);
                commit.backed_out = !summary.backed_out_by.is_empty();
                let (files, symbols) = name_changes(&token, &summary);
                commit.files = files.into_iter().take(MAX_NAMES).collect();
                commit.more_symbols = symbols.len().saturating_sub(MAX_NAMES) as u32;
                commit.symbols = symbols.into_iter().take(MAX_NAMES).collect();

                let (mut commit_votes, commit_renamed) = commit_votes(&token, &summary);
                if Some(i) == biggest {
                    let words = message_votes(
                        repo,
                        &tree,
                        &token,
                        rev,
                        &summary.message,
                        &summary.iso_date,
                    );
                    for (word, n) in words {
                        *commit_votes.entry(word).or_default() += n;
                    }
                }
                let total: f64 = commit_votes.values().sum();
                if total > 0.0 && !commit.backed_out {
                    for (word, n) in commit_votes {
                        *votes.entry(word).or_default() += n / total * commit.removed as f64;
                    }
                    renamed.extend(commit_renamed);
                }
            }
            result.commits.push(commit);
        }

        let total: f64 = votes.values().sum();
        let mut ranked: Vec<(String, f64)> = votes.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        if let Some((top, n)) = ranked.first()
            && renamed.contains(top)
            && *n >= total * 0.5
        {
            result.renamed_to = Some(top.clone());
        }
        result.replaced_by = ranked
            .iter()
            .take(3)
            .filter(|(_, n)| *n >= total * 0.15)
            .map(|(word, n)| Replacement {
                token: word.clone(),
                percent: (n / total * 100.0).round() as u32,
            })
            .collect();
    }

    if opts.view {
        let mut years: BTreeMap<i32, YearTotals> = BTreeMap::new();
        let mut headers = 0;
        for (record, &relative) in records.iter().zip(analysis.relative.iter()) {
            let delta = delta(record);
            // (After the record: before it, less what it removed.)
            let after = relative - gone(delta) as i64 + came(delta) as i64;
            if let Some(date) = date(record) {
                let year = years.entry(date.year()).or_insert_with(|| YearTotals {
                    year: date.year(),
                    relative: after,
                    ..Default::default()
                });
                year.added += came(delta);
                year.removed += gone(delta);
            }
            let (revs, record_header) = match record {
                TokenDeltaRecord::Detail(detail) => {
                    let rev = detail.desc.source_rev.clone();
                    let record_header = if headers < MAX_VIEW_HEADERS {
                        headers += 1;
                        header(&rev).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    (vec![rev], record_header)
                }
                TokenDeltaRecord::Summary(summary) => {
                    (summary.desc.source_revs.clone(), String::new())
                }
            };
            result.records.push(HistoryRecord {
                when: when(record),
                revs,
                header: record_header,
                added: delta.added,
                evolved_from: delta.evolved_from,
                removed: delta.removed,
                evolved_into: delta.evolved_into,
                relative: after,
            });
        }
        result.years = years.into_values().collect();
    }

    result.token = token;
    Ok(Some(result))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const RENAME: &str = "0801a75c76c3da8d8a2c2f9f9af7def4ac953cb7";
    const READD: &str = "4424263193cb9ba03d1bf8b07c222d7a2d80f987";

    #[test]
    fn test_last_removals() {
        // (BackstagePass: renamed in one commit, and one instance re-added
        // since.)
        assert_eq!(
            last_removals(&[-1, 37, 36, 35, 43, -5]),
            Some(LastRemovals {
                end: 1,
                removed: 37,
                peak: 43,
                peak_index: 4,
            })
        );
        // (nsnull: a few instances removed since its removal, which was most
        // of what it had.)
        assert_eq!(
            last_removals(&[1, 3, 1, -1, 25000, 25714, 20000]).map(|r| (r.end, r.removed)),
            Some((4, 25000))
        );
        // (Too few to note.)
        assert_eq!(last_removals(&[1, 2, -4]), None);
        assert_eq!(last_removals(&[]), None);
    }

    #[test]
    fn test_header_html() {
        assert_eq!(
            header_html("Bug 12345 - Fix <b> & more\n\nDetails"),
            r#"Bug <a href="https://bugzilla.mozilla.org/show_bug.cgi?id=12345">12345</a> - Fix &lt;b> &amp; more"#
        );
        let long = format!("No bug - {}", "é".repeat(300));
        assert_eq!(header_html(&long).chars().count(), MAX_HEADER_CHARS + 1);
        assert!(header_html(&long).ends_with('…'));
    }

    #[test]
    fn test_renamed_part() {
        assert_eq!(
            renamed_part(
                "BackstagePass",
                "js/xpconnect/src/BackstagePass.h",
                "js/xpconnect/src/SystemGlobal.h"
            ),
            Some("SystemGlobal".to_string())
        );
        assert_eq!(
            renamed_part("Widget", "dom/WidgetUtils.cpp", "dom/GadgetUtils.cpp"),
            Some("Gadget".to_string())
        );
        // (Not renamed: only moved, or renamed otherwise.)
        assert_eq!(renamed_part("Widget", "a/Widget.h", "b/Widget.h"), None);
        assert_eq!(renamed_part("Widget", "a/Widget.h", "a/Gadget.cpp"), None);
        assert_eq!(renamed_part("Widget", "a/Gadget.h", "a/Gizmo.h"), None);
    }

    pub(crate) fn detail(rev: &str, date: &str, delta: &str) -> String {
        format!(
            r#"{{"type":"Detail","source_rev":"{}","syntax_rev":"0000000000000000000000000000000000000000","iso_date":"{}T12:00:00Z"{}}}"#,
            rev, date, delta
        )
    }

    pub(crate) fn journal(header: &str, records: &[String]) -> String {
        let mut text = header.to_string();
        for record in records {
            text.push('\n');
            text.push_str(record);
        }
        text
    }

    pub(crate) fn token_path(token: &str) -> String {
        token_timeline_path(token).to_string_lossy().into_owned()
    }

    /// Commit `files` (path, contents) to `repo`'s HEAD, on top of `parent`.
    pub(crate) fn commit_files(
        repo: &Repository,
        parent: Option<Oid>,
        files: &[(String, String)],
    ) -> Oid {
        let mut index = git2::Index::new().unwrap();
        for (path, contents) in files {
            let blob = repo.blob(contents.as_bytes()).unwrap();
            index
                .add(&git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: 0o100644,
                    uid: 0,
                    gid: 0,
                    file_size: contents.len() as u32,
                    id: blob,
                    flags: path.len() as u16,
                    flags_extended: 0,
                    path: path.as_bytes().to_vec(),
                })
                .unwrap();
        }
        let tree = repo.find_tree(index.write_tree_to(repo).unwrap()).unwrap();
        let sig = git2::Signature::new("test", "test@example.com", &git2::Time::new(0, 0)).unwrap();
        let parents: Vec<git2::Commit> = parent
            .into_iter()
            .map(|p| repo.find_commit(p).unwrap())
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "timeline", &tree, &parent_refs)
            .unwrap()
    }

    /// A history with BackstagePass's (trimmed) journal and its rename's
    /// rev-summary, as firefox's has them, a case variant, a token removed by
    /// commits summarized in a week (Gadget), and one whose removal is in a
    /// segment (Doohickey).
    pub(crate) fn test_history(name: &str) -> (std::path::PathBuf, TreeHistory) {
        let dir =
            std::env::temp_dir().join(format!("token-history-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Repository::init(dir.join("syntax")).unwrap();
        let repo = Repository::init(dir.join("timeline")).unwrap();

        // The week's commits' details, which the summary's pred has.
        let gadget = token_path("Gadget");
        let c0 = commit_files(
            &repo,
            None,
            &[(
                gadget.clone(),
                journal(
                    "{}",
                    &[
                        detail(&"b".repeat(40), "2020-03-04", r#","removed":15"#),
                        detail(&"a".repeat(40), "2020-03-03", r#","removed":5,"added":1"#),
                        detail(&"9".repeat(40), "2019-01-01", r#","added":30"#),
                    ],
                ),
            )],
        );
        let files = vec![
            (
                token_path("BackstagePass"),
                journal(
                    "{}",
                    &[
                        detail(READD, "2025-10-15", r#","added":1"#),
                        detail(RENAME, "2024-12-10", r#","evolved_into":33,"removed":5"#),
                        r#"{"type":"Summary","source_revs":["e8aba69e01f24f54c656c9c8cb010c56e068ae39","b5d69ff02ca73ef969ff58a29a20f84f9064a0c3"],"preds":[],"iso_week_range":[2022,48,48],"added":3,"removed":2}"#.to_string(),
                        detail(&"6".repeat(40), "2018-08-10", r#","evolved_from":1"#),
                        detail(&"5".repeat(40), "2017-01-01", r#","added":40"#),
                    ],
                ),
            ),
            (
                token_path("backstagePass"),
                journal("{}", &[detail(&"7".repeat(40), "2019-05-02", r#","added":2"#)]),
            ),
            (
                token_path("SystemGlobal"),
                journal("{}", &[detail(RENAME, "2024-12-10", r#","evolved_from":33"#)]),
            ),
            (
                gadget.clone(),
                journal(
                    "{}",
                    &[format!(
                        r#"{{"type":"Summary","source_revs":["{}","{}"],"preds":[{{"timeline_rev":"{}","path":"{}"}}],"iso_week_range":[2020,10,10],"added":1,"removed":20}}"#,
                        "b".repeat(40),
                        "a".repeat(40),
                        c0,
                        gadget
                    ),
                    detail(&"9".repeat(40), "2019-01-01", r#","added":30"#)],
                ),
            ),
            (
                token_path("Doohickey"),
                journal(r#"{"segments":[2012]}"#, &[]),
            ),
            // (A big journal's head, with its last two years, and its older
            // years' segments; and a case variant.)
            (
                token_path("Widget"),
                journal(
                    r#"{"segments":[2024,2023]}"#,
                    &[detail(&"e".repeat(40), "2025-03-05", r#","added":2"#)],
                ),
            ),
            (
                format!("{}.d/2024.ndjson", token_path("Widget")),
                journal(
                    "{}",
                    &[detail(&"f".repeat(40), "2024-06-05", r#","removed":10"#)],
                ),
            ),
            (
                format!("{}.d/2023.ndjson", token_path("Widget")),
                journal(
                    "{}",
                    &[detail(&"1".repeat(40), "2023-02-01", r#","added":50"#)],
                ),
            ),
            (
                token_path("widget"),
                journal("{}", &[detail(&"2".repeat(40), "2024-09-04", r#","added":1"#)]),
            ),
            // (A journal with its token's instances, as exact histories have.)
            (
                token_path("Gizmo"),
                journal(
                    r#"{"count":5}"#,
                    &[
                        detail(&"3".repeat(40), "2026-02-04", r#","removed":4"#),
                        detail(&"4".repeat(40), "2025-02-05", r#","added":9"#),
                    ],
                ),
            ),
            (
                format!("{}.d/2012.ndjson", token_path("Doohickey")),
                journal(
                    "{}",
                    &[
                        detail(&"c".repeat(40), "2012-07-30", r#","removed":500"#),
                        detail(&"d".repeat(40), "2001-01-01", r#","added":500"#),
                    ],
                ),
            ),
        ];
        commit_files(&repo, Some(c0), &files);

        let rev_summary = format!(
            r#"{{"source_rev":"{rev}","syntax_rev":"x","timeline_rev":"y",
            "message":"Bug 1767888 - Rename BackstagePass to SystemGlobal. r=mccr8\n\nDifferential Revision: https://phabricator.services.mozilla.com/D230155",
            "iso_date":"2024-12-10T16:08:58Z","unmapped_author":"a",
            "file_deltas":{{
              "dom/bindings/BindingUtils.cpp":{{"change":"changed","symbol_deltas":{{
                "dom::GlobalObject::GetAsSupports":{{"change":"changed","token_changes":{{"BackstagePass":{{"evolved_into":1}},"SystemGlobal":{{"evolved_from":1}}}}}}}}}},
              "ipc/testshell/XPCShellEnvironment.cpp":{{"change":"changed","symbol_deltas":{{
                "XPCShellEnvironment::Init":{{"change":"changed","token_changes":{{"BackstagePass":{{"evolved_into":1}},"SystemGlobal":{{"evolved_from":1}},"systemGlobal":{{"evolved_from":3}}}}}}}}}},
              "js/xpconnect/src/SystemGlobal.h":{{"change":"evolved","moved_from":"js/xpconnect/src/BackstagePass.h","symbol_deltas":{{
                "BackstagePass":{{"change":"removed","evolved_into":"SystemGlobal","token_changes":{{"BackstagePass":{{"evolved_into":2}}}}}},
                "BackstagePass::GetPrincipal":{{"change":"removed","evolved_into":"SystemGlobal::GetPrincipal","token_changes":{{}}}},
                "SystemGlobal":{{"change":"evolved","evolved_from":"BackstagePass","token_changes":{{"SystemGlobal":{{"evolved_from":2}}}}}}}}}}
            }}}}"#,
            rev = RENAME
        );
        let path = dir.join("rev-summaries").join(rev_summary_path(RENAME));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, rev_summary).unwrap();

        let history = TreeHistory::open(dir.to_str().unwrap(), None).unwrap();
        (dir, history)
    }

    fn lookup(
        history: &TreeHistory,
        token: &str,
        opts: TokenHistoryOptions,
    ) -> Option<TokenHistory> {
        token_history(history, token, opts, &|rev| {
            Some(format!("header of {}", &rev[..4]))
        })
        .unwrap()
    }

    #[test]
    fn test_token_history() {
        let (dir, history) = test_history("rename");

        let found = lookup(&history, "BackstagePass", Default::default()).unwrap();
        assert_eq!(found.token, "BackstagePass");
        assert_eq!(found.variants, vec!["backstagePass".to_string()]);
        assert_eq!((found.removed, found.peak), (37, 37));
        assert_eq!(
            (found.since.as_str(), found.until.as_str()),
            ("2024-12-10", "2024-12-10")
        );
        assert_eq!(
            (found.latest.as_str(), found.oldest.as_str()),
            ("2025-10-15", "2017-01-01")
        );
        assert!(found.read_all);
        assert_eq!(found.commits.len(), 1);
        let commit = &found.commits[0];
        assert_eq!(
            (commit.rev.as_str(), commit.removed, commit.added),
            (RENAME, 38, 0)
        );
        assert_eq!(
            commit.header,
            r#"Bug <a href="https://bugzilla.mozilla.org/show_bug.cgi?id=1767888">1767888</a> - Rename BackstagePass to SystemGlobal. r=mccr8"#
        );
        assert_eq!(
            commit.files,
            vec![NameChange {
                name: "js/xpconnect/src/BackstagePass.h".to_string(),
                to: Some("js/xpconnect/src/SystemGlobal.h".to_string()),
            }]
        );
        assert_eq!(
            commit
                .symbols
                .iter()
                .map(|s| (s.name.as_str(), s.to.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("BackstagePass", Some("SystemGlobal")),
                (
                    "BackstagePass::GetPrincipal",
                    Some("SystemGlobal::GetPrincipal")
                ),
            ]
        );
        // (Its symbol and file were renamed to it, and its instances evolved
        // into it, as the message, and SystemGlobal's journal, say.)
        assert_eq!(found.renamed_to.as_deref(), Some("SystemGlobal"));
        assert_eq!(
            found.replaced_by,
            vec![Replacement {
                token: "SystemGlobal".to_string(),
                percent: 97,
            }]
        );
        assert!(found.records.is_empty() && found.years.is_empty());

        // (Another case's search finds the variant with the removals, unless
        // it's case-sensitive.)
        let found = lookup(&history, "backstagepass", Default::default()).unwrap();
        assert_eq!(found.token, "BackstagePass");
        assert_eq!(found.variants, vec!["backstagePass".to_string()]);
        let case_sensitive = TokenHistoryOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(lookup(&history, "backstagepass", case_sensitive), None);
        let found = lookup(&history, "backstagePass", case_sensitive).unwrap();
        assert_eq!((found.token.as_str(), found.removed), ("backstagePass", 0));

        // (A journal with a count has the instances there are.)
        let found = lookup(&history, "Gizmo", Default::default()).unwrap();
        assert_eq!((found.count, found.removed), (Some(5), 4));
        assert_eq!(
            lookup(&history, "BackstagePass", Default::default())
                .unwrap()
                .count,
            None
        );

        // (Not a token, or without a journal.)
        assert_eq!(lookup(&history, "Backstage Pass", Default::default()), None);
        assert_eq!(lookup(&history, "Thingamajig", Default::default()), None);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_token_history_summaries_and_segments() {
        let (dir, history) = test_history("weeks");

        // (The week's commit which removed the most, from the summary's pred.)
        let found = lookup(&history, "Gadget", Default::default()).unwrap();
        assert_eq!((found.removed, found.since.as_str()), (19, "2020-W10"));
        assert_eq!(found.commits.len(), 1);
        let commit = &found.commits[0];
        assert_eq!(
            (commit.rev.as_str(), commit.removed),
            ("b".repeat(40).as_str(), 15)
        );
        assert_eq!(
            commit.week,
            Some(WeekTotals {
                week: "2020-W10".to_string(),
                commits: 2,
                removed: 20,
                added: 1,
            })
        );
        // (Without a rev-summary, nothing says what replaced it.)
        assert_eq!(commit.header, "");
        assert_eq!((found.renamed_to, found.replaced_by), (None, vec![]));

        // (Its removal is in a segment.)
        let found = lookup(&history, "Doohickey", Default::default()).unwrap();
        assert_eq!((found.removed, found.since.as_str()), (500, "2012-07-30"));
        assert!(found.read_all);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_token_history_view() {
        let (dir, history) = test_history("view");
        let view = TokenHistoryOptions {
            view: true,
            ..Default::default()
        };
        let found = lookup(&history, "BackstagePass", view).unwrap();
        assert_eq!(
            found
                .records
                .iter()
                .map(|r| (r.when.as_str(), r.relative, r.header.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("2025-10-15", 0, "header of 4424"),
                ("2024-12-10", -1, "header of 0801"),
                ("2022-W48", 37, ""),
                ("2018-08-10", 36, "header of 6666"),
                ("2017-01-01", 35, "header of 5555"),
            ]
        );
        assert_eq!(found.records[2].revs.len(), 2);
        assert_eq!(
            found
                .years
                .iter()
                .map(|y| (y.year, y.added, y.removed, y.relative))
                .collect::<Vec<_>>(),
            vec![
                (2017, 40, 0, 35),
                (2018, 1, 0, 36),
                (2022, 3, 2, 37),
                (2024, 0, 38, -1),
                (2025, 1, 0, 0),
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
