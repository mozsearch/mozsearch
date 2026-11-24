//! A try push's review view, the `try-review:` query term's (see
//! asuth-gecko-notes' try-review-ui-plan.md): its patches, oldest first, and
//! each one's changes as a reviewer reads them, interfaces (IDL, IPDL) first,
//! then definitions (headers, and the definitions it adds or whose
//! declarations it changes), then their uses (the other changes in function
//! bodies), and tests last; each file's hunks under the classes and functions
//! they're in (by the commit's outline diff), with their code before and
//! after (Then, Now, or both).  Every patch, file, class, function and hunk
//! is a part the reviewer can mark (see static/js/review-state.js and
//! try-review.js), by an id that doesn't change when the push is imported
//! again: the push's own commit's hash, the file's path, the definition's
//! pretty name, and a hash of the hunk's changed lines.
//!
//! The view is everything the page shows, in it: no requests after it loads
//! (which the CDN's browser check could answer; see waf-notes.md), but for
//! the review's marks, which are the browser's (IndexedDB).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::hyperblame::outline_diff::{OutlineDiff, OutlineEntry, OutlineStatus};
use crate::rendered_docs::escape;

pub const TRY_REVIEW_MIME: &str = "application/vnd.searchfox.try-review+html";

/// The lines of context around a hunk's changes.
pub const CONTEXT_LINES: usize = 6;

/// The most files a view has of a patch (the rest are left out, with a note
/// saying so), and the most lines of a file's hunks it shows.
const MAX_FILES: usize = 200;
const MAX_FILE_LINES: usize = 4000;

// ## A commit's diff

/// A commit, and its changes from its first parent (see `commit_diff_data`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitDiff {
    pub rev: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub author: String,
    /// When it was committed (RFC 3339).
    pub date: String,
    pub message: String,
    pub files: Vec<FileDiff>,
}

/// A file's changes in a commit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    /// Its path before, if it's renamed (or copied).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    /// "added", "removed", "renamed", "changed", or "binary".
    pub status: String,
    pub hunks: Vec<Hunk>,
}

/// A hunk of a file's changes, with `CONTEXT_LINES` of context.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hunk {
    pub lines: Vec<HunkLine>,
}

/// A line of a hunk: kept (" "), removed ("-"), or added ("+"), with its
/// numbers before and after, and as syntax-highlighted HTML on each side, if
/// it's had (see `add_line_html`; not kept in the view's cache).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HunkLine {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<u32>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then_html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_html: Option<String>,
    /// Its blame strips' cells on each side, if its revision's blame is had
    /// (see `add_line_html`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then_blame: Option<BlameCell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_blame: Option<BlameCell>,
    /// If a later patch of the push changes it (as it is with its patch), that
    /// change, its index in the view's `later` (see `add_later_changes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub later: Option<usize>,
}

/// A later patch's change of a line a patch adds or keeps (see
/// `add_later_changes`): the patch (its index), the id of the element of its
/// hunk changing it, and that hunk's lines it adds (their HTML, the first
/// few), for the line's popup (see try-review.js's `addLaterChanges`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LaterChange {
    pub patch: usize,
    pub element: String,
    pub lines: Vec<String>,
}

/// A line's blame strip (see `hyperblame::page_blame::StripLine`): its
/// blame's index in the view's `BLAME_INFOS` and its line there (see
/// hyperblame.js's `HyperblameContexts`), its commit (for its color), and
/// its classes and data attributes (`StripLine::strip_attrs`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlameCell {
    pub ctx: usize,
    pub line: u32,
    pub commit: Option<String>,
    pub classes: String,
    pub data: String,
}

impl Hunk {
    /// Its changed lines' new-side positions (a removed line's is where it
    /// was), first and last.
    fn new_range(&self) -> Option<(u32, u32)> {
        let mut next = None;
        let mut range: Option<(u32, u32)> = None;
        // (Walking back, a removed line's position is the next kept or
        // added line's, or the last one's plus one.)
        // (Blank lines aside, unless they're all it changes.)
        let mut positions = vec![];
        let mut blank_positions = vec![];
        for line in self.lines.iter().rev() {
            let into = match line.text.trim().is_empty() {
                true => &mut blank_positions,
                false => &mut positions,
            };
            match line.new {
                Some(new) => {
                    next = Some(new);
                    if line.kind == "+" {
                        into.push(new);
                    }
                }
                None => into.push(next.unwrap_or_else(|| self.last_new() + 1)),
            }
        }
        if positions.is_empty() {
            positions = blank_positions;
        }
        for position in positions {
            range = Some(match range {
                Some((first, last)) => (first.min(position), last.max(position)),
                None => (position, position),
            });
        }
        range
    }

    fn last_new(&self) -> u32 {
        self.lines
            .iter()
            .filter_map(|line| line.new)
            .max()
            .unwrap_or(0)
    }

    /// Its removed lines' old-side numbers, first and last.
    fn old_range(&self) -> Option<(u32, u32)> {
        let removed = |blank: bool| -> Vec<u32> {
            self.lines
                .iter()
                .filter(|line| line.kind == "-" && (blank || !line.text.trim().is_empty()))
                .filter_map(|line| line.old)
                .collect()
        };
        let mut lines = removed(false);
        if lines.is_empty() {
            lines = removed(true);
        }
        Some((*lines.iter().min()?, *lines.iter().max()?))
    }

    fn count(&self, kind: &str) -> u32 {
        self.lines.iter().filter(|line| line.kind == kind).count() as u32
    }
}

/// A path in `git diff`'s headers (without its prefix, `--no-prefix`), which
/// git quotes, C style, when it has special characters.
fn diff_path(path: &str) -> String {
    let path = path.trim_end_matches('\t');
    let Some(quoted) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_string();
    };
    let mut bytes = vec![];
    let mut chars = quoted.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => bytes.push(b'\n'),
            Some('t') => bytes.push(b'\t'),
            Some(d @ '0'..='7') => {
                let mut value = d.to_digit(8).unwrap();
                for _ in 0..2 {
                    if let Some(d) = chars.peek().and_then(|c| c.to_digit(8)) {
                        value = value * 8 + d;
                        chars.next();
                    }
                }
                bytes.push(value as u8);
            }
            Some(other) => {
                let mut buf = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
            None => {}
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The files of `git diff-tree -p --no-prefix` output.
pub fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = vec![];
    let mut old = 0u32;
    let mut new = 0u32;
    let mut in_hunk = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // (Its path, if its two halves are the same one, which a file
            // without "---" and "+++" lines, ex: a binary one, needs.)
            let half = rest.len() / 2;
            let path = (rest.len() % 2 == 1
                && rest.is_char_boundary(half)
                && rest.as_bytes()[half] == b' '
                && rest[..half] == rest[half + 1..])
                .then(|| diff_path(&rest[..half]))
                .unwrap_or_default();
            files.push(FileDiff {
                path,
                status: "changed".to_string(),
                ..Default::default()
            });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if in_hunk {
            match line.as_bytes().first() {
                Some(b' ') | Some(b'-') | Some(b'+') => {
                    let kind = &line[..1];
                    let text = line[1..].to_string();
                    let hunk = file.hunks.last_mut().unwrap();
                    let (o, n) = match kind {
                        " " => (Some(old), Some(new)),
                        "-" => (Some(old), None),
                        _ => (None, Some(new)),
                    };
                    if o.is_some() {
                        old += 1;
                    }
                    if n.is_some() {
                        new += 1;
                    }
                    hunk.lines.push(HunkLine {
                        kind: kind.to_string(),
                        old: o,
                        new: n,
                        text,
                        ..Default::default()
                    });
                    continue;
                }
                // ("\ No newline at end of file".)
                Some(b'\\') => continue,
                _ => in_hunk = false,
            }
        }
        if let Some(rest) = line.strip_prefix("@@ -") {
            // ("@@ -a,b +c,d @@ context", the counts optional.)
            let mut parts = rest.split(' ');
            let start = |part: Option<&str>| {
                part.and_then(|p| {
                    p.trim_start_matches('+')
                        .split(',')
                        .next()?
                        .parse::<u32>()
                        .ok()
                })
                .unwrap_or(0)
            };
            old = start(parts.next()).max(1);
            new = start(parts.next()).max(1);
            file.hunks.push(Hunk::default());
            in_hunk = true;
        } else if let Some(path) = line.strip_prefix("rename from ") {
            file.old_path = Some(diff_path(path));
            file.status = "renamed".to_string();
        } else if let Some(path) = line.strip_prefix("copy from ") {
            file.old_path = Some(diff_path(path));
            file.status = "renamed".to_string();
        } else if let Some(path) = line
            .strip_prefix("rename to ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            file.path = diff_path(path);
        } else if line.starts_with("new file mode") {
            file.status = "added".to_string();
        } else if line.starts_with("deleted file mode") {
            file.status = "removed".to_string();
        } else if line.starts_with("Binary files ") {
            file.status = "binary".to_string();
        } else if let Some(path) = line.strip_prefix("--- ") {
            if path != "/dev/null" {
                file.path = diff_path(path);
            }
        } else if let Some(path) = line.strip_prefix("+++ ") {
            if path != "/dev/null" {
                file.path = diff_path(path);
            }
        }
    }
    // (Else, a renamed file's is its old one's.)
    for file in &mut files {
        if file.path.is_empty() {
            file.path = file.old_path.clone().unwrap_or_default();
        }
    }
    files.retain(|file| !file.path.is_empty());
    files
}

/// A commit's metadata and its changes from its first parent (with
/// `CONTEXT_LINES` of context), from the tree's git repository.
pub fn commit_diff_data(
    cfg: &crate::file_format::config::Config,
    tree_name: &str,
    rev: &str,
) -> std::result::Result<CommitDiff, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let rev = commit.id().to_string();
    let parent = commit.parent_id(0).ok().map(|id| id.to_string());
    // (The system git, as `rev_diff_data` uses.)
    let mut command = std::process::Command::new("git");
    command
        .env("GIT_LITERAL_PATHSPECS", "1")
        .arg("diff-tree")
        .arg("-p")
        .arg("-M")
        .arg("--patience")
        .arg("--no-prefix")
        .arg("--no-commit-id")
        .arg(format!("-U{}", CONTEXT_LINES));
    if parent.is_none() {
        command.arg("--root");
    }
    let output = command
        .arg(&rev)
        .current_dir(tree_config.get_git_path()?)
        .output()
        .map_err(|_| "Diff failed")?;
    if !output.status.success() {
        return Err("Diff failed");
    }
    let author = commit.author();
    let time = commit.time();
    let date = chrono::DateTime::from_timestamp(time.seconds(), 0)
        .map(|date| date.to_rfc3339())
        .unwrap_or_default();
    Ok(CommitDiff {
        rev,
        parent,
        author: author.name().unwrap_or("").to_string(),
        date,
        message: String::from_utf8_lossy(commit.message_bytes()).into_owned(),
        files: parse_diff(&crate::git_ops::decode_bytes(output.stdout)),
    })
}

/// A hunk of a diff with all of its file as context (ex: the data API's
/// `rev-diff`'s, `git diff-tree -U100000`'s), as the hunks of its changes
/// with `context` lines of context around them.
fn with_context(hunk: &Hunk, context: usize) -> Vec<Hunk> {
    let changed: Vec<usize> = (0..hunk.lines.len())
        .filter(|i| hunk.lines[*i].kind != " ")
        .collect();
    let mut ranges: Vec<(usize, usize)> = vec![];
    for i in changed {
        let (start, end) = (
            i.saturating_sub(context),
            (i + context).min(hunk.lines.len() - 1),
        );
        match ranges.last_mut() {
            Some((_, last)) if start <= *last + 1 => *last = (*last).max(end),
            _ => ranges.push((start, end)),
        }
    }
    ranges
        .into_iter()
        .map(|(start, end)| Hunk {
            lines: hunk.lines[start..=end].to_vec(),
        })
        .collect()
}

/// A commit's diff from a server without `AbstractServer::commit_diff` of
/// its own (ex: a remote server whose data API is older than its
/// `commit-diff` route): its metadata from `commit_info_json` (its header,
/// parent, date and Phabricator revision), its files from its explore data,
/// and their diffs from `rev_diff`.
pub async fn commit_diff_from_parts(
    server: &(dyn AbstractServer + Send + Sync),
    rev: &str,
) -> Result<CommitDiff> {
    lazy_static! {
        static ref TAG: regex::Regex = regex::Regex::new(r"<[^>]*>").unwrap();
    }
    let text =
        |html: &str| crate::html_escape::unescape_xml(&TAG.replace_all(html, "")).into_owned();
    let info: Value = serde_json::from_str(&server.commit_info_json(rev).await?)?;
    let info = info.get(0).cloned().unwrap_or_default();
    let header = info["header"].as_str().unwrap_or("");
    // (Its summary, then "<br><i>Author <email>, date</i>".)
    let (summary, byline) = header.split_once("<br>").unwrap_or((header, ""));
    let byline = text(byline);
    let author = byline
        .split_once(" <")
        .or_else(|| byline.split_once(", "))
        .map_or(byline.as_str(), |(name, _)| name)
        .trim()
        .to_string();
    let mut message = text(summary).trim().to_string();
    if let Some(phab) = info["phab"].as_str() {
        message.push_str(&format!("\n\nDifferential Revision: {}\n", phab));
    }
    let date = info["date"]
        .as_str()
        .and_then(|date| chrono::DateTime::parse_from_rfc2822(date).ok())
        .map(|date| date.to_rfc3339())
        .unwrap_or_default();
    let explore = server.explore(&[rev.to_string()], None).await?;
    let paths: Vec<String> = explore["file_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["files"].as_array().into_iter().flatten())
        .filter_map(|file| file["path"].as_str().map(str::to_string))
        .collect();
    let mut files = vec![];
    for path in paths {
        let Some(diff) = server.rev_diff(rev, &path).await? else {
            continue;
        };
        for mut file in parse_diff(&diff.diff) {
            file.hunks = file
                .hunks
                .iter()
                .flat_map(|hunk| with_context(hunk, CONTEXT_LINES))
                .collect();
            files.push(file);
        }
    }
    Ok(CommitDiff {
        rev: rev.to_string(),
        parent: info["parent"].as_str().map(str::to_string),
        author,
        date,
        message,
        files,
    })
}

// ## The view

/// Where a hunk is presented: interfaces, then definitions, then uses, then
/// tests (see the module's documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Idl,
    Definitions,
    Uses,
    Tests,
}

impl Section {
    pub const ALL: [Section; 4] = [
        Section::Idl,
        Section::Definitions,
        Section::Uses,
        Section::Tests,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Section::Idl => "idl",
            Section::Definitions => "definitions",
            Section::Uses => "uses",
            Section::Tests => "tests",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::Idl => "Interfaces",
            Section::Definitions => "Definitions",
            Section::Uses => "Uses",
            Section::Tests => "Tests",
        }
    }

    /// What it has, for its sticky heading.
    pub fn about(self) -> &'static str {
        match self {
            Section::Idl => "its IDL, WebIDL and IPDL, which the code implements",
            Section::Definitions => {
                "its headers, and the definitions it adds or whose declarations it changes, before their uses"
            }
            Section::Uses => "its other changes, in what the definitions' users do",
            Section::Tests => "its tests, last",
        }
    }
}

/// A try push's review view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewView {
    pub push: String,
    /// The review's key (see review-state.js): `{tree}/try/{push}`.
    pub key: String,
    /// What the review is of, for finding it later: `["rev", hash]`,
    /// `["bug", n]`, `["phab", n]`.
    pub indices: Vec<Value>,
    pub patches: Vec<ReviewPatch>,
    /// The patches' groups (see `group_patches`).
    #[serde(default)]
    pub groups: Vec<PatchGroup>,
    /// The push's commits which only select try's tasks, left out.
    pub task_selections: usize,
    /// Whether the push's stack has more commits than the view.
    pub truncated: bool,
    /// What its page said of its semantic analysis (see `try_import::TryPush`):
    /// the head of its stack rebased onto the indexed revision, whose pages
    /// have it, or why its import's analysis failed; the trunk's revisions it
    /// has besides the stack; and the commits that have landed, which it
    /// leaves out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_failure: Option<crate::try_import::SemanticFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_trunk: Option<crate::try_import::SemanticTrunk>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landed: Vec<crate::try_import::LandedCommit>,
    /// Its groups' parts of the push's slicing, for their diagrams (see
    /// `add_slicing`; not kept in the view's cache).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slicing: Option<ReviewSlicing>,
    /// Its code's symbols' SYM_INFO, now (the indexed revision's) and patched
    /// (the push's rebased head's), as `{"now": ..., "patched": ...}`, and
    /// its blames' infos, for the page's `SYM_INFOS` and `BLAME_INFOS` (see
    /// `add_line_html`; not kept in the view's cache).
    #[serde(skip)]
    pub sym_infos: Option<Value>,
    #[serde(skip)]
    pub blame_infos: Vec<Value>,
    /// The later patches' changes of the lines patches add or keep (see
    /// `add_later_changes`; not kept in the view's cache).
    #[serde(skip)]
    pub later: Vec<LaterChange>,
    /// The SYM_INFO of its block diagrams' nodes (see `GroupSlicing::block`),
    /// as the page's script's JSON (not kept in the view's cache).
    #[serde(skip)]
    pub block_sym_info: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewPatch {
    /// Its part's id: `p:<rev>`.
    pub id: String,
    pub rev: String,
    /// The commit whose code the review shows: its commit in the push's
    /// stack rebased onto the indexed revision, if it has one (see
    /// `rebased_commits`), else itself.  And that one's parent, its files'
    /// "then" (see `add_line_html`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub summary: String,
    pub author: String,
    pub date: String,
    pub backout: bool,
    pub bugs: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phab: Option<u64>,
    pub files: Vec<ReviewFile>,
    /// How many of its files it leaves out (over `MAX_FILES`).
    #[serde(default)]
    pub files_left_out: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewFile {
    /// Its part's id: `f:<rev>:<path>`.
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    pub path_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    /// The classes and functions its hunks are in, and the classes those
    /// functions are in, in the file's order.
    pub defs: Vec<ReviewDef>,
    pub hunks: Vec<ReviewHunk>,
    /// Whether its hunks were cut (over `MAX_FILE_LINES` lines).
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewDef {
    /// Its part's id: `d:<rev>:<path>:<pretty>`.
    pub id: String,
    pub pretty: String,
    pub name: String,
    pub kind: String,
    /// "class" or "function".
    pub level: String,
    /// The id of the definition it's in, if it's in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// What the commit did to it (see `OutlineStatus`).
    pub status: String,
    /// It's only declared here (ex: a method in its class in a header).
    #[serde(default)]
    pub decl: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewHunk {
    /// Its part's id: `h:<rev>:<path>:<hash of its changed lines>`.
    pub id: String,
    /// The id of the innermost definition it's in, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub def: Option<String>,
    pub section: Section,
    pub lines: Vec<HunkLine>,
    pub added: u32,
    pub removed: u32,
}

/// A stable hash (FNV-1a, 64 bits) of a hunk's changed lines, for its id.
fn hunk_hash(path: &str, hunk: &Hunk) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    add(path.as_bytes());
    for line in &hunk.lines {
        if line.kind != " " {
            add(line.kind.as_bytes());
            add(line.text.as_bytes());
            add(b"\n");
        }
    }
    format!("{:016x}", hash)
}

/// The level of an outline entry's kind, if it's a part of the review (the
/// others, ex: namespaces and fields, are their containers').
fn def_level(kind: &str) -> Option<&'static str> {
    match kind {
        "class" | "struct" | "union" | "enum" | "interface" | "protocol" | "trait" | "impl"
        | "message" | "dictionary" | "module" => Some("class"),
        "function" | "method" | "constructor" | "destructor" | "lambda" | "macro" => {
            Some("function")
        }
        _ => None,
    }
}

fn status_name(status: OutlineStatus) -> &'static str {
    match status {
        OutlineStatus::Unchanged => "unchanged",
        OutlineStatus::Changed => "changed",
        OutlineStatus::Added => "added",
        OutlineStatus::Removed => "removed",
        OutlineStatus::Renamed => "renamed",
        OutlineStatus::Moved => "moved",
    }
}

fn extension(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or("", |(_, ext)| ext)
}

fn is_idl(path: &str) -> bool {
    matches!(extension(path), "idl" | "webidl" | "ipdl" | "ipdlh")
}

fn is_header(path: &str) -> bool {
    matches!(extension(path), "h" | "hh" | "hpp" | "hxx" | "inc") || path.ends_with(".d.ts")
}

/// Whether a path looks like a test's, for files the index doesn't have (ex:
/// those the push adds), whose path kind it can't say.
fn looks_like_test(path: &str) -> bool {
    path.split('/').any(|part| {
        matches!(
            part,
            "test" | "tests" | "gtest" | "gtests" | "mochitest" | "crashtests" | "reftests"
        )
    }) || path.starts_with("testing/web-platform/tests/")
}

/// The innermost outline entry (an index into `entries`) whose lines (or,
/// for a removed one, whose old lines) hold a hunk's changes, and its
/// ancestors (by their pretty names' "::"), innermost first.
fn hunk_entries(entries: &[OutlineEntry], hunk: &Hunk) -> Vec<usize> {
    let new_range = hunk.new_range();
    let old_range = hunk.old_range();
    let holds = |entry: &OutlineEntry| -> bool {
        let new_holds = match (entry.lines, new_range) {
            (Some((first, last)), Some((a, b))) => first <= a && b <= last,
            _ => false,
        };
        // (A removed definition has only its old lines.)
        let old_holds = entry.status == OutlineStatus::Removed
            && match (entry.from_lines, old_range) {
                (Some((first, last)), Some((a, b))) => first <= a && b <= last,
                _ => false,
            };
        new_holds || old_holds
    };
    // (The innermost: the deepest holding it, then the shortest.)
    let mut holding: Vec<usize> = (0..entries.len())
        .filter(|i| def_level(&entries[*i].kind).is_some() && holds(&entries[*i]))
        .collect();
    holding.sort_by_key(|i| {
        let entry = &entries[*i];
        let span = entry
            .lines
            .or(entry.from_lines)
            .map_or(u32::MAX, |(a, b)| b - a);
        (std::cmp::Reverse(entry.depth), span)
    });
    holding
}

/// A hunk split where its changed lines' innermost definitions change (ex: a
/// hunk changing a function's signature and adding the function after it),
/// each part with its context (up to `CONTEXT_LINES` lines, but not into
/// another part's changes), so that each part is in one definition.
fn split_hunk(entries: &[OutlineEntry], hunk: &Hunk) -> Vec<Hunk> {
    let last_new = hunk.last_new();
    // (A removed line's new-side position: the next kept or added line's.)
    let mut next_new = vec![last_new + 1; hunk.lines.len()];
    let mut next = last_new + 1;
    for (i, line) in hunk.lines.iter().enumerate().rev() {
        if let Some(new) = line.new {
            next = new;
        }
        next_new[i] = next;
    }
    let innermost = |i: usize| -> Option<usize> {
        let line = &hunk.lines[i];
        let position = line.new.unwrap_or(next_new[i]);
        (0..entries.len())
            .filter(|e| {
                let entry = &entries[*e];
                def_level(&entry.kind).is_some()
                    && match (entry.status, line.kind.as_str()) {
                        (OutlineStatus::Removed, "-") => entry
                            .from_lines
                            .zip(line.old)
                            .is_some_and(|((first, last), old)| first <= old && old <= last),
                        (OutlineStatus::Removed, _) => false,
                        _ => entry
                            .lines
                            .is_some_and(|(first, last)| first <= position && position <= last),
                    }
            })
            .max_by_key(|e| {
                let entry = &entries[*e];
                let span = entry.lines.or(entry.from_lines).map_or(0, |(a, b)| b - a);
                (entry.depth, std::cmp::Reverse(span))
            })
    };
    // (The runs of changed lines with the same innermost definition, as
    // [definition, first changed line, last changed line].)
    // (A blank changed line is in the run beside it, rather than a run of
    // its own: the next one's, if it's before any.)
    let mut groups: Vec<(Option<usize>, usize, usize)> = vec![];
    let mut leading_blank: Option<usize> = None;
    for (i, line) in hunk.lines.iter().enumerate() {
        if line.kind == " " {
            continue;
        }
        if line.text.trim().is_empty() {
            match groups.last_mut() {
                Some((_, _, last)) => *last = i,
                None => {
                    leading_blank.get_or_insert(i);
                }
            }
            continue;
        }
        let def = innermost(i);
        match groups.last_mut() {
            Some((group_def, _, last)) if *group_def == def => *last = i,
            _ => groups.push((def, leading_blank.take().unwrap_or(i), i)),
        }
    }
    if groups.len() < 2 {
        return vec![hunk.clone()];
    }
    let mut parts = vec![];
    for (g, (_, first, last)) in groups.iter().enumerate() {
        let start = first.saturating_sub(CONTEXT_LINES).max(match g {
            0 => 0,
            _ => groups[g - 1].2 + 1,
        });
        let end = (last + CONTEXT_LINES)
            .min(hunk.lines.len() - 1)
            .min(groups.get(g + 1).map_or(usize::MAX, |next| next.1 - 1));
        parts.push(Hunk {
            lines: hunk.lines[start..=end].to_vec(),
        });
    }
    parts
}

/// A file of a patch, its hunks under their definitions, from its diff and
/// the commit's outline diff of it.
fn review_file(
    rev: &str,
    file: FileDiff,
    outline: Option<&OutlineDiff>,
    path_kind: String,
    subsystem: Option<String>,
) -> ReviewFile {
    let file_id = format!("f:{}:{}", rev, file.path);
    let is_test = path_kind == "test";
    let mut defs: BTreeMap<String, (usize, ReviewDef)> = BTreeMap::new();
    let mut hunks = vec![];
    let mut lines = 0;
    let mut truncated = false;
    let split: Vec<Hunk> = file
        .hunks
        .iter()
        .flat_map(|hunk| match outline {
            Some(outline) => split_hunk(&outline.entries, hunk),
            None => vec![hunk.clone()],
        })
        .collect();
    for hunk in &split {
        if lines + hunk.lines.len() > MAX_FILE_LINES && !hunks.is_empty() {
            truncated = true;
            break;
        }
        lines += hunk.lines.len();
        // (The definitions holding it: the innermost first, whose own
        // definition it is; the outer ones are its ancestors.)
        let held = outline.map_or(vec![], |outline| hunk_entries(&outline.entries, hunk));
        let mut chain: Vec<&OutlineEntry> = vec![];
        if let (Some(outline), Some(first)) = (outline, held.first()) {
            let inner = &outline.entries[*first];
            chain.push(inner);
            // (Its containers: the entries holding it whose pretty names it
            // starts with, outer ones last.)
            for i in &held[1..] {
                let entry = &outline.entries[*i];
                if inner.pretty.starts_with(&format!("{}::", entry.pretty)) {
                    chain.push(entry);
                }
            }
        }
        let mut parent: Option<String> = None;
        for (order, entry) in chain.iter().rev().enumerate() {
            let id = format!("d:{}:{}:{}", rev, file.path, entry.pretty);
            let first_line = entry.lines.or(entry.from_lines).map_or(0, |(a, _)| a) as usize;
            defs.entry(id.clone()).or_insert_with(|| {
                (
                    first_line * 16 + order,
                    ReviewDef {
                        id: id.clone(),
                        pretty: entry.pretty.clone(),
                        name: entry.name.clone(),
                        kind: entry.kind.clone(),
                        level: def_level(&entry.kind).unwrap_or("function").to_string(),
                        parent: parent.clone(),
                        status: status_name(entry.status).to_string(),
                        decl: entry.decl,
                    },
                )
            });
            parent = Some(id);
        }
        let inner = chain.first();
        let section = if is_test {
            Section::Tests
        } else if is_idl(&file.path) {
            Section::Idl
        } else if is_header(&file.path)
            || file.status == "added"
            || inner.is_some_and(|entry| {
                entry.decl
                    || entry.status == OutlineStatus::Added
                    || def_level(&entry.kind) == Some("class")
                    // (Its signature: its first lines, ex: a function's
                    // parameters' changes, before its callers' uses.)
                    || match (entry.lines, hunk.new_range()) {
                        (Some((first, _)), Some((a, _))) => a <= first + 1,
                        _ => false,
                    }
            })
        {
            Section::Definitions
        } else {
            Section::Uses
        };
        // (Its context within its definition: not the end of the one before
        // it, nor the start of the one after it.)
        let lines = match inner.and_then(|entry| entry.lines.or(entry.from_lines)) {
            Some((first, last)) => {
                let within = |line: &HunkLine| {
                    let number = match inner.is_some_and(|e| e.status == OutlineStatus::Removed) {
                        true => line.old,
                        false => line.new,
                    };
                    line.kind != " " || number.is_some_and(|n| first <= n && n <= last)
                };
                let start = hunk.lines.iter().position(within).unwrap_or(0);
                let end = hunk
                    .lines
                    .iter()
                    .rposition(within)
                    .unwrap_or(hunk.lines.len() - 1);
                hunk.lines[start..=end].to_vec()
            }
            None => hunk.lines.clone(),
        };
        hunks.push(ReviewHunk {
            id: format!("h:{}:{}:{}", rev, file.path, hunk_hash(&file.path, hunk)),
            def: parent,
            section,
            added: hunk.count("+"),
            removed: hunk.count("-"),
            lines,
        });
    }
    let mut defs: Vec<(usize, ReviewDef)> = defs.into_values().collect();
    defs.sort_by_key(|(order, _)| *order);
    ReviewFile {
        id: file_id,
        path: file.path,
        old_path: file.old_path,
        status: file.status,
        path_kind,
        subsystem,
        defs: defs.into_iter().map(|(_, def)| def).collect(),
        hunks,
        truncated,
    }
}

/// The bugs and Phabricator revision a commit's message names.
fn message_refs(summary: &str, message: &str) -> (Vec<u64>, Option<u64>) {
    lazy_static! {
        static ref BUG: regex::Regex = regex::Regex::new(r"(?i)\bbug\s*(\d{3,9})\b").unwrap();
        static ref PHAB: regex::Regex = regex::Regex::new(
            r"(?m)^Differential Revision:\s*https?://phabricator\.services\.mozilla\.com/D(\d+)"
        )
        .unwrap();
    }
    let mut bugs: Vec<u64> = BUG
        .captures_iter(summary)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    bugs.dedup();
    let phab = PHAB.captures(message).and_then(|c| c[1].parse().ok());
    (bugs, phab)
}

/// The review view of the try push `push`, from its stack's commits' diffs
/// and outline diffs.
/// The commits of the try push `push`'s stack (`revs`) rebased onto the
/// indexed revision (see rebase-try-push.sh), by their own, as its explore
/// data names them (see `explore::ExploreCommit::rebased`), if it has them.
async fn rebased_commits(
    server: &(dyn AbstractServer + Send + Sync),
    push: &str,
    revs: &[String],
) -> BTreeMap<String, String> {
    let stack_url = crate::try_import::push_url_path(push);
    let Ok(explore) = server.explore(revs, Some(&stack_url)).await else {
        return BTreeMap::new();
    };
    explore["commits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|commit| {
            Some((
                commit["rev"].as_str()?.to_string(),
                commit["rebased"].as_str()?.to_string(),
            ))
        })
        .collect()
}

pub async fn review_view(
    server: &(dyn AbstractServer + Send + Sync),
    push: &str,
) -> Result<ReviewView> {
    let tree = server.tree_info()?.name;
    let Some(try_push) = server.try_push(push).await? else {
        return Err(ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::DataLayer,
            message: format!(
                "The try push {} isn't imported (its page, /{}/try/hg/{}, can import it).",
                push, tree, push
            ),
        }));
    };
    let mut indices: Vec<Value> = vec![json!(["rev", push])];
    let mut patches = vec![];
    let revs: Vec<String> = try_push.stack.iter().map(|c| c.rev.clone()).collect();
    let rebased = rebased_commits(server, push, &revs).await;
    for commit in &try_push.stack {
        // (Its code is its rebased commit's, if it has one, whose parent's
        // analysis, for its stack's first commit changing a file, is the
        // indexed revision's; its parts are its own's, which stay the same
        // when it's rebased again.)
        let shown = rebased.get(&commit.rev).cloned();
        let diff = server
            .commit_diff(shown.as_deref().unwrap_or(&commit.rev))
            .await?;
        let summary = diff.message.lines().next().unwrap_or("").to_string();
        let (bugs, phab) = message_refs(&summary, &diff.message);
        indices.push(json!(["rev", commit.rev]));
        for bug in &bugs {
            indices.push(json!(["bug", bug]));
        }
        if let Some(phab) = phab {
            indices.push(json!(["phab", phab]));
        }
        let files_left_out = diff.files.len().saturating_sub(MAX_FILES);
        let mut files = vec![];
        for file in diff.files.into_iter().take(MAX_FILES) {
            let outline = match (&diff.parent, file.status.as_str()) {
                (_, "binary") | (None, _) => None,
                (Some(parent), _) => server
                    .outline_diff(parent, &diff.rev, &file.path, true)
                    .await
                    .ok()
                    .and_then(|value| serde_json::from_value::<OutlineDiff>(value).ok()),
            };
            // (By the tree's path kinds' rules, for the files the indexed
            // revision doesn't have, ex: those the push adds.)
            let (path_kind, subsystem) = match server.path_facet_info(&file.path) {
                (kind, _, subsystem) if !kind.is_empty() => {
                    (kind.to_string(), subsystem.map(|s| s.to_string()))
                }
                _ if looks_like_test(&file.path) => ("test".to_string(), None),
                _ => ("normal".to_string(), None),
            };
            files.push(review_file(
                &commit.rev,
                file,
                outline.as_ref(),
                path_kind,
                subsystem,
            ));
        }
        patches.push(ReviewPatch {
            id: format!("p:{}", commit.rev),
            parent: diff.parent.clone(),
            rev: commit.rev.clone(),
            shown,
            summary,
            author: diff.author,
            date: diff.date,
            backout: commit.backout,
            bugs,
            phab,
            files,
            files_left_out,
        });
    }
    let mut seen = BTreeSet::new();
    indices.retain(|index| seen.insert(index.to_string()));
    let groups = group_patches(&patches);
    Ok(ReviewView {
        push: push.to_string(),
        key: format!("{}/try/{}", tree, push),
        indices,
        patches,
        groups,
        task_selections: try_push.task_selections,
        truncated: try_push.truncated,
        semantic_head: try_push.semantic_head,
        semantic_failure: try_push.semantic_failure,
        semantic_trunk: try_push.semantic_trunk,
        landed: try_push.landed,
        slicing: None,
        sym_infos: None,
        blame_infos: vec![],
        later: vec![],
        block_sym_info: None,
    })
}

// ## Syntax highlighting

/// The most bytes of a file whose lines are highlighted (bigger ones' are
/// plain text).
const MAX_HIGHLIGHTED_FILE: usize = 2 * 1024 * 1024;

/// A source listing's row's code (see `chunked_gzip::ROW_START`): its
/// `source-line` cell's HTML, with its symbols.
fn row_code(row: &str) -> Option<String> {
    const CODE: &str = r#"<code role="cell" class="source-line">"#;
    let start = row.find(CODE)? + CODE.len();
    let end = row.rfind("</code>")?;
    (start <= end).then(|| row[start..end].trim_end_matches('\n').to_string())
}

/// The view's hunks' lines' HTML (see `HunkLine`), as the source listings
/// have it, and their blame:
/// - Each line's code, highlighted, from the listing of the push's rebased
///   head (patched, for the lines a patch adds or keeps which are the head's
///   too) or of the indexed revision (now, for the lines before a patch which
///   are the indexed revision's, as all of those of the stack's first patch
///   changing a file are, the stack being rebased onto it), with their
///   symbols, whose SYM_INFO the view has (`sym_infos`, for the page's
///   `SYM_INFOS` by side; see context-menu.js); the other lines formatted as
///   their listings would be (`format::format_code`, without links).
/// - Each line's blame strip, by its revision's token-centric blame (the
///   patch's commit's, and its parent's), whose infos the view has
///   (`blame_infos`, for the page's `BLAME_INFOS`; see hyperblame.js).
/// Remote servers have no files' texts or blame, so their lines are plain.
pub async fn add_line_html(server: &(dyn AbstractServer + Send + Sync), view: &mut ReviewView) {
    async fn text(
        server: &(dyn AbstractServer + Send + Sync),
        texts: &mut BTreeMap<(String, String), Option<String>>,
        rev: &str,
        path: &str,
    ) -> Option<String> {
        let key = (rev.to_string(), path.to_string());
        if !texts.contains_key(&key) {
            let text = server.rev_text(rev, path).await.ok().flatten();
            texts.insert(key.clone(), text);
        }
        texts.get(&key).cloned().flatten()
    }
    fn formatted(path: &str, text: &str) -> Option<Vec<String>> {
        if text.len() > MAX_HIGHLIGHTED_FILE {
            return None;
        }
        match crate::languages::select_formatting(path) {
            crate::languages::FormatAs::Binary => None,
            format => Some(
                crate::format::format_code(
                    None,
                    &crate::file_format::jumpref::TreeJumprefs::default(),
                    format,
                    path,
                    text,
                    &[],
                )
                .0
                .into_iter()
                .map(|line| line.line)
                .collect(),
            ),
        }
    }
    // (The listing's rows of `listing`'s file at `path` for the lines of
    // `text` that `listing_text` has too, by their lines in `text`.)
    async fn listing_code(
        listing: &(dyn AbstractServer + Send + Sync),
        path: &str,
        text: &str,
        listing_text: &str,
        lines: &BTreeSet<u32>,
    ) -> BTreeMap<u32, String> {
        let map = crate::rev_lines::line_map(text.as_bytes(), listing_text.as_bytes());
        let at: BTreeMap<u32, u32> = lines
            .iter()
            .filter_map(|line| Some((*line, (*map.get((*line as usize).checked_sub(1)?)?)?)))
            .collect();
        let wanted: BTreeSet<u32> = at.values().copied().collect();
        if wanted.is_empty() {
            return BTreeMap::new();
        }
        let Ok(mut rows) = listing
            .fetch_html_lines(vec![(ustr::ustr(path), wanted)])
            .await
        else {
            return BTreeMap::new();
        };
        let Some(rows) = rows.remove(&ustr::ustr(path)) else {
            return BTreeMap::new();
        };
        at.into_iter()
            .filter_map(|(line, listing_line)| {
                Some((line, row_code(&rows.get(&listing_line)?.html)?))
            })
            .collect()
    }
    let meta = server.try_delta_meta();
    let base_server = meta.as_ref().and_then(|meta| server.old_side(&meta.base));
    let mut texts: BTreeMap<(String, String), Option<String>> = BTreeMap::new();
    let mut syms: [Vec<ustr::Ustr>; 2] = [vec![], vec![]];
    let mut blame_infos: Vec<Value> = vec![];
    // (A revision's file's blame, as a cell maker, once each.)
    let mut blames: BTreeMap<
        (String, String),
        Option<(usize, crate::hyperblame::page_blame::PageBlame)>,
    > = BTreeMap::new();
    for patch in &mut view.patches {
        let shown = patch.shown.clone().unwrap_or_else(|| patch.rev.clone());
        let parent = patch.parent.clone();
        for file in &mut patch.files {
            if file.status == "binary" {
                continue;
            }
            let old_path = file.old_path.clone().unwrap_or_else(|| file.path.clone());
            let now_text = match file.status.as_str() {
                "removed" => None,
                _ => text(server, &mut texts, &shown, &file.path).await,
            };
            let then_text = match (file.status.as_str(), &parent) {
                ("added", _) | (_, None) => None,
                (_, Some(parent)) => text(server, &mut texts, parent, &old_path).await,
            };
            let now_lines: BTreeSet<u32> = file
                .hunks
                .iter()
                .flat_map(|h| h.lines.iter().filter_map(|l| l.new))
                .collect();
            let then_lines: BTreeSet<u32> = file
                .hunks
                .iter()
                .flat_map(|h| h.lines.iter().filter_map(|l| l.old))
                .collect();
            // (Its lines from the listings: patched, the head's; now, the
            // indexed revision's.)
            let mut now_code = BTreeMap::new();
            let mut then_code = BTreeMap::new();
            if let Some(meta) = &meta {
                if let (Some(now_text), Some(head_text)) = (
                    &now_text,
                    text(server, &mut texts, &meta.head, &file.path).await,
                ) {
                    now_code =
                        listing_code(server, &file.path, now_text, &head_text, &now_lines).await;
                }
                if let (Some(then_text), Some(base), Some(base_text)) = (
                    &then_text,
                    base_server.as_deref(),
                    text(server, &mut texts, &meta.base, &old_path).await,
                ) {
                    then_code =
                        listing_code(base, &old_path, then_text, &base_text, &then_lines).await;
                }
            }
            syms[0].extend(
                then_code
                    .values()
                    .flat_map(|html| crate::abstract_server::sym_info::html_symbols(html)),
            );
            syms[1].extend(
                now_code
                    .values()
                    .flat_map(|html| crate::abstract_server::sym_info::html_symbols(html)),
            );
            let now_fmt = now_text.as_deref().and_then(|t| formatted(&file.path, t));
            let then_fmt = then_text.as_deref().and_then(|t| formatted(&old_path, t));
            // (Their blames.)
            let mut blame_of = async |rev: Option<&str>, path: &str| -> Option<usize> {
                let rev = rev?;
                let key = (rev.to_string(), path.to_string());
                if !blames.contains_key(&key) {
                    let page = server
                        .token_blame(rev, path)
                        .await
                        .ok()
                        .flatten()
                        .map(|page| {
                            blame_infos
                                .push(serde_json::to_value(&page.info).unwrap_or(Value::Null));
                            (blame_infos.len() - 1, page)
                        });
                    blames.insert(key.clone(), page);
                }
                blames.get(&key)?.as_ref().map(|(ctx, _)| *ctx)
            };
            let now_ctx = match file.status.as_str() {
                "removed" => None,
                _ => blame_of(Some(&shown), &file.path).await,
            };
            let then_ctx = match file.status.as_str() {
                "added" => None,
                _ => blame_of(parent.as_deref(), &old_path).await,
            };
            let cell = |ctx: Option<usize>, line: Option<u32>| -> Option<BlameCell> {
                let ctx = ctx?;
                let line = line?;
                let (_, page) = blames.values().flatten().find(|(index, _)| *index == ctx)?;
                let strip = page.lines.get((line as usize).checked_sub(1)?)?;
                let (classes, data) = strip.strip_attrs();
                Some(BlameCell {
                    ctx,
                    line,
                    commit: strip
                        .commit
                        .and_then(|commit| page.info.commits.get(commit))
                        .map(|commit| commit.0.clone()),
                    classes,
                    data,
                })
            };
            let at = |lines: &Option<Vec<String>>, n: Option<u32>| -> Option<String> {
                lines.as_ref()?.get((n? as usize).checked_sub(1)?).cloned()
            };
            for hunk in &mut file.hunks {
                for line in &mut hunk.lines {
                    if line.kind != "+" {
                        line.then_html = line
                            .old
                            .and_then(|n| then_code.get(&n).cloned())
                            .or_else(|| at(&then_fmt, line.old));
                        line.then_blame = cell(then_ctx, line.old);
                    }
                    if line.kind != "-" {
                        line.now_html = line
                            .new
                            .and_then(|n| now_code.get(&n).cloned())
                            .or_else(|| at(&now_fmt, line.new));
                        line.now_blame = cell(now_ctx, line.new);
                    }
                }
                // (And the words its changes change.)
                mark_word_changes(&mut hunk.lines);
            }
        }
    }
    // (The symbols' SYM_INFO, now by the indexed revision's server, and
    // patched by the push's.)
    let mut infos = vec![];
    for (side, side_server) in [(0, base_server.as_deref().unwrap_or(server)), (1, server)] {
        let mut info = BTreeMap::new();
        let mut traversed = ustr::UstrMap::default();
        crate::abstract_server::sym_info::add_syms_info(
            side_server,
            std::mem::take(&mut syms[side]),
            &mut info,
            &mut traversed,
        )
        .await;
        infos.push(crate::abstract_server::sym_info::script_json(&info));
    }
    view.sym_infos = Some(json!({
        "now": serde_json::from_str::<Value>(&infos[0]).unwrap_or(json!({})),
        "patched": serde_json::from_str::<Value>(&infos[1]).unwrap_or(json!({})),
    }));
    view.blame_infos = blame_infos;
    add_later_changes(view, &texts);
}

/// The most of a later change's added lines its popup shows.
const LATER_LINES: usize = 6;

/// Marks the lines patches add or keep which later patches change (see
/// `HunkLine::later`): each such line (as it is with its patch) is followed
/// through the next patches changing its file, by their line diffs (with
/// `texts`, the files' texts by revision and path, as `add_line_html` had
/// them), to the first one removing it, whose hunk doing so is its change.
/// (The patches in between leave the file as it was.)
fn add_later_changes(view: &mut ReviewView, texts: &BTreeMap<(String, String), Option<String>>) {
    let text = |rev: &Option<String>, path: &str| -> Option<&String> {
        texts.get(&(rev.clone()?, path.to_string()))?.as_ref()
    };
    // (Each patch's files, by their paths then and now.)
    struct Change {
        patch: usize,
        file: usize,
    }
    let shown =
        |patch: &ReviewPatch| Some(patch.shown.clone().unwrap_or_else(|| patch.rev.clone()));
    let mut later: Vec<LaterChange> = vec![];
    let mut marks: Vec<(usize, usize, usize, usize, usize)> = vec![];
    for (k, patch) in view.patches.iter().enumerate() {
        for (f, file) in patch.files.iter().enumerate() {
            if file.status == "removed" || file.status == "binary" {
                continue;
            }
            // (The later patches changing the file, in order, following
            // renames.)
            let mut path = file.path.clone();
            let mut changes: Vec<Change> = vec![];
            for (j, other) in view.patches.iter().enumerate().skip(k + 1) {
                if let Some(g) = other
                    .files
                    .iter()
                    .position(|o| o.old_path.as_deref().unwrap_or(&o.path) == path)
                {
                    path = other.files[g].path.clone();
                    changes.push(Change { patch: j, file: g });
                }
            }
            if changes.is_empty() {
                continue;
            }
            // (Each later change's line map, from its parent's file to its
            // own.)
            let maps: Vec<Option<Vec<Option<u32>>>> = changes
                .iter()
                .map(|change| {
                    let other = &view.patches[change.patch];
                    let other_file = &other.files[change.file];
                    let before = text(
                        &other.parent,
                        other_file.old_path.as_deref().unwrap_or(&other_file.path),
                    )?;
                    let after = text(&shown(other), &other_file.path)?;
                    Some(crate::rev_lines::line_map(
                        before.as_bytes(),
                        after.as_bytes(),
                    ))
                })
                .collect();
            for (h, hunk) in file.hunks.iter().enumerate() {
                for (l, line) in hunk.lines.iter().enumerate() {
                    if line.kind == "-" {
                        continue;
                    }
                    let Some(mut position) = line.new else {
                        continue;
                    };
                    for (change, map) in changes.iter().zip(&maps) {
                        let other_file = &view.patches[change.patch].files[change.file];
                        // (Its hunk removing the line, if any.)
                        if let Some(other_hunk) = other_file.hunks.iter().find(|other_hunk| {
                            other_hunk
                                .lines
                                .iter()
                                .any(|o| o.kind == "-" && o.old == Some(position))
                        }) {
                            let lines = other_hunk
                                .lines
                                .iter()
                                .filter(|o| o.kind == "+")
                                .take(LATER_LINES)
                                .map(|o| {
                                    o.now_html
                                        .clone()
                                        .unwrap_or_else(|| escape(&o.text).to_string())
                                })
                                .collect();
                            later.push(LaterChange {
                                patch: change.patch,
                                element: element_id(&other_hunk.id, other_hunk.section),
                                lines,
                            });
                            marks.push((k, f, h, l, later.len() - 1));
                            break;
                        }
                        match map
                            .as_ref()
                            .and_then(|map| *map.get((position as usize).checked_sub(1)?)?)
                        {
                            Some(next) => position = next,
                            // (Removed where its hunks don't say, ex: a
                            // diff's other alignment; or its map's unknown.)
                            None => break,
                        }
                    }
                }
            }
        }
    }
    for (k, f, h, l, index) in marks {
        view.patches[k].files[f].hunks[h].lines[l].later = Some(index);
    }
    view.later = later;
}

// ## Patch groups

/// The most changed lines (added and removed) a group of patches has, unless
/// it's one patch: a group past it is more than takes in at once (see
/// try-review-ui-plan.md's "Open" on tuning it).
pub const GROUP_LINES: u32 = 400;

/// A group of related patches (see `group_patches`), for reading them
/// together: its patches (indices into `ReviewView::patches`), consecutive,
/// its honorary members (related patches of other groups), and what relates
/// them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatchGroup {
    pub patches: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub honorary: Vec<usize>,
    /// What the patches share (ex: "widget.h", "class Widget", "bug 2345"),
    /// the first few.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
}

fn patch_lines(patch: &ReviewPatch) -> u32 {
    patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .map(|hunk| hunk.added + hunk.removed)
        .sum()
}

/// What a patch changes that relates it to others: its files, the classes
/// it changes, and its files' directories ("dir " and its path).
fn patch_keys(patch: &ReviewPatch) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for file in &patch.files {
        keys.insert(file.path.clone());
        if let Some((dir, _)) = file.path.rsplit_once('/') {
            keys.insert(format!("dir {}", dir));
        }
        for def in &file.defs {
            if def.level == "class" {
                keys.insert(format!("class {}", def.pretty));
            }
        }
    }
    keys
}

/// How much sharing a key relates patches: a file or a class, 3, a
/// directory, 1, divided by how many of the push's patches have it (sharing
/// what most of a stack's patches change, ex: ipc/glue, says little).
fn key_weight(key: &str, patches_with: usize) -> f64 {
    let weight = if key.starts_with("dir ") { 1.0 } else { 3.0 };
    weight / patches_with.max(1) as f64
}

/// The relation strong enough to group patches, or to make one an honorary
/// member of another's group (a file or a class only a few patches share),
/// and the weak one that groups consecutive patches of the same bug (a
/// directory few patches share).
const STRONG_RELATION: f64 = 1.0;
const WEAK_RELATION: f64 = 0.3;

/// The patches' groups: a patch joins the group of the patches before it if
/// it's related to one of them strongly, or weakly (and they're of the same
/// bugs) and not more to the patch after it, as long as the group's changes
/// stay under `GROUP_LINES` (a patch past it alone is a group of its own);
/// and as honorary members of a group, the other groups' patches strongly
/// related to its own.  (The patch is the unit of review, and a stack's
/// patches are usually in the order to read them, so the groups are of
/// consecutive patches; and better too small than too much to take in.  A
/// stack's patches are usually all of one bug, which says nothing of how
/// they're related.)
pub fn group_patches(patches: &[ReviewPatch]) -> Vec<PatchGroup> {
    let keys: Vec<BTreeSet<String>> = patches.iter().map(patch_keys).collect();
    let lines: Vec<u32> = patches.iter().map(patch_lines).collect();
    let mut patches_with: BTreeMap<&String, usize> = BTreeMap::new();
    for patch_keys in &keys {
        for key in patch_keys {
            *patches_with.entry(key).or_default() += 1;
        }
    }
    let relation = |a: usize, b: usize| -> f64 {
        keys[a]
            .intersection(&keys[b])
            .map(|key| key_weight(key, patches_with[key]))
            .sum()
    };
    let same_bugs = |a: usize, b: usize| {
        let (a, b) = (&patches[a].bugs, &patches[b].bugs);
        a.is_empty() && b.is_empty() || a.iter().any(|bug| b.contains(bug))
    };
    let mut groups: Vec<PatchGroup> = vec![];
    let mut group_lines = 0;
    for i in 0..patches.len() {
        let joins = groups.last().is_some_and(|group| {
            let (best, related) = group.patches.iter().map(|j| (*j, relation(*j, i))).fold(
                (None, 0.0),
                |(best, max), (j, r)| match r > max {
                    true => (Some(j), r),
                    false => (best, max),
                },
            );
            let next = match i + 1 < patches.len() {
                true => relation(i, i + 1),
                false => 0.0,
            };
            group_lines + lines[i] <= GROUP_LINES
                && (related >= STRONG_RELATION
                    || (related >= WEAK_RELATION
                        && best.is_some_and(|j| same_bugs(j, i))
                        && related >= next))
        });
        if joins {
            groups.last_mut().unwrap().patches.push(i);
            group_lines += lines[i];
        } else {
            groups.push(PatchGroup {
                patches: vec![i],
                honorary: vec![],
                shared: vec![],
            });
            group_lines = lines[i];
        }
    }
    for group in &mut groups {
        // (What its patches share, most telling first.)
        if group.patches.len() > 1 {
            let mut shared: BTreeMap<&String, usize> = BTreeMap::new();
            for i in &group.patches {
                for key in &keys[*i] {
                    *shared.entry(key).or_default() += 1;
                }
            }
            let mut shared: Vec<(&String, f64)> = shared
                .into_iter()
                .filter(|(_, n)| *n > 1)
                .map(|(key, n)| (key, n as f64 * key_weight(key, patches_with[key])))
                .collect();
            shared.sort_by(|(a, x), (b, y)| y.total_cmp(x).then_with(|| a.cmp(b)));
            group.shared = shared
                .into_iter()
                .take(4)
                .map(|(key, _)| {
                    key.strip_prefix("dir ")
                        .map_or(key.clone(), |dir| format!("{}/", dir))
                })
                .collect();
        }
        // (Honorary members: the other groups' patches strongly related to
        // one of its own, the nearest first, a few.)
        let mut honorary: Vec<usize> = (0..patches.len())
            .filter(|i| !group.patches.contains(i))
            .filter(|i| {
                group
                    .patches
                    .iter()
                    .any(|j| relation(*i, *j) >= STRONG_RELATION)
            })
            .collect();
        let first = group.patches[0];
        honorary.sort_by_key(|i| (*i as isize - first as isize).abs());
        honorary.truncate(4);
        honorary.sort();
        group.honorary = honorary;
    }
    groups
}

// ## Facets

/// The order of path kinds (see per-file-info.toml's `sort_order`), for the
/// files' order in a section.
fn path_kind_rank(kind: &str) -> u32 {
    match kind {
        "normal" => 0,
        "third_party" => 1,
        "test" => 2,
        "generated" => 3,
        _ => 4,
    }
}

/// The values of a file's facets (in a patch's section; see `view_facets`):
/// its patch, section, path kind, subsystem, directory and type, ancestors
/// included.
fn file_facet_values(
    patch_index: usize,
    file: &ReviewFile,
    section: Section,
) -> Vec<(&'static str, Vec<(String, String)>)> {
    let mut out = vec![
        (
            "patch",
            vec![(
                format!("p{}", patch_index + 1),
                format!("Patch {}", patch_index + 1),
            )],
        ),
        (
            "section",
            vec![(section.slug().to_string(), section.label().to_string())],
        ),
    ];
    if let Some(subsystem) = &file.subsystem {
        let mut values = vec![];
        let mut so_far = String::new();
        for part in subsystem.split('/') {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(part);
            values.push((so_far.clone(), part.to_string()));
        }
        out.push(("subsystem", values));
    }
    let dirs: Vec<&str> = file.path.split('/').collect();
    if dirs.len() > 1 {
        let mut values = vec![];
        let mut so_far = String::new();
        for part in &dirs[..dirs.len() - 1] {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(part);
            values.push((so_far.clone(), part.to_string()));
        }
        out.push(("directory", values));
    }
    out.push((
        "type",
        crate::cmd_pipeline::facets::type_value_path(&file.path),
    ));
    out
}

/// The navigation panel's facets of the view (see `TextFile::facets`), whose
/// items are its patches' sections' files.
pub fn view_facets(view: &ReviewView) -> Value {
    use crate::cmd_pipeline::facets::{FacetValueView, FacetView};
    #[derive(Default)]
    struct Level {
        name: String,
        count: u32,
        nested: BTreeMap<String, Level>,
    }
    let mut roots: BTreeMap<&'static str, Level> = BTreeMap::new();
    for (i, patch) in view.patches.iter().enumerate() {
        for file in &patch.files {
            let sections: BTreeSet<Section> = file.hunks.iter().map(|h| h.section).collect();
            for section in sections {
                for (key, path) in file_facet_values(i, file, section) {
                    let mut level = roots.entry(key).or_default();
                    for (value, name) in path {
                        level = level.nested.entry(value).or_default();
                        level.name = name;
                        level.count += 1;
                    }
                }
            }
        }
    }
    fn views(level: Level, by_count: bool) -> Vec<FacetValueView> {
        let mut values: Vec<(String, Level)> = level.nested.into_iter().collect();
        if by_count {
            values.sort_by(|(a_value, a), (b_value, b)| {
                b.count.cmp(&a.count).then_with(|| a_value.cmp(b_value))
            });
        }
        values
            .into_iter()
            .map(|(value, level)| FacetValueView {
                value,
                title: level.name.clone(),
                name: level.name.clone(),
                count: level.count,
                values: views(level, by_count),
                ..Default::default()
            })
            .collect()
    }
    let mut facets = vec![];
    for (key, label) in [
        ("patch", "Patch"),
        ("section", "Section"),
        ("subsystem", "Subsystem"),
        ("directory", "Directory"),
        ("type", "Type"),
    ] {
        let Some(mut level) = roots.remove(key) else {
            continue;
        };
        if level.nested.len() < 2 && key != "subsystem" && key != "directory" {
            continue;
        }
        // (Patches and sections in their own order; the rest by count.)
        let by_count = !matches!(key, "patch" | "section");
        if key == "section" {
            let order: Vec<&str> = Section::ALL.iter().map(|s| s.slug()).collect();
            let mut nested: Vec<(String, Level)> =
                std::mem::take(&mut level.nested).into_iter().collect();
            nested.sort_by_key(|(value, _)| order.iter().position(|o| o == value));
            let values: Vec<FacetValueView> = nested
                .into_iter()
                .map(|(value, level)| FacetValueView {
                    value,
                    title: level.name.clone(),
                    name: level.name.clone(),
                    count: level.count,
                    ..Default::default()
                })
                .collect();
            facets.push(FacetView::new(key, label.to_string(), values));
            continue;
        }
        if key == "patch" {
            let mut nested: Vec<(String, Level)> =
                std::mem::take(&mut level.nested).into_iter().collect();
            nested.sort_by_key(|(value, _)| value[1..].parse::<usize>().unwrap_or(0));
            let values: Vec<FacetValueView> = nested
                .into_iter()
                .map(|(value, level)| FacetValueView {
                    value,
                    title: level.name.clone(),
                    name: level.name.clone(),
                    count: level.count,
                    ..Default::default()
                })
                .collect();
            facets.push(FacetView::new(key, label.to_string(), values));
            continue;
        }
        facets.push(FacetView::new(
            key,
            label.to_string(),
            views(level, by_count),
        ));
    }
    json!({ "facets": facets, "noun": "file", "nouns": "files" })
}

// ## Dataflow slicing

/// The most units of a group's slicing its diagram has (the rest are left
/// out, and it says so).
const MAX_GROUP_UNITS: usize = 60;

/// The most classes a group's block diagram has (see `GroupSlicing::classes`).
const MAX_BLOCK_CLASSES: usize = 12;

/// The push's slicing (`slice-rev:`'s; see `dataflow::slice`) by the view's
/// patch groups (see `group_slicings`), or why it isn't there.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReviewSlicing {
    /// Its query (ex: `slice-rev:<base>..<head>`) and revision (the head),
    /// for the link to its page.
    pub query: String,
    pub rev: String,
    /// By group (the view's `groups`, or each patch's own).
    pub groups: Vec<GroupSlicing>,
    /// Why there are no diagrams, if there aren't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A patch group's part of the push's slicing, for its diagram
/// (try-review.js's): the units of its patches' commits (their changes'
/// functions, or their other lines), their neighbors in other groups, and
/// the links between them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GroupSlicing {
    pub nodes: Vec<SliceNode>,
    pub edges: Vec<SliceEdge>,
    /// The classes its patches' units are in (their symbols), most units
    /// first, at most `MAX_BLOCK_CLASSES`, for its block diagram (see
    /// `block`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    /// Its block diagram, as SVG (see cmd_try_review.rs's `block_diagrams`;
    /// not in the page's data).
    #[serde(skip)]
    pub block: Option<String>,
    /// How many of its units its diagram leaves out (past
    /// `MAX_GROUP_UNITS`).
    #[serde(default)]
    pub left_out: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SliceNode {
    /// Its function's name (its pretty name), else its class's (its
    /// declarations'), else its file's path.
    pub label: String,
    /// Its class's, for the diagram's records (see graph-records.js).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub path: String,
    /// What its patch does to them: "added", "removed" or "changed" (both),
    /// and its lines, the ones it adds (in the push's head's coordinates),
    /// or else the ones it removes (in its base's).
    pub change: String,
    pub lines: Vec<u32>,
    /// Its patch (the view's index), and the part of it its lines are in (a
    /// definition's, else its file's), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// If it's another group's (a neighbor), that group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<usize>,
    /// The text of the lines it adds, and of those it removes.
    #[serde(default)]
    pub text: Vec<String>,
    #[serde(default)]
    pub old_text: Vec<String>,
    /// Its card's lines, in the push's head: the ones it adds, and the
    /// others on their dataflow (see `card_lines`), if they're known.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub card: Vec<CardLine>,
    /// Its units (their ids in the slicing).
    #[serde(skip)]
    pub units: Vec<usize>,
}

/// A line of a node's card (see `SliceNode::card`): its number and text, and
/// whether its patch adds it, or else how much it's on its changes'
/// dataflow, and why (see `dataflow::impact::LineImpact`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CardLine {
    pub line: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub added: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<String>,
}

/// The most lines of its own a card shows, and of its dataflow's (the
/// likeliest).
const CARD_ADDED_LINES: usize = 12;
const CARD_FLOW_LINES: usize = 8;

/// The nodes' cards' lines (see `SliceNode::card`), by the slicing's
/// `slice-format:lines` (`lines`; see `dataflow::impact::slicing_lines`):
/// each node's added lines, and the lines on their dataflow in its units'
/// functions, the likeliest first, by `text` (a path's text in the push's
/// head, if it's had).
pub fn card_lines(
    groups: &mut [GroupSlicing],
    lines: &Value,
    text: &BTreeMap<String, Vec<String>>,
) {
    for node in groups.iter_mut().flat_map(|group| group.nodes.iter_mut()) {
        if node.change == "removed" {
            continue;
        }
        let Some(file_text) = text.get(&node.path) else {
            continue;
        };
        let impacts = &lines["files"][&node.path];
        let mut flow: Vec<(u32, f32, Vec<String>)> = node
            .units
            .iter()
            .flat_map(|id| {
                lines["units"][id.to_string()]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|line| {
                let line = line.as_u64()? as u32;
                let impact = &impacts[line.to_string()];
                let why = impact["why"]
                    .as_array()
                    .map(|why| {
                        why.iter()
                            .filter_map(|w| w.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Some((line, impact["d"].as_f64().unwrap_or(0.0) as f32, why))
            })
            .filter(|(line, _, _)| !node.lines.contains(line))
            .collect();
        flow.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        flow.dedup_by_key(|(line, _, _)| *line);
        flow.truncate(CARD_FLOW_LINES);
        let text_at = |line: u32| -> Option<String> {
            file_text.get((line as usize).checked_sub(1)?).cloned()
        };
        let mut card: Vec<CardLine> = node
            .lines
            .iter()
            .take(CARD_ADDED_LINES)
            .filter_map(|line| {
                Some(CardLine {
                    line: *line,
                    text: text_at(*line)?,
                    added: true,
                    ..Default::default()
                })
            })
            .chain(flow.into_iter().filter_map(|(line, d, why)| {
                Some(CardLine {
                    line,
                    text: text_at(line)?,
                    added: false,
                    d: Some(d),
                    why,
                })
            }))
            .collect();
        card.sort_by_key(|line| line.line);
        node.card = card;
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SliceEdge {
    /// Its nodes (their indices), the earlier one in the order to read them
    /// first (see `read_rank`).
    pub from: usize,
    pub to: usize,
    /// What they share (see `link_reason`), the first few.
    pub why: Vec<String>,
}

/// A path's place in the order to read a patch's changes (see `Section`):
/// interfaces, headers, the rest, tests.
fn read_rank(path: &str) -> u8 {
    if is_idl(path) {
        0
    } else if is_header(path) {
        1
    } else if looks_like_test(path) {
        3
    } else {
        2
    }
}

/// A symbol's readable name, by the slicing's units' functions (ex: a
/// function's), or else by its kind's form (an IPDL message's,
/// `IPC_mozilla_dom_PFoo_Bar` as `PFoo::Bar`).
fn symbol_name(slicing: &crate::dataflow::slice::Slicing, sym: &str) -> String {
    if let Some(func) = slicing
        .units
        .iter()
        .find(|unit| unit.func_sym.is_some_and(|s| s.as_str() == sym))
        .and_then(|unit| unit.func.as_deref())
    {
        return short_name(func);
    }
    match sym.strip_prefix("IPC_") {
        Some(message) => {
            let parts: Vec<&str> = message.split('_').collect();
            parts[parts.len().saturating_sub(2)..].join("::")
        }
        None => sym.to_string(),
    }
}

/// A pretty name's last two parts (ex: `Class::Method`).
fn short_name(pretty: &str) -> String {
    let parts: Vec<&str> = pretty.split("::").collect();
    parts[parts.len().saturating_sub(2)..].join("::")
}

/// What a slicing's link's units share, readably (its `why`; see
/// `dataflow::slice::Link`).
fn link_reason(slicing: &crate::dataflow::slice::Slicing, why: &str) -> String {
    match why.split_once(' ') {
        Some(("function", sym)) => format!("calls or changes {}", symbol_name(slicing, sym)),
        Some(("mention", sym)) => format!("mentions {}", symbol_name(slicing, sym)),
        Some(("binding", sym)) => format!("the IPC message {}", symbol_name(slicing, sym)),
        Some(("local", _)) => "a local value".to_string(),
        Some(("field", name)) => format!("the field {}", name),
        _ => why.to_string(),
    }
}

/// A slicing's unit's class's pretty name, if it's in one.
fn unit_class(unit: &crate::dataflow::slice::Unit) -> Option<&str> {
    Some(unit.class.as_deref()?.trim_start_matches("T_"))
}

/// A slicing's unit's node's label (see `SliceNode`): its function's name,
/// else its class's (its declarations'), else its file's path.
fn unit_label(unit: &crate::dataflow::slice::Unit) -> String {
    unit.func
        .clone()
        .or_else(|| unit_class(unit).map(str::to_string))
        .unwrap_or_else(|| unit.path.clone())
}

/// The view's groups, or, a view's from before groups, each patch's own.
fn view_groups(view: &ReviewView) -> Vec<PatchGroup> {
    match view.groups.is_empty() {
        false => view.groups.clone(),
        true => (0..view.patches.len())
            .map(|i| PatchGroup {
                patches: vec![i],
                honorary: vec![],
                shared: vec![],
            })
            .collect(),
    }
}

/// The push's slicing by the view's groups (see `GroupSlicing`): each unit is
/// its commits' patches' (`rebased` maps the commits of the push's stack
/// rebased onto the indexed revision, which its slicing's units name, to the
/// push's own; see `explore::ExploreCommit::rebased`), its last one's for
/// what it adds, its first's for what it removes, or, if its commits aren't
/// known, the first patch changing its file's; a group's diagram has its
/// units and their neighbors in other groups, in the order to read them.
pub fn group_slicings(
    view: &ReviewView,
    slicing: &crate::dataflow::slice::Slicing,
    rebased: &BTreeMap<String, String>,
) -> Vec<GroupSlicing> {
    let groups = view_groups(view);
    let mut group_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (g, group) in groups.iter().enumerate() {
        for &i in &group.patches {
            group_of.insert(i, g);
        }
    }
    let patch_of: BTreeMap<&str, usize> = view
        .patches
        .iter()
        .enumerate()
        .map(|(i, patch)| (patch.rev.as_str(), i))
        .collect();
    fn changes_file<'a>(patch: &'a ReviewPatch, path: &str) -> Option<&'a ReviewFile> {
        patch
            .files
            .iter()
            .find(|file| file.path == path || file.old_path.as_deref() == Some(path))
    }
    // (Each unit's patch, and part.)
    let placed: Vec<(Option<usize>, Option<String>)> = slicing
        .units
        .iter()
        .map(|unit| {
            let patches: Vec<usize> = unit
                .commits
                .iter()
                .filter_map(|commit| {
                    let commit = rebased.get(commit).unwrap_or(commit);
                    patch_of.get(commit.as_str()).copied()
                })
                .collect();
            let patch = match unit.removed {
                false => patches.last().copied(),
                true => patches.first().copied(),
            }
            .or_else(|| {
                view.patches
                    .iter()
                    .position(|patch| changes_file(patch, &unit.path).is_some())
            });
            // (Its function's definition, else its class's, else its file.)
            let part = patch.and_then(|i| {
                let file = changes_file(&view.patches[i], &unit.path)?;
                let def = unit
                    .func
                    .as_deref()
                    .and_then(|func| file.defs.iter().find(|def| def.pretty == func))
                    .or_else(|| {
                        let class = unit_class(unit)?;
                        file.defs
                            .iter()
                            .find(|def| def.level == "class" && def.pretty == class)
                    });
                Some(def.map_or(file.id.clone(), |def| def.id.clone()))
            });
            (patch, part)
        })
        .collect();
    let mut neighbors: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for link in &slicing.links {
        neighbors.entry(link.from).or_default().insert(link.to);
        neighbors.entry(link.to).or_default().insert(link.from);
    }
    let order = |id: &usize| {
        let unit = &slicing.units[*id];
        (read_rank(&unit.path), placed[*id].0, unit.path.clone(), *id)
    };
    groups
        .iter()
        .enumerate()
        .map(|(g, _)| {
            let group = |id: &usize| placed[*id].0.and_then(|i| group_of.get(&i).copied());
            let mut members: Vec<usize> = (0..slicing.units.len())
                .filter(|id| group(id) == Some(g))
                .collect();
            members.sort_by_key(order);
            let left_out = members.len().saturating_sub(MAX_GROUP_UNITS);
            members.truncate(MAX_GROUP_UNITS);
            let mut others: Vec<usize> = members
                .iter()
                .flat_map(|id| neighbors.get(id).into_iter().flatten())
                .filter(|id| group(id) != Some(g))
                .copied()
                .collect::<BTreeSet<usize>>()
                .into_iter()
                .collect();
            others.sort_by_key(order);
            // (A patch's units of a function, what it removes and what it
            // adds, are one node, as are those of a class's declarations, or
            // a file's other lines; those of unknown patches are their own.)
            let mut keys: Vec<(Option<usize>, String)> = vec![];
            let mut node_units: Vec<Vec<usize>> = vec![];
            let mut index: BTreeMap<usize, usize> = BTreeMap::new();
            for id in members.iter().chain(&others) {
                let unit = &slicing.units[*id];
                let key = match (placed[*id].0, &placed[*id].1) {
                    (Some(patch), Some(part)) => {
                        (Some(patch), format!("{}\n{}", part, unit_label(unit)))
                    }
                    _ => (None, id.to_string()),
                };
                let n = match keys.iter().position(|k| *k == key) {
                    Some(n) => n,
                    None => {
                        keys.push(key);
                        node_units.push(vec![]);
                        keys.len() - 1
                    }
                };
                node_units[n].push(*id);
                index.insert(*id, n);
            }
            let own = members
                .iter()
                .map(|id| index[id])
                .collect::<BTreeSet<usize>>()
                .len();
            let nodes = node_units
                .iter()
                .map(|ids| {
                    let unit = &slicing.units[ids[0]];
                    let (removed, added): (Vec<&crate::dataflow::slice::Unit>, Vec<_>) = ids
                        .iter()
                        .map(|id| &slicing.units[*id])
                        .partition(|unit| unit.removed);
                    let lines_of = |units: &[&crate::dataflow::slice::Unit]| -> Vec<u32> {
                        units
                            .iter()
                            .flat_map(|unit| unit.lines.iter().copied())
                            .collect::<BTreeSet<u32>>()
                            .into_iter()
                            .collect()
                    };
                    let text_of = |units: &[&crate::dataflow::slice::Unit]| -> Vec<String> {
                        units.iter().flat_map(|unit| unit.text.clone()).collect()
                    };
                    SliceNode {
                        label: unit_label(unit),
                        scope: unit_class(unit).map(str::to_string),
                        path: added.first().unwrap_or(&unit).path.clone(),
                        change: match (removed.is_empty(), added.is_empty()) {
                            (true, _) => "added",
                            (false, true) => "removed",
                            (false, false) => "changed",
                        }
                        .to_string(),
                        lines: match added.is_empty() {
                            true => lines_of(&removed),
                            false => lines_of(&added),
                        },
                        patch: placed[ids[0]].0,
                        part: placed[ids[0]].1.clone(),
                        other: group(&ids[0]).filter(|other| *other != g),
                        text: text_of(&added),
                        old_text: text_of(&removed),
                        card: vec![],
                        units: ids.clone(),
                    }
                })
                .collect();
            // (A pair's links are one edge, but between two neighbors.)
            let mut edges: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
            for link in &slicing.links {
                let (Some(&a), Some(&b)) = (index.get(&link.from), index.get(&link.to)) else {
                    continue;
                };
                if a == b || (a >= own && b >= own) {
                    continue;
                }
                let why = edges.entry((a.min(b), a.max(b))).or_default();
                let reason = link_reason(slicing, &link.why);
                if why.len() < 4 && !why.contains(&reason) {
                    why.push(reason);
                }
            }
            // (Its units' classes, most units first.)
            let mut class_counts: BTreeMap<String, usize> = BTreeMap::new();
            for id in &members {
                if let Some(class) = &slicing.units[*id].class {
                    *class_counts.entry(class.to_string()).or_default() += 1;
                }
            }
            let mut classes: Vec<(String, usize)> = class_counts.into_iter().collect();
            classes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            GroupSlicing {
                nodes,
                edges: edges
                    .into_iter()
                    .map(|((from, to), why)| SliceEdge { from, to, why })
                    .collect(),
                classes: classes
                    .into_iter()
                    .take(MAX_BLOCK_CLASSES)
                    .map(|(class, _)| class)
                    .collect(),
                block: None,
                left_out,
            }
        })
        .collect()
}

// ## Rendering

/// The view's parts, for review-state.js: `[id, parent id, level, label]`.
fn parts(view: &ReviewView) -> Vec<Value> {
    let mut out = vec![];
    for (i, patch) in view.patches.iter().enumerate() {
        out.push(json!([
            patch.id,
            Value::Null,
            "patch",
            format!("Patch {}: {}", i + 1, patch.summary)
        ]));
        for file in &patch.files {
            out.push(json!([file.id, patch.id, "file", file.path]));
            for def in &file.defs {
                out.push(json!([
                    def.id,
                    def.parent.clone().unwrap_or_else(|| file.id.clone()),
                    def.level,
                    def.pretty
                ]));
            }
            for hunk in &file.hunks {
                let first = hunk
                    .lines
                    .iter()
                    .find_map(|line| line.new.or(line.old))
                    .unwrap_or(0);
                out.push(json!([
                    hunk.id,
                    hunk.def.clone().unwrap_or_else(|| file.id.clone()),
                    "hunk",
                    format!("{} at line {}", file.path, first)
                ]));
            }
        }
    }
    out
}

/// A part's slot for its strip of marks (see review-state.js's `strip`), on
/// source cards (files, definitions, hunks), after their headings.
fn strip_slot(part: &str) -> String {
    format!(
        r#"<span class="tr-strip" data-part="{}"></span>"#,
        escape(part)
    )
}

/// A part's slot for a button of its mark, which opens its strip (see
/// review-state.js's `stateButton`), where its state matters more than
/// changing it (ex: a patch's heading).
fn state_slot(part: &str) -> String {
    format!(
        r#"<span class="tr-state" data-part="{}"></span>"#,
        escape(part)
    )
}

/// The id of a part's element in a section (a file's and a definition's
/// elements are in each section with their hunks), for the URL's `#` (see
/// try-review.js's `remember`): a hash of them, which stays the same when the
/// push is imported again.
fn element_id(part: &str, section: Section) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in part.bytes().chain([b'\n']).chain(section.slug().bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("tr-{:012x}", hash & 0xffff_ffff_ffff)
}

/// The classes of the marks of a change's words which the other side doesn't
/// have (see `mark_word_changes`): its removed lines', and its added lines'.
pub const WORD_REMOVED_CLASS: &str = "tr-word-removed";
pub const WORD_ADDED_CLASS: &str = "tr-word-added";

/// The least share of a change's words (but whitespace) which both of its
/// sides have, of those of the side with fewer, for its words to be marked
/// (see `mark_word_changes`): a change whose sides share less rewrote its
/// lines, rather than editing them, and all of their words would be marked.
const WORD_DIFF_MIN_SHARED: f64 = 1.0 / 3.0;

/// Mark the words of each change of a hunk's `lines` (a run of removed lines
/// and the run of added lines after it) which the other side doesn't have, in
/// their HTML (see `highlight::mark_html`), as interdiffs and commit messages'
/// word diffs do, so that what a change keeps of the lines it replaces shows:
/// the removed lines' as `WORD_REMOVED_CLASS`, and the added lines' as
/// `WORD_ADDED_CLASS`.  But not those of a change whose sides share less than
/// `WORD_DIFF_MIN_SHARED` of their words, nor of one with a long line (whose
/// row has a note instead of its code; see `chunked_gzip::LONG_LINE_CHARS`).
pub fn mark_word_changes(lines: &mut [HunkLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != "-" {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].kind == "-" {
            i += 1;
        }
        let removed = i - start;
        while i < lines.len() && lines[i].kind == "+" {
            i += 1;
        }
        if i - start > removed {
            mark_change(&mut lines[start..i], removed);
        }
    }
}

/// `mark_word_changes`'s marks of a change: `lines`, whose first `removed`
/// lines are its removed ones, and the others its added ones.
fn mark_change(lines: &mut [HunkLine], removed: usize) {
    use crate::cmd_pipeline::highlight::{html_text, mark_html};
    use crate::file_format::chunked_gzip::LONG_LINE_NOTE;

    // Each side's lines' HTML (or their text, escaped) and text, and their
    // words, each with its line and its byte range in it (each line's end
    // being a word of its own, so that the words of different lines don't
    // match).
    type Side = (Vec<(String, String)>, Vec<(usize, std::ops::Range<usize>)>);
    let side = |lines: &[HunkLine], then: bool| -> Option<Side> {
        let mut texts = vec![];
        for line in lines {
            let html = match then {
                true => line.then_html.clone(),
                false => line.now_html.clone(),
            }
            .unwrap_or_else(|| escape(&line.text).to_string());
            if html.contains(LONG_LINE_NOTE) {
                return None;
            }
            let mut text = html_text(&html);
            if text.ends_with('\n') {
                text.pop();
            }
            texts.push((html, text));
        }
        let mut words = vec![];
        for (index, (_, text)) in texts.iter().enumerate() {
            for word in crate::format::message_words(text) {
                let start = word.as_ptr() as usize - text.as_ptr() as usize;
                words.push((index, start..start + word.len()));
            }
            words.push((index, text.len()..text.len()));
        }
        Some((texts, words))
    };
    let (Some((old_texts, old_words)), Some((new_texts, new_words))) = (
        side(&lines[..removed], true),
        side(&lines[removed..], false),
    ) else {
        return;
    };
    fn word_of<'a>(
        texts: &'a [(String, String)],
        (index, range): &(usize, std::ops::Range<usize>),
    ) -> &'a str {
        match range.is_empty() {
            true => "\n",
            false => &texts[*index].1[range.clone()],
        }
    }
    let old: Vec<&str> = old_words
        .iter()
        .map(|word| word_of(&old_texts, word))
        .collect();
    let new: Vec<&str> = new_words
        .iter()
        .map(|word| word_of(&new_texts, word))
        .collect();
    let ops = similar::capture_diff_slices(similar::Algorithm::Patience, &old, &new);

    // (Whether they share enough to be marked.)
    let count_words = |words: &[&str]| words.iter().filter(|word| !word.trim().is_empty()).count();
    let shared: usize = ops
        .iter()
        .filter(|op| op.tag() == similar::DiffTag::Equal)
        .map(|op| count_words(&old[op.old_range()]))
        .sum();
    let fewer = count_words(&old).min(count_words(&new));
    if fewer == 0 || (shared as f64) < WORD_DIFF_MIN_SHARED * fewer as f64 {
        return;
    }

    // Each line's marks: the ranges of its side's words in the ops the other
    // side doesn't have, merged.
    let marks = |words: &[(usize, std::ops::Range<usize>)],
                 ranges: Vec<std::ops::Range<usize>>,
                 count: usize| {
        let mut marks: Vec<Vec<std::ops::Range<usize>>> = vec![vec![]; count];
        for range in ranges {
            for (index, word) in &words[range] {
                if word.is_empty() {
                    continue;
                }
                let line = &mut marks[*index];
                match line.last_mut() {
                    Some(last) if last.end == word.start => last.end = word.end,
                    _ => line.push(word.clone()),
                }
            }
        }
        marks
    };
    let changed =
        |side: fn(&similar::DiffOp) -> std::ops::Range<usize>| -> Vec<std::ops::Range<usize>> {
            ops.iter()
                .filter(|op| op.tag() != similar::DiffTag::Equal)
                .map(side)
                .collect()
        };
    let old_marks = marks(
        &old_words,
        changed(similar::DiffOp::old_range),
        old_texts.len(),
    );
    let new_marks = marks(
        &new_words,
        changed(similar::DiffOp::new_range),
        new_texts.len(),
    );
    for (index, line) in lines.iter_mut().enumerate() {
        let (texts, marks, class, html) = match index < removed {
            true => (
                &old_texts,
                &old_marks[index],
                WORD_REMOVED_CLASS,
                &mut line.then_html,
            ),
            false => (
                &new_texts,
                &new_marks[index - removed],
                WORD_ADDED_CLASS,
                &mut line.now_html,
            ),
        };
        if marks.is_empty() {
            continue;
        }
        let index = if index < removed {
            index
        } else {
            index - removed
        };
        let marks: Vec<(std::ops::Range<usize>, &str)> =
            marks.iter().map(|range| (range.clone(), class)).collect();
        *html = Some(mark_html(&texts[index].0, &marks));
    }
}

/// A side of a hunk: its lines before (`then`, kept and removed) or after
/// (kept and added), as a source listing's rows (so that their blame strips'
/// popups and their symbols' menus work as a listing's do; see blame.js,
/// hyperblame.js and context-menu.js).
/// A side's rows of a hunk's `lines` (`then`, as they were, else as the patch
/// has them), each with its key (`data-line`, `line_key`'s: `key`, the
/// patch's side's file, and its line there), by which the page's other rows
/// of the same line (ex: a moved line's, in its hunk and where it moved from)
/// are shown with it when it's hovered over (see try-review.js's
/// `addLineHover`).
fn side_html(lines: &[HunkLine], then: bool, key: &str) -> String {
    let mut html = String::new();
    // (Where the other side has lines this one doesn't, a marker saying how
    // many, which shows when only one side does.)
    let mut other = 0;
    let marker = |n: usize| {
        format!(
            r#"<div role="row" class="source-line-with-number tr-line tr-other-side"><div role="cell" class="tr-blame"></div><span class="tr-lno"></span><code role="cell" class="tr-text">{} {} line{} {}</code></div>"#,
            if then { "+" } else { "\u{2212}" },
            n,
            if n == 1 { "" } else { "s" },
            if then { "added" } else { "removed" }
        )
    };
    // (A strip's color alternates when its commit changes, as a listing's
    // does.)
    let mut last_commit: Option<String> = None;
    let mut color = false;
    for line in lines {
        if (then && line.kind == "+") || (!then && line.kind == "-") {
            other += 1;
            continue;
        }
        if other > 0 {
            html.push_str(&marker(other));
            other = 0;
        }
        let (number, class) = match (then, line.kind.as_str()) {
            (true, "-") => (line.old, " tr-removed"),
            (false, "+") => (line.new, " tr-added"),
            (true, _) => (line.old, ""),
            (false, _) => (line.new, ""),
        };
        let code = match then {
            true => line.then_html.as_deref(),
            false => line.now_html.as_deref(),
        };
        let blame = match then {
            true => line.then_blame.as_ref(),
            false => line.now_blame.as_ref(),
        };
        let strip = match blame {
            Some(cell) => {
                let color_class = match &cell.commit {
                    Some(commit) => {
                        if last_commit.as_ref() != Some(commit) {
                            color = !color;
                        }
                        last_commit = Some(commit.clone());
                        if color { " c1" } else { " c2" }
                    }
                    None => "",
                };
                format!(
                    r#"<div class="blame-strip{}{}" data-hb-ctx="{}" data-hb-line="{}"{} role="button" aria-label="blame" aria-expanded="false"></div>"#,
                    color_class, cell.classes, cell.ctx, cell.line, cell.data
                )
            }
            None => String::new(),
        };
        // (A line a later patch changes says so; see try-review.js's
        // `addLaterChanges`.)
        let later = match (then, line.later) {
            (false, Some(index)) => format!(r#" tr-later" data-later="{}"#, index),
            _ => String::new(),
        };
        let data_line = match number {
            Some(n) => format!(r#" data-line="{}""#, escape(&line_key(key, n))),
            None => String::new(),
        };
        html.push_str(&format!(
            r#"<div role="row" class="source-line-with-number tr-line{}{}"{}><div role="cell" class="tr-blame">{}</div><span class="tr-lno">{}</span><code role="cell" class="source-line tr-text">{}</code></div>"#,
            class,
            later,
            data_line,
            strip,
            number.map_or(String::new(), |n| n.to_string()),
            code.map_or_else(|| escape(&line.text).to_string(), str::to_string)
        ));
    }
    if other > 0 {
        html.push_str(&marker(other));
    }
    html
}

/// The key of the line `n` of a patch's side's file (`side_key`'s), for
/// `side_html`'s rows.
fn line_key(side: &str, n: u32) -> String {
    format!("{}:{}", side, n)
}

/// The key of patch `patch`'s file `path`, as it was (`then`) or as the patch
/// has it, for its lines' keys (see `side_html`).
fn side_key(patch: usize, then: bool, path: &str) -> String {
    format!("{}:{}:{}", patch, if then { "o" } else { "n" }, path)
}

/// A hunk's heading's lines, linked to the commit's file (with the push's
/// stack; see stack-strip.js), and its added and removed lines' counts.
fn hunk_lines_html(
    tree: &str,
    rev: &str,
    stack: &str,
    file: &ReviewFile,
    hunk: &ReviewHunk,
) -> String {
    let first = hunk.lines.iter().find_map(|line| line.new).unwrap_or(0);
    let last = hunk
        .lines
        .iter()
        .filter_map(|line| line.new)
        .max()
        .unwrap_or(first);
    let link = format!(
        "/{}/rev/{}/{}?stack={}#{}",
        escape(tree),
        escape(rev),
        crate::url_encode_path::url_encode_path(&file.path),
        escape(stack),
        first
    );
    let delta = [
        (hunk.added > 0).then(|| format!("+{}", hunk.added)),
        (hunk.removed > 0).then(|| format!("\u{2212}{}", hunk.removed)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let lines = match first == last {
        true => format!("line {}", first),
        false => format!("lines {}&ndash;{}", first, last),
    };
    format!(
        r#"<a class="tr-hunk-lines" href="{}">{}</a> <span class="tr-delta">{}</span>"#,
        link,
        lines,
        escape(&delta)
    )
}

/// A hunk's code, then and now (a new file has none then, and a removed one
/// none now).
/// (Each side's code's symbols are in its SYM_INFO, the page's `SYM_INFOS`
/// "now" or "patched"; see `add_line_html`.)
/// (And the code before a patch is searched in the indexed revision, rather
/// than the push, whose context the page is in; see context-menu.js's
/// `searchContext`.)
fn hunk_code_html(tree: &str, patch: usize, file: &ReviewFile, hunk: &ReviewHunk) -> String {
    let then_key = side_key(patch, true, file.old_path.as_deref().unwrap_or(&file.path));
    let now_key = side_key(patch, false, &file.path);
    let then_attrs = format!(
        r#"class="file then-code" role="table" data-sym-info="now" data-tree="{}" data-search="{}""#,
        escape(tree),
        escape(&crate::file_format::config::search_url(tree))
    );
    const NOW: &str = r#"class="file now-code" role="table" data-sym-info="patched""#;
    let then = then_attrs.as_str();
    match file.status.as_str() {
        "added" => format!(
            r#"<div {}>{}</div>"#,
            NOW.replace("now-code", "now-code now-only"),
            side_html(&hunk.lines, false, &now_key)
        ),
        "removed" => format!(
            r#"<div {}>{}</div>"#,
            then.replace("then-code", "then-code then-only"),
            side_html(&hunk.lines, true, &then_key)
        ),
        _ => format!(
            r#"<div class="then-now"><div {}>{}</div><div {}>{}</div></div>"#,
            then,
            side_html(&hunk.lines, true, &then_key),
            NOW,
            side_html(&hunk.lines, false, &now_key)
        ),
    }
}

/// A hunk's card: its heading (its lines, then its strip), and its code; or,
/// `folded` onto its definition's heading (its only hunk there), its code.
fn hunk_html(
    tree: &str,
    rev: &str,
    stack: &str,
    patch: usize,
    file: &ReviewFile,
    hunk: &ReviewHunk,
    folded: bool,
) -> String {
    let head = match folded {
        true => String::new(),
        false => format!(
            r#"<div class="tr-hunk-head">{} {}</div>"#,
            hunk_lines_html(tree, rev, stack, file, hunk),
            strip_slot(&hunk.id)
        ),
    };
    format!(
        r#"<div class="tr-hunk{folded}" data-part="{id}" id="{element}">{head}{code}</div>"#,
        folded = if folded { " tr-hunk-folded" } else { "" },
        id = escape(&hunk.id),
        element = element_id(&hunk.id, hunk.section),
        head = head,
        code = hunk_code_html(tree, patch, file, hunk),
    )
}

/// A file's hunks of a section, under their definitions, in the file's
/// order.
fn file_html(
    tree: &str,
    view: &ReviewView,
    patch_index: usize,
    patch: &ReviewPatch,
    file: &ReviewFile,
    section: Section,
) -> String {
    let stack = crate::try_import::push_url_path(&view.push);
    let hunks: Vec<&ReviewHunk> = file.hunks.iter().filter(|h| h.section == section).collect();
    // (The definitions of the section's hunks, and their ancestors.)
    let by_id: BTreeMap<&str, &ReviewDef> = file.defs.iter().map(|d| (d.id.as_str(), d)).collect();
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    for hunk in &hunks {
        let mut id = hunk.def.as_deref();
        while let Some(def) = id.and_then(|id| by_id.get(id)) {
            wanted.insert(def.id.as_str());
            id = def.parent.as_deref();
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn children_html(
        tree: &str,
        rev: &str,
        stack: &str,
        patch_index: usize,
        file: &ReviewFile,
        parent: Option<&str>,
        hunks: &[&ReviewHunk],
        wanted: &BTreeSet<&str>,
    ) -> String {
        let mut html = String::new();
        // (The hunks directly in it, then its definitions, each in the file's
        // order; hunks and definitions interleave by their first lines.)
        let mut items: Vec<(u32, String)> = vec![];
        for hunk in hunks.iter().filter(|h| h.def.as_deref() == parent) {
            let first = hunk.lines.iter().find_map(|l| l.new.or(l.old)).unwrap_or(0);
            items.push((
                first,
                hunk_html(tree, rev, stack, patch_index, file, hunk, false),
            ));
        }
        for (order, def) in file
            .defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.parent.as_deref() == parent && wanted.contains(d.id.as_str()))
        {
            let first = hunks
                .iter()
                .filter(|h| {
                    let mut id = h.def.as_deref();
                    while let Some(current) = id {
                        if current == def.id {
                            return true;
                        }
                        id = file
                            .defs
                            .iter()
                            .find(|d| d.id == current)
                            .and_then(|d| d.parent.as_deref());
                    }
                    false
                })
                .filter_map(|h| h.lines.iter().find_map(|l| l.new.or(l.old)))
                .min()
                .unwrap_or(order as u32);
            let status = match def.status.as_str() {
                "added" => r#" <span class="tr-def-status tr-def-added">added</span>"#,
                "removed" => r#" <span class="tr-def-status tr-def-removed">removed</span>"#,
                "renamed" => r#" <span class="tr-def-status">renamed</span>"#,
                "moved" => r#" <span class="tr-def-status">moved</span>"#,
                _ => "",
            };
            // (Its only hunk, if it has no definitions of its own here, is
            // folded onto its heading: its lines there, then the code.)
            let own: Vec<&&ReviewHunk> = hunks
                .iter()
                .filter(|h| h.def.as_deref() == Some(def.id.as_str()))
                .collect();
            let nested = file.defs.iter().any(|d| {
                d.parent.as_deref() == Some(def.id.as_str()) && wanted.contains(d.id.as_str())
            });
            let (lines, children) = match (own.as_slice(), nested) {
                ([only], false) => (
                    format!(" {}", hunk_lines_html(tree, rev, stack, file, only)),
                    hunk_html(tree, rev, stack, patch_index, file, only, true),
                ),
                _ => (
                    String::new(),
                    children_html(
                        tree,
                        rev,
                        stack,
                        patch_index,
                        file,
                        Some(&def.id),
                        hunks,
                        wanted,
                    ),
                ),
            };
            let section = hunks.first().map_or(Section::Uses, |h| h.section);
            items.push((
                first,
                format!(
                    r#"<div class="tr-def tr-def-{level}" data-part="{id}" id="{element}"><div class="tr-def-head"><span class="tr-def-kind">{kind}</span> <code class="tr-def-name" title="{pretty}">{name}</code>{decl}{status}{lines} {strip}</div>{children}</div>"#,
                    level = escape(&def.level),
                    id = escape(&def.id),
                    element = element_id(&def.id, section),
                    strip = strip_slot(&def.id),
                    kind = escape(&def.kind),
                    pretty = escape(&def.pretty),
                    name = escape(&def.name),
                    decl = if def.decl {
                        r#" <span class="tr-def-decl">declaration</span>"#
                    } else {
                        ""
                    },
                    status = status,
                    lines = lines,
                    children = children,
                ),
            ));
        }
        items.sort_by_key(|(first, _)| *first);
        for (_, item) in items {
            html.push_str(&item);
        }
        html
    }
    let facets: BTreeMap<&str, Vec<String>> = file_facet_values(patch_index, file, section)
        .into_iter()
        .map(|(key, path)| {
            (
                key,
                path.into_iter().map(|(value, _)| value).collect::<Vec<_>>(),
            )
        })
        .collect();
    let status = match file.status.as_str() {
        "added" => r#" <span class="tr-file-status tr-file-added">new file</span>"#.to_string(),
        "removed" => r#" <span class="tr-file-status tr-file-removed">removed</span>"#.to_string(),
        "renamed" => format!(
            r#" <span class="tr-file-status">renamed from {}</span>"#,
            escape(file.old_path.as_deref().unwrap_or(""))
        ),
        "binary" => r#" <span class="tr-file-status">binary</span>"#.to_string(),
        _ => String::new(),
    };
    format!(
        r#"<section class="tr-file" data-part="{id}" id="{element}" data-path="{path}" data-facets="{facets}"><h5 class="tr-file-head"><a href="/{tree}/rev/{rev}/{link}?stack={stack}">{path}</a>{status} {strip}</h5>{children}{truncated}</section>"#,
        id = escape(&file.id),
        element = element_id(&file.id, section),
        path = escape(&file.path),
        facets = escape(&serde_json::to_string(&facets).unwrap_or_default()),
        strip = strip_slot(&file.id),
        tree = escape(tree),
        // (The file in the push's stack rebased onto the indexed revision, if
        // it has its semantic analysis, but a removed one; else in the
        // commit.)
        rev = escape(match (&view.semantic_head, file.status.as_str()) {
            (Some(head), status) if status != "removed" => head,
            _ => &patch.rev,
        }),
        link = crate::url_encode_path::url_encode_path(&file.path),
        stack = escape(&stack),
        status = status,
        // (Its hunks' lines link to the commit whose code they are.)
        children = children_html(
            tree,
            patch.shown.as_deref().unwrap_or(&patch.rev),
            &stack,
            patch_index,
            file,
            None,
            &hunks,
            &wanted
        ),
        truncated = if file.truncated {
            r#"<p class="tr-note">Its other changes are left out here (it has too many lines of them).</p>"#
        } else {
            ""
        },
    )
}

/// A patch's section: its heading, then its sections' files.
fn patch_html(tree: &str, view: &ReviewView, i: usize) -> String {
    let short = |rev: &str| rev.chars().take(12).collect::<String>();
    let patch = &view.patches[i];
    let mut html = String::new();
    let bugs = patch
        .bugs
        .iter()
        .map(|bug| {
            format!(r#"<a href="https://bugzilla.mozilla.org/show_bug.cgi?id={bug}">bug {bug}</a>"#)
        })
        .collect::<Vec<_>>();
    let refs = bugs
        .into_iter()
        .chain(patch.phab.map(|d| {
            format!(r#"<a href="https://phabricator.services.mozilla.com/D{d}">D{d}</a>"#)
        }))
        .collect::<Vec<_>>()
        .join(", ");
    html.push_str(&format!(
        r#"<section class="tr-patch{backout}" id="tr-patch-{n}" data-part="{id}"><h3 class="tr-patch-head">{strip} <span class="tr-patch-n">Patch {n} of {count}</span> <a class="tr-patch-rev" href="/{tree}/commit/{rev}">{short}</a> <span class="tr-patch-summary">{summary}</span><span class="tr-patch-meta">{author}{refs}</span></h3>"#,
        backout = if patch.backout { " tr-backout" } else { "" },
        n = i + 1,
        count = view.patches.len(),
        id = escape(&patch.id),
        strip = state_slot(&patch.id),
        tree = escape(tree),
        rev = escape(&patch.rev),
        short = escape(&short(&patch.rev)),
        summary = escape(&patch.summary),
        author = escape(&patch.author),
        refs = if refs.is_empty() { String::new() } else { format!("; {}", refs) },
    ));
    let present: Vec<Section> = Section::ALL
        .into_iter()
        .filter(|section| {
            patch
                .files
                .iter()
                .any(|f| f.hunks.iter().any(|h| h.section == *section))
        })
        .collect();
    if present.is_empty() {
        html.push_str(r#"<p class="tr-note">It changes no text.</p>"#);
    }
    for section in &present {
        html.push_str(&format!(
            r#"<div class="tr-section tr-section-{slug}" data-section="{slug}"><h4 class="tr-section-head"><span class="tr-section-label">{label}</span> <span class="tr-section-about">{about}</span><span class="tr-section-jumps">{jumps}<button type="button" class="tr-jump-picker" data-patch="{patch}" title="Go to one of the patch's files, classes or functions" aria-expanded="false" aria-haspopup="true" aria-controls="tr-jump-menu">Jump to</button></span></h4>"#,
            slug = section.slug(),
            label = section.label(),
            about = section.about(),
            patch = i,
            jumps = present
                .iter()
                .filter(|other| *other != section)
                .map(|other| format!(
                    r##"<a class="tr-section-jump" href="#tr-patch-{}-{}">{}</a>"##,
                    i + 1,
                    other.slug(),
                    other.label()
                ))
                .collect::<Vec<_>>()
                .join(""),
        ));
        // (Its anchor, for the other sections' jumps.)
        html.push_str(&format!(
            r#"<span class="tr-anchor" id="tr-patch-{}-{}"></span>"#,
            i + 1,
            section.slug()
        ));
        let mut files: Vec<&ReviewFile> = patch
            .files
            .iter()
            .filter(|f| f.hunks.iter().any(|h| h.section == *section))
            .collect();
        files.sort_by(|a, b| {
            path_kind_rank(&a.path_kind)
                .cmp(&path_kind_rank(&b.path_kind))
                .then_with(|| is_idl(&b.path).cmp(&is_idl(&a.path)))
                .then_with(|| is_header(&b.path).cmp(&is_header(&a.path)))
                .then_with(|| a.path.cmp(&b.path))
        });
        for file in files {
            html.push_str(&file_html(tree, view, i, patch, file, *section));
        }
        html.push_str("</div>");
    }
    if patch.files_left_out > 0 {
        html.push_str(&format!(
            r#"<p class="tr-note">{} more of its files are left out.</p>"#,
            patch.files_left_out
        ));
    }
    html.push_str("</section>");
    html
}

/// The URL of the page of the push's slicing.
fn slicing_url(tree: &str, slicing: &ReviewSlicing) -> String {
    format!(
        "/{}/query/default?q={}&rev={}",
        tree,
        urlencoding::encode(&slicing.query),
        urlencoding::encode(&slicing.rev)
    )
}

/// A group's block diagram's figure (see `GroupSlicing::block`), if it has
/// one: the classes its patches change, how they call each other and their
/// strongest neighbors (by the roll-ups of the push's context), with a link
/// to the diagram's page (`block-classes:`), whose panel can change it.
fn block_html(tree: &str, view: &ReviewView, g: usize) -> String {
    let Some(slicing) = &view.slicing else {
        return String::new();
    };
    let Some((group, svg)) = slicing
        .groups
        .get(g)
        .and_then(|group| Some((group, group.block.as_ref()?)))
    else {
        return String::new();
    };
    let query = format!("block-classes:{}", group.classes.join(","));
    format!(
        r#"<figure class="tr-block"><figcaption><span class="tr-slicing-title">Its classes</span> <span class="tr-slicing-about">{about} (<a href="{url}">the block diagram's page</a>)</span></figcaption><div class="tr-block-diagram">{svg}</div></figure>"#,
        about = escape(&format!(
            "the {} its patches change, how they call each other, and their strongest neighbors",
            if group.classes.len() == 1 {
                "class"
            } else {
                "classes"
            }
        )),
        url = escape(&format!(
            "/{}/query/default?q={}&rev={}",
            tree,
            urlencoding::encode(&query),
            urlencoding::encode(&slicing.rev)
        )),
        svg = svg,
    )
}

/// A group's dataflow diagram's figure, which try-review.js draws (see
/// `GroupSlicing`), if its patches' changes have units.
fn slicing_html(tree: &str, view: &ReviewView, g: usize) -> String {
    let Some(slicing) = &view.slicing else {
        return String::new();
    };
    let Some(group) = slicing
        .groups
        .get(g)
        .filter(|group| !group.nodes.is_empty())
    else {
        return String::new();
    };
    let plural =
        |n: usize, one: &str, many: &str| format!("{} {}", n, if n == 1 { one } else { many });
    let own = group
        .nodes
        .iter()
        .filter(|node| node.other.is_none())
        .count();
    let others = group.nodes.len() - own;
    let about = [
        Some(format!(
            "{}, by what they share",
            plural(own, "change", "changes")
        )),
        (others > 0).then(|| {
            format!(
                "with {} of other groups' patches",
                plural(others, "change", "changes")
            )
        }),
        (group.left_out > 0).then(|| format!("{} left out", group.left_out)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("; ");
    format!(
        r#"<figure class="tr-slicing" data-group="{g}"><figcaption><span class="tr-slicing-title">Their dataflow</span> <span class="tr-slicing-about">{about} (<a href="{url}">the push's slicing</a>)</span> <button type="button" class="tr-slicing-code" aria-pressed="false">Show their code</button></figcaption><div class="tr-slicing-canvas crash-graph-canvas"></div></figure>"#,
        g = g,
        about = escape(&about),
        url = escape(&slicing_url(tree, slicing)),
    )
}

/// A group of patches' heading (see `group_patches`): its patches, what they
/// share, and its honorary members (links to them).
fn group_head_html(view: &ReviewView, g: usize) -> String {
    let group = &view.groups[g];
    let numbers = |patches: &[usize]| -> String {
        match (patches.first(), patches.last()) {
            (Some(first), Some(last)) if first == last => format!("patch {}", first + 1),
            (Some(first), Some(last)) => format!("patches {}\u{2013}{}", first + 1, last + 1),
            _ => String::new(),
        }
    };
    let shared = match group.shared.as_slice() {
        [] => String::new(),
        shared => format!(
            ", which share {}",
            shared
                .iter()
                .map(|s| format!("<code>{}</code>", escape(s)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let honorary = match group.honorary.as_slice() {
        [] => String::new(),
        honorary => format!(
            r#"<span class="tr-group-honorary">; related: {}</span>"#,
            honorary
                .iter()
                .map(|i| format!(
                    r##"<a href="#tr-patch-{n}" title="{summary}">patch {n}</a>"##,
                    n = i + 1,
                    summary = escape(&view.patches[*i].summary)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!(
        r#"<div class="tr-group-head"><span class="tr-group-n">Group {n} of {count}</span>: {patches}{shared}{honorary}</div>"#,
        n = g + 1,
        count = view.groups.len(),
        patches = numbers(&group.patches),
        shared = shared,
        honorary = honorary,
    )
}

/// What the push's page said of its semantic analysis (see
/// `ReviewView::semantic_head`), as the view's notes.
fn semantic_notes_html(tree: &str, view: &ReviewView) -> String {
    let short = |rev: &str| rev.chars().take(12).collect::<String>();
    let mut html = String::new();
    if let Some(head) = &view.semantic_head {
        html.push_str(&format!(
            r#"<p class="tr-note tr-semantic-head">Its stack rebased onto the indexed revision has semantic analysis: <a class="tr-browse" href="/{tree}/rev/{head}/">browse it</a>, with jumps and references, or see <a class="tr-canvas" href="/{tree}/rev-canvas/{head}">the functions it adds or changes and their calls on a canvas</a>.</p>"#,
            tree = escape(tree),
            head = escape(head),
        ));
        if let Some(trunk) = &view.semantic_trunk {
            html.push_str(&format!(
                r#"<p class="tr-note tr-semantic-trunk">Its stack is on a newer trunk revision than the indexed one (<a href="/{tree}/commit/{base}">{short}</a>), and needs its changes, so its semantic analysis is of it with the {revisions} since the indexed one.</p>"#,
                tree = escape(tree),
                base = escape(&trunk.stack_base),
                short = escape(&short(&trunk.stack_base)),
                revisions = match trunk.revisions {
                    1 => "revision".to_string(),
                    n => format!("{} revisions", n),
                },
            ));
        }
    }
    if !view.landed.is_empty() {
        html.push_str(&format!(
            r#"<div class="tr-note tr-landed">{} landed since it diverged from the trunk, so its semantic analysis leaves {} out:<ul>"#,
            match view.landed.len() {
                1 => "One of its commits has".to_string(),
                n => format!("{} of its commits have", n),
            },
            if view.landed.len() == 1 { "it" } else { "them" }
        ));
        for commit in &view.landed {
            html.push_str(&format!(
                r#"<li><a href="/{tree}/commit/{rev}">{summary}</a>, as <a class="tr-landed-as" href="/{tree}/commit/{landed}">{short}</a>{same}</li>"#,
                tree = escape(tree),
                rev = escape(&commit.rev),
                summary = escape(&commit.rev_summary),
                landed = escape(&commit.landed),
                short = escape(&short(&commit.landed)),
                same = if commit.same_changes {
                    String::new()
                } else {
                    format!(
                        " (the same Phabricator revision, which may have changed before it landed: {})",
                        escape(&commit.summary)
                    )
                },
            ));
        }
        html.push_str("</ul></div>");
    }
    if let Some(failure) = &view.semantic_failure {
        if failure.all_landed {
            html.push_str(r#"<p class="tr-note tr-semantic-failure">All of its commits (but those selecting try's tasks) have landed, so there's nothing for its semantic analysis to analyze: the indexed revision has its changes.</p>"#);
        } else {
            html.push_str(&format!(
                r#"<p class="tr-note tr-semantic-failure">Its semantic analysis, for jumps and references in what it changes, failed in its <code>{step}</code> step{error}{commit}{conflicts}</p>"#,
                step = escape(&failure.step),
                error = match &failure.error {
                    Some(error) => format!(": {}", escape(error)),
                    None => ".".to_string(),
                },
                commit = match &failure.conflicting_commit {
                    Some(commit) => format!(" The commit whose changes conflict: {}.", escape(commit)),
                    None => String::new(),
                },
                conflicts = match failure.conflicts.as_slice() {
                    [] => String::new(),
                    files => format!(
                        " The files whose changes conflict: {}.",
                        escape(&files.join(", "))
                    ),
                },
            ));
        }
    }
    html
}

/// The view's HTML, for the query page.
pub fn render_review(tree: &str, view: &ReviewView) -> String {
    let mut html = String::new();
    let short = |rev: &str| rev.chars().take(12).collect::<String>();
    let files: BTreeSet<&str> = view
        .patches
        .iter()
        .flat_map(|p| p.files.iter().map(|f| f.path.as_str()))
        .collect();
    html.push_str(&format!(
        r#"<div class="try-review then-now-view facet-items" id="try-review" data-then-now="both" data-then-now-narrow="now" data-then-now-columns="80" data-key="{key}">"#,
        key = escape(&view.key)
    ));
    html.push_str(&crate::html_escape::json_data_block(
        "TRY_REVIEW",
        &json!({
            "key": view.key,
            "indices": view.indices,
            "parts": parts(view),
            // (For the patch picker: each patch's part and summary, and the
            // groups, with what they share and their honorary members.)
            "patches": view
                .patches
                .iter()
                .map(|patch| json!({ "part": patch.id, "summary": patch.summary }))
                .collect::<Vec<_>>(),
            "groups": view.groups,
            "tree": tree,
            // (The groups' diagrams' data; see `group_slicings`.)
            "slicing": view.slicing,
            // (The later patches' changes of lines; see `add_later_changes`.)
            "later": view.later,
        }),
    ));
    // (Its code's symbols' SYM_INFO by side, and its blames; see
    // `add_line_html`.)
    if let Some(sym_infos) = &view.sym_infos {
        html.push_str(&crate::html_escape::json_data_block("SYM_INFOS", sym_infos));
    }
    if !view.blame_infos.is_empty() {
        html.push_str(&crate::html_escape::json_data_block(
            "BLAME_INFOS",
            &json!(view.blame_infos),
        ));
    }
    // (And its block diagrams' nodes', for their context menus.)
    if let Some(sym_info) = &view.block_sym_info {
        html.push_str(&crate::html_escape::json_merge_block_from_json(
            "SYM_INFO", sym_info,
        ));
    }
    // (The breadcrumbs' trail, which try-review.js puts in the page's.)
    html.push_str(&format!(
        r##"<span class="tr-crumbs" hidden><span class="path-separator">/</span><a href="/{tree}/try/hg/{push}">try push {short}</a><span class="path-separator">/</span><span class="tr-crumb-review">review</span><span class="path-separator">/</span><span class="tr-crumb-patch"><a class="tr-crumb-patch-link" href="#tr-patch-1">Patch 1</a><button type="button" class="crash-crumb-button" id="tr-patch-picker" title="Pick a patch" aria-expanded="false" aria-haspopup="true" aria-controls="tr-patch-menu"></button><div id="tr-patch-menu" title="The patches" role="menu" class="context-menu initially-hidden"></div></span><span class="tr-crumb-then-now" hidden></span></span><div id="tr-jump-menu" title="The patch's files, classes and functions" role="menu" class="context-menu initially-hidden"></div>"##,
        tree = escape(tree),
        push = escape(&view.push),
        short = escape(&short(&view.push)),
    ));
    html.push_str(&format!(
        r#"<div class="tr-head"><h2>Review of the try push <a href="/{tree}/try/hg/{push}">{short}</a></h2><p class="tr-summary">{patches} {patch_word}, changing {files} {file_word}. Each patch's changes are in the order to read them: {order}. Your marks and notes on its parts are kept in this browser, and shown in its other tabs of it.</p>{modes}</div>"#,
        tree = escape(tree),
        push = escape(&view.push),
        short = escape(&short(&view.push)),
        patches = view.patches.len(),
        patch_word = if view.patches.len() == 1 { "patch" } else { "patches" },
        files = files.len(),
        file_word = if files.len() == 1 { "file" } else { "files" },
        order = "interfaces, then definitions, then their uses, then tests",
        // (Its code is its stack rebased onto the indexed revision's, if it
        // has one; see `review_view`.)
        modes = match view.patches.iter().any(|patch| patch.shown.is_some()) {
            true => crate::crash_stack::then_now_modes_labeled_html(
                ["Now", "Patched", "Now &amp; Patched"],
                "The code before each patch, with the push's stack rebased onto the indexed revision: that revision's, before the stack's first patch changing a file.",
                "The code with each patch, with the push's stack rebased onto the indexed revision."
            ),
            false => crate::crash_stack::then_now_modes_labeled_html(
                ["Now", "Patched", "Now &amp; Patched"],
                "The code before each patch.",
                "The code with each patch."
            ),
        },
    ));
    html.push_str(&semantic_notes_html(tree, view));
    if view.task_selections > 0 || view.truncated {
        html.push_str(&format!(
            r#"<p class="tr-note">{}</p>"#,
            [
                (view.task_selections > 0).then(|| format!(
                    "Its {} commit{} selecting try's tasks {} left out.",
                    view.task_selections,
                    if view.task_selections == 1 { "" } else { "s" },
                    if view.task_selections == 1 {
                        "is"
                    } else {
                        "are"
                    }
                )),
                view.truncated
                    .then(|| "Its stack has more commits than the review shows.".to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        ));
    }
    // (The patches by their groups, each with a heading if there are
    // several; a view from before groups has each patch in a group of its
    // own.)
    let mut view = view.clone();
    view.groups = view_groups(&view);
    if let Some(note) = view.slicing.as_ref().and_then(|s| s.note.as_ref()) {
        html.push_str(&format!(
            r#"<p class="tr-note">Its changes' dataflow couldn't be sliced, for its diagrams ({}); <a href="{}">its slicing's page</a> says more.</p>"#,
            escape(note),
            escape(&slicing_url(tree, view.slicing.as_ref().unwrap())),
        ));
    }
    for (g, group) in view.groups.iter().enumerate() {
        html.push_str(&format!(
            r#"<section class="tr-group" id="tr-group-{}">"#,
            g + 1
        ));
        if view.groups.len() > 1 {
            html.push_str(&group_head_html(&view, g));
        }
        html.push_str(&block_html(tree, &view, g));
        html.push_str(&slicing_html(tree, &view, g));
        for i in &group.patches {
            html.push_str(&patch_html(tree, &view, *i));
        }
        html.push_str("</section>");
    }
    html.push_str("</div>");
    // (crash-view.js for its breadcrumbs' menus, `CrashMenu`.)
    for script in [
        "review-state.js",
        "then-now.js",
        "crash-view.js",
        "graph.js",
        "graph-records.js",
        "try-review.js",
    ] {
        html.push_str(&format!(
            r#"<script defer src="/{}/static/js/{}"></script>"#,
            escape(tree),
            script
        ));
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk_line(kind: &str, text: &str, html: Option<&str>) -> HunkLine {
        HunkLine {
            kind: kind.to_string(),
            text: text.to_string(),
            then_html: html.filter(|_| kind != "+").map(str::to_string),
            now_html: html.filter(|_| kind != "-").map(str::to_string),
            ..Default::default()
        }
    }

    /// A change's words which the other side doesn't have are marked in its
    /// lines' HTML (around their tags, whose attributes can have `>`), each
    /// line's words with its own line's; but not a rewrite's, nor a long
    /// line's.
    #[test]
    fn test_mark_word_changes() {
        let mut lines = vec![
            hunk_line(" ", "int f() {", None),
            hunk_line(
                "-",
                "  return foo(a, b);",
                Some(
                    r#"  <span class="syn_reserved">return</span> <span data-symbols="F_foo">foo</span>(a, b);"#,
                ),
            ),
            hunk_line(
                "+",
                "  return bar(a, b, c);",
                Some(
                    r#"  <span class="syn_reserved">return</span> <span data-symbols="F_<T_X>_bar">bar</span>(a, b, c);"#,
                ),
            ),
            hunk_line(" ", "}", None),
        ];
        mark_word_changes(&mut lines);
        assert_eq!(
            lines[1].then_html.as_deref(),
            Some(
                r#"  <span class="syn_reserved">return</span> <span data-symbols="F_foo"><mark class="tr-word-removed">foo</mark></span>(a, b);"#
            )
        );
        assert_eq!(
            lines[2].now_html.as_deref(),
            Some(
                r#"  <span class="syn_reserved">return</span> <span data-symbols="F_<T_X>_bar"><mark class="tr-word-added">bar</mark></span>(a, b<mark class="tr-word-added">, c</mark>);"#
            )
        );
        assert_eq!(lines[0].then_html, None, "(kept lines aren't marked)");

        // A change of two lines, and an entity.
        let mut two = vec![
            hunk_line("-", "a = f(1);", None),
            hunk_line("-", "if (a < b)", None),
            hunk_line("+", "a = f(1, x);", None),
            hunk_line("+", "if (a <= b)", None),
        ];
        mark_word_changes(&mut two);
        assert_eq!(two[0].then_html, None, "(nothing of it is removed)");
        assert_eq!(two[1].then_html, None);
        assert_eq!(
            two[2].now_html.as_deref(),
            Some(r#"a = f(1<mark class="tr-word-added">, x</mark>);"#)
        );
        assert_eq!(
            two[3].now_html.as_deref(),
            Some(r#"if (a &lt;<mark class="tr-word-added">=</mark> b)"#)
        );

        // A rewrite isn't marked.
        let mut rewrite = vec![
            hunk_line("-", "int x = 1;", None),
            hunk_line("+", "return compute(y);", None),
        ];
        mark_word_changes(&mut rewrite);
        assert_eq!(
            (rewrite[0].then_html.clone(), rewrite[1].now_html.clone()),
            (None, None)
        );

        // Nor a change with a long line's note.
        let note = format!(
            "{}A line of 2,001 characters, too long to show here</span>",
            crate::file_format::chunked_gzip::LONG_LINE_NOTE
        );
        let mut long = vec![
            hunk_line("-", "x = 1;", Some(&note)),
            hunk_line("+", "x = 2;", None),
        ];
        mark_word_changes(&mut long);
        assert_eq!(long[1].now_html, None);
    }

    const DIFF: &str = "diff --git a.cpp a.cpp
index 1111111..2222222 100644
--- a.cpp
+++ a.cpp
@@ -1,4 +1,5 @@ namespace a {
 int f() {
-  return 1;
+  int x = 2;
+  return x;
 }

diff --git \"b c.h\" \"b c.h\"
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ \"b c.h\"
@@ -0,0 +1,2 @@
+// b
+int g();
diff --git old.txt new.txt
similarity index 90%
rename from old.txt
rename to new.txt
diff --git img.png img.png
index 4444444..5555555 100644
Binary files img.png and img.png differ
";

    #[test]
    fn test_parse_diff() {
        let files = parse_diff(DIFF);
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].path, "a.cpp");
        assert_eq!(files[0].status, "changed");
        let hunk = &files[0].hunks[0];
        assert_eq!(hunk.lines.len(), 5);
        assert_eq!(hunk.lines[1].kind, "-");
        assert_eq!((hunk.lines[1].old, hunk.lines[1].new), (Some(2), None));
        assert_eq!((hunk.lines[3].old, hunk.lines[3].new), (None, Some(3)));
        assert_eq!((hunk.lines[4].old, hunk.lines[4].new), (Some(3), Some(4)));
        assert_eq!(hunk.new_range(), Some((2, 3)));
        assert_eq!(hunk.old_range(), Some((2, 2)));
        assert_eq!(files[1].path, "b c.h");
        assert_eq!(files[1].status, "added");
        assert_eq!(files[1].hunks[0].lines[0].new, Some(1));
        assert_eq!(files[2].path, "new.txt");
        assert_eq!(files[2].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[2].status, "renamed");
        assert_eq!(files[3].status, "binary");
    }

    #[test]
    fn test_message_refs() {
        let (bugs, phab) = message_refs(
            "Bug 2090040 - Use nsIPrincipal. r?#ipc-reviewers",
            "Bug 2090040 - Use nsIPrincipal.\n\nDifferential Revision: https://phabricator.services.mozilla.com/D271234\n",
        );
        assert_eq!(bugs, vec![2090040]);
        assert_eq!(phab, Some(271234));
    }

    fn patch(rev: &str, files: &[&str], bug: u64, lines: u32) -> ReviewPatch {
        ReviewPatch {
            id: format!("p:{}", rev),
            rev: rev.to_string(),
            shown: None,
            parent: None,
            summary: String::new(),
            author: String::new(),
            date: String::new(),
            backout: false,
            bugs: vec![bug],
            phab: None,
            files: files
                .iter()
                .map(|path| ReviewFile {
                    id: format!("f:{}:{}", rev, path),
                    path: path.to_string(),
                    old_path: None,
                    status: "changed".to_string(),
                    path_kind: "normal".to_string(),
                    subsystem: None,
                    defs: vec![],
                    hunks: vec![ReviewHunk {
                        id: String::new(),
                        def: None,
                        section: Section::Uses,
                        lines: vec![],
                        // (The patch's lines, across its files.)
                        added: lines / files.len() as u32,
                        removed: 0,
                    }],
                    truncated: false,
                })
                .collect(),
            files_left_out: 0,
        }
    }

    #[test]
    fn test_group_slicings() {
        // (Two patches in two groups: the first changes A::f in a.cpp, and
        // the second b.cpp.)
        let mut first = patch("aaaa", &["a.cpp"], 1, 10);
        first.files[0].defs.push(ReviewDef {
            id: "d:aaaa:a.cpp:A::f".to_string(),
            pretty: "A::f".to_string(),
            name: "f".to_string(),
            kind: "method".to_string(),
            level: "function".to_string(),
            parent: None,
            status: "changed".to_string(),
            decl: false,
        });
        let view = ReviewView {
            push: "bbbb".to_string(),
            key: String::new(),
            indices: vec![],
            patches: vec![first, patch("bbbb", &["b.cpp"], 2, 10)],
            groups: vec![],
            task_selections: 0,
            truncated: false,
            semantic_head: None,
            semantic_failure: None,
            semantic_trunk: None,
            landed: vec![],
            slicing: None,
            sym_infos: None,
            blame_infos: vec![],
            later: vec![],
            block_sym_info: None,
        };
        // (The first's commit rebased is "rrrr"; what the push removes from
        // a.cpp has no commits.)
        let slicing: crate::dataflow::slice::Slicing = serde_json::from_value(json!({
            "units": [
                {"id": 0, "path": "a.cpp", "lines": [3], "func": "A::f", "func_sym": "_ZN1A1fEv",
                 "class": "T_A", "commits": ["rrrr"], "keys": [], "text": ["  x = 1;"]},
                {"id": 1, "path": "b.cpp", "lines": [7], "func": "g", "commits": ["bbbb"],
                 "keys": [], "text": ["  g();"]},
                {"id": 2, "path": "a.cpp", "lines": [9], "removed": true, "keys": [], "text": ["old();"]},
                {"id": 3, "path": "a.cpp", "lines": [3], "func": "A::f", "removed": true,
                 "commits": ["rrrr"], "keys": [], "text": ["  x = 0;"]},
            ],
            "links": [
                {"from": 0, "to": 1, "why": "field mX"},
                {"from": 1, "to": 0, "why": "function _ZN1A1fEv"},
                {"from": 0, "to": 2, "why": "local V_1"},
                {"from": 3, "to": 1, "why": "field mY"},
                {"from": 3, "to": 0, "why": "flows: x"},
            ],
            "clusters": [[0, 1, 2]],
            "notes": [],
        }))
        .unwrap();
        let rebased: BTreeMap<String, String> = [("rrrr".to_string(), "aaaa".to_string())]
            .into_iter()
            .collect();
        let groups = group_slicings(&view, &slicing, &rebased);
        assert_eq!(groups.len(), 2);
        // The first's: its units (A::f's, by its rebased commit, under its
        // definition, what it removes and adds of it one node; and what it
        // removes from its file), then the second group's neighbor.
        let first = &groups[0];
        let labels: Vec<&str> = first.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["A::f", "a.cpp", "g"]);
        assert_eq!(first.nodes[0].part.as_deref(), Some("d:aaaa:a.cpp:A::f"));
        assert_eq!(first.nodes[0].scope.as_deref(), Some("A"));
        assert_eq!(first.nodes[0].change, "changed");
        assert_eq!(first.nodes[0].text, vec!["  x = 1;"]);
        assert_eq!(first.nodes[0].old_text, vec!["  x = 0;"]);
        assert_eq!(first.nodes[1].part.as_deref(), Some("f:aaaa:a.cpp"));
        assert_eq!(first.nodes[1].change, "removed");
        assert_eq!(first.nodes[2].other, Some(1));
        // (A pair's links are one edge, with their reasons.)
        let edges: Vec<(usize, usize, Vec<String>)> = first
            .edges
            .iter()
            .map(|e| (e.from, e.to, e.why.clone()))
            .collect();
        assert_eq!(
            edges,
            vec![
                (0, 1, vec!["a local value".to_string()]),
                (
                    0,
                    2,
                    vec![
                        "the field mX".to_string(),
                        "calls or changes A::f".to_string(),
                        "the field mY".to_string()
                    ]
                ),
            ]
        );
        // The second's: its unit, and the first's neighbor.
        let second: Vec<(&str, Option<usize>)> = groups[1]
            .nodes
            .iter()
            .map(|n| (n.label.as_str(), n.other))
            .collect();
        assert_eq!(second, vec![("g", None), ("A::f", Some(0))]);

        // Its cards' lines: A::f's added line, and the likeliest lines on its
        // dataflow (of its unit's lines), from the head's file.
        let mut groups = groups;
        let lines = json!({
            "files": {"a.cpp": {
                "1": {"d": 0.5, "why": ["Carries x"]},
                "2": {"d": 1.0, "why": ["Writes x, which line 3 reads"]},
                "5": {"d": 0.7, "why": ["Not A::f's"]},
            }},
            "units": {"0": [1, 2], "1": [5]},
        });
        let text: BTreeMap<String, Vec<String>> = [(
            "a.cpp".to_string(),
            ["void A::f() {", "  int x;", "  x = 1;", "}", "g();"]
                .iter()
                .map(|line| line.to_string())
                .collect(),
        )]
        .into_iter()
        .collect();
        card_lines(&mut groups, &lines, &text);
        let card: Vec<(u32, &str, bool, Option<f32>)> = groups[0].nodes[0]
            .card
            .iter()
            .map(|line| (line.line, line.text.as_str(), line.added, line.d))
            .collect();
        assert_eq!(
            card,
            vec![
                (1, "void A::f() {", false, Some(0.5)),
                (2, "  int x;", false, Some(1.0)),
                (3, "  x = 1;", true, None),
            ]
        );
        // (Not what's removed, which isn't in the head.)
        assert!(groups[0].nodes[1].card.is_empty());
    }

    #[test]
    fn test_add_later_changes() {
        // (Patch 1 adds `x = 1;`, which patch 2 changes to `x = 2;`.)
        let line = |kind: &str, old: Option<u32>, new: Option<u32>, text: &str| HunkLine {
            kind: kind.to_string(),
            old,
            new,
            text: text.to_string(),
            ..Default::default()
        };
        let hunk = |id: &str, lines: Vec<HunkLine>| ReviewHunk {
            id: id.to_string(),
            def: None,
            section: Section::Uses,
            lines,
            added: 1,
            removed: 0,
        };
        let mut first = patch("r1", &["a.cpp"], 1, 1);
        first.parent = Some("p0".to_string());
        first.files[0].hunks = vec![hunk(
            "h1",
            vec![
                line(" ", Some(1), Some(1), "a"),
                line("+", None, Some(2), "x = 1;"),
                line(" ", Some(2), Some(3), "b"),
            ],
        )];
        let mut second = patch("r2", &["a.cpp"], 1, 2);
        second.parent = Some("r1".to_string());
        second.files[0].hunks = vec![hunk(
            "h2",
            vec![
                line(" ", Some(1), Some(1), "a"),
                line("-", Some(2), None, "x = 1;"),
                line("+", None, Some(2), "x = 2;"),
                line(" ", Some(3), Some(3), "b"),
            ],
        )];
        let mut view = ReviewView {
            push: "r2".to_string(),
            key: String::new(),
            indices: vec![],
            patches: vec![first, second],
            groups: vec![],
            task_selections: 0,
            truncated: false,
            semantic_head: None,
            semantic_failure: None,
            semantic_trunk: None,
            landed: vec![],
            slicing: None,
            sym_infos: None,
            blame_infos: vec![],
            later: vec![],
            block_sym_info: None,
        };
        let texts: BTreeMap<(String, String), Option<String>> = [
            ("p0", "a\nb\n"),
            ("r1", "a\nx = 1;\nb\n"),
            ("r2", "a\nx = 2;\nb\n"),
        ]
        .into_iter()
        .map(|(rev, text)| {
            (
                (rev.to_string(), "a.cpp".to_string()),
                Some(text.to_string()),
            )
        })
        .collect();
        add_later_changes(&mut view, &texts);
        let lines = &view.patches[0].files[0].hunks[0].lines;
        assert_eq!(
            lines[1].later,
            Some(0),
            "patch 2 changes the line patch 1 adds"
        );
        assert_eq!(lines[0].later, None, "but not the lines around it");
        assert_eq!(
            view.later,
            vec![LaterChange {
                patch: 1,
                element: element_id("h2", Section::Uses),
                lines: vec!["x = 2;".to_string()],
            }]
        );
        // (And patch 2's own lines have no later changes.)
        assert!(
            view.patches[1].files[0].hunks[0]
                .lines
                .iter()
                .all(|l| l.later.is_none())
        );
    }

    #[test]
    fn test_split_hunk() {
        // (A hunk changing f's signature, and adding g after it.)
        let diff = "diff --git a.cpp a.cpp
--- a.cpp
+++ a.cpp
@@ -1,4 +1,8 @@
-int f(int a) {
+int f(int a, bool b) {
   return a;
 }
 
+int g() {
+  return 1;
+}
+
";
        let hunk = &parse_diff(diff)[0].hunks[0];
        let entries: Vec<OutlineEntry> = serde_json::from_value(json!([
            {"pretty": "f", "name": "f", "kind": "function", "depth": 0, "status": "changed", "lines": [1, 3]},
            {"pretty": "g", "name": "g", "kind": "function", "depth": 0, "status": "added", "lines": [5, 7]},
        ]))
        .unwrap();
        let parts = split_hunk(&entries, hunk);
        assert_eq!(parts.len(), 2);
        // (Each with its context, but not the other's changes.)
        assert_eq!(parts[0].lines.first().unwrap().text, "int f(int a) {");
        assert!(parts[0].lines.iter().all(|line| !line.text.contains("g()")));
        assert_eq!(
            parts[1].lines.iter().find(|l| l.kind == "+").unwrap().text,
            "int g() {"
        );
        assert_eq!(hunk_entries(&entries, &parts[0]), vec![0]);
        assert_eq!(hunk_entries(&entries, &parts[1]), vec![1]);
    }

    #[test]
    fn test_with_context() {
        let lines = |kinds: &str| Hunk {
            lines: kinds
                .chars()
                .enumerate()
                .map(|(i, kind)| HunkLine {
                    kind: kind.to_string(),
                    old: Some(i as u32 + 1),
                    new: Some(i as u32 + 1),
                    text: format!("{}", i),
                    ..Default::default()
                })
                .collect(),
        };
        // (Changes at 2 and 12, with 2 lines of context: two hunks; at 2 and
        // 5: one.)
        let far = with_context(&lines("  -         +   "), 2);
        assert_eq!(far.len(), 2);
        assert_eq!(far[0].lines.first().unwrap().text, "0");
        assert_eq!(far[0].lines.last().unwrap().text, "4");
        assert_eq!(far[1].lines.first().unwrap().text, "10");
        let near = with_context(&lines("  -  +     "), 2);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].lines.len(), 8);
    }

    #[test]
    fn test_group_patches() {
        let patches = vec![
            patch("a", &["x.h", "x.cpp"], 1, 50),
            patch("b", &["x.cpp", "y.cpp"], 1, 50),
            // (Too big to join, and alone.)
            patch("c", &["y.cpp"], 1, 500),
            patch("d", &["z.cpp"], 2, 10),
            // (Shares x.h with the first group, but isn't next to it.)
            patch("e", &["x.h"], 3, 10),
        ];
        let groups = group_patches(&patches);
        let members: Vec<Vec<usize>> = groups.iter().map(|g| g.patches.clone()).collect();
        assert_eq!(members, vec![vec![0, 1], vec![2], vec![3], vec![4]]);
        assert_eq!(groups[0].shared[0], "x.cpp");
        assert_eq!(groups[0].honorary, vec![2, 4]);
        assert_eq!(groups[2].honorary, Vec::<usize>::new());
    }

    /// Bug 2090040's push 2be539751b0f, its patches' files: 1 and 5 share
    /// PBackground's files (but aren't consecutive), 3 and 4
    /// NeckoChannelParams.ipdlh and BackgroundUtils.cpp, 5 and 6 only
    /// dom/storage, and 2 nothing; and all of them share ipc/glue, but 2 and
    /// 6, and the bug.
    #[test]
    fn test_group_patches_bug_2090040() {
        let bug = 2090040;
        let patches = vec![
            patch(
                "1",
                &[
                    "dom/broadcastchannel/BroadcastChannel.cpp",
                    "dom/network/UDPSocketChild.cpp",
                    "dom/storage/StorageActivityService.cpp",
                    "ipc/glue/BackgroundChildImpl.cpp",
                    "ipc/glue/BackgroundChildImpl.h",
                    "ipc/glue/BackgroundParentImpl.cpp",
                    "ipc/glue/BackgroundParentImpl.h",
                    "ipc/glue/PBackground.ipdl",
                ],
                bug,
                126,
            ),
            patch(
                "2",
                &[
                    "netwerk/protocol/http/HttpChannelChild.cpp",
                    "netwerk/protocol/http/HttpChannelParent.cpp",
                    "netwerk/protocol/http/PHttpChannel.ipdl",
                ],
                bug,
                22,
            ),
            patch(
                "3",
                &[
                    "dom/serviceworkers/ServiceWorkerPrivate.cpp",
                    "ipc/glue/BackgroundUtils.cpp",
                    "ipc/glue/BackgroundUtils.h",
                    "netwerk/ipc/NeckoChannelParams.ipdlh",
                ],
                bug,
                83,
            ),
            patch(
                "4",
                &[
                    "ipc/glue/BackgroundUtils.cpp",
                    "netwerk/ipc/DocumentLoadListener.cpp",
                    "netwerk/ipc/NeckoChannelParams.ipdlh",
                    "netwerk/ipc/NeckoParent.cpp",
                ],
                bug,
                129,
            ),
            patch(
                "5",
                &[
                    "dom/storage/LocalStorageManager.cpp",
                    "dom/storage/PBackgroundLocalStorageCache.ipdl",
                    "dom/storage/StorageIPC.cpp",
                    "ipc/glue/BackgroundChildImpl.cpp",
                    "ipc/glue/BackgroundChildImpl.h",
                    "ipc/glue/BackgroundParentImpl.cpp",
                    "ipc/glue/BackgroundParentImpl.h",
                    "ipc/glue/PBackground.ipdl",
                ],
                bug,
                124,
            ),
            patch(
                "6",
                &[
                    "dom/storage/PBackgroundSessionStorageCache.ipdl",
                    "dom/storage/SessionStorageManager.cpp",
                    "toolkit/components/sessionstore/SessionStoreUtils.cpp",
                ],
                bug,
                53,
            ),
        ];
        let groups = group_patches(&patches);
        let members: Vec<Vec<usize>> = groups.iter().map(|g| g.patches.clone()).collect();
        assert_eq!(members, vec![vec![0], vec![1], vec![2, 3], vec![4, 5]]);
        assert_eq!(groups[0].honorary, vec![4]);
        assert_eq!(groups[3].honorary, vec![0]);
        assert_eq!(groups[3].shared, vec!["dom/storage/"]);
    }

    #[test]
    fn test_hunk_hash_ignores_context() {
        let files = parse_diff(DIFF);
        let mut moved = files[0].hunks[0].clone();
        for line in &mut moved.lines {
            line.old = line.old.map(|n| n + 10);
            line.new = line.new.map(|n| n + 10);
            if line.kind == " " {
                line.text.push_str(" // changed context");
            }
        }
        assert_eq!(
            hunk_hash("a.cpp", &files[0].hunks[0]),
            hunk_hash("a.cpp", &moved)
        );
    }
}
