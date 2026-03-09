//! Extract command — scan documents and extract keyword candidates.

use anyhow::Context;
use clap::Args;
use colophon_core::config::Config;
use colophon_core::extract;
use tracing::{debug, instrument};

/// Arguments for the `extract` subcommand.
#[derive(Args, Debug, Default)]
pub struct ExtractArgs {
    /// Source directory to scan (overrides config)
    #[arg(short, long)]
    pub dir: Option<String>,

    /// Output file for candidates (default: colophon-candidates.yaml)
    #[arg(short, long, default_value = "colophon-candidates.yaml")]
    pub output: String,
}

/// Run the extraction pipeline.
#[instrument(name = "cmd_extract", skip_all)]
pub fn cmd_extract(args: ExtractArgs, json: bool, config: &Config) -> anyhow::Result<()> {
    debug!("executing extract command");

    let mut source = config.source.clone();
    if let Some(dir) = args.dir {
        source.dir = dir;
    }

    let candidates =
        extract::run(&source, &config.extract).context("extraction pipeline failed")?;

    if json {
        let json_out = serde_json::to_string_pretty(&candidates)
            .context("failed to serialize candidates as JSON")?;
        println!("{json_out}");
    } else {
        let yaml = candidates
            .to_yaml()
            .context("failed to serialize candidates")?;
        std::fs::write(&args.output, &yaml)
            .with_context(|| format!("failed to write {}", args.output))?;

        println!(
            "Extracted {} candidates from {} documents",
            candidates.candidates.len(),
            candidates.document_count
        );
        println!("Written to: {}", args.output);
    }

    Ok(())
}
