use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues};
use super::transforms::{path_case, path_glob_transform};

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

    /// Chart the matches over time (`text-timeline:`, `re-timeline:`): all
    /// of the candidates at once, preferring files' every revision.
    #[clap(long, value_parser)]
    timeline: bool,

    /// Only matches in this scope (a class's, function's, ... qualified
    /// name), in the files `path` names, or else in the files defining it.
    #[clap(long, value_parser)]
    scope: Option<String>,

    /// Show backed out revisions and their backouts (which are otherwise
    /// left out where the backout undid the revision; see
    /// `history_grep::hide_backout_pairs`).
    #[clap(long, value_parser)]
    show_backouts: bool,

    /// What the query had that a history grep doesn't take, which the page
    /// says it ignored (see `chew_query`'s exclusive groups).
    #[clap(long, value_parser)]
    ignored: Option<String>,
}

/// The files defining `scope` (a qualified name), by its symbols' crossref
/// definitions, up to `MAX_PATH_FILES`.
async fn scope_files(server: &(dyn AbstractServer + Send + Sync), scope: &str) -> Vec<String> {
    let mut files: Vec<String> = vec![];
    let Ok(symbols) = server.search_identifiers(scope, true, false, 16).await else {
        return files;
    };
    for (symbol, _) in symbols {
        let Ok(Some(crossref)) = server.crossref_lookup(&symbol).await else {
            continue;
        };
        for def in crossref.definitions.iter().flatten() {
            let path = def.path.to_string();
            if !files.contains(&path) && files.len() < MAX_PATH_FILES {
                files.push(path);
            }
        }
    }
    files
}

/// The most lines the tree's text search may find with the text now, for its
/// files (see `now_files`).
const MAX_NOW_LINES: usize = 1000;

/// The files which have a history grep's text now (by the tree's text search,
/// in its paths, with its case), if they're few enough for their every
/// revision to be checked (`MAX_PATH_FILES`), or why they aren't.
async fn now_files(
    server: &(dyn AbstractServer + Send + Sync),
    text: &str,
    args: &HistoryGrepArgs,
) -> std::result::Result<Vec<String>, String> {
    // (livegrep's RE2 has POSIX syntax: its metacharacters escaped, and no
    // `(?i)`, which the paths' case is anyway.)
    let mut pattern = String::new();
    for c in text.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    let path = |re: &str| re.trim_start_matches("(?i)").to_string();
    let found = server
        .search_text(
            &pattern,
            !args.case_sensitive,
            &path(&args.pathre),
            &path(&args.not_pathre),
            MAX_NOW_LINES,
        )
        .await
        .map_err(|err| format!("the tree's text search for it failed ({:?})", err))?;
    if found.timed_out {
        return Err("the tree's text search for it timed out".to_string());
    }
    if found.limit_hit {
        return Err(format!("over {} lines have it now", MAX_NOW_LINES));
    }
    let files: Vec<String> = found.by_file.iter().map(|file| file.file.to_string()).collect();
    if files.len() > MAX_PATH_FILES {
        return Err(format!(
            "{} files have it now, more than the {} whose every revision can be checked (a path: can name fewer)",
            files.len(),
            MAX_PATH_FILES
        ));
    }
    Ok(files)
}

/// A page of the token journals' revisions, with the revisions of the files
/// which have the text now (`now`, on the first page): theirs, and the
/// journals' others (which may be the same file's then, by another path,
/// or another file's), newest first.
fn merge_now(mut found: HistoryGrep, now_files: &[String], now: Option<HistoryGrep>) -> HistoryGrep {
    let in_now = |path: &str| now_files.iter().any(|file| file == path);
    match now {
        Some(now) => {
            let seen: std::collections::HashSet<(String, String)> = now
                .rows
                .iter()
                .map(|row| (row.rev.clone(), row.path.clone()))
                .collect();
            found.rows.retain(|row| {
                !in_now(&row.path) && !seen.contains(&(row.rev.clone(), row.path.clone()))
            });
            found.now_candidates = now.candidates;
            found.rows.extend(now.rows);
            // (Stable: each's own order within a day.)
            found.rows.sort_by(|a, b| b.date.cmp(&a.date));
        }
        // (A later page: the first had theirs.)
        None => found.rows.retain(|row| !in_now(&row.path)),
    }
    found
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
        // (Ignoring case, but with `case:sensitive`, as the files'.)
        let pathre = path_case(
            match (&self.args.pathre, &self.args.path) {
                (Some(pathre), _) => pathre.clone(),
                (None, Some(path)) => path_glob_transform(path),
                (None, None) => String::new(),
            },
            self.args.case_sensitive,
        );
        let not_pathre = path_case(
            self.args.not_pathre.clone().unwrap_or_default(),
            self.args.case_sensitive,
        );
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
        // (A scope's files, if no path names them: those defining it.)
        if let Some(scope) = &self.args.scope
            && pathre.is_empty()
        {
            files = scope_files(server, scope).await;
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
            prefer_files: self.args.timeline,
            timeline: self.args.timeline,
            scope: self.args.scope.clone(),
            show_backouts: self.args.show_backouts,
            ignored: self.args.ignored.clone(),
        };
        // (Text, searched for in the token journals' revisions: also every
        // revision of the files which have it now, as `path:` would check
        // them, which sees what the journals can't (ex: a match made by
        // changing the code around its tokens, or in a string).  See
        // `now_files`.)
        let now = match &args.text {
            Some(text) if args.files.is_empty() && args.scope.is_none() && !args.timeline => {
                Some(now_files(server, text, &args).await)
            }
            _ => None,
        };
        let found = server.history_grep(&args, &self.limits, self.tier).await;
        let (found, now) = match (found, now) {
            (Err(ServerError::Unsupported), _) => {
                return Ok(PipelineValues::HistoryGrep(HistoryGrep {
                    pattern: args.pattern,
                    text: args.text,
                    unsupported: true,
                    ..Default::default()
                }));
            }
            (found, None) => return found.map(PipelineValues::HistoryGrep),
            (found, Some(now)) => (found, now),
        };
        let now_files = match now {
            Ok(files) => files,
            Err(note) => {
                return found.map(|mut found| {
                    found.now_note = Some(note);
                    PipelineValues::HistoryGrep(found)
                });
            }
        };
        // (Their every revision, on the first page, all of them.)
        let mut now_found: Option<HistoryGrep> = None;
        if args.page == 0 && !now_files.is_empty() {
            let mut now_args = HistoryGrepArgs {
                files: now_files.clone(),
                prefer_files: true,
                ..args.clone()
            };
            loop {
                match server.history_grep(&now_args, &self.limits, self.tier).await {
                    Ok(page) => {
                        let next = page.next_page;
                        match &mut now_found {
                            None => now_found = Some(page),
                            Some(all) => {
                                all.rows.extend(page.rows);
                                all.checked += page.checked;
                                all.bytes += page.bytes;
                            }
                        }
                        match next {
                            Some(next) => now_args.page = next,
                            None => break,
                        }
                    }
                    Err(ServerError::TooExpensive(refused)) => {
                        let note = format!(
                            "checking their every revision is too expensive: {}",
                            refused.details.join("; ")
                        );
                        return found.map(|mut found| {
                            found.now_note = Some(note);
                            PipelineValues::HistoryGrep(found)
                        });
                    }
                    Err(err) => return Err(err),
                }
            }
        }
        let mut found = match (found, now_found) {
            (Ok(found), now_found) => merge_now(found, &now_files, now_found),
            // (The token journals' revisions couldn't be checked, ex: the text
            // has no name the history keeps, but the files which have it now
            // could be.)
            (Err(ServerError::TooExpensive(refused)), Some(mut now_found)) => {
                now_found.tokens_note = Some(refused.details.join("; "));
                now_found.now_candidates = now_found.candidates;
                now_found.now_files = now_files.clone();
                now_found
            }
            (Err(err), _) => return Err(err),
        };
        found.now_files = now_files;
        Ok(PipelineValues::HistoryGrep(found))
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

            "corrections": liquid::model::Value::Array(vec![]),
        });
        build_and_parse_query_results().render(&globals).unwrap()
    }

    fn row(rev: &str, date: &str, path: &str) -> HistoryGrepRow {
        HistoryGrepRow {
            rev: rev.repeat(40),
            date: date.to_string(),
            path: path.to_string(),
            ..Default::default()
        }
    }

    /// The files which have the text now's every revision, with the token
    /// journals' others, newest first; and on a later page, the journals'
    /// others alone.
    #[test]
    fn test_merge_now() {
        let found = HistoryGrep {
            tokens: vec!["SchemeIs".to_string()],
            rows: vec![
                row("a", "2026-06-22", "old/b.js"),
                row("c", "2026-01-01", "now.cpp"),
            ],
            ..Default::default()
        };
        let now = HistoryGrep {
            candidates: 230,
            rows: vec![
                row("d", "2026-09-18", "now.cpp"),
                row("a", "2026-06-22", "old/b.js"),
                row("e", "2019-07-30", "now.cpp"),
            ],
            ..Default::default()
        };
        let files = vec!["now.cpp".to_string()];
        let merged = super::merge_now(found.clone(), &files, Some(now));
        let revs: Vec<(char, &str)> = merged
            .rows
            .iter()
            .map(|row| (row.rev.chars().next().unwrap(), row.path.as_str()))
            .collect();
        assert_eq!(
            revs,
            vec![('d', "now.cpp"), ('a', "old/b.js"), ('e', "now.cpp")]
        );
        assert_eq!(merged.now_candidates, 230);
        let later = super::merge_now(found, &files, None);
        assert_eq!(later.rows.len(), 1);
        assert_eq!(later.rows[0].path, "old/b.js");
    }

    /// The page says which files' every revision it checked too, or why it
    /// didn't, and that it could only check those.
    #[test]
    fn test_render_now() {
        let mut grep = HistoryGrep {
            pattern: "Foo".to_string(),
            text: Some("Foo".to_string()),
            tokens: vec!["Foo".to_string()],
            now_files: vec!["a.cpp".to_string(), "b.cpp".to_string()],
            now_candidates: 12,
            ..Default::default()
        };
        let page = render(&grep);
        assert!(page.contains("every revision (12) of the 2 files which have it now"), "{}", page);
        // (Its files listed in a collapsed `<details>`, as file name matches
        // are, with their paths' links.)
        assert!(page.contains(r#"<details class="history-grep-now-files">"#), "{}", page);
        assert!(page.contains("The 2 files which have it now</summary>"), "{}", page);
        assert!(page.contains(r#"<a href="/tests/source/b.cpp">b.cpp</a>"#), "{}", page);
        grep.now_files.clear();
        grep.now_note = Some("60 files have it now".to_string());
        let page = render(&grep);
        assert!(page.contains("weren't checked: 60 files have it now"), "{}", page);
        grep.now_note = None;
        grep.now_files = vec!["a.cpp".to_string()];
        grep.tokens.clear();
        grep.files = vec!["a.cpp".to_string()];
        grep.tokens_note = Some("no name every match contains".to_string());
        let page = render(&grep);
        assert!(page.contains("Only the files which have it now"), "{}", page);
        assert!(!page.contains("And every revision"), "{}", page);
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
                removed_line_counts: vec![1],
                ..Default::default()
            }],
            ..Default::default()
        };
        let page = render(&grep);
        assert!(
            page.contains("Where matches of\n    <code>Foo</code>"),
            "{}",
            page
        );
        assert!(page.contains("revisions 4–10"));
        assert!(page.contains("1 → 0"));
        assert!(page.contains("− <code>Foo();</code>"));
        // (The next page's link replaces the query's page.)
        assert!(
            page.contains("q=text-history%3AFoo%20page%3A10"),
            "{}",
            page
        );
        assert!(page.contains("revisions 4–10, newest first"), "{}", page);
        assert!(page.contains("a.cpp had 1 match before aaaaaaaaaaaa, and 0 after"));
        grep.tokens.clear();
        grep.files = vec!["a.cpp".to_string()];
        // (A value once, with how many more of it.)
        grep.rows[0].added_values =
            vec![vec![Some("4.0".to_string())], vec![Some("5.0".to_string())]];
        grep.rows[0].added_value_counts = vec![3, 1];
        let page = render(&grep);
        assert!(page.contains("of\n    1 file."), "{}", page);
        assert!(
            page.contains("+ <code>4.0</code> <span class=\"history-grep-times\">×3</span>"),
            "{}",
            page
        );
        assert!(page.contains("+ <code>5.0</code></div>"), "{}", page);
        grep.unsupported = true;
        assert!(render(&grep).contains("no history"));
    }
}
