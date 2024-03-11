use async_trait::async_trait;
use clap::Args;
use ustr::{ustr, Ustr};

use super::interface::{
    PipelineCommand, PipelineValues, SymbolCrossrefInfo, SymbolCrossrefInfoList, SymbolMetaFlags, SymbolQuality, SymbolRelation
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

    /// Discards symbols whose pretty identifier is not an exact match for the
    /// symbol's `from_ident`.  This allows us to discard symbols from
    /// search-identifiers which were not an absolute identifier match.
    #[clap(short, long, value_parser)]
    exact_match: bool,
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
        let symbol_list: Vec<(String, SymbolQuality, Option<Ustr>)> = match input {
            PipelineValues::SymbolList(sl) => sl
                .symbols
                .into_iter()
                .map(|info| (info.symbol.to_string(), info.quality, info.from_identifier))
                .collect(),
            // Right now we're assuming that we're the first command in the
            // pipeline so that we would have no inputs if someone wants to use
            // arguments...
            PipelineValues::Void => self
                .args
                .symbols
                .iter()
                .map(|sym| (sym.clone(), SymbolQuality::ExplicitSymbol, None))
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
        for (symbol, quality, from_ident) in symbol_list {
            let info = server.crossref_lookup(&symbol, false).await?;

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
                flags: SymbolMetaFlags::default(),
            };
            if let (true, Some(pretty)) = (self.args.exact_match, from_ident) {
                if pretty.to_lowercase() != crossref_info.get_pretty().to_lowercase() {
                    continue;
                }
            }
            if self.args.methods {
                if let Some(method_syms) = crossref_info.get_method_symbols() {
                    for method_sym in method_syms {
                        let method_info = server.crossref_lookup(&method_sym, false).await?;
                        symbol_crossref_infos.push(SymbolCrossrefInfo {
                            symbol: method_sym,
                            crossref_info: method_info,
                            relation: SymbolRelation::Queried,
                            quality: crossref_info.quality.clone(),
                            overloads_hit: vec![],
                            flags: SymbolMetaFlags::default(),
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
