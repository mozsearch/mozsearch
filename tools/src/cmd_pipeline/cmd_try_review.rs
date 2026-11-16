use async_trait::async_trait;
use clap::{Args, ValueEnum};

use std::collections::{BTreeMap, BTreeSet};

use clap::Parser;

use super::cmd_dataflow::DataflowCommand;
use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use super::parser::{Command, ToolOpts};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};
use crate::try_review::{
    ReviewSlicing, ReviewView, TRY_REVIEW_MIME, add_line_html, card_lines, group_slicings,
    render_review, review_view, view_facets,
};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum TryReviewFormat {
    /// The view's HTML, for the query page.
    Html,
    /// The view's model (see `try_review::ReviewView`), as JSON.
    Json,
}

/// A try push's review view (see `try_review`): the `try-review:` query
/// term's.
#[derive(Debug, Args)]
pub struct TryReview {
    /// The push's head (its `try/hg/<push>` page's).
    #[clap(value_parser)]
    push: String,

    #[clap(long, value_enum, default_value = "html")]
    format: TryReviewFormat,
}

#[derive(Debug)]
pub struct TryReviewCommand {
    pub args: TryReview,
}

/// The `dataflow` command slicing `spec` (`<base>..<head>`) in `format`
/// (ex: "json"), as the query `slice-rev:<spec> slice-format:<format>` has
/// it (so that they share the step cache's slicing).
fn slicing_command(spec: &str, format: &str) -> Option<DataflowCommand> {
    let opts = ToolOpts::try_parse_from([
        "searchfox-tool",
        "dataflow",
        "--slice-rev",
        spec,
        "--format",
        format,
    ])
    .ok()?;
    match opts.cmd {
        Command::Dataflow(args) => Some(DataflowCommand { args }),
        _ => None,
    }
}

/// The groups' block diagrams (see `try_review::GroupSlicing::block`): each
/// group's classes' (`block-diagram --classes`), by the roll-ups of the
/// push's context (which have its patches' calls; see `Rollup::with_delta`),
/// rendered as the query page's diagrams are (`graph --format mozsearch
/// --hier subsystem`), and their nodes' SYM_INFO, as the page's script's
/// JSON.
async fn block_diagrams(
    server: &(dyn AbstractServer + Send + Sync),
    slicing: &mut ReviewSlicing,
) -> Option<String> {
    let parse = |args: &[&str]| -> Option<Command> {
        Some(
            ToolOpts::try_parse_from(std::iter::once("searchfox-tool").chain(args.iter().copied()))
                .ok()?
                .cmd,
        )
    };
    let mut symbols = BTreeMap::new();
    for group in &mut slicing.groups {
        if group.classes.is_empty() {
            continue;
        }
        let classes = group.classes.join(",");
        let (Some(Command::BlockDiagram(block_args)), Some(Command::Graph(graph_args))) = (
            parse(&["block-diagram", "--classes", &classes]),
            parse(&["graph", "--format", "mozsearch", "--hier", "subsystem"]),
        ) else {
            return None;
        };
        let block = super::cmd_block_diagram::BlockDiagramCommand { args: block_args };
        let graph = super::cmd_graph::GraphCommand { args: graph_args };
        let Ok(collection) = block.execute(server, PipelineValues::Void).await else {
            continue;
        };
        if let Ok(PipelineValues::GraphResultsBundle(bundle)) =
            graph.execute(server, collection).await
        {
            group.block = Some(
                bundle
                    .graphs
                    .into_iter()
                    .map(|graph| graph.graph)
                    .collect::<String>(),
            );
            symbols.extend(
                bundle
                    .symbols
                    .into_iter()
                    .map(|(sym, data)| (sym, Some(data))),
            );
        }
    }
    (!symbols.is_empty()).then(|| crate::abstract_server::sym_info::script_json(&symbols))
}

/// The slicing `spec` of the push's semantic analysis (see `try_delta`), if
/// it has one.
fn slicing_spec(server: &(dyn AbstractServer + Send + Sync)) -> Option<String> {
    let meta = server.try_delta_meta()?;
    Some(format!("{}..{}", meta.base, meta.head))
}

/// The push's slicing, by the view's groups, for their diagrams (see
/// `try_review::ReviewSlicing`): `slice-rev:<base>..<head>`'s, of its
/// semantic analysis's revisions, as the try canvas's clusters are, which
/// the step cache keeps (once imported, the try push's precomputing has it;
/// see the web-server's `precompute_try_push`), with the push's commits
/// rebased onto the indexed revision, which its units name, by the explore
/// data's.  None if the push has no semantic analysis.
async fn review_slicing(
    server: &(dyn AbstractServer + Send + Sync),
    view: &ReviewView,
) -> Option<ReviewSlicing> {
    let spec = slicing_spec(server)?;
    let mut out = ReviewSlicing {
        query: format!("slice-rev:{}", spec),
        rev: server.try_delta_meta()?.head,
        ..Default::default()
    };
    let value = match slicing_command(&spec, "json")?
        .execute(server, PipelineValues::Void)
        .await
    {
        Ok(PipelineValues::JsonValue(json)) => Ok(json.value),
        Ok(_) => Err("its slicing isn't JSON".to_string()),
        Err(ServerError::StickyProblem(details) | ServerError::TransientProblem(details)) => {
            Err(details.message)
        }
        Err(err) => Err(format!("{:?}", err)),
    };
    let slicing = value.and_then(|value| {
        serde_json::from_value::<crate::dataflow::slice::Slicing>(value)
            .map_err(|err| err.to_string())
    });
    match slicing {
        Ok(slicing) => {
            // (The rebased commits its units name, by the patches'.)
            let rebased: BTreeMap<String, String> = view
                .patches
                .iter()
                .filter_map(|patch| Some((patch.shown.clone()?, patch.rev.clone())))
                .collect();
            out.groups = group_slicings(view, &slicing, &rebased);
            // (Its cards' lines: those on the changes' dataflow, in the
            // head's files.)
            if let Some(PipelineValues::JsonValue(lines)) = slicing_command(&spec, "lines")?
                .execute(server, PipelineValues::Void)
                .await
                .ok()
            {
                let paths: BTreeSet<String> = out
                    .groups
                    .iter()
                    .flat_map(|group| &group.nodes)
                    .filter(|node| node.change != "removed")
                    .map(|node| node.path.clone())
                    .collect();
                let mut text = BTreeMap::new();
                for path in paths {
                    if let Ok(source) = server.fetch_raw_source(&path).await {
                        text.insert(path, source.lines().map(str::to_string).collect());
                    }
                }
                card_lines(&mut out.groups, &lines.value, &text);
            }
        }
        Err(why) => out.note = Some(why),
    }
    Some(out)
}

#[async_trait]
impl PipelineCommand for TryReviewCommand {
    /// Its slicing's, if neither it nor the view's HTML is cached (see
    /// `review_slicing`).
    fn estimate(&self, server: &(dyn AbstractServer + Send + Sync)) -> u64 {
        let push = self.args.push.trim().to_lowercase();
        if crate::step_cache::is_cached(server.cache_identity(), "try-review-html", &push) {
            return 0;
        }
        slicing_spec(server)
            .and_then(|spec| slicing_command(&spec, "json"))
            .map_or(0, |command| command.estimate(server))
    }

    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let push = self.args.push.trim().to_lowercase();
        if push.len() < 12 || !push.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::BadInput,
                message: format!(
                    "try-review: needs a try push's revision (12 to 40 hexadecimal digits), not {:?}",
                    self.args.push
                ),
            }));
        }
        // (Cached by the push; the server's identity covers the index's and
        // the code's.  A push imported again has the same commits.)
        let value =
            crate::step_cache::cached(server.cache_identity(), "try-review", &push, || async {
                Ok::<_, ServerError>(serde_json::to_value(review_view(server, &push).await?)?)
            })
            .await?;
        if self.args.format == TryReviewFormat::Json {
            return Ok(PipelineValues::JsonValue(JsonValue { value }));
        }
        let mut view: ReviewView = serde_json::from_value(value)?;
        let tree = server.tree_info()?.name;
        let facets = view_facets(&view);
        // (Its HTML, with its lines highlighted, cached too, by whether its
        // slicing could be had, which it may be later.)
        let identity = server.cache_identity();
        let mut key = push.clone();
        if !crate::step_cache::is_cached(identity, "try-review-html", &key) {
            view.slicing = review_slicing(server, &view).await;
            if let Some(slicing) = view.slicing.as_mut() {
                view.block_sym_info = block_diagrams(server, slicing).await;
            }
            if view.slicing.as_ref().is_some_and(|s| s.note.is_some()) {
                key = format!("{} unsliced", push);
            }
        }
        let contents =
            crate::step_cache::cached(server.cache_identity(), "try-review-html", &key, || async {
                let mut view = view;
                add_line_html(server, &mut view).await;
                Ok::<_, ServerError>(serde_json::Value::String(render_review(&tree, &view)))
            })
            .await?;
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: TRY_REVIEW_MIME.to_string(),
            contents: contents.as_str().unwrap_or_default().to_string(),
            transient: false,
            facets: Some(facets),
        }))
    }
}
