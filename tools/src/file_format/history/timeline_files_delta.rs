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

use serde::{Deserialize, Serialize};

use super::timeline_common::{
    DetailRecordRef, FileSyntaxDelta, SummaryRecordRef, SymbolSyntaxDeltaGroup, TimelineRecord,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FileDeltaRecord {
    Detail(FileDeltaDetailRecord),
    Summary(FileDeltaSummaryRecord),
}

impl TimelineRecord for FileDeltaRecord {
    fn dedupe_key(&self) -> String {
        match self {
            FileDeltaRecord::Detail(d) => format!("D{}", d.desc.source_rev),
            FileDeltaRecord::Summary(s) => format!("S{:?}", s.desc.iso_week_range),
        }
    }

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
