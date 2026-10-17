//! "Dataflow for babies": naive, syntax-directed dataflow from tree-sitter
//! parses plus searchfox's analysis records, for following values forwards
//! and backwards through code, across calls and binding layers, without a
//! compiler's help (see `ir` for what's understood, and `trace` for how
//! values are followed).

pub mod frontend;
pub mod ir;
pub mod oracle;
pub mod records;
pub mod render;
pub mod slice;
pub mod summary;
pub mod trace;
