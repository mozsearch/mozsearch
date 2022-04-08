use crate::{cmd_pipeline::{PipelineCommand, cmd_prod_filter::ProductionFilterCommand}, structopt::StructOpt};
use tracing::{trace, trace_span};
use url::Url;

use crate::{
    abstract_server::{
        make_local_server, make_remote_server, ErrorDetails, ErrorLayer, Result, ServerError,
    },
    cmd_pipeline::parser::{Command, OutputFormat, ToolOpts},
};

use super::{cmd_filter_analysis::FilterAnalysisCommand, cmd_merge_analyses::MergeAnalysesCommand, cmd_crossref_lookup::CrossrefLookupCommand, cmd_search_identifiers::SearchIdentifiersCommand, cmd_graph::GraphCommand};
use super::cmd_search::SearchCommand;
use super::cmd_show_html::ShowHtmlCommand;
use super::cmd_traverse::TraverseCommand;

use super::interface::ServerPipeline;

/// Build a command pipeline from a shell-y string where we use pipe boundaries
/// to delineate the separate pipeline steps.
///
/// The shell-words module is used to parse `arg_str` into shell words, which we
/// then break into separate sub-commands whenever we see a `|`.  We then pass
/// these sub-commands to the structopt parsing `from_iter` method, taking care
/// to stuff our binary name into the first arg.
pub fn build_pipeline(bin_name: &str, arg_str: &str) -> Result<(ServerPipeline, OutputFormat)> {
    let span = trace_span!("build_pipeline", arg_str);
    let _span_guard = span.enter();

    let all_args = match shell_words::split(arg_str) {
        Ok(parsed) => parsed,
        Err(err) => {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: err.to_string(),
            }));
        }
    };

    let mut server_kind = "none";
    let mut server = None;
    let mut output_format = None;
    let mut first_time = true;

    let mut commands: Vec<Box<dyn PipelineCommand>> = vec![];

    for arg_slices in all_args.split(|v| v == "|") {
        let mut fake_args = vec![bin_name.to_string()];
        fake_args.extend(arg_slices.iter().cloned());

        let opts = match ToolOpts::from_iter_safe(fake_args) {
            Ok(opts) => opts,
            Err(err) => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::BadInput,
                    message: err.to_string(),
                }));
            }
        };
        //println!("Pipeline segment: {:?}", opts);

        if first_time {
            (server_kind, server) = match Url::parse(&opts.server) {
                Ok(url) => ("remote", Some(make_remote_server(url, &opts.tree)?)),
                Err(_) => ("local", Some(make_local_server(&opts.server, &opts.tree)?)),
            };
            output_format = Some(opts.output_format);
            first_time = false;
        }

        trace!(cmd = ?opts.cmd);

        match opts.cmd {
            Command::CrossrefLookup(cl) => {
                commands.push(Box::new(CrossrefLookupCommand { args: cl }))
            }

            Command::FilterAnalysis(fa) => {
                commands.push(Box::new(FilterAnalysisCommand { args: fa }));
            }

            Command::Graph(g) => {
                commands.push(Box::new(GraphCommand { args: g }));
            }

            Command::MergeAnalyses(ma) => {
                commands.push(Box::new(MergeAnalysesCommand{ args: ma }))
            }

            Command::ProductionFilter(pf) => {
                commands.push(Box::new(ProductionFilterCommand { args: pf }))
            }

            Command::Search(q) => {
                commands.push(Box::new(SearchCommand { args: q }))
            }

            Command::SearchIdentifiers(si) => {
                commands.push(Box::new(SearchIdentifiersCommand { args: si }))
            },

            Command::ShowHtml(sh) => {
                commands.push(Box::new(ShowHtmlCommand { args: sh }));
            }

            Command::Traverse(t) => {
                commands.push(Box::new(TraverseCommand { args: t }));
            }
        }
    }

    Ok((
        ServerPipeline {
            server_kind: server_kind.to_string(),
            server: server.unwrap(),
            commands,
        },
        output_format.unwrap(),
    ))
}
