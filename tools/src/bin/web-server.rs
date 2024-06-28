extern crate env_logger;
extern crate hyper;
extern crate tools;

use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use hyper::header::{ContentType, Location};
use hyper::method::Method;
use hyper::mime::Mime;
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use hyper::uri;

use tools::blame;
use tools::file_format::config;
use tools::file_format::identifiers::IdentMap;
use tools::format;
use tools::git_ops;

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
            status: StatusCode::Ok,
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
            .. WebResponse::default()
        }
    }

    fn json(body: String) -> WebResponse {
        WebResponse {
            content_type: "application/json".to_owned(),
            output: body,
            .. WebResponse::default()
        }
    }

    fn internal_error(body: String) -> WebResponse {
        WebResponse {
            status: StatusCode::InternalServerError,
            output: body,
            .. WebResponse::default()
        }
    }

    fn not_found() -> WebResponse {
        WebResponse {
            status: StatusCode::NotFound,
            output: "Not found".to_owned(),
            .. WebResponse::default()
        }
    }

    fn redirect(url: String) -> WebResponse {
        WebResponse {
            status: StatusCode::MovedPermanently,
            redirect_location: Some(url),
            .. WebResponse::default()
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
        .. WebResponse::default()
    }
}

fn handle(
    cfg: &config::Config,
    ident_map: &HashMap<String, IdentMap>,
    req: WebRequest,
) -> WebResponse {
    let path = req.path.to_owned();
    let path = path[1..].split('/').collect::<Vec<_>>();

    if path.len() > 0 && path[0] == "static" {
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
        "rev" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            let mut writer = Vec::new();
            match format::format_path(cfg, &tree_name, &rev, &path, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "hgrev" => {
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
                .current_dir(&git_path)
                .output();
            match output_result {
                Ok(output) if output.status.success() => {
                    WebResponse::redirect(format!("/{}/rev/{}/{}",
                        tree_name,
                        git_ops::decode_bytes(output.stdout).trim(),
                        path[3..].join("/"))
                    )
                }
                Ok(_) => WebResponse::not_found(),
                Err(err) => WebResponse::internal_error(format!("{:?}", err)),
            }
        }

        "source" => {
            let path = path.clone().split_off(2);
            let path = path.join("/");

            let tree_config = &cfg.trees[*tree_name];

            let path = format!("{}/file/{}", tree_config.paths.index_path, path);
            return handle_static(path, Some("text/html"));
        }

        "diff" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            let mut writer = Vec::new();
            match format::format_diff(cfg, &tree_name, &rev, &path, &mut writer) {
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
            match format::format_commit(cfg, &tree_name, &rev, &mut writer) {
                Ok(()) => WebResponse::html(String::from_utf8(writer).unwrap()),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "commit-info" => {
            if path.len() < 3 {
                return WebResponse::not_found();
            }

            let rev = &path[2];
            match blame::get_commit_info(&cfg, tree_name, rev) {
                Ok(json) => WebResponse::json(json),
                Err(err) => WebResponse::internal_error(err.to_owned()),
            }
        }

        "complete" => {
            if let Some(ids) = ident_map.get(&tree_name.to_string()) {
                let json = ids.lookup_json(&path[2], false, false, 6);
                WebResponse::json(json)
            } else {
                return WebResponse::not_found();
            }
        }

        _ => WebResponse::not_found(),
    }
}

fn main() {
    env_logger::init();

    let cfg = config::load(&env::args().nth(1).unwrap(), true, None);

    let ident_map = IdentMap::load(&cfg);

    let internal_data = Mutex::new((cfg, ident_map));

    let handler = move |req: Request, mut res: Response| {
        if req.method != Method::Get {
            *res.status_mut() = StatusCode::MethodNotAllowed;
            let resp = format!("Invalid method").into_bytes();
            if let Err(e) = res.send(&resp) {
                eprintln!("Error when replying to {}: {:?}", req.uri, e);
            }
            return;
        }

        let path = match req.uri {
            uri::RequestUri::AbsolutePath(path) => path,
            uri::RequestUri::AbsoluteUri(url) => url.path().to_owned(),
            _ => panic!("Unexpected URI"),
        };

        let guard = match internal_data.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let (ref cfg, ref ident_map) = *guard;

        let response = handle(&cfg, &ident_map, WebRequest { path: &path });

        *res.status_mut() = response.status;
        let output = response.output.into_bytes();
        let mime: Mime = response.content_type.parse().unwrap();
        res.headers_mut().set(ContentType(mime));
        if let Some(loc) = response.redirect_location {
            res.headers_mut().set(Location(loc));
        }
        if let Err(e) = res.send(&output) {
            eprintln!("Error when replying to {}: {:?}", path, e);
        }
    };

    {
        // We *append* to the status file because other server components
        // also write to this file when they are done starting up, and we
        // don't want to clobber those messages.
        let mut status_out = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&env::args().nth(2).unwrap())
            .unwrap();
        writeln!(status_out, "web-server.rs loaded").unwrap();
    }

    println!("On 8001");
    // Use 4 threads instead of the 2 that would be automatically chosen on our
    // AWS boxes.
    let _listening = hyper::Server::http("0.0.0.0:8001").unwrap().handle_threads(handler, 4);
}
