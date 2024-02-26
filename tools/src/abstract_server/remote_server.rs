use async_trait::async_trait;
use futures_core::stream::BoxStream;
use serde_json::{from_str, Value};
use url::{ParseError, Url};
use ustr::Ustr;

use super::{
    server_interface::{
        AbstractServer, ErrorDetails, ErrorLayer, FileMatches, Result, SearchfoxIndexRoot,
        ServerError,
    },
    HtmlFileRoot, TextMatches, TreeInfo,
};

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
#[derive(Clone, Debug)]
struct RemoteServer {
    tree_name: String,
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
    fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync> {
        Box::new(self.clone())
    }

    fn tree_info(&self) -> Result<TreeInfo> {
        Ok(TreeInfo {
            name: self.tree_name.clone(),
        })
    }

    fn translate_path(&self, _root: SearchfoxIndexRoot, _sf_path: &str) -> Result<String> {
        // Remote servers don't have local filesystem paths.
        Err(ServerError::Unsupported)
    }

    async fn fetch_raw_analysis(&self, sf_path: &str) -> Result<BoxStream<Value>> {
        let url = self.raw_analysis_base_url.join(sf_path)?;
        let raw_str = get(url).await?.text().await?;
        let values: Result<Vec<Value>> = raw_str
            .lines()
            .map(|s| from_str(s).map_err(|e| ServerError::from(e)))
            .collect();
        Ok(Box::pin(tokio_stream::iter(values?)))
    }

    async fn fetch_raw_source(&self, _sf_path: &str) -> Result<String> {
        // I'm not sure we actually expose the underlying raw file?
        Err(ServerError::Unsupported)
    }

    async fn fetch_html(&self, root: HtmlFileRoot, sf_path: &str) -> Result<String> {
        // We don't have access to raw templates, so just call that unsupported.
        // Note that we could special-case for "help.html" here since it does
        // get explicitly exposed as "index.html", but it's also fine to only
        // validate this for local files.
        if root == HtmlFileRoot::FormattedTemplate {
            return Err(ServerError::Unsupported);
        }
        // Our tree-relative paths should not start with a slash
        let norm_path = if sf_path.starts_with('/') {
            &sf_path[1..]
        } else {
            sf_path
        };
        // We don't both caring about the presence of ".." here because we don't
        // have any security-ish things to worry about for a public web server.

        let url = self.source_base_url.join(norm_path)?;
        let html = get(url).await?.text().await?;
        Ok(html)
    }

    async fn crossref_lookup(&self, _symbol: &str, _extra_processing: bool) -> Result<Value> {
        // Let's require local index for now; we'll expose this once this
        // mechanism is exposed to the web so we can talk to the corresponding
        // local server over https.
        //
        // That is, we could build this on top of the existing router.py, but
        // the legacy rep is definitely not what we want and although the
        // "sorch" endpoint that's an artifact of the fancy-branch prototype
        // is closer, it's probably better if that doesn't get stabilized.
        Err(ServerError::Unsupported)
    }

    async fn jumpref_lookup(&self, _symbol: &str) -> Result<Value> {
        // Same rationale for `crossref_lookup` above.
        Err(ServerError::Unsupported)
    }

    async fn search_files(
        &self,
        _pathre: &str,
        _is_dir: bool,
        _limit: usize,
    ) -> Result<FileMatches> {
        // Not yet; see interface comment.
        Err(ServerError::Unsupported)
    }

    async fn search_identifiers(
        &self,
        _needle: &str,
        _exact_match: bool,
        _ignore_case: bool,
        _match_limit: usize,
    ) -> Result<Vec<(Ustr, Ustr)>> {
        // Same rationale as crossref_lookup.
        Err(ServerError::Unsupported)
    }

    async fn search_text(
        &self,
        _pattern: &str,
        _fold_case: bool,
        _path: &str,
        _limit: usize,
    ) -> Result<TextMatches> {
        // It's not clear we ever want to implement this.
        Err(ServerError::Unsupported)
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
        tree_name: tree_name.to_string(),
        server_base_url,
        tree_base_url,
        source_base_url,
        raw_analysis_base_url,
        search_url,
    }))
}
