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

/// Errors that can occur during curation.
#[derive(Error, Debug)]
pub enum CurateError {
    /// The `claude` CLI was not found in PATH.
    #[error("claude CLI not found in PATH — install from https://claude.com/claude-code")]
    ClaudeNotFound,

    /// The `claude` CLI exited with an error.
    #[error("claude CLI failed (exit {exit_code:?}): {stderr}")]
    ClaudeFailed {
        /// Process exit code (None if killed by signal).
        exit_code: Option<i32>,
        /// Captured stderr output.
        stderr: String,
    },

    /// Failed to parse the Claude CLI JSON response.
    #[error("failed to parse claude response: {detail}")]
    ParseResponse {
        /// What went wrong.
        detail: String,
    },

    /// Candidates file is missing or empty.
    #[error("no candidates to curate: {0}")]
    NoCandidates(
        /// Path to the candidates file.
        String,
    ),

    /// Failed to read or write files.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to serialize output.
    #[error("failed to serialize curated terms: {0}")]
    Serialize(#[from] serde_yaml::Error),
}

/// Result type alias using [`CurateError`].
pub type CurateResult<T> = Result<T, CurateError>;

/// Errors that can occur during rendering.
#[derive(Error, Debug)]
pub enum RenderError {
    /// Failed to read or write files.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to deserialize the curated terms file.
    #[error("failed to parse terms file: {0}")]
    ParseTerms(#[from] serde_yaml::Error),

    /// Cycle detected in parent chain.
    #[error("cycle in parent chain: {chain}")]
    ParentCycle {
        /// The cycle path (e.g., "A -> B -> A").
        chain: String,
    },

    /// No terms found in the terms file.
    #[error("no terms in {0}")]
    NoTerms(String),
}

/// Result type alias using [`RenderError`].
pub type RenderResult<T> = Result<T, RenderError>;
