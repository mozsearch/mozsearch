#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate chrono;
extern crate git2;
extern crate linkify;
extern crate regex;
extern crate rustc_serialize;

pub mod file_format;

pub mod blame;
pub mod config;
pub mod describe;
pub mod format;
pub mod git_ops;
pub mod languages;
pub mod links;
pub mod output;
pub mod tokenize;

pub fn find_source_file(path: &str, files_root: &str, objdir: &str) -> String {
    if path.starts_with("__GENERATED__") {
        return path.replace("__GENERATED__", objdir);
    }
    format!("{}/{}", files_root, path)
}
