//! Marking the hits and the search's other matches in `/query/`'s excerpts
//! (rows of the rendered files; see `cmd_augment_results`), as `/search/`
//! bolds its matches.
//!
//! A row's code cell's text, with its tags skipped and its entities decoded,
//! is the source line (output-file only escapes `&` and `<` in text), so byte
//! ranges in the line map onto the row's HTML.  Tags' attribute values can
//! have `>` (ex: `data-symbols="T_Foo<int>"`), so tags end at the first `>`
//! outside quotes.

use std::fmt::Write;
use std::ops::Range;

use regex::Regex;

const CODE_START: &str = r#"<code role="cell" class="source-line">"#;
const CODE_END: &str = "</code>";

/// The class of the marks of a result's hits: a semantic result's token, or a
/// text search's matches that no semantic result on the line covers.
pub const HIT_CLASS: &str = "query-hit";
/// The class of the marks of the search's other matches in the rows (ex: a
/// class's name in the definition of its constructor).
pub const MATCH_CLASS: &str = "query-match";

/// The range of the code cell's contents in `row`.
fn code_cell(row: &str) -> Option<Range<usize>> {
    let start = row.find(CODE_START)? + CODE_START.len();
    let end = row.rfind(CODE_END)?;
    (start <= end).then_some(start..end)
}

/// The length of the tag at the start of `html`, through its `>` (outside
/// quotes).
fn tag_len(html: &str) -> usize {
    let mut quote = None;
    for (i, c) in html.char_indices().skip(1) {
        match quote {
            None if c == '>' => return i + 1,
            None if c == '"' || c == '\'' => quote = Some(c),
            Some(q) if c == q => quote = None,
            _ => {}
        }
    }
    html.len()
}

/// The length and character of the entity at the start of `html`, if it's
/// one.
fn entity(html: &str) -> Option<(usize, char)> {
    // (`;` is ASCII, so its byte position is a character boundary.)
    let end = html.bytes().take(12).position(|b| b == b';')?;
    let c = match &html[1..end] {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        name => {
            let code = match name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => name.strip_prefix('#')?.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((end + 1, c))
}

/// The pieces of `html`: each character of text (decoded) with its HTML, and
/// each tag (without a character).
fn pieces(html: &str) -> impl Iterator<Item = (&str, Option<char>)> {
    let mut rest = html;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let (len, c) = match first {
            '<' => (tag_len(rest), None),
            '&' => entity(rest).map_or((1, Some('&')), |(len, c)| (len, Some(c))),
            c => (c.len_utf8(), Some(c)),
        };
        let (piece, tail) = rest.split_at(len);
        rest = tail;
        Some((piece, c))
    })
}

/// The source line of `row` (without its newline).
pub fn row_text(row: &str) -> Option<String> {
    let cell = code_cell(row)?;
    let mut text: String = pieces(&row[cell]).filter_map(|(_, c)| c).collect();
    if text.ends_with('\n') {
        text.pop();
    }
    Some(text)
}

/// `row` with `marks` (byte ranges of its line, sorted and disjoint, with
/// their classes) in `<mark>`s, which close before tags and reopen after them,
/// so they nest (ex: a match across tokens is a mark in each token).
pub fn mark_row(row: &str, marks: &[(Range<usize>, &str)]) -> String {
    let Some(cell) = code_cell(row) else {
        return row.to_string();
    };
    let mut out = String::with_capacity(row.len() + 40 * marks.len());
    out.push_str(&row[..cell.start]);
    let mut marks = marks.iter().peekable();
    let mut pos = 0;
    let mut open = false;
    for (html, c) in pieces(&row[cell.clone()]) {
        let Some(c) = c else {
            if open {
                out.push_str("</mark>");
                open = false;
            }
            out.push_str(html);
            continue;
        };
        while marks.next_if(|(range, _)| range.end <= pos).is_some() {}
        if let Some((_, class)) = marks
            .peek()
            .filter(|(range, _)| !open && range.start <= pos)
        {
            let _ = write!(out, r#"<mark class="{}">"#, class);
            open = true;
        }
        out.push_str(html);
        pos += c.len_utf8();
        if open && marks.next_if(|(range, _)| range.end <= pos).is_some() {
            out.push_str("</mark>");
            open = false;
        }
    }
    if open {
        out.push_str("</mark>");
    }
    out.push_str(&row[cell.end..]);
    out
}

/// `row` with its hits and matches marked: `hits` are byte ranges of the
/// result's key line without its leading whitespace (like crossref's lines),
/// given for the key line's row with the result's line (`expected`, crossref's
/// or livegrep's, which must be the row's, or the hits are left out), and the
/// matches are `pattern`'s other matches in the row's line.
pub fn highlight_row(
    row: &str,
    hits: &[(u32, u32)],
    expected: &str,
    pattern: Option<&Regex>,
) -> String {
    if hits.is_empty() && pattern.is_none() {
        return row.to_string();
    }
    let Some(text) = row_text(row) else {
        return row.to_string();
    };
    let trimmed = text.trim_start();
    let indent = text.len() - trimmed.len();

    let mut hit_ranges: Vec<Range<usize>> = vec![];
    if trimmed.starts_with(expected.trim()) {
        for &(start, end) in hits {
            let range = start as usize + indent..end as usize + indent;
            if range.start < range.end
                && text.is_char_boundary(range.start)
                && text.get(range.clone()).is_some()
            {
                hit_ranges.push(range);
            }
        }
    }
    hit_ranges.sort_by_key(|range| (range.start, range.end));
    // (Merged results' hits can overlap.)
    let mut merged: Vec<Range<usize>> = vec![];
    for range in hit_ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }

    let mut marks: Vec<(Range<usize>, &str)> = merged
        .iter()
        .map(|range| (range.clone(), HIT_CLASS))
        .collect();
    if let Some(pattern) = pattern {
        for found in pattern.find_iter(&text) {
            let range = found.range();
            if !range.is_empty()
                && !merged
                    .iter()
                    .any(|hit| hit.start < range.end && range.start < hit.end)
            {
                marks.push((range, MATCH_CLASS));
            }
        }
    }
    if marks.is_empty() {
        return row.to_string();
    }
    marks.sort_by_key(|(range, _)| range.start);
    mark_row(row, &marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(code: &str) -> String {
        format!(
            "<div role=\"row\" id=\"line-7\" class=\"source-line-with-number\">\n  <div role=\"cell\" class=\"line-number\" data-line-number=\"7\"></div>\n  <code role=\"cell\" class=\"source-line\">{}\n</code>\n</div>\n",
            code
        )
    }

    #[test]
    fn test_row_text() {
        // Entities are decoded, and tags skipped, even with `>` in quotes.
        let r = row(
            r#"  <span class="syn_type" data-symbols="T_Foo<int>">Foo</span>&lt;int> a &amp;&amp; b;"#,
        );
        assert_eq!(row_text(&r).unwrap(), "  Foo<int> a && b;");
    }

    #[test]
    fn test_mark_row() {
        let r =
            row(r#"<span class="syn_def" data-symbols="a>b">WorkerPrivate</span>::Worker&lt;x"#);
        // A mark inside a token, and one across the token's end and an
        // entity.
        let marked = mark_row(&r, &[(0..6, MATCH_CLASS), (10..22, HIT_CLASS)]);
        assert!(marked.contains(
            r#"<span class="syn_def" data-symbols="a>b"><mark class="query-match">Worker</mark>Priv<mark class="query-hit">ate</mark></span><mark class="query-hit">::Worker&lt;</mark>x"#
        ));
        assert_eq!(row_text(&marked), row_text(&r));
    }

    #[test]
    fn test_highlight_row() {
        let r = row(
            r#"    <span class="syn_def">WorkerPrivate</span>::<span class="syn_def">WorkerPrivate</span>("#,
        );
        let pattern = Regex::new("(?i)workerpriv").unwrap();
        // The hit is the constructor (bytes 15-28 without the indentation),
        // and the class's name is a match.
        let marked = highlight_row(
            &r,
            &[(15, 28)],
            "WorkerPrivate::WorkerPrivate(",
            Some(&pattern),
        );
        assert!(marked.contains(
            r#"<span class="syn_def"><mark class="query-match">WorkerPriv</mark>ate</span>::<span class="syn_def"><mark class="query-hit">WorkerPrivate</mark></span>("#
        ));
        // A row that isn't the result's line keeps only the matches.
        let marked = highlight_row(&r, &[(15, 28)], "something else", Some(&pattern));
        assert_eq!(marked.matches("query-hit").count(), 0);
        assert_eq!(marked.matches("query-match").count(), 2);
        // Without hits or matches, the row is unchanged.
        assert_eq!(highlight_row(&r, &[], "", None), r);
    }
}
