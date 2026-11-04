use std::collections::BTreeMap;

use async_trait::async_trait;
use clap::{Args, ValueEnum};
use ustr::{Ustr, UstrMap};

use super::interface::{JsonValue, PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, Result, ServerError, sym_info};
use crate::lock_held::{
    CARDS_SCRIPT, LOCK_HELD_MIME, LockHeldView, card_line_requests, lock_held_view,
    render_lock_held, view_facets,
};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum LockHeldFormat {
    /// The view's HTML, for the query page.
    Html,
    /// The view's model (see `lock_held::LockHeldView`), as JSON.
    Json,
}

/// The code that runs with a mutex held (see `lock_held`): the
/// `lock-held:` query term's.
#[derive(Debug, Args)]
pub struct LockHeld {
    /// The mutex's symbol (a field's or a global's, ex:
    /// `F_<T_mozilla::dom::WorkerPrivate>_mMutex`).
    #[clap(value_parser)]
    mutex: String,

    #[clap(long, value_enum, default_value = "html")]
    format: LockHeldFormat,
}

#[derive(Debug)]
pub struct LockHeldCommand {
    pub args: LockHeld,
}

#[async_trait]
impl PipelineCommand for LockHeldCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        // (Cached by the mutex; the server's identity covers the index's and
        // the code's.)
        let mutex = self.args.mutex.clone();
        let value =
            crate::step_cache::cached(server.cache_identity(), "lock-held", &mutex, || async {
                let mut view = lock_held_view(server, &mutex).await?;
                // (Its files' subsystems, the tree's.)
                view.add_subsystems(&|path| {
                    server
                        .file_facet_info(path)
                        .and_then(|(_, subsystem)| subsystem)
                        .map(|subsystem| subsystem.to_string())
                });
                Ok::<_, ServerError>(serde_json::to_value(&view)?)
            })
            .await?;
        if self.args.format == LockHeldFormat::Json {
            return Ok(PipelineValues::JsonValue(JsonValue { value }));
        }
        let view: LockHeldView = serde_json::from_value(value)?;
        let tree = server.tree_info()?.name;
        let (rows, _) = crate::crash_stack::fetch_rows(server, card_line_requests(&view)).await;
        // (Its cards' symbols' SYM_INFO, for their context menus.)
        let syms: Vec<Ustr> = rows
            .values()
            .flat_map(|file| file.values())
            .flat_map(|row| sym_info::html_symbols(&row.html))
            .collect();
        let mut info = BTreeMap::new();
        let mut traversed = UstrMap::default();
        sym_info::add_syms_info(server, syms, &mut info, &mut traversed).await;
        let contents = format!(
            "{}<script>window.SYM_INFO = Object.assign(window.SYM_INFO || {{}}, {});</script>{}",
            render_lock_held(&tree, &view, &rows),
            sym_info::script_json(&info),
            CARDS_SCRIPT
        );
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: LOCK_HELD_MIME.to_string(),
            contents,
            transient: false,
            facets: Some(view_facets(&view)),
        }))
    }
}
