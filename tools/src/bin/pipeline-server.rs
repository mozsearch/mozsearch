use std::{sync::Arc, env, collections::{BTreeMap, HashMap}};

use axum::{http::StatusCode, extract::{Path, Query}, routing::get, Router, Extension, Json, response::{IntoResponse, Response}};
use tools::{abstract_server::{AbstractServer, make_all_local_servers, ServerError}, query::chew_query::chew_query, cmd_pipeline::builder::build_pipeline_graph};


async fn handle_query(
    local_servers: Extension<Arc<BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>>>,
    Path((tree, preset)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>
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

    Ok(Json(result).into_response())
}

#[tokio::main]
async fn main() {
    let local_servers = Arc::new(make_all_local_servers(&env::args().nth(1).unwrap()).unwrap());

    // build our application with a single route
    let app = Router::new()
        .route("/:tree/query/:preset", get(handle_query))
        .layer(Extension(local_servers));

    axum::Server::bind(&"0.0.0.0:8002".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
