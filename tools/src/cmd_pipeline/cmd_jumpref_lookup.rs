use async_trait::async_trait;
use clap::Args;

use super::interface::{
    PipelineCommand, PipelineValues, JsonValueList, JsonValue,
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Return the jumpref data for one or more symbols received via pipeline or as
/// explicit arguments and provide it as JSON, specifically as a JsonValueList
/// containing JsonValue instances.  If we end up wanting to do additional
/// processing on jumpref data (like showing the html that it references), the
/// output should probably be given its own type.
#[derive(Debug, Args)]
pub struct JumprefLookup {
    /// Explicit symbols to lookup.
    #[clap(value_parser)]
    symbols: Vec<String>,
}

#[derive(Debug)]
pub struct JumprefLookupCommand {
    pub args: JumprefLookup,
}

#[async_trait]
impl PipelineCommand for JumprefLookupCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        // Because this pipeline stage can receive symbols from unfiltered user
        // input and we have no reason to believe the `Ustr` interned symbol
        // table contains all potentially known strings, we must operate in
        // String space until we get values back from the crossref lookup!
        let symbol_list: Vec<String> = match input {
            PipelineValues::SymbolList(sl) => sl
                .symbols
                .into_iter()
                .map(|info| info.symbol.to_string())
                .collect(),
            // Right now we're assuming that we're the first command in the
            // pipeline so that we would have no inputs if someone wants to use
            // arguments...
            PipelineValues::Void => self
                .args
                .symbols
                .iter()
                .map(|sym| sym.clone())
                .collect(),
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "jumpref-lookup needs a Void or SymbolList".to_string(),
                }));
            }
        };

        let mut jumpref_values = vec![];
        for symbol in symbol_list {
            let info = server.jumpref_lookup(&symbol).await?;
            jumpref_values.push(JsonValue { value: info });
        }

        Ok(PipelineValues::JsonValueList(
            JsonValueList {
                values: jumpref_values,
            },
        ))
    }
}
