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

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, Weekday};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
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

/// The fields of a journal record's line which say which week it's of (see
/// `record_week`).
#[derive(Deserialize)]
struct WeekFields<'a> {
    #[serde(borrow)]
    iso_date: Option<Cow<'a, str>>,
    iso_week_range: Option<(u16, u8, u8)>,
}

/// Prepend `record` to the journal `text` (a header line followed by a line per
/// record) as `consolidate_appended` would after the record was appended by a
/// revision dated `now` (None to not consolidate), if that wouldn't summarize
/// any weeks, which is almost always the case.  Then only the weeks of the
/// journal's lines are parsed, and they're kept as they are, rather than
/// parsing and serializing every record, which was most of the timeline's time
/// for hot tokens' journals.  Returns None if weeks would be summarized, or if
/// the journal is empty or a line can't be read.
pub fn prepend_unconsolidated<R: Summarize>(
    text: &str,
    record: &R,
    now: Option<&str>,
) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let (header, rest) = match text.split_once('\n') {
        Some((header, rest)) => (header, Some(rest)),
        None => (text, None),
    };
    let cutoff = now
        .and_then(iso_week)
        .and_then(week_monday)
        .map(|monday| monday - Duration::weeks(LAG_WEEKS));
    if let Some(cutoff) = cutoff {
        let mut counts: HashMap<IsoWeek, usize> = HashMap::new();
        let mut count_week = |week: Option<IsoWeek>| {
            if let Some(week) = week
                && week_monday(week).is_some_and(|monday| monday <= cutoff)
            {
                let count = counts.entry(week).or_default();
                *count += 1;
                return *count >= MIN_RECORDS;
            }
            false
        };
        if count_week(record_week(record)) {
            return None;
        }
        for line in rest.into_iter().flat_map(str::lines) {
            let fields: WeekFields = serde_json::from_str(line).ok()?;
            let week = match fields.iso_week_range {
                Some((year, week, _)) => Some((year as i32, week as u32)),
                None => fields.iso_date.as_deref().and_then(iso_week),
            };
            if count_week(week) {
                return None;
            }
        }
    }
    let record = serde_json::to_string(record).ok()?;
    let mut prepended = String::with_capacity(text.len() + record.len() + 2);
    prepended.push_str(header);
    prepended.push('\n');
    prepended.push_str(&record);
    if let Some(rest) = rest {
        prepended.push('\n');
        prepended.push_str(rest);
    }
    Some(prepended)
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
    merge_versions(versions, &mut |group, preds, week| {
        let details = expand_records(group, load)?;
        Ok(summarize(&details, preds, week))
    })
}

/// The fields of a journal record's line which merging needs.
#[derive(Deserialize)]
struct MergeFields<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    source_rev: Option<Cow<'a, str>>,
    #[serde(borrow)]
    iso_date: Option<Cow<'a, str>>,
    source_revs: Option<Vec<String>>,
    iso_week_range: Option<(u16, u8, u8)>,
}

/// A journal record's line, with the fields merging needs (a summary's without
/// its preds).
#[derive(Clone)]
struct MergeLine<'a> {
    line: &'a str,
    source_rev: Option<Cow<'a, str>>,
    iso_date: Option<Cow<'a, str>>,
    summary: Option<SummaryRecordRef>,
}

impl<'a> MergeLine<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let fields: MergeFields = serde_json::from_str(line).ok()?;
        let summary = match &*fields.kind {
            "Detail" => {
                fields.source_rev.as_ref()?;
                fields.iso_date.as_ref()?;
                None
            }
            "Summary" => Some(SummaryRecordRef {
                source_revs: fields.source_revs?,
                preds: vec![],
                iso_week_range: fields.iso_week_range?,
            }),
            _ => return None,
        };
        Some(MergeLine {
            line,
            source_rev: fields.source_rev,
            iso_date: fields.iso_date,
            summary,
        })
    }
}

impl TimelineRecord for MergeLine<'_> {
    fn iso_date(&self) -> Option<&str> {
        match self.summary {
            Some(_) => None,
            None => self.iso_date.as_deref(),
        }
    }

    fn detail_source_rev(&self) -> Option<&str> {
        match self.summary {
            Some(_) => None,
            None => self.source_rev.as_deref(),
        }
    }

    fn summary_ref(&self) -> Option<&SummaryRecordRef> {
        self.summary.as_ref()
    }
}

/// `merge_journal_versions` for the texts of the versions (a header line and a
/// line per record): the merged journal, with the lines of the records it keeps
/// as they were and the first version's header, if no summaries need to be
/// summarized again, which is almost always the case.  Then only a few fields
/// of the records are parsed, rather than parsing and serializing every record,
/// which made firefox's merges slow.  Returns None if summaries need to be
/// summarized again, if no version has any lines, or if a line can't be read.
pub fn merge_journal_texts(versions: &[(JournalVersionRef, &str)]) -> Option<String> {
    let mut header = None;
    let mut parsed = vec![];
    for (version, text) in versions {
        let mut lines = text.lines();
        let Some(first) = lines.next() else {
            continue;
        };
        header.get_or_insert(first);
        let records = lines.map(MergeLine::parse).collect::<Option<Vec<_>>>()?;
        parsed.push((version.clone(), records));
    }
    let header = header?;
    let merged = merge_versions(parsed, &mut |_, _, _| Err(String::new())).ok()?;
    let mut text = String::with_capacity(
        header.len()
            + merged
                .iter()
                .map(|record| record.line.len() + 1)
                .sum::<usize>(),
    );
    text.push_str(header);
    for record in merged {
        text.push('\n');
        text.push_str(record.line);
    }
    Some(text)
}

/// `merge_journal_versions` with `resummarize` making the summary of a group of
/// overlapping summaries from them, the versions they came from, and their week.
fn merge_versions<R: TimelineRecord + Clone>(
    versions: Vec<(JournalVersionRef, Vec<R>)>,
    resummarize: &mut impl FnMut(Vec<R>, Vec<JournalVersionRef>, IsoWeek) -> Result<R, String>,
) -> Result<Vec<R>, String> {
    // ## Distinct summaries, with the versions they came from, in the order
    // they're first found.  (Journals get a summary per week, so after years of
    // history, comparing each summary with every other one made firefox's
    // merges take close to a minute each.)
    let mut summaries: Vec<(R, BTreeSet<usize>)> = vec![];
    {
        let mut by_revs: HashMap<Vec<&str>, usize> = HashMap::new();
        for (idx, (_, records)) in versions.iter().enumerate() {
            for record in records {
                let Some(summary) = record.summary_ref() else {
                    continue;
                };
                let mut revs: Vec<&str> = summary.source_revs.iter().map(String::as_str).collect();
                revs.sort_unstable();
                revs.dedup();
                match by_revs.get(&revs) {
                    Some(&existing) => {
                        summaries[existing].1.insert(idx);
                    }
                    None => {
                        by_revs.insert(revs, summaries.len());
                        summaries.push((record.clone(), BTreeSet::from([idx])));
                    }
                }
            }
        }
    }

    // ## Overlapping summaries (necessarily of the same week, since each
    // revision has one date) get summarized again.  They're the groups of
    // summaries connected by sharing source revisions, ordered by their last
    // summary (which is the order in which they're added below, and so the
    // order of a week's summaries, which sort the same).
    let mut parents: Vec<usize> = (0..summaries.len()).collect();
    fn root(parents: &mut [usize], mut idx: usize) -> usize {
        while parents[idx] != idx {
            parents[idx] = parents[parents[idx]];
            idx = parents[idx];
        }
        idx
    }
    {
        let mut first_with_rev: HashMap<&str, usize> = HashMap::new();
        for (idx, (record, _)) in summaries.iter().enumerate() {
            for rev in &record.summary_ref().unwrap().source_revs {
                match first_with_rev.get(rev.as_str()) {
                    Some(&other) => {
                        let (a, b) = (root(&mut parents, other), root(&mut parents, idx));
                        if a != b {
                            parents[a] = b;
                        }
                    }
                    None => {
                        first_with_rev.insert(rev.as_str(), idx);
                    }
                }
            }
        }
    }
    let mut groups_by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for idx in 0..summaries.len() {
        let group_root = root(&mut parents, idx);
        groups_by_root.entry(group_root).or_default().push(idx);
    }
    let mut groups: Vec<Vec<usize>> = groups_by_root.into_values().collect();
    groups.sort_by_key(|group| *group.last().unwrap());

    let mut final_summaries = vec![];
    for group in groups {
        if let [only] = group[..] {
            final_summaries.push(summaries[only].0.clone());
            continue;
        }
        let week = record_week(&summaries[group[0]].0).unwrap();
        let group_records: Vec<R> = group.iter().map(|idx| summaries[*idx].0.clone()).collect();
        let sources: BTreeSet<usize> = group
            .iter()
            .flat_map(|idx| summaries[*idx].1.iter().copied())
            .collect();
        let preds = sources.iter().map(|idx| versions[*idx].0.clone()).collect();
        final_summaries.push(resummarize(group_records, preds, week)?);
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

    /// `merge_journal_versions` as it was before it was linear, to check that
    /// it gives the same results.
    fn merge_journal_versions_reference<R: Summarize>(
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

    type Versions = Vec<(JournalVersionRef, Vec<TokenDeltaRecord>)>;

    /// Random versions of a journal for merging, and the journal versions their
    /// summaries' preds refer to.
    fn random_merge_cases(count: usize) -> Vec<(Versions, Versions)> {
        // A little xorshift, for reproducible random journals.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut cases = vec![];
        for case in 0..count {
            // Revisions in 3 weeks of 2024, each with a date in its week.
            let revs: Vec<TokenDeltaRecord> = (0..12)
                .map(|i| {
                    let week = i % 3;
                    let date = format!("2024-01-{:02}T00:00:{:02}Z", 1 + 7 * week + i % 5, i);
                    detail(&format!("r{}", i), &date, 1 << (i % 10))
                })
                .collect();
            let mut preds = vec![];
            let mut versions = vec![];
            for v in 0..(2 + next(2)) {
                let mut records = vec![];
                for week in 0..3u32 {
                    let in_week: Vec<&TokenDeltaRecord> =
                        revs.iter().skip(week as usize).step_by(3).collect();
                    match next(4) {
                        // Some of the week's details.
                        0 | 1 => records.extend(
                            in_week
                                .iter()
                                .filter(|_| next(2) == 0)
                                .map(|r| (*r).clone()),
                        ),
                        // One or two summaries of subsets (sometimes
                        // overlapping, sometimes the same as another
                        // version's), with their pred.
                        _ => {
                            for _ in 0..(1 + next(2)) {
                                let subset: Vec<TokenDeltaRecord> = in_week
                                    .iter()
                                    .filter(|_| next(3) != 0)
                                    .map(|r| (*r).clone())
                                    .collect();
                                if subset.is_empty() {
                                    continue;
                                }
                                let pred = version(&format!("pred-{}-{}", case, preds.len()));
                                preds.push((pred.clone(), subset.clone()));
                                records.push(summarize(&subset, vec![pred], (2024, week + 1)));
                            }
                        }
                    }
                }
                records.sort_by_key(|record| std::cmp::Reverse(sort_key(record)));
                versions.push((version(&format!("v{}-{}", case, v)), records));
            }
            cases.push((versions, preds));
        }
        cases
    }

    #[test]
    fn test_merge_journal_versions_like_reference() {
        let (mut resummarized, mut weeks_with_several) = (0, 0);
        for (case, (versions, preds)) in random_merge_cases(500).into_iter().enumerate() {
            let mut load = loader(preds);
            let expected = merge_journal_versions_reference(versions.clone(), &mut load).unwrap();
            let actual = merge_journal_versions(versions, &mut load).unwrap();
            assert_eq!(
                serde_json::to_string(&actual).unwrap(),
                serde_json::to_string(&expected).unwrap(),
                "case {}",
                case
            );
            let mut summaries_by_week: BTreeMap<u8, usize> = BTreeMap::new();
            for record in &actual {
                if let Some(summary) = record.summary_ref() {
                    resummarized += (summary.preds.len() > 1) as usize;
                    *summaries_by_week
                        .entry(summary.iso_week_range.1)
                        .or_default() += 1;
                }
            }
            weeks_with_several += summaries_by_week.values().filter(|&&n| n > 1).count();
        }
        // The cases include overlapping summaries, which get summarized
        // again, and weeks with several summaries, whose order matters.
        assert!(resummarized > 0 && weeks_with_several > 0);
    }

    #[test]
    fn test_merge_journal_texts() {
        use crate::file_format::history::io_helpers::record_file_contents_to_string;
        use crate::file_format::history::timeline_tokens::TokenHeader;
        let (mut fast, mut resummarized) = (0, 0);
        for (case, (versions, preds)) in random_merge_cases(500).into_iter().enumerate() {
            let texts: Vec<(JournalVersionRef, String)> = versions
                .iter()
                .map(|(version, records)| {
                    let text = record_file_contents_to_string(&TokenHeader::default(), records);
                    (version.clone(), text)
                })
                .collect();
            let texts: Vec<(JournalVersionRef, &str)> = texts
                .iter()
                .map(|(version, text)| (version.clone(), text.as_str()))
                .collect();
            // (Only summarizing summaries again loads journal versions.)
            let mut loads = 0;
            let mut load_pred = loader(preds);
            let merged = merge_journal_versions(versions, &mut |version| {
                loads += 1;
                load_pred(version)
            })
            .unwrap();
            match merge_journal_texts(&texts) {
                Some(text) => {
                    assert_eq!(loads, 0, "case {}", case);
                    assert_eq!(
                        text,
                        record_file_contents_to_string(&TokenHeader::default(), &merged),
                        "case {}",
                        case
                    );
                    fast += 1;
                }
                None => {
                    assert!(loads > 0, "case {}", case);
                    resummarized += 1;
                }
            }
        }
        assert!(fast > 0 && resummarized > 0);
        // Versions without any lines are skipped, like parents without the
        // journal.
        let text = r#"{}
{"type":"Detail","source_rev":"a","syntax_rev":"s","iso_date":"2024-01-01T00:00:00Z","added":1}"#;
        assert_eq!(
            merge_journal_texts(&[(version("x"), ""), (version("y"), text)]).as_deref(),
            Some(text)
        );
        assert_eq!(merge_journal_texts(&[(version("x"), "")]), None);
    }

    #[test]
    fn test_prepend_unconsolidated() {
        use crate::file_format::history::io_helpers::record_file_contents_to_string;
        use crate::file_format::history::timeline_tokens::TokenHeader;
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let (mut fast, mut consolidated) = (0, 0);
        for case in 0..300 {
            // A journal of details and summaries over 6 weeks, newest first.
            let mut records = vec![];
            for week in (0..6u32).rev() {
                let in_week: Vec<TokenDeltaRecord> = (0..next(3))
                    .map(|i| {
                        let date =
                            format!("2024-01-{:02}T00:00:{:02}Z", 1 + 7 * week + i as u32, i);
                        detail(&format!("r{}-{}-{}", case, week, i), &date, 1)
                    })
                    .collect();
                if next(2) == 0 && !in_week.is_empty() {
                    records.push(summarize(&in_week, vec![version("p")], (2024, week + 1)));
                } else {
                    records.extend(in_week);
                }
            }
            let text = record_file_contents_to_string(&TokenHeader::default(), &records);
            let date = format!("2024-01-{:02}T12:00:00Z", 1 + next(49));
            let new = detail(&format!("new{}", case), &date, 2);
            let now = format!("2024-02-{:02}T00:00:00Z", 1 + next(20));

            let mut expected = records.clone();
            expected.insert(0, new.clone());
            let summarized =
                consolidate_appended(&mut expected, &now, &version("p"), &mut loader(vec![]))
                    .unwrap_or(1);
            match prepend_unconsolidated(&text, &new, Some(&now)) {
                Some(prepended) => {
                    assert_eq!(summarized, 0, "case {}", case);
                    assert_eq!(
                        prepended,
                        record_file_contents_to_string(&TokenHeader::default(), &expected),
                        "case {}",
                        case
                    );
                    fast += 1;
                }
                None => {
                    assert!(summarized > 0, "case {}", case);
                    consolidated += 1;
                }
            }
        }
        assert!(fast > 0 && consolidated > 0);
    }

    /// Time merging a real journal with the version without its newest record
    /// with `merge_journal_texts` and by parsing, merging and serializing every
    /// record.  The journals are the files in $JOURNAL_DIR (token journals)
    /// and its files-delta/ and future/ directories.
    #[test]
    #[ignore]
    fn bench_merge_journal_texts() {
        use crate::file_format::history::timeline_files_delta::FileDeltaHeader;
        use crate::file_format::history::timeline_future::FutureHeader;
        use crate::file_format::history::timeline_tokens::TokenHeader;
        let dir = std::path::PathBuf::from(std::env::var("JOURNAL_DIR").expect("JOURNAL_DIR"));
        for (subdir, kind) in [("", 0), ("files-delta", 1), ("future", 2)] {
            let Ok(entries) = std::fs::read_dir(dir.join(subdir)) else {
                continue;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.extension().is_none_or(|ext| ext != "ndjson") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let (header, rest) = text.split_once('\n').unwrap();
                let older = match rest.split_once('\n') {
                    Some((_, older)) => format!("{}\n{}", header, older),
                    None => header.to_string(),
                };
                let texts = [
                    (version("x"), text.as_str()),
                    (version("y"), older.as_str()),
                ];
                let (fast_time, full_time, same) = match kind {
                    0 => time_merges::<TokenHeader, TokenDeltaRecord>(&texts),
                    1 => time_merges::<FileDeltaHeader, FileDeltaRecord>(&texts),
                    _ => time_merges::<FutureHeader, FutureRecord>(&texts),
                };
                println!(
                    "{}: {} bytes: fast {:?}, full parse, merge and serialize {:?}, same: {}",
                    path.display(),
                    text.len(),
                    fast_time,
                    full_time,
                    same
                );
            }
        }
    }

    /// The times to merge `texts` with `merge_journal_texts` and by parsing,
    /// merging and serializing every record, and whether they gave the same
    /// journal.
    fn time_merges<H: Serialize + DeserializeOwned, R: Summarize>(
        texts: &[(JournalVersionRef, &str)],
    ) -> (std::time::Duration, std::time::Duration, bool) {
        use crate::file_format::history::io_helpers::{
            read_record_file_contents, record_file_contents_to_string,
        };
        let start = std::time::Instant::now();
        let fast = merge_journal_texts(texts);
        let fast_time = start.elapsed();
        let start = std::time::Instant::now();
        let mut header = None;
        let versions: Vec<(JournalVersionRef, Vec<R>)> = texts
            .iter()
            .map(|(version, text)| {
                let (h, records): (H, Vec<R>) = read_record_file_contents(text.as_bytes()).unwrap();
                header.get_or_insert(h);
                (version.clone(), records)
            })
            .collect();
        let merged =
            merge_journal_versions(versions, &mut |_| Err("no preds".to_string())).unwrap();
        let full = record_file_contents_to_string(&header.unwrap(), &merged);
        let full_time = start.elapsed();
        (fast_time, full_time, fast.as_deref() == Some(full.as_str()))
    }

    /// Time prepending a record to real token journals (the files in
    /// $JOURNAL_DIR) with `prepend_unconsolidated` and by parsing and
    /// serializing every record.
    #[test]
    #[ignore]
    fn bench_prepend_unconsolidated() {
        use crate::file_format::history::io_helpers::{
            read_record_file_contents, record_file_contents_to_string,
        };
        use crate::file_format::history::timeline_tokens::TokenHeader;
        let dir = std::env::var("JOURNAL_DIR").expect("JOURNAL_DIR");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "ndjson") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let new = detail("new", "2008-02-20T00:00:00Z", 1);
            let now = Some("2008-02-20T00:00:00Z");
            let start = std::time::Instant::now();
            let fast = prepend_unconsolidated(&text, &new, now);
            let fast_time = start.elapsed();
            let start = std::time::Instant::now();
            let (header, mut records): (TokenHeader, Vec<TokenDeltaRecord>) =
                read_record_file_contents(text.as_bytes()).unwrap();
            records.insert(0, new.clone());
            let full = record_file_contents_to_string(&header, &records);
            let full_time = start.elapsed();
            println!(
                "{}: {} bytes: fast {:?} ({}), full parse and serialize {:?}, same: {}",
                path.display(),
                text.len(),
                fast_time,
                if fast.is_some() {
                    "no summaries"
                } else {
                    "would summarize"
                },
                full_time,
                fast.as_deref().is_none_or(|fast| fast == full)
            );
        }
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
