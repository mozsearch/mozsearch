use crate::file_format::config::Config;
use crate::file_format::doc_trees::{read_doc_trees, DocTrees};
use std::sync::OnceLock;

pub fn find_doc_url(cfg: &Config, src_path: &str) -> Option<String> {
    static DOC_TREES: OnceLock<DocTrees> = OnceLock::new();

    if DOC_TREES.get().is_none() {
        DOC_TREES
            .set(match &cfg.doc_trees_path {
                Some(doc_trees_path) => read_doc_trees(doc_trees_path),
                None => DocTrees::new_empty(),
            })
            .unwrap();
    }

    match DOC_TREES.get().unwrap().find(src_path) {
        // TODO: Make the URL configurable.
        Some(target_path) => {
            Some("https://firefox-source-docs.mozilla.org/".to_string() + target_path.as_str())
        }
        None => None,
    }
}
