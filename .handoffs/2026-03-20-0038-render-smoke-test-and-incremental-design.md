# Handoff: Render Smoke Test + Incremental Curate Design

**Date:** 2026-03-20
**Branch:** `feat/render` (uncommitted changes span typst extraction, cost estimation, render, and this session's fixes)
**State:** Yellow — render pipeline works end-to-end against real corpus, PDF compiles with glossary and index, but extract reports phantom locations for terms only found in non-prose positions.

## Where things stand

Colophon's full pipeline (extract → curate → render) was smoke-tested against a 67-file Typst documentation corpus. The PDF compiled successfully: 670 markers across 67 files, glossary with hierarchical terms and see-also references, and an in-dexter-generated index. Four render bugs were found and fixed during testing. Incremental curate design (Approach B) was brainstormed, approved, and spec'd.

## Decisions made

- **Default extensions now include "typ"** — `config.rs:58`, matching supported file types
- **Render falls back to terms file's `source_dir`** — `commands/render.rs:57-63`, chain: `--dir` > `terms.source_dir` > `config.source.dir`
- **AST-aware term search for Typst render** — `render/typst.rs:collect_prose_ranges()` + `find_term_offset_in_prose()` prevent markers inside labels, links, code, headings
- **Field access guard** — skip matches where next char is `.`+alpha to prevent Typst syntax errors like `#index[term].field`
- **Glossary escapes Typst-significant chars** — `escape_typst()` handles `@`, `#`, `<`, `$` in definitions
- **`--main-only` flag** — only inserts markers at `main: true` locations, dramatically thinning the index
- **Incremental curate: Approach B** — compact existing index + delta candidates to Claude. Spec: `record/superpowers/specs/2026-03-20-incremental-curate-design.md`

## What's next

1. **Fix phantom locations in extract** — extract reports locations from concatenated prose text, but render (now AST-aware) correctly skips non-prose positions. Fix by verifying locations against original source byte ranges during extract for `.typ` files. Use existing `collect_prose_ranges` + `find_term_offset_in_prose` from `render/typst.rs`.
2. **Implement incremental curate** — spec is written and approved. Needs: delta schema file (`config/curate-delta-schema.yaml`), diffing logic, delta system prompt, merge logic, `--full-rebuild` flag. Core code estimate: ~200 lines new, heavy reuse of existing post-processing.
3. **Run `just check`** — session focused on real-world testing; full test suite not run. 39 render tests pass individually. Verify nothing broke in extract/curate tests from config changes.

## Landmines

- **Uncommitted work spans 4 features.** The `feat/render` branch has uncommitted changes from cost estimation, typst extraction, render, AND this session's fixes. Clay manages his own commits.
- **`collect_prose_ranges` is in `render/typst.rs` but extract needs it too.** When fixing phantom locations (item 1 above), consider moving it to a shared location or to `extract/typst.rs`.
- **Test count not verified this session.** Individual render tests pass (39/39) but full suite was not run. The config default change (`extensions: ["md"]` → `["md", "typ"]`) touches test assertions — one was updated (`config.rs:773`), verify no others.
- **`record/` not `docs/`** — hooks enforce ADRs and specs under `record/`. The for-the-record hook will block writes to `docs/decisions/` or `docs/superpowers/`.
