use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DuxError {
    #[error("Path does not exist: {0}")]
    PathNotFound(PathBuf),

    #[error("Path is not a directory: {0}")]
    NotADirectory(PathBuf),

    #[error("Permission denied: {0}")]
    PermissionDenied(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Scan was cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, DuxError>;
