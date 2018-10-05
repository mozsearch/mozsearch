#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate rustc_serialize;
extern crate git2;
extern crate regex;
extern crate chrono;
extern crate linkify;

pub mod file_format;

pub mod config;
pub mod blame;
pub mod output;
pub mod languages;
pub mod format;
pub mod tokenize;
pub mod links;

pub fn find_source_file(path: &str, files_root: &str, objdir: &str) -> String {
    if path.starts_with("__GENERATED__") {
        return path.replace("__GENERATED__", objdir);
    }
    format!("{}/{}", files_root, path)
}
