use urlencoding;
use std::borrow::Cow;

pub fn url_encode_path(path: &str) -> String {
    path
        .split('/')
        .map(|p| urlencoding::encode(p))
        .collect::<Vec<Cow<'_, str>>>()
        .join("/")
}
