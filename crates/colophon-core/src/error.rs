//! Error types for colophon-core

use thiserror::Error;

/// Errors that can occur when working with configuration.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Failed to deserialize configuration.
    #[error("invalid configuration: {0}")]
    Deserialize(#[from] Box<figment::Error>),

    /// Configuration file not found after searching all locations.
    #[error("no configuration file found")]
    NotFound,
}

/// Result type alias using [`ConfigError`].
pub type ConfigResult<T> = Result<T, ConfigError>;

/// Errors that can occur during extraction.
#[derive(Error, Debug)]
pub enum ExtractError {
    /// Failed to read a source file.
    #[error("failed to read {path}: {source}")]
    ReadFile {
        /// Path to the file that could not be read.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to walk the source directory.
    #[error("failed to walk directory: {0}")]
    WalkDir(#[from] walkdir::Error),

    /// Failed to serialize candidates.
    #[error("failed to serialize candidates: {0}")]
    Serialize(#[from] serde_yaml::Error),

    /// No documents found in source directory.
    #[error("no documents found in {0}")]
    NoDocuments(
        /// The directory that was scanned.
        String,
    ),
}

/// Result type alias using [`ExtractError`].
pub type ExtractResult<T> = Result<T, ExtractError>;
