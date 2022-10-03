mod local_index;
mod remote_server;
mod server_interface;

pub use local_index::{make_all_local_servers, make_local_server};
pub use remote_server::make_remote_server;
pub use server_interface::{
    AbstractServer, ErrorDetails, ErrorLayer, FileMatch, FileMatches, HtmlFileRoot, Result,
    SearchfoxIndexRoot, ServerError, TextMatches, TextMatchesByFile, TreeInfo,
};
