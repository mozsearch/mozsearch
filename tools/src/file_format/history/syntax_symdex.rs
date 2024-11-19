//! This file defines the ND-JSON records we write into files under
//! `history/syntax/symdex`.
//!
//! Symdex files are organized on a per-language basis and serve as very simple
//! cross-references from a pretty symbol identifier to the source files that
//! contain the declarations and definitions for that symbol and, if the symbol
//! is class-like, its members, fields, and any nested classes.  We currently
//! aren't planning to have entries for namespaces, but that's a weakly held
//! decision.
//!
//! Because the files represent an aggregation of information derived from
//! potentially multiple source files and our processing model only re-processes
//! files that are changed, our general approach for updating these files is:
//! - For every updated `history/syntax/files-struct` file that is updated
//!   because of its source file, we create a `SymdexRecord` from each of its
//!   `FileStructureRow` records and bin them based on their "pretty" and their
//!   parent's "pretty" if appropriate for the parent type.
//! - Once we've processed all the files, we process the resulting map structure
//!   on a per-symdex-file basis based on the "pretty" values:
//!   - We load the existing symdex file.
//!   - We filter out all `SymdexRecord` records for any files we have new
//!     records for.  (This saves us from having to compute any deltas.)
//!   - We append all the new records.
//!   - We sort the symdex records by their "pretty".
//!   - We write out the updated symdex file (including the leading header).

use serde::{Deserialize, Serialize};

use super::syntax_files_struct::FileStructureRow;

// First record in a symdex file; it wants to be completely independent of the
// rest of the contents of the file and so there's not much to put in here other
// than if we eventually want to handle overload / "stop symbol" semantics for
// some reason.
#[derive(Debug, Serialize, Deserialize)]
pub struct SymdexHeader {}

/// The contents of a FileStructuredRow with a path added.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SymdexRecord {
    #[serde(flatten)]
    pub file_row: FileStructureRow,

    pub path: String,
}
