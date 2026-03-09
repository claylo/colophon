//! Core library for colophon.
//!
//! This crate provides the foundational types and functionality used by the
//! `colophon` CLI and any downstream consumers.
//!
//! # Modules
//!
//! - [`config`] - Configuration loading and management
//! - [`error`] - Error types and result aliases
//!
//! # Quick Start
//!
//! ```no_run
//! use colophon_core::{Config, ConfigLoader};
//!
//! let config = ConfigLoader::new()
//!     .with_user_config(true)
//!     .load()
//!     .expect("Failed to load configuration");
//!
//! println!("Log level: {:?}", config.log_level);
//! ```
#![deny(unsafe_code)]

pub mod config;

pub mod error;

pub use config::{Config, ConfigLoader, LogLevel};

pub use error::{ConfigError, ConfigResult};
