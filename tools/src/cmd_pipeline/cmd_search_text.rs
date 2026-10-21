use async_trait::async_trait;
use clap::Args;

use super::{
    interface::{PipelineCommand, PipelineValues},
    transforms::path_glob_transform,
};

use crate::abstract_server::{
    AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError, TextPattern,
};

/// Perform a fulltext search against our livegrep/codesearch server over gRPC.
/// This is local-only at this time.
/// The widest pattern codesearch (livegrep) takes: what a pattern can match
/// at most, in characters (`kMaxWidth` in its src/tools/limits.h), else it
/// answers "Parse error".  A text search for something longer (ex: a pasted
/// URL) is of its first `MAX_PATTERN_WIDTH` characters, and its matches are
/// the lines with all of it.
pub const MAX_PATTERN_WIDTH: usize = 200;

/// Search the tree's files' text: for `text` (as it is), or the regular
/// expression `re`.
#[derive(Debug, Args)]
pub struct SearchText {
    /// Text to search for; this will be regexp escaped.
    #[clap(value_parser)]
    text: Option<String>,

    /// Search for a regular expression.  This can't be used if `text` is used.
    #[clap(long, value_parser)]
    re: Option<String>,

    /// Constrain matching path patterns with a non-regexp path constraint that
    /// will be escaped into a regexp.
    #[clap(long, value_parser)]
    path: Option<String>,

    /// Constrain matching path patterns with a regexp.
    #[clap(long, value_parser)]
    pathre: Option<String>,

    /// Leave out files whose paths match a regexp (ex: the query language's
    /// `-path:`).
    #[clap(long, value_parser)]
    not_pathre: Option<String>,

    /// Should this be case-sensitive?  By default we are case-insensitive.
    #[clap(short, long, value_parser)]
    case_sensitive: bool,

    #[clap(short, long, value_parser, default_value = "0")]
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
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        // (A text wider than codesearch takes is searched for by a piece of
        // it, whose matches are then those of all of it; see
        // `MAX_PATTERN_WIDTH`.)
        let mut whole_text: Option<regex::Regex> = None;
        let re_pattern = if let Some(re) = &self.args.re {
            re.clone()
        } else if let Some(text) = &self.args.text {
            if text.chars().count() > MAX_PATTERN_WIDTH {
                whole_text = regex::RegexBuilder::new(&regex::escape(text))
                    .case_insensitive(!self.args.case_sensitive)
                    .build()
                    .ok();
            }
            match whole_text {
                Some(_) => regex::escape(&text.chars().take(MAX_PATTERN_WIDTH).collect::<String>()),
                None => regex::escape(text),
            }
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

        let mut matches = server
            .search_text(
                &re_pattern,
                !self.args.case_sensitive,
                &pathre_pattern,
                self.args.not_pathre.as_deref().unwrap_or_default(),
                self.args.limit,
            )
            .await?;
        if let Some(whole) = &whole_text {
            for file in &mut matches.by_file {
                file.matches.retain_mut(|m| match whole.find(&m.line_str) {
                    Some(found) => {
                        m.bounds.start = found.start() as i32;
                        m.bounds.end_exclusive = found.end() as i32;
                        true
                    }
                    None => false,
                });
            }
            matches.by_file.retain(|file| !file.matches.is_empty());
        }
        matches.pattern = Some(TextPattern {
            re: match (&whole_text, &self.args.text) {
                (Some(_), Some(text)) => regex::escape(text),
                _ => re_pattern,
            },
            case_sensitive: self.args.case_sensitive,
        });

        Ok(PipelineValues::TextMatches(matches))
    }
}
