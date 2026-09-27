//! Consolidating the timeline journals' detail records into weekly summary
//! records so that journals don't grow without bound.  (Every append rewrites a
//! journal, so without this a hot token's journal gets rewritten with its whole
//! history for every revision touching the token.)
//!
//! ## Policy
//!
//! When a revision appends a detail record to a journal, every week which began
//! at least `LAG_WEEKS` weeks before the revision's week and has at least
//! `MIN_RECORDS` records is replaced by a single summary record.  The lag keeps
//! recent history detailed and gives commits from merged branches time to show
//! up before their week is summarized; weeks with a single record aren't worth
//! summarizing.  Weeks are ISO weeks of the commit dates (in UTC).
//!
//! Summaries always aggregate detail records, and a week's existing summaries
//! (from merges, or from commits which showed up after their week was
//! summarized) are expanded first (see `hyperblame::journals`).  So a summary
//! always equals the aggregate of its expanded details, which is what
//! `verify_summaries` checks.  A new summary's pred is the journal version the
//! revision appended to, which has all of the records it replaces.
//!
//! ## Merges
//!
//! Merges union their parents' versions of a journal (`merge_journal_versions`),
//! and the parents may have summarized the same week independently.  Summaries
//! with the same source revisions are deduplicated, and overlapping ones are
//! expanded and summarized again with each of their parents' journals as preds.
//! Detail records for revisions a summary covers are dropped.  So a source
//! revision only ever appears in one record of a journal.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, Weekday};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::file_format::history::timeline_common::{
    ChangeKind, JournalVersionRef, SummaryRecordRef, SymbolSyntaxDelta, SymbolSyntaxDeltaGroup,
    TimelineRecord, TokenDeltaDetails,
};
use crate::file_format::history::timeline_files_delta::{FileDeltaRecord, FileDeltaSummaryRecord};
use crate::file_format::history::timeline_future::{
    FutureFileChanges, FutureRecord, FutureSummaryRecord,
};
use crate::file_format::history::timeline_tokens::{TokenDeltaRecord, TokenDeltaSummaryRecord};
use crate::hyperblame::journals::{JournalReader, expand_records};

/// How many weeks before a revision's week a week must begin for the revision
/// to summarize it.
pub const LAG_WEEKS: i64 = 2;

/// The minimum number of records (details or summaries) a week needs to be
/// worth summarizing.
pub const MIN_RECORDS: usize = 2;

/// An ISO (year, week).
type IsoWeek = (i32, u32);

/// The ISO week of an ISO 8601 date and time.
pub fn iso_week(iso_date: &str) -> Option<IsoWeek> {
    let week = DateTime::parse_from_rfc3339(iso_date)
        .ok()?
        .naive_utc()
        .date()
        .iso_week();
    Some((week.year(), week.week()))
}

fn week_monday((year, week): IsoWeek) -> Option<NaiveDate> {
    NaiveDate::from_isoywd_opt(year, week, Weekday::Mon)
}

/// The week of a detail or summary record.
fn record_week<R: TimelineRecord>(record: &R) -> Option<IsoWeek> {
    match record.summary_ref() {
        Some(summary) => Some((
            summary.iso_week_range.0 as i32,
            summary.iso_week_range.1 as u32,
        )),
        None => iso_week(record.iso_date()?),
    }
}

/// A key for ordering records newest first: detail records' dates, and for
/// summaries the end of their (newest) week, so they sort before the week's
/// remaining details.
fn sort_key<R: TimelineRecord>(record: &R) -> String {
    match record.summary_ref() {
        Some(summary) => {
            let (year, week, _) = summary.iso_week_range;
            match NaiveDate::from_isoywd_opt(year as i32, week as u32, Weekday::Sun) {
                Some(sunday) => format!("{}T23:59:59Z", sunday),
                None => String::new(),
            }
        }
        None => record.iso_date().unwrap_or("").to_string(),
    }
}

/// Journal records which can be summarized.
pub trait Summarize: TimelineRecord + Clone + Serialize + DeserializeOwned {
    /// A summary record with the common fields `desc` aggregating `details`,
    /// which are detail records ordered newest first.
    fn summarize_details(desc: SummaryRecordRef, details: &[&Self]) -> Self;
}

/// A summary record aggregating detail records (in any order).
pub fn summarize<R: Summarize>(details: &[R], preds: Vec<JournalVersionRef>, week: IsoWeek) -> R {
    // Order the details canonically so that aggregates which depend on the
    // order (ex: the most recent move of a file) don't depend on how the
    // details were found.
    let mut sorted: Vec<&R> = details.iter().collect();
    sorted.sort_by(|a, b| {
        (b.iso_date(), b.detail_source_rev()).cmp(&(a.iso_date(), a.detail_source_rev()))
    });
    let desc = SummaryRecordRef {
        source_revs: sorted
            .iter()
            .map(|d| d.detail_source_rev().unwrap().to_string())
            .collect(),
        preds,
        iso_week_range: (week.0 as u16, week.1 as u8, week.1 as u8),
    };
    R::summarize_details(desc, &sorted)
}

/// Consolidate the records (newest first) of a journal which a revision dated
/// `now` just appended its detail record to; see the module docs.  `pred` is
/// the journal version it appended to, and `load` loads journal versions for
/// expanding summaries.  Returns the number of summaries written.
pub fn consolidate_appended<R: Summarize>(
    records: &mut Vec<R>,
    now: &str,
    pred: &JournalVersionRef,
    load: &mut impl FnMut(&JournalVersionRef) -> Result<Vec<R>, String>,
) -> Result<usize, String> {
    let Some(cutoff) = iso_week(now)
        .and_then(week_monday)
        .map(|monday| monday - Duration::weeks(LAG_WEEKS))
    else {
        return Ok(0);
    };
    let mut by_week: BTreeMap<IsoWeek, Vec<usize>> = BTreeMap::new();
    for (idx, record) in records.iter().enumerate() {
        if let Some(week) = record_week(record)
            && week_monday(week).is_some_and(|monday| monday <= cutoff)
        {
            by_week.entry(week).or_default().push(idx);
        }
    }
    by_week.retain(|_, idxs| idxs.len() >= MIN_RECORDS);
    if by_week.is_empty() {
        return Ok(0);
    }

    let mut summaries = vec![];
    let mut replaced: HashSet<usize> = HashSet::new();
    for (week, idxs) in &by_week {
        let week_records: Vec<R> = idxs.iter().map(|idx| records[*idx].clone()).collect();
        let details = expand_records(week_records, load)?;
        summaries.push(summarize(&details, vec![pred.clone()], *week));
        replaced.extend(idxs.iter().copied());
    }
    let mut idx = 0;
    records.retain(|_| {
        idx += 1;
        !replaced.contains(&(idx - 1))
    });
    // Each summary goes before the first remaining record which is older than
    // its week, leaving the other records in place.
    for summary in summaries {
        let key = sort_key(&summary);
        let pos = records
            .iter()
            .position(|record| sort_key(record) < key)
            .unwrap_or(records.len());
        records.insert(pos, summary);
    }
    Ok(by_week.len())
}

/// Union the versions of a journal from the parents of a merge, each with the
/// journal version it came from; see the module docs.  `load` loads journal
/// versions for expanding summaries.  Returns the records newest first.
///
/// Without summaries, this deduplicates the detail records (the first version
/// wins) and orders them by date (stably, so the earlier versions win ties).
pub fn merge_journal_versions<R: Summarize>(
    versions: Vec<(JournalVersionRef, Vec<R>)>,
    load: &mut impl FnMut(&JournalVersionRef) -> Result<Vec<R>, String>,
) -> Result<Vec<R>, String> {
    // ## Distinct summaries, with the versions they came from.
    let mut summaries: Vec<(R, BTreeSet<String>, BTreeSet<usize>)> = vec![];
    for (idx, (_, records)) in versions.iter().enumerate() {
        for record in records {
            let Some(summary) = record.summary_ref() else {
                continue;
            };
            let revs: BTreeSet<String> = summary.source_revs.iter().cloned().collect();
            match summaries.iter_mut().find(|(_, other, _)| *other == revs) {
                Some((_, _, sources)) => {
                    sources.insert(idx);
                }
                None => summaries.push((record.clone(), revs, BTreeSet::from([idx]))),
            }
        }
    }

    // ## Overlapping summaries (necessarily of the same week, since each
    // revision has one date) get summarized again.
    let mut groups: Vec<Vec<usize>> = vec![];
    for idx in 0..summaries.len() {
        let overlapping: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group
                    .iter()
                    .any(|other| !summaries[*other].1.is_disjoint(&summaries[idx].1))
            })
            .map(|(group_idx, _)| group_idx)
            .collect();
        let mut merged = vec![idx];
        for group_idx in overlapping.into_iter().rev() {
            merged.extend(groups.remove(group_idx));
        }
        groups.push(merged);
    }
    let mut final_summaries = vec![];
    for group in groups {
        if let [only] = group[..] {
            final_summaries.push(summaries[only].0.clone());
            continue;
        }
        let week = record_week(&summaries[group[0]].0).unwrap();
        let group_records: Vec<R> = group.iter().map(|idx| summaries[*idx].0.clone()).collect();
        let details = expand_records(group_records, load)?;
        let sources: BTreeSet<usize> = group
            .iter()
            .flat_map(|idx| summaries[*idx].2.iter().copied())
            .collect();
        let preds = sources.iter().map(|idx| versions[*idx].0.clone()).collect();
        final_summaries.push(summarize(&details, preds, week));
    }

    // ## Details which no summary covers.
    let covered: HashSet<String> = final_summaries
        .iter()
        .flat_map(|summary| summary.summary_ref().unwrap().source_revs.iter().cloned())
        .collect();
    let mut seen = HashSet::new();
    let mut merged: Vec<R> = vec![];
    for (_, records) in versions {
        for record in records {
            if let Some(rev) = record.detail_source_rev()
                && !covered.contains(rev)
                && seen.insert(rev.to_string())
            {
                merged.push(record);
            }
        }
    }
    merged.extend(final_summaries);
    // (`sort_by` is stable.)
    merged.sort_by_key(|record| std::cmp::Reverse(sort_key(record)));
    Ok(merged)
}

/// Check that each summary record in a journal version equals the aggregate
/// of its expanded detail records (apart from the order of its source
/// revisions and its preds).  Returns the number of summaries checked and
/// descriptions of any which don't match.
pub fn verify_summaries<R: Summarize>(
    reader: &mut JournalReader,
    version: &JournalVersionRef,
) -> Result<(usize, Vec<String>), String> {
    let normalize = |record: &R| -> Result<Value, String> {
        let mut value = serde_json::to_value(record).map_err(|e| e.to_string())?;
        if let Some(object) = value.as_object_mut() {
            object.remove("preds");
            if let Some(Value::Array(revs)) = object.get_mut("source_revs") {
                revs.sort_by_key(|rev| rev.to_string());
            }
        }
        Ok(value)
    };
    let mut checked = 0;
    let mut problems = vec![];
    for record in reader.records::<R>(version)? {
        let Some(summary) = record.summary_ref() else {
            continue;
        };
        checked += 1;
        let details = reader.expand(vec![record.clone()])?;
        let week = record_week(&record).unwrap();
        let expected = summarize(&details, summary.preds.clone(), week);
        if normalize(&record)? != normalize(&expected)? {
            problems.push(format!(
                "{}:{}: summary for week {:?} doesn't match its details: {} vs {}",
                version.timeline_rev,
                version.path,
                summary.iso_week_range,
                serde_json::to_string(&record).unwrap_or_default(),
                serde_json::to_string(&expected).unwrap_or_default()
            ));
        }
    }
    Ok((checked, problems))
}

// ## Aggregation for each kind of journal.

impl Summarize for TokenDeltaRecord {
    fn summarize_details(desc: SummaryRecordRef, details: &[&Self]) -> Self {
        let mut delta = TokenDeltaDetails::default();
        for detail in details {
            if let TokenDeltaRecord::Detail(detail) = detail {
                delta.accumulate(&detail.delta);
            }
        }
        TokenDeltaRecord::Summary(TokenDeltaSummaryRecord { desc, delta })
    }
}

impl Summarize for FutureRecord {
    fn summarize_details(desc: SummaryRecordRef, details: &[&Self]) -> Self {
        let details: Vec<_> = details
            .iter()
            .filter_map(|record| match record {
                FutureRecord::Detail(detail) => Some(detail),
                FutureRecord::Summary(_) => None,
            })
            .collect();
        let newest_move = details
            .iter()
            .find_map(|d| d.file_changes.file_moved_to.clone());
        let oldest_origin = details
            .iter()
            .rev()
            .find(|d| d.file_changes.file_moved_from.is_some());
        let revs = |sets: &mut dyn Iterator<Item = &BTreeMap<String, _>>| -> BTreeSet<String> {
            sets.flat_map(|set| set.keys().cloned()).collect()
        };
        FutureRecord::Summary(FutureSummaryRecord {
            desc,
            file_changes: FutureFileChanges {
                file_deleted: details.iter().any(|d| d.file_changes.file_deleted),
                file_moved_to: newest_move,
                file_moved_from: oldest_origin.and_then(|d| d.file_changes.file_moved_from.clone()),
                file_copied: oldest_origin.is_some_and(|d| d.file_changes.file_copied),
            },
            removed_token_revs: revs(&mut details.iter().map(|d| &d.extinguished_tokens)),
            moved_token_revs: revs(&mut details.iter().map(|d| &d.moved_out_tokens)),
            evolved_token_revs: revs(&mut details.iter().map(|d| &d.evolved_tokens)),
        })
    }
}

/// Fold a (newer) symbol delta into a week's aggregate for the symbol.  The
/// week's change kind is Removed if the symbol was last removed, otherwise
/// Added if it was first added, otherwise Evolved if it ever evolved, otherwise
/// Changed.
fn absorb_symbol_delta(week: &mut SymbolSyntaxDelta, newer: &SymbolSyntaxDelta) {
    week.change = match (week.change, newer.change) {
        (_, ChangeKind::Removed) => ChangeKind::Removed,
        (ChangeKind::Added, _) | (ChangeKind::Removed, ChangeKind::Added) => ChangeKind::Added,
        (ChangeKind::Evolved, _) | (_, ChangeKind::Evolved) => ChangeKind::Evolved,
        _ => ChangeKind::Changed,
    };
    if week.evolved_from.is_none() {
        week.evolved_from = newer.evolved_from.clone();
    }
    if newer.evolved_into.is_some() {
        week.evolved_into = newer.evolved_into.clone();
    }
    week.token_totals.accumulate(&newer.token_totals);
    for (token, delta) in &newer.token_changes {
        week.token_changes
            .entry(token.clone())
            .or_default()
            .accumulate(delta);
    }
}

impl Summarize for FileDeltaRecord {
    fn summarize_details(desc: SummaryRecordRef, details: &[&Self]) -> Self {
        let mut symbol_group = SymbolSyntaxDeltaGroup::default();
        // Oldest first, so later changes are folded into earlier ones.
        for record in details.iter().rev() {
            let FileDeltaRecord::Detail(detail) = record else {
                continue;
            };
            for (symbol, delta) in &detail.delta.symbol_group.symbol_deltas {
                match symbol_group.symbol_deltas.get_mut(symbol) {
                    Some(week) => absorb_symbol_delta(week, delta),
                    None => {
                        symbol_group
                            .symbol_deltas
                            .insert(symbol.clone(), delta.clone());
                    }
                }
            }
        }
        FileDeltaRecord::Summary(FileDeltaSummaryRecord { desc, symbol_group })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::timeline_common::DetailRecordRef;
    use crate::file_format::history::timeline_tokens::TokenDeltaDetailRecord;

    fn detail(rev: &str, iso_date: &str, added: u32) -> TokenDeltaRecord {
        TokenDeltaRecord::Detail(TokenDeltaDetailRecord {
            desc: DetailRecordRef {
                source_rev: rev.to_string(),
                syntax_rev: format!("syntax-{}", rev),
                iso_date: iso_date.to_string(),
                backs_out: vec![],
            },
            delta: TokenDeltaDetails {
                added,
                ..Default::default()
            },
        })
    }

    fn version(rev: &str) -> JournalVersionRef {
        JournalVersionRef {
            timeline_rev: rev.to_string(),
            path: "tokens/fo/o_/foo.ndjson".to_string(),
        }
    }

    fn describe(records: &[TokenDeltaRecord]) -> Vec<String> {
        records
            .iter()
            .map(|record| match record {
                TokenDeltaRecord::Detail(d) => d.desc.source_rev.clone(),
                TokenDeltaRecord::Summary(s) => format!(
                    "S{}:{}:{}",
                    s.desc.iso_week_range.1,
                    s.desc.source_revs.join("+"),
                    s.delta.added
                ),
            })
            .collect()
    }

    /// A loader over a fixed set of journal versions.
    fn loader(
        versions: Vec<(JournalVersionRef, Vec<TokenDeltaRecord>)>,
    ) -> impl FnMut(&JournalVersionRef) -> Result<Vec<TokenDeltaRecord>, String> {
        move |version| {
            versions
                .iter()
                .find(|(v, _)| v == version)
                .map(|(_, records)| records.clone())
                .ok_or_else(|| format!("no {:?}", version))
        }
    }

    #[test]
    fn test_weeks() {
        // 2024-01-01 is a Monday in ISO week 1; 2023-01-01 is a Sunday in week
        // 52 of 2022.
        assert_eq!(iso_week("2024-01-01T00:00:00Z"), Some((2024, 1)));
        assert_eq!(iso_week("2023-01-01T12:00:00Z"), Some((2022, 52)));
        assert_eq!(iso_week("garbage"), None);
    }

    #[test]
    fn test_consolidate_appended() {
        // Weeks 1 (b, a) and 2 (c) of 2024, and d in week 4.
        let mut records = vec![
            detail("d", "2024-01-22T00:00:00Z", 8),
            detail("c", "2024-01-08T00:00:00Z", 4),
            detail("b", "2024-01-03T00:00:00Z", 2),
            detail("a", "2024-01-01T00:00:00Z", 1),
        ];
        let mut load = loader(vec![]);
        // In week 2, week 1 isn't old enough yet.
        let mut early = records.clone();
        assert_eq!(
            consolidate_appended(&mut early, "2024-01-14T23:59:59Z", &version("p"), &mut load),
            Ok(0)
        );
        // In week 3, week 1 is summarized; week 2 only has one record.
        let parent = records.clone();
        assert_eq!(
            consolidate_appended(
                &mut records,
                "2024-01-15T00:00:00Z",
                &version("p"),
                &mut load
            ),
            Ok(1)
        );
        assert_eq!(describe(&records), vec!["d", "c", "S1:b+a:3"]);

        // A late commit for week 1 gets summarized with the existing summary,
        // which gets expanded from its pred.
        records.insert(1, detail("late", "2024-01-02T00:00:00Z", 16));
        let mut load = loader(vec![(version("p"), parent)]);
        assert_eq!(
            consolidate_appended(
                &mut records,
                "2024-01-23T00:00:00Z",
                &version("q"),
                &mut load
            ),
            Ok(1)
        );
        assert_eq!(describe(&records), vec!["d", "c", "S1:b+late+a:19"]);
        let TokenDeltaRecord::Summary(summary) = &records[2] else {
            panic!();
        };
        assert_eq!(summary.desc.preds, vec![version("q")]);
    }

    #[test]
    fn test_merge_journal_versions() {
        let a = detail("a", "2024-01-01T00:00:00Z", 1);
        let b = detail("b", "2024-01-02T00:00:00Z", 2);
        let c = detail("c", "2024-01-03T00:00:00Z", 4);
        let d = detail("d", "2024-01-22T00:00:00Z", 8);
        let week1 = (2024, 1);
        // Each parent summarized week 1 from a different journal version.
        let p1 = vec![a.clone(), b.clone()];
        let p2 = vec![b.clone(), c.clone()];
        let s1 = summarize(&p1, vec![version("p1")], week1);
        let s2 = summarize(&p2, vec![version("p2")], week1);
        let s1_again = summarize(&p1, vec![version("other")], week1);
        let mut load = loader(vec![(version("p1"), p1.clone()), (version("p2"), p2)]);

        // Without summaries, details are deduplicated and ordered by date.
        let merged = merge_journal_versions(
            vec![
                (version("x"), vec![b.clone(), a.clone()]),
                (version("y"), vec![d.clone(), b.clone()]),
            ],
            &mut load,
        )
        .unwrap();
        assert_eq!(describe(&merged), vec!["d", "b", "a"]);

        // Identical summaries are deduplicated and details they cover dropped.
        let merged = merge_journal_versions(
            vec![
                (version("x"), vec![d.clone(), s1.clone()]),
                (version("y"), vec![s1_again, a.clone()]),
            ],
            &mut load,
        )
        .unwrap();
        assert_eq!(describe(&merged), vec!["d", "S1:b+a:3"]);

        // Overlapping summaries are summarized again, with the merged versions
        // as preds.
        let merged = merge_journal_versions(
            vec![
                (version("x"), vec![s1]),
                (version("y"), vec![d.clone(), s2]),
            ],
            &mut load,
        )
        .unwrap();
        assert_eq!(describe(&merged), vec!["d", "S1:c+b+a:7"]);
        let TokenDeltaRecord::Summary(summary) = &merged[1] else {
            panic!();
        };
        assert_eq!(summary.desc.preds, vec![version("x"), version("y")]);
    }
}
