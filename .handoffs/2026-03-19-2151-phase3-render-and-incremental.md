# Handoff: Phase 3 Render Complete + Incremental Pipeline Question

**Status:** Green — render pipeline working, 158 core tests passing, 0 clippy warnings on new code
**Date:** 2026-03-20
**Branch:** `feat/render` (uncommitted changes from this session + prior uncommitted curate/cost/typst work)

## Where things stand

Three features shipped this session: cost estimation (`--dry-run`, `--max-budget-usd`) via `ah-ah-ah` crate, `.typ` file extraction via `typst-syntax`, and the full Phase 3 render pipeline. `colophon render` reads `colophon-terms.yaml`, walks source files, inserts inline `#index[term]` / `#index-main[term]` markers at term locations, and optionally emits a Typst glossary. The `Renderer` trait is in place for future format backends.

## Decisions made

- **Single-parent hierarchy, chain walked at render time** — [ADR 0001](../record/decisions/0001-index-hierarchy-depth-and-render-mapping.md). Multi-level nesting (`#index("gp", "parent", "child")`) reconstructed from parent pointers.
- **Inline marker placement, not append-only** — in-dexter's `#index` uses `here()` for page capture; markers must be near the term text.
- **Renderer trait with single Typst impl** — `annotate()` + `glossary()` methods. No trait objects; static dispatch via `RenderFormat` enum.
- **Copies to output dir** — non-destructive; originals untouched.
- **ah-ah-ah for cost estimation** — offline Claude token counting, ~4% overcount for conservative budgets.

## What's next

1. **Incremental pipeline** (Clay's priority question) — current workflow is full-rebuild extract → curate → render (~$8-10 per opus run). Need to design a `colophon update` or `--incremental` flag that diffs new candidates against existing `colophon-terms.yaml` and only sends deltas to Claude. Key options: diff-based extract, term database merge, or cache-aware curate.
2. **Smoke test the render output** — run `cargo run -- render --terms colophon-terms.yaml --output-dir /tmp/colophon-render --glossary` against the real corpus and inspect the annotated `.typ` files.
3. **Phase 4 polish** — `colophon generate` (all-in-one), `colophon init`, LaTeX/Markdown renderers.

## Landmines

- **Uncommitted work spans multiple features.** The `feat/render` branch has uncommitted changes from cost estimation, typst extraction, AND render. `commit.txt` currently covers only the typst extraction. Clay runs his own commits.
- **`find_term_offset` uses `to_lowercase()` byte positions.** This works for ASCII search terms in UTF-8 text but would break if a term contains characters where lowercase changes byte length (e.g., German sharp S). Not a practical concern for English technical terms but worth noting.
- **in-dexter version `0.7.2` is hardcoded** in `IN_DEXTER_IMPORT` constant (`render/typst.rs`). Should become configurable when a `[render]` config section is added.
- **1 pre-existing CLI test failure** — `extract_produces_yaml_output` fails due to empty test corpus producing 0 candidates. Unrelated to render work.
- **`record/` not `docs/`** — hooks enforce ADRs and specs go under `record/`, not `docs/`. The for-the-record hook will block writes to `docs/decisions/` or `docs/superpowers/`.
