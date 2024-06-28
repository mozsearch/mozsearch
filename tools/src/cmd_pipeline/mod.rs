extern crate clap;

pub mod builder;
pub mod interface;
pub mod parser;
pub mod symbol_graph;
pub mod transforms;

mod cmd_augment_results;
mod cmd_cat_html;
mod cmd_batch_render;
mod cmd_compile_results;
mod cmd_crossref_expand;
mod cmd_crossref_lookup;
mod cmd_filter_analysis;
mod cmd_format_symbols;
mod cmd_fuse_crossrefs;
mod cmd_graph;
mod cmd_jumpref_lookup;
mod cmd_merge_analyses;
mod cmd_prod_filter;
mod cmd_query;
mod cmd_render;
mod cmd_search;
mod cmd_search_files;
mod cmd_search_identifiers;
mod cmd_search_text;
mod cmd_show_html;
mod cmd_tokenize_source;
mod cmd_traverse;
mod cmd_webtest;

pub use builder::{build_pipeline};
pub use interface::{PipelineCommand, PipelineValues};
