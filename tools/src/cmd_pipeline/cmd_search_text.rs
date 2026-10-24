use async_trait::async_trait;
use clap::Args;

use super::{
    full_grep::{FullGrep, capture_names, compile, full_grep},
    interface::{PipelineCommand, PipelineValues},
    transforms::path_glob_transform,
};

use crate::abstract_server::{
    AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError, TextPattern,
};
use crate::query_limits::{QueryLimits, Tier};
use crate::regex_plan::{self, RegexPlan};

/// Perform a fulltext search against our livegrep/codesearch server over gRPC.
/// This is local-only at this time.
/// The widest pattern codesearch (livegrep) takes: what a pattern can match
/// at most, in characters (`kMaxWidth` in its src/tools/limits.h), else it
/// answers "Parse error".  A text search for something longer (ex: a pasted
/// URL) is of its first `MAX_PATTERN_WIDTH` characters, and its matches are
/// the lines with all of it.
pub const MAX_PATTERN_WIDTH: usize = 200;

/// Search the tree's files' text: for `text` (as it is), or the regular
/// expression `re`, with livegrep, or with the fuller engine for patterns
/// livegrep can't run (see `full_grep`).
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

    /// Tally the values its matches' capture groups captured (the query
    /// language's `tally:`): "all", a group's number, or its name (see
    /// `cmd_compile_results::Tally`).
    #[clap(long, value_parser)]
    tally: Option<String>,
}

#[derive(Debug)]
pub struct SearchTextCommand {
    pub args: SearchText,
    /// `re`'s analysis, if fancy-regex parses it (see `regex_plan`).
    plan: Option<RegexPlan>,
    /// What the fuller engine may read (see `query_limits`).
    limits: QueryLimits,
    tier: Tier,
}

impl SearchTextCommand {
    pub fn new(args: SearchText) -> Self {
        let plan = args
            .re
            .as_deref()
            .and_then(|re| regex_plan::plan(re, !args.case_sensitive).ok());
        SearchTextCommand {
            args,
            plan,
            limits: QueryLimits::unlimited(),
            tier: Tier::Local,
        }
    }

    /// Search for `re` with the fuller engine.
    async fn full_grep(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        re: &str,
        plan: &RegexPlan,
        pathre: &str,
    ) -> Result<PipelineValues> {
        let search = FullGrep {
            pattern: re,
            fold_case: !self.args.case_sensitive,
            pathre,
            not_pathre: self.args.not_pathre.as_deref().unwrap_or_default(),
            limit: self.args.limit,
        };
        let mut matches = full_grep(server, &search, plan, &self.limits, self.tier).await?;
        matches.pattern = Some(TextPattern {
            re: re.to_string(),
            case_sensitive: self.args.case_sensitive,
            spans_lines: plan.spans_lines,
            capture_names: compile(re, !self.args.case_sensitive)
                .map(|re| capture_names(&re))
                .unwrap_or_default(),
            tally: self.args.tally.clone(),
        });
        Ok(PipelineValues::TextMatches(matches))
    }
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

        // (A pattern livegrep can't run goes to the fuller engine, as does
        // one it can't parse after all, ex: one wider than it takes.)
        if let (Some(re), Some(plan)) = (&self.args.re, &self.plan)
            && !plan.livegrep_can_run()
        {
            return self.full_grep(server, re, plan, &pathre_pattern).await;
        }
        let searched = server
            .search_text(
                &re_pattern,
                !self.args.case_sensitive,
                &pathre_pattern,
                self.args.not_pathre.as_deref().unwrap_or_default(),
                self.args.limit,
            )
            .await;
        let mut matches = match (searched, &self.args.re, &self.plan) {
            (Ok(matches), _, _) => matches,
            (
                Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::BadInput,
                    ..
                })),
                Some(re),
                Some(plan),
            ) => return self.full_grep(server, re, plan, &pathre_pattern).await,
            (Err(err), _, _) => return Err(err),
        };
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
        let re = match (&whole_text, &self.args.text) {
            (Some(_), Some(text)) => regex::escape(text),
            _ => re_pattern,
        };
        let capture_names = match &self.args.re {
            Some(_) => compile(&re, !self.args.case_sensitive)
                .map(|re| capture_names(&re))
                .unwrap_or_default(),
            None => vec![],
        };
        matches.pattern = Some(TextPattern {
            re,
            case_sensitive: self.args.case_sensitive,
            spans_lines: false,
            capture_names,
            tally: self.args.tally.clone(),
        });

        Ok(PipelineValues::TextMatches(matches))
    }

    /// A pattern livegrep can't run is an expensive search (see `full_grep`).
    fn set_limits(&mut self, limits: &QueryLimits, tier: Tier) -> bool {
        self.limits = limits.clone();
        self.tier = tier;
        self.plan
            .as_ref()
            .is_some_and(|plan| !plan.livegrep_can_run())
    }
}
