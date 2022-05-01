mod local_index;
mod remote_server;
mod server_interface;

pub use local_index::make_local_server;
pub use remote_server::make_remote_server;
pub use server_interface::{
    AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError, TextMatches, TextMatchesByFile,
};
