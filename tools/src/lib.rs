#[macro_use]
extern crate lazy_static;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate tracing;

#[cfg(not(target_arch = "wasm32"))]
extern crate log;
#[cfg(not(target_arch = "wasm32"))]
extern crate clap;
#[cfg(not(target_arch = "wasm32"))]
extern crate chrono;
#[cfg(not(target_arch = "wasm32"))]
extern crate git2;
#[cfg(not(target_arch = "wasm32"))]
extern crate include_dir;
#[cfg(not(target_arch = "wasm32"))]
extern crate itertools;
#[cfg(not(target_arch = "wasm32"))]
extern crate linkify;
#[cfg(not(target_arch = "wasm32"))]
extern crate regex;
#[cfg(not(target_arch = "wasm32"))]
extern crate lexical_sort;
#[cfg(not(target_arch = "wasm32"))]
extern crate liquid;
#[cfg(not(target_arch = "wasm32"))]
extern crate query_parser;
#[cfg(not(target_arch = "wasm32"))]
extern crate tracing_subscriber;
#[cfg(not(target_arch = "wasm32"))]
extern crate uuid;

pub mod file_format;
pub mod css_analyzer;

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
pub mod tokenize;

mod symbol_graph_edge_kind;
