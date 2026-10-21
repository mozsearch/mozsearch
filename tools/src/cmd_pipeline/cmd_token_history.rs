use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues, TokenHistoryValue};
use crate::abstract_server::{AbstractServer, Result, ServerError};
use crate::file_format::history::timeline_tokens::is_trackable_token;
use crate::hyperblame::token_history::TokenHistoryOptions;

/// What the history says about a token's removals (see
/// `hyperblame::token_history`): the commits which removed most of its
/// instances, and what replaced it.  For the default term's note of a search's
/// token (see `cmd_compile_results`), and with `--view`, the `token-history:`
/// term's view, with its journal's records.
#[derive(Debug, Args)]
pub struct TokenHistoryArgs {
    /// The token: the query's text, so that a search for anything else (ex:
    /// several words) has no history.
    #[clap(value_parser)]
    token: String,

    /// Only the token's history, not its case variants'.
    #[clap(long, value_parser)]
    case_sensitive: bool,

    /// The view's: more of the journal, and its records and yearly totals.
    #[clap(long, value_parser)]
    view: bool,
}

#[derive(Debug)]
pub struct TokenHistoryCommand {
    pub args: TokenHistoryArgs,
}

#[async_trait]
impl PipelineCommand for TokenHistoryCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let token = self.args.token.trim();
        let mut value = TokenHistoryValue {
            token: token.to_string(),
            history: None,
            unsupported: false,
        };
        if !is_trackable_token(token) {
            return Ok(PipelineValues::TokenHistory(value));
        }
        let opts = TokenHistoryOptions {
            case_sensitive: self.args.case_sensitive,
            view: self.args.view,
        };
        match server.token_history(token, opts).await {
            Ok(history) => value.history = history,
            Err(ServerError::Unsupported) => value.unsupported = true,
            // (The note is an extra: a search shouldn't fail for it.)
            Err(_) if !self.args.view => {}
            Err(err) => return Err(err),
        }
        Ok(PipelineValues::TokenHistory(value))
    }
}

#[cfg(test)]
mod tests {
    use crate::hyperblame::token_history::{RemovalCommit, TokenHistory};
    use crate::templating::builder::build_and_parse_query_results;

    /// The query page with `results`.
    fn render(results: serde_json::Value) -> String {
        let null = liquid::model::Value::Nil;
        let globals = liquid::object!({
            "results": liquid::model::to_value(&results).unwrap(),
            "query": "q",
            "interpretation": null.clone(),
            "dwim": null.clone(),
            "preset": "default",
            "tree": "tests",
            "search_tree": "tests",
            "search_url": "/tests/search",
            "query_search": true,
            "search_rev": "",
            "try_push": "",
            "try_base": "",
            "logs": null.clone(),
            "debug": false,
            "SYM_INFO_STR": "{}",
            "file_facets": null,
        });
        build_and_parse_query_results().render(&globals).unwrap()
    }

    #[test]
    fn test_render() {
        // (A history without files, symbols, a week or a replacement, whose
        // empty fields the templates compare.)
        let history = TokenHistory {
            token: "Gadget".to_string(),
            removed: 19,
            peak: 19,
            since: "2020-W10".to_string(),
            until: "2020-W10".to_string(),
            removals: 1,
            commits: vec![RemovalCommit {
                rev: "b".repeat(40),
                date: "2020-03-04".to_string(),
                removed: 15,
                ..Default::default()
            }],
            ..Default::default()
        };
        let note = render(serde_json::json!({ "FlattenedResultsBundle": {
            "path_kind_results": [],
            "content_type": "text/plain",
            "token_history": history,
        }}));
        assert!(note.contains("19 instances of <code>Gadget</code> were removed"));
        assert!(note.contains("token-history:Gadget%20case:sensitive"));
        let view = render(serde_json::json!({ "TokenHistory": {
            "token": "gadget",
            "history": history,
            "unsupported": false,
        }}));
        assert!(view.contains("The history of <code>Gadget</code>"));
        let none = render(serde_json::json!({ "TokenHistory": {
            "token": "Gizmo",
            "history": null,
            "unsupported": true,
        }}));
        assert!(none.contains("no token-centric history"));
    }
}
