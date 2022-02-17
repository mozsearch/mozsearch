use async_trait::async_trait;
use structopt::StructOpt;

use super::interface::{
    PipelineCommand, PipelineValues, SymbolCrossrefInfo, SymbolCrossrefInfoList, SymbolList,
};

use crate::abstract_server::{AbstractServer, Result};

/// Render a received graph into a dot, svg, or json-wrapped-svg which also
/// includes embedded crossref information.
#[derive(Debug, StructOpt)]
pub struct Graph {
    /// Explicit symbols to lookup.
    symbols: Vec<String>,
    // TODO: It might make sense to provide a way to filter the looked up data
    // by kind, although that could of course be its own command too.
}

/// ## Graph Implementation Thoughts / Rationale ##
///
/// ### Latency, Pre-Computation, and Interaction ###
///
/// #### Pre-Graph Status Quo ####
///
/// Current searchfox UX is that while search may take a few seconds (the first
/// time the query is experienced; we do cache), when they arrive, you'll have
/// all the results you're going to get unless you continue typing.  There's no
/// async trickle-in.
///
/// While there can be upsides to async data retrieval, this primarily makes
/// sense for cases where the data being asynchronously populated is reliably
/// known to be at the end of the current results list.  Asynchronous retrieval
/// that leads to visual and interaction instability can be frustrating,
/// especially when it's not clear if the results have stabilized.
///
/// One thing we haven't done yet in the normal searchfox UI (but did experiment
/// with in the fancy branch) is to allow iterative (faceted) filtering of the
/// displayed results.  There has only been the ability to collapse sections of
/// results.  But we could do more with this.
///
/// #### Application to Graphing ####
///
/// When building the graph, we will potentially gather information about edges
/// to nodes that don't make the initial cut for presentation.  But rather than
/// discarding them, we'll keep them around in the dataset that we serve up so
/// that the collapsed clusters can be interactively expanded or additional data
/// (ex: on fields accessed) can be provided in a detail display when clicking
/// on nodes, etc.
///
/// Using IndexedDB as an example of what this means, from the fancy branch we
/// already know that edges can potentially fall into the following groups:
/// - In-module function calls.  This covers both "boring" getters/setters and
///   assertions that don't express interseting control flow, as well as more
///   significant helper modules that potentially in turn call other non-boring
///   methods.
/// - Cross-module function calls to non-core-infrastructure modules.  In IDB
///   this would mean Quota Manager and mozStorage are both moduleles that
///   involve core application-domain logic.
/// - Cross-module function calls to "boring" core-infrastructure modules.  For
///   example, the fancy branch elides all calls to smart pointers and XPCOM
///   string classes because these usually are not interesting on their own and
///   the field is instead more interesting.  Note that the fancy branch ended
///   up filtering to only in-module edges eventually, which meant that this
///   additional filtering was somewhat mooted and potentially was not
///   sufficient as it would not have prevented data structure spam, etc.
///
/// As noted above, the fancy branch prototype found that limiting calls to the
/// same module as determined by source path provided a reasonable filtering,
/// but it's quite possible that the interesting bits are in fact happening in
/// other modules.  So, at least as long as a work limit isn't it, we could
/// traverse into the other modules but make a choice at presentation time to
/// collapse those edges by having clustered by module and simpifying the edges
/// so that they go to a single node representation of the cluster that can be
/// clicked on to be expanded.
///
/// The expansion can be handled by using existing JS code (built on graphviz
/// compiled to WASM) that can animate a transition between the different
/// rendered graph states.
///
/// Note that this is currently an end state of the proposal at
/// https://bugzilla.mozilla.org/show_bug.cgi?id=1749232 and we won't be
/// implementing this initially, but this will inform how the graph is modeled.
/// That said, it's quite possible most of this logic will be implemented as a
/// graph transformation pass that clusters nodes.  The initial transitive
/// traversal might instead be focused on a work limit heuristic based on rough
/// order-of-magnitude weight adjustments.
///
pub struct GraphCommand {
    pub args: Graph,
}

#[async_trait]
impl PipelineCommand for GraphCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        // XXX this is still just crossref-lookup cut-and-pasted
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
