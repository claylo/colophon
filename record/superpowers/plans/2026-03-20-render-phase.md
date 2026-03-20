# Phase 3: Render Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `colophon render` command that reads curated terms and produces annotated source files with in-dexter index markers plus an optional Typst glossary.

**Architecture:** `Renderer` trait in `colophon-core` with `annotate()` and `glossary()` methods. `TypstRenderer` is the first implementation. Orchestrator in `render::run()` handles file I/O, term-to-annotation conversion, and parent chain walking. Thin CLI command in `colophon` wires args to the orchestrator.

**Tech Stack:** Rust, typst-syntax (already a dep), in-dexter marker format, serde_yaml for terms file I/O, thiserror for `RenderError`, tabled + indicatif for CLI output.

**Spec:** `record/superpowers/specs/2026-03-20-render-phase-design.md`

---

### Task 1: RenderError type

**Files:**
- Modify: `crates/colophon-core/src/error.rs`
- Modify: `crates/colophon-core/src/lib.rs` (re-export)

- [ ] **Step 1: Add `RenderError` enum to `error.rs`**

```rust
/// Errors that can occur during rendering.
#[derive(Error, Debug)]
pub enum RenderError {
    /// Failed to read or write files.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to deserialize the curated terms file.
    #[error("failed to parse terms file: {0}")]
    ParseTerms(#[from] serde_yaml::Error),

    /// Cycle detected in parent chain.
    #[error("cycle in parent chain: {chain}")]
    ParentCycle {
        /// The cycle path (e.g., "A -> B -> A").
        chain: String,
    },

    /// No terms found in the terms file.
    #[error("no terms in {0}")]
    NoTerms(String),
}

/// Result type alias using [`RenderError`].
pub type RenderResult<T> = Result<T, RenderError>;
```

- [ ] **Step 2: Add re-exports to `lib.rs`**

Add `RenderError, RenderResult` to the `pub use error::` line.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p colophon-core`

- [ ] **Step 4: Commit**

```
feat(core): add RenderError type
```

---

### Task 2: Render trait and Annotation type

**Files:**
- Create: `crates/colophon-core/src/render/mod.rs`
- Modify: `crates/colophon-core/src/lib.rs` (add `pub mod render`)

- [ ] **Step 1: Create `render/mod.rs` with types and trait**

```rust
//! Render pipeline — annotate source files with index markers.

pub mod typst;

use crate::curate::terms::CuratedTermsFile;

/// A single index marker to place in a source file.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// Canonical term text.
    pub term: String,
    /// Parent chain from root to immediate parent (empty if top-level).
    pub parent_chain: Vec<String>,
    /// Whether this is a substantive discussion (bold page number).
    pub main: bool,
    /// Byte offset in the source where this marker should be inserted.
    pub byte_offset: usize,
}

/// Output statistics from a render run.
#[derive(Debug, Default, Serialize)]
pub struct RenderOutput {
    /// Number of source files that received annotations.
    pub files_annotated: usize,
    /// Total markers inserted across all files.
    pub markers_inserted: usize,
    /// How many of those markers were main (bold).
    pub markers_main: usize,
    /// Terms that could not be located in any source file.
    pub terms_not_found: usize,
    /// Number of terms in the glossary (if generated).
    pub glossary_terms: usize,
}

/// Format-specific rendering of index markers and glossary.
pub trait Renderer {
    /// Insert index markers into source file content.
    ///
    /// `annotations` are sorted by descending `byte_offset` so insertions
    /// don't shift subsequent offsets. Returns the annotated content.
    fn annotate(&self, source: &str, annotations: &[Annotation]) -> String;

    /// Emit a standalone glossary document from the curated terms.
    fn glossary(&self, terms: &CuratedTermsFile) -> String;
}
```

- [ ] **Step 2: Add `pub mod render` to `lib.rs`**

Add between `pub mod extract;` and the re-exports. Update the module doc comment to list the render module.

- [ ] **Step 3: Create empty `render/typst.rs`**

```rust
//! Typst renderer — in-dexter index markers and term list glossary.
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p colophon-core`

- [ ] **Step 5: Commit**

```
feat(core): add Renderer trait and Annotation type
```

---

### Task 3: Typst marker formatting

**Files:**
- Modify: `crates/colophon-core/src/render/typst.rs`

- [ ] **Step 1: Write failing tests for marker formatting**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Annotation;

    #[test]
    fn marker_top_level() {
        let r = TypstRenderer;
        assert_eq!(r.format_marker("OAuth", &[], false), "#index[OAuth]");
    }

    #[test]
    fn marker_top_level_main() {
        let r = TypstRenderer;
        assert_eq!(r.format_marker("OAuth", &[], true), "#index-main[OAuth]");
    }

    #[test]
    fn marker_child() {
        let r = TypstRenderer;
        assert_eq!(
            r.format_marker("OAuth", &["authentication".to_string()], false),
            r#"#index("authentication", "OAuth")"#
        );
    }

    #[test]
    fn marker_child_main() {
        let r = TypstRenderer;
        assert_eq!(
            r.format_marker("OAuth", &["authentication".to_string()], true),
            r#"#index-main("authentication", "OAuth")"#
        );
    }

    #[test]
    fn marker_grandchild() {
        let r = TypstRenderer;
        assert_eq!(
            r.format_marker(
                "OAuth",
                &["security".to_string(), "authentication".to_string()],
                false
            ),
            r#"#index("security", "authentication", "OAuth")"#
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core -E 'test(render::typst)'`
Expected: FAIL (no `TypstRenderer` or `format_marker`)

- [ ] **Step 3: Implement `TypstRenderer` with `format_marker`**

```rust
use crate::curate::terms::CuratedTermsFile;
use crate::render::Renderer;

/// Typst renderer using in-dexter index markers.
pub struct TypstRenderer;

impl TypstRenderer {
    /// Format a single index marker call.
    pub fn format_marker(&self, term: &str, parent_chain: &[String], main: bool) -> String {
        let func = if main { "#index-main" } else { "#index" };
        if parent_chain.is_empty() {
            format!("{func}[{term}]")
        } else {
            let args: Vec<String> = parent_chain
                .iter()
                .map(|p| format!("\"{p}\""))
                .chain(std::iter::once(format!("\"{term}\"")))
                .collect();
            format!("{func}({})", args.join(", "))
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core -E 'test(render::typst)'`
Expected: PASS

- [ ] **Step 5: Commit**

```
feat(core): TypstRenderer marker formatting
```

---

### Task 4: Typst `annotate()` implementation

**Files:**
- Modify: `crates/colophon-core/src/render/typst.rs`

- [ ] **Step 1: Write failing tests for `annotate()`**

```rust
    #[test]
    fn annotate_single_marker() {
        let r = TypstRenderer;
        let source = "OAuth provides delegated authorization.";
        let annotations = vec![Annotation {
            term: "OAuth".to_string(),
            parent_chain: Vec::new(),
            main: false,
            byte_offset: 5, // right after "OAuth"
        }];
        let result = r.annotate(source, &annotations);
        assert_eq!(result, "OAuth#index[OAuth] provides delegated authorization.");
    }

    #[test]
    fn annotate_multiple_markers_descending() {
        let r = TypstRenderer;
        let source = "OAuth uses TLS for security.";
        let annotations = vec![
            Annotation {
                term: "TLS".to_string(),
                parent_chain: Vec::new(),
                main: false,
                byte_offset: 14, // after "TLS"
            },
            Annotation {
                term: "OAuth".to_string(),
                parent_chain: Vec::new(),
                main: true,
                byte_offset: 5, // after "OAuth"
            },
        ];
        // annotations must be sorted descending by byte_offset
        let mut sorted = annotations;
        sorted.sort_by(|a, b| b.byte_offset.cmp(&a.byte_offset));
        let result = r.annotate(source, &sorted);
        assert_eq!(
            result,
            "OAuth#index-main[OAuth] uses TLS#index[TLS] for security."
        );
    }

    #[test]
    fn annotate_with_parent_chain() {
        let r = TypstRenderer;
        let source = "OAuth is important.";
        let annotations = vec![Annotation {
            term: "OAuth".to_string(),
            parent_chain: vec!["authentication".to_string()],
            main: false,
            byte_offset: 5,
        }];
        let result = r.annotate(source, &annotations);
        assert!(result.contains(r#"#index("authentication", "OAuth")"#));
    }

    #[test]
    fn annotate_prepends_import() {
        let r = TypstRenderer;
        let source = "OAuth is important.";
        let annotations = vec![Annotation {
            term: "OAuth".to_string(),
            parent_chain: Vec::new(),
            main: false,
            byte_offset: 5,
        }];
        let result = r.annotate(source, &annotations);
        assert!(result.starts_with("#import \"@preview/in-dexter:"));
    }

    #[test]
    fn annotate_skips_import_if_present() {
        let r = TypstRenderer;
        let source = "#import \"@preview/in-dexter:0.7.2\": *\n\nOAuth is important.";
        let annotations = vec![Annotation {
            term: "OAuth".to_string(),
            parent_chain: Vec::new(),
            main: false,
            byte_offset: source.find("OAuth").unwrap() + 5,
        }];
        let result = r.annotate(source, &annotations);
        // Should not have duplicate import
        assert_eq!(result.matches("in-dexter").count(), 1);
    }

    #[test]
    fn annotate_empty_annotations() {
        let r = TypstRenderer;
        let source = "No terms here.";
        let result = r.annotate(source, &[]);
        assert_eq!(result, source);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core -E 'test(render::typst)'`

- [ ] **Step 3: Implement `annotate()` on `TypstRenderer`**

The implementation inserts markers at descending byte offsets (so earlier offsets stay valid), then prepends the in-dexter import if not already present and at least one marker was inserted.

```rust
/// In-dexter import line prepended to annotated files.
const IN_DEXTER_IMPORT: &str = "#import \"@preview/in-dexter:0.7.2\": *\n";

impl Renderer for TypstRenderer {
    fn annotate(&self, source: &str, annotations: &[Annotation]) -> String {
        if annotations.is_empty() {
            return source.to_string();
        }

        let mut result = source.to_string();

        // Insert markers at descending byte offsets.
        for ann in annotations {
            let marker = self.format_marker(&ann.term, &ann.parent_chain, ann.main);
            if ann.byte_offset <= result.len() {
                result.insert_str(ann.byte_offset, &marker);
            }
        }

        // Prepend import if not already present.
        if !result.contains("in-dexter") {
            result.insert_str(0, IN_DEXTER_IMPORT);
        }

        result
    }

    fn glossary(&self, _terms: &CuratedTermsFile) -> String {
        todo!() // Task 5
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core -E 'test(render::typst)'`
Expected: PASS

- [ ] **Step 5: Commit**

```
feat(core): TypstRenderer annotate() with import preamble
```

---

### Task 5: Typst `glossary()` implementation

**Files:**
- Modify: `crates/colophon-core/src/render/typst.rs`

- [ ] **Step 1: Write failing tests for `glossary()`**

```rust
    use crate::curate::terms::{CuratedTerm, CuratedTermsFile, TermLocation};

    fn sample_terms() -> CuratedTermsFile {
        CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 3,
            terms: vec![
                CuratedTerm {
                    term: "API key".to_string(),
                    definition: "A credential for authenticating API requests.".to_string(),
                    parent: None,
                    aliases: Vec::new(),
                    see_also: Vec::new(),
                    children: Vec::new(),
                    locations: Vec::new(),
                },
                CuratedTerm {
                    term: "authentication".to_string(),
                    definition: "The process of verifying identity.".to_string(),
                    parent: None,
                    aliases: Vec::new(),
                    see_also: Vec::new(),
                    children: vec!["OAuth".to_string()],
                    locations: Vec::new(),
                },
                CuratedTerm {
                    term: "OAuth".to_string(),
                    definition: "An open standard for token-based authorization.".to_string(),
                    parent: Some("authentication".to_string()),
                    aliases: vec!["OAuth 2.0".to_string()],
                    see_also: vec!["API key".to_string()],
                    children: Vec::new(),
                    locations: Vec::new(),
                },
                CuratedTerm {
                    term: "TLS".to_string(),
                    definition: "Transport Layer Security protocol.".to_string(),
                    parent: None,
                    aliases: Vec::new(),
                    see_also: Vec::new(),
                    children: Vec::new(),
                    locations: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn glossary_contains_all_terms() {
        let r = TypstRenderer;
        let g = r.glossary(&sample_terms());
        assert!(g.contains("/ API key:"));
        assert!(g.contains("/ authentication:"));
        assert!(g.contains("/ OAuth:"));
        assert!(g.contains("/ TLS:"));
    }

    #[test]
    fn glossary_children_indented() {
        let r = TypstRenderer;
        let g = r.glossary(&sample_terms());
        // OAuth is a child of authentication — should be indented
        assert!(g.contains("  / OAuth:"));
    }

    #[test]
    fn glossary_see_also_rendered() {
        let r = TypstRenderer;
        let g = r.glossary(&sample_terms());
        assert!(g.contains("_See also: API key_"));
    }

    #[test]
    fn glossary_has_header_comment() {
        let r = TypstRenderer;
        let g = r.glossary(&sample_terms());
        assert!(g.starts_with("// Generated by colophon"));
    }

    #[test]
    fn glossary_alphabetical_order() {
        let r = TypstRenderer;
        let g = r.glossary(&sample_terms());
        let api_pos = g.find("/ API key:").unwrap();
        let auth_pos = g.find("/ authentication:").unwrap();
        let tls_pos = g.find("/ TLS:").unwrap();
        assert!(api_pos < auth_pos, "API key should come before authentication");
        assert!(auth_pos < tls_pos, "authentication should come before TLS");
    }

    #[test]
    fn glossary_empty_terms() {
        let r = TypstRenderer;
        let terms = CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: ".".to_string(),
            document_count: 0,
            terms: Vec::new(),
        };
        let g = r.glossary(&terms);
        assert!(g.contains("// Generated by colophon"));
        assert!(!g.contains("/ "));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core -E 'test(render::typst::tests::glossary)'`

- [ ] **Step 3: Implement `glossary()`**

Emit top-level terms first (no parent), then recursively emit children indented. Alphabetical sort within each level.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core -E 'test(render::typst)'`
Expected: PASS

- [ ] **Step 5: Commit**

```
feat(core): TypstRenderer glossary() with hierarchy and see_also
```

---

### Task 6: Parent chain walker and byte-offset search

**Files:**
- Modify: `crates/colophon-core/src/render/mod.rs`

- [ ] **Step 1: Write failing tests for parent chain and offset search**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::curate::terms::{CuratedTerm, CuratedTermsFile, TermLocation};

    fn term(name: &str, parent: Option<&str>) -> CuratedTerm {
        CuratedTerm {
            term: name.to_string(),
            definition: format!("{name} definition."),
            parent: parent.map(String::from),
            aliases: Vec::new(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: Vec::new(),
        }
    }

    #[test]
    fn parent_chain_top_level() {
        let terms = vec![term("OAuth", None)];
        let chain = build_parent_chain("OAuth", &terms);
        assert!(chain.is_empty());
    }

    #[test]
    fn parent_chain_one_level() {
        let terms = vec![term("OAuth", Some("authentication")), term("authentication", None)];
        let chain = build_parent_chain("OAuth", &terms);
        assert_eq!(chain, vec!["authentication"]);
    }

    #[test]
    fn parent_chain_two_levels() {
        let terms = vec![
            term("OAuth", Some("authentication")),
            term("authentication", Some("security")),
            term("security", None),
        ];
        let chain = build_parent_chain("OAuth", &terms);
        assert_eq!(chain, vec!["security", "authentication"]);
    }

    #[test]
    fn parent_chain_cycle_detected() {
        let terms = vec![
            term("A", Some("B")),
            term("B", Some("A")),
        ];
        let chain = build_parent_chain("A", &terms);
        // Should not loop forever; returns partial chain or empty
        assert!(chain.len() <= 2);
    }

    #[test]
    fn parent_chain_dangling_parent() {
        let terms = vec![term("OAuth", Some("nonexistent"))];
        let chain = build_parent_chain("OAuth", &terms);
        // Dangling parent — chain stops at the known part
        assert_eq!(chain, vec!["nonexistent"]);
    }

    #[test]
    fn find_term_offset_basic() {
        let text = "OAuth provides authorization.";
        let offset = find_term_offset(text, "OAuth");
        assert_eq!(offset, Some(5)); // right after "OAuth"
    }

    #[test]
    fn find_term_offset_case_insensitive() {
        let text = "The oauth protocol is common.";
        let offset = find_term_offset(text, "OAuth");
        assert_eq!(offset, Some(9)); // right after "oauth"
    }

    #[test]
    fn find_term_offset_not_found() {
        let text = "Nothing relevant here.";
        let offset = find_term_offset(text, "OAuth");
        assert_eq!(offset, None);
    }

    #[test]
    fn find_term_offset_multibyte() {
        let text = "Use → OAuth for auth.";
        let offset = find_term_offset(text, "OAuth");
        assert!(offset.is_some());
        let o = offset.unwrap();
        assert_eq!(&text[o - 5..o], "OAuth");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p colophon-core -E 'test(render::tests)'`

- [ ] **Step 3: Implement `build_parent_chain()` and `find_term_offset()`**

`build_parent_chain`: walk parent pointers, collect into Vec, detect cycles via a visited set, reverse to root-first order.

`find_term_offset`: case-insensitive byte-offset search. Find the first occurrence of the term text, return the byte offset immediately after the match.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core -E 'test(render::tests)'`
Expected: PASS

- [ ] **Step 5: Commit**

```
feat(core): parent chain walker and byte-offset term search
```

---

### Task 7: Render orchestrator (`run()`)

**Files:**
- Modify: `crates/colophon-core/src/render/mod.rs`

- [ ] **Step 1: Write failing test for orchestrator**

```rust
    #[test]
    fn run_annotates_source_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("auth.typ"),
            "OAuth provides delegated authorization.\n",
        ).unwrap();

        let output_dir = tmp.path().join("out");

        let terms = CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms: vec![CuratedTerm {
                term: "OAuth".to_string(),
                definition: "Auth standard.".to_string(),
                parent: None,
                aliases: Vec::new(),
                see_also: Vec::new(),
                children: Vec::new(),
                locations: vec![TermLocation {
                    file: "auth.typ".to_string(),
                    main: true,
                    context: "OAuth provides".to_string(),
                }],
            }],
        };

        let renderer = super::typst::TypstRenderer;
        let result = run_with_renderer(
            &terms,
            source_dir.to_str().unwrap(),
            &["typ".to_string()],
            output_dir.to_str().unwrap(),
            false,
            &renderer,
        );
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.files_annotated, 1);
        assert_eq!(output.markers_inserted, 1);
        assert_eq!(output.markers_main, 1);

        // Verify output file exists and contains marker
        let annotated = std::fs::read_to_string(output_dir.join("auth.typ")).unwrap();
        assert!(annotated.contains("#index-main[OAuth]"));
        assert!(annotated.contains("in-dexter"));
    }

    #[test]
    fn run_alias_generates_canonical_annotation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        // Source mentions "OAuth 2.0" (an alias), not "OAuth" (canonical)
        std::fs::write(
            source_dir.join("api.typ"),
            "The OAuth 2.0 protocol is widely used.\n",
        ).unwrap();

        let output_dir = tmp.path().join("out");

        let terms = CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms: vec![CuratedTerm {
                term: "OAuth".to_string(),
                definition: "Auth standard.".to_string(),
                parent: None,
                aliases: vec!["OAuth 2.0".to_string()],
                see_also: Vec::new(),
                children: Vec::new(),
                locations: vec![TermLocation {
                    file: "api.typ".to_string(),
                    main: false,
                    context: "OAuth 2.0 protocol".to_string(),
                }],
            }],
        };

        let renderer = super::typst::TypstRenderer;
        let result = run_with_renderer(
            &terms, source_dir.to_str().unwrap(), &["typ".to_string()],
            output_dir.to_str().unwrap(), false, &renderer,
        ).unwrap();
        assert_eq!(result.markers_inserted, 1);

        let annotated = std::fs::read_to_string(output_dir.join("api.typ")).unwrap();
        // Marker uses canonical term name, not the alias
        assert!(annotated.contains("#index[OAuth]"));
    }

    #[test]
    fn run_one_marker_per_term_per_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        // "OAuth" appears three times in the same file
        std::fs::write(
            source_dir.join("auth.typ"),
            "OAuth is great. OAuth is flexible. OAuth is everywhere.\n",
        ).unwrap();

        let output_dir = tmp.path().join("out");

        let terms = CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms: vec![CuratedTerm {
                term: "OAuth".to_string(),
                definition: "Auth.".to_string(),
                parent: None,
                aliases: Vec::new(),
                see_also: Vec::new(),
                children: Vec::new(),
                locations: vec![TermLocation {
                    file: "auth.typ".to_string(),
                    main: false,
                    context: String::new(),
                }],
            }],
        };

        let renderer = super::typst::TypstRenderer;
        let result = run_with_renderer(
            &terms, source_dir.to_str().unwrap(), &["typ".to_string()],
            output_dir.to_str().unwrap(), false, &renderer,
        ).unwrap();
        assert_eq!(result.markers_inserted, 1);

        let annotated = std::fs::read_to_string(output_dir.join("auth.typ")).unwrap();
        assert_eq!(annotated.matches("#index[OAuth]").count(), 1);
    }

    #[test]
    fn run_terms_not_found_counted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("empty.typ"),
            "Nothing relevant here.\n",
        ).unwrap();

        let output_dir = tmp.path().join("out");

        let terms = CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms: vec![CuratedTerm {
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
            }],
        };

        let renderer = super::typst::TypstRenderer;
        let result = run_with_renderer(
            &terms, source_dir.to_str().unwrap(), &["typ".to_string()],
            output_dir.to_str().unwrap(), false, &renderer,
        ).unwrap();
        assert_eq!(result.markers_inserted, 0);
        assert_eq!(result.terms_not_found, 1);
    }

    #[test]
    fn run_glossary_written_to_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(source_dir.join("auth.typ"), "OAuth is great.\n").unwrap();

        let output_dir = tmp.path().join("out");

        let terms = CuratedTermsFile {
            version: 1,
            generated: "2026-03-20T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            terms: vec![CuratedTerm {
                term: "OAuth".to_string(),
                definition: "Auth standard.".to_string(),
                parent: None,
                aliases: Vec::new(),
                see_also: Vec::new(),
                children: Vec::new(),
                locations: vec![TermLocation {
                    file: "auth.typ".to_string(),
                    main: false,
                    context: String::new(),
                }],
            }],
        };

        let renderer = super::typst::TypstRenderer;
        let result = run_with_renderer(
            &terms, source_dir.to_str().unwrap(), &["typ".to_string()],
            output_dir.to_str().unwrap(), true, &renderer,
        ).unwrap();
        assert_eq!(result.glossary_terms, 1);

        let glossary_path = output_dir.join("glossary.typ");
        assert!(glossary_path.exists(), "glossary.typ should be written");
        let glossary = std::fs::read_to_string(glossary_path).unwrap();
        assert!(glossary.contains("/ OAuth:"));
    }
```

- [ ] **Step 2: Implement `run_with_renderer()`**

The orchestrator:
1. Read source files from `source_dir` matching `extensions`
2. For each file, collect matching term locations
3. For each term-location, search for the canonical term AND each alias
   in the source text. First match wins (one marker per term per file).
   Alias matches use the canonical term name in the annotation.
4. Build parent chain for each matched term
5. Sort annotations descending by byte offset
6. Call `renderer.annotate()` and write to output dir, preserving directory structure
7. If `glossary` is true, call `renderer.glossary()` and write `glossary.typ`
8. Return `RenderOutput` stats (including `terms_not_found` count)

Also add a public `run()` that dispatches to the right renderer via a `RenderFormat` enum (currently just `Typst`).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core -E 'test(render)'`
Expected: PASS

- [ ] **Step 4: Commit**

```
feat(core): render orchestrator with file I/O
```

---

### Task 8: CLI command

**Files:**
- Create: `crates/colophon/src/commands/render.rs`
- Modify: `crates/colophon/src/commands/mod.rs`
- Modify: `crates/colophon/src/lib.rs` (add `Render` variant to `Commands`)
- Modify: `crates/colophon/src/main.rs` (wire up match arm)

- [ ] **Step 1: Create `commands/render.rs`**

```rust
//! Render command — annotate source files with index markers.

use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use clap::Args;
use colophon_core::config::Config;
use colophon_core::curate::terms::CuratedTermsFile;
use colophon_core::render;
use tabled::{builder::Builder, settings::Style};
use tracing::{debug, instrument};

#[derive(Args, Debug, Default)]
pub struct RenderArgs {
    /// Path to curated terms file
    #[arg(long, default_value = "colophon-terms.yaml")]
    pub terms: String,

    /// Output format
    #[arg(long, default_value = "typst")]
    pub format: String,

    /// Output directory for annotated files
    #[arg(short, long, default_value = ".")]
    pub output_dir: String,

    /// Also emit a standalone glossary document
    #[arg(long)]
    pub glossary: bool,
}

#[instrument(name = "cmd_render", skip_all)]
pub fn cmd_render(args: RenderArgs, json: bool, config: &Config) -> anyhow::Result<()> {
    debug!("executing render command");
    // ... read terms, call render::run, display summary table
}
```

- [ ] **Step 2: Wire into CLI**

Add `pub mod render;` to `commands/mod.rs`.
Add `Render(commands::render::RenderArgs)` variant to `Commands` enum in `lib.rs`.
Add match arm in `main.rs`.

- [ ] **Step 3: Implement `cmd_render` body**

Read terms file, call `render::run()`, display summary table with tabled, handle `--json` output.

- [ ] **Step 4: Verify help output**

Run: `cargo run -- render --help`

- [ ] **Step 5: Run full test suite**

Run: `cargo nextest run -p colophon-core -p colophon --test-threads=4`
Run: `cargo clippy --all-targets --all-features -p colophon-core -p colophon`

- [ ] **Step 6: Commit**

```
feat(cli): add colophon render command
```

---

### Task 9: Full clippy + test validation

- [ ] **Step 1: Run `just clippy`**

Expect: zero warnings on new code (pre-existing warnings acceptable).

- [ ] **Step 2: Run `cargo nextest run`**

Expect: all tests pass.

- [ ] **Step 3: Run `cargo test --doc`**

Expect: doc tests pass.

- [ ] **Step 4: Manual smoke test**

If `colophon-terms.yaml` exists in the repo:
```
cargo run -- render --terms colophon-terms.yaml --output-dir /tmp/colophon-render --glossary
ls /tmp/colophon-render/
```

Verify annotated files contain `#index` markers and `glossary.typ` is a valid term list.
