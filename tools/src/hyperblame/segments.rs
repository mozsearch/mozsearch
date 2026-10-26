//! Yearly segments of big journals.
//!
//! A revision which touches a journal rewrites the whole journal, and the most
//! common tokens' journals grew to megabytes (in the full firefox history,
//! `value`'s was 5.5 MB, mostly of 1,455 weekly summaries), so rewriting them
//! was most of what build-timeline-tree wrote.  So once a token journal gets
//! bigger than `SPLIT_BYTES`, the journal's file (the head) only keeps the
//! records of the current ISO year (and in its first weeks the previous one's;
//! see `keep_from`), and each earlier year's records are in a segment beside
//! it, `PATH.d/YEAR.ndjson`, with an empty header line.  Segments are rarely
//! rewritten, since weeks that old only change when a merge brings in records
//! for them.  The head's header lists its segments' years, newest first.
//! (Heads of two years, past 256 KB, still had token journals be 77% of what
//! build-timeline-tree gave git fast-import in firefox's 2020s: 24.75 of 32.3
//! GiB in 4,000 revisions of 2020; with heads of a year past 32 KB, 7.87 of
//! 15.4 GiB.)
//!
//! A journal version (ex: summaries' `preds`) names the head, and is the
//! logical journal: the head's header and records, and then the segments'
//! records, newest first (see `join`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use git2::{Repository, Tree};
use serde_json::Value;

use super::consolidation::{IsoWeek, LAG_WEEKS, line_iso_week};

/// How big a token journal can get before its older years go into segments.
pub const SPLIT_BYTES: usize = 32 << 10;

/// The first ISO year whose records a journal's head keeps when the newest
/// revision is of ISO week `week`: that week's year, but the previous one too
/// until `LAG_WEEKS` and a week into the year, so that consolidation has
/// summarized the old year's last weeks (which it does `LAG_WEEKS` after they
/// begin) in the head before they go into a segment.
pub fn keep_from(week: IsoWeek) -> i32 {
    let (year, number) = week;
    NaiveDate::from_isoywd_opt(year, number, Weekday::Mon)
        .map(|monday| (monday - Duration::weeks(LAG_WEEKS + 1)).iso_week().year())
        .unwrap_or(year)
}

/// The path of the segment of the journal at `journal` for `year`.
pub fn segment_path(journal: &Path, year: i32) -> PathBuf {
    let mut path = journal.as_os_str().to_owned();
    path.push(format!(".d/{}.ndjson", year));
    PathBuf::from(path)
}

/// The size of the journal at `path` in `tree`, with its segments, from their
/// objects' headers (without reading them), if it has one.  (Its head alone
/// has the latest year or two, of a journal past `SPLIT_BYTES`.)
pub fn journal_bytes(repo: &Repository, tree: &Tree, path: &Path) -> Option<u64> {
    let entry = tree.get_path(path).ok()?;
    let odb = repo.odb().ok()?;
    let mut bytes = odb.read_header(entry.id()).ok()?.0 as u64;
    let mut segments = path.as_os_str().to_owned();
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

/// The path of the token journal whose segment is at `path`, if `path` is a
/// segment's (see `segment_path`).
pub fn segment_journal(path: &str) -> Option<&str> {
    let (journal, file) = path.rsplit_once(".d/")?;
    let year = file.strip_suffix(".ndjson")?;
    (path.starts_with("tokens/") && journal.ends_with(".ndjson") && year.parse::<i32>().is_ok())
        .then_some(journal)
}

/// The years of a journal's segments, newest first, from its header line.
pub fn segment_years(header: &str) -> Vec<i32> {
    // (Most headers are `{}`.)
    if !header.contains("\"segments\"") {
        return vec![];
    }
    serde_json::from_str::<Value>(header)
        .ok()
        .and_then(|value| {
            value.get("segments")?.as_array().map(|years| {
                years
                    .iter()
                    .filter_map(Value::as_i64)
                    .map(|year| year as i32)
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// `header` with `years` (newest first) as its segments' years.
pub fn with_segment_years(header: &str, years: &[i32]) -> String {
    let mut value: Value =
        serde_json::from_str(header).unwrap_or(Value::Object(Default::default()));
    if let Some(object) = value.as_object_mut() {
        if years.is_empty() {
            object.remove("segments");
        } else {
            object.insert("segments".to_string(), Value::from(years.to_vec()));
        }
    }
    value.to_string()
}

/// The instances a token journal's header says its token has (see
/// `TokenHeader::count`), if it says.
pub fn header_count(header: &str) -> Option<i64> {
    if !header.contains("\"count\"") {
        return None;
    }
    serde_json::from_str::<Value>(header)
        .ok()?
        .get("count")?
        .as_i64()
}

/// `header` with `count` as its token's instances.
pub fn with_header_count(header: &str, count: i64) -> String {
    let mut value: Value =
        serde_json::from_str(header).unwrap_or(Value::Object(Default::default()));
    if let Some(object) = value.as_object_mut() {
        object.insert("count".to_string(), Value::from(count));
    }
    value.to_string()
}

/// A token journal's text (a head, or a joined journal) with its header's
/// count changed by `change`: from 0 if `new` (the journal's first record),
/// else from its count, if it has one (an older history's journals, without
/// counts, keep none).
pub fn change_header_count(contents: &str, change: i64, new: bool) -> Option<String> {
    let (header, rest) = contents.split_once('\n').unwrap_or((contents, ""));
    let count = match new {
        true => 0,
        false => header_count(header)?,
    };
    let header = with_header_count(header, count + change);
    Some(match rest.is_empty() && !contents.contains('\n') {
        true => header,
        false => format!("{}\n{}", header, rest),
    })
}

/// The ISO year of a record line, if it has a week.
pub fn line_year(line: &str) -> Option<i32> {
    line_iso_week(line).map(|(year, _)| year)
}

/// A journal's text: its header line and record lines.
fn text<'a>(header: &str, lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut text = header.to_string();
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    text
}

/// The logical journal of a head and its segments' texts (newest first).
pub fn join<'a>(head: &str, segments: impl IntoIterator<Item = &'a str>) -> String {
    let mut text = head.to_string();
    for segment in segments {
        for line in segment.lines().skip(1) {
            text.push('\n');
            text.push_str(line);
        }
    }
    text
}

/// A journal's head and its segments (year and text, newest first).
#[derive(Debug, PartialEq)]
pub struct Segmented {
    pub head: String,
    pub segments: Vec<(i32, String)>,
}

/// Split a logical journal into its head, with its records of `keep_from` and
/// later years (and any without a week), and segments for earlier years.
pub fn split(logical: &str, keep_from: i32) -> Segmented {
    let (header, rest) = logical.split_once('\n').unwrap_or((logical, ""));
    let mut head_lines = vec![];
    let mut years: BTreeMap<i32, Vec<&str>> = BTreeMap::new();
    for line in rest.lines() {
        match line_year(line) {
            Some(year) if year < keep_from => years.entry(year).or_default().push(line),
            _ => head_lines.push(line),
        }
    }
    let segment_years: Vec<i32> = years.keys().rev().copied().collect();
    Segmented {
        head: text(&with_segment_years(header, &segment_years), head_lines),
        segments: years
            .into_iter()
            .rev()
            .map(|(year, lines)| (year, text("{}", lines)))
            .collect(),
    }
}

/// Records by year (newest first): each year and its records.
pub type YearRecords<'a> = Vec<(i32, Vec<&'a str>)>;

/// The records of a head (its header and records, newest first) of years
/// before `keep_from`, by year (newest first), and the head without them (with
/// its header as it was), if its oldest record is of such a year.  (Records are
/// newest first, so heads which don't need it are cheap to rule out.)
pub fn take_old_years(head: &str, keep_from: i32) -> Option<(String, YearRecords<'_>)> {
    let (header, rest) = head.split_once('\n')?;
    if !rest
        .lines()
        .next_back()
        .and_then(line_year)
        .is_some_and(|year| year < keep_from)
    {
        return None;
    }
    let Segmented { head, .. } = split(head, keep_from);
    let (_, kept) = head.split_once('\n').unwrap_or((&head, ""));
    let kept = text(header, kept.lines());
    let mut years: BTreeMap<i32, Vec<&str>> = BTreeMap::new();
    for line in rest.lines() {
        if let Some(year) = line_year(line).filter(|year| *year < keep_from) {
            years.entry(year).or_default().push(line);
        }
    }
    Some((kept, years.into_iter().rev().collect()))
}

/// A segment with `lines` (newest first) added to its records, keeping them
/// newest first by week.
pub fn add_to_segment(segment: Option<&str>, lines: &[&str]) -> String {
    let existing: Vec<&str> = segment
        .and_then(|text| text.split_once('\n'))
        .map(|(_, rest)| rest.lines().collect())
        .unwrap_or_default();
    let week = |line: &str| line_iso_week(line);
    let mut merged = Vec::with_capacity(existing.len() + lines.len());
    let (mut a, mut b) = (
        existing.into_iter().peekable(),
        lines.iter().copied().peekable(),
    );
    loop {
        match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => {
                if week(y) > week(x) {
                    merged.push(b.next().unwrap());
                } else {
                    merged.push(a.next().unwrap());
                }
            }
            (Some(_), None) => merged.push(a.next().unwrap()),
            (None, Some(_)) => merged.push(b.next().unwrap()),
            (None, None) => break,
        }
    }
    text("{}", merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(rev: &str, date: &str) -> String {
        format!(
            r#"{{"type":"Detail","source_rev":"{rev}","syntax_rev":"s{rev}","iso_date":"{date}T00:00:00Z","added":1}}"#
        )
    }

    fn summary(revs: &[&str], year: u16, week: u8) -> String {
        format!(
            r#"{{"type":"Summary","source_revs":{:?},"preds":[],"iso_week_range":[{year},{week},{week}],"added":2}}"#,
            revs
        )
    }

    fn journal(header: &str, lines: &[String]) -> String {
        text(header, lines.iter().map(String::as_str))
    }

    #[test]
    fn test_paths_and_headers() {
        assert_eq!(
            segment_path(Path::new("tokens/28/90/value.ndjson"), 2023),
            PathBuf::from("tokens/28/90/value.ndjson.d/2023.ndjson")
        );
        assert_eq!(segment_years("{}"), Vec::<i32>::new());
        let header = with_segment_years("{}", &[2023, 2022]);
        assert_eq!(header, r#"{"segments":[2023,2022]}"#);
        assert_eq!(segment_years(&header), vec![2023, 2022]);
        assert_eq!(with_segment_years(&header, &[]), "{}");
    }

    #[test]
    fn test_split_and_join() {
        // Newest first: 2026 and 2025 stay in the head (in 2026's first
        // weeks), 2024 and 2022 go.
        let lines = vec![
            detail("f", "2026-01-12"),
            summary(&["e"], 2025, 40),
            summary(&["d", "c"], 2024, 7),
            summary(&["b"], 2024, 2),
            summary(&["a"], 2022, 30),
        ];
        let logical = journal("{}", &lines);
        let segmented = split(&logical, keep_from((2026, 3)));
        assert_eq!(
            segmented.head,
            journal(r#"{"segments":[2024,2022]}"#, &lines[..2])
        );
        assert_eq!(
            segmented.segments,
            vec![
                (2024, journal("{}", &lines[2..4])),
                (2022, journal("{}", &lines[4..])),
            ]
        );
        // Joining them gives the logical journal back, with the head's header.
        let joined = join(
            &segmented.head,
            segmented.segments.iter().map(|(_, text)| text.as_str()),
        );
        assert_eq!(joined, journal(r#"{"segments":[2024,2022]}"#, &lines));
        // A head whose oldest record is recent has nothing to take.
        assert_eq!(take_old_years(&segmented.head, keep_from((2026, 3))), None);
        // The ISO year of 2024-12-30 is 2025.
        assert_eq!(line_year(&detail("x", "2024-12-30")), Some(2025));
    }

    #[test]
    fn test_segment_journal() {
        let journal = "tokens/fo/o_/foo.ndjson";
        let segment = segment_path(Path::new(journal), 2019);
        assert_eq!(segment_journal(segment.to_str().unwrap()), Some(journal));
        assert_eq!(segment_journal(journal), None);
        assert_eq!(segment_journal("files-delta/a.ndjson.d/2019.ndjson"), None);
    }

    #[test]
    fn test_keep_from() {
        // The previous year until 3 weeks into the year (`LAG_WEEKS` = 2), so
        // that its last weeks get summarized in the head; then the year alone.
        assert_eq!(LAG_WEEKS, 2);
        assert_eq!(keep_from((2026, 1)), 2025);
        assert_eq!(keep_from((2026, 3)), 2025);
        assert_eq!(keep_from((2026, 4)), 2026);
        assert_eq!(keep_from((2026, 52)), 2026);
        // (2020 has 53 ISO weeks.)
        assert_eq!(keep_from((2021, 3)), 2020);
        assert_eq!(keep_from((2021, 4)), 2021);
    }

    #[test]
    fn test_take_old_years_and_add_to_segment() {
        // A head which a new year left with 2024's records.
        let lines = vec![
            detail("g", "2026-01-13"),
            summary(&["f"], 2025, 30),
            summary(&["e"], 2024, 50),
            summary(&["d"], 2024, 3),
        ];
        let head = journal(r#"{"segments":[2023]}"#, &lines);
        let (kept, old) = take_old_years(&head, keep_from((2026, 3))).unwrap();
        assert_eq!(kept, journal(r#"{"segments":[2023]}"#, &lines[..2]));
        assert_eq!(
            old,
            vec![(2024, vec![lines[2].as_str(), lines[3].as_str()])]
        );

        // Lines go into an existing segment by week, newest first.
        let segment = journal("{}", &[summary(&["c"], 2024, 40), summary(&["b"], 2024, 1)]);
        let added = add_to_segment(
            Some(&segment),
            &[&summary(&["x"], 2024, 45), &summary(&["y"], 2024, 20)],
        );
        assert_eq!(
            added,
            journal(
                "{}",
                &[
                    summary(&["x"], 2024, 45),
                    summary(&["c"], 2024, 40),
                    summary(&["y"], 2024, 20),
                    summary(&["b"], 2024, 1),
                ]
            )
        );
        assert_eq!(
            add_to_segment(None, &[&lines[2]]),
            journal("{}", &lines[2..3])
        );
    }
}
