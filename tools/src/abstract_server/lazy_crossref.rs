//! Lazy computation that could have taken place in crossref.rs but did not.
//!
//! Logic goes in here that likely should run in crossref.rs once we're:
//! - Certain we want the functionality.
//! - Largely done iterating on the logic.  crossref for m-c takes 21 minutes so
//!   it is hard to have a tight experimentation loop.  It's also the case that
//!   `make build-test-repo` is much slower now since we added Java/Kotlin
//!   support.
//!
//! Functionality currently under development:
//! - Argument string population.  Currently for C++ we emit the argRanges on
//!   source and target records but not the string payloads.  (This was partly
//!   done for cost reasons, but also because we want to be able to use the
//!   ranges to know what semantic records they cover.)
//!
//! Some shorter term potential functionality:
//! - Inferred thread usage for methods that are looked up; classes would be
//!   useful too but is something where it either needs to happen in crossref
//!   proper or the lazy crossref mechanism here needs to become stateful and
//!   cache some things.  That's likely a dangerous path in terms of the
//!   potential for it to stick around and get increasingly messy.
//!   - Arguably any commands that want to know things for classes should
//!     probably support arbitrary predicates/checks which can only be
//!     determined by performing on-the-fly per-method checks, so this should
//!     also not be a limiting factor although it would probably be a huge
//!     performance win if it was reliably precomputed for those use-cases.
//!
//! Some longer term potential functionality:
//! - Improved argument processing to leverage the semantic tokens.
//! - Some level of dataflow analysis.  Note that this would require the C++
//!   indexer to emit additional information and/or using tree-sitter to help
//!   detect writes/assignment.

use super::{local_index::LocalIndex, server_interface::Result};

use serde_json::{Value};

/// Perform the actual lazy cross-reference process.
///
/// Note that while we make an effort to be efficient within this method in
/// terms of loading the contents of source files at most once, this is
/// fundamentally not efficient when multiple calls are made to this method
/// where it's highly likely we could have reused the line-parsed files where it
/// is very likely for there to be file overlap.  (That said, we do expect this
/// to be fine in most cases and this is a trade-off we are intentionally
/// making.)
pub async fn perform_lazy_crossref(_server: &LocalIndex, val: Value) -> Result<Value> {
    // Consume the "uses" array if present so we can transform it.
    /*
    let use_path_containers: Vec<PathSearchResult> = match val.get_mut("uses") {
        Some(uses) => from_value(uses.take()).unwrap_or_default(),
        _ => vec![]
    };

    let mut source_file: HashMap<String, Vec<String>>

    for use_path_container in &use_path_containers {

    }
    */

    // ## Figure out what source files we need to load

    // ### Determine files to load for uses

    // ## Load the source files in

    // ## Process Source files

    // ### Walk the argument excerpts

    Ok(val)
}
