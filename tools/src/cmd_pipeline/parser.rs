use clap::arg_enum;
use structopt::StructOpt;

use super::cmd_crossref_lookup::CrossrefLookup;
use super::cmd_filter_analysis::FilterAnalysis;
use super::cmd_graph::Graph;
use super::cmd_merge_analyses::MergeAnalyses;
use super::cmd_prod_filter::ProductionFilter;
use super::cmd_search::Search;
use super::cmd_search_identifiers::SearchIdentifiers;
use super::cmd_show_html::ShowHtml;
use super::cmd_traverse::Traverse;

arg_enum! {
    #[derive(Debug, PartialEq)]
    pub enum OutputFormat {
        // Pretty-printed JSON.
        Pretty,
        // Un-pretty-printed JSON.
        Concise,
    }
}

#[derive(Debug, StructOpt)]
pub struct ToolOpts {
    /// URL of the server to query or the path to the root of the index tree if
    /// using local data.
    #[structopt(
        long,
        default_value = "https://searchfox.org/",
        env = "SEARCHFOX_SERVER"
    )]
    pub server: String,

    /// The name of the indexed tree to use.
    #[structopt(long, default_value = "mozilla-central", env = "SEARCHFOX_TREE")]
    pub tree: String,

    #[structopt(long, short, possible_values = &OutputFormat::variants(), case_insensitive = true, default_value = "concise")]
    pub output_format: OutputFormat,

    #[structopt(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, StructOpt)]
pub enum Command {
    CrossrefLookup(CrossrefLookup),
    FilterAnalysis(FilterAnalysis),
    Graph(Graph),
    MergeAnalyses(MergeAnalyses),
    ProductionFilter(ProductionFilter),
    Search(Search),
    SearchIdentifiers(SearchIdentifiers),
    ShowHtml(ShowHtml),
    Traverse(Traverse),
}
