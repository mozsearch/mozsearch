use async_trait::async_trait;
use clap::{Args, ValueEnum};
use itertools::Itertools;

use super::{
    interface::{BatchGroupItem, BatchGroups, PipelineCommand, PipelineValues},
    transforms::{path_case, path_glob_transform},
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

    /// Leave out paths matching a regexp (ex: the query language's `-path:`).
    #[clap(long, value_parser)]
    not_pathre: Option<String>,

    /// The most files to list (like `/search/`'s), noting whether there were
    /// more (see `FileMatches::limit_hit`).
    #[clap(short, long, value_parser, default_value = "4000")]
    limit: usize,

    #[clap(long, value_parser)]
    include_dirs: bool,

    /// Match the paths' case (the query language's `case:sensitive`); they
    /// ignore it otherwise, as router.py's `/search/` does.
    #[clap(long, value_parser)]
    case_sensitive: bool,

    #[clap(long, short, value_parser, value_enum)]
    group_by: Option<GroupFilesBy>,
}

#[derive(Debug)]
pub struct SearchFilesCommand {
    pub args: SearchFiles,
}

const FILE_MATCH_LIMIT: usize = 2_000_000;

impl SearchFiles {
    /// The regexps of the paths to match and to leave out (see
    /// `AbstractServer::search_files`), ignoring case unless it's
    /// `case_sensitive`.
    fn patterns(&self) -> (String, String) {
        let pathre = if let Some(pathre) = &self.pathre {
            pathre.clone()
        } else if let Some(path) = &self.path {
            path_glob_transform(path)
        } else {
            "".to_string()
        };
        let not_pathre = self.not_pathre.clone().unwrap_or_default();
        (
            path_case(pathre, self.case_sensitive),
            path_case(not_pathre, self.case_sensitive),
        )
    }
}

#[async_trait]
impl PipelineCommand for SearchFilesCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let (pathre_pattern, not_pathre_pattern) = self.args.patterns();

        // A zero limit implies no limit, but the server currently needs us to
        // provide a limit because it uses take().  Also, it's probably
        // reasonable to have a bit of a limit, so we also use this as a max.
        let use_limit = if self.args.limit == 0 || self.args.limit > FILE_MATCH_LIMIT {
            FILE_MATCH_LIMIT
        } else {
            self.args.limit
        };

        // (One more than the limit, to know if there were more.)
        let mut matches = server
            .search_files(
                &pathre_pattern,
                &not_pathre_pattern,
                self.args.include_dirs,
                use_limit + 1,
            )
            .await?;
        matches.limit_hit = matches.file_matches.len() > use_limit;
        matches.file_matches.truncate(use_limit);

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
                            limit_hit: false,
                        }),
                    })
                    .collect();

                Ok(PipelineValues::BatchGroups(BatchGroups { groups }))
            }
            None => Ok(PipelineValues::FileMatches(matches)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SearchFiles;
    use clap::Parser;

    #[derive(Parser)]
    struct Command {
        #[clap(flatten)]
        args: SearchFiles,
    }

    /// The paths' regexps ignore case (as router.py's file search does),
    /// unless asked not to (`case:sensitive`).
    #[test]
    fn test_patterns() {
        let patterns = |args: &[&str]| {
            Command::parse_from(std::iter::once("search-files").chain(args.iter().copied()))
                .args
                .patterns()
        };
        assert_eq!(
            patterns(&["httpbasechannel.cpp"]),
            (r"(?i)httpbasechannel\.cpp".to_string(), String::new())
        );
        assert_eq!(
            patterns(&["--pathre", "^netwerk/", "--not-pathre", "test"]),
            ("(?i)^netwerk/".to_string(), "(?i)test".to_string())
        );
        assert_eq!(
            patterns(&["HttpBaseChannel.cpp", "--case-sensitive"]),
            (r"HttpBaseChannel\.cpp".to_string(), String::new())
        );
        assert_eq!(patterns(&[]), (String::new(), String::new()));
        let re = regex::Regex::new(&patterns(&["httpbasechannel.cpp"]).0).unwrap();
        assert!(re.is_match("netwerk/protocol/http/HttpBaseChannel.cpp"));
    }
}
