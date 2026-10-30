//! The literate view of a Bugzilla comment (the `bug-comment:` query term's,
//! through the `bug-comment` command, which the DWIM rule "bugzilla-comment"
//! sends Bugzilla comments' URLs to; see query/dwim.toml): the comment's
//! blocks (its paragraphs, its lists' items, its code blocks), each beside
//! the code it links to, as the crash view puts frames beside their code (see
//! `crash_stack`).
//!
//! A comment (Bugzilla's REST API's `raw_text`, Markdown) is split into its
//! top-level blocks, each list's items being blocks of their own, and its
//! links (and its bare URLs, which Bugzilla links) found: searchfox's (ex:
//! `https://searchfox.org/mozilla-central/rev/<rev>/xpcom/build/XPCOMInit.cpp#257-260`),
//! hg.mozilla.org's and GitHub's links to files' lines are excerpts of the
//! tree's files, their lines mapped from the links' revisions to the indexed
//! one's (see `AbstractServer::line_map`; a searchfox link's revision may be
//! the tree's old repository's, an hg link's an hg revision, mapped by
//! `AbstractServer::map_rev`), with the linked lines highlighted, saying if
//! they changed since; and crash-stats reports' links are links to their
//! crash views (see `crash_stack::crash_report_stack`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::Arc;

use lazy_static::lazy_static;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;
use serde::Serialize;
use ustr::{Ustr, UstrMap, ustr};

use crate::abstract_server::{
    AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError, sym_info,
};
use crate::file_format::jumpref::JumprefData;
use crate::rendered_docs::escape;

/// Where Bugzilla is, unless the config says (`Config::bugzilla_url`, ex:
/// tests' fixtures, `file://...`, whose files are the REST API's paths with
/// `.json`, without their queries; see `fetch_bugzilla`).
pub const BUGZILLA_URL: &str = "https://bugzilla.mozilla.org";

fn problem(message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::BadInput,
        message,
    })
}

/// Bugzilla's REST API's response for `path` (ex: "rest/bug/1915788/comment")
/// at `base` (see `BUGZILLA_URL`): one of the last few asked for, if it is.
pub async fn fetch_bugzilla(base: &str, path: &str) -> Result<Arc<serde_json::Value>> {
    use std::sync::{Mutex, OnceLock};
    static RECENT: Mutex<Vec<(String, Arc<serde_json::Value>)>> = Mutex::new(vec![]);
    const RECENT_RESPONSES: usize = 16;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let url = format!("{}/{}", base.trim_end_matches('/'), path);
    let recent = RECENT
        .lock()
        .unwrap()
        .iter()
        .find(|(recent, _)| *recent == url)
        .map(|(_, response)| response.clone());
    if let Some(response) = recent {
        return Ok(response);
    }
    let text = match url.strip_prefix("file://") {
        Some(file) => {
            let file = format!("{}.json", file.split('?').next().unwrap_or(file));
            std::fs::read_to_string(&file)
                .map_err(|e| problem(format!("Couldn't read {}: {}", file, e)))?
        }
        None => {
            let client = CLIENT.get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(concat!("searchfox/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .unwrap_or_default()
            });
            let fetched = async {
                client
                    .get(&url)
                    .header("Accept", "application/json")
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await
            };
            fetched.await.map_err(|e| {
                problem(match e.status() {
                    Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN) => {
                        "Bugzilla doesn't show that bug to searchfox (it's not public)".to_string()
                    }
                    Some(reqwest::StatusCode::NOT_FOUND) => "Bugzilla has no such bug".to_string(),
                    _ => format!("Couldn't get {} from Bugzilla: {}", path, e),
                })
            })?
        }
    };
    let response: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| problem(format!("Bugzilla's {} isn't JSON: {}", path, e)))?;
    let response = Arc::new(response);
    let mut recent = RECENT.lock().unwrap();
    recent.push((url, response.clone()));
    if recent.len() > RECENT_RESPONSES {
        recent.remove(0);
    }
    Ok(response)
}

/// A comment of a bug: the bug's number, the comment's (0 is the bug's
/// description), and of the bug, its summary and where and what it is (ex:
/// "Core :: DOM: Content Processes, RESOLVED WORKSFORME"), if Bugzilla says;
/// and the comment's author and time, and its blocks.
#[derive(Debug, Serialize)]
pub struct BugComment {
    pub bug: u64,
    pub comment: u32,
    pub summary: Option<String>,
    pub about: Option<String>,
    pub author: Option<String>,
    pub time: Option<String>,
    pub blocks: Vec<CommentBlock>,
}

/// A block of a comment: its HTML, and the cards beside it, for its links.
#[derive(Debug, Serialize)]
pub struct CommentBlock {
    pub html: String,
    pub cards: Vec<CommentCard>,
}

/// A card beside a comment's block: a code link's excerpt, or a link to a
/// crash report's crash view.
#[derive(Debug, Serialize)]
pub enum CommentCard {
    Code(CodeExcerpt),
    Crash { id: String },
}

/// A link to a file's lines (see `code_link`): the link, the file's path, the
/// revision it's of (if it's a permalink) and its kind ("git" or "hg"), and
/// the lines (ranges, as the link has them).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodeLink {
    pub url: String,
    pub path: String,
    pub rev: Option<String>,
    pub rev_kind: &'static str,
    pub lines: Vec<(u32, u32)>,
}

/// How a code link's lines are its lines in the indexed revision.
#[derive(Debug, PartialEq, Serialize)]
pub enum LineMapping {
    /// Mapped from its revision (the tree's, ex: an old repository's
    /// revision's or an hg revision's git revision).
    Mapped { tree_rev: String },
    /// Its lines are now's (it's not a permalink).
    Now,
    /// The tree's repository doesn't have its revision (or the file at it),
    /// so its lines are now's.
    UnknownRev,
    /// The server can't tell (ex: a remote one), so its lines are now's.
    Unmapped,
}

/// A code link's excerpt: the link, how its lines are mapped, its lines now
/// (those unchanged since), how many changed since, where its lines are now
/// (the first and last; a changed line's where it would be, after the last
/// line before it which didn't change), and the excerpt's rows (see
/// `crash_stack::FrameRow`).
#[derive(Debug, Serialize)]
pub struct CodeExcerpt {
    pub link: CodeLink,
    pub mapping: LineMapping,
    pub lines_now: Vec<u32>,
    pub changed: usize,
    pub around: Option<(u32, u32)>,
    pub rows: Vec<crate::crash_stack::FrameRow>,
}

lazy_static! {
    /// A bare URL in a comment's text (which Bugzilla links), without its
    /// trailing punctuation.
    static ref BARE_URL: Regex =
        Regex::new(r#"https?://[^\s<>"'\[\](){}]+[^\s<>"'\[\](){}.,;:!?]"#).unwrap();
    static ref SEARCHFOX_LINK: Regex = Regex::new(
        r"^https?://searchfox\.org/[^/]+/(?:source|rev/(?P<rev>[0-9a-f]{7,40}))/(?P<path>[^?#]+)(?:\?[^#]*)?(?:#(?P<lines>[0-9][0-9,-]*))?$"
    )
    .unwrap();
    static ref HG_LINK: Regex = Regex::new(
        r"^https?://hg(?:-edge)?\.mozilla\.org/[^?#]+?/(?:file|annotate)/(?P<rev>[0-9a-f]{12,40})/(?P<path>[^?#]+)(?:\?[^#]*)?(?:#l(?P<line>\d+))?$"
    )
    .unwrap();
    static ref GITHUB_LINK: Regex = Regex::new(
        r"^https?://github\.com/[^/]+/[^/]+/blob/(?P<ref>[^/]+)/(?P<path>[^?#]+)(?:\?[^#]*)?(?:#L(?P<start>\d+)(?:-L(?P<end>\d+))?)?$"
    )
    .unwrap();
    static ref CRASH_LINK: Regex = Regex::new(
        r"^https?://crash-stats\.mozilla\.org/report/index/(?:bp-)?(?P<id>[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"
    )
    .unwrap();
}

/// The file and lines a link links to, if it's a code link: searchfox's (its
/// fragment's lines, ex: "216-221,240-241"), hg.mozilla.org's (`#l12`) or
/// GitHub's (`#L12-L20`; a permalink if its ref is a commit's).
pub fn code_link(url: &str) -> Option<CodeLink> {
    let decode = |path: &str| {
        urlencoding::decode(path)
            .map(|path| path.into_owned())
            .unwrap_or_else(|_| path.to_string())
    };
    if let Some(captures) = SEARCHFOX_LINK.captures(url) {
        let lines = captures
            .name("lines")
            .map(|lines| {
                lines
                    .as_str()
                    .split(',')
                    .filter_map(|range| {
                        let (start, end) = range.split_once('-').unwrap_or((range, range));
                        let (start, end) = (start.parse().ok()?, end.parse().ok()?);
                        (start > 0 && start <= end).then_some((start, end))
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Some(CodeLink {
            url: url.to_string(),
            path: decode(&captures["path"]),
            rev: captures.name("rev").map(|rev| rev.as_str().to_string()),
            rev_kind: "git",
            lines,
        });
    }
    if let Some(captures) = HG_LINK.captures(url) {
        let line: Option<u32> = captures.name("line").and_then(|l| l.as_str().parse().ok());
        return Some(CodeLink {
            url: url.to_string(),
            path: decode(&captures["path"]),
            rev: Some(captures["rev"].to_string()),
            rev_kind: "hg",
            lines: line.map(|line| vec![(line, line)]).unwrap_or_default(),
        });
    }
    if let Some(captures) = GITHUB_LINK.captures(url) {
        let start: Option<u32> = captures.name("start").and_then(|l| l.as_str().parse().ok());
        let end: Option<u32> = captures.name("end").and_then(|l| l.as_str().parse().ok());
        let reference = &captures["ref"];
        let is_commit = reference.len() == 40 && reference.bytes().all(|b| b.is_ascii_hexdigit());
        return Some(CodeLink {
            url: url.to_string(),
            path: decode(&captures["path"]),
            rev: is_commit.then(|| reference.to_string()),
            rev_kind: "git",
            lines: start
                .map(|start| vec![(start, end.unwrap_or(start).max(start))])
                .unwrap_or_default(),
        });
    }
    None
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Whether a link's URL may be one in the view (http, https or mailto).
fn safe_url(url: &str) -> bool {
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| url.to_ascii_lowercase().starts_with(scheme))
}

/// `text`'s HTML, with its bare URLs linked, adding them to `urls`.
fn autolinked(text: &str, urls: &mut Vec<String>) -> String {
    let mut html = String::new();
    let mut at = 0;
    for found in BARE_URL.find_iter(text) {
        html.push_str(&escape(&text[at..found.start()]));
        html.push_str(&format!(
            r#"<a href="{}">{}</a>"#,
            escape(found.as_str()),
            escape(found.as_str())
        ));
        urls.push(found.as_str().to_string());
        at = found.end();
    }
    html.push_str(&escape(&text[at..]));
    html
}

/// A comment's Markdown's top-level blocks (each list's items being blocks
/// of their own, in their lists, keeping their numbers), as HTML (raw HTML
/// shown as text; soft line breaks kept, as Bugzilla does), with their links'
/// URLs.
pub fn comment_blocks(text: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks = vec![];
    let mut html = String::new();
    let mut urls: Vec<String> = vec![];
    // (The depth of the containers the events are in, the top-level list
    // an item is in, as its opening tag, and the next item's number.)
    let mut depth = 0;
    let mut top_list: Option<(bool, u64)> = None;
    // (The links' closings: an unsafe link's is a span's.)
    let mut link_ends: Vec<&'static str> = vec![];
    let mut in_code = false;
    let mut cell_tags: Vec<&'static str> = vec![];
    let finish =
        |html: &mut String, urls: &mut Vec<String>, blocks: &mut Vec<(String, Vec<String>)>| {
            if !html.trim().is_empty() {
                blocks.push((std::mem::take(html), std::mem::take(urls)));
            }
            html.clear();
            urls.clear();
        };
    for event in Parser::new_ext(text, options()) {
        match event {
            Event::Start(tag) => {
                // (A top-level list's items are blocks, in a list of their
                // own, numbered as they were.)
                if depth == 0 {
                    if let Tag::List(start) = &tag {
                        top_list = Some((start.is_some(), start.unwrap_or(1)));
                        depth += 1;
                        continue;
                    }
                }
                if depth == 1
                    && let (Tag::Item, Some((ordered, number))) = (&tag, top_list)
                {
                    html.push_str(&match ordered {
                        true => format!(r#"<ol start="{}"><li>"#, number),
                        false => "<ul><li>".to_string(),
                    });
                    top_list = Some((ordered, number + 1));
                    depth += 1;
                    continue;
                }
                depth += 1;
                match tag {
                    Tag::Paragraph => html.push_str("<p>"),
                    Tag::Heading { level, .. } => html.push_str(&format!("<{}>", level)),
                    Tag::BlockQuote(_) => html.push_str("<blockquote>"),
                    Tag::CodeBlock(_) => {
                        in_code = true;
                        html.push_str("<pre><code>");
                    }
                    Tag::List(Some(start)) => html.push_str(&format!(r#"<ol start="{}">"#, start)),
                    Tag::List(None) => html.push_str("<ul>"),
                    Tag::Item => html.push_str("<li>"),
                    Tag::Emphasis => html.push_str("<em>"),
                    Tag::Strong => html.push_str("<strong>"),
                    Tag::Strikethrough => html.push_str("<del>"),
                    Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                        if safe_url(&dest_url) {
                            urls.push(dest_url.to_string());
                            html.push_str(&format!(r#"<a href="{}">"#, escape(&dest_url)));
                            link_ends.push("</a>");
                        } else {
                            html.push_str("<span>");
                            link_ends.push("</span>");
                        }
                    }
                    Tag::Table(_) => html.push_str("<table>"),
                    Tag::TableHead => {
                        cell_tags.push("th");
                        html.push_str("<thead><tr>");
                    }
                    Tag::TableRow => html.push_str("<tr>"),
                    Tag::TableCell => {
                        html.push_str(&format!("<{}>", cell_tags.last().copied().unwrap_or("td")))
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                depth -= 1;
                if depth == 0 && matches!(tag, TagEnd::List(_)) {
                    top_list = None;
                    continue;
                }
                if depth == 1 && top_list.is_some() && matches!(tag, TagEnd::Item) {
                    html.push_str(match top_list {
                        Some((true, _)) => "</li></ol>",
                        _ => "</li></ul>",
                    });
                    finish(&mut html, &mut urls, &mut blocks);
                    continue;
                }
                match tag {
                    TagEnd::Paragraph => html.push_str("</p>"),
                    TagEnd::Heading(level) => html.push_str(&format!("</{}>", level)),
                    TagEnd::BlockQuote(_) => html.push_str("</blockquote>"),
                    TagEnd::CodeBlock => {
                        in_code = false;
                        html.push_str("</code></pre>");
                    }
                    TagEnd::List(true) => html.push_str("</ol>"),
                    TagEnd::List(false) => html.push_str("</ul>"),
                    TagEnd::Item => html.push_str("</li>"),
                    TagEnd::Emphasis => html.push_str("</em>"),
                    TagEnd::Strong => html.push_str("</strong>"),
                    TagEnd::Strikethrough => html.push_str("</del>"),
                    TagEnd::Link | TagEnd::Image => {
                        html.push_str(link_ends.pop().unwrap_or("</a>"));
                    }
                    TagEnd::Table => html.push_str("</table>"),
                    TagEnd::TableHead => {
                        cell_tags.pop();
                        html.push_str("</tr></thead>");
                    }
                    TagEnd::TableRow => html.push_str("</tr>"),
                    TagEnd::TableCell => {
                        html.push_str(&format!("</{}>", cell_tags.last().copied().unwrap_or("td")))
                    }
                    _ => {}
                }
                if depth == 0 {
                    finish(&mut html, &mut urls, &mut blocks);
                }
            }
            Event::Text(text) => {
                if !link_ends.is_empty() || in_code {
                    html.push_str(&escape(&text));
                } else {
                    html.push_str(&autolinked(&text, &mut urls));
                }
            }
            Event::Code(code) => html.push_str(&format!("<code>{}</code>", escape(&code))),
            Event::Html(raw) | Event::InlineHtml(raw) => html.push_str(&escape(&raw)),
            Event::SoftBreak => html.push_str("<br>"),
            Event::HardBreak => html.push_str("<br>"),
            Event::Rule => {
                html.push_str("<hr>");
                if depth == 0 {
                    finish(&mut html, &mut urls, &mut blocks);
                }
            }
            Event::TaskListMarker(checked) => html.push_str(if checked { "☑ " } else { "☐ " }),
            _ => {}
        }
    }
    finish(&mut html, &mut urls, &mut blocks);
    blocks
}

/// How many lines around a link's lines its excerpt shows, and at most how
/// many lines an excerpt has, and how many code links a comment's excerpts
/// are of (the rest are links).
const CONTEXT_LINES: u32 = 2;
const MAX_EXCERPT_LINES: u32 = 60;
const MAX_CODE_LINKS: usize = 40;

/// A code link's lines in the indexed revision: its revision's lines mapped
/// (trying it as the tree's revision, then as its old repository's, or as an
/// hg revision), or its lines as they are, if it's not a permalink, or can't
/// be mapped (see `LineMapping`); and for each of its lines, its line now,
/// if it's unchanged.
async fn map_link(
    server: &(dyn AbstractServer + Send + Sync),
    link: &CodeLink,
) -> (LineMapping, Option<Vec<Option<u32>>>) {
    let Some(rev) = &link.rev else {
        return (LineMapping::Now, None);
    };
    let candidates: Vec<Option<String>> = match link.rev_kind {
        "hg" => vec![server.map_rev("hg", rev).await.ok().flatten()],
        _ => vec![
            Some(rev.clone()),
            server.map_rev("old", rev).await.ok().flatten(),
        ],
    };
    for tree_rev in candidates.into_iter().flatten() {
        match server.line_map(&tree_rev, &link.path).await {
            Ok(Some(map)) => return (LineMapping::Mapped { tree_rev }, Some(map)),
            Err(ServerError::Unsupported) => return (LineMapping::Unmapped, None),
            _ => {}
        }
    }
    (LineMapping::UnknownRev, None)
}

/// The view of comment `comment` of bug `bug` (see the module's
/// documentation), its code links' excerpts from `server`'s tree.
pub async fn bug_comment(
    server: &(dyn AbstractServer + Send + Sync),
    bug: u64,
    comment: u32,
) -> Result<BugComment> {
    let comments = server
        .bugzilla_json(&format!("rest/bug/{}/comment", bug))
        .await?;
    let found = comments["bugs"][bug.to_string()]["comments"]
        .as_array()
        .and_then(|comments| {
            comments
                .iter()
                .find(|c| c["count"].as_u64() == Some(comment as u64))
        })
        .ok_or_else(|| problem(format!("Bug {} has no comment {}", bug, comment)))?;
    let text = found["raw_text"]
        .as_str()
        .or_else(|| found["text"].as_str())
        .unwrap_or_default()
        .to_string();
    let author = found["creator"].as_str().map(str::to_string);
    let time = found["creation_time"].as_str().map(str::to_string);
    // (The bug's summary and where it is, if Bugzilla says.)
    let info = server
        .bugzilla_json(&format!(
            "rest/bug/{}?include_fields=summary,product,component,status,resolution",
            bug
        ))
        .await
        .ok();
    let bug_info = info.as_ref().map(|info| &info["bugs"][0]);
    let field = |name: &str| {
        bug_info
            .and_then(|info| info[name].as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let summary = field("summary");
    let about = match (field("product"), field("component"), field("status")) {
        (Some(product), Some(component), status) => Some(format!(
            "{} :: {}{}",
            product,
            component,
            status.map_or_else(String::new, |status| match field("resolution") {
                Some(resolution) => format!(", {} {}", status, resolution),
                None => format!(", {}", status),
            })
        )),
        _ => None,
    };

    // The blocks, and their cards: their code links' excerpts (once each,
    // at most MAX_CODE_LINKS), and their crash reports.
    let mut code_links = 0;
    let mut blocks = vec![];
    let mut maps: BTreeMap<
        (Option<String>, &'static str, String),
        (LineMapping, Option<Vec<Option<u32>>>),
    > = BTreeMap::new();
    for (html, urls) in comment_blocks(&text) {
        let mut cards = vec![];
        let mut seen = BTreeSet::new();
        for url in urls {
            if !seen.insert(url.clone()) {
                continue;
            }
            if let Some(captures) = CRASH_LINK.captures(&url) {
                cards.push(CommentCard::Crash {
                    id: captures["id"].to_string(),
                });
                continue;
            }
            let Some(link) = code_link(&url).filter(|link| !link.lines.is_empty()) else {
                continue;
            };
            if code_links >= MAX_CODE_LINKS {
                continue;
            }
            code_links += 1;
            let key = (link.rev.clone(), link.rev_kind, link.path.clone());
            if !maps.contains_key(&key) {
                let mapped = map_link(server, &link).await;
                maps.insert(key.clone(), mapped);
            }
            let (mapping, map) = &maps[&key];
            // Its lines now (those unchanged), and where the changed ones'
            // would be (after the last line before them which didn't).
            let mut lines_now = vec![];
            let mut changed = 0;
            let mut span: Option<(u32, u32)> = None;
            for &(start, end) in &link.lines {
                for line in start..=end.min(start + MAX_EXCERPT_LINES) {
                    let now = match map {
                        None => Some(line),
                        Some(map) => map.get(line as usize - 1).copied().flatten(),
                    };
                    let at = now.unwrap_or_else(|| {
                        let map = map.as_ref().unwrap();
                        (1..line)
                            .rev()
                            .find_map(|before| {
                                Some(
                                    map.get(before as usize - 1).copied().flatten()?
                                        + (line - before),
                                )
                            })
                            .unwrap_or(line)
                    });
                    match now {
                        Some(now) => lines_now.push(now),
                        None => changed += 1,
                    }
                    span = Some(span.map_or((at, at), |(a, b)| (a.min(at), b.max(at))));
                }
            }
            let mapping = match mapping {
                LineMapping::Mapped { tree_rev } => LineMapping::Mapped {
                    tree_rev: tree_rev.clone(),
                },
                LineMapping::Now => LineMapping::Now,
                LineMapping::UnknownRev => LineMapping::UnknownRev,
                LineMapping::Unmapped => LineMapping::Unmapped,
            };
            cards.push(CommentCard::Code(CodeExcerpt {
                link,
                mapping,
                lines_now,
                changed,
                around: span,
                rows: vec![],
            }));
        }
        blocks.push(CommentBlock { html, cards });
    }

    // The excerpts' rows: around their lines (now), folded between.
    let mut wanted: Vec<(usize, usize, Ustr, BTreeSet<u32>, BTreeSet<u32>)> = vec![];
    for (b, block) in blocks.iter().enumerate() {
        for (c, card) in block.cards.iter().enumerate() {
            let CommentCard::Code(excerpt) = card else {
                continue;
            };
            let Some((first, last)) = excerpt.around else {
                continue;
            };
            let start = first.saturating_sub(CONTEXT_LINES).max(1);
            let end = (last + CONTEXT_LINES).min(start + MAX_EXCERPT_LINES);
            let all: BTreeSet<u32> = (start..=end).collect();
            let mut shown: BTreeSet<u32> = match excerpt.lines_now.is_empty() {
                true => all.clone(),
                false => excerpt
                    .lines_now
                    .iter()
                    .flat_map(|line| line.saturating_sub(CONTEXT_LINES)..=line + CONTEXT_LINES)
                    .filter(|line| all.contains(line))
                    .collect(),
            };
            let hidden: Vec<u32> = all.difference(&shown).copied().collect();
            for run in crate::crash_stack::runs(hidden.into_iter()) {
                if run.len() < 3 {
                    shown.extend(run);
                }
            }
            wanted.push((b, c, ustr(&excerpt.link.path), all, shown));
        }
    }
    let mut requests: BTreeMap<Ustr, BTreeSet<u32>> = BTreeMap::new();
    for (_, _, path, all, _) in &wanted {
        requests.entry(*path).or_default().extend(all);
    }
    let rows = crate::crash_stack::fetch_rows(server, requests).await;
    for (b, c, path, all, shown) in wanted {
        let Some(file_rows) = rows.get(&path) else {
            continue;
        };
        let CommentCard::Code(excerpt) = &mut blocks[b].cards[c] else {
            continue;
        };
        for line in all {
            let Some(row) = file_rows.get(&line) else {
                continue;
            };
            excerpt.rows.push(crate::crash_stack::FrameRow {
                line,
                html: row.html.clone(),
                folded: !shown.contains(&line),
                crashing: excerpt.lines_now.contains(&line),
            });
        }
    }

    Ok(BugComment {
        bug,
        comment,
        summary,
        about,
        author,
        time,
        blocks,
    })
}

/// The SYM_INFO of the view's excerpts' symbols (for their context menus).
pub async fn comment_sym_info(
    server: &(dyn AbstractServer + Send + Sync),
    view: &BugComment,
) -> BTreeMap<Ustr, Option<JumprefData>> {
    let syms: Vec<Ustr> = view
        .blocks
        .iter()
        .flat_map(|block| &block.cards)
        .flat_map(|card| match card {
            CommentCard::Code(excerpt) => excerpt.rows.iter().collect::<Vec<_>>(),
            CommentCard::Crash { .. } => vec![],
        })
        .flat_map(|row| sym_info::html_symbols(&row.html))
        .collect();
    let mut info = BTreeMap::new();
    let mut traversed = UstrMap::default();
    sym_info::add_syms_info(server, syms, &mut info, &mut traversed).await;
    info
}

/// Lines' ranges, as "257-260, 270" (for "257, 258, 259, 260, 270").
fn ranges(lines: &[u32]) -> String {
    let sorted: BTreeSet<u32> = lines.iter().copied().collect();
    crate::crash_stack::runs(sorted.into_iter())
        .iter()
        .map(|run| match run.as_slice() {
            [line] => line.to_string(),
            run => format!("{}-{}", run[0], run[run.len() - 1]),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The view (see the module's documentation), for the query page (the
/// `bug-comment` command's HTML, which the query page shows as it is): its
/// notice, its blocks beside their cards, and its excerpts' SYM_INFO.
pub fn render_bug_comment(
    tree_name: &str,
    view: &BugComment,
    sym_info: &BTreeMap<Ustr, Option<JumprefData>>,
    writer: &mut dyn Write,
) -> std::result::Result<(), &'static str> {
    use crate::url_encode_path::url_encode_path;

    let comment_url = format!(
        "{}/show_bug.cgi?id={}#c{}",
        BUGZILLA_URL, view.bug, view.comment
    );
    let mut html = String::new();
    html.push_str(r#"<div class="crash-view bug-comment-view">"#);
    let excerpts = view
        .blocks
        .iter()
        .flat_map(|block| &block.cards)
        .filter(|card| matches!(card, CommentCard::Code(_)))
        .count();
    html.push_str(&format!(
        r#"<div class="crash-notice"><p><a class="bug-comment-link" href="{}">Comment {}</a> on bug {}{}{}{}{}: its blocks, each beside the code it links to in {} at the indexed revision ({} excerpt{}), with the linked lines highlighted (mapped from the links' revisions).</p></div>"#,
        escape(&comment_url),
        view.comment,
        view.bug,
        view.summary
            .as_ref()
            .map_or_else(String::new, |summary| format!(", <i>{}</i>", escape(summary))),
        view.about
            .as_ref()
            .map_or_else(String::new, |about| format!(" ({})", escape(about))),
        view.author
            .as_ref()
            .map_or_else(String::new, |author| format!(", by {}", escape(author))),
        view.time
            .as_ref()
            .map_or_else(String::new, |time| format!(" on {}", escape(&time[..time.len().min(10)]))),
        escape(tree_name),
        excerpts,
        if excerpts == 1 { "" } else { "s" }
    ));
    html.push_str(r#"<div class="crash-stack bug-comment-blocks" role="list">"#);
    for (i, block) in view.blocks.iter().enumerate() {
        html.push_str(&format!(
            r#"<section class="crash-frame bug-comment-block" role="listitem" id="block-{}">"#,
            i
        ));
        html.push_str(r#"<div class="bug-comment-cards">"#);
        for card in &block.cards {
            match card {
                CommentCard::Crash { id } => html.push_str(&format!(
                    r#"<div class="bug-comment-card bug-comment-crash"><a href="/{}/query/default?q=crash-report%3A{}">The crash report {}'s stack, beside its code</a></div>"#,
                    escape(tree_name),
                    escape(id),
                    escape(id)
                )),
                CommentCard::Code(excerpt) => {
                    html.push_str(&excerpt_html(tree_name, excerpt, &url_encode_path(&excerpt.link.path)));
                }
            }
        }
        html.push_str("</div>");
        html.push_str(&format!(
            r#"<div class="bug-comment-text">{}</div></section>"#,
            block.html
        ));
    }
    html.push_str("</div></div>");
    writer
        .write_all(html.as_bytes())
        .map_err(|_| "Write error")?;
    writeln!(
        writer,
        r#"<script>window.SYM_INFO = Object.assign(window.SYM_INFO || {{}}, {});
// (An excerpt's folded lines show on request.)
document.addEventListener("click", event => {{
  const unfold = event.target.closest(".crash-unfold");
  if (!unfold) {{
    return;
  }}
  unfold.previousElementSibling.hidden = false;
  unfold.remove();
}});</script>"#,
        serde_json::to_string(sym_info)
            .unwrap_or_else(|_| "{}".to_string())
            .replace('<', "\\u003c")
    )
    .map_err(|_| "Write error")?;
    Ok(())
}

/// A code link's card: its heading (the file and its lines now, linked to the
/// tree's, and the link as it was), what its lines are, and its excerpt.
fn excerpt_html(tree_name: &str, excerpt: &CodeExcerpt, encoded_path: &str) -> String {
    let link = &excerpt.link;
    let url = format!("/{}/source/{}", tree_name, encoded_path);
    let linked: Vec<u32> = link.lines.iter().flat_map(|&(a, b)| a..=b).collect();
    let now = ranges(&excerpt.lines_now);
    let mut html = String::from(r#"<div class="bug-comment-card">"#);
    // (Its lines now, or if they all changed since, near where they would
    // be.)
    let (anchor, shown) = match (excerpt.lines_now.first(), excerpt.around) {
        (Some(first), _) => (*first, format!("{}#{}", link.path, now)),
        (None, Some((near, _))) => (near, format!("{}, near line {}", link.path, near)),
        (None, None) => (1, link.path.clone()),
    };
    html.push_str(&format!(
        r#"<div class="crash-code-heading"><a href="{}#{}">{}</a> <a class="bug-comment-as-linked" href="{}">as linked</a></div>"#,
        escape(&url),
        anchor,
        escape(&shown),
        escape(&link.url)
    ));
    let then = ranges(&linked);
    let note = match &excerpt.mapping {
        LineMapping::Mapped { .. } if excerpt.changed == linked.len() => {
            r#"<span class="crash-frame-changed">Its lines have changed since.</span>"#.to_string()
        }
        LineMapping::Mapped { .. } if excerpt.changed > 0 => format!(
            r#"<span class="crash-frame-changed">{} of its {} lines changed since</span>{}."#,
            excerpt.changed,
            linked.len(),
            if now != then {
                format!(" (lines {} then)", escape(&then))
            } else {
                String::new()
            }
        ),
        LineMapping::Mapped { .. } if now != then => {
            format!("Lines {} at its revision.", escape(&then))
        }
        LineMapping::Mapped { .. } => String::new(),
        LineMapping::Now => "Not a permalink: its lines now.".to_string(),
        LineMapping::UnknownRev => {
            "This tree doesn't have its revision: its line numbers, now.".to_string()
        }
        LineMapping::Unmapped => "Whether its lines changed since isn't known here.".to_string(),
    };
    // (Not for a file the tree doesn't have.)
    if !note.is_empty() && !excerpt.rows.is_empty() {
        html.push_str(&format!(r#"<div class="crash-frame-where">{}</div>"#, note));
    }
    if excerpt.rows.is_empty() {
        html.push_str(&format!(
            r#"<div class="crash-code-missing">{}</div>"#,
            "Not a file in this tree."
        ));
        html.push_str("</div>");
        return html;
    }
    html.push_str(&format!(
        r#"<div class="file crash-rows" role="table" data-tree="{}" data-search="{}">"#,
        escape(tree_name),
        escape(&crate::file_format::config::search_url(tree_name))
    ));
    let mut folded: Vec<&crate::crash_stack::FrameRow> = vec![];
    let flush = |run: &mut Vec<&crate::crash_stack::FrameRow>, html: &mut String| {
        if run.is_empty() {
            return;
        }
        html.push_str(&format!(
            r#"<div class="crash-fold" role="rowgroup" hidden>{}</div><button type="button" class="crash-unfold">Show {} more line{}</button>"#,
            run.iter()
                .map(|row| crate::format::excerpt_row_html(&row.html, row.line, &url, "sp-citation-lineno", 0, None))
                .collect::<String>(),
            run.len(),
            if run.len() == 1 { "" } else { "s" }
        ));
        run.clear();
    };
    for row in &excerpt.rows {
        if row.folded {
            folded.push(row);
            continue;
        }
        flush(&mut folded, &mut html);
        html.push_str(&crate::format::excerpt_row_html(
            &row.html,
            row.line,
            &url,
            "sp-citation-lineno",
            0,
            row.crashing.then_some("crash-line"),
        ));
    }
    flush(&mut folded, &mut html);
    html.push_str("</div></div>");
    html
}

/// A `bug-comment:` term's bug and comment: "1915788/20", "1915788#c20" or
/// "1915788" (its comment 0, the bug's description).
pub fn parse_bug_comment(value: &str) -> Option<(u64, u32)> {
    let value = value.trim();
    let (bug, comment) = match value.split_once(['/', '#']) {
        Some((bug, comment)) => (bug, comment.trim_start_matches('c')),
        None => (value, "0"),
    };
    Some((bug.parse().ok()?, comment.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_link() {
        let link = code_link("https://searchfox.org/mozilla-central/rev/cb5faf5dd5176494302068c553da97b4d08aa339/xpcom/build/Omnijar.cpp#216-221,240-241").unwrap();
        assert_eq!(link.path, "xpcom/build/Omnijar.cpp");
        assert_eq!(
            link.rev.as_deref(),
            Some("cb5faf5dd5176494302068c553da97b4d08aa339")
        );
        assert_eq!(link.lines, [(216, 221), (240, 241)]);
        let link =
            code_link("https://searchfox.org/firefox-main/source/dom/ipc/ContentChild.cpp#753")
                .unwrap();
        assert_eq!((link.rev, link.lines), (None, vec![(753, 753)]));
        let link =
            code_link("https://hg.mozilla.org/mozilla-central/file/0123456789ab/dom/Foo.cpp#l12")
                .unwrap();
        assert_eq!((link.rev_kind, link.lines), ("hg", vec![(12, 12)]));
        let link =
            code_link("https://github.com/mozilla-firefox/firefox/blob/main/dom/Foo.cpp#L12-L20")
                .unwrap();
        assert_eq!((link.rev, link.lines), (None, vec![(12, 20)]));
        assert_eq!(code_link("https://example.org/dom/Foo.cpp#L1"), None);
        assert_eq!(parse_bug_comment("1915788/20"), Some((1915788, 20)));
        assert_eq!(parse_bug_comment("1915788#c20"), Some((1915788, 20)));
        assert_eq!(parse_bug_comment("1915788"), Some((1915788, 0)));
        assert_eq!(parse_bug_comment("x/1"), None);
        assert_eq!(ranges(&[257, 258, 259, 260, 270]), "257-260, 270");
    }

    #[test]
    fn test_comment_blocks() {
        let text = "Going through `NS_InitXPCOM`:\n\n* https://searchfox.org/m/rev/abc1234/a.cpp#1-2 - first.\n* The [second](https://searchfox.org/m/rev/abc1234/b.cpp#3) one\n  * nested https://example.org/x.\n\n1. one\n2. two\n\n```\n0 libxul.so https://not-linked.example/\n```\n\n<b>raw</b>\nline two";
        let blocks = comment_blocks(text);
        let summary: Vec<(&str, Vec<&str>)> = blocks
            .iter()
            .map(|(html, urls)| (html.as_str(), urls.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(summary.len(), 7, "{:#?}", summary);
        assert_eq!(
            summary[0].0,
            "<p>Going through <code>NS_InitXPCOM</code>:</p>"
        );
        assert_eq!(
            summary[1],
            (
                r#"<ul><li><a href="https://searchfox.org/m/rev/abc1234/a.cpp#1-2">https://searchfox.org/m/rev/abc1234/a.cpp#1-2</a> - first.</li></ul>"#,
                vec!["https://searchfox.org/m/rev/abc1234/a.cpp#1-2"]
            )
        );
        // (A nested list stays in its item; its URLs are the item's.)
        assert!(summary[2].0.starts_with(r#"<ul><li>The <a href="https://searchfox.org/m/rev/abc1234/b.cpp#3">second</a> one<ul><li>nested <a href="https://example.org/x">"#));
        assert_eq!(
            summary[2].1,
            [
                "https://searchfox.org/m/rev/abc1234/b.cpp#3",
                "https://example.org/x"
            ]
        );
        // (Ordered lists' items keep their numbers.)
        assert_eq!(summary[3].0, r#"<ol start="1"><li>one</li></ol>"#);
        assert_eq!(summary[4].0, r#"<ol start="2"><li>two</li></ol>"#);
        // (Code blocks' URLs aren't links, and raw HTML is text.)
        assert!(
            blocks
                .iter()
                .all(|(_, urls)| !urls.iter().any(|u| u.contains("not-linked")))
        );
        assert!(
            blocks
                .iter()
                .any(|(html, _)| html.contains("&lt;b&gt;raw&lt;/b&gt;<br>line two"))
        );
    }
}
