//! Token-centric "hyperblame" history logic shared by `build-timeline-tree` and
//! anything that wants to reason about the token-centric history data.
//!
//! See `file_format/history` for the on-disk representations.

pub mod backouts;
pub mod history_config;
pub mod inference;
pub mod source_mapping;
pub mod stats;
pub mod suffix_array;
