# Handoff: Incremental Curate Pipeline + README Rewrite

**Date:** 2026-03-20
**Branch:** `main` (squash-merged as PR #6, commit `40ea5ff`)
**State:** Green — 250/251 tests pass (1 pre-existing failure), clippy clean, deny clean.

## Where things stand

The full colophon pipeline (extract → curate → render) now supports incremental curate. When `colophon-terms.yaml` exists, `colophon curate` auto-detects incremental mode: re-extracts locally, diffs against existing terms, sends only new candidates to Claude, merges the delta back. Zero-cost mechanical refresh when no new terms surface. `--full-rebuild` forces full mode. README rewritten with real pipeline documentation, tagline updated across all distribution files.

## Decisions made

- **Incremental is default** when terms file exists (per spec). `--full-rebuild` is the escape hatch.
- **`full_candidates` defaults to `true`** — more context produces better results for fewer tokens overall. The `--full` CLI flag is now a no-op but kept for explicitness.
- **`RenderConfig` struct** replaces 8 positional params to fix clippy `too_many_arguments` nursery lint.
- **`parse_delta_stream` duplicates `parse_stream`** rather than genericizing — two modes don't justify the abstraction.
- **Thinking file appended in incremental mode** (not overwritten) — provides audit trail of incremental updates.

## What's next

1. **Test incremental curate against real corpus** — the pipeline is untested end-to-end with actual Claude invocation. Run `colophon extract && colophon curate` on the 67-file Typst corpus, then run again with a small edit to exercise incremental mode.
2. **Fix phantom locations in extract** — extract reports locations from concatenated prose; render uses AST-aware positions. 28 not-found terms result. Share `collect_prose_ranges` from `render/typst.rs` with the extract module. See previous handoff for details.
3. **Fix `extract_produces_yaml_output` test** — pre-existing CLI test failure. The test expects candidates in stdout but extract writes to file and prints a table to stderr.

## Landmines

- **`_candidates_yaml` unused param** in `run_incremental` (`curate/mod.rs`) — the incremental path re-serializes only the new candidate subset. Parameter kept for API symmetry with `run()`. The `_` prefix suppresses the warning.
- **`serde_yaml` added to CLI crate** (`crates/colophon/Cargo.toml`) — needed for serializing the new candidates subset in `cmd_curate_incremental`. It was already a transitive dependency via colophon-core.
- **Config examples may have stale `full_candidates` comment** — the YAML example was fixed but the TOML example's comment may have been reverted during the squash. Verify with `grep full_candidates config/*.example`.
