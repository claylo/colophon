//! Render command — annotate source files with index markers.

use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use clap::Args;
use colophon_core::config::Config;
use colophon_core::curate::terms::CuratedTermsFile;
use colophon_core::render::{self, RenderFormat};
use tabled::{builder::Builder, settings::Style};
use tracing::{debug, instrument};

/// Arguments for the `render` subcommand.
#[derive(Args, Debug, Default)]
pub struct RenderArgs {
    /// Path to curated terms file
    #[arg(long, default_value = "colophon-terms.yaml")]
    pub terms: String,

    /// Source directory to scan (defaults to source_dir from terms file)
    #[arg(short, long)]
    pub dir: Option<String>,

    /// Output format
    #[arg(long, default_value = "typst")]
    pub format: String,

    /// Output directory for annotated files
    #[arg(short, long, default_value = ".")]
    pub output_dir: String,

    /// Also emit a standalone glossary document
    #[arg(long)]
    pub glossary: bool,

    /// Only insert markers for main (substantive) locations
    #[arg(long)]
    pub main_only: bool,

    /// Spacing between glossary entries (Typst length, e.g. "12pt", "1.5em")
    #[arg(long)]
    pub glossary_spacing: Option<String>,
}

/// Run the render pipeline.
#[instrument(name = "cmd_render", skip_all)]
pub fn cmd_render(args: RenderArgs, json: bool, config: &Config) -> anyhow::Result<()> {
    debug!("executing render command");

    // Parse format.
    let format = match args.format.as_str() {
        "typst" => RenderFormat::Typst,
        other => anyhow::bail!("unsupported render format: {other} (supported: typst)"),
    };

    // Read the curated terms file.
    let terms_yaml = std::fs::read_to_string(&args.terms)
        .with_context(|| format!("failed to read {}", args.terms))?;
    let terms = CuratedTermsFile::from_yaml(&terms_yaml)
        .with_context(|| format!("failed to parse {}", args.terms))?;

    // Use --dir if provided, otherwise fall back to the terms file's source_dir,
    // and finally to config.source.dir.
    let source_dir = args.dir.unwrap_or_else(|| {
        if !terms.source_dir.is_empty() {
            terms.source_dir.clone()
        } else {
            config.source.dir.clone()
        }
    });

    if !json {
        eprintln!(
            "Rendering {} terms from {} (source: {}, format: {}, glossary: {})...",
            terms.terms.len(),
            args.terms,
            source_dir,
            args.format,
            if args.glossary { "yes" } else { "no" },
        );
    }

    // Ensure output directory exists.
    let output_dir = Path::new(&args.output_dir);
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .with_context(|| format!("failed to create output directory {}", args.output_dir))?;
    }

    let start = Instant::now();
    let result = render::run(
        &terms,
        &source_dir,
        &config.source.extensions,
        &args.output_dir,
        args.glossary,
        args.main_only,
        args.glossary_spacing.as_deref(),
        format,
    )
    .context("render pipeline failed")?;
    let elapsed = start.elapsed();

    if json {
        let json_out =
            serde_json::to_string_pretty(&result).context("failed to serialize as JSON")?;
        println!("{json_out}");
    } else {
        let mut builder = Builder::default();
        builder.push_record(["Files annotated", &result.files_annotated.to_string()]);
        builder.push_record(["Markers inserted", &result.markers_inserted.to_string()]);
        builder.push_record(["  (main)", &result.markers_main.to_string()]);
        if result.terms_not_found > 0 {
            builder.push_record(["Terms not found", &result.terms_not_found.to_string()]);
        }
        if result.glossary_terms > 0 {
            builder.push_record(["Glossary terms", &result.glossary_terms.to_string()]);
        }
        builder.push_record(["Output directory", &args.output_dir]);
        builder.push_record(["Time", &format!("{:.1}s", elapsed.as_secs_f64())]);

        let table = builder.build().with(Style::rounded()).to_string();
        eprintln!("\n{table}");

        if !result.not_found_details.is_empty() {
            eprintln!("\nTerms not found in source:");
            for nf in &result.not_found_details {
                eprintln!("  {} (expected in {})", nf.term, nf.file);
            }
        }
    }

    Ok(())
}
