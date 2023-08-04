//! This file defines the ND-JSON records we write into files under
//! `history/timeline/files-delta`.

use serde::{Deserialize, Serialize};

use super::timeline_common::{SummaryRecordRef, DetailRecordRef, SymbolSyntaxDeltaGroup};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileDeltaHeader {

}

/// Details changes from a specific revision for the source file containing this
/// record.  We do not currently
#[derive(Debug, Serialize, Deserialize)]
pub struct FileDeltaDetailRecord {
    #[serde(flatten)]
    pub desc: DetailRecordRef,

    #[serde(flatten)]
    pub symbol_group: SymbolSyntaxDeltaGroup,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileDeltaSummaryRecord {
    #[serde(flatten)]
    pub desc: SummaryRecordRef,

    #[serde(flatten)]
    pub symbol_group: SymbolSyntaxDeltaGroup,
}

/// Internally tagged enum for our detail and summary types.  This ends up
/// serializing as `{"type": "Detail" , ...}` or `{"type": "Summary", ...}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FileDeltaRecord {
    Detail(FileDeltaDetailRecord),
    Summary(FileDeltaSummaryRecord),
}
