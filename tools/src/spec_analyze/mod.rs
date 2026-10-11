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
//!
//! Definitions also get structured records (see `spec_structured`), with
//! kinds from their types (see `dfn_kind`), their parents (by what they're
//! `for`, see `ForResolver`), and their anchors' URLs in the rendered spec.
//! Algorithms' kinds are callable (`specAlgorithm`, and `specAbstractOp` for
//! ecmarkup's abstract operations), so the references in their steps are
//! calls, for diagrams and traversals across specs.

pub mod bikeshed;
pub mod ecmarkup;
pub mod markup;
pub mod wattsi;

use std::ops::Range;
use std::path::{Path, PathBuf};

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;

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
    /// Other URLs of the rendered spec, which references to it can start
    /// with, ex: its series' unversioned URL for the current level of a CSS
    /// spec ("https://drafts.csswg.org/css-grid/" for css-grid-2).
    #[serde(default)]
    pub aliases: Vec<String>,
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

    /// The URL of an anchor in the rendered spec with `shortname`, if it's
    /// configured.
    pub fn rendered_url(&self, shortname: &str, anchor: &str) -> Option<String> {
        let spec = self.specs.iter().find(|spec| spec.shortname() == shortname)?;
        Some(format!("{}#{}", spec.rendered.as_deref()?, anchor))
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
        // (The longest URL that matches, since specs can be under others,
        // ex: CSP Embedded Enforcement under CSP.)
        let matching = |url: &str| -> Option<usize> {
            let url = url.trim_start_matches("http:").trim_start_matches("https:");
            (page.starts_with(url) || format!("{}/", page) == url).then_some(url.len())
        };
        let (spec, _) = self
            .specs
            .iter()
            .filter_map(|spec| {
                let best = spec
                    .rendered
                    .iter()
                    .chain(&spec.aliases)
                    .filter_map(|url| matching(url))
                    .max()?;
                Some((spec, best))
            })
            .max_by_key(|&(_, len)| len)?;
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

/// The structured record kind of a definition of a type (Bikeshed's and
/// Wattsi's dfn types, ex: "interface"): for IDL, the kinds of Gecko's WebIDL
/// (see scripts/webidl-analyze.py), so that specs' interfaces are classes and
/// their operations methods, and otherwise spec kinds (see docs/analysis.md).
pub fn dfn_kind(dfn_type: &str) -> &'static str {
    match dfn_type {
        "interface" | "namespace" | "dictionary" | "exception" => "class",
        "method" | "constructor" | "stringifier" | "serializer" | "iterator" => "method",
        "attribute" | "dict-member" => "field",
        "const" => "const",
        "enum" => "enum",
        "enum-value" => "enumConstant",
        "typedef" => "typedef",
        "callback" => "callback",
        "argument" => "parameter",
        "event" => "event",
        "abstract-op" => "specAbstractOp",
        "element" => "element",
        "element-attr" => "elementAttr",
        "attr-value" => "elementAttrValue",
        "element-state" => "elementState",
        "property" => "cssProperty",
        "descriptor" => "cssDescriptor",
        "value" => "cssValue",
        "type" => "cssType",
        "at-rule" => "cssAtRule",
        "function" => "cssFunction",
        "selector" => "cssSelector",
        "grammar" => "specGrammar",
        "scheme" => "urlScheme",
        "http-header" => "httpHeader",
        "permission" => "permission",
        "cddl-module" => "cddlModule",
        "cddl-type" => "cddlType",
        "cddl-key" => "cddlKey",
        "cddl-parameter" => "cddlParameter",
        "cddl-value" => "cddlValue",
        _ => "specDfn",
    }
}

/// The kinds of the parents of definitions of a kind (what they're `for`),
/// in order of preference, if they're particular (else any).
fn parent_kinds(kind: &str) -> &'static [&'static str] {
    match kind {
        "method" | "field" | "const" => &["class"],
        "enumConstant" => &["enum"],
        "parameter" => &["method", "callback"],
        "elementAttr" | "elementState" => &["element"],
        "elementAttrValue" => &["elementAttr"],
        "cssDescriptor" => &["cssAtRule"],
        "specDfn" | "specAlgorithm" | "specAbstractOp" => &["specDfn", "specAlgorithm"],
        _ => &[],
    }
}

/// A spec's definitions by their linking texts (and `for/text`), for finding
/// the parents of definitions by what they're `for` (ex: an IDL method's
/// interface, an attribute value's attribute, or a concept's concept).
#[derive(Default)]
pub struct ForResolver {
    /// (symbol, kind), by linking text.
    by_text: HashMap<String, Vec<(String, &'static str)>>,
}

impl ForResolver {
    /// Add a definition, with its linking texts, and what it's for.
    pub fn add(&mut self, sym: &str, kind: &'static str, texts: &[String], fors: &[String]) {
        for text in texts {
            let mut keys = vec![text.clone()];
            keys.extend(fors.iter().map(|f| format!("{}/{}", f, text)));
            for key in keys {
                let entry = self.by_text.entry(key).or_default();
                if !entry.iter().any(|(s, _)| s == sym) {
                    entry.push((sym.to_string(), kind));
                }
            }
        }
    }

    /// The symbol of the parent of a definition of `kind` which is for
    /// `dfn_for`, if there's one.
    pub fn parent(&self, dfn_for: &str, kind: &str) -> Option<&str> {
        let candidates = self
            .by_text
            .get(dfn_for)
            .or_else(|| self.by_text.get(&dfn_for.to_lowercase()))?;
        let preferred = parent_kinds(kind);
        preferred
            .iter()
            .find_map(|want| candidates.iter().find(|(_, k)| k == want))
            .or_else(|| preferred.is_empty().then(|| candidates.first()).flatten())
            .map(|(sym, _)| sym.as_str())
    }
}

/// A definition's structured record: its kind, pretty name, parent, and
/// anchor's URL in the rendered spec (`rendered`), with the `spec` implKind.
#[allow(clippy::too_many_arguments)]
pub fn spec_structured(
    records: &mut Vec<Record>,
    loc: Location,
    pretty: &str,
    sym: &str,
    kind: &str,
    parentsym: Option<&str>,
    rendered: Option<&str>,
) {
    let mut value = json!({
        "loc": loc,
        "structured": 1,
        "pretty": pretty,
        "sym": sym,
        "kind": kind,
        "implKind": "spec",
    });
    if let Some(parentsym) = parentsym {
        value["parentsym"] = json!(parentsym);
    }
    if let Some(rendered) = rendered {
        value["rendered"] = json!(rendered);
    }
    records.push(Record::Structured(loc, value));
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

/// A spec's source with the files it includes (ex: ecmarkup's
/// `<emu-import>`s, Bikeshed's `<pre class=include>`s) inlined, as the spec's
/// build tool inlines them, as one text, and where each of its spans came
/// from.  (A file included more than once is one file, whose positions are
/// its first inclusion's.)
pub struct Combined {
    pub text: String,
    /// The files, by their paths in the tree, with their sources and lines.
    files: Vec<(String, String, Lines)>,
    /// (start in `text`, end in `text`, file, start in the file).
    spans: Vec<(usize, usize, usize, usize)>,
}

/// A file's includes (given its path and source): each one's range, which the
/// included file replaces, and the included file's path in the tree.
pub type FindIncludes<'a> = &'a dyn Fn(&str, &str) -> Vec<(Range<usize>, String)>;

/// `path`, with `..` and `.` components resolved.
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = vec![];
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

impl Combined {
    /// The source at `main` (a path in the tree, which `read` reads) with
    /// its includes, if it can be read.
    pub fn build(
        main: &str,
        read: &dyn Fn(&str) -> Option<String>,
        includes: FindIncludes,
    ) -> Option<Combined> {
        let mut combined = Combined {
            text: String::new(),
            files: vec![],
            spans: vec![],
        };
        combined.inline(main, read, includes, 0)?;
        Some(combined)
    }

    fn inline(
        &mut self,
        path: &str,
        read: &dyn Fn(&str) -> Option<String>,
        includes: FindIncludes,
        depth: u32,
    ) -> Option<()> {
        let file = match self.file(path) {
            Some(file) => file,
            None => {
                let source = read(path)?;
                let lines = Lines::new(&source);
                self.files.push((path.to_string(), source, lines));
                self.files.len() - 1
            }
        };
        let source = self.files[file].1.clone();
        let mut at = 0;
        for (range, target) in includes(path, &source) {
            self.copy(&source, at..range.start, file);
            // (A missing include, or one too deep, stays as it is.)
            if depth > 16 || self.inline(&target, read, includes, depth + 1).is_none() {
                self.copy(&source, range.clone(), file);
            }
            at = range.end;
        }
        self.copy(&source, at..source.len(), file);
        Some(())
    }

    fn copy(&mut self, source: &str, range: Range<usize>, file: usize) {
        if range.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(&source[range.clone()]);
        self.spans.push((start, self.text.len(), file, range.start));
    }

    /// The files' paths, by their numbers (the main source's first).
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(path, _, _)| path.as_str())
    }

    /// The number of the file at `path`, if it's one of them.
    pub fn file(&self, path: &str) -> Option<usize> {
        self.files.iter().position(|(p, _, _)| p == path)
    }

    /// Where a position in a file (a 1-based line and 1-based column in
    /// characters, ex: Bikeshed's) is in `text`.
    pub fn offset(&self, file: usize, line: usize, col: usize) -> Option<usize> {
        let (_, source, lines) = self.files.get(file)?;
        let local = lines.char_position(source, line, col)?;
        self.spans
            .iter()
            .find(|&&(start, end, f, file_start)| {
                f == file && file_start <= local && local < file_start + (end - start)
            })
            .map(|&(start, _, _, file_start)| start + (local - file_start))
    }

    /// The file and location of a range of `text` (up to the end of its
    /// span, if it spans files).
    pub fn place(&self, range: &Range<usize>) -> Option<(usize, Location)> {
        let i = self.spans.partition_point(|&(start, _, _, _)| start <= range.start);
        let &(start, end, file, file_start) = self.spans.get(i.checked_sub(1)?)?;
        if range.start >= end {
            return None;
        }
        let local = file_start + (range.start - start)..file_start + (range.end.min(end) - start);
        Some((file, self.files[file].2.location(&local)))
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
            [[spec]]
            name = "css-grid-2"
            shortname = "css-grid"
            rendered = "https://drafts.csswg.org/css-grid-2/"
            aliases = ["https://drafts.csswg.org/css-grid/"]
            [[spec]]
            name = "css-grid-3"
            rendered = "https://drafts.csswg.org/css-grid-3/"
            [[spec]]
            name = "csp"
            rendered = "https://w3c.github.io/webappsec-csp/"
            [[spec]]
            name = "csp-embedded-enforcement"
            rendered = "https://w3c.github.io/webappsec-csp/embedded/"
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
        assert_eq!(
            sym("https://drafts.csswg.org/css-grid/#grid-container").as_deref(),
            Some("SPEC_css@2Dgrid_grid@2Dcontainer")
        );
        assert_eq!(
            sym("https://drafts.csswg.org/css-grid-3/#masonry").as_deref(),
            Some("SPEC_css@2Dgrid@2D3_masonry")
        );
        assert_eq!(
            sym("https://w3c.github.io/webappsec-csp/embedded/#csp-attribute").as_deref(),
            Some("SPEC_csp@2Dembedded@2Denforcement_csp@2Dattribute")
        );
        assert_eq!(
            sym("https://w3c.github.io/webappsec-csp/#policy").as_deref(),
            Some("SPEC_csp_policy")
        );
        assert_eq!(sym("https://dom.spec.whatwg.org/"), None);
        assert_eq!(sym("https://url.spec.whatwg.org/#concept-url"), None);
    }
}
