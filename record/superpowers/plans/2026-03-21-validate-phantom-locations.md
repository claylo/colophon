# Validate Phantom Locations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect unresolvable term locations after curate and suggest alias patches so render can place all markers.

**Architecture:** Extract shared Typst prose-range functions into `typst_prose.rs`. Add `validate.rs` module that checks each term+location against source files (canonical term, then aliases, then heuristic word-drop/plural). Integrate into CLI curate command as post-write advisory output.

**Tech Stack:** Rust, typst-syntax (existing dep), walkdir (existing dep)

---

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| Create | `crates/colophon-core/src/typst_prose.rs` | Shared AST-aware prose range + search functions |
| Create | `crates/colophon-core/src/validate.rs` | Post-curate location validation + alias suggestions |
| Modify | `crates/colophon-core/src/lib.rs` | Register new modules |
| Modify | `crates/colophon-core/src/render/typst.rs` | Remove moved functions, re-export from `typst_prose` |
| Modify | `crates/colophon/src/commands/curate.rs` | Call validation after writing terms, display suggestions |

---

### Task 1: Extract shared `typst_prose` module

Move AST-aware prose functions from `render/typst.rs` into a standalone module that both extract and render can use.

**Files:**
- Create: `crates/colophon-core/src/typst_prose.rs`
- Modify: `crates/colophon-core/src/lib.rs`
- Modify: `crates/colophon-core/src/render/typst.rs`

- [ ] **Step 1: Create `typst_prose.rs` with functions moved from `render/typst.rs`**

Move these functions (and their helpers) from `render/typst.rs:182-304`:
- `collect_prose_ranges(source: &str) -> Vec<(usize, usize)>` (make `pub`)
- `walk_for_ranges(node, base_offset, ranges, in_heading)` (keep private)
- `merge_ranges(ranges)` (keep private)
- `find_term_offset_in_prose(source, term, prose_ranges) -> Option<usize>` (make `pub`)

The module doc should note these are shared between extract validation and render.

```rust
//! Typst AST-aware prose range utilities.
//!
//! Shared between the extract validation pass and the render pipeline.
//! Walks the typst-syntax AST to identify byte ranges of prose text
//! (Text and Space nodes), excluding headings, code, math, labels,
//! refs, links, and function calls.
```

- [ ] **Step 2: Move prose-range tests from `render/typst.rs` to `typst_prose.rs`**

Move these tests from `render/typst.rs` into a `#[cfg(test)] mod tests` block in `typst_prose.rs`:
- `prose_ranges_skip_labels`
- `prose_ranges_skip_link_targets`
- `find_in_prose_skips_link_label`
- `find_in_prose_skips_code`
- `find_in_prose_multiword_across_nodes`
- `find_in_prose_skips_field_access_position`
- `find_in_prose_field_access_only_occurrence_skipped`
- `find_in_prose_not_found_only_in_syntax`

- [ ] **Step 3: Update `render/typst.rs` — remove moved code, import from `crate::typst_prose`**

Replace the `// ── AST-aware prose search ──` section (lines 182-304 and tests 593-696) with imports:

```rust
// Re-export for render/mod.rs convenience.
pub(crate) use crate::typst_prose::{collect_prose_ranges, find_term_offset_in_prose};
```

- [ ] **Step 4: Register module in `lib.rs`**

Add `pub mod typst_prose;` between `render` and the `pub use` block.

- [ ] **Step 5: Run tests to verify nothing broke**

Run: `cargo nextest run -p colophon-core`
Expected: All existing tests pass — same count as before, just relocated.

- [ ] **Step 6: Commit**

```bash
git add crates/colophon-core/src/typst_prose.rs crates/colophon-core/src/lib.rs crates/colophon-core/src/render/typst.rs
git commit -m "refactor: extract shared typst_prose module from render"
```

---

### Task 2: Create `validate.rs` with core types and resolution logic

**Files:**
- Create: `crates/colophon-core/src/validate.rs`
- Modify: `crates/colophon-core/src/lib.rs`

- [ ] **Step 1: Write failing test — all locations resolve, empty suggestions**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::curate::terms::{CuratedTerm, CuratedTermsFile, TermLocation};
    use std::fs;
    use tempfile::TempDir;

    fn terms_file(terms: Vec<CuratedTerm>) -> CuratedTermsFile {
        CuratedTermsFile {
            version: 1,
            generated: "2026-03-21T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms,
        }
    }

    #[test]
    fn all_locations_resolve_no_suggestions() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("auth.typ"), "OAuth provides authorization.\n").unwrap();

        let terms = terms_file(vec![CuratedTerm {
            term: "OAuth".to_string(),
            definition: "Auth.".to_string(),
            parent: None,
            aliases: Vec::new(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: vec![TermLocation {
                file: "auth.typ".to_string(),
                main: true,
                context: String::new(),
            }],
        }]);

        let report = validate_locations(
            &terms,
            src.to_str().unwrap(),
            &["typ".to_string()],
        );
        assert_eq!(report.resolved, 1);
        assert_eq!(report.unresolved, 0);
        assert!(report.suggestions.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p colophon-core validate`
Expected: FAIL — module doesn't exist yet.

- [ ] **Step 3: Implement core types and `validate_locations` — canonical + alias resolution**

```rust
//! Post-curate validation — detect unresolvable locations and suggest aliases.

use std::path::Path;

use crate::curate::terms::CuratedTermsFile;
use crate::typst_prose;

/// A suggestion to add an alias so render can find the term.
#[derive(Debug, Clone)]
pub struct AliasSuggestion {
    /// Canonical term that can't be found.
    pub term: String,
    /// Source file where the term is expected.
    pub file: String,
    /// Text that WAS found in prose and should be added as an alias.
    pub suggested_alias: String,
}

/// Summary of a validation pass.
#[derive(Debug, Default)]
pub struct ValidationReport {
    /// Locations where canonical term or an alias was found.
    pub resolved: usize,
    /// Locations where neither canonical nor alias matched.
    pub unresolved: usize,
    /// Suggested aliases for unresolved locations.
    pub suggestions: Vec<AliasSuggestion>,
}

/// Check each term+location against source files.
///
/// For Typst files (`.typ`), uses AST-aware prose ranges.
/// For other files, uses simple case-insensitive search.
///
/// When a location can't be resolved by canonical term or aliases,
/// tries heuristic word-drop and singular/plural to suggest an alias.
pub fn validate_locations(
    terms: &CuratedTermsFile,
    source_dir: &str,
    extensions: &[String],
) -> ValidationReport {
    let source_path = Path::new(source_dir);
    let mut report = ValidationReport::default();

    // Cache: file path -> (content, Option<prose_ranges>)
    let mut file_cache: std::collections::HashMap<String, (String, Option<Vec<(usize, usize)>>)> =
        std::collections::HashMap::new();

    for term in &terms.terms {
        for loc in &term.locations {
            let file_path = source_path.join(&loc.file);
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            // Skip files with non-matching extensions.
            if !extensions.iter().any(|e| e == ext) {
                continue;
            }

            // Load and cache file content + prose ranges.
            let entry = file_cache.entry(loc.file.clone()).or_insert_with(|| {
                match std::fs::read_to_string(&file_path) {
                    Ok(content) => {
                        let ranges = if ext == "typ" {
                            Some(typst_prose::collect_prose_ranges(&content))
                        } else {
                            None
                        };
                        (content, ranges)
                    }
                    Err(_) => (String::new(), None),
                }
            });

            let (content, ranges) = entry;
            if content.is_empty() {
                report.unresolved += 1;
                continue;
            }

            // Try canonical term.
            let found = try_find(content, &term.term, ranges.as_deref());
            if found {
                report.resolved += 1;
                continue;
            }

            // Try each alias.
            let alias_found = term
                .aliases
                .iter()
                .any(|alias| try_find(content, alias, ranges.as_deref()));
            if alias_found {
                report.resolved += 1;
                continue;
            }

            // Unresolved — try heuristic.
            if let Some(suggested) = suggest_alias(content, &term.term, ranges.as_deref()) {
                report.suggestions.push(AliasSuggestion {
                    term: term.term.clone(),
                    file: loc.file.clone(),
                    suggested_alias: suggested,
                });
            }
            report.unresolved += 1;
        }
    }

    report
}

/// Try to find a term in file content, using AST-aware search for Typst.
fn try_find(content: &str, term: &str, prose_ranges: Option<&[(usize, usize)]>) -> bool {
    match prose_ranges {
        Some(ranges) => typst_prose::find_term_offset_in_prose(content, term, ranges).is_some(),
        None => {
            let lower = content.to_lowercase();
            lower.contains(&term.to_lowercase())
        }
    }
}
```

- [ ] **Step 4: Register module in `lib.rs`**

Add `pub mod validate;` after `pub mod render;`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p colophon-core validate`
Expected: PASS

- [ ] **Step 6: Write failing test — unresolved location with no suggestion**

```rust
#[test]
fn unresolved_location_no_matching_text() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("empty.typ"), "Nothing relevant here.\n").unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "OAuth".to_string(),
        definition: "Auth.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "empty.typ".to_string(),
            main: false,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.resolved, 0);
    assert_eq!(report.unresolved, 1);
    assert!(report.suggestions.is_empty());
}
```

- [ ] **Step 7: Run test — should pass (no suggestion for completely absent term)**

Run: `cargo nextest run -p colophon-core validate`
Expected: PASS

- [ ] **Step 8: Write failing test — alias resolves**

```rust
#[test]
fn alias_resolves_location() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("chat.typ"), "Slack is a messaging platform.\n").unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "Slack integration".to_string(),
        definition: "Chat.".to_string(),
        parent: None,
        aliases: vec!["Slack".to_string()],
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "chat.typ".to_string(),
            main: false,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.resolved, 1);
    assert_eq!(report.unresolved, 0);
}
```

- [ ] **Step 9: Run test — should pass**

Run: `cargo nextest run -p colophon-core validate`
Expected: PASS

- [ ] **Step 10: Write test — markdown file uses simple search (not AST-aware)**

```rust
#[test]
fn markdown_file_uses_simple_search() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("auth.md"), "# Auth\n\nOAuth provides authorization.\n").unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "OAuth".to_string(),
        definition: "Auth.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "auth.md".to_string(),
            main: true,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["md".to_string()]);
    assert_eq!(report.resolved, 1);
    assert_eq!(report.unresolved, 0);
}
```

- [ ] **Step 11: Run test — should pass**

Run: `cargo nextest run -p colophon-core markdown_file`
Expected: PASS

- [ ] **Step 12: Write test — missing source file counts as unresolved**

```rust
#[test]
fn missing_source_file_counts_as_unresolved() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    // Don't create the file — location points to nonexistent file.

    let terms = terms_file(vec![CuratedTerm {
        term: "OAuth".to_string(),
        definition: "Auth.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "gone.typ".to_string(),
            main: false,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.resolved, 0);
    assert_eq!(report.unresolved, 1);
    assert!(report.suggestions.is_empty());
}
```

- [ ] **Step 13: Run test — should pass**

Run: `cargo nextest run -p colophon-core missing_source`
Expected: PASS

- [ ] **Step 14: Commit**

```bash
git add crates/colophon-core/src/validate.rs crates/colophon-core/src/lib.rs
git commit -m "feat(core): add validate module with location resolution"
```

---

### Task 3: Implement suggestion heuristic

Add `suggest_alias()` — tries word-drop and singular/plural to find what's actually in the source prose.

**Files:**
- Modify: `crates/colophon-core/src/validate.rs`

- [ ] **Step 1: Write failing test — compound term suggests dropped-first-word alias**

```rust
#[test]
fn suggests_alias_for_compound_term_drop_first_word() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("cloud.typ"),
        "Deploy to Bedrock for managed inference.\n",
    )
    .unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "Amazon Bedrock".to_string(),
        definition: "AWS.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "cloud.typ".to_string(),
            main: false,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.unresolved, 1);
    assert_eq!(report.suggestions.len(), 1);
    assert_eq!(report.suggestions[0].suggested_alias, "Bedrock");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p colophon-core suggest`
Expected: FAIL — `suggest_alias` returns `None`.

- [ ] **Step 3: Implement `suggest_alias`**

```rust
/// Try heuristic word-drop and singular/plural to find a matching substring.
///
/// For multi-word terms, tries progressively shorter suffixes and prefixes.
/// For any term, tries toggling a trailing 's' (singular/plural).
/// Returns the first match found in prose.
fn suggest_alias(
    content: &str,
    term: &str,
    prose_ranges: Option<&[(usize, usize)]>,
) -> Option<String> {
    let words: Vec<&str> = term.split_whitespace().collect();

    if words.len() >= 2 {
        // Try suffixes (drop from front): "Amazon Bedrock" → "Bedrock"
        for start in 1..words.len() {
            let suffix = words[start..].join(" ");
            if try_find(content, &suffix, prose_ranges) {
                return Some(suffix);
            }
        }

        // Try prefixes (drop from end): "Slack integration" → "Slack"
        for end in (1..words.len()).rev() {
            let prefix = words[..end].join(" ");
            if try_find(content, &prefix, prose_ranges) {
                return Some(prefix);
            }
        }
    }

    // Singular/plural toggle.
    if let Some(toggled) = toggle_plural(term) {
        if try_find(content, &toggled, prose_ranges) {
            return Some(toggled);
        }
    }

    None
}

/// Toggle trailing 's' for basic singular/plural heuristic.
fn toggle_plural(term: &str) -> Option<String> {
    if let Some(stem) = term.strip_suffix('s') {
        Some(stem.to_string())
    } else {
        Some(format!("{term}s"))
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p colophon-core suggest`
Expected: PASS

- [ ] **Step 5: Write test — multi-word suffix "Vertex AI" from "Google Vertex AI"**

```rust
#[test]
fn suggests_multi_word_suffix() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("cloud.typ"),
        "Deploy to Vertex AI for inference.\n",
    )
    .unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "Google Vertex AI".to_string(),
        definition: "GCP.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "cloud.typ".to_string(),
            main: false,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.suggestions.len(), 1);
    assert_eq!(report.suggestions[0].suggested_alias, "Vertex AI");
}
```

- [ ] **Step 6: Run test — should pass (suffix logic handles 3-word terms)**

Run: `cargo nextest run -p colophon-core suggests_multi`
Expected: PASS

- [ ] **Step 7: Write test — singular/plural suggestion**

```rust
#[test]
fn suggests_singular_for_plural_term() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("perms.typ"),
        "Each plugin can request elevated access.\n",
    )
    .unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "plugins".to_string(),
        definition: "Extensions.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![TermLocation {
            file: "perms.typ".to_string(),
            main: false,
            context: String::new(),
        }],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.suggestions.len(), 1);
    assert_eq!(report.suggestions[0].suggested_alias, "plugin");
}
```

- [ ] **Step 8: Run test — should pass**

Run: `cargo nextest run -p colophon-core suggests_singular`
Expected: PASS

- [ ] **Step 9: Write test — deduplicates suggestions across locations**

If the same term is unresolved in multiple files with the same suggested alias, only emit one suggestion per unique (term, suggested_alias) pair.

```rust
#[test]
fn deduplicates_suggestions_across_files() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("a.typ"), "Bedrock is a service.\n").unwrap();
    fs::write(src.join("b.typ"), "Use Bedrock for inference.\n").unwrap();

    let terms = terms_file(vec![CuratedTerm {
        term: "Amazon Bedrock".to_string(),
        definition: "AWS.".to_string(),
        parent: None,
        aliases: Vec::new(),
        see_also: Vec::new(),
        children: Vec::new(),
        locations: vec![
            TermLocation { file: "a.typ".to_string(), main: false, context: String::new() },
            TermLocation { file: "b.typ".to_string(), main: false, context: String::new() },
        ],
    }]);

    let report = validate_locations(&terms, src.to_str().unwrap(), &["typ".to_string()]);
    assert_eq!(report.unresolved, 2);
    // Both files suggest "Bedrock" but we only need one unique suggestion per term.
    let unique: std::collections::HashSet<(&str, &str)> = report
        .suggestions
        .iter()
        .map(|s| (s.term.as_str(), s.suggested_alias.as_str()))
        .collect();
    assert_eq!(unique.len(), 1);
}
```

Note: this test requires deduplication. Add dedup logic after collecting suggestions: retain only the first suggestion per (term, suggested_alias) pair. Update `validate_locations` to dedup before returning.

- [ ] **Step 10: Implement dedup and run test**

At the end of `validate_locations`, before returning:
```rust
report.suggestions.dedup_by(|a, b| a.term == b.term && a.suggested_alias == b.suggested_alias);
```

This works because suggestions are naturally grouped by term (we iterate terms in order).

Run: `cargo nextest run -p colophon-core dedup`
Expected: PASS

- [ ] **Step 11: Run full test suite**

Run: `cargo nextest run -p colophon-core`
Expected: All tests pass including new validate tests.

- [ ] **Step 12: Commit**

```bash
git add crates/colophon-core/src/validate.rs
git commit -m "feat(core): add alias suggestion heuristic to validate"
```

---

### Task 4: Integrate validation into CLI curate command

**Files:**
- Modify: `crates/colophon/src/commands/curate.rs`

- [ ] **Step 1: Add validation call after writing terms in `cmd_curate`**

After the terms file is written (line ~188 in `cmd_curate`, after `std::fs::write(&terms_path, &yaml)?`), add:

```rust
// Validate locations and suggest aliases.
let validation = colophon_core::validate::validate_locations(
    &result.terms_file,
    &result.terms_file.source_dir,
    &config.source.extensions,
);
```

Store the report for display after the summary table.

- [ ] **Step 2: Display suggestions after the summary table**

After the existing `eprintln!("\n{table}")` and editorial output, add:

```rust
if !validation.suggestions.is_empty() {
    eprintln!("\n⚠ {} location(s) can't be resolved — suggested aliases:", validation.unresolved);
    for s in &validation.suggestions {
        eprintln!("  \"{}\" → add alias \"{}\"", s.term, s.suggested_alias);
    }
    eprintln!("\nAdd these aliases to colophon-terms.yaml to improve render coverage.");
}
```

Wait — Clay said no emojis unless explicitly requested. The `⚠` is a unicode character commonly used in CLI output, not an emoji per se. But to be safe, use a text indicator instead:

```rust
if !validation.suggestions.is_empty() {
    eprintln!();
    eprintln!("Validation: {} resolved, {} unresolved", validation.resolved, validation.unresolved);
    eprintln!("Suggested aliases for unresolved locations:");
    for s in &validation.suggestions {
        eprintln!("  {} -> add alias \"{}\"", s.term, s.suggested_alias);
    }
}
```

- [ ] **Step 3: Add same validation call to `cmd_curate_incremental`**

After the incremental terms file is written (after `std::fs::write(terms_path, &yaml)?`), add the same validation + display block. Extract a helper function to avoid duplication:

```rust
fn display_validation(
    terms: &CuratedTermsFile,
    extensions: &[String],
) {
    let report = colophon_core::validate::validate_locations(
        terms,
        &terms.source_dir,
        extensions,
    );
    if !report.suggestions.is_empty() {
        eprintln!();
        eprintln!(
            "Validation: {} resolved, {} unresolved",
            report.resolved, report.unresolved
        );
        eprintln!("Suggested aliases for unresolved locations:");
        for s in &report.suggestions {
            eprintln!("  {} -> add alias \"{}\"", s.term, s.suggested_alias);
        }
    }
}
```

Call from both `cmd_curate` and `cmd_curate_incremental` (not in `--json` or `--dry-run` mode).

- [ ] **Step 4: Pass extensions through to validation**

The `cmd_curate` function receives `config: &Config` which has `config.source.extensions`. Pass this to the helper. In `cmd_curate_incremental`, the config is `CurateConfig` which doesn't have extensions — thread the full `Config` or just extensions through. Simplest: change `cmd_curate_incremental` signature to accept `source_extensions: &[String]` and pass from caller.

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p colophon`
Expected: Compiles clean.

- [ ] **Step 6: Commit**

```bash
git add crates/colophon/src/commands/curate.rs
git commit -m "feat(cli): show alias suggestions after curate"
```

---

### Task 5: Clippy + final verification

- [ ] **Step 1: Run clippy on full workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: No warnings.

- [ ] **Step 2: Run full test suite**

Run: `cargo nextest run`
Expected: All tests pass (previous count + new validate tests).

- [ ] **Step 3: Fix any issues, commit if needed**
