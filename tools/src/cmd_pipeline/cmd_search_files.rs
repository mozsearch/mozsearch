use async_trait::async_trait;
use clap::{Args, ValueEnum};
use itertools::Itertools;

use super::{
    interface::{BatchGroupItem, BatchGroups, PipelineCommand, PipelineValues},
    transforms::path_glob_transform,
};

use crate::abstract_server::{AbstractServer, FileMatches, Result};

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum GroupFilesBy {
    Directory,
}

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

    #[clap(short, long, value_parser, default_value = "2000")]
    limit: usize,

    #[clap(long, value_parser)]
    include_dirs: bool,

    #[clap(long, short, value_parser, value_enum)]
    group_by: Option<GroupFilesBy>,
}

#[derive(Debug)]
pub struct SearchFilesCommand {
    pub args: SearchFiles,
}

const FILE_MATCH_LIMIT: usize = 2_000_000;

#[async_trait]
impl PipelineCommand for SearchFilesCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let pathre_pattern = if let Some(pathre) = &self.args.pathre {
            pathre.clone()
        } else if let Some(path) = &self.args.path {
            path_glob_transform(path)
        } else {
            "".to_string()
        };

        // A zero limit implies no limit, but the server currently needs us to
        // provide a limit because it uses take().  Also, it's probably
        // reasonable to have a bit of a limit, so we also use this as a max.
        let use_limit = if self.args.limit == 0 || self.args.limit > FILE_MATCH_LIMIT {
            FILE_MATCH_LIMIT
        } else {
            self.args.limit
        };

        let matches = server
            .search_files(&pathre_pattern, self.args.include_dirs, use_limit)
            .await?;

        match self.args.group_by {
            Some(GroupFilesBy::Directory) => {
                let groups: Vec<_> = matches
                    .file_matches
                    .into_iter()
                    .into_group_map_by(|f| f.get_containing_dir())
                    .into_iter()
                    .map(|(dir, matches)| BatchGroupItem {
                        name: dir.to_string(),
                        value: PipelineValues::FileMatches(FileMatches {
                            file_matches: matches,
                        }),
                    })
                    .collect();

                Ok(PipelineValues::BatchGroups(BatchGroups { groups }))
            }
            None => Ok(PipelineValues::FileMatches(matches)),
        }
    }
}
