use std::path::Path;

use languages::FormatAs;

use regex::Regex;

/// Tries to find some text that describes the contents of the file.
/// Adapted from DXR code at https://github.com/mozilla/dxr/blob/master/dxr/plugins/descriptor/__init__.py
pub fn describe_file(contents: &str, path: &Path, format: &FormatAs) -> Option<String> {
    // Only look in the first 5k chars of the file, since running regex matches on giant files
    // can be expensive and most likely the thing we're looking for will be early in the file.
    let substr_end = contents.char_indices().nth(5000).map(|ix| ix.0).unwrap_or(contents.len());
    let substr = &contents[0..substr_end];

    // DXR also does a search for "filename: <description>" which I've never seen in any file so
    // I'm omitting that here. We can add it if needed.
    match format {
        FormatAs::FormatTagLike(_) => describe_html(substr),
        FormatAs::FormatCLike(ref spec) => {
            if spec.rust_tweaks {
                describe_from_rust_comment(substr)
                    .or_else(|| describe_from_c_comment(substr))
            } else if spec.c_style_comments {
                describe_from_c_comment(substr)
            } else if spec.triple_quote_literals {
                describe_py(substr)
            } else {
                None
            }
        },
        FormatAs::Binary => None,
        FormatAs::Plain => {
            let stem = path.file_stem()?.to_str()?;
            if stem.eq_ignore_ascii_case("README") {
                describe_readme(substr)
            } else {
                None
            }
        }
    }
}

/// Returns the content of the title tag
fn describe_html(contents: &str) -> Option<String> {
    lazy_static! {
        static ref TITLE_REGEX: Regex = {
            Regex::new(r#"(?i)<title>((?s).*?)</title>"#).unwrap()
        };
    }
    TITLE_REGEX.captures(contents)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Returns the first docstring (single-quoted preferred)
fn describe_py(contents: &str) -> Option<String> {
    lazy_static! {
        static ref DOCSTRING_SINGLE_REGEX: Regex = {
            Regex::new(r#"'''\s*((?s).*?)'''"#).unwrap()
        };
        static ref DOCSTRING_DOUBLE_REGEX: Regex = {
            Regex::new(r#""""\s*((?s).*?)""""#).unwrap()
        };
    }
    DOCSTRING_SINGLE_REGEX.captures(contents)
        .or_else(|| DOCSTRING_DOUBLE_REGEX.captures(contents))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Returns the first nonempty line
fn describe_readme(contents: &str) -> Option<String> {
    contents.lines()
        .filter(|s| !s.trim().is_empty())
        .next()
        .map(str::to_string)
}

/// Returns the first Rust-style module doc-comment (`//!`).
fn describe_from_rust_comment(contents: &str) -> Option<String> {
    let mut description = String::new();
    let mut in_comment = false;
    for line in contents.lines() {
        if !line.starts_with("//!") {
            if !in_comment {
                continue;
            }
            return Some(description);
        }
        in_comment = true;
        if !description.is_empty() {
            description.push_str("\n");
        }
        description.push_str(line.trim_start_matches("//!"));
    }
    None
}

/// Returns the first C-style comment that's not vim/modeline/license boilerplate
fn describe_from_c_comment(contents: &str) -> Option<String> {
    lazy_static! {
        // Matches C-style comments, including any leading '*' characters on
        // wrapped lines. The LEADING_STARS regex is used to remove those.
        static ref COMMENT_REGEX: Regex = {
            Regex::new(r#"(?:/\*[*\s]*)((?s).*?)\*/"#).unwrap()
        };
        static ref LEADING_STARS: Regex = {
            Regex::new(r#"(?m)^\s*\*"#).unwrap()
        };
    }
    for captures in COMMENT_REGEX.captures_iter(contents) {
        if let Some(comment_match) = captures.get(1) {
            let comment_text = comment_match.as_str();
            if comment_text.contains("tab-width") ||
                comment_text.contains("vim:") ||
                // Checking common case-variants is probably cheaper than lowercasing
                // comment_text (which allocates a new string) and doing a lowercase search
                comment_text.contains("license") ||
                comment_text.contains("LICENSE") ||
                comment_text.contains("License") {
                continue;
            }
            return Some(LEADING_STARS.replace_all(comment_text, "").into_owned());
        }
    }
    None
}
