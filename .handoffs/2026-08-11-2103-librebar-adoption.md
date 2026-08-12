# Handoff: librebar 0.6 Adoption

**Date:** 2026-08-11
**Branch:** `chore/librebar-migration`
**State:** Green — 261/261 tests pass, clippy clean, cargo-deny clean.

## Where things stand

Colophon now uses librebar 0.6 as its application foundation. Five commits replace hand-rolled config loading, CLI flag wiring, and structured logging with librebar's equivalents — and gain crash dumps, update notifications, CLI Spec schema, and built-in completions for free. No public release has shipped, so this is not a breaking change for users.

## What changed

| Before | After |
|--------|-------|
| figment for config discovery + merge | `librebar::config::ConfigLoader` (thin wrapper preserves API) |
| 508-line `observability.rs` with custom JSON log layer | `librebar::logging` via builder `.logging()` call |
| Hand-rolled `--quiet`, `--verbose`, `--color`, `--json`, `--chdir`, `--config`, `--version-only` | `librebar::cli::CommonArgs` flatten |
| `--json` boolean toggle | `--format auto\|text\|json` with pipe detection |
| Manual `command()` + `HelpShort` wiring | `librebar::cli::parse_with` (compact help by default) |
| `clap_mangen` in xtask | `librebar::cli::render_manpage` |
| No crash handling | `librebar::crash` — JSON panic dumps in XDG cache |
| No update notifications | `librebar::update` available via `info`/`doctor` |
| No agent discovery | `colophon schema` outputs CLI Spec 0.2 JSON |

### Deps removed (direct)
figment, directories, tracing-subscriber, tracing-appender, clap_mangen

### Deps added (direct)
librebar 0.6 (in colophon-core: `config` feature only; in colophon CLI: `cli`, `config`, `logging`, `crash`, `diagnostics`, `update`; in xtask: `cli`)

### Also in this branch
- All workspace deps bumped to latest (`cargo update` + version spec bumps for tabled 0.21, typst-syntax 0.15, which 8)
- MSRV bumped 1.89.0 → 1.92.0 (required by typst-syntax 0.15)
- Rust toolchain bumped 1.94.1 → 1.97.1
- Clippy fixes for 1.97 (`sort_by` → `sort_by_key`, collapsible if-in-match)
- `cargo-deny` `--config` flag position fixed for 0.20
- RUSTSEC-2026-0173 cleared (tabled 0.21 without derive drops `proc-macro-error2`)

## Decisions made

- **colophon-core gets `config` only** — no process-global state in the library crate. CLI, logging, crash, diagnostics, update are binary concerns. Matches bito's pattern.
- **Keep colophon's own `ConfigSources` struct** — populated from librebar's but preserves the existing public API shape. Avoids rippling `#[non_exhaustive]` changes through every consumer.
- **Keep colophon's own `LogLevel` enum** — librebar exports one with `Trace`, but switching would touch too many files for no gain right now.
- **`ConfigError::Deserialize` uses `Box<dyn Error>`** — not `Box<figment::Error>` or `Box<librebar::Error>`. No dependency leaks in the public error type.
- **`--format auto` pipe behavior accepted** — piped stdout now gets JSON by default. Integration tests updated to use `--format text` where they need stderr summary tables.
- **`clap_complete` stays as direct xtask dep** — librebar doesn't re-export the completion generator types. This matches bito's pattern.

## What's next

1. **Wire up update notifications** — `librebar::update::UpdateChecker` on `info` and `doctor` commands, like bito does. Requires a GitHub release to check against.
2. **Wire up diagnostics** — `librebar::diagnostics::DoctorRunner` + `DebugBundle` to replace the hand-rolled doctor command with librebar's framework. Currently doctor still uses its own report struct.
3. **Env var config overrides** — librebar's config loader now supports `COLOPHON_*` env vars with `__` nesting automatically. This is new behavior — not documented or tested yet.
4. **Consider switching to librebar's `LogLevel`** — gains `Trace` variant. Minor, low priority.
5. **Cut first public release** — this branch plus any remaining polish.

## Landmines

- **`--format auto` changes piped output.** Scripts that pipe colophon output and expect text will now get JSON. Since there's been no public release, this isn't a user-facing break, but internal scripts or CI jobs may need `--format text`.
- **`colophon schema` and `colophon completions` are new subcommands.** They appear in help text and manpages. Tests cover `schema`; `completions` is tested via xtask.
- **`log_retention_days`** — librebar reads this field off the config via serde if present. Colophon's `Config` struct doesn't have this field. This means log retention uses librebar's default (whatever that is). Add the field if you want control over it.
- **MSRV is now 1.92.0** — CI and README should reflect this if they reference the MSRV.
