//! HTML for history digests (`file_format::recency::Recency`) on `/query/`'s
//! pages: blots on results' lines, and sparklines on the "Last changed" facet's
//! values.

use std::fmt::Write;

use crate::file_format::recency::{BINS, Recency};

/// The cell of a row with the blot of a result's digest, if it's the result's
/// key line (see `blot`).  Other rows have an empty cell, which keeps the rows'
/// code aligned.
pub fn blot_cell(recency: Option<&Recency>, label: Option<&str>) -> String {
    match recency {
        Some(recency) => blot(recency, label, r#" role="cell""#),
        None => r#"<span role="cell" class="query-recency"></span>"#.to_string(),
    }
}

/// The blot of a digest: a square per bin of age (newest first), shaded by how
/// much changed then, with the element's `attributes`, and a title saying whose
/// history it is (`label`, ex: "History of Foo::Bar\nfrom history context Foo
/// in foo.cpp") and what changed.  The empty squares are the element's
/// background, so only the others are elements (`<i>`s whose classes say
/// their bins and shades), since pages have thousands of blots.
pub fn blot(recency: &Recency, label: Option<&str>, attributes: &str) -> String {
    let title = match label {
        Some(label) => format!("{}\n{}", label, recency.describe()),
        None => recency.describe(),
    };
    let mut html = format!(
        r#"<span{} class="query-recency" title="{}">"#,
        attributes,
        title
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
            .replace('\n', "&#10;")
    );
    for bin in 0..BINS {
        let level = recency.level(bin);
        if level > 0 {
            let _ = write!(html, r#"<i class="b{} r{}"></i>"#, bin, level);
        }
    }
    html.push_str("</span>");
    html
}

/// A sparkline of a digest: a bar per bin of age (newest first), as tall as how
/// much changed then (logarithmically, relative to the bin that changed most).
pub fn sparkline(recency: &Recency) -> String {
    const BAR: u32 = 3;
    const GAP: u32 = 1;
    const HEIGHT: f64 = 12.0;
    let most = recency.0.iter().copied().max().unwrap_or(0);
    let mut svg = format!(
        r#"<svg class="recency-sparkline" width="{}" height="{}" aria-hidden="true">"#,
        BINS as u32 * (BAR + GAP) - GAP,
        HEIGHT
    );
    for (bin, &tokens) in recency.0.iter().enumerate() {
        if tokens == 0 {
            continue;
        }
        let height = 1.0 + (tokens as f64).ln_1p() / (most as f64).ln_1p() * (HEIGHT - 1.0);
        let _ = write!(
            svg,
            r#"<rect x="{}" y="{:.1}" width="{}" height="{:.1}"/>"#,
            bin as u32 * (BAR + GAP),
            HEIGHT - height,
            BAR,
            height
        );
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blot_cell() {
        let recency = Recency([150, 0, 0, 0, 0, 3, 0, 0, 0, 0]);
        let cell = blot_cell(
            Some(&recency),
            Some("History of Foo<T>::Bar\nfrom history context ns in a.cpp"),
        );
        assert_eq!(
            cell,
            r#"<span role="cell" class="query-recency" title="History of Foo&lt;T>::Bar&#10;from history context ns in a.cpp&#10;Tokens changed: 150 under 1 week ago, 3 3-6 months ago"><i class="b0 r3"></i><i class="b5 r1"></i></span>"#
        );
        assert_eq!(
            blot_cell(None, None),
            r#"<span role="cell" class="query-recency"></span>"#
        );
    }

    #[test]
    fn test_sparkline() {
        let svg = sparkline(&Recency([100, 0, 0, 0, 0, 0, 0, 0, 0, 1]));
        // The bin that changed most is full height, and others are at least
        // 1px (and empty bins have no bars).
        assert!(svg.contains(r#"<rect x="0" y="0.0" width="3" height="12.0"/>"#));
        assert!(svg.contains(r#"<rect x="36" y="#));
        assert_eq!(svg.matches("<rect").count(), 2);
    }
}
