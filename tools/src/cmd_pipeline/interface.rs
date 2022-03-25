use async_trait::async_trait;
use clap::arg_enum;
use serde_json::Value;
use std::collections::{HashSet};
use structopt::StructOpt;

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
pub enum PipelineValues {
    IdentifierList(IdentifierList),
    SymbolList(SymbolList),
    SymbolCrossrefInfoList(SymbolCrossrefInfoList),
    SymbolGraphCollection(SymbolGraphCollection),
    JsonValue(JsonValue),
    JsonRecords(JsonRecords),
    HtmlExcerpts(HtmlExcerpts),
    FileBlob(FileBlob),
    Void,
}

/// A list of (searchfox) identifiers.
pub struct IdentifierList {
    pub identifiers: Vec<String>,
}

/// A list of (searchfox) symbols.
pub struct SymbolList {
    pub symbols: Vec<String>,
    /// If present, these correspond to the identifiers that give us the
    /// symbols.  This is used in cases where an non-exact_match identifier
    /// search is performed and so we may not actually know what the identifiers
    /// actually were.
    pub from_identifiers: Option<Vec<String>>,
}

/// A symbol and its cross-reference information.
pub struct SymbolCrossrefInfo {
    pub symbol: String,
    pub crossref_info: Value,
}

/// A list of `SymbolCrossrefInfo`s.
pub struct SymbolCrossrefInfoList {
    pub symbol_crossref_infos: Vec<SymbolCrossrefInfo>,
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

/// A single JSON value, usually expected to be from a search query.
///
/// It might make sense to add a type-indicating value or origin of the JSON,
/// but for now this will only be from the query.
pub struct JsonValue {
    pub value: Value,
}

/// JSON Analysis Records grouped by (source) file.
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

pub struct FileBlob {
    pub mime_type: String,
    pub contents: Vec<u8>,
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
