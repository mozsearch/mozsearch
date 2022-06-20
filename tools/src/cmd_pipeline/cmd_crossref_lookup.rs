use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{
    PipelineCommand, PipelineValues, SymbolCrossrefInfo, SymbolCrossrefInfoList, SymbolList, SymbolWithContext, SymbolQuality, SymbolRelation,
};

use crate::abstract_server::{AbstractServer, Result, ServerError, ErrorDetails, ErrorLayer};

/// Return the crossref data for one or more symbols received via pipeline or as
/// explicit arguments.
#[derive(Debug, StructOpt)]
pub struct CrossrefLookup {
    /// Explicit symbols to lookup.
    symbols: Vec<String>,
    // TODO: It might make sense to provide a way to filter the looked up data
    // by kind, although that could of course be its own command too.

    /// If the looked up symbol turns out to be a class with methods, instead of
    /// adding the class to the set, add its methods.
    #[structopt(long)]
    methods: bool,
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
                symbols: self.args.symbols.iter().map(|sym| {
                    SymbolWithContext {
                        symbol: sym.clone(),
                        quality: SymbolQuality::ExplicitSymbol,
                        from_identifier: None,
                    }
                }).collect(),
            },
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "crossref-lookup needs a Void or SymbolList".to_string(),
                }));
            }
        };

        let mut symbol_crossref_infos = vec![];
        for sym_ctx in symbol_list.symbols {
            let info = server.crossref_lookup(&sym_ctx.symbol).await?;

            let crossref_info = SymbolCrossrefInfo {
                symbol: sym_ctx.symbol,
                crossref_info: info,
                relation: SymbolRelation::Queried,
                quality: sym_ctx.quality,
                overloads_hit: vec![],
            };
            if self.args.methods {
                if let Some(method_syms) = crossref_info.get_method_symbols() {
                    for method_sym in method_syms {
                        let method_info = server.crossref_lookup(&method_sym).await?;
                        symbol_crossref_infos.push(SymbolCrossrefInfo {
                            symbol: method_sym,
                            crossref_info: method_info,
                            relation: SymbolRelation::Queried,
                            quality: crossref_info.quality.clone(),
                            overloads_hit: vec![],
                        });
                    }
                    continue;
                }
            }

            symbol_crossref_infos.push(crossref_info);
        }

        Ok(PipelineValues::SymbolCrossrefInfoList(
            SymbolCrossrefInfoList {
                symbol_crossref_infos,
            },
        ))
    }
}
