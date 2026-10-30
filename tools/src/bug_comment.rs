//! The literate view of a Bugzilla comment (the `bug-comment:` query term's,
//! through the `bug-comment` command, which the DWIM rule "bugzilla-comment"
//! sends Bugzilla comments' URLs to; see query/dwim.toml): the comment as
//! Bugzilla shows it (its header, its author's name and roles, and its text,
//! as Markdown, or as plain text for comments Bugzilla shows so, ex: older
//! ones and bots'), split into its blocks (its paragraphs, its lists' items,
//! its code blocks), each beside the code it links to, as the crash view puts
//! frames beside their code (see `crash_stack`).
//!
//! A comment (Bugzilla's REST API's `raw_text`) is split into its top-level
//! blocks, each list's items being blocks of their own, and its links (and
//! its bare URLs, which Bugzilla links) found, and its mentions of bugs,
//! comments and attachments linked, as Bugzilla does: searchfox's (ex:
//! `https://searchfox.org/mozilla-central/rev/<rev>/xpcom/build/XPCOMInit.cpp#257-260`),
//! hg.mozilla.org's and GitHub's links to files' lines are excerpts of the
//! tree's files, their lines mapped from the links' revisions to the indexed
//! one's (see `AbstractServer::line_map`; a searchfox link's revision may be
//! the tree's old repository's, an hg link's an hg revision, mapped by
//! `AbstractServer::map_rev`), with the linked lines highlighted, saying if
//! they changed since.  crash-stats reports' links are cards of the reports
//! (what crashed), and blocks listing a crash's frames (crash-stats' bug
//! template's "Top 10 frames:", `N  MODULE  FUNCTION  PATH:LINE` lines) are
//! the frames beside their code: the report's (see
//! `crash_stack::crash_report_stack`), or if crash-stats doesn't have it any
//! more, the listed frames' lines at the tree's head when the crash was
//! submitted (its ID's date; see `AbstractServer::rev_at`), roughly, else
//! their functions, found by their names, as a signature's are (see
//! `crash_stack::crash_stack`).
//!
//! What the view couldn't get (ex: the bug's summary, a crash report, a
//! file's lines) it says, where it would have been; and if any of it may go
//! away (ex: a server which didn't answer), the view isn't cached (see
//! `BugComment::transient`).

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
use crate::crash_stack::{CrashFrame, CrashReport};
use crate::file_format::jumpref::JumprefData;
use crate::rendered_docs::escape;

/// Where Bugzilla is, unless the config says (`Config::bugzilla_url`, ex:
/// tests' fixtures, `file://...`, whose files are the REST API's paths with
/// `.json`, and pages' paths with `.html`, without their queries; see
/// `fetch_bugzilla`).
pub const BUGZILLA_URL: &str = "https://bugzilla.mozilla.org";

fn problem(message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::BadInput,
        message,
    })
}

/// Bugzilla's response for `path` at `base`, as text (see `fetch_bugzilla`
/// and `fetch_bugzilla_page`), a file's (`extension` after its path without
/// its query) for a `file://` base.  A bug which isn't there, or isn't
/// public, is a sticky problem; Bugzilla not answering is a transient one.
async fn fetch_bugzilla_text(base: &str, path: &str, extension: &str) -> Result<String> {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let url = format!("{}/{}", base.trim_end_matches('/'), path);
    if let Some(file) = url.strip_prefix("file://") {
        let file = format!("{}{}", file.split('?').next().unwrap_or(file), extension);
        return std::fs::read_to_string(&file)
            .map_err(|e| problem(format!("Couldn't read {}: {}", file, e)));
    }
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
            .send()
            .await?
            .error_for_status()?
            .text()
            .await
    };
    fetched.await.map_err(|e| match e.status() {
        Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN) => {
            problem("Bugzilla doesn't show that bug to searchfox (it's not public)".to_string())
        }
        Some(reqwest::StatusCode::NOT_FOUND) => problem("Bugzilla has no such bug".to_string()),
        _ => ServerError::TransientProblem(ErrorDetails {
            layer: ErrorLayer::ServerLayer,
            message: format!("Couldn't get {} from Bugzilla: {}", path, e),
        }),
    })
}

/// Bugzilla's REST API's response for `path` (ex: "rest/bug/1915788/comment")
/// at `base` (see `BUGZILLA_URL`): one of the last few asked for, if it is.
pub async fn fetch_bugzilla(base: &str, path: &str) -> Result<Arc<serde_json::Value>> {
    use std::sync::Mutex;
    static RECENT: Mutex<Vec<(String, Arc<serde_json::Value>)>> = Mutex::new(vec![]);
    const RECENT_RESPONSES: usize = 16;

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
    let text = fetch_bugzilla_text(base, path, ".json").await?;
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

/// Bugzilla's page `path` (ex: "show_bug.cgi?id=1915788") at `base`, which
/// says what its REST API doesn't: which comments it shows as Markdown (see
/// `comment_is_markdown`).
pub async fn fetch_bugzilla_page(base: &str, path: &str) -> Result<String> {
    fetch_bugzilla_text(base, path, ".html").await
}

/// Whether a bug's page (see `fetch_bugzilla_page`) shows its comment `n` as
/// Markdown (its `comment-text` has `markdown-body`), if it has it.
fn comment_is_markdown(page: &str, n: u32) -> Option<bool> {
    let pattern = Regex::new(&format!(r#"class="comment-text([^"]*)"\s+id="ct-{}""#, n)).ok()?;
    let classes = pattern.captures(page)?;
    Some(classes[1].contains("markdown-body"))
}

/// A comment of a bug: the bug's number, the comment's (0 is the bug's
/// description), and of the bug, its summary, where it is (ex: "Core :: DOM:
/// Content Processes") and its status (ex: "RESOLVED WORKSFORME"), if
/// Bugzilla says; the comment's author (their login), their name as Bugzilla
/// shows it (ex: "Gabriele Svelto [:gsvelto]") and their profile's ID, if it
/// says, and their roles in the bug ("Reporter", "Assignee"); the comment's
/// time, whether Bugzilla shows it as Markdown, and its blocks.
#[derive(Debug, Serialize)]
pub struct BugComment {
    pub bug: u64,
    pub comment: u32,
    pub summary: Option<String>,
    pub about: Option<String>,
    pub status: Option<String>,
    pub author: Option<String>,
    pub author_name: Option<String>,
    pub author_id: Option<u64>,
    pub roles: Vec<&'static str>,
    pub time: Option<String>,
    pub markdown: bool,
    pub blocks: Vec<CommentBlock>,
    /// What the view couldn't get besides its blocks' cards' (which they
    /// say), ex: the bug's summary.
    pub problems: Vec<String>,
    /// Whether any of what it couldn't get may be got if tried again (ex:
    /// Bugzilla didn't answer), so that the view isn't cached.
    pub transient: bool,
}

/// A block of a comment: its HTML; whether it's a list's item which another
/// of its items follows (so that they're as close as a list's items); and the
/// cards beside it, for its links.
#[derive(Debug, Serialize)]
pub struct CommentBlock {
    pub html: String,
    pub item_followed: bool,
    pub cards: Vec<CommentCard>,
}

/// A card beside a comment's block: a code link's excerpt; a crash report's
/// link's report (what crashed, or why the report isn't available), with its
/// top frames if the comment doesn't list them; or a block listing a crash's
/// frames' frames (see `FramesFrom`).
#[derive(Debug, Serialize)]
pub enum CommentCard {
    Code(CodeExcerpt),
    Crash {
        id: String,
        report: std::result::Result<CrashReport, String>,
        frames: Vec<CrashFrame>,
    },
    Frames {
        from: FramesFrom,
        frames: Vec<CrashFrame>,
        problem: Option<String>,
    },
}

/// Where a comment's listed frames' code is from: the crash report's frames
/// (`Report`, of its ID), or if the report isn't available, the listed
/// frames' lines at the tree's head at a time, roughly (`Estimated`: of the
/// crash report's ID's date, or the comment's time, and the revision; see
/// `AbstractServer::rev_at`), else their functions, found by their names, as
/// a crash signature's are (`Names`).
#[derive(Debug, Serialize)]
pub enum FramesFrom {
    Report { id: String },
    Estimated { time: String, rev: String },
    Names,
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
#[derive(Clone, Debug, PartialEq, Serialize)]
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
    /// Mapping them failed (why), so its lines are now's.
    Failed(String),
}

/// A code link's excerpt: the link, how its lines are mapped, its lines now
/// (those unchanged since), how many changed since, where its lines are now
/// (the first and last; a changed line's where it would be, after the last
/// line before it which didn't change), the excerpt's rows (see
/// `crash_stack::FrameRow`), and why they couldn't be fetched, if they
/// couldn't, of a file the tree has.
#[derive(Debug, Serialize)]
pub struct CodeExcerpt {
    pub link: CodeLink,
    pub mapping: LineMapping,
    pub lines_now: Vec<u32>,
    pub changed: usize,
    pub around: Option<(u32, u32)>,
    pub rows: Vec<crate::crash_stack::FrameRow>,
    pub problem: Option<String>,
}

/// A frame a comment lists (see `listed_frames`): its number, its function's
/// name (or its module's address, ex: "libart.so@0x351e30"), and its file
/// and line, if it says.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedFrame {
    pub index: usize,
    pub function: String,
    pub location: Option<(String, u32)>,
}

lazy_static! {
    /// A bare URL in a comment's text (which Bugzilla links), without its
    /// trailing punctuation.
    static ref BARE_URL: Regex =
        Regex::new(r#"https?://[^\s<>"'\[\](){}]+[^\s<>"'\[\](){}.,;:!?]"#).unwrap();
    /// A comment's mention of a bug (and maybe its comment), of a comment of
    /// its bug, or of an attachment, which Bugzilla links.
    static ref MENTION: Regex = Regex::new(
        r"(?i)\b(?:bug\s*#?\s*(?P<bug>[1-9]\d*)(?:\s*,?\s*comment\s*#?\s*(?P<bug_comment>\d+))?|comment\s*#?\s*(?P<comment>\d+)|attachment\s*#?\s*(?P<attachment>[1-9]\d*))\b"
    )
    .unwrap();
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
    /// A listed frame's columns' separator (see `listed_frames`).
    static ref COLUMNS: Regex = Regex::new(r"\s{2,}|\t").unwrap();
    /// A listed frame's location, ex: "dom/ipc/ContentChild.cpp:653".
    static ref LOCATION: Regex = Regex::new(r"^(?P<path>[\w.+/-]+\.\w+):(?P<line>\d+)$").unwrap();
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

/// The frames a code block (or a plain text comment's paragraph) lists, if
/// it lists a crash's frames, as crash-stats' bug template's "Top 10 frames:"
/// do: every line `N  MODULE  FUNCTION` and maybe `  PATH:LINE` (columns
/// separated by two spaces or more, or a tab).
pub fn listed_frames(text: &str) -> Option<Vec<ListedFrame>> {
    let mut frames = vec![];
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let columns: Vec<&str> = COLUMNS.split(line).collect();
        let [index, _module, function, rest @ ..] = columns.as_slice() else {
            return None;
        };
        let index: usize = index.parse().ok()?;
        let location = match rest {
            [] => None,
            [location] => {
                let captures = LOCATION.captures(location)?;
                Some((captures["path"].to_string(), captures["line"].parse().ok()?))
            }
            _ => return None,
        };
        frames.push(ListedFrame {
            index,
            function: function.to_string(),
            location,
        });
    }
    (!frames.is_empty()).then_some(frames)
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

/// `text`'s HTML, its mentions of bugs, of `bug`'s comments and of
/// attachments linked to them in Bugzilla, as Bugzilla does.
fn mentions_linked(text: &str, bug: u64) -> String {
    let mut html = String::new();
    let mut at = 0;
    for captures in MENTION.captures_iter(text) {
        let found = captures.get(0).unwrap();
        let href = if let Some(other) = captures.name("bug") {
            match captures.name("bug_comment") {
                Some(comment) => format!(
                    "{}/show_bug.cgi?id={}#c{}",
                    BUGZILLA_URL,
                    other.as_str(),
                    comment.as_str()
                ),
                None => format!("{}/show_bug.cgi?id={}", BUGZILLA_URL, other.as_str()),
            }
        } else if let Some(comment) = captures.name("comment") {
            format!(
                "{}/show_bug.cgi?id={}#c{}",
                BUGZILLA_URL,
                bug,
                comment.as_str()
            )
        } else {
            format!(
                "{}/attachment.cgi?id={}",
                BUGZILLA_URL, &captures["attachment"]
            )
        };
        html.push_str(&escape(&text[at..found.start()]));
        html.push_str(&format!(
            r#"<a class="bmo-mention" href="{}">{}</a>"#,
            escape(&href),
            escape(found.as_str())
        ));
        at = found.end();
    }
    html.push_str(&escape(&text[at..]));
    html
}

/// `text`'s HTML, with its bare URLs linked, adding them to `urls`, and its
/// mentions linked (see `mentions_linked`).
fn autolinked(text: &str, bug: u64, urls: &mut Vec<String>) -> String {
    let mut html = String::new();
    let mut at = 0;
    for found in BARE_URL.find_iter(text) {
        html.push_str(&mentions_linked(&text[at..found.start()], bug));
        html.push_str(&format!(
            r#"<a href="{}">{}</a>"#,
            escape(found.as_str()),
            escape(found.as_str())
        ));
        urls.push(found.as_str().to_string());
        at = found.end();
    }
    html.push_str(&mentions_linked(&text[at..], bug));
    html
}

/// A block of a comment's text (see `comment_blocks` and `plain_blocks`):
/// its HTML, its links' URLs, the frames it lists (see `listed_frames`), and
/// whether it's a list's item which another of its items follows.
#[derive(Debug, Default, PartialEq)]
pub struct TextBlock {
    pub html: String,
    pub urls: Vec<String>,
    pub frames: Option<Vec<ListedFrame>>,
    pub item_followed: bool,
}

/// A Markdown comment's (of `bug`) top-level blocks (each list's items being
/// blocks of their own, in their lists, keeping their numbers), as HTML (raw
/// HTML shown as text; soft line breaks kept, as Bugzilla does), with their
/// links' URLs, and their code blocks' listed frames.
pub fn comment_blocks(text: &str, bug: u64) -> Vec<TextBlock> {
    let mut blocks: Vec<TextBlock> = vec![];
    let mut block = TextBlock::default();
    // (The depth of the containers the events are in, the top-level list
    // an item is in, as its opening tag, and the next item's number.)
    let mut depth = 0;
    let mut top_list: Option<(bool, u64)> = None;
    // (How many of its items have started.)
    let mut list_items = 0;
    // (The links' closings: an unsafe link's is a span's.)
    let mut link_ends: Vec<&'static str> = vec![];
    // (A code block's text, for its listed frames.)
    let mut code: Option<String> = None;
    let mut cell_tags: Vec<&'static str> = vec![];
    let finish = |block: &mut TextBlock, blocks: &mut Vec<TextBlock>| {
        if !block.html.trim().is_empty() {
            blocks.push(std::mem::take(block));
        }
        *block = TextBlock::default();
    };
    for event in Parser::new_ext(text, options()) {
        match event {
            Event::Start(tag) => {
                // (A top-level list's items are blocks, in a list of their
                // own, numbered as they were.)
                if depth == 0
                    && let Tag::List(start) = &tag
                {
                    top_list = Some((start.is_some(), start.unwrap_or(1)));
                    list_items = 0;
                    depth += 1;
                    continue;
                }
                if depth == 1
                    && let (Tag::Item, Some((ordered, number))) = (&tag, top_list)
                {
                    // (The list's previous item, if any, is followed by it.)
                    if list_items > 0
                        && let Some(last) = blocks.last_mut()
                    {
                        last.item_followed = true;
                    }
                    list_items += 1;
                    block.html.push_str(&match ordered {
                        true => format!(r#"<ol start="{}"><li>"#, number),
                        false => "<ul><li>".to_string(),
                    });
                    top_list = Some((ordered, number + 1));
                    depth += 1;
                    continue;
                }
                depth += 1;
                let html = &mut block.html;
                match tag {
                    Tag::Paragraph => html.push_str("<p>"),
                    Tag::Heading { level, .. } => html.push_str(&format!("<{}>", level)),
                    Tag::BlockQuote(_) => html.push_str("<blockquote>"),
                    Tag::CodeBlock(_) => {
                        code = Some(String::new());
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
                            block.urls.push(dest_url.to_string());
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
                    block.html.push_str(match top_list {
                        Some((true, _)) => "</li></ol>",
                        _ => "</li></ul>",
                    });
                    finish(&mut block, &mut blocks);
                    continue;
                }
                let html = &mut block.html;
                match tag {
                    TagEnd::Paragraph => html.push_str("</p>"),
                    TagEnd::Heading(level) => html.push_str(&format!("</{}>", level)),
                    TagEnd::BlockQuote(_) => html.push_str("</blockquote>"),
                    TagEnd::CodeBlock => {
                        html.push_str("</code></pre>");
                        if let Some(text) = code.take() {
                            block.frames = listed_frames(&text);
                        }
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
                    finish(&mut block, &mut blocks);
                }
            }
            Event::Text(text) => match &mut code {
                Some(code) => {
                    code.push_str(&text);
                    block.html.push_str(&escape(&text));
                }
                None if !link_ends.is_empty() => block.html.push_str(&escape(&text)),
                None => {
                    let linked = autolinked(&text, bug, &mut block.urls);
                    block.html.push_str(&linked);
                }
            },
            Event::Code(code) => block
                .html
                .push_str(&format!("<code>{}</code>", escape(&code))),
            Event::Html(raw) | Event::InlineHtml(raw) => block.html.push_str(&escape(&raw)),
            Event::SoftBreak => block.html.push_str("<br>"),
            Event::HardBreak => block.html.push_str("<br>"),
            Event::Rule => {
                block.html.push_str("<hr>");
                if depth == 0 {
                    finish(&mut block, &mut blocks);
                }
            }
            Event::TaskListMarker(checked) => {
                block.html.push_str(if checked { "☑ " } else { "☐ " })
            }
            _ => {}
        }
    }
    finish(&mut block, &mut blocks);
    blocks
}

/// A plain text comment's (of `bug`; one Bugzilla doesn't show as Markdown)
/// blocks: its paragraphs (separated by blank lines), as Bugzilla shows them
/// (their lines kept, see the view's CSS; their URLs and mentions linked, and
/// quoted lines, `> ...`, as quotes), with their links' URLs and their listed
/// frames.
pub fn plain_blocks(text: &str, bug: u64) -> Vec<TextBlock> {
    lazy_static! {
        static ref BLANK_LINES: Regex = Regex::new(r"\n[ \t]*\n").unwrap();
    }
    BLANK_LINES
        .split(text.trim_matches('\n'))
        .filter(|paragraph| !paragraph.trim().is_empty())
        .map(|paragraph| {
            let mut urls = vec![];
            let html = paragraph
                .lines()
                .map(|line| match line.starts_with('>') {
                    true => format!(
                        r#"<span class="quote">{}</span>"#,
                        autolinked(line, bug, &mut urls)
                    ),
                    false => autolinked(line, bug, &mut urls),
                })
                .collect::<Vec<_>>()
                .join("\n");
            TextBlock {
                html,
                urls,
                frames: listed_frames(paragraph),
                item_followed: false,
            }
        })
        .collect()
}

/// How many lines around a link's lines its excerpt shows, and at most how
/// many lines an excerpt has, and how many code links a comment's excerpts
/// are of (the rest are links), and how many of a crash report's frames its
/// card shows if the comment doesn't list them.
const CONTEXT_LINES: u32 = 2;
const MAX_EXCERPT_LINES: u32 = 60;
const MAX_CODE_LINKS: usize = 40;
const REPORT_FRAMES: usize = 10;

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
    let mut failure: Option<String> = None;
    let mut mapped = |result: Result<Option<String>>| match result {
        Ok(rev) => rev,
        Err(ServerError::Unsupported) => None,
        Err(err) => {
            failure.get_or_insert(err.message());
            None
        }
    };
    let candidates: Vec<Option<String>> = match link.rev_kind {
        "hg" => vec![mapped(server.map_rev("hg", rev).await)],
        _ => vec![Some(rev.clone()), mapped(server.map_rev("old", rev).await)],
    };
    for tree_rev in candidates.into_iter().flatten() {
        match server.line_map(&tree_rev, &link.path).await {
            Ok(Some(map)) => return (LineMapping::Mapped { tree_rev }, Some(map)),
            Ok(None) => {}
            Err(ServerError::Unsupported) => return (LineMapping::Unmapped, None),
            Err(err) => {
                failure.get_or_insert(err.message());
            }
        }
    }
    match failure {
        Some(failure) => (LineMapping::Failed(failure), None),
        None => (LineMapping::UnknownRev, None),
    }
}

/// When the crash report `id` was submitted, from its ID, whose last 6 digits
/// are its date (YYMMDD, ex: "240829" in "6c95ec01-b53b-4a69-b815-dbed50240829"):
/// that day's start, in UTC.
fn crash_id_day(id: &str) -> Option<String> {
    let digits = id.get(id.len().checked_sub(6)?..)?;
    let date = chrono::NaiveDate::parse_from_str(&format!("20{}", digits), "%Y%m%d").ok()?;
    Some(format!("{}T00:00:00Z", date.format("%Y-%m-%d")))
}

/// The listed frames' stack (see `listed_frames`), whose crash report isn't
/// available, as the report's would be (see
/// `crash_stack::report_threads_stack`), their lines at `rev` (the tree's
/// head when the crash was submitted, roughly), which is an estimate of the
/// build's revision: a frame's calls inlined in it are those listed before it
/// with its number.
async fn estimated_stack(
    server: &(dyn AbstractServer + Send + Sync),
    listed: &[ListedFrame],
    rev: &str,
) -> Vec<CrashFrame> {
    use crate::crash_stack::{FrameSource, ReportFrame, ReportThread, ThreadInfo};
    let frames = listed
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            let inlined = listed
                .get(i + 1)
                .is_some_and(|next| next.index == frame.index);
            let source = match &frame.location {
                Some((path, line)) => FrameSource {
                    file: Some(format!("git:tree:{}:{}", path, rev)),
                    path: Some(path.clone()),
                    rev: Some(rev.to_string()),
                    line: Some(*line),
                    inlined,
                    estimated: true,
                    ..Default::default()
                },
                None => FrameSource {
                    inlined,
                    ..Default::default()
                },
            };
            ReportFrame {
                frame: frame.index,
                function: Some(frame.function.clone()),
                source,
                address: None,
            }
        })
        .collect();
    let thread = ThreadInfo {
        index: 0,
        name: None,
    };
    let summary = CrashReport {
        id: String::new(),
        product: String::new(),
        signature: String::new(),
        reason: None,
        crash_type: None,
        address: None,
        thread_count: 1,
        crashing_thread: Some(thread.clone()),
    };
    let threads = vec![ReportThread { thread, frames }];
    crate::crash_stack::report_threads_stack(server, summary, threads, false)
        .await
        .frames
}

/// The listed frames' functions in the tree, found by their names (without
/// their arguments; those listed again in a row once), as a crash signature's
/// are (see `crash_stack::crash_stack`).
async fn named_stack(
    server: &(dyn AbstractServer + Send + Sync),
    listed: &[ListedFrame],
) -> Result<Vec<CrashFrame>> {
    let mut names: Vec<&str> = vec![];
    for frame in listed {
        let name = crate::crash_stack::without_arguments(&frame.function);
        if names.last() != Some(&name) {
            names.push(name);
        }
    }
    Ok(crate::crash_stack::crash_stack(server, &names.join(" | "))
        .await?
        .frames)
}

/// What a view couldn't get (see `BugComment::problems`).
#[derive(Default)]
struct Problems {
    notes: Vec<String>,
    transient: bool,
}

impl Problems {
    /// `err`, which a card says (or with `note`, the view).
    fn saw(&mut self, err: &ServerError) {
        self.transient |= err.is_transient();
    }

    fn note(&mut self, note: String, err: &ServerError) {
        self.notes.push(note);
        self.saw(err);
    }
}

/// The view of comment `comment` of bug `bug` (see the module's
/// documentation), its code links' excerpts from `server`'s tree.
pub async fn bug_comment(
    server: &(dyn AbstractServer + Send + Sync),
    bug: u64,
    comment: u32,
) -> Result<BugComment> {
    let mut problems = Problems::default();
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

    // (The bug's summary, where it is and its status, and who reported it
    // and is assigned it, if Bugzilla says.)
    let info = match server
        .bugzilla_json(&format!(
            "rest/bug/{}?include_fields=summary,product,component,status,resolution,creator,assigned_to",
            bug
        ))
        .await
    {
        Ok(info) => Some(info),
        Err(err) => {
            problems.note(
                format!("Couldn't get the bug's summary: {}.", err.message()),
                &err,
            );
            None
        }
    };
    let bug_info = info.as_ref().map(|info| &info["bugs"][0]);
    let field = |name: &str| {
        bug_info
            .and_then(|info| info[name].as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let summary = field("summary");
    let about = match (field("product"), field("component")) {
        (Some(product), Some(component)) => Some(format!("{} :: {}", product, component)),
        _ => None,
    };
    let status = field("status").map(|status| match field("resolution") {
        Some(resolution) => format!("{} {}", status, resolution),
        None => status,
    });
    let mut roles = vec![];
    if author.is_some() && field("creator") == author {
        roles.push("Reporter");
    }
    if author.is_some() && field("assigned_to") == author {
        roles.push("Assignee");
    }
    // (Its author's name, as Bugzilla shows it, if it says.)
    let (mut author_name, mut author_id) = (None, None);
    if let Some(author) = &author
        && let Ok(users) = server
            .bugzilla_json(&format!(
                "rest/user?names={}&include_fields=id,name,real_name",
                urlencoding::encode(author)
            ))
            .await
        && let Some(user) = users["users"].as_array().and_then(|users| {
            users
                .iter()
                .find(|user| user["name"].as_str() == Some(author))
        })
    {
        author_name = user["real_name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        author_id = user["id"].as_u64();
    }
    // (Whether Bugzilla shows the comment as Markdown: as most are, if its
    // page doesn't say.)
    let markdown = match server
        .bugzilla_page(&format!("show_bug.cgi?id={}", bug))
        .await
    {
        Ok(page) => comment_is_markdown(&page, comment).unwrap_or(true),
        Err(_) => true,
    };
    let text_blocks = match markdown {
        true => comment_blocks(&text, bug),
        false => plain_blocks(&text, bug),
    };

    // Its crash reports (once each), and the listed frames' reports: the
    // nearest before them, else the first.
    let crash_ids: Vec<String> = text_blocks
        .iter()
        .flat_map(|block| &block.urls)
        .filter_map(|url| Some(CRASH_LINK.captures(url)?["id"].to_string()))
        .fold(vec![], |mut ids, id| {
            if !ids.contains(&id) {
                ids.push(id);
            }
            ids
        });
    let mut stacks: BTreeMap<String, Result<crate::crash_stack::CrashStack>> = BTreeMap::new();
    for id in &crash_ids {
        let stack = crate::crash_stack::crash_report_stack(server, id, false).await;
        if let Err(err) = &stack {
            problems.saw(err);
        }
        stacks.insert(id.clone(), stack);
    }
    let mut frames_reports: Vec<Option<String>> = vec![];
    let mut last_crash: Option<String> = None;
    for block in &text_blocks {
        frames_reports.push(
            block
                .frames
                .as_ref()
                .and(last_crash.clone().or_else(|| crash_ids.first().cloned())),
        );
        if let Some(id) = block
            .urls
            .iter()
            .rev()
            .find_map(|url| Some(CRASH_LINK.captures(url)?["id"].to_string()))
        {
            last_crash = Some(id);
        }
    }
    let listed_reports: BTreeSet<&String> = frames_reports.iter().flatten().collect();

    // The blocks, and their cards: their code links' excerpts (once each,
    // at most MAX_CODE_LINKS), their crash reports, and their listed frames.
    let mut code_links = 0;
    let mut blocks = vec![];
    let mut maps: BTreeMap<
        (Option<String>, &'static str, String),
        (LineMapping, Option<Vec<Option<u32>>>),
    > = BTreeMap::new();
    for (b, block) in text_blocks.iter().enumerate() {
        let mut cards = vec![];
        let mut seen = BTreeSet::new();
        for url in &block.urls {
            if !seen.insert(url.clone()) {
                continue;
            }
            if let Some(captures) = CRASH_LINK.captures(url) {
                let id = captures["id"].to_string();
                let (report, frames) = match &stacks[&id] {
                    Ok(stack) => (
                        stack.report.clone().ok_or_else(String::new),
                        match listed_reports.contains(&id) {
                            true => vec![],
                            false => stack
                                .frames
                                .iter()
                                .filter(|frame| frame.index < REPORT_FRAMES)
                                .cloned()
                                .collect(),
                        },
                    ),
                    Err(err) => (Err(err.message()), vec![]),
                };
                cards.push(CommentCard::Crash { id, report, frames });
                continue;
            }
            let Some(link) = code_link(url).filter(|link| !link.lines.is_empty()) else {
                continue;
            };
            if code_links >= MAX_CODE_LINKS {
                continue;
            }
            code_links += 1;
            let key = (link.rev.clone(), link.rev_kind, link.path.clone());
            if !maps.contains_key(&key) {
                let mapped = map_link(server, &link).await;
                if matches!(mapped.0, LineMapping::Failed(_)) {
                    problems.transient = true;
                }
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
            cards.push(CommentCard::Code(CodeExcerpt {
                link,
                mapping: mapping.clone(),
                lines_now,
                changed,
                around: span,
                rows: vec![],
                problem: None,
            }));
        }
        // (Its listed frames' code: its report's frames, or if the report
        // isn't available, the listed frames' lines at the tree's head when
        // the crash was submitted (or the comment was written), roughly,
        // else their functions by their names.)
        if let Some(listed) = &block.frames {
            let report = frames_reports[b].clone();
            // (The report's frames as far as the listed ones go, with their
            // inlined calls, which are frames of their own.)
            let last = listed.iter().map(|frame| frame.index).max().unwrap_or(0);
            let card = match report.as_ref().map(|id| (id, &stacks[id])) {
                Some((id, Ok(stack))) => CommentCard::Frames {
                    from: FramesFrom::Report { id: id.clone() },
                    frames: stack
                        .frames
                        .iter()
                        .filter(|frame| frame.index <= last)
                        .cloned()
                        .collect(),
                    problem: None,
                },
                _ => {
                    let when = report
                        .as_deref()
                        .and_then(crash_id_day)
                        .or_else(|| time.clone());
                    let at = match &when {
                        Some(when) if listed.iter().any(|frame| frame.location.is_some()) => {
                            server.rev_at(when).await.ok().flatten()
                        }
                        _ => None,
                    };
                    match (when, at) {
                        (Some(when), Some(at)) => CommentCard::Frames {
                            frames: estimated_stack(server, listed, &at.rev).await,
                            from: FramesFrom::Estimated {
                                time: when,
                                rev: at.rev,
                            },
                            problem: None,
                        },
                        _ => match named_stack(server, listed).await {
                            Ok(frames) => CommentCard::Frames {
                                from: FramesFrom::Names,
                                frames,
                                problem: None,
                            },
                            Err(err) => {
                                problems.saw(&err);
                                CommentCard::Frames {
                                    from: FramesFrom::Names,
                                    frames: vec![],
                                    problem: Some(format!(
                                        "Couldn't find its frames' functions: {}",
                                        err.message()
                                    )),
                                }
                            }
                        },
                    }
                }
            };
            cards.push(card);
        }
        blocks.push(CommentBlock {
            html: block.html.clone(),
            item_followed: block.item_followed,
            cards,
        });
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
    let (rows, failed) = crate::crash_stack::fetch_rows(server, requests).await;
    for (b, c, path, all, shown) in wanted {
        let CommentCard::Code(excerpt) = &mut blocks[b].cards[c] else {
            continue;
        };
        let Some(file_rows) = rows.get(&path) else {
            // (A file the tree doesn't have says so; one it has, why its
            // lines couldn't be fetched.)
            if let Some(err) = failed.get(&path)
                && server.file_facet_info(&path).is_some()
            {
                excerpt.problem = Some(crate::crash_stack::row_problem(err));
                problems.transient = true;
            }
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
    let frame_problems = blocks
        .iter()
        .flat_map(|block| &block.cards)
        .any(|card| match card {
            CommentCard::Crash { frames, .. } | CommentCard::Frames { frames, .. } => {
                frames.iter().any(|frame| frame.problem.is_some())
            }
            CommentCard::Code(_) => false,
        });
    problems.transient |= frame_problems;

    Ok(BugComment {
        bug,
        comment,
        summary,
        about,
        status,
        author,
        author_name,
        author_id,
        roles,
        time,
        markdown,
        blocks,
        problems: problems.notes,
        transient: problems.transient,
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
            CommentCard::Crash { frames, .. } | CommentCard::Frames { frames, .. } => {
                frames.iter().flat_map(|frame| &frame.rows).collect()
            }
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

/// A time as the view shows it (ex: "2024-08-30 07:26 UTC", for
/// "2024-08-30T07:26:47Z"), which its script shows as how long ago it was,
/// as Bugzilla does.
fn shown_time(time: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(time) {
        Ok(time) => time.format("%Y-%m-%d %H:%M UTC").to_string(),
        Err(_) => time.to_string(),
    }
}

/// The view (see the module's documentation), for the query page (the
/// `bug-comment` command's HTML, which the query page shows as it is): the
/// bug, what the view couldn't get, the comment's header beside what the
/// cards are, its blocks beside their cards, and its excerpts' SYM_INFO.
pub fn render_bug_comment(
    tree_name: &str,
    view: &BugComment,
    sym_info: &BTreeMap<Ustr, Option<JumprefData>>,
    writer: &mut dyn Write,
) -> std::result::Result<(), &'static str> {
    use crate::url_encode_path::url_encode_path;

    let bug_url = format!("{}/show_bug.cgi?id={}", BUGZILLA_URL, view.bug);
    let comment_url = format!("{}#c{}", bug_url, view.comment);
    let mut html = String::new();
    html.push_str(r#"<div class="crash-view bug-comment-view">"#);

    // The bug.
    html.push_str(&format!(
        r#"<div class="bug-comment-bug"><a class="bug-comment-bug-id" href="{}">Bug {}</a>{}"#,
        escape(&bug_url),
        view.bug,
        view.summary
            .as_ref()
            .map_or_else(String::new, |summary| format!(
                r#" <span class="bug-comment-bug-summary">{}</span>"#,
                escape(summary)
            ))
    ));
    let about: Vec<String> = [
        view.about.as_ref().map(|about| escape(about)),
        view.status.as_ref().map(|status| {
            format!(
                r#"<span class="bug-comment-bug-status">{}</span>"#,
                escape(status)
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !about.is_empty() {
        html.push_str(&format!(
            r#"<div class="bug-comment-bug-about">{}</div>"#,
            about.join(" · ")
        ));
    }
    html.push_str("</div>");
    if !view.problems.is_empty() {
        html.push_str(r#"<div class="query-problem bug-comment-problems" role="alert">"#);
        for problem in &view.problems {
            html.push_str(&format!("<p>{}</p>", escape(problem)));
        }
        if view.transient {
            html.push_str("<p>It may work if you try again later.</p>");
        }
        html.push_str("</div>");
    }

    // The comment's header, as Bugzilla's (its author, their roles, its
    // number and its time), beside what the cards are.
    let excerpts = view
        .blocks
        .iter()
        .flat_map(|block| &block.cards)
        .filter(|card| matches!(card, CommentCard::Code(_)))
        .count();
    let name = view
        .author_name
        .clone()
        .or_else(|| view.author.clone())
        .unwrap_or_else(|| "Someone".to_string());
    let author = match view.author_id {
        Some(id) => format!(
            r#"<a class="bmo-author" href="{}/user_profile?user_id={}">{}</a>"#,
            BUGZILLA_URL,
            id,
            escape(&name)
        ),
        None => format!(r#"<span class="bmo-author">{}</span>"#, escape(&name)),
    };
    html.push_str(r#"<div class="crash-frame bug-comment-head">"#);
    html.push_str(&format!(
        r#"<div class="bmo-change-head"><span class="bmo-avatar" aria-hidden="true">{}</span><div class="bmo-change-who"><div>{}{}</div><div><a class="bmo-change-name bug-comment-link" href="{}">{}</a>{}</div></div></div>"#,
        escape(&name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
        author,
        view.roles
            .iter()
            .map(|role| format!(r#"<span class="bmo-role">{}</span>"#, role))
            .collect::<String>(),
        escape(&comment_url),
        match view.comment {
            0 => "Description".to_string(),
            n => format!("Comment {}", n),
        },
        view.time.as_ref().map_or_else(String::new, |time| format!(
            r#" &bull; <time class="bmo-change-time" datetime="{}" title="{}">{}</time>"#,
            escape(time),
            escape(&shown_time(time)),
            escape(&shown_time(time))
        ))
    ));
    html.push_str(&format!(
        r#"<div class="crash-notice bug-comment-notice"><p>Beside each of its blocks, the code it links to in {} at the indexed revision ({} excerpt{}), with the linked lines highlighted (mapped from the links' revisions), and the code of the crashes it links to.</p></div></div>"#,
        escape(tree_name),
        excerpts,
        if excerpts == 1 { "" } else { "s" }
    ));

    // Its blocks, each beside its cards.
    let text_class = match view.markdown {
        true => "bug-comment-text comment-text markdown-body",
        false => "bug-comment-text comment-text",
    };
    html.push_str(r#"<div class="crash-stack bug-comment-blocks" role="list">"#);
    for (b, block) in view.blocks.iter().enumerate() {
        html.push_str(&format!(
            r#"<section class="crash-frame bug-comment-block{}" role="listitem" id="block-{}">"#,
            if block.item_followed {
                " bug-comment-item"
            } else {
                ""
            },
            b
        ));
        html.push_str(&format!(
            r#"<div class="{}">{}</div>"#,
            text_class, block.html
        ));
        html.push_str(r#"<div class="bug-comment-cards">"#);
        for card in &block.cards {
            match card {
                CommentCard::Crash { id, report, frames } => {
                    html.push_str(&crash_card_html(tree_name, id, report, frames, b))
                }
                CommentCard::Frames {
                    from,
                    frames,
                    problem,
                } => html.push_str(&frames_card_html(
                    tree_name,
                    from,
                    frames,
                    problem.as_deref(),
                    b,
                )),
                CommentCard::Code(excerpt) => {
                    html.push_str(&excerpt_html(
                        tree_name,
                        excerpt,
                        &url_encode_path(&excerpt.link.path),
                    ));
                }
            }
        }
        html.push_str("</div></section>");
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
}});
// (The comment's time, as Bugzilla shows it: how long ago it was, its time in
// its title.)
for (const time of document.querySelectorAll(".bmo-change-time[datetime]")) {{
  const seconds = (Date.now() - Date.parse(time.getAttribute("datetime"))) / 1000;
  const units = [["year", 365 * 86400], ["month", 30 * 86400], ["day", 86400], ["hour", 3600], ["minute", 60]];
  const [unit, size] = units.find(([, size]) => seconds >= size) || [];
  const count = unit && Math.floor(seconds / size);
  time.textContent = unit ? `${{count}} ${{unit}}${{count == 1 ? "" : "s"}} ago` : "just now";
}}</script>"#,
        serde_json::to_string(sym_info)
            .unwrap_or_else(|_| "{}".to_string())
            .replace('<', "\\u003c")
    )
    .map_err(|_| "Write error")?;
    Ok(())
}

/// A crash report's card: what crashed (or why the report isn't available),
/// linked to its crash view, and its top frames, if it has them (see
/// `CommentCard::Crash`).
fn crash_card_html(
    tree_name: &str,
    id: &str,
    report: &std::result::Result<CrashReport, String>,
    frames: &[CrashFrame],
    block: usize,
) -> String {
    let mut html = String::from(r#"<div class="bug-comment-card bug-comment-crash">"#);
    let link = format!(
        r#"<a href="https://crash-stats.mozilla.org/report/index/{}">Crash report {}</a>"#,
        escape(id),
        escape(&id[..id.len().min(8)])
    );
    match report {
        Ok(report) => {
            let (what, thread) = crate::crash_stack::report_crashed(report);
            html.push_str(&format!(
                r#"<p>{} ({}): {}, in {}.{} <a class="bug-comment-crash-view" href="/{}/query/default?q=crash-report%3A{}">Its stack, beside its code</a>.</p>"#,
                link,
                escape(&report.product),
                what,
                thread,
                report.reason.as_ref().map_or_else(String::new, |reason| format!(
                    r#" Its MOZ_CRASH reason: <code>{}</code>."#,
                    escape(reason)
                )),
                escape(tree_name),
                escape(id)
            ));
        }
        Err(message) => html.push_str(&format!(
            r#"<p>{}: <span class="view-problem">{}.</span></p>"#,
            link,
            escape(message)
        )),
    }
    for (i, frame) in frames.iter().enumerate() {
        html.push_str(&crate::crash_stack::frame_html(
            tree_name,
            frame,
            &format!("block-{}-frame-{}", block, i),
        ));
    }
    html.push_str("</div>");
    html
}

/// A block's listed frames' card (see `CommentCard::Frames`): where their
/// code is from, and the frames beside their code.
fn frames_card_html(
    tree_name: &str,
    from: &FramesFrom,
    frames: &[CrashFrame],
    problem: Option<&str>,
    block: usize,
) -> String {
    let mut html = String::from(r#"<div class="bug-comment-card bug-comment-frames">"#);
    let note = match from {
        FramesFrom::Report { .. } => {
            "Its frames, from the crash report, each beside its code at the indexed revision, its line from the report highlighted (mapped from the build's revision).".to_string()
        }
        FramesFrom::Estimated { time, rev } => format!(
            r#"The crash report isn't available, so these are its frames' lines at {}'s head <a href="/{}/rev/{}/">{}</a>, as of {} (when the crash was submitted, by its ID, or else when the comment was written), roughly, which the build it crashed in was probably at or a little before: mapped to the indexed revision, as a report's would be from its build's."#,
            escape(tree_name),
            escape(tree_name),
            escape(rev),
            escape(&rev[..rev.len().min(12)]),
            escape(&shown_time(time))
        ),
        FramesFrom::Names => {
            "These are its frames' functions in the tree now, found by their names, as a crash signature's are, with the lines they're believed to have been at (their calls of the frame above) highlighted.".to_string()
        }
    };
    html.push_str(&format!(r#"<div class="crash-frame-where">{}</div>"#, note));
    if let Some(problem) = problem {
        html.push_str(&format!(
            r#"<div class="crash-code-missing view-problem">{}</div>"#,
            escape(problem)
        ));
    }
    for (i, frame) in frames.iter().enumerate() {
        html.push_str(&crate::crash_stack::frame_html(
            tree_name,
            frame,
            &format!("block-{}-frame-{}", block, i),
        ));
    }
    html.push_str("</div>");
    html
}

/// A code link's card: its heading (the file and its lines now, linked to the
/// tree's, and the link as it was), what its lines are, and its excerpt (or
/// why there's none).
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
        LineMapping::Failed(failure) => format!(
            r#"<span class="view-problem">Couldn't map its lines from its revision ({}):</span> its line numbers, now."#,
            escape(failure)
        ),
    };
    // (Not for a file the tree doesn't have.)
    if !note.is_empty() && !excerpt.rows.is_empty() {
        html.push_str(&format!(r#"<div class="crash-frame-where">{}</div>"#, note));
    }
    if excerpt.rows.is_empty() {
        html.push_str(&match &excerpt.problem {
            Some(problem) => format!(
                r#"<div class="crash-code-missing view-problem">{}</div>"#,
                escape(problem)
            ),
            None => r#"<div class="crash-code-missing">Not a file in this tree.</div>"#.to_string(),
        });
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
        assert_eq!(
            crash_id_day("6c95ec01-b53b-4a69-b815-dbed50240829").as_deref(),
            Some("2024-08-29T00:00:00Z")
        );
        assert_eq!(crash_id_day("6c95ec01-b53b-4a69-b815-dbed50241329"), None);
    }

    #[test]
    fn test_comment_blocks() {
        let text = "Going through `NS_InitXPCOM` (see bug 1234, comment 5):\n\n* https://searchfox.org/m/rev/abc1234/a.cpp#1-2 - first.\n* The [second](https://searchfox.org/m/rev/abc1234/b.cpp#3) one\n  * nested https://example.org/x.\n\n1. one\n2. two\n\n```\n0 libxul.so https://not-linked.example/\n```\n\n<b>raw</b>\nline two";
        let blocks = comment_blocks(text, 99);
        let summary: Vec<(&str, Vec<&str>)> = blocks
            .iter()
            .map(|block| {
                (
                    block.html.as_str(),
                    block.urls.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(summary.len(), 7, "{:#?}", summary);
        // (Bugs and their comments are linked, as Bugzilla does.)
        assert_eq!(
            summary[0].0,
            r#"<p>Going through <code>NS_InitXPCOM</code> (see <a class="bmo-mention" href="https://bugzilla.mozilla.org/show_bug.cgi?id=1234#c5">bug 1234, comment 5</a>):</p>"#
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
        // (A list's items but its last are followed by its next ones.)
        assert_eq!(
            blocks.iter().map(|b| b.item_followed).collect::<Vec<_>>(),
            [false, true, false, true, false, false, false]
        );
        // (Ordered lists' items keep their numbers.)
        assert_eq!(summary[3].0, r#"<ol start="1"><li>one</li></ol>"#);
        assert_eq!(summary[4].0, r#"<ol start="2"><li>two</li></ol>"#);
        // (Code blocks' URLs aren't links, nor frames unless they list them,
        // and raw HTML is text.)
        assert!(
            blocks
                .iter()
                .all(|block| !block.urls.iter().any(|u| u.contains("not-linked")))
        );
        assert!(blocks.iter().all(|block| block.frames.is_none()));
        assert!(
            blocks
                .iter()
                .any(|block| block.html.contains("&lt;b&gt;raw&lt;/b&gt;<br>line two"))
        );
    }

    #[test]
    fn test_listed_frames() {
        let text = "Top 10 frames:\n```\n0  libxul.so  mozilla::dom::ContentChild::~ContentChild()  dom/ipc/ContentChild.cpp:653\n1  libxul.so  mozilla::dom::ContentProcess::~ContentProcess()  dom/ipc/ContentProcess.cpp:67\n4  libart.so  libart.so@0x351e30\n5  ?  @0x0000000112d9a1ac\n```\n";
        let blocks = comment_blocks(text, 1);
        let frames = blocks[1].frames.as_ref().unwrap();
        assert_eq!(
            frames[0],
            ListedFrame {
                index: 0,
                function: "mozilla::dom::ContentChild::~ContentChild()".to_string(),
                location: Some(("dom/ipc/ContentChild.cpp".to_string(), 653)),
            }
        );
        assert_eq!(
            (
                frames[2].index,
                frames[2].function.as_str(),
                &frames[2].location
            ),
            (4, "libart.so@0x351e30", &None)
        );
        assert_eq!(frames.len(), 4);
        assert_eq!(
            listed_frames("0 libxul.so https://not-linked.example/"),
            None
        );
        assert_eq!(listed_frames("We're tripping over an assertion."), None);
    }

    #[test]
    fn test_plain_blocks() {
        let text = "(In reply to Someone from comment #3)\n> Is it *this* one?\n\nYes: bug 1234567 again.\nhttps://example.org/x\n\n\n0  xul.dll  Foo::Bar()  dom/Foo.cpp:12\n";
        let blocks = plain_blocks(text, 99);
        assert_eq!(blocks.len(), 3);
        // (Lines kept, quoted lines as quotes, Markdown left as it is.)
        assert_eq!(
            blocks[0].html,
            r#"(In reply to Someone from <a class="bmo-mention" href="https://bugzilla.mozilla.org/show_bug.cgi?id=99#c3">comment #3</a>)
<span class="quote">&gt; Is it *this* one?</span>"#
        );
        assert_eq!(blocks[1].urls, ["https://example.org/x"]);
        assert!(
            blocks[1]
                .html
                .contains(r#"show_bug.cgi?id=1234567">bug 1234567</a>"#)
        );
        assert_eq!(blocks[2].frames.as_ref().map(Vec::len), Some(1));
        assert_eq!(
            comment_is_markdown(
                r#"<div class="comment-text  " id="ct-3" data-comment-id="1">x</div><div
      class="comment-text markdown-body "
      id="ct-4" data-comment-id="2">"#,
                3
            ),
            Some(false)
        );
        assert_eq!(
            comment_is_markdown(
                r#"<div
      class="comment-text markdown-body "
      id="ct-4" data-comment-id="2">"#,
                4
            ),
            Some(true)
        );
        assert_eq!(comment_is_markdown("", 4), None);
    }
}
