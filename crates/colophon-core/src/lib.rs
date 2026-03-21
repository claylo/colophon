//! Core library for colophon.
//!
//! This crate provides the foundational types and functionality used by the
//! `colophon` CLI and any downstream consumers.
//!
//! # Modules
//!
//! - [`config`] - Configuration loading and management
//! - [`error`] - Error types and result aliases
//! - [`extract`] - Extraction pipeline for keyword candidates
//! - [`curate`] - Claude-powered term curation pipeline
//! - [`render`] - Render pipeline for index markers and glossary
//!
//! # Quick Start
//!
//! ```no_run
//! use colophon_core::{Config, ConfigLoader};
//!
//! let (config, _sources) = ConfigLoader::new()
//!     .with_user_config(true)
//!     .load()
//!     .expect("Failed to load configuration");
//!
//! println!("Log level: {:?}", config.log_level);
//! ```
#![deny(unsafe_code)]

pub mod config;

pub mod curate;

pub mod error;

pub mod extract;

pub mod observability;

pub mod render;

pub(crate) mod typst_prose;

pub mod validate;

pub use config::{Config, ConfigLoader, LogLevel};

pub use error::{
    ConfigError, ConfigResult, CurateError, CurateResult, ExtractError, ExtractResult, RenderError,
    RenderResult,
};
