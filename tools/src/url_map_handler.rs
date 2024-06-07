use crate::file_format::url_map::{ URLMap, URLMapItem, read_url_map };
use crate::file_format::analysis_manglings::mangle_file;

static mut URL_MAP_PATH: Option<String> = None;

pub fn set_url_map_path(path: &str) {
    unsafe {
        URL_MAP_PATH = Some(path.to_string());
    }
}

pub fn get_file_paths_for_url(url: &str) -> Option<Vec<URLMapItem>> {
    lazy_static! {
        static ref URL_MAP: URLMap = unsafe {
            read_url_map(URL_MAP_PATH.as_ref().map(|s| s.as_str()))
        };
    }

    let sym = format!("URL_{}", mangle_file(url));
    return URL_MAP.get(&sym)
}
