use async_trait::async_trait;
use clap::Args;

use super::interface::{
    PipelineCommand, PipelineValues, SymbolList, SymbolQuality, SymbolWithContext,
};

use crate::abstract_server::{AbstractServer, Result};

/// Return the crossref data for one or more symbols received via pipeline or as
/// explicit arguments.
#[derive(Debug, Args)]
pub struct SearchIdentifiers {
    /// Explicit identifiers to search.
    #[clap(value_parser)]
    identifiers: Vec<String>,

    /// Should this be an exact-match?  By default we do a prefix search.
    #[clap(short, long, value_parser)]
    exact_match: bool,

    /// Should this be case-sensitive?  By default we are case-insensitive.
    #[clap(short, long, value_parser)]
    case_sensitive: bool,

    /// Should this only match types as indicated by the `T_` convention?
    /// Currently a hack and this should instead be handled by having the
    /// crossref-lookup do the filtering.
    #[clap(long, value_parser)]
    types_only: bool,

    /// Minimum identifier length to search for.  The default of 3 is derived
    /// from router.py's `is_trivial_search` heuristic requiring a length of 3,
    /// although it was only required along one axis.
    #[clap(long, value_parser, default_value = "3")]
    min_length: usize,

    #[clap(short, long, value_parser, default_value = "1000")]
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
        let identifier_list: Vec<String> = match input {
            PipelineValues::IdentifierList(il) => il
                .identifiers
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            // Right now we're assuming that we're the first command in the
            // pipeline so that we would have no inputs if someone wants to use
            // arguments...
            PipelineValues::Void => self.args.identifiers.clone(),
            // TODO: Figure out a better way to handle a nonsensical pipeline
            // configuration / usage.
            _ => {
                return Ok(PipelineValues::Void);
            }
        };

        let mut symbols: Vec<SymbolWithContext> = vec![];
        for id in identifier_list {
            // Skip any identifiers that are shorter than our minimum length.
            if id.len() < self.args.min_length {
                continue;
            }

            for (sym, from_ident) in server
                .search_identifiers(
                    &id,
                    self.args.exact_match,
                    !self.args.case_sensitive,
                    self.args.limit,
                )
                .await?
            {
                if self.args.types_only && !sym.starts_with("T_") {
                    continue;
                }

                let quality = match (
                    &self.args.exact_match,
                    id.as_str() == from_ident.as_str(),
                    &id,
                    &from_ident,
                ) {
                    (true, _, _, _) => SymbolQuality::ExplicitIdentifier,
                    (false, true, _, _) => SymbolQuality::ExactIdentifier,
                    (_, _, searched, result) => SymbolQuality::IdentifierPrefix(
                        searched.len() as u32,
                        (result.len() - searched.len()) as u32,
                    ),
                };
                symbols.push(SymbolWithContext {
                    symbol: sym,
                    quality,
                    from_identifier: Some(from_ident),
                });
            }
        }

        Ok(PipelineValues::SymbolList(SymbolList { symbols }))
    }
}
