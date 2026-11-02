use async_trait::async_trait;
use clap::{Args, ValueEnum};

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::crash_sets::{CrashSetView, render_crash_set, set_facets, set_view};

/// The MIME type of the crash set view's HTML (see `crash_sets`), which the
/// query page shows as it is.
pub const CRASH_SET_MIME: &str = "application/vnd.searchfox.crash-set+html";

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum CrashSetFormat {
    /// The view's HTML, for the query page.
    Html,
    /// The set's analysis (see `crash_sets::CrashSetView`), as JSON.
    Json,
}

/// A crash set's view (see `crash_sets`): its crashes, by their families and
/// variants: the `crash-set:` query term's.
#[derive(Debug, Args)]
pub struct CrashSet {
    /// The set's ID (16 hex digits; see `crash_sets::set_id`).
    #[clap(value_parser)]
    id: String,

    #[clap(long, value_enum, default_value = "html")]
    format: CrashSetFormat,
}

#[derive(Debug)]
pub struct CrashSetCommand {
    pub args: CrashSet,
}

fn problem(layer: ErrorLayer, message: String) -> ServerError {
    ServerError::StickyProblem(ErrorDetails { layer, message })
}

#[async_trait]
impl PipelineCommand for CrashSetCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let store = server.crash_sets().ok_or(ServerError::Unsupported)?;
        let rules = server.crash_thread_rules().ok_or_else(|| {
            problem(
                ErrorLayer::ServerLayer,
                "This server has no crash-threads.toml for crash sets' analyses".to_string(),
            )
        })?;
        // (The set's analysis is cached by the set, which never changes, and
        // the rules; the server's identity covers the code's.)
        let id = self.args.id.clone();
        let value = crate::step_cache::cached(
            server.cache_identity(),
            "crash-set",
            &format!("{} {}", id, rules.fingerprint()),
            || async {
                let mut view = set_view(&store, &rules, &id)
                    .map_err(|message| problem(ErrorLayer::BadInput, message))?;
                // (Its files' subsystems, the tree's.)
                view.add_subsystems(&|path| {
                    server
                        .file_facet_info(path)
                        .and_then(|(_, subsystem)| subsystem)
                        .map(|subsystem| subsystem.to_string())
                });
                Ok::<_, ServerError>(serde_json::to_value(&view)?)
            },
        )
        .await?;
        match self.args.format {
            CrashSetFormat::Json => Ok(PipelineValues::JsonValue(JsonValue { value })),
            CrashSetFormat::Html => {
                let view: CrashSetView = serde_json::from_value(value)?;
                let tree = server.tree_info()?.name;
                Ok(PipelineValues::TextFile(TextFile {
                    mime_type: CRASH_SET_MIME.to_string(),
                    contents: render_crash_set(&tree, &view),
                    transient: false,
                    // (Its crashes' facets; see `crash_sets::set_facets`.)
                    facets: Some(set_facets(&view)),
                }))
            }
        }
    }
}
