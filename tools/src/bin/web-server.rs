extern crate hyper;
extern crate env_logger;
extern crate tools;

use std::thread;
use std::sync::mpsc;

use hyper::server::{Request, Response, Handler, Fresh};
use hyper::uri;

use tools::config;
use tools::blame;

static PHRASE: &'static [u8] = b"Hello World!";

struct WebRequest {
    path: String,
}

fn main_thread(tx: mpsc::Sender<u32>, rx: mpsc::Receiver<WebRequest>) {
    let cfg = config::load();
    let blame_data = blame::load(&cfg);

    let req = rx.recv().unwrap();
    //println!("main got {}", v);
    tx.send(22).unwrap();
}

struct ReqHandler {
    tx: mpsc::Sender<WebRequest>,
    rx: mpsc::Receiver<u32>,
}

impl Handler for ReqHandler {
    fn handle<'a, 'k>(&'a self, req: Request<'a, 'k>, res: Response<'a, Fresh>) {
        res.send(PHRASE).unwrap();
    }
}

fn main() {
    env_logger::init().unwrap();

    let (tx1, rx2) = mpsc::channel();
    let (tx2, rx1) : (mpsc::Sender<WebRequest>, _) = mpsc::channel();
    let th = thread::spawn(move || { main_thread(tx1, rx1); });

    let handler = ReqHandler { tx: tx2, rx: rx2 };

    /*
    let handler = move |req: Request, res: Response| {
        let path = match req.uri {
            uri::RequestUri::AbsolutePath(path) => path,
            uri::RequestUri::AbsoluteUri(url) => url.path().to_owned(),
            _ => panic!("Unexpected URI"),
        };



        if req.method != Method::Get {
            return Ok(Response::with((status::MethodNotAllowed,
                                      format!("Method not allowed"))));
        }

        let url = &req.url;
        if url.path.len() < 2 {
            return Ok(Response::with((status::NotFound,
                                      format!("Not found"))));
        }

        let tree_name = &url.path[0];
        let kind = &url.path[1];

        match &kind[..] {
            "rev" => {
                if url.path.len() < 3 {
                    return Ok(Response::with((status::NotFound,
                                              format!("Not found"))));
                }

                let rev = &url.path[2];
                let path = url.path.clone().split_off(3);
                let path = path.join("/");

                format::format_path(cfg, &tree_name, &rev, &path, &mut writer);
                
                Ok(Response::with((status::Ok, "")))
            },

            "commit-info" => {
                if url.path.len() < 3 {
                    return Ok(Response::with((status::NotFound,
                                              format!("Not found"))));
                }

                let rev = &url.path[2];
                let json = blame::get_commit_info(&blame_data, tree_name, rev);

                let content_type = "application/json".parse::<Mime>().unwrap();
                Ok(Response::with((content_type, status::Ok, json)))
            },

            _ => {
                return Ok(Response::with((status::NotFound,
                                          format!("Not found"))));
            }
        }
    };
         */

    println!("On 8001");
    let _listening = hyper::Server::http("127.0.0.1:8001").unwrap().handle(handler);

    th.join().unwrap();
}
