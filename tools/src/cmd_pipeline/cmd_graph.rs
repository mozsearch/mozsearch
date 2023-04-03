use async_trait::async_trait;
use clap::{Args, ValueEnum};
use dot_generator::*;
use dot_structures::*;
use regex::{self, Captures, Regex};
use serde_json::Value;

use graphviz_rust::cmd::{CommandArg, Format};
use graphviz_rust::exec;
use graphviz_rust::printer::{DotPrinter, PrinterContext};

use super::interface::{
    GraphResultsBundle, PipelineCommand, PipelineValues, RenderedGraph, TextFile,
};
use super::symbol_graph::DerivedSymbolInfo;

use crate::abstract_server::{AbstractServer, Result};

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum GraphFormat {
    // JSON format, useful for when GraphMode is Hier.
    Json,
    // Raw dot syntax without any layout performed.
    RawDot,
    SVG,
    PNG,
    // Dot with layout information.
    Dot,
    // Transformed SVG accompanied by symbol metadata in a JSON structure.
    Mozsearch,
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum GraphMode {
    /// No hierarchy, everything is a node, there are no clusters.
    Flat,
    /// Derive a hierarchical relationship with clusters from the input graph.
    Hier,
}

/// Render a received graph into a dot, svg, or json-wrapped-svg which also
/// includes embedded crossref information.
#[derive(Debug, Args)]
pub struct Graph {
    #[clap(long, short, value_parser, value_enum, default_value = "svg")]
    pub format: GraphFormat,

    #[clap(long, short, value_parser, value_enum, default_value = "hier")]
    pub mode: GraphMode,

    /// Enable debug mode which currently means forcing the format to be Json.
    /// This is currently structured this way because this is intended to be
    /// used as a flag translated by `query_core.toml` and we avoid problems
    /// where the default format argument created by the pipelien fights a user
    /// controlled value.  But it also makes sense to have this debug flag be
    /// something explicit and for the explicit use of the query syntax.
    #[clap(long, value_parser)]
    pub debug: bool,

    #[clap(long, value_parser)]
    pub colorize_callees: Vec<String>,
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
#[derive(Debug)]
pub struct GraphCommand {
    pub args: Graph,
}

/// Convert tunneled symbol identifiers in the SVG into `data-symbols`
/// attributes.  Specific conversions:
/// - `<title>` tags holding the node identifiers.
/// - `<a xlink...>` tags holding links resulting from HTML labels./
///
/// Currently our general behavior is to drop anything that has an identifier
/// that starts with "SYN_" and to assume that everything else is a valid
/// symbol.  The original fancy branch prototype instead generated completely
/// synthetic identifiers in all cases which were added to a map so that
/// data-symbols could be looked up in the map.  I think there's something to be
/// said for for having the identifiers be more self-descriptive as we're
/// currently doing, but arguably it probably makes more sense to generate a
/// mangled pretty identifier with compensation made for any collisions.
///
/// TODO: As proposed above, potentially move towards allocating identifiers
/// with a lookup map.
fn transform_svg(svg: &str) -> String {
    lazy_static! {
        static ref RE_TITLE: Regex = Regex::new(">\n<title>([^<]+)</title>").unwrap();
        static ref RE_XLINK: Regex = Regex::new(r#"<a xlink:href="([^"]+)" xlink:title="[^"]+">"#).unwrap();
    }
    let titled = RE_TITLE
        .replace_all(svg, |caps: &Captures| {
            let captured = caps.get(1).unwrap().as_str();
            // Do not transform the `g` title of "g" to data-symbols.  Although
            // maybe we should be providing it a better title?  Although maybe
            // a straight-up heading explaining the graph is even better, as I
            // think this is where we're going to want to put the dual UI.
            if captured == "g" || captured.starts_with("SYN_") {
                ">".to_string()
            } else {
                format!(" data-symbols=\"{}\">", captured)
            }
        });
    RE_XLINK
        .replace_all(&titled, |caps: &Captures| {
            let captured = caps.get(1).unwrap().as_str();
            if captured.starts_with("SYN_") {
                "<g>".to_string()
            } else {
                format!("<g data-symbols=\"{}\">", captured)
            }
        }).replace("</a>", "</g>")
}

#[async_trait]
impl PipelineCommand for GraphCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let mut graphs = match input {
            PipelineValues::SymbolGraphCollection(sgc) => sgc,
            // TODO: Figure out a better way to handle a nonsensical pipeline
            // configuration / usage.
            _ => {
                return Ok(PipelineValues::Void);
            }
        };

        let decorate_node = |node: &mut Node, sym_info: &DerivedSymbolInfo| {
            for (i, colorize) in self.args.colorize_callees.iter().enumerate() {
                if let Some(Value::Array(arr)) = sym_info.crossref_info.get("callees") {
                    for callee in arr {
                        if let Some(Value::String(pretty)) = callee.get("pretty") {
                            if pretty.ends_with(colorize) {
                                node.attributes.push(attr!("colorscheme", "pastel28"));
                                node.attributes.push(attr!("style", "filled"));
                                node.attributes.push(attr!("fillcolor", i + 1));
                            }
                        }
                    }
                }
            }
        };

        let dot_graph = match self.args.mode {
            GraphMode::Flat => {
                graphs.graph_to_graphviz(graphs.graphs.len() - 1, decorate_node)
            }
            GraphMode::Hier => {
                graphs.derive_hierarchical_graph(graphs.graphs.len() - 1, server).await?;
                //return Ok(PipelineValues::SymbolGraphCollection(graphs));
                graphs.hierarchical_graph_to_graphviz(graphs.hierarchical_graphs.len() - 1)
            }
        };
        if self.args.format == GraphFormat::RawDot {
            let raw_dot_str = dot_graph.print(&mut PrinterContext::default());
            return Ok(PipelineValues::TextFile(TextFile {
                mime_type: "text/x-dot".to_string(),
                contents: raw_dot_str,
            }));
        }

        // Currently our debug mode is to just force ourselves to render the graph
        // as JSON.
        let use_format = match (self.args.debug, &self.args.format) {
            (true, _) => GraphFormat::Json,
            (_, format) => format.clone()
        };

        let (format, mime_type) = match &use_format {
            GraphFormat::SVG | GraphFormat::Mozsearch => (Format::Svg, "image/svg+xml".to_string()),
            GraphFormat::PNG => (Format::Png, "image/png".to_string()),
            _ => (Format::Dot, "text/x-dot".to_string()),
        };
        let graph_contents = exec(
            dot_graph,
            &mut PrinterContext::default(),
            vec![CommandArg::Format(format)],
        )?;
        match use_format {
            GraphFormat::Json => {
                Ok(PipelineValues::SymbolGraphCollection(graphs))
            }
            GraphFormat::Mozsearch => Ok(PipelineValues::GraphResultsBundle(GraphResultsBundle {
                graphs: vec![RenderedGraph {
                    graph: transform_svg(&graph_contents),
                }],
                symbols: graphs.symbols_meta_to_jumpref_json_destructive(),
                overloads_hit: graphs.overloads_hit,
            })),
            _ => Ok(PipelineValues::TextFile(TextFile {
                mime_type,
                contents: graph_contents,
            })),
        }
    }
}
