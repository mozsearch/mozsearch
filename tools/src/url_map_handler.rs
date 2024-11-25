use crate::file_format::analysis_manglings::mangle_file;
use crate::file_format::config::Config;
use crate::file_format::url_map::{read_url_map, URLMap, URLMapItem};
use std::sync::OnceLock;

pub fn get_file_paths_for_url(cfg: Option<&Config>, url: &str) -> Option<Vec<URLMapItem>> {
    static URL_MAP: OnceLock<URLMap> = OnceLock::new();

    if URL_MAP.get().is_none() {
        URL_MAP
            .set(match cfg {
                Some(cfg) => match &cfg.url_map_path {
                    Some(url_map_path) => read_url_map(url_map_path),
                    None => URLMap::new_empty(),
                },
                None => URLMap::new_empty(),
            })
            .unwrap();
    }

    let url_map_key = format!("URL_{}", mangle_file(url));
    URL_MAP.get().unwrap().get(&url_map_key)
}
