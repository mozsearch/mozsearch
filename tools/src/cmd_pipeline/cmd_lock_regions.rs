use async_trait::async_trait;
use clap::{Args, ValueEnum};

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::dataflow::ir::{FuncIr, LockKind};
use crate::dataflow::locks::{HeldRegion, held_regions};
use crate::dataflow::trace::{TraceOpts, Tracer};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum LockRegionsFormat {
    Text,
    Json,
}

/// Where a source file's functions hold mutexes (see `dataflow::locks`):
/// their scoped lock guards' and proof parameters' regions, and what runs in
/// them with the mutexes held (calls, fields accessed, other locks acquired).
#[derive(Debug, Args)]
pub struct LockRegions {
    /// Tree-relative source file path.
    #[clap(value_parser)]
    file: String,

    /// Only the function whose symbol is this, or whose pretty name ends
    /// with it.
    #[clap(long)]
    function: Option<String>,

    #[clap(long, value_enum, default_value = "text")]
    format: LockRegionsFormat,
}

#[derive(Debug)]
pub struct LockRegionsCommand {
    pub args: LockRegions,
}

/// A held region as text (for checks and debugging).
fn region_text(func: &FuncIr, region: &HeldRegion) -> String {
    let lines = |lines: &[u32]| {
        lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let ranges = |ranges: &[(u32, u32)]| {
        ranges
            .iter()
            .map(|(first, last)| format!("{}-{}", first, last))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = format!(
        "  {} {} {}{} lines {}-{}\n",
        match region.kind {
            LockKind::Proof => "proof",
            _ => "held",
        },
        region.guard,
        match region.param {
            Some(param) => format!(
                "param {}",
                func.params
                    .get(param)
                    .map_or("?".to_string(), |p| p.name.clone())
            ),
            None => region.mutex_text.clone(),
        },
        match &region.mutex {
            Some(sym) => format!(" [{}]", sym),
            None => String::new(),
        },
        region.line,
        region.end_line
    );
    out.push_str(&format!("    held lines: {}\n", lines(&region.held_lines)));
    if !region.released.is_empty() {
        out.push_str(&format!("    released: {}\n", ranges(&region.released)));
    }
    if !region.lambdas.is_empty() {
        out.push_str(&format!("    lambdas: {}\n", ranges(&region.lambdas)));
    }
    if !region.called_lambdas.is_empty() {
        out.push_str(&format!(
            "    lambdas made outside it, called in it: {}\n",
            ranges(&region.called_lambdas)
        ));
    }
    for call in &region.calls {
        out.push_str(&format!(
            "    call {} {}{}{}\n",
            call.line,
            call.callee_pretty,
            match call.kind {
                crate::dataflow::ir::CallKind::Call => "",
                crate::dataflow::ir::CallKind::Construct => " (construct)",
                crate::dataflow::ir::CallKind::Macro => " (macro)",
            },
            match call.proof_args.as_slice() {
                [] => String::new(),
                args => format!(
                    ", passing the proof as argument {}",
                    args.iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        ));
    }
    for field in &region.fields {
        let mut how = vec![];
        if !field.reads.is_empty() {
            how.push(format!("read {}", lines(&field.reads)));
        }
        if !field.writes.is_empty() {
            how.push(format!("written {}", lines(&field.writes)));
        }
        if !field.calls.is_empty() {
            how.push(format!("methods called {}", lines(&field.calls)));
        }
        out.push_str(&format!(
            "    field {} [{}]: {}\n",
            field.name,
            field.sym,
            how.join("; ")
        ));
    }
    for lock in &region.locks {
        out.push_str(&format!(
            "    acquires {} {}{} at {}\n",
            lock.guard,
            lock.mutex_text,
            match &lock.mutex {
                Some(sym) => format!(" [{}]", sym),
                None => String::new(),
            },
            lock.line
        ));
    }
    out
}

#[async_trait]
impl PipelineCommand for LockRegionsCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let mut tracer = Tracer::new(server, TraceOpts::default());
        let file = tracer.file(&self.args.file).await.ok_or_else(|| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!("No dataflow IR for {}", self.args.file),
            })
        })?;
        let wanted = |func: &FuncIr| match &self.args.function {
            Some(name) => func.sym == name.as_str() || func.pretty.ends_with(name.as_str()),
            None => true,
        };
        let funcs: Vec<(&FuncIr, Vec<HeldRegion>)> = file
            .funcs
            .iter()
            .filter(|func| wanted(func))
            .map(|func| (func, held_regions(func)))
            .filter(|(_, regions)| !regions.is_empty())
            .collect();
        match self.args.format {
            LockRegionsFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::json!(
                    funcs
                        .iter()
                        .map(|(func, regions)| serde_json::json!({
                            "sym": func.sym,
                            "pretty": func.pretty,
                            "path": func.path,
                            "line": func.line,
                            "end_line": func.end_line,
                            "regions": regions,
                        }))
                        .collect::<Vec<_>>()
                ),
            })),
            LockRegionsFormat::Text => {
                let mut contents = String::new();
                for (func, regions) in &funcs {
                    contents.push_str(&format!(
                        "{} ({}:{}-{})\n",
                        func.pretty, func.path, func.line, func.end_line
                    ));
                    for region in regions {
                        contents.push_str(&region_text(func, region));
                    }
                }
                Ok(PipelineValues::TextFile(TextFile {
                    mime_type: "text/plain".to_string(),
                    contents,
                    transient: false,
                    facets: None,
                }))
            }
        }
    }
}
