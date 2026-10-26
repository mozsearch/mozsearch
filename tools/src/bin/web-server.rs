extern crate env_logger;
extern crate hyper;
extern crate tools;

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use hyper::http;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, StatusCode, header};

use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tools::abstract_server::{
    AbstractServer, make_local_server, make_remote_servers, remote_trees_config, sym_info,
};
use tools::admission;
use tools::commit_index::CommitIndex;
use tools::data_api;
use tools::diagnostics::diagnostics_from_config;
use tools::file_format::config;
use tools::file_format::jumpref::PrefetchedJumprefs;
use tools::file_format::per_file_info::path_facet_map;
use tools::format;
use ustr::UstrMap;

use tools::sidecars;
use tools::url_encode_path::url_decode_path;

struct WebRequest<'a> {
    path: &'a str,
    /// The query string (ex: a history page's `scope=...`), or "".
    query: &'a str,
    /// Whether it's a POST (only `/{tree}/try/{kind}/{id}`'s and
    /// `/{tree}/spec-prs/sync`'s are allowed).
    post: bool,
    /// The site's origin, as the browser has it (ex: "https://searchfox.org"),
    /// for what needs absolute URLs (an OpenSearch description); see
    /// `request_origin`.
    origin: String,
}

struct WebResponse {
    status: StatusCode,
    content_type: String,
    redirect_location: Option<String>,
    output: String,
    /// Whether it mustn't be cached (ex: the status of a try import).
    no_store: bool,
    /// Whether other sites' pages may read it (ex: a try push's canvas's
    /// model, for a service keeping notes on it).
    shared: bool,
}

impl Default for WebResponse {
    fn default() -> WebResponse {
        WebResponse {
            status: StatusCode::OK,
            content_type: "text/plain".to_owned(),
            redirect_location: None,
            output: String::new(),
            no_store: false,
            shared: false,
        }
    }
}

impl WebResponse {
    fn html(body: String) -> WebResponse {
        WebResponse {
            content_type: "text/html".to_owned(),
            output: body,
            ..WebResponse::default()
        }
    }

    fn json(body: String) -> WebResponse {
        WebResponse {
            content_type: "application/json".to_owned(),
            output: body,
            ..WebResponse::default()
        }
    }

    fn internal_error(body: String) -> WebResponse {
        WebResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            output: body,
            ..WebResponse::default()
        }
    }

    fn not_found() -> WebResponse {
        WebResponse {
            status: StatusCode::NOT_FOUND,
            output: "Not found".to_owned(),
            ..WebResponse::default()
        }
    }

    fn redirect(url: String) -> WebResponse {
        WebResponse {
            status: StatusCode::MOVED_PERMANENTLY,
            redirect_location: Some(url),
            ..WebResponse::default()
        }
    }

    /// Where to go for now (ex: by a DWIM rule, which may change).
    fn found(url: String) -> WebResponse {
        WebResponse {
            status: StatusCode::FOUND,
            redirect_location: Some(url),
            ..WebResponse::default()
        }
    }

    /// Where to go after a POST.
    fn see_other(url: String) -> WebResponse {
        WebResponse {
            status: StatusCode::SEE_OTHER,
            redirect_location: Some(url),
            ..WebResponse::default()
        }
    }

    /// A response nothing may cache (ex: a try import's status): browsers
    /// (`Cache-Control: no-store`), nor nginx (`X-Accel-Expires: 0`, which it
    /// heeds even where it ignores `Cache-Control`; see nginx-setup.py).
    fn uncached(self) -> WebResponse {
        WebResponse {
            no_store: true,
            ..self
        }
    }

    fn shared(self) -> WebResponse {
        WebResponse {
            shared: true,
            ..self
        }
    }
}

fn handle_static(path: String, content_type: Option<&str>) -> WebResponse {
    let source_file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => {
            return WebResponse::not_found();
        }
    };
    let mut reader = BufReader::new(&source_file);
    let mut input = String::new();
    match reader.read_to_string(&mut input) {
        Ok(_) => {}
        Err(_) => {
            return WebResponse::not_found();
        }
    }

    let inferred_content_type = match Path::new(&path).extension() {
        Some(ext) => match ext.to_str().unwrap() {
            "css" => "text/css",
            "js" => "text/javascript",
            _ => "text/html",
        },
        None => "text/html",
    };
    let content_type = match content_type {
        Some(ct) => ct,
        None => inferred_content_type,
    };

    WebResponse {
        content_type: content_type.to_owned(),
        output: input,
        ..WebResponse::default()
    }
}

/// Write a page's response (to a GET request at `raw_path`, without a query)
/// as a sidecar file (see `tools::sidecars`), which nginx serves from then
/// on, if it's of a route whose pages nginx looks for, and succeeded with HTML
/// or JSON and nothing a file can't have (ex: CORS headers); and return
/// whether it's there, in which case nginx mustn't cache the response too.
fn write_page_sidecar(cfg: &config::Config, raw_path: &str, response: &WebResponse) -> bool {
    if response.status != StatusCode::OK
        || response.no_store
        || response.shared
        || response.redirect_location.is_some()
    {
        return false;
    }
    let Some(suffix) = sidecars::page_suffix(&response.content_type) else {
        return false;
    };
    let path = url_decode_path(raw_path);
    let Some((tree_name, rest)) = path.strip_prefix('/').and_then(|path| path.split_once('/'))
    else {
        return false;
    };
    let route = rest.split('/').next().unwrap_or_default();
    let Some(tree_config) = cfg.trees.get(tree_name) else {
        return false;
    };
    sidecars::PAGE_ROUTES.contains(&route)
        && sidecars::write(
            &tree_config.paths.index_path,
            tree_name,
            &format!("{}{}", rest, suffix),
            &response.output,
        )
}

/// How long precomputing a try push's slicing may take (see
/// `precompute_try_push`; big pushes' take minutes).
const PRECOMPUTE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Precompute what the pages of the try push `rev`, which the tree has just
/// imported (see `tools::try_import`), show which takes long to compute, so
/// that they're ready by the time its import says it's done, if it has a
/// semantic analysis (see `try_delta`): its canvas's page (see
/// `format::format_try_canvas`), written as a sidecar file, as its first
/// request's would be (see `write_page_sidecar`), and its slicing, in the
/// formats its pages ask for (the canvas's clusters' JSON, the slicing's
/// subway map, the "Open the slicing" link's, and the lines on its changes'
/// dataflow, its excerpts'; see `format::outline_excerpts_json`), which the pipeline-server's
/// step cache keeps (see `tools::step_cache`).  (Those are also lost when the
/// servers start over with another index or binary, after which they're
/// computed again when asked for.)  What happened to each is logged.
fn precompute_try_push(cfg: &config::Config, tree_name: &str, rev: &str) {
    let started = Instant::now();
    let Some(tree_config) = cfg.trees.get(tree_name) else {
        return;
    };
    let Some(head) =
        tools::try_import::try_push(cfg, tree_name, rev).and_then(|push| push.semantic_head)
    else {
        return;
    };
    let Some(server) = tree_server(tree_name) else {
        return;
    };
    let Some(base) = server
        .try_overlay(&head)
        .and_then(|overlay| overlay.try_delta_meta())
        .map(|meta| meta.base)
    else {
        return;
    };

    // (What happened to each, for the log.)
    let mut outcomes = vec![];
    let mut html = Vec::new();
    match format::format_try_canvas(server.as_ref(), tree_name, &head, &mut html) {
        Ok(()) => {
            let written = sidecars::write(
                &tree_config.paths.index_path,
                tree_name,
                &format!("rev-canvas/{}.page.html", head),
                &String::from_utf8_lossy(&html),
            );
            outcomes.push(format!(
                "canvas {}",
                if written { "written" } else { "not written" }
            ));
        }
        Err(err) => outcomes.push(format!("canvas failed: {}", err)),
    }

    // (The pipeline-server's, as its `PIPELINE_SERVER_ADDR` says, here.)
    let addr = env::var("PIPELINE_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8002".to_string());
    let addr = addr.replace("0.0.0.0", "127.0.0.1");
    let slicing = format!("slice-rev:{}..{}", base, head);
    let queries = [
        (
            "json",
            format!("{} slice-format:json", slicing),
            "application/json",
        ),
        ("subway", slicing.clone(), "text/html"),
        (
            "lines",
            format!("{} slice-format:lines", slicing),
            "application/json",
        ),
    ];
    let client = reqwest::Client::new();
    for (name, query, accept) in queries {
        let url = format!(
            "http://{}/{}/query/default?q={}&rev={}",
            addr,
            tree_name,
            urlencoding::encode(&query),
            head
        );
        let request = client
            .get(&url)
            .header("Accept", accept)
            .timeout(PRECOMPUTE_TIMEOUT)
            .send();
        let query_started = Instant::now();
        // (With its body, which is what takes long to compute.)
        let outcome = tokio::runtime::Handle::current().block_on(async {
            let response = request.await?;
            let status = response.status();
            response.bytes().await.map(|_| status)
        });
        outcomes.push(match outcome {
            Ok(status) => format!(
                "{} {} ({:.1} s)",
                name,
                status.as_u16(),
                query_started.elapsed().as_secs_f64()
            ),
            Err(err) => format!("{} failed: {}", name, err),
        });
    }
    eprintln!(
        "Precomputed try push {}'s canvas and slicing in {:.1} s: {}",
        rev,
        started.elapsed().as_secs_f64(),
        outcomes.join(", ")
    );
}

/// The trees' servers made so far (see `tree_server`), or, for a local server
/// over remote trees, theirs (see `main`).
static SERVERS: LazyLock<Mutex<HashMap<String, Arc<dyn AbstractServer + Send + Sync>>>> =
    LazyLock::new(Default::default);

/// The tree's server (see `tools::abstract_server`), for the pages built over
/// one rather than over the index's files (ex: try pushes' canvases), made
/// when first needed: its local index (whose big parts load as they're
/// needed).
fn tree_server(tree_name: &str) -> Option<Arc<dyn AbstractServer + Send + Sync>> {
    let mut servers = SERVERS.lock().unwrap();
    if let Some(server) = servers.get(tree_name) {
        return Some(server.clone());
    }
    let server: Arc<dyn AbstractServer + Send + Sync> =
        Arc::from(make_local_server(&env::args().nth(1)?, tree_name).ok()?);
    servers.insert(tree_name.to_string(), server.clone());
    Some(server)
}

/// The page of the file at `path` in the revision `rev` over the tree's
/// server, with the jumprefs of its analysis's symbols looked up ahead of
/// time (see `format::render_rev_file`), if it's a file there.
fn rev_file_page(
    cfg: &config::Config,
    tree_name: &str,
    server: &(dyn AbstractServer + Send + Sync),
    rev: &str,
    path: &str,
) -> Result<Option<WebResponse>, tools::abstract_server::ServerError> {
    let (server, rev_, path_) = (server.clonify(), rev.to_string(), path.to_string());
    let file = format::run_on_runtime(async move {
        let Some(file) = server.rev_file(&rev_, &path_).await? else {
            return Ok(None);
        };
        let mut jumprefs = BTreeMap::new();
        if !file.analysis.is_empty() {
            // (A try push's rebased head's are the push's.)
            let source = server
                .try_overlay(&file.rev)
                .unwrap_or_else(|| server.clonify());
            let mut traversed = UstrMap::default();
            let syms = format::page_symbols(&path_, &file.analysis);
            sym_info::add_syms_info(source.as_ref(), syms, &mut jumprefs, &mut traversed).await;
        }
        Ok::<_, tools::abstract_server::ServerError>(Some((file, jumprefs)))
    })?;
    let Some((file, jumprefs)) = file else {
        return Ok(None);
    };
    let mut writer = Vec::new();
    Ok(Some(
        match format::render_rev_file(
            cfg,
            tree_name,
            path,
            file,
            &PrefetchedJumprefs(jumprefs),
            &mut writer,
        ) {
            Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
            Err(err) => WebResponse::internal_error(err.to_owned()),
        },
    ))
}

/// A redirect to the tree's `page` (ex: "rev", with `rest` after the revision)
/// for the revision `rev` of another kind (see `AbstractServer::map_rev`).
fn redirect_mapped_rev(
    tree_name: &str,
    kind: &str,
    rev: &str,
    page: &str,
    rest: &str,
) -> WebResponse {
    let Some(server) = tree_server(tree_name) else {
        return WebResponse::not_found();
    };
    let (server, kind_, rev_) = (server.clonify(), kind.to_string(), rev.to_string());
    match format::run_on_runtime(async move { server.map_rev(&kind_, &rev_).await }) {
        Ok(Some(mapped)) => WebResponse::redirect(match rest {
            "" => format!("/{}/{}/{}", tree_name, page, mapped),
            rest => format!("/{}/{}/{}/{}", tree_name, page, mapped, rest),
        }),
        Ok(None) => WebResponse::not_found(),
        Err(err) => WebResponse::internal_error(server_error_message(&err)),
    }
}

/// What a server's error says, for a page (the message of its reasons, ex:
/// "Bad revision").
fn server_error_message(err: &tools::abstract_server::ServerError) -> String {
    use tools::abstract_server::ServerError;
    match err {
        ServerError::StickyProblem(details) | ServerError::TransientProblem(details) => {
            details.message.clone()
        }
        ServerError::Unsupported => "Not available for this tree".to_string(),
        ServerError::TooExpensive(refused) => refused.message(),
    }
}

/// A request's site's origin: its host (nginx's X-Forwarded-Host, else its
/// Host) and its scheme (the load balancer's X-Forwarded-Proto, else http), if
/// it's a host's.
fn request_origin(headers: &http::HeaderMap) -> String {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let host = header("x-forwarded-host")
        .or(header("host"))
        .filter(|host| {
            host.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-.:[]".contains(c))
        })
        .unwrap_or("localhost");
    let scheme = match header("x-forwarded-proto") {
        Some("https") => "https",
        _ => "http",
    };
    format!("{}://{}", scheme, host)
}

/// The tree's OpenSearch description (`/{tree}/opensearch.xml`, which its
/// pages link to; see `output::opensearch_link`): searching with `/search`
/// (router.py's, or a tree searching with queries' normalization into them;
/// see the `search` route), with suggestions of identifiers (see the
/// `suggest` route).  (Its URLs are forever: Firefox doesn't update them.)
/// (Firefox wants absolute URLs, and the link's title for the name.)
fn opensearch_description(tree_name: &str, origin: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/" xmlns:moz="http://www.mozilla.org/2006/browser/search/">
  <ShortName>Searchfox ({tree})</ShortName>
  <Description>Search {tree} with Searchfox</Description>
  <InputEncoding>UTF-8</InputEncoding>
  <Image width="52" height="52" type="image/png">{origin}/{tree}/static/icons/search.png</Image>
  <Url type="text/html" method="get" template="{origin}/{tree}/search?q={{searchTerms}}"/>
  <Url type="application/x-suggestions+json" method="get" template="{origin}/{tree}/suggest?q={{searchTerms}}"/>
  <Url type="application/opensearchdescription+xml" rel="self" template="{origin}/{tree}/opensearch.xml"/>
  <moz:SearchForm>{origin}/{tree}/source/</moz:SearchForm>
</OpenSearchDescription>
"#,
        tree = tree_name,
        origin = origin,
    )
}

/// A request's query string's parameters.
fn query_params(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect()
}

/// The tree's query page for the query (ex: normalized), and what it was
/// before, if it's different (which the page shows; see pipeline-server.rs),
/// in a try push's context (`rev`).
fn query_url(tree_name: &str, query: &str, input: &str, rev: Option<&str>) -> String {
    let mut url = url::form_urlencoded::Serializer::new(String::new());
    url.append_pair("q", query);
    if input.trim() != query {
        url.append_pair("input", input);
    }
    if let Some(rev) = rev.filter(|rev| !rev.is_empty()) {
        url.append_pair("rev", rev);
    }
    format!("/{}/query/default?{}", tree_name, url.finish())
}

/// The trees' servers for the data API's routes the web-server serves (see
/// `handle`), if the tree's API is on.
struct WebApiServers {
    enabled: bool,
}

impl data_api::ApiServers for WebApiServers {
    fn server(&self, tree: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        self.enabled
            .then(|| tree_server(tree))
            .flatten()
            .map(|server| server.clonify())
    }
}

fn handle(cfg: &config::Config, req: WebRequest) -> WebResponse {
    let path = url_decode_path(req.path);
    let path = path[1..].split('/').collect::<Vec<_>>();

    if !path.is_empty() && path[0] == "static" {
        let path = cfg.mozsearch_path.clone() + req.path;
        return handle_static(path, None);
    }

    // The data API's routes for try pushes' imports, which this server
    // performs (see `data_api::Host`); nginx routes its others to the
    // pipeline-server.
    if path.len() >= 4 && path[1] == "api" {
        let path_and_query = match req.query {
            "" => req.path.to_string(),
            query => format!("{}?{}", req.path, query),
        };
        let enabled = cfg
            .trees
            .get(path[0])
            .is_some_and(|tree| tree.paths.data_api);
        let post = req.post;
        let (status, content_type, body) = format::run_on_runtime(async move {
            data_api::respond_parts(
                &WebApiServers { enabled },
                data_api::Host::WebServer,
                post,
                &path_and_query,
            )
            .await
        });
        return WebResponse {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            content_type: content_type.to_string(),
            output: String::from_utf8_lossy(&body).into_owned(),
            ..WebResponse::default()
        }
        .uncached()
        .shared();
    }

    if path.len() < 2 {
        return WebResponse::not_found();
    }

    let tree_name = &path[0];
    let kind = &path[1];

    // Upstream revisions of a forest of specs' repos (ex: from a permalink to
    // a spec's own repository) are their forest commits'; see
    // `format::forest_rev`.  (A path in another spec than the revision's is an
    // error, rather than that spec's file at the time.)
    if matches!(&kind[..], "rev" | "rev-rendered" | "diff" | "commit")
        && path.len() >= 3
        && let Some((forest_rev, spec)) = format::forest_rev(cfg, tree_name, path[2])
    {
        let file_path = path[3..].join("/");
        if !file_path.is_empty() && file_path.split('/').next() != Some(spec.as_str()) {
            return WebResponse::internal_error(format!(
                "{} is a revision of {}, not of {}",
                path[2], spec, file_path
            ));
        }
        return WebResponse::redirect(format!(
            "/{}/{}/{}/{}",
            tree_name, kind, forest_rev, file_path
        ));
    }

    println!("DBG {:?} {} {}", path, tree_name, kind);

    // A tree's try pushes' pages are the tree's that imports them, if it
    // doesn't (see `try_import::importing_tree`).
    if matches!(&kind[..], "try" | "try-status")
        && let Some(other) = tools::try_import::importing_tree(cfg, tree_name)
    {
        return WebResponse::see_other(format!("/{}/{}/{}", other, kind, path[2..].join("/")))
            .uncached();
    }

    match &kind[..] {
        "diagnostics" => match serde_json::to_string(&diagnostics_from_config(cfg, tree_name)) {
            Ok(s) => WebResponse::json(s),
            Err(err) => WebResponse::internal_error(err.to_string()),
        },

        "rev" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            // (A revision another tree has, ex: crash-stats' links to
            // firefox-main for revisions only on beta.)
            if let Some(other_tree) = format::tree_for_rev(cfg, tree_name, rev) {
                return WebResponse::redirect(format!("/{}/rev/{}/{}", other_tree, rev, path));
            }

            // A permalink to a revision at which the file is the same as at
            // the indexed revision goes to its source listing (with its
            // semantic information), which says so (see file-timeline.js's
            // PermalinkNote), unless it's for exactly that revision
            // (`?exactly`).
            let exactly =
                url::form_urlencoded::parse(req.query.as_bytes()).any(|(key, _)| key == "exactly");
            if !exactly && format::same_as_head(cfg, tree_name, rev, &path) {
                return WebResponse::redirect(format!(
                    "/{}/source/{}?permalink={}",
                    tree_name, path, rev
                ));
            }

            let mut writer = Vec::new();
            // (Directories' listings over the tree's server; see
            // `tree_server`.)
            if let Some(server) = tree_server(tree_name) {
                let (dir_server, rev_, path_) = (server.clonify(), rev.to_string(), path.clone());
                match format::run_on_runtime(async move { dir_server.rev_dir(&rev_, &path_).await })
                {
                    Ok(Some(data)) => {
                        return match format::render_dir_listing(
                            tree_name,
                            rev,
                            &path,
                            data,
                            &mut writer,
                        ) {
                            Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                            Err(err) => WebResponse::internal_error(err.to_owned()),
                        };
                    }
                    Ok(None) => match rev_file_page(cfg, tree_name, server.as_ref(), rev, &path) {
                        Ok(Some(response)) => return response,
                        Ok(None) => {}
                        Err(_) if cfg.trees.get(*tree_name).is_some_and(|t| t.git.is_some()) => {}
                        Err(err) => return WebResponse::internal_error(server_error_message(&err)),
                    },
                    Err(_) if cfg.trees.get(*tree_name).is_some_and(|t| t.git.is_some()) => {}
                    Err(err) => return WebResponse::internal_error(server_error_message(&err)),
                }
            }
            match format::format_path(cfg, tree_name, rev, &path, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // The hyperblame data files (`commits.json` and `lines-K.json`) of the
        // token-centric blame of `/rev/` pages, as
        // `/{tree}/rev-hyperblame/{rev}/{path}/{file}`.  The tip's are static
        // files; see `format::hyperblame_files`.
        "rev-hyperblame" => {
            if path.len() < 5 {
                return WebResponse::not_found();
            }

            let rev = path[2];

            // Peephole histories (see `format::peephole_json`) as
            // `/{tree}/rev-hyperblame/{rev}/{path}/peephole/{tokens}.json` for
            // comma-separated token indices, futures (see `format::future_json`)
            // as `.../future/{token}.json`, and where tokens were before they
            // were introduced (see `format::before_json`) as
            // `.../before/{token}.json`; and the data files (`commits.json`
            // and `lines-K.json`); over the tree's server (see
            // `format::hyperblame_json`).
            let (file_path, file) = if path.len() >= 6
                && matches!(path[path.len() - 2], "peephole" | "future" | "before")
            {
                (
                    path[3..path.len() - 2].join("/"),
                    format!("{}/{}", path[path.len() - 2], path[path.len() - 1]),
                )
            } else {
                (
                    path[3..path.len() - 1].join("/"),
                    path[path.len() - 1].to_string(),
                )
            };
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, rev) = (server.clonify(), rev.to_string());
            match format::run_on_runtime(async move {
                server.hyperblame_json(&rev, &file_path, &file).await
            }) {
                Ok(json) => WebResponse::json(json),
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        // How a file's outline changed between two revisions, as
        // `/{tree}/rev-outline/{from}/{to}/{path}`, where `from` may be
        // `since:YYYY-MM-DD`; see `format::outline_diff_data`.
        "rev-outline" => {
            if path.len() < 5 {
                return WebResponse::not_found();
            }
            let file_path = path[4..].join("/");
            // (Over the tree's server.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, from, to) = (server.clonify(), path[2].to_string(), path[3].to_string());
            match format::run_on_runtime(async move {
                server.outline_diff(&from, &to, &file_path, false).await
            }) {
                Ok(diff) => WebResponse::json(diff.to_string()),
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        // The page of how a file's outline changed between two revisions, as
        // `/{tree}/outline-diff/{from}/{to}/{path}`; see
        // `format::render_outline_diff`.
        "outline-diff" => {
            if path.len() < 5 {
                return WebResponse::not_found();
            }
            let file_path = path[4..].join("/");
            // (Over the tree's server.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server_, from, to, path_) = (
                server.clonify(),
                path[2].to_string(),
                path[3].to_string(),
                file_path.clone(),
            );
            let diff = format::run_on_runtime(async move {
                server_.outline_diff(&from, &to, &path_, true).await
            })
            .map_err(|err| server_error_message(&err))
            .and_then(|diff| serde_json::from_value(diff).map_err(|err| err.to_string()));
            let mut writer = Vec::new();
            match diff.and_then(|diff| {
                format::render_outline_diff(tree_name, &file_path, diff, &mut writer)
                    .map_err(str::to_string)
            }) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err),
            }
        }

        // Rendered views of documents, at the tip as `/{tree}/rendered/{path}`
        // and at other revisions as `/{tree}/rev-rendered/{rev}/{path}`; see
        // `format::format_rendered`.
        "rendered" | "rev-rendered" => {
            let (rev, file_path) = if *kind == "rendered" {
                (None, path[2..].join("/"))
            } else {
                if path.len() < 4 {
                    return WebResponse::not_found();
                }
                (Some(path[2]), path[3..].join("/"))
            };
            if let Some(rev) = rev
                && let Some(other_tree) = format::tree_for_rev(cfg, tree_name, rev)
            {
                return WebResponse::redirect(format!(
                    "/{}/rev-rendered/{}/{}",
                    other_tree, rev, file_path
                ));
            }

            // (Links to specs' anchors and lines go to the pages with them,
            // with their queries, ex: what a search going to a spec's URL's
            // anchor was for; see `dwim::searched_url`.)
            if let Some(url) = format::rendered_redirect(cfg, tree_name, rev, &file_path) {
                let url = match (req.query, url.split_once('#')) {
                    ("", _) => url,
                    (query, Some((url, fragment))) => format!("{}?{}#{}", url, query, fragment),
                    (query, None) => format!("{}?{}", url, query),
                };
                return WebResponse::redirect(url);
            }

            let mut writer = Vec::new();
            match format::format_rendered(cfg, tree_name, rev, &file_path, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // The view of a spec's algorithm beside the code citing it and its
        // steps, as `/{tree}/algorithm/{sym}` (in the spec's tree, which a
        // linked tree's redirects to); see `format::format_algorithm`.
        "algorithm" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }
            let sym = path[2..].join("/");
            if let Some(url) = format::algorithm_redirect(cfg, tree_name, &sym) {
                return WebResponse::redirect(url);
            }
            let mut writer = Vec::new();
            match format::format_algorithm(cfg, tree_name, &sym, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // A try push's canvas: the functions it adds or changes and the calls
        // among them, at the head of its rebased stack, as
        // `/{tree}/rev-canvas/{head}`, and its model, as
        // `/{tree}/rev-canvas/{head}/canvas.json`, which other sites' pages
        // can read (ex: a service keeping notes on it); see
        // `format::format_try_canvas`.
        "rev-canvas" => {
            // (Over the tree's server; see `tree_server`.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            match path.len() {
                3 => {
                    let mut writer = Vec::new();
                    match format::format_try_canvas(
                        server.as_ref(),
                        tree_name,
                        path[2],
                        &mut writer,
                    ) {
                        Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                        Err(err) => WebResponse::internal_error(err.to_owned()),
                    }
                }
                4 if path[3] == "canvas.json" => {
                    match format::try_canvas_json(server.as_ref(), tree_name, path[2]) {
                        Ok(json) => WebResponse::json(json).shared(),
                        Err(err) => WebResponse::internal_error(err.to_owned()),
                    }
                }
                _ => WebResponse::not_found(),
            }
        }

        // The view of a spec's algorithm in a try push's context, beside the
        // code citing it at the head of the push's rebased stack, as
        // `/{tree}/rev-algorithm/{head}/{sym}`, compared with the spec tree's
        // view's code or, with `?versus={head}`, another push's; see
        // `format::format_try_algorithm`.
        "rev-algorithm" => {
            if path.len() < 4 {
                return WebResponse::not_found();
            }
            let sym = path[3..].join("/");
            let versus = url::form_urlencoded::parse(req.query.as_bytes())
                .find(|(key, _)| key == "versus")
                .map(|(_, value)| value.into_owned())
                .filter(|versus| !versus.is_empty());
            let mut writer = Vec::new();
            match format::format_try_algorithm(
                cfg,
                tree_name,
                path[2],
                &sym,
                versus.as_deref(),
                &mut writer,
            ) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "hgrev" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            redirect_mapped_rev(tree_name, "hg", path[2], "rev", &path[3..].join("/"))
        }

        "oldrev" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            redirect_mapped_rev(tree_name, "old", path[2], "rev", &path[3..].join("/"))
        }

        "source" => {
            let path = path.clone().split_off(2);
            let path = path.join("/");

            // (Usually nginx's; over the tree's server.  A file's listing, or
            // else a directory's, as nginx's `try_files` does.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let server = server.clonify();
            match format::run_on_runtime(async move {
                use tools::abstract_server::HtmlFileRoot;
                match server.fetch_html(HtmlFileRoot::FormattedFile, &path).await {
                    Ok(html) => Ok(html),
                    Err(_) => server.fetch_html(HtmlFileRoot::FormattedDir, &path).await,
                }
            }) {
                Ok(html) => WebResponse::html(html),
                Err(_) => WebResponse::not_found(),
            }
        }

        "diff" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            // (Over the tree's server; a revision which doesn't change the
            // file shows the file.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server_, rev_, path_) = (server.clonify(), rev.to_string(), path.clone());
            match format::run_on_runtime(async move { server_.rev_diff(&rev_, &path_).await }) {
                Ok(Some(diff)) => {
                    let mut writer = Vec::new();
                    match format::render_diff(cfg, tree_name, rev, &path, diff, &mut writer) {
                        Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                        Err(err) => WebResponse::internal_error(err.to_owned()),
                    }
                }
                Ok(None) => match rev_file_page(cfg, tree_name, server.as_ref(), rev, &path) {
                    Ok(Some(response)) => response,
                    Ok(None) => WebResponse::internal_error("Invalid path".to_owned()),
                    Err(err) => WebResponse::internal_error(server_error_message(&err)),
                },
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        // `/{tree}/interdiff/{A}/{B}` lists the files either side changed, and
        // `/{tree}/interdiff/{A}/{B}/{path}` is a file's interdiff, where each
        // side is comma-separated revisions; see `format::interdiff_file_data`.
        "interdiff" => {
            if path.len() < 4 {
                return WebResponse::not_found();
            }

            // (Over the tree's server.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, a, b) = (server.clonify(), path[2].to_string(), path[3].to_string());
            let mut writer = Vec::new();
            let result = if path.len() == 4 {
                format::run_on_runtime(async move { server.interdiff_files(&a, &b).await })
                    .map_err(|err| server_error_message(&err))
                    .and_then(|data| {
                        format::render_interdiff_files(tree_name, data, &mut writer)
                            .map_err(str::to_string)
                    })
            } else {
                let file_path = path[4..].join("/");
                let path_ = file_path.clone();
                format::run_on_runtime(async move { server.interdiff_file(&a, &b, &path_).await })
                    .map_err(|err| server_error_message(&err))
                    .and_then(|data| {
                        format::render_interdiff_file(cfg, tree_name, &file_path, data, &mut writer)
                            .map_err(str::to_string)
                    })
            };
            match result {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err),
            }
        }

        "olddiff" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            redirect_mapped_rev(tree_name, "old", path[2], "diff", &path[3..].join("/"))
        }

        // `/{tree}/explore/bug/{bugs}` and `/{tree}/explore/phab/{revs}`; see
        // `format::explore_page_data`.  And their files' patch stack strips, as
        // `.../stack-strip/{rev}/{path}`; see `format::stack_strip_json`.
        "explore" => {
            // (Over the tree's server.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            if path.len() >= 7 && path[4] == "stack-strip" {
                let stack = path[1..4].join("/");
                let file_path = path[6..].join("/");
                let (server, rev) = (server.clonify(), path[5].to_string());
                return match format::run_on_runtime(async move {
                    server.stack_strip_json(&stack, &rev, &file_path).await
                }) {
                    Ok(json) => WebResponse::json(json),
                    Err(err) => WebResponse::internal_error(server_error_message(&err)),
                };
            }
            // (Its files' excerpts, `.../excerpts/{from}/{to}/{path}`; see
            // `format::outline_excerpts_json`.)
            if path.len() >= 8 && path[4] == "excerpts" {
                let stack = path[1..4].join("/");
                let file_path = path[7..].join("/");
                return match format::outline_excerpts_json(
                    server.as_ref(),
                    tree_name,
                    &stack,
                    path[5],
                    path[6],
                    &file_path,
                ) {
                    Ok(json) => WebResponse::json(json),
                    Err(err) => WebResponse::internal_error(err.to_owned()),
                };
            }
            if path.len() != 4 {
                return WebResponse::not_found();
            }

            let (server_, kind, keys) =
                (server.clonify(), path[2].to_string(), path[3].to_string());
            match format::run_on_runtime(async move { server_.explore_keys(&kind, &keys).await }) {
                Ok(data) => {
                    let mut writer = Vec::new();
                    match format::render_explore_page(server.as_ref(), tree_name, data, &mut writer)
                    {
                        Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                        Err(err) => WebResponse::internal_error(err.to_owned()),
                    }
                }
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        "commit" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = path[2];

            // (Over the tree's server.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, rev_) = (server.clonify(), rev.to_string());
            match format::run_on_runtime(async move { server.commit_page(&rev_).await }) {
                Ok(data) => {
                    let mut writer = Vec::new();
                    match format::render_commit(tree_name, rev, data, &mut writer) {
                        Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                        Err(err) => WebResponse::internal_error(err.to_owned()),
                    }
                }
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        "oldcommit" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            redirect_mapped_rev(tree_name, "old", path[2], "commit", "")
        }

        // We don't have an "oldcommit-info" because this endpoint is only for
        // AJAX-y use by the current blame strip UI that fetches commit-info on
        // demand, and these links are considered mozsearch-internal and will
        // never be generated for anything but a current revision.
        "commit-info" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            // (Over the tree's server.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, revs) = (server.clonify(), path[2].to_string());
            match format::run_on_runtime(async move { server.commit_info_json(&revs).await }) {
                Ok(json) => WebResponse::json(json),
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        // A try push's page (`/{tree}/try/{kind}/{id}`, ex: `try/hg/{rev}`; see
        // `tools::try_import::url_push`), or (POST) asking for its import, and
        // its import's status; see `tools::try_import`.  And its files' patch
        // stack strips, as `.../stack-strip/{rev}/{path}`; see
        // `format::stack_strip_json`.
        "try" => {
            if path.len() >= 7 && path[4] == "stack-strip" {
                let stack = path[1..4].join("/");
                let file_path = path[6..].join("/");
                // (Over the tree's server; see `tree_server`.)
                let Some(server) = tree_server(tree_name) else {
                    return WebResponse::not_found();
                };
                let (server, rev) = (server.clonify(), path[5].to_string());
                // (Cached once it's there: an imported push doesn't change.)
                return match format::run_on_runtime(async move {
                    server.stack_strip_json(&stack, &rev, &file_path).await
                }) {
                    Ok(json) => WebResponse::json(json),
                    Err(err) => WebResponse::internal_error(server_error_message(&err)).uncached(),
                };
            }
            // (And its files' excerpts; see `format::outline_excerpts_json`.)
            if path.len() >= 8 && path[4] == "excerpts" {
                let stack = path[1..4].join("/");
                let file_path = path[7..].join("/");
                let Some(server) = tree_server(tree_name) else {
                    return WebResponse::not_found();
                };
                return match format::outline_excerpts_json(
                    server.as_ref(),
                    tree_name,
                    &stack,
                    path[5],
                    path[6],
                    &file_path,
                ) {
                    Ok(json) => WebResponse::json(json),
                    Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
                };
            }
            let push = match path.len() {
                4 => tools::try_import::url_push(cfg, tree_name, path[2], path[3]),
                _ => None,
            };
            let Some(push) = push else {
                return WebResponse::not_found();
            };
            // (Over the tree's server; see `tree_server`.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let mut notice = None;
            if req.post {
                match format::request_try_import(server.as_ref(), &push) {
                    Ok(()) => {
                        // (A pull request's page may be cached as imported,
                        // before this import of its newer commits.)
                        return WebResponse::see_other(format!(
                            "/{}/{}{}",
                            tree_name,
                            tools::try_import::push_url_path(&push),
                            if tools::try_import::is_pull_request(&push) {
                                "?refresh"
                            } else {
                                ""
                            }
                        ))
                        .uncached();
                    }
                    Err(err) => notice = Some(err),
                }
            }
            let mut writer = Vec::new();
            match format::format_try(
                cfg,
                server.as_ref(),
                tree_name,
                &push,
                notice.as_deref(),
                &mut writer,
            ) {
                // (The imported push's page doesn't change, but its import's
                // status's and a POST's do.)
                Ok(true) if notice.is_none() => {
                    WebResponse::html(String::from_utf8(writer).unwrap())
                }
                Ok(_) => WebResponse::html(String::from_utf8(writer).unwrap()).uncached(),
                Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
            }
        }

        // A scope's history; see `format::history_data`.
        "history" => {
            if path.len() < 4 {
                return WebResponse::not_found();
            }
            let file_path = path[3..].join("/");
            // (Its scope, or a token whose context is the scope; over the
            // tree's server.)
            let params: HashMap<String, String> = url::form_urlencoded::parse(req.query.as_bytes())
                .into_owned()
                .collect();
            let scope = params.get("scope").cloned();
            let token = params
                .get("token")
                .and_then(|token| token.parse::<usize>().ok());
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, rev) = (server.clonify(), path[2].to_string());
            match format::run_on_runtime(async move {
                server
                    .history(&rev, &file_path, scope.as_deref(), token)
                    .await
            }) {
                Ok(data) => {
                    let mut writer = Vec::new();
                    match format::render_history(tree_name, data, &mut writer) {
                        Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                        Err(err) => WebResponse::internal_error(err.to_owned()),
                    }
                }
                Err(err) => WebResponse::internal_error(server_error_message(&err)),
            }
        }

        // (`/{tree}/try-status/{kind}/{id}`, as for `try`.)
        "try-status" => {
            let push = match path.len() {
                4 => tools::try_import::url_push(cfg, tree_name, path[2], path[3]),
                _ => None,
            };
            let Some(push) = push else {
                return WebResponse::not_found();
            };
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            match format::format_try_status(server.as_ref(), &push) {
                Ok(json) => WebResponse::json(json).uncached(),
                Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
            }
        }

        // A tree of specs' summaries of its spec repositories' open pull
        // requests (`/{tree}/spec-prs`, and a repository's at
        // `/{tree}/spec-prs/{spec}`), and (POST) asking for a sync of them
        // (`/{tree}/spec-prs/sync`, with `?spec=` for a repository's), and
        // its status (`/{tree}/spec-prs/sync-status`); see `tools::spec_prs`.
        // (Uncached: syncs change them.)
        "spec-prs" => {
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let params = query_params(req.query);
            let spec = match path.get(2).copied().unwrap_or_default() {
                "sync" | "sync-status" => params.get("spec").map(String::as_str),
                spec => Some(spec).filter(|spec| !spec.is_empty()),
            };
            // (Of the tree's spec repositories, if it knows them; see
            // `try_import::spec_repos`.)
            let repos = tools::try_import::spec_repos(cfg, tree_name);
            if path.len() > 3
                || spec.is_some_and(|spec| {
                    !tools::spec_prs::valid_spec(spec)
                        || (!repos.is_empty() && !repos.iter().any(|repo| repo.name == spec))
                })
            {
                return WebResponse::not_found();
            }
            let page = |notice: Option<&str>| {
                let mut writer = Vec::new();
                match format::format_spec_prs(server.as_ref(), tree_name, spec, notice, &mut writer)
                {
                    Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()).uncached(),
                    Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
                }
            };
            match (path.get(2).copied(), req.post) {
                (Some("sync-status"), false) => {
                    match format::format_spec_prs_sync_status(server.as_ref(), spec) {
                        Ok(json) => WebResponse::json(json).uncached(),
                        Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
                    }
                }
                (Some("sync"), true) => {
                    match format::request_spec_prs_sync(server.as_ref(), spec) {
                        Ok(()) => WebResponse::see_other(match spec {
                            Some(spec) => format!("/{}/spec-prs/{}", tree_name, spec),
                            None => format!("/{}/spec-prs", tree_name),
                        })
                        .uncached(),
                        Err(err) => page(Some(&err)),
                    }
                }
                (Some("sync" | "sync-status"), _) | (_, true) => WebResponse::not_found(),
                _ => page(None),
            }
        }

        // What a person searched for (from a search box, a keyword or
        // OpenSearch engine, or router.py's search's URLs, with its form's
        // fields), in a tree searching with queries (nginx sends the requests
        // for JSON, and the other trees', to router.py): its query (see
        // `tools::query::normalize`), whose page (with the input) also says
        // where a DWIM rule says it may be (see pipeline-server.rs), in a try
        // push's context if it was (`rev`).
        "search" if config::search_endpoint(tree_name) == config::SearchEndpoint::Query => {
            let params = query_params(req.query);
            let param = |name: &str| params.get(name).map(String::as_str);
            let input = param("q").unwrap_or_default();
            let form = tools::query::normalize::SearchForm {
                path: param("path"),
                case: param("case") == Some("true"),
                regexp: param("regexp") == Some("true"),
            };
            let normalization = tools::query::normalize::normalize_search(input, &form);
            // (A DWIM rule's target which is where to go at once, without
            // searching, saying where the search is, for its page to offer;
            // see `dwim::DwimTarget::always`.)
            let has_path = |path: &str| {
                tree_server(tree_name).is_some_and(|server| server.file_facet_info(path).is_some())
            };
            let lookup = |kind: &str, capture: &dyn Fn(&str) -> Option<String>| match kind {
                "spec" => format::spec_url_target(
                    cfg,
                    tree_name,
                    &capture("url")?,
                    capture("anchor").as_deref(),
                ),
                _ => None,
            };
            if let Some(target) =
                tools::query::dwim::dwim_target(tree_name, input, &has_path, &lookup)
                    .filter(|target| target.always)
            {
                let search = query_url(
                    tree_name,
                    &normalization.query,
                    &normalization.query,
                    param("rev"),
                );
                return WebResponse::found(tools::query::dwim::searched_url(
                    &target.url,
                    input,
                    &search,
                ));
            }
            WebResponse::found(query_url(
                tree_name,
                &normalization.query,
                input,
                param("rev"),
            ))
        }

        "opensearch.xml" if cfg.trees.contains_key(*tree_name) => WebResponse {
            content_type: "application/opensearchdescription+xml".to_string(),
            output: opensearch_description(tree_name, &req.origin),
            ..WebResponse::default()
        }
        .uncached(),

        // An OpenSearch engine's suggestions (see `opensearch_description`):
        // the identifiers the input's the prefix of (see `dwim::suggestion`),
        // as Firefox has them: `[input, [suggestion, ...]]`.  (Only a tree
        // searching with queries trims the spaces after those that look like
        // URLs to Firefox's address bar; router.py would search for them.)
        "suggest" => {
            let params = query_params(req.query);
            let input = params.get("q").cloned().unwrap_or_default();
            let mut suggestions: Vec<String> = vec![];
            if let Some((before, needle)) = tools::query::dwim::suggestion_needle(&input)
                && let Some(server) = tree_server(tree_name)
            {
                let (server, needle) = (server.clonify(), needle.to_string());
                let ids = format::run_on_runtime(async move {
                    server.search_identifiers(&needle, false, true, 50).await
                })
                .unwrap_or_default();
                let query_tree =
                    config::search_endpoint(tree_name) == config::SearchEndpoint::Query;
                for (_, id) in ids {
                    let suggestion = match query_tree {
                        true => tools::query::dwim::suggestion(before, &id),
                        false => format!("{}{}", before, id),
                    };
                    if !suggestions.contains(&suggestion) {
                        suggestions.push(suggestion);
                    }
                    if suggestions.len() == 10 {
                        break;
                    }
                }
            }
            WebResponse {
                content_type: "application/x-suggestions+json".to_string(),
                output: serde_json::json!([input, suggestions]).to_string(),
                ..WebResponse::default()
            }
        }

        "complete" => {
            // (Over the tree's server, as `IdentMap::lookup_json` would give
            // them.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let (server, prefix) = (server.clonify(), path[2].to_string());
            match format::run_on_runtime(async move {
                server.search_identifiers(&prefix, false, false, 6).await
            }) {
                Ok(ids) => {
                    let ids: Vec<serde_json::Value> = ids
                        .into_iter()
                        .map(|(symbol, id)| serde_json::json!({ "id": id, "symbol": symbol }))
                        .collect();
                    WebResponse::json(serde_json::Value::Array(ids).to_string())
                }
                Err(_) => WebResponse::not_found(),
            }
        }

        _ => WebResponse::not_found(),
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    // The config file of the trees to serve, or for a local server over
    // remote trees (ex: for developing their try pushes' pages and canvases
    // without indexing them), their URLs (see `make_remote_servers`), with
    // their responses cached where SEARCHFOX_REMOTE_CACHE says, if anywhere,
    // and their pages' static files and templates from MOZSEARCH_PATH (by
    // default, the checkout of this binary).  Its pages over the trees'
    // servers work (see `tree_server`); the others need the trees' files.
    let trees = env::args().nth(1).unwrap();
    let cfg = if trees.contains("://") {
        let cache = env::var("SEARCHFOX_REMOTE_CACHE").ok();
        let servers = make_remote_servers(&trees, cache.as_deref()).unwrap();
        let mozsearch_path = env::var("MOZSEARCH_PATH").unwrap_or_else(|_| {
            // (tools/target/release/web-server's.)
            env::current_exe()
                .ok()
                .and_then(|exe| Some(exe.ancestors().nth(4)?.to_string_lossy().into_owned()))
                .unwrap_or_default()
        });
        let cfg = remote_trees_config(&servers, &mozsearch_path);
        SERVERS.lock().unwrap().extend(
            servers
                .into_iter()
                .map(|(name, server)| (name, Arc::from(server))),
        );
        cfg
    } else {
        config::load(&trees, true, None, None, None)
    };

    let cfg = Arc::new(cfg);

    {
        // We *append* to the status file because other server components
        // also write to this file when they are done starting up, and we
        // don't want to clobber those messages.
        let mut status_out = OpenOptions::new()
            .append(true)
            .create(true)
            .open(env::args().nth(2).unwrap())
            .unwrap();
        writeln!(status_out, "web-server.rs loaded").unwrap();
    }

    // The `/explore/` pages facet their files on the files' path kinds and
    // subsystems, which take a few seconds to load for firefox, so load them
    // now for the trees with explore pages (those with a commit index).
    {
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            for (tree_name, tree_config) in &cfg.trees {
                let has_commit_index = tree_config.git.as_ref().is_some_and(|git| {
                    git.history.as_ref().is_some_and(|history| {
                        let branch = tree_config.paths.git_branch.as_deref().unwrap_or("HEAD");
                        CommitIndex::open_branch(
                            &Path::new(&history.path).join("commit-index"),
                            branch,
                        )
                        .is_some()
                    })
                });
                if has_commit_index {
                    let start = std::time::Instant::now();
                    let loaded = path_facet_map(&cfg, tree_name).is_some();
                    println!(
                        "Loaded {}'s path kinds and subsystems ({}) in {:?}",
                        tree_name,
                        if loaded { "ok" } else { "failed" },
                        start.elapsed()
                    );
                }
            }
        });
    }

    // Try pushes' imports, one at a time, as they're asked for; see
    // `tools::try_import`.
    {
        let cfg = cfg.clone();
        let runtime = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            // (Precomputing uses the server's runtime; see `run_on_runtime`.)
            let _runtime = runtime.enter();
            tools::try_import::IMPORTS.run(&cfg, &|tree_name, rev| {
                precompute_try_push(&cfg, tree_name, rev)
            })
        });
    }

    // The syncs of trees of specs' pull request summaries, one at a time, as
    // they're asked for; see `tools::spec_prs`.
    {
        let cfg = cfg.clone();
        std::thread::spawn(move || tools::spec_prs::SYNCS.run(&cfg));
    }

    // The eviction of sidecar files when the disk fills up; see
    // `tools::sidecars`.
    {
        let cfg = cfg.clone();
        std::thread::spawn(move || sidecars::janitor(&cfg));
    }

    // Limit ourselves to processing 4 requests at the same time.
    static SEMAPHORE: Semaphore = Semaphore::const_new(4);
    // Peephole histories and futures (see `format::peephole_json` and
    // `format::future_json`) are relatively expensive and lower priority than
    // pages, so only one of them is processed at a time, and they wait for this
    // before taking one of the above.
    static PEEPHOLE_SEMAPHORE: Semaphore = Semaphore::const_new(1);

    // (Elsewhere with WEB_SERVER_ADDR, ex: beside a production server's on a
    // development machine.)
    let addr: SocketAddr = env::var("WEB_SERVER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8001".to_string())
        .parse()
        .unwrap();
    let server = TcpListener::bind(addr).await.unwrap();
    println!("Listening on http://{addr}");
    loop {
        let (stream, _) = server.accept().await.unwrap();
        let io = TokioIo::new(stream);

        let cfg = cfg.clone();
        tokio::spawn(async move {
            let handler = async |req: http::Request<_>| {
                // (Only a try push's page takes POSTs, to import it, and the
                // pull request summaries' syncs' (see `tools::spec_prs`), as
                // do the data API's routes for those.)
                let post = req.method() == Method::POST
                    && (matches!(req.uri().path().split('/').nth(2), Some("try" | "spec-prs"))
                        || req.uri().path().ends_with("/api/v1/try-import")
                        || req.uri().path().ends_with("/api/v1/spec-prs-sync"));
                if req.method() != Method::GET && !post {
                    return http::Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body("Invalid method".to_string());
                }

                // Requests from outside wait for their client's tokens, or are
                // turned away, and agents and bots only get some of the
                // permits below; see `tools::admission`.
                let _admitted =
                    match admission::admit_request(req.headers(), req.uri().path()).await {
                        Ok(admitted) => admitted,
                        Err(retry) => {
                            return http::Response::builder()
                                .status(StatusCode::TOO_MANY_REQUESTS)
                                .header(header::CONTENT_TYPE, "text/plain")
                                .header(header::RETRY_AFTER, retry.as_secs().to_string())
                                .body("Too many requests; please slow down.".to_string());
                        }
                    };

                let response = {
                    let low_priority = {
                        let path = req.uri().path();
                        ["/peephole/", "/future/", "/before/"]
                            .iter()
                            .any(|kind| path.contains(kind))
                    };
                    let _peephole_permit = if low_priority {
                        Some(PEEPHOLE_SEMAPHORE.acquire().await.unwrap())
                    } else {
                        None
                    };
                    let _permit = SEMAPHORE.acquire().await.unwrap();
                    let cfg = cfg.clone();
                    match tokio::task::spawn_blocking(move || {
                        let path = req.uri().path();
                        let query = req.uri().query().unwrap_or("");
                        let origin = request_origin(req.headers());
                        let response = handle(
                            &cfg,
                            WebRequest {
                                path,
                                query,
                                post,
                                origin,
                            },
                        );
                        let written =
                            !post && query.is_empty() && write_page_sidecar(&cfg, path, &response);
                        (response, written)
                    })
                    .await
                    {
                        Ok(response) => response,

                        Err(join_err) if join_err.is_panic() => {
                            eprintln!("Request handler panicked: {:?}", join_err);
                            (
                                WebResponse::internal_error("Internal server error".to_owned()),
                                false,
                            )
                        }

                        Err(join_err) => {
                            eprintln!(
                                "Request handler task failed or was cancelled: {:?}",
                                join_err
                            );
                            (
                                WebResponse::internal_error("Internal server error".to_owned()),
                                false,
                            )
                        }
                    }
                };
                let (response, written_as_sidecar) = response;

                let mut builder = http::Response::builder()
                    .status(response.status)
                    .header(header::CONTENT_TYPE, response.content_type);

                if let Some(loc) = response.redirect_location {
                    builder = builder.header(header::LOCATION, loc);
                }
                if response.no_store {
                    builder = builder
                        .header(header::CACHE_CONTROL, "no-store")
                        .header("X-Accel-Expires", "0");
                }
                if response.shared {
                    builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
                }
                // (nginx serves the page's sidecar file from now on.)
                if written_as_sidecar {
                    builder = builder.header("X-Accel-Expires", "0");
                }

                builder.body(response.output)
            };

            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(handler))
                .await
            {
                println!("Error serving connection: {:?}", err);
            }
        });
    }
}
