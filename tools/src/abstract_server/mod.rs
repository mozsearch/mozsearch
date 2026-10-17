mod local_index;
mod remote_server;
mod server_interface;
pub mod sym_info;
mod try_overlay;

pub use local_index::{make_all_local_servers, make_local_server};
pub use remote_server::make_remote_server;
pub use try_overlay::TryOverlay;
pub use server_interface::{
    AbstractServer, CommitInfo, ErrorDetails, ErrorLayer, FileMatch, FileMatches, HtmlFileRoot,
    Result, SearchfoxIndexRoot, ServerError, TextBounds, TextMatchInFile, TextMatches,
    TextMatchesByFile, TextPattern, TreeInfo,
};
