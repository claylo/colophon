---
status: proposed
date: 2026-03-20
decision-makers: [Clay Loveless]
consulted: []
informed: []
---

# 0001: Single-parent hierarchy for index entries, with multi-level reconstruction at render time

## Context and Problem Statement

How should colophon model index entry hierarchy in the curated term database, given that render targets like in-dexter support arbitrary nesting (`#index("networking", "authentication", "OAuth")`)?

The curate phase asks Claude to assign a `parent` field per term. The render phase must translate this into nested index markers. We need to decide whether to expand the curation schema to capture deeper structure now, or keep the current single-parent model and reconstruct depth at render time.

## Decision Drivers

- In-dexter supports arbitrary-depth nested entries via positional args or bang syntax
- Professional back-of-book indexes commonly use 2-3 levels of nesting
- The curation prompt already asks Claude for parent assignments, producing implicit chains (A -> B -> C)
- Schema complexity affects Claude's output quality: more fields = more chances for inconsistency
- Entry inversion ("access control, role-based" alongside "role-based access control") is a common professional indexing pattern we don't yet support

## Considered Options

- **Option 1: Keep single `parent` field, walk chains at render time**
- **Option 2: Add explicit `path` array to schema (e.g., `["networking", "authentication", "OAuth"]`)**
- **Option 3: Add `subentries` nesting to the schema (recursive structure)**

## Decision Outcome

Chosen option: "Option 1: Keep single `parent` field, walk chains at render time", because it keeps the curation schema simple, avoids burdening Claude with redundant structural data, and the multi-level chain is already implicit in the parent pointers.

### Consequences

- Good, because the curation prompt and schema stay minimal, reducing output inconsistency
- Good, because the existing 41 curate tests and real corpus output remain valid
- Good, because render-time chain walking is trivial: follow parent pointers, reverse the list, done
- Bad, because parent chain validation (cycle detection, dangling refs beyond one level) must happen at render time rather than during curation
- Bad, because we can't represent entry inversions ("access control, role-based") without a separate `inversions` field in the schema, which we're deferring

### Confirmation

Render phase implementation must include a test that reconstructs a 3-level chain (grandparent -> parent -> child) into the correct in-dexter `#index("grandparent", "parent", "child")` call.

## Pros and Cons of the Options

### Option 1: Single parent, walk chains at render time

Current model. Each term has `parent: Option<String>`. Post-processing already inverts these into `children` lists.

- Good, because no schema or prompt changes needed
- Good, because Claude only makes one judgment call per term (who's my parent?) instead of reconstructing a full path
- Neutral, because depth is limited by what Claude assigns, not by the schema
- Bad, because cycle detection and orphan chain handling fall to the render phase

### Option 2: Explicit `path` array

Each term carries its full ancestry: `path: ["networking", "authentication", "OAuth"]`.

- Good, because render output is a direct 1:1 mapping to in-dexter's positional args
- Bad, because Claude must maintain consistency across all terms' paths (if "authentication" moves, every child's path must update)
- Bad, because it duplicates information already derivable from parent pointers
- Bad, because the schema change would invalidate existing curated output

### Option 3: Recursive `subentries` nesting

Terms contain nested `subentries` arrays, producing a tree structure.

- Good, because it directly models the index tree
- Bad, because Claude's structured output with recursive schemas is unreliable at depth > 2
- Bad, because it's a fundamentally different data model, requiring rewrite of post-processing
- Bad, because flat-with-parent-pointers is easier to merge, diff, and hand-edit

## More Information

- In-dexter nesting API: `#index("Parent", "Child")` for 2-level, `#index("A", "B", "C")` for 3-level
- In-dexter also supports bang grouping: `#index("A!B!C")` via `use-bang-grouping: true`
- Entry inversion is a separate concern — defer to a future ADR when render phase reveals concrete needs
- Related: [Phase 2 curate plan](../../docs/plans/2026-03-10-phase2-curate.md), [Architecture plan](../../.claude/plans/001-architecture.md)
