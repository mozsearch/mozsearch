use std::{cell::Cell, collections::HashMap, rc::Rc};

use async_trait::async_trait;
use clap::Parser;
use lol_html::{element, HtmlRewriter, Settings};
use ustr::UstrMap;

use super::interface::{PipelineCommand, PipelineValues};
use crate::abstract_server::{
    AbstractServer, ErrorDetails, ErrorLayer, HtmlFileRoot, Result, ServerError,
};

/// Augment a FlattenedResultsBundle by scraping the rendered HTML output files
/// for lines of interest plus any context, plus applying any predicates that
/// run against data baked into the output file (like coverage data or history
/// data).
///
/// It's possible the performance for this will be horrible, although it may be
/// possible to throw some of the following at this model:
/// - Having the HTML line extraction run as multiple parallel tasks.
/// - Increasing VM RAM to allow for more caching.
/// - Switching to a VM with local SSD for better I/O.
/// - Pre-computing line offsets (in the uncompresed HTML, noting that we do
///   gzip the HTML and probably still want to seek through that) so that we can
///   reduce the amount of HTML parsing required (even if we still have to seek
///   through a lot).
/// - Create some alternate on-disk representation for the rendered HTML files
///   that's intentionally distinct from the pre-gzipped files.  This could be
///   a different compression format and/or sharded files (maybe with duplicate
///   data at the start/end so we never have to span shards normally).
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
        server: &Box<dyn AbstractServer + Send + Sync>,
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

        // ### Build up map of HTML lines
        //
        // This bit could potentially run in parallel.  We're not going to yet
        // because:
        // - Wanna see how bad it is.
        // - Don't wanna have this experimental thing steal all the resources
        //   from the non-experimental router.py and web-server.rs yet!

        let mut path_line_contents: UstrMap<HashMap<u32, String>> = UstrMap::default();

        for (path, lines_to_show) in
            results.compute_path_line_sets(self.args.before, self.args.after)
        {
            // XXX Doing this as a single string received in a lump is fine for
            // our testing use-case, but this may need to be reconsidered in
            // production.  Or maybe production really wants the performance?
            // Production certainly should have the RAM for our known worst
            // case scenarios.
            let html_str = server
                .fetch_html(HtmlFileRoot::FormattedFile, &path)
                .await?;

            let file_lines = path_line_contents.entry(path).or_default();

            // ### HTML Extraction: What We Want
            //
            // We want the full line container which looks like:
            // - div id="line-N" class="source-line-with-number" role="row"
            //   - div role="cell"
            //     - div class="cov-strip cov-uncovered cov-known"
            //   - div role="cell"
            //     - div class="blame-strip c2" data-blame="..."
            //   - div role="cell" class="line-number" data-line-number="N"
            //   - code role="cell" class="source-line"
            //     - ex: span class="sync_comment"
            //     - ex: span class="syn_def syn_type" data-symbols="..." data-i
            //
            // ### HTML Extraction Low Level Details
            //
            // Until https://github.com/cloudflare/lol-html/issues/40 or
            // the spin-off https://github.com/cloudflare/lol-html/issues/78
            // are implemented, lol_html doesn't explicitly provide a way to
            // derive the value of an element.
            //
            // So we attempt a hack where we use a custom output sink that is
            // kept aware of where we are in the file.  The good news is that
            // since lol_html is oriented around minimal memory allocation, we
            // can generally control when flushes happen.

            let mut writing_line: u32 = 0;
            let cur_line = Cell::new(0u32);
            let want_cur_line = Cell::new(false);
            let suppressing = Rc::new(Cell::new(false));
            let nesting_suppress = suppressing.clone();

            let mut buf = vec![];

            let mut rewrite = HtmlRewriter::new(
                Settings {
                    element_content_handlers: vec![
                        element!(r#"div.nesting-container"#, move |el| {
                            nesting_suppress.set(true);
                            let end_suppress = nesting_suppress.clone();
                            el.on_end_tag(move |_end| {
                                end_suppress.set(true);
                                Ok(())
                            })?;
                            Ok(())
                        }),
                        element!(r#"div.source-line-with-number"#, |el| {
                            suppressing.set(false);
                            if let Some(id_str) = el.get_attribute("id") {
                                let id_parts: Vec<&str> = id_str.split("-").collect();
                                if id_parts.len() == 2 && id_parts[0] == "line" {
                                    let lno = id_parts[1].parse().unwrap_or(0);
                                    cur_line.set(lno);
                                    want_cur_line.set(lines_to_show.contains(&lno));
                                }
                            }

                            Ok(())
                        }),
                    ],
                    ..Settings::default()
                },
                |c: &[u8]| {
                    if suppressing.get() {
                        return;
                    }

                    // We were actively writing and potentially have some
                    // buffer.
                    if writing_line > 0 {
                        // We're done writing; flush!
                        if cur_line.get() != writing_line {
                            file_lines
                                .insert(writing_line, String::from_utf8_lossy(&buf).to_string());
                            writing_line = 0;
                            buf.clear();
                        }
                        // We're still writing!
                        else {
                            // Write into the buffer and then leave, because we
                            // don't need to consider switching into writing, as
                            // we're still here.
                            buf.extend_from_slice(c);
                            return;
                        }
                    }
                    // We either closed out writing or weren't writing.  But now
                    // we need to see if we should be writing!
                    if cur_line.get() > 0 && want_cur_line.get() {
                        writing_line = cur_line.get();
                        buf.extend_from_slice(c);
                    }
                    // Otherwise, this wasn't interesting.
                },
            );

            rewrite.write(html_str.as_bytes()).unwrap();
            rewrite.end().unwrap();
        }

        // ## Ingest the new lines.
        results.ingest_html_lines(&path_line_contents, self.args.before, self.args.after);

        Ok(PipelineValues::FlattenedResultsBundle(results))
    }
}
