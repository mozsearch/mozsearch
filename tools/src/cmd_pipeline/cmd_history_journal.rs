use async_trait::async_trait;
use clap::{Args, ValueEnum};

use super::interface::{JsonValue, JsonValueList, PipelineCommand, PipelineValues};
use crate::abstract_server::{AbstractServer, Result};
use crate::hyperblame::journals::JournalKind;

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum HistoryJournalKind {
    /// The physical-path journal of what happened to a file's tokens.
    Future,
    /// The logical-path journal of per-symbol changes to a file.
    FilesDelta,
    /// The journal of changes involving a token.
    Tokens,
}

/// Show a token-centric history timeline journal (see `hyperblame::journals`),
/// optionally expanding its summary records into the detail records they
/// summarize, which is how we verify history consolidation.
#[derive(Debug, Args)]
pub struct HistoryJournal {
    /// The kind of journal.
    #[clap(value_enum)]
    kind: HistoryJournalKind,

    /// The tree-relative path for future and files-delta journals, or the token
    /// for token journals.
    #[clap(value_parser)]
    target: String,

    /// The (full) source revision to show the journal as of, rather than the
    /// history's head.
    #[clap(long, value_parser)]
    rev: Option<String>,

    /// Replace summary records with the detail records they summarize.
    #[clap(long, value_parser)]
    expand: bool,
}

#[derive(Debug)]
pub struct HistoryJournalCommand {
    pub args: HistoryJournal,
}

#[async_trait]
impl PipelineCommand for HistoryJournalCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let kind = match self.args.kind {
            HistoryJournalKind::Future => JournalKind::Future,
            HistoryJournalKind::FilesDelta => JournalKind::FilesDelta,
            HistoryJournalKind::Tokens => JournalKind::Tokens,
        };
        let records = server
            .fetch_history_journal(
                kind,
                &self.args.target,
                self.args.rev.as_deref(),
                self.args.expand,
            )
            .await?;
        Ok(PipelineValues::JsonValueList(JsonValueList {
            values: records
                .into_iter()
                .map(|value| JsonValue { value })
                .collect(),
        }))
    }
}
