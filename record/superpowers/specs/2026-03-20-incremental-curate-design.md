# Incremental Curate Pipeline

**Status:** Approved design, not yet implemented
**Date:** 2026-03-20

## Problem

The curate pipeline sends the entire candidate corpus (~150K-180K tokens) to Claude every run, costing ~$8 uncached / ~$5 cached with Opus. During active writing — where a few files change between runs — this is wasteful. Most terms are already curated; only new candidates need Claude's attention.

## Design Summary

Make `colophon curate` incremental by default when a `colophon-terms.yaml` already exists. Always re-extract (cheap, local), diff fresh candidates against existing curated terms, send only new candidates to Claude with a compact summary of the existing index for context, then merge the delta response back.

**Cost target:** ~$0.50 for a typical small edit (5-10 new candidates) vs $8 full rebuild. Zero-dollar mechanical-only runs when no new terms surface.

## Detection & Mode Selection

When `colophon curate` runs:

1. Check if `colophon-terms.yaml` exists at expected path
2. **YES** + no `--full-rebuild` flag &rarr; **incremental mode**
3. **NO** or `--full-rebuild` flag &rarr; **full mode** (current behavior, unchanged)

The terms file is the sole state artifact. No manifest, no hash database, no additional state files.

## Diffing

Two independent operations, one semantic (requires Claude), one mechanical (free):

### Mechanical: Location Refresh

Re-map locations for ALL existing terms using fresh candidates. This is the same alias-lookup and location-mapping that curate post-processing already performs (`map_locations` in `curate/mod.rs`). New file appearances get `main: false` by default. Disappeared locations are dropped.

### Semantic: New Term Detection

Build a lookup set from existing curated terms:

```
known_keys = { term.to_lowercase(), alias.to_lowercase(), ... }
             for each curated term
```

A fresh candidate is **new** if `!known_keys.contains(candidate.term.to_lowercase())`. Everything matched by an existing term or alias is already covered.

### Stale Detection

A curated term is **potentially stale** if no fresh candidate matches its name or any alias AND it has at least one location. Terms with zero locations (i.e., previously `suggested` terms from Claude) are excluded from stale detection — they never had candidate backing and would show up as stale noise on every incremental run.

### What We Don't Diff On

- Score changes (YAKE/TF-IDF shifted) -- irrelevant, already curated
- Context snippet changes (surrounding prose changed) -- no curation impact
- Location count changes (more/fewer files) -- handled mechanically

The diff is a simple set operation, not a deep comparison.

## Delta Prompt

In incremental mode, Claude gets a different system prompt and smaller payload.

### System Prompt (Incremental)

```
You are a professional book indexer updating an existing back-of-book index
with new candidate terms.

EXISTING INDEX (for context -- do NOT regenerate these):
[compact listing: term | parent | aliases | see_also]

NEW CANDIDATES to integrate:
[full detail: term, score, locations with context snippets]

POTENTIALLY STALE terms (no longer found in corpus):
[term names only]

Instructions:
1. For each new candidate: add as new term, merge as alias of existing, or discard
2. For stale terms: recommend keep (suggested/conceptual) or remove
3. If a new term should be a child of an existing term, set parent accordingly
4. If new terms create new see_also relationships, include the modification
5. Return ONLY additions, modifications, and removals -- not unchanged terms
```

### Output Schema (Incremental)

A separate JSON Schema file (`config/curate-delta-schema.yaml`, converted to JSON at runtime) is required for incremental mode. The full-mode schema (`config/curate-schema.yaml`) enforces `{ terms, suggested }` which would reject the delta response format. The curate pipeline selects the appropriate schema based on mode.

```yaml
type: object
additionalProperties: false
properties:
  additions:                    # required (may be empty array)
    type: array
    items:
      type: object
      required: [term, definition]
      properties:
        term: { type: string }
        definition: { type: string }
        parent: { type: string }          # optional
        aliases: { type: array, items: { type: string } }    # optional
        see_also: { type: array, items: { type: string } }   # optional
        main_files: { type: array, items: { type: string } } # optional
  modifications:                # required (may be empty array)
    type: array
    items:
      type: object
      required: [term, reason]  # reason is REQUIRED -- justifies the change
      properties:
        term: { type: string }
        definition: { type: string }      # optional -- only if changed
        parent: { type: string }          # optional -- only if reparented
        aliases: { type: array, items: { type: string } }    # optional
        see_also: { type: array, items: { type: string } }   # optional
        reason: { type: string }
  removals:                     # required (may be empty array)
    type: array
    items:
      type: object
      required: [term, reason]
      properties:
        term: { type: string }
        reason: { type: string }
  suggested:                    # required (may be empty array)
    type: array
    items:
      type: object
      required: [term, definition]
      properties:
        term: { type: string }
        definition: { type: string }
        parent: { type: string }          # optional
        # No aliases, see_also, or main_files -- suggested terms have no
        # candidate backing, same as full-mode suggested schema.
required: [additions, modifications, removals, suggested]
```

All four top-level arrays are required but may be empty. This matches the full-mode schema's convention of requiring `terms` and `suggested`. The implementation task includes authoring this as `config/curate-delta-schema.yaml` with the same rigor as `config/curate-schema.yaml`.

### Compact Index Format

The existing index is serialized as a pipe-delimited listing for token efficiency:

```
OAuth | parent: authentication | aliases: OAuth 2.0, OAuth2 | see_also: API key
API key | aliases: API keys | see_also: OAuth, authentication
authentication | children: OAuth, SSO | see_also: authorization
TLS | (top-level, no relationships)
```

Fields omitted when empty. No definitions, no locations. One line per term.

### Token Budget

Compact existing index: ~200 terms at ~50 tokens each = ~10K tokens.
New candidates (typical small edit): ~50 candidates at ~100 tokens each = ~5K tokens.
System prompt + schema: ~2K tokens.
Total: ~17K input tokens vs ~160K for full rebuild.

No definitions in the compact index -- saves tokens and Claude can write definitions from candidate context without overlap risk.

Modifications allow definition changes only when `reason` is provided, allowing Claude to adjust related definitions when new terms shift meaning (e.g., adding "PKCE" may refine the "OAuth" definition).

## Merge

After Claude returns `{ additions, modifications, removals, suggested }`:

1. **Remove:** Drop curated terms listed in `removals`. Log term and reason.
2. **Modify:** For each `modifications` entry, find matching curated term by name. Sparse update -- only patch fields Claude returned. Missing fields are preserved.
3. **Add:** Append `additions` and `suggested` as new `CuratedTerm` entries.
4. **Validate referential integrity:** Check all `parent` pointers resolve to existing terms. Dangling parents (e.g., a modification reparents to a term that was simultaneously removed) are nullified -- set `parent` to `None` and log a warning. Same warn-and-nullify behavior as current `post_process`.
5. **Re-map locations:** Run existing `map_locations` post-processing against fresh candidates for ALL terms (existing + new). Mechanical refresh. `main_files` for existing terms are handled here, not by Claude in modifications -- Claude only provides `main_files` for additions.
6. **Re-invert hierarchy:** Rebuild `children` arrays from `parent` pointers. Same code path as current post-processing.
7. **Sort & write:** Alphabetical sort, write `colophon-terms.yaml`.

Steps 5-7 are identical to current full-curate post-processing. The merge logic (steps 1-4) is ~60 lines of new code.

### Thinking Trace

In incremental mode, `colophon-curated-thinking.md` is appended rather than overwritten:

```
\n---\n## Incremental update 2026-03-20T14:30:00Z\n\n[thinking content]
```

In full mode, the file is overwritten (current behavior). This provides an audit trail of incremental updates while keeping full rebuilds clean.

## Fallback & Thresholds

Delta ratio: `new_candidates.len() / total_candidates.len()`

| Ratio | Behavior |
|-------|----------|
| 0% | Skip Claude entirely. Re-map locations mechanically. Print "No new terms found. Locations refreshed." |
| 1-40% | Normal incremental mode |
| 40-70% | Proceed incremental + print warning: consider `--full-rebuild` for better cross-term relationships |
| 70%+ | Proceed incremental + print stronger recommendation for full rebuild |
| `--full-rebuild` | Full mode, always |

No hard gate at any threshold. Warnings are informational. The 0% case (pure location refresh) is the sweet spot during revision-heavy phases -- zero dollars, sub-second. The 0% path skips Claude CLI discovery entirely (`find_claude()` is not called), so it works even if `claude` is not in PATH.

Cost estimation (`--dry-run`) uses delta token count. Dry-run output shows both: "Incremental: ~$0.45 | Full rebuild would be: ~$8.20"

## CLI Interface Changes

```
colophon curate [existing flags...] [--full-rebuild]
```

- `--full-rebuild` -- Force full curate even when terms file exists

Warning thresholds (40% / 70%) are fixed. No flag to override them -- a configurable threshold that only shifts a warning message isn't worth the CLI surface area.

### Output (Incremental)

```
Mode:           incremental (194 unchanged, 3 added, 1 modified, 2 removed)
New candidates: 47 of 512 (9%)
Model:          opus
Cost:           $0.48
Locations:      refreshed across 61 files
```

### Output (Mechanical Only, 0% New)

```
Mode:           incremental (no new terms)
Locations:      refreshed across 61 files
Cost:           $0.00
```

## What Doesn't Change

- `colophon extract` -- always runs full corpus, no changes
- `colophon render` -- consumes `colophon-terms.yaml` as before, no changes
- Full curate mode -- identical to current behavior when terms file absent or `--full-rebuild` used
- JSON schema for full mode -- unchanged
- Config file format -- unchanged (new fields are additive)
