use async_trait::async_trait;
use clap::Args;
use ustr::ustr;

use super::interface::{
    PipelineCommand, PipelineValues, SymbolCrossrefInfo, SymbolCrossrefInfoList, SymbolQuality,
    SymbolRelation,
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Return the crossref data for one or more symbols received via pipeline or as
/// explicit arguments.
#[derive(Debug, Args)]
pub struct CrossrefLookup {
    /// Explicit symbols to lookup.
    #[clap(value_parser)]
    symbols: Vec<String>,
    // TODO: It might make sense to provide a way to filter the looked up data
    // by kind, although that could of course be its own command too.
    /// If the looked up symbol turns out to be a class with methods, instead of
    /// adding the class to the set, add its methods.
    #[clap(long, action)]
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
        // Because this pipeline stage can receive symbols from unfiltered user
        // input and we have no reason to believe the `Ustr` interned symbol
        // table contains all potentially known strings, we must operate in
        // String space until we get values back from the crossref lookup!
        let symbol_list: Vec<(String, SymbolQuality)> = match input {
            PipelineValues::SymbolList(sl) => sl
                .symbols
                .into_iter()
                .map(|info| (info.symbol.to_string(), info.quality))
                .collect(),
            // Right now we're assuming that we're the first command in the
            // pipeline so that we would have no inputs if someone wants to use
            // arguments...
            PipelineValues::Void => self
                .args
                .symbols
                .iter()
                .map(|sym| (sym.clone(), SymbolQuality::ExplicitSymbol))
                .collect(),
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "crossref-lookup needs a Void or SymbolList".to_string(),
                }));
            }
        };

        let mut symbol_crossref_infos = vec![];
        let mut unknown_symbols = vec![];
        for (symbol, quality) in symbol_list {
            let info = server.crossref_lookup(&symbol).await?;

            if info.is_null() {
                unknown_symbols.push(symbol);
                continue;
            }

            let crossref_info = SymbolCrossrefInfo {
                // Now that we've validted that the symbol exists via crossref
                // lookup, we know it's safe to mint a Ustr for it if it doesn't
                // exist.  (Otherwise hostile/broken callers could explode our
                // interning table.)
                symbol: ustr(&symbol),
                crossref_info: info,
                relation: SymbolRelation::Queried,
                quality,
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
                unknown_symbols,
            },
        ))
    }
}
