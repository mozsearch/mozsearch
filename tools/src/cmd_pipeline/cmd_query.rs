use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{JsonValue, PipelineCommand, PipelineValues};
use crate::{
    abstract_server::{AbstractServer, Result},
};

#[derive(Debug, StructOpt)]
pub struct Query {
  /// Query string
  query: String,
}


pub struct QueryCommand {
  pub args: Query,
}

#[async_trait]
impl PipelineCommand for QueryCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let value = server.perform_query(&self.args.query).await?;

        Ok(PipelineValues::JsonValue(JsonValue {
          value
        }))
    }
}
