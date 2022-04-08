use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{
    PipelineCommand, PipelineValues, SymbolCrossrefInfo, SymbolCrossrefInfoList, SymbolList,
};

use crate::abstract_server::{AbstractServer, Result};

/// Return the crossref data for one or more symbols received via pipeline or as
/// explicit arguments.
#[derive(Debug, StructOpt)]
pub struct CrossrefLookup {
    /// Explicit symbols to lookup.
    symbols: Vec<String>,
    // TODO: It might make sense to provide a way to filter the looked up data
    // by kind, although that could of course be its own command too.
}


#[derive(Debug)]
pub struct CrossrefLookupCommand {
    pub args: CrossrefLookup,
}

#[async_trait]
impl PipelineCommand for CrossrefLookupCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let symbol_list = match input {
            PipelineValues::SymbolList(sl) => sl,
            // Right now we're assuming that we're the first command in the
            // pipeline so that we would have no inputs if someone wants to use
            // arguments...
            PipelineValues::Void => SymbolList {
                symbols: self.args.symbols.clone(),
                from_identifiers: None,
            },
            // TODO: Figure out a better way to handle a nonsensical pipeline
            // configuration / usage.
            _ => {
                return Ok(PipelineValues::Void);
            }
        };

        let mut symbol_crossref_infos = vec![];
        for symbol in symbol_list.symbols {
            let info = server.crossref_lookup(&symbol).await?;
            symbol_crossref_infos.push(SymbolCrossrefInfo {
                symbol,
                crossref_info: info,
            });
        }

        Ok(PipelineValues::SymbolCrossrefInfoList(
            SymbolCrossrefInfoList {
                symbol_crossref_infos,
            },
        ))
    }
}
