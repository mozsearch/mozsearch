use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{
    JsonRecords, PipelineCommand, PipelineValues, RecordType, SymbolicQueryOpts,
};
use crate::{
    abstract_server::{AbstractServer, Result},
    cmd_pipeline::interface::JsonRecordsByFile,
};

#[derive(Debug, StructOpt)]
pub struct FilterAnalysis {
    /// Tree-relative analysis file path
    file: String,

    #[structopt(long, short, possible_values = &RecordType::variants(), case_insensitive = true)]
    record_type: Option<Vec<RecordType>>,

    #[structopt(long, short)]
    kind: Option<String>,

    #[structopt(flatten)]
    query_opts: SymbolicQueryOpts,
}

/// Filter a stream of analysis records via raw JSON manipulation rather than
/// using the strongly typed `analysis.rs` types.
pub struct FilterAnalysisCommand {
    pub args: FilterAnalysis,
}

#[async_trait]
impl PipelineCommand for FilterAnalysisCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let records = server.fetch_raw_analysis(&self.args.file).await?;
        let mut filtered = records;

        // ## Filter by record type
        if let Some(record_types) = &self.args.record_type {
            filtered = filtered
                .into_iter()
                .filter(|val| {
                    // Record type is currently indicated via boolean presence of
                    // "source", "target", or "structured" so check for the
                    // stringified version of the enum value.
                    for rt in record_types {
                        if val.get(rt.to_string().to_lowercase()).is_some() {
                            return true;
                        }
                    }
                    false
                })
                .collect();
        }

        // ## Filter by kind
        if let Some(kind) = &self.args.kind {
            // kind varies by record type:
            // - target: "kind" is a single valued attribute
            // - source: kind is baked into the comma-delimited "syntax"
            filtered = filtered
                .into_iter()
                .filter(
                    |val| match (val["source"].is_number(), val["target"].is_number()) {
                        // source: consult "syntax"
                        (true, _) => match val["syntax"].as_str() {
                            None => false,
                            Some(actual) => actual.split(",").next().unwrap_or("") == kind,
                        },
                        // target: consult "kind"
                        (false, true) => match val["kind"].as_str() {
                            None => false,
                            Some(actual) => actual == kind,
                        },
                        _ => false,
                    },
                )
                .collect();
        }

        // ## Filter by symbol
        if let Some(symbol) = &self.args.query_opts.symbol {
            // "sym" is optionally
            filtered = filtered
                .into_iter()
                .filter(|val| match val["sym"].as_str() {
                    None => false,
                    Some(actual) => actual.split(",").any(|s| s == symbol),
                })
                .collect();
        }

        // ## Filter by identifier
        if let Some(identifier) = &self.args.query_opts.identifier {
            filtered = filtered
                .into_iter()
                .filter(|val| {
                    match val["pretty"].as_str() {
                        None => false,
                        // source records have a space-delimited prefix that we want
                        // to skip; by using split/last we handle it being optional.
                        Some(actual) => actual.split(" ").last().unwrap_or("") == identifier,
                    }
                })
                .collect();
        }

        Ok(PipelineValues::JsonRecords(JsonRecords {
            by_file: vec![JsonRecordsByFile {
                file: self.args.file.clone(),
                records: filtered,
            }],
        }))
    }
}
