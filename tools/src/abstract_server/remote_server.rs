//! A tree of a remote searchfox server (see `make_remote_server`), over HTTP:
//! the index's primitives over its data API (see `data_api`), which returns
//! them as the tools' types serialize, so that the code above this can't tell
//! a remote tree from a local one (ex: the checks' remote pass, or a
//! contributor's tools and servers working over firefox-main without indexing
//! it), and the rest over its pages (rendered sources, the router's search).
//! With `try_overlay`, the tree as a try push changes it (the API's `rev=`).
//!
//! Responses are kept in memory for the process, by URL (see `Memo`), so
//! that a request in flight is made once, and lookups prefetched (see
//! `AbstractServer::prefetch_crossref`) are found at once.
//!
//! Responses can be cached on disk, but only where `remote_cache` says (see
//! `ToolOpts::remote_cache`; there's no default location, so that nothing
//! fills a disk by surprise): under `{cache}/{host}/{tree}/{indexed rev}/`,
//! since they're immutable for an index (try pushes' responses have their
//! heads in their URLs), with the other indexed revisions' removed.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::DateTime;
use futures_core::stream::BoxStream;
use serde::de::DeserializeOwned;
use serde_json::{Value, from_str};
use tokio::sync::OnceCell;
use url::{ParseError, Url};
use ustr::{Ustr, UstrMap, ustr};

use crate::file_format::chunked_gzip::Row;

use crate::{
    dataflow::slice::ChangedLines,
    file_format::{
        code_coverage_report, crossref::CrossrefData, jumpref::JumprefData, recency::FileRecency,
    },
    git_ops::RevisionCoverage,
    hyperblame::journals::JournalKind,
    hyperblame::token_history::{TokenHistory, TokenHistoryOptions},
    try_delta::TryDeltaMeta,
};

use super::{
    CommitInfo, HtmlFileRoot, TextMatches, TreeInfo,
    server_interface::{
        AbstractServer, ErrorDetails, ErrorLayer, FileMatches, Result, SearchfoxIndexRoot,
        ServerError,
    },
};

/// reqwest won't return an error for an unhappy status code itself; someone
/// would need to call `Response::error_from_status`, so for now we'll generally
/// assume everything is some kind of transient problem.
impl From<reqwest::Error> for ServerError {
    fn from(err: reqwest::Error) -> ServerError {
        ServerError::TransientProblem(ErrorDetails {
            layer: ErrorLayer::ServerLayer,
            message: err.to_string(),
        })
    }
}

impl From<ParseError> for ServerError {
    fn from(err: ParseError) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::BadInput,
            message: err.to_string(),
        })
    }
}

/// The size cap the API's requests ask for (see `data_api`): the responses
/// whole, as a local index's.
const MAX_BYTES: &str = "1073741824";
/// How many requests prefetching makes at once.
const PREFETCH_CONCURRENCY: usize = 16;
/// About how many bytes of responses the memo keeps (see `Memo`).
const MEMO_BYTES: usize = 256 << 20;
/// The most results the API's searches return (`data_api::MAX_LIMIT`).
const MAX_LIMIT: usize = 2_000_001;
/// How many times a request turned away for its client's rate (429) is
/// retried, after the `Retry-After` it's told (at most `MAX_RETRY_AFTER`).
const RETRIES: usize = 3;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The one HTTP client (connections kept alive across requests), which says
/// what it is (an agent, for the servers' admission; see
/// `admission::RequestClass`).
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let builder = || {
            reqwest::Client::builder().user_agent(concat!(
                "searchfox-remote-server/",
                env!("CARGO_PKG_VERSION")
            ))
        };
        builder().build().unwrap_or_else(|err| {
            // (Without the system's CA certificates, ex: in a nix build's
            // sandbox, whose tests use a local http:// server, only http://
            // servers work.)
            log::warn!("No TLS roots, so only http:// servers will work: {}", err);
            builder()
                .tls_certs_only(std::iter::empty())
                .build()
                .unwrap()
        })
    })
}

fn problem(sticky: bool, layer: ErrorLayer, message: String) -> ServerError {
    let details = ErrorDetails { layer, message };
    if sticky {
        ServerError::StickyProblem(details)
    } else {
        ServerError::TransientProblem(details)
    }
}

/// The body of the response for `url`, retrying when turned away for the
/// client's rate; other unhappy statuses are errors (with the API's error
/// message, if it says one, so that they read as the tree's server's own).
async fn get_bytes(url: Url, accept: Option<&str>) -> Result<Vec<u8>> {
    request_bytes(reqwest::Method::GET, url, accept).await
}

/// `get_bytes`, with `method`.
async fn request_bytes(method: reqwest::Method, url: Url, accept: Option<&str>) -> Result<Vec<u8>> {
    let mut retries = 0;
    loop {
        let mut request = client().request(method.clone(), url.clone());
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS && retries < RETRIES {
            retries += 1;
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()?.parse().ok())
                .map_or(Duration::from_secs(1), Duration::from_secs);
            tokio::time::sleep(retry_after.min(MAX_RETRY_AFTER)).await;
            continue;
        }
        let bytes = response.bytes().await?.to_vec();
        if status.is_success() {
            return Ok(bytes);
        }
        let error = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body["error"].as_str().map(str::to_string));
        log::debug!("Server status of {} for {}: {:?}", status, url, error);
        let message =
            error.unwrap_or_else(|| format!("Server status of {} for {}", status, url.path()));
        return Err(match status.as_u16() {
            422 => problem(true, ErrorLayer::BadInput, message),
            404 => problem(true, ErrorLayer::DataLayer, message),
            429 => problem(false, ErrorLayer::ServerLayer, message),
            _ if status.is_server_error() => problem(false, ErrorLayer::ServerLayer, message),
            _ => problem(true, ErrorLayer::ServerLayer, message),
        });
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(ServerError::from)
}

/// Run `future` to completion from a synchronous method (some of
/// `AbstractServer`'s are), on the multi-threaded runtime they're called on.
fn block_on<F: Future>(future: F) -> Option<F::Output> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
        return None;
    }
    Some(tokio::task::block_in_place(|| handle.block_on(future)))
}

/// The disk cache of a tree's API responses (see the module's comment).
struct ResponseCache {
    /// `{cache}/{host}/{tree}`.
    dir: PathBuf,
}

impl ResponseCache {
    /// Where `url`'s response is for the indexed revision `rev`.
    fn path(&self, rev: &str, url: &Url) -> PathBuf {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        url.as_str().hash(&mut hasher);
        let hash = format!("{:016x}", hasher.finish());
        self.dir.join(rev).join(&hash[..2]).join(hash)
    }

    /// The cached response, if it's there (and for this URL, in case of the
    /// hash's collisions).
    fn read(&self, rev: &str, url: &Url) -> Option<Vec<u8>> {
        let bytes = fs::read(self.path(rev, url)).ok()?;
        let newline = bytes.iter().position(|b| *b == b'\n')?;
        (&bytes[..newline] == url.as_str().as_bytes()).then(|| bytes[newline + 1..].to_vec())
    }

    fn write(&self, rev: &str, url: &Url, body: &[u8]) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = self.path(rev, url);
        let Some(dir) = path.parent() else {
            return;
        };
        let temp = dir.join(format!(
            ".{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let written = fs::create_dir_all(dir).and_then(|_| {
            let mut file = fs::File::create(&temp)?;
            file.write_all(url.as_str().as_bytes())?;
            file.write_all(b"\n")?;
            file.write_all(body)?;
            fs::rename(&temp, &path)
        });
        if let Err(err) = written {
            log::warn!("Couldn't cache {}: {}", url, err);
            let _ = fs::remove_file(&temp);
        }
    }

    /// Remove the responses of the tree's other indexed revisions (named by
    /// their revisions, so nothing else).
    fn remove_others(&self, rev: &str) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name != rev && crate::try_delta::is_full_rev(&name) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// The API's responses by URL, for the process (all of its remote trees'
/// servers'), so that the requests for the same URL in flight at once are
/// made once, and later ones are answered from memory: a tree's responses
/// don't change while it's the same index (as for the disk cache), and a
/// server's indexed revision is looked up once (see `base_info`).  Kept to
/// about `MEMO_BYTES`: when the current generation's responses are half of
/// that, they become the previous generation, which replaces the one before,
/// and responses found in the previous one move back into the current one.
#[derive(Default)]
struct Memo {
    current: HashMap<String, Arc<OnceCell<Arc<Vec<u8>>>>>,
    previous: HashMap<String, Arc<OnceCell<Arc<Vec<u8>>>>>,
    current_bytes: usize,
}

impl Memo {
    fn get() -> &'static Mutex<Memo> {
        static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
        MEMO.get_or_init(Default::default)
    }

    /// The cell of `url`'s response (empty while it's being fetched).
    fn cell(url: &str) -> Arc<OnceCell<Arc<Vec<u8>>>> {
        let mut memo = Memo::get().lock().unwrap();
        if let Some(cell) = memo.current.get(url) {
            return cell.clone();
        }
        let cell = memo.previous.remove(url).unwrap_or_default();
        if let Some(body) = cell.get() {
            memo.current_bytes += body.len();
        }
        memo.current.insert(url.to_string(), cell.clone());
        cell
    }

    /// Count a fetched response's bytes, starting a new generation if the
    /// current one is full.
    fn add(bytes: usize) {
        let mut memo = Memo::get().lock().unwrap();
        memo.current_bytes += bytes;
        if memo.current_bytes > MEMO_BYTES / 2 {
            memo.previous = std::mem::take(&mut memo.current);
            memo.current_bytes = 0;
        }
    }
}

#[derive(Clone)]
struct RemoteServer {
    tree_name: String,
    /// `{tree}/api/v1/`.
    api_base_url: Url,
    source_base_url: Url,
    search_url: Url,
    /// The head of the try push whose context this is, if it's a
    /// `try_overlay`.
    rev: Option<String>,
    /// The API's `tree-info` of the tree as indexed, and with `rev`, once.
    base_info: Arc<OnceCell<Value>>,
    info: Arc<OnceCell<Value>>,
    cache: Option<Arc<ResponseCache>>,
}

impl RemoteServer {
    /// The URL of the API's `route` (with the file `path`, if any) with
    /// `params`, in this server's try push's context.
    fn api_url(&self, route: &str, path: Option<&str>, params: &[(&str, &str)]) -> Result<Url> {
        let mut url = self.api_base_url.join(route)?;
        if let Some(path) = path {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| problem(true, ErrorLayer::BadInput, "not a base URL".to_string()))?;
            segments.extend(path.strip_prefix('/').unwrap_or(path).split('/'));
        }
        {
            let mut query = url.query_pairs_mut();
            for (name, value) in params {
                query.append_pair(name, value);
            }
            if let Some(rev) = &self.rev {
                query.append_pair("rev", rev);
            }
            query.append_pair("max_bytes", MAX_BYTES);
        }
        Ok(url)
    }

    async fn tree_info_value(&self, rev: Option<&str>) -> Result<Value> {
        let mut url = self.api_base_url.join("tree-info")?;
        if let Some(rev) = rev {
            url.query_pairs_mut().append_pair("rev", rev);
        }
        parse(&get_bytes(url, None).await?)
    }

    /// The tree's `tree-info` as indexed (whose revision keys the cache).
    async fn base_info(&self) -> Result<&Value> {
        self.base_info
            .get_or_try_init(|| async {
                let info = self.tree_info_value(None).await?;
                if let (Some(cache), Some(rev)) = (&self.cache, info["commit"]["rev"].as_str()) {
                    cache.remove_others(rev);
                }
                Ok(info)
            })
            .await
    }

    /// The `tree-info` of this server's tree (in its try push's context).
    async fn info(&self) -> Result<&Value> {
        match &self.rev {
            None => self.base_info().await,
            Some(rev) => {
                self.info
                    .get_or_try_init(|| self.tree_info_value(Some(rev)))
                    .await
            }
        }
    }

    /// The body of the API's response for `route` (see `api_url`) of the tree
    /// as indexed, which changes (ex: a try push's import's status), so not
    /// cached, `post`ed if it asks for something.
    async fn api_live(&self, route: &str, params: &[(&str, &str)], post: bool) -> Result<Vec<u8>> {
        let base = RemoteServer {
            rev: None,
            ..self.clone()
        };
        let url = base.api_url(route, None, params)?;
        let method = match post {
            true => reqwest::Method::POST,
            false => reqwest::Method::GET,
        };
        request_bytes(method, url, None).await
    }

    /// The body of the API's response for `route` (see `api_url`), from the
    /// memo or the disk cache if it's there.
    async fn api(
        &self,
        route: &str,
        path: Option<&str>,
        params: &[(&str, &str)],
    ) -> Result<Arc<Vec<u8>>> {
        let url = self.api_url(route, path, params)?;
        let cell = Memo::cell(url.as_str());
        let body = cell
            .get_or_try_init(|| async {
                let body = self.fetch(url.clone()).await?;
                Memo::add(body.len());
                Ok::<_, ServerError>(Arc::new(body))
            })
            .await?;
        Ok(body.clone())
    }

    /// Fetch `route`'s responses for `syms` into the memo, at once (as
    /// `api`, so that the lookups they're for find them there).
    async fn prefetch_route(&self, route: &str, syms: &[Ustr]) {
        use futures_util::stream::{self, StreamExt};
        let mut syms: Vec<Ustr> = syms.to_vec();
        syms.sort();
        syms.dedup();
        stream::iter(syms)
            .map(|sym| async move {
                let _ = self.api(route, None, &[("sym", sym.as_str())]).await;
            })
            .buffer_unordered(PREFETCH_CONCURRENCY)
            .collect::<Vec<()>>()
            .await;
    }

    /// `url`'s response, from the disk cache if it's there.
    async fn fetch(&self, url: Url) -> Result<Vec<u8>> {
        let cache = match &self.cache {
            Some(cache) => self.base_info().await?["commit"]["rev"]
                .as_str()
                .map(|rev| (cache, rev.to_string())),
            None => None,
        };
        if let Some((cache, rev)) = &cache
            && let Some(body) = cache.read(rev, &url)
        {
            return Ok(body);
        }
        let body = get_bytes(url.clone(), None).await?;
        if let Some((cache, rev)) = &cache {
            cache.write(rev, &url, &body);
        }
        Ok(body)
    }
}

#[async_trait]
impl AbstractServer for RemoteServer {
    fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync> {
        Box::new(self.clone())
    }

    fn tree_info(&self) -> Result<TreeInfo> {
        Ok(TreeInfo {
            name: self.tree_name.clone(),
        })
    }

    fn commit_info(&self) -> Result<Option<CommitInfo>> {
        let info = block_on(self.info()).ok_or(ServerError::Unsupported)??;
        let commit = &info["commit"];
        let (Some(rev), Some(header), Some(date)) = (
            commit["rev"].as_str(),
            commit["header"].as_str(),
            commit["date"]
                .as_str()
                .and_then(|date| DateTime::parse_from_rfc3339(date).ok()),
        ) else {
            return Ok(None);
        };
        Ok(Some(CommitInfo {
            rev: rev.to_string(),
            header: header.to_string(),
            summary: commit["summary"].as_str().unwrap_or_default().to_string(),
            date,
        }))
    }

    fn path_kinds(&self) -> Vec<(Ustr, Ustr)> {
        let Some(Ok(info)) = block_on(self.base_info()) else {
            return vec![];
        };
        info["path_kinds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|kind| Some((ustr(kind[0].as_str()?), ustr(kind[1].as_str()?))))
            .collect()
    }

    fn tree_config(&self) -> Result<Value> {
        let body = block_on(self.api("config", None, &[])).ok_or(ServerError::Unsupported)??;
        parse(&body)
    }

    fn translate_path(&self, _root: SearchfoxIndexRoot, _sf_path: &str) -> Result<String> {
        // Remote servers don't have local filesystem paths.
        Err(ServerError::Unsupported)
    }

    async fn coverage_history(&self, _sf_path: &str) -> Result<Option<Vec<RevisionCoverage>>> {
        Ok(None)
    }

    async fn coverage_summary(
        &self,
        _sf_path: &str,
    ) -> Result<Option<code_coverage_report::NodeMetadata>> {
        Ok(None)
    }

    async fn fetch_raw_analysis<'a>(&self, sf_path: &str) -> Result<BoxStream<'a, Value>> {
        // (Processed, as the local index's are; see `data_api`.)
        let body = self.api("analysis", Some(sf_path), &[]).await?;
        let text = String::from_utf8_lossy(&body);
        let values: Result<Vec<Value>> = text
            .lines()
            .map(|line| from_str(line).map_err(ServerError::from))
            .collect();
        Ok(Box::pin(tokio_stream::iter(values?)))
    }

    async fn fetch_formatted_lines(&self, sf_path: &str) -> Result<(Vec<String>, String)> {
        let body: Value = parse(&self.api("formatted-lines", Some(sf_path), &[]).await?)?;
        let lines = serde_json::from_value(body["lines"].clone())?;
        Ok((lines, body["sym_info"].as_str().unwrap_or("{}").to_string()))
    }

    fn file_facet_info(&self, path: &str) -> Option<(Ustr, Option<Ustr>)> {
        let body = block_on(self.api("file-info", Some(path), &[]))?.ok()?;
        let info: Value = parse(&body).ok()?;
        Some((
            ustr(info["path_kind"].as_str()?),
            info["subsystem"].as_str().map(ustr),
        ))
    }

    async fn file_recency_lookup(&self, path: &str) -> Result<Option<FileRecency>> {
        parse(&self.api("file-recency", Some(path), &[]).await?)
    }

    async fn linked_crossref_lookup(
        &self,
        tree: &str,
        symbol: &str,
    ) -> Result<Option<CrossrefData>> {
        parse(
            &self
                .api("linked-crossref", None, &[("tree", tree), ("sym", symbol)])
                .await?,
        )
    }

    async fn fetch_raw_source(&self, sf_path: &str) -> Result<String> {
        let body = self.api("source", Some(sf_path), &[]).await?;
        String::from_utf8(body.to_vec())
            .map_err(|err| problem(true, ErrorLayer::DataLayer, err.to_string()))
    }

    async fn fetch_history_journal(
        &self,
        kind: JournalKind,
        target: &str,
        source_rev: Option<&str>,
        expand: bool,
    ) -> Result<Vec<Value>> {
        let kind = match kind {
            JournalKind::Future => "future",
            JournalKind::FilesDelta => "files-delta",
            JournalKind::Tokens => "tokens",
        };
        let mut params = vec![("kind", kind), ("target", target)];
        if let Some(rev) = source_rev {
            params.push(("source_rev", rev));
        }
        if expand {
            params.push(("expand", "true"));
        }
        parse(&self.api("journal", None, &params).await?)
    }

    async fn token_history(
        &self,
        token: &str,
        opts: TokenHistoryOptions,
    ) -> Result<Option<TokenHistory>> {
        let mut params = vec![("token", token)];
        if opts.case_sensitive {
            params.push(("case_sensitive", "true"));
        }
        if opts.view {
            params.push(("view", "true"));
        }
        parse(&self.api("token-history", None, &params).await?)
    }

    async fn fetch_html(&self, root: HtmlFileRoot, sf_path: &str) -> Result<String> {
        // We don't have access to raw templates, so just call that unsupported.
        // Note that we could special-case for "help.html" here since it does
        // get explicitly exposed as "index.html", but it's also fine to only
        // validate this for local files.
        if root == HtmlFileRoot::FormattedTemplate {
            return Err(ServerError::Unsupported);
        }
        // Our tree-relative paths should not start with a slash
        let norm_path = sf_path.strip_prefix('/').unwrap_or(sf_path);
        // We don't both caring about the presence of ".." here because we don't
        // have any security-ish things to worry about for a public web server.

        let url = self.source_base_url.join(norm_path)?;
        let html = get_bytes(url, None).await?;
        Ok(String::from_utf8_lossy(&html).to_string())
    }

    async fn fetch_html_lines(
        &self,
        requests: Vec<(Ustr, std::collections::BTreeSet<u32>)>,
    ) -> Result<UstrMap<std::collections::HashMap<u32, Row>>> {
        // (The server's rows, ex: a try push's head's, for its files.)
        let mut rows = UstrMap::default();
        for (path, lines) in requests {
            if lines.is_empty() {
                continue;
            }
            let spec = crate::data_api::format_lines(&lines);
            let body = self
                .api("html-lines", Some(&path), &[("lines", spec.as_str())])
                .await?;
            rows.insert(path, parse(&body)?);
        }
        Ok(rows)
    }

    async fn crossref_lookup(&self, symbol: &str) -> Result<Option<CrossrefData>> {
        parse(&self.api("crossref", None, &[("sym", symbol)]).await?)
    }

    async fn jumpref_lookup(&self, symbol: &str) -> Result<Option<JumprefData>> {
        parse(&self.api("jumpref", None, &[("sym", symbol)]).await?)
    }

    async fn prefetch_crossref(&self, syms: &[Ustr]) {
        self.prefetch_route("crossref", syms).await;
    }

    async fn prefetch_jumpref(&self, syms: &[Ustr]) {
        self.prefetch_route("jumpref", syms).await;
    }

    async fn prefetch_files(&self, paths: &[String]) {
        use futures_util::stream::{self, StreamExt};
        let mut paths: Vec<String> = paths.to_vec();
        paths.sort();
        paths.dedup();
        // (Each file's source, and its analysis.)
        let fetches: Vec<(bool, String)> = paths
            .into_iter()
            .flat_map(|path| [(true, path.clone()), (false, path)])
            .collect();
        stream::iter(fetches)
            .map(|(source, path)| async move {
                let route = if source { "source" } else { "analysis" };
                let _ = self.api(route, Some(&path), &[]).await;
            })
            .buffer_unordered(PREFETCH_CONCURRENCY)
            .collect::<Vec<()>>()
            .await;
    }

    async fn search_files(
        &self,
        pathre: &str,
        not_pathre: &str,
        include_dirs: bool,
        limit: usize,
    ) -> Result<FileMatches> {
        if limit == 0 {
            return Ok(FileMatches {
                file_matches: vec![],
                limit_hit: false,
            });
        }
        let limit = limit.min(MAX_LIMIT).to_string();
        let mut params = vec![("re", pathre), ("limit", limit.as_str())];
        if !not_pathre.is_empty() {
            params.push(("not_re", not_pathre));
        }
        if include_dirs {
            params.push(("dirs", "true"));
        }
        parse(&self.api("files", None, &params).await?)
    }

    async fn search_identifiers(
        &self,
        needle: &str,
        exact_match: bool,
        ignore_case: bool,
        match_limit: usize,
    ) -> Result<Vec<(Ustr, Ustr)>> {
        if match_limit == 0 || needle.is_empty() {
            return Ok(vec![]);
        }
        let limit = match_limit.min(MAX_LIMIT).to_string();
        let mut params = vec![("q", needle), ("limit", limit.as_str())];
        if exact_match {
            params.push(("exact", "true"));
        }
        if ignore_case {
            params.push(("ignore_case", "true"));
        }
        parse(&self.api("identifiers", None, &params).await?)
    }

    async fn search_text(
        &self,
        pattern: &str,
        fold_case: bool,
        path: &str,
        not_path: &str,
        limit: usize,
    ) -> Result<TextMatches> {
        let limit = limit.clamp(1, MAX_LIMIT).to_string();
        let mut params = vec![("re", pattern), ("limit", limit.as_str())];
        if fold_case {
            params.push(("fold_case", "true"));
        }
        if !path.is_empty() {
            params.push(("path", path));
        }
        if !not_path.is_empty() {
            params.push(("not_path", not_path));
        }
        parse(&self.api("text", None, &params).await?)
    }

    async fn perform_query(&self, q: &str) -> Result<Value> {
        let mut url = self.search_url.clone();
        // If adding more parameters, considering using `query_pairs_mut()`.
        url.set_query(Some(&format!("q={}", q)));
        let raw = get_bytes(url, Some("application/json")).await?;
        serde_json::from_slice(&raw).map_err(|err| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::ServerLayer,
                message: err.to_string(),
            })
        })
    }

    async fn try_import_status(&self, push: &str) -> Result<crate::try_import::ImportStatus> {
        parse(
            &self
                .api_live("try-status", &[("push", push)], false)
                .await?,
        )
    }

    async fn request_try_import(&self, push: &str) -> Result<()> {
        self.api_live("try-import", &[("push", push)], true).await?;
        Ok(())
    }

    async fn try_push(&self, push: &str) -> Result<Option<crate::try_import::TryPush>> {
        parse(&self.api_live("try-push", &[("push", push)], false).await?)
    }

    async fn rev_file(&self, rev: &str, path: &str) -> Result<Option<crate::format::RevFile>> {
        parse(
            &self
                .api("rev-file", None, &[("commit", rev), ("path", path)])
                .await?,
        )
    }

    async fn hyperblame_json(&self, rev: &str, path: &str, file: &str) -> Result<String> {
        let body = self
            .api(
                "hyperblame",
                None,
                &[("commit", rev), ("path", path), ("file", file)],
            )
            .await?;
        String::from_utf8(body.to_vec())
            .map_err(|err| problem(true, ErrorLayer::DataLayer, err.to_string()))
    }

    async fn stack_strip_json(&self, stack: &str, rev: &str, path: &str) -> Result<String> {
        let body = self
            .api(
                "stack-strip",
                None,
                &[("stack", stack), ("commit", rev), ("path", path)],
            )
            .await?;
        String::from_utf8(body.to_vec())
            .map_err(|err| problem(true, ErrorLayer::DataLayer, err.to_string()))
    }

    async fn history(
        &self,
        rev: &str,
        path: &str,
        scope: Option<&str>,
        token: Option<usize>,
    ) -> Result<Value> {
        let token = token.map(|token| token.to_string());
        let mut params = vec![("commit", rev), ("path", path)];
        if let Some(scope) = scope {
            params.push(("scope", scope));
        }
        if let Some(token) = &token {
            params.push(("token", token.as_str()));
        }
        parse(&self.api("history", None, &params).await?)
    }

    async fn rev_diff(&self, rev: &str, path: &str) -> Result<Option<crate::format::RevDiff>> {
        parse(
            &self
                .api("rev-diff", None, &[("commit", rev), ("path", path)])
                .await?,
        )
    }

    async fn commit_page(&self, rev: &str) -> Result<Value> {
        parse(&self.api("commit", None, &[("commit", rev)]).await?)
    }

    async fn commit_info_json(&self, revs: &str) -> Result<String> {
        let body = self.api("commit-info", None, &[("revs", revs)]).await?;
        String::from_utf8(body.to_vec())
            .map_err(|err| problem(true, ErrorLayer::DataLayer, err.to_string()))
    }

    async fn interdiff_files(&self, a: &str, b: &str) -> Result<Value> {
        parse(&self.api("interdiff", None, &[("a", a), ("b", b)]).await?)
    }

    async fn interdiff_file(
        &self,
        a: &str,
        b: &str,
        path: &str,
    ) -> Result<crate::format::InterdiffFile> {
        parse(
            &self
                .api(
                    "interdiff-file",
                    None,
                    &[("a", a), ("b", b), ("path", path)],
                )
                .await?,
        )
    }

    async fn outline_diff(
        &self,
        from: &str,
        to: &str,
        path: &str,
        all_from_lines: bool,
    ) -> Result<Value> {
        let mut params = vec![("from", from), ("to", to), ("path", path)];
        if all_from_lines {
            params.push(("all", "true"));
        }
        parse(&self.api("outline-diff", None, &params).await?)
    }

    async fn explore_keys(&self, kind: &str, keys: &str) -> Result<Value> {
        parse(
            &self
                .api("explore-keys", None, &[("kind", kind), ("keys", keys)])
                .await?,
        )
    }

    async fn map_rev(&self, kind: &str, rev: &str) -> Result<Option<String>> {
        parse(
            &self
                .api("map-rev", None, &[("kind", kind), ("from", rev)])
                .await?,
        )
    }

    async fn rev_dir(&self, rev: &str, path: &str) -> Result<Option<Value>> {
        parse(
            &self
                .api("rev-dir", None, &[("commit", rev), ("path", path)])
                .await?,
        )
    }

    async fn explore(&self, revs: &[String], stack_url: Option<&str>) -> Result<Value> {
        let revs = revs.join(",");
        let mut params = vec![("revs", revs.as_str())];
        if let Some(stack_url) = stack_url {
            params.push(("stack_url", stack_url));
        }
        parse(&self.api("explore", None, &params).await?)
    }

    fn try_overlay(&self, head: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        // (If the server has the push's analysis.)
        let info = block_on(self.tree_info_value(Some(head)))?.ok()?;
        Some(Box::new(RemoteServer {
            rev: Some(head.to_string()),
            info: Arc::new(OnceCell::new_with(Some(info))),
            ..self.clone()
        }))
    }

    fn try_delta_meta(&self) -> Option<TryDeltaMeta> {
        self.rev.as_ref()?;
        let info = block_on(self.info())?.ok()?;
        serde_json::from_value(info["try_meta"].clone()).ok()
    }

    fn old_side(&self, rev: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        // (A try push's base is the indexed revision.)
        (self.try_delta_meta()?.base == rev).then(|| {
            Box::new(RemoteServer {
                rev: None,
                info: Arc::new(OnceCell::new()),
                ..self.clone()
            }) as Box<dyn AbstractServer + Send + Sync>
        })
    }

    fn rev_changed_lines(&self, spec: &str) -> Result<ChangedLines> {
        let body = block_on(self.api("changed-lines", None, &[("spec", spec)]))
            .ok_or(ServerError::Unsupported)??;
        parse(&body)
    }
}

/// The remote trees at `urls` (separated by semicolons, ex:
/// `https://searchfox.org/firefox-disco/;https://searchfox.org/wubspecs/`), by
/// their names (their URLs' last segments), with their responses cached under
/// `cache_dir`, if given: for servers and tools working over remote trees
/// without configurations of their own (their `tree_config`s say what they
/// need of theirs).
pub fn make_remote_servers(
    urls: &str,
    cache_dir: Option<&str>,
) -> Result<std::collections::BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>> {
    let mut servers = std::collections::BTreeMap::new();
    for url in urls.split(';').map(str::trim).filter(|url| !url.is_empty()) {
        let mut url = Url::parse(url)?;
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        let tree_name = url
            .path_segments()
            .and_then(|segments| segments.filter(|s| !s.is_empty()).last())
            .map(str::to_string)
            .ok_or_else(|| problem(true, ErrorLayer::BadInput, format!("No tree in {}", url)))?;
        let server_base_url = url.join("../")?;
        servers.insert(
            tree_name.clone(),
            make_remote_server(server_base_url, &tree_name, cache_dir)?,
        );
    }
    Ok(servers)
}

/// The configuration of remote trees (see `make_remote_servers`), from their
/// servers' `config` routes (see `AbstractServer::tree_config`), for a local
/// server over them (with `mozsearch_path`, its checkout, for its static
/// files and templates): their paths point nowhere (their files are their
/// servers'), and they have no git repositories here.  Also says where their
/// pages search (see `config::search_endpoint`).
pub fn remote_trees_config(
    servers: &std::collections::BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>,
    mozsearch_path: &str,
) -> crate::file_format::config::Config {
    use crate::file_format::config::{self, TreeConfig, TreeConfigPaths};
    let mut trees = std::collections::BTreeMap::new();
    for (name, server) in servers {
        let mut settings = server
            .tree_config()
            .unwrap_or_else(|_| serde_json::json!({}));
        if let Some(settings) = settings.as_object_mut() {
            for (key, value) in settings.iter_mut() {
                if key.ends_with("_path") && value.is_string() {
                    *value = Value::String(format!("/nonexistent/remote-tree/{}/{}", name, key));
                }
            }
            settings.insert("git_path".to_string(), Value::Null);
        }
        let Ok(paths) = serde_json::from_value::<TreeConfigPaths>(settings) else {
            log::warn!("{}'s configuration isn't one", name);
            continue;
        };
        config::register_search_endpoint(name, paths.search_endpoint);
        trees.insert(name.clone(), TreeConfig { paths, git: None });
    }
    config::Config {
        tree_index_paths: trees
            .iter()
            .map(|(name, tree)| (name.clone(), tree.paths.index_path.clone()))
            .collect(),
        trees,
        mozsearch_path: mozsearch_path.to_string(),
        config_repo_path: String::new(),
        url_map_path: None,
        doc_trees_path: None,
        config_file: String::new(),
    }
}

/// The tree `tree_name` of the searchfox server at `server_base_url`, with
/// its responses cached under `cache_dir`, if given (see the module's
/// comment).
pub fn make_remote_server(
    server_base_url: Url,
    tree_name: &str,
    cache_dir: Option<&str>,
) -> Result<Box<dyn AbstractServer + Send + Sync>> {
    let tree_base_url = server_base_url.join(&format!("{}/", tree_name))?;
    let api_base_url = tree_base_url.join("api/v1/")?;
    let source_base_url = tree_base_url.join("source/")?;
    let search_url = tree_base_url.join("search")?;
    let cache = cache_dir.map(|dir| {
        let host = match server_base_url.port() {
            Some(port) => format!("{}:{}", server_base_url.host_str().unwrap_or("-"), port),
            None => server_base_url.host_str().unwrap_or("-").to_string(),
        };
        Arc::new(ResponseCache {
            dir: PathBuf::from(dir).join(host).join(tree_name),
        })
    });

    Ok(Box::new(RemoteServer {
        tree_name: tree_name.to_string(),
        api_base_url,
        source_base_url,
        search_url,
        rev: None,
        base_info: Arc::new(OnceCell::new()),
        info: Arc::new(OnceCell::new()),
        cache,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_api::fake::Fake;
    use std::collections::{BTreeMap, BTreeSet};
    use tokio_stream::StreamExt;

    /// A remote server over the data API of `data_api::fake::Fake`'s tree,
    /// served on a local port, with its responses cached under `cache`; and
    /// the API server's task.
    async fn remote(
        cache: &str,
    ) -> (
        Box<dyn AbstractServer + Send + Sync>,
        tokio::task::JoinHandle<()>,
    ) {
        let servers: crate::data_api::Servers = Arc::new(BTreeMap::from([(
            "fake".to_string(),
            Box::new(Fake) as Box<dyn AbstractServer + Send + Sync>,
        )]));
        let app = crate::data_api::router(servers, BTreeSet::from(["fake".to_string()]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (make_remote_server(url, "fake", Some(cache)).unwrap(), task)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_remote_server() {
        let cache = std::env::temp_dir().join(format!("remote-server-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&cache);
        let (server, task) = remote(cache.to_str().unwrap()).await;

        // The local tree's results, as the same types.
        let entry = server.crossref_lookup("A").await.unwrap().unwrap();
        let local = Fake.crossref_lookup("A").await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(entry).unwrap(),
            serde_json::to_value(local).unwrap()
        );
        assert!(server.crossref_lookup("Z").await.unwrap().is_none());
        assert!(server.jumpref_lookup("A").await.unwrap().is_none());
        assert_eq!(
            server.fetch_raw_source("dir/a.cpp").await.unwrap(),
            "int a;\n"
        );
        let records: Vec<Value> = server
            .fetch_raw_analysis("dir/a.cpp")
            .await
            .unwrap()
            .collect()
            .await;
        assert_eq!(
            records,
            vec![serde_json::json!({"loc": "1:4-5", "sym": "A"})]
        );
        assert_eq!(
            server
                .search_identifiers("a", true, false, 2)
                .await
                .unwrap(),
            vec![(ustr("A"), ustr("a")); 2]
        );
        assert_eq!(
            server.commit_info().unwrap().unwrap().rev,
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(
            server.tree_config().unwrap(),
            serde_json::json!({"search_endpoint": "query"})
        );
        assert_eq!(
            server.fetch_formatted_lines("dir/a.cpp").await.unwrap(),
            (vec!["int <b>a</b>;".to_string()], "{}".to_string())
        );
        assert_eq!(
            server.file_facet_info("dir/a.cpp"),
            Some((ustr("normal"), None))
        );
        assert_eq!(server.file_facet_info("dir/b.cpp"), None);
        // Errors as such.
        assert!(matches!(
            server.fetch_raw_source("dir/b.cpp").await,
            Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                ..
            }))
        ));
        assert!(matches!(
            server.search_files("(", "", false, 10).await,
            Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                ..
            }))
        ));
        assert!(server.try_overlay(&"0".repeat(40)).is_none());

        // Cached under the indexed revision, so that they're there without
        // the server.
        let host = cache.read_dir().unwrap().next().unwrap().unwrap().path();
        assert!(
            host.join("fake/0123456789abcdef0123456789abcdef01234567")
                .is_dir()
        );
        task.abort();
        let _ = task.await;
        assert_eq!(
            server.fetch_raw_source("dir/a.cpp").await.unwrap(),
            "int a;\n"
        );
        assert!(server.crossref_lookup("Z").await.unwrap().is_none());
        assert!(server.fetch_raw_source("dir/c.cpp").await.is_err());
        fs::remove_dir_all(&cache).unwrap();
    }
}
