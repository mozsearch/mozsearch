use clap::{Parser, Subcommand, ValueEnum};

use super::cmd_augment_results::AugmentResults;
use super::cmd_batch_render::BatchRender;
use super::cmd_cat_html::CatHtml;
use super::cmd_compile_results::CompileResults;
use super::cmd_crossref_expand::CrossrefExpand;
use super::cmd_crossref_lookup::CrossrefLookup;
use super::cmd_filter_analysis::FilterAnalysis;
use super::cmd_format_symbols::FormatSymbols;
use super::cmd_fuse_crossrefs::FuseCrossrefs;
use super::cmd_graph::Graph;
use super::cmd_jumpref_lookup::JumprefLookup;
use super::cmd_merge_analyses::MergeAnalyses;
use super::cmd_prod_filter::ProductionFilter;
use super::cmd_query::Query;
use super::cmd_render::Render;
use super::cmd_search::Search;
use super::cmd_search_files::SearchFiles;
use super::cmd_search_identifiers::SearchIdentifiers;
use super::cmd_search_text::SearchText;
use super::cmd_show_html::ShowHtml;
use super::cmd_tokenize_source::TokenizeSource;
use super::cmd_traverse::Traverse;

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum OutputFormat {
    // Pretty-printed JSON.
    Pretty,
    // Un-pretty-printed JSON.
    Concise,
}

#[derive(Debug, Parser)]
pub struct ToolOpts {
    /// URL of the server to query or the path to the root of the index tree if
    /// using local data.
    #[clap(
        long,
        value_parser,
        default_value = "https://searchfox.org/",
        env = "SEARCHFOX_SERVER"
    )]
    pub server: String,

    /// The name of the indexed tree to use.
    #[clap(
        long,
        value_parser,
        default_value = "mozilla-central",
        env = "SEARCHFOX_TREE"
    )]
    pub tree: String,

    #[clap(
        long,
        short,
        value_parser,
        value_enum,
        default_value = "concise"
    )]
    pub output_format: OutputFormat,

    #[clap(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    AugmentResults(AugmentResults),
    BatchRender(BatchRender),
    CatHtml(CatHtml),
    CrossrefExpand(CrossrefExpand),
    CrossrefLookup(CrossrefLookup),
    FilterAnalysis(FilterAnalysis),
    FormatSymbols(FormatSymbols),
    Graph(Graph),
    JumprefLookup(JumprefLookup),
    MergeAnalyses(MergeAnalyses),
    ProductionFilter(ProductionFilter),
    Query(Query),
    Render(Render),
    Search(Search),
    SearchFiles(SearchFiles),
    SearchIdentifiers(SearchIdentifiers),
    SearchText(SearchText),
    ShowHtml(ShowHtml),
    TokenizeSource(TokenizeSource),
    Traverse(Traverse),
}

#[derive(Debug, Parser)]
pub struct JunctionOpts {
    #[structopt(subcommand)]
    pub cmd: JunctionCommand,
}

#[derive(Debug, Subcommand)]
pub enum JunctionCommand {
    CompileResults(CompileResults),
    FuseCrossrefs(FuseCrossrefs),
}
