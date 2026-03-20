# Incremental Curate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `colophon curate` incremental by default when `colophon-terms.yaml` exists — diffs fresh candidates against existing curated terms, sends only new terms to Claude, merges the delta. Cost: ~$0.50 for small edits, $0 for pure location refreshes. `--full-rebuild` forces full mode.

**Architecture:** Auto-detect existing terms file → incremental mode. `--full-rebuild` → full mode (current behavior). Pipeline: re-extract (cheap) → diff candidates vs. existing terms → if 0% new: mechanical location refresh only → if >0% new: build compact index + delta prompt → invoke Claude with delta schema → merge response (remove → modify → add) → re-map locations → rebuild hierarchy → write. Same `claude::invoke()` is reused — only the prompt, payload, and schema change.

**Tech Stack:** Existing Rust crates (`serde`, `serde_json`, `serde_yaml`, `indicatif`). New compile-time embedded schema (`config/curate-delta-schema.json`). `ah-ah-ah` for token counting. No new dependencies.

**Spec:** `record/superpowers/specs/2026-03-20-incremental-curate-design.md`

**Also in this changeset:** `full_candidates` defaults to `true` (more context = better results). The `--full` CLI flag becomes a no-op but is kept for explicitness.

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `config/curate-delta-schema.yaml` | Create | Delta response JSON Schema (YAML source) |
| `config/curate-delta-schema.json` | Create | Delta schema compiled to JSON (embedded at build) |
| `crates/colophon-core/src/curate/terms.rs` | Modify | Add `ClaudeDeltaOutput` and sub-types for deserialization |
| `crates/colophon-core/src/curate/incremental.rs` | Create | Diffing, compact index formatting, merge logic |
| `crates/colophon-core/src/curate/claude.rs` | Modify | Add incremental system prompt, delta schema embed, `system_prompt_for_incremental()`, `stdin_payload_for_incremental()` |
| `crates/colophon-core/src/curate/mod.rs` | Modify | Wire `estimate_cost_incremental()` and `run_incremental()` public API |
| `crates/colophon-core/src/error.rs` | Modify | Add `CurateError::NoExistingTerms` variant |
| `crates/colophon/src/commands/curate.rs` | Modify | Add `--full-rebuild` flag, auto-detect incremental mode, delta-specific output table |

---

## Task 1: Delta Schema

**Files:**
- Create: `config/curate-delta-schema.yaml`
- Create: `config/curate-delta-schema.json`

- [ ] **Step 1: Write the delta schema YAML**

```yaml
# JSON Schema for colophon curate incremental (delta) output.
#
# This schema is used when --incremental is passed. Claude returns only
# changes to the existing index, not the full term database.
#
# Converted to JSON via: yj < config/curate-delta-schema.yaml > config/curate-delta-schema.json

type: object
required: [additions, modifications, removals, suggested]
additionalProperties: false

properties:
  additions:
    description: >
      New terms to add to the index. Each entry follows the same structure
      as full-mode terms. Aliases MUST be exact strings from the new
      candidate input for location mapping.
    type: array
    items:
      type: object
      required: [term, definition]
      additionalProperties: false
      properties:
        term:
          description: Canonical display form, properly cased.
          type: string
        definition:
          description: One-sentence glossary definition (~20-30 words).
          type: string
        parent:
          description: Parent term for hierarchy (omit if top-level).
          type: string
        aliases:
          description: Other surface forms from the candidate input.
          type: array
          items:
            type: string
        see_also:
          description: Related terms (must exist in output or existing index).
          type: array
          items:
            type: string
        main_files:
          description: Files with substantive discussion of this term.
          type: array
          items:
            type: string

  modifications:
    description: >
      Changes to existing terms. Only include fields that are changing.
      A reason is REQUIRED to justify each modification.
    type: array
    items:
      type: object
      required: [term, reason]
      additionalProperties: false
      properties:
        term:
          description: Exact name of the existing term being modified.
          type: string
        definition:
          description: Updated definition (only if changed).
          type: string
        parent:
          description: New parent (only if reparented).
          type: string
        aliases:
          description: Updated alias list (replaces existing).
          type: array
          items:
            type: string
        see_also:
          description: Updated see_also list (replaces existing).
          type: array
          items:
            type: string
        reason:
          description: Why this modification is needed.
          type: string

  removals:
    description: >
      Terms to remove from the index. Only for terms that are genuinely
      stale — no longer relevant to the corpus.
    type: array
    items:
      type: object
      required: [term, reason]
      additionalProperties: false
      properties:
        term:
          description: Exact name of the term to remove.
          type: string
        reason:
          description: Why this term should be removed.
          type: string

  suggested:
    description: >
      New terms Claude spotted that the extraction missed. Same as
      full-mode suggested — no locations, no aliases.
    type: array
    items:
      type: object
      required: [term, definition]
      additionalProperties: false
      properties:
        term:
          description: Canonical display form.
          type: string
        definition:
          description: One-sentence glossary definition.
          type: string
        parent:
          description: Parent term for hierarchy (omit if top-level).
          type: string
```

- [ ] **Step 2: Convert to JSON**

Run: `yj < config/curate-delta-schema.yaml > config/curate-delta-schema.json`
Verify: `python3 -c "import json; json.load(open('config/curate-delta-schema.json'))"`

- [ ] **Step 3: Commit**

```bash
git add config/curate-delta-schema.yaml config/curate-delta-schema.json
git commit -m "feat(core): add delta JSON Schema for incremental curate"
```

---

## Task 2: Delta Response Types

**Files:**
- Modify: `crates/colophon-core/src/curate/terms.rs`

- [ ] **Step 1: Write failing tests for delta deserialization**

Add to the `#[cfg(test)] mod tests` block in `terms.rs`:

```rust
#[test]
fn delta_output_deserializes() {
    let json = r#"{
        "additions": [
            {
                "term": "PKCE",
                "definition": "Proof Key for Code Exchange.",
                "parent": "OAuth",
                "aliases": ["Proof Key for Code Exchange"],
                "see_also": ["OAuth"],
                "main_files": ["auth.md"]
            }
        ],
        "modifications": [
            {
                "term": "OAuth",
                "definition": "Updated definition.",
                "reason": "PKCE changes the OAuth landscape"
            }
        ],
        "removals": [
            {
                "term": "deprecated_term",
                "reason": "No longer in corpus"
            }
        ],
        "suggested": [
            {
                "term": "bearer token",
                "definition": "A type of access token."
            }
        ]
    }"#;
    let output: ClaudeDeltaOutput = serde_json::from_str(json).unwrap();
    assert_eq!(output.additions.len(), 1);
    assert_eq!(output.additions[0].term, "PKCE");
    assert_eq!(output.additions[0].aliases, vec!["Proof Key for Code Exchange"]);
    assert_eq!(output.modifications.len(), 1);
    assert_eq!(output.modifications[0].term, "OAuth");
    assert!(output.modifications[0].definition.is_some());
    assert_eq!(output.removals.len(), 1);
    assert_eq!(output.removals[0].term, "deprecated_term");
    assert_eq!(output.suggested.len(), 1);
}

#[test]
fn delta_output_empty_arrays() {
    let json = r#"{
        "additions": [],
        "modifications": [],
        "removals": [],
        "suggested": []
    }"#;
    let output: ClaudeDeltaOutput = serde_json::from_str(json).unwrap();
    assert!(output.additions.is_empty());
    assert!(output.modifications.is_empty());
    assert!(output.removals.is_empty());
    assert!(output.suggested.is_empty());
}

#[test]
fn delta_modification_minimal() {
    let json = r#"{
        "additions": [],
        "modifications": [{"term": "OAuth", "reason": "just testing"}],
        "removals": [],
        "suggested": []
    }"#;
    let output: ClaudeDeltaOutput = serde_json::from_str(json).unwrap();
    let m = &output.modifications[0];
    assert!(m.definition.is_none());
    assert!(m.parent.is_none());
    assert!(m.aliases.is_none());
    assert!(m.see_also.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core delta_output`
Expected: FAIL — `ClaudeDeltaOutput` not defined.

- [ ] **Step 3: Add the delta types**

Add these structs to `terms.rs` (above the `#[cfg(test)]` block, after `ClaudeSuggested`):

```rust
/// Delta response from Claude in incremental mode.
///
/// Matches the JSON Schema in `config/curate-delta-schema.yaml`.
/// Contains only changes — existing unchanged terms are not included.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClaudeDeltaOutput {
    /// New terms to add.
    pub additions: Vec<ClaudeTerm>,
    /// Modifications to existing terms (sparse updates).
    pub modifications: Vec<DeltaModification>,
    /// Terms to remove.
    pub removals: Vec<DeltaRemoval>,
    /// Terms Claude spotted that extraction missed.
    #[serde(default)]
    pub suggested: Vec<ClaudeSuggested>,
}

/// A modification to an existing term. Only changed fields are present.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeltaModification {
    /// Exact name of the existing term being modified.
    pub term: String,
    /// Updated definition (only if changed).
    pub definition: Option<String>,
    /// New parent (only if reparented).
    pub parent: Option<String>,
    /// Updated alias list (replaces existing if present).
    pub aliases: Option<Vec<String>>,
    /// Updated see_also list (replaces existing if present).
    pub see_also: Option<Vec<String>>,
    /// Justification for the change.
    pub reason: String,
}

/// A term to remove from the index.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeltaRemoval {
    /// Exact name of the term to remove.
    pub term: String,
    /// Why it should be removed.
    pub reason: String,
}
```

Note: `additions` reuses `ClaudeTerm` (same structure). `suggested` reuses `ClaudeSuggested`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core delta_output`
Expected: PASS (all 3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/colophon-core/src/curate/terms.rs
git commit -m "feat(core): add delta response types for incremental curate"
```

---

## Task 3: Incremental Diffing and Compact Index

This is the core logic module — all pure functions, no I/O, easy to test.

**Files:**
- Create: `crates/colophon-core/src/curate/incremental.rs`
- Modify: `crates/colophon-core/src/curate/mod.rs` (add `pub mod incremental;`)

- [ ] **Step 1: Register the module**

In `crates/colophon-core/src/curate/mod.rs`, add after `pub mod terms;`:

```rust
pub mod incremental;
```

(Change visibility to `pub` so CLI can access diff stats for display.)

- [ ] **Step 2: Write failing tests for `diff_candidates`**

Create `crates/colophon-core/src/curate/incremental.rs` with tests first:

```rust
//! Incremental curate pipeline — diff, format, and merge.

use std::collections::HashSet;

use crate::curate::terms::{CuratedTerm, CuratedTermsFile, TermLocation};
use crate::extract::candidates::{CandidatesFile, Candidate, CandidateLocation};

/// Result of diffing fresh candidates against existing curated terms.
#[derive(Debug)]
pub struct CandidateDiff {
    /// Candidates not matched by any existing term or alias.
    pub new_candidates: Vec<Candidate>,
    /// Existing terms whose name and aliases have zero candidate matches
    /// AND have at least one location (excludes suggested-only terms).
    pub stale_terms: Vec<String>,
    /// Total candidates in the fresh extraction.
    pub total_candidates: usize,
}

impl CandidateDiff {
    /// Ratio of new candidates to total (0.0–1.0).
    pub fn new_ratio(&self) -> f64 {
        if self.total_candidates == 0 {
            return 0.0;
        }
        self.new_candidates.len() as f64 / self.total_candidates as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curated(name: &str, aliases: &[&str], has_locations: bool) -> CuratedTerm {
        CuratedTerm {
            term: name.to_string(),
            definition: format!("{name} def."),
            parent: None,
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: if has_locations {
                vec![TermLocation {
                    file: "doc.typ".to_string(),
                    main: false,
                    context: String::new(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn candidate(name: &str) -> Candidate {
        Candidate {
            term: name.to_string(),
            score: 0.5,
            locations: vec![CandidateLocation {
                file: "doc.typ".to_string(),
                context: format!("{name} context"),
            }],
        }
    }

    fn terms_file(terms: Vec<CuratedTerm>) -> CuratedTermsFile {
        CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms,
        }
    }

    fn candidates_file(candidates: Vec<Candidate>) -> CandidatesFile {
        CandidatesFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            candidates,
        }
    }

    #[test]
    fn diff_no_new_candidates() {
        let existing = terms_file(vec![
            curated("OAuth", &["OAuth 2.0"], true),
            curated("TLS", &[], true),
        ]);
        let fresh = candidates_file(vec![
            candidate("OAuth"),
            candidate("OAuth 2.0"),
            candidate("TLS"),
        ]);
        let diff = diff_candidates(&existing, &fresh);
        assert!(diff.new_candidates.is_empty());
        assert!(diff.stale_terms.is_empty());
        assert_eq!(diff.total_candidates, 3);
        assert_eq!(diff.new_ratio(), 0.0);
    }

    #[test]
    fn diff_new_candidate_found() {
        let existing = terms_file(vec![curated("OAuth", &[], true)]);
        let fresh = candidates_file(vec![candidate("OAuth"), candidate("PKCE")]);
        let diff = diff_candidates(&existing, &fresh);
        assert_eq!(diff.new_candidates.len(), 1);
        assert_eq!(diff.new_candidates[0].term, "PKCE");
        assert_eq!(diff.new_ratio(), 0.5);
    }

    #[test]
    fn diff_stale_term_detected() {
        let existing = terms_file(vec![
            curated("OAuth", &[], true),
            curated("removed_term", &[], true),
        ]);
        let fresh = candidates_file(vec![candidate("OAuth")]);
        let diff = diff_candidates(&existing, &fresh);
        assert_eq!(diff.stale_terms, vec!["removed_term"]);
    }

    #[test]
    fn diff_suggested_terms_not_stale() {
        // Suggested terms have no locations — they should NOT be flagged stale.
        let existing = terms_file(vec![
            curated("OAuth", &[], true),
            curated("bearer token", &[], false), // no locations = suggested
        ]);
        let fresh = candidates_file(vec![candidate("OAuth")]);
        let diff = diff_candidates(&existing, &fresh);
        assert!(diff.stale_terms.is_empty());
    }

    #[test]
    fn diff_case_insensitive() {
        let existing = terms_file(vec![curated("OAuth", &[], true)]);
        let fresh = candidates_file(vec![candidate("oauth")]);
        let diff = diff_candidates(&existing, &fresh);
        assert!(diff.new_candidates.is_empty());
    }

    #[test]
    fn diff_alias_match_covers_candidate() {
        let existing = terms_file(vec![curated("OAuth", &["OAuth 2.0"], true)]);
        let fresh = candidates_file(vec![candidate("OAuth 2.0")]);
        let diff = diff_candidates(&existing, &fresh);
        assert!(diff.new_candidates.is_empty());
    }

    #[test]
    fn diff_empty_candidates() {
        let existing = terms_file(vec![curated("OAuth", &[], true)]);
        let fresh = candidates_file(vec![]);
        let diff = diff_candidates(&existing, &fresh);
        assert_eq!(diff.new_ratio(), 0.0);
        assert_eq!(diff.stale_terms, vec!["OAuth"]);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core diff_`
Expected: FAIL — `diff_candidates` not defined.

- [ ] **Step 4: Implement `diff_candidates`**

Add above the `#[cfg(test)]` block:

```rust
/// Diff fresh candidates against existing curated terms.
///
/// A candidate is "new" if its term (lowercased) does not match any
/// existing curated term name or alias. A curated term is "stale" if
/// its name and aliases have zero matches in fresh candidates AND it
/// has at least one location (suggested-only terms are excluded).
pub fn diff_candidates(existing: &CuratedTermsFile, fresh: &CandidatesFile) -> CandidateDiff {
    // Build known-keys set from existing terms + aliases.
    let mut known_keys: HashSet<String> = HashSet::new();
    for term in &existing.terms {
        known_keys.insert(term.term.to_lowercase());
        for alias in &term.aliases {
            known_keys.insert(alias.to_lowercase());
        }
    }

    // Partition fresh candidates into known vs. new.
    let new_candidates: Vec<Candidate> = fresh
        .candidates
        .iter()
        .filter(|c| !known_keys.contains(&c.term.to_lowercase()))
        .cloned()
        .collect();

    // Build fresh-keys set for stale detection.
    let fresh_keys: HashSet<String> = fresh
        .candidates
        .iter()
        .map(|c| c.term.to_lowercase())
        .collect();

    // A term is stale if none of its keys appear in fresh candidates
    // AND it has locations (suggested-only terms are excluded).
    let stale_terms: Vec<String> = existing
        .terms
        .iter()
        .filter(|t| !t.locations.is_empty())
        .filter(|t| {
            let keys = std::iter::once(t.term.to_lowercase())
                .chain(t.aliases.iter().map(|a| a.to_lowercase()));
            !keys.into_iter().any(|k| fresh_keys.contains(&k))
        })
        .map(|t| t.term.clone())
        .collect();

    CandidateDiff {
        new_candidates,
        stale_terms,
        total_candidates: fresh.candidates.len(),
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core diff_`
Expected: PASS (all 7 tests).

- [ ] **Step 6: Write failing tests for `format_compact_index`**

Add to the test module:

```rust
#[test]
fn compact_index_basic() {
    let existing = terms_file(vec![
        CuratedTerm {
            term: "OAuth".to_string(),
            definition: "Auth standard.".to_string(),
            parent: Some("authentication".to_string()),
            aliases: vec!["OAuth 2.0".to_string(), "OAuth2".to_string()],
            see_also: vec!["API key".to_string()],
            children: Vec::new(),
            locations: vec![TermLocation {
                file: "auth.typ".to_string(),
                main: true,
                context: String::new(),
            }],
        },
        CuratedTerm {
            term: "TLS".to_string(),
            definition: "Encryption.".to_string(),
            parent: None,
            aliases: Vec::new(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: vec![TermLocation {
                file: "sec.typ".to_string(),
                main: false,
                context: String::new(),
            }],
        },
    ]);
    let compact = format_compact_index(&existing);
    assert!(compact.contains("OAuth | parent: authentication | aliases: OAuth 2.0, OAuth2 | see_also: API key"));
    assert!(compact.contains("TLS | (top-level, no relationships)"));
}

#[test]
fn compact_index_children_shown() {
    let existing = terms_file(vec![CuratedTerm {
        term: "authentication".to_string(),
        definition: "Identity verification.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: vec!["OAuth".to_string(), "SSO".to_string()],
        locations: vec![TermLocation {
            file: "auth.typ".to_string(),
            main: true,
            context: String::new(),
        }],
    }]);
    let compact = format_compact_index(&existing);
    assert!(compact.contains("children: OAuth, SSO"));
}

#[test]
fn compact_index_empty() {
    let existing = terms_file(vec![]);
    let compact = format_compact_index(&existing);
    assert!(compact.is_empty());
}
```

- [ ] **Step 7: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core compact_index`
Expected: FAIL — `format_compact_index` not defined.

- [ ] **Step 8: Implement `format_compact_index`**

Add above `#[cfg(test)]`:

```rust
/// Format the existing index as a compact pipe-delimited listing.
///
/// One line per term. No definitions, no locations — just relationships.
/// Designed for token efficiency (~50 tokens per term).
pub fn format_compact_index(existing: &CuratedTermsFile) -> String {
    let mut out = String::new();
    for term in &existing.terms {
        out.push_str(&term.term);

        let mut parts: Vec<String> = Vec::new();

        if let Some(ref parent) = term.parent {
            parts.push(format!("parent: {parent}"));
        }
        if !term.children.is_empty() {
            parts.push(format!("children: {}", term.children.join(", ")));
        }
        if !term.aliases.is_empty() {
            parts.push(format!("aliases: {}", term.aliases.join(", ")));
        }
        if !term.see_also.is_empty() {
            parts.push(format!("see_also: {}", term.see_also.join(", ")));
        }

        if parts.is_empty() {
            out.push_str(" | (top-level, no relationships)");
        } else {
            for part in &parts {
                out.push_str(" | ");
                out.push_str(part);
            }
        }

        out.push('\n');
    }
    out
}
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core compact_index`
Expected: PASS (all 3 tests).

- [ ] **Step 10: Commit**

```bash
git add crates/colophon-core/src/curate/incremental.rs crates/colophon-core/src/curate/mod.rs
git commit -m "feat(core): add incremental diffing and compact index format"
```

---

## Task 4: Merge Logic

**Files:**
- Modify: `crates/colophon-core/src/curate/incremental.rs`

- [ ] **Step 1: Write failing tests for `merge_delta`**

Add to the test module in `incremental.rs`. These tests need access to the delta types, so add an import at the top of the test module:

```rust
use crate::curate::terms::{ClaudeDeltaOutput, ClaudeTerm, ClaudeSuggested, DeltaModification, DeltaRemoval};
```

Tests:

```rust
fn delta(
    additions: Vec<ClaudeTerm>,
    modifications: Vec<DeltaModification>,
    removals: Vec<DeltaRemoval>,
    suggested: Vec<ClaudeSuggested>,
) -> ClaudeDeltaOutput {
    ClaudeDeltaOutput { additions, modifications, removals, suggested }
}

fn addition(term: &str) -> ClaudeTerm {
    ClaudeTerm {
        term: term.to_string(),
        definition: format!("{term} definition."),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        main_files: Vec::new(),
    }
}

#[test]
fn merge_removals() {
    let mut terms = vec![
        curated("OAuth", &[], true),
        curated("deprecated", &[], true),
        curated("TLS", &[], true),
    ];
    let delta = delta(
        vec![],
        vec![],
        vec![DeltaRemoval {
            term: "deprecated".to_string(),
            reason: "gone".to_string(),
        }],
        vec![],
    );
    let log = merge_delta(&mut terms, &delta);
    assert_eq!(terms.len(), 2);
    assert!(terms.iter().all(|t| t.term != "deprecated"));
    assert_eq!(log.removed, 1);
}

#[test]
fn merge_modifications_sparse() {
    let mut terms = vec![CuratedTerm {
        term: "OAuth".to_string(),
        definition: "Old definition.".to_string(),
        parent: None,
        aliases: vec!["OAuth 2.0".to_string()],
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "auth.typ".to_string(),
            main: true,
            context: String::new(),
        }],
    }];
    let delta = delta(
        vec![],
        vec![DeltaModification {
            term: "OAuth".to_string(),
            definition: Some("New definition.".to_string()),
            parent: None,
            aliases: None, // not changed — should preserve existing
            see_also: None,
            reason: "updated".to_string(),
        }],
        vec![],
        vec![],
    );
    let log = merge_delta(&mut terms, &delta);
    assert_eq!(terms[0].definition, "New definition.");
    assert_eq!(terms[0].aliases, vec!["OAuth 2.0"]); // preserved
    assert_eq!(log.modified, 1);
}

#[test]
fn merge_modifications_reparent() {
    let mut terms = vec![curated("OAuth", &[], true)];
    let delta = delta(
        vec![],
        vec![DeltaModification {
            term: "OAuth".to_string(),
            definition: None,
            parent: Some("security".to_string()),
            aliases: None,
            see_also: None,
            reason: "reparent".to_string(),
        }],
        vec![],
        vec![],
    );
    merge_delta(&mut terms, &delta);
    assert_eq!(terms[0].parent.as_deref(), Some("security"));
}

#[test]
fn merge_additions() {
    let mut terms = vec![curated("OAuth", &[], true)];
    let delta = delta(vec![addition("PKCE")], vec![], vec![], vec![]);
    let log = merge_delta(&mut terms, &delta);
    assert_eq!(terms.len(), 2);
    assert!(terms.iter().any(|t| t.term == "PKCE"));
    assert_eq!(log.added, 1);
}

#[test]
fn merge_suggested() {
    let mut terms = vec![curated("OAuth", &[], true)];
    let delta = delta(
        vec![],
        vec![],
        vec![],
        vec![ClaudeSuggested {
            term: "bearer token".to_string(),
            definition: "A token type.".to_string(),
            parent: Some("OAuth".to_string()),
        }],
    );
    let log = merge_delta(&mut terms, &delta);
    let bt = terms.iter().find(|t| t.term == "bearer token").unwrap();
    assert!(bt.locations.is_empty());
    assert_eq!(bt.parent.as_deref(), Some("OAuth"));
    assert_eq!(log.suggested, 1);
}

#[test]
fn merge_modification_dangling_target_skipped() {
    // Modify a term that doesn't exist — should be logged and skipped.
    let mut terms = vec![curated("OAuth", &[], true)];
    let delta = delta(
        vec![],
        vec![DeltaModification {
            term: "nonexistent".to_string(),
            definition: Some("X".to_string()),
            parent: None,
            aliases: None,
            see_also: None,
            reason: "ghost".to_string(),
        }],
        vec![],
        vec![],
    );
    let log = merge_delta(&mut terms, &delta);
    assert_eq!(log.modified, 0);
    assert_eq!(terms.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core merge_`
Expected: FAIL — `merge_delta` and `MergeLog` not defined.

- [ ] **Step 3: Implement `merge_delta`**

Add above `#[cfg(test)]`:

```rust
use super::terms::{
    ClaudeDeltaOutput, CuratedTerm, CuratedTermsFile, TermLocation,
};
use crate::extract::candidates::{Candidate, CandidateLocation, CandidatesFile};

/// Log of what merge_delta changed.
#[derive(Debug, Default)]
pub struct MergeLog {
    /// Terms removed.
    pub removed: usize,
    /// Terms modified.
    pub modified: usize,
    /// Terms added.
    pub added: usize,
    /// Suggested terms added.
    pub suggested: usize,
}

/// Apply a delta response to an existing term list.
///
/// Mutates `terms` in place. Order: remove → modify → add → suggested.
/// Returns a log of what changed.
pub fn merge_delta(terms: &mut Vec<CuratedTerm>, delta: &ClaudeDeltaOutput) -> MergeLog {
    let mut log = MergeLog::default();

    // 1. Remove
    let remove_set: HashSet<String> = delta
        .removals
        .iter()
        .map(|r| r.term.to_lowercase())
        .collect();
    let before = terms.len();
    terms.retain(|t| !remove_set.contains(&t.term.to_lowercase()));
    log.removed = before - terms.len();

    for r in &delta.removals {
        tracing::info!(term = %r.term, reason = %r.reason, "removed term");
    }

    // 2. Modify (sparse)
    for m in &delta.modifications {
        let Some(existing) = terms.iter_mut().find(|t| t.term.eq_ignore_ascii_case(&m.term))
        else {
            tracing::warn!(term = %m.term, "modification target not found — skipping");
            continue;
        };
        if let Some(ref def) = m.definition {
            existing.definition = def.clone();
        }
        if m.parent.is_some() {
            existing.parent = m.parent.clone();
        }
        if let Some(ref aliases) = m.aliases {
            existing.aliases = aliases.clone();
        }
        if let Some(ref see_also) = m.see_also {
            existing.see_also = see_also.clone();
        }
        log.modified += 1;
        tracing::info!(term = %m.term, reason = %m.reason, "modified term");
    }

    // 3. Add
    for a in &delta.additions {
        terms.push(CuratedTerm {
            term: a.term.clone(),
            definition: a.definition.clone(),
            parent: a.parent.clone(),
            aliases: a.aliases.clone(),
            see_also: a.see_also.clone(),
            children: Vec::new(),
            locations: Vec::new(), // filled by re-map step
        });
        log.added += 1;
    }

    // 4. Suggested
    for s in &delta.suggested {
        terms.push(CuratedTerm {
            term: s.term.clone(),
            definition: s.definition.clone(),
            parent: s.parent.clone(),
            aliases: Vec::new(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: Vec::new(),
        });
        log.suggested += 1;
    }

    log
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core merge_`
Expected: PASS (all 6 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/colophon-core/src/curate/incremental.rs
git commit -m "feat(core): add delta merge logic for incremental curate"
```

---

## Task 5: Incremental Prompt and Schema in Claude Module

**Files:**
- Modify: `crates/colophon-core/src/curate/claude.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module in `claude.rs`:

```rust
#[test]
fn incremental_system_prompt_contains_index_and_instructions() {
    let config = CurateConfig::default();
    let compact_index = "OAuth | parent: auth | aliases: OAuth 2.0\nTLS | (top-level)\n";
    let prompt = build_incremental_system_prompt(&config, compact_index);
    assert!(prompt.contains("updating an existing back-of-book index"));
    assert!(prompt.contains("EXISTING INDEX"));
    assert!(prompt.contains("OAuth | parent: auth"));
    assert!(prompt.contains("For each new candidate"));
}

#[test]
fn incremental_stdin_payload_contains_candidates_and_stale() {
    let config = CurateConfig {
        full_candidates: true,
        ..CurateConfig::default()
    };
    let candidates_yaml = "- term: PKCE\n  score: 0.9\n";
    let stale = vec!["old_term".to_string()];
    let payload = build_incremental_stdin_payload(&config, candidates_yaml, &stale);
    assert!(payload.contains("PKCE"));
    assert!(payload.contains("POTENTIALLY STALE"));
    assert!(payload.contains("old_term"));
}

#[test]
fn incremental_stdin_payload_no_stale_section_when_empty() {
    let config = CurateConfig {
        full_candidates: true,
        ..CurateConfig::default()
    };
    let payload = build_incremental_stdin_payload(&config, "candidates", &[]);
    assert!(!payload.contains("POTENTIALLY STALE"));
}

#[test]
fn delta_schema_json_is_valid() {
    let value: serde_json::Value =
        serde_json::from_str(DELTA_SCHEMA_JSON).expect("delta schema should be valid JSON");
    assert_eq!(value["type"], "object");
    assert!(value["properties"]["additions"].is_object());
    assert!(value["properties"]["modifications"].is_object());
    assert!(value["properties"]["removals"].is_object());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core incremental_system_prompt`
Expected: FAIL — functions and constants not defined.

- [ ] **Step 3: Add delta schema embed, incremental prompt, and payload builders**

Add to `claude.rs`:

At the top, alongside `SCHEMA_JSON`:
```rust
/// JSON Schema for the curate delta output (incremental mode).
const DELTA_SCHEMA_JSON: &str = include_str!("../../../../config/curate-delta-schema.json");
```

Then add these functions (near the existing `build_system_prompt`/`build_stdin_payload`):

```rust
/// The incremental system prompt base.
const DEFAULT_INCREMENTAL_SYSTEM_PROMPT_BASE: &str = r#"You are a professional book indexer updating an existing back-of-book index with new candidate terms.

EXISTING INDEX (for context — do NOT regenerate these):
"#;

/// Instructions appended after the existing index in incremental mode.
const INCREMENTAL_INSTRUCTIONS: &str = r#"
Instructions:
1. For each new candidate: add as new term, merge as alias of existing, or discard
2. For stale terms: recommend keep (suggested/conceptual) or remove
3. If a new term should be a child of an existing term, set parent accordingly
4. If new terms create new see_also relationships with existing terms, include the modification
5. Return ONLY additions, modifications, and removals — not unchanged terms
6. Every modification MUST include a reason"#;

/// Build the incremental system prompt including the compact existing index.
pub(super) fn build_incremental_system_prompt(config: &CurateConfig, compact_index: &str) -> String {
    let base = config
        .system_prompt
        .as_deref()
        .map_or_else(
            || format!("{DEFAULT_INCREMENTAL_SYSTEM_PROMPT_BASE}{compact_index}{INCREMENTAL_INSTRUCTIONS}"),
            |custom| format!("{custom}\n\nEXISTING INDEX:\n{compact_index}{INCREMENTAL_INSTRUCTIONS}"),
        );

    let format_suffix = if config.full_candidates {
        INPUT_FORMAT_FULL
    } else {
        INPUT_FORMAT_COMPACT
    };

    format!("{base}{format_suffix}")
}

/// Build the incremental stdin payload: new candidates + stale terms.
pub(super) fn build_incremental_stdin_payload(
    config: &CurateConfig,
    new_candidates_yaml: &str,
    stale_terms: &[String],
) -> String {
    let mut payload = String::from("NEW CANDIDATES to integrate:\n");
    payload.push_str(new_candidates_yaml);

    if !stale_terms.is_empty() {
        payload.push_str("\n\nPOTENTIALLY STALE terms (no longer found in corpus):\n");
        for term in stale_terms {
            payload.push_str("- ");
            payload.push_str(term);
            payload.push('\n');
        }
    }

    let instruction = config
        .prompt
        .as_deref()
        .map(|p| format!("{p}\n\n{DEFAULT_INSTRUCTION}"))
        .unwrap_or_else(|| DEFAULT_INSTRUCTION.to_string());

    format!("{payload}\n\n{instruction}")
}

/// Return the incremental system prompt for cost estimation.
pub(super) fn system_prompt_for_incremental(config: &CurateConfig, compact_index: &str) -> String {
    build_incremental_system_prompt(config, compact_index)
}

/// Return the incremental stdin payload for cost estimation.
pub(super) fn stdin_payload_for_incremental(
    config: &CurateConfig,
    new_candidates_yaml: &str,
    stale_terms: &[String],
) -> String {
    build_incremental_stdin_payload(config, new_candidates_yaml, stale_terms)
}

/// Return the delta JSON schema string.
pub(super) const fn delta_schema_json() -> &'static str {
    DELTA_SCHEMA_JSON
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core incremental_ delta_schema`
Expected: PASS (all 4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/colophon-core/src/curate/claude.rs
git commit -m "feat(core): add incremental prompt and delta schema to claude module"
```

---

## Task 6: Wire Incremental Pipeline in `curate/mod.rs`

**Files:**
- Modify: `crates/colophon-core/src/curate/mod.rs`
- Modify: `crates/colophon-core/src/error.rs`

- [ ] **Step 1: Add `NoExistingTerms` error variant**

In `error.rs`, add to `CurateError`:

```rust
/// Incremental mode requires an existing terms file but none was found.
#[error("no existing terms file found at {0} — run full curate first")]
NoExistingTerms(String),
```

- [ ] **Step 2: Add public API functions**

Add to `curate/mod.rs` (after the existing `run` function):

```rust
/// Estimate cost for incremental curate.
///
/// Requires the compact index and new candidates to size the prompt.
pub fn estimate_cost_incremental(
    new_candidates_yaml: &str,
    compact_index: &str,
    stale_terms: &[String],
    config: &CurateConfig,
) -> CostEstimate {
    let system_prompt = claude::system_prompt_for_incremental(config, compact_index);
    let stdin_payload =
        claude::stdin_payload_for_incremental(config, new_candidates_yaml, stale_terms);
    let schema_json = claude::delta_schema_json();

    cost::estimate(
        &system_prompt,
        &stdin_payload,
        schema_json,
        config.max_output_tokens,
        &config.model,
    )
}

/// Run the incremental curation pipeline.
///
/// Diffs fresh candidates against existing curated terms, sends only
/// new candidates to Claude, and merges the delta response back.
/// When there are no new candidates (0% delta), performs a mechanical
/// location refresh without invoking Claude.
pub fn run_incremental(
    existing: &CuratedTermsFile,
    candidates: &CandidatesFile,
    candidates_yaml: &str,
    config: &CurateConfig,
    extra_args: &[String],
    progress: &ProgressBar,
) -> CurateResult<IncrementalOutput> {
    use incremental::{diff_candidates, format_compact_index, merge_delta};

    let diff = diff_candidates(existing, candidates);

    tracing::info!(
        new = diff.new_candidates.len(),
        stale = diff.stale_terms.len(),
        total = diff.total_candidates,
        ratio = format!("{:.1}%", diff.new_ratio() * 100.0),
        "incremental diff complete"
    );

    // Threshold warnings.
    let ratio = diff.new_ratio();
    if ratio >= 0.7 {
        tracing::warn!("{}% new candidates — strongly recommend --full-rebuild for better cross-term relationships", (ratio * 100.0) as u32);
    } else if ratio >= 0.4 {
        tracing::warn!("{}% new candidates — consider --full-rebuild for better results", (ratio * 100.0) as u32);
    }

    let mut terms = existing.terms.clone();

    // If there are new candidates, invoke Claude for semantic curation.
    let (merge_log, invoke_result) = if diff.new_candidates.is_empty() {
        progress.set_message("No new terms — refreshing locations only");
        (incremental::MergeLog::default(), None)
    } else {
        // Build the incremental prompt components.
        let compact_index = format_compact_index(existing);

        // Serialize new candidates for the prompt.
        let new_candidates_file = CandidatesFile {
            version: candidates.version,
            generated: candidates.generated.clone(),
            source_dir: candidates.source_dir.clone(),
            document_count: candidates.document_count,
            candidates: diff.new_candidates,
        };
        let new_yaml = serde_yaml::to_string(&new_candidates_file)?;

        // Invoke Claude with delta schema.
        let result = claude::invoke_incremental(
            config,
            &new_yaml,
            &compact_index,
            &diff.stale_terms,
            extra_args,
            progress,
        )?;

        let log = merge_delta(&mut terms, &result.output);
        (log, Some(result))
    };

    // Re-map locations for ALL terms using fresh candidates.
    remap_locations(&mut terms, candidates);

    // Rebuild parent→children hierarchy.
    rebuild_children(&mut terms);

    // Validate referential integrity.
    validate_parents(&mut terms);

    // Sort and truncate.
    terms.sort_by(|a, b| a.term.to_lowercase().cmp(&b.term.to_lowercase()));
    terms.truncate(config.max_terms);

    let (thinking, editorial, turns, thinking_tokens, usage) = invoke_result
        .map(|r| (r.thinking, r.editorial, r.turns, r.thinking_tokens, r.usage))
        .unwrap_or_default();

    Ok(IncrementalOutput {
        terms_file: CuratedTermsFile {
            version: existing.version,
            generated: crate::extract::format_timestamp(std::time::SystemTime::now()),
            source_dir: candidates.source_dir.clone(),
            document_count: candidates.document_count,
            terms,
        },
        merge_log,
        diff_ratio: ratio,
        thinking,
        editorial,
        turns,
        thinking_tokens,
        usage,
    })
}
```

- [ ] **Step 3: Add the `IncrementalOutput` struct and helper functions**

```rust
/// Result of the incremental curation pipeline.
pub struct IncrementalOutput {
    /// The updated curated terms file.
    pub terms_file: CuratedTermsFile,
    /// What the merge changed.
    pub merge_log: incremental::MergeLog,
    /// Ratio of new candidates to total (0.0–1.0).
    pub diff_ratio: f64,
    /// Accumulated thinking output from Claude.
    pub thinking: String,
    /// Editorial summary text.
    pub editorial: String,
    /// Number of API turns used.
    pub turns: usize,
    /// Number of thinking token deltas received.
    pub thinking_tokens: usize,
    /// Actual token usage.
    pub usage: TokenUsage,
}

/// Re-map locations for all terms using fresh candidate data.
///
/// This is the same join-by-name-and-alias logic as `post_process`,
/// extracted so incremental mode can reuse it.
fn remap_locations(terms: &mut [CuratedTerm], candidates: &CandidatesFile) {
    let candidate_map: HashMap<String, &crate::extract::candidates::Candidate> = candidates
        .candidates
        .iter()
        .map(|c| (c.term.to_lowercase(), c))
        .collect();

    for term in terms.iter_mut() {
        let lookup_keys: Vec<String> = std::iter::once(term.term.to_lowercase())
            .chain(term.aliases.iter().map(|a| a.to_lowercase()))
            .collect();

        let mut locations = Vec::new();
        let mut seen_files = std::collections::HashSet::new();

        for key in &lookup_keys {
            if let Some(candidate) = candidate_map.get(key) {
                for loc in &candidate.locations {
                    if seen_files.insert(loc.file.clone()) {
                        // Preserve existing main flags if term already has locations.
                        let was_main = term
                            .locations
                            .iter()
                            .any(|existing| existing.file == loc.file && existing.main);
                        locations.push(TermLocation {
                            file: loc.file.clone(),
                            main: was_main,
                            context: loc.context.clone(),
                        });
                    }
                }
            }
        }

        term.locations = locations;
    }
}

/// Rebuild children arrays from parent pointers.
fn rebuild_children(terms: &mut [CuratedTerm]) {
    let parent_map: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for t in terms.iter() {
            if let Some(ref parent) = t.parent {
                map.entry(parent.clone()).or_default().push(t.term.clone());
            }
        }
        map
    };
    for term in terms.iter_mut() {
        term.children = parent_map
            .get(&term.term)
            .cloned()
            .unwrap_or_default();
        term.children.sort_unstable();
    }
}

/// Warn on dangling parent refs; nullify if parent was removed.
fn validate_parents(terms: &mut [CuratedTerm]) {
    let term_set: std::collections::HashSet<String> =
        terms.iter().map(|t| t.term.clone()).collect();
    for t in terms.iter_mut() {
        if let Some(ref parent) = t.parent
            && !term_set.contains(parent)
        {
            tracing::warn!(
                term = %t.term,
                parent = %parent,
                "dangling parent ref — nullifying"
            );
            t.parent = None;
        }
    }
}
```

Note: `remap_locations` preserves existing `main` flags for terms that already have locations (unlike full mode which uses `main_files` from Claude). For additions, `main` defaults to `false` — Claude provides `main_files` in additions and these are applied via the candidate map.

- [ ] **Step 4: Add `invoke_incremental` to `claude.rs`**

In `claude.rs`, add after the existing `invoke` function:

```rust
/// Invoke the Claude CLI in incremental mode with the delta schema.
///
/// Same streaming machinery as `invoke`, different prompt and schema.
pub(super) fn invoke_incremental(
    config: &CurateConfig,
    new_candidates_yaml: &str,
    compact_index: &str,
    stale_terms: &[String],
    extra_args: &[String],
    progress: &ProgressBar,
) -> Result<IncrementalInvokeResult, CurateError> {
    let claude_path = find_claude()?;

    let system_prompt = build_incremental_system_prompt(config, compact_index);
    let stdin_payload =
        build_incremental_stdin_payload(config, new_candidates_yaml, stale_terms);

    let no_plugins_dir = create_no_plugins_dir()?;
    let settings_file = write_settings_file(&config.claude_settings)?;

    let plugin_dir_str = no_plugins_dir.path().to_string_lossy().to_string();
    let settings_path_str = settings_file.path().to_string_lossy().to_string();

    let mut cmd = Command::new(&claude_path);
    cmd.arg("--print")
        .arg("--json-schema")
        .arg(DELTA_SCHEMA_JSON)
        .arg("--model")
        .arg(&config.model)
        .arg("--system-prompt")
        .arg(&system_prompt)
        .arg("--output-format")
        .arg("stream-json")
        .arg("--include-partial-messages")
        .arg("--verbose")
        .arg("--tools")
        .arg("")
        .arg("--no-session-persistence")
        .arg("--disable-slash-commands")
        .arg("--effort")
        .arg(&config.effort)
        .arg("--plugin-dir")
        .arg(&plugin_dir_str)
        .arg("--settings")
        .arg(&settings_path_str)
        .arg("--setting-sources")
        .arg("local");

    for arg in extra_args {
        cmd.arg(arg);
    }

    cmd.env("CLAUDECODE", "")
        .env(
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            config.max_output_tokens.to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    tracing::debug!(
        model = %config.model,
        effort = %config.effort,
        payload_len = stdin_payload.len(),
        "invoking claude CLI (incremental mode)"
    );

    let mut child = cmd.spawn().map_err(|e| CurateError::ClaudeFailed {
        exit_code: None,
        stderr: format!("failed to spawn claude: {e}"),
    })?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CurateError::ClaudeFailed {
                exit_code: None,
                stderr: "failed to open claude stdin".to_string(),
            })?;
        stdin
            .write_all(stdin_payload.as_bytes())
            .map_err(|e| CurateError::ClaudeFailed {
                exit_code: None,
                stderr: format!("stdin write failed: {e}"),
            })?;
    }

    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| CurateError::ClaudeFailed {
            exit_code: None,
            stderr: "failed to open claude stderr".to_string(),
        })?;
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr_pipe), &mut buf);
        buf
    });

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CurateError::ClaudeFailed {
            exit_code: None,
            stderr: "failed to open claude stdout".to_string(),
        })?;

    let result = parse_delta_stream(stdout, progress)?;

    let status = child.wait().map_err(|e| CurateError::ClaudeFailed {
        exit_code: None,
        stderr: format!("failed to wait for claude: {e}"),
    })?;

    let captured_stderr = stderr_thread.join().unwrap_or_default();
    if !captured_stderr.is_empty() {
        tracing::debug!(stderr_len = captured_stderr.len(), "claude stderr captured");
    }

    if !status.success() && result.is_none() {
        return Err(CurateError::ClaudeFailed {
            exit_code: status.code(),
            stderr: captured_stderr,
        });
    }

    result.ok_or_else(|| CurateError::ParseResponse {
        detail: "no valid structured output found in incremental stream".to_string(),
    })
}
```

Also add the result type and delta stream parser (reuses same JSONL parsing, different output type):

```rust
use super::terms::ClaudeDeltaOutput;

/// Result of an incremental Claude CLI invocation.
pub(super) struct IncrementalInvokeResult {
    pub output: ClaudeDeltaOutput,
    pub thinking: String,
    pub editorial: String,
    pub turns: usize,
    pub thinking_tokens: usize,
    pub usage: TokenUsage,
}

/// Parse streaming JSONL for incremental mode (delta schema).
fn parse_delta_stream(
    stdout: impl std::io::Read,
    progress: &ProgressBar,
) -> Result<Option<IncrementalInvokeResult>, CurateError> {
    let reader = BufReader::new(stdout);

    let mut thinking = String::new();
    let mut editorial = String::new();
    let mut current_turn_json = String::new();
    let mut last_valid_output: Option<ClaudeDeltaOutput> = None;
    let mut thinking_deltas: usize = 0;
    let mut json_bytes: usize = 0;
    let mut turn_count: usize = 0;
    let mut usage = TokenUsage::default();

    progress.set_message("Starting incremental...");
    progress.enable_steady_tick(Duration::from_millis(120));

    for line in reader.lines() {
        let line = line.map_err(|e| CurateError::ClaudeFailed {
            exit_code: None,
            stderr: format!("failed to read stream: {e}"),
        })?;

        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if event["type"] != "stream_event" {
            continue;
        }

        let evt = &event["event"];
        match evt["type"].as_str() {
            Some("message_start") => {
                turn_count += 1;
                current_turn_json.clear();

                if let Some(msg_usage) = evt["message"]["usage"].as_object() {
                    if let Some(n) = msg_usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        usage.input_tokens += n as usize;
                    }
                    if let Some(n) = msg_usage
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64())
                    {
                        usage.cache_creation_input_tokens += n as usize;
                    }
                    if let Some(n) = msg_usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64())
                    {
                        usage.cache_read_input_tokens += n as usize;
                    }
                }
            }
            Some("content_block_delta") => match evt["delta"]["type"].as_str() {
                Some("thinking_delta") => {
                    if let Some(text) = evt["delta"]["thinking"].as_str() {
                        thinking.push_str(text);
                        thinking_deltas += 1;
                        progress.set_message(format!("Thinking... ({thinking_deltas} tokens)"));
                    }
                }
                Some("text_delta") => {
                    if let Some(text) = evt["delta"]["text"].as_str() {
                        editorial.push_str(text);
                    }
                }
                Some("input_json_delta") => {
                    if let Some(partial) = evt["delta"]["partial_json"].as_str() {
                        current_turn_json.push_str(partial);
                        json_bytes += partial.len();
                        progress
                            .set_message(format!("Generating delta... ({json_bytes} bytes)"));
                    }
                }
                _ => {}
            },
            Some("message_delta") => {
                if let Some(n) = evt["usage"]["output_tokens"].as_u64() {
                    usage.output_tokens += n as usize;
                }
            }
            Some("message_stop") => {
                if !current_turn_json.is_empty() {
                    match serde_json::from_str::<ClaudeDeltaOutput>(&current_turn_json) {
                        Ok(output) => {
                            tracing::debug!(
                                turn = turn_count,
                                additions = output.additions.len(),
                                modifications = output.modifications.len(),
                                removals = output.removals.len(),
                                "valid delta output from turn"
                            );
                            last_valid_output = Some(output);
                        }
                        Err(e) => {
                            tracing::warn!(
                                turn = turn_count,
                                error = %e,
                                "failed to parse delta JSON"
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(ref output) = last_valid_output {
        progress.finish_with_message(format!(
            "Delta: +{} ~{} -{} ({} turns)",
            output.additions.len(),
            output.modifications.len(),
            output.removals.len(),
            turn_count,
        ));
    } else {
        progress.finish_with_message("No valid delta output");
    }

    Ok(last_valid_output.map(|output| IncrementalInvokeResult {
        output,
        thinking,
        editorial,
        turns: turn_count,
        thinking_tokens: thinking_deltas,
        usage,
    }))
}
```

- [ ] **Step 5: Run clippy to verify everything compiles**

Run: `just clippy`
Expected: Clean pass.

- [ ] **Step 6: Commit**

```bash
git add crates/colophon-core/src/curate/mod.rs crates/colophon-core/src/curate/claude.rs crates/colophon-core/src/error.rs
git commit -m "feat(core): wire incremental curate pipeline with Claude invocation"
```

---

## Task 7: CLI Integration

**Files:**
- Modify: `crates/colophon/src/commands/curate.rs`

- [ ] **Step 1: Add `--full-rebuild` flag to `CurateArgs`**

```rust
/// Force full rebuild even when curated terms file exists
#[arg(long)]
pub full_rebuild: bool,
```

- [ ] **Step 2: Add auto-detect incremental branch in `cmd_curate`**

After the existing `let candidates = CandidatesFile::from_yaml(...)` block and before `let estimate = curate::estimate_cost(...)`, add auto-detection:

```rust
// Auto-detect incremental mode: if terms file exists and --full-rebuild not set.
let terms_path = Path::new(&args.output_dir).join("colophon-terms.yaml");
if !args.full_rebuild && terms_path.exists() {
    return cmd_curate_incremental(args, json, &curate_config, candidates, &candidates_yaml, &terms_path);
}
```

Then add the new function:

```rust
/// Run incremental curation pipeline.
fn cmd_curate_incremental(
    args: CurateArgs,
    json: bool,
    config: &CurateConfig,
    candidates: CandidatesFile,
    candidates_yaml: &str,
    terms_path: &Path,
) -> anyhow::Result<()> {
    let output_dir = Path::new(&args.output_dir);

    // Load existing terms file.
    let existing_yaml = std::fs::read_to_string(terms_path)
        .with_context(|| format!("failed to read {}", terms_path.display()))?;
    let existing = CuratedTermsFile::from_yaml(&existing_yaml)
        .with_context(|| format!("failed to parse {}", terms_path.display()))?;

    // Diff to get stats for display and cost estimation.
    let diff = colophon_core::curate::incremental::diff_candidates(&existing, &candidates);
    let compact_index = colophon_core::curate::incremental::format_compact_index(&existing);

    if diff.new_candidates.is_empty() {
        // 0% delta — mechanical refresh only, zero cost.
        if !json {
            eprintln!("No new terms found. Refreshing locations...");
        }
    }

    // Serialize new candidates for cost estimation.
    let new_candidates_file = CandidatesFile {
        version: candidates.version,
        generated: candidates.generated.clone(),
        source_dir: candidates.source_dir.clone(),
        document_count: candidates.document_count,
        candidates: diff.new_candidates.clone(),
    };
    let new_yaml = serde_yaml::to_string(&new_candidates_file)?;

    // Cost estimation.
    if !diff.new_candidates.is_empty() {
        let estimate = curate::estimate_cost_incremental(
            &new_yaml,
            &compact_index,
            &diff.stale_terms,
            config,
        );

        if args.dry_run {
            if json {
                let out = serde_json::json!({
                    "mode": "incremental",
                    "new_candidates": diff.new_candidates.len(),
                    "total_candidates": diff.total_candidates,
                    "stale_terms": diff.stale_terms.len(),
                    "input_tokens": estimate.input_tokens,
                    "max_output_tokens": estimate.max_output_tokens,
                    "model": estimate.model,
                    "estimated_usd": estimate.estimated_usd,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                eprintln!(
                    "Incremental: {} new of {} candidates ({:.0}%), {} stale",
                    diff.new_candidates.len(),
                    diff.total_candidates,
                    diff.new_ratio() * 100.0,
                    diff.stale_terms.len(),
                );
                eprintln!("Cost estimate: {estimate}");
            }
            return Ok(());
        }

        // Budget check.
        let budget = args.max_budget_usd.or(config.max_budget_usd);
        if let Some(budget) = budget
            && estimate.pricing_known
            && estimate.estimated_usd > budget
        {
            anyhow::bail!(
                "estimated cost ${:.2} exceeds budget ${:.2}",
                estimate.estimated_usd,
                budget,
            );
        }
    } else if args.dry_run {
        if json {
            let out = serde_json::json!({
                "mode": "incremental",
                "new_candidates": 0,
                "total_candidates": diff.total_candidates,
                "estimated_usd": 0.0,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            eprintln!("No new terms. Location refresh only — $0.00");
        }
        return Ok(());
    }

    // Set up progress.
    let pb = if json {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .expect("valid template"),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
        eprintln!(
            "Incremental curate: {} new of {} candidates ({:.0}%), {} stale (model: {})...",
            diff.new_candidates.len(),
            diff.total_candidates,
            diff.new_ratio() * 100.0,
            diff.stale_terms.len(),
            config.model,
        );
        pb
    };

    let start = Instant::now();
    let result = curate::run_incremental(
        &existing,
        &candidates,
        candidates_yaml,
        config,
        &args.claude_args,
        &pb,
    )
    .context("incremental curation failed")?;
    let elapsed = start.elapsed();

    // Write output.
    let thinking_path = output_dir.join("colophon-curated-thinking.md");

    if json {
        let json_out = serde_json::to_string_pretty(&result.terms_file)?;
        println!("{json_out}");
    } else {
        let yaml = result.terms_file.to_yaml()?;
        std::fs::write(&terms_path, &yaml)?;

        // Append thinking for incremental (audit trail).
        if !result.thinking.is_empty() {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&thinking_path)?;
            writeln!(
                f,
                "\n---\n## Incremental update {}\n\n{}",
                result.terms_file.generated, result.thinking
            )?;
        }

        let terms_size = std::fs::metadata(&terms_path).map(|m| m.len()).unwrap_or(0);
        let ml = &result.merge_log;
        let unchanged = result.terms_file.terms.len()
            .saturating_sub(ml.added + ml.suggested);

        let mut builder = Builder::default();
        builder.push_record(["Mode", "incremental"]);
        builder.push_record(["Unchanged", &unchanged.to_string()]);
        builder.push_record(["Added", &ml.added.to_string()]);
        builder.push_record(["  (suggested)", &ml.suggested.to_string()]);
        builder.push_record(["Modified", &ml.modified.to_string()]);
        builder.push_record(["Removed", &ml.removed.to_string()]);
        builder.push_record([
            "New candidates",
            &format!(
                "{} of {} ({:.0}%)",
                (result.diff_ratio * diff.total_candidates as f64) as usize,
                diff.total_candidates,
                result.diff_ratio * 100.0,
            ),
        ]);
        if result.turns > 0 {
            builder.push_record(["Model", &config.model]);
            builder.push_record(["Turns", &result.turns.to_string()]);

            let u = &result.usage;
            if u.input_tokens > 0 {
                builder.push_record(["Input tokens", &u.input_tokens.to_string()]);
            }
            if u.output_tokens > 0 {
                builder.push_record(["Output tokens", &u.output_tokens.to_string()]);
            }
            if let Some(pricing) = ModelPricing::for_model(&config.model) {
                let actual_cost = u.actual_cost(&pricing);
                builder.push_record(["Actual cost", &format!("${actual_cost:.4}")]);
            }
        } else {
            builder.push_record(["Cost", "$0.00"]);
        }
        builder.push_record(["Time", &format!("{:.1}s", elapsed.as_secs_f64())]);
        builder.push_record([
            &format!("{}", terms_path.display()),
            &human_size(terms_size),
        ]);

        let table = builder.build().with(Style::rounded()).to_string();
        eprintln!("\n{table}");

        if !result.editorial.is_empty() {
            eprintln!("\n{}", result.editorial);
        }
    }

    Ok(())
}
```

Add the missing import at the top of `commands/curate.rs`:

```rust
use colophon_core::curate::terms::CuratedTermsFile;
```

- [ ] **Step 3: Run clippy**

Run: `just clippy`
Expected: Clean pass.

- [ ] **Step 4: Commit**

```bash
git add crates/colophon/src/commands/curate.rs
git commit -m "feat(cli): add auto-detect incremental curate with --full-rebuild escape hatch"
```

---

## Task 8: Integration Test — CLI Help and Full-Rebuild Flag

**Files:**
- Modify: `crates/colophon/tests/cli.rs`

- [ ] **Step 1: Add CLI integration tests**

```rust
#[test]
fn curate_help_shows_full_rebuild_flag() {
    Command::cargo_bin("colophon")
        .unwrap()
        .args(["curate", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--full-rebuild"));
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo nextest run -p colophon curate_help_shows_full_rebuild`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/colophon/tests/cli.rs
git commit -m "test(cli): add integration test for --full-rebuild flag"
```

---

## Task 9: Final Verification

- [ ] **Step 1: Run full check suite**

Run: `just check`
Expected: All green (except the pre-existing `extract_produces_yaml_output` failure).

- [ ] **Step 2: Verify clippy nursery is clean**

Run: `just clippy`
Expected: No warnings.

- [ ] **Step 3: Run doc tests**

Run: `cargo test --doc`
Expected: PASS.
