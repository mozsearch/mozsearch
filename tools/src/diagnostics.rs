//! Created to emit JSON-payloads that characterize unprivileged
//! meta-information about the given mozsearch tree in a non-interactive fashion
//! to aid in debugging and also to potentially support a future AJAXy dashboard
//! that can help people understand what the state of the various servers /
//! trees is.
//!
//! The dashboard would be AJAXy because our use of multiple servers that have
//! decoupled life-cycles is not well-suited to our current static-rendering
//! approach.  Additionally, we currently don't want the web-servers to have any
//! IAM access at all to know what the other servers might be, etc.

use serde_json::{json, Value};

use crate::file_format::config::Config;

/// Emit relevant runtime data for this tree.  We're not interested in surfacing
/// the static values from our config.json but instead on the dynamic state that
/// we would otherwise have to scrape the indexer log for or run commands
/// against git.
///
/// It is a specific non-goal to provide any information about usage stats or
/// anything that might make someone think there's any value in polling the
/// web endpoint serving this.  If there's anything that makes this value vary
/// over time (and therefore prevents caching of our returned response), we
/// should be very explicit about why that is.
pub fn diagnostics_from_config(cfg: &Config, tree_name: &str) -> Value {
    let tree_config = &cfg.trees[tree_name];

    let mut git_stats = Value::Null;

    if let Some(gitdata) = &tree_config.git {
        git_stats = json!({
            "blame": json!({
                "count": gitdata.blame_map.len()
            }),
            "hg": json!({
                "count": gitdata.hg_map.len()
            }),
            "old": json!({
                "count": gitdata.old_map.len()
            }),
            "mailmap": json!({
                "count": gitdata.mailmap.entries.len()
            }),
            "blame_ignore": json!({
                "count": gitdata.blame_ignore.entries.len()
            })
        });
    }

    json!({
        "git": git_stats,
    })
}
