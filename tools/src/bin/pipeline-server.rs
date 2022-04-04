use axum::{routing::get, Router};

//async fn handle_pipeline()

#[tokio::main]
async fn main() {
    // build our application with a single route
    let app = Router::new()
        .route("/:tree/query/:preset", get(|| async { "Hello, World!" }));

    axum::Server::bind(&"0.0.0.0:8002".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
