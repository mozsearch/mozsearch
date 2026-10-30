//! The crash view (the `crash-signature:` query term's, through the
//! `crash-stack` command, which the DWIM rule "crash-stats-signature" sends
//! crash-stats signatures' URLs to; see query/dwim.toml): a crash's stack
//! beside its frames' code, as the algorithm view puts a spec's steps beside
//! the code citing them.  A row per frame,
//! innermost first: the frame's function's code, with the lines believed to
//! be where it was (its calls of the frame inside it) highlighted, beside the
//! frame.
//!
//! A signature (see crash-stats' siggen) is its annotations (ex: "OOM |
//! large") and then its frames' functions' names, `" | "`-separated, without
//! their arguments.  Its frames are found by their functions' pretty names
//! (`AbstractServer::search_identifiers`, exactly, else without template
//! arguments), and where a frame was in its function by its calls of the
//! frame inside it: that function's (and what it overrides') uses whose
//! context is the frame's function (crossref's `uses`), which also tells
//! which of a name's functions (ex: overloads) it is.  The innermost frame's
//! line isn't known.
//!
//! A crash report (the `crash-report:` term's, which the DWIM rule
//! "crash-stats-report" sends crash-stats reports' URLs to; see
//! `crash_report_stack`) knows more: crash-stats' processed crash (its
//! stackwalker's output, public) has each frame's file, line and revision
//! (ex: `git:github.com/mozilla-firefox/firefox:dom/base/nsINode.cpp:<rev>`),
//! and the calls inlined in it, each with its own file and line, which are
//! frames of their own here.  Their lines are mapped from the build's
//! revision to the indexed one's (see `AbstractServer::line_map`), and those
//! which changed since say so.  It shows the crashing thread's stack, or with
//! `crash-threads:all`, every thread's.
//!
//! Columns (the grid's `--crash-columns`): the frames' code, and the frames.
//! A third column could later show the machine code around each frame's
//! address (a report's frames have their modules and offsets;
//! `CrashFrame::address`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use serde::Serialize;
use ustr::{Ustr, UstrMap, UstrSet, ustr};

use crate::abstract_server::{AbstractServer, Result, ServerError, sym_info};
use crate::file_format::jumpref::JumprefData;

/// A crash's stack, as the crash view shows it.
#[derive(Debug, Serialize)]
pub struct CrashStack {
    /// The signature, as crash-stats has it.
    pub signature: String,
    /// Its annotations before its frames (ex: "OOM", "large"), with what
    /// they mean, if we know.
    pub annotations: Vec<(String, Option<&'static str>)>,
    /// Its frames, innermost first.
    pub frames: Vec<CrashFrame>,
    /// For a crash report's stack (rather than a signature's): the report,
    /// whose crashing thread's stack `frames` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<CrashReport>,
    /// For a report's every thread (`crash-threads:all`): the other threads'
    /// stacks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<ThreadStack>,
}

/// A frame of a crash's stack.
#[derive(Clone, Debug, Serialize)]
pub struct CrashFrame {
    /// Its position, 0 for the innermost.
    pub index: usize,
    /// Its function's name, as the signature has it.
    pub name: String,
    /// Its function in the tree, if it's found.
    pub function: Option<FrameFunction>,
    /// Its function's other definitions' files and lines (ex: a header's
    /// inline stub, or another platform's), up to 3.
    pub other_definitions: Vec<(Ustr, u32)>,
    /// The lines of its function believed to be where the frame was: its
    /// calls of the frame inside it (none for the innermost frame, or if
    /// they're not found).
    pub lines: Vec<u32>,
    /// The function `lines` call, when it's not the frame inside's but a
    /// function calling it, whose frame the stack doesn't have (inlined).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<Ustr>,
    /// Its function's excerpt: its first lines, and those around `lines`,
    /// with the others between folded.
    pub rows: Vec<FrameRow>,
    /// Where its machine code is (a module and an offset in it), which only
    /// crash reports have (not signatures): for a disassembly column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// For a crash report's frame: where the report says it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FrameSource>,
    /// Why its excerpt's lines couldn't be fetched, if they couldn't, of a
    /// file the tree has (see `row_problem`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// A frame's function in the tree.
#[derive(Clone, Debug, Serialize)]
pub struct FrameFunction {
    pub sym: Ustr,
    pub pretty: Ustr,
    /// Its definition's file and line.
    pub path: Ustr,
    pub line: u32,
}

/// A line of a frame's excerpt.
#[derive(Clone, Debug, Serialize)]
pub struct FrameRow {
    pub line: u32,
    /// Its row's HTML (see `chunked_gzip::Row`).
    pub html: String,
    /// Whether it's folded (shown on request).
    pub folded: bool,
    /// Whether it's believed to be where the frame was.
    pub crashing: bool,
}

/// Annotations that signatures start with (crash-stats' siggen's), and what
/// they mean.
const ANNOTATIONS: &[(&str, Option<&str>)] = &[
    ("OOM", Some("Out of memory: an allocation failed.")),
    ("large", Some("The allocation was large (over 256 KB).")),
    ("small", Some("The allocation was small.")),
    ("unknown", Some("The allocation's size isn't known.")),
    ("shutdownhang", Some("The process hung at shutdown.")),
    (
        "IPCError-browser",
        Some("The parent process killed a child process over an IPC error."),
    ),
    (
        "IPCError-content",
        Some("A content process crashed over an IPC error."),
    ),
    (
        "AsyncShutdownTimeout",
        Some("A shutdown blocker timed out."),
    ),
];

/// A signature's annotations and its frames' names (see `ANNOTATIONS`; an
/// AsyncShutdownTimeout is followed by its phase and its blocker, which
/// aren't frames either).
pub fn parse_signature(signature: &str) -> (Vec<(String, Option<&'static str>)>, Vec<String>) {
    let mut parts = signature
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .peekable();
    let mut annotations = vec![];
    while let Some(part) = parts.peek() {
        let Some((_, meaning)) = ANNOTATIONS.iter().find(|(name, _)| name == part) else {
            break;
        };
        let async_shutdown = *part == "AsyncShutdownTimeout";
        annotations.push((part.to_string(), *meaning));
        parts.next();
        if async_shutdown {
            for _ in 0..2 {
                if let Some(part) = parts.next() {
                    annotations.push((part.to_string(), None));
                }
            }
        }
    }
    (annotations, parts.map(str::to_string).collect())
}

/// A name without its template arguments (`<...>`, nested).
fn without_template_args(name: &str) -> String {
    let mut depth = 0;
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The symbols of the functions a frame's name names: the identifiers which
/// are its name, else its name without its template arguments; those whose
/// pretty names are it, if any are (an exact search also finds members
/// named so, ex: `nsTSubstring::NS_ABORT_OOM` for `NS_ABORT_OOM`).
async fn frame_symbols(server: &(dyn AbstractServer + Send + Sync), name: &str) -> Vec<Ustr> {
    let mut syms = vec![];
    for needle in [name.to_string(), without_template_args(name)] {
        if !syms.is_empty() || needle.is_empty() {
            break;
        }
        let mut found = vec![];
        for (sym, _) in server
            .search_identifiers(&needle, true, false, 50)
            .await
            .unwrap_or_default()
        {
            if !found.contains(&sym) {
                found.push(sym);
            }
        }
        server.prefetch_crossref(&found).await;
        let mut exact = vec![];
        for sym in &found {
            if let Ok(Some(data)) = server.crossref_lookup(sym).await
                && data
                    .meta
                    .as_ref()
                    .is_some_and(|meta| meta.pretty == needle.as_str())
            {
                exact.push(*sym);
            }
        }
        syms = if exact.is_empty() { found } else { exact };
    }
    syms
}

/// The calls of `callees` in `callers`: the lines of their uses in them, by
/// the caller and the file, with the callee each is of.
async fn calls_in(
    server: &(dyn AbstractServer + Send + Sync),
    callers: &UstrSet,
    callees: &[Ustr],
) -> BTreeMap<(Ustr, Ustr), BTreeMap<u32, Ustr>> {
    let mut calls: BTreeMap<(Ustr, Ustr), BTreeMap<u32, Ustr>> = BTreeMap::new();
    for callee in callees {
        let Ok(Some(data)) = server.crossref_lookup(callee).await else {
            continue;
        };
        for path_hits in data.uses.iter().flatten() {
            for hit in &path_hits.lines {
                if callers.contains(&hit.contextsym) {
                    calls
                        .entry((hit.contextsym, path_hits.path))
                        .or_default()
                        .entry(hit.lineno)
                        .or_insert(*callee);
                }
            }
        }
    }
    calls
}

/// How many functions calling a frame's function are looked at for a call
/// of one of them in the frame's caller (see `crash_stack`).
const MAX_INTERMEDIATES: usize = 64;

/// Whether a path is a header's (whose definitions are less likely what a
/// stack has, ex: an inline stub beside the real one in a source file).
fn is_header(path: &str) -> bool {
    [".h", ".hh", ".hpp", ".hxx", ".inc", ".inl"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// How many lines of a function's start an excerpt shows, and around each of
/// its frame's lines.
const HEAD_LINES: u32 = 3;
const CONTEXT_LINES: u32 = 3;
/// How many lines an excerpt shows of a function whose frame's line isn't
/// known.
const UNKNOWN_LINE_LINES: u32 = 10;

/// The stack of the crash signature `signature`, its frames found in
/// `server`'s tree.
pub async fn crash_stack(
    server: &(dyn AbstractServer + Send + Sync),
    signature: &str,
) -> Result<CrashStack> {
    let (annotations, names) = parse_signature(signature);
    let mut candidates: Vec<Vec<Ustr>> = vec![];
    for name in &names {
        candidates.push(frame_symbols(server, name).await);
    }
    let all_syms: Vec<Ustr> = candidates.iter().flatten().copied().collect();
    server.prefetch_crossref(&all_syms).await;

    // Each frame's function (of its name's), and its calls of the frame
    // inside it, by the uses of that frame's functions (and of what they
    // override) in the frame's functions; else its calls of a function that
    // calls them (ex: a template, like IPC::ReadParam calling a ParamTraits'
    // Read, whose frame the stack doesn't have, as it was inlined).
    let mut chosen: Vec<Option<Ustr>> = vec![None; names.len()];
    // (The file of a frame's calls, their lines, and the function they're of
    // when it's not the frame inside's.)
    let mut lines: Vec<(Option<Ustr>, Vec<u32>, Option<Ustr>)> =
        vec![(None, vec![], None); names.len()];
    for i in 1..names.len() {
        let callers: UstrSet = candidates[i].iter().copied().collect();
        if callers.is_empty() {
            continue;
        }
        let mut callees: Vec<Ustr> = candidates[i - 1].clone();
        for callee in &candidates[i - 1] {
            if let Ok(Some(data)) = server.crossref_lookup(callee).await
                && let Some(meta) = &data.meta
            {
                callees.extend(meta.overrides.iter().map(|o| o.sym));
            }
        }
        let mut calls = calls_in(server, &callers, &callees).await;
        if calls.is_empty() {
            let mut intermediates: Vec<Ustr> = vec![];
            for callee in &callees {
                let Ok(Some(data)) = server.crossref_lookup(callee).await else {
                    continue;
                };
                for hit in data.uses.iter().flatten().flat_map(|p| &p.lines) {
                    if !hit.contextsym.is_empty()
                        && !callers.contains(&hit.contextsym)
                        && !intermediates.contains(&hit.contextsym)
                        && intermediates.len() < MAX_INTERMEDIATES
                    {
                        intermediates.push(hit.contextsym);
                    }
                }
            }
            server.prefetch_crossref(&intermediates).await;
            calls = calls_in(server, &callers, &intermediates).await;
        }
        // (The caller with the most calls, if several.)
        if let Some(((caller, path), call_lines)) =
            calls.into_iter().max_by_key(|(_, lines)| lines.len())
        {
            chosen[i] = Some(caller);
            let via = call_lines
                .values()
                .find(|callee| !callees.contains(callee))
                .copied();
            lines[i] = (Some(path), call_lines.into_keys().collect(), via);
            if chosen[i - 1].is_none() && candidates[i - 1].len() == 1 {
                chosen[i - 1] = Some(candidates[i - 1][0]);
            }
        }
    }

    // Each frame's function's definition: in the file of its calls, else in
    // a source file rather than a header, if it has several.
    let mut frames = vec![];
    for (index, name) in names.iter().enumerate() {
        let sym = chosen[index].or_else(|| candidates[index].first().copied());
        let mut function = None;
        let mut other_definitions = vec![];
        if let Some(sym) = sym
            && let Ok(Some(data)) = server.crossref_lookup(&sym).await
        {
            let defs = data.definitions.as_deref().unwrap_or_default();
            let call_path = lines[index].0;
            let def = defs
                .iter()
                .find(|d| Some(d.path) == call_path)
                .or_else(|| defs.iter().find(|d| !is_header(&d.path)))
                .or_else(|| defs.first());
            if let Some(def) = def
                && let Some(hit) = def.lines.first()
            {
                let pretty = data
                    .meta
                    .as_ref()
                    .map(|meta| meta.pretty)
                    .unwrap_or_else(|| ustr(name));
                function = Some(FrameFunction {
                    sym,
                    pretty,
                    path: def.path,
                    line: hit.lineno,
                });
                other_definitions = defs
                    .iter()
                    .filter(|d| d.path != def.path)
                    .filter_map(|d| Some((d.path, d.lines.first()?.lineno)))
                    .take(3)
                    .collect();
            }
        }
        // (Calls in another file than the definition's aren't its.)
        let (frame_lines, via) = match (&function, lines[index].0) {
            (Some(function), Some(path)) if path == function.path => {
                (lines[index].1.clone(), lines[index].2)
            }
            _ => (vec![], None),
        };
        let via = match via {
            Some(via) => match server.crossref_lookup(&via).await {
                Ok(Some(data)) => data.meta.map(|meta| meta.pretty).or(Some(via)),
                _ => Some(via),
            },
            None => None,
        };
        frames.push(CrashFrame {
            index,
            name: name.clone(),
            function,
            other_definitions,
            lines: frame_lines,
            via,
            rows: vec![],
            address: None,
            source: None,
            problem: None,
        });
    }

    // The excerpts' lines, and their rows.
    let mut wanted: Vec<(usize, BTreeSet<u32>, BTreeSet<u32>)> = vec![];
    for (i, frame) in frames.iter().enumerate() {
        let Some(function) = &frame.function else {
            continue;
        };
        let start = function.line;
        let (shown, end) = match frame.lines.iter().max() {
            None => (
                (start..start + UNKNOWN_LINE_LINES).collect::<BTreeSet<u32>>(),
                start + UNKNOWN_LINE_LINES - 1,
            ),
            Some(&last) => {
                let mut shown: BTreeSet<u32> = (start..start + HEAD_LINES).collect();
                for &line in &frame.lines {
                    shown.extend(
                        line.saturating_sub(CONTEXT_LINES).max(start)..=line + CONTEXT_LINES,
                    );
                }
                (shown, last + CONTEXT_LINES)
            }
        };
        let all: BTreeSet<u32> = (start..=end.max(start)).collect();
        // (Folding fewer than 3 lines saves nothing.)
        let mut shown = shown;
        let folded_runs: Vec<Vec<u32>> = runs(all.difference(&shown).copied());
        for run in folded_runs {
            if run.len() < 3 {
                shown.extend(run);
            }
        }
        wanted.push((i, all, shown));
    }
    let mut requests: BTreeMap<Ustr, BTreeSet<u32>> = BTreeMap::new();
    for (i, all, _) in &wanted {
        let path = frames[*i].function.as_ref().unwrap().path;
        requests.entry(path).or_default().extend(all);
    }
    let (rows, failed) = fetch_rows(server, requests).await;
    for (i, all, shown) in wanted {
        let frame = &mut frames[i];
        let path = frame.function.as_ref().unwrap().path;
        let Some(file_rows) = rows.get(&path) else {
            frame.problem = failed.get(&path).map(row_problem);
            continue;
        };
        for line in all {
            let Some(row) = file_rows.get(&line) else {
                continue;
            };
            frame.rows.push(FrameRow {
                line,
                html: row.html.clone(),
                folded: !shown.contains(&line),
                crashing: frame.lines.contains(&line),
            });
        }
    }

    Ok(CrashStack {
        signature: signature.to_string(),
        annotations,
        frames,
        report: None,
        threads: vec![],
    })
}

/// The rows of a file's lines, by their numbers.
pub(crate) type FileRows = std::collections::HashMap<u32, crate::file_format::chunked_gzip::Row>;

/// The rows of `requests`' files' lines (see
/// `AbstractServer::fetch_html_lines`), all at once, or if that fails (ex:
/// for a file the tree doesn't have), each file's that doesn't; and the
/// errors of the files whose rows couldn't be fetched.
pub(crate) async fn fetch_rows(
    server: &(dyn AbstractServer + Send + Sync),
    requests: BTreeMap<Ustr, BTreeSet<u32>>,
) -> (UstrMap<FileRows>, UstrMap<ServerError>) {
    let requests: Vec<(Ustr, BTreeSet<u32>)> = requests.into_iter().collect();
    if let Ok(rows) = server.fetch_html_lines(requests.clone()).await {
        return (rows, UstrMap::default());
    }
    let mut rows = UstrMap::default();
    let mut failed = UstrMap::default();
    for request in requests {
        let path = request.0;
        match server.fetch_html_lines(vec![request]).await {
            Ok(found) => rows.extend(found),
            Err(err) => {
                failed.insert(path, err);
            }
        }
    }
    (rows, failed)
}

/// What an excerpt says when its file's lines couldn't be fetched (see
/// `fetch_rows`).
pub(crate) fn row_problem(err: &ServerError) -> String {
    format!("Couldn't get its lines: {}", err.message())
}

/// Runs of consecutive numbers.
pub(crate) fn runs(numbers: impl Iterator<Item = u32>) -> Vec<Vec<u32>> {
    let mut runs: Vec<Vec<u32>> = vec![];
    for n in numbers {
        match runs.last_mut() {
            Some(run) if run.last() == Some(&(n - 1)) => run.push(n),
            _ => runs.push(vec![n]),
        }
    }
    runs
}

/// A crash report, as the crash view shows it (see `crash_report_stack`):
/// its ID, what crashed (ex: "Firefox 160.0a1 nightly, build 20261009095351,
/// on macOS 14.6"), its signature, its MOZ_CRASH reason, its crash's type and
/// address, and its threads (how many, and the crashing one).
#[derive(Clone, Debug, Serialize)]
pub struct CrashReport {
    pub id: String,
    pub product: String,
    pub signature: String,
    pub reason: Option<String>,
    pub crash_type: Option<String>,
    pub address: Option<String>,
    pub thread_count: usize,
    pub crashing_thread: Option<ThreadInfo>,
}

/// A thread of a crash report: its index, and its name, if it has one.
#[derive(Clone, Debug, Serialize)]
pub struct ThreadInfo {
    pub index: usize,
    pub name: Option<String>,
}

/// A thread's stack, of a crash report's other threads (see
/// `CrashStack::threads`).
#[derive(Debug, Serialize)]
pub struct ThreadStack {
    pub thread: ThreadInfo,
    pub frames: Vec<CrashFrame>,
}

/// Where a crash report's frame was: its file (a repository's, `git:` or
/// `hg:`, ex: "git:github.com/mozilla-firefox/firefox:dom/base/nsINode.cpp:
/// <rev>", whose `path` and `rev` it is; or another, ex: a system library's,
/// or Rust's standard library's `/rustc/...`), its line, whether it's a call
/// inlined in its frame (whose number it has), and in the tree: the
/// revision (`tree_rev`, an hg revision's git revision), and the line's line
/// at the indexed revision (`indexed_line`), if the line is unchanged since
/// (`changed` if it changed; neither if the tree's repository doesn't have the
/// revision or the file), or if the server can't tell (`unmapped`, ex: a
/// remote server), the same line.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FrameSource {
    pub file: Option<String>,
    pub path: Option<String>,
    pub rev: Option<String>,
    pub line: Option<u32>,
    pub inlined: bool,
    pub tree_rev: Option<String>,
    pub indexed_line: Option<u32>,
    pub changed: bool,
    pub unmapped: bool,
    /// Whether `rev` is an estimate of the build's (ex: a Bugzilla comment's
    /// frames' tree's head then, whose report isn't available; see
    /// `bug_comment`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
}

/// Where crash-stats' processed crashes are (`{}` being a crash's ID),
/// unless the config says (`Config::crash_reports_url`, ex: tests' fixtures).
pub const CRASH_REPORTS_URL: &str =
    "https://crash-stats.mozilla.org/api/ProcessedCrash/?crash_id={}";

/// Whether `id` is a crash report's ID (a UUID, ex:
/// "b40697bb-fa73-46e6-bcd3-0f1ce0261030").
pub fn is_crash_id(id: &str) -> bool {
    id.split('-').map(str::len).eq([8, 4, 4, 4, 12])
        && id
            .bytes()
            .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The crash report `id` (crash-stats' processed crash, as JSON), from
/// `template`'s URL (or file, `file://`), `{}` being its ID (see
/// `CRASH_REPORTS_URL`): one of the last few asked for, if it is (for
/// reloads, and for a report's views with and without its other threads).
pub async fn fetch_crash_report(
    template: &str,
    id: &str,
) -> Result<std::sync::Arc<serde_json::Value>> {
    use crate::abstract_server::{ErrorDetails, ErrorLayer};
    use std::sync::{Arc, Mutex, OnceLock};
    static RECENT: Mutex<Vec<(String, Arc<serde_json::Value>)>> = Mutex::new(vec![]);
    const RECENT_REPORTS: usize = 8;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let problem = |message: String| {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::BadInput,
            message,
        })
    };
    if !is_crash_id(id) {
        return Err(problem(format!("{:?} isn't a crash report's ID", id)));
    }
    let recent = RECENT
        .lock()
        .unwrap()
        .iter()
        .find(|(recent, _)| recent == id)
        .map(|(_, report)| report.clone());
    if let Some(report) = recent {
        return Ok(report);
    }
    // (crash-stats keeps reports for a while; one it doesn't have may have
    // expired.  Its other failures may go away.)
    let missing = || {
        problem(format!(
            "crash-stats doesn't have the crash report {} (it only keeps them for a while)",
            id
        ))
    };
    let url = template.replace("{}", id);
    let text = match url.strip_prefix("file://") {
        Some(path) => std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => missing(),
            _ => problem(format!("Couldn't read crash report {}: {}", id, e)),
        })?,
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
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await
            };
            fetched.await.map_err(|e| match e.status() {
                Some(reqwest::StatusCode::NOT_FOUND) => missing(),
                _ => ServerError::TransientProblem(ErrorDetails {
                    layer: ErrorLayer::ServerLayer,
                    message: format!("Couldn't get crash report {} from crash-stats: {}", id, e),
                }),
            })?
        }
    };
    let report: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| problem(format!("Crash report {} isn't JSON: {}", id, e)))?;
    let report = Arc::new(report);
    let mut recent = RECENT.lock().unwrap();
    recent.push((id.to_string(), report.clone()));
    if recent.len() > RECENT_REPORTS {
        recent.remove(0);
    }
    Ok(report)
}

/// A frame of a crash report's thread's stack, or a call inlined in it (see
/// `parse_report`): its frame's number, its function's name (with its
/// arguments, ex: "mozilla::Foo::Bar(int, char const*)"), where it was, and
/// for a frame (rather than an inlined call) its module and offset in it
/// (ex: "XUL+0x4c556c").
#[derive(Clone, Debug)]
pub struct ReportFrame {
    pub frame: usize,
    pub function: Option<String>,
    pub source: FrameSource,
    pub address: Option<String>,
}

/// A crash report's thread, and its stack's frames (see `parse_report`).
#[derive(Debug)]
pub struct ReportThread {
    pub thread: ThreadInfo,
    pub frames: Vec<ReportFrame>,
}

/// A crash report's summary (see `CrashReport`) and its threads' stacks:
/// each frame's inlined calls (innermost first) and then the frame, as the
/// stackwalker lists them, innermost first.
pub fn parse_report(id: &str, report: &serde_json::Value) -> (CrashReport, Vec<ReportThread>) {
    let text = |value: &serde_json::Value| value.as_str().map(str::to_string);
    let dump = &report["json_dump"];
    let source = |entry: &serde_json::Value, inlined: bool| {
        let file = text(&entry["file"]);
        let (path, rev) = file
            .as_deref()
            .and_then(repository_file)
            .map_or((None, None), |(path, rev)| (Some(path), Some(rev)));
        FrameSource {
            file,
            path,
            rev,
            line: entry["line"]
                .as_u64()
                .filter(|line| *line > 0)
                .map(|line| line as u32),
            inlined,
            ..Default::default()
        }
    };
    let mut threads = vec![];
    for (index, thread) in dump["threads"].as_array().into_iter().flatten().enumerate() {
        let mut frames = vec![];
        for frame in thread["frames"].as_array().into_iter().flatten() {
            let number = frame["frame"].as_u64().map_or(frames.len(), |n| n as usize);
            for inline in frame["inlines"].as_array().into_iter().flatten() {
                frames.push(ReportFrame {
                    frame: number,
                    function: text(&inline["function"]),
                    source: source(inline, true),
                    address: None,
                });
            }
            let offset = text(&frame["module_offset"]).map(|offset| {
                format!(
                    "0x{}",
                    offset.trim_start_matches("0x").trim_start_matches('0')
                )
            });
            let address = match (text(&frame["module"]), offset) {
                (Some(module), Some(offset)) => Some(format!("{}+{}", module, offset)),
                (Some(module), None) => Some(module),
                (None, _) => text(&frame["offset"]),
            };
            frames.push(ReportFrame {
                frame: number,
                function: text(&frame["function"]),
                source: source(frame, false),
                address,
            });
        }
        threads.push(ReportThread {
            thread: ThreadInfo {
                index,
                name: text(&thread["thread_name"]),
            },
            frames,
        });
    }
    let mut product = [
        text(&report["product"]),
        text(&report["version"]),
        text(&report["release_channel"]),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let build = text(&report["build"]).or_else(|| report["build"].as_u64().map(|b| b.to_string()));
    if let Some(build) = build {
        product.push_str(&format!(", build {}", build));
    }
    if let Some(os) = text(&report["os_pretty_version"]).or_else(|| text(&report["os_name"])) {
        product.push_str(&format!(", on {}", os));
    }
    let crashing_thread = dump["crash_info"]["crashing_thread"]
        .as_u64()
        .and_then(|index| threads.get(index as usize))
        .map(|thread: &ReportThread| thread.thread.clone());
    let summary = CrashReport {
        id: id.to_string(),
        product,
        signature: text(&report["signature"]).unwrap_or_default(),
        reason: text(&report["moz_crash_reason"]),
        crash_type: text(&dump["crash_info"]["type"]),
        address: text(&dump["crash_info"]["address"]),
        thread_count: threads.len(),
        crashing_thread,
    };
    (summary, threads)
}

/// The path and revision of a frame's file in a repository (`git:` or `hg:`;
/// ex: "git:github.com/mozilla-firefox/firefox:mfbt/Assertions.h:7af81fe4...").
fn repository_file(file: &str) -> Option<(String, String)> {
    let rest = file
        .strip_prefix("git:")
        .or_else(|| file.strip_prefix("hg:"))?;
    let (repo_path, rev) = rest.rsplit_once(':')?;
    let (_, path) = repo_path.split_once(':')?;
    let rev_ok = rev.len() >= 12 && rev.bytes().all(|b| b.is_ascii_hexdigit());
    (rev_ok && !path.is_empty()).then(|| (path.to_string(), rev.to_string()))
}

/// A function's name without its arguments (ex: "mozilla::Foo::Bar" for
/// "mozilla::Foo::Bar(int, char const*) const"), as signatures have them
/// (but for a call operator's, "operator()").
pub(crate) fn without_arguments(name: &str) -> &str {
    let mut depth = 0;
    for (at, c) in name.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            '(' if depth == 0
                && name[..at].ends_with("operator")
                && name[at..].starts_with("()") => {}
            '(' if depth == 0 && at > 0 => return &name[..at],
            _ => {}
        }
    }
    name
}

/// How many lines around a crash report's other threads' frames' lines their
/// excerpts show (whose functions aren't looked up).
const THREAD_CONTEXT_LINES: u32 = 2;

/// The stack of the crash report `id` (its crashing thread's, or with
/// `all_threads`, every thread's; see the module's documentation), its
/// frames found in `server`'s tree: their files' lines, mapped from the
/// build's revision to the indexed one (`AbstractServer::line_map`; an
/// `hg:` file's revision by `AbstractServer::map_rev`), and the crashing
/// thread's frames' functions, by their names (as a signature's), whose
/// definitions in the frames' files are the nearest before their lines.
pub async fn crash_report_stack(
    server: &(dyn AbstractServer + Send + Sync),
    id: &str,
    all_threads: bool,
) -> Result<CrashStack> {
    let report = server.crash_report(id).await?;
    let (summary, threads) = parse_report(id, &report);
    Ok(report_threads_stack(server, summary, threads, all_threads).await)
}

/// The stack of a crash report's `threads` (see `crash_report_stack`; or of
/// frames a Bugzilla comment lists, whose revision is estimated, see
/// `bug_comment`).
pub async fn report_threads_stack(
    server: &(dyn AbstractServer + Send + Sync),
    summary: CrashReport,
    threads: Vec<ReportThread>,
    all_threads: bool,
) -> CrashStack {
    let (annotations, _) = parse_signature(&summary.signature);
    let crashing = summary
        .crashing_thread
        .as_ref()
        .map_or(0, |thread| thread.index);

    // Each file's lines' lines at the indexed revision, by the report's
    // revision and the path, with the tree's revision (and None for the map
    // if the server can't tell, ex: a remote one's).
    type LineMap = Option<(String, Option<Vec<Option<u32>>>)>;
    let mut maps: BTreeMap<(String, String), LineMap> = BTreeMap::new();
    let mut resolved: Vec<(ThreadInfo, Vec<CrashFrame>)> = vec![];
    for thread in threads {
        if thread.thread.index != crashing && !all_threads {
            continue;
        }
        let mut frames = vec![];
        for frame in thread.frames {
            let mut source = frame.source;
            if let (Some(path), Some(rev)) = (source.path.clone(), source.rev.clone()) {
                let key = (rev.clone(), path.clone());
                if !maps.contains_key(&key) {
                    let is_hg = source.file.as_deref().is_some_and(|f| f.starts_with("hg:"));
                    let tree_rev = match is_hg {
                        true => server.map_rev("hg", &rev).await.ok().flatten(),
                        false => Some(rev.clone()),
                    };
                    let map = match &tree_rev {
                        Some(tree_rev) => match server.line_map(tree_rev, &path).await {
                            Ok(Some(map)) => Some(Some(map)),
                            Err(ServerError::Unsupported) => Some(None),
                            _ => None,
                        },
                        None => None,
                    };
                    maps.insert(key.clone(), tree_rev.zip(map));
                }
                if let Some(Some((tree_rev, map))) = maps.get(&key) {
                    source.tree_rev = Some(tree_rev.clone());
                    if let Some(line) = source.line {
                        match map {
                            Some(map) => {
                                source.indexed_line = map.get(line as usize - 1).copied().flatten();
                                source.changed = source.indexed_line.is_none();
                            }
                            None => {
                                source.indexed_line = Some(line);
                                source.unmapped = true;
                            }
                        }
                    }
                }
            }
            let name = frame.function.clone().unwrap_or_else(|| {
                frame
                    .address
                    .clone()
                    .unwrap_or_else(|| "(unknown)".to_string())
            });
            frames.push(CrashFrame {
                index: frame.frame,
                name,
                function: None,
                other_definitions: vec![],
                lines: source.indexed_line.into_iter().collect(),
                via: None,
                rows: vec![],
                address: frame.address,
                source: Some(source),
                problem: None,
            });
        }
        resolved.push((thread.thread, frames));
    }

    // The crashing thread's frames' functions: of their names', those with
    // definitions in the frames' files, the nearest before their lines.
    if let Some((_, frames)) = resolved
        .iter_mut()
        .find(|(thread, _)| thread.index == crashing)
    {
        for frame in frames.iter_mut() {
            let Some(source) = &frame.source else {
                continue;
            };
            let (Some(path), Some(line)) = (source.path.clone(), source.indexed_line) else {
                continue;
            };
            let name = without_arguments(&frame.name).to_string();
            if name.is_empty() || name.starts_with('{') {
                continue;
            }
            let mut best: Option<FrameFunction> = None;
            // (A Rust frame's name has its crate, ex: "tools::blame::f", which
            // pretty names may not, ex: "blame::f".)
            let mut syms = frame_symbols(server, &name).await;
            if syms.is_empty()
                && let Some((_, rest)) = name.split_once("::")
            {
                syms = frame_symbols(server, rest).await;
            }
            for sym in syms {
                let Ok(Some(data)) = server.crossref_lookup(&sym).await else {
                    continue;
                };
                let pretty = data
                    .meta
                    .as_ref()
                    .map_or_else(|| ustr(&name), |meta| meta.pretty);
                for def in data.definitions.as_deref().unwrap_or_default() {
                    if def.path.as_str() != path {
                        continue;
                    }
                    for hit in &def.lines {
                        if hit.lineno <= line && best.as_ref().is_none_or(|b| hit.lineno > b.line) {
                            best = Some(FrameFunction {
                                sym,
                                pretty,
                                path: def.path,
                                line: hit.lineno,
                            });
                        }
                    }
                }
            }
            frame.function = best;
        }
    }

    // The excerpts: of a frame whose function is known, from its start to
    // its line (folded between); else around its line (or where it would be,
    // if it changed since: after the last line before it which didn't).
    let mut wanted: Vec<(usize, usize, Ustr, BTreeSet<u32>, BTreeSet<u32>)> = vec![];
    for (t, (thread, frames)) in resolved.iter().enumerate() {
        let context = match thread.index == crashing {
            true => CONTEXT_LINES,
            false => THREAD_CONTEXT_LINES,
        };
        for (i, frame) in frames.iter().enumerate() {
            let Some(source) = &frame.source else {
                continue;
            };
            let (Some(path), Some(rev), Some(line)) = (&source.path, &source.rev, source.line)
            else {
                continue;
            };
            let Some(Some((_, map))) = maps.get(&(rev.clone(), path.clone())) else {
                continue;
            };
            let at = source.indexed_line.unwrap_or_else(|| {
                (1..line)
                    .rev()
                    .find_map(|before| {
                        let mapped = map.as_ref()?.get(before as usize - 1).copied().flatten()?;
                        Some(mapped + (line - before))
                    })
                    .unwrap_or(line)
            });
            let start = frame
                .function
                .as_ref()
                .map_or(at.saturating_sub(context).max(1), |function| function.line);
            let all: BTreeSet<u32> = (start..=at + context).collect();
            let mut shown: BTreeSet<u32> = (start..start + HEAD_LINES)
                .chain(at.saturating_sub(context).max(1)..=at + context)
                .filter(|line| all.contains(line))
                .collect();
            for run in runs(
                all.difference(&shown)
                    .copied()
                    .collect::<Vec<_>>()
                    .into_iter(),
            ) {
                if run.len() < 3 {
                    shown.extend(run);
                }
            }
            wanted.push((t, i, ustr(path), all, shown));
        }
    }
    let mut requests: BTreeMap<Ustr, BTreeSet<u32>> = BTreeMap::new();
    for (_, _, path, all, _) in &wanted {
        requests.entry(*path).or_default().extend(all);
    }
    let (rows, failed) = fetch_rows(server, requests).await;
    for (t, i, path, all, shown) in wanted {
        let frame = &mut resolved[t].1[i];
        let Some(file_rows) = rows.get(&path) else {
            // (Not of a file the tree doesn't have, which the frame says.)
            if server.file_facet_info(&path).is_some() {
                frame.problem = failed.get(&path).map(row_problem);
            }
            continue;
        };
        for line in all {
            let Some(row) = file_rows.get(&line) else {
                continue;
            };
            frame.rows.push(FrameRow {
                line,
                html: row.html.clone(),
                folded: !shown.contains(&line),
                crashing: frame.lines.contains(&line),
            });
        }
    }

    let mut frames = vec![];
    let mut others = vec![];
    for (thread, thread_frames) in resolved {
        if thread.index == crashing {
            frames = thread_frames;
        } else {
            others.push(ThreadStack {
                thread,
                frames: thread_frames,
            });
        }
    }
    CrashStack {
        signature: summary.signature.clone(),
        annotations,
        frames,
        report: Some(summary),
        threads: others,
    }
}

/// The SYM_INFO of the stack's excerpts' symbols (for their context menus).
pub async fn stack_sym_info(
    server: &(dyn AbstractServer + Send + Sync),
    stack: &CrashStack,
) -> BTreeMap<Ustr, Option<JumprefData>> {
    let syms: Vec<Ustr> = stack
        .frames
        .iter()
        .flat_map(|frame| &frame.rows)
        .flat_map(|row| sym_info::html_symbols(&row.html))
        .collect();
    let mut info = BTreeMap::new();
    let mut traversed = UstrMap::default();
    sym_info::add_syms_info(server, syms, &mut info, &mut traversed).await;
    info
}

/// The crash view of `stack` (see the module's documentation), for the query
/// page (`crash-stack`'s HTML, which the query page shows as it is): its
/// notice, its rows, and its excerpts' SYM_INFO.
pub fn render_crash_stack(
    tree_name: &str,
    stack: &CrashStack,
    sym_info: &BTreeMap<Ustr, Option<JumprefData>>,
    writer: &mut dyn Write,
) -> std::result::Result<(), &'static str> {
    use crate::rendered_docs::escape;

    let crash_stats = format!(
        "https://crash-stats.mozilla.org/signature/?signature={}",
        url_encode_query(&stack.signature)
    );
    let query_url =
        |query: &str| format!("/{}/query/default?q={}", tree_name, url_encode_query(query));
    let found = stack.frames.iter().filter(|f| f.function.is_some()).count();
    let mut html = String::new();
    html.push_str(r#"<div class="crash-view">"#);
    match &stack.report {
        None => html.push_str(&format!(
            r#"<div class="crash-notice"><p>The stack of the crash signature <a class="crash-signature" href="{}">{}</a>, innermost frame first: each frame beside its function's code in {}, with the lines it's believed to have been at (its calls of the frame above it) highlighted.  {} of its {} frames are functions in the tree.</p>"#,
            escape(&crash_stats),
            escape(&stack.signature),
            escape(tree_name),
            found,
            stack.frames.len()
        )),
        Some(report) => {
            let (what, thread) = report_crashed(report);
            let in_tree = stack
                .frames
                .iter()
                .filter(|f| f.source.as_ref().is_some_and(|s| s.tree_rev.is_some()))
                .count();
            html.push_str(&format!(
                r#"<div class="crash-notice"><p>The crash report <a class="crash-report-link" href="https://crash-stats.mozilla.org/report/index/{}">{}</a> ({}): {}, in {} of its {}.  Its crashing thread's stack, innermost frame first (with the calls inlined in a frame as frames of their own), each frame beside its code in {} at the indexed revision, with its line from the report highlighted.  {} of its {} frames are in the tree's files at the build's revision.</p>"#,
                escape(&report.id),
                escape(&report.id),
                escape(&report.product),
                what,
                thread,
                match report.thread_count {
                    1 => "1 thread".to_string(),
                    n => format!("{} threads", n),
                },
                escape(tree_name),
                in_tree,
                stack.frames.len()
            ));
            if let Some(reason) = &report.reason {
                html.push_str(&format!(
                    r#"<p class="crash-reason">Its MOZ_CRASH reason: <code>{}</code></p>"#,
                    escape(reason)
                ));
            }
            let mut links = vec![];
            if !report.signature.is_empty() {
                links.push(format!(
                    r#"its signature's stack, <a class="crash-signature" href="{}">{}</a>"#,
                    escape(&query_url(&format!(
                        "crash-signature:'{}'",
                        report.signature.replace('\\', "\\\\").replace('\'', "\\'")
                    ))),
                    escape(&report.signature)
                ));
            }
            if report.thread_count > 1 {
                let all = format!("crash-report:{} crash-threads:all", report.id);
                links.push(match stack.threads.is_empty() {
                    true => format!(
                        r#"<a class="crash-all-threads" href="{}">all {} threads' stacks</a>"#,
                        escape(&query_url(&all)),
                        report.thread_count
                    ),
                    false => format!(
                        r#"<a class="crash-crashing-thread" href="{}">only the crashing thread's stack</a> (the others are below it)"#,
                        escape(&query_url(&format!("crash-report:{}", report.id)))
                    ),
                });
            }
            if !links.is_empty() {
                html.push_str(&format!(
                    r#"<p class="crash-links">See {}.</p>"#,
                    links.join("; ")
                ));
            }
        }
    }
    if !stack.annotations.is_empty() {
        html.push_str(r#"<ul class="crash-annotations">"#);
        for (annotation, meaning) in &stack.annotations {
            html.push_str(&format!(
                r#"<li><code>{}</code>{}</li>"#,
                escape(annotation),
                meaning.map_or_else(String::new, |m| format!(": {}", escape(m)))
            ));
        }
        html.push_str("</ul>");
    }
    html.push_str("</div>");

    html.push_str(r#"<div class="crash-stack" role="list">"#);
    for (position, frame) in stack.frames.iter().enumerate() {
        html.push_str(&frame_html(
            tree_name,
            frame,
            &format!("frame-{}", position),
        ));
    }
    html.push_str("</div>");
    // (A report's other threads' stacks, with `crash-threads:all`.)
    for thread in &stack.threads {
        html.push_str(&format!(
            r#"<h3 class="crash-thread-heading" id="thread-{}">Thread #{}{}</h3><div class="crash-stack" role="list">"#,
            thread.thread.index,
            thread.thread.index,
            thread
                .thread
                .name
                .as_ref()
                .map_or_else(String::new, |name| format!(" ({})", escape(name)))
        ));
        for (position, frame) in thread.frames.iter().enumerate() {
            html.push_str(&frame_html(
                tree_name,
                frame,
                &format!("thread-{}-frame-{}", thread.thread.index, position),
            ));
        }
        html.push_str("</div>");
    }
    // (The view's.)
    html.push_str("</div>");
    writer
        .write_all(html.as_bytes())
        .map_err(|_| "Write error")?;

    // (The excerpts' symbols' SYM_INFO, beside the page's, with its "<"s
    // escaped.)
    writeln!(
        writer,
        r#"<script>window.SYM_INFO = Object.assign(window.SYM_INFO || {{}}, {});
// (A frame's folded lines show on request.)
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

/// How a crash report's crash was (ex: "SIGSEGV at 0x0"), and in which thread
/// (ex: "thread #0 (GeckoMain)"), as HTML.
pub(crate) fn report_crashed(report: &CrashReport) -> (String, String) {
    use crate::rendered_docs::escape;
    let thread = match &report.crashing_thread {
        Some(ThreadInfo {
            index,
            name: Some(name),
        }) => format!("thread #{} ({})", index, escape(name)),
        Some(ThreadInfo { index, name: None }) => format!("thread #{}", index),
        None => "a thread".to_string(),
    };
    let what = match (&report.crash_type, &report.address) {
        (Some(kind), Some(address)) => {
            format!("{} at {}", escape(kind), escape(address))
        }
        (Some(kind), None) => escape(kind),
        _ => "It crashed".to_string(),
    };
    (what, thread)
}

/// A frame's row of the crash view (see `render_crash_stack`; and of a
/// Bugzilla comment's frames, see `bug_comment`), with its id: its code (its
/// function's excerpt, or for a crash report's frame, its file's) and what
/// it is.
pub(crate) fn frame_html(tree_name: &str, frame: &CrashFrame, id: &str) -> String {
    use crate::rendered_docs::escape;
    use crate::url_encode_path::url_encode_path;

    let mut html = String::new();
    let source = frame.source.as_ref();
    html.push_str(&format!(
        r#"<section class="crash-frame{}" role="listitem" id="{}">"#,
        if source.is_some_and(|s| s.inlined) {
            " crash-frame-inlined"
        } else {
            ""
        },
        id
    ));
    // The code column: the excerpt of the function's file (or the frame's,
    // for a report's), from its heading's line.
    html.push_str(r#"<div class="crash-code">"#);
    let excerpt_path = frame
        .function
        .as_ref()
        .map(|function| function.path.to_string())
        .or_else(|| source.and_then(|s| s.path.clone()))
        .filter(|_| !frame.rows.is_empty());
    if let Some(path) = excerpt_path {
        let url = format!("/{}/source/{}", tree_name, url_encode_path(&path));
        let line = frame
            .function
            .as_ref()
            .map(|function| function.line)
            .or_else(|| source.and_then(|s| s.indexed_line))
            .or_else(|| frame.rows.first().map(|row| row.line))
            .unwrap_or(1);
        html.push_str(&format!(
            r#"<div class="crash-code-heading"><a href="{}#{}">{}:{}</a></div>"#,
            escape(&url),
            line,
            escape(&path),
            line
        ));
        html.push_str(&format!(
            r#"<div class="file crash-rows" role="table" data-tree="{}" data-search="{}">"#,
            escape(tree_name),
            escape(&crate::file_format::config::search_url(tree_name))
        ));
        let mut folded_run: Vec<&FrameRow> = vec![];
        let flush = |run: &mut Vec<&FrameRow>, html: &mut String| {
            if run.is_empty() {
                return;
            }
            html.push_str(&format!(
                r#"<div class="crash-fold" role="rowgroup" hidden>{}</div><button type="button" class="crash-unfold">Show {} more line{}</button>"#,
                run.iter()
                    .map(|row| crate::format::excerpt_row_html(
                        &row.html,
                        row.line,
                        &url,
                        "sp-citation-lineno",
                        0,
                        None
                    ))
                    .collect::<String>(),
                run.len(),
                if run.len() == 1 { "" } else { "s" }
            ));
            run.clear();
        };
        for row in &frame.rows {
            if row.folded {
                folded_run.push(row);
                continue;
            }
            flush(&mut folded_run, &mut html);
            html.push_str(&crate::format::excerpt_row_html(
                &row.html,
                row.line,
                &url,
                "sp-citation-lineno",
                0,
                row.crashing.then_some("crash-line"),
            ));
        }
        flush(&mut folded_run, &mut html);
        html.push_str("</div>");
    } else {
        let missing = match source {
            None => "Not a function in this tree.".to_string(),
            Some(FrameSource {
                path: Some(path),
                tree_rev: None,
                rev: Some(rev),
                ..
            }) => format!(
                "This tree's repository doesn't have <code>{}</code> at the build's revision, {}.",
                escape(path),
                escape(&rev[..rev.len().min(12)])
            ),
            Some(FrameSource {
                file: Some(file), ..
            }) if source.is_some_and(|s| s.path.is_none()) => {
                format!("Not this tree's code (<code>{}</code>).", escape(file))
            }
            Some(_) => "No source.".to_string(),
        };
        match &frame.problem {
            Some(problem) => html.push_str(&format!(
                r#"<div class="crash-code-missing view-problem">{}</div>"#,
                escape(problem)
            )),
            None => html.push_str(&format!(
                r#"<div class="crash-code-missing">{}</div>"#,
                missing
            )),
        }
    }
    html.push_str("</div>");

    // The frame column.
    html.push_str(&format!(
        r#"<div class="crash-frame-info"><span class="crash-frame-index">#{}</span> "#,
        frame.index
    ));
    if source.is_some_and(|s| s.inlined) {
        html.push_str(r#"<span class="crash-frame-inline" title="A call inlined in the frame">inlined</span> "#);
    }
    match &frame.function {
        Some(function) => html.push_str(&format!(
            r#"<a class="crash-frame-name" href="/{}/source/{}#{}">{}</a>"#,
            escape(tree_name),
            escape(&url_encode_path(&function.path)),
            function.line,
            escape(&frame.name)
        )),
        None => html.push_str(&format!(
            r#"<span class="crash-frame-name crash-frame-unresolved">{}</span>"#,
            escape(&frame.name)
        )),
    }
    let where_ = match source {
        // A signature's frame: where it's believed to have been.
        None => match (
            frame.function.is_some(),
            frame.lines.as_slice(),
            frame.index,
        ) {
            (false, _, _) => String::new(),
            (true, [], 0) => "Where in it the crash was isn't known.".to_string(),
            (true, [], _) => "Its call of the frame above isn't found.".to_string(),
            (true, lines, _) => {
                let call = match &frame.via {
                    Some(via) => format!(
                        "of <code>{}</code>, which calls the frame above (its frame inlined)",
                        escape(via)
                    ),
                    None => "of the frame above".to_string(),
                };
                match lines {
                    [line] => format!("At line {}, its call {}.", line, call),
                    _ => format!(
                        "At one of lines {}, its calls {}.",
                        lines
                            .iter()
                            .map(|l| l.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                        call
                    ),
                }
            }
        },
        // A report's frame: where the report says.
        Some(source) => match (&source.path, source.line, &source.tree_rev) {
            (Some(path), Some(line), Some(tree_rev)) => {
                let build = format!(
                    r#"<a class="crash-frame-build" href="/{}/rev/{}/{}#{}">{}</a>"#,
                    escape(tree_name),
                    escape(tree_rev),
                    escape(&url_encode_path(path)),
                    line,
                    match source.estimated {
                        true => "the tree's revision then",
                        false => "the build's revision",
                    }
                );
                match source.indexed_line {
                    Some(_) if source.unmapped => format!(
                        "At line {} at {} (whether it changed since isn't known here).",
                        line, build
                    ),
                    Some(now) if now == line => format!("At line {} (at {}, as now).", line, build),
                    Some(now) => {
                        format!("At line {} at {}, which is line {} now.", line, build, now)
                    }
                    None => format!(
                        r#"At line {} at {}, <span class="crash-frame-changed">which has changed since</span>."#,
                        line, build
                    ),
                }
            }
            (Some(path), Some(line), None) => {
                format!(
                    "At <code>{}:{}</code> at {}.",
                    escape(path),
                    line,
                    match source.estimated {
                        true => "the tree's revision then",
                        false => "the build's revision",
                    }
                )
            }
            (None, Some(line), _) => source.file.as_ref().map_or_else(String::new, |file| {
                format!("At <code>{}:{}</code>.", escape(file), line)
            }),
            _ => String::new(),
        },
    };
    if !where_.is_empty() {
        html.push_str(&format!(
            r#"<div class="crash-frame-where">{}</div>"#,
            where_
        ));
    }
    if let Some(address) = &frame.address
        && source.is_some()
    {
        html.push_str(&format!(
            r#"<div class="crash-frame-address">{}</div>"#,
            escape(address)
        ));
    }
    if !frame.other_definitions.is_empty() {
        html.push_str(&format!(
            r#"<div class="crash-frame-where">Also defined at {}.</div>"#,
            frame
                .other_definitions
                .iter()
                .map(|(path, line)| format!(
                    r#"<a href="/{}/source/{}#{}">{}:{}</a>"#,
                    escape(tree_name),
                    escape(&url_encode_path(path)),
                    line,
                    escape(path),
                    line
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    html.push_str("</div></section>");
    html
}

/// `text` escaped for a URL's query's value.
fn url_encode_query(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_signature() {
        let (annotations, frames) = parse_signature(
            "OOM | large | NS_ABORT_OOM | IPC::ParamTraits<JSStructuredCloneData>::Read | mozilla::dom::ipc::StructuredCloneData::ReadIPCParams",
        );
        assert_eq!(
            annotations
                .iter()
                .map(|(a, _)| a.as_str())
                .collect::<Vec<_>>(),
            ["OOM", "large"]
        );
        assert_eq!(
            frames,
            [
                "NS_ABORT_OOM",
                "IPC::ParamTraits<JSStructuredCloneData>::Read",
                "mozilla::dom::ipc::StructuredCloneData::ReadIPCParams"
            ]
        );
        // (An AsyncShutdownTimeout's phase and blocker aren't frames.)
        let (annotations, frames) = parse_signature(
            "AsyncShutdownTimeout | profile-before-change | IOUtils: waiting for IO | mozilla::Foo",
        );
        assert_eq!(annotations.len(), 3);
        assert_eq!(frames, ["mozilla::Foo"]);
        assert_eq!(
            without_template_args("IPC::ParamTraits<nsTArray<int>>::Read"),
            "IPC::ParamTraits::Read"
        );
        assert_eq!(
            runs([1, 2, 4, 5, 6, 9].into_iter()),
            [vec![1, 2], vec![4, 5, 6], vec![9]]
        );
    }

    #[test]
    fn test_parse_report() {
        let rev = "7af81fe47ac81825fa31f451dddfaeb736e109ba";
        let file = |path: &str| format!("git:github.com/mozilla-firefox/firefox:{}:{}", path, rev);
        let report = serde_json::json!({
            "product": "Firefox", "version": "160.0a1", "release_channel": "nightly",
            "build": "20261009095351", "os_pretty_version": "macOS 14.6",
            "signature": "arena_t::GetNonFullBinRun | arena_t::MallocSmall",
            "moz_crash_reason": "MOZ_CRASH(oops)",
            "json_dump": {
                "crash_info": {"type": "EXC_BAD_ACCESS", "address": "0x0", "crashing_thread": 1},
                "threads": [
                    {"thread_name": "Main", "frames": [{"frame": 0, "module": "libc.so", "module_offset": "0x0000000000001234"}]},
                    {"thread_name": "IPC I/O Parent", "frames": [
                        {"frame": 0, "function": "malloc", "file": file("memory/build/malloc_decls.h"), "line": 58,
                         "module": "libmozglue.dylib", "module_offset": "0x000000000000cfe0",
                         "inlines": [
                             {"function": "MOZ_CrashSequence(void*, long)", "file": file("mfbt/Assertions.h"), "line": 266},
                             {"function": "arena_t::GetNonFullBinRun(arena_bin_t*)", "file": file("memory/build/mozjemalloc.cpp"), "line": 2020}
                         ]},
                        {"frame": 1, "function": "core::ops::function::FnOnce::call_once", "file": "/rustc/abc/library/core/src/ops/function.rs", "line": 250, "module": "XUL", "module_offset": "0x10"}
                    ]}
                ]
            }
        });
        let (summary, threads) = parse_report("b40697bb-fa73-46e6-bcd3-0f1ce0261030", &report);
        assert_eq!(
            summary.product,
            "Firefox 160.0a1 nightly, build 20261009095351, on macOS 14.6"
        );
        assert_eq!(summary.reason.as_deref(), Some("MOZ_CRASH(oops)"));
        assert_eq!(summary.thread_count, 2);
        let crashing = summary.crashing_thread.unwrap();
        assert_eq!(
            (crashing.index, crashing.name.as_deref()),
            (1, Some("IPC I/O Parent"))
        );
        // (A frame's inlined calls, innermost first, then the frame.)
        let frames = &threads[1].frames;
        assert_eq!(
            frames
                .iter()
                .map(|f| (
                    f.frame,
                    f.source.inlined,
                    f.function.as_deref().unwrap_or("")
                ))
                .collect::<Vec<_>>(),
            [
                (0, true, "MOZ_CrashSequence(void*, long)"),
                (0, true, "arena_t::GetNonFullBinRun(arena_bin_t*)"),
                (0, false, "malloc"),
                (1, false, "core::ops::function::FnOnce::call_once"),
            ]
        );
        assert_eq!(
            frames[1].source.path.as_deref(),
            Some("memory/build/mozjemalloc.cpp")
        );
        assert_eq!(frames[1].source.rev.as_deref(), Some(rev));
        assert_eq!(frames[1].source.line, Some(2020));
        assert_eq!(
            frames[2].address.as_deref(),
            Some("libmozglue.dylib+0xcfe0")
        );
        // (Rust's standard library's files aren't a repository's.)
        assert_eq!(frames[3].source.path, None);
        assert_eq!(
            threads[0].frames[0].address.as_deref(),
            Some("libc.so+0x1234")
        );
        assert_eq!(threads[0].frames[0].function, None);

        assert_eq!(
            without_arguments("mozilla::Foo<int(char)>::Bar(int, char const*) const"),
            "mozilla::Foo<int(char)>::Bar"
        );
        assert_eq!(
            without_arguments("tools::blame::find_phab_rev"),
            "tools::blame::find_phab_rev"
        );
        assert_eq!(
            without_arguments("mozilla::DefaultDelete<T>::operator()(mozilla::ipc::..."),
            "mozilla::DefaultDelete<T>::operator()"
        );
        assert!(is_crash_id("b40697bb-fa73-46e6-bcd3-0f1ce0261030"));
        assert!(!is_crash_id("b40697bb-fa73-46e6-bcd3-0f1ce026103"));
        assert!(!is_crash_id("../../etc/passwd-xxxx-xxxx-xxxx-xxxxxxxxxxxx"));
    }
}
