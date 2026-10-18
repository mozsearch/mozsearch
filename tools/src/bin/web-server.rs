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
    /// Whether it's a POST (only `/{tree}/try/{rev}`'s are allowed).
    post: bool,
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

    /// Where to go after a POST.
    fn see_other(url: String) -> WebResponse {
        WebResponse {
            status: StatusCode::SEE_OTHER,
            redirect_location: Some(url),
            ..WebResponse::default()
        }
    }

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
    }
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

            // (Links to specs' anchors and lines go to the pages with them.)
            if let Some(url) = format::rendered_redirect(cfg, tree_name, rev, &file_path) {
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
            if path.len() != 4 {
                return WebResponse::not_found();
            }

            let (server, kind, keys) = (server.clonify(), path[2].to_string(), path[3].to_string());
            match format::run_on_runtime(async move { server.explore_keys(&kind, &keys).await }) {
                Ok(data) => {
                    let mut writer = Vec::new();
                    match format::render_explore_page(tree_name, data, &mut writer) {
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

        // A try push's page, or (POST) asking for its import, and its import's
        // status; see `tools::try_import`.  And its files' patch stack strips,
        // as `.../stack-strip/{rev}/{path}`; see `format::stack_strip_json`.
        "try" => {
            if path.len() >= 6 && path[3] == "stack-strip" {
                let stack = path[1..3].join("/");
                let file_path = path[5..].join("/");
                // (Over the tree's server; see `tree_server`.)
                let Some(server) = tree_server(tree_name) else {
                    return WebResponse::not_found();
                };
                let (server, rev) = (server.clonify(), path[4].to_string());
                return match format::run_on_runtime(async move {
                    server.stack_strip_json(&stack, &rev, &file_path).await
                }) {
                    Ok(json) => WebResponse::json(json).uncached(),
                    Err(err) => WebResponse::internal_error(server_error_message(&err)).uncached(),
                };
            }
            if path.len() != 3 {
                return WebResponse::not_found();
            }
            let rev = path[2];
            // (Over the tree's server; see `tree_server`.)
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            let mut notice = None;
            if req.post {
                match format::request_try_import(server.as_ref(), rev) {
                    Ok(()) => {
                        return WebResponse::see_other(format!("/{}/try/{}", tree_name, rev))
                            .uncached();
                    }
                    Err(err) => notice = Some(err),
                }
            }
            let mut writer = Vec::new();
            match format::format_try(
                server.as_ref(),
                tree_name,
                rev,
                notice.as_deref(),
                &mut writer,
            ) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()).uncached(),
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

        "try-status" => {
            if path.len() != 3 {
                return WebResponse::not_found();
            }
            let Some(server) = tree_server(tree_name) else {
                return WebResponse::not_found();
            };
            match format::format_try_status(server.as_ref(), path[2]) {
                Ok(json) => WebResponse::json(json).uncached(),
                Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
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
        std::thread::spawn(move || tools::try_import::IMPORTS.run(&cfg));
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
                // (Only a try push's page takes POSTs, to import it, as does
                // the data API's route for that.)
                let post = req.method() == Method::POST
                    && (req.uri().path().split('/').nth(2) == Some("try")
                        || req.uri().path().ends_with("/api/v1/try-import"));
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
                        let response = handle(&cfg, WebRequest { path, query, post });
                        let written =
                            !post && query.is_empty() && write_page_sidecar(&cfg, path, &response);
                        (response, written)
                    })
                    .await
                    {
                        Ok(response) => response,

                        Err(join_err) if join_err.is_panic() => {
                            eprintln!("Request handler panicked: {:?}", join_err);
                            (WebResponse::internal_error("Internal server error".to_owned()), false)
                        }

                        Err(join_err) => {
                            eprintln!(
                                "Request handler task failed or was cancelled: {:?}",
                                join_err
                            );
                            (WebResponse::internal_error("Internal server error".to_owned()), false)
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
                    builder = builder.header(header::CACHE_CONTROL, "no-store");
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
