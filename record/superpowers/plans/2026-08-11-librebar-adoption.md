# librebar 0.6 Adoption — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hand-rolled CLI flags, config loading, and observability with librebar 0.6 — gaining crash dumps, update checks, CLI Spec schema, and diagnostics framework for free.

**Architecture:** Split librebar across the workspace like bito: `colophon-core` gets `config` only (no process-global state), `colophon` (CLI binary) gets `cli`, `config`, `logging`, `crash`, `diagnostics`, `update`. The xtask gets `cli` only for completions/manpages via librebar generators. Config types stay in colophon-core — only the loading mechanism changes.

**Tech Stack:** librebar 0.6, clap 4.6 (through `librebar::cli::clap` re-export), camino 1.2

**Reference implementation:** `~/source/claylo/bito` — completed this migration 2026-08-07.

---

## File Structure

### Files to Create
- None — this is a replacement, not an addition.

### Files to Modify
- `crates/colophon-core/Cargo.toml` — add librebar, remove figment, directories, tracing-subscriber, tracing-appender
- `crates/colophon-core/src/config.rs` — replace `ConfigLoader` with thin wrapper over `librebar::config::ConfigLoader`
- `crates/colophon-core/src/error.rs` — change `ConfigError::Deserialize` from `figment::Error` to `Box<dyn Error>`
- `crates/colophon-core/src/observability.rs` — delete all contents (becomes empty or removed)
- `crates/colophon-core/src/lib.rs` — remove `pub mod observability`
- `crates/colophon/Cargo.toml` — add librebar, remove direct clap (use re-export)
- `crates/colophon/src/lib.rs` — replace `Cli` struct with `CommonArgs` flatten, replace `command()` with `parse_with`, add schema metadata
- `crates/colophon/src/main.rs` — rewrite to librebar init builder pattern
- `crates/colophon/src/commands/mod.rs` — no changes needed (banner stays)
- `crates/colophon/src/commands/doctor.rs` — use `librebar::config` for directory resolution
- `crates/colophon/src/commands/info.rs` — adapt to new config/CLI types
- `crates/colophon/src/commands/extract.rs` — adapt `json` bool to output format enum
- `crates/colophon/src/commands/curate.rs` — adapt `json` bool to output format enum
- `crates/colophon/src/commands/render.rs` — adapt `json` bool to output format enum
- `crates/colophon/tests/cli.rs` — update for new flag behavior (--format replaces --json)
- `crates/colophon/tests/config_integration.rs` — update for new config loader API
- `xtask/Cargo.toml` — add librebar cli feature, remove clap_complete and clap_mangen
- `xtask/src/main.rs` — switch to librebar generators
- `xtask/src/commands/completions.rs` — use librebar completion generator
- `xtask/src/commands/man.rs` — use librebar manpage generator

### Files to Delete
- `crates/colophon-core/src/observability.rs` — replaced by `librebar::logging`

---

## Task 1: Add librebar to colophon-core, replace config loader

This is the foundation. Replace figment-based `ConfigLoader` with a thin wrapper over `librebar::config::ConfigLoader`. Config types (`Config`, `SourceConfig`, `ExtractConfig`, etc.) stay exactly as they are — only the loading mechanism changes.

**Files:**
- Modify: `crates/colophon-core/Cargo.toml`
- Modify: `crates/colophon-core/src/config.rs`
- Modify: `crates/colophon-core/src/error.rs`

- [ ] **Step 1: Update colophon-core dependencies**

In `crates/colophon-core/Cargo.toml`, add librebar and remove figment and directories:

```toml
# Replace this:
figment = { version = "0.10", features = ["toml", "yaml", "json"] }
# ...
directories = "6.0"

# With this:
librebar = { version = "0.6", default-features = false, features = ["config"] }
```

Keep all other dependencies unchanged.

- [ ] **Step 2: Update ConfigError to not leak figment**

In `crates/colophon-core/src/error.rs`, change the `Deserialize` variant:

```rust
// Before:
#[error("invalid configuration: {0}")]
Deserialize(#[from] Box<figment::Error>),

// After:
#[error("invalid configuration: {0}")]
Deserialize(Box<dyn std::error::Error + Send + Sync>),
```

Remove the `#[from]` since we'll construct it explicitly.

- [ ] **Step 3: Rewrite ConfigLoader as a thin wrapper**

Replace the `ConfigLoader` struct and its methods in `crates/colophon-core/src/config.rs`. Remove all figment imports. The new loader delegates to librebar:

```rust
use librebar::config::ConfigLoader as LibrebarLoader;

const APP_NAME: &str = "colophon";

pub struct ConfigLoader {
    inner: LibrebarLoader,
    explicit_files: Vec<Utf8PathBuf>,
}

impl ConfigLoader {
    pub fn new() -> Self {
        Self {
            inner: LibrebarLoader::new(APP_NAME),
            explicit_files: Vec::new(),
        }
    }

    pub fn with_project_search<P: AsRef<Utf8Path>>(mut self, path: P) -> Self {
        self.inner = self.inner.with_project_search(path);
        self
    }

    pub fn with_file<P: AsRef<Utf8Path>>(mut self, path: P) -> Self {
        self.explicit_files.push(path.as_ref().to_path_buf());
        self.inner = self.inner.with_file(path);
        self
    }

    pub fn with_user_config(mut self, include: bool) -> Self {
        if !include {
            self.inner = self.inner.without_user_config();
        }
        self
    }

    pub fn with_boundary_marker<S: Into<String>>(mut self, marker: S) -> Self {
        self.inner = self.inner.with_boundary_marker(marker);
        self
    }

    pub fn without_boundary_marker(mut self) -> Self {
        self.inner = self.inner.without_boundary_marker();
        self
    }

    #[tracing::instrument(skip(self))]
    pub fn load(self) -> ConfigResult<(Config, ConfigSources)> {
        let (config, lb_sources) = self.inner
            .load::<Config>()
            .map_err(|e| ConfigError::Deserialize(Box::new(e)))?;
        
        let sources = ConfigSources {
            project_file: lb_sources.project_file().map(Utf8PathBuf::from),
            user_file: lb_sources.user_file().map(Utf8PathBuf::from),
            explicit_files: self.explicit_files,
        };
        
        Ok((config, sources))
    }

    pub fn load_or_error(self) -> ConfigResult<(Config, ConfigSources)> {
        // librebar's load already returns an error if no config found
        // when using load_required; but we match our existing API
        self.load()
    }
}
```

Note: Check librebar's actual `ConfigSources` API — the exact accessor names (`project_file()`, `user_file()`) need to match what librebar exposes. Reference `~/source/claylo/bito/crates/bito-core/src/config.rs` for the working pattern.

- [ ] **Step 4: Replace XDG directory helpers**

Replace the `project_dirs()`, `user_config_dir()`, `user_cache_dir()`, `user_data_dir()`, `user_data_local_dir()` functions with librebar delegations:

```rust
pub fn user_config_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_config_dir(APP_NAME)
}

pub fn user_cache_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_cache_dir(APP_NAME)
}

pub fn user_data_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_data_dir(APP_NAME)
}

pub fn user_data_local_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_data_local_dir(APP_NAME)
}
```

Check librebar's actual function signatures — they may take `&str` or return `Utf8PathBuf` directly. Reference bito-core's config.rs for the exact signatures.

- [ ] **Step 5: Remove stale helpers**

Delete:
- The `find_project_config()` method
- The `find_user_config()` method
- The `merge_file()` method
- The `CONFIG_EXTENSIONS` constant
- The `project_dirs()` function

These are all replaced by librebar's config discovery.

- [ ] **Step 6: Run tests to verify config loading still works**

```bash
cargo nextest run -p colophon-core -- config
```

All config tests should pass since the Config struct and its serde Deserialize impl are unchanged — only the loader changed. Fix any that break due to API differences.

- [ ] **Step 7: Commit**

```
chore(config): replace figment with librebar config loader

Thin wrapper over librebar::config::ConfigLoader preserves colophon's
existing ConfigLoader API. Config types unchanged. Removes figment
and directories as direct dependencies.
```

---

## Task 2: Delete observability module, add librebar to CLI crate

Remove the hand-rolled JSONL logging layer and observability setup. The CLI crate will use `librebar::logging` via the init builder in Task 3.

**Files:**
- Modify: `crates/colophon-core/src/lib.rs`
- Delete: `crates/colophon-core/src/observability.rs`
- Modify: `crates/colophon/Cargo.toml`

- [ ] **Step 1: Remove observability module from core**

In `crates/colophon-core/src/lib.rs`, remove:

```rust
pub mod observability;
```

- [ ] **Step 2: Delete observability.rs**

```bash
rm crates/colophon-core/src/observability.rs
```

- [ ] **Step 3: Remove tracing-subscriber and tracing-appender from colophon-core**

In `crates/colophon-core/Cargo.toml`, remove:

```toml
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
```

Keep `tracing = "0.1"` — core still uses `tracing::debug!`, `tracing::instrument`, etc.

- [ ] **Step 4: Update CLI crate dependencies**

In `crates/colophon/Cargo.toml`:

```toml
# Add:
librebar = { version = "0.6", features = [
    "cli", "config", "logging", "crash", "diagnostics", "update",
] }

# Remove direct clap (use librebar's re-export):
# clap = { version = "4.6", features = ["derive"] }   <-- delete this line
```

Keep: `owo-colors`, `indicatif`, `tabled`, `serde`, `serde_json`, `serde_yaml`, `anyhow`, `tracing`, `camino`.

- [ ] **Step 5: Verify it compiles (will have errors in main.rs/lib.rs — expected)**

```bash
cargo check -p colophon-core 2>&1 | head -20
```

Core should compile clean. The colophon CLI crate will have errors because main.rs still references observability — that's fixed in Task 3.

- [ ] **Step 6: Commit core changes only**

```
refactor(core): remove observability module

Logging setup moves to the CLI binary via librebar::logging.
Core crate keeps tracing instrumentation but no longer owns
the subscriber or log writer.
```

---

## Task 3: Rewrite CLI struct and main.rs with librebar

Replace the hand-rolled `Cli` struct with `CommonArgs` flatten, and rewrite `main.rs` to use the librebar init builder.

**Files:**
- Modify: `crates/colophon/src/lib.rs`
- Modify: `crates/colophon/src/main.rs`

- [ ] **Step 1: Rewrite lib.rs with CommonArgs**

Replace the entire `Cli` struct and `command()` function. Key changes:
- Use `librebar::cli::clap::{Parser, Subcommand}` instead of direct clap
- Flatten `CommonArgs` instead of hand-rolling flags
- Add `schema_metadata()` function
- Remove `ColorChoice` enum (librebar handles it)
- Keep `BANNER` and `ENV_HELP`

```rust
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

#[derive(Parser)]
#[command(name = "colophon")]
#[command(about = "Generate book indexes and glossaries from Markdown or Typst.", long_about = None)]
#[command(version, arg_required_else_help = true)]
#[command(before_help = BANNER)]
#[command(after_help = ENV_HELP)]
pub struct Cli {
    #[command(flatten)]
    pub common: librebar::cli::CommonArgs,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

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

pub fn schema_metadata() -> librebar::cli::SchemaMetadata {
    librebar::cli::SchemaMetadata::new()
        .error(
            librebar::cli::ErrorMetadata::new("config_invalid")
                .exit_code(2)
                .retryable(false)
                .description("Configuration could not be discovered, parsed, or deserialized"),
        )
}
```

**Important:** Check if `CommonArgs` includes `-c/--config`. If it does, colophon's existing `-c` (short for `--config`) won't collide — they're the same flag. But verify at runtime.

- [ ] **Step 2: Rewrite main.rs with librebar init builder**

```rust
#![deny(unsafe_code)]

use anyhow::Context;
use colophon::{Cli, Commands, commands};
use tracing::debug;

fn main() -> anyhow::Result<()> {
    let cli = librebar::cli::parse_with::<Cli>(colophon::schema_metadata());

    if cli.common.apply(env!("CARGO_PKG_VERSION"))?.is_exit() {
        return Ok(());
    }

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

    let result = match command {
        Commands::Curate(args) => commands::curate::cmd_curate(args, json, &config),
        Commands::Doctor(args) => {
            commands::doctor::cmd_doctor(args, json, &config_sources, &cwd)
        }
        Commands::Extract(args) => commands::extract::cmd_extract(args, json, &config),
        Commands::Info(args) => commands::info::cmd_info(args, json, &config, &config_sources),
        Commands::Render(args) => commands::render::cmd_render(args, json, &config),
    };
    if let Err(ref err) = result {
        tracing::error!(error = %err, "fatal error");
    }
    result
}
```

Note: `cli.common.apply()` handles `--color`, `--version-only`, and `--chdir` in the correct order — replacing colophon's manual `cli.color.apply()`, version check, and `set_current_dir`. `cli.common.config_path()` provides the `--config`/`-c` value. Check librebar's actual method names.

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p colophon
```

Fix any type mismatches. The most likely issues:
- `config_path()` return type may differ from what we expect
- `app.config()` returns `&C` not `C` — may need `.clone()`
- `app.cli()` field names for verbose/quiet

- [ ] **Step 4: Commit**

```
refactor(cli): adopt librebar for CLI parsing and init

Replace hand-rolled --quiet, --verbose, --color, --json, --chdir,
--config, and --version-only with librebar CommonArgs flatten.
Gains --format auto|text|json, CLI Spec schema subcommand,
crash dumps, and structured logging via librebar builder.
```

---

## Task 4: Update xtask for librebar generators

Replace clap_complete and clap_mangen with librebar's built-in generators.

**Files:**
- Modify: `xtask/Cargo.toml`
- Modify: `xtask/src/commands/completions.rs`
- Modify: `xtask/src/commands/man.rs`

- [ ] **Step 1: Update xtask dependencies**

In `xtask/Cargo.toml`:

```toml
[dependencies]
clap = { version = "4.6", features = ["derive"] }
colophon = { path = "../crates/colophon" }
librebar = { version = "0.6", default-features = false, features = ["cli"] }
```

Remove `clap_complete` and `clap_mangen`.

- [ ] **Step 2: Update completions command**

In `xtask/src/commands/completions.rs`, use librebar's completion generator. Check bito's xtask for the exact API — it likely uses `librebar::cli::generate_completions::<Cli>()` or similar. The pattern from bito's xtask is the reference.

- [ ] **Step 3: Update man command**

In `xtask/src/commands/man.rs`, use `librebar::cli::generate_manpages::<colophon::Cli>()`. Filter out the `help` subcommand junk page (known gotcha from bito migration).

- [ ] **Step 4: Verify xtask works**

```bash
cargo xtask completions --out-dir /tmp/colophon-completions
cargo xtask man --out-dir /tmp/colophon-man
ls /tmp/colophon-completions/ /tmp/colophon-man/
```

- [ ] **Step 5: Commit**

```
refactor(xtask): use librebar generators for completions and manpages

Removes clap_complete and clap_mangen as direct dependencies.
```

---

## Task 5: Update integration tests

The CLI behavior changes: `--json` becomes `--format json`, `-h`/`--help` both print compact help, and new subcommands (`schema`, `completions`) appear.

**Files:**
- Modify: `crates/colophon/tests/cli.rs`
- Modify: `crates/colophon/tests/config_integration.rs`

- [ ] **Step 1: Update cli.rs test helpers**

The `cmd()` helper stays the same. Update tests that use `--json` to use `--format json`:

```rust
// Before:
cmd().arg("extract").arg("--json").arg("--dir").arg(...)
// After:
cmd().arg("extract").arg("--format").arg("json").arg("--dir").arg(...)
```

- [ ] **Step 2: Update help/version tests**

Both `-h` and `--help` now print compact help (same output). The `schema` and `completions` subcommands appear in help text. Add a test:

```rust
#[test]
fn schema_subcommand_outputs_json() {
    cmd()
        .arg("schema")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"colophon\""));
}
```

- [ ] **Step 3: Update config_integration.rs**

The config loader API is unchanged (thin wrapper), so most tests should pass. If any tests construct a `ConfigLoader` with methods that changed names, update them.

- [ ] **Step 4: Run full test suite**

```bash
cargo nextest run
```

Fix any remaining failures.

- [ ] **Step 5: Commit**

```
test: update integration tests for librebar CLI changes

--json replaced by --format json, schema subcommand added,
help output unified for -h and --help.
```

---

## Task 6: Update command implementations for output format

Each command currently takes `json: bool`. This still works since we compute `json` from `output_format()` in main.rs. But verify each command handles the banner suppression correctly — librebar's CommonArgs may handle this differently.

**Files:**
- Modify: `crates/colophon/src/commands/mod.rs` (if banner logic needs updating)
- Modify: `crates/colophon/src/commands/doctor.rs`
- Modify: `crates/colophon/src/commands/info.rs`

- [ ] **Step 1: Update doctor to use librebar config dirs**

In `commands/doctor.rs`, replace `config::user_config_dir()` etc. with the versions from colophon-core (which now delegate to librebar). These should be the same function names — verify they still work.

- [ ] **Step 2: Verify banner suppression**

The banner in `commands/mod.rs` checks `stderr().is_terminal()`. With `--format auto`, piped output auto-detects JSON — but the banner is on stderr, so it should still show. Verify this is correct behavior.

- [ ] **Step 3: Run the full check suite**

```bash
just check
```

All 269+ tests should pass, clippy clean, deny clean.

- [ ] **Step 4: Commit**

```
refactor(commands): adapt to librebar output format
```

---

## Task 7: Clean up removed dependencies from workspace

Final cleanup — verify the lockfile is clean and no stale deps remain.

**Files:**
- Modify: `Cargo.lock` (via `cargo update`)

- [ ] **Step 1: Run cargo update**

```bash
cargo update
```

Verify figment, directories, tracing-subscriber, tracing-appender, clap_complete, and clap_mangen are gone from the lockfile.

- [ ] **Step 2: Run full verification**

```bash
just check
just outdated
```

- [ ] **Step 3: Verify new features work**

```bash
# CLI Spec schema
cargo run -- schema

# Crash dump (trigger with RUST_BACKTRACE=1 on a bad input)
# Update check (runs on info/doctor)
cargo run -- info
cargo run -- doctor
```

- [ ] **Step 4: Commit**

```
chore(deps): remove stale dependencies after librebar adoption

Removes figment, directories, tracing-subscriber, tracing-appender,
clap_complete, clap_mangen. librebar 0.6 provides all of these.
```

---

## Gotchas Checklist (from bito's migration)

Before starting, read `~/source/claylo/bito/record/superpowers/plans/2026-08-03-librebar-adoption.md` for the full "what actually happened" notes. Key traps:

- [ ] **Flag collision**: `CommonArgs` owns `-c/--config`. Colophon has `-c` as short for `--config` — should be the same flag, but verify no panic.
- [ ] **Config key names**: librebar reads `log_dir`, `log_level`, `log_retention_days` off the config struct via serde. Colophon's `Config` uses `log_dir` and `log_level` (matching). Add a `log_retention_days` field or verify librebar doesn't require it.
- [ ] **`--format auto` piped behavior**: With `--format auto` (the default), piped stdout gets JSON. This is new behavior. Integration tests that pipe may break.
- [ ] **xtask junk manpages**: Filter out `colophon-help.1` from manpage generation.
- [ ] **Don't remove figment until the last import is gone**: Task 1 replaces the loader. Only then remove figment from Cargo.toml.
