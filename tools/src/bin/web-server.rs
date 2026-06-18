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
use tools::blame;
use tools::diagnostics::diagnostics_from_config;
use tools::file_format::config;
use tools::file_format::identifiers::IdentMap;
use tools::format;
use tools::git_ops;

use tools::url_encode_path::url_decode_path;

struct WebRequest<'a> {
    path: &'a str,
}

struct WebResponse {
    status: StatusCode,
    content_type: String,
    redirect_location: Option<String>,
    output: String,
}

impl Default for WebResponse {
    fn default() -> WebResponse {
        WebResponse {
            status: StatusCode::OK,
            content_type: "text/plain".to_owned(),
            redirect_location: None,
            output: String::new(),
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

    println!("DBG {:?} {} {}", path, tree_name, kind);

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

            let mut writer = Vec::new();
            match format::format_path(cfg, tree_name, rev, &path, &mut writer) {
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
                (Some(gitdata), Ok(old_oid)) => match gitdata.old_map.get(&old_oid) {
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

        "olddiff" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let tree_config = &cfg.trees[*tree_name];
            let old_rev = path[2];
            match (&tree_config.git, Oid::from_str(old_rev)) {
                (Some(gitdata), Ok(old_oid)) => match gitdata.old_map.get(&old_oid) {
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
                (Some(gitdata), Ok(old_oid)) => match gitdata.old_map.get(&old_oid) {
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

    // Limit ourselves to processing 4 requests at the same time.
    static SEMAPHORE: Semaphore = Semaphore::const_new(4);

    let addr: SocketAddr = "0.0.0.0:8001".parse().unwrap();
    let server = TcpListener::bind(addr).await.unwrap();
    println!("Listening on http://{addr}");
    loop {
        let (stream, _) = server.accept().await.unwrap();
        let io = TokioIo::new(stream);

        let handler = async |req: http::Request<_>| {
            if req.method() != Method::GET {
                return http::Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .body("Invalid method".to_string());
            }

            let response = {
                let _ = SEMAPHORE.acquire().await.unwrap();
                let cfg = cfg.clone();
                let ident_map = ident_map.clone();
                tokio::task::spawn_blocking(move || {
                    let path = req.uri().path();
                    handle(&cfg, &ident_map, WebRequest { path })
                })
                .await
                .unwrap()
            };

            let mut builder = http::Response::builder()
                .status(response.status)
                .header(header::CONTENT_TYPE, response.content_type);

            if let Some(loc) = response.redirect_location {
                builder = builder.header(header::LOCATION, loc);
            }

            builder.body(response.output)
        };

        if let Err(err) = http1::Builder::new()
            .serve_connection(io, service_fn(handler))
            .await
        {
            println!("Error serving connection: {:?}", err);
        }
    }
}
