# Handoff: Glossary Polish + Incremental Curate Ready

**Date:** 2026-03-20
**Branch:** `main` (all work committed and merged)
**State:** Green — render pipeline complete, 41 render tests passing, PDF compiles cleanly with glossary and index.

## Where things stand

The full colophon pipeline (extract, curate, render) is working end-to-end against a real 67-file Typst corpus. This session polished the glossary output: code-mode `#terms()` for spacing control, `<term-xxx>` label anchors with linked see-also cross-references, and expanded Typst character escaping (`_`, `*`). All committed to `main` as PRs #4 and #5.

## Decisions made

- **Code-mode glossary over markup mode** — `#terms()` with `terms.item()` exposes `tight` and `spacing` parameters that markup `/ term: def` doesn't. Top-level defaults to `tight: false`; nested children stay `tight: true`.
- **Label anchors for glossary terms** — each term gets `<term-kebab-name>`, see-also entries become `#link(<term-xxx>)[Name]`. Enables in-PDF navigation.
- **Extended `escape_typst()`** — now escapes `@`, `#`, `<`, `$`, `_`, `*` in definition text. Prevents `__` from becoming emphasis, `*` from becoming bold.

## What's next

1. **Implement incremental curate** — spec is approved at `record/superpowers/specs/2026-03-20-incremental-curate-design.md`. Start by writing an implementation plan from the spec, then execute. Key modules to add/modify: `curate/mod.rs` (diffing, delta prompt, merge), `curate/claude.rs` (incremental system prompt, delta schema), `commands/curate.rs` (`--full-rebuild` flag). Estimated ~200 lines new code with heavy reuse of existing post-processing.
2. **Fix phantom locations in extract** — extract reports locations from concatenated prose text; render (AST-aware) correctly skips non-prose positions. 28 not-found terms result from this mismatch. Use `collect_prose_ranges` + `find_term_offset_in_prose` (already in `render/typst.rs`) during extract for `.typ` files.

## Landmines

- **`collect_prose_ranges` lives in `render/typst.rs`** — when fixing phantom locations (item 2), this function needs to be accessible from the extract module. Move it to a shared location or duplicate it.
- **`escape_typst()` runs only on definitions, not term names** — term names are escaped separately in `emit_term`. If a term name contains `_` or `*`, the label generation via `term_label()` handles it (strips non-alphanumeric), but the display name in `[term <label>]` uses `escape_typst()`.
- **Glossary `tight: false` is hardcoded** — no CLI flag to toggle it. If someone wants tight glossary, they'd need to pass `--glossary-spacing 0pt` as a workaround. Could add `--glossary-tight` later if needed.
- **Full test suite not run this session** — only render tests (41/41). Run `just check` before starting new work.
