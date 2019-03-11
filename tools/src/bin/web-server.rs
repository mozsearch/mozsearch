extern crate hyper;
extern crate env_logger;
extern crate tools;

use std::sync::Mutex;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::io::{Read, Write};
use std::path::Path;
use std::env;
use std::collections::HashMap;

use hyper::status::StatusCode;
use hyper::method::Method;
use hyper::server::{Request, Response};
use hyper::header::ContentType;
use hyper::mime::Mime;
use hyper::uri;

use tools::config;
use tools::blame;
use tools::format;
use tools::file_format::identifiers::IdentMap;

struct WebRequest {
    path: String,
}

struct WebResponse {
    status: StatusCode,
    content_type: String,
    output: String,
}

fn not_found() -> WebResponse {
    WebResponse {
        status: StatusCode::NotFound,
        content_type: "text/plain".to_owned(),
        output: "Not found".to_owned()
    }
}

fn handle_static(path: String, content_type: Option<&str>) -> WebResponse {
    let source_file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => {
            return not_found();
        },
    };
    let mut reader = BufReader::new(&source_file);
    let mut input = String::new();
    match reader.read_to_string(&mut input) {
        Ok(_) => {},
        Err(_) => {
            return not_found();
        }
    }

    let inferred_content_type = match Path::new(&path).extension() {
        Some(ext) =>
            match ext.to_str().unwrap() {
                "css" => "text/css",
                "js" => "text/javascript",
                _ => "text/html",
            },
        None => "text/html"
    };
    let content_type = match content_type {
        Some(ct) => ct,
        None => inferred_content_type,
    };

    WebResponse { status: StatusCode::Ok, content_type: content_type.to_owned(), output: input }
}

fn handle(cfg: &config::Config, ident_map: &HashMap<String, IdentMap>, req: WebRequest) -> WebResponse {
    let path = req.path.clone();
    let path = path[1..].split('/').collect::<Vec<_>>();

    if path.len() > 0 && path[0] == "static" {
        let path = cfg.mozsearch_path.clone() + &req.path;
        return handle_static(path, None);
    }

    if path.len() < 2 {
        return not_found();
    }

    let tree_name = &path[0];
    let kind = &path[1];

    println!("DBG {:?} {} {}", path, tree_name, kind);

    match &kind[..] {
        "rev" => {
            if path.len() < 3 {
                return not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            let mut writer = Vec::new();
            match format::format_path(cfg, &tree_name, &rev, &path, &mut writer) {
                Ok(()) => {
                    let output = String::from_utf8(writer).unwrap();
                    WebResponse { status: StatusCode::Ok, content_type: "text/html".to_owned(), output: output }
                },
                Err(err) =>
                    WebResponse {
                        status: StatusCode::InternalServerError,
                        content_type: "text/plain".to_owned(),
                        output: err.to_owned(),
                    }
            }
        },

        "source" => {
            let path = path.clone().split_off(2);
            let path = path.join("/");

            let tree_config = cfg.trees.get(*tree_name).unwrap();

            let path = format!("{}/file/{}", tree_config.paths.index_path, path);
            return handle_static(path, Some("text/html"));
        },

        "diff" => {
            if path.len() < 3 {
                return not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            let mut writer = Vec::new();
            match format::format_diff(cfg, &tree_name, &rev, &path, &mut writer) {
                Ok(()) => {
                    let output = String::from_utf8(writer).unwrap();
                    WebResponse { status: StatusCode::Ok, content_type: "text/html".to_owned(), output: output }
                },
                Err(err) =>
                    WebResponse {
                        status: StatusCode::InternalServerError,
                        content_type: "text/plain".to_owned(),
                        output: err.to_owned(),
                    }
            }
        },

        "commit" => {
            if path.len() < 3 {
                return not_found();
            }

            let rev = &path[2];

            let mut writer = Vec::new();
            match format::format_commit(cfg, &tree_name, &rev, &mut writer) {
                Ok(()) => {
                    let output = String::from_utf8(writer).unwrap();
                    WebResponse { status: StatusCode::Ok, content_type: "text/html".to_owned(), output: output }
                },
                Err(err) =>
                    WebResponse {
                        status: StatusCode::InternalServerError,
                        content_type: "text/plain".to_owned(),
                        output: err.to_owned(),
                    }
            }
        },

        "commit-info" => {
            if path.len() < 3 {
                return not_found();
            }

            let rev = &path[2];
            match blame::get_commit_info(&cfg, tree_name, rev) {
                Ok(json) =>
                    WebResponse {
                        status: StatusCode::Ok,
                        content_type: "application/json".to_owned(),
                        output: json
                    },
                Err(err) =>
                    WebResponse {
                        status: StatusCode::InternalServerError,
                        content_type: "text/plain".to_owned(),
                        output: err.to_owned(),
                    }
            }
        },

        "complete" => {
            let ids = ident_map.get(&tree_name.to_string()).unwrap();
            let json = ids.lookup_json(&path[2], false, false, 6);
            WebResponse {
                status: StatusCode::Ok,
                content_type: "application/json".to_owned(),
                output: json
            }
        },

        _ => {
            not_found()
        }
    }
}

fn main() {
    env_logger::init();

    let cfg = config::load(&env::args().nth(1).unwrap(), true);
    let ident_map = IdentMap::load(&cfg);

    let internal_data = Mutex::new((cfg, ident_map));

    let handler = move |req: Request, mut res: Response| {
        if req.method != Method::Get {
            *res.status_mut() = StatusCode::MethodNotAllowed;
            let resp = format!("Invalid method").into_bytes();
            res.send(&resp).unwrap();
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

        let response = handle(&cfg, &ident_map, WebRequest { path: path });

        *res.status_mut() = response.status;
        let output = response.output.into_bytes();
        let mime: Mime = response.content_type.parse().unwrap();
        res.headers_mut().set(ContentType(mime));
        res.send(&output).unwrap();
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
    let _listening = hyper::Server::http("0.0.0.0:8001").unwrap().handle(handler);
}
