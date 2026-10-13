//! A tree as a try push changes it (see `try_delta`): the tree's index, with
//! the push's semantic delta on top, for queries in the push's context (see
//! pipeline-server's `rev`) and its head's pages.
//!
//! - Symbols' crossref data is the tree's with the push's patches, and their
//!   jumprefs the push's if it changes them.
//! - Identifier searches find the identifiers it adds too.
//! - Text searches find the base's matches in files it doesn't change, and
//!   matches in its versions of the files it changes or adds.
//! - File searches find the files it adds, and not those it removes.
//! - The files it changes, and those whose analysis it changes, have its
//!   sources, analysis, and rendered lines.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use futures_core::stream::BoxStream;
use serde_json::Value;
use ustr::{Ustr, UstrMap, ustr};

use super::server_interface::{
    AbstractServer, CommitInfo, ErrorDetails, ErrorLayer, FileMatch, FileMatches, HtmlFileRoot,
    Result, SearchfoxIndexRoot, ServerError, TextBounds, TextMatchInFile, TextMatches,
    TextMatchesByFile, TreeInfo,
};
use crate::blame;
use crate::file_format::analysis::{AnalysisSource, WithLocation, read_analysis, read_source};
use crate::file_format::chunked_gzip::{Row, extract_rows};
use crate::file_format::code_coverage_report;
use crate::file_format::crossref::CrossrefData;
use crate::file_format::jumpref::{JumprefData, TreeJumprefs};
use crate::file_format::recency::FileRecency;
use crate::file_format::repo_data_ingestion::ConcisePerFileInfo;
use crate::format::{format_code, source_rows_html};
use crate::git_ops::{RevisionCoverage, git_time_to_chrono};
use crate::hyperblame::journals::JournalKind;
use crate::languages::select_formatting;
use crate::try_delta::{FileChange, TryDelta};

fn not_found(path: &str) -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::DataLayer,
        message: format!("The try push removes {}", path),
    })
}

pub struct TryOverlay {
    base: Box<dyn AbstractServer + Send + Sync>,
    delta: Arc<TryDelta>,
    /// The tree's git repository, which has the head's commit.
    git_path: Option<String>,
    /// The tree's jumprefs, with the push's (see `TreeJumprefs::delta`).
    jumprefs: TreeJumprefs,
}

impl TryOverlay {
    pub fn new(
        base: Box<dyn AbstractServer + Send + Sync>,
        delta: Arc<TryDelta>,
        git_path: Option<String>,
        mut jumprefs: TreeJumprefs,
    ) -> Self {
        jumprefs.delta = delta.jumprefs();
        TryOverlay {
            base,
            delta,
            git_path,
            jumprefs,
        }
    }

    fn repo(&self) -> Result<git2::Repository> {
        let path = self.git_path.as_deref().ok_or(ServerError::Unsupported)?;
        git2::Repository::open(path).map_err(|err| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::ServerLayer,
                message: err.to_string(),
            })
        })
    }

    /// The file's contents at the head.
    fn head_source(&self, path: &str) -> Result<String> {
        if let Some(file) = self.delta.generated_source(path) {
            return std::fs::read_to_string(&file).map_err(|err| {
                ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::DataLayer,
                    message: format!("{}: {}", file.display(), err),
                })
            });
        }
        let repo = self.repo()?;
        let blob = repo
            .revparse_single(&self.delta.meta.head)
            .and_then(|object| object.peel_to_tree())
            .and_then(|tree| tree.get_path(std::path::Path::new(path)))
            .and_then(|entry| entry.to_object(&repo))
            .and_then(|object| object.peel_to_blob())
            .map_err(|err| {
                ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::DataLayer,
                    message: format!("{} at {}: {}", path, self.delta.meta.head, err),
                })
            })?;
        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    /// Whether the push changes the file's contents or analysis, so that its
    /// lines are rendered here rather than the base's.
    fn renders(&self, path: &str) -> bool {
        matches!(
            self.delta.change(path),
            Some(FileChange::Added | FileChange::Modified)
        ) || self.delta.meta.analyzed.contains_key(path)
    }

    /// The file's source records at the head: the push's if it changes them,
    /// else the base's.
    fn head_sources(&self, path: &str) -> Vec<WithLocation<Vec<AnalysisSource>>> {
        let file = match self.delta.analysis_file(path) {
            Some(file) => file.to_string_lossy().into_owned(),
            // (The push changes the file, and its analysis doesn't have it,
            // ex: Rust, or C++ the platform doesn't build: none, rather than
            // the base's, at its lines.)
            None if self.delta.change(path).is_some() => return vec![],
            None => match self
                .base
                .translate_path(SearchfoxIndexRoot::CompressedAnalysis, path)
            {
                Ok(file) => file,
                Err(_) => return vec![],
            },
        };
        if !std::path::Path::new(&file).exists() {
            return vec![];
        }
        read_analysis(&file, &mut read_source)
    }

    /// The source text of the file at the head (changed files' from git).
    fn source_at_head<'a>(&'a self, path: &'a str) -> impl Future<Output = Result<String>> + 'a {
        async move {
            match self.delta.change(path) {
                Some(FileChange::Added | FileChange::Modified) => self.head_source(path),
                Some(FileChange::Removed) => Err(not_found(path)),
                None => self.base.fetch_raw_source(path).await,
            }
        }
    }
}

#[async_trait]
impl AbstractServer for TryOverlay {
    fn clonify(&self) -> Box<dyn AbstractServer + Send + Sync> {
        Box::new(TryOverlay {
            base: self.base.clonify(),
            delta: self.delta.clone(),
            git_path: self.git_path.clone(),
            jumprefs: self.jumprefs.clone(),
        })
    }

    fn tree_info(&self) -> Result<TreeInfo> {
        self.base.tree_info()
    }

    fn commit_info(&self) -> Result<Option<CommitInfo>> {
        let repo = self.repo()?;
        let Ok(commit) = repo
            .revparse_single(&self.delta.meta.head)
            .and_then(|object| object.peel_to_commit())
        else {
            return Ok(None);
        };
        Ok(Some(CommitInfo {
            rev: commit.id().to_string(),
            header: blame::commit_header(&commit).unwrap_or_default(),
            date: git_time_to_chrono(commit.time()),
        }))
    }

    fn file_facet_info(&self, path: &str) -> Option<(Ustr, Option<Ustr>)> {
        self.base.file_facet_info(path)
    }

    fn path_kinds(&self) -> Vec<(Ustr, Ustr)> {
        self.base.path_kinds()
    }

    fn translate_path(&self, root: SearchfoxIndexRoot, sf_path: &str) -> Result<String> {
        self.base.translate_path(root, sf_path)
    }

    async fn coverage_history(&self, sf_path: &str) -> Result<Option<Vec<RevisionCoverage>>> {
        self.base.coverage_history(sf_path).await
    }

    async fn coverage_summary(
        &self,
        sf_path: &str,
    ) -> Result<Option<code_coverage_report::NodeMetadata>> {
        self.base.coverage_summary(sf_path).await
    }

    async fn fetch_raw_analysis<'a>(&self, sf_path: &str) -> Result<BoxStream<'a, Value>> {
        let path = sf_path.strip_prefix('/').unwrap_or(sf_path);
        let Some(file) = self.delta.analysis_file(path) else {
            // (Like `head_sources`.)
            if self.delta.change(path).is_some() {
                return Ok(Box::pin(tokio_stream::iter(vec![])));
            }
            return self.base.fetch_raw_analysis(sf_path).await;
        };
        let values = read_analysis(&file.to_string_lossy(), &mut |value, _, _| Some(value))
            .into_iter()
            .flat_map(|with_loc| with_loc.data)
            .collect::<Vec<Value>>();
        Ok(Box::pin(tokio_stream::iter(values)))
    }

    async fn fetch_raw_source(&self, sf_path: &str) -> Result<String> {
        let path = sf_path.strip_prefix('/').unwrap_or(sf_path);
        self.source_at_head(path).await
    }

    async fn fetch_formatted_lines(&self, sf_path: &str) -> Result<(Vec<String>, String)> {
        let path = sf_path.strip_prefix('/').unwrap_or(sf_path);
        if !self.renders(path) {
            return self.base.fetch_formatted_lines(sf_path).await;
        }
        let source = self.source_at_head(path).await?;
        let sources = self.head_sources(path);
        let (lines, sym_json) = format_code(
            None,
            &self.jumprefs,
            select_formatting(path),
            path,
            &source,
            &sources,
        );
        Ok((lines.into_iter().map(|line| line.line).collect(), sym_json))
    }

    async fn fetch_html(&self, root: HtmlFileRoot, sf_path: &str) -> Result<String> {
        let path = sf_path.strip_prefix('/').unwrap_or(sf_path);
        if root != HtmlFileRoot::FormattedFile || !self.renders(path) {
            return self.base.fetch_html(root, sf_path).await;
        }
        let source = self.source_at_head(path).await?;
        let sources = self.head_sources(path);
        Ok(source_rows_html(&self.jumprefs, path, &source, &sources).0)
    }

    async fn fetch_html_lines(
        &self,
        requests: Vec<(Ustr, BTreeSet<u32>)>,
    ) -> Result<UstrMap<HashMap<u32, Row>>> {
        let (rendered, rest): (Vec<_>, Vec<_>) = requests
            .into_iter()
            .partition(|(path, _)| self.renders(path));
        let mut rows = self.base.fetch_html_lines(rest).await?;
        for (path, lines) in rendered {
            let html = self.fetch_html(HtmlFileRoot::FormattedFile, &path).await?;
            rows.insert(path, extract_rows(&html, &lines));
        }
        Ok(rows)
    }

    async fn crossref_lookup(&self, symbol: &str) -> Result<Option<CrossrefData>> {
        let base = self.base.crossref_lookup(symbol).await?;
        match self.delta.crossref_patch(symbol)? {
            Some(patch) => Ok(patch.apply(base)),
            None => Ok(base),
        }
    }

    async fn linked_crossref_lookup(
        &self,
        tree: &str,
        symbol: &str,
    ) -> Result<Option<CrossrefData>> {
        self.base.linked_crossref_lookup(tree, symbol).await
    }

    async fn jumpref_lookup(&self, symbol: &str) -> Result<Option<JumprefData>> {
        self.jumprefs.lookup(symbol)
    }

    async fn file_recency_lookup(&self, path: &str) -> Result<Option<FileRecency>> {
        self.base.file_recency_lookup(path).await
    }

    async fn search_files(
        &self,
        pathre: &str,
        include_dirs: bool,
        limit: usize,
    ) -> Result<FileMatches> {
        let mut matches = self.base.search_files(pathre, include_dirs, limit).await?;
        matches
            .file_matches
            .retain(|file| self.delta.change(&file.path) != Some(FileChange::Removed));
        let re = regex::Regex::new(pathre)?;
        for (path, change) in &self.delta.meta.changed {
            if *change == FileChange::Added && re.is_match(path) {
                matches.file_matches.push(FileMatch {
                    path: ustr(path),
                    concise: ConcisePerFileInfo {
                        path_kind: ustr(""),
                        is_dir: false,
                        file_size: 0,
                        bugzilla_component: None,
                        subsystem: None,
                        tags: vec![],
                        description: None,
                        info: Value::Null,
                        coverage: None,
                    },
                });
            }
        }
        matches.file_matches.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
        if matches.file_matches.len() > limit {
            matches.file_matches.truncate(limit);
            matches.limit_hit = true;
        }
        Ok(matches)
    }

    async fn search_identifiers(
        &self,
        needle: &str,
        exact_match: bool,
        ignore_case: bool,
        match_limit: usize,
    ) -> Result<Vec<(Ustr, Ustr)>> {
        let mut results = self
            .base
            .search_identifiers(needle, exact_match, ignore_case, match_limit)
            .await?;
        if let Some(identifiers) = self.delta.identifiers() {
            for ir in identifiers.lookup(needle, exact_match, ignore_case, match_limit) {
                if !results.contains(&(ir.symbol, ir.id)) {
                    results.push((ir.symbol, ir.id));
                }
            }
        }
        results.truncate(match_limit);
        Ok(results)
    }

    async fn search_text(
        &self,
        pattern: &str,
        fold_case: bool,
        path: &str,
        limit: usize,
    ) -> Result<TextMatches> {
        let mut matches = self.base.search_text(pattern, fold_case, path, limit).await?;
        // (The push's versions of the files it changes replace the base's.)
        matches
            .by_file
            .retain(|file| self.delta.change(&file.file).is_none());
        let re = regex::RegexBuilder::new(pattern)
            .case_insensitive(fold_case)
            .build()?;
        let path_re = if path.is_empty() {
            None
        } else {
            Some(regex::Regex::new(path)?)
        };
        let mut count: usize = matches.by_file.iter().map(|file| file.matches.len()).sum();
        let mut by_path: BTreeMap<String, TextMatchesByFile> = matches
            .by_file
            .drain(..)
            .map(|file| (file.file.to_string(), file))
            .collect();
        for (file, change) in &self.delta.meta.changed {
            if *change == FileChange::Removed
                || path_re.as_ref().is_some_and(|re| !re.is_match(file))
            {
                continue;
            }
            let Ok(source) = self.head_source(file) else {
                continue;
            };
            let mut file_matches = vec![];
            for (i, line) in source.lines().enumerate() {
                if count >= limit {
                    matches.limit_hit = true;
                    break;
                }
                if let Some(m) = re.find(line) {
                    file_matches.push(TextMatchInFile {
                        line_num: i as u32 + 1,
                        bounds: TextBounds {
                            start: m.start() as i32,
                            end_exclusive: m.end() as i32,
                        },
                        line_str: line.to_string(),
                    });
                    count += 1;
                }
            }
            if !file_matches.is_empty() {
                let path_kind = self
                    .base
                    .file_facet_info(file)
                    .map_or_else(|| ustr(""), |(kind, _)| kind);
                by_path.insert(
                    file.clone(),
                    TextMatchesByFile {
                        file: ustr(file),
                        path_kind,
                        matches: file_matches,
                    },
                );
            }
        }
        matches.by_file = by_path.into_values().collect();
        Ok(matches)
    }

    async fn perform_query(&self, q: &str) -> Result<Value> {
        self.base.perform_query(q).await
    }

    fn try_delta_meta(&self) -> Option<crate::try_delta::TryDeltaMeta> {
        Some(self.delta.meta.clone())
    }

    async fn fetch_history_journal(
        &self,
        kind: JournalKind,
        target: &str,
        source_rev: Option<&str>,
        expand: bool,
    ) -> Result<Vec<Value>> {
        self.base
            .fetch_history_journal(kind, target, source_rev, expand)
            .await
    }
}
