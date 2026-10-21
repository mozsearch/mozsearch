//! Tokens' timelines, for the `token-timeline:` view (see `cmd_token_timeline`
//! and static/js/token-timeline.js): a line a token (and its case variants,
//! unless case-sensitive), of how many more of its instances than now the tree
//! had each week, from its journals (see `timeline_tokens`), with markers at
//! the weeks which changed the lines the most, which say what the commits
//! changing them the most were, and which other tokens they changed the most.
//!
//! ## Weeks
//!
//! The journals' summary records are weekly (see `consolidation`), and their
//! detail records (the last weeks', and weeks with one record) are aggregated
//! by ISO week, as the case variants' are, so that a line has a point a week at
//! most (ex: firefox-shared's history since 1998 is ~1,500 weeks).  As in
//! `token_history`, a line is relative to now (removals minus additions, newest
//! first): journals count instances added and removed, and the oldest history
//! (firefox's CVS conversion) doesn't sum to the instances there are.
//!
//! ## Markers
//!
//! A week gets a marker if it changed a line by at least `MARKER_SHARE` of the
//! line's range (and `MIN_MARKER_CHANGE` instances), the `MAX_MARKERS` weeks
//! which changed a line the most, by that share.  Only those weeks' summary
//! records are expanded (from their preds' heads), for their commits, of which
//! the `MAX_MARKER_COMMITS` which changed the lines the most (by their net
//! changes, over the case variants) get their rev-summaries read: their
//! headers, and the other tokens they changed the most (case variants
//! together; the `MAX_OTHER_TOKENS` with the most instances added and removed
//! for how common they are, see `other_tokens`), whose timelines the view links
//! to.
//!
//! ## Bounds
//!
//! At most `MAX_TOKENS` tokens (case-insensitively distinct), `MAX_VARIANTS`
//! case variants each, reading their journals' heads and their yearly segments
//! (big journals' heads only keep their last two ISO years; see `segments`),
//! newest first, within `BUDGET` bytes in all (`MAX_TOKEN_BUDGET` a token,
//! shared by its variants).  A token with a variant whose segments weren't all
//! read has its line since the oldest week read of such variants (its
//! `read_all` is false).  `MAX_EXPANSIONS` summaries expanded, and the markers'
//! rev-summaries (see `token_history::read_rev_summary`).

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{NaiveDate, Weekday};
use git2::{Repository, Tree};
use serde::{Deserialize, Serialize};

use super::consolidation::iso_week;
use super::token_blame::TreeHistory;
use super::token_history::{
    came, case_variants, delta, gone, header_html, read_journal, read_rev_summary, summary_details,
};
use crate::file_format::history::timeline_common::TokenDeltaDetails;
use crate::file_format::history::timeline_tokens::{
    TokenDeltaRecord, is_trackable_token, token_timeline_path,
};

/// How many tokens (case-insensitively distinct) a timeline has at most.
pub const MAX_TOKENS: usize = 16;
/// How many bytes of journals a timeline reads, in all and a token.
pub const BUDGET: usize = 16 << 20;
const MAX_TOKEN_BUDGET: usize = 4 << 20;
/// How many case variants of a token are aggregated.
const MAX_VARIANTS: usize = 8;
/// How many markers a timeline has, and how much a week must change a line for
/// one.
const MAX_MARKERS: usize = 12;
const MARKER_SHARE: f64 = 0.05;
const MIN_MARKER_CHANGE: u32 = 3;
/// How many of a marker's commits are described, and with how many other tokens.
const MAX_MARKER_COMMITS: usize = 3;
const MAX_OTHER_TOKENS: usize = 10;
/// How many of a commit's most changed other tokens are ranked by rarity.
const MAX_RANKED_OTHERS: usize = 40;
/// How many summary records are expanded for the markers' commits.
const MAX_EXPANSIONS: usize = 16;

#[derive(Clone, Copy, Debug, Default)]
pub struct TokenTimelineOptions {
    /// Only the tokens' journals, not their case variants'.
    pub case_sensitive: bool,
}

/// Tokens' timelines; see the module docs.  (Its fields all serialize, empty
/// or not, for token-timeline.js.)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenTimeline {
    /// A line a token, in the order asked.
    pub series: Vec<TimelineSeries>,
    /// The weeks which changed the lines the most, oldest first.
    pub markers: Vec<TimelineMarker>,
    /// The tokens asked for beyond `MAX_TOKENS`, left out.
    #[serde(default)]
    pub dropped: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineSeries {
    /// The token, as asked.
    pub token: String,
    /// The journals' tokens (it and its case variants) aggregated, with the
    /// instances each added and removed (in the weeks read).
    #[serde(default)]
    pub variants: Vec<VariantTotals>,
    /// The weeks which changed it, oldest first.
    #[serde(default)]
    pub weeks: Vec<WeekPoint>,
    /// How many more instances than now it had before its oldest week.
    pub start: i64,
    /// Whether all of its journals were read (else its line is since its
    /// oldest week).
    pub read_all: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VariantTotals {
    pub token: String,
    pub added: u32,
    pub removed: u32,
}

/// A week of a line: its ISO week ("2024-W50") and Monday, the instances it
/// added and removed (including those evolved from and into other tokens), and
/// how many more instances than now the token had at its end.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeekPoint {
    pub week: String,
    pub date: String,
    pub added: u32,
    pub removed: u32,
    pub relative: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineMarker {
    pub week: String,
    pub date: String,
    /// The lines' changes that week.
    #[serde(default)]
    pub changes: Vec<SeriesChange>,
    /// The week's commits which changed the lines the most, and how many
    /// commits changed them that week.
    #[serde(default)]
    pub commits: Vec<MarkerCommit>,
    pub week_commits: u32,
}

/// A change of a line (by its index in `series`), or of one of its tokens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SeriesChange {
    pub series: usize,
    pub token: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MarkerCommit {
    pub rev: String,
    /// Its date (YYYY-MM-DD).
    pub date: String,
    /// Its message's first line (HTML), if its rev-summary was read.
    #[serde(default)]
    pub header: String,
    /// Its changes of the lines' tokens, by journal token.
    #[serde(default)]
    pub changes: Vec<SeriesChange>,
    /// The other tokens it changed the most, if its rev-summary lists its
    /// files' changes (a commit changing too many files only has totals).
    #[serde(default)]
    pub others: Vec<OtherToken>,
    pub others_known: bool,
}

/// A token a commit changed, with its case variants (unless case-sensitive).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OtherToken {
    /// The variant it changed the most.
    pub token: String,
    #[serde(default)]
    pub variants: Vec<String>,
    pub added: u32,
    pub removed: u32,
}

type Week = (i32, u32);

fn week_label((year, week): Week) -> String {
    format!("{}-W{:02}", year, week)
}

fn week_monday((year, week): Week) -> String {
    NaiveDate::from_isoywd_opt(year, week, Weekday::Mon)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn record_week(record: &TokenDeltaRecord) -> Option<Week> {
    match record {
        TokenDeltaRecord::Detail(detail) => iso_week(&detail.desc.iso_date),
        TokenDeltaRecord::Summary(summary) => {
            let (year, week, _) = summary.desc.iso_week_range;
            Some((year as i32, week as u32))
        }
    }
}

/// The key tokens are case-insensitively the same by.
fn case_key(token: &str, case_sensitive: bool) -> String {
    match case_sensitive {
        true => token.to_string(),
        false => token.to_lowercase(),
    }
}

/// A token's journals as read: its variants' records, and whether each was
/// read whole (its head and all of its segments).
struct SeriesData {
    token: String,
    journals: Vec<(String, Vec<TokenDeltaRecord>, bool)>,
    read_all: bool,
}

/// Commits' changes of the lines' tokens: by commit, its date, and changes.
type CommitChanges = BTreeMap<String, (String, Vec<SeriesChange>)>;

/// The changes of `week`'s commits (from the journals' detail records, and
/// expanded summary records), within `expansions`.
fn week_commits(
    repo: &Repository,
    data: &[SeriesData],
    week: Week,
    expansions: &mut usize,
) -> CommitChanges {
    let mut commits = CommitChanges::new();
    let mut add = |rev: &str, iso_date: &str, change: SeriesChange| {
        let entry = commits
            .entry(rev.to_string())
            .or_insert_with(|| (iso_date.chars().take(10).collect(), vec![]));
        entry.1.push(change);
    };
    for (series, series_data) in data.iter().enumerate() {
        for (token, records, _) in &series_data.journals {
            for record in records {
                if record_week(record) != Some(week) {
                    continue;
                }
                let details = match record {
                    TokenDeltaRecord::Detail(detail) => vec![detail.clone()],
                    TokenDeltaRecord::Summary(summary) => {
                        if *expansions >= MAX_EXPANSIONS {
                            continue;
                        }
                        *expansions += 1;
                        summary_details(repo, summary, &|_| true)
                    }
                };
                for detail in details {
                    add(
                        &detail.desc.source_rev,
                        &detail.desc.iso_date,
                        SeriesChange {
                            series,
                            token: token.clone(),
                            added: came(&detail.delta),
                            removed: gone(&detail.delta),
                        },
                    );
                }
            }
        }
    }
    commits
}

/// The other tokens a commit's rev-summary says it changed the most (not the
/// lines' tokens, `exclude`'s keys), case variants together unless
/// `case_sensitive`: of the `MAX_RANKED_OTHERS` with the most instances added
/// and removed, those with the most for how common they are (by the square
/// root of their journals' heads' sizes), since the most changed are mostly
/// common words (ex: a Cargo.lock's `version` and `checksum`, or HTML's `div`,
/// rather than the `value_parser` and `Args` of an upgrade to clap v3).
fn other_tokens(
    repo: &Repository,
    tree: &Tree,
    summary: &crate::file_format::history::rev_summaries::RevSummaryRecord,
    exclude: &HashSet<String>,
    case_sensitive: bool,
) -> Vec<OtherToken> {
    let mut by_token: HashMap<&str, TokenDeltaDetails> = HashMap::new();
    for file in summary.file_deltas.values() {
        for symbol in file.delta.symbol_group.symbol_deltas.values() {
            for (token, delta) in &symbol.token_changes {
                by_token.entry(token).or_default().accumulate(delta);
            }
        }
    }
    let mut groups: HashMap<String, OtherToken> = HashMap::new();
    let mut best: HashMap<String, u32> = HashMap::new();
    for (token, delta) in by_token {
        let key = case_key(token, case_sensitive);
        if exclude.contains(&key) || !is_trackable_token(token) {
            continue;
        }
        let (added, removed) = (came(&delta), gone(&delta));
        if added + removed == 0 {
            continue;
        }
        let group = groups.entry(key.clone()).or_default();
        group.variants.push(token.to_string());
        group.added += added;
        group.removed += removed;
        let most = best.entry(key).or_default();
        if added + removed > *most || group.token.is_empty() {
            *most = added + removed;
            group.token = token.to_string();
        }
    }
    let mut others: Vec<OtherToken> = groups
        .into_values()
        .map(|mut group| {
            group.variants.sort();
            group
        })
        .collect();
    others.sort_by(|a, b| {
        (b.added + b.removed)
            .cmp(&(a.added + a.removed))
            .then_with(|| a.token.cmp(&b.token))
    });
    others.truncate(MAX_RANKED_OTHERS);
    let odb = repo.odb().ok();
    let journal_kib = |token: &str| {
        let size = tree
            .get_path(&token_timeline_path(token))
            .ok()
            .and_then(|entry| odb.as_ref()?.read_header(entry.id()).ok())
            .map_or(0, |(size, _)| size);
        size as f64 / 1024.0
    };
    let mut scored: Vec<(f64, OtherToken)> = others
        .into_iter()
        .map(|other| {
            let kib: f64 = other.variants.iter().map(|v| journal_kib(v)).sum();
            (
                (other.added + other.removed) as f64 / kib.max(1.0).sqrt(),
                other,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.token.cmp(&b.1.token)));
    scored
        .into_iter()
        .take(MAX_OTHER_TOKENS)
        .map(|(_, other)| other)
        .collect()
}

/// The timelines of `tokens` (see the module docs).  Tokens which aren't ones
/// the history tracks, or without journals, have empty lines.
pub fn token_timeline(
    history: &TreeHistory,
    tokens: &[String],
    opts: TokenTimelineOptions,
) -> Result<TokenTimeline, String> {
    token_timeline_within(history, tokens, opts, BUDGET)
}

/// `token_timeline`, reading `budget` bytes of journals at most.
fn token_timeline_within(
    history: &TreeHistory,
    tokens: &[String],
    opts: TokenTimelineOptions,
    budget: usize,
) -> Result<TokenTimeline, String> {
    let repo: &Repository = &history.timeline;
    let mut timeline = TokenTimeline::default();
    let Some(head) = history.head_timeline_commit() else {
        return Ok(timeline);
    };
    let tree = head.tree().map_err(|e| e.to_string())?;

    // The tokens, case-insensitively distinct, up to MAX_TOKENS.
    let mut seen = HashSet::new();
    let mut asked = vec![];
    for token in tokens {
        if !seen.insert(case_key(token, opts.case_sensitive)) {
            continue;
        }
        if asked.len() < MAX_TOKENS {
            asked.push(token.clone());
        } else {
            timeline.dropped.push(token.clone());
        }
    }
    let token_budget = MAX_TOKEN_BUDGET.min(budget / asked.len().max(1));

    // Their journals.
    let mut data = vec![];
    for token in &asked {
        let mut series = SeriesData {
            token: token.clone(),
            journals: vec![],
            read_all: true,
        };
        if is_trackable_token(token) {
            let variants: Vec<String> = case_variants(repo, &tree, token)
                .into_iter()
                .filter(|variant| !opts.case_sensitive || variant == token)
                .take(MAX_VARIANTS)
                .collect();
            let variant_budget = token_budget / variants.len().max(1);
            for variant in variants {
                if let Some(journal) = read_journal(repo, &tree, &variant, variant_budget)? {
                    series.read_all &= journal.read_all;
                    series
                        .journals
                        .push((variant, journal.records, journal.read_all));
                }
            }
        }
        data.push(series);
    }

    // The lines: by week, oldest first, since the newest of the oldest weeks
    // of their journals not read whole (those whose older segments didn't fit
    // in the budget; see `segments`), so that the line has all of its
    // variants' changes.
    let mut changes_by_week: BTreeMap<Week, Vec<(usize, u32, u32)>> = BTreeMap::new();
    for (index, series_data) in data.iter().enumerate() {
        let mut weeks: BTreeMap<Week, (u32, u32)> = BTreeMap::new();
        let mut variants = vec![];
        let since = series_data
            .journals
            .iter()
            .filter(|(_, _, read_all)| !read_all)
            .filter_map(|(_, records, _)| records.last().and_then(record_week))
            .max();
        for (token, records, _) in &series_data.journals {
            let mut totals = VariantTotals {
                token: token.clone(),
                ..Default::default()
            };
            for record in records {
                let Some(week) = record_week(record) else {
                    continue;
                };
                if since.is_some_and(|since| week < since) {
                    continue;
                }
                let delta = delta(record);
                let entry = weeks.entry(week).or_default();
                entry.0 += came(delta);
                entry.1 += gone(delta);
                totals.added += came(delta);
                totals.removed += gone(delta);
            }
            variants.push(totals);
        }
        // (Relative to now: going back from the newest week.)
        let mut relative = 0i64;
        let mut points = vec![];
        for (week, (added, removed)) in weeks.iter().rev() {
            points.push(WeekPoint {
                week: week_label(*week),
                date: week_monday(*week),
                added: *added,
                removed: *removed,
                relative,
            });
            relative += *removed as i64 - *added as i64;
            changes_by_week
                .entry(*week)
                .or_default()
                .push((index, *added, *removed));
        }
        points.reverse();
        timeline.series.push(TimelineSeries {
            token: series_data.token.clone(),
            variants,
            weeks: points,
            start: relative,
            read_all: series_data.read_all,
        });
    }

    // The markers: the weeks changing a line by the biggest shares of its
    // range.
    let ranges: Vec<i64> = timeline
        .series
        .iter()
        .map(|series| {
            let values = series
                .weeks
                .iter()
                .map(|point| point.relative)
                .chain([series.start, 0]);
            let (min, max) = values.fold((i64::MAX, i64::MIN), |(min, max), v| {
                (min.min(v), max.max(v))
            });
            (max - min).max(1)
        })
        .collect();
    let mut scored: Vec<(f64, Week)> = changes_by_week
        .iter()
        .filter_map(|(week, changes)| {
            let score = changes
                .iter()
                .filter(|(_, added, removed)| added.abs_diff(*removed) >= MIN_MARKER_CHANGE)
                .map(|(series, added, removed)| {
                    added.abs_diff(*removed) as f64 / ranges[*series] as f64
                })
                .fold(0.0, f64::max);
            (score >= MARKER_SHARE).then_some((score, *week))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(MAX_MARKERS);
    let mut marker_weeks: Vec<Week> = scored.into_iter().map(|(_, week)| week).collect();
    marker_weeks.sort();

    let exclude: HashSet<String> = asked
        .iter()
        .map(|token| case_key(token, opts.case_sensitive))
        .collect();
    let mut expansions = 0;
    for week in marker_weeks {
        let mut marker = TimelineMarker {
            week: week_label(week),
            date: week_monday(week),
            changes: changes_by_week[&week]
                .iter()
                .map(|(series, added, removed)| SeriesChange {
                    series: *series,
                    token: asked[*series].clone(),
                    added: *added,
                    removed: *removed,
                })
                .collect(),
            ..Default::default()
        };
        let commits = week_commits(repo, &data, week, &mut expansions);
        marker.week_commits = commits.len() as u32;
        // (By their net changes of the lines, the biggest first.)
        let net = |changes: &[SeriesChange]| {
            let mut by_series: HashMap<usize, i64> = HashMap::new();
            for change in changes {
                *by_series.entry(change.series).or_default() +=
                    change.added as i64 - change.removed as i64;
            }
            by_series.values().map(|n| n.unsigned_abs()).sum::<u64>()
        };
        let mut ranked: Vec<(String, (String, Vec<SeriesChange>))> = commits.into_iter().collect();
        ranked.sort_by(|a, b| net(&b.1.1).cmp(&net(&a.1.1)).then_with(|| a.0.cmp(&b.0)));
        for (rev, (date, changes)) in ranked.into_iter().take(MAX_MARKER_COMMITS) {
            let mut commit = MarkerCommit {
                rev: rev.clone(),
                date,
                changes,
                ..Default::default()
            };
            if let Some(summary) = read_rev_summary(&history.path, &rev) {
                commit.header = header_html(&summary.message);
                commit.others_known = summary.file_totals.is_none();
                commit.others = other_tokens(repo, &tree, &summary, &exclude, opts.case_sensitive);
            }
            marker.commits.push(commit);
        }
        timeline.markers.push(marker);
    }

    Ok(timeline)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::hyperblame::token_history::read_blob;
    use crate::hyperblame::token_history::tests::{RENAME, test_history, token_path};
    use chrono::Datelike;

    /// The year of a week's Monday.
    fn year(point: &WeekPoint) -> i32 {
        NaiveDate::parse_from_str(&point.date, "%Y-%m-%d")
            .map(|date| date.year())
            .unwrap_or(0)
    }

    fn timeline(history: &TreeHistory, tokens: &[&str], case_sensitive: bool) -> TokenTimeline {
        let tokens: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
        token_timeline(history, &tokens, TokenTimelineOptions { case_sensitive }).unwrap()
    }

    #[test]
    fn test_token_timeline() {
        let (dir, history) = test_history("timeline");

        let found = timeline(&history, &["BackstagePass", "SystemGlobal"], false);
        assert_eq!(found.series.len(), 2);
        let backstage = &found.series[0];
        // (With its case variant's week.)
        assert_eq!(
            backstage
                .variants
                .iter()
                .map(|v| (v.token.as_str(), v.added, v.removed))
                .collect::<Vec<_>>(),
            vec![("BackstagePass", 45, 40), ("backstagePass", 2, 0)]
        );
        assert_eq!(
            backstage
                .weeks
                .iter()
                .map(|p| (p.week.as_str(), p.added, p.removed, p.relative))
                .collect::<Vec<_>>(),
            vec![
                ("2016-W52", 40, 0, 33),
                ("2018-W32", 1, 0, 34),
                ("2019-W18", 2, 0, 36),
                ("2022-W48", 3, 2, 37),
                ("2024-W50", 0, 38, -1),
                ("2025-W42", 1, 0, 0),
            ]
        );
        assert_eq!(backstage.start, -7);
        assert_eq!(year(&backstage.weeks[0]), 2016);
        assert!(backstage.read_all);

        // (The weeks changing a line the most are markers: the rename's, with
        // its commit, and the other tokens it changed, case variants together,
        // but not the lines'; and the week adding most of it.)
        assert_eq!(
            found
                .markers
                .iter()
                .map(|m| m.week.as_str())
                .collect::<Vec<_>>(),
            vec!["2016-W52", "2024-W50"]
        );
        let marker = &found.markers[1];
        assert_eq!(
            (marker.week.as_str(), marker.date.as_str()),
            ("2024-W50", "2024-12-09")
        );
        assert_eq!(
            marker
                .changes
                .iter()
                .map(|c| (c.series, c.added, c.removed))
                .collect::<Vec<_>>(),
            vec![(0, 0, 38), (1, 33, 0)]
        );
        assert_eq!(marker.week_commits, 1);
        let commit = &marker.commits[0];
        assert_eq!(commit.rev, RENAME);
        assert!(
            commit
                .header
                .contains("Rename BackstagePass to SystemGlobal")
        );
        assert_eq!(
            commit
                .changes
                .iter()
                .map(|c| (c.series, c.token.as_str(), c.added, c.removed))
                .collect::<Vec<_>>(),
            vec![(0, "BackstagePass", 0, 38), (1, "SystemGlobal", 33, 0)]
        );
        assert!(commit.others_known);
        assert!(commit.others.is_empty());

        // (Without SystemGlobal's line, it's one of the others, with its case
        // variant.)
        let found = timeline(&history, &["backstagepass"], false);
        let marker = found.markers.iter().find(|m| m.week == "2024-W50").unwrap();
        let others = &marker.commits[0].others;
        assert_eq!(others[0].token, "SystemGlobal");
        assert_eq!(
            others[0].variants,
            vec!["SystemGlobal".to_string(), "systemGlobal".to_string()]
        );
        assert_eq!((others[0].added, others[0].removed), (7, 0));

        // (Case-sensitively, the variant is its own line.)
        let found = timeline(&history, &["BackstagePass", "backstagePass"], true);
        assert_eq!(found.series[1].variants.len(), 1);
        assert_eq!(found.series[1].weeks.len(), 1);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_token_timeline_weeks_and_bounds() {
        let (dir, history) = test_history("timeline-weeks");

        // (A summarized week's commit, from its pred.)
        let found = timeline(&history, &["Gadget"], false);
        let marker = found.markers.iter().find(|m| m.week == "2020-W10").unwrap();
        assert_eq!(marker.week_commits, 2);
        assert_eq!(marker.commits[0].rev, "b".repeat(40));
        assert_eq!(marker.commits[0].changes[0].removed, 15);

        // (A token without a journal has an empty line, and those beyond
        // MAX_TOKENS are left out.)
        let mut tokens: Vec<String> = (0..20).map(|i| format!("Token{}", i)).collect();
        tokens.insert(0, "Doohickey".to_string());
        tokens.insert(1, "DOOHICKEY".to_string());
        let found = token_timeline(&history, &tokens, Default::default()).unwrap();
        assert_eq!(found.series.len(), MAX_TOKENS);
        assert_eq!(found.dropped.len(), 21 - MAX_TOKENS);
        assert_eq!(found.series[0].weeks.len(), 2);
        assert!(found.series[1].weeks.is_empty());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_token_timeline_segments_budget() {
        let (dir, history) = test_history("timeline-segments");
        let repo: &Repository = &history.timeline;
        let tree = history.head_timeline_commit().unwrap().tree().unwrap();
        let size = |path: &str| read_blob(repo, &tree, path).unwrap().len();
        let head = size(&token_path("Widget"));
        let newest_segment = size(&format!("{}.d/2024.ndjson", token_path("Widget")));

        // (All of it: Widget's head, its 2024 and 2023 segments, and widget.)
        let found = timeline(&history, &["Widget"], false);
        let series = &found.series[0];
        assert!(series.read_all);
        assert_eq!(
            series
                .weeks
                .iter()
                .map(|p| p.week.as_str())
                .collect::<Vec<_>>(),
            vec!["2023-W05", "2024-W23", "2024-W36", "2025-W10"]
        );
        assert_eq!(series.start, -43);

        // (A budget for Widget's head and newest segment, but not its 2023 one:
        // its line is since its oldest week read, which widget's newer oldest
        // week, read whole, doesn't change.)
        let budget = 2 * (head + newest_segment);
        let tokens = vec!["Widget".to_string()];
        let found = token_timeline_within(&history, &tokens, Default::default(), budget).unwrap();
        let series = &found.series[0];
        assert!(!series.read_all);
        assert_eq!(
            series
                .weeks
                .iter()
                .map(|p| p.week.as_str())
                .collect::<Vec<_>>(),
            vec!["2024-W23", "2024-W36", "2025-W10"]
        );
        assert_eq!(series.start, 7);

        fs::remove_dir_all(dir).unwrap();
    }
}
