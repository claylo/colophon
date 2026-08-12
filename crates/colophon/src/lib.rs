//! Library interface for the `colophon` CLI.
//!
//! This crate exposes the CLI's argument parser and command structure as a library,
//! primarily for documentation generation and testing. The actual entry point is
//! in `main.rs`.
//!
//! # Structure
//!
//! - [`Cli`] - The root argument parser (clap derive)
//! - [`Commands`] - Available subcommands
//! - [`commands`] - Command implementations
//!
//! # Documentation Generation
//!
//! `xtask` builds man pages and shell completions from [`Cli`] through
//! librebar's generators, which render the same augmented command tree that
//! `librebar::cli::parse_with` uses at runtime.
pub mod commands;

use librebar::cli::clap::{Parser, Subcommand};

const BANNER: &str = "\
┌─┐┌─┐┬  ┌─┐┌─┐┬ ┬┌─┐┌┐┌
│  │ ││  │ │├─┘├─┤│ ││││
└─┘└─┘┴─┘└─┘┴  ┴ ┴└─┘┘└┘";

const ENV_HELP: &str = "\
ENVIRONMENT VARIABLES:
    RUST_LOG             Log filter (e.g., debug, colophon=trace)
    COLOPHON_LOG_PATH    Log file path (rotated daily)
    COLOPHON_LOG_DIR     Log directory
";

/// Command-line interface definition for colophon.
#[derive(Parser)]
#[command(name = "colophon")]
#[command(about = "Generate book indexes and glossaries from Markdown or Typst.", long_about = None)]
#[command(version, arg_required_else_help = true)]
#[command(before_help = BANNER)]
#[command(after_help = ENV_HELP)]
pub struct Cli {
    /// Flags shared by every librebar-based CLI.
    ///
    /// Supplies `-q`, `-v`, `-C`, `-c`, `--color`, `--format`, and
    /// `--version-only`, all global. colophon declares none of these itself:
    /// redeclaring one is a clap name collision, which panics at startup
    /// rather than failing to compile.
    #[command(flatten)]
    pub common: librebar::cli::CommonArgs,

    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available subcommands for the CLI.
#[derive(Subcommand)]
pub enum Commands {
    /// Curate extracted candidates into an index using Claude
    Curate(commands::curate::CurateArgs),
    /// Diagnose configuration and environment
    Doctor(commands::doctor::DoctorArgs),
    /// Extract keyword candidates from documents
    Extract(commands::extract::ExtractArgs),
    /// Show package information
    Info(commands::info::InfoArgs),
    /// Render curated terms as index markers in source files
    Render(commands::render::RenderArgs),
}

/// Machine-readable declaration of colophon's exit contract.
///
/// Consumed by `librebar::cli::parse_with`, which validates it and serves it
/// from the `schema` subcommand.
pub fn schema_metadata() -> librebar::cli::SchemaMetadata {
    librebar::cli::SchemaMetadata::new().error(
        librebar::cli::ErrorMetadata::new("config_invalid")
            .exit_code(2)
            .retryable(false)
            .description("Configuration could not be loaded or deserialized"),
    )
}
