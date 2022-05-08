use async_trait::async_trait;
use structopt::StructOpt;

use super::{interface::{PipelineCommand, PipelineValues}, transforms::path_glob_transform};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Perform a fulltext search against our livegrep/codesearch server over gRPC.
/// This is local-only at this time.
#[derive(Debug, StructOpt)]
pub struct SearchText {
    /// Text to search for; this will be regexp escaped.
    text: Option<String>,

    /// Search for a regular expression.  This can't be used if `text` is used.
    #[structopt(long)]
    re: Option<String>,

    /// Constrain matching path patterns with a non-regexp path constraint that
    /// will be escaped into a regexp.
    #[structopt(long)]
    path: Option<String>,

    /// Constrain matching path patterns with a regexp.
    #[structopt(long)]
    pathre: Option<String>,

    /// Should this be case-sensitive?  By default we are case-insensitive.
    #[structopt(short, long)]
    case_sensitive: bool,

    #[structopt(short, long, default_value = "0")]
    limit: usize,
}

#[derive(Debug)]
pub struct SearchTextCommand {
    pub args: SearchText,
}

#[async_trait]
impl PipelineCommand for SearchTextCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let re_pattern = if let Some(re) = &self.args.re {
            re.clone()
        } else if let Some(text) = &self.args.text {
            regex::escape(text)
        } else {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: "Missing search text or `re` pattern!".to_string(),
            }));
        };

        let pathre_pattern = if let Some(pathre) = &self.args.pathre {
            pathre.clone()
        } else if let Some(path) = &self.args.path {
            path_glob_transform(path)
        } else {
            "".to_string()
        };

        let matches = server
            .search_text(
                &re_pattern,
                !self.args.case_sensitive,
                &pathre_pattern,
                self.args.limit,
            )
            .await?;

        Ok(PipelineValues::TextMatches(matches))
    }
}
