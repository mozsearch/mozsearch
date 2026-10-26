//! The data API: the abstract server over HTTP (`/{tree}/api/v1/...`), which
//! the pipeline-server serves (see `router`), for `RemoteServer` (tools and
//! servers working over a remote tree) and other clients: an
//! `AbstractServer` primitive per route, as JSON as its Rust types serialize
//! (so that clients deserialize the same types), of the tree as indexed or,
//! with `rev=`, as a try push changes it (see `AbstractServer::try_overlay`).
//!
//! Routes (GET, with parameters in the query string):
//! - `tree-info`: the tree's name, the commit of its files (with `rev=`, the
//!   try push's head), its path kinds, and with `rev=`, the try push's
//!   `TryDeltaMeta`.
//! - `config`: the tree's settings in the config file (see
//!   `AbstractServer::tree_config`), without its `private` ones.
//! - `crossref?sym=[&fields=]`: the symbol's crossref entry (`null` if it has
//!   none), only its comma-separated `fields` if given (ex: `meta,callees`).
//! - `jumpref?sym=`: the symbol's jumpref (`null` if it has none).
//! - `identifiers?q=[&exact=true][&ignore_case=true][&limit=]`: identifiers
//!   starting with `q` (or equal to it, if `exact`), as `[symbol,
//!   identifier]`s.
//! - `files?re=[&not_re=][&dirs=true][&limit=]`: the files (and directories, if `dirs`)
//!   whose paths match the regular expression.
//! - `source/{path}`: a file's text.
//! - `analysis/{path}`: a file's analysis records, one JSON record a line, as
//!   the tools read them (see `fetch_raw_analysis`).
//! - `changed-lines?spec=`: the lines a revision (`<rev>`, from its first
//!   parent, or `<rev>..<rev>`) changes, in the coordinates of the tree's
//!   files (see `rev_changed_lines`).
//! - `formatted-lines/{path}`: a file's source listing's lines' HTML, and
//!   their SYM_INFO (see `fetch_formatted_lines`).
//! - `html-lines/{path}?lines=`: rows of a file's source listing (as
//!   `chunked_gzip::Row`s, by line), for lines like `1-40,52` (see
//!   `fetch_html_lines`).
//! - `file-info/{path}`: a file's (or directory's) path kind and subsystem
//!   (`null` if the tree doesn't have it).
//! - `file-recency/{path}`: a file's history digests (`null` without them).
//! - `journal?kind=&target=[&source_rev=][&expand=true]`: a history timeline
//!   journal's records (`kind` is `future`, `files-delta` or `tokens`; see
//!   `fetch_history_journal`).
//! - `token-history?token=[&case_sensitive=true][&view=true]`: what the
//!   history says about a token's removals (`null` if it has no journal for
//!   it; see `token_history`).
//! - `token-timeline?tokens=a,b[&case_sensitive=true]`: tokens' timelines, by
//!   week, with markers at the weeks changing them the most (see
//!   `token_timeline`).
//! - `token-names?q=&match=prefix|suffix|contains[&limit=]`: the tokens of the
//!   history's journals matching `q` (see `token_names`).
//! - `linked-crossref?tree=&sym=`: a linked tree's crossref entry for a
//!   symbol of the tree's (see `linked_crossref_lookup`).
//! - `text?re=[&fold_case=true][&path=][&not_path=][&limit=]`: the lines
//!   matching a regular expression (livegrep's; see `search_text`), in files
//!   whose paths match `path`, but `not_path`.
//!
//! - `try-push?push=`: an imported try push's stack, and its semantic head or
//!   why it has none (see `try_import::TryPush`; `null` if it isn't
//!   imported).
//! - `explore?revs=a,b[&stack_url=]`: the data of the explore page of commits
//!   (see `format::explore_data`).
//! - `rev-dir?commit=[&path=]`: the data of a directory's listing at a
//!   revision (`null` if it isn't a directory there; see
//!   `format::rev_dir_data`).
//! - `rev-file?commit=&path=`: the data of a file's page at a revision (its
//!   text, its analysis if it's a try push's head's, its blame and coverage;
//!   `null` if it isn't a file there; see `format::rev_file_data`).
//! - `hyperblame?commit=&path=&file=`: a file's token-centric blame data file
//!   at a revision (see `format::hyperblame_json`).
//! - `history?commit=&path=[&scope=][&token=]`: the data of a history page
//!   (see `format::history_data`).
//! - `rev-diff?commit=&path=`: the data of a file's diff in a revision (`null`
//!   if it doesn't change it; see `format::rev_diff_data`).
//! - `commit?commit=`: the data of a commit's page (see
//!   `format::commit_page_data`).
//! - `commit-info?revs=a,b`: what the blame popups show about commits (see
//!   `blame::get_commit_info`).
//! - `interdiff?a=&b=` and `interdiff-file?a=&b=&path=`: the data of the
//!   interdiff of the files two sides (comma-separated revisions) changed,
//!   and of a file's (see `format::interdiff_files_data` and
//!   `interdiff_file_data`).
//! - `outline-diff?from=&to=&path=[&all=true]`: a file's outline diff (see
//!   `format::outline_diff_data`).
//! - `explore-keys?kind=bug|phab&keys=`: the data of the explore page of bugs
//!   or Phabricator revisions (see `format::explore_page_data`).
//! - `map-rev?kind=hg|old&from=`: the tree's revision for an hg revision or
//!   an old git repository's (`null` if it has none).
//! - `stack-strip?stack=&commit=&path=`: a file's stack strip for a stack
//!   page (see `format::stack_strip_json`).
//! - `try-status?push=` and POST `try-import?push=`: a try push's import's
//!   status, and asking for its import; served by the web-server, which
//!   performs imports (see `Host`).
//! - `spec-prs[?spec=]`: the summaries of the open pull requests of the
//!   tree's spec repositories, or of one (see `spec_prs`); and
//!   `spec-prs-sync-status[?spec=]` and POST `spec-prs-sync[?spec=]`, their
//!   syncs' status, and asking for one; served by the web-server, which
//!   performs the syncs (and whose summaries they change).
//!
//! All of them take `rev=` (a try push's head) and `max_bytes=`.
//!
//! Requests are checked strictly, since nginx caches responses by their URLs
//! (so unknown parameters would only make copies), and a client sending bad
//! ones should hear about it (and the WAF can count them):
//! - 422, with a JSON `error`: unknown, repeated or malformed parameters,
//!   malformed percent-encoding, and paths that aren't plain relative paths
//!   (`..`, `.` or empty segments, absolute paths, backslashes, control
//!   characters).  Only the tree's files are served (see `source_path_known`).
//! - 406, with the response's `bytes` and the `max_bytes` asked for: a
//!   response bigger than the caller's cap, 4 MiB unless `max_bytes` says
//!   otherwise, so that a naive client doesn't get a huge crossref entry (ex:
//!   `std::forward`'s) by surprise; it can raise it, or ask for fewer
//!   `fields`.
//! - 404: an unknown tree, or one whose API is off (see
//!   `TreeConfigPaths::data_api`), an unknown route or version, a missing
//!   file, or a try push without an analysis.
//!
//! Admission: the API's requests have their own buckets, and are charged for
//! their processing time (see `admission::Pool`).  Responses are for any
//! origin (`Access-Control-Allow-Origin: *`), since the trees are public.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::Method;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::abstract_server::{AbstractServer, ErrorLayer, ServerError};
use crate::file_format::token_names::NameMatch;
use crate::hyperblame::journals::JournalKind;
use crate::hyperblame::token_history::TokenHistoryOptions;
use crate::hyperblame::token_timeline::TokenTimelineOptions;
use crate::try_delta::is_full_rev;

/// The responses' size cap, unless the request's `max_bytes` says otherwise.
pub const DEFAULT_MAX_BYTES: usize = 4 * 1024 * 1024;
/// The most results `identifiers` and `files` return (as the pipeline's
/// `search-files` asks for at most).
const MAX_LIMIT: usize = 2_000_001;
const DEFAULT_LIMIT: usize = 1000;
const MAX_SYM_BYTES: usize = 8192;
const MAX_NEEDLE_BYTES: usize = 1024;
const MAX_PATH_BYTES: usize = 4096;

pub type Servers = Arc<BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>>;

#[derive(Clone)]
struct ApiState {
    servers: Servers,
    /// The trees whose API is on.
    enabled: Arc<BTreeSet<String>>,
}

/// The routes of the API of `servers`' trees in `enabled`.
pub fn router(servers: Servers, enabled: BTreeSet<String>) -> Router {
    Router::new()
        .route("/{tree}/api/{*rest}", any(handle))
        .with_state(ApiState {
            servers,
            enabled: Arc::new(enabled),
        })
}

async fn handle(
    State(state): State<ApiState>,
    method: Method,
    headers: header::HeaderMap,
    uri: Uri,
) -> Response {
    let response = match respond(
        &state,
        Host::PipelineServer,
        method == Method::POST,
        accepts_gzip(&headers),
        &uri,
    )
    .await
    {
        Ok(body) => body.into_response(),
        Err(error) => error.into_response(),
    };
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    parts.headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    Response::from_parts(parts, body)
}

/// A response's body.
#[derive(Debug)]
pub struct Body {
    content_type: &'static str,
    bytes: Vec<u8>,
    /// The bytes' `Content-Encoding`, if they're encoded (ex: a normalized
    /// analysis file's gzip; see `AbstractServer::normalized_analysis_file`),
    /// and their length decoded, which `max_bytes` is of.
    encoding: Option<&'static str>,
    decoded_len: usize,
}

impl Body {
    fn new(content_type: &'static str, bytes: Vec<u8>) -> Body {
        let decoded_len = bytes.len();
        Body {
            content_type,
            bytes,
            encoding: None,
            decoded_len,
        }
    }

    fn json(value: &Value) -> Body {
        Body::new(
            "application/json",
            serde_json::to_vec(value).unwrap_or_default(),
        )
    }
}

impl IntoResponse for Body {
    fn into_response(self) -> Response {
        match self.encoding {
            Some(encoding) => (
                [
                    (header::CONTENT_TYPE, self.content_type),
                    (header::CONTENT_ENCODING, encoding),
                    (header::VARY, "Accept-Encoding"),
                ],
                self.bytes,
            )
                .into_response(),
            None => ([(header::CONTENT_TYPE, self.content_type)], self.bytes).into_response(),
        }
    }
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    body: Value,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(self.body)).into_response()
    }
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        body: json!({ "error": message.into() }),
    }
}

fn not_found(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        body: json!({ "error": message.into() }),
    }
}

/// A server's error: bad input's as such, and the rest without their details
/// (which can have the server's paths), which are logged.
fn server_error(route: &str, error: ServerError) -> ApiError {
    match error {
        ServerError::StickyProblem(details) | ServerError::TransientProblem(details)
            if matches!(details.layer, ErrorLayer::BadInput) =>
        {
            bad(details.message)
        }
        ServerError::Unsupported => not_found(format!("No {} for this tree", route)),
        ServerError::TooExpensive(refused) => ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body: refused.to_json(),
        },
        error => {
            log::warn!("data API {}: {:?}", route, error);
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: json!({ "error": "Server error" }),
            }
        }
    }
}

/// A route: its name, whether it takes a path, and its parameters (besides
/// `rev` and `max_bytes`).
struct Route {
    name: &'static str,
    takes_path: bool,
    params: &'static [&'static str],
    /// Which server serves it (see `Host`).
    host: Host,
    /// Whether it's a POST (ex: asking for a try push's import), rather than
    /// a GET.
    post: bool,
}

/// Which server serves a route: the pipeline-server, the API's host, or for
/// try pushes' imports, the web-server, which performs them (and so knows
/// which are waiting and running), to which nginx routes them, uncached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    PipelineServer,
    WebServer,
}

const ROUTES: &[Route] = &[
    Route {
        name: "try-status",
        takes_path: false,
        params: &["push"],
        host: Host::WebServer,
        post: false,
    },
    Route {
        name: "try-import",
        takes_path: false,
        params: &["push"],
        host: Host::WebServer,
        post: true,
    },
    Route {
        name: "try-push",
        takes_path: false,
        params: &["push"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "spec-prs",
        takes_path: false,
        params: &["spec"],
        host: Host::WebServer,
        post: false,
    },
    Route {
        name: "spec-prs-sync-status",
        takes_path: false,
        params: &["spec"],
        host: Host::WebServer,
        post: false,
    },
    Route {
        name: "spec-prs-sync",
        takes_path: false,
        params: &["spec"],
        host: Host::WebServer,
        post: true,
    },
    Route {
        name: "explore",
        takes_path: false,
        params: &["revs", "stack_url"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "rev-dir",
        takes_path: false,
        params: &["commit", "path"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "rev-file",
        takes_path: false,
        params: &["commit", "path"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "hyperblame",
        takes_path: false,
        params: &["commit", "path", "file"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "history",
        takes_path: false,
        params: &["commit", "path", "scope", "token"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "rev-diff",
        takes_path: false,
        params: &["commit", "path"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "commit",
        takes_path: false,
        params: &["commit"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "commit-info",
        takes_path: false,
        params: &["revs"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "interdiff",
        takes_path: false,
        params: &["a", "b"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "explore-keys",
        takes_path: false,
        params: &["kind", "keys"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "map-rev",
        takes_path: false,
        params: &["kind", "from"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "interdiff-file",
        takes_path: false,
        params: &["a", "b", "path"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "outline-diff",
        takes_path: false,
        params: &["from", "to", "path", "all"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "stack-strip",
        takes_path: false,
        params: &["stack", "commit", "path"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "tree-info",
        takes_path: false,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "config",
        takes_path: false,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "crossref",
        takes_path: false,
        params: &["sym", "fields"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "jumpref",
        takes_path: false,
        params: &["sym"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "identifiers",
        takes_path: false,
        params: &["q", "exact", "ignore_case", "limit"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "files",
        takes_path: false,
        params: &["re", "not_re", "dirs", "limit"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "source",
        takes_path: true,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "analysis",
        takes_path: true,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "changed-lines",
        takes_path: false,
        params: &["spec"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "formatted-lines",
        takes_path: true,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "html-lines",
        takes_path: true,
        params: &["lines"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "file-info",
        takes_path: true,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "file-recency",
        takes_path: true,
        params: &[],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "journal",
        takes_path: false,
        params: &["kind", "target", "source_rev", "expand"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "token-history",
        takes_path: false,
        params: &["token", "case_sensitive", "view"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "token-timeline",
        takes_path: false,
        params: &["tokens", "case_sensitive"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "token-names",
        takes_path: false,
        params: &["q", "match", "limit"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "linked-crossref",
        takes_path: false,
        params: &["tree", "sym"],
        host: Host::PipelineServer,
        post: false,
    },
    Route {
        name: "text",
        takes_path: false,
        params: &["re", "fold_case", "path", "not_path", "limit"],
        host: Host::PipelineServer,
        post: false,
    },
];

const COMMON_PARAMS: &[&str] = &["rev", "max_bytes"];

/// Percent-decode `text` strictly: a `%` must start two hex digits, and the
/// result must be UTF-8 without control characters; `+` is a space in query
/// strings (`plus_is_space`).
fn decode(text: &str, plus_is_space: bool) -> Result<String, ApiError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(|| bad("Malformed percent-encoding"))?;
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let text = String::from_utf8(out).map_err(|_| bad("Not UTF-8"))?;
    if text.chars().any(char::is_control) {
        return Err(bad("Control characters"));
    }
    Ok(text)
}

/// A query string's parameters, all of them `name=value` (no empty ones),
/// decoded.
fn parse_query(query: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return Ok(vec![]);
    };
    query
        .split('&')
        .map(|param| {
            let (name, value) = param
                .split_once('=')
                .ok_or_else(|| bad(format!("Malformed parameter: {:?}", param)))?;
            Ok((decode(name, true)?, decode(value, true)?))
        })
        .collect()
}

/// Whether `path` is a plain relative path within the tree: no empty, `.` or
/// `..` segments (so not absolute, and not escaping it), no backslashes, and
/// no control characters.
pub fn valid_tree_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_BYTES
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// The most commits an explore page has (see `explore::MAX_COMMITS`).
const MAX_EXPLORE_REVS: usize = 200;

/// A try push's name (see `try_import::normalize_push`), from the `push`
/// parameter.
fn push_param(params: &Params) -> Result<String, ApiError> {
    crate::try_import::normalize_push(params.required("push")?).ok_or_else(|| {
        bad("push must be a try push's revision (12 to 40 hexadecimal digits), or a spec's pull request (SPEC/NUMBER)")
    })
}

/// A spec repository's directory in the forest, from the optional `spec`
/// parameter.
fn spec_param(params: &Params) -> Result<Option<String>, ApiError> {
    match params.get("spec") {
        None => Ok(None),
        Some(spec) if crate::spec_prs::valid_spec(spec) => Ok(Some(spec.to_string())),
        Some(_) => Err(bad(
            "spec must be a spec repository's directory in the forest (ex: dom)",
        )),
    }
}

/// Whether `rev` is a revision (7 to 40 lowercase hexadecimal digits).
fn is_hex_rev(rev: &str) -> bool {
    (7..=40).contains(&rev.len())
        && rev
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A revision, from the `commit` parameter.
fn commit_param(params: &Params) -> Result<String, ApiError> {
    commit_param_named(params, "commit")
}

/// A revision, from the parameter `name`.
fn commit_param_named(params: &Params, name: &str) -> Result<String, ApiError> {
    let commit = params.required(name)?;
    is_hex_rev(commit)
        .then(|| commit.to_string())
        .ok_or_else(|| {
            bad(format!(
                "{} must be a revision (7 to 40 hexadecimal digits)",
                name
            ))
        })
}

/// Comma-separated revisions (at most `MAX_EXPLORE_REVS`), from the parameter
/// `name`.
fn revs_param<'a>(params: &'a Params, name: &str) -> Result<&'a str, ApiError> {
    let revs = params.required(name)?;
    (revs.split(',').count() <= MAX_EXPLORE_REVS && revs.split(',').all(is_hex_rev))
        .then_some(revs)
        .ok_or_else(|| {
            bad(format!(
                "{} must be revisions (hexadecimal, comma-separated)",
                name
            ))
        })
}

/// The most lines `html-lines` gives at once.
const MAX_LINES: usize = 100_000;

/// The lines of a `lines` parameter, ex: `1-40,52`.
pub fn parse_lines(spec: &str) -> Option<BTreeSet<u32>> {
    let mut lines = BTreeSet::new();
    for part in spec.split(',') {
        let (first, last) = match part.split_once('-') {
            Some((first, last)) => (first.parse::<u32>().ok()?, last.parse::<u32>().ok()?),
            None => {
                let line = part.parse::<u32>().ok()?;
                (line, line)
            }
        };
        if first == 0 || last < first || (last - first) as usize >= MAX_LINES {
            return None;
        }
        lines.extend(first..=last);
        if lines.len() > MAX_LINES {
            return None;
        }
    }
    Some(lines)
}

/// The `lines` parameter of `lines`, as ranges (see `parse_lines`).
pub fn format_lines(lines: &BTreeSet<u32>) -> String {
    let mut ranges: Vec<(u32, u32)> = vec![];
    for &line in lines {
        match ranges.last_mut() {
            Some((_, last)) if *last + 1 == line => *last = line,
            _ => ranges.push((line, line)),
        }
    }
    ranges
        .iter()
        .map(|&(first, last)| match first == last {
            true => first.to_string(),
            false => format!("{}-{}", first, last),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// A request's parameters (known to be its route's, once each).
struct Params(BTreeMap<String, String>);

impl Params {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<&str, ApiError> {
        self.get(name)
            .ok_or_else(|| bad(format!("Missing parameter: {}", name)))
    }

    fn flag(&self, name: &str) -> Result<bool, ApiError> {
        match self.get(name) {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(_) => Err(bad(format!("{} must be true or false", name))),
        }
    }

    fn limit(&self) -> Result<usize, ApiError> {
        match self.get("limit") {
            None => Ok(DEFAULT_LIMIT),
            Some(limit) => limit
                .parse()
                .ok()
                .filter(|limit| (1..=MAX_LIMIT).contains(limit))
                .ok_or_else(|| bad(format!("limit must be from 1 to {}", MAX_LIMIT))),
        }
    }

    fn symbol(&self) -> Result<&str, ApiError> {
        let sym = self.required("sym")?;
        if sym.is_empty() || sym.len() > MAX_SYM_BYTES {
            return Err(bad(format!("sym must have 1 to {} bytes", MAX_SYM_BYTES)));
        }
        Ok(sym)
    }
}

/// The response to a request for `uri`.
/// Whether a request's `Accept-Encoding` takes gzip.
fn accepts_gzip(headers: &header::HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|coding| coding.split(';').next().is_some_and(|c| c.trim() == "gzip"))
}

/// The response to a request for `uri` (a POST if `post`), by the routes
/// served by `host`, gzipped as it is if it's stored so and the request
/// `accepts_gzip` (see `Body::encoding`).
pub async fn respond(
    state: &(impl ApiServers + ?Sized),
    host: Host,
    post: bool,
    accepts_gzip: bool,
    uri: &Uri,
) -> Result<Body, ApiError> {
    let path = uri.path().strip_prefix('/').unwrap_or(uri.path());
    let mut parts = path.splitn(4, '/');
    let tree = decode(parts.next().unwrap_or_default(), false)?;
    let Some(server) = state.server(&tree) else {
        return Err(not_found(format!("No data API for tree {:?}", tree)));
    };
    if parts.next() != Some("api") || parts.next() != Some("v1") {
        return Err(not_found("Unknown API version"));
    }
    let rest = parts.next().unwrap_or_default();
    let (name, path_arg) = match rest.split_once('/') {
        Some((name, path_arg)) => (name, Some(path_arg)),
        None => (rest, None),
    };
    let Some(route) = ROUTES
        .iter()
        .find(|route| route.name == name && route.takes_path == path_arg.is_some())
    else {
        return Err(not_found(format!("Unknown route {:?}", rest)));
    };
    if route.host != host {
        return Err(not_found(format!(
            "{} is served by the {:?}",
            route.name, route.host
        )));
    }
    if route.post != post {
        return Err(ApiError {
            status: StatusCode::METHOD_NOT_ALLOWED,
            body: json!({ "error": format!("{} is a {}", route.name, if route.post { "POST" } else { "GET" }) }),
        });
    }

    let mut params = BTreeMap::new();
    for (param, value) in parse_query(uri.query())? {
        if !route.params.contains(&param.as_str()) && !COMMON_PARAMS.contains(&param.as_str()) {
            return Err(bad(format!("Unknown parameter: {}", param)));
        }
        if params.insert(param.clone(), value).is_some() {
            return Err(bad(format!("Repeated parameter: {}", param)));
        }
    }
    let params = Params(params);
    let max_bytes = match params.get("max_bytes") {
        None => DEFAULT_MAX_BYTES,
        Some(max) => max
            .parse()
            .map_err(|_| bad("max_bytes must be a number of bytes"))?,
    };
    let file_path = match path_arg {
        Some(path_arg) => {
            let file_path = decode(path_arg, false)?;
            if !valid_tree_path(&file_path) {
                return Err(bad(format!("Not a path in the tree: {:?}", file_path)));
            }
            Some(file_path)
        }
        None => None,
    };

    let server: Box<dyn AbstractServer + Send + Sync> = match params.get("rev") {
        Some(rev) => {
            if !is_full_rev(rev) {
                return Err(bad("rev must be a full revision"));
            }
            server
                .try_overlay(rev)
                .ok_or_else(|| not_found(format!("No analysis of a try push for {}", rev)))?
        }
        None => server.clonify(),
    };

    let body = route_body(
        route.name,
        server,
        &params,
        file_path.as_deref(),
        accepts_gzip,
    )
    .await?;
    if body.decoded_len > max_bytes {
        return Err(ApiError {
            status: StatusCode::NOT_ACCEPTABLE,
            body: json!({
                "error": "The response is bigger than max_bytes",
                "bytes": body.decoded_len,
                "max_bytes": max_bytes,
            }),
        });
    }
    Ok(body)
}

/// The trees' servers, for `respond` (the pipeline-server's, and tests').
pub trait ApiServers {
    /// The server of `tree`, if its API is on.
    fn server(&self, tree: &str) -> Option<Box<dyn AbstractServer + Send + Sync>>;
}

impl ApiServers for ApiState {
    fn server(&self, tree: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        if !self.enabled.contains(tree) {
            return None;
        }
        self.servers.get(tree).map(|server| server.clonify())
    }
}

/// The status, content type and body of the response to a request for
/// `path_and_query` (see `respond`), for servers not using axum (the
/// web-server).
pub async fn respond_parts(
    state: &(impl ApiServers + ?Sized),
    host: Host,
    post: bool,
    path_and_query: &str,
) -> (u16, &'static str, Vec<u8>) {
    let result = match path_and_query.parse::<Uri>() {
        Ok(uri) => respond(state, host, post, false, &uri).await,
        Err(_) => Err(bad("Malformed URL")),
    };
    match result {
        Ok(body) => (200, body.content_type, body.bytes),
        Err(error) => (
            error.status.as_u16(),
            "application/json",
            serde_json::to_vec(&error.body).unwrap_or_default(),
        ),
    }
}

/// Whether `path` is one of the tree's files (or directories), as indexed,
/// or as the try push (if `server` is its overlay) changes it: the files the
/// API serves (not, say, the checkout's `.git/`).
fn source_path_known(server: &(dyn AbstractServer + Send + Sync), path: &str) -> bool {
    server.file_facet_info(path).is_some()
        || server
            .try_delta_meta()
            .is_some_and(|meta| meta.changed.contains_key(path))
}

async fn route_body(
    route: &str,
    server: Box<dyn AbstractServer + Send + Sync>,
    params: &Params,
    file_path: Option<&str>,
    accepts_gzip: bool,
) -> Result<Body, ApiError> {
    let error = |e| server_error(route, e);
    match route {
        "tree-info" => {
            let info = server.tree_info().map_err(error)?;
            let commit = server.commit_info().map_err(error)?;
            Ok(Body::json(&json!({
                "api": "v1",
                "name": info.name,
                "commit": commit.map(|commit| json!({
                    "rev": commit.rev,
                    "header": commit.header,
                    "summary": commit.summary,
                    "date": commit.date.to_rfc3339(),
                })),
                "path_kinds": server.path_kinds(),
                "expanded_subsystems": server.expanded_subsystems(),
                "try_meta": server.try_delta_meta(),
            })))
        }
        "try-status" => {
            let status = server
                .try_import_status(&push_param(params)?)
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(status).unwrap_or_default(),
            ))
        }
        "try-import" => {
            let push = push_param(params)?;
            server.request_try_import(&push).await.map_err(error)?;
            let status = server.try_import_status(&push).await.map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(status).unwrap_or_default(),
            ))
        }
        "try-push" => {
            let push = server.try_push(&push_param(params)?).await.map_err(error)?;
            Ok(Body::json(&serde_json::to_value(push).unwrap_or_default()))
        }
        "spec-prs" => {
            let summaries = server
                .spec_prs(spec_param(params)?.as_deref())
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(summaries).unwrap_or_default(),
            ))
        }
        "spec-prs-sync-status" => {
            let status = server
                .spec_prs_sync_status(spec_param(params)?.as_deref())
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(status).unwrap_or_default(),
            ))
        }
        "spec-prs-sync" => {
            let spec = spec_param(params)?;
            server
                .request_spec_prs_sync(spec.as_deref())
                .await
                .map_err(error)?;
            let status = server
                .spec_prs_sync_status(spec.as_deref())
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(status).unwrap_or_default(),
            ))
        }
        "rev-dir" => {
            let commit = commit_param(params)?;
            let path = params.get("path").unwrap_or_default();
            if !path.is_empty() && !valid_tree_path(path.trim_end_matches('/')) {
                return Err(bad(format!("Not a path in the tree: {:?}", path)));
            }
            let data = server.rev_dir(&commit, path).await.map_err(error)?;
            Ok(Body::json(&data.unwrap_or(Value::Null)))
        }
        "rev-file" => {
            let commit = commit_param(params)?;
            let path = params.required("path")?;
            if !valid_tree_path(path) {
                return Err(bad(format!("Not a path in the tree: {:?}", path)));
            }
            let file = server.rev_file(&commit, path).await.map_err(error)?;
            Ok(Body::json(&serde_json::to_value(file).unwrap_or_default()))
        }
        "history" => {
            let commit = commit_param(params)?;
            let path = params.required("path")?;
            if !valid_tree_path(path) {
                return Err(bad(format!("Not a path in the tree: {:?}", path)));
            }
            let scope = params.get("scope");
            if scope.is_some_and(|scope| scope.len() > MAX_SYM_BYTES) {
                return Err(bad("scope is too long"));
            }
            let token = match params.get("token") {
                Some(token) => Some(
                    token
                        .parse::<usize>()
                        .map_err(|_| bad("token must be a token's index"))?,
                ),
                None => None,
            };
            let data = server
                .history(&commit, path, scope, token)
                .await
                .map_err(error)?;
            Ok(Body::json(&data))
        }
        "rev-diff" => {
            let commit = commit_param(params)?;
            let path = params.required("path")?;
            if !valid_tree_path(path) {
                return Err(bad(format!("Not a path in the tree: {:?}", path)));
            }
            let diff = server.rev_diff(&commit, path).await.map_err(error)?;
            Ok(Body::json(&serde_json::to_value(diff).unwrap_or_default()))
        }
        "commit" => {
            let data = server
                .commit_page(&commit_param(params)?)
                .await
                .map_err(error)?;
            Ok(Body::json(&data))
        }
        "commit-info" => {
            let revs = params.required("revs")?;
            let hex = |rev: &str| {
                (7..=40).contains(&rev.len())
                    && rev
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            };
            if revs.split(',').count() > MAX_EXPLORE_REVS || !revs.split(',').all(hex) {
                return Err(bad("revs must be revisions (hexadecimal)"));
            }
            let json = server.commit_info_json(revs).await.map_err(error)?;
            Ok(Body::new("application/json", json.into_bytes()))
        }
        "map-rev" => {
            let kind = params.required("kind")?;
            let rev = params.required("from")?;
            let valid_rev =
                (7..=40).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit());
            if !matches!(kind, "hg" | "old") || !valid_rev {
                return Err(bad("kind must be hg or old, and from a revision"));
            }
            let mapped = server.map_rev(kind, rev).await.map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(mapped).unwrap_or_default(),
            ))
        }
        "explore-keys" => {
            let kind = params.required("kind")?;
            let keys = params.required("keys")?;
            let valid_keys = !keys.is_empty()
                && keys.len() <= 1000
                && keys
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b',' || b == b'-');
            if !matches!(kind, "bug" | "phab") || !valid_keys {
                return Err(bad("kind must be bug or phab, and keys bugs or revisions"));
            }
            Ok(Body::json(
                &server.explore_keys(kind, keys).await.map_err(error)?,
            ))
        }
        "interdiff" | "interdiff-file" => {
            let a = revs_param(params, "a")?;
            let b = revs_param(params, "b")?;
            if route == "interdiff" {
                return Ok(Body::json(
                    &server.interdiff_files(a, b).await.map_err(error)?,
                ));
            }
            let path = params.required("path")?;
            if !valid_tree_path(path) {
                return Err(bad(format!("Not a path in the tree: {:?}", path)));
            }
            let file = server.interdiff_file(a, b, path).await.map_err(error)?;
            Ok(Body::json(&serde_json::to_value(file).unwrap_or_default()))
        }
        "outline-diff" => {
            let from = params.required("from")?;
            let to = commit_param_named(params, "to")?;
            let valid_from = match from.strip_prefix("since:") {
                Some(day) => {
                    day.len() == 10 && day.bytes().all(|b| b.is_ascii_digit() || b == b'-')
                }
                None => is_hex_rev(from),
            };
            let path = params.required("path")?;
            if !valid_from || !valid_tree_path(path) {
                return Err(bad(
                    "from must be a revision or since:YYYY-MM-DD, and path a path in the tree",
                ));
            }
            let diff = server
                .outline_diff(from, &to, path, params.flag("all")?)
                .await
                .map_err(error)?;
            Ok(Body::json(&diff))
        }
        "hyperblame" => {
            let commit = commit_param(params)?;
            let path = params.required("path")?;
            let file = params.required("file")?;
            let valid_file = file.len() <= 1000
                && file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b",./-".contains(&b));
            if !valid_tree_path(path) || !valid_file {
                return Err(bad(
                    "path must be a path in the tree, and file a hyperblame file",
                ));
            }
            let json = server
                .hyperblame_json(&commit, path, file)
                .await
                .map_err(error)?;
            Ok(Body::new("application/json", json.into_bytes()))
        }
        "stack-strip" => {
            let commit = commit_param(params)?;
            let path = params.required("path")?;
            let stack = params.required("stack")?;
            let valid_stack = stack.len() <= 1000
                && stack
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_.,-".contains(&b));
            if !valid_tree_path(path) || !valid_stack {
                return Err(bad(
                    "path must be a path in the tree, and stack a stack page",
                ));
            }
            let json = server
                .stack_strip_json(stack, &commit, path)
                .await
                .map_err(error)?;
            Ok(Body::new("application/json", json.into_bytes()))
        }
        "explore" => {
            let revs: Vec<String> = params
                .required("revs")?
                .split(',')
                .map(str::to_string)
                .collect();
            let hex = |rev: &String| {
                (7..=40).contains(&rev.len())
                    && rev
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            };
            if revs.len() > MAX_EXPLORE_REVS || !revs.iter().all(hex) {
                return Err(bad(format!(
                    "revs must be at most {} revisions (hexadecimal)",
                    MAX_EXPLORE_REVS
                )));
            }
            let stack_url = params.get("stack_url");
            if stack_url.is_some_and(|url| {
                url.len() > 200
                    || !url
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b))
            }) {
                return Err(bad(
                    "stack_url must be a page of the tree, ex: try/hg/<push>",
                ));
            }
            let data = server.explore(&revs, stack_url).await.map_err(error)?;
            Ok(Body::json(&data))
        }
        "config" => {
            let mut config = server.tree_config().map_err(error)?;
            if let Some(config) = config.as_object_mut() {
                config.remove("private");
            }
            Ok(Body::json(&config))
        }
        "crossref" => {
            let sym = params.symbol()?;
            let fields: Option<BTreeSet<&str>> = match params.get("fields") {
                Some(fields) => {
                    let fields: BTreeSet<&str> = fields.split(',').collect();
                    let valid = |field: &&str| {
                        !field.is_empty()
                            && field
                                .bytes()
                                .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'-')
                    };
                    if !fields.iter().all(valid) {
                        return Err(bad("fields must be crossref entries' fields"));
                    }
                    Some(fields)
                }
                None => None,
            };
            let entry = server.crossref_lookup(sym).await.map_err(error)?;
            let mut value = serde_json::to_value(entry).unwrap_or_default();
            if let (Some(fields), Some(object)) = (fields, value.as_object_mut()) {
                object.retain(|key, _| fields.contains(key.as_str()));
            }
            Ok(Body::json(&value))
        }
        "jumpref" => {
            let jumpref = server
                .jumpref_lookup(params.symbol()?)
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(jumpref).unwrap_or_default(),
            ))
        }
        "identifiers" => {
            let needle = params.required("q")?;
            if needle.is_empty() || needle.len() > MAX_NEEDLE_BYTES {
                return Err(bad(format!("q must have 1 to {} bytes", MAX_NEEDLE_BYTES)));
            }
            let ids = server
                .search_identifiers(
                    needle,
                    params.flag("exact")?,
                    params.flag("ignore_case")?,
                    params.limit()?,
                )
                .await
                .map_err(error)?;
            Ok(Body::json(&serde_json::to_value(ids).unwrap_or_default()))
        }
        "files" => {
            let pathre = params.required("re")?;
            if pathre.len() > MAX_NEEDLE_BYTES {
                return Err(bad(format!(
                    "re must have at most {} bytes",
                    MAX_NEEDLE_BYTES
                )));
            }
            let not_pathre = params.get("not_re").unwrap_or_default();
            if not_pathre.len() > MAX_NEEDLE_BYTES {
                return Err(bad(format!(
                    "not_re must have at most {} bytes",
                    MAX_NEEDLE_BYTES
                )));
            }
            let matches = server
                .search_files(pathre, not_pathre, params.flag("dirs")?, params.limit()?)
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(matches).unwrap_or_default(),
            ))
        }
        "source" => {
            let path = file_path.unwrap_or_default();
            if !source_path_known(server.as_ref(), path) {
                return Err(not_found(format!("No such file: {}", path)));
            }
            match server.fetch_raw_source(path).await {
                Ok(text) => Ok(Body::new("text/plain; charset=utf-8", text.into_bytes())),
                Err(_) => Err(not_found(format!("No such file: {}", path))),
            }
        }
        "analysis" => {
            let path = file_path.unwrap_or_default();
            // (A normalized analysis file as it is, gzipped, rather than its
            // records parsed, processed and serialized in memory; nginx asks
            // for gzip, and decompresses it for clients which don't.)
            if accepts_gzip && let Some(file) = server.normalized_analysis_file(path) {
                let bytes = tokio::fs::read(&file)
                    .await
                    .map_err(|_| not_found(format!("No analysis of {}", path)))?;
                // (gzip's trailer has the decoded length, mod 2^32.)
                let decoded_len = bytes.len().checked_sub(4).map_or(0, |at| {
                    u32::from_le_bytes(bytes[at..].try_into().unwrap()) as usize
                });
                return Ok(Body {
                    content_type: "application/x-ndjson",
                    bytes,
                    encoding: Some("gzip"),
                    decoded_len,
                });
            }
            // (Else its share of the memory budget, by its length: its records
            // parsed and serialized take ~9 times that, ex: 355 MB for 40 MB;
            // see `memory_budget`.)
            let estimate = server.analysis_file_len(path).unwrap_or(0) * 9;
            let _share = crate::memory_budget::budget().acquire(estimate).await;
            let Ok(mut records) = server.fetch_raw_analysis(path).await else {
                return Err(not_found(format!("No analysis of {}", path)));
            };
            let mut bytes = vec![];
            while let Some(record) = records.next().await {
                serde_json::to_writer(&mut bytes, &record).unwrap_or_default();
                bytes.push(b'\n');
            }
            Ok(Body::new("application/x-ndjson", bytes))
        }
        "changed-lines" => {
            let spec = params.required("spec")?;
            let hex = |rev: &str| {
                (7..=40).contains(&rev.len())
                    && rev
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            };
            let valid = match spec.split_once("..") {
                Some((from, to)) => hex(from) && hex(to),
                None => hex(spec),
            };
            if !valid {
                return Err(bad("spec must be <rev> or <rev>..<rev> (hexadecimal)"));
            }
            let spec = spec.to_string();
            let lines = tokio::task::spawn_blocking(move || server.rev_changed_lines(&spec))
                .await
                .map_err(|e| error(e.into()))?
                .map_err(error)?;
            Ok(Body::json(&serde_json::to_value(lines).unwrap_or_default()))
        }
        "formatted-lines" => {
            let path = file_path.unwrap_or_default();
            if !source_path_known(server.as_ref(), path) {
                return Err(not_found(format!("No such file: {}", path)));
            }
            let (lines, sym_info) = server.fetch_formatted_lines(path).await.map_err(error)?;
            Ok(Body::json(&json!({ "lines": lines, "sym_info": sym_info })))
        }
        "html-lines" => {
            let path = file_path.unwrap_or_default();
            if !source_path_known(server.as_ref(), path) {
                return Err(not_found(format!("No such file: {}", path)));
            }
            let lines = parse_lines(params.required("lines")?).ok_or_else(|| {
                bad(format!(
                    "lines must be like 1-40,52 (at most {} of them)",
                    MAX_LINES
                ))
            })?;
            let key = ustr::ustr(path);
            let mut rows = server
                .fetch_html_lines(vec![(key, lines)])
                .await
                .map_err(error)?;
            let rows: BTreeMap<u32, _> =
                rows.remove(&key).unwrap_or_default().into_iter().collect();
            Ok(Body::json(&serde_json::to_value(rows).unwrap_or_default()))
        }
        "file-info" => {
            let info = server.file_facet_info(file_path.unwrap_or_default()).map(
                |(path_kind, subsystem)| json!({ "path_kind": path_kind, "subsystem": subsystem }),
            );
            Ok(Body::json(&info.unwrap_or(Value::Null)))
        }
        "file-recency" => {
            let recency = server
                .file_recency_lookup(file_path.unwrap_or_default())
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(recency).unwrap_or_default(),
            ))
        }
        "journal" => {
            let kind = match params.required("kind")? {
                "future" => JournalKind::Future,
                "files-delta" => JournalKind::FilesDelta,
                "tokens" => JournalKind::Tokens,
                _ => return Err(bad("kind must be future, files-delta or tokens")),
            };
            let target = params.required("target")?;
            let valid_target = match kind {
                JournalKind::Tokens => !target.is_empty() && target.len() <= MAX_SYM_BYTES,
                _ => valid_tree_path(target),
            };
            if !valid_target {
                return Err(bad("target must be a path in the tree, or a token"));
            }
            let source_rev = params.get("source_rev");
            if source_rev.is_some_and(|rev| !is_full_rev(rev)) {
                return Err(bad("source_rev must be a full revision"));
            }
            let records = server
                .fetch_history_journal(kind, target, source_rev, params.flag("expand")?)
                .await
                .map_err(error)?;
            Ok(Body::json(&Value::Array(records)))
        }
        "token-history" => {
            let token = params.required("token")?;
            if token.is_empty() || token.len() > MAX_SYM_BYTES {
                return Err(bad("token must be a token"));
            }
            let opts = TokenHistoryOptions {
                case_sensitive: params.flag("case_sensitive")?,
                view: params.flag("view")?,
            };
            let history = server.token_history(token, opts).await.map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(history).unwrap_or_default(),
            ))
        }
        "token-timeline" => {
            // (Tokens are words, without commas; the timeline takes at most
            // MAX_TOKENS of them.)
            let tokens: Vec<String> = params
                .required("tokens")?
                .split(',')
                .filter(|token| !token.is_empty())
                .map(str::to_string)
                .collect();
            if tokens.is_empty() || tokens.iter().any(|token| token.len() > MAX_SYM_BYTES) {
                return Err(bad("tokens must be comma-separated tokens"));
            }
            let opts = TokenTimelineOptions {
                case_sensitive: params.flag("case_sensitive")?,
            };
            let timeline = server.token_timeline(&tokens, opts).await.map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(timeline).unwrap_or_default(),
            ))
        }
        "token-names" => {
            let query = params.required("q")?;
            if query.len() > MAX_SYM_BYTES {
                return Err(bad("q is too long"));
            }
            let how = match params.get("match").unwrap_or("contains") {
                "prefix" => NameMatch::Prefix,
                "suffix" => NameMatch::Suffix,
                "contains" => NameMatch::Contains,
                _ => return Err(bad("match must be prefix, suffix or contains")),
            };
            let matches = server
                .token_names(query, how, params.limit()?)
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(matches).unwrap_or_default(),
            ))
        }
        "linked-crossref" => {
            let tree = params.required("tree")?;
            if tree.is_empty()
                || !tree
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            {
                return Err(bad("tree must be a tree's name"));
            }
            let entry = server
                .linked_crossref_lookup(tree, params.symbol()?)
                .await
                .map_err(error)?;
            Ok(Body::json(&serde_json::to_value(entry).unwrap_or_default()))
        }
        "text" => {
            let pattern = params.required("re")?;
            if pattern.is_empty() || pattern.len() > MAX_NEEDLE_BYTES {
                return Err(bad(format!("re must have 1 to {} bytes", MAX_NEEDLE_BYTES)));
            }
            let path = params.get("path").unwrap_or_default();
            if path.len() > MAX_NEEDLE_BYTES {
                return Err(bad(format!(
                    "path must have at most {} bytes",
                    MAX_NEEDLE_BYTES
                )));
            }
            let not_path = params.get("not_path").unwrap_or_default();
            if not_path.len() > MAX_NEEDLE_BYTES {
                return Err(bad(format!(
                    "not_path must have at most {} bytes",
                    MAX_NEEDLE_BYTES
                )));
            }
            let matches = server
                .search_text(
                    pattern,
                    params.flag("fold_case")?,
                    path,
                    not_path,
                    params.limit()?,
                )
                .await
                .map_err(error)?;
            Ok(Body::json(
                &serde_json::to_value(matches).unwrap_or_default(),
            ))
        }
        _ => Err(not_found(format!("Unknown route {:?}", route))),
    }
}

/// A fake tree, for tests of the API (and `RemoteServer`'s, over it).
#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// `dir/a.cpp`'s normalized analysis file (see `Fake`), gzipped.
    pub fn normalized_fixture() -> std::path::PathBuf {
        use std::io::Write;
        static PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        PATH.get_or_init(|| {
            let path =
                std::env::temp_dir().join(format!("data-api-normalized-{}.gz", std::process::id()));
            let mut gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&path).unwrap(),
                flate2::Compression::default(),
            );
            gz.write_all(b"{\"loc\":\"1:4-5\",\"sym\":\"A\"}\n")
                .unwrap();
            gz.finish().unwrap();
            path
        })
        .clone()
    }

    use crate::abstract_server::{
        ErrorDetails, FileMatches, HtmlFileRoot, Result, SearchfoxIndexRoot, TextMatches, TreeInfo,
    };
    use crate::file_format::crossref::CrossrefData;
    use crate::file_format::jumpref::JumprefData;
    use crate::hyperblame::journals::JournalKind;
    use async_trait::async_trait;
    use futures_core::stream::BoxStream;
    use ustr::{Ustr, ustr};

    /// A tree with the file `dir/a.cpp`, whose source is "int a;\n", and
    /// the symbol `A` with a big crossref entry.
    #[derive(Clone)]
    pub struct Fake;

    fn missing() -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::DataLayer,
            message: "missing".to_string(),
        })
    }

    #[async_trait]
    impl AbstractServer for Fake {
        fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync> {
            Box::new(Fake)
        }
        fn tree_info(&self) -> Result<TreeInfo> {
            Ok(TreeInfo {
                name: "fake".to_string(),
            })
        }
        fn commit_info(&self) -> Result<Option<crate::abstract_server::CommitInfo>> {
            Ok(Some(crate::abstract_server::CommitInfo {
                rev: "0123456789abcdef0123456789abcdef01234567".to_string(),
                header: "Fake".to_string(),
                summary: "Fake".to_string(),
                date: chrono::DateTime::parse_from_rfc3339("2026-10-17T12:00:00+01:00").unwrap(),
            }))
        }
        fn file_facet_info(&self, path: &str) -> Option<(Ustr, Option<Ustr>)> {
            (path == "dir/a.cpp" || path == "dir").then(|| (ustr("normal"), None))
        }
        fn tree_config(&self) -> Result<Value> {
            Ok(json!({"search_endpoint": "query", "private": {"key": "secret"}}))
        }
        fn translate_path(&self, _root: SearchfoxIndexRoot, _path: &str) -> Result<String> {
            Err(ServerError::Unsupported)
        }
        async fn coverage_history(
            &self,
            _path: &str,
        ) -> Result<Option<Vec<crate::git_ops::RevisionCoverage>>> {
            Err(ServerError::Unsupported)
        }
        async fn coverage_summary(
            &self,
            _path: &str,
        ) -> Result<Option<crate::file_format::code_coverage_report::NodeMetadata>> {
            Err(ServerError::Unsupported)
        }
        fn normalized_analysis_file(&self, path: &str) -> Option<String> {
            (path == "dir/a.cpp").then(|| normalized_fixture().to_string_lossy().into_owned())
        }
        async fn fetch_raw_analysis<'a>(&self, path: &str) -> Result<BoxStream<'a, Value>> {
            if path != "dir/a.cpp" {
                return Err(missing());
            }
            let records = vec![json!({"loc": "1:4-5", "sym": "A"})];
            Ok(Box::pin(tokio_stream::iter(records)))
        }
        async fn fetch_raw_source(&self, path: &str) -> Result<String> {
            match path {
                "dir/a.cpp" => Ok("int a;\n".to_string()),
                _ => Err(missing()),
            }
        }
        async fn fetch_formatted_lines(&self, path: &str) -> Result<(Vec<String>, String)> {
            match path {
                "dir/a.cpp" => Ok((vec!["int <b>a</b>;".to_string()], "{}".to_string())),
                _ => Err(missing()),
            }
        }
        async fn fetch_html(&self, _root: HtmlFileRoot, _path: &str) -> Result<String> {
            Err(ServerError::Unsupported)
        }
        async fn crossref_lookup(&self, symbol: &str) -> Result<Option<CrossrefData>> {
            if symbol != "A" {
                return Ok(None);
            }
            Ok(Some(CrossrefData {
                idl_syms: Some(vec![ustr("B"); 1000]),
                ..Default::default()
            }))
        }
        async fn jumpref_lookup(&self, _symbol: &str) -> Result<Option<JumprefData>> {
            Ok(None)
        }
        async fn search_files(
            &self,
            pathre: &str,
            _not_pathre: &str,
            _include_dirs: bool,
            _limit: usize,
        ) -> Result<FileMatches> {
            regex::Regex::new(pathre)?;
            Ok(FileMatches {
                file_matches: vec![],
                limit_hit: false,
            })
        }
        async fn search_identifiers(
            &self,
            _needle: &str,
            _exact: bool,
            _ignore_case: bool,
            limit: usize,
        ) -> Result<Vec<(Ustr, Ustr)>> {
            Ok(vec![(ustr("A"), ustr("a")); limit.min(3)])
        }
        async fn search_text(
            &self,
            _pattern: &str,
            _fold_case: bool,
            _path: &str,
            _not_path: &str,
            _limit: usize,
        ) -> Result<TextMatches> {
            Err(ServerError::Unsupported)
        }
        async fn perform_query(&self, _q: &str) -> Result<Value> {
            Err(ServerError::Unsupported)
        }
        async fn fetch_history_journal(
            &self,
            _kind: JournalKind,
            _target: &str,
            _source_rev: Option<&str>,
            _expand: bool,
        ) -> Result<Vec<Value>> {
            Err(ServerError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Fake, normalized_fixture};
    use super::*;

    struct FakeServers;

    impl ApiServers for FakeServers {
        fn server(&self, tree: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
            (tree == "fake").then(|| Box::new(Fake) as Box<dyn AbstractServer + Send + Sync>)
        }
    }

    /// The status and body of the response for `url`.
    async fn get(url: &str) -> (StatusCode, String) {
        let uri: Uri = url.parse().unwrap();
        match respond(&FakeServers, Host::PipelineServer, false, false, &uri).await {
            Ok(body) => (StatusCode::OK, String::from_utf8(body.bytes).unwrap()),
            Err(error) => (error.status, error.body.to_string()),
        }
    }

    #[test]
    fn test_valid_tree_path() {
        assert!(valid_tree_path("dom/base/Document.cpp"));
        assert!(valid_tree_path("__GENERATED__/dist/include/a.h"));
        assert!(valid_tree_path("a/.hidden/b..c"));
        for path in [
            "",
            "/etc/passwd",
            "../etc/passwd",
            "a/../../etc/passwd",
            "a/..",
            "./a",
            "a//b",
            "a/",
            "a\\..\\b",
            "a\0b",
            "a\nb",
        ] {
            assert!(!valid_tree_path(path), "{:?}", path);
        }
    }

    #[test]
    fn test_routes_params() {
        // (A route's own parameters can't be the common ones, ex: `rev`, a
        // try push's context.)
        for route in ROUTES {
            for param in route.params {
                assert!(!COMMON_PARAMS.contains(param), "{}: {}", route.name, param);
            }
        }
    }

    #[test]
    fn test_lines() {
        let lines: BTreeSet<u32> = [1, 2, 3, 5, 7, 8].into();
        assert_eq!(format_lines(&lines), "1-3,5,7-8");
        assert_eq!(parse_lines("1-3,5,7-8"), Some(lines));
        for spec in ["", "0", "3-1", "a", "1-", "1-1000000", "1,,2"] {
            assert_eq!(parse_lines(spec), None, "{:?}", spec);
        }
    }

    #[test]
    fn test_decode() {
        assert_eq!(decode("a%2Fb+c", true).unwrap(), "a/b c");
        assert_eq!(decode("a+b", false).unwrap(), "a+b");
        assert_eq!(decode("%E2%9C%93", false).unwrap(), "\u{2713}");
        for text in ["%", "%2", "%zz", "%FF", "%00", "%0a"] {
            assert!(decode(text, false).is_err(), "{:?}", text);
        }
    }

    #[tokio::test]
    async fn test_paths() {
        assert_eq!(
            get("/fake/api/v1/source/dir/a.cpp").await,
            (StatusCode::OK, "int a;\n".to_string())
        );
        assert_eq!(
            get("/fake/api/v1/source/dir%2Fa.cpp").await.0,
            StatusCode::OK
        );
        // Traversal attempts, encoded or not, and other paths that aren't
        // plain relative ones, are bad requests.
        for url in [
            "/fake/api/v1/source/../../etc/passwd",
            "/fake/api/v1/source/dir/../../../etc/passwd",
            "/fake/api/v1/source/..%2F..%2Fetc%2Fpasswd",
            "/fake/api/v1/source/%2e%2e/%2e%2e/etc/passwd",
            "/fake/api/v1/source/%2E%2E%2F%2E%2E%2Fetc%2Fpasswd",
            "/fake/api/v1/source//etc/passwd",
            "/fake/api/v1/source/%2Fetc%2Fpasswd",
            "/fake/api/v1/source/dir/./a.cpp",
            "/fake/api/v1/source/dir//a.cpp",
            "/fake/api/v1/source/..%5C..%5Cetc%5Cpasswd",
            "/fake/api/v1/source/dir/a.cpp%00.txt",
            "/fake/api/v1/source/%c0%ae%c0%ae/etc/passwd",
            "/fake/api/v1/source/dir/a.cpp%",
            "/fake/api/v1/analysis/../dir/a.cpp",
        ] {
            assert_eq!(
                get(url).await.0,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{}",
                url
            );
        }
        // Only the tree's files.
        assert_eq!(
            get("/fake/api/v1/source/.git/config").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get("/fake/api/v1/source/dir/b.cpp").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get("/fake/api/v1/analysis/dir/a.cpp").await,
            (
                StatusCode::OK,
                "{\"loc\":\"1:4-5\",\"sym\":\"A\"}\n".to_string()
            )
        );
        assert_eq!(
            get("/fake/api/v1/analysis/dir/b.cpp").await.0,
            StatusCode::NOT_FOUND
        );
    }

    /// A normalized analysis file is served as it is, gzipped, to requests
    /// taking gzip (with its decoded length checked against `max_bytes`),
    /// and parsed and serialized for the others.
    #[tokio::test]
    async fn test_normalized_analysis() {
        let uri: Uri = "/fake/api/v1/analysis/dir/a.cpp".parse().unwrap();
        let body = respond(&FakeServers, Host::PipelineServer, false, true, &uri)
            .await
            .unwrap();
        assert_eq!(body.encoding, Some("gzip"));
        assert_eq!(body.bytes, std::fs::read(normalized_fixture()).unwrap());
        assert_eq!(
            body.decoded_len,
            "{\"loc\":\"1:4-5\",\"sym\":\"A\"}\n".len()
        );
        let response = body.into_response();
        assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(response.headers()[header::VARY], "Accept-Encoding");

        let plain = respond(&FakeServers, Host::PipelineServer, false, false, &uri)
            .await
            .unwrap();
        assert_eq!(plain.encoding, None);
        assert_eq!(plain.bytes, b"{\"loc\":\"1:4-5\",\"sym\":\"A\"}\n");

        let small: Uri = "/fake/api/v1/analysis/dir/a.cpp?max_bytes=10"
            .parse()
            .unwrap();
        assert_eq!(
            respond(&FakeServers, Host::PipelineServer, false, true, &small)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_ACCEPTABLE
        );
    }

    #[tokio::test]
    async fn test_params() {
        assert_eq!(
            get("/fake/api/v1/jumpref?sym=A").await,
            (StatusCode::OK, "null".to_string())
        );
        // (Without the private settings.)
        assert_eq!(
            get("/fake/api/v1/config").await,
            (
                StatusCode::OK,
                "{\"search_endpoint\":\"query\"}".to_string()
            )
        );
        assert_eq!(
            get("/fake/api/v1/identifiers?q=a&exact=true&limit=2").await,
            (StatusCode::OK, "[[\"A\",\"a\"],[\"A\",\"a\"]]".to_string())
        );
        for url in [
            // Unknown, repeated, malformed and missing parameters, and bad
            // values.
            "/fake/api/v1/jumpref?sym=A&utm_source=x",
            "/fake/api/v1/jumpref?sym=A&sym=B",
            "/fake/api/v1/jumpref?sym=A&",
            "/fake/api/v1/jumpref?sym",
            "/fake/api/v1/jumpref?sym=%zz",
            "/fake/api/v1/jumpref",
            "/fake/api/v1/jumpref?sym=",
            "/fake/api/v1/tree-info?sym=A",
            "/fake/api/v1/identifiers?q=a&exact=yes",
            "/fake/api/v1/identifiers?q=a&limit=0",
            "/fake/api/v1/identifiers?q=a&limit=-1",
            "/fake/api/v1/identifiers?q=a&limit=99999999999",
            "/fake/api/v1/files?re=(",
            "/fake/api/v1/crossref?sym=A&fields=meta,%3Cb%3E",
            "/fake/api/v1/crossref?sym=A&max_bytes=lots",
            "/fake/api/v1/crossref?sym=A&rev=main",
            "/fake/api/v1/changed-lines?spec=HEAD",
            "/fake/api/v1/changed-lines?spec=:/fix",
            "/fake/api/v1/changed-lines?spec=abcdef1...abcdef2",
            "/fake/api/v1/journal?kind=past&target=a.cpp",
            "/fake/api/v1/journal?kind=future&target=../a.cpp",
            "/fake/api/v1/journal?kind=future&target=a.cpp&source_rev=HEAD",
            "/fake/api/v1/linked-crossref?tree=../x&sym=A",
            "/fake/api/v1/text?re=",
            "/fake/api/v1/formatted-lines/../a.cpp",
            "/fake/api/v1/try-push?push=main",
            "/fake/api/v1/explore?revs=abcdef1,HEAD",
            "/fake/api/v1/explore?revs=abcdef1&stack_url=%3Cscript%3E",
        ] {
            assert_eq!(
                get(url).await.0,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{}",
                url
            );
        }
        // (The web-server's routes, and POSTs as GETs.)
        assert_eq!(
            get("/fake/api/v1/try-status?push=0123456789ab").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            respond(
                &FakeServers,
                Host::WebServer,
                false,
                false,
                &"/fake/api/v1/try-import?push=0123456789ab".parse().unwrap()
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            respond(
                &FakeServers,
                Host::WebServer,
                false,
                false,
                &"/fake/api/v1/spec-prs?spec=../dom".parse().unwrap()
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        for url in [
            "/other/api/v1/tree-info",
            "/fake/api/v2/tree-info",
            "/fake/api/v1/nothing",
            "/fake/api/v1/source",
            "/fake/api/v1/tree-info/x",
            // (A try push the tree doesn't have.)
            "/fake/api/v1/tree-info?rev=0123456789012345678901234567890123456789",
        ] {
            assert_eq!(get(url).await.0, StatusCode::NOT_FOUND, "{}", url);
        }
    }

    #[tokio::test]
    async fn test_size_cap() {
        // `A`'s entry is ~4 KB: over a smaller cap, it's not acceptable, with
        // its size; under a bigger one, or with only some fields, it's there.
        let (status, body) = get("/fake/api/v1/crossref?sym=A&max_bytes=1000").await;
        assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["max_bytes"], 1000);
        assert!(body["bytes"].as_u64().unwrap() > 1000);
        assert_eq!(
            get("/fake/api/v1/crossref?sym=A&max_bytes=10000").await.0,
            StatusCode::OK
        );
        assert_eq!(
            get("/fake/api/v1/crossref?sym=A&max_bytes=1000&fields=meta,callees").await,
            (StatusCode::OK, "{}".to_string())
        );
        assert_eq!(
            get("/fake/api/v1/crossref?sym=Z").await,
            (StatusCode::OK, "null".to_string())
        );
    }
}
