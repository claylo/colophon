//! Extract command — scan documents and extract keyword candidates.

use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Args;
use colophon_core::config::Config;
use colophon_core::extract;
use indicatif::{ProgressBar, ProgressStyle};
use tabled::{builder::Builder, settings::Style};
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

/// Format a byte count as a human-readable size.
fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Run the extraction pipeline.
#[instrument(name = "cmd_extract", skip_all)]
pub fn cmd_extract(args: ExtractArgs, json: bool, config: &Config) -> anyhow::Result<()> {
    debug!("executing extract command");

    let mut source = config.source.clone();
    if let Some(dir) = args.dir {
        source.dir = dir;
    }

    let pb = if json {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} [{bar:30.cyan/dim}] {pos}/{len}  {msg}")
                .expect("valid template")
                .progress_chars("━╸─"),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
        pb
    };

    let start = Instant::now();
    let candidates = extract::run_with_progress(&source, &config.extract, &pb)
        .context("extraction pipeline failed")?;
    let elapsed = start.elapsed();

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

        let file_size = std::fs::metadata(&args.output)
            .map(|m| m.len())
            .unwrap_or(0);

        let mut builder = Builder::default();
        builder.push_record(["Source", &candidates.source_dir]);
        builder.push_record(["Documents", &candidates.document_count.to_string()]);
        builder.push_record(["Candidates", &candidates.candidates.len().to_string()]);
        builder.push_record(["Time", &format!("{:.1}s", elapsed.as_secs_f64())]);
        builder.push_record(["Output", &args.output]);
        builder.push_record(["Size", &human_size(file_size)]);

        let table = builder.build().with(Style::rounded()).to_string();
        eprintln!("\n{table}");
    }

    Ok(())
}
