//! The analysis of web specs' sources (`spec-analyze`): the terms they
//! define and their references to them (and to other specs' terms), as
//! analysis records of `SPEC_<shortname>_<anchor>` symbols, where the anchor
//! is the term's id in the rendered spec, so that a URL to the rendered spec
//! names the symbol.
//!
//! The specs are a tree's `specs.toml`'s (see `SpecsConfig`), and each is
//! analyzed according to its format:
//! - `wattsi`: the HTML Standard's `source`, whose cross-references we
//!   resolve as Wattsi does (see `wattsi`);
//! - `ecmarkup`: ECMA-262's (and TC39's other specs') `spec.html`, which we
//!   link as ecmarkup does (see `ecmarkup`);
//! - `bikeshed`: Bikeshed's `.bs` sources, whose definitions and references
//!   Bikeshed itself finds, with their positions (`bikeshed index`, see
//!   `bikeshed`).
//!
//! Records' contexts are the spec's sections and algorithms (as the spec
//! tokenizer's, see `spec_tokenizer::html_contexts`), and within an
//! algorithm's steps, the step's number (ex: "navigate step 12.6").

pub mod bikeshed;
pub mod ecmarkup;
pub mod markup;
pub mod wattsi;

use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::file_format::analysis::{AnalysisKind, Location, SourceRange};
use crate::semantic_analyzer::{Record, source_record, target_record};

/// A tree's specs (ex: the wubspecs tree's `specs.toml`, which also
/// configures its forest; see `spec_forest::ForestConfig`).
#[derive(Debug, Default, Deserialize)]
pub struct SpecsConfig {
    #[serde(default, rename = "spec")]
    pub specs: Vec<SpecEntry>,
    /// The directory of the `specs.toml`, which specs' `index`es are
    /// relative to.
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SpecFormat {
    Wattsi,
    Ecmarkup,
    Bikeshed,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SpecEntry {
    /// The spec's directory in the forest.
    pub name: String,
    /// The spec's shortname, for its symbols (default: its name), as in the
    /// browser-specs index (ex: "ecmascript" for ECMA-262).
    #[serde(default)]
    pub shortname: Option<String>,
    /// The spec's directory in the tree (default: its name).
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub format: Option<SpecFormat>,
    /// The spec's source, in its directory (default: `source` for Wattsi,
    /// `spec.html` for ecmarkup, and `index.bs` for Bikeshed).
    #[serde(default)]
    pub source: Option<String>,
    /// For Bikeshed, the spec's index (`bikeshed index --json`'s output) to
    /// use rather than running Bikeshed, relative to the `specs.toml` (ex:
    /// for the test tree, which doesn't have Bikeshed).
    #[serde(default)]
    pub index: Option<String>,
    /// The rendered spec's URL, which its anchors are fragments of, and which
    /// other specs' references to it start with (ex:
    /// "https://dom.spec.whatwg.org/").
    #[serde(default)]
    pub rendered: Option<String>,
}

impl SpecEntry {
    pub fn shortname(&self) -> &str {
        self.shortname.as_deref().unwrap_or(&self.name)
    }

    pub fn dir(&self) -> &str {
        self.path.as_deref().unwrap_or(&self.name)
    }

    /// The path of the spec's source in the tree, if its format has one.
    pub fn source_path(&self) -> Option<String> {
        let source = match (self.format?, &self.source) {
            (_, Some(source)) => source.as_str(),
            (SpecFormat::Wattsi, None) => "source",
            (SpecFormat::Ecmarkup, None) => "spec.html",
            (SpecFormat::Bikeshed, None) => "index.bs",
        };
        Some(match self.dir() {
            "" => source.to_string(),
            dir => format!("{}/{}", dir, source),
        })
    }
}

impl SpecsConfig {
    pub fn load(path: &Path) -> Result<SpecsConfig, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let mut config: SpecsConfig =
            toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
        config.dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(config)
    }

    /// The symbol a URL to one of the specs names, if it's to one of them
    /// (with a fragment).
    pub fn url_symbol(&self, url: &str) -> Option<String> {
        let (spec, anchor) = self.url_spec(url)?;
        Some(spec_symbol(spec.shortname(), &anchor))
    }

    /// The spec a URL is to, and its anchor, if it's to one of them (with a
    /// fragment).
    pub fn url_spec(&self, url: &str) -> Option<(&SpecEntry, String)> {
        let (page, anchor) = url.split_once('#')?;
        if anchor.is_empty() {
            return None;
        }
        let page = page.trim_start_matches("http:").trim_start_matches("https:");
        let spec = self.specs.iter().find(|spec| {
            spec.rendered.as_deref().is_some_and(|rendered| {
                let rendered = rendered
                    .trim_start_matches("http:")
                    .trim_start_matches("https:");
                page.starts_with(rendered) || format!("{}/", page) == rendered
            })
        })?;
        Some((spec, percent_decode(anchor)))
    }
}

/// `text` with its `%XX` escapes decoded (when they're UTF-8).
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// The symbol of an anchor in a spec: `SPEC_<shortname>_<anchor>`, with
/// every byte but ASCII letters, digits, `_`, and `/` escaped as `@XX` (as
/// for `FILE_` symbols, see `mangle_file`, but for every byte of non-ASCII
/// characters).
pub fn spec_symbol(shortname: &str, anchor: &str) -> String {
    let mut out = String::from("SPEC_");
    escape_into(shortname, &mut out);
    out.push('_');
    escape_into(anchor, &mut out);
    out
}

fn escape_into(text: &str, out: &mut String) {
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'/' {
            out.push(b as char);
        } else {
            out.push_str(&format!("@{:02X}", b));
        }
    }
}

/// A context name from `spec_tokenizer`, without its escapes (see
/// `config_tokenizer::escape_name`).
pub(crate) fn unescape_context_name(name: &str) -> String {
    name.replace("%20", " ")
        .replace("%09", "\t")
        .replace("%3A%3A", "::")
        .replace("%25", "%")
}

/// The start of each line of a source, for records' locations.
pub struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    /// The byte offset of a 1-based line and 1-based column in characters
    /// (ex: Bikeshed's positions), if `src` has it.
    pub fn char_position(&self, src: &str, line: usize, col: usize) -> Option<usize> {
        let start = *self.starts.get(line.checked_sub(1)?)?;
        let end = self.starts.get(line).map_or(self.len, |&next| next);
        let text = &src[start..end];
        let col = col.checked_sub(1)?;
        text.char_indices()
            .nth(col)
            .map(|(at, _)| start + at)
            .or_else(|| (col == text.chars().count()).then_some(end))
    }

    pub fn new(src: &str) -> Lines {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Lines {
            starts,
            len: src.len(),
        }
    }

    /// The location of `range`, which ends at the end of its first line if
    /// it spans lines.
    pub fn location(&self, range: &Range<usize>) -> Location {
        let line = self.starts.partition_point(|&start| start <= range.start) - 1;
        let line_end = self.starts.get(line + 1).map_or(self.len, |&next| next - 1);
        Location {
            lineno: line as u32 + 1,
            col_start: (range.start - self.starts[line]) as u32,
            col_end: (range.end.min(line_end).max(range.start) - self.starts[line]) as u32,
        }
    }
}

/// What a record is in: its context's name and symbol.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InContext {
    pub name: String,
    pub sym: String,
}

/// A definition's or reference's records: a source record, whose pretty
/// name is prefixed with what the symbol is (ex: "dfn html navigate"), and a
/// target record.
pub fn spec_records(
    records: &mut Vec<Record>,
    loc: Location,
    def: bool,
    what: &str,
    pretty: &str,
    sym: &str,
    context: &InContext,
) {
    let (syntax, kind) = if def {
        ("def", AnalysisKind::Def)
    } else {
        ("use", AnalysisKind::Use)
    };
    records.push(source_record(
        loc,
        syntax,
        format!("{} {}", what, pretty),
        sym.to_string(),
        SourceRange::default(),
    ));
    records.push(target_record(
        loc,
        kind,
        pretty.to_string(),
        sym.to_string(),
        context.name.clone(),
        context.sym.clone(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec_symbol() {
        assert_eq!(spec_symbol("html", "navigate"), "SPEC_html_navigate");
        assert_eq!(
            spec_symbol("service-workers", "dfn-job's queue"),
            "SPEC_service@2Dworkers_dfn@2Djob@27s@20queue"
        );
        assert_eq!(spec_symbol("html", "é"), "SPEC_html_@C3@A9");
    }

    #[test]
    fn test_url_symbol() {
        let config: SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "html"
            rendered = "https://html.spec.whatwg.org/"
            [[spec]]
            name = "dom"
            rendered = "https://dom.spec.whatwg.org/"
            [[spec]]
            name = "ecma262"
            shortname = "ecmascript"
            rendered = "https://tc39.es/ecma262/"
            "#,
        )
        .unwrap();
        let sym = |url| config.url_symbol(url);
        assert_eq!(
            sym("https://dom.spec.whatwg.org/#concept-node").as_deref(),
            Some("SPEC_dom_concept@2Dnode")
        );
        assert_eq!(
            sym("https://html.spec.whatwg.org/multipage/browsing-the-web.html#navigate").as_deref(),
            Some("SPEC_html_navigate")
        );
        assert_eq!(
            sym("http://tc39.es/ecma262/#sec-tostring").as_deref(),
            Some("SPEC_ecmascript_sec@2Dtostring")
        );
        assert_eq!(
            sym("https://tc39.es/ecma262#sec-%25array%25").as_deref(),
            Some("SPEC_ecmascript_sec@2D@25array@25")
        );
        assert_eq!(sym("https://dom.spec.whatwg.org/"), None);
        assert_eq!(sym("https://url.spec.whatwg.org/#concept-url"), None);
    }
}
