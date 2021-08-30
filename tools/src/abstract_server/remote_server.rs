use async_trait::async_trait;
use serde_json::{from_str, Value};
use url::{ParseError, Url};

use super::server_interface::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// reqwest won't return an error for an unhappy status code itself; someone
/// would need to call `Response::error_from_status`, so for now we'll generally
/// assume everything is some kind of transient problem.
impl From<reqwest::Error> for ServerError {
    fn from(err: reqwest::Error) -> ServerError {
        ServerError::TransientProblem(ErrorDetails {
            layer: ErrorLayer::ServerLayer,
            message: err.to_string(),
        })
    }
}

impl From<ParseError> for ServerError {
    fn from(err: ParseError) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::BadInput,
            message: err.to_string(),
        })
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct RemoteServer {
    server_base_url: Url,
    tree_base_url: Url,
    source_base_url: Url,
    raw_analysis_base_url: Url,
    search_url: Url,
}

async fn get(url: Url) -> Result<reqwest::Response> {
    //println!("Using URL {}", url);
    let res = reqwest::get(url).await?;

    if !res.status().is_success() {
        if res.status().is_server_error() {
            return Err(ServerError::TransientProblem(ErrorDetails {
                layer: ErrorLayer::ServerLayer,
                message: format!("Server status of {}", res.status()),
            }));
        } else {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                message: format!("Server status of {}", res.status()),
            }));
        }
    }

    Ok(res)
}

async fn get_json(url: Url) -> Result<reqwest::Response> {
    let client = reqwest::Client::new();
    let res = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await?;

    if !res.status().is_success() {
        if res.status().is_server_error() {
            return Err(ServerError::TransientProblem(ErrorDetails {
                layer: ErrorLayer::ServerLayer,
                message: format!("Server status of {}", res.status()),
            }));
        } else {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                message: format!("Server status of {}", res.status()),
            }));
        }
    }

    Ok(res)
}

#[async_trait]
impl AbstractServer for RemoteServer {
    async fn fetch_raw_analysis(&self, sf_path: &str) -> Result<Vec<Value>> {
        let url = self.raw_analysis_base_url.join(sf_path)?;
        let raw_str = get(url).await?.text().await?;
        raw_str
            .lines()
            .map(|s| from_str(s).map_err(|e| ServerError::from(e)))
            .collect()
    }

    async fn fetch_html(&self, sf_path: &str) -> Result<String> {
        let url = self.source_base_url.join(sf_path)?;
        let html = get(url).await?.text().await?;
        Ok(html)
    }

    async fn perform_query(&self, q: &str) -> Result<Value> {
        let mut url = self.search_url.clone();
        // If adding more parameters, considering using `query_pairs_mut()`.
        url.set_query(Some(&format!("q={}", q)));
        let raw_str = get_json(url).await?.text().await?;
        match from_str(&raw_str) {
            Ok(json) => Ok(json),
            Err(err) => Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::ServerLayer,
                message: err.to_string(),
            })),
        }
    }
}

pub fn make_remote_server(
    server_base_url: Url,
    tree_name: &str,
) -> Result<Box<dyn AbstractServer + Send + Sync>> {
    let tree_base_url = server_base_url.join(&format!("{}/", tree_name))?;
    let source_base_url = tree_base_url.join("source/")?;
    let raw_analysis_base_url = tree_base_url.join("raw-analysis/")?;
    let search_url = tree_base_url.join("search")?;

    Ok(Box::new(RemoteServer {
        server_base_url,
        tree_base_url,
        source_base_url,
        raw_analysis_base_url,
        search_url,
    }))
}
