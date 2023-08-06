//! This file defines the ND-JSON records we write into files under
//! `history/syntax/files-struct`.  The contents of the files are intended to
//! always reflect the current state of the tree for the source revision they
//! are derived from.  Our goal is to be purely functional in doing this; our
//! output is expected to be purely a function of the contents of the files.
//!
//! Currently we're using `String` instead of `Ustr` because the code is
//! intended to be run in parallel so lock contention is not helpful and any
//! memory pressure concerns would not come from strings where Ustr is likely to
//! be a major help.  (However, ropes would be quite useful.)

use serde::{Deserialize, Serialize};

/// This record is the first JSON record in the file and provides file-level
/// information.  Currently we emit "check the plug" debugging information like
/// what tree-sitter parser was used to derive the contents, as well as
/// indicating specifically when we did not recognize the file as a supported
/// type and therefore had no parser.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileStructureHeader {
    // The tree-sitter language we parsed this as, or None if we did not parse
    // this file using tree-sitter.  If the value is None, we expect there will
    // be no `FileStructureRow` records following the header.
    pub lang: Option<String>,
}

/// The remainder of the records in the file after the header.  These are
/// expected to be emitted in the order they are encountered in the source file.
/// Accordingly, we don't encode any position information since we would expect
/// these to be subject to churn which makes diffs derived from the changes in
/// this file harder to usefully derive information from.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct FileStructureRow {
    /// The pretty identifier for this symbol.  It's currently assumed that we
    /// can get the pretty identifier for the containing namespace/class by
    /// popping off the last segment of the pretty idenfitier using "::" as the
    /// delimiter.
    pub pretty: String,

    /// Is this a definition?  If it's not a definition, it's a declaration.
    #[serde(rename = "isDef")]
    pub is_def: bool,

    /// The "structured" kind, or at least a best-effort attempt to match it.  We
    /// primarily care about "class", "method", and "field" as those have clear
    /// benefit to listing in the symdex.
    pub kind: String,
}
