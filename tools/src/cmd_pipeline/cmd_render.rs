use async_trait::async_trait;
use clap::Args;

use super::interface::{PipelineCommand, PipelineValues};
use crate::{
    abstract_server::{
        AbstractServer, ErrorDetails, ErrorLayer, Result, SearchfoxIndexRoot, ServerError,
    },
    templating::builder::{build_and_parse_search_template, build_and_parse_help_index}, file_utils::write_file_ensuring_parent_dir,
};

/// Render a single template, potentially processing pipeline input.
#[derive(Debug, Args)]
pub struct Render {
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
pub struct RenderCommand {
    pub args: Render,
}

#[async_trait]
impl PipelineCommand for RenderCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let tree_info = server.tree_info()?;

        match self.args.task.as_str() {
            "search-template" => {
                let template = build_and_parse_search_template();

                let liquid_globals = liquid::object!({
                    "tree": tree_info.name,
                    // the header always needs this
                    "query": "",
                });
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
                    SearchfoxIndexRoot::IndexTemplates,
                    "search.html",
                )?;
                write_file_ensuring_parent_dir(&output_path, &rendered)?;
                Ok(PipelineValues::Void)
            }
            "help" => {
                let template = build_and_parse_help_index();

                let content_path = server.translate_path(
                    SearchfoxIndexRoot::ConfigRepo,
                    "help.html"
                )?;
                let content = std::fs::read_to_string(content_path)?;

                let liquid_globals = liquid::object!({
                    "tree": tree_info.name,
                    // the header always needs this
                    "query": "",
                    "content": content,
                });
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
                    SearchfoxIndexRoot::IndexTemplates,
                    "help.html",
                )?;
                write_file_ensuring_parent_dir(&output_path, &rendered)?;
                Ok(PipelineValues::Void)

            }
            unknown => Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::ConfigLayer,
                message: format!("Unknown task type: {}", unknown),
            })),
        }
    }
}
