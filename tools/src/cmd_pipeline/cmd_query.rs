use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{JsonValue, PipelineCommand, PipelineValues};
use crate::{
    abstract_server::{AbstractServer, Result},
};

/// Run a traditional searchfox query against the web server.  This will turn
/// into a no-op when run against a local index at this time, but in the future
/// may be able to spin up the necessary pieces.
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
