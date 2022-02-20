use async_trait::async_trait;
use futures_core::stream::BoxStream;
use serde_json::Value;

pub type Result<T> = std::result::Result<T, ServerError>;

// JSON parse errors are sticky data problems.
impl From<serde_json::Error> for ServerError {
    fn from(err: serde_json::Error) -> ServerError {
        ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::DataLayer,
            message: err.to_string(),
        })
    }
}

/// Express whether the error seems to be happening in the server or the data.
#[derive(Debug)]
pub enum ErrorLayer {
    /// The request itself has structural issues like a malformed URL.  This
    /// should not be used for cases where the user input results in a search
    /// miss (which should instead be part of the result payload), but is
    /// appropriate for cases where the user input makes it impossible to return
    /// a hit or a miss, like an incorrectly constructed pipeline.
    ///
    /// For example, we would want to throw an error in an "insta" check if the
    /// pipeline is not a valid pipeline.  Or similarly, a shell script invoking
    /// searchfox-tool wants to experience an error if the command pipeline is
    /// incorrect.
    ///
    /// This does potentially end up ambiguous in the web UI case if the web UI
    /// allows the user to construct pipelines that aren't validated before
    /// being sent to the server.  In that case we would want to treat the error
    /// akin to a search miss and not generate errors that would trip alarms.
    /// (Our "insta" checks of course help avoid such problems becoming serious
    /// silent errors, as they would/should not be quieted.)
    BadInput,
    /// The error seems to involve server logic, so it may or may not be an issue
    /// with the underlying data.
    ServerLayer,
    /// The error seems to be related to the indexed data in question rather
    /// than the server, like the data was not indexed.
    DataLayer,
    /// We're not sure if it was a server issue or a data issue.
    UnknownLayer,
}

/// ServerError payload to provide details about what went wrong for
/// investigation purposes.  In the future, this could wrap the
/// underlying errors we've seen.
#[derive(Debug)]
pub struct ErrorDetails {
    /// Attempt to distinguish failures due to server bugs from failures due to
    /// indexing bugs.  For example a 500 response from a server would be a
    /// `ServerLayer` problem, but if a 404 was instead returned, that would be
    /// a `DataLayer` problem.
    pub layer: ErrorLayer,
    /// Stringified version of the lower level error.
    pub message: String,
}

/// Does a retry makes sense or not?
///
/// Actually performing retries could of course happen either below this
/// abstraction layer or above it.  The argument for above is that the
/// `cmd_pipeline` could make more informed scheduling decisions with
/// appropriately long back-offs than this lower layer would be able to.  But
/// that's all speculative at this point and this type is really being
/// introduced because we need a unifying error type.
#[derive(Debug)]
pub enum ServerError {
    /// An error that will persist for at least this index.  For example a 404.
    StickyProblem(ErrorDetails),
    /// An error that might go away if retried later.  For example a 504 "Gateway
    /// timeout".
    TransientProblem(ErrorDetails),
    Unsupported,
}

/// Unified exposure for interacting with a local Searchfox index on disk or
/// a remote searchfox server over HTTPS talking to the web-server.
///
/// The primary goal is for our tests to verify both our on-disk representations
/// and that these are exposed to searchfox users correctly.  It's also our hope
/// that this can be used by searchfox contributors to investigate problems and
/// how things currently work more efficiently and enjoyably than manually doing
/// so.
///
/// ## Runtime Assumptions
///
/// We assume that we are operating in a tokio multi-threaded runtime and that
/// all blocking operations for any implementations of these traits should
/// responsibly make use of
/// https://docs.rs/tokio/1.5.0/tokio/task/index.html#blocking-and-yielding so
/// that full parallelism can be maintained.  In particular, this means that
/// mmap-based lookups which can fault and block on IO should likely use
/// https://docs.rs/tokio/1.5.0/tokio/task/index.html#block_in_place.
///
/// ## Abstraction Level / Library Use
///
/// Currently existing analysis-file processing and other logic:
/// - Uses synchronous I/O
///
/// In the end, it likely would make sense for the analysis mechanism to:
/// - Support async I/O
/// - Use async streams via https://docs.rs/tokio-stream/0.1.5/tokio_stream/
///   on a per-record or per-line granularity, quite possibly using our analysis
///   types for analysis records instead of untyped JSON.
///
/// But I'm introducing this interface right now in an attempt to provide
/// increased test coverage before making more extensive refactorings.  So for
/// now, this interface will do the simplest thing possible.
///
#[async_trait]
pub trait AbstractServer {
    /// Convert a searchfox tree-local path into an absolute analysis path on
    /// disk.  This fundamentally only works for local indices.
    fn translate_analysis_path(&self, sf_path: &str) -> Result<String>;

    async fn fetch_raw_analysis(&self, sf_path: &str) -> Result<BoxStream<Value>>;

    async fn fetch_html(&self, sf_path: &str) -> Result<String>;

    /// Retrieve the JSON contents of the crossref database for the given
    /// symbol.
    async fn crossref_lookup(&self, symbol: &str) -> Result<Value>;

    /// Given an identifier (prefix), return pairs of matching identifiers and
    /// symbols that correspond to those identifiers.
    ///
    /// If `exact_match` is true, then this is just a (potentially case-insensitive)
    /// lookup.  If it's false, then this is a prefix search that skips anything
    /// that looks like hierarchy traversal.  That is, if we are searching for
    /// a needle of "Foo", this will match "Food" and "Fool" but not
    /// "Food::Pizza" or "Food.Pizza" because `:` and `.` are considered
    /// indications of hierarchy traversal.
    async fn search_identifiers(&self, needle: &str, exact_match: bool, ignore_case: bool, match_limit: usize) -> Result<Vec<(String, String)>>;

    async fn perform_query(&self, q: &str) -> Result<Value>;
}
