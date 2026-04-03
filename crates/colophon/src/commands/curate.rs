//! Curate command — AI-powered term curation via the Claude CLI.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Args;
use colophon_core::config::{Config, CurateConfig};
use colophon_core::curate;
use colophon_core::curate::cost::ModelPricing;
use colophon_core::curate::terms::CuratedTermsFile;
use colophon_core::extract::candidates::CandidatesFile;
use indicatif::{ProgressBar, ProgressStyle};
use tabled::{builder::Builder, settings::Style};
use tracing::{debug, instrument};

/// Arguments for the `curate` subcommand.
#[derive(Args, Debug, Default)]
pub struct CurateArgs {
    /// Path to candidates file (overrides config)
    #[arg(long)]
    pub candidates: Option<String>,

    /// Output directory for curated files
    #[arg(short, long, default_value = ".")]
    pub output_dir: String,

    /// Claude model to use (overrides config)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Send full YAML with context snippets instead of compact format
    #[arg(long)]
    pub full: bool,

    /// Estimate cost and exit without invoking Claude
    #[arg(long)]
    pub dry_run: bool,

    /// Abort if estimated cost exceeds this amount (USD)
    #[arg(long)]
    pub max_budget_usd: Option<f64>,

    /// Force full rebuild even when curated terms file exists
    #[arg(long)]
    pub full_rebuild: bool,

    /// Additional arguments passed through to the claude CLI
    #[arg(last = true)]
    pub claude_args: Vec<String>,
}

/// Display validation results (alias suggestions for unresolved locations).
fn display_validation(
    terms: &colophon_core::curate::terms::CuratedTermsFile,
    source_extensions: &[String],
) {
    let report =
        colophon_core::validate::validate_locations(terms, &terms.source_dir, source_extensions);
    if report.unresolved > 0 {
        eprintln!();
        eprintln!(
            "Validation: {} resolved, {} unresolved",
            report.resolved, report.unresolved
        );
        if !report.suggestions.is_empty() {
            eprintln!("Suggested aliases for unresolved locations:");
            for s in &report.suggestions {
                eprintln!("  {} -> add alias \"{}\"", s.term, s.suggested_alias);
            }
        }
        for detail in &report.unresolved_no_suggestion {
            if detail.file_missing {
                eprintln!("  {} in {} (file not found)", detail.term, detail.file);
            } else {
                eprintln!(
                    "  {} in {} (not found, no suggestion)",
                    detail.term, detail.file
                );
            }
        }
    }
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

/// Run the curation pipeline.
#[instrument(name = "cmd_curate", skip_all)]
pub fn cmd_curate(mut args: CurateArgs, json: bool, config: &Config) -> anyhow::Result<()> {
    debug!("executing curate command");
    if !json {
        super::banner();
    }

    let mut curate_config = config.curate.clone();
    if let Some(model) = args.model.take() {
        curate_config.model = model;
    }
    if let Some(ref candidates_path) = args.candidates {
        curate_config.candidates = candidates_path.clone();
    }
    if args.full {
        curate_config.full_candidates = true;
    }

    // Ensure output directory exists.
    let output_dir = Path::new(&args.output_dir);
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .with_context(|| format!("failed to create output directory {}", args.output_dir))?;
    }

    // Read the candidates file (keep raw YAML for --full mode).
    let candidates_yaml = std::fs::read_to_string(&curate_config.candidates)
        .with_context(|| format!("failed to read {}", curate_config.candidates))?;
    let candidates = CandidatesFile::from_yaml(&candidates_yaml)
        .with_context(|| format!("failed to parse {}", curate_config.candidates))?;

    // Auto-detect incremental mode: if terms file exists and --full-rebuild not set.
    let terms_path = Path::new(&args.output_dir).join("colophon-terms.yaml");
    if !args.full_rebuild && terms_path.exists() {
        return cmd_curate_incremental(
            args,
            json,
            &curate_config,
            candidates,
            &terms_path,
            &config.source.extensions,
        );
    }

    // Cost estimation (always run, needed for --dry-run and --max-budget-usd).
    let estimate = curate::estimate_cost(&candidates, &candidates_yaml, &curate_config);

    if args.dry_run {
        if json {
            let out = serde_json::json!({
                "input_tokens": estimate.input_tokens,
                "max_output_tokens": estimate.max_output_tokens,
                "model": estimate.model,
                "estimated_usd": estimate.estimated_usd,
                "estimated_cached_usd": estimate.estimated_cached_usd,
                "pricing_known": estimate.pricing_known,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            eprintln!("Cost estimate: {estimate}");
        }
        return Ok(());
    }

    // Budget check before invoking Claude (CLI flag overrides config).
    let budget = args.max_budget_usd.or(curate_config.max_budget_usd);
    if let Some(budget) = budget
        && estimate.pricing_known
        && estimate.estimated_usd > budget
    {
        anyhow::bail!(
            "estimated cost ${:.2} exceeds budget ${:.2} — use --full=false or a cheaper model to reduce cost",
            estimate.estimated_usd,
            budget,
        );
    }

    // Set up progress bar (hidden in --json mode).
    let pb = if json {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .expect("valid template"),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
        eprintln!(
            "Curating {} candidates from {} documents (model: {}, mode: {}, est: {estimate})...",
            candidates.candidates.len(),
            candidates.document_count,
            curate_config.model,
            if curate_config.full_candidates {
                "full"
            } else {
                "compact"
            },
        );
        pb
    };

    let start = Instant::now();
    let result = curate::run(
        &candidates,
        &candidates_yaml,
        &curate_config,
        &config.extract.known_terms,
        &args.claude_args,
        &pb,
    )
    .context("curation pipeline failed")?;
    let elapsed = start.elapsed();

    // Write output files.
    let terms_path = output_dir.join("colophon-terms.yaml");
    let thinking_path = output_dir.join("colophon-curated-thinking.md");

    if json {
        let json_out = serde_json::to_string_pretty(&result.terms_file)
            .context("failed to serialize as JSON")?;
        println!("{json_out}");
    } else {
        let yaml = result
            .terms_file
            .to_yaml()
            .context("failed to serialize curated terms")?;
        std::fs::write(&terms_path, &yaml)
            .with_context(|| format!("failed to write {}", terms_path.display()))?;

        // Save thinking output.
        if !result.thinking.is_empty() {
            std::fs::write(&thinking_path, &result.thinking)
                .with_context(|| format!("failed to write {}", thinking_path.display()))?;
        }

        // File sizes for the table.
        let terms_size = std::fs::metadata(&terms_path).map(|m| m.len()).unwrap_or(0);
        let thinking_size = std::fs::metadata(&thinking_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Count suggested terms from the curated output.
        let suggested_count = result
            .terms_file
            .terms
            .iter()
            .filter(|t| t.locations.is_empty())
            .count();

        let mut builder = Builder::default();
        builder.push_record(["Curated terms", &result.terms_file.terms.len().to_string()]);
        builder.push_record(["  (suggested)", &suggested_count.to_string()]);
        builder.push_record(["Model", &curate_config.model]);
        builder.push_record([
            "Mode",
            if curate_config.full_candidates {
                "full"
            } else {
                "compact"
            },
        ]);
        builder.push_record(["Turns", &result.turns.to_string()]);
        builder.push_record(["Thinking tokens", &result.thinking_tokens.to_string()]);

        // Actual token usage.
        let u = &result.usage;
        if u.input_tokens > 0 {
            builder.push_record(["Input tokens", &u.input_tokens.to_string()]);
        }
        if u.output_tokens > 0 {
            builder.push_record(["Output tokens", &u.output_tokens.to_string()]);
        }
        if u.cache_creation_input_tokens > 0 {
            builder.push_record([
                "Cache write tokens",
                &u.cache_creation_input_tokens.to_string(),
            ]);
        }
        if u.cache_read_input_tokens > 0 {
            builder.push_record(["Cache read tokens", &u.cache_read_input_tokens.to_string()]);
        }
        if let Some(pricing) = ModelPricing::for_model(&curate_config.model) {
            let actual_cost = u.actual_cost(&pricing);
            builder.push_record(["Actual cost", &format!("${actual_cost:.4}")]);
        }

        builder.push_record(["Time", &format!("{:.0}s", elapsed.as_secs_f64())]);
        builder.push_record([
            &format!("{}", terms_path.display()),
            &human_size(terms_size),
        ]);
        if thinking_size > 0 {
            builder.push_record([
                &format!("{}", thinking_path.display()),
                &human_size(thinking_size),
            ]);
        }

        let table = builder.build().with(Style::rounded()).to_string();
        eprintln!("\n{table}");

        // Print editorial summary if present.
        if !result.editorial.is_empty() {
            eprintln!("\n{}", result.editorial);
        }

        display_validation(&result.terms_file, &config.source.extensions);
    }

    Ok(())
}

/// Run incremental curation pipeline.
fn cmd_curate_incremental(
    args: CurateArgs,
    json: bool,
    config: &CurateConfig,
    candidates: CandidatesFile,
    terms_path: &Path,
    source_extensions: &[String],
) -> anyhow::Result<()> {
    let output_dir = Path::new(&args.output_dir);

    // Load existing terms file.
    let existing_yaml = std::fs::read_to_string(terms_path)
        .with_context(|| format!("failed to read {}", terms_path.display()))?;
    let existing = CuratedTermsFile::from_yaml(&existing_yaml)
        .with_context(|| format!("failed to parse {}", terms_path.display()))?;

    // Diff to get stats for display and cost estimation.
    let diff = colophon_core::curate::incremental::diff_candidates(&existing, &candidates);
    let compact_index = colophon_core::curate::incremental::format_compact_index(&existing);

    if diff.new_candidates.is_empty() && !json {
        eprintln!("No new terms found. Refreshing locations...");
    }

    // Serialize new candidates for cost estimation.
    let new_candidates_file = CandidatesFile {
        version: candidates.version,
        generated: candidates.generated.clone(),
        source_dir: candidates.source_dir.clone(),
        document_count: candidates.document_count,
        candidates: diff.new_candidates.clone(),
    };
    let new_yaml = serde_yaml::to_string(&new_candidates_file)?;

    // Cost estimation.
    if !diff.new_candidates.is_empty() {
        let estimate =
            curate::estimate_cost_incremental(&new_yaml, &compact_index, &diff.stale_terms, config);

        if args.dry_run {
            if json {
                let out = serde_json::json!({
                    "mode": "incremental",
                    "new_candidates": diff.new_candidates.len(),
                    "total_candidates": diff.total_candidates,
                    "stale_terms": diff.stale_terms.len(),
                    "input_tokens": estimate.input_tokens,
                    "max_output_tokens": estimate.max_output_tokens,
                    "model": estimate.model,
                    "estimated_usd": estimate.estimated_usd,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                eprintln!(
                    "Incremental: {} new of {} candidates ({:.0}%), {} stale",
                    diff.new_candidates.len(),
                    diff.total_candidates,
                    diff.new_ratio() * 100.0,
                    diff.stale_terms.len(),
                );
                eprintln!("Cost estimate: {estimate}");
            }
            return Ok(());
        }

        // Budget check.
        let budget = args.max_budget_usd.or(config.max_budget_usd);
        if let Some(budget) = budget
            && estimate.pricing_known
            && estimate.estimated_usd > budget
        {
            anyhow::bail!(
                "estimated cost ${:.2} exceeds budget ${:.2}",
                estimate.estimated_usd,
                budget,
            );
        }
    } else if args.dry_run {
        if json {
            let out = serde_json::json!({
                "mode": "incremental",
                "new_candidates": 0,
                "total_candidates": diff.total_candidates,
                "estimated_usd": 0.0,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            eprintln!("No new terms. Location refresh only — $0.00");
        }
        return Ok(());
    }

    // Set up progress.
    let pb = if json {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .expect("valid template"),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
        eprintln!(
            "Incremental curate: {} new of {} candidates ({:.0}%), {} stale (model: {})...",
            diff.new_candidates.len(),
            diff.total_candidates,
            diff.new_ratio() * 100.0,
            diff.stale_terms.len(),
            config.model,
        );
        pb
    };

    let start = Instant::now();
    let result = curate::run_incremental(&existing, &candidates, config, &args.claude_args, &pb)
        .context("incremental curation failed")?;
    let elapsed = start.elapsed();

    // Write output.
    let thinking_path = output_dir.join("colophon-curated-thinking.md");

    if json {
        let json_out = serde_json::to_string_pretty(&result.terms_file)?;
        println!("{json_out}");
    } else {
        let yaml = result.terms_file.to_yaml()?;
        std::fs::write(terms_path, &yaml)?;

        // Append thinking for incremental (audit trail).
        if !result.thinking.is_empty() {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&thinking_path)?;
            writeln!(
                f,
                "\n---\n## Incremental update {}\n\n{}",
                result.terms_file.generated, result.thinking
            )?;
        }

        let terms_size = std::fs::metadata(terms_path).map(|m| m.len()).unwrap_or(0);
        let ml = &result.merge_log;
        let unchanged = result
            .terms_file
            .terms
            .len()
            .saturating_sub(ml.added + ml.suggested);

        let mut builder = Builder::default();
        builder.push_record(["Mode", "incremental"]);
        builder.push_record(["Unchanged", &unchanged.to_string()]);
        builder.push_record(["Added", &ml.added.to_string()]);
        builder.push_record(["  (suggested)", &ml.suggested.to_string()]);
        builder.push_record(["Modified", &ml.modified.to_string()]);
        builder.push_record(["Removed", &ml.removed.to_string()]);
        builder.push_record([
            "New candidates",
            &format!(
                "{} of {} ({:.0}%)",
                (result.diff_ratio * diff.total_candidates as f64) as usize,
                diff.total_candidates,
                result.diff_ratio * 100.0,
            ),
        ]);
        if result.turns > 0 {
            builder.push_record(["Model", &config.model]);
            builder.push_record(["Turns", &result.turns.to_string()]);

            let u = &result.usage;
            if u.input_tokens > 0 {
                builder.push_record(["Input tokens", &u.input_tokens.to_string()]);
            }
            if u.output_tokens > 0 {
                builder.push_record(["Output tokens", &u.output_tokens.to_string()]);
            }
            if let Some(pricing) = ModelPricing::for_model(&config.model) {
                let actual_cost = u.actual_cost(&pricing);
                builder.push_record(["Actual cost", &format!("${actual_cost:.4}")]);
            }
        } else {
            builder.push_record(["Cost", "$0.00"]);
        }
        builder.push_record(["Time", &format!("{:.1}s", elapsed.as_secs_f64())]);
        builder.push_record([
            &format!("{}", terms_path.display()),
            &human_size(terms_size),
        ]);

        let table = builder.build().with(Style::rounded()).to_string();
        eprintln!("\n{table}");

        if !result.editorial.is_empty() {
            eprintln!("\n{}", result.editorial);
        }

        display_validation(&result.terms_file, source_extensions);
    }

    Ok(())
}
