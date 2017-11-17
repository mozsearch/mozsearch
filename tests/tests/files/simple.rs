

use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Loader {
    deps_dir: PathBuf,
}

impl Loader {
    pub fn new(deps_dir: PathBuf) -> Self {
        Self { deps_dir }
    }

    fn needs_hard_reload(&self, _: &Path) -> bool { true }

    fn set_path_prefix(&mut self, _: &Path) {}

    fn abs_path_prefix(&self) -> Option<PathBuf> { None }
    fn search_directories(&self) -> Vec<PathBuf> {
        vec![self.deps_dir.clone()]
    }
}
