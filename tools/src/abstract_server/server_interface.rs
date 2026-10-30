use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, FixedOffset};
use futures_core::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use ustr::{Ustr, UstrMap, ustr};

use crate::file_format::chunked_gzip::{Row, extract_rows};
use crate::file_format::code_coverage_report;
use crate::file_format::crossref::CrossrefData;
use crate::file_format::jumpref::JumprefData;
use crate::file_format::recency::FileRecency;
use crate::file_format::repo_data_ingestion::ConcisePerFileInfo;
use crate::file_format::token_names::{NameMatch, TokenNameMatches};
use crate::git_ops::RevisionCoverage;
use crate::hyperblame::journals::JournalKind;
use crate::hyperblame::token_history::{TokenHistory, TokenHistoryOptions};
use crate::hyperblame::token_timeline::{TokenTimeline, TokenTimelineOptions};

pub type Result<T> = std::result::Result<T, ServerError>;

// JSON parse errors are sticky data problems.
impl From<serde_json::Error> for ServerError {
    fn from(err: serde_json::Error) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::DataLayer,
            message: err.to_string(),
        })
    }
}

// RegExps that are part of our code will be unwrap()ed inline to panic in
// tests, and config file regexps should have their errors handled inline,
// leaving us able to assume (and transform) any remaining regexp errors as
// relating to user input.
impl From<regex::Error> for ServerError {
    fn from(err: regex::Error) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::BadInput,
            message: format!("bad regexp: {}", err),
        })
    }
}

impl From<tokio::task::JoinError> for ServerError {
    fn from(err: tokio::task::JoinError) -> ServerError {
        // There was a debugging case where I needed to uncomment this, but at
        // least when using pipeline-server, we get a useful backtrace in the
        // "pipeline-server.err" file and so this should not be necessary.
        /*
        if let Ok(reason) = err.try_into_panic() {
            // Resume the panic on the main task
            std::panic::resume_unwind(reason);
        }
        */
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::RuntimeInvariantViolation,
            message: format!("task panicked?: {}", err),
        })
    }
}

impl From<liquid::Error> for ServerError {
    fn from(err: liquid::Error) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::ConfigLayer,
            message: format!("Liquid error: {}", err),
        })
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        if let ServerError::TooExpensive(refused) = &self {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(refused.to_json()),
            )
                .into_response();
        }
        let body = format!("Error: {:#?}", self);
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

impl ServerError {
    /// What it says, for a page (its details' message, ex: "Bug 1234567 has
    /// no comment 9").
    pub fn message(&self) -> String {
        match self {
            ServerError::StickyProblem(details) | ServerError::TransientProblem(details) => {
                details.message.clone()
            }
            ServerError::Unsupported => "Not available for this tree".to_string(),
            ServerError::TooExpensive(refused) => refused.message(),
        }
    }

    /// Whether it may go away if tried again later (so that a page saying
    /// it isn't cached).
    pub fn is_transient(&self) -> bool {
        matches!(self, ServerError::TransientProblem(_))
    }
}

/// `fetch_html` needs to fetch from a number of parallel directories in the
/// index directory, and this enum maps to those directories.
#[derive(Debug, Eq, PartialEq)]

pub enum HtmlFileRoot {
    /// `INDEX_ROOT/file` root, unified remotely.
    FormattedFile,
    /// `INDEX_ROOT/dir` root for local, unified remotely.
    FormattedDir,
    /// `INDEX_ROOT/templates` root for local, not available remotely.
    FormattedTemplate,
}

/// Express whether the error seems to be happening in the server or the data.
#[derive(Debug)]
pub enum ErrorLayer {
    /// The request itself has structural issues like a malformed URL.  This
    /// should not be used for cases where the user input results in a search
    /// miss (which should instead be part of the result payload), but is
    /// appropriate for cases where the user input makes it impossible to return
    /// a hit or a miss, like an incorrectly constructed pipeline.
    ///
    /// For example, we would want to throw an error in an "insta" check if the
    /// pipeline is not a valid pipeline.  Or similarly, a shell script invoking
    /// searchfox-tool wants to experience an error if the command pipeline is
    /// incorrect.
    ///
    /// This does potentially end up ambiguous in the web UI case if the web UI
    /// allows the user to construct pipelines that aren't validated before
    /// being sent to the server.  In that case we would want to treat the error
    /// akin to a search miss and not generate errors that would trip alarms.
    /// (Our "insta" checks of course help avoid such problems becoming serious
    /// silent errors, as they would/should not be quieted.)
    BadInput,
    /// The problem seems to involve configuration data, for example in the
    /// query to pipeline mappings.
    ConfigLayer,
    /// The error seems to involve server logic, so it may or may not be an issue
    /// with the underlying data.
    ServerLayer,
    /// The error seems to be related to the indexed data in question rather
    /// than the server, like the data was not indexed.
    DataLayer,
    /// Our data structure doesn't work like it's supposed to and we don't want
    /// to panic, so we return this instead.
    RuntimeInvariantViolation,
    /// We're not sure if it was a server issue or a data issue.
    UnknownLayer,
}

/// ServerError payload to provide details about what went wrong for
/// investigation purposes.  In the future, this could wrap the
/// underlying errors we've seen.
#[derive(Debug)]
pub struct ErrorDetails {
    /// Attempt to distinguish failures due to server bugs from failures due to
    /// indexing bugs.  For example a 500 response from a server would be a
    /// `ServerLayer` problem, but if a 404 was instead returned, that would be
    /// a `DataLayer` problem.
    pub layer: ErrorLayer,
    /// Stringified version of the lower level error.
    pub message: String,
}

/// Does a retry makes sense or not?
///
/// Actually performing retries could of course happen either below this
/// abstraction layer or above it.  The argument for above is that the
/// `cmd_pipeline` could make more informed scheduling decisions with
/// appropriately long back-offs than this lower layer would be able to.  But
/// that's all speculative at this point and this type is really being
/// introduced because we need a unifying error type.
#[derive(Debug)]
pub enum ServerError {
    /// An error that will persist for at least this index.  For example a 404.
    StickyProblem(ErrorDetails),
    /// An error that might go away if retried later.  For example a 504 "Gateway
    /// timeout".
    TransientProblem(ErrorDetails),
    Unsupported,
    /// The query's plan would cost more than its tier's limits allow (see
    /// `query_limits`): a 422, `query-too-expensive`.
    TooExpensive(Box<crate::query_limits::TooExpensive>),
}

/// Livegrep/codesearch bounds
#[derive(Deserialize, Serialize)]
pub struct TextBounds {
    pub start: i32,
    pub end_exclusive: i32,
}

/// Livegrep/codesearch line hit results
#[derive(Deserialize, Serialize)]
pub struct TextMatchInFile {
    pub line_num: u32,
    pub bounds: TextBounds,
    // This will vary a lot and so can never be a Ustr.
    pub line_str: String,
    /// The matches starting on the line, when `search-text`'s fuller engine
    /// found them (see `full_grep`; livegrep only gives `bounds`, the
    /// line's first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<TextHit>,
    /// The lines after this one that `hits` span (for multi-line matches).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_lines: Vec<String>,
}

/// A match found by `search-text`'s fuller engine (see `full_grep`): where
/// it starts and ends (1-based lines, and byte offsets in them; a multi-line
/// match ends on a later line), and what its capture groups captured.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TextHit {
    pub start_line: u32,
    pub start: u32,
    pub end_line: u32,
    pub end: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<Option<String>>,
}

#[derive(Deserialize, Serialize)]
pub struct TextMatchesByFile {
    pub file: Ustr,
    pub path_kind: Ustr,
    pub matches: Vec<TextMatchInFile>,
}

/// Livegrep/codesearch text search results clustered by file.
#[derive(Deserialize, Serialize)]
pub struct TextMatches {
    pub by_file: Vec<TextMatchesByFile>,
    /// Whether livegrep stopped at its limit on matches, or its timeout, so
    /// there may have been more matches.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub limit_hit: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
    /// The search's pattern (set by `search-text`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<TextPattern>,
}

/// A fulltext search's pattern: livegrep only gives each line's first match,
/// so this finds the others (to tell whether semantic results cover them; see
/// `cmd_compile_results`) and the matches in excerpts (see
/// `cmd_pipeline::highlight`).
#[derive(Clone, Deserialize, Serialize)]
pub struct TextPattern {
    /// An RE2 regexp, as livegrep takes, which the `regex` crate mostly agrees
    /// with (and does for escaped text), or one of the fuller engine's (see
    /// `full_grep`).
    pub re: String,
    pub case_sensitive: bool,
    /// Whether its matches can span lines (see `full_grep`), so that a line
    /// alone can't be matched against it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spans_lines: bool,
    /// The names of its capture groups (None for unnamed ones), whose texts
    /// the results' `captures` have, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capture_names: Vec<Option<String>>,
    /// Which of its capture groups to tally the values of (see
    /// `cmd_compile_results::Tally`), if asked to (`tally:`): "all", a group's
    /// number, or its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tally: Option<String>,
}

impl TextPattern {
    /// The pattern as a `Regex`, if the `regex` crate takes it and lines can
    /// be matched against it alone.
    pub fn regex(&self) -> Option<regex::Regex> {
        if self.spans_lines {
            return None;
        }
        regex::RegexBuilder::new(&self.re)
            .case_insensitive(!self.case_sensitive)
            .build()
            .ok()
    }
}

#[derive(Deserialize, Serialize)]
pub struct FileMatch {
    pub path: Ustr,
    pub concise: ConcisePerFileInfo<Ustr>,
}

impl FileMatch {
    pub fn get_containing_dir(&self) -> Ustr {
        match self.path.rfind('/') {
            // We don't want to include the trailing "/"
            Some(offset) => ustr(&self.path[0..offset]),
            None => ustr(""),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct FileMatches {
    pub file_matches: Vec<FileMatch>,
    /// Whether there were more matches than the limit (see `search-files`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub limit_hit: bool,
}

pub enum SearchfoxIndexRoot {
    /// Already gzipped analysis files.  Note that `fetch_raw_analysis` exists
    /// and should be used in preference to this for reading file contents.
    CompressedAnalysis,
    /// The root of the config repo.
    ConfigRepo,
    /// The templates dir under the index root, home of the search template and
    /// the rendered help.html, and also the input of the help.html.
    /// This differs from IndexPages because the pages directory is intended to
    /// be exposed directly to the web, whereas if we expose the templates
    /// pages, we do so individually via symlinks.
    IndexTemplates,
    /// The "pages" dir under the index root.  This directory is intended to be
    /// exposed to the web in its entirety, which differs from IndexTemplates,
    /// which is not.  This is the home of the settings page.
    IndexPages,
    /// Directory listings.
    UncompressedDirectoryListing,
}

pub struct TreeInfo {
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct CommitInfo {
    pub rev: String,
    /// Its header's HTML (see `blame::commit_header`).
    pub header: String,
    /// Its message's first line.
    pub summary: String,
    pub date: DateTime<FixedOffset>,
}

/// Unified exposure for interacting with a local Searchfox index on disk or
/// a remote searchfox server over HTTPS talking to the web-server.
///
/// The primary goal is for our tests to verify both our on-disk representations
/// and that these are exposed to searchfox users correctly.  It's also our hope
/// that this can be used by searchfox contributors to investigate problems and
/// how things currently work more efficiently and enjoyably than manually doing
/// so.
///
/// ## Runtime Assumptions
///
/// We assume that we are operating in a tokio multi-threaded runtime and that
/// all blocking operations for any implementations of these traits should
/// responsibly make use of
/// https://docs.rs/tokio/1.5.0/tokio/task/index.html#blocking-and-yielding so
/// that full parallelism can be maintained.  In particular, this means that
/// mmap-based lookups which can fault and block on IO should likely use
/// https://docs.rs/tokio/1.5.0/tokio/task/index.html#block_in_place.
///
/// ## Abstraction Level / Library Use
///
/// Currently existing analysis-file processing and other logic:
/// - Uses synchronous I/O
///
/// In the end, it likely would make sense for the analysis mechanism to:
/// - Support async I/O
/// - Use async streams via https://docs.rs/tokio-stream/0.1.5/tokio_stream/
///   on a per-record or per-line granularity, quite possibly using our analysis
///   types for analysis records instead of untyped JSON.
///
/// But I'm introducing this interface right now in an attempt to provide
/// increased test coverage before making more extensive refactorings.  So for
/// now, this interface will do the simplest thing possible.
///
#[async_trait]
pub trait AbstractServer {
    /// Clone this server so that the command can kick off some parallelism.
    /// Currently the idea is that the server implementations are fairly cheap
    /// objects around either something that either:
    /// - Can safely be used from multiple threads through use of an Arc wrapper
    ///   around the data in question so we don't need to duplicate it.
    /// - Is something like libgit2 where everything we do is actually mutation
    ///   and there's no sane way to share it across multiple threads and so
    ///   we either start from a fresh state for each new clone or we have it
    ///   operate through some kind of request pool or something.
    ///
    /// So far we've only had to deal with immutable data so it hasn't mattered.
    fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync>;

    /// Return info about this tree, primarily for templating purposes.
    fn tree_info(&self) -> Result<TreeInfo>;

    /// Return info about the latest commit, primarily for templating purposes.
    fn commit_info(&self) -> Result<Option<CommitInfo>>;

    /// The URL of a line of the file at `path` in its rendered view, with
    /// "{}" for the line, if it's a spec tree's file with one (see
    /// `format::rendered_line_template`), which search results' lines link to.
    fn rendered_line_template(&self, _path: &str) -> Option<String> {
        None
    }

    /// The rendered blocks with `lines` of the file at `path`, those the
    /// index's excerpts have (see `format::spec_excerpts`), for search
    /// results' lines in specs' sources.
    fn spec_excerpts(&self, _path: &str, lines: &[u32]) -> Vec<Option<(String, u32, String)>> {
        vec![None; lines.len()]
    }

    /// The path kind and subsystem (ex: "Firefox/Sidebar") of the file at
    /// `path` in the indexed revision, if we know it, for faceting results
    /// (see `cmd_pipeline::facets::file_facets`).
    fn file_facet_info(&self, _path: &str) -> Option<(Ustr, Option<Ustr>)> {
        None
    }

    /// The tree's path kinds' keys and names (see per-file-info.toml), in
    /// their display order.
    fn path_kinds(&self) -> Vec<(Ustr, Ustr)> {
        vec![]
    }

    /// The subsystems whose nested values the subsystem facet shows at first
    /// (see per-file-info.toml's `FacetsConfig`).
    fn expanded_subsystems(&self) -> Vec<String> {
        vec![]
    }

    /// Convert a searchfox tree-local path into an absolute path on disk using
    /// the requested root.  This fundamentally only works for local indices.
    /// Note that many paths also have uncompressed (pre compress-outputs.sh)
    /// and compressed variants (post compress-outputs.sh).  In general the
    /// uncompressed variants only make sense for parts of the indexing process
    /// using searchfox-tool.  All checks will happen after compression has
    /// happened.
    ///
    /// TODO: Consider changing uses of this to instead operate in terms of
    /// read or write requests.  Currently this is only used for single-threaded
    /// synchronous stuff, but it could make sense to overhaul this if:
    /// - We're trying to move to parallelism or where the written files could
    ///   result in read data dependencies where they'd want to know when the
    ///   write happens.
    /// - We think this could make some kinds of testing easier or faster.
    fn translate_path(&self, root: SearchfoxIndexRoot, sf_path: &str) -> Result<String>;

    async fn coverage_history(&self, sf_path: &str) -> Result<Option<Vec<RevisionCoverage>>>;

    async fn coverage_summary(
        &self,
        sf_path: &str,
    ) -> Result<Option<code_coverage_report::NodeMetadata>>;

    /// Fetch the contents of the analysis file for the given searchfox
    /// tree-local path, decompressing if it's compressed.
    async fn fetch_raw_analysis<'a>(&self, sf_path: &str) -> Result<BoxStream<'a, Value>>;

    /// The path of the gzipped analysis file of the tree-local path
    /// `sf_path` whose records are as `fetch_raw_analysis` gives them (see
    /// the normalize-analyses tool), if the server has one, which the data
    /// API can serve as it is, rather than parsing, processing and
    /// serializing them.
    fn normalized_analysis_file(&self, _sf_path: &str) -> Option<String> {
        None
    }

    /// The length of the analysis records of `sf_path` (decoded), if the
    /// server has them locally, for the data API's estimate of the memory
    /// serving them takes (see `memory_budget`).
    fn analysis_file_len(&self, _sf_path: &str) -> Option<u64> {
        None
    }

    /// Fetch the contents of a raw (not HTML rendered) source file
    /// corresponding to the indexed revision like you would get out of revision
    /// control.
    ///
    /// TODO: In the future this should probably take a revision descriptor so
    /// we can actually check the source file out if needed.
    async fn fetch_raw_source(&self, sf_path: &str) -> Result<String>;

    /// Fetch the lines in the rendered HTML file.
    ///
    /// Returns a tuple of a list of lines, 0-th item for line 1,
    /// and a JSON string for the SYM_INFO object.
    async fn fetch_formatted_lines(&self, sf_path: &str) -> Result<(Vec<String>, String)>;

    /// Fetch the contents of a rendered HTML file, decompressing if it's
    /// compressed.  If `is_file` is true, this will be from the INDEX/file
    /// sub-tree.  If `is_file` is false, we're treating it as a directory
    /// request and we'll fetch the relevant `index.html.gz` file from the
    /// INDEX/dir sub-tree.
    async fn fetch_html(&self, root: HtmlFileRoot, sf_path: &str) -> Result<String>;

    /// Fetch the rows (see `chunked_gzip::ROW_START`) of lines (1-based) of
    /// rendered HTML files, with the symbols of their innermost nesting
    /// containers, by path and line, for excerpts of results (see
    /// `cmd_augment_results`).  By default this fetches the whole files, but
    /// local indexes only read the parts of the files with the lines, several
    /// files at a time (see `chunked_gzip`).
    async fn fetch_html_lines(
        &self,
        requests: Vec<(Ustr, BTreeSet<u32>)>,
    ) -> Result<UstrMap<HashMap<u32, Row>>> {
        let mut rows = UstrMap::default();
        for (path, lines) in requests {
            let html = self.fetch_html(HtmlFileRoot::FormattedFile, &path).await?;
            rows.insert(path, extract_rows(&html, &lines));
        }
        Ok(rows)
    }

    /// Retrieve the JSON contents of the crossref database for the given
    /// symbol.
    async fn crossref_lookup(&self, symbol: &str) -> Result<Option<CrossrefData>>;

    /// Retrieve a linked tree's crossref data for a symbol of that tree's
    /// (see `TreeConfigPaths::linked_trees`), ex: a Gecko function in
    /// firefox-disco implementing an algorithm of wubspecs', for wubspecs,
    /// with its path results and callees tagged with the tree (see
    /// `CrossrefData::tag_tree`).  (Trees without linked trees have none.)
    async fn linked_crossref_lookup(
        &self,
        _tree: &str,
        _symbol: &str,
    ) -> Result<Option<CrossrefData>> {
        Ok(None)
    }

    /// Retrieve the JSON contents of the jumpref database for the given
    /// symbol.
    async fn jumpref_lookup(&self, symbol: &str) -> Result<Option<JumprefData>>;

    /// Look symbols' crossref entries up ahead of the calls of
    /// `crossref_lookup` that are about to (one at a time), where that's
    /// worth it, so that those find them at once: a remote server's round
    /// trips (see `RemoteServer`'s memo).  Nothing for local indexes, whose
    /// lookups are microseconds.
    async fn prefetch_crossref(&self, _syms: &[Ustr]) {}

    /// `prefetch_crossref`, for `jumpref_lookup`.
    async fn prefetch_jumpref(&self, _syms: &[Ustr]) {}

    /// `prefetch_crossref`, for `fetch_raw_source` and `fetch_raw_analysis`
    /// of files.
    async fn prefetch_files(&self, _paths: &[String]) {}

    /// A file's history digests, if the tree has a history (see
    /// `file_format::recency::FileRecency`).
    async fn file_recency_lookup(&self, _path: &str) -> Result<Option<FileRecency>> {
        Ok(None)
    }

    /// Search the list of all files using a (potentially empty) regexp string
    /// and optionally enforcing a limit.  The underlying list of files should
    /// be equivalent to the union of the `repo-files` and `objdir-files`
    /// listings.
    ///
    /// This call's structure was chosen for consistency with the other search
    /// calls but it potentially could be reasonable to instead just have a
    /// primitive that allows the caller to request the file listings from the
    /// index dir root.  Symmetry broke in favor of this seeming like a more
    /// useful API that is decoupled from file formats.  Also, because it
    /// probably makes sense that file metadata lookup might want to be random
    /// access like `crossref_lookup` or that we might even just use
    /// `crossref_lookup` with the file path mangled into a searchfox internal
    /// symbol (like we've already started to do for C++ includes).
    ///
    /// It could also make sense for this API to eventually be more powerful and
    /// to support non-path constraints like tests being enabled/disabled/etc.
    /// but that would benefit from performing an analysis of filters we could
    /// feasibly provide and that people agree would be useful.
    ///
    /// Note that this will initially be local-only and whether it makes sense
    /// as a remote API really hinges on a rationale for not just remoting
    /// the new "query" mechanism.
    ///
    /// Paths matching `not_pathre` (if it's not empty) are left out, before
    /// the limit (ex: the query language's `-path:`).
    async fn search_files(
        &self,
        pathre: &str,
        not_pathre: &str,
        include_dirs: bool,
        limit: usize,
    ) -> Result<FileMatches>;

    /// Given an identifier (prefix), return pairs of the symbols that
    /// correspond to matching identifiers and those identifiers: (symbol,
    /// identifier).
    ///
    /// If `exact_match` is true, then this is just a (potentially case-insensitive)
    /// lookup.  If it's false, then this is a prefix search that skips anything
    /// that looks like hierarchy traversal.  That is, if we are searching for
    /// a needle of "Foo", this will match "Food" and "Fool" but not
    /// "Food::Pizza" or "Food.Pizza" because `:` and `.` are considered
    /// indications of hierarchy traversal.
    async fn search_identifiers(
        &self,
        needle: &str,
        exact_match: bool,
        ignore_case: bool,
        match_limit: usize,
    ) -> Result<Vec<(Ustr, Ustr)>>;

    /// Given an re2 search pattern and additional config info, run a
    /// livegrep codesearch against an already-running codesearch server.  In
    /// the future while our rust code may be responsible for starting the
    /// codesearch server and keeping it running, for now that responsibility
    /// continues to fall to the `router.py` webserver using `codesearch.py`.
    /// Files whose paths match `not_path` (if it's not empty) are left out,
    /// by livegrep, before its limit (ex: the query language's `-path:`).
    async fn search_text(
        &self,
        pattern: &str,
        fold_case: bool,
        path: &str,
        not_path: &str,
        limit: usize,
    ) -> Result<TextMatches>;

    async fn perform_query(&self, q: &str) -> Result<Value>;

    /// The tree as a try push changes it (see `try_delta`), for the head of
    /// its stack rebased onto the indexed revision, if the tree has its
    /// semantic delta: for queries in the push's context (see
    /// pipeline-server's `rev`).
    fn try_overlay(&self, _head: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        None
    }

    /// The tree's configuration: its settings in the config file (see
    /// `TreeConfigPaths`), without its `private` ones, for introspection (ex:
    /// a local server over a remote tree; see `RemoteServer`).
    fn tree_config(&self) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The status of the import of the try push `push` (its revision; see
    /// `try_import::normalize_push`) into the tree (see `try_import`).  (Only
    /// the server performing the tree's imports knows of those waiting and
    /// running: the web-server, whose `try-status` data API route this is.)
    async fn try_import_status(&self, _push: &str) -> Result<crate::try_import::ImportStatus> {
        Err(ServerError::Unsupported)
    }

    /// Ask for the try push `push` to be imported into the tree (see
    /// `try_import::TryImports::request`), with its reasons not to (ex: too
    /// many waiting) as `BadInput` errors.
    async fn request_try_import(&self, _push: &str) -> Result<()> {
        Err(ServerError::Unsupported)
    }

    /// The try push `push`, if the tree imported it (see
    /// `try_import::TryPush`).
    async fn try_push(&self, _push: &str) -> Result<Option<crate::try_import::TryPush>> {
        Err(ServerError::Unsupported)
    }

    /// The summaries of the open pull requests of the tree's spec
    /// repositories (see `spec_prs`), all of them or `spec`'s.
    async fn spec_prs(&self, _spec: Option<&str>) -> Result<Vec<crate::spec_prs::RepoSummary>> {
        Err(ServerError::Unsupported)
    }

    /// The status of the syncs of the summaries of the tree's spec
    /// repositories' pull requests (or of `spec`'s; see `spec_prs`).  (Only
    /// the server performing them knows of them: the web-server, whose
    /// `spec-prs-sync-status` data API route this is.)
    async fn spec_prs_sync_status(
        &self,
        _spec: Option<&str>,
    ) -> Result<crate::spec_prs::SyncStatus> {
        Err(ServerError::Unsupported)
    }

    /// Ask for a sync of the summaries of the tree's spec repositories' pull
    /// requests (or of `spec`'s; see `spec_prs::SpecPrSyncs::request`), with
    /// its reasons not to as `BadInput` errors.
    async fn request_spec_prs_sync(&self, _spec: Option<&str>) -> Result<()> {
        Err(ServerError::Unsupported)
    }

    /// The imports of the open pull requests of the tree's spec repository
    /// `spec` (see `spec_prs::pr_imports`).  (As `try_import_status`, the
    /// web-server's, whose `spec-prs-imports` data API route this is.)
    async fn spec_pr_imports(&self, _spec: &str) -> Result<Vec<crate::spec_prs::PrImport>> {
        Err(ServerError::Unsupported)
    }

    /// Corrections of the typos of a query which found nothing (see
    /// `typos::correct_query`), best first.
    async fn typo_corrections(&self, _query: &str) -> Result<Vec<String>> {
        Err(ServerError::Unsupported)
    }

    /// The spec pull requests imported into the tree (see
    /// `try_import::imported_pull_requests`).  (The web-server's, whose
    /// `spec-prs-imported` data API route this is, which the indexer asks for
    /// them, to import them again into the next index; see
    /// `spec_prs::start`.)
    async fn spec_prs_imported(&self) -> Result<Vec<String>> {
        Err(ServerError::Unsupported)
    }

    /// Ask for the imports of the open pull requests of the tree's spec
    /// repository `spec` which need them, or of those of `numbers` which do
    /// (see `spec_prs::request_pr_imports`), with its reasons not to as
    /// `BadInput` errors; how many it asked for.
    async fn request_spec_pr_imports(
        &self,
        _spec: &str,
        _numbers: Option<&[u32]>,
    ) -> Result<usize> {
        Err(ServerError::Unsupported)
    }

    /// The data of the tree's landing page (see `format::landing_data`), for
    /// rendering it at index time (see `cmd_render`).
    async fn landing(&self) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The data of an explore page of the commits `revs`, oldest first (see
    /// `format::explore_data`), whose files link to their views with the
    /// stack strip of the page at `stack_url` (ex: `try/hg/<push>`), if any.
    async fn explore(&self, _revs: &[String], _stack_url: Option<&str>) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The data of the listing of the directory at `path` in the revision
    /// `rev` (see `format::rev_dir_data`), if it's a directory there.
    async fn rev_dir(&self, _rev: &str, _path: &str) -> Result<Option<Value>> {
        Err(ServerError::Unsupported)
    }

    /// The data of the page of the file at `path` in the revision `rev` (see
    /// `format::rev_file_data`), if it's a file there.
    async fn rev_file(&self, _rev: &str, _path: &str) -> Result<Option<crate::format::RevFile>> {
        Err(ServerError::Unsupported)
    }

    /// A token-centric blame data file of the file at `path` in the revision
    /// `rev` (see `format::hyperblame_json`).
    async fn hyperblame_json(&self, _rev: &str, _path: &str, _file: &str) -> Result<String> {
        Err(ServerError::Unsupported)
    }

    /// The stack strip of the file at `path` in the revision `rev` for the
    /// stack page `stack` (ex: `try/hg/<push>`; see `format::stack_strip_json`).
    async fn stack_strip_json(&self, _stack: &str, _rev: &str, _path: &str) -> Result<String> {
        Err(ServerError::Unsupported)
    }

    /// The data of the history page of `scope` (or the context of `token`, or
    /// the whole file) of the file at `path` as of `rev` (see
    /// `format::history_data`).
    async fn history(
        &self,
        _rev: &str,
        _path: &str,
        _scope: Option<&str>,
        _token: Option<usize>,
    ) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The data of the diff of the file at `path` in the revision `rev`, if
    /// it changes it (see `format::rev_diff_data`).
    async fn rev_diff(&self, _rev: &str, _path: &str) -> Result<Option<crate::format::RevDiff>> {
        Err(ServerError::Unsupported)
    }

    /// The data of the page of the commit `rev` (see
    /// `format::commit_page_data`).
    async fn commit_page(&self, _rev: &str) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// What the blame popups show about the commits `revs` (comma-separated;
    /// see `blame::get_commit_info`), as JSON.
    async fn commit_info_json(&self, _revs: &str) -> Result<String> {
        Err(ServerError::Unsupported)
    }

    /// The data of the interdiff of the files the sides `a` and `b`
    /// (comma-separated revisions) changed (see
    /// `format::interdiff_files_data`).
    async fn interdiff_files(&self, _a: &str, _b: &str) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The data of the interdiff of the file at `path` between the sides `a`
    /// and `b` (see `format::interdiff_file_data`).
    async fn interdiff_file(
        &self,
        _a: &str,
        _b: &str,
        _path: &str,
    ) -> Result<crate::format::InterdiffFile> {
        Err(ServerError::Unsupported)
    }

    /// The outline diff of the file at `path` between `from` (a revision, or
    /// `since:YYYY-MM-DD`) and `to`, with all of the "from" file's
    /// definitions' lines if `all_from_lines` (see
    /// `format::outline_diff_data`).
    async fn outline_diff(
        &self,
        _from: &str,
        _to: &str,
        _path: &str,
        _all_from_lines: bool,
    ) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The data of the explore page of the bugs (`kind` "bug") or Phabricator
    /// revisions ("phab") `keys` (comma-separated; see
    /// `format::explore_page_data`).
    async fn explore_keys(&self, _kind: &str, _keys: &str) -> Result<Value> {
        Err(ServerError::Unsupported)
    }

    /// The crash report `id` (crash-stats' processed crash; see
    /// `crash_stack::crash_report_stack`), from crash-stats, unless the
    /// config says where (`Config::crash_reports_url`).
    async fn crash_report(&self, id: &str) -> Result<std::sync::Arc<Value>> {
        crate::crash_stack::fetch_crash_report(crate::crash_stack::CRASH_REPORTS_URL, id).await
    }

    /// Bugzilla's REST API's response for `path` (ex:
    /// "rest/bug/1915788/comment"; see `bug_comment::fetch_bugzilla`), from
    /// bugzilla.mozilla.org, unless the config says where
    /// (`Config::bugzilla_url`).
    async fn bugzilla_json(&self, path: &str) -> Result<std::sync::Arc<Value>> {
        crate::bug_comment::fetch_bugzilla(crate::bug_comment::BUGZILLA_URL, path).await
    }

    /// Bugzilla's page `path` (ex: "show_bug.cgi?id=1915788"; see
    /// `bug_comment::fetch_bugzilla_page`), as `bugzilla_json`'s.
    async fn bugzilla_page(&self, path: &str) -> Result<String> {
        crate::bug_comment::fetch_bugzilla_page(crate::bug_comment::BUGZILLA_URL, path).await
    }

    /// The head the tree's branch had at `time` (see
    /// `commit_index::parse_time`), roughly: its main line's last commit which
    /// had landed by then (see `commit_index::CommitIndex::rev_at`), if it
    /// had one.  (Unsupported by remote servers.)
    async fn rev_at(&self, _time: &str) -> Result<Option<crate::commit_index::RevAt>> {
        Err(ServerError::Unsupported)
    }

    /// For each line of the file at `path` in revision `rev` (line `l` at `l -
    /// 1`), its line in the indexed revision's version, if it's unchanged
    /// since (see `rev_lines::line_map`), or None if the tree's repository
    /// doesn't have the revision or the file.  (Unsupported by remote
    /// servers.)
    async fn line_map(&self, _rev: &str, _path: &str) -> Result<Option<Vec<Option<u32>>>> {
        Err(ServerError::Unsupported)
    }

    /// The tree's git revision for the revision `rev` of another kind: an hg
    /// revision (`kind` "hg"; ex: firefox's, by git-cinnabar's mapping) or a
    /// revision of the tree's old git repository ("old"; see
    /// `TreeConfigPaths::oldgit_path`), if it has one.
    async fn map_rev(&self, _kind: &str, _rev: &str) -> Result<Option<String>> {
        Err(ServerError::Unsupported)
    }

    /// The try push this is the tree as changed by, if it's a `try_overlay`.
    fn try_delta_meta(&self) -> Option<crate::try_delta::TryDeltaMeta> {
        None
    }

    /// The server with the analysis of revision `rev`'s files, if there's one
    /// (ex: a try push's base, the indexed revision), for a patch's old side
    /// (see `dataflow::slice::ChangedLines::removed_lines`).
    fn old_side(&self, _rev: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        None
    }

    /// What this server's steps' results depend on, for caching them (see
    /// `step_cache`): its index's identity, and a try push's delta's.  None
    /// if they shouldn't be cached (ex: a remote server's).
    fn cache_identity(&self) -> Option<String> {
        None
    }

    /// The lines a revision changes (a commit's changes from its first
    /// parent, or `<a>..<b>`'s), in the coordinates of the files this
    /// analyzes (the indexed revision's, or a try push's head's), for
    /// slicing them (see `rev_lines` and `dataflow::slice`).
    fn rev_changed_lines(&self, _spec: &str) -> Result<crate::dataflow::slice::ChangedLines> {
        Err(ServerError::Unsupported)
    }

    /// Fetch the records of a token-centric history timeline journal (see
    /// `hyperblame::journals`) as JSON values, newest to oldest.  `target` is a
    /// tree-relative path for future and files-delta journals, and a token for
    /// token journals.  The journal is as of the source revision `source_rev`
    /// if provided (which must be a full revision), or else the history's
    /// head.  If `expand` is true, summary records are replaced by the detail
    /// records they summarize.
    ///
    /// This is currently local-only.
    async fn fetch_history_journal(
        &self,
        kind: JournalKind,
        target: &str,
        source_rev: Option<&str>,
        expand: bool,
    ) -> Result<Vec<Value>>;

    /// What the history says about `token`'s removals: the commits which
    /// removed most of its instances, and what replaced it (see
    /// `hyperblame::token_history`), or None if the history has no journal
    /// for it.  Unsupported if the tree has no token-centric history.
    async fn token_history(
        &self,
        _token: &str,
        _opts: TokenHistoryOptions,
    ) -> Result<Option<TokenHistory>> {
        Err(ServerError::Unsupported)
    }

    /// The timelines of `tokens`: a line a token (with its case variants) of
    /// its instances by week, and markers at the weeks changing them the most,
    /// with their commits (see `hyperblame::token_timeline`).  Unsupported if
    /// the tree has no token-centric history.
    async fn token_timeline(
        &self,
        _tokens: &[String],
        _opts: TokenTimelineOptions,
    ) -> Result<TokenTimeline> {
        Err(ServerError::Unsupported)
    }

    /// A page of the revisions where a pattern's matches in a file changed
    /// (see `hyperblame::history_grep`), within `limits` (of `tier`).
    /// Unsupported if the tree has no token-centric history.
    async fn history_grep(
        &self,
        _args: &crate::hyperblame::history_grep::HistoryGrepArgs,
        _limits: &crate::query_limits::QueryLimits,
        _tier: crate::query_limits::Tier,
    ) -> Result<crate::hyperblame::history_grep::HistoryGrep> {
        Err(ServerError::Unsupported)
    }

    /// The tokens of the history's journals which start with, end with, or
    /// contain `query`, case- and accent-insensitively (see
    /// `file_format::token_names`), up to `limit`.  Unsupported without a
    /// token-name index (ex: the tree has no token-centric history).
    async fn token_names(
        &self,
        _query: &str,
        _how: NameMatch,
        _limit: usize,
    ) -> Result<TokenNameMatches> {
        Err(ServerError::Unsupported)
    }
}
