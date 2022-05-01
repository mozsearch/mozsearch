use async_trait::async_trait;
use clap::arg_enum;
use serde::Serialize;
use serde_json::{to_string_pretty, Value};
use std::{collections::{HashSet}, fmt::Debug};
use structopt::StructOpt;
use tracing::{trace, trace_span};

use crate::abstract_server::{TextMatches, TextMatchesByFile};
pub use crate::abstract_server::{AbstractServer, Result};

use super::symbol_graph::SymbolGraphCollection;

arg_enum! {
  #[derive(Debug, PartialEq)]
  pub enum RecordType {
      Source,
      Target,
      Structured,
  }
}

#[derive(Debug, StructOpt)]
pub struct SymbolicQueryOpts {
    /// Exact symbol match
    #[structopt(short)]
    pub symbol: Option<String>,

    /// Exact identifier match
    #[structopt(short)]
    pub identifier: Option<String>,
}

/// The input and output of each pipeline segment
#[derive(Serialize)]
pub enum PipelineValues {
    IdentifierList(IdentifierList),
    SymbolList(SymbolList),
    SymbolCrossrefInfoList(SymbolCrossrefInfoList),
    SymbolGraphCollection(SymbolGraphCollection),
    JsonValue(JsonValue),
    JsonRecords(JsonRecords),
    TextMatches(TextMatches),
    HtmlExcerpts(HtmlExcerpts),
    StructuredResultsBundle(StructuredResultsBundle),
    FlattenedResultsBundle(FlattenedResultsBundle),
    TextFile(TextFile),
    Void,
}

/// A list of (searchfox) identifiers.
#[derive(Serialize)]
pub struct IdentifierList {
    pub identifiers: Vec<String>,
}

/// A list of (searchfox) symbols.
#[derive(Serialize)]
pub struct SymbolList {
    pub symbols: Vec<String>,
    /// If present, these correspond to the identifiers that give us the
    /// symbols.  This is used in cases where an non-exact_match identifier
    /// search is performed and so we may not actually know what the identifiers
    /// actually were.
    pub from_identifiers: Option<Vec<String>>,
}

/// A symbol and its cross-reference information.
#[derive(Serialize)]
pub struct SymbolCrossrefInfo {
    pub symbol: String,
    pub crossref_info: Value,
}

/// A list of `SymbolCrossrefInfo`s.
#[derive(Serialize)]
pub struct SymbolCrossrefInfoList {
    pub symbol_crossref_infos: Vec<SymbolCrossrefInfo>,
}

/// A mixture of file names (paths), SymbolCrossrefInfo instances, and text
/// matches by file.  This gets compiled into a `FlattenedResultsBundle` by the
/// `compile-results` pipeline command.
#[derive(Serialize)]
pub struct StructuredResultsBundle {
    pub file_names: Vec<String>,
    pub symbol_crossref_infos: Vec<SymbolCrossrefInfo>,
    pub text_matches_by_file: Vec<TextMatchesByFile>,
}

/// router.py-style mozsearch compiled results that has top-level path-kind
/// (normal/test/generated) result clusters, where each cluster has file names /
/// paths and line hits grouped by symbol-with-kind and by file name/path
/// beneath that.
///
/// Line results can contain raw source text or HTML-rendered excerpts if
/// augmented by the `show-html` command.
#[derive(Serialize)]
pub struct FlattenedResultsBundle {
    pub path_kind_results: Vec<FlattenedPathKindGroupResults>,
    pub content_type: String,
}

#[derive(Serialize)]
pub struct FlattenedPathKindGroupResults {
    pub path_kind: String,
    pub file_names: Vec<String>,
}

#[derive(Serialize)]
pub struct FlattenedKindGroupResults {
    pub pretty: String,
    pub symbols: Option<Vec<String>>,
    pub kind: String,
    pub by_file: Vec<FlattenedResultsByFile>,
}

#[derive(Serialize)]
pub struct FlattenedResultsByFile {
    pub file: String,
    pub line_spans: Vec<FlattenedLineSpan>,
}

/// Represents a range of lines in a file.
#[derive(Serialize)]
pub struct FlattenedLineSpan {
    pub line_range: (u32, u32),
    pub contents: String,
}

/// JSON records are raw analysis records from a single file (for now)
#[derive(Serialize)]
pub struct JsonRecordsByFile {
    pub file: String,
    pub records: Vec<Value>,
}

impl JsonRecordsByFile {
    /// Return the set of lines covered by the records in this structure.
    ///
    /// A HashSet is returned for ease of consumption even though it would
    /// almost certainly be more efficient to return a vec that the caller
    /// caller can consume in concert with a parallel traversal of (ex) the
    /// generated HTML for the given file.
    pub fn line_set(&self) -> HashSet<u32> {
        let mut line_set = HashSet::new();
        for value in &self.records {
            if let Some(loc) = value["loc"].as_str() {
                let lno = loc.split(":").next().unwrap_or("0").parse().unwrap_or(0);
                line_set.insert(lno);
            }
        }

        line_set
    }
}

/// A single JSON value, usually expected to be from a search query.
///
/// It might make sense to add a type-indicating value or origin of the JSON,
/// but for now this will only be from the query.
#[derive(Serialize)]
pub struct JsonValue {
    pub value: Value,
}

/// JSON Analysis Records grouped by (source) file.
#[derive(Serialize)]
pub struct JsonRecords {
    pub by_file: Vec<JsonRecordsByFile>,
}

#[derive(Serialize)]
pub struct HtmlExcerptsByFile {
    pub file: String,
    pub excerpts: Vec<String>,
}

#[derive(Serialize)]
pub struct HtmlExcerpts {
    pub by_file: Vec<HtmlExcerptsByFile>,
}

#[derive(Serialize)]
pub struct TextFile {
    pub mime_type: String,
    pub contents: String,
}

/// A command that takes a single input and produces a single output.  At the
/// start of the pipeline, the input may be ignored / expected to be void.
#[async_trait]
pub trait PipelineCommand : Debug {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues>;
}

/// A command that takes multiple inputs and produces a single output.
/// XXX speculative while implementing parallel processing.
#[async_trait]
pub trait PipelineJunctionCommand : Debug {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: Vec<PipelineValues>,
    ) -> Result<PipelineValues>;
}

/// A linear pipeline sequence.
pub struct ServerPipeline {
    pub server_kind: String,
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub commands: Vec<Box<dyn PipelineCommand>>,
}

pub struct NamedPipeline {
    /// Previous pipeline's output to consume.
    pub input_name: Option<String>,
    pub output_name: String,
    pub commands: Vec<Box<dyn PipelineCommand>>,
}

pub struct JunctionInvocation {
    pub input_names: Vec<String>,
    pub output_name: String,
    pub command: Box<dyn PipelineJunctionCommand>,
}

pub struct ParallelPipelines {
    pub pipelines: Vec<NamedPipeline>,
    pub junctions: Vec<JunctionInvocation>,
}

///
pub struct ServerPipelineGraph {
    pub server_kind: String,
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub pipelines: Vec<ParallelPipelines>,
}

impl ServerPipeline {
    pub async fn run(&self, traced: bool) -> Result<PipelineValues> {
        let mut cur_values = PipelineValues::Void;

        for cmd in &self.commands {
            let span = trace_span!("run_pipeline_step", cmd = ?cmd);
            let _span_guard = span.enter();

            match cmd.execute(&self.server, cur_values).await {
                Ok(next_values) => {
                    cur_values = next_values;
                }
                Err(err) => {
                    trace!(err = ?err);
                    return Err(err);
                }
            }

            if traced {
                let value_str = to_string_pretty(&cur_values).unwrap();
                trace!(output_json = %value_str);
            }
        }

        Ok(cur_values)
    }
}

impl ServerPipelineGraph {
    pub async fn run(&self, _traced: bool) -> Result<PipelineValues> {
        let cur_values = PipelineValues::Void;

        // XXX impl

        Ok(cur_values)
    }
}
