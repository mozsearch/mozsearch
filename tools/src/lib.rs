#[macro_use]
extern crate lazy_static;
extern crate rustc_serialize;
extern crate git2;
extern crate regex;
extern crate chrono;

pub mod analysis;
pub mod config;
pub mod blame;
pub mod output;
pub mod languages;
pub mod format;
pub mod tokenize;

pub fn find_source_file(path: &str, tree_root: &str, objdir: &str) -> String {
    if path.starts_with("__GENERATED__") {
        return path.replace("__GENERATED__", objdir);
    }
    format!("{}/{}", tree_root, path)
}
