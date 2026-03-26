---
audit: 2026-03-21-current-repo-review
last_updated: 2026-03-25
status:
  fixed: 2
  mitigated: 1
  accepted: 0
  disputed: 0
  deferred: 0
  open: 0
---

# Actions Taken: Current Repo Review

Summary of remediation status for the [2026-03-21 current repo review](index.md).

---

## 2026-03-25 — Validation always reports unresolved locations

**Disposition:** fixed
**Addresses:** [curate-validation-suppresses-unresolved-without-suggestion](index.md#curate-validation-suppresses-unresolved-without-suggestion)
**Commit:** pending (branch `fix/audit-remediation`)
**Author:** @claylo

`display_validation()` now triggers on `report.unresolved > 0` instead of `!report.suggestions.is_empty()`. Added `UnresolvedDetail` struct and `unresolved_no_suggestion` vec to `ValidationReport` so the CLI can report missing files and no-match locations even when the heuristic has nothing to suggest.

```rust crates/colophon/src/commands/curate.rs:60
if report.unresolved > 0 {
    // Always show summary + details, not just when suggestions exist
}
```

---

## 2026-03-25 — Main-file detection uses path component matching

**Disposition:** fixed
**Addresses:** [main-file-detection-uses-substring-match](index.md#main-file-detection-uses-substring-match)
**Commit:** pending (branch `fix/audit-remediation`)
**Author:** @claylo

Switched from `loc.file.contains(mf.as_str())` to `Path::new(&loc.file).ends_with(mf.as_str())`. `Path::ends_with` compares path components, so `old-auth.md` no longer matches `auth.md` while `chapters/auth.md` still does. Regression test added with overlapping filenames (`auth.md`, `old-auth.md`, `appendix/auth.md.bak`).

```rust crates/colophon-core/src/curate/mod.rs:351
.any(|mf| std::path::Path::new(&loc.file).ends_with(mf.as_str()))
```

---

## 2026-03-25 — Char-boundary guards for Unicode casefold offset drift

**Disposition:** mitigated
**Addresses:** [unicode-casefold-offsets-drift-from-source](index.md#unicode-casefold-offsets-drift-from-source)
**Commit:** pending (branch `fix/audit-remediation`)
**Author:** @claylo

Added `is_char_boundary()` checks at all three sites where lowercased-buffer offsets are used against original source text:

- `find_term_offset` returns `None` when offset isn't a valid char boundary in the original
- `find_term_offset_in_prose` skips matches with invalid offsets and advances past multibyte chars instead of bare `+1`
- `TypstRenderer::annotate` checks char boundary before `insert_str`

This prevents panics but doesn't fix the root cause — terms near expanding Unicode case folds will silently become "not found" instead of being placed correctly. The correct fix is to scan the original buffer while comparing normalized slices, preserving true source spans. That's a medium-effort change for a future pass; the current corpus is English ASCII and unaffected.
