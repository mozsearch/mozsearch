use std::borrow::Cow;
use urlencoding;

pub fn url_encode_path(path: &str) -> String {
    path.split('/')
        .map(|p| urlencoding::encode(p))
        .collect::<Vec<Cow<'_, str>>>()
        .join("/")
}

pub fn url_decode_path(path: &str) -> String {
    match urlencoding::decode(path) {
        Ok(s) => s.to_string(),
        Err(_) => path.to_string(),
    }
}
