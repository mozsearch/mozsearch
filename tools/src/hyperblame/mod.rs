//! Token-centric "hyperblame" history logic shared by `build-timeline-tree` and
//! anything that wants to reason about the token-centric history data.
//!
//! See `file_format/history` for the on-disk representations.

pub mod backouts;
pub mod consolidation;
pub mod explore;
pub mod file_changes;
pub mod future;
pub mod history_config;
pub mod inference;
pub mod interdiff;
pub mod journals;
pub mod outline_diff;
pub mod page_blame;
pub mod page_data_cache;
pub mod peephole;
pub mod recency;
pub mod renames;
pub mod segments;
pub mod stack_strip;
pub mod stats;
pub mod suffix_array;
pub mod token_blame;
