use async_trait::async_trait;
use clap::Args;
use serde_json::json;

use super::interface::{PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, Result, ServerError};
use crate::hyperblame::token_timeline::{MAX_TOKENS, TokenTimelineOptions};

/// The MIME type of a `token-timeline:` view's model (`{"tokens": [...],
/// "timeline": TokenTimeline, "unsupported": bool}`, with its "<"s escaped),
/// which the query page has token-timeline.js render.
pub const TOKEN_TIMELINE_MIME: &str = "application/vnd.searchfox.token-timeline+json";

/// Tokens' timelines (see `hyperblame::token_timeline`): a line a token (and
/// its case variants) of its instances by week, with markers at the weeks
/// changing them the most, for the `token-timeline:` terms of a query, which
/// add their tokens.
#[derive(Debug, Args)]
pub struct TokenTimelineArgs {
    /// The tokens (case-insensitively distinct, at most
    /// `token_timeline::MAX_TOKENS`).
    #[clap(value_parser, required = true)]
    tokens: Vec<String>,

    /// Only the tokens' journals, not their case variants'.
    #[clap(long, value_parser)]
    case_sensitive: bool,
}

#[derive(Debug)]
pub struct TokenTimelineCommand {
    pub args: TokenTimelineArgs,
}

#[async_trait]
impl PipelineCommand for TokenTimelineCommand {
    fn estimate(&self, _server: &(dyn AbstractServer + Send + Sync)) -> u64 {
        // (Up to `token_timeline::BUDGET` of journals, parsed.)
        use crate::memory_budget::mib;
        mib(16) + self.args.tokens.len().min(MAX_TOKENS) as u64 * mib(4)
    }

    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let tokens: Vec<String> = self
            .args
            .tokens
            .iter()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .collect();
        let opts = TokenTimelineOptions {
            case_sensitive: self.args.case_sensitive,
        };
        let model = match server.token_timeline(&tokens, opts).await {
            Ok(timeline) => json!({ "tokens": tokens, "timeline": timeline, "unsupported": false }),
            Err(ServerError::Unsupported) => {
                json!({ "tokens": tokens, "timeline": null, "unsupported": true })
            }
            Err(err) => return Err(err),
        };
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: TOKEN_TIMELINE_MIME.to_string(),
            contents: model.to_string().replace('<', "\\u003c"),
            transient: false,
        }))
    }
}
