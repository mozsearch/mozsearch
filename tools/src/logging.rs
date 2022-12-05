use std::{collections::HashMap, sync::Mutex};

use serde_json::{json, Map, Value};
use tokio::{
    sync::oneshot::{self, Receiver, Sender},
    task::JoinHandle,
};
use tracing::{info, info_span, span::EnteredSpan};
use tracing_forest::{processor::from_fn, traits::*, tree::Tree, worker_task};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter, Layer, Registry};
use uuid::Uuid;

#[allow(dead_code)]
struct LogGlobal {
    handle: JoinHandle<()>,
}

lazy_static! {
    static ref SPAN_MAP: Mutex<HashMap<uuid::Uuid, Sender<Tree>>> = Mutex::new(HashMap::new());
    static ref LOG_GLOBAL: Mutex<Option<LogGlobal>> = Mutex::new(None);
}

/// Mechanism for creating a logging span that, using tracing-forest, will
/// aggregate a hierarchy of all of everything that was nested under the span
/// as long as all `tokio::spawn`ed tasks have had `tracing::Instrument` used to
/// call `.instrument(some_span_to_wrap_the_task)` or `.in_current_span()`.
///
/// It is necessary to have called `init_logging()` below to have set up the
/// necessary machinery which will install tracing-forest as the subscriber.
pub struct LoggedSpan {
    span: EnteredSpan,
    rx: Receiver<Tree>,
}

pub fn render_forest_to_value(tree: &Tree) -> Value {
    match tree {
        Tree::Span(span) => {
            json!({
                "name": span.name(),
                "nodes": span.nodes().iter().map(render_forest_to_value).collect::<Vec<Value>>(),
            })
        }
        Tree::Event(event) => {
            let mut obj = Map::new();
            if let Some(msg) = event.message() {
                obj.insert("message".to_string(), json!(msg));
            }
            for field in event.fields() {
                obj.insert(field.key().to_string(), json!(field.value()));
            }
            json!(obj)
        }
    }
}

impl LoggedSpan {
    pub fn new_logged_span(name: &str) -> LoggedSpan {
        let id = Uuid::new_v4();

        //let id_str = id.as_simple().to_string();
        let span = info_span!(parent: None, "logged_span", name, uuid = %id).entered();
        info!("logged_span_start");
        let (tx, rx) = oneshot::channel();

        {
            let mut span_map = SPAN_MAP.lock().unwrap();
            span_map.insert(id.clone(), tx);
        }

        LoggedSpan { span, rx }
    }

    pub async fn retrieve(self) -> Tree {
        info!("logged_span_end");
        drop(self.span);
        let tree = self.rx.await.unwrap();
        tree
    }

    pub async fn retrieve_serde_json(self) -> Value {
        let tree = self.retrieve().await;
        render_forest_to_value(&tree)
    }
}

/// Initialize logging; for now we currently always use a hard-coded value of
/// tools=trace for the `LoggedSpan` mechanism because that's all we care about,
/// but if you set the environment variable `RUST_LOG` to a non-empty value, we
/// will enable pretty/verbose logging (although we can change that if desired).
//#[allow(unused_must_use)]
pub fn init_logging() {
    {
        let global_opt = LOG_GLOBAL.lock().unwrap();
        if global_opt.is_some() {
            return;
        }
    }

    let mut layers = Vec::new();
    // If RUST_LOG is present and *non-empty* then interpret it and use it.
    // Because of limitations in our shell-scripts for our tests, we will
    // frequently set RUST_LOG unconditionally but potentially with an empty
    // value, and we don't want that to be interpreted as a desire to enable
    // logging.
    if let Ok(rustlog) = std::env::var("RUST_LOG") {
        if !rustlog.is_empty() {
            if let Ok(env_filter) = EnvFilter::try_from_default_env() {
                let layer = tracing_subscriber::fmt::layer()
                    .with_span_events(FmtSpan::ENTER | FmtSpan::EXIT)
                    //.pretty()
                    .compact()
                    // We primarily expect this to go in our log which can be
                    // excerpted for email purposes, and so ANSI isn't helpful
                    // for this.
                    .with_ansi(false)
                    // In general we don't care about the wall time that much,
                    // and it takes up a lot of columns, especially in tracing
                    // which includes sub-second granularities.
                    //
                    // Also, if we leave time enabled, we have to fix
                    // send-warning-email.py to deal with the sub-seconds.
                    .without_time()
                    // I had enabled the thread ids for diagnosing complicated
                    // async issues, but ideally we won't see this much, so this
                    // will just be noise most of the time.
                    //.with_thread_ids(true)
                    .with_filter(env_filter)
                    .boxed();
                layers.push(layer);
            }
        }
    }

    let handle = tokio::spawn(
        worker_task()
            .set_global(true)
            .map_receiver(|_| {
                // Return our new receiver to replace the initial receiver.
                from_fn(|tree| {
                    // For every tree we receive, see if it has a UUID that we're
                    // looking for, and if so, put it in the map so that it can be
                    // extracted.
                    if let Tree::Span(span) = &tree {
                        let mut span_map = SPAN_MAP.lock().unwrap();

                        let id = span.uuid();
                        if let Some(tx) = span_map.remove(&id) {
                            tx.send(tree).unwrap();
                        }
                    }
                    Ok(())
                })
            })
            .build_with(|layer| {
                layers.push(layer.boxed());
                Registry::default()
                    .with(layers)
                    .with(EnvFilter::new("tools=trace"))
            })
            // set this up to run forever?
            .on(async {
                tokio::signal::ctrl_c().await.expect("Ctrl-C sad!");
            }),
    );


    {
        let mut global_opt = LOG_GLOBAL.lock().unwrap();
        *global_opt = Some(LogGlobal { handle });
    }
}
