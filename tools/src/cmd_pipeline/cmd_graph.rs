use async_trait::async_trait;
use clap::arg_enum;
use structopt::StructOpt;

use graphviz_rust::cmd::{CommandArg, Format};
use graphviz_rust::exec;
use graphviz_rust::printer::{DotPrinter, PrinterContext};

use super::interface::{FileBlob, PipelineCommand, PipelineValues};

use crate::abstract_server::{AbstractServer, Result};

arg_enum! {
    #[derive(Debug, PartialEq)]
    pub enum GraphFormat {
        // Raw dot syntax without any layout performed.
        RawDot,
        SVG,
        PNG,
        // Dot with layout information.
        Dot,
    }
}

/// Render a received graph into a dot, svg, or json-wrapped-svg which also
/// includes embedded crossref information.
#[derive(Debug, StructOpt)]
pub struct Graph {
    #[structopt(long, short, possible_values = &GraphFormat::variants(), case_insensitive = true, default_value = "svg")]
    pub format: GraphFormat,
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
///   this would mean Quota Manager and mozStorage are both modules that
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
        _server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let graphs = match input {
            PipelineValues::SymbolGraphCollection(sgc) => sgc,
            // TODO: Figure out a better way to handle a nonsensical pipeline
            // configuration / usage.
            _ => {
                return Ok(PipelineValues::Void);
            }
        };

        let dot_graph = graphs.graph_to_graphviz(graphs.graphs.len() - 1);
        if self.args.format == GraphFormat::RawDot {
            let raw_dot_str = dot_graph.print(&mut PrinterContext::default());
            return Ok(PipelineValues::FileBlob(FileBlob {
                mime_type: "text/x-dot".to_string(),
                contents: raw_dot_str.as_bytes().to_vec(),
            }));
        }
        let (format, mime_type) = match self.args.format {
            GraphFormat::SVG => (Format::Svg, "image/svg+xml".to_string()),
            GraphFormat::PNG => (Format::Png, "image/png".to_string()),
            _ => (Format::Dot, "text/x-dot".to_string()),
        };
        let graph_contents = exec(
            dot_graph,
            &mut PrinterContext::default(),
            vec![CommandArg::Format(format)],
        )?.as_bytes().to_vec();
        Ok(PipelineValues::FileBlob(FileBlob {
            mime_type,
            contents: graph_contents,
        }))
    }
}
