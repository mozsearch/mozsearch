use crate::abstract_server::{ErrorDetails, ErrorLayer, Result, ServerError};

pub fn write_file_ensuring_parent_dir(file_path: &str, contents: &str) -> Result<()> {
    let as_path = std::path::Path::new(file_path);
    let parent_path = match as_path.parent() {
        Some(p) => p,
        None => {
            return Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                message: format!("Problem getting parent of '{}'", file_path),
            }));
        }
    };
    if let Err(e) = std::fs::create_dir_all(parent_path) {
        return Err(ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::DataLayer,
            message: format!("Problem creating parent of '{}': {}", file_path, e),
        }));
    }
    std::fs::write(as_path, contents)?;
    Ok(())
}
