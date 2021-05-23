use async_trait::async_trait;
use clap::arg_enum;
use serde_json::Value;
use std::collections::HashSet;
use structopt::StructOpt;

pub use crate::abstract_server::{AbstractServer, Result};

arg_enum! {
  #[derive(Debug, PartialEq)]
  pub enum RecordType {
      Source,
      Target,
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
pub enum PipelineValues {
    JsonRecords(JsonRecords),
    HtmlExcerpts(HtmlExcerpts),
    Void,
}

/// JSON records are raw analysis records from a single file (for now)
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

pub struct JsonRecords {
    pub by_file: Vec<JsonRecordsByFile>,
}

pub struct HtmlExcerptsByFile {
    pub file: String,
    pub excerpts: Vec<String>,
}

pub struct HtmlExcerpts {
    pub by_file: Vec<HtmlExcerptsByFile>,
}

#[async_trait]
pub trait PipelineCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues>;
}

pub struct ServerPipeline {
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub commands: Vec<Box<dyn PipelineCommand>>,
}

impl ServerPipeline {
    pub async fn run(&self) -> Result<PipelineValues> {
        let mut cur_values = PipelineValues::Void;

        for cmd in &self.commands {
            match cmd.execute(&self.server, cur_values).await {
                Ok(next_values) => {
                    cur_values = next_values;
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }

        Ok(cur_values)
    }
}
