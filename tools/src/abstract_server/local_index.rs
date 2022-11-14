use async_trait::async_trait;
use flate2::read::GzDecoder;
use futures_core::stream::BoxStream;
use serde_json::{from_str, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing::{trace};
use ustr::{ustr, Ustr};

use super::server_interface::{
    AbstractServer, ErrorDetails, ErrorLayer, FileMatches, HtmlFileRoot, Result,
    SearchfoxIndexRoot, ServerError, TextBounds, TextMatchInFile,
};
use super::{TextMatches, TextMatchesByFile, TreeInfo};

use crate::file_format::config::{load, TreeConfig, TreeConfigPaths};
use crate::file_format::crossref_lookup::CrossrefLookupMap;
use crate::file_format::identifiers::IdentMap;
use crate::file_format::per_file_info::FileLookupMap;

pub mod livegrep {
    tonic::include_proto!("_");
}

use livegrep::code_search_client::CodeSearchClient;
use livegrep::Query;

/// IO errors amount to a 404 for our purposes which means a sticky problem.
impl From<std::io::Error> for ServerError {
    fn from(err: std::io::Error) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::ServerLayer,
            message: err.to_string(),
        })
    }
}

impl From<tonic::Status> for ServerError {
    fn from(status: tonic::Status) -> ServerError {
        // There are gRPC codes accessible via code() but for now, especially
        // since we lack the ability to restart the server, it seems safe to
        // assume any problem will not magically fix itself.
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::ServerLayer,
            message: status.to_string(),
        })
    }
}

impl From<tonic::transport::Error> for ServerError {
    fn from(err: tonic::transport::Error) -> ServerError {
        // There are gRPC codes accessible via code() but for now, especially
        // since we lack the ability to restart the server, it seems safe to
        // assume any problem will not magically fix itself.
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::ServerLayer,
            message: err.to_string(),
        })
    }
}

/// Read newline-delimited JSON that's been gzip-compressed.
async fn read_gzipped_ndjson_from_file(path: &str) -> Result<Vec<Value>> {
    let mut f = File::open(path).await?;
    // We read the entirety to a buffer because
    // https://github.com/serde-rs/json/issues/160 suggests that the buffered
    // reader performance is likely to be much worse.
    //
    // When we want to go async here,
    // https://github.com/rust-lang/flate2-rs/pull/214 suggests that we want to
    // use the `async-compression` crate.
    let mut buffer = Vec::new();
    f.read_to_end(&mut buffer).await?;

    let mut gz = GzDecoder::new(&buffer[..]);

    let mut raw_str = String::new();
    gz.read_to_string(&mut raw_str)?;

    // let mut raw_str = String::new();
    // f.read_to_string(&mut raw_str).await?;

    raw_str
        .lines()
        .map(|s| from_str(s).map_err(|e| ServerError::from(e)))
        .collect()
}

/// Helper to ensure that our path-ish use of &str's does not ever try and do
/// something that can escape a hackily constructed path.  We probably should
/// move to using path types more directly.
fn validate_absoluteish_path(path: &str) -> Result<()> {
    if path.split("/").any(|x| x == "..") {
        Err(ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::BadInput,
            message: "All paths must be absolute-ish".to_string(),
        }))
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct LocalIndex {
    // We only hold onto the TreeConfigPaths portion of the config because the
    // git data is not `Sync`.  Specifically, the compiler says:
    //
    // "within `TreeConfig`, the trait `Sync` is not implemented for
    // `*mut libgit2_sys::git_repository`"
    //
    // When we need to do local git stuff, we will be able to accomplish this by
    // creating a new `git2::Repository` on demand from the git path.  This is
    // already done in `build-blame.rs` for its compute threads and that's
    // likely the model we should use.
    config_paths: TreeConfigPaths,
    config_repo_path: String,
    tree_name: String,
    // Note: IdentMap internally handles the identifiers db not existing
    ident_map: Option<IdentMap>,
    // But for crossref, it's on us.
    crossref_lookup_map: Option<CrossrefLookupMap>,
    file_lookup_map: FileLookupMap,
}

#[async_trait]
impl AbstractServer for LocalIndex {
    fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync> {
        Box::new(self.clone())
    }

    fn tree_info(&self) -> Result<TreeInfo> {
        Ok(TreeInfo {
            name: self.tree_name.clone(),
        })
    }

    fn translate_path(&self, root: SearchfoxIndexRoot, sf_path: &str) -> Result<String> {
        match root {
            SearchfoxIndexRoot::CompressedAnalysis => Ok(format!(
                "{}/analysis/{}.gz",
                self.config_paths.index_path, sf_path
            )),
            SearchfoxIndexRoot::ConfigRepo => Ok(format!("{}/{}", self.config_repo_path, sf_path)),
            SearchfoxIndexRoot::IndexTemplates => Ok(format!(
                "{}/templates/{}",
                self.config_paths.index_path, sf_path
            )),
            SearchfoxIndexRoot::UncompressedDirectoryListing => Ok(format!(
                "{}/dir/{}/index.html",
                self.config_paths.index_path, sf_path
            )),
        }
    }

    async fn fetch_raw_analysis(&self, sf_path: &str) -> Result<BoxStream<Value>> {
        // We normalize off any leading "/" mainly to support our test cases
        // being able to use "/" to indicate they're interested in a root dir.
        let norm_path = if sf_path.starts_with('/') {
            &sf_path[1..]
        } else {
            sf_path
        };
        // We don't want anyone trying to construct a path that escapes the
        // sub-tree.
        validate_absoluteish_path(norm_path)?;
        let full_path = format!("{}/analysis/{}.gz", self.config_paths.index_path, norm_path);
        let values = read_gzipped_ndjson_from_file(&full_path).await?;
        Ok(Box::pin(tokio_stream::iter(values)))
    }

    async fn fetch_html(&self, root: HtmlFileRoot, sf_path: &str) -> Result<String> {
        // We normalize off any leading "/" mainly to support our test cases
        // being able to use "/" to indicate they're interested in a root dir.
        let norm_path = if sf_path.starts_with('/') {
            &sf_path[1..]
        } else {
            sf_path
        };
        // We don't want anyone trying to construct a path that escapes the
        // sub-tree.
        validate_absoluteish_path(norm_path)?;
        let (full_path, is_gzipped) = match root {
            HtmlFileRoot::FormattedFile => (
                format!("{}/file/{}.gz", self.config_paths.index_path, norm_path),
                true,
            ),
            HtmlFileRoot::FormattedDir => {
                // Our tree-relative paths should not start with a slash

                // We want a trailing slash for directories, and the input is allowed
                // to do either.  The exception is that for the root directory, ""
                // is the right choice because our "no leading /" rule trumps our
                // "yes trailing /" rule for path manipulation.
                let norm_path = if norm_path == "" {
                    "".to_string()
                } else if norm_path.ends_with('/') {
                    norm_path.to_string()
                } else {
                    format!("{}/", norm_path)
                };
                (
                    format!(
                        "{}/dir/{}index.html.gz",
                        self.config_paths.index_path, norm_path
                    ),
                    true,
                )
            }
            HtmlFileRoot::FormattedTemplate => (
                format!("{}/templates/{}", self.config_paths.index_path, norm_path),
                false,
            ),
        };

        if !is_gzipped {
            let mut f = File::open(full_path).await?;
            let mut raw_str = String::new();
            f.read_to_string(&mut raw_str).await?;
            return Ok(raw_str);
        }

        let mut f = File::open(full_path).await?;
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).await?;

        // When we want to go async here,
        // https://github.com/rust-lang/flate2-rs/pull/214 suggests that we want
        // to use the `async-compression` crate.
        let mut gz = GzDecoder::new(&buffer[..]);

        let mut raw_str = String::new();
        gz.read_to_string(&mut raw_str)?;

        Ok(raw_str)
    }

    async fn crossref_lookup(&self, symbol: &str) -> Result<Value> {
        let now = Instant::now();
        let result = match &self.crossref_lookup_map {
            Some(crossref) => crossref.lookup(symbol),
            None => Ok(Value::Null),
        };
        trace!(
            duration_us = now.elapsed().as_micros() as u64,
            "crossref_lookup: {}",
            symbol
        );
        result
    }

    async fn search_files(
        &self,
        pathre: &str,
        include_dirs: bool,
        limit: usize,
    ) -> Result<FileMatches> {
        self.file_lookup_map
            .search_files(pathre, include_dirs, limit)
    }

    async fn search_identifiers(
        &self,
        needle: &str,
        exact_match: bool,
        ignore_case: bool,
        match_limit: usize,
    ) -> Result<Vec<(Ustr, Ustr)>> {
        if let Some(ident_map) = &self.ident_map {
            let now = Instant::now();
            let mut results = vec![];
            for ir in ident_map.lookup(needle, exact_match, ignore_case, match_limit) {
                results.push((ir.symbol, ir.id));
            }
            trace!(
                duration_us = now.elapsed().as_micros() as u64,
                "search_identifiers: {}",
                needle
            );
            Ok(results)
        } else {
            Ok(vec![])
        }
    }

    async fn search_text(
        &self,
        pattern: &str,
        fold_case: bool,
        path: &str,
        limit: usize,
    ) -> Result<TextMatches> {
        let now = Instant::now();

        let endpoint = format!("http://localhost:{}", self.config_paths.codesearch_port);
        trace!("search_text: connecting to {}", endpoint);

        let mut client = CodeSearchClient::connect(endpoint).await?;

        let query = tonic::Request::new(Query {
            line: pattern.into(),
            file: path.into(),
            repo: "".into(),
            tags: "".into(),
            fold_case,
            not_file: "".into(),
            not_repo: "".into(),
            not_tags: "".into(),
            // 0 falls back to the default, I believe.
            max_matches: limit as i32,
            filename_only: false,
            // 0 should pick the default of 0.
            context_lines: 0,
        });

        trace!("search_text: connected, issuing query: {}", pattern);
        let response = client.search(query).await?.into_inner();

        trace!(
            duration_us = now.elapsed().as_micros() as u64,
            "search_text: query completed: {}",
            pattern
        );

        let mut by_file: BTreeMap<String, TextMatchesByFile> = BTreeMap::new();
        for result in response.results {
            let left = result.bounds.as_ref().map_or(0, |b| b.left);
            let right = result.bounds.as_ref().map_or(0, |b| b.right);
            by_file
                .entry(result.path.to_string())
                .or_insert_with(|| {
                    let path = ustr(&result.path);
                    let path_kind = self
                        .file_lookup_map
                        .lookup_file_from_ustr(&path)
                        .map_or_else(|| ustr(""), |fi| fi.path_kind.clone());
                    TextMatchesByFile {
                        file: path,
                        path_kind,
                        matches: vec![],
                    }
                })
                .matches
                .push(TextMatchInFile {
                    line_num: result.line_number as u32,
                    bounds: TextBounds {
                        start: left,
                        end_exclusive: right,
                    },
                    line_str: result.line,
                });
        }

        Ok(TextMatches {
            by_file: by_file.into_values().collect(),
        })
    }

    async fn perform_query(&self, _q: &str) -> Result<Value> {
        // TODO: For this to work, we want to be able to directly invoke the
        // underpinnings of the web server, which entails porting router.py into
        // web-server.rs, an act which may involve building it on some of this
        // infrastructure...
        Err(ServerError::Unsupported)
    }
}

fn fab_server(
    tree_config: TreeConfig,
    tree_name: &str,
    config_repo_path: &str,
) -> Result<Box<dyn AbstractServer + Send + Sync>> {
    let ident_path = format!("{}/identifiers", tree_config.paths.index_path);
    let ident_map = IdentMap::new(&ident_path);

    let crossref_path = format!("{}/crossref", tree_config.paths.index_path);
    let crossref_extra_path = format!("{}/crossref-extra", tree_config.paths.index_path);

    let crossref_lookup_map = CrossrefLookupMap::new(&crossref_path, &crossref_extra_path);

    let file_lookup_path = format!(
        "{}/concise-per-file-info.json",
        tree_config.paths.index_path
    );

    let file_lookup_map = FileLookupMap::new(&file_lookup_path);

    Ok(Box::new(LocalIndex {
        // We don't need the blame_map and hg_map (yet)
        config_paths: tree_config.paths,
        config_repo_path: config_repo_path.to_string(),
        tree_name: tree_name.to_string(),
        ident_map,
        crossref_lookup_map,
        file_lookup_map,
    }))
}

pub fn make_local_server(
    config_path: &str,
    tree_name: &str,
) -> Result<Box<dyn AbstractServer + Send + Sync>> {
    let mut config = load(config_path, false, Some(&tree_name));
    let tree_config = match config.trees.remove(&tree_name.to_string()) {
        Some(t) => t,
        None => {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!("bad tree name: {}", &tree_name),
            }))
        }
    };

    fab_server(tree_config, tree_name, &config.config_repo_path)
}

pub fn make_all_local_servers(
    config_path: &str,
) -> Result<BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>> {
    let config = load(config_path, false, None);
    let mut servers = BTreeMap::new();
    for (tree_name, tree_config) in config.trees {
        let server = fab_server(tree_config, &tree_name, &config.config_repo_path)?;
        servers.insert(tree_name, server);
    }
    Ok(servers)
}
