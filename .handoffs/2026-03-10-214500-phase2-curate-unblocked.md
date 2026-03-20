# Handoff: Phase 2 Curate — Unblocked and Ported to Rust

**Status:** Green — curate pipeline working end-to-end, extract has progress bars, 41 curate+config tests passing
**Date:** 2026-03-10
**Branch:** `main` (uncommitted changes)

## Where things stand

The curate pipeline is fully operational. The `claude -p` speed problem (7+ min) was caused by the CLI bootstrapping MCP servers, plugins, and slash commands on every invocation. A set of "headless flags" reduces this to ~5 seconds of startup overhead, with actual curation taking ~5 minutes for 500 candidates on opus. The pipeline runs as `cargo run -- curate --full --output-dir out/` and produces `colophon-terms.yaml` + `colophon-curated-thinking.md`.

Both `extract` and `curate` commands now have `indicatif` progress bars and `tabled` summary output.

## Decisions made

- **Headless CLI over raw API** — `claude --print` with runtime-disabling flags, not `ureq` + `ANTHROPIC_API_KEY`. Users authenticate via their existing Claude subscription.
- **Full YAML over compact** — 175K tokens of context produces better definitions, more suggested terms, and fewer turns than 7K compact format. `--full` flag controls this.
- **Streaming JSONL** — `--output-format stream-json` with line-by-line parsing. Last valid JSON object across turns wins (handles `max_tokens` truncation gracefully).
- **Thinking output as audit trail** — `thinking_delta` events captured to `colophon-curated-thinking.md`. Shows editorial rationale for every keep/kill/merge decision.
- **MSRV 1.89.0** — bumped from 1.88.0 for `typst-syntax` 0.14.2.
- **`curate.claude_settings`** — config subsection extracted as JSON, written to temp file for `--settings`. Supports `alwaysThinkingEnabled`, `effortLevel`, `fastMode`.

## What's next

1. **Cost estimation** — blocked on Clay splitting token counting out of `bito-lint` into its own crate. Once available, add `--dry-run` and `--max-budget-usd` passthrough. Track `cache_creation_input_tokens` from `message_start` events.
2. **`.typ` file extraction** — `typst-syntax` 0.14.2 is added but no extraction code yet. Wire into `extract/mod.rs` alongside the pulldown-cmark markdown path.
3. **Prompt tuning** — current system prompt produces 75-100 terms from 500 candidates. Experiment with removing the "err on inclusion" directive for smaller corpora, or splitting large candidate sets into batches.
4. **`human_size()` dedup** — duplicated between `commands/extract.rs` and `commands/curate.rs`. Extract to a shared `commands/fmt.rs` when a third use appears.

## Landmines

- **`--plugin-dir` must be a valid skeleton**, not an empty string. Empty string falls back to the default plugin dir and loads all plugins. The skeleton needs `installed_plugins.json` (`{"version":2,"plugins":{}}`), `known_marketplaces.json` (`{}`), and empty `cache/` + `marketplaces/` dirs. The Rust code creates this in a temp dir via `tempfile::TempDir`.
- **`--verbose` is required** when combining `--print` with `--output-format stream-json`. Without it, the CLI errors immediately.
- **`CLAUDECODE=""` not `unset`** — set to empty string, don't remove. The Rust code uses `cmd.env("CLAUDECODE", "")`.
- **`CLAUDE_CODE_MAX_OUTPUT_TOKENS=64000`** — without this, the model hits `max_tokens` at 32K on turn 1 (all thinking, no output), wasting a full turn.
- **Multi-turn JSON concatenation** — `input_json_delta` events concatenate across turns in raw stream output. The Rust parser handles this by resetting the JSON buffer at each `message_start` and taking the last valid parse. The bash script at `scripts/curate-request` does NOT handle this — its `jq` filter blindly concatenates, producing invalid JSON if the run uses multiple turns.
- **`CurateConfig` no longer derives `Eq`** — changed to `PartialEq` only because `serde_json::Value` (used for `claude_settings`) doesn't implement `Eq`. `Config` only requires `PartialEq` so this is safe.
- **The bash prototype at `scripts/curate-request`** is useful for debugging but diverges from the Rust implementation. Output filenames are hardcoded to `-04` suffixes from the last test run.
