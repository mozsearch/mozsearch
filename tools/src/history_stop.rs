//! Stopping the history tools (`build-syntax-token-tree` and
//! `build-timeline-tree`) between revisions.  When the file named by the
//! `HISTORY_STOP_FILE` environment variable exists, they finish the revision
//! they're processing, write what they have (including the notes recording the
//! revisions they've processed; see `source_mapping`), and exit with
//! `STOPPED_EXIT_CODE`.  scripts/build-history.py uses this to restart itself
//! and the tools in place without losing anything, ex: to upgrade them in a
//! running reblame.  A tool can also end a run early the same way for reasons
//! of its own (ex: build-timeline-tree's `MAX_CHECKPOINTS`), exiting with
//! `ENDED_EARLY_EXIT_CODE`, which tells build-history.py to just run it again.

use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The exit status of a history tool which stopped because it was asked to.
/// (EX_TEMPFAIL, from sysexits.h.)
pub const STOPPED_EXIT_CODE: i32 = 75;

/// The exit status of a history tool which ended its run before its
/// `COMMIT_LIMIT` with revisions left to process, so that it should be run
/// again.
pub const ENDED_EARLY_EXIT_CODE: i32 = 76;

/// Whether the tool has been asked to stop.  This checks for the file, which is
/// cheap enough to do for every revision.
pub fn stop_requested() -> bool {
    static STOP_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
    STOP_FILE
        .get_or_init(|| env::var_os("HISTORY_STOP_FILE").map(PathBuf::from))
        .as_ref()
        .is_some_and(|path| path.exists())
}
