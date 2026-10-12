//! Bikeshed sources (`.bs`): their definitions, headings, and references, as
//! Bikeshed itself finds and resolves them (its linking is too intricate to
//! reimplement: by text alone, only about half of local links resolve
//! uniquely), from `bikeshed index --json` (in mozsearch's fork of Bikeshed,
//! see its `bikeshed/index`), which has their positions in the source.
//!
//! - `<dfn>`s (and other definitions) are definitions of their ids, as are
//!   headings;
//! - links (autolinks like `[=tree=]` and `{{Node}}`, and `<a>`s) are uses of
//!   the ids they link to, or of the other specs' terms they link to, if
//!   they're to one of the tree's specs (see `SpecsConfig::url_symbol`).
//!
//! Records' ranges are found in the source from Bikeshed's positions: an
//! autolink's syntax, an element's contents, or, in IDL blocks, the name.
//! Positions in the files the source includes (`<pre class=include>`, ex:
//! Markdown files with sections of the spec) are in those files, which get
//! their records.
//! Their contexts are the spec tokenizer's (for Bikeshed, see
//! `spec_tokenizer::bikeshed_contexts`): the algorithm they're in (named and
//! symbolized by its first definition), with the step (from Bikeshed's
//! ordered lists' items), or else their section.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::process::Command;

use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;

use super::markup::Doc;
use super::{
    Combined, ForResolver, InContext, Lines, SpecsConfig, dfn_kind, normalize_path, spec_records,
    spec_structured, spec_symbol, unescape_context_name,
};
use crate::file_format::analysis::Location;
use crate::semantic_analyzer::{Record, sort_records};
use crate::tree_sitter_support::spec_tokenizer::bikeshed_contexts;

/// `bikeshed index --json`'s output (version 2), whose positions are in the
/// source, or, with a `file`, in the file by that path (relative to the
/// source's directory) that it includes.
#[derive(Debug, Default, Deserialize)]
pub struct BikeshedIndex {
    #[serde(rename = "bikeshed-index")]
    pub version: u32,
    #[serde(default)]
    pub dfns: Vec<IndexDfn>,
    #[serde(default)]
    pub links: Vec<IndexLink>,
    #[serde(default)]
    pub headings: Vec<IndexHeading>,
    #[serde(default)]
    pub steps: Vec<IndexStep>,
}

#[derive(Debug, Deserialize)]
pub struct IndexDfn {
    pub line: usize,
    pub col: usize,
    #[serde(default)]
    pub file: Option<String>,
    pub id: String,
    #[serde(rename = "type", default)]
    pub dfn_type: Option<String>,
    #[serde(rename = "for", default)]
    pub dfn_for: Vec<String>,
    /// Its linking texts.
    #[serde(default)]
    pub lt: Vec<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub idl: bool,
}

#[derive(Debug, Deserialize)]
pub struct IndexLink {
    pub line: usize,
    pub col: usize,
    #[serde(default)]
    pub file: Option<String>,
    pub href: String,
    #[serde(rename = "type", default)]
    pub link_type: Option<String>,
    #[serde(default)]
    pub lt: Option<String>,
    #[serde(default)]
    pub syntax: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub idl: bool,
}

#[derive(Debug, Deserialize)]
pub struct IndexHeading {
    pub line: usize,
    pub col: usize,
    #[serde(default)]
    pub file: Option<String>,
    pub id: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct IndexStep {
    pub line: usize,
    pub col: usize,
    #[serde(default)]
    pub file: Option<String>,
    pub path: String,
}

/// Run `bikeshed index --json` (or `$BIKESHED index --json`) on the source.
pub fn run_bikeshed(source_path: &Path) -> Result<String, String> {
    let bikeshed = std::env::var("BIKESHED").unwrap_or_else(|_| "bikeshed".to_string());
    let dir = source_path.parent().unwrap_or(Path::new("."));
    let name = source_path
        .file_name()
        .ok_or_else(|| format!("{}: not a file", source_path.display()))?;
    let output = Command::new(&bikeshed)
        .args(["--silent", "index", "--json"])
        .arg(name)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("couldn't run {}: {}", bikeshed, e))?;
    if !output.status.success() {
        return Err(format!(
            "{} index failed ({}): {}",
            bikeshed,
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

pub fn parse_index(json: &str) -> Result<BikeshedIndex, String> {
    let index: BikeshedIndex = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if index.version != 2 {
        return Err(format!("unknown bikeshed index version {}", index.version));
    }
    Ok(index)
}

/// The analysis's counts, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    pub dfns: usize,
    pub headings: usize,
    pub links: usize,
    /// Links to other specs which aren't the tree's.
    pub external: usize,
    /// Definitions and links whose positions aren't in the source.
    pub unplaced: usize,
}

/// A record to write.
struct Pending {
    range: Range<usize>,
    def: bool,
    sym: String,
    what: String,
    pretty: String,
}

/// The records of a Bikeshed source with its `index`, of the spec with
/// `shortname`, among `config`'s specs (but for the files it includes).
pub fn analyze(
    src: &str,
    index: &BikeshedIndex,
    shortname: &str,
    config: &SpecsConfig,
) -> (Vec<Record>, Stats) {
    let lines = Lines::new(src);
    let locate = |file: Option<&str>, line: usize, col: usize| match file {
        Some(_) => None,
        None => lines.char_position(src, line, col),
    };
    let place = |range: &Range<usize>| Some((0, lines.location(range)));
    let (mut files, stats) = analyze_placed(src, index, shortname, config, 1, &locate, &place);
    (files.pop().unwrap_or_default(), stats)
}

lazy_static! {
    static ref INCLUDE: Regex =
        Regex::new(r#"<pre\s+class=["']?include["']?\s*>((?s:.*?))</pre>"#).unwrap();
    static ref INCLUDE_PATH: Regex = Regex::new(r"(?m)^\s*path:\s*(\S+)").unwrap();
}

/// The records of a Bikeshed spec's source at `main` (a path in the tree,
/// which `read` reads) with its `index`, and of the files it includes, by
/// their paths, as Bikeshed builds the spec from them.
pub fn analyze_with_includes(
    main: &str,
    read: &dyn Fn(&str) -> Option<String>,
    index: &BikeshedIndex,
    shortname: &str,
    config: &SpecsConfig,
) -> Option<(Vec<(String, Vec<Record>)>, Stats)> {
    // (Bikeshed resolves includes' paths, even in included files, relative
    // to the spec's source.)
    let dir = main.rsplit_once('/').map_or("", |(dir, _)| dir);
    let resolve = |path: &str| normalize_path(&format!("{}/{}", dir, path));
    let includes = |_: &str, source: &str| -> Vec<(Range<usize>, String)> {
        INCLUDE
            .captures_iter(source)
            .filter_map(|include| {
                let path = INCLUDE_PATH.captures(include.get(1)?.as_str())?;
                Some((include.get(0)?.range(), resolve(&path[1])))
            })
            .collect()
    };
    let combined = Combined::build(main, read, &includes)?;
    let mut files: HashMap<Option<&str>, Option<usize>> = HashMap::new();
    files.insert(None, Some(0));
    for item_file in index_files(index) {
        files
            .entry(Some(item_file))
            .or_insert_with(|| combined.file(&resolve(item_file)));
    }
    let locate = |file: Option<&str>, line: usize, col: usize| {
        combined.offset((*files.get(&file)?)?, line, col)
    };
    let place = |range: &Range<usize>| combined.place(range);
    let (records, stats) = analyze_placed(
        &combined.text,
        index,
        shortname,
        config,
        combined.paths().count(),
        &locate,
        &place,
    );
    let paths = combined.paths().map(str::to_string);
    Some((paths.zip(records).collect(), stats))
}

/// The included files the index has positions in.
fn index_files(index: &BikeshedIndex) -> impl Iterator<Item = &str> {
    let dfns = index.dfns.iter().map(|d| &d.file);
    let links = index.links.iter().map(|l| &l.file);
    let headings = index.headings.iter().map(|h| &h.file);
    let steps = index.steps.iter().map(|s| &s.file);
    dfns.chain(links)
        .chain(headings)
        .chain(steps)
        .filter_map(|file| file.as_deref())
}

/// The records of a Bikeshed source (with the files it includes) with its
/// `index`, for each of the `files` its text is from, by where `locate` says
/// the index's positions are in it (given their files), and where `place`
/// says its ranges are.
fn analyze_placed(
    src: &str,
    index: &BikeshedIndex,
    shortname: &str,
    config: &SpecsConfig,
    files: usize,
    locate: &dyn Fn(Option<&str>, usize, usize) -> Option<usize>,
    place: &dyn Fn(&Range<usize>) -> Option<(usize, Location)>,
) -> (Vec<Vec<Record>>, Stats) {
    let mut stats = Stats::default();
    let (items, contexts) = bikeshed_contexts(src);
    let doc = Doc::from_items(src, items);
    // Elements, by where their start tags start.
    let elements: HashMap<usize, usize> = (1..doc.nodes.len())
        .filter(|&node| doc.name(node).is_some())
        .map(|node| (doc.nodes[node].range.start, node))
        .collect();
    let line_end = |at: usize| src[at..].find('\n').map_or(src.len(), |e| at + e);
    // An element's contents, if one starts at `at`.
    let contents = |at: usize| -> Option<Range<usize>> {
        let range = doc.content_range(*elements.get(&at)?);
        let text = &src[range.clone()];
        let start = range.start + (text.len() - text.trim_start().len());
        let end = range.start + text.trim_end().len();
        Some(start..end.max(start))
    };

    let mut pending: Vec<Pending> = vec![];
    // Definitions' and headings' (what, pretty), by id.
    let mut defs: HashMap<&str, (String, String)> = HashMap::new();
    // Definitions' positions, for algorithms' symbols.
    let mut dfn_starts: Vec<(usize, String)> = vec![];
    // Definitions' (symbol, kind, what they're for, id), for their structured
    // records, and their parents by what they're for.
    let mut structured_defs: Vec<(String, &'static str, Option<String>, String)> = vec![];
    let mut resolver = ForResolver::default();
    for dfn in &index.dfns {
        let Some(at) = locate(dfn.file.as_deref(), dfn.line, dfn.col) else {
            stats.unplaced += 1;
            continue;
        };
        stats.dfns += 1;
        let what = dfn.dfn_type.clone().unwrap_or_else(|| "dfn".to_string());
        let pretty = match dfn.dfn_for.first() {
            Some(dfn_for) => format!("{} {}/{}", shortname, dfn_for, dfn.text),
            None => format!("{} {}", shortname, dfn.text),
        };
        let range = if dfn.idl && src[at..].starts_with(&dfn.text) {
            at..at + dfn.text.len()
        } else {
            contents(at).unwrap_or_else(|| at..line_end(at).min(at + dfn.text.len().max(1)))
        };
        let sym = spec_symbol(shortname, &dfn.id);
        if !dfn.idl {
            dfn_starts.push((at, sym.clone()));
        }
        let kind = dfn_kind(dfn.dfn_type.as_deref().unwrap_or("dfn"));
        let texts = if dfn.lt.is_empty() {
            vec![dfn.text.clone()]
        } else {
            dfn.lt.clone()
        };
        resolver.add(&sym, kind, &texts, &dfn.dfn_for);
        structured_defs.push((
            sym.clone(),
            kind,
            dfn.dfn_for.first().cloned(),
            dfn.id.clone(),
        ));
        defs.insert(&dfn.id, (what.clone(), pretty.clone()));
        pending.push(Pending {
            range,
            def: true,
            sym,
            what,
            pretty,
        });
    }
    let mut heading_syms: HashMap<usize, String> = HashMap::new();
    for heading in &index.headings {
        let Some(at) = locate(heading.file.as_deref(), heading.line, heading.col) else {
            stats.unplaced += 1;
            continue;
        };
        stats.headings += 1;
        // (A Markdown heading's line, without its `#`s and id.)
        let range = contents(at).unwrap_or_else(|| {
            let end = line_end(at);
            let line = &src[at..end];
            let title_start = at + (line.len() - line.trim_start_matches(['#', ' ']).len());
            let title = &src[title_start..end];
            let title = title.split(" {#").next().unwrap_or(title);
            let title = title.trim_end().trim_end_matches(['#', ' ']);
            title_start..title_start + title.len()
        });
        let sym = spec_symbol(shortname, &heading.id);
        let pretty = format!("{} {}", shortname, heading.text);
        structured_defs.push((sym.clone(), "specSection", None, heading.id.clone()));
        heading_syms.insert(line_start(src, at), sym.clone());
        heading_syms.insert(at, sym.clone());
        defs.entry(&heading.id)
            .or_insert_with(|| ("section".to_string(), pretty.clone()));
        pending.push(Pending {
            range,
            def: true,
            sym,
            what: "section".to_string(),
            pretty,
        });
    }
    for link in &index.links {
        let Some(at) = locate(link.file.as_deref(), link.line, link.col) else {
            stats.unplaced += 1;
            continue;
        };
        let label = link.lt.clone().unwrap_or_else(|| link.text.clone());
        let link_type = link
            .link_type
            .clone()
            .unwrap_or_else(|| "anchor".to_string());
        let (sym, what, pretty) = match link.href.strip_prefix('#') {
            Some(id) => match defs.get(id) {
                Some((what, pretty)) => (spec_symbol(shortname, id), what.clone(), pretty.clone()),
                None => (
                    spec_symbol(shortname, id),
                    link_type,
                    format!("{} {}", shortname, label),
                ),
            },
            None => match config.url_spec(&link.href) {
                Some((spec, anchor)) => (
                    spec_symbol(spec.shortname(), &anchor),
                    link_type,
                    format!("{} {}", spec.shortname(), label),
                ),
                None => {
                    stats.external += 1;
                    continue;
                }
            },
        };
        stats.links += 1;
        let range = match link
            .syntax
            .as_deref()
            .and_then(|syntax| match_syntax(src, at, syntax))
        {
            Some(end) => at..end,
            _ if link.idl && src[at..].starts_with(&link.text) => at..at + link.text.len(),
            _ => contents(at).unwrap_or(at..at + 1),
        };
        pending.push(Pending {
            range,
            def: false,
            sym,
            what,
            pretty,
        });
    }

    // Contexts: the innermost algorithm, with the step, or else the section.
    dfn_starts.sort();
    let mut steps: Vec<(usize, &str)> = index
        .steps
        .iter()
        .filter_map(|step| {
            let at = locate(step.file.as_deref(), step.line, step.col)?;
            Some((at, step.path.as_str()))
        })
        .collect();
    steps.sort();
    let mut sections: Vec<(usize, InContext)> = contexts
        .iter()
        .filter(|c| c.end.is_none())
        .map(|c| {
            (
                c.start,
                InContext {
                    name: unescape_context_name(&c.name),
                    sym: heading_syms.get(&c.start).cloned().unwrap_or_default(),
                },
            )
        })
        .collect();
    sections.sort_by_key(|(start, _)| *start);
    let mut algorithms: Vec<(Range<usize>, InContext)> = contexts
        .iter()
        .filter_map(|c| {
            let end = c.end?;
            let first = dfn_starts.partition_point(|(start, _)| *start < c.start);
            let sym = dfn_starts
                .get(first)
                .filter(|(start, _)| *start < end)
                .map_or(String::new(), |(_, sym)| sym.clone());
            Some((
                c.start..end,
                InContext {
                    name: unescape_context_name(&c.name),
                    sym,
                },
            ))
        })
        .collect();
    algorithms.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
    let context_at = |pos: usize| -> InContext {
        if let Some((range, context)) = algorithms
            .iter()
            .filter(|(range, _)| range.contains(&pos))
            .last()
        {
            let i = steps.partition_point(|(start, _)| *start <= pos);
            return match i.checked_sub(1).map(|i| steps[i]) {
                Some((start, path)) if start >= range.start => InContext {
                    name: format!("{} step {}", context.name, path),
                    sym: context.sym.clone(),
                },
                _ => context.clone(),
            };
        }
        let i = sections.partition_point(|(start, _)| *start <= pos);
        i.checked_sub(1)
            .map_or_else(InContext::default, |i| sections[i].1.clone())
    };

    // Structured records: definitions' kinds (algorithms' are callable),
    // parents, and anchors' URLs.
    let algorithm_syms: std::collections::HashSet<&str> = algorithms
        .iter()
        .map(|(_, context)| context.sym.as_str())
        .filter(|sym| !sym.is_empty())
        .collect();
    let mut structured: HashMap<String, (&'static str, Option<String>, Option<String>)> =
        HashMap::new();
    for (sym, kind, dfn_for, id) in &structured_defs {
        let kind = if *kind == "specDfn" && algorithm_syms.contains(sym.as_str()) {
            "specAlgorithm"
        } else {
            kind
        };
        let parent = dfn_for
            .as_deref()
            .and_then(|f| resolver.parent(f, kind))
            .filter(|parent| parent != sym)
            .map(str::to_string);
        structured
            .entry(sym.clone())
            .or_insert((kind, parent, config.rendered_url(shortname, id)));
    }

    let mut per_file: Vec<Vec<Record>> = (0..files).map(|_| vec![]).collect();
    for p in pending {
        let Some((file, loc)) = place(&p.range) else {
            continue;
        };
        let context = context_at(p.range.start);
        let records = &mut per_file[file];
        spec_records(records, loc, p.def, &p.what, &p.pretty, &p.sym, &context);
        if p.def
            && let Some((kind, parent, rendered)) = structured.remove(&p.sym)
        {
            spec_structured(
                records,
                loc,
                &p.pretty,
                &p.sym,
                kind,
                parent.as_deref(),
                rendered.as_deref(),
            );
        }
    }
    for records in &mut per_file {
        sort_records(records);
    }
    (per_file, stats)
}

/// Where the autolink `syntax` (as Bikeshed gives it, which may have spaces
/// for the source's line breaks, and leaves out a link's own text, ex:
/// `[=tree=]` for `[=tree|trees=]`) ends if it's at `src[at..]`.
fn match_syntax(src: &str, at: usize, syntax: &str) -> Option<usize> {
    match_verbatim(src, at, syntax).or_else(|| match_delimiters(src, at, syntax))
}

/// Where the autolink ends by its delimiters (ex: `[=` and `=]`), within a
/// few lines.
fn match_delimiters(src: &str, at: usize, syntax: &str) -> Option<usize> {
    let chars: Vec<char> = syntax.chars().collect();
    if chars.len() < 3 {
        return None;
    }
    let open_len = if chars[1].is_alphanumeric() { 1 } else { 2 };
    let close_len = if chars[chars.len() - 2].is_alphanumeric() {
        1
    } else {
        2
    };
    let open: String = chars[..open_len].iter().collect();
    let close: String = chars[chars.len() - close_len..].iter().collect();
    let rest = src[at..].strip_prefix(open.as_str())?;
    let limit = rest.char_indices().nth(500).map_or(rest.len(), |(i, _)| i);
    let end = rest[..limit].find(close.as_str())?;
    Some(at + open.len() + end + close.len())
}

fn match_verbatim(src: &str, at: usize, syntax: &str) -> Option<usize> {
    let mut text = src[at..].char_indices().peekable();
    let mut wanted = syntax.chars().peekable();
    while let Some(c) = wanted.next() {
        if c.is_whitespace() {
            while wanted.peek().is_some_and(|c| c.is_whitespace()) {
                wanted.next();
            }
            if !text.peek().is_some_and(|(_, t)| t.is_whitespace()) {
                return None;
            }
            while text.peek().is_some_and(|(_, t)| t.is_whitespace()) {
                text.next();
            }
        } else if text.next().map(|(_, t)| t) != Some(c) {
            return None;
        }
    }
    Some(at + text.peek().map_or(src.len() - at, |&(i, _)| i))
}

/// The start of the line `at` is on.
fn line_start(src: &str, at: usize) -> usize {
    src[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(records: &[Record]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) => Some(format!(
                    "{}:{}-{} {} {} | {} ({})",
                    t.loc.lineno,
                    t.loc.col_start,
                    t.loc.col_end,
                    t.data.kind.to_ustr(),
                    t.data.sym,
                    t.data.context,
                    t.data.contextsym
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_match_syntax() {
        let src = "See [=validate and\n  extract|validating=] it.";
        assert_eq!(
            match_syntax(src, 4, "[=validate and extract|validating=]"),
            Some(41)
        );
        // (Bikeshed leaves out a link's own text.)
        assert_eq!(match_syntax(src, 4, "[=validate and extract=]"), Some(41));
        assert_eq!(match_syntax(src, 4, "{{validate}}"), None);
    }

    #[test]
    fn test_includes() {
        // (A spec whose sections are in Markdown files it includes, by paths
        // relative to it even in included files, ex: scheduling-apis.  Their
        // contexts are as in the built spec, ex: the source's reference after
        // the include is in the included file's section.)
        let files: HashMap<&str, &str> = HashMap::from([
            (
                "s/spec/index.bs",
                "<pre class=metadata>\nTitle: S\n</pre>\n\n<pre class=include>\npath: intro.md\n</pre>\n\nSee [=gizmo=].\n",
            ),
            (
                "s/spec/intro.md",
                "Intro {#intro}\n=====\n\nA <dfn>gizmo</dfn>.\n\n<pre class=\"include\">\n<!-- (Nested.) -->\npath: parts/wind.md\n</pre>\n",
            ),
            (
                "s/spec/parts/wind.md",
                "To <dfn>wind</dfn> a [=gizmo=]:\n\n1. Turn it.\n",
            ),
        ]);
        let read = |path: &str| files.get(path).map(|s| s.to_string());
        let index = parse_index(
            r##"{"bikeshed-index": 2,
            "dfns": [
              {"line": 4, "col": 3, "file": "intro.md", "id": "gizmo", "type": "dfn", "text": "gizmo"},
              {"line": 1, "col": 4, "file": "parts/wind.md", "id": "wind", "type": "dfn", "text": "wind"}
            ],
            "links": [
              {"line": 9, "col": 5, "href": "#gizmo", "type": "dfn", "lt": "gizmo", "syntax": "[=gizmo=]", "text": "gizmo"},
              {"line": 1, "col": 22, "file": "parts/wind.md", "href": "#gizmo", "type": "dfn", "lt": "gizmo", "syntax": "[=gizmo=]", "text": "gizmo"},
              {"line": 1, "col": 1, "file": "missing.md", "href": "#gizmo", "type": "dfn", "text": "gizmo"}
            ],
            "headings": [{"line": 1, "col": 1, "file": "intro.md", "id": "intro", "level": 2, "text": "Intro"}],
            "steps": [{"line": 3, "col": 1, "file": "parts/wind.md", "path": "1"}]
            }"##,
        )
        .unwrap();
        let (records, stats) = analyze_with_includes(
            "s/spec/index.bs",
            &read,
            &index,
            "s",
            &SpecsConfig::default(),
        )
        .unwrap();
        let summaries: Vec<(String, Vec<String>)> = records
            .iter()
            .map(|(path, records)| (path.clone(), summary(records)))
            .collect();
        assert_eq!(
            summaries,
            vec![
                (
                    "s/spec/index.bs".to_string(),
                    vec!["9:4-13 use SPEC_s_gizmo | Intro (SPEC_s_intro)".to_string()]
                ),
                (
                    "s/spec/intro.md".to_string(),
                    vec![
                        "1:0-5 def SPEC_s_intro | Intro (SPEC_s_intro)".to_string(),
                        "4:7-12 def SPEC_s_gizmo | Intro (SPEC_s_intro)".to_string(),
                    ]
                ),
                (
                    "s/spec/parts/wind.md".to_string(),
                    vec![
                        "1:8-12 def SPEC_s_wind | wind (SPEC_s_wind)".to_string(),
                        "1:21-30 use SPEC_s_gizmo | wind (SPEC_s_wind)".to_string(),
                    ]
                ),
            ]
        );
        assert_eq!((stats.dfns, stats.links, stats.unplaced), (2, 2, 1));
    }

    #[test]
    fn test_analysis() {
        // (A spec like tests/tests/files/specs/example.bs, and the index
        // Bikeshed gives it.)
        let src = "<pre class=metadata>\nTitle: X\n</pre>\n\nIntroduction {#intro}\n=====\n\nA <dfn>widget</dfn> has a <dfn for=widget>state</dfn>, see [[HTML]].\n\n# Widgets # {#widgets}\n\nTo <dfn>poke</dfn> a [=widget=]:\n\n1. Set its [=widget/state=].\n1. Return.\n\n<pre class=idl>\ninterface Gadget {};\n</pre>\n\nSee <a href=\"https://dom.spec.whatwg.org/#concept-node\">nodes</a> and <{img}>.\n";
        let index = parse_index(
            r##"{"bikeshed-index": 2,
            "dfns": [
              {"line": 8, "col": 3, "id": "widget", "type": "dfn", "for": [], "text": "widget", "idl": false},
              {"line": 8, "col": 27, "id": "widget-state", "type": "dfn", "for": ["widget"], "text": "state", "idl": false},
              {"line": 12, "col": 4, "id": "poke", "type": "dfn", "for": [], "text": "poke", "idl": false},
              {"line": 18, "col": 11, "id": "gadget", "type": "interface", "for": [], "text": "Gadget", "idl": true}
            ],
            "links": [
              {"line": 8, "col": 60, "href": "#biblio-html", "type": "biblio", "lt": "HTML", "syntax": "[[HTML]]", "text": "[HTML]", "idl": false},
              {"line": 12, "col": 22, "href": "#widget", "type": "dfn", "lt": "widget", "syntax": "[=widget=]", "text": "widget", "idl": false},
              {"line": 14, "col": 12, "href": "#widget-state", "type": "dfn", "lt": "state", "syntax": "[=widget/state=]", "text": "state", "idl": false},
              {"line": 21, "col": 5, "href": "https://dom.spec.whatwg.org/#concept-node", "type": null, "lt": null, "syntax": null, "text": "nodes", "idl": false},
              {"line": 21, "col": 70, "href": "https://html.spec.whatwg.org/multipage/embedded-content.html#the-img-element", "type": "element", "lt": "img", "syntax": "<{img}>", "text": "img", "idl": false}
            ],
            "headings": [
              {"line": 5, "col": 1, "id": "intro", "level": 2, "text": "Introduction"},
              {"line": 10, "col": 1, "id": "widgets", "level": 2, "text": "Widgets"}
            ],
            "algorithms": [],
            "steps": [{"line": 14, "col": 1, "path": "1"}, {"line": 15, "col": 1, "path": "2"}]
            }"##,
        )
        .unwrap();
        let config: SpecsConfig = toml::from_str(
            "[[spec]]\nname = \"dom\"\nrendered = \"https://dom.spec.whatwg.org/\"\n",
        )
        .unwrap();
        let (records, stats) = analyze(src, &index, "x", &config);
        assert_eq!(
            summary(&records),
            vec![
                "5:0-12 def SPEC_x_intro | Introduction (SPEC_x_intro)",
                "8:7-13 def SPEC_x_widget | Introduction (SPEC_x_intro)",
                "8:42-47 def SPEC_x_widget@2Dstate | Introduction (SPEC_x_intro)",
                "8:59-67 use SPEC_x_biblio@2Dhtml | Introduction (SPEC_x_intro)",
                "10:2-9 def SPEC_x_widgets | Widgets (SPEC_x_widgets)",
                "12:8-12 def SPEC_x_poke | poke (SPEC_x_poke)",
                "12:21-31 use SPEC_x_widget | poke (SPEC_x_poke)",
                "14:11-27 use SPEC_x_widget@2Dstate | poke step 1 (SPEC_x_poke)",
                "18:10-16 def SPEC_x_gadget | Widgets (SPEC_x_widgets)",
                "21:56-61 use SPEC_dom_concept@2Dnode | Widgets (SPEC_x_widgets)",
            ]
        );
        assert_eq!(
            (stats.dfns, stats.headings, stats.links, stats.external),
            (4, 2, 4, 1)
        );
        let structured: Vec<String> = records
            .iter()
            .filter_map(|record| match record {
                Record::Structured(_, value) => Some(format!(
                    "{} {} {}",
                    value["sym"].as_str().unwrap(),
                    value["kind"].as_str().unwrap(),
                    value["parentsym"].as_str().unwrap_or("-"),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            structured,
            vec![
                "SPEC_x_intro specSection -",
                "SPEC_x_widget specDfn -",
                "SPEC_x_widget@2Dstate specDfn SPEC_x_widget",
                "SPEC_x_widgets specSection -",
                "SPEC_x_poke specAlgorithm -",
                "SPEC_x_gadget class -",
            ]
        );
    }
}
