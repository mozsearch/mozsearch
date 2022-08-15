use async_trait::async_trait;
use serde_json::{to_value};
use clap::Args;

use super::{interface::{JsonValue, PipelineCommand, PipelineValues}, builder::build_pipeline_graph};
use crate::{
    abstract_server::{AbstractServer, Result}, query::chew_query::chew_query,
};

#[derive(Debug, Args)]
pub struct BatchRender {
    /// Preconfigured rendering task.  This could be an enum or sub-command, but
    /// for now we're just going for strings.
    task: String,
}

#[derive(Debug)]
pub struct BatchRenderCommand {
    pub args: BatchRender,
}

#[async_trait]
impl PipelineCommand for BatchRenderCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let pipeline_plan = chew_query(&self.args.query)?;

        if self.args.dump_pipeline {
            return Ok(PipelineValues::JsonValue(JsonValue { value: to_value(pipeline_plan)? }));
        }

        let graph = build_pipeline_graph(server.clonify(), pipeline_plan)?;

        let result = graph.run(true).await;
        result
    }
}
