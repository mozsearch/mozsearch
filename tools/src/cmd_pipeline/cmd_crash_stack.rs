use async_trait::async_trait;
use clap::{Args, ValueEnum};

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::crash_stack::{crash_report_stack, crash_stack, render_crash_stack, stack_sym_info};

/// The MIME type of the crash view's HTML (see `crash_stack`), which the query
/// page shows as it is.
pub const CRASH_STACK_MIME: &str = "application/vnd.searchfox.crash-stack+html";

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum CrashStackFormat {
    /// The crash view's HTML, for the query page.
    Html,
    /// The stack (see `crash_stack::CrashStack`), as JSON.
    Json,
}

/// Which of a crash report's threads' stacks the crash view shows.
#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum CrashThreads {
    /// The crashing thread's.
    Crashing,
    /// Every thread's, the crashing thread's first.
    All,
}

/// A crash signature's stack beside its frames' code (see `crash_stack`): the
/// `crash-signature:` query term's; or a crash report's (`--report`, the
/// `crash-report:` term's, with `--threads`, `crash-threads:`).
#[derive(Debug, Args)]
pub struct CrashStack {
    /// The signature, as crash-stats has it (ex: "OOM | large | NS_ABORT_OOM
    /// | IPC::ParamTraits<JSStructuredCloneData>::Read").
    #[clap(value_parser)]
    signature: Option<String>,

    /// A crash report's ID (crash-stats', ex:
    /// "b40697bb-fa73-46e6-bcd3-0f1ce0261030"), rather than a signature.
    #[clap(long, value_parser)]
    report: Option<String>,

    /// Which of a report's threads' stacks to show.
    #[clap(long, value_enum, default_value = "crashing")]
    threads: CrashThreads,

    #[clap(long, value_enum, default_value = "html")]
    format: CrashStackFormat,
}

#[derive(Debug)]
pub struct CrashStackCommand {
    pub args: CrashStack,
}

#[async_trait]
impl PipelineCommand for CrashStackCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let stack = match (&self.args.report, &self.args.signature) {
            (Some(report), _) => {
                crash_report_stack(server, report, self.args.threads == CrashThreads::All).await?
            }
            (None, Some(signature)) => crash_stack(server, signature).await?,
            (None, None) => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::BadInput,
                    message: "crash-stack needs a signature or a report".to_string(),
                }));
            }
        };
        match self.args.format {
            CrashStackFormat::Json => Ok(PipelineValues::JsonValue(JsonValue {
                value: serde_json::to_value(&stack)?,
            })),
            CrashStackFormat::Html => {
                let tree = server.tree_info()?.name;
                let sym_info = stack_sym_info(server, &stack).await;
                let mut html = Vec::new();
                render_crash_stack(&tree, &stack, &sym_info, &mut html).map_err(|message| {
                    ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::ServerLayer,
                        message: message.to_string(),
                    })
                })?;
                // (Frames whose lines couldn't be fetched, which they say,
                // may be got if tried again.)
                let transient = stack
                    .frames
                    .iter()
                    .chain(stack.threads.iter().flat_map(|thread| &thread.frames))
                    .any(|frame| frame.problem.is_some());
                Ok(PipelineValues::TextFile(TextFile {
                    mime_type: CRASH_STACK_MIME.to_string(),
                    contents: String::from_utf8(html).unwrap_or_default(),
                    transient,
                }))
            }
        }
    }
}
