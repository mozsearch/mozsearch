extern crate env_logger;
extern crate hyper;
extern crate tools;

use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use git2::Oid;
use hyper::http;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, StatusCode, header};

use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tools::admission;
use tools::blame;
use tools::commit_index::CommitIndex;
use tools::diagnostics::diagnostics_from_config;
use tools::file_format::config;
use tools::file_format::identifiers::IdentMap;
use tools::file_format::per_file_info::path_facet_map;
use tools::format;
use tools::git_ops;

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

fn handle(
    cfg: &config::Config,
    ident_map: &HashMap<String, IdentMap>,
    req: WebRequest,
) -> WebResponse {
    let path = url_decode_path(req.path);
    let path = path[1..].split('/').collect::<Vec<_>>();

    if !path.is_empty() && path[0] == "static" {
        let path = cfg.mozsearch_path.clone() + req.path;
        return handle_static(path, None);
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

            let rev = &path[2];
            let file_name = path[path.len() - 1];

            // Peephole histories (see `format::peephole_json`) as
            // `/{tree}/rev-hyperblame/{rev}/{path}/peephole/{tokens}.json` for
            // comma-separated token indices, futures (see `format::future_json`)
            // as `.../future/{token}.json`, and where tokens were before they
            // were introduced (see `format::before_json`) as
            // `.../before/{token}.json`.  (Data files have other names.)
            let tokens: Option<Vec<u32>> = file_name
                .strip_suffix(".json")
                .and_then(|tokens| tokens.split(',').map(|t| t.parse::<u32>().ok()).collect());
            if let Some(tokens) = tokens
                && path.len() >= 6
                && matches!(path[path.len() - 2], "peephole" | "future" | "before")
            {
                let file_path = path[3..path.len() - 2].join("/");
                let json = match path[path.len() - 2] {
                    "peephole" => format::peephole_json(cfg, tree_name, rev, &file_path, tokens),
                    "future" => format::future_json(cfg, tree_name, rev, &file_path, tokens[0]),
                    _ => format::before_json(cfg, tree_name, rev, &file_path, tokens[0]),
                };
                return match json {
                    Ok(json) => WebResponse::json(json),
                    Err(err) => WebResponse::internal_error(err.to_owned()),
                };
            }

            let file_path = path[3..path.len() - 1].join("/");

            match format::hyperblame_file(cfg, tree_name, rev, &file_path, file_name) {
                Ok(json) => WebResponse::json(json),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // How a file's outline changed between two revisions, as
        // `/{tree}/rev-outline/{from}/{to}/{path}`, where `from` may be
        // `since:YYYY-MM-DD`; see `format::outline_json`.
        "rev-outline" => {
            if path.len() < 5 {
                return WebResponse::not_found();
            }
            let file_path = path[4..].join("/");
            match format::outline_json(cfg, tree_name, path[2], path[3], &file_path) {
                Ok(json) => WebResponse::json(json),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // The page of how a file's outline changed between two revisions, as
        // `/{tree}/outline-diff/{from}/{to}/{path}`; see
        // `format::format_outline_diff`.
        "outline-diff" => {
            if path.len() < 5 {
                return WebResponse::not_found();
            }
            let file_path = path[4..].join("/");
            let mut writer = Vec::new();
            match format::format_outline_diff(
                cfg,
                tree_name,
                path[2],
                path[3],
                &file_path,
                &mut writer,
            ) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
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
        "rev-canvas" => match path.len() {
            3 => {
                let mut writer = Vec::new();
                match format::format_try_canvas(cfg, tree_name, path[2], &mut writer) {
                    Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                    Err(err) => WebResponse::internal_error(err.to_owned()),
                }
            }
            4 if path[3] == "canvas.json" => match format::try_canvas_json(cfg, tree_name, path[2])
            {
                Ok(json) => WebResponse::json(json).shared(),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            },
            _ => WebResponse::not_found(),
        },

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

            let tree_config = &cfg.trees[*tree_name];
            let git_path = match tree_config.get_git_path() {
                Ok(git_path) => git_path,
                Err(_) => return WebResponse::not_found(),
            };

            let hg_rev = path[2];
            // (The system git, with SHA-1 collision detection, since this works on a
            // source repo; see `fast_import_git` for our git.)
            let output_result = Command::new("git")
                .arg("cinnabar")
                .arg("hg2git")
                .arg(hg_rev)
                .current_dir(git_path)
                .output();
            match output_result {
                Ok(output) if output.status.success() => WebResponse::redirect(format!(
                    "/{}/rev/{}/{}",
                    tree_name,
                    git_ops::decode_bytes(output.stdout).trim(),
                    path[3..].join("/")
                )),
                Ok(_) => WebResponse::not_found(),
                Err(err) => WebResponse::internal_error(format!("{:?}", err)),
            }
        }

        "oldrev" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let tree_config = &cfg.trees[*tree_name];
            let old_rev = path[2];
            match (&tree_config.git, Oid::from_str(old_rev)) {
                (Some(gitdata), Ok(old_oid)) => match gitdata.new_rev_for_old_rev(old_oid) {
                    Some(new_oid) => WebResponse::redirect(format!(
                        "/{}/rev/{}/{}",
                        tree_name,
                        new_oid,
                        path[3..].join("/")
                    )),
                    _ => WebResponse::not_found(),
                },
                _ => WebResponse::not_found(),
            }
        }

        "source" => {
            let path = path.clone().split_off(2);
            let path = path.join("/");

            let tree_config = &cfg.trees[*tree_name];

            let path = format!("{}/file/{}", tree_config.paths.index_path, path);
            handle_static(path, Some("text/html"))
        }

        "diff" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            let mut writer = Vec::new();
            match format::format_diff(cfg, tree_name, rev, &path, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // `/{tree}/interdiff/{A}/{B}` lists the files either side changed, and
        // `/{tree}/interdiff/{A}/{B}/{path}` is a file's interdiff, where each
        // side is comma-separated revisions; see `format::format_interdiff`.
        "interdiff" => {
            if path.len() < 4 {
                return WebResponse::not_found();
            }

            let mut writer = Vec::new();
            let result = if path.len() == 4 {
                format::format_interdiff_files(cfg, tree_name, path[2], path[3], &mut writer)
            } else {
                let file_path = path[4..].join("/");
                format::format_interdiff(cfg, tree_name, path[2], path[3], &file_path, &mut writer)
            };
            match result {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "olddiff" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let tree_config = &cfg.trees[*tree_name];
            let old_rev = path[2];
            match (&tree_config.git, Oid::from_str(old_rev)) {
                (Some(gitdata), Ok(old_oid)) => match gitdata.new_rev_for_old_rev(old_oid) {
                    Some(new_oid) => WebResponse::redirect(format!(
                        "/{}/diff/{}/{}",
                        tree_name,
                        new_oid,
                        path[3..].join("/")
                    )),
                    _ => WebResponse::not_found(),
                },
                _ => WebResponse::not_found(),
            }
        }

        // `/{tree}/explore/bug/{bugs}` and `/{tree}/explore/phab/{revs}`; see
        // `format::format_explore`.  And their files' patch stack strips, as
        // `.../stack-strip/{rev}/{path}`; see `format::stack_strip_json`.
        "explore" => {
            if path.len() >= 7 && path[4] == "stack-strip" {
                let stack = path[1..4].join("/");
                let file_path = path[6..].join("/");
                return match format::stack_strip_json(cfg, tree_name, &stack, path[5], &file_path) {
                    Ok(json) => WebResponse::json(json),
                    Err(err) => WebResponse::internal_error(err.to_owned()),
                };
            }
            if path.len() != 4 {
                return WebResponse::not_found();
            }

            let mut writer = Vec::new();
            match format::format_explore(cfg, tree_name, path[2], path[3], &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "commit" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];

            let mut writer = Vec::new();
            match format::format_commit(cfg, tree_name, rev, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "oldcommit" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let tree_config = &cfg.trees[*tree_name];
            let old_rev = path[2];
            match (&tree_config.git, Oid::from_str(old_rev)) {
                (Some(gitdata), Ok(old_oid)) => match gitdata.new_rev_for_old_rev(old_oid) {
                    Some(new_oid) => {
                        WebResponse::redirect(format!("/{}/commit/{}", tree_name, new_oid,))
                    }
                    _ => WebResponse::not_found(),
                },
                _ => WebResponse::not_found(),
            }
        }

        // We don't have an "oldcommit-info" because this endpoint is only for
        // AJAX-y use by the current blame strip UI that fetches commit-info on
        // demand, and these links are considered mozsearch-internal and will
        // never be generated for anything but a current revision.
        "commit-info" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];
            match blame::get_commit_info(cfg, tree_name, rev) {
                Ok(json) => WebResponse::json(json),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        // A try push's page, or (POST) asking for its import, and its import's
        // status; see `tools::try_import`.  And its files' patch stack strips,
        // as `.../stack-strip/{rev}/{path}`; see `format::stack_strip_json`.
        "try" => {
            if path.len() >= 6 && path[3] == "stack-strip" {
                let stack = path[1..3].join("/");
                let file_path = path[5..].join("/");
                return match format::stack_strip_json(cfg, tree_name, &stack, path[4], &file_path) {
                    Ok(json) => WebResponse::json(json).uncached(),
                    Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
                };
            }
            if path.len() != 3 {
                return WebResponse::not_found();
            }
            let rev = path[2];
            let mut notice = None;
            if req.post {
                match format::request_try_import(cfg, tree_name, rev) {
                    Ok(()) => {
                        return WebResponse::see_other(format!("/{}/try/{}", tree_name, rev))
                            .uncached();
                    }
                    Err(err) => notice = Some(err),
                }
            }
            let mut writer = Vec::new();
            match format::format_try(cfg, tree_name, rev, notice, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()).uncached(),
                Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
            }
        }

        // A scope's history; see `format::format_history`.
        "history" => {
            if path.len() < 4 {
                return WebResponse::not_found();
            }
            let file_path = path[3..].join("/");
            let mut writer = Vec::new();
            match format::format_history(
                cfg,
                tree_name,
                path[2],
                &file_path,
                req.query,
                &mut writer,
            ) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "try-status" => {
            if path.len() != 3 {
                return WebResponse::not_found();
            }
            match format::format_try_status(cfg, tree_name, path[2]) {
                Ok(json) => WebResponse::json(json).uncached(),
                Err(err) => WebResponse::internal_error(err.to_owned()).uncached(),
            }
        }

        "complete" => {
            if let Some(ids) = ident_map.get(&tree_name.to_string()) {
                let json = ids.lookup_json(path[2], false, false, 6);
                WebResponse::json(json)
            } else {
                WebResponse::not_found()
            }
        }

        _ => WebResponse::not_found(),
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let cfg = config::load(&env::args().nth(1).unwrap(), true, None, None, None);
    let ident_map = IdentMap::load(&cfg);

    let cfg = Arc::new(cfg);
    let ident_map = Arc::new(ident_map);

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
        std::thread::spawn(move || tools::sidecars::janitor(&cfg));
    }

    // Limit ourselves to processing 4 requests at the same time.
    static SEMAPHORE: Semaphore = Semaphore::const_new(4);
    // Peephole histories and futures (see `format::peephole_json` and
    // `format::future_json`) are relatively expensive and lower priority than
    // pages, so only one of them is processed at a time, and they wait for this
    // before taking one of the above.
    static PEEPHOLE_SEMAPHORE: Semaphore = Semaphore::const_new(1);

    let addr: SocketAddr = "0.0.0.0:8001".parse().unwrap();
    let server = TcpListener::bind(addr).await.unwrap();
    println!("Listening on http://{addr}");
    loop {
        let (stream, _) = server.accept().await.unwrap();
        let io = TokioIo::new(stream);

        let cfg = cfg.clone();
        let ident_map = ident_map.clone();
        tokio::spawn(async move {
            let handler = async |req: http::Request<_>| {
                // (Only a try push's page takes POSTs, to import it.)
                let post = req.method() == Method::POST
                    && req.uri().path().split('/').nth(2) == Some("try");
                if req.method() != Method::GET && !post {
                    return http::Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body("Invalid method".to_string());
                }

                // Requests from outside wait for their client's tokens, or are
                // turned away, and agents and bots only get some of the
                // permits below; see `tools::admission`.
                let _class_permit =
                    match admission::admit_request(req.headers(), req.uri().path()).await {
                        Ok((_, permit)) => permit,
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
                    let ident_map = ident_map.clone();
                    match tokio::task::spawn_blocking(move || {
                        let path = req.uri().path();
                        let query = req.uri().query().unwrap_or("");
                        handle(&cfg, &ident_map, WebRequest { path, query, post })
                    })
                    .await
                    {
                        Ok(response) => response,

                        Err(join_err) if join_err.is_panic() => {
                            eprintln!("Request handler panicked: {:?}", join_err);
                            WebResponse::internal_error("Internal server error".to_owned())
                        }

                        Err(join_err) => {
                            eprintln!(
                                "Request handler task failed or was cancelled: {:?}",
                                join_err
                            );
                            WebResponse::internal_error("Internal server error".to_owned())
                        }
                    }
                };

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
