use async_trait::async_trait;
use clap::Args;

use super::{
    interface::{PipelineCommand, PipelineValues, FileMatches, FileMatch},
    transforms::path_glob_transform,
};

use crate::abstract_server::{AbstractServer, Result};

/// Perform a fulltext search against our livegrep/codesearch server over gRPC.
/// This is local-only at this time.
#[derive(Debug, Args)]
pub struct SearchFiles {
    /// Path to search for; this will be searchfox glob-transformed.
    #[clap(value_parser)]
    path: Option<String>,

    /// Constrain matching path patterns with a regexp.
    #[clap(long, value_parser)]
    pathre: Option<String>,

    #[clap(short, long, value_parser, default_value = "1000")]
    limit: usize,
}

#[derive(Debug)]
pub struct SearchFilesCommand {
    pub args: SearchFiles,
}

#[async_trait]
impl PipelineCommand for SearchFilesCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let pathre_pattern = if let Some(pathre) = &self.args.pathre {
            pathre.clone()
        } else if let Some(path) = &self.args.path {
            path_glob_transform(path)
        } else {
            "".to_string()
        };

        let matches = server
            .search_files(&pathre_pattern, self.args.limit)
            .await?;

        Ok(PipelineValues::FileMatches(FileMatches {
            file_matches: matches.into_iter().map(|path| {
                FileMatch {
                    path
                }
            }).collect()
        }))
    }
}
