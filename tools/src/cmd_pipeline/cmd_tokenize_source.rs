use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues, TextFile};
use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    tree_sitter_support::cst_tokenizer::hypertokenize_source_file,
};

/// Tokenize the given source file using the syntax-token-tree tokenizer.  We
/// do also have the HTML formatting hand-rolled tokenizers, but they aren't
/// hooked up right now.
#[derive(Debug, Args)]
pub struct TokenizeSource {
    /// Tree-relative source file path or directory.
    #[clap(value_parser)]
    file: String,
}

#[derive(Debug)]
pub struct TokenizeSourceCommand {
    pub args: TokenizeSource,
}

#[async_trait]
impl PipelineCommand for TokenizeSourceCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let source_str = server.fetch_raw_source(&self.args.file).await?;

        let token_lines = match hypertokenize_source_file(&self.args.file, &source_str) {
            Ok(content) => content,
            Err(e) => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::DataLayer,
                    message: e,
                }));
            }
        };

        Ok(PipelineValues::TextFile(TextFile {
            mime_type: "text/plain".to_string(),
            contents: token_lines.join("\n"),
        }))
    }
}
