//! Where a rendered spec page's blocks are, for search results' excerpts of
//! them (see `format::spec_excerpt_blocks`): each block's section (the
//! nearest heading before it, with its number), and if it's in an
//! algorithm's steps, the algorithm and the step, as a "found in", since a
//! spec's blocks can say the same thing in many places (ex: the HTML
//! Standard's notes "This will terminate the entire resolve a module
//! specifier algorithm, without any further fallbacks.").

use super::fragment;
use crate::rendered_docs::escape;
use crate::spec_analyze::markup::Doc;

/// A heading of a page: its node, where it starts, its level, its number
/// (its `sp-secno`), its title, and its id (or its ecmarkup clause's).
struct Heading {
    node: usize,
    start: usize,
    level: u8,
    number: Option<String>,
    title: String,
    id: Option<String>,
}

/// The "found in" of each of a rendered spec page's blocks (by where their
/// strips go in its `html`, `strip_ats`; see `Rendered::blocks`), as HTML,
/// its links to the page (`url`): its section, the nearest heading before
/// it (a heading's, the nearest before it of a higher level), its number
/// and its title, linked to its id; and if it's in an algorithm's steps (an
/// `<ol>`'s `<li>`s), the algorithm, the definition just before its list
/// (with its link to its view), linked to its id, and the step (ex: "1.1.1";
/// or of the section's algorithm, the step alone: an ecmarkup spec's
/// clause's, "3.a", or a heading defining its term's, as spec-analyze takes
/// the algorithm after it to be its); None if it has neither.
pub fn block_contexts(
    html: &str,
    strip_ats: &[usize],
    url: &str,
    ecmarkup: bool,
) -> Vec<Option<String>> {
    let doc = Doc::parse(html);
    let headings = headings(&doc);
    strip_ats
        .iter()
        .map(|&at| {
            let node = node_at(&doc, at)?;
            let mut found = vec![];
            if let Some(heading) = section(&headings, &doc, node, at) {
                let text = match &heading.number {
                    Some(number) => format!("§{} {}", number, heading.title),
                    None => heading.title.clone(),
                };
                found.push(match &heading.id {
                    Some(id) => format!(
                        r#"<a href="{}{}">{}</a>"#,
                        escape(url),
                        escape(&fragment(id)),
                        escape(&text)
                    ),
                    None => escape(&text),
                });
            }
            if let Some(step) = algorithm_step(&doc, &headings, node, url, ecmarkup) {
                found.push(step);
            }
            (!found.is_empty()).then(|| found.join(" › "))
        })
        .collect()
}

/// The page's headings, in order.
fn headings(doc: &Doc) -> Vec<Heading> {
    let mut headings = vec![];
    for node in 1..doc.nodes.len() {
        let level = match doc.name(node) {
            Some("h1") => 1,
            Some("h2") => 2,
            Some("h3") => 3,
            Some("h4") => 4,
            Some("h5") => 5,
            Some("h6") => 6,
            _ => continue,
        };
        let mut number = None;
        let mut title = String::new();
        for &child in &doc.nodes[node].children {
            if doc.is(child, "span") && doc.attr(child, "class").as_deref() == Some("sp-secno") {
                let text = doc.text_content(child);
                number = Some(text.trim().trim_end_matches('.').to_string())
                    .filter(|number| !number.is_empty());
            } else {
                title.push_str(&text_without_view_links(doc, child));
            }
        }
        let id = doc.attr(node, "id").or_else(|| {
            doc.ancestors(node).find_map(|ancestor| {
                let clause = doc
                    .attr(ancestor, "class")
                    .is_some_and(|class| class.split(' ').any(|c| c == "sp-emu-clause"));
                clause.then(|| doc.attr(ancestor, "id")).flatten()
            })
        });
        headings.push(Heading {
            node,
            start: doc.nodes[node].range.start,
            level,
            number,
            title: short_title(&title.split_whitespace().collect::<Vec<_>>().join(" ")),
            id,
        });
    }
    headings
}

/// A heading's title, shortened if it's a long signature (ex: ecmarkup's
/// "EvaluateImportCall ( specifierExpr: a Parse Node, ... ): either a normal
/// completion ..." is "EvaluateImportCall ( … )").
fn short_title(title: &str) -> String {
    const LONG: usize = 60;
    match title.split_once(" (") {
        Some((name, _)) if title.chars().count() > LONG => format!("{} ( … )", name),
        _ => title.to_string(),
    }
}

/// The node's text (see `Doc::text_content`), but its algorithms' links to
/// their views' ("⇄", `sp-algorithm-link`).
fn text_without_view_links(doc: &Doc, node: usize) -> String {
    let is_view_link = |n: usize| {
        doc.is(n, "a")
            && doc
                .attr(n, "class")
                .is_some_and(|class| class.split(' ').any(|c| c == "sp-algorithm-link"))
    };
    if is_view_link(node) {
        return String::new();
    }
    match doc.nodes[node].children.is_empty() {
        true => doc.text_content(node),
        false => doc.nodes[node]
            .children
            .iter()
            .map(|&child| text_without_view_links(doc, child))
            .collect(),
    }
}

/// The innermost node with the byte `at`.
fn node_at(doc: &Doc, at: usize) -> Option<usize> {
    let last = doc.nodes.partition_point(|node| node.range.start <= at);
    let mut node = last.checked_sub(1)?;
    while !(doc.nodes[node].range.start <= at && at < doc.nodes[node].range.end.max(at + 1)) {
        node = doc.nodes[node].parent?;
    }
    (node != 0).then_some(node)
}

/// The heading of the section of the block at `at` (whose innermost node is
/// `node`): the nearest before it, or for a heading's block, the nearest
/// before it of a higher level.
fn section<'h>(headings: &'h [Heading], doc: &Doc, node: usize, at: usize) -> Option<&'h Heading> {
    let before = &headings[..headings.partition_point(|heading| heading.start < at)];
    // (A heading's block's node is the heading, or the heading is its first
    // element.)
    let own = std::iter::once(node)
        .chain(doc.nodes[node].children.iter().copied())
        .find_map(|n| {
            headings
                .iter()
                .find(|heading| heading.start == doc.nodes[n].range.start)
        });
    match own {
        Some(own) => before
            .iter()
            .rev()
            .find(|heading| heading.level < own.level),
        None => before.last(),
    }
}

/// The algorithm and the step of the block whose innermost node is `node`,
/// if it's in an algorithm's steps, as HTML (see `block_contexts`).
fn algorithm_step(
    doc: &Doc,
    headings: &[Heading],
    node: usize,
    url: &str,
    ecmarkup: bool,
) -> Option<String> {
    // (Its outermost step's list.)
    let list = std::iter::once(node)
        .chain(doc.ancestors(node))
        .filter(|&n| doc.is(n, "li") && doc.nodes[n].parent.is_some_and(|p| doc.is(p, "ol")))
        .last()
        .and_then(|li| doc.nodes[li].parent)?;
    let path = crate::spec_analyze::wattsi::step_path(doc, node, doc.nodes[list].range.start);
    if path.is_empty() {
        return None;
    }
    // (An ecmarkup algorithm is its clause's, whose heading is the section.)
    if ecmarkup {
        return doc
            .ancestors(list)
            .any(|n| {
                doc.is(n, "emu-alg")
                    || doc
                        .attr(n, "class")
                        .is_some_and(|class| class.split(' ').any(|c| c == "sp-emu-alg"))
            })
            .then(|| format!("step {}", ecmarkup_step(&path)));
    }
    // (The definition just before the list, with its link to its view.)
    let parent = doc.nodes[list].parent?;
    let siblings = &doc.nodes[parent].children;
    let index = siblings.iter().position(|&n| n == list)?;
    let before = siblings[..index]
        .iter()
        .rev()
        .copied()
        .find(|&n| doc.name(n).is_some())?;
    let has_view_link = descendants(doc, before).any(|n| {
        doc.is(n, "a")
            && doc
                .attr(n, "class")
                .is_some_and(|class| class.split(' ').any(|c| c == "sp-algorithm-link"))
    });
    if !has_view_link {
        // (A heading defining its term's algorithm, after it: the step
        // alone, the heading being the section.)
        let start = doc.nodes[list].range.start;
        let heading =
            headings[..headings.partition_point(|heading| heading.start < start)].last()?;
        return descendants(doc, heading.node)
            .any(|n| doc.is(n, "dfn"))
            .then(|| format!("step {}", path));
    }
    let dfn = descendants(doc, before).find(|&n| doc.is(n, "dfn"))?;
    let name = doc
        .text_content(dfn)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name = match doc.attr(dfn, "id") {
        Some(id) => format!(
            r#"<a href="{}{}">{}</a>"#,
            escape(url),
            escape(&fragment(&id)),
            escape(&name)
        ),
        None => escape(&name),
    };
    Some(format!("{} step {}", name, path))
}

/// `node`'s descendants, in order.
fn descendants<'d>(doc: &'d Doc, node: usize) -> impl Iterator<Item = usize> + 'd {
    let range = doc.nodes[node].range.clone();
    (node + 1..doc.nodes.len()).take_while(move |&n| doc.nodes[n].range.start < range.end)
}

/// An ecmarkup step's number, as ecmarkup numbers its algorithms' steps: a
/// number, then a letter, then a roman numeral, and so on (ex: "3.1.2" is
/// "3.a.ii").
fn ecmarkup_step(path: &str) -> String {
    path.split('.')
        .enumerate()
        .map(|(depth, number)| {
            let Ok(n) = number.parse::<usize>() else {
                return number.to_string();
            };
            match depth % 3 {
                1 => alphabetic(n),
                2 => roman(n),
                _ => n.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// `n` (from 1) as a lowercase letter, then "aa", "ab", ...
fn alphabetic(n: usize) -> String {
    let mut n = n;
    let mut letters = vec![];
    while n > 0 {
        n -= 1;
        letters.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    letters.iter().rev().collect()
}

/// `n` (from 1) as a lowercase roman numeral.
fn roman(n: usize) -> String {
    const NUMERALS: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_contexts() {
        // (A page as a Wattsi spec's renders: headings with their numbers,
        // an algorithm's definition with its link to its view before its
        // steps, a note in a nested step, and a list which isn't an
        // algorithm's.)
        let html = concat!(
            r#"<div class="rd-block"><h2 id="resolution"><span class="sp-secno">8.1.5.3</span> The resolution algorithm</h2></div>"#,
            r#"<div class="rd-block"><p>Intro.</p></div>"#,
            r#"<div class="rd-block"><p>To <dfn id="resolving-an-imports-match">resolve an imports match</dfn><a class="sp-algorithm-link" href="/t/algorithm/x">⇄</a>:</p></div>"#,
            r#"<ol><li><div class="rd-block"><p>For each:</p></div><ol><li><div class="rd-block"><p>If so:</p></div>"#,
            r#"<ol><li><div class="rd-block"><p>Throw.</p></div><div class="rd-block"><p class="sp-note">This will terminate.</p></div></li></ol>"#,
            r#"</li></ol></li></ol>"#,
            r#"<div class="rd-block"><h3 id="sub"><span class="sp-secno">8.1.5.4</span> <code>Sub</code> section</h3></div>"#,
            r#"<ol><li><div class="rd-block"><p>Not an algorithm's.</p></div></li></ol>"#,
            r#"<div class="rd-block"><h4 id="hook"><span class="sp-secno">8.1.5.5</span> <dfn>Hook</dfn><a class="sp-algorithm-link" href="/t/algorithm/hook">⇄</a>(<var>x</var>)</h4></div>"#,
            r#"<div class="rd-block"><p>It must:</p></div><ol><li><div class="rd-block"><p>A.</p></div><ol><li><div class="rd-block"><p>B.</p></div></li></ol></li></ol>"#,
        );
        let strip_ats: Vec<usize> = html
            .match_indices(r#"<div class="rd-block">"#)
            .map(|(at, tag)| at + tag.len())
            .collect();
        let contexts = block_contexts(html, &strip_ats, "/t/rendered/s/p.html", false);
        let section =
            r##"<a href="/t/rendered/s/p.html#resolution">§8.1.5.3 The resolution algorithm</a>"##;
        let algorithm = r##"<a href="/t/rendered/s/p.html#resolving-an-imports-match">resolve an imports match</a>"##;
        assert_eq!(contexts[0], None, "(The first heading has no section.)");
        assert_eq!(contexts[1].as_deref(), Some(section));
        assert_eq!(contexts[2].as_deref(), Some(section));
        assert_eq!(
            contexts[3].as_deref(),
            Some(format!("{} › {} step 1", section, algorithm).as_str())
        );
        assert_eq!(
            contexts[6].as_deref(),
            Some(format!("{} › {} step 1.1.1", section, algorithm).as_str()),
            "A step's note is in its step"
        );
        assert_eq!(
            contexts[7].as_deref(),
            Some(section),
            "A heading's section is the one it's in"
        );
        assert_eq!(
            contexts[8].as_deref(),
            Some(r##"<a href="/t/rendered/s/p.html#sub">§8.1.5.4 Sub section</a>"##),
            "A list which isn't an algorithm's has no steps"
        );
        assert_eq!(
            contexts[12].as_deref(),
            Some(r##"<a href="/t/rendered/s/p.html#hook">§8.1.5.5 Hook(x)</a> › step 1.1"##),
            "A heading defining its term's algorithm's steps are the section's"
        );

        // (An ecmarkup spec's clause's algorithm, its steps numbered as
        // ecmarkup numbers them, and its heading's id its clause's.)
        let html = concat!(
            r#"<div id="sec-tostring" class="sp-emu-clause"><div class="rd-block"><h3><span class="sp-secno">7.1.17</span> ToString ( <var>arg</var> )</h3></div>"#,
            r#"<div class="sp-emu-alg"><ol><li><div class="rd-block">Step.</div><ol><li><div class="rd-block">Sub.</div><ol><li><div class="rd-block">Subsub.</div></li></ol></li></ol></li></ol></div></div>"#,
        );
        let strip_ats: Vec<usize> = html
            .match_indices(r#"<div class="rd-block">"#)
            .map(|(at, tag)| at + tag.len())
            .collect();
        let contexts = block_contexts(html, &strip_ats, "/t/rendered/ecma/spec.html", true);
        assert_eq!(
            contexts[3].as_deref(),
            Some(
                r##"<a href="/t/rendered/ecma/spec.html#sec-tostring">§7.1.17 ToString ( arg )</a> › step 1.a.i"##
            )
        );
        assert_eq!(ecmarkup_step("12.27.4.30"), "12.aa.iv.30");
        assert_eq!(
            short_title(
                "EvaluateImportCall ( specifierExpr: a Parse Node, optional optionsExpr: a Parse Node, ): a completion"
            ),
            "EvaluateImportCall ( … )"
        );
        assert_eq!(short_title("ToString ( arg )"), "ToString ( arg )");
    }
}
