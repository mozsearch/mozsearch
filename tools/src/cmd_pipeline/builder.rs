use crate::{
    cmd_pipeline::{
        cmd_prod_filter::ProductionFilterCommand, cmd_query::QueryCommand,
        cmd_search_text::SearchTextCommand, PipelineCommand,
    },
    structopt::StructOpt,
};
use tracing::{trace, trace_span};
use url::Url;

use crate::{
    abstract_server::{
        make_local_server, make_remote_server, ErrorDetails, ErrorLayer, Result, ServerError,
    },
    cmd_pipeline::parser::{Command, OutputFormat, ToolOpts},
};

use super::{cmd_search::SearchCommand, cmd_search_files::SearchFilesCommand, cmd_crossref_expand::CrossrefExpandCommand};
use super::cmd_show_html::ShowHtmlCommand;
use super::cmd_traverse::TraverseCommand;
use super::{
    cmd_crossref_lookup::CrossrefLookupCommand, cmd_filter_analysis::FilterAnalysisCommand,
    cmd_graph::GraphCommand, cmd_merge_analyses::MergeAnalysesCommand,
    cmd_search_identifiers::SearchIdentifiersCommand,
};

use super::interface::ServerPipeline;

pub fn fab_command_from_opts(opts: ToolOpts) -> Result<Box<dyn PipelineCommand>> {
    match opts.cmd {
        Command::CrossrefExpand(ce) => Ok(Box::new(CrossrefExpandCommand { args: ce })),

        Command::CrossrefLookup(cl) => Ok(Box::new(CrossrefLookupCommand { args: cl })),

        Command::FilterAnalysis(fa) => Ok(Box::new(FilterAnalysisCommand { args: fa })),

        Command::Graph(g) => Ok(Box::new(GraphCommand { args: g })),

        Command::MergeAnalyses(ma) => Ok(Box::new(MergeAnalysesCommand { args: ma })),

        Command::ProductionFilter(pf) => Ok(Box::new(ProductionFilterCommand { args: pf })),

        Command::Query(q) => Ok(Box::new(QueryCommand { args: q })),

        Command::Search(q) => Ok(Box::new(SearchCommand { args: q })),

        Command::SearchFiles(sf) => Ok(Box::new(SearchFilesCommand { args: sf })),

        Command::SearchIdentifiers(si) => Ok(Box::new(SearchIdentifiersCommand { args: si })),

        Command::SearchText(st) => Ok(Box::new(SearchTextCommand { args: st })),

        Command::ShowHtml(sh) => Ok(Box::new(ShowHtmlCommand { args: sh })),

        Command::Traverse(t) => Ok(Box::new(TraverseCommand { args: t })),
    }
}

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
            output_format = Some(opts.output_format.clone());
            first_time = false;
        }

        trace!(cmd = ?opts.cmd);
        commands.push(fab_command_from_opts(opts)?);
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

/*
pub fn build_pipeline_graph(query: QueryPipelineGroupBuilder) -> ServerPipelineGraph {

}
*/
