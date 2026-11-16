use async_trait::async_trait;
use clap::{Args, ValueEnum};

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::try_review::{ReviewView, TRY_REVIEW_MIME, render_review, review_view, view_facets};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum TryReviewFormat {
    /// The view's HTML, for the query page.
    Html,
    /// The view's model (see `try_review::ReviewView`), as JSON.
    Json,
}

/// A try push's review view (see `try_review`): the `try-review:` query
/// term's.
#[derive(Debug, Args)]
pub struct TryReview {
    /// The push's head (its `try/hg/<push>` page's).
    #[clap(value_parser)]
    push: String,

    #[clap(long, value_enum, default_value = "html")]
    format: TryReviewFormat,
}

#[derive(Debug)]
pub struct TryReviewCommand {
    pub args: TryReview,
}

#[async_trait]
impl PipelineCommand for TryReviewCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let push = self.args.push.trim().to_lowercase();
        if push.len() < 12 || !push.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!(
                    "try-review: needs a try push's revision (12 to 40 hexadecimal digits), not {:?}",
                    self.args.push
                ),
            }));
        }
        // (Cached by the push; the server's identity covers the index's and
        // the code's.  A push imported again has the same commits.)
        let value =
            crate::step_cache::cached(server.cache_identity(), "try-review", &push, || async {
                Ok::<_, ServerError>(serde_json::to_value(review_view(server, &push).await?)?)
            })
            .await?;
        if self.args.format == TryReviewFormat::Json {
            return Ok(PipelineValues::JsonValue(JsonValue { value }));
        }
        let view: ReviewView = serde_json::from_value(value)?;
        let tree = server.tree_info()?.name;
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: TRY_REVIEW_MIME.to_string(),
            contents: render_review(&tree, &view),
            transient: false,
            facets: Some(view_facets(&view)),
        }))
    }
}
