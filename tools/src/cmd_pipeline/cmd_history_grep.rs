use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues};
use super::transforms::path_glob_transform;

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::hyperblame::history_grep::{HistoryGrep, HistoryGrepArgs, MAX_PATH_FILES};
use crate::query_limits::{QueryLimits, Tier};

/// The revisions where a pattern's matches in a file changed (the query
/// language's `text-history:` and `re-history:`; see
/// `hyperblame::history_grep`), a page of them, newest first.
#[derive(Debug, Args)]
pub struct HistoryGrepCommandArgs {
    /// Text to look for; this will be regexp escaped.
    #[clap(value_parser)]
    text: Option<String>,

    /// A regular expression to look for.  This can't be used if `text` is.
    #[clap(long, value_parser)]
    re: Option<String>,

    #[clap(short, long, value_parser)]
    case_sensitive: bool,

    /// Only files whose paths match this (and whose every revision is checked
    /// if it names few enough; see `history_grep`).
    #[clap(long, value_parser)]
    pathre: Option<String>,

    /// The same, as a path (glob), as the query language's `path:`.
    #[clap(long, value_parser)]
    path: Option<String>,

    #[clap(long, value_parser)]
    not_pathre: Option<String>,

    /// Only revisions since this date (YYYY-MM-DD).
    #[clap(long, value_parser)]
    since: Option<String>,

    /// Only revisions until this date (YYYY-MM-DD).
    #[clap(long, value_parser)]
    until: Option<String>,

    /// Where the page starts in the candidate revisions.
    #[clap(long, value_parser, default_value = "0")]
    page: usize,
}

#[derive(Debug)]
pub struct HistoryGrepCommand {
    pub args: HistoryGrepCommandArgs,
    limits: QueryLimits,
    tier: Tier,
}

impl HistoryGrepCommand {
    pub fn new(args: HistoryGrepCommandArgs) -> Self {
        HistoryGrepCommand {
            args,
            limits: QueryLimits::unlimited(),
            tier: Tier::Local,
        }
    }
}

#[async_trait]
impl PipelineCommand for HistoryGrepCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let (pattern, text) = match (&self.args.re, &self.args.text) {
            (Some(re), _) => (re.clone(), None),
            (None, Some(text)) => (regex::escape(text), Some(text.clone())),
            (None, None) => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::BadInput,
                    message: "Missing text or `re` pattern to look for".to_string(),
                }));
            }
        };
        let pathre = match (&self.args.pathre, &self.args.path) {
            (Some(pathre), _) => pathre.clone(),
            (None, Some(path)) => path_glob_transform(path),
            (None, None) => String::new(),
        };
        let not_pathre = self.args.not_pathre.clone().unwrap_or_default();
        // (The files a path names, if few, whose every revision can be
        // checked.)
        let mut files = vec![];
        if !pathre.is_empty()
            && let Ok(matches) = server
                .search_files(&pathre, &not_pathre, false, MAX_PATH_FILES + 1)
                .await
            && !matches.limit_hit
            && matches.file_matches.len() <= MAX_PATH_FILES
        {
            files = matches
                .file_matches
                .into_iter()
                .map(|file| file.path.to_string())
                .collect();
        }
        let args = HistoryGrepArgs {
            pattern,
            text,
            case_sensitive: self.args.case_sensitive,
            pathre,
            not_pathre,
            files,
            since: self.args.since.clone(),
            until: self.args.until.clone(),
            page: self.args.page,
            prefer_files: false,
        };
        match server.history_grep(&args, &self.limits, self.tier).await {
            Ok(found) => Ok(PipelineValues::HistoryGrep(found)),
            Err(ServerError::Unsupported) => Ok(PipelineValues::HistoryGrep(HistoryGrep {
                pattern: args.pattern,
                text: args.text,
                unsupported: true,
                ..Default::default()
            })),
            Err(err) => Err(err),
        }
    }

    /// A history grep is an expensive command (see `query_limits`).
    fn set_limits(&mut self, limits: &QueryLimits, tier: Tier) -> bool {
        self.limits = limits.clone();
        self.tier = tier;
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::hyperblame::history_grep::{HistoryGrep, HistoryGrepRow};
    use crate::templating::builder::build_and_parse_query_results;

    fn render(grep: &HistoryGrep) -> String {
        let null = liquid::model::Value::Nil;
        let results = serde_json::json!({ "HistoryGrep": grep });
        let globals = liquid::object!({
            "results": liquid::model::to_value(&results).unwrap(),
            "query": "text-history:Foo page:3",
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
            "file_facets": null.clone(),
            "too_expensive": null,
        });
        build_and_parse_query_results().render(&globals).unwrap()
    }

    #[test]
    fn test_render() {
        let mut grep = HistoryGrep {
            pattern: "Foo".to_string(),
            text: Some("Foo".to_string()),
            tokens: vec!["Foo".to_string()],
            candidates: 30,
            page_start: 3,
            page_end: 10,
            next_page: Some(10),
            checked: 12,
            rows: vec![HistoryGrepRow {
                rev: "a".repeat(40),
                date: "2022-08-15".to_string(),
                header: "Remove <b>Foo</b>".to_string(),
                path: "a.cpp".to_string(),
                before: 1,
                after: 0,
                removed_lines: vec!["Foo();".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let page = render(&grep);
        assert!(page.contains("Where matches of\n    <code>Foo</code>"), "{}", page);
        assert!(page.contains("revisions 4–10"));
        assert!(page.contains("1 → 0"));
        assert!(page.contains("− <code>Foo();</code>"));
        // (The next page's link replaces the query's page.)
        assert!(page.contains("q=text-history%3AFoo%20page%3A10"), "{}", page);
        grep.tokens.clear();
        grep.files = vec!["a.cpp".to_string()];
        grep.rows[0].added_values = vec![vec![Some("4.0".to_string())]];
        let page = render(&grep);
        assert!(page.contains("of\n    1 file."), "{}", page);
        assert!(page.contains("+ <code>4.0</code>"));
        grep.unsupported = true;
        assert!(render(&grep).contains("no history"));
    }
}
