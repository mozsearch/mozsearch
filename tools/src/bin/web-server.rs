extern crate hyper;
extern crate env_logger;
extern crate tools;

use std::thread;
use std::sync::{mpsc, Mutex};
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::path::Path;
use std::env;

use hyper::status::StatusCode;
use hyper::method::Method;
use hyper::server::{Request, Response, Handler};
use hyper::header::ContentType;
use hyper::mime::Mime;
use hyper::uri;

use tools::config;
use tools::blame;
use tools::format;

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

fn handle_static(cfg: &config::Config, req: WebRequest) -> WebResponse {
    let path = cfg.mozsearch_path.clone() + &req.path;
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

    let content_type = match Path::new(&path).extension() {
        Some(ext) =>
            match ext.to_str().unwrap() {
                "css" => "text/css",
                "js" => "text/javascript",
                _ => "text/html",
            },
        None => "text/html"
    };
    
    WebResponse { status: StatusCode::Ok, content_type: content_type.to_owned(), output: input }
}

fn handle(cfg: &config::Config, req: WebRequest) -> WebResponse {
    let path = req.path.clone();
    let path = path[1..].split('/').collect::<Vec<_>>();

    if path.len() > 0 && path[0] == "static" {
        return handle_static(cfg, req);
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

        "commit" => {
            if path.len() < 3 {
                return not_found();
            }

            let rev = &path[2];
            let path = path.clone().split_off(3);
            let path = path.join("/");

            let mut writer = Vec::new();
            match format::format_commit(cfg, &tree_name, &rev, &path, &mut writer) {
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

        _ => {
            not_found()
        }
    }
}

fn main_thread(tx: mpsc::Sender<WebResponse>, rx: mpsc::Receiver<WebRequest>) {
    let cfg = config::load(&env::args().nth(1).unwrap(), true);

    loop {
        let req = rx.recv().unwrap();
        println!("main got {}", req.path);

        let response = handle(&cfg, req);
        tx.send(response).unwrap();
    }
}

fn main() {
    env_logger::init().unwrap();

    let (tx1, rx2) = mpsc::channel();
    let (tx2, rx1) = mpsc::channel();
    let th = thread::spawn(move || { main_thread(tx1, rx1); });

    let channels = Mutex::new((tx2, rx2));

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

        let guard = channels.lock().unwrap();
        let (ref tx, ref rx) = *guard;

        tx.send(WebRequest { path: path }).unwrap();
        let response = rx.recv().unwrap();

        *res.status_mut() = response.status;
        let output = response.output.into_bytes();
        let mime: Mime = response.content_type.parse().unwrap();
        res.headers_mut().set(ContentType(mime));
        res.send(&output).unwrap();
    };

    println!("On 8001");
    let _listening = hyper::Server::http("127.0.0.1:8001").unwrap().handle(handler);

    th.join().unwrap();
}
