use std::collections::BTreeSet;

use async_trait::async_trait;
use clap::Parser;
use ustr::Ustr;

use super::interface::{PipelineCommand, PipelineValues};
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
        let path_line_contents = server.fetch_html_lines(requests).await?;

        // ## Ingest the new lines.
        results.ingest_html_lines(&path_line_contents, self.args.before, self.args.after);

        Ok(PipelineValues::FlattenedResultsBundle(results))
    }
}
