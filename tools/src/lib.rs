extern crate serde;
extern crate serde_json;

#[cfg(not(target_arch = "wasm32"))]
extern crate chrono;
#[cfg(not(target_arch = "wasm32"))]
extern crate clap;
#[cfg(not(target_arch = "wasm32"))]
extern crate git2;
#[cfg(not(target_arch = "wasm32"))]
extern crate include_dir;
#[cfg(not(target_arch = "wasm32"))]
extern crate itertools;
#[cfg(not(target_arch = "wasm32"))]
extern crate log;
#[cfg(not(target_arch = "wasm32"))]
#[macro_use]
extern crate lazy_static;
#[cfg(not(target_arch = "wasm32"))]
extern crate lexical_sort;
#[cfg(not(target_arch = "wasm32"))]
extern crate linkify;
#[cfg(not(target_arch = "wasm32"))]
extern crate liquid;
#[cfg(not(target_arch = "wasm32"))]
extern crate query_parser;
#[cfg(not(target_arch = "wasm32"))]
extern crate regex;
#[cfg(not(target_arch = "wasm32"))]
#[macro_use]
extern crate tracing;
#[cfg(not(target_arch = "wasm32"))]
extern crate tracing_subscriber;
#[cfg(not(target_arch = "wasm32"))]
extern crate uuid;

pub mod css_analyzer;
pub mod file_format;

#[cfg(not(target_arch = "wasm32"))]
pub mod abstract_server;
#[cfg(not(target_arch = "wasm32"))]
pub mod cmd_pipeline;
#[cfg(not(target_arch = "wasm32"))]
pub mod query;
#[cfg(not(target_arch = "wasm32"))]
pub mod templating;
#[cfg(not(target_arch = "wasm32"))]
pub mod tree_sitter_support;

#[cfg(not(target_arch = "wasm32"))]
pub mod blame;
#[cfg(not(target_arch = "wasm32"))]
pub mod describe;
#[cfg(not(target_arch = "wasm32"))]
pub mod diagnostics;
#[cfg(not(target_arch = "wasm32"))]
pub mod doc_trees_handler;
#[cfg(not(target_arch = "wasm32"))]
pub mod file_utils;
#[cfg(not(target_arch = "wasm32"))]
pub mod format;
#[cfg(not(target_arch = "wasm32"))]
pub mod git_ops;
#[cfg(not(target_arch = "wasm32"))]
pub mod glob_helper;
#[cfg(not(target_arch = "wasm32"))]
pub mod languages;
#[cfg(not(target_arch = "wasm32"))]
pub mod links;
#[cfg(not(target_arch = "wasm32"))]
pub mod logging;
#[cfg(not(target_arch = "wasm32"))]
pub mod output;
#[cfg(not(target_arch = "wasm32"))]
mod symbol_graph_edge_kind;
#[cfg(not(target_arch = "wasm32"))]
pub mod tokenize;
#[cfg(not(target_arch = "wasm32"))]
pub mod url_encode_path;
#[cfg(not(target_arch = "wasm32"))]
pub mod url_map_handler;

mod utils;
