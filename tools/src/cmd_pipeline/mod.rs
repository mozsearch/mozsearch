extern crate clap;
extern crate structopt;

pub mod builder;
pub mod interface;
pub mod parser;
pub mod symbol_graph;

mod cmd_crossref_lookup;
mod cmd_filter_analysis;
mod cmd_graph;
mod cmd_merge_analyses;
mod cmd_prod_filter;
mod cmd_query;
mod cmd_search_identifiers;
mod cmd_show_html;
mod cmd_traverse;

pub use builder::{build_pipeline};
pub use interface::{PipelineCommand, PipelineValues};
