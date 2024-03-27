use std::{cell::Cell, rc::Rc};

use async_trait::async_trait;
use lol_html::{element, HtmlRewriter, Settings};
use clap::Args;

use super::interface::{JsonRecords, PipelineCommand, PipelineValues};
use crate::{
    abstract_server::{AbstractServer, Result, HtmlFileRoot},
    cmd_pipeline::interface::{HtmlExcerpts, HtmlExcerptsByFile},
};

/// Output the HTML lines corresponding to the JSON records received via input.
///
/// There's also likely a use-case to process an HTML file as a root where we
/// then filter lines based on "data-symbols".  It's not immediately clear if
/// this command should also handle that or not.  There's also the question of
/// whether symbol filtering would only excerpt the span associated with the
/// symbol or would walk up to the enclosing line.  The answers should probably
/// be driven by command-line use-cases; in particular, the experience of
/// evolving a more targeted query.  Having to modify a command up-stream should
/// be considered undesirable.
#[derive(Debug, Args)]
pub struct ShowHtml {}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ShowHtmlCommand {
    pub args: ShowHtml,
}

#[async_trait]
impl PipelineCommand for ShowHtmlCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let jr = match input {
            PipelineValues::JsonRecords(jr) => jr,
            _ => JsonRecords { by_file: vec![] },
        };

        let mut html_by_file = vec![];

        for fr in jr.by_file {
            // ## For each file!
            let lines_to_show = fr.line_set();

            // XXX Doing this as a single string received in a lump is fine for
            // our testing use-case, but this may need to be reconsidered in
            // production.  Or maybe production really wants the performance?
            // Production certainly should have the RAM for our known worst
            // case scenarios.
            let html_str = server.fetch_html(HtmlFileRoot::FormattedFile, &fr.file).await?;

            let mut file_excerpts = vec![];

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
                        element!(
                            r#"div.nesting-container"#,
                            move |el| {
                                nesting_suppress.set(true);
                                let end_suppress = nesting_suppress.clone();
                                el.on_end_tag(move |_end| {
                                    end_suppress.set(true);
                                    Ok(())
                                })?;
                                Ok(())
                            }
                        ),
                        element!(
                            r#"div.source-line-with-number"#,
                            |el| {
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
                            }
                        )
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
                            writing_line = 0;
                            file_excerpts.push(String::from_utf8_lossy(&buf).to_string());
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

            html_by_file.push(HtmlExcerptsByFile {
                file: fr.file.clone(),
                excerpts: file_excerpts,
            });
        }

        Ok(PipelineValues::HtmlExcerpts(HtmlExcerpts {
            by_file: html_by_file,
        }))
    }
}
