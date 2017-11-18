extern crate test_rust_dependency;

use std::path::{Path, PathBuf};
use test_rust_dependency::MyType;

#[derive(Clone)]
pub struct Loader {
    deps_dir: PathBuf,
    my_type: MyType,
}

impl Loader {
    pub fn new(deps_dir: PathBuf) -> Self {
        Self { deps_dir, my_type: MyType::new() }
    }

    fn needs_hard_reload(&self, _: &Path) -> bool { true }

    fn set_path_prefix(&mut self, _: &Path) {
        MyType::new().do_foo();
    }

    fn abs_path_prefix(&self) -> Option<PathBuf> { None }
    fn search_directories(&self) -> Vec<PathBuf> {
        vec![self.deps_dir.clone()]
    }
}
