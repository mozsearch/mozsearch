use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{IdentifierList, PipelineCommand, PipelineValues, SymbolList};

use crate::abstract_server::{AbstractServer, Result};

/// Return the crossref data for one or more symbols received via pipeline or as
/// explicit arguments.
#[derive(Debug, StructOpt)]
pub struct SearchIdentifiers {
    /// Explicit identifiers to search.
    identifiers: Vec<String>,

    /// Should this be an exact-match?  By default we do a prefix search.
    #[structopt(short, long)]
    exact_match: bool,

    /// Should this be case-sensitive?  By default we are case-insensitive.
    #[structopt(short, long)]
    case_sensitive: bool,

    #[structopt(short, long, default_value = "0")]
    limit: usize,
}

#[derive(Debug)]
pub struct SearchIdentifiersCommand {
    pub args: SearchIdentifiers,
}

#[async_trait]
impl PipelineCommand for SearchIdentifiersCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let identifier_list = match input {
            PipelineValues::IdentifierList(il) => il,
            // Right now we're assuming that we're the first command in the
            // pipeline so that we would have no inputs if someone wants to use
            // arguments...
            PipelineValues::Void => IdentifierList {
                identifiers: self.args.identifiers.clone(),
            },
            // TODO: Figure out a better way to handle a nonsensical pipeline
            // configuration / usage.
            _ => {
                return Ok(PipelineValues::Void);
            }
        };

        let mut symbols: Vec<String> = vec![];
        let mut from_identifiers: Vec<String> = vec![];
        for id in identifier_list.identifiers {
            for (sym, from_ident) in server
                .search_identifiers(
                    &id,
                    self.args.exact_match,
                    !self.args.case_sensitive,
                    self.args.limit,
                )
                .await?
            {
                symbols.push(sym);
                from_identifiers.push(from_ident);
            }
        }

        Ok(PipelineValues::SymbolList(SymbolList {
            symbols,
            from_identifiers: Some(from_identifiers),
        }))
    }
}
