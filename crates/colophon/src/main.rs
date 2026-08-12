//! colophon CLI
#![deny(unsafe_code)]

use anyhow::Context;
use colophon::{Cli, Commands, commands};
use tracing::debug;

fn main() -> anyhow::Result<()> {
    // `parse_with` answers `schema` and `completions` before config, logging,
    // or any other work happens.
    let cli = librebar::cli::parse_with::<Cli>(colophon::schema_metadata());

    // Applies color, then --version-only, then -C, in that fixed order: the
    // version query cannot be broken by an unrelated bad -C, and the directory
    // change lands before config discovery walks up from cwd.
    if cli.common.apply(env!("CARGO_PKG_VERSION"))?.is_exit() {
        return Ok(());
    }

    // arg_required_else_help ensures we have --version-only or a subcommand
    let Some(command) = cli.command else {
        return Ok(());
    };

    let json = matches!(
        cli.common.output_format(),
        librebar::cli::ResolvedOutputFormat::Json
    );

    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let cwd = camino::Utf8PathBuf::try_from(cwd).map_err(|e| {
        anyhow::anyhow!(
            "current directory is not valid UTF-8: {}",
            e.into_path_buf().display()
        )
    })?;
    let config_path = cli.common.config_path()?;

    let mut loader = colophon_core::ConfigLoader::new().with_project_search(&cwd);
    if let Some(ref path) = config_path {
        loader = loader.with_file(path);
    }
    let (config, config_sources) = loader.load().context("failed to load configuration")?;

    // Hold `app` for the rest of main: dropping it flushes and closes the log
    // writer. librebar reads log_dir and log_level off the config we hand it,
    // so colophon-core stays authoritative over config semantics.
    let app = librebar::init(env!("CARGO_PKG_NAME"))
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_cli(cli.common)
        .with_config(config)
        .logging()
        .crash_handler()
        .start()
        .context("failed to initialize logging and crash handling")?;

    let config = app.config().clone();

    debug!(
        verbose = app.cli().verbose,
        quiet = app.cli().quiet,
        json,
        "CLI initialized"
    );

    // Execute command
    let result = match command {
        Commands::Curate(args) => commands::curate::cmd_curate(args, json, &config),
        Commands::Doctor(args) => commands::doctor::cmd_doctor(args, json, &config_sources, &cwd),
        Commands::Extract(args) => commands::extract::cmd_extract(args, json, &config),
        Commands::Info(args) => commands::info::cmd_info(args, json, &config, &config_sources),
        Commands::Render(args) => commands::render::cmd_render(args, json, &config),
    };
    // Logged here rather than after `app` drops, because `app` owns the log writer.
    if let Err(ref err) = result {
        tracing::error!(error = %err, "fatal error");
    }
    result
}
