use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;
use clap::Parser;
use ustr::{Ustr, UstrMap, ustr};

use super::interface::{PipelineCommand, PipelineValues, PresentationKind};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Augment a FlattenedResultsBundle by scraping the rendered HTML output files
/// for lines of interest plus any context, plus applying any predicates that
/// run against data baked into the output file (like coverage data or history
/// data).
///
/// Big queries' results are in many files (ex: 1477 files with 7.8 GB of HTML
/// for "nsIPrincipal" on firefox), so rather than decompressing and parsing
/// the whole files (16s for that), local indexes read only the chunks of the
/// files with the lines (output-file writes the files as gzips with chunks of
/// lines; see `chunked_gzip`), several files at a time, and find the rows by
/// searching for their markup (0.2-0.3s).
///
/// Alternately, we might:
/// - Run a post-file-rendering phase and effectively re-compute the crossref
///   database with HTML baked in and some number of extra lines of context?
/// - Have the crossref database include some extra context and have tokenizer
///   state included at the first line point so the tokenizer can do a bare
///   bones syntax highlighting.
#[derive(Debug, Parser)]
pub struct AugmentResults {
    /// Lines of context before a hit.
    #[clap(short, long, value_parser, default_value = "0")]
    before: u32,

    /// Lines of context after a hit.
    #[clap(short, long, value_parser, default_value = "0")]
    after: u32,
}

/// A row of a rendered file (see `chunked_gzip::ROW_START`) as `/query/`'s
/// results show it, like `/search/`'s: without the cells of the coverage and
/// blame strips, which are noise there, without the macros' expansions,
/// which the results don't show (and which can be most of a row, ex: 58 KB
/// for a line using `NS_ENSURE_SUCCESS`), and not as a line that sticks at
/// the top of its nesting (`nesting-sticky-line`, whose opaque background
/// would cover the marks' underlines of the row above; see `highlight`) or
/// says what nesting it's in (`data-nesting`, for `chunked_gzip`).
pub fn excerpt_row(row: &str) -> String {
    // The strips' cells are lines of their own (see `format::format_code`),
    // after the row's tag.
    let mut lines = String::with_capacity(row.len());
    for line in row.split_inclusive('\n') {
        if line.starts_with("<div role=\"row\"") {
            lines.push_str(&line.replacen(" nesting-sticky-line", "", 1));
            continue;
        }
        let cell = line.trim_start();
        if cell.starts_with("<div role=\"cell\"><div")
            && (cell.contains("cov-strip") || cell.contains("blame-strip"))
        {
            continue;
        }
        lines.push_str(line);
    }
    strip_attribute(
        &strip_attribute(&lines, " data-expansions=\""),
        " data-nesting=\"",
    )
}

/// `html` without the attribute whose name (with its leading space, `=`, and
/// quote) is `attribute`, where it's in tags (attribute values have no quotes,
/// which are entities, but the text could look like an attribute).
fn strip_attribute(html: &str, attribute: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find(attribute) {
        let in_tag = rest[..i].rfind('<') > rest[..i].rfind('>');
        let value = &rest[i + attribute.len()..];
        match value.find('"') {
            Some(end) if in_tag => {
                out.push_str(&rest[..i]);
                rest = &value[end + 1..];
            }
            _ => {
                out.push_str(&rest[..i + attribute.len()]);
                rest = value;
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug)]
pub struct AugmentResultsCommand {
    pub args: AugmentResults,
}

#[async_trait]
impl PipelineCommand for AugmentResultsCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let mut results = match input {
            PipelineValues::FlattenedResultsBundle(frb) => frb,
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "augment-resultst needs a FlattenedResultsBundle".to_string(),
                }));
            }
        };

        // ### Fetch the rows of the lines and their context
        //
        // Local indexes only read the chunks of the rendered files with the
        // lines, several files at a time; see `chunked_gzip`.
        let requests: Vec<(Ustr, BTreeSet<u32>)> = results
            .compute_path_line_sets(self.args.before, self.args.after)
            .into_iter()
            .map(|(path, lines)| (path, lines.into_iter().collect()))
            .collect();
        let mut path_line_contents: UstrMap<HashMap<u32, String>> = UstrMap::default();
        // The symbols of the innermost nesting containers of each line: the
        // one around it, and the one around it that it doesn't start (see
        // `chunked_gzip::Row`).
        let mut path_line_nesting: UstrMap<HashMap<u32, (Option<String>, Option<String>)>> =
            UstrMap::default();
        for (path, rows) in server.fetch_html_lines(requests).await? {
            let contents = path_line_contents.entry(path).or_default();
            let nesting = path_line_nesting.entry(path).or_default();
            for (line, row) in rows {
                nesting.insert(line, (row.nesting_sym, row.enclosing_sym));
                contents.insert(line, excerpt_row(&row.html));
            }
        }

        // ### Contexts from the rendered files
        //
        // Crossref's results say what they're in ("// found in"), except at
        // namespace scope (ex: forward declarations, and definitions of
        // classes, or of methods outside their classes), and textual
        // occurrences are just lines, so those get the innermost nesting
        // containers (ex: functions, classes, namespaces) around their key
        // lines, with their symbols' pretty names: whatever textual
        // occurrences are in (as for crossref's uses), and what's around
        // crossref's results other than what they start (ex: a class's
        // namespace, not the class, for its first line).
        let context_sym = |kind: &PresentationKind, path: &Ustr, line: u32| -> Option<&str> {
            let (nesting, enclosing) = path_line_nesting.get(path)?.get(&line)?;
            if *kind == PresentationKind::TextualOccurrences {
                nesting.as_deref()
            } else {
                enclosing.as_deref()
            }
        };
        let mut prettys: HashMap<&str, Option<Ustr>> = HashMap::new();
        for path_kind_group in &results.path_kind_results {
            for kind_group in &path_kind_group.kind_groups {
                for file in &kind_group.by_file {
                    for span in &file.line_spans {
                        if !span.context.is_empty() {
                            continue;
                        }
                        if let Some(sym) = context_sym(&kind_group.kind, &file.file, span.key_line)
                        {
                            prettys.entry(sym).or_default();
                        }
                    }
                }
            }
        }
        for (sym, pretty) in prettys.iter_mut() {
            *pretty = server
                .jumpref_lookup(sym)
                .await?
                .map(|jumpref| jumpref.pretty);
        }
        for path_kind_group in &mut results.path_kind_results {
            for kind_group in &mut path_kind_group.kind_groups {
                for file in &mut kind_group.by_file {
                    let path = file.file;
                    for span in &mut file.line_spans {
                        if !span.context.is_empty() {
                            continue;
                        }
                        let Some(sym) = context_sym(&kind_group.kind, &path, span.key_line) else {
                            continue;
                        };
                        if let Some(Some(pretty)) = prettys.get(sym) {
                            span.context = *pretty;
                            span.contextsym = ustr(sym);
                        }
                    }
                }
            }
        }

        // ## Ingest the new lines.
        results.ingest_html_lines(&path_line_contents, self.args.before, self.args.after);

        Ok(PipelineValues::FlattenedResultsBundle(results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_excerpt_row() {
        let row = "<div role=\"row\" id=\"line-7\" class=\"source-line-with-number nesting-sticky-line\" data-nesting=\"NS_a,_Z1fv\">\n  <div role=\"cell\"><div role=\"button\" aria-expanded=\"false\" class=\"cov-strip cov-no-data\" aria-label=\"uncovered\"></div></div>\n  <div role=\"cell\"><div class=\"blame-strip c1\" data-hyperblame=\"1:0:2\" role=\"button\" aria-label=\"blame\" aria-expanded=\"false\"></div></div>\n  <div role=\"cell\" class=\"line-number\" data-line-number=\"7\"></div>\n  <code role=\"cell\" class=\"source-line\">  <span data-expansions=\"{&quot;M_1&quot;:{&quot;&quot;:&quot;x&quot;}}\" class=\"syn_macro\" data-symbols=\"M_1\">NS_ENSURE_SUCCESS</span>(rv, \" data-expansions=\"text\");\n</code>\n</div>\n";
        assert_eq!(
            excerpt_row(row),
            "<div role=\"row\" id=\"line-7\" class=\"source-line-with-number\">\n  <div role=\"cell\" class=\"line-number\" data-line-number=\"7\"></div>\n  <code role=\"cell\" class=\"source-line\">  <span class=\"syn_macro\" data-symbols=\"M_1\">NS_ENSURE_SUCCESS</span>(rv, \" data-expansions=\"text\");\n</code>\n</div>\n"
        );
    }
}
