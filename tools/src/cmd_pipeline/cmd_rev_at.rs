use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::rendered_docs::escape;

/// The MIME type of `rev-at`'s HTML, which the query page shows as it is.
pub const REV_AT_MIME: &str = "application/vnd.searchfox.rev-at+html";

/// The head the tree's branch had at a time, roughly (its main line's last
/// commit which had landed by then; see `commit_index::CommitIndex::rev_at`):
/// the `rev-at:` query term's, which `/search` goes through to the tree at
/// that revision (see query/dwim.toml's "rev-at").
#[derive(Debug, Args)]
pub struct RevAt {
    /// The time: "2024-08-30T07:26:47Z", "2024-08-30 07:26" (UTC), or a day,
    /// "2024-08-30" (its end).
    #[clap(value_parser)]
    time: String,
}

#[derive(Debug)]
pub struct RevAtCommand {
    pub args: RevAt,
}

#[async_trait]
impl PipelineCommand for RevAtCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let tree = server.tree_info()?.name;
        let time = crate::commit_index::parse_time(&self.args.time).ok_or_else(|| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!(
                    "{:?} isn't a time (ex: 2024-08-30, 2024-08-30 07:26, or 2024-08-30T07:26:47Z)",
                    self.args.time
                ),
            })
        })?;
        let shown = time.replace('T', " ").replace('Z', " UTC");
        let html = match server.rev_at(&time).await? {
            None => format!(
                r#"<div class="rev-at"><p>{} has no commits which had landed by {}.</p></div>"#,
                escape(&tree),
                escape(&shown)
            ),
            Some(at) => {
                // (Its summary and author, as blame popups show them.)
                let header = server
                    .commit_info_json(&at.rev)
                    .await
                    .ok()
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                    .and_then(|info| info[0]["header"].as_str().map(str::to_string));
                format!(
                    r#"<div class="rev-at"><p>At {}, {}'s head was roughly <a class="rev-at-rev" href="/{}/rev/{}/">{}</a>: the last commit of its main line which had landed by then (at {}).  Commits are dated by when they first landed anywhere (ex: on autoland, up to a day or so before mozilla-central and its nightly builds had them).</p>{}<p><a href="/{}/rev/{}/">Its files</a> · <a href="/{}/commit/{}">its commit</a></p></div>"#,
                    escape(&shown),
                    escape(&tree),
                    escape(&tree),
                    escape(&at.rev),
                    escape(&at.rev[..at.rev.len().min(12)]),
                    escape(&at.landed.replace('T', " ").replace('Z', " UTC")),
                    header.map_or_else(String::new, |header| format!(
                        r#"<div class="rev-at-commit">{}</div>"#,
                        header
                    )),
                    escape(&tree),
                    escape(&at.rev),
                    escape(&tree),
                    escape(&at.rev)
                )
            }
        };
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: REV_AT_MIME.to_string(),
            contents: html,
            transient: false,
        }))
    }
}
