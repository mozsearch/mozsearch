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
use liquid::Template;
use tools::{
    abstract_server::{make_all_local_servers, AbstractServer, ServerError},
    cmd_pipeline::{builder::build_pipeline_graph, PipelineValues},
    query::chew_query::chew_query,
    templating::builder::build_and_parse_query_results,
};

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

    let query = match params.get("q") {
        Some(q) => q,
        None => {
            return Ok((StatusCode::BAD_REQUEST, "No 'q' parameter, no results!").into_response());
        }
    };

    let pipeline_plan = chew_query(query)?;

    let graph = build_pipeline_graph(server.clonify(), pipeline_plan)?;

    let result = graph.run(true).await?;

    let accept = headers
        .get("accept")
        .map(|x| x.to_str().unwrap_or_else(|_| "text/html"));
    let make_html = match accept {
        Some("application/json") => false,
        _ => true,
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
