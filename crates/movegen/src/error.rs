//! Errors.

use std::path::Path;

/// A `movegen` error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Invalid configuration.
    #[error("config: {0}")]
    Config(String),
    /// Invalid or outdated lock.
    #[error("lock: {0}")]
    Lock(String),
    /// A package could not be read.
    #[error("package {0}")]
    Package(String),
    /// The dependency closure is incomplete.
    #[error("closure: {0}")]
    Closure(String),
    /// Generation failed.
    #[error("generate: {0}")]
    Generate(String),
    /// Fetching from a network failed.
    #[error("fetch: {0}")]
    Fetch(String),
    /// Filesystem error.
    #[error("{path}: {message}")]
    Io {
        /// Path involved.
        path: String,
        /// Error message.
        message: String,
    },
}

impl Error {
    pub(crate) fn io(path: &Path, e: &std::io::Error) -> Self {
        Error::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        }
    }
}
