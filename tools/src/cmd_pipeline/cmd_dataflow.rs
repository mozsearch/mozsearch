use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;
use clap::{Args, ValueEnum};
use ustr::Ustr;

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    dataflow::{
        frontend::FileIr,
        ir::{EventKind, FuncIr, Step},
        oracle::Oracle,
        render::{DotOptions, render_dot, render_text},
        slice::{
            ChangedLines, FileChange, Slicing, render_slicing_dot, render_slicing_text, slice,
        },
        subway::subway_map,
        summary::{IpdlSlots, Summary, field_edges, summarize_in},
        taint::{TaintSpec, audit_slicing},
        trace::{Dir, TraceOpts, Tracer, find_place},
    },
    file_format::local_symbols::parameter_symbol,
};

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum DataflowDirection {
    Forward,
    Backward,
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum DataflowFormat {
    Text,
    Dot,
    /// The DOT rendered by graphviz, with nodes linking to their source
    /// lines (for the web).
    Svg,
    Json,
    /// A slicing's clusters as a subway map (see `dataflow::subway`), as
    /// JSON for static/js/subway.js (for the web).
    Subway,
}

/// Naive dataflow analysis of a source file's functions (see `dataflow`):
/// what the frontend understood of them, or traces of values through them.
#[derive(Debug, Args)]
pub struct Dataflow {
    /// Tree-relative source file path.
    #[clap(value_parser)]
    file: Option<String>,

    /// The file and line, as `<path>#<line>`, instead of `file` and
    /// `--line` (as query terms give them).
    #[clap(long)]
    at: Option<String>,

    /// The function (by symbol, pretty name, or a suffix of its pretty name)
    /// to analyze, else all of the file's (or the one containing `--line`).
    #[clap(long)]
    function: Option<String>,

    /// The function containing this line, and where traces start.
    #[clap(long)]
    line: Option<u32>,

    /// Follow this symbol's value (a local, parameter, field or global) from
    /// `--line`, in the direction `--direction` says, instead of a place
    /// named by `--forward`/`--backward` (as a context menu does).
    #[clap(long)]
    symbol: Option<String>,

    #[clap(long, value_enum, default_value_t = DataflowDirection::Forward)]
    direction: DataflowDirection,

    /// Follow where this place's value goes (by its name as the IR shows it,
    /// ex: "aRetVal", "*aRetVal", "this->mName").
    #[clap(long)]
    forward: Option<String>,

    /// Follow where this place's value comes from ("return" for the
    /// function's returns).
    #[clap(long)]
    backward: Option<String>,

    #[clap(long, value_enum, default_value_t = DataflowFormat::Text)]
    format: DataflowFormat,

    /// How many calls, returns, fields and bindings to cross at most.
    #[clap(long, default_value_t = 6)]
    max_hops: usize,

    /// How many callers, implementations, or uses of a field to follow from
    /// each.
    #[clap(long, default_value_t = 8)]
    fanout: usize,

    #[clap(long, default_value_t = 600)]
    max_items: usize,

    /// Consider all of a function's events, not just those after (forwards)
    /// or before (backwards) where the value is.
    #[clap(long)]
    unordered: bool,

    /// Enter accessors' calls (getters, setters and trivial methods), rather
    /// than stepping over them by their precomputed summaries (see
    /// `dataflow::trace`'s `accessor_summary`).
    #[clap(long)]
    no_summaries: bool,

    /// Slice the changed lines in this (local) JSON file (see
    /// `dataflow::slice::ChangedLines`) into clusters, instead (`file` is
    /// ignored).
    #[clap(long)]
    slice: Option<String>,

    /// Slice changed lines into clusters, instead, given as
    /// `<path>:<lines>[;<path>:<lines>...]`, with lines like `10-12,20`.
    #[clap(long)]
    changed: Option<String>,

    /// Slice the lines a revision changes into clusters, instead: a commit's
    /// (from its first parent) or `<a>..<b>`'s, in the coordinates of the
    /// files the server has the analysis of (the indexed revision's, or a
    /// try push's head's; see `rev_lines`).
    #[clap(long)]
    slice_rev: Option<String>,

    /// Show only this cluster of a slicing (1 for the biggest).
    #[clap(long)]
    slice_cluster: Option<usize>,

    /// Taint values of these (comma-separated) types (by name, qualified or
    /// not) from content processes in subway maps, besides those
    /// ontology-mapping.toml labels "taint:content" (see `dataflow::taint`).
    #[clap(long)]
    taint_type: Option<String>,

    /// Take calls of these (comma-separated) functions (by pretty name, or
    /// its last components) as validating all of their arguments, besides
    /// those ontology-mapping.toml labels "detaint:N".
    #[clap(long)]
    detaint_by: Option<String>,

    /// Taint the parameters of tainted types of these (comma-separated)
    /// functions (by pretty name, or its last components), besides the
    /// `Recv` methods of messages from content processes.
    #[clap(long)]
    taint_from: Option<String>,

    /// Audit a type's taint, instead: map the functions receiving its values
    /// from content processes (the `Recv` methods of messages with
    /// parameters of the type), with their validations (see
    /// `dataflow::taint::audit_slicing`); `--format text` lists their
    /// parameters' statuses.
    #[clap(long)]
    taint_audit: Option<String>,

    /// How many hops to trace values written by changes, for `--slice`.
    #[clap(long, default_value_t = 3)]
    slice_hops: usize,

    /// Use the oracle answers in this directory (see `dataflow::oracle`).
    #[clap(long)]
    oracle: Option<String>,

    /// Write questions for an oracle, as JSON lines, about the calls in the
    /// files listed in this (local) file that the analysis can't see into,
    /// instead (`file` is ignored).
    #[clap(long)]
    questions_paths: Option<String>,

    /// Like `--questions-paths`, for these (comma-separated) tree paths.
    #[clap(long)]
    questions: Option<String>,

    /// How many questions to write at most.
    #[clap(long, default_value_t = 50)]
    max_questions: usize,

    /// Report how much the frontends understood of the files listed in this
    /// (local) file, one tree path per line, instead (`file` is ignored).
    #[clap(long)]
    stats_paths: Option<String>,

    /// Like `--stats-paths`, for these (comma-separated) tree paths.
    #[clap(long)]
    stats: Option<String>,

    /// Show the file's functions' dataflow summaries (see
    /// `dataflow::summary`; `--function` for one), instead, as
    /// `dataflow-summarize` precomputes them.
    #[clap(long)]
    summary: bool,

    /// Show the edges between fields (and between functions and other
    /// objects' fields) that the summaries of the functions in these
    /// (comma-separated) tree paths are, composed through their calls, as
    /// `dataflow-summarize` precomputes them for a whole tree, instead (`file`
    /// is ignored).
    #[clap(long)]
    field_edges: Option<String>,
}

/// Tree paths given inline (comma-separated) or in a local file (one per
/// line).
fn paths_arg(inline: &Option<String>, file: &Option<String>) -> Result<Option<Vec<String>>> {
    if let Some(inline) = inline {
        return Ok(Some(
            inline
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
        ));
    }
    let Some(file) = file else {
        return Ok(None);
    };
    Ok(Some(
        std::fs::read_to_string(file)
            .map_err(|e| data_error(format!("{}: {}", file, e)))?
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect(),
    ))
}

/// A taint audit's subway map (see `dataflow::subway::TaintInfo`) as text:
/// its sources' statuses, the unvalidated first, then its notes.
fn render_audit_text(map: &serde_json::Value) -> String {
    let cards = map["cards"].as_array().cloned().unwrap_or_default();
    let lines = map["lines"].as_array().cloned().unwrap_or_default();
    let mut rows: Vec<(usize, String)> = vec![];
    for source in map["taint"]["sources"].as_array().into_iter().flatten() {
        let status = source["status"].as_str().unwrap_or("?");
        let order = ["unvalidated", "partly", "validated", "unused"]
            .iter()
            .position(|s| *s == status)
            .unwrap_or(4);
        let card = &cards[source["card"].as_u64().unwrap_or(0) as usize];
        let line = &lines[source["line"].as_u64().unwrap_or(0) as usize];
        let first = card["stations"][0]["rows"][0]["line"].as_u64().unwrap_or(0);
        let validators: BTreeSet<&str> = map["taint"]["validations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["card"] == source["card"] && v["guards"].as_bool() == Some(true))
            .filter_map(|v| v["by"].as_str())
            .collect();
        rows.push((
            order,
            format!(
                "{:<12} {} {} ({}:{}): {} of {} stops tainted{}",
                status,
                card["func"].as_str().unwrap_or("?"),
                line["label"].as_str().unwrap_or("?"),
                card["path"].as_str().unwrap_or("?"),
                first,
                source["tainted_stops"],
                source["stops"],
                match validators.is_empty() {
                    true => String::new(),
                    false => format!(
                        ", validated by {}",
                        validators.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                }
            ),
        ));
    }
    rows.sort();
    let mut out: Vec<String> = rows.into_iter().map(|(_, row)| row).collect();
    for note in map["notes"].as_array().into_iter().flatten() {
        out.push(format!("Note: {}", note.as_str().unwrap_or("")));
    }
    out.join("\n") + "\n"
}

/// Parse `--changed`'s `<path>:<lines>[;...]`.
fn parse_changed(spec: &str) -> Result<ChangedLines> {
    let mut files = std::collections::BTreeMap::new();
    for part in spec.split(';').filter(|p| !p.trim().is_empty()) {
        let Some((path, lines)) = part.trim().rsplit_once(':') else {
            return Err(data_error(format!("Bad --changed part {:?}", part)));
        };
        let mut added = vec![];
        for range in lines.split(',').filter(|r| !r.is_empty()) {
            let (start, end) = match range.split_once('-') {
                Some((a, b)) => (a.parse::<u32>(), b.parse::<u32>()),
                None => (range.parse::<u32>(), range.parse::<u32>()),
            };
            let (Ok(start), Ok(end)) = (start, end) else {
                return Err(data_error(format!("Bad --changed lines {:?}", range)));
            };
            added.extend(start..=end);
        }
        // (A path given again adds lines.)
        files
            .entry(path.to_string())
            .or_insert_with(|| FileChange {
                added: vec![],
                removed: vec![],
                commits: Default::default(),
            })
            .added
            .extend(added);
    }
    Ok(ChangedLines {
        commits: vec![],
        files,
        notes: vec![],
        from: None,
        removed_lines: Default::default(),
    })
}

#[derive(Debug)]
pub struct DataflowCommand {
    pub args: Dataflow,
}

/// The MIME type of a subway map's JSON (see `dataflow::subway`), which the
/// query page shows with static/js/subway.js.
pub const SUBWAY_MIME: &str = "application/vnd.searchfox.subway+json";

/// DOT rendered as SVG by graphviz, to inline in a page.
fn svg_of(dot: String) -> Result<PipelineValues> {
    let svg = graphviz_rust::exec_dot(
        dot,
        vec![graphviz_rust::cmd::CommandArg::Format(
            graphviz_rust::cmd::Format::Svg,
        )],
    )
    .map_err(|e| data_error(format!("graphviz: {}", e)))?;
    let svg = String::from_utf8_lossy(&svg).to_string();
    // (No XML declaration or doctype, inlined.)
    let svg = match svg.find("<svg") {
        Some(start) => svg[start..].to_string(),
        None => svg,
    };
    text(svg, "image/svg+xml")
}

fn text(contents: String, mime: &str) -> Result<PipelineValues> {
    Ok(PipelineValues::TextFile(TextFile {
        mime_type: mime.to_string(),
        contents,
    }))
}

/// Counts of what the frontends understood (see `--stats-paths`).
#[derive(Default)]
struct Stats {
    counts: std::collections::BTreeMap<String, usize>,
}

impl Stats {
    fn add(&mut self, key: &str, n: usize) {
        *self.counts.entry(key.to_string()).or_default() += n;
    }

    fn add_file(&mut self, file: &FileIr) {
        use crate::dataflow::ir::{Atom, CallKind, EventKind, QuestionKind, Root, Val};
        let lang = format!("{:?}", file.lang);
        self.add(&format!("{}-files", lang), 1);
        for f in &file.funcs {
            self.add(&format!("{}-functions", lang), 1);
            if f.sym.starts_with('?') {
                self.add(&format!("{}-functions-without-records", lang), 1);
            }
            let unresolved_name = |vals: &[Val]| {
                vals.iter()
                    .filter(|v| matches!(&v.atom, Atom::Read(p) | Atom::Addr(p) if matches!(p.root, Root::Local(s) if s.starts_with("name:"))))
                    .count()
            };
            for e in &f.events {
                let kind = match &e.kind {
                    EventKind::Assign { place, vals, .. } => {
                        if matches!(place.root, Root::Local(s) if s.starts_with("name:")) {
                            self.add(&format!("{}-places-unresolved-names", lang), 1);
                        }
                        self.add(
                            &format!("{}-reads-unresolved-names", lang),
                            unresolved_name(vals),
                        );
                        "assign"
                    }
                    EventKind::Call(_) => "call",
                    EventKind::Return { vals } => {
                        self.add(
                            &format!("{}-reads-unresolved-names", lang),
                            unresolved_name(vals),
                        );
                        "return"
                    }
                    EventKind::Test { vals } => {
                        self.add(
                            &format!("{}-reads-unresolved-names", lang),
                            unresolved_name(vals),
                        );
                        "test"
                    }
                };
                self.add(&format!("{}-events-{}", lang, kind), 1);
                if e.macro_name.is_some() {
                    self.add(&format!("{}-events-from-macro-expansions", lang), 1);
                }
            }
            for c in &f.calls {
                self.add(&format!("{}-calls", lang), 1);
                match (c.kind, c.callee.is_empty()) {
                    (CallKind::Macro, _) => {
                        self.add(&format!("{}-calls-unexpanded-macros", lang), 1)
                    }
                    (_, true) => self.add(&format!("{}-calls-without-callee-symbols", lang), 1),
                    (_, false) => self.add(&format!("{}-calls-with-callee-symbols", lang), 1),
                }
                if c.callee.len() > 1 {
                    self.add(&format!("{}-calls-with-several-callees", lang), 1);
                }
            }
            for q in &f.questions {
                let kind = match q.kind {
                    QuestionKind::UnresolvedCall => "unresolved-call",
                    QuestionKind::Macro => "macro",
                    QuestionKind::Unparsed => "unparsed",
                };
                self.add(&format!("{}-questions-{}", lang, kind), 1);
            }
        }
    }

    fn render(&self) -> String {
        self.counts
            .iter()
            .map(|(k, v)| format!("{:>8} {}", v, k))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn data_error(message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::DataLayer,
        message,
    })
}

/// The functions' summaries, as `dataflow-summarize` makes them: with the
/// methods they override, and seeing through the code IPDL generates (see
/// `ipdl_slots_of`), which is returned too, for `field_edges`.
async fn summaries_of(
    server: &(dyn AbstractServer + Send + Sync),
    funcs: &[&FuncIr],
) -> (Vec<Summary>, IpdlSlots) {
    let syms: Vec<Ustr> = funcs.iter().map(|f| f.sym).collect();
    server.prefetch_crossref(&syms).await;
    let mut overrides: HashMap<Ustr, Vec<Ustr>> = HashMap::new();
    for f in funcs {
        if let Some(meta) = server
            .crossref_lookup(&f.sym)
            .await
            .ok()
            .flatten()
            .and_then(|data| data.meta)
        {
            overrides.insert(f.sym, meta.overrides.iter().map(|o| o.sym).collect());
        }
    }
    let ipdl = ipdl_slots_of(server, funcs, &overrides).await;
    let summaries = funcs
        .iter()
        .map(|f| {
            let overrides = overrides.get(&f.sym).cloned().unwrap_or_default();
            summarize_in(f, &ipdl, overrides)
        })
        .collect();
    (summaries, ipdl)
}

/// What the code IPDL generates is (see `summary::IpdlSlots`), as far as the
/// functions use it, from crossref's entries of the IPDL owners of their
/// callees, of the fields they assign to, and of the methods they override
/// (rather than the whole tree's, which `dataflow-summarize` reads from the
/// IPDL files' analysis): the structs' fields and constructors, and the
/// messages' parameters (by their `Send` methods' and `Recv` methods'
/// parameters' symbols; see `parameter_symbol`).
async fn ipdl_slots_of(
    server: &(dyn AbstractServer + Send + Sync),
    funcs: &[&FuncIr],
    overrides: &HashMap<Ustr, Vec<Ustr>>,
) -> IpdlSlots {
    let meta_of = async |sym: &Ustr| {
        server
            .crossref_lookup(sym)
            .await
            .ok()
            .flatten()
            .and_then(|data| data.meta)
    };
    let mut syms: BTreeSet<Ustr> = BTreeSet::new();
    for f in funcs {
        syms.extend(f.calls.iter().flat_map(|call| call.callee.iter().copied()));
        for event in &f.events {
            if let EventKind::Assign { place, .. } = &event.kind {
                syms.extend(place.path.iter().filter_map(|step| match step {
                    Step::Field(sym) => Some(*sym),
                    _ => None,
                }));
            }
        }
        syms.extend(overrides.get(&f.sym).into_iter().flatten().copied());
    }
    syms.retain(|sym| sym.starts_with("_Z"));
    let syms: Vec<Ustr> = syms.into_iter().collect();
    server.prefetch_crossref(&syms).await;
    let mut owners: BTreeSet<Ustr> = BTreeSet::new();
    for sym in &syms {
        let Some(owner) = meta_of(sym).await.and_then(|meta| meta.slot_owner) else {
            continue;
        };
        if owner.sym.starts_with("IPDL_") {
            owners.insert(owner.sym);
        } else if owner.sym.starts_with("IPC_") {
            // (A message's `Send` or `Recv` method: its parameters' owners,
            // and the message's, for its slots.)
            owners.insert(owner.sym);
            for i in 0..32 {
                let param = ustr::ustr(&parameter_symbol(sym, i));
                match meta_of(&param).await.and_then(|meta| meta.slot_owner) {
                    Some(param_owner) if param_owner.sym.starts_with("IPC_") => {
                        owners.insert(param_owner.sym);
                    }
                    _ => break,
                }
            }
        }
    }
    let owners: Vec<Ustr> = owners.into_iter().collect();
    server.prefetch_crossref(&owners).await;
    let mut records = vec![];
    for owner in &owners {
        if let Some(meta) = meta_of(owner).await {
            records.push(serde_json::to_value(&meta).unwrap());
        }
    }
    IpdlSlots::from_structured(&records)
}

#[async_trait]
impl PipelineCommand for DataflowCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let (file_path, line) = match &self.args.at {
            Some(at) => {
                let Some((path, line)) = at.rsplit_once('#') else {
                    return Err(data_error(format!("--at {:?} isn't <path>#<line>", at)));
                };
                let line: u32 = line
                    .parse()
                    .map_err(|_| data_error(format!("--at {:?} isn't <path>#<line>", at)))?;
                (path.to_string(), Some(line))
            }
            None => (self.args.file.clone().unwrap_or_default(), self.args.line),
        };
        // (Servers without raw sources, ex: remote ones, can't analyze; checks
        // skip commands their servers don't support.)
        if let Err(ServerError::Unsupported) = server.fetch_raw_source(&file_path).await {
            return Err(ServerError::Unsupported);
        }
        let opts = TraceOpts {
            max_items: self.args.max_items,
            max_hops: self.args.max_hops,
            fanout: self.args.fanout,
            ordered: !self.args.unordered,
            summaries: !self.args.no_summaries,
        };
        let mut tracer = Tracer::new(server, opts);
        if let Some(dir) = &self.args.oracle {
            let oracle = Oracle::load(std::path::Path::new(dir)).map_err(data_error)?;
            tracer.oracle = Some(std::sync::Arc::new(oracle));
        }
        if let Some(paths) = paths_arg(&self.args.questions, &self.args.questions_paths)? {
            let questions = tracer.questions(&paths, self.args.max_questions).await;
            return text(
                questions
                    .iter()
                    .map(|q| serde_json::to_string(q).unwrap())
                    .collect::<Vec<_>>()
                    .join("\n"),
                "application/jsonl",
            );
        }
        if let Some(spec) = &self.args.slice_rev {
            // (Cached, by everything slicing depends on; see `step_cache`.)
            let args = format!(
                "{} hops {} {:?} oracle {:?}",
                spec, self.args.slice_hops, tracer.opts, self.args.oracle
            );
            let value =
                crate::step_cache::cached(server.cache_identity(), "slice-rev", &args, || async {
                    let changes = server.rev_changed_lines(spec)?;
                    // (What it removes, in the old side's analysis, if it's
                    // here; see `AbstractServer::old_side`.)
                    let old_server = changes.from.as_deref().and_then(|rev| server.old_side(rev));
                    let mut old_tracer = old_server.as_deref().map(|old| {
                        let mut old_tracer = Tracer::new(old, tracer.opts.clone());
                        old_tracer.oracle = tracer.oracle.clone();
                        old_tracer
                    });
                    let slicing = slice(
                        &mut tracer,
                        &changes,
                        self.args.slice_hops,
                        old_tracer.as_mut(),
                    )
                    .await;
                    Ok::<_, ServerError>(serde_json::to_value(&slicing).unwrap())
                })
                .await?;
            let slicing: Slicing = serde_json::from_value(value)
                .map_err(|e| data_error(format!("A cached slicing: {}", e)))?;
            return self
                .render_slicing(server, &mut tracer, slicing, Some(args))
                .await;
        }
        if let Some(ty) = &self.args.taint_audit {
            // (Cached, as slicings are; see `step_cache`.)
            let args = format!("taint-audit {} {:?}", ty, tracer.opts);
            let value = crate::step_cache::cached(
                server.cache_identity(),
                "taint-audit",
                &args,
                || async {
                    let slicing = audit_slicing(&mut tracer, ty).await;
                    Ok::<_, ServerError>(serde_json::to_value(&slicing).unwrap())
                },
            )
            .await?;
            let slicing: Slicing = serde_json::from_value(value)
                .map_err(|e| data_error(format!("A cached audit: {}", e)))?;
            return self
                .render_slicing(server, &mut tracer, slicing, Some(args))
                .await;
        }
        let changes = match (&self.args.changed, &self.args.slice) {
            (Some(spec), _) => Some(parse_changed(spec)?),
            (None, Some(slice_file)) => {
                let json = std::fs::read_to_string(slice_file)
                    .map_err(|e| data_error(format!("{}: {}", slice_file, e)))?;
                Some(
                    serde_json::from_str::<ChangedLines>(&json)
                        .map_err(|e| data_error(format!("{}: {}", slice_file, e)))?,
                )
            }
            (None, None) => None,
        };
        if let Some(changes) = changes {
            let slicing = slice(&mut tracer, &changes, self.args.slice_hops, None).await;
            return self
                .render_slicing(server, &mut tracer, slicing, None)
                .await;
        }
        if let Some(paths) = paths_arg(&self.args.field_edges, &None)? {
            let mut files = vec![];
            for path in &paths {
                if let Some(file) = tracer.file(path).await {
                    files.push(file);
                }
            }
            let funcs: Vec<&FuncIr> = files
                .iter()
                .flat_map(|file| file.funcs.iter().filter(|f| f.has_body))
                .collect();
            let (mut summaries, ipdl) = summaries_of(server, &funcs).await;
            summaries.sort_by(|a, b| a.sym.cmp(&b.sym));
            summaries.dedup_by(|a, b| a.sym == b.sym);
            let edges = field_edges(&summaries, &ipdl);
            return Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::to_value(&edges).unwrap(),
            }));
        }
        if let Some(paths) = paths_arg(&self.args.stats, &self.args.stats_paths)? {
            let mut stats = Stats::default();
            for path in &paths {
                let Some(file) = tracer.file(path).await else {
                    stats.add("files-failed", 1);
                    continue;
                };
                stats.add_file(&file);
                // (Whether calls' callees have definitions we could enter.)
                let mut callees: Vec<ustr::Ustr> = file
                    .funcs
                    .iter()
                    .flat_map(|f| f.calls.iter())
                    .filter_map(|c| c.callee.first().copied())
                    .collect();
                callees.sort();
                callees.dedup();
                for callee in callees {
                    let prefix = file.lang;
                    match server.crossref_lookup(&callee).await.ok().flatten() {
                        Some(xref) => {
                            let defs = xref.definitions.as_ref().is_some_and(|d| !d.is_empty());
                            let overrides = xref
                                .meta
                                .as_ref()
                                .is_some_and(|m| !m.overridden_by_syms.is_empty());
                            let slot = xref.meta.as_ref().is_some_and(|m| m.slot_owner.is_some());
                            stats.add(&format!("{:?}-callees", prefix), 1);
                            if defs {
                                stats.add(&format!("{:?}-callees-with-defs", prefix), 1);
                            } else if overrides {
                                stats.add(&format!("{:?}-callees-with-overrides", prefix), 1);
                            } else if slot {
                                stats.add(&format!("{:?}-callees-binding-slots", prefix), 1);
                            } else {
                                stats.add(&format!("{:?}-callees-without-defs", prefix), 1);
                            }
                        }
                        None => stats.add(&format!("{:?}-callees-not-in-crossref", prefix), 1),
                    }
                }
            }
            return text(stats.render(), "text/plain");
        }
        if let Some(sym) = &self.args.symbol {
            let dir = match self.args.direction {
                DataflowDirection::Forward => Dir::Forward,
                DataflowDirection::Backward => Dir::Backward,
            };
            let Some(line) = line else {
                return Err(data_error("--symbol needs --line".to_string()));
            };
            tracer
                .seed_symbol(&file_path, line, ustr::ustr(sym), dir)
                .await
                .map_err(data_error)?;
            tracer.run(dir).await;
            return self.render_trace(server, &tracer);
        }
        let Some(file) = tracer.file(&file_path).await else {
            return Err(data_error(tracer.graph.notes.join("\n")));
        };
        let funcs: Vec<usize> = (0..file.funcs.len())
            .filter(|&i| {
                let f = &file.funcs[i];
                match (&self.args.function, line) {
                    (Some(name), _) => {
                        f.sym == name.as_str()
                            || f.pretty == *name
                            || f.pretty.ends_with(&format!("::{}", name))
                    }
                    (None, Some(line)) => f.line <= line && line <= f.end_line,
                    (None, None) => true,
                }
            })
            .collect();
        if self.args.summary {
            let funcs: Vec<&FuncIr> = funcs
                .iter()
                .map(|&i| &file.funcs[i])
                .filter(|f| f.has_body)
                .collect();
            let (summaries, _) = summaries_of(server, &funcs).await;
            return Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::to_value(&summaries).unwrap(),
            }));
        }
        let (dir, place_name) = match (&self.args.forward, &self.args.backward) {
            (Some(p), _) => (Dir::Forward, p.clone()),
            (None, Some(p)) => (Dir::Backward, p.clone()),
            (None, None) => {
                // What the frontend understood.
                let funcs: Vec<_> = funcs.iter().map(|&i| &file.funcs[i]).collect();
                return match self.args.format {
                    DataflowFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                        value: serde_json::to_value(&funcs).unwrap(),
                    })),
                    _ => text(
                        funcs
                            .iter()
                            .map(|f| f.to_string())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        "text/plain",
                    ),
                };
            }
        };
        // (The innermost function, for a line.)
        let Some(&func) = funcs
            .iter()
            .min_by_key(|&&i| file.funcs[i].end_line - file.funcs[i].line)
        else {
            return Err(data_error("No such function".to_string()));
        };
        if dir == Dir::Backward && place_name == "return" {
            tracer.seed_returns(file.clone(), func);
        } else {
            let Some(place) = find_place(&file.funcs[func], &place_name) else {
                return Err(data_error(format!(
                    "No place {:?} in {}",
                    place_name, file.funcs[func].pretty
                )));
            };
            tracer.seed(file.clone(), func, place, line, dir);
        }
        tracer.run(dir).await;
        self.render_trace(server, &tracer)
    }
}

impl DataflowCommand {
    /// A slicing (or its `--slice-cluster`) in the format asked for.  (Its
    /// subway map is cached by `cache_args`, the slicing's step cache
    /// arguments, if it's cached; see `step_cache`.)
    async fn render_slicing(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        tracer: &mut Tracer<'_>,
        mut slicing: Slicing,
        cache_args: Option<String>,
    ) -> Result<PipelineValues> {
        if let Some(n) = self.args.slice_cluster {
            slicing.only_cluster(n).map_err(data_error)?;
        }
        let audit_text =
            self.args.taint_audit.is_some() && self.args.format == DataflowFormat::Text;
        if audit_text || self.args.format == DataflowFormat::Subway {
            let first = self.args.slice_cluster.unwrap_or(1);
            let identity = cache_args.as_ref().and_then(|_| server.cache_identity());
            let list = |arg: &Option<String>| -> Vec<String> {
                arg.iter()
                    .flat_map(|s| s.split(','))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            let mut taint = TaintSpec {
                types: list(&self.args.taint_type),
                validators: list(&self.args.detaint_by),
                sources: list(&self.args.taint_from),
            };
            taint.types.extend(self.args.taint_audit.iter().cloned());
            let args = format!(
                "{:?} cluster {:?} taint {:?}",
                cache_args, self.args.slice_cluster, taint
            );
            let value = crate::step_cache::cached(identity, "subway", &args, || async {
                let map = subway_map(tracer, &slicing, first, &taint).await;
                Ok::<_, ServerError>(serde_json::to_value(&map).unwrap())
            })
            .await?;
            if audit_text {
                return text(render_audit_text(&value), "text/plain");
            }
            // (Its "<"s escaped, as it's inlined in a page's script.)
            return text(value.to_string().replace('<', "\\u003c"), SUBWAY_MIME);
        }
        match self.args.format {
            DataflowFormat::Text => text(render_slicing_text(&slicing), "text/plain"),
            DataflowFormat::Dot => text(
                render_slicing_dot(&slicing, &DotOptions::default()),
                "text/vnd.graphviz",
            ),
            DataflowFormat::Svg => svg_of(render_slicing_dot(
                &slicing,
                &DotOptions {
                    link_tree: Some(server.tree_info()?.name),
                    link_rev: server.try_delta_meta().map(|meta| meta.head),
                    title: false,
                },
            )),
            DataflowFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::to_value(&slicing).unwrap(),
            })),
            DataflowFormat::Subway => unreachable!(),
        }
    }

    fn render_trace(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        tracer: &Tracer<'_>,
    ) -> Result<PipelineValues> {
        match self.args.format {
            DataflowFormat::Text => text(render_text(&tracer.graph), "text/plain"),
            DataflowFormat::Dot => text(
                render_dot(&tracer.graph, &DotOptions::default()),
                "text/vnd.graphviz",
            ),
            DataflowFormat::Svg => {
                let tree = server.tree_info()?.name;
                let dot = render_dot(
                    &tracer.graph,
                    &DotOptions {
                        link_tree: Some(tree),
                        link_rev: server.try_delta_meta().map(|meta| meta.head),
                        title: true,
                    },
                );
                svg_of(dot)
            }
            DataflowFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::to_value(&tracer.graph).unwrap(),
            })),
            DataflowFormat::Subway => Err(data_error(
                "Subway maps are of slicings (slice-rev)".to_string(),
            )),
        }
    }
}
