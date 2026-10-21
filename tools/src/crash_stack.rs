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
//! Columns (the grid's `--crash-columns`): the frames' code, and the frames.
//! A third column could later show the machine code around each frame's
//! address, from a crash report (rather than a signature), whose frames have
//! their modules and offsets (`CrashFrame::address`); and a report's frames
//! have their files' lines too, rather than believed ones.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use serde::Serialize;
use ustr::{Ustr, UstrMap, UstrSet, ustr};

use crate::abstract_server::{AbstractServer, Result, sym_info};
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
}

/// A frame of a crash's stack.
#[derive(Debug, Serialize)]
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
}

/// A frame's function in the tree.
#[derive(Debug, Serialize)]
pub struct FrameFunction {
    pub sym: Ustr,
    pub pretty: Ustr,
    /// Its definition's file and line.
    pub path: Ustr,
    pub line: u32,
}

/// A line of a frame's excerpt.
#[derive(Debug, Serialize)]
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
    let rows = server
        .fetch_html_lines(requests.into_iter().collect())
        .await
        .unwrap_or_default();
    for (i, all, shown) in wanted {
        let frame = &mut frames[i];
        let path = frame.function.as_ref().unwrap().path;
        let Some(file_rows) = rows.get(&path) else {
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
    })
}

/// Runs of consecutive numbers.
fn runs(numbers: impl Iterator<Item = u32>) -> Vec<Vec<u32>> {
    let mut runs: Vec<Vec<u32>> = vec![];
    for n in numbers {
        match runs.last_mut() {
            Some(run) if run.last() == Some(&(n - 1)) => run.push(n),
            _ => runs.push(vec![n]),
        }
    }
    runs
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
    use crate::url_encode_path::url_encode_path;

    let crash_stats = format!(
        "https://crash-stats.mozilla.org/signature/?signature={}",
        url_encode_query(&stack.signature)
    );
    let found = stack.frames.iter().filter(|f| f.function.is_some()).count();
    let mut html = String::new();
    html.push_str(r#"<div class="crash-view">"#);
    html.push_str(&format!(
        r#"<div class="crash-notice"><p>The stack of the crash signature <a class="crash-signature" href="{}">{}</a>, innermost frame first: each frame beside its function's code in {}, with the lines it's believed to have been at (its calls of the frame above it) highlighted.  {} of its {} frames are functions in the tree.</p>"#,
        escape(&crash_stats),
        escape(&stack.signature),
        escape(tree_name),
        found,
        stack.frames.len()
    ));
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
    for frame in &stack.frames {
        html.push_str(&format!(
            r#"<section class="crash-frame" role="listitem" id="frame-{}">"#,
            frame.index
        ));
        // The code column.
        html.push_str(r#"<div class="crash-code">"#);
        if let Some(function) = &frame.function {
            let url = format!("/{}/source/{}", tree_name, url_encode_path(&function.path));
            html.push_str(&format!(
                r#"<div class="crash-code-heading"><a href="{}#{}">{}:{}</a></div>"#,
                escape(&url),
                function.line,
                escape(&function.path),
                function.line
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
            html.push_str(r#"<div class="crash-code-missing">Not a function in this tree.</div>"#);
        }
        html.push_str("</div>");
        // The frame column.
        html.push_str(&format!(
            r#"<div class="crash-frame-info"><span class="crash-frame-index">#{}</span> "#,
            frame.index
        ));
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
        let where_ = match (
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
        };
        if !where_.is_empty() {
            html.push_str(&format!(
                r#"<div class="crash-frame-where">{}</div>"#,
                where_
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
    }
    // (The stack's, and the view's.)
    html.push_str("</div></div>");
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
}
