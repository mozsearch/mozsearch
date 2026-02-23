use std::{
    env, fs,
    ops::Deref,
    path::{Path, PathBuf},
};

#[allow(unused)]
pub struct TempDir(PathBuf);

#[allow(unused)]
impl TempDir {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let path = env::temp_dir().join(path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Deref for TempDir {
    type Target = PathBuf;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
