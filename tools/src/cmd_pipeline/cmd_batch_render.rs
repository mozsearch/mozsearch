use std::collections::HashMap;

use async_trait::async_trait;
use clap::Args;
use ustr::Ustr;

use super::interface::{PipelineCommand, PipelineValues};
use crate::{
    abstract_server::{
        AbstractServer, ErrorDetails, ErrorLayer, FileMatches, Result, SearchfoxIndexRoot,
        ServerError,
    },
    file_format::config::{self, SearchEndpoint},
    file_utils::write_file_ensuring_parent_dir,
    format::{add_coverage_panel_item, tests_firefox_dev_panel_item},
    output::PanelSection,
    templating::builder::build_and_parse_dir_listing,
};

#[derive(Debug, Args)]
pub struct BatchRender {
    /// Preconfigured rendering task.  This could be an enum or sub-command, but
    /// for now we're just going for strings.
    #[clap(value_parser)]
    task: String,
}

/// General operation:
/// - We take a `BatchGroups` as input.
/// - We iterate over each batch group and pass it to the liquid template
///   associated with this task.
/// - We expand the path template associated with the task and write it out.
#[derive(Debug)]
pub struct BatchRenderCommand {
    pub args: BatchRender,
}

#[async_trait]
impl PipelineCommand for BatchRenderCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let batch_groups = match input {
            PipelineValues::BatchGroups(bg) => bg,
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "batch-render needs BatchGroups".to_string(),
                }));
            }
        };

        match self.args.task.as_str() {
            "dir" => {
                let template = build_and_parse_dir_listing();
                let tree_info = server.tree_info()?;
                let commit_info = server.commit_info()?;
                // Each directory's own per-file info is in the group of its
                // parent directory.
                let mut dir_infos = HashMap::new();
                for item in &batch_groups.groups {
                    if let PipelineValues::FileMatches(fm) = &item.value {
                        for file in &fm.file_matches {
                            if file.concise.is_dir {
                                dir_infos.insert(file.path, file.concise.clone());
                            }
                        }
                    }
                }
                // (Each directory's coverage, first, as the server's lookups
                // are async; then their pages, rendered and written by as
                // many threads as there are cores, as a big tree has ~100k
                // directories: ~3 minutes of firefox's output step on one.)
                let mut dirs = vec![];
                for item in batch_groups.groups {
                    if let PipelineValues::FileMatches(fm) = item.value {
                        let coverage_history = server.coverage_history(&item.name).await?;

                        let mut panel = vec![];
                        if coverage_history.is_some() {
                            let coverage_summary = server.coverage_summary(&item.name).await?;
                            add_coverage_panel_item(&mut panel, coverage_summary.as_ref());
                        }

                        let dir_concise = dir_infos.get(&Ustr::from(&item.name));
                        if dir_concise.is_some_and(|info| info.info.get("test_results").is_some()) {
                            panel.push(PanelSection {
                                name: "Other Tools".to_owned(),
                                items: vec![tests_firefox_dev_panel_item(&item.name, true)],
                                raw_items: vec![],
                            });
                        }
                        dirs.push((item.name, fm, coverage_history, panel));
                    }
                }

                let rev_box = commit_info.as_ref().map(|info| {
                    let date = info.date.format("%F %T %z").to_string();
                    let date = liquid::model::DateTime::from_str(&date).unwrap();
                    liquid::object!({
                        "long": info.rev,
                        "short": &info.rev[..8],
                        "desc_html": info.header.as_str(),
                        "date": date,
                    })
                });
                let render = |(name, fm, coverage_history, panel): &(
                    String,
                    FileMatches,
                    _,
                    Vec<PanelSection>,
                )|
                 -> Result<()> {
                    let dir_concise = dir_infos.get(&Ustr::from(name));
                    let mut liquid_globals = liquid::object!({
                        "tree": tree_info.name,
                        // the header always needs this
                        "query": "",
                        "search_url": config::search_url(&tree_info.name),
                        "search_box_url": config::search_box_url(&tree_info.name, false),
                        "query_search": config::search_endpoint(&tree_info.name) == SearchEndpoint::Query,
                        "path": name,
                        "files": fm.file_matches,
                        "panel": panel,
                        "coverage_history": coverage_history,
                        "dir_concise": dir_concise,
                    });
                    match &rev_box {
                        Some(rev_box) => {
                            liquid_globals.insert("rev_box".into(), rev_box.clone().into());
                        }
                        None => {
                            liquid_globals.insert("rev_box".into(), liquid::model::Value::Nil);
                        }
                    }

                    let rendered = match template.render(&liquid_globals) {
                        Ok(r) => r,
                        Err(e) => {
                            return Err(ServerError::StickyProblem(ErrorDetails {
                                layer: ErrorLayer::ConfigLayer,
                                message: format!("Template problems: {}", e),
                            }));
                        }
                    };
                    let output_path = server
                        .translate_path(SearchfoxIndexRoot::UncompressedDirectoryListing, name)?;
                    write_file_ensuring_parent_dir(&output_path, &rendered)?;
                    Ok(())
                };
                let next = std::sync::atomic::AtomicUsize::new(0);
                let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
                std::thread::scope(|scope| {
                    let workers: Vec<_> = (0..threads.min(dirs.len().max(1)))
                        .map(|_| {
                            scope.spawn(|| -> Result<()> {
                                loop {
                                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let Some(dir) = dirs.get(i) else {
                                        return Ok(());
                                    };
                                    render(dir)?;
                                }
                            })
                        })
                        .collect();
                    workers
                        .into_iter()
                        .try_for_each(|worker| worker.join().unwrap())
                })?;
                Ok(PipelineValues::Void)
            }
            unknown => Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::ConfigLayer,
                message: format!("Unknown task type: {}", unknown),
            })),
        }
    }
}
