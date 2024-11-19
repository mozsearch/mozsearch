use async_trait::async_trait;
use clap::Args;

use super::interface::{
    PipelineJunctionCommand, PipelineValues, SymbolCrossrefInfoList, SymbolMetaFlags,
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Experimental junction enabling multiple query chains that output
/// crossref-lookup to be unified into a single SymbolCrossrefInfoList but with
/// different annotatins/metadata.
///
/// The driving use-case right now is calls-between-source/calls-between-target
/// where the crossref-lookups support exploding a class into its methods.  This
/// is very experimental as it seems quite possible that an approach that's more
/// explicitly aware of hierarchy and/or could leverage some pair-wise
/// precomputations could be useful.
#[derive(Debug, Args)]
pub struct FuseCrossrefs {}

#[allow(dead_code)]
#[derive(Debug)]
pub struct FuseCrossrefsCommand {
    pub args: FuseCrossrefs,
}

#[async_trait]
impl PipelineJunctionCommand for FuseCrossrefsCommand {
    async fn execute(
        &self,
        _server: &(dyn AbstractServer + Send + Sync),
        input: Vec<(String, PipelineValues)>,
    ) -> Result<PipelineValues> {
        let mut fused_crossref = vec![];
        let mut fused_unknown = vec![];

        // We currently don't care about the name of the input because we only
        // match by type, but one could imagine a scenario in which they serve
        // as labels we want to propagate.
        for (name, pipe_value) in input {
            let add_flags = match name.as_ref() {
                "source" => SymbolMetaFlags::Source,
                "target" => SymbolMetaFlags::Target,
                _ => SymbolMetaFlags::default(),
            };
            match pipe_value {
                PipelineValues::SymbolCrossrefInfoList(mut scil) => {
                    for mut info in scil.symbol_crossref_infos {
                        info.flags |= add_flags;
                        fused_crossref.push(info);
                    }
                    fused_unknown.append(&mut scil.unknown_symbols);
                }
                _ => {
                    return Err(ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::ConfigLayer,
                        message: "fuse-crossrefs got something weird".to_string(),
                    }));
                }
            }
        }

        Ok(PipelineValues::SymbolCrossrefInfoList(
            SymbolCrossrefInfoList {
                symbol_crossref_infos: fused_crossref,
                unknown_symbols: fused_unknown,
            },
        ))
    }
}
