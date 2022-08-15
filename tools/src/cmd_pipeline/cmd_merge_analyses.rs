use async_trait::async_trait;
use serde_json::{from_str, Value};
use clap::Args;

use super::interface::{
    JsonRecords, PipelineCommand, PipelineValues,
};
use crate::{
    abstract_server::{AbstractServer, Result, ServerError},
    cmd_pipeline::interface::JsonRecordsByFile,
    file_format::merger::merge_files,
};

/// Merge analysis files from different build configs into one analysis file.
#[derive(Debug, Args)]
pub struct MergeAnalyses {
    /// Tree-relative analysis file paths
    #[clap(value_parser)]
    files: Vec<String>,

    /// The list of platforms to claim the files came from.
    #[clap(long, short, value_parser)]
    platforms: Vec<String>,
}

/// Command brought into existence to test the analysis-merging logic of
/// `merge-analyses.rs`.
#[derive(Debug)]
pub struct MergeAnalysesCommand {
    pub args: MergeAnalyses,
}

#[async_trait]
impl PipelineCommand for MergeAnalysesCommand {
    ///
    ///
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let abs_paths: Result<Vec<String>> = self
            .args
            .files
            .iter()
            .map(|f| server.translate_analysis_path(f))
            .collect();

        let mut merged_output = Vec::new();
        merge_files(&abs_paths?, &self.args.platforms, &mut merged_output);

        let values: Result<Vec<Value>> = std::str::from_utf8(merged_output.as_slice())
            .unwrap()
            .lines()
            .map(|s| from_str(s).map_err(|e| ServerError::from(e)))
            .collect();

        Ok(PipelineValues::JsonRecords(JsonRecords {
            by_file: vec![JsonRecordsByFile {
                file: self.args.files.iter().next().unwrap().clone(),
                records: values?,
            }],
        }))
    }
}
