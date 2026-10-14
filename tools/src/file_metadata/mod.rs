//! A file's metadata from the rules a tree declares it by, with the rules'
//! positions, so that the metadata can link to the rules: the Bugzilla
//! component of moz.build files' `Files` rules (see `mozbuild`), the review
//! guidance of `review-context.toml`'s rules (see `review_context`), and the
//! conditions of test manifests' sections (see `test_manifests`).
//! `metadata-analyze` writes them for `per-file-info.toml`'s ingestion into
//! the detailed per-file info.

pub mod mozbuild;
pub mod review_context;
pub mod test_manifests;

use regex::Regex;

/// The 1-based lines of a text's byte offsets.
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { starts }
    }

    pub fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset)
    }
}

/// mozbuild's `mozpack.path.match`'s pattern as a regex: `*` matches within
/// a directory, `**` across directories (`**/` also none), and a pattern
/// matching one of a path's ancestor directories matches the path.
pub fn mozpath_regex(pattern: &str) -> Regex {
    lazy_static! {
        static ref STAR_STAR: Regex = Regex::new(r"(^|/)\\\*\\\*/").unwrap();
        static ref STAR_STAR_END: Regex = Regex::new(r"(^|/)\\\*\\\*$").unwrap();
    }
    let escaped = regex::escape(pattern);
    let p = STAR_STAR.replace_all(&escaped, "${1}(?:.+/)?");
    let p = STAR_STAR_END.replace_all(&p, "(?:${1}.+)?");
    let p = p.replace(r"\*", "[^/]*");
    Regex::new(&format!("^{}(?:/.*)?$", p)).unwrap()
}

/// Python's `fnmatch.translate`'s regex for a `fnmatch.fnmatchcase` pattern,
/// where `*` also matches `/`: `*`, `?`, and `[...]` classes (`[!...]`
/// negated), else literal.
pub fn fnmatch_regex(pattern: &str) -> Regex {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::from("^(?s:");
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '*' => {
                // (Consecutive stars are one.)
                while i < chars.len() && chars[i] == '*' {
                    i += 1;
                }
                out.push_str(".*");
            }
            '?' => out.push('.'),
            '[' => {
                // The class runs to the next "]", not counting a "]" right
                // after the "[" (or "[!"); without one, "[" is literal.
                let mut j = i;
                if j < chars.len() && chars[j] == '!' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ']' {
                    j += 1;
                }
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j >= chars.len() {
                    out.push_str(r"\[");
                    continue;
                }
                let mut class: String = chars[i..j].iter().collect();
                i = j + 1;
                let negated = class.starts_with('!');
                if negated {
                    class.remove(0);
                }
                let escaped: String = class
                    .chars()
                    .map(|c| match c {
                        '\\' | '[' | ']' | '^' | '&' | '~' => format!("\\{}", c),
                        c => c.to_string(),
                    })
                    .collect();
                out.push('[');
                if negated {
                    out.push('^');
                }
                out.push_str(&escaped);
                out.push(']');
            }
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out.push_str(")$");
    Regex::new(&out).unwrap_or_else(|_| Regex::new("^$").unwrap())
}

/// A line's code and its comment, after a `#` outside of (TOML or Python)
/// strings.
pub fn split_comment(line: &str) -> (&str, Option<&str>) {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => return (&line[..i], Some(line[i + 1..].trim())),
                _ => {}
            },
        }
    }
    (line, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mozpath_regex() {
        let m = |path: &str, pattern: &str| mozpath_regex(pattern).is_match(path);
        assert!(m("foo", "*"));
        assert!(m("foo", "f*"));
        assert!(m("foo", "fo*o"));
        assert!(!m("foo/bar", "foo/*/bar"));
        assert!(m("foo/bar", "foo"));
        assert!(m("foo/bar", "foo/**/bar"));
        assert!(m("foo/bar", "**/bar"));
        assert!(m("a/b/c.cpp", "**/*.cpp"));
        assert!(m("c.cpp", "**/*.cpp"));
        assert!(m("bar/x/y", "bar/**"));
        assert!(!m("barx/y", "bar/**"));
        assert!(m("a.b", "a.b"));
        assert!(!m("axb", "a.b"));
    }

    #[test]
    fn test_fnmatch_regex() {
        let m = |path: &str, pattern: &str| fnmatch_regex(pattern).is_match(path);
        assert!(m("dom/media/webaudio/AudioNode.cpp", "dom/media/**"));
        assert!(m("dom/media/x", "dom/media/*"));
        assert!(m("a/test/b.js", "**/test/**"));
        assert!(!m("test/b.js", "**/test/**"));
        assert!(m("x.h", "?.[ch]"));
        assert!(!m("x.c", "?.[!c]"));
        assert!(m("[x", "[x"));
        assert!(!m("Dom/x", "dom/*"));
    }

    #[test]
    fn test_split_comment() {
        assert_eq!(split_comment(r#"  "a # b", # c"#), (r#"  "a # b", "#, Some("c")));
        assert_eq!(split_comment(r#""it's" # x"#).1, Some("x"));
        assert_eq!(split_comment("x = 1").1, None);
    }

    #[test]
    fn test_line_index() {
        let index = LineIndex::new("a\nbc\n\nd");
        assert_eq!(
            [0, 1, 2, 4, 5, 6].map(|o| index.line(o)),
            [1, 1, 2, 2, 3, 4]
        );
    }
}
