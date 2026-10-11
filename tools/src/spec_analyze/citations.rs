//! Citations of web specs in a tree's other files, ex: Gecko's comments
//! citing the HTML Standard's algorithms, and the steps they implement:
//!
//! ```cpp
//! // https://html.spec.whatwg.org/#navigate
//! nsresult nsDocShell::InternalLoad(...) {
//!   // Step 12.6. If ...
//! ```
//!
//! - Each URL with an anchor to one of the specs of the tree's `specs.toml`
//!   (or of its linked spec tree's) is a `doc-use` of the symbol it names
//!   (see `SpecsConfig::url_symbol`), wherever it is in the file (comments,
//!   or strings, ex: error messages).
//! - Each step comment ("Step 12.6.", "Steps 1-3:") in the scope of a line
//!   citing a spec is a `doc-use` of the same symbol, whose context has the
//!   step (ex: "nsDocShell::InternalLoad step 12.6").  A citing line's scope
//!   is the definition it precedes (one with a nesting range, in the file's
//!   other analysis, named within a few lines of its next line of code, ex:
//!   after a return type), or else the rest of the innermost definition it's
//!   in, or else, without nesting ranges, the rest of its indentation's block
//!   (see `indent_scope_end`).  A
//!   step is in the scope of the last citing line before it whose scope it's
//!   in, so later citations take over, and nested ones end with their scopes.
//! - Records' contexts are the definitions their scopes are of.

use std::ops::Range;
use std::path::Path;

use lazy_static::lazy_static;
use regex::Regex;

use super::{Lines, SpecsConfig, spec_symbol};
use crate::file_format::analysis::{
    AnalysisKind, read_analysis, read_source, read_target,
};
use crate::semantic_analyzer::{Record, sort_records, source_record, target_record};

lazy_static! {
    static ref URL: Regex = Regex::new(r#"https?://[^\s"'<>`\\{}|^]+"#).unwrap();
    /// A comment's step, at its start (after the comment's markers), ex:
    /// "Step 12.6.", "Steps 3.a-c", "Step 4 and 5:".
    static ref STEP: Regex = Regex::new(
        r"(?i)^[ \t]*(?:/{2,}|/\*+|\*+|#+|--|<!--)?[!<]?[ \t]*\(?(steps?[ \t]+(?P<path>\d+(?:\.(?:\d+|[a-z]{1,4}))*(?:[ \t]*(?:-|–|to|through|and|,)[ \t]*(?:\d+|[a-z]{1,4})(?:\.(?:\d+|[a-z]{1,4}))*)*))"
    )
    .unwrap();
}

/// A definition in a file's other analysis with a nesting range: the line of
/// its name, its nesting range's lines (1-based, `end` exclusive), and its
/// pretty name and symbol, which records in its scope have as their context.
#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    pub line: u32,
    pub lines: Range<u32>,
    pub pretty: String,
    pub sym: String,
}

/// The definitions with nesting ranges in an analysis file, if it exists.
pub fn analysis_defs(analysis_path: &Path) -> Vec<Def> {
    let Some(path) = analysis_path.to_str() else {
        return vec![];
    };
    if !analysis_path.exists() {
        return vec![];
    }
    let sources = read_analysis(path, &mut read_source);
    let targets = read_analysis(path, &mut read_target);
    let mut defs = vec![];
    for source in sources {
        for data in &source.data {
            let range = &data.nesting_range;
            if range.is_empty() || !data.syntax.iter().any(|s| s.as_str() == "def") {
                continue;
            }
            // (The target record of the definition, for its pretty name.)
            let Some(target) = targets
                .iter()
                .filter(|t| t.loc == source.loc)
                .flat_map(|t| &t.data)
                .find(|t| t.kind == AnalysisKind::Def && data.sym.contains(&t.sym))
            else {
                continue;
            };
            defs.push(Def {
                line: source.loc.lineno,
                lines: range.start_lineno.min(source.loc.lineno)..range.end_lineno + 1,
                pretty: target.pretty.to_string(),
                sym: target.sym.to_string(),
            });
        }
    }
    defs
}

/// The analysis's counts, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    /// URLs to the specs with anchors.
    pub citations: usize,
    /// Step comments in their scopes.
    pub steps: usize,
}

/// A line citing a spec: its first citation's symbol and pretty name, and its
/// scope's last line and definition.
struct Citing {
    line: u32,
    sym: String,
    pretty: String,
    scope_end: u32,
    def: Option<Def>,
}

/// Whether a line is (part of) a comment, by its start, in the usual
/// languages' syntaxes.
fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "/*", "*", "#", "<!--", "--", "\"\"\"", "'''"]
        .iter()
        .any(|marker| line.starts_with(marker))
        || line.trim_end().ends_with("*/")
}

/// Statements' keywords, whose blocks a comment before them is in rather than
/// about (ex: an algorithm's steps in a function's body, around an `if`).
const CONTROL: &[&str] = &[
    "if", "else", "elif", "for", "foreach", "while", "do", "switch", "match", "loop", "try",
    "catch", "with", "case", "unless", "until",
];

/// A line's indentation, in columns (tabs to 8).
fn indent(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 8 } else { 1 })
        .sum()
}

/// A URL's end, without trailing punctuation that's the text's (ex: a
/// sentence's period, a parenthetical's or a block comment's end).
fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let trimmed = url
            .trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '\'', '"'])
            .trim_end_matches("*/");
        let trimmed = match trimmed.strip_suffix([')', ']']) {
            // (Unless it's the URL's own, ex: an anchor's `(` ... `)`.)
            Some(inner) if inner.matches(['(', '[']).count() <= inner.matches([')', ']']).count() => {
                inner
            }
            _ => trimmed,
        };
        if trimmed.len() == url.len() {
            return url;
        }
        url = trimmed;
    }
}

/// The last line (1-based) of the scope of the citing line at `line` (by
/// index) without nesting ranges: if the code it precedes opens a block within
/// a few lines at its indentation (ex: a function's body, after a return type
/// and a signature, but not a statement's, like an `if`'s), that block,
/// through its closing line; or else (ex: statements in a function's body) the
/// rest of the block it's in, until a line indented less (or at the top
/// level, a blank line).
fn indent_scope_end(lines: &[&str], line: usize) -> u32 {
    let level = indent(lines[line]);
    let blank = |l: &str| l.trim().is_empty();
    let Some(code) = (line + 1..lines.len()).find(|&l| !blank(lines[l]) && !is_comment(lines[l]))
    else {
        return lines.len() as u32;
    };
    let keyword: String = lines[code]
        .trim_start()
        .trim_start_matches('}')
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let next_code = |from: usize| (from..lines.len()).find(|&l| !blank(lines[l]));
    // (The line opening the block, if the code does.)
    let mut opening = None;
    if indent(lines[code]) <= level && !CONTROL.contains(&keyword.as_str()) {
        let mut at = Some(code);
        for _ in 0..4 {
            let Some(l) = at.filter(|&l| indent(lines[l]) <= level) else {
                break;
            };
            let next = next_code(l + 1);
            if lines[l].trim_end().ends_with(['{', ':'])
                || next.is_some_and(|n| indent(lines[n]) > level)
            {
                opening = Some(l);
                break;
            }
            at = next;
        }
    }
    if let Some(opening) = opening {
        for l in opening + 1..lines.len() {
            if blank(lines[l]) || indent(lines[l]) > level {
                continue;
            }
            let closing = lines[l].trim_start().starts_with(['}', ')', ']']);
            return if closing { l as u32 + 1 } else { l as u32 };
        }
        return lines.len() as u32;
    }
    (code..lines.len())
        .find(|&l| {
            if level == 0 {
                blank(lines[l])
            } else {
                !blank(lines[l]) && indent(lines[l]) < level
            }
        })
        .map_or(lines.len() as u32, |l| l as u32)
}

/// The records of the citations of `config`'s specs in a file's source, whose
/// other analysis has `defs`.
pub fn citation_records(src: &str, defs: &[Def], config: &SpecsConfig) -> (Vec<Record>, Stats) {
    let mut stats = Stats::default();
    let mut records = vec![];
    let lines_index = Lines::new(src);
    let lines: Vec<&str> = src.split('\n').collect();

    // The citations, and the lines citing specs.
    let mut citations: Vec<(Range<usize>, String, String)> = vec![];
    let mut citing: Vec<Citing> = vec![];
    for m in URL.find_iter(src) {
        let url = trim_url(m.as_str());
        if !url.contains('#') {
            continue;
        }
        let Some((spec, anchor)) = config.url_spec(url) else {
            continue;
        };
        // (Without a text fragment's directive, ex: `#x:~:text=y`.)
        let anchor = anchor.split(":~:").next().unwrap_or_default().to_string();
        if anchor.is_empty() {
            continue;
        }
        let sym = spec_symbol(spec.shortname(), &anchor);
        let pretty = format!("{} #{}", spec.shortname(), anchor);
        let range = m.start()..m.start() + url.len();
        let line = lines_index.location(&range).lineno;
        citations.push((range, sym.clone(), pretty.clone()));
        if citing.last().is_some_and(|c| c.line == line) {
            continue;
        }
        // The scope: the definition it precedes, or the rest of the one it's
        // in, or its indentation's.
        let index = line as usize - 1;
        let code = (index + 1..lines.len())
            .find(|&l| !lines[l].trim().is_empty() && !is_comment(lines[l]))
            .map(|l| l as u32 + 1);
        let innermost = |at: u32| {
            defs.iter()
                .filter(|d| d.lines.contains(&at))
                .min_by_key(|d| d.lines.end - d.lines.start)
        };
        // (Not past the end of the definition it's in, ex: a `}`.)
        let inside = innermost(line);
        let preceded = code
            .filter(|&code| !lines[code as usize - 1].trim_start().starts_with(['}', ')', ']']))
            .and_then(|code| {
                defs.iter()
                    .filter(|d| code <= d.line && d.line < code + 4)
                    .filter(|d| inside.is_none_or(|i| i.lines.start <= d.line && d.lines.end <= i.lines.end))
                    .min_by_key(|d| (d.line, std::cmp::Reverse(d.lines.end - d.lines.start)))
            });
        let (scope_end, def) = match preceded.or(inside) {
            Some(def) => (def.lines.end - 1, Some(def.clone())),
            None => (indent_scope_end(&lines, index), None),
        };
        citing.push(Citing {
            line,
            sym,
            pretty,
            scope_end,
            def,
        });
    }
    let context_of = |line: u32| -> Option<&Citing> {
        citing
            .iter()
            .filter(|c| c.line <= line && line <= c.scope_end)
            .max_by_key(|c| c.line)
    };
    for (range, sym, pretty) in citations {
        stats.citations += 1;
        let loc = lines_index.location(&range);
        let def = context_of(loc.lineno).and_then(|c| c.def.as_ref());
        records.push(source_record(
            loc,
            "use",
            format!("spec {}", pretty),
            sym.clone(),
            Default::default(),
        ));
        records.push(target_record(
            loc,
            AnalysisKind::DocUse,
            pretty,
            sym,
            def.map_or(String::new(), |d| d.pretty.clone()),
            def.map_or(String::new(), |d| d.sym.clone()),
        ));
    }

    // Steps, in their scopes.
    let mut offset = 0;
    for (index, line) in lines.iter().enumerate() {
        let start = offset;
        offset += line.len() + 1;
        let lineno = index as u32 + 1;
        let Some(c) = context_of(lineno).filter(|c| c.line < lineno) else {
            continue;
        };
        let Some(step) = STEP.captures(line) else {
            continue;
        };
        let (Some(whole), Some(path)) = (step.get(1), step.name("path")) else {
            continue;
        };
        stats.steps += 1;
        let path = path.as_str().trim_end_matches(['.', ':']);
        let loc = lines_index.location(&(start + whole.start()..start + whole.end()));
        let context = match &c.def {
            Some(def) => format!("{} step {}", def.pretty, path),
            None => format!("step {}", path),
        };
        records.push(source_record(
            loc,
            "use",
            format!("spec {}", c.pretty),
            c.sym.clone(),
            Default::default(),
        ));
        records.push(target_record(
            loc,
            AnalysisKind::DocUse,
            c.pretty.clone(),
            c.sym.clone(),
            context,
            c.def.as_ref().map_or(String::new(), |d| d.sym.clone()),
        ));
    }
    sort_records(&mut records);
    (records, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(records: &[Record]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) => Some(format!(
                    "{}:{}-{} {} {} | {} ({})",
                    t.loc.lineno,
                    t.loc.col_start,
                    t.loc.col_end,
                    t.data.kind.to_ustr(),
                    t.data.sym,
                    t.data.context,
                    t.data.contextsym
                )),
                _ => None,
            })
            .collect()
    }

    fn config() -> SpecsConfig {
        toml::from_str(
            "[[spec]]\nname = \"html\"\nrendered = \"https://html.spec.whatwg.org/\"\n\
             [[spec]]\nname = \"dom\"\nrendered = \"https://dom.spec.whatwg.org/\"\n",
        )
        .unwrap()
    }

    #[test]
    fn test_trim_url() {
        assert_eq!(trim_url("https://a.org/#b."), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#b)."), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#f(x)"), "https://a.org/#f(x)");
        assert_eq!(trim_url("https://a.org/#b*/"), "https://a.org/#b");
        let src = "// https://dom.spec.whatwg.org/#concept-node:~:text=node%20tree\n";
        let (records, _) = citation_records(src, &[], &config());
        assert_eq!(summary(&records), vec!["1:3-63 doc-use SPEC_dom_concept@2Dnode |  ()"]);
    }

    #[test]
    fn test_with_nesting_ranges() {
        let src = "\
// https://html.spec.whatwg.org/multipage/browsing-the-web.html#navigate
void Navigate() {
  // Step 1. Let x be y.
  Foo();
  // https://dom.spec.whatwg.org/#concept-tree-order (step 2's)
  // Step 1.
  // Step 12.6: If z.
}

// Step 3.
void Other() {}

// https://dom.spec.whatwg.org/#concept-node
nsresult
nsNode::Do()
{
  // Step 2.
}
";
        let def = |line: u32, lines: Range<u32>, pretty: &str| Def {
            line,
            lines,
            pretty: pretty.to_string(),
            sym: format!("_{}", pretty),
        };
        let defs = vec![
            def(2, 2..9, "Navigate"),
            def(11, 11..12, "Other"),
            def(15, 15..19, "nsNode::Do"),
        ];
        let (records, stats) = citation_records(src, &defs, &config());
        assert_eq!(
            summary(&records),
            vec![
                "1:3-72 doc-use SPEC_html_navigate | Navigate (_Navigate)",
                "3:5-11 doc-use SPEC_html_navigate | Navigate step 1 (_Navigate)",
                "5:5-52 doc-use SPEC_dom_concept@2Dtree@2Dorder | Navigate (_Navigate)",
                "6:5-11 doc-use SPEC_dom_concept@2Dtree@2Dorder | Navigate step 1 (_Navigate)",
                "7:5-14 doc-use SPEC_dom_concept@2Dtree@2Dorder | Navigate step 12.6 (_Navigate)",
                "13:3-44 doc-use SPEC_dom_concept@2Dnode | nsNode::Do (_nsNode::Do)",
                "17:5-11 doc-use SPEC_dom_concept@2Dnode | nsNode::Do step 2 (_nsNode::Do)",
            ]
        );
        assert_eq!((stats.citations, stats.steps), (3, 4));
    }

    #[test]
    fn test_signatures_without_nesting_ranges() {
        // (A return type and a signature on their own lines, before the
        // body's `{`, as in Gecko.)
        let src = "\
// https://html.spec.whatwg.org/#navigate
nsresult
Navigate(int aX)
{
  // Step 1.
}

// Step 2.
";
        let (records, _) = citation_records(src, &[], &config());
        assert_eq!(
            summary(&records),
            vec![
                "1:3-41 doc-use SPEC_html_navigate |  ()",
                "5:5-11 doc-use SPEC_html_navigate | step 1 ()",
            ]
        );
    }

    #[test]
    fn test_with_indentation() {
        let src = "\
# https://html.spec.whatwg.org/#navigate
def navigate():
    # Steps 1-2.
    foo()
    if x:
        # Step 3.a
        bar()

# Step 4.
def other():
    pass

class C:
    def f(self):
        # https://dom.spec.whatwg.org/#concept-node
        # Step 1.
        if y:
            pass
        # Step 2.
    # Step 3.
";
        let (records, stats) = citation_records(src, &[], &config());
        assert_eq!(
            summary(&records),
            vec![
                "1:2-40 doc-use SPEC_html_navigate |  ()",
                "3:6-15 doc-use SPEC_html_navigate | step 1-2 ()",
                "6:10-18 doc-use SPEC_html_navigate | step 3.a ()",
                "15:10-51 doc-use SPEC_dom_concept@2Dnode |  ()",
                "16:10-16 doc-use SPEC_dom_concept@2Dnode | step 1 ()",
                "19:10-16 doc-use SPEC_dom_concept@2Dnode | step 2 ()",
            ]
        );
        assert_eq!((stats.citations, stats.steps), (2, 4));
    }
}
