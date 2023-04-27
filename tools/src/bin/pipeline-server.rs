use std::{
    collections::{BTreeMap, HashMap},
    env,
    sync::Arc,
};

use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Html},
    routing::get,
    Extension, Json, Router,
};
use axum_macros::debug_handler;
use liquid::Template;
use serde_json::Value;
use tools::{
    abstract_server::{make_all_local_servers, AbstractServer, ServerError},
    cmd_pipeline::{builder::build_pipeline_graph, PipelineValues},
    query::chew_query::chew_query,
    templating::builder::build_and_parse_query_results, logging::{LoggedSpan, init_logging},
};
use tracing::Instrument;

#[debug_handler]
async fn handle_query(
    local_servers: Extension<Arc<BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>>>,
    templates: Extension<Arc<SomeTemplates>>,
    headers: HeaderMap,
    Path((tree, preset)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ServerError> {
    let server = match local_servers.get(&tree) {
        Some(s) => s,
        None => {
            return Ok((StatusCode::NOT_FOUND, format!("No such tree: {}", tree)).into_response());
        }
    };

    if preset.as_str() != "default" {
        return Ok((StatusCode::NOT_FOUND, format!("No such preset: {}", preset)).into_response());
    }

    let logged_span = LoggedSpan::new_logged_span("query");
    let maybe_log = params.contains_key("debug");

    let query = match params.get("q") {
        Some(q) => q,
        None => {
            return Ok((StatusCode::BAD_REQUEST, "No 'q' parameter, no results!").into_response());
        }
    };

    let graph = {
        let _log_entered = logged_span.span.clone().entered();

        let pipeline_plan = chew_query(query)?;

        build_pipeline_graph(server.clonify(), pipeline_plan)?
    };

    let result = graph.run(true).instrument(logged_span.span.clone()).await?;

    let accept = headers
        .get("accept")
        .map(|x| x.to_str().unwrap_or_else(|_| "text/html"));
    let make_html = match accept {
        Some("application/json") => false,
        _ => true,
    };

    let logs = logged_span.retrieve_serde_json().await;
    let logs = if maybe_log {
        logs
    } else {
        Value::Null
    };

    if make_html {
        let sym_info_str = match &result {
            PipelineValues::GraphResultsBundle(grb) => {
                serde_json::to_string(&grb.symbols).unwrap_or_else(|_| "{}".to_string())
            }
            _ => "{}".to_string(),
        };

        let globals = liquid::object!({
            "results": result,
            "query": query.clone(),
            "preset": preset.clone(),
            "tree": tree.clone(),
            "logs": logs,
            "SYM_INFO_STR": sym_info_str,
        });

        let output = templates.query_results.render(&globals)?;
        Ok(Html(output).into_response())
    } else {
        Ok(Json(result).into_response())
    }
}

struct SomeTemplates {
    query_results: Template,
}

#[tokio::main]
async fn main() {
    init_logging();

    let local_servers = Arc::new(make_all_local_servers(&env::args().nth(1).unwrap()).unwrap());
    let templates = Arc::new(SomeTemplates {
        query_results: build_and_parse_query_results(),
    });

    // build our application with a single route
    let app = Router::new()
        .route("/:tree/query/:preset", get(handle_query))
        .layer(Extension(local_servers))
        .layer(Extension(templates));

    axum::Server::bind(&"0.0.0.0:8002".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
