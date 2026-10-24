//! Info boxes: the boxes atop source listings and rendered views (ex: a
//! test's skip conditions, an obsolete spec's successors), each its own
//! liquid template, `info_boxes/<name>.liquid`: mozsearch's (in
//! `config_defaults/info_boxes/`), unless the config repo's `info_boxes/` has
//! one of the same name, which overrides it (and an empty one leaves the box
//! out), and the config repo's others.  They're rendered in their names'
//! order with the same globals (see output-file's and
//! `format::format_rendered`'s): `tree`, `path`, `config`, `concise` and
//! `detailed` (the file's per-file info, for source listings), and `spec`
//! (the spec whose file it is, if it's one's; see `format::spec_globals`).
//!
//! Source listings' navigation panel sections from templates (ex: a test's
//! manifests and CI results, which its info box summarizes) are loaded the
//! same way, from `panel_sections/` (see `load_dir`), each rendering its whole
//! section (its heading too).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use lazy_static::lazy_static;
use liquid::Template;
use regex::Regex;
use tracing::error;

use super::builder::build_and_parse;
use crate::file_format::config::Config;

pub struct InfoBoxes {
    /// The templates, by their names, in order.
    templates: Vec<(String, Template)>,
}

impl InfoBoxes {
    /// The info boxes of `cfg`'s config repo (see the module docs).
    pub fn load(cfg: &Config) -> InfoBoxes {
        Self::load_dir(cfg, "info_boxes")
    }

    /// The templates of `dir` (ex: "panel_sections"), mozsearch's and the
    /// config repo's, the same way as info boxes'.
    pub fn load_dir(cfg: &Config, dir: &str) -> InfoBoxes {
        let dirs = [
            format!("{}/config_defaults/{}", cfg.mozsearch_path, dir),
            format!("{}/{}", cfg.config_repo_path, dir),
        ];
        let mut sources: BTreeMap<String, String> = BTreeMap::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("liquid") {
                    continue;
                }
                let (Some(name), Ok(source)) = (
                    path.file_stem().and_then(|name| name.to_str()),
                    std::fs::read_to_string(&path),
                ) else {
                    continue;
                };
                sources.insert(name.to_string(), source);
            }
        }
        InfoBoxes {
            templates: sources
                .into_iter()
                .filter(|(_, source)| !source.trim().is_empty())
                .map(|(name, source)| {
                    let template = build_and_parse(&source);
                    (name, template)
                })
                .collect(),
        }
    }

    /// The info boxes of `cfg`'s config repo, loaded once (for the web
    /// server).
    pub fn cached(cfg: &Config) -> Arc<InfoBoxes> {
        static CACHE: LazyLock<Mutex<HashMap<String, Arc<InfoBoxes>>>> =
            LazyLock::new(Default::default);
        let key = format!("{}\n{}", cfg.mozsearch_path, cfg.config_repo_path);
        CACHE
            .lock()
            .unwrap()
            .entry(key)
            .or_insert_with(|| Arc::new(InfoBoxes::load(cfg)))
            .clone()
    }

    /// The names of the boxes, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.templates.iter().map(|(name, _)| name.as_str())
    }

    /// The boxes, rendered with `globals`, in order (but those which fail to,
    /// which are logged).
    pub fn render(&self, globals: &liquid::Object) -> String {
        lazy_static! {
            static ref RE_WHITESPACE_CLEANUP: Regex = Regex::new(r#"(\n *)+\n"#).unwrap();
        }
        let rendered: String = self
            .templates
            .iter()
            .map(|(name, template)| {
                template.render(globals).unwrap_or_else(|e| {
                    error!("info box {} failed to render: {}", name, e);
                    String::new()
                })
            })
            .collect();
        // It's really difficult to get whitespace right in the templates right
        // now.  While it probably makes sense to just pass what we get from
        // this through a formatter in general, for now let's at least just use
        // this exciting regex to clean things up.
        RE_WHITESPACE_CLEANUP
            .replace_all(&rendered, "\n")
            .to_string()
    }

    /// Each template's rendering with `globals`, in order, but those which
    /// render nothing (or fail to, which are logged).
    pub fn render_each(&self, globals: &liquid::Object) -> Vec<String> {
        lazy_static! {
            static ref RE_WHITESPACE_CLEANUP: Regex = Regex::new(r#"(\n *)+\n"#).unwrap();
        }
        self.templates
            .iter()
            .filter_map(|(name, template)| {
                let rendered = template.render(globals).unwrap_or_else(|e| {
                    error!("{} failed to render: {}", name, e);
                    String::new()
                });
                let rendered = RE_WHITESPACE_CLEANUP.replace_all(rendered.trim(), "\n").to_string();
                (!rendered.is_empty()).then_some(rendered)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overrides() {
        let root = std::env::temp_dir().join(format!("info-boxes-test-{}", std::process::id()));
        let defaults = root.join("mozsearch/config_defaults/info_boxes");
        let repo = root.join("config/info_boxes");
        std::fs::create_dir_all(&defaults).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(defaults.join("a.liquid"), "A{{ path }}").unwrap();
        std::fs::write(defaults.join("b.liquid"), "B").unwrap();
        std::fs::write(defaults.join("c.liquid"), "C").unwrap();
        std::fs::write(defaults.join("notes.txt"), "not a box").unwrap();
        // (The config repo's override one, leave one out, and add one.)
        std::fs::write(repo.join("b.liquid"), "b{{ tree }}").unwrap();
        std::fs::write(repo.join("c.liquid"), "\n").unwrap();
        std::fs::write(repo.join("d.liquid"), "D").unwrap();
        // (One which fails to render, for these globals, is left out.)
        std::fs::write(repo.join("e.liquid"), "{{ concise.info.test }}").unwrap();
        let cfg = Config {
            trees: Default::default(),
            mozsearch_path: root.join("mozsearch").to_string_lossy().into_owned(),
            config_repo_path: root.join("config").to_string_lossy().into_owned(),
            url_map_path: None,
            doc_trees_path: None,
            tree_index_paths: Default::default(),
            config_file: String::new(),
            query_limits: Default::default(),
        };
        let boxes = InfoBoxes::load(&cfg);
        assert_eq!(boxes.names().collect::<Vec<_>>(), vec!["a", "b", "d", "e"]);
        let globals = liquid::object!({ "tree": "t", "path": "p" });
        assert_eq!(boxes.render(&globals), "ApbtD");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
