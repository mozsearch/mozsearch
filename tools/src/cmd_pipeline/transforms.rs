use regex::{Captures, Regex};

/// Apply the searchfox path glob transformation ported from `router.py`.
pub fn path_glob_transform(s: &str) -> String {
    lazy_static! {
        static ref RE_TO_ESCAPE: Regex = Regex::new("[()|.]").unwrap();
        static ref RE_STARS: Regex = Regex::new(r"\*\*?").unwrap();
        static ref RE_BRACES: Regex = Regex::new(r"\{([^}]*)\}").unwrap();
    }
    let backslashed = RE_TO_ESCAPE.replace_all(s, r"\$0");
    let starred = RE_STARS.replace_all(&backslashed, |caps: &Captures| {
        if caps.get(0).unwrap().as_str().len() == 1 {
            "[^/]*"
        } else {
            ".*"
        }
    });
    let questioned = str::replace(&starred, "?", ".");
    let braced = RE_BRACES.replace_all(&questioned, |caps: &Captures| {
        let inside_braces = caps.get(1).unwrap().as_str();
        format!("({})", str::replace(inside_braces, ",", "|"))
    });
    braced.to_string()
}

/// The path regexp `pattern`, ignoring case unless `case_sensitive`: a
/// query's paths ignore case, as router.py's `/search/` does, but with
/// `case:sensitive` (see `SearchFiles`).
pub fn path_case(pattern: String, case_sensitive: bool) -> String {
    match case_sensitive || pattern.is_empty() {
        true => pattern,
        false => format!("(?i){}", pattern),
    }
}

#[test]
fn test_path_glob_transform() {
    // Test coverage for the cases we documented on the help page.
    assert_eq!(path_glob_transform("test"), "test");
    assert_eq!(path_glob_transform("^js/src"), "^js/src");

    assert_eq!(path_glob_transform("*.cpp"), "[^/]*\\.cpp");
    assert_eq!(path_glob_transform("*.cpp$"), "[^/]*\\.cpp$");

    assert_eq!(
        path_glob_transform("^js/src/*.cpp$"),
        "^js/src/[^/]*\\.cpp$"
    );
    assert_eq!(path_glob_transform("^js/src/**.cpp$"), "^js/src/.*\\.cpp$");
    assert_eq!(
        path_glob_transform("^js/src/**.{cpp,h}$"),
        "^js/src/.*\\.(cpp|h)$"
    );
}
