# Handoff: Post-Curate Location Validation

**Date:** 2026-03-21
**Branch:** `fix/validate-phantom-locations`
**State:** Green — 260/260 tests pass, clippy clean.

## Where things stand

The validate module is complete and integrated into the CLI. After `colophon curate` writes `colophon-terms.yaml`, it checks each term+location against source files and suggests missing aliases when the canonical term can't be found. A `commit.txt` is ready for squash-merge. The `chore/update-template` branch work (conflict resolution, Homebrew tap rename) was committed separately and merged earlier — this branch builds on top of it.

## Decisions made

- **Phantom locations are 96% a curate problem**, not extract — Claude renames terms during curation ("Slack" -> "Slack integration") but doesn't always add the original form as an alias. Only 6 of 132 phantoms were singular/plural mismatches.
- **Shared `typst_prose` module** — extracted `collect_prose_ranges` and `find_term_offset_in_prose` from `render/typst.rs` into `crate::typst_prose` (pub(crate)) so both render and validate can use AST-aware prose search.
- **Suggestion heuristic order**: suffixes first (drop from front: "Amazon Bedrock" -> "Bedrock"), then prefixes (drop from end), then singular/plural toggle. This order favors the dominant real-world pattern where generic prefixes get prepended during curation.
- **Validation is advisory, not blocking** — suggestions print to stderr after the curate summary table. Skipped in `--json` and `--dry-run` modes.
- **Fixed pre-existing test** — `extract_produces_yaml_output` was asserting on stdout but extract writes its table to stderr. Changed to `.stderr()` assertions.

## What's next

1. **Squash-merge this branch** — `commit.txt` is ready. 6 commits to squash.
2. **Test incremental curate against real corpus** — still untested end-to-end with actual Claude invocation on the 67-file Typst corpus at `~/source/claude/build`. Run `colophon extract && colophon curate` then again with a small edit.
3. **Consider tightening curate prompt** — tell Claude to always include the original extracted term as an alias when renaming. This reduces phantoms at the source rather than patching after.
4. **Fix phantom locations in extract** (lower priority) — the 6 singular/plural mismatches could be caught during extract by verifying candidates against original source. The `typst_prose` module is now available for this.

## Landmines

- **`toggle_plural` is intentionally naive** — `strip_suffix('s')` turns "status" into "statu" and "access" into "acces". These are harmless (they won't match anything) but look wrong if you read the code without context.
- **Suffix-before-prefix heuristic** can suggest common words for 2-word terms — e.g., "OAuth providers" would suggest "providers" instead of "OAuth". Works well for the dominant pattern (generic prefix + specific term) but poorly for the inverse.
- **`validate_locations` reads source files** — it needs the actual source directory to be accessible. If called with a stale `source_dir` in the terms file, it silently counts everything as unresolved.
- **`_candidates_yaml` unused param** in `run_incremental` (`curate/mod.rs`) — pre-existing, not from this branch. The `_` prefix suppresses the warning.
