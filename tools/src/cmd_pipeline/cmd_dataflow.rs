use async_trait::async_trait;
use clap::{Args, ValueEnum};

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    dataflow::{
        frontend::FileIr,
        oracle::Oracle,
        render::{DotOptions, render_dot, render_text},
        slice::{ChangedLines, FileChange, render_slicing_dot, render_slicing_text, slice},
        trace::{Dir, TraceOpts, Tracer, find_place},
    },
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

    /// Slice the changed lines in this (local) JSON file (see
    /// `dataflow::slice::ChangedLines`) into clusters, instead (`file` is
    /// ignored).
    #[clap(long)]
    slice: Option<String>,

    /// Slice changed lines into clusters, instead, given as
    /// `<path>:<lines>[;<path>:<lines>...]`, with lines like `10-12,20`.
    #[clap(long)]
    changed: Option<String>,

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
        files.insert(
            path.to_string(),
            FileChange {
                added,
                removed: vec![],
            },
        );
    }
    Ok(ChangedLines {
        commits: vec![],
        files,
    })
}

#[derive(Debug)]
pub struct DataflowCommand {
    pub args: Dataflow,
}

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
            let slicing = slice(&mut tracer, &changes, self.args.slice_hops).await;
            return match self.args.format {
                DataflowFormat::Text => text(render_slicing_text(&slicing), "text/plain"),
                DataflowFormat::Dot => text(render_slicing_dot(&slicing), "text/vnd.graphviz"),
                DataflowFormat::Svg => svg_of(render_slicing_dot(&slicing)),
                DataflowFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                    value: serde_json::to_value(&slicing).unwrap(),
                })),
            };
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
                        title: true,
                    },
                );
                svg_of(dot)
            }
            DataflowFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::to_value(&tracer.graph).unwrap(),
            })),
        }
    }
}
