use async_trait::async_trait;
use flate2::read::GzDecoder;
use futures_core::stream::BoxStream;
use serde_json::{Value, from_str};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing::trace;
use ustr::{Ustr, UstrMap, ustr};

use super::server_interface::{
    AbstractServer, ErrorDetails, ErrorLayer, FileMatches, HtmlFileRoot, Result,
    SearchfoxIndexRoot, ServerError, TextBounds, TextMatchInFile,
};
use super::{CommitInfo, TextMatches, TextMatchesByFile, TreeInfo};

use crate::blame;
use crate::cmd_pipeline::interface::split_file_key;
use crate::file_format::analysis::{read_analyses, read_source};
use crate::file_format::analysis_manglings::escape_record_symbol_commas;
use crate::file_format::bisectable_mmap::BisectableMmap;
use crate::file_format::chunked_gzip::{self, Row};
use crate::file_format::code_coverage_report;
use crate::file_format::config::{GitData, TreeConfig, TreeConfigPaths, load};
use crate::file_format::crossref::CrossrefData;
use crate::file_format::history::timeline_common::JournalVersionRef;
use crate::file_format::identifiers::IdentMap;
use crate::file_format::jumpref::{JumprefData, TreeJumprefs};
use crate::file_format::local_symbols::process_local_symbols;
use crate::file_format::per_file_info::FileLookupMap;
use crate::file_format::recency::FileRecency;
use crate::file_format::repo_data_ingestion::RepoIngestionConfig;
use crate::format::format_code;
use crate::git_ops::{RevisionCoverage, coverage_history, coverage_summary, git_time_to_chrono};
use crate::hyperblame::journals::{JournalKind, JournalReader};
use crate::hyperblame::token_history::{
    TokenHistory, TokenHistoryOptions, header_html, token_history,
};
use crate::languages::select_formatting;
use crate::source_mapping::{NotesRefs, SourceMapping, default_notes_ref};
use crate::try_delta::TryDelta;

use super::TryOverlay;
use std::sync::Arc;

pub mod livegrep {
    // Compilation gets upset about IndexSpec, CloneOptions, PathSpec, and
    // RepoSpec never being constructed in `_.rs` without this.  (It's named
    // `_.rs` because no package name is specified and this is also why we're
    // saying to include "_".)
    #![allow(dead_code)]
    tonic::include_proto!("_");
}

use livegrep::code_search_client::CodeSearchClient;
use livegrep::{Query, search_stats};

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

/// Read newline-delimited JSON that's possibly been gzip-compressed.
/// A file's length decoded: a gzipped one's (by its name) from gzip's trailer
/// (mod 2^32), else its length.
pub(crate) fn decoded_file_len(path: &str) -> Option<u64> {
    use std::io::{Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    if !path.ends_with(".gz") {
        return Some(file.metadata().ok()?.len());
    }
    file.seek(SeekFrom::End(-4)).ok()?;
    let mut trailer = [0u8; 4];
    file.read_exact(&mut trailer).ok()?;
    Some(u32::from_le_bytes(trailer) as u64)
}

async fn read_maybe_gzipped_ndjson_from_file(path: &str) -> Result<Vec<Value>> {
    let used_path = if path.ends_with(".gz") && !fs::exists(path).unwrap_or(false) {
        &path[..path.len() - 3]
    } else {
        path
    };

    let mut f = File::open(used_path).await?;

    let mut raw_str = String::new();

    // We read the entirety to a buffer because
    // https://github.com/serde-rs/json/issues/160 suggests that the buffered
    // reader performance is likely to be much worse.
    //
    // When we want to go async here,
    // https://github.com/rust-lang/flate2-rs/pull/214 suggests that we want to
    // use the `async-compression` crate.
    if used_path.ends_with(".gz") {
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).await?;
        let mut gz = GzDecoder::new(&buffer[..]);

        gz.read_to_string(&mut raw_str)?;
    } else {
        f.read_to_string(&mut raw_str).await?;
    }

    raw_str
        .lines()
        .map(|s| from_str(s).map_err(ServerError::from))
        .collect()
}

/// Helper to ensure that our path-ish use of &str's does not ever try and do
/// something that can escape a hackily constructed path.  We probably should
/// move to using path types more directly.
fn bad_input(message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::BadInput,
        message,
    })
}

/// A try push's revision, normalized (see `try_import::normalize_rev`).
fn normalized_push(push: &str) -> Result<String> {
    crate::try_import::normalize_rev(push)
        .ok_or_else(|| bad_input(format!("Not a try push's revision: {}", push)))
}

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
pub struct LocalIndex {
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
    /// Every configured tree's index path, for the tree's `linked_trees`.
    tree_index_paths: BTreeMap<String, String>,
    config_repo_path: String,
    /// The config file, for what needs the tree's whole configuration, git
    /// repositories included (ex: its try pushes' imports), which it loads
    /// then (see `tree_config_loaded`), since they aren't `Sync`.
    config_file: String,
    tree_name: String,
    // Note: IdentMap internally handles the identifiers db not existing
    ident_map: Option<IdentMap>,
    // But for crossref, it's on us.
    crossref_lookup_map: Option<BisectableMmap<CrossrefData>>,
    /// The linked trees' crossrefs (see `TreeConfigPaths::linked_trees`), with
    /// their prefixes, names, and index paths, whose data the tree's symbols
    /// with those prefixes get too (see `CrossrefData::add_linked`).
    linked_crossrefs: Vec<(String, Ustr, String, BisectableMmap<CrossrefData>)>,
    /// The tree's jumpref, and its linked trees'.
    jumprefs: TreeJumprefs,
    /// The files' history digests, for trees with histories (see
    /// `file_format::recency::FileRecency`).
    file_recency_lookup_map: Option<BisectableMmap<FileRecency>>,
    /// The files' concise information, loaded when first needed (it's big,
    /// ex: ~260 MB of JSON for firefox), from `file_lookup_path`.
    file_lookup_map: Arc<OnceLock<FileLookupMap>>,
    file_lookup_path: String,
    /// The tree's path kinds' keys and names, in display order.
    path_kinds: Vec<(Ustr, Ustr)>,
    head_info: Option<CommitInfo>,
    /// Whether its gzipped analysis files are as `fetch_raw_analysis` gives
    /// them (its `analysis-normalized` stamp; see compress-outputs.sh and
    /// the normalize-analyses tool).
    analysis_normalized: bool,
}

impl LocalIndex {
    /// `f` of the tree's git data (`tree_config_loaded`'s), or None if the
    /// tree has no coverage repo (or no git data): directory listings look up
    /// each directory's coverage (firefox's ~44k), and opening the git data
    /// opens the tree's repos and its history (~2.7 ms each time on config8).
    fn with_coverage_git<T>(&self, f: impl FnOnce(&GitData) -> Option<T>) -> Option<T> {
        self.config_paths.coverage_repo()?;
        let cfg = self.tree_config_loaded();
        f(cfg.trees.get(&self.tree_name)?.git.as_ref()?)
    }

    /// The tree's configuration, with its git repositories (see
    /// `config_file`), loaded once per thread (git repositories aren't
    /// `Sync`, but they're `Send`).
    fn tree_config_loaded(&self) -> std::rc::Rc<crate::file_format::config::Config> {
        thread_local! {
            static LOADED: std::cell::RefCell<
                HashMap<(String, String), std::rc::Rc<crate::file_format::config::Config>>,
            > = Default::default();
        }
        LOADED.with(|loaded| {
            loaded
                .borrow_mut()
                .entry((self.config_file.clone(), self.tree_name.clone()))
                .or_insert_with(|| {
                    std::rc::Rc::new(load(
                        &self.config_file,
                        false,
                        Some(&self.tree_name),
                        None,
                        None,
                    ))
                })
                .clone()
        })
    }

    fn file_lookup_map(&self) -> &FileLookupMap {
        self.file_lookup_map
            .get_or_init(|| FileLookupMap::new(&self.file_lookup_path))
    }

    fn normalize_and_validate_path<'a>(&self, sf_path: &'a str) -> Result<&'a str> {
        // We normalize off any leading "/" mainly to support our test cases
        // being able to use "/" to indicate they're interested in a root dir.
        let norm_path = sf_path.strip_prefix('/').unwrap_or(sf_path);
        // We don't want anyone trying to construct a path that escapes the
        // sub-tree.
        validate_absoluteish_path(norm_path)?;

        Ok(norm_path)
    }
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

    fn commit_info(&self) -> Result<Option<CommitInfo>> {
        Ok(self.head_info.clone())
    }

    async fn try_import_status(&self, push: &str) -> Result<crate::try_import::ImportStatus> {
        let push = normalized_push(push)?;
        let cfg = self.tree_config_loaded();
        Ok(crate::try_import::IMPORTS.status(&cfg, &self.tree_name, &push))
    }

    async fn request_try_import(&self, push: &str) -> Result<()> {
        let push = normalized_push(push)?;
        let cfg = self.tree_config_loaded();
        crate::try_import::IMPORTS
            .request(&cfg, &self.tree_name, &push)
            .map(|_| ())
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn try_push(&self, push: &str) -> Result<Option<crate::try_import::TryPush>> {
        let push = normalized_push(push)?;
        let cfg = self.tree_config_loaded();
        Ok(crate::try_import::try_push(&cfg, &self.tree_name, &push))
    }

    async fn rev_file(&self, rev: &str, path: &str) -> Result<Option<crate::format::RevFile>> {
        let cfg = self.tree_config_loaded();
        crate::format::rev_file_data(&cfg, &self.tree_name, rev, path)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn hyperblame_json(&self, rev: &str, path: &str, file: &str) -> Result<String> {
        let cfg = self.tree_config_loaded();
        crate::format::hyperblame_json(&cfg, &self.tree_name, rev, path, file)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn stack_strip_json(&self, stack: &str, rev: &str, path: &str) -> Result<String> {
        let cfg = self.tree_config_loaded();
        crate::format::stack_strip_json(&cfg, &self.tree_name, stack, rev, path)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn history(
        &self,
        rev: &str,
        path: &str,
        scope: Option<&str>,
        token: Option<usize>,
    ) -> Result<Value> {
        let cfg = self.tree_config_loaded();
        crate::format::history_data(&cfg, &self.tree_name, rev, path, scope, token)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn rev_diff(&self, rev: &str, path: &str) -> Result<Option<crate::format::RevDiff>> {
        let cfg = self.tree_config_loaded();
        crate::format::rev_diff_data(&cfg, &self.tree_name, rev, path)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn commit_page(&self, rev: &str) -> Result<Value> {
        let cfg = self.tree_config_loaded();
        crate::format::commit_page_data(&cfg, &self.tree_name, rev)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn commit_info_json(&self, revs: &str) -> Result<String> {
        let cfg = self.tree_config_loaded();
        crate::blame::get_commit_info(&cfg, &self.tree_name, revs)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn interdiff_files(&self, a: &str, b: &str) -> Result<Value> {
        let cfg = self.tree_config_loaded();
        crate::format::interdiff_files_data(&cfg, &self.tree_name, a, b)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn interdiff_file(
        &self,
        a: &str,
        b: &str,
        path: &str,
    ) -> Result<crate::format::InterdiffFile> {
        let cfg = self.tree_config_loaded();
        crate::format::interdiff_file_data(&cfg, &self.tree_name, a, b, path)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn outline_diff(
        &self,
        from: &str,
        to: &str,
        path: &str,
        all_from_lines: bool,
    ) -> Result<Value> {
        let cfg = self.tree_config_loaded();
        let diff =
            crate::format::outline_diff_data(&cfg, &self.tree_name, from, to, path, all_from_lines)
                .map_err(|message| bad_input(message.to_string()))?;
        Ok(serde_json::to_value(diff)?)
    }

    async fn landing(&self) -> Result<Value> {
        // (The whole tree's roll-ups, for its system diagrams.)
        let root = self
            .crossref_lookup(&crate::file_format::rollup::root_subsystem_sym())
            .await?;
        let data_rollups = root.as_ref().is_some_and(|data| data.data_rollup.is_some());
        let call_rollup = root.and_then(|data| data.call_rollup);
        let cfg = self.tree_config_loaded();
        crate::format::landing_data(&cfg, &self.tree_name, call_rollup, data_rollups)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn explore_keys(&self, kind: &str, keys: &str) -> Result<Value> {
        let cfg = self.tree_config_loaded();
        crate::format::explore_page_data(&cfg, &self.tree_name, kind, keys)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn map_rev(&self, kind: &str, rev: &str) -> Result<Option<String>> {
        match kind {
            "hg" => {
                let Some(git_path) = &self.config_paths.git_path else {
                    return Ok(None);
                };
                // (The system git, with SHA-1 collision detection, since
                // this works on a source repo; see `fast_import_git` for our
                // git.)
                let output = std::process::Command::new("git")
                    .arg("cinnabar")
                    .arg("hg2git")
                    .arg(rev)
                    .current_dir(git_path)
                    .output()?;
                let mapped = crate::git_ops::decode_bytes(output.stdout)
                    .trim()
                    .to_string();
                // (Unknown revisions map to zeroes.)
                Ok((output.status.success()
                    && !mapped.is_empty()
                    && mapped.bytes().any(|b| b != b'0'))
                .then_some(mapped))
            }
            "old" => {
                let cfg = self.tree_config_loaded();
                let Ok(old) = git2::Oid::from_str(rev) else {
                    return Ok(None);
                };
                Ok(cfg
                    .trees
                    .get(&self.tree_name)
                    .and_then(|tree| tree.git.as_ref())
                    .and_then(|git| git.new_rev_for_old_rev(old))
                    .map(|new| new.to_string()))
            }
            _ => Err(bad_input(format!("Unknown kind of revision: {}", kind))),
        }
    }

    async fn rev_dir(&self, rev: &str, path: &str) -> Result<Option<Value>> {
        let cfg = self.tree_config_loaded();
        crate::format::rev_dir_data(&cfg, &self.tree_name, rev, path)
            .map_err(|message| bad_input(message.to_string()))
    }

    async fn explore(&self, revs: &[String], stack_url: Option<&str>) -> Result<Value> {
        let cfg = self.tree_config_loaded();
        let git = cfg
            .trees
            .get(&self.tree_name)
            .and_then(|tree| tree.git.as_ref())
            .ok_or(ServerError::Unsupported)?;
        let mut refs = vec![];
        for rev in revs {
            let commit = git
                .repo
                .revparse_single(rev)
                .and_then(|object| object.peel_to_commit())
                .map_err(|_| bad_input(format!("No commit {}", rev)))?;
            refs.push(crate::commit_index::commit_ref(&commit));
        }
        crate::hyperblame::explore::mark_history_backouts(
            git.history.as_ref().map(|h| Path::new(&h.path)),
            &mut refs,
        );
        crate::format::explore_data(&cfg, &self.tree_name, &refs, None, stack_url)
            .map_err(|message| bad_input(message.to_string()))
    }

    fn tree_config(&self) -> Result<Value> {
        let mut config = serde_json::to_value(&self.config_paths)?;
        if let Some(config) = config.as_object_mut() {
            config.remove("private");
        }
        Ok(config)
    }

    fn cache_identity(&self) -> Option<String> {
        // (Its `all-files`, which indexing rewrites, as the sidecars' stamp.)
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(format!("{}/all-files", self.config_paths.index_path)).ok()?;
        Some(format!(
            "{} {} {} {}",
            self.tree_name,
            meta.ino(),
            meta.mtime(),
            meta.len()
        ))
    }

    fn rev_changed_lines(&self, spec: &str) -> Result<crate::dataflow::slice::ChangedLines> {
        let bad = |message: String| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message,
            })
        };
        let path = self
            .config_paths
            .git_path
            .as_deref()
            .ok_or(ServerError::Unsupported)?;
        let repo = git2::Repository::open(path).map_err(|err| bad(err.message().to_string()))?;
        // (The indexed revision's files.)
        let head = self
            .head_info
            .as_ref()
            .map_or("HEAD".to_string(), |info| info.rev.clone());
        crate::rev_lines::rev_changed_lines(&repo, &head, spec).map_err(bad)
    }

    fn file_facet_info(&self, path: &str) -> Option<(Ustr, Option<Ustr>)> {
        self.file_lookup_map()
            .lookup_file_from_str(path)
            .map(|info| (info.path_kind, info.subsystem))
    }

    fn path_kinds(&self) -> Vec<(Ustr, Ustr)> {
        self.path_kinds.clone()
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
            SearchfoxIndexRoot::IndexPages => Ok(format!(
                "{}/pages/{}",
                self.config_paths.index_path, sf_path
            )),
            SearchfoxIndexRoot::UncompressedDirectoryListing => Ok(format!(
                "{}/dir/{}/index.html",
                self.config_paths.index_path, sf_path
            )),
        }
    }

    async fn coverage_history(&self, sf_path: &str) -> Result<Option<Vec<RevisionCoverage>>> {
        let norm_path = self.normalize_and_validate_path(sf_path)?;
        Ok(self.with_coverage_git(|git| coverage_history(Some(git), norm_path)))
    }

    async fn coverage_summary(
        &self,
        sf_path: &str,
    ) -> Result<Option<code_coverage_report::NodeMetadata>> {
        let norm_path = self.normalize_and_validate_path(sf_path)?;
        Ok(self.with_coverage_git(|git| {
            let commit = git.repo.head().ok()?.peel_to_commit().ok()?;
            let coverage_rev = format!("refs/tags/reverse/all/all/{}", commit.id());
            coverage_summary(Some(git), &coverage_rev, norm_path)
        }))
    }

    async fn fetch_raw_analysis<'a>(&self, sf_path: &str) -> Result<BoxStream<'a, Value>> {
        let norm_path = self.normalize_and_validate_path(sf_path)?;
        let full_path = self.translate_path(SearchfoxIndexRoot::CompressedAnalysis, norm_path)?;
        let mut values = read_maybe_gzipped_ndjson_from_file(&full_path).await?;
        // (As `read_analyses` reads them, unless they're written so; see
        // `escape_record_symbol_commas` and `normalized_analysis_file`.)
        if !self.analysis_normalized {
            values.iter_mut().for_each(escape_record_symbol_commas);
            process_local_symbols(&mut values);
        }
        Ok(Box::pin(tokio_stream::iter(values)))
    }

    fn normalized_analysis_file(&self, sf_path: &str) -> Option<String> {
        if !self.analysis_normalized {
            return None;
        }
        let norm_path = self.normalize_and_validate_path(sf_path).ok()?;
        let full_path = self
            .translate_path(SearchfoxIndexRoot::CompressedAnalysis, norm_path)
            .ok()?;
        Path::new(&full_path).is_file().then_some(full_path)
    }

    fn analysis_file_len(&self, sf_path: &str) -> Option<u64> {
        let norm_path = self.normalize_and_validate_path(sf_path).ok()?;
        let full_path = self
            .translate_path(SearchfoxIndexRoot::CompressedAnalysis, norm_path)
            .ok()?;
        // (Or uncompressed, as `read_maybe_gzipped_ndjson_from_file` reads it.)
        decoded_file_len(&full_path).or_else(|| decoded_file_len(full_path.strip_suffix(".gz")?))
    }

    async fn fetch_raw_source(&self, sf_path: &str) -> Result<String> {
        let norm_path = self.normalize_and_validate_path(sf_path)?;
        let full_path = if norm_path.starts_with("__GENERATED__/") {
            format!(
                "{}/{}",
                self.config_paths.objdir_path,
                norm_path.strip_prefix("__GENERATED__/").unwrap()
            )
        } else {
            format!("{}/{}", self.config_paths.files_path, norm_path)
        };

        let mut f = File::open(full_path).await?;
        let mut raw_str = String::new();
        f.read_to_string(&mut raw_str).await?;
        Ok(raw_str)
    }

    async fn fetch_history_journal(
        &self,
        kind: JournalKind,
        target: &str,
        source_rev: Option<&str>,
        expand: bool,
    ) -> Result<Vec<Value>> {
        let problem = |layer: ErrorLayer, message: String| {
            ServerError::StickyProblem(ErrorDetails { layer, message })
        };
        let Some(history_path) = &self.config_paths.history_path else {
            return Err(problem(
                ErrorLayer::ConfigLayer,
                "this tree has no history".to_string(),
            ));
        };
        let repo = git2::Repository::open(format!("{}/timeline", history_path))
            .map_err(|e| problem(ErrorLayer::DataLayer, e.to_string()))?;
        let branch_ref = self
            .config_paths
            .git_branch
            .as_ref()
            .map_or("HEAD".to_string(), |branch| {
                format!("refs/heads/{}", branch)
            });
        let timeline_rev = match source_rev {
            // The timeline repo's notes map source revisions to timeline commits;
            // see `source_mapping`.
            Some(rev) => {
                let rev = git2::Oid::from_str(rev)
                    .map_err(|e| problem(ErrorLayer::BadInput, e.to_string()))?;
                let notes_refs = NotesRefs {
                    write: default_notes_ref(&repo, &branch_ref),
                    read: vec![],
                };
                SourceMapping::open(&repo, &notes_refs)
                    .lookup(&repo, rev)
                    .ok_or_else(|| {
                        problem(
                            ErrorLayer::BadInput,
                            format!("no history for source revision {}", rev),
                        )
                    })?
            }
            None => repo
                .refname_to_id(&branch_ref)
                .map_err(|e| problem(ErrorLayer::DataLayer, e.to_string()))?,
        };
        let version = JournalVersionRef {
            timeline_rev: timeline_rev.to_string(),
            path: kind.journal_path(target),
        };
        JournalReader::new(&repo)
            .records_json(kind, &version, expand)
            .map_err(|e| problem(ErrorLayer::DataLayer, e))
    }

    async fn token_history(
        &self,
        token: &str,
        opts: TokenHistoryOptions,
    ) -> Result<Option<TokenHistory>> {
        // (Trees without the token-centric history have none.)
        let cfg = self.tree_config_loaded();
        let Some(git) = cfg
            .trees
            .get(&self.tree_name)
            .and_then(|tree| tree.git.as_ref())
        else {
            return Err(ServerError::Unsupported);
        };
        let Some(history) = &git.history else {
            return Err(ServerError::Unsupported);
        };
        // (The view's records' commits' headers, from the tree's repo.)
        let header = |rev: &str| {
            let commit = git2::Oid::from_str(rev)
                .ok()
                .and_then(|oid| git.repo.find_commit(oid).ok())?;
            Some(header_html(commit.message().ok()?))
        };
        token_history(history, token, opts, &header).map_err(|message| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                message,
            })
        })
    }

    async fn fetch_formatted_lines(&self, sf_path: &str) -> Result<(Vec<String>, String)> {
        let norm_path = self.normalize_and_validate_path(sf_path)?;
        let source = self.fetch_raw_source(sf_path).await?;
        let analysis_path =
            self.translate_path(SearchfoxIndexRoot::CompressedAnalysis, norm_path)?;
        let analysis = read_analyses(&[analysis_path], &mut read_source);

        let (raw_lines, sym_json) = format_code(
            None,
            &self.jumprefs,
            select_formatting(sf_path),
            sf_path,
            source.as_str(),
            &analysis,
        );

        let lines = raw_lines.into_iter().map(|line| line.line).collect();

        Ok((lines, sym_json))
    }

    async fn fetch_html_lines(
        &self,
        requests: Vec<(Ustr, BTreeSet<u32>)>,
    ) -> Result<UstrMap<HashMap<u32, Row>>> {
        let mut files = Vec::with_capacity(requests.len());
        for (key, lines) in requests {
            // (A linked tree's file's rows are in its index; see `file_key`.)
            let (tree, path) = split_file_key(&key);
            let index_path = match tree {
                None => &self.config_paths.index_path,
                Some(tree) => match self.linked_crossrefs.iter().find(|(_, t, _, _)| t == tree) {
                    Some((_, _, index_path, _)) => index_path,
                    None => continue,
                },
            };
            let norm_path = self.normalize_and_validate_path(path)?;
            let gz_path = format!("{}/file/{}.gz", index_path, norm_path);
            files.push((key, gz_path, lines));
        }
        tokio::task::spawn_blocking(move || read_rows_of_files(files)).await?
    }

    async fn fetch_html(&self, root: HtmlFileRoot, sf_path: &str) -> Result<String> {
        let norm_path = self.normalize_and_validate_path(sf_path)?;
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
                let norm_path = if norm_path.is_empty() {
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

    async fn crossref_lookup(&self, symbol: &str) -> Result<Option<CrossrefData>> {
        let now = Instant::now();
        let mut result = match &self.crossref_lookup_map {
            Some(crossref) => crossref.lookup(symbol),
            None => Ok(None),
        };
        for (prefix, tree, _, crossref) in &self.linked_crossrefs {
            if !symbol.starts_with(prefix.as_str()) {
                continue;
            }
            if let Ok(own) = &mut result
                && let Ok(Some(linked)) = crossref.lookup(symbol)
            {
                own.get_or_insert_with(CrossrefData::default)
                    .add_linked(linked, *tree);
            }
        }
        trace!(
            duration_us = now.elapsed().as_micros() as u64,
            "crossref_lookup: {}", symbol
        );
        result
    }

    async fn linked_crossref_lookup(
        &self,
        tree: &str,
        symbol: &str,
    ) -> Result<Option<CrossrefData>> {
        let Some((_, tree, _, crossref)) =
            self.linked_crossrefs.iter().find(|(_, t, _, _)| t == tree)
        else {
            return Ok(None);
        };
        let mut data = crossref.lookup(symbol)?;
        if let Some(data) = &mut data {
            data.tag_tree(*tree);
        }
        Ok(data)
    }

    async fn file_recency_lookup(&self, path: &str) -> Result<Option<FileRecency>> {
        match &self.file_recency_lookup_map {
            Some(file_recency) => file_recency.lookup(path),
            None => Ok(None),
        }
    }

    async fn jumpref_lookup(&self, symbol: &str) -> Result<Option<JumprefData>> {
        let now = Instant::now();
        let result = self.jumprefs.lookup(symbol);
        trace!(
            duration_us = now.elapsed().as_micros() as u64,
            "jumpref_lookup: {}", symbol
        );
        result
    }

    async fn search_files(
        &self,
        pathre: &str,
        not_pathre: &str,
        include_dirs: bool,
        limit: usize,
    ) -> Result<FileMatches> {
        self.file_lookup_map()
            .search_files(pathre, not_pathre, include_dirs, limit)
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
                result_count = results.len(),
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
        not_path: &str,
        limit: usize,
    ) -> Result<TextMatches> {
        let now = Instant::now();

        let endpoint = format!("http://localhost:{}", self.config_paths.codesearch_port);
        trace!("search_text: connecting to {}", endpoint);

        let mut client = CodeSearchClient::connect(endpoint).await?;

        // Before multiple paths were allowed, an empty path constraint allowed
        // us to skip the match; now if we pass an empty path in a vec, that
        // will fail to match, so we want to pass an empty vec.
        let use_path = if path.is_empty() {
            vec![]
        } else {
            vec![path.into()]
        };

        let query = tonic::Request::new(Query {
            line: pattern.into(),
            file: use_path,
            repo: "".into(),
            tags: "".into(),
            fold_case,
            not_file: if not_path.is_empty() {
                vec![]
            } else {
                vec![not_path.into()]
            },
            not_repo: "".into(),
            not_tags: "".into(),
            // 0 falls back to the default, I believe.
            max_matches: limit as i32,
            filename_only: false,
            // 0 should pick the default of 0.
            context_lines: 0,
        });

        trace!("search_text: connected, issuing query: {}", pattern);
        let response = match client.search(query).await {
            Ok(response) => response.into_inner(),
            // (What codesearch says of a pattern it won't run, too wide or
            // too big; see `cmd_search_text::MAX_PATTERN_WIDTH`.)
            Err(status)
                if status.code() == tonic::Code::InvalidArgument
                    && status.message() == "Parse error" =>
            {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::BadInput,
                    message: format!(
                        "The text search can't run this regular expression: it can match more than {} characters, or it's too complex.",
                        crate::cmd_pipeline::cmd_search_text::MAX_PATTERN_WIDTH
                    ),
                }));
            }
            Err(status) => return Err(status.into()),
        };

        trace!(
            duration_us = now.elapsed().as_micros() as u64,
            result_count = response.results.len(),
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
                        .file_lookup_map()
                        .lookup_file_from_ustr(&path)
                        .map_or_else(|| ustr(""), |fi| fi.path_kind);
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

        let exit_reason = response
            .stats
            .as_ref()
            .map(|stats| stats.exit_reason())
            .unwrap_or_default();
        Ok(TextMatches {
            by_file: by_file.into_values().collect(),
            limit_hit: exit_reason == search_stats::ExitReason::MatchLimit,
            timed_out: exit_reason == search_stats::ExitReason::Timeout,
            pattern: None,
        })
    }

    fn try_overlay(&self, head: &str) -> Option<Box<dyn AbstractServer + Send + Sync>> {
        let delta = TryDelta::open(&self.config_paths.index_path, head)?;
        Some(Box::new(TryOverlay::new(
            self.clonify(),
            Arc::new(delta),
            self.config_paths.git_path.clone(),
            self.jumprefs.clone(),
        )))
    }

    async fn perform_query(&self, _q: &str) -> Result<Value> {
        // TODO: For this to work, we want to be able to directly invoke the
        // underpinnings of the web server, which entails porting router.py into
        // web-server.rs, an act which may involve building it on some of this
        // infrastructure...
        Err(ServerError::Unsupported)
    }
}

/// The most threads reading the lines of rendered files for a query (see
/// `fetch_html_lines`).
const MAX_HTML_LINES_THREADS: usize = 8;

/// The rows of lines of the rendered files at paths (see
/// `chunked_gzip::read_rows`), by path and line, several files at a time,
/// since big queries' results are in many files (ex: 1477 for "nsIPrincipal"
/// on firefox).
fn read_rows_of_files(
    files: Vec<(Ustr, String, BTreeSet<u32>)>,
) -> Result<UstrMap<HashMap<u32, Row>>> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_HTML_LINES_THREADS)
        .min(files.len())
        .max(1);
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(files.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some((path, gz_path, lines)) =
                    files.get(next.fetch_add(1, Ordering::Relaxed))
                {
                    let rows = chunked_gzip::read_rows(Path::new(gz_path), lines);
                    results.lock().unwrap().push((*path, rows));
                }
            });
        }
    });
    let mut rows = UstrMap::default();
    for (path, file_rows) in results.into_inner().unwrap() {
        rows.insert(path, file_rows?);
    }
    Ok(rows)
}

fn fab_server(
    tree_config: TreeConfig,
    tree_name: &str,
    config_file: &str,
    config_repo_path: &str,
    tree_index_paths: &BTreeMap<String, String>,
    per_file_info_toml: Option<String>,
) -> Result<Box<dyn AbstractServer + Send + Sync>> {
    let path_kinds = per_file_info_toml
        .and_then(|toml| toml::from_str::<RepoIngestionConfig>(&toml).ok())
        .map_or_else(Vec::new, |config| {
            let mut kinds: Vec<_> = config.pathkind.into_iter().collect();
            kinds.sort_by_key(|(_, kind)| kind.sort_order);
            kinds
                .into_iter()
                .map(|(key, kind)| (key, kind.name))
                .collect()
        });
    let ident_path = format!("{}/identifiers", tree_config.paths.index_path);
    let ident_map = IdentMap::new(&ident_path);

    let crossref_path = format!("{}/crossref", tree_config.paths.index_path);
    let crossref_extra_path = format!("{}/crossref-extra", tree_config.paths.index_path);

    let crossref_lookup_map = BisectableMmap::new(&crossref_path, &crossref_extra_path);

    let jumprefs = TreeJumprefs::open(&tree_config.paths, tree_index_paths);
    let linked_crossrefs = tree_config
        .paths
        .linked_trees
        .iter()
        .filter_map(|(prefix, tree)| {
            let index_path = tree_index_paths.get(tree)?;
            // (A linked tree which isn't indexed (yet) has no data.)
            let (inline, extra) = (
                format!("{}/crossref", index_path),
                format!("{}/crossref-extra", index_path),
            );
            if !Path::new(&inline).exists() || !Path::new(&extra).exists() {
                return None;
            }
            let crossref = BisectableMmap::new(&inline, &extra)?;
            Some((prefix.clone(), ustr(tree), index_path.clone(), crossref))
        })
        .collect();

    // (Only trees with histories have these, and `BisectableMmap::new` expects
    // them.)
    let file_recency_path = format!("{}/file-recency", tree_config.paths.index_path);
    let file_recency_extra_path = format!("{}/file-recency-extra", tree_config.paths.index_path);
    let file_recency_lookup_map =
        if Path::new(&file_recency_path).exists() && Path::new(&file_recency_extra_path).exists() {
            BisectableMmap::new(&file_recency_path, &file_recency_extra_path)
        } else {
            None
        };

    let file_lookup_path = format!(
        "{}/concise-per-file-info.json",
        tree_config.paths.index_path
    );

    let head_info = match tree_config.get_git() {
        Ok(git) => match git.repo.head() {
            Ok(r) => match r.target() {
                Some(rev) => {
                    let commit = git.repo.find_commit(rev).unwrap();
                    let header = blame::commit_header(&commit).unwrap();
                    let date = git_time_to_chrono(commit.time());
                    Some(CommitInfo {
                        rev: rev.to_string(),
                        header,
                        summary: commit
                            .summary()
                            .ok()
                            .flatten()
                            .unwrap_or_default()
                            .to_string(),
                        date,
                    })
                }
                None => None,
            },
            Err(_) => None,
        },
        Err(_) => None,
    };

    let analysis_normalized = Path::new(&tree_config.paths.index_path)
        .join("analysis-normalized")
        .is_file();
    Ok(Box::new(LocalIndex {
        // We don't need the blame and hg maps (yet)
        config_paths: tree_config.paths,
        tree_index_paths: tree_index_paths.clone(),
        config_repo_path: config_repo_path.to_string(),
        config_file: config_file.to_string(),
        tree_name: tree_name.to_string(),
        ident_map,
        crossref_lookup_map,
        linked_crossrefs,
        jumprefs,
        file_recency_lookup_map,
        file_lookup_map: Arc::new(OnceLock::new()),
        file_lookup_path,
        path_kinds,
        head_info,
        analysis_normalized,
    }))
}

pub fn make_local_server(
    config_path: &str,
    tree_name: &str,
) -> Result<Box<dyn AbstractServer + Send + Sync>> {
    let mut config = load(config_path, false, Some(tree_name), None, None);
    let tree_config = match config.trees.remove(tree_name) {
        Some(t) => t,
        None => {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!("bad tree name: {}", &tree_name),
            }));
        }
    };

    let per_file_info_toml = config
        .read_tree_config_file_with_default("per-file-info.toml")
        .ok();
    fab_server(
        tree_config,
        tree_name,
        &config.config_file,
        &config.config_repo_path,
        &config.tree_index_paths,
        per_file_info_toml,
    )
}

pub fn make_all_local_servers(
    config_path: &str,
) -> Result<BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>> {
    let config = load(config_path, false, None, None, None);
    // (The config repo's, so the same for every tree.)
    let per_file_info_toml = config
        .read_tree_config_file_with_default("per-file-info.toml")
        .ok();
    let mut servers = BTreeMap::new();
    for (tree_name, tree_config) in config.trees {
        let server = fab_server(
            tree_config,
            &tree_name,
            &config.config_file,
            &config.config_repo_path,
            &config.tree_index_paths,
            per_file_info_toml.clone(),
        )?;
        servers.insert(tree_name, server);
    }
    Ok(servers)
}
