use regex::{Regex, Captures};

pub fn mangle_file(filename: &str) -> String {
    lazy_static! {
        // The column portion can potentially be singular I think so we just
        // treat the half as its own group.
        static ref RE: Regex = Regex::new(r"[^a-zA-z0-9_/]").unwrap();
    }
    RE.replace_all(filename, |c: &Captures| {
        format!("@{:02X}", c[0].as_bytes()[0])
    }).to_string()
}

pub fn make_file_sym_from_path(path: &str) -> String {
    format!("FILE_{}", mangle_file(path))
}

#[test]
fn test_mangle_file() {
    assert_eq!(mangle_file("path/foo.h"), "path/foo@2Eh");
    assert_eq!(make_file_sym_from_path("path/foo.h"), "FILE_path/foo@2Eh");
}
