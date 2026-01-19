use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues};
use crate::links;
use crate::{
    abstract_server::{
        AbstractServer, ErrorDetails, ErrorLayer, Result, SearchfoxIndexRoot, ServerError,
    },
    file_utils::write_file_ensuring_parent_dir,
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

fn entity_replace(s: &str) -> String {
    s.replace("&", "&amp;").replace("<", "&lt;")
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
                for item in batch_groups.groups {
                    if let PipelineValues::FileMatches(fm) = item.value {
                        let mut liquid_globals = liquid::object!({
                            "tree": tree_info.name,
                            // the header always needs this
                            "query": "",
                            "path": item.name,
                            "files": fm.file_matches,
                            "panel": liquid::model::Value::Nil,
                        });
                        if let Some(info) = &commit_info {
                            liquid_globals.insert("rev_box".into(), liquid::object!({
                                "long": info.rev,
                                "short": &info.rev[..8],
                                "desc_html": links::linkify_commit_header(&entity_replace(info.header.as_str())),
                            }).into());
                        } else {
                            liquid_globals.insert("rev_box".into(), liquid::model::Value::Nil);
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
                        let output_path = server.translate_path(
                            SearchfoxIndexRoot::UncompressedDirectoryListing,
                            &item.name,
                        )?;
                        write_file_ensuring_parent_dir(&output_path, &rendered)?;
                    }
                }
                Ok(PipelineValues::Void)
            }
            unknown => Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::ConfigLayer,
                message: format!("Unknown task type: {}", unknown),
            })),
        }
    }
}
