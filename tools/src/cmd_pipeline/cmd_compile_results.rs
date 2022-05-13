use async_trait::async_trait;
use structopt::StructOpt;

use super::{interface::{PipelineJunctionCommand, PipelineValues, FlattenedResultsBundle}, transforms::path_glob_transform};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/*

 */

#[derive(Debug, StructOpt)]
pub struct CompileResults {
    #[structopt(short, long, default_value = "0")]
    limit: usize,
}

#[derive(Debug)]
pub struct CompileResultsCommand {
    pub args: CompileResults,
}

#[async_trait]
impl PipelineJunctionCommand for CompileResultsCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: Vec<PipelineValues>,
    ) -> Result<PipelineValues> {

        let mut path_kind_results = vec![];

        Ok(PipelineValues::FlattenedResultsBundle(FlattenedResultsBundle {
            path_kind_results,
            content_type: "text/plain".to_string(),
        }))
    }
}
