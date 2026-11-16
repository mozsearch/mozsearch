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
/// numbers before and after.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HunkLine {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<u32>,
    pub text: String,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewPatch {
    /// Its part's id: `p:<rev>`.
    pub id: String,
    pub rev: String,
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
        hunks.push(ReviewHunk {
            id: format!("h:{}:{}:{}", rev, file.path, hunk_hash(&file.path, hunk)),
            def: parent,
            section,
            added: hunk.count("+"),
            removed: hunk.count("-"),
            lines: hunk.lines.clone(),
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
    for commit in &try_push.stack {
        let diff = server.commit_diff(&commit.rev).await?;
        let summary = diff.message.lines().next().unwrap_or("").to_string();
        let (bugs, phab) = message_refs(&summary, &diff.message);
        indices.push(json!(["rev", diff.rev]));
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
            let (path_kind, subsystem) = match server.file_facet_info(&file.path) {
                Some((kind, subsystem)) => (kind.to_string(), subsystem.map(|s| s.to_string())),
                None if looks_like_test(&file.path) => ("test".to_string(), None),
                None => ("normal".to_string(), None),
            };
            files.push(review_file(
                &diff.rev,
                file,
                outline.as_ref(),
                path_kind,
                subsystem,
            ));
        }
        patches.push(ReviewPatch {
            id: format!("p:{}", diff.rev),
            rev: diff.rev,
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
    })
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
/// it changes, and its bugs.
fn patch_keys(patch: &ReviewPatch) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for file in &patch.files {
        keys.insert(file.path.clone());
        for def in &file.defs {
            if def.level == "class" {
                keys.insert(format!("class {}", def.pretty));
            }
        }
    }
    for bug in &patch.bugs {
        keys.insert(format!("bug {}", bug));
    }
    keys
}

/// The patches' groups: consecutive patches sharing a file, a class or a
/// bug, as long as the group's changes stay under `GROUP_LINES` (a patch
/// past it alone is a group of its own); and as honorary members of a group,
/// the other groups' patches sharing a file or a class with it.  (The patch
/// is the unit of review, and a stack's patches are usually in the order to
/// read them, so the groups are of consecutive patches; and better too small
/// than too much to take in.)
pub fn group_patches(patches: &[ReviewPatch]) -> Vec<PatchGroup> {
    let keys: Vec<BTreeSet<String>> = patches.iter().map(patch_keys).collect();
    let lines: Vec<u32> = patches.iter().map(patch_lines).collect();
    let mut groups: Vec<PatchGroup> = vec![];
    let mut group_lines = 0;
    for i in 0..patches.len() {
        let joins = groups.last().is_some_and(|group| {
            group_lines + lines[i] <= GROUP_LINES
                && group
                    .patches
                    .iter()
                    .any(|j| !keys[*j].is_disjoint(&keys[i]))
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
        // (What its patches share, files and classes first.)
        if group.patches.len() > 1 {
            let mut shared: BTreeMap<&String, usize> = BTreeMap::new();
            for i in &group.patches {
                for key in &keys[*i] {
                    *shared.entry(key).or_default() += 1;
                }
            }
            let mut shared: Vec<(&String, usize)> =
                shared.into_iter().filter(|(_, n)| *n > 1).collect();
            shared.sort_by_key(|(key, n)| (key.starts_with("bug "), std::cmp::Reverse(*n)));
            group.shared = shared
                .into_iter()
                .take(4)
                .map(|(key, _)| key.clone())
                .collect();
        }
        // (Honorary members: the others sharing a file or a class, not just
        // a bug, the nearest first, a few.)
        let members: BTreeSet<&String> = group
            .patches
            .iter()
            .flat_map(|i| keys[*i].iter())
            .filter(|key| !key.starts_with("bug "))
            .collect();
        let mut honorary: Vec<usize> = (0..patches.len())
            .filter(|i| !group.patches.contains(i))
            .filter(|i| keys[*i].iter().any(|key| members.contains(key)))
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

fn strip_slot(part: &str) -> String {
    format!(
        r#"<span class="tr-strip" data-part="{}"></span>"#,
        escape(part)
    )
}

/// A side of a hunk: its lines before (`then`, kept and removed) or after
/// (kept and added).
fn side_html(hunk: &ReviewHunk, then: bool) -> String {
    let mut html = String::new();
    for line in &hunk.lines {
        let (number, class) = match (then, line.kind.as_str()) {
            (true, "+") | (false, "-") => continue,
            (true, "-") => (line.old, " tr-removed"),
            (false, "+") => (line.new, " tr-added"),
            (true, _) => (line.old, ""),
            (false, _) => (line.new, ""),
        };
        html.push_str(&format!(
            r#"<div class="tr-line{}"><span class="tr-lno">{}</span><code class="tr-text">{}</code></div>"#,
            class,
            number.map_or(String::new(), |n| n.to_string()),
            escape(&line.text)
        ));
    }
    html
}

fn hunk_html(tree: &str, rev: &str, file: &ReviewFile, hunk: &ReviewHunk) -> String {
    let first = hunk.lines.iter().find_map(|line| line.new).unwrap_or(0);
    let last = hunk
        .lines
        .iter()
        .filter_map(|line| line.new)
        .max()
        .unwrap_or(first);
    let link = format!(
        "/{}/rev/{}/{}#{}",
        escape(tree),
        escape(rev),
        crate::url_encode_path::url_encode_path(&file.path),
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
    // (A new file has no code then, and a removed one none now.)
    let code = match file.status.as_str() {
        "added" => format!(
            r#"<div class="now-code now-only">{}</div>"#,
            side_html(hunk, false)
        ),
        "removed" => format!(
            r#"<div class="then-code then-only">{}</div>"#,
            side_html(hunk, true)
        ),
        _ => format!(
            r#"<div class="then-now"><div class="then-code">{}</div><div class="now-code">{}</div></div>"#,
            side_html(hunk, true),
            side_html(hunk, false)
        ),
    };
    format!(
        r#"<div class="tr-hunk" data-part="{id}"><div class="tr-hunk-head">{strip} <a class="tr-hunk-lines" href="{link}">lines {first}&ndash;{last}</a> <span class="tr-delta">{delta}</span></div>{code}</div>"#,
        id = escape(&hunk.id),
        strip = strip_slot(&hunk.id),
        link = link,
        first = first,
        last = last,
        delta = escape(&delta),
        code = code,
    )
}

/// A file's hunks of a section, under their definitions, in the file's
/// order.
fn file_html(
    tree: &str,
    patch_index: usize,
    patch: &ReviewPatch,
    file: &ReviewFile,
    section: Section,
) -> String {
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
    fn children_html(
        tree: &str,
        rev: &str,
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
            items.push((first, hunk_html(tree, rev, file, hunk)));
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
            items.push((
                first,
                format!(
                    r#"<div class="tr-def tr-def-{level}" data-part="{id}"><div class="tr-def-head">{strip} <span class="tr-def-kind">{kind}</span> <code class="tr-def-name" title="{pretty}">{name}</code>{decl}{status}</div>{children}</div>"#,
                    level = escape(&def.level),
                    id = escape(&def.id),
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
                    children = children_html(tree, rev, file, Some(&def.id), hunks, wanted),
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
        r#"<section class="tr-file" data-part="{id}" data-path="{path}" data-facets="{facets}"><h5 class="tr-file-head">{strip} <a href="/{tree}/rev/{rev}/{link}">{path}</a>{status}</h5>{children}{truncated}</section>"#,
        id = escape(&file.id),
        path = escape(&file.path),
        facets = escape(&serde_json::to_string(&facets).unwrap_or_default()),
        strip = strip_slot(&file.id),
        tree = escape(tree),
        rev = escape(&patch.rev),
        link = crate::url_encode_path::url_encode_path(&file.path),
        status = status,
        children = children_html(tree, &patch.rev, file, None, &hunks, &wanted),
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
        strip = strip_slot(&patch.id),
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
            r#"<div class="tr-section tr-section-{slug}" data-section="{slug}"><h4 class="tr-section-head"><span class="tr-section-label">{label}</span> <span class="tr-section-about">{about}</span><span class="tr-section-jumps">{jumps}</span></h4>"#,
            slug = section.slug(),
            label = section.label(),
            about = section.about(),
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
            html.push_str(&file_html(tree, i, patch, file, *section));
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
        r#"<div class="try-review then-now-view facet-items" id="try-review" data-then-now="both" data-then-now-narrow="now" data-key="{key}">"#,
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
        }),
    ));
    // (The breadcrumbs' trail, which try-review.js puts in the page's.)
    html.push_str(&format!(
        r##"<span class="tr-crumbs" hidden><span class="path-separator">/</span><a href="/{tree}/try/hg/{push}">try push {short}</a><span class="path-separator">/</span><span class="tr-crumb-review">review</span><span class="path-separator">/</span><span class="tr-crumb-patch"><a class="tr-crumb-patch-link" href="#tr-patch-1">Patch 1</a><button type="button" class="crash-crumb-button" id="tr-patch-picker" title="Pick a patch" aria-expanded="false" aria-haspopup="true" aria-controls="tr-patch-menu"></button><div id="tr-patch-menu" title="The patches" role="menu" class="context-menu initially-hidden"></div></span><span class="tr-crumb-then-now" hidden></span></span>"##,
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
        modes = crate::crash_stack::then_now_modes_html(
            "The code before each change.",
            "The code after each change."
        ),
    ));
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
    if view.groups.is_empty() {
        view.groups = (0..view.patches.len())
            .map(|i| PatchGroup {
                patches: vec![i],
                honorary: vec![],
                shared: vec![],
            })
            .collect();
    }
    for (g, group) in view.groups.iter().enumerate() {
        html.push_str(&format!(
            r#"<section class="tr-group" id="tr-group-{}">"#,
            g + 1
        ));
        if view.groups.len() > 1 {
            html.push_str(&group_head_html(&view, g));
        }
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
                        added: lines,
                        removed: 0,
                    }],
                    truncated: false,
                })
                .collect(),
            files_left_out: 0,
        }
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
