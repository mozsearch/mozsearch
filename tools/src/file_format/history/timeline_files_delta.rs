//! This file defines the ND-JSON records we write into files under
//! `history/timeline/files-delta`.  These files are organized on a "logical"
//! basis, which means that when a file is renamed/moved, its files-delta file
//! is moved too, and when a file is copied, the new file's files-delta file
//! starts as a copy of the original file's.  When a file is deleted, its
//! files-delta file is deleted too; "history/timeline/future" maintains the
//! physical tombstone.
//!
//! The first line of each file is a `FileDeltaHeader` and the remaining lines
//! are `FileDeltaRecord`s ordered from newest to oldest.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use super::timeline_common::{
    ChangeKind, DetailRecordRef, FileSyntaxDelta, JournalVersionRef, RecordKind, SummaryRecordRef,
    SymbolSyntaxDelta, SymbolSyntaxDeltaGroup, TimelineRecord, detail_record_ref,
    summary_record_ref,
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FileDeltaHeader {}

/// Details changes from a specific revision for the source file containing this
/// record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDeltaDetailRecord {
    #[serde(flatten)]
    pub desc: DetailRecordRef,

    #[serde(flatten)]
    pub delta: FileSyntaxDelta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDeltaSummaryRecord {
    #[serde(flatten)]
    pub desc: SummaryRecordRef,

    #[serde(flatten)]
    pub symbol_group: SymbolSyntaxDeltaGroup,
}

/// Internally tagged enum for our detail and summary types.  This ends up
/// serializing as `{"type": "Detail" , ...}` or `{"type": "Summary", ...}`.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum FileDeltaRecord {
    Detail(FileDeltaDetailRecord),
    Summary(FileDeltaSummaryRecord),
}

/// Every field a `FileDeltaRecord` line can have (see `RecordKind`).
#[derive(Deserialize)]
struct FileDeltaRecordFields {
    #[serde(rename = "type")]
    kind: RecordKind,
    source_rev: Option<String>,
    syntax_rev: Option<String>,
    iso_date: Option<String>,
    #[serde(default)]
    backs_out: Vec<String>,
    source_revs: Option<Vec<String>>,
    preds: Option<Vec<JournalVersionRef>>,
    iso_week_range: Option<(u16, u8, u8)>,
    change: Option<ChangeKind>,
    #[serde(default)]
    moved_from: Option<String>,
    #[serde(default)]
    copied: bool,
    symbol_deltas: Option<BTreeMap<String, SymbolSyntaxDelta>>,
}

impl<'de> Deserialize<'de> for FileDeltaRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let f = FileDeltaRecordFields::deserialize(deserializer)?;
        let symbol_group = SymbolSyntaxDeltaGroup {
            symbol_deltas: f
                .symbol_deltas
                .ok_or_else(|| D::Error::missing_field("symbol_deltas"))?,
        };
        Ok(match f.kind {
            RecordKind::Detail => FileDeltaRecord::Detail(FileDeltaDetailRecord {
                desc: detail_record_ref(f.source_rev, f.syntax_rev, f.iso_date, f.backs_out)?,
                delta: FileSyntaxDelta {
                    change: f.change.ok_or_else(|| D::Error::missing_field("change"))?,
                    moved_from: f.moved_from,
                    copied: f.copied,
                    symbol_group,
                },
            }),
            RecordKind::Summary => FileDeltaRecord::Summary(FileDeltaSummaryRecord {
                desc: summary_record_ref(f.source_revs, f.preds, f.iso_week_range)?,
                symbol_group,
            }),
        })
    }
}

impl TimelineRecord for FileDeltaRecord {
    fn iso_date(&self) -> Option<&str> {
        match self {
            FileDeltaRecord::Detail(d) => Some(&d.desc.iso_date),
            FileDeltaRecord::Summary(_) => None,
        }
    }

    fn detail_source_rev(&self) -> Option<&str> {
        match self {
            FileDeltaRecord::Detail(d) => Some(&d.desc.source_rev),
            FileDeltaRecord::Summary(_) => None,
        }
    }

    fn summary_ref(&self) -> Option<&SummaryRecordRef> {
        match self {
            FileDeltaRecord::Detail(_) => None,
            FileDeltaRecord::Summary(s) => Some(&s.desc),
        }
    }
}
