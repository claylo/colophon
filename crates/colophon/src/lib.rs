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
//! The [`command()`] function returns the clap `Command` for generating man pages
//! and shell completions via `xtask`.
pub mod commands;

use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;

/// Color output preference.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum ColorChoice {
    /// Detect terminal capabilities automatically.
    #[default]
    Auto,
    /// Always emit colors.
    Always,
    /// Never emit colors.
    Never,
}

impl ColorChoice {
    /// Configure global color output based on this choice.
    ///
    /// Call this once at startup to set the color mode.
    pub fn apply(self) {
        match self {
            Self::Auto => {} // owo-colors auto-detects by default
            Self::Always => owo_colors::set_override(true),
            Self::Never => owo_colors::set_override(false),
        }
    }
}

const ENV_HELP: &str = "\
ENVIRONMENT VARIABLES:
    RUST_LOG                Log filter (e.g., debug, colophon=trace)
    COLOPHON_LOG_PATH     Log file path (rotated daily)
    COLOPHON_LOG_DIR      Log directory
";
/// Command-line interface definition for colophon.
#[derive(Parser)]
#[command(name = "colophon")]
#[command(about = "Generate book indexes and glossaries from Markdown or Typst.", long_about = None)]
#[command(version, arg_required_else_help = true)]
#[command(after_long_help = ENV_HELP)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Print only the version number (for scripting)
    #[arg(long)]
    pub version_only: bool,

    /// Path to configuration file (overrides discovery)
    #[arg(short, long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Run as if started in DIR
    #[arg(short = 'C', long, global = true)]
    pub chdir: Option<PathBuf>,

    /// Only print errors (suppresses warnings/info)
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// More detail (repeatable; e.g. -vv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Colorize output
    #[arg(long, global = true, value_enum, default_value_t)]
    pub color: ColorChoice,

    /// Output as JSON (for scripting)
    #[arg(long, global = true)]
    pub json: bool,
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

/// Returns the clap command for documentation generation
pub fn command() -> clap::Command {
    Cli::command()
}
