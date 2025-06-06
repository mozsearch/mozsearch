use async_trait::async_trait;
use clap::Args;
use tokio_stream::StreamExt;

use super::interface::{
    JsonRecords, PipelineCommand, PipelineValues, RecordType, SymbolicQueryOpts,
};
use crate::{
    abstract_server::{AbstractServer, Result},
    cmd_pipeline::interface::JsonRecordsByFile,
};

/// Filter the contents of a single analysis file.
#[derive(Debug, Args)]
pub struct FilterAnalysis {
    /// Tree-relative analysis file path
    #[clap(value_parser)]
    file: String,

    #[clap(long, short, value_parser, value_enum)]
    record_type: Option<Vec<RecordType>>,

    #[clap(long, short, value_parser)]
    kind: Option<String>,

    #[clap(flatten)]
    query_opts: SymbolicQueryOpts,
}

#[derive(Debug)]
pub struct FilterAnalysisCommand {
    pub args: FilterAnalysis,
}

/// ### Implementation Note
/// Filtering is currently performed via generic JSON rather than the strongly
/// typed `analysis.rs` types, but this pre-dates the change to using serde-json
/// and it probably makes sense to switch to using the raw types.
#[async_trait]
impl PipelineCommand for FilterAnalysisCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let mut filtered = server.fetch_raw_analysis(&self.args.file).await?;

        // ## Filter by record type
        if let Some(record_types) = &self.args.record_type {
            filtered = Box::pin(filtered.filter(move |val| {
                // Record type is currently indicated via boolean presence of
                // "source", "target", or "structured" so check for the
                // stringified version of the enum value.
                for rt in record_types {
                    if val.get(rt.name()).is_some() {
                        return true;
                    }
                }
                false
            }));
        }

        // ## Filter by kind
        if let Some(kind) = &self.args.kind {
            // kind varies by record type:
            // - target: "kind" is a single valued attribute
            // - source: kind is baked into the comma-delimited "syntax"
            filtered = Box::pin(filtered.filter(move |val| {
                match (val["source"].is_number(), val["target"].is_number()) {
                    // source: consult "syntax"
                    (true, _) => match val["syntax"].as_str() {
                        None => false,
                        Some(actual) => actual.split(",").any(|k| k == kind),
                    },
                    // target: consult "kind"
                    (false, true) => match val["kind"].as_str() {
                        None => false,
                        Some(actual) => actual == kind,
                    },
                    _ => false,
                }
            }));
        }

        // ## Filter by symbol
        if let Some(symbol) = &self.args.query_opts.symbol {
            // "sym" is optionally
            filtered = Box::pin(filtered.filter(move |val| match val["sym"].as_str() {
                None => false,
                Some(actual) => actual.split(",").any(|s| s == symbol),
            }));
        }

        // ## Filter by symbol prefix
        if let Some(symbol_prefix) = &self.args.query_opts.symbol_prefix {
            // "sym" is optionally
            filtered = Box::pin(filtered.filter(move |val| match val["sym"].as_str() {
                None => false,
                Some(actual) => actual.split(",").any(|s| s.starts_with(symbol_prefix)),
            }));
        }

        // ## Filter by identifier
        if let Some(identifier) = &self.args.query_opts.identifier {
            filtered = Box::pin(filtered.filter(move |val| {
                match val["pretty"].as_str() {
                    None => false,
                    // source records have a space-delimited prefix that we want
                    // to skip; by using split/last we handle it being optional.
                    Some(actual) => actual.split(" ").last().unwrap_or("") == identifier,
                }
            }));
        }

        Ok(PipelineValues::JsonRecords(JsonRecords {
            by_file: vec![JsonRecordsByFile {
                file: self.args.file.clone(),
                records: filtered.collect().await,
            }],
        }))
    }
}
