use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::bug_comment::{bug_comment, comment_sym_info, parse_bug_comment, render_bug_comment};

/// The MIME type of a Bugzilla comment's view's HTML (see `bug_comment`),
/// which the query page shows as it is.
pub const BUG_COMMENT_MIME: &str = "application/vnd.searchfox.bug-comment+html";

/// A Bugzilla comment's blocks beside the code they link to (see
/// `bug_comment`): the `bug-comment:` query term's.
#[derive(Debug, Args)]
pub struct BugComment {
    /// The bug and the comment: "1915788/20", "1915788#c20", or "1915788"
    /// (comment 0, the bug's description).
    #[clap(value_parser)]
    comment: String,
}

#[derive(Debug)]
pub struct BugCommentCommand {
    pub args: BugComment,
}

#[async_trait]
impl PipelineCommand for BugCommentCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let problem = |message: String| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message,
            })
        };
        let (bug, comment) = parse_bug_comment(&self.args.comment).ok_or_else(|| {
            problem(format!(
                "{:?} isn't a bug's comment (ex: 1915788/20)",
                self.args.comment
            ))
        })?;
        let view = bug_comment(server, bug, comment).await?;
        let tree = server.tree_info()?.name;
        let sym_info = comment_sym_info(server, &view).await;
        let mut html = Vec::new();
        render_bug_comment(&tree, &view, &sym_info, &mut html)
            .map_err(|message| problem(message.to_string()))?;
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: BUG_COMMENT_MIME.to_string(),
            contents: String::from_utf8(html).unwrap_or_default(),
            transient: view.transient,
        }))
    }
}
