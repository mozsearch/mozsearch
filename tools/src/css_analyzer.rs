use cssparser;

use crate::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisTarget, LineRange, Location, SourceRange, SourceTag,
    TargetTag, WithLocation,
};

// NOTE: This does the same as analysis_manglings::mangle_file without regex
//       dependency.  regex increases the wasm file size by ~800kB.
fn mangle_name(name: &str) -> String {
    let mut s = String::new();

    for c in name.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'/' => {
                s.push(c as char);
            }
            _ => {
                s.push_str(format!("@{:02X}", { c }).as_str());
            }
        }
    }

    s
}

fn to_loc(
    first_line: u32,
    start: &cssparser::SourceLocation,
    end: &cssparser::SourceLocation,
) -> Location {
    // cssparser::SourceLocation uses 0-origin line and 1-origin column.
    // analysis::Location uses 1-origin line and 0-origin column.
    // first_line is 1-origin.
    Location {
        lineno: first_line + start.line,
        col_start: start.column - 1,
        col_end: end.column - 1,
    }
}

fn to_source(
    loc: Location,
    syntax: Vec<String>,
    pretty: String,
    sym: String,
) -> WithLocation<AnalysisSource<String>> {
    WithLocation {
        data: AnalysisSource {
            source: SourceTag::Source,
            syntax,
            pretty,
            sym: vec![sym],
            no_crossref: false,
            nesting_range: SourceRange::default(),
            type_pretty: None,
            type_sym: None,
            arg_ranges: vec![],
            expansion_info: None,
            confidence: None,
        },
        loc,
    }
}

fn to_target(
    loc: Location,
    kind: AnalysisKind,
    pretty: String,
    sym: String,
) -> WithLocation<AnalysisTarget<String>> {
    WithLocation {
        data: AnalysisTarget {
            target: TargetTag::Target,
            kind,
            pretty,
            sym,
            context: "".to_string(),
            contextsym: "".to_string(),
            peek_range: LineRange {
                start_lineno: 0,
                end_lineno: 0,
            },
            arg_ranges: vec![],
            slot: None,
            ctxtra: vec![],
            nameless: false,
        },
        loc,
    }
}

fn analyze_css_block<F>(
    input: &mut cssparser::Parser,
    first_line: u32,
    namespace: Option<&str>,
    is_curly_children: bool,
    is_inside_lhs: bool,
    callback: &mut F,
) where
    F: FnMut(String),
{
    use cssparser::Token::*;
    let mut start = input.current_source_location();
    let mut is_lhs = is_inside_lhs;
    let mut after_at_property = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        let end = input.current_source_location();
        let mut has_block = false;
        let mut is_curly = false;
        match token {
            Ident(name) => {
                if name.starts_with("--") {
                    let loc = to_loc(first_line, &start, &end);
                    let source_pretty = format!("property {}", name.as_ref());
                    let target_pretty = name.to_string();
                    let sym = custom_property_sym(namespace, name.as_ref());

                    let (syntax, kind) = if after_at_property {
                        after_at_property = false;
                        (
                            vec!["decl".to_string(), "cssprop".to_string()],
                            AnalysisKind::Decl,
                        )
                    } else if is_lhs {
                        (
                            vec!["def".to_string(), "cssprop".to_string()],
                            AnalysisKind::Def,
                        )
                    } else {
                        (
                            vec!["use".to_string(), "cssprop".to_string()],
                            AnalysisKind::Use,
                        )
                    };

                    let source = to_source(loc, syntax, source_pretty, sym.clone());
                    callback(serde_json::to_string(&source).unwrap());
                    let target = to_target(loc, kind, target_pretty, sym);
                    callback(serde_json::to_string(&target).unwrap());
                }
            }
            Colon => {
                if is_curly_children {
                    is_lhs = false;
                }
            }
            AtKeyword(name) => {
                if name == "property" {
                    after_at_property = true;
                }
            }
            Semicolon => {
                if is_curly_children {
                    is_lhs = true;
                }
            }
            QuotedString(s) | UnquotedUrl(s) => {
                let is_moz_src = s.starts_with("moz-src:///");
                if s.starts_with("chrome://") || s.starts_with("resource://") || is_moz_src {
                    let loc = to_loc(first_line, &start, &end);
                    let source_pretty = format!("file {}", s.as_ref());
                    let target_pretty = s.to_string();
                    let sym = if is_moz_src {
                        format!("FILE_{}", mangle_name(s.as_ref().get(11..).unwrap()))
                    } else {
                        format!("URL_{}", mangle_name(s.as_ref()))
                    };
                    let syntax = vec!["use".to_string(), "file".to_string()];
                    let kind = AnalysisKind::Use;

                    let source = to_source(loc, syntax, source_pretty, sym.clone());
                    callback(serde_json::to_string(&source).unwrap());
                    let target = to_target(loc, kind, target_pretty, sym);
                    callback(serde_json::to_string(&target).unwrap());
                }
            }
            CurlyBracketBlock => {
                has_block = true;
                is_curly = true;
            }
            Function(_) | ParenthesisBlock | SquareBracketBlock => {
                has_block = true;
            }
            _ => {}
        };

        if has_block {
            let mut is_child_lhs = false;
            if is_curly {
                is_child_lhs = true;
            }
            let _: Result<(), cssparser::ParseError<()>> = input.parse_nested_block(|input| {
                analyze_css_block(
                    input,
                    first_line,
                    namespace,
                    is_curly,
                    is_child_lhs,
                    callback,
                );
                Ok(())
            });
        }

        start = end;
    }
}

/// A custom property's symbol: `CSSPROP_--name` (mangled), or in a
/// namespace (see `analyze_css`), `CSSPROP_<namespace>/--name` (mangled, the
/// namespace keeping its `/`s, as a global one has none).
pub fn custom_property_sym(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(namespace) => format!("{}{}", custom_property_prefix(namespace), mangle_name(name)),
        None => format!("CSSPROP_{}", mangle_name(name)),
    }
}

/// A namespace's custom properties' symbols' prefix (see
/// `custom_property_sym`), which their mangled names follow.
pub fn custom_property_prefix(namespace: &str) -> String {
    format!("CSSPROP_{}/", mangle_name(namespace))
}

/// The namespace of a file's custom properties: its own path for a file of
/// a tree's tests' (under `tests_root`, ex: web-platform-tests'
/// `testing/web-platform`), as each of their documents' custom properties
/// are its own (and not those of the Firefox UI, whose share a namespace).
pub fn document_namespace<'a>(path: &'a str, tests_root: Option<&str>) -> Option<&'a str> {
    let root = tests_root.filter(|r| !r.is_empty())?;
    path.strip_prefix(root)
        .is_some_and(|rest| rest.starts_with('/'))
        .then_some(path)
}

/// A file's CSS analysis: its custom properties' (in `namespace`, if any;
/// see `custom_property_sym`) and its URLs' uses.
pub fn analyze_css<F>(
    path: String,
    first_line: u32,
    text: String,
    namespace: Option<&str>,
    callback: &mut F,
) where
    F: FnMut(String),
{
    let mut input = cssparser::ParserInput::new(text.as_str());
    let mut input = cssparser::Parser::new(&mut input);

    if !path.is_empty() {
        let loc = Location {
            lineno: 1,
            col_start: 0,
            col_end: 0,
        };
        let pretty = format!("file {}", path);
        let sym = format!("FILE_{}", mangle_name(path.as_str()));
        let kind = AnalysisKind::Def;
        let target = to_target(loc, kind, pretty, sym);

        callback(serde_json::to_string(&target).unwrap());
    }

    analyze_css_block(&mut input, first_line, namespace, false, false, callback);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_property_namespaces() {
        let root = Some("testing/web-platform");
        assert_eq!(
            document_namespace("testing/web-platform/tests/css/a.html", root),
            Some("testing/web-platform/tests/css/a.html")
        );
        assert_eq!(
            document_namespace("testing/web-platform-x/a.html", root),
            None
        );
        assert_eq!(document_namespace("browser/themes/a.css", root), None);
        assert_eq!(
            document_namespace("testing/web-platform/a.html", None),
            None
        );
        let syms = |namespace: Option<&str>| -> Vec<String> {
            let mut lines = vec![];
            analyze_css(
                "".to_string(),
                1,
                ":root { --gap: 1px; } p { margin: var(--gap); }".to_string(),
                namespace,
                &mut |line| lines.push(line),
            );
            lines
                .iter()
                .filter(|l| l.contains("\"target\""))
                .map(|l| {
                    let value: serde_json::Value = serde_json::from_str(l).unwrap();
                    format!(
                        "{} {}",
                        value["kind"].as_str().unwrap(),
                        value["sym"].as_str().unwrap()
                    )
                })
                .collect()
        };
        assert_eq!(
            syms(None),
            vec!["def CSSPROP_@2D@2Dgap", "use CSSPROP_@2D@2Dgap"]
        );
        assert_eq!(
            syms(Some("t/a.html")),
            vec![
                "def CSSPROP_t/a@2Ehtml/@2D@2Dgap",
                "use CSSPROP_t/a@2Ehtml/@2D@2Dgap"
            ]
        );
    }
}
