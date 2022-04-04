#[macro_use]
extern crate lazy_static;
extern crate log;
extern crate clap;
extern crate chrono;
extern crate git2;
extern crate include_dir;
#[macro_use]
extern crate itertools;
extern crate linkify;
extern crate regex;
#[macro_use]
extern crate malloc_size_of_derive;
extern crate jemalloc_sys;
extern crate jemallocator;
extern crate liquid;
extern crate malloc_size_of;
extern crate query_parser;
extern crate serde;
extern crate serde_json;
extern crate structopt;
#[macro_use]
extern crate tracing;
extern crate tracing_subscriber;

pub mod abstract_server;
pub mod cmd_pipeline;
pub mod file_format;
pub mod query;
pub mod templating;

pub mod blame;
pub mod config;
pub mod describe;
pub mod format;
pub mod git_ops;
pub mod glob_helper;
pub mod languages;
pub mod links;
pub mod output;
pub mod tokenize;

#[global_allocator]
static A: jemallocator::Jemalloc = jemallocator::Jemalloc;

pub fn find_source_file(path: &str, files_root: &str, objdir: &str) -> String {
    if path.starts_with("__GENERATED__") {
        return path.replace("__GENERATED__", objdir);
    }
    format!("{}/{}", files_root, path)
}
