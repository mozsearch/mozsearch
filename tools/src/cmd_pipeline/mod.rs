extern crate clap;
extern crate structopt;

pub mod builder;
pub mod interface;
pub mod parser;

mod cmd_filter_analysis;
mod cmd_show_html;

pub use builder::{build_pipeline};
pub use interface::{PipelineCommand, PipelineValues};
