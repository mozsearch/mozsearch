use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues, TokenNamesValue};
use crate::abstract_server::{AbstractServer, Result, ServerError};
use crate::file_format::token_names::NameMatch;

/// The tokens of the history's journals matching a pattern (see
/// `file_format::token_names`): `Foo*` those starting with `Foo`, `*Foo` those
/// ending with it, and `Foo` (or `*Foo*`) those containing it, case- and
/// accent-insensitively; the `token-names:` term's.
#[derive(Debug, Args)]
pub struct TokenNamesArgs {
    #[clap(value_parser)]
    pattern: String,

    /// The most names to list.
    #[clap(long, value_parser, default_value = "200")]
    limit: usize,
}

#[derive(Debug)]
pub struct TokenNamesCommand {
    pub args: TokenNamesArgs,
}

/// A pattern's text and how it matches names.
pub fn parse_pattern(pattern: &str) -> (&str, NameMatch) {
    let pattern = pattern.trim();
    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some(rest), _) if rest.ends_with('*') => (&rest[..rest.len() - 1], NameMatch::Contains),
        (Some(rest), _) => (rest, NameMatch::Suffix),
        (None, Some(rest)) => (rest, NameMatch::Prefix),
        (None, None) => (pattern, NameMatch::Contains),
    }
}

#[async_trait]
impl PipelineCommand for TokenNamesCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let (query, how) = parse_pattern(&self.args.pattern);
        let mut value = TokenNamesValue {
            query: query.to_string(),
            how,
            names: vec![],
            more: false,
            unsupported: false,
        };
        match server
            .token_names(query, how, self.args.limit.clamp(1, 1000))
            .await
        {
            Ok(matches) => {
                value.names = matches.names;
                value.more = matches.more;
            }
            Err(ServerError::Unsupported) => value.unsupported = true,
            Err(err) => return Err(err),
        }
        Ok(PipelineValues::TokenNames(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pattern() {
        assert_eq!(parse_pattern("Foo*"), ("Foo", NameMatch::Prefix));
        assert_eq!(parse_pattern("*Foo"), ("Foo", NameMatch::Suffix));
        assert_eq!(parse_pattern("*Foo*"), ("Foo", NameMatch::Contains));
        assert_eq!(parse_pattern("Foo"), ("Foo", NameMatch::Contains));
    }
}
