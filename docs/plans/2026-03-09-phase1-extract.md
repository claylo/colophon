# Phase 1: Extract Pipeline — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the `colophon extract` command that scans a directory of Markdown files, extracts keyword candidates using YAKE + TF-IDF, and writes a `colophon-candidates.yaml` file.

**Architecture:** Fat core / thin CLI. All extraction logic lives in `colophon-core` under an `extract` module tree. The CLI crate adds an `Extract` variant to `Commands` and a thin `commands/extract.rs` that calls into core. The pipeline: walk directory -> parse markdown -> strip non-prose -> run YAKE per-doc -> run TF-IDF across corpus -> merge/score -> serialize to YAML.

**Tech Stack:** `pulldown-cmark` 0.13 (markdown parsing), `yake-rust` 1.0 (YAKE keyword extraction, MIT), hand-rolled TF-IDF (~50 lines), `walkdir` 2 (directory traversal), `serde_yaml` 0.9 (YAML output).

**Test runner:** `cargo nextest run` (never `cargo test`). Doc tests: `cargo test --doc`.

**Lint/check:** `just check` runs fmt + clippy + deny + test + doc-test.

---

### Task 1: Add Phase 1 Dependencies to colophon-core

**Files:**
- Modify: `crates/colophon-core/Cargo.toml`

**Step 1: Add dependencies**

Add to `[dependencies]` in `crates/colophon-core/Cargo.toml`:

```toml
# Markdown parsing
pulldown-cmark = "0.13"

# Keyword extraction (YAKE — TF-IDF is hand-rolled)
yake-rust = { version = "1.0", default-features = false, features = ["en"] }

# YAML serialization for candidates output
serde_yaml = "0.9"

# Directory traversal
walkdir = "2"
```

**Step 2: Verify it compiles**

Run: `cargo build -p colophon-core`
Expected: success (no code uses the deps yet, just checking resolution)

**Step 3: Run deny check**

Run: `just deny`
Expected: pass. If any license issues surface, add the license to `deny.toml`'s allow list (all these crates are MIT/Apache-2.0).

**Step 4: Commit**

```
feat(core): add extraction pipeline dependencies

pulldown-cmark, keyword_extraction (YAKE + TF-IDF),
stop-words, serde_yaml, walkdir
```

---

### Task 2: Extend Config with Source and Extract Sections

**Files:**
- Modify: `crates/colophon-core/src/config.rs`
- Test: unit tests in same file

**Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `config.rs`:

```rust
#[test]
fn test_source_and_extract_config_from_toml() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
log_level = "info"

[source]
dir = "docs/"
extensions = ["md", "mdx"]
exclude = ["README.md"]

[extract]
ngram_range = [1, 4]
min_score = 0.05
max_candidates = 300
"#,
    )
    .unwrap();

    let config_path = Utf8PathBuf::try_from(config_path).unwrap();
    let (config, _) = ConfigLoader::new()
        .with_user_config(false)
        .with_file(&config_path)
        .load()
        .unwrap();

    assert_eq!(config.source.dir, "docs/");
    assert_eq!(config.source.extensions, vec!["md", "mdx"]);
    assert_eq!(config.source.exclude, vec!["README.md"]);
    assert_eq!(config.extract.ngram_range, [1, 4]);
    assert_eq!(config.extract.min_score, 0.05);
    assert_eq!(config.extract.max_candidates, 300);
}

#[test]
fn test_source_and_extract_defaults() {
    let config = Config::default();
    assert_eq!(config.source.dir, ".");
    assert_eq!(config.source.extensions, vec!["md"]);
    assert!(config.source.exclude.is_empty());
    assert_eq!(config.extract.ngram_range, [1, 3]);
    assert_eq!(config.extract.min_score, 0.1);
    assert_eq!(config.extract.max_candidates, 500);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo nextest run -p colophon-core test_source_and_extract`
Expected: FAIL — `Config` has no `source` or `extract` fields

**Step 3: Implement the config structs**

Add these structs above the existing `Config` struct in `config.rs`:

```rust
/// Configuration for source file discovery.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct SourceConfig {
    /// Root directory to scan for content files.
    pub dir: String,
    /// File extensions to include (without dots).
    pub extensions: Vec<String>,
    /// File names to exclude from processing.
    pub exclude: Vec<String>,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            dir: ".".to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        }
    }
}

/// Configuration for the keyword extraction pipeline.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ExtractConfig {
    /// N-gram range for keyword extraction [min, max].
    pub ngram_range: [usize; 2],
    /// Minimum relevance score threshold (0.0 to 1.0).
    pub min_score: f64,
    /// Maximum number of candidates to emit.
    pub max_candidates: usize,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            ngram_range: [1, 3],
            min_score: 0.1,
            max_candidates: 500,
        }
    }
}
```

Add the fields to `Config`:

```rust
pub struct Config {
    pub log_level: LogLevel,
    pub log_dir: Option<Utf8PathBuf>,
    /// Source file discovery settings.
    pub source: SourceConfig,
    /// Keyword extraction pipeline settings.
    pub extract: ExtractConfig,
}
```

**Step 4: Run test to verify it passes**

Run: `cargo nextest run -p colophon-core test_source_and_extract`
Expected: PASS

**Step 5: Run full check**

Run: `cargo nextest run -p colophon-core`
Expected: all existing tests still pass

**Step 6: Commit**

```
feat(core): add source and extract config sections
```

---

### Task 3: Create extract module skeleton with candidate types

**Files:**
- Create: `crates/colophon-core/src/extract/mod.rs`
- Create: `crates/colophon-core/src/extract/candidates.rs`
- Modify: `crates/colophon-core/src/lib.rs`
- Modify: `crates/colophon-core/src/error.rs`

**Step 1: Write the failing test**

Create `crates/colophon-core/src/extract/candidates.rs` with test at bottom:

```rust
//! Candidate types for the extraction pipeline.

use serde::{Deserialize, Serialize};

/// A single keyword candidate extracted from the corpus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    /// The keyword or phrase as it appears in text.
    pub term: String,
    /// Relevance score (lower = more relevant for YAKE, higher = more for TF-IDF).
    /// Normalized to 0.0..1.0 where higher is better.
    pub score: f64,
    /// Which documents this term was found in.
    pub locations: Vec<CandidateLocation>,
}

/// Where a candidate term was found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateLocation {
    /// Relative path to the source file.
    pub file: String,
    /// A short snippet of surrounding context.
    pub context: String,
}

/// The full candidates output file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidatesFile {
    /// Schema version.
    pub version: u32,
    /// When this file was generated (ISO 8601).
    pub generated: String,
    /// Source directory that was scanned.
    pub source_dir: String,
    /// Number of documents processed.
    pub document_count: usize,
    /// Extracted candidates, sorted by score descending.
    pub candidates: Vec<Candidate>,
}

impl CandidatesFile {
    /// Serialize to YAML string.
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Deserialize from YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidates_file_roundtrip_yaml() {
        let file = CandidatesFile {
            version: 1,
            generated: "2026-03-09T12:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 3,
            candidates: vec![
                Candidate {
                    term: "OAuth".to_string(),
                    score: 0.95,
                    locations: vec![CandidateLocation {
                        file: "03-auth.md".to_string(),
                        context: "OAuth provides delegated authorization".to_string(),
                    }],
                },
                Candidate {
                    term: "authentication".to_string(),
                    score: 0.82,
                    locations: vec![
                        CandidateLocation {
                            file: "03-auth.md".to_string(),
                            context: "Authentication verifies identity".to_string(),
                        },
                        CandidateLocation {
                            file: "07-api.md".to_string(),
                            context: "API authentication is required".to_string(),
                        },
                    ],
                },
            ],
        };

        let yaml = file.to_yaml().expect("serialization should succeed");
        let parsed = CandidatesFile::from_yaml(&yaml).expect("deserialization should succeed");
        assert_eq!(file, parsed);
    }

    #[test]
    fn test_empty_candidates_file() {
        let file = CandidatesFile {
            version: 1,
            generated: "2026-03-09T12:00:00Z".to_string(),
            source_dir: ".".to_string(),
            document_count: 0,
            candidates: Vec::new(),
        };

        let yaml = file.to_yaml().unwrap();
        assert!(yaml.contains("version: 1"));
        assert!(yaml.contains("candidates: []"));
    }
}
```

**Step 2: Create the module file**

Create `crates/colophon-core/src/extract/mod.rs`:

```rust
//! Extraction pipeline — scan markdown files and extract keyword candidates.

pub mod candidates;
```

**Step 3: Wire into lib.rs**

Add to `crates/colophon-core/src/lib.rs`:

```rust
pub mod extract;
```

**Step 4: Add extraction error variants**

Add to `error.rs`:

```rust
/// Errors that can occur during extraction.
#[derive(Error, Debug)]
pub enum ExtractError {
    /// Failed to read a source file.
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },

    /// Failed to walk the source directory.
    #[error("failed to walk directory: {0}")]
    WalkDir(#[from] walkdir::Error),

    /// Failed to serialize candidates.
    #[error("failed to serialize candidates: {0}")]
    Serialize(#[from] serde_yaml::Error),

    /// No documents found in source directory.
    #[error("no documents found in {0}")]
    NoDocuments(String),
}

/// Result type alias using [`ExtractError`].
pub type ExtractResult<T> = Result<T, ExtractError>;
```

Add to `lib.rs` exports:

```rust
pub use error::{ExtractError, ExtractResult};
```

**Step 5: Run tests**

Run: `cargo nextest run -p colophon-core`
Expected: PASS (including the two new candidates tests)

**Step 6: Commit**

```
feat(core): add extract module skeleton with candidate types
```

---

### Task 4: Implement Markdown text extraction

**Files:**
- Create: `crates/colophon-core/src/extract/markdown.rs`
- Modify: `crates/colophon-core/src/extract/mod.rs`

**Step 1: Write the failing tests**

Create `crates/colophon-core/src/extract/markdown.rs`:

```rust
//! Markdown parsing and prose text extraction.
//!
//! Strips frontmatter, code blocks, URLs, and image refs while
//! preserving alt text and prose content for keyword analysis.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Extract prose text from a Markdown document.
///
/// Strips: YAML frontmatter, code blocks (inline and fenced),
/// raw HTML, URLs, image paths. Keeps: headings, paragraphs,
/// list items, alt text from images, table cells.
pub fn extract_prose(markdown: &str) -> String {
    let content = strip_frontmatter(markdown);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(&content, options);
    let mut prose = String::new();
    let mut in_code_block = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Text(text) if !in_code_block => {
                if !prose.is_empty() && !prose.ends_with('\n') && !prose.ends_with(' ') {
                    prose.push(' ');
                }
                prose.push_str(&text);
            }
            Event::SoftBreak | Event::HardBreak => prose.push('\n'),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                prose.push('\n');
            }
            _ => {}
        }
    }

    prose.trim().to_string()
}

/// Strip YAML frontmatter (lines between opening and closing `---`).
fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }

    // Find the closing ---
    if let Some(end) = trimmed[3..].find("\n---") {
        // Skip past the closing --- and its newline
        let after = &trimmed[3 + end + 4..];
        after.to_string()
    } else {
        // No closing ---, treat as normal content
        content.to_string()
    }
}

/// Extract a context snippet around a term occurrence in text.
///
/// Returns up to `window` characters on each side of the first occurrence.
pub fn extract_context(text: &str, term: &str, window: usize) -> Option<String> {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();
    let pos = lower_text.find(&lower_term)?;

    let start = pos.saturating_sub(window);
    let end = (pos + term.len() + window).min(text.len());

    // Snap to word boundaries
    let start = text[..start]
        .rfind(char::is_whitespace)
        .map_or(start, |i| i + 1);
    let end = text[end..]
        .find(char::is_whitespace)
        .map_or(end, |i| end + i);

    Some(text[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_plain_prose() {
        let md = "# Hello World\n\nThis is a paragraph.\n\nAnother paragraph here.\n";
        let prose = extract_prose(md);
        assert!(prose.contains("Hello World"));
        assert!(prose.contains("This is a paragraph."));
        assert!(prose.contains("Another paragraph here."));
    }

    #[test]
    fn test_strips_code_blocks() {
        let md = "Some text.\n\n```rust\nfn main() {}\n```\n\nMore text.\n";
        let prose = extract_prose(md);
        assert!(prose.contains("Some text."));
        assert!(prose.contains("More text."));
        assert!(!prose.contains("fn main"));
    }

    #[test]
    fn test_strips_inline_code() {
        let md = "Use `println!` for output.\n";
        let prose = extract_prose(md);
        // pulldown-cmark emits inline code as Code event, not Text
        assert!(prose.contains("Use"));
        assert!(prose.contains("for output."));
        assert!(!prose.contains("println!"));
    }

    #[test]
    fn test_strips_frontmatter() {
        let md = "---\ntitle: Test\ndate: 2026-01-01\n---\n\n# Real Content\n\nParagraph.\n";
        let prose = extract_prose(md);
        assert!(!prose.contains("title:"));
        assert!(!prose.contains("2026-01-01"));
        assert!(prose.contains("Real Content"));
        assert!(prose.contains("Paragraph."));
    }

    #[test]
    fn test_no_frontmatter() {
        let md = "# Just a heading\n\nNo frontmatter here.\n";
        let prose = extract_prose(md);
        assert!(prose.contains("Just a heading"));
        assert!(prose.contains("No frontmatter here."));
    }

    #[test]
    fn test_keeps_alt_text() {
        // pulldown-cmark emits alt text as Text inside Image tag
        let md = "![descriptive alt text](image.png)\n";
        let prose = extract_prose(md);
        assert!(prose.contains("descriptive alt text"));
    }

    #[test]
    fn test_keeps_list_items() {
        let md = "- First item\n- Second item\n- Third item\n";
        let prose = extract_prose(md);
        assert!(prose.contains("First item"));
        assert!(prose.contains("Second item"));
        assert!(prose.contains("Third item"));
    }

    #[test]
    fn test_extract_context() {
        let text = "The OAuth protocol provides delegated authorization for web applications.";
        let ctx = extract_context(text, "OAuth", 20).unwrap();
        assert!(ctx.contains("OAuth"));
        assert!(ctx.len() < text.len()); // Should be a snippet, not the full text
    }

    #[test]
    fn test_extract_context_not_found() {
        let text = "No matching term here.";
        assert!(extract_context(text, "OAuth", 20).is_none());
    }

    #[test]
    fn test_extract_context_case_insensitive() {
        let text = "The oauth protocol is widely used.";
        let ctx = extract_context(text, "OAuth", 10).unwrap();
        assert!(ctx.contains("oauth"));
    }

    #[test]
    fn test_empty_document() {
        assert_eq!(extract_prose(""), "");
    }

    #[test]
    fn test_only_frontmatter() {
        let md = "---\ntitle: Only metadata\n---\n";
        let prose = extract_prose(md);
        assert!(prose.is_empty() || prose.trim().is_empty());
    }
}
```

**Step 2: Wire into extract/mod.rs**

Add to `crates/colophon-core/src/extract/mod.rs`:

```rust
pub mod markdown;
```

**Step 3: Run tests to verify they pass**

Run: `cargo nextest run -p colophon-core extract::markdown`
Expected: PASS — the implementation is included with the tests in this task since `extract_prose` and `extract_context` are pure functions with no external dependencies to mock.

Note: Some tests may fail if pulldown-cmark's event model differs from expectations. If `test_strips_inline_code` or `test_keeps_alt_text` fail, adjust the `extract_prose` function to match pulldown-cmark's actual event emission. Run tests iteratively and fix.

**Step 4: Commit**

```
feat(core): implement markdown prose extraction
```

---

### Task 5: Implement keyword extraction (YAKE + TF-IDF)

**Files:**
- Create: `crates/colophon-core/src/extract/keywords.rs`
- Modify: `crates/colophon-core/src/extract/mod.rs`

**Step 1: Write the failing test**

Create `crates/colophon-core/src/extract/keywords.rs`:

```rust
//! Keyword extraction using YAKE (per-document) and TF-IDF (cross-corpus).

use keyword_extraction::tf_idf::{TfIdf, TfIdfParams};
use keyword_extraction::yake::{Yake, YakeParams};

/// A scored keyword from extraction.
#[derive(Debug, Clone)]
pub struct ScoredKeyword {
    /// The keyword or phrase.
    pub term: String,
    /// Normalized score (0.0..1.0, higher = more relevant).
    pub score: f64,
}

/// Extract keywords from a single document using YAKE.
///
/// YAKE scores are inverted (lower = more relevant), so we normalize
/// them to higher = more relevant for consistency.
pub fn extract_yake(
    text: &str,
    stop_words: &[String],
    max_keywords: usize,
) -> Vec<ScoredKeyword> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let yake = Yake::new(YakeParams::WithDefaults(text, stop_words));
    let scored = yake.get_ranked_keyword_scores(max_keywords);

    if scored.is_empty() {
        return Vec::new();
    }

    // YAKE: lower score = more relevant. Invert to higher = better.
    let max_score = scored
        .iter()
        .map(|(_, s)| *s as f64)
        .fold(f64::NEG_INFINITY, f64::max);

    scored
        .into_iter()
        .map(|(term, score)| {
            let normalized = if max_score > 0.0 {
                1.0 - (score as f64 / max_score)
            } else {
                0.0
            };
            ScoredKeyword {
                term,
                score: normalized.max(0.0),
            }
        })
        .collect()
}

/// Extract keywords across a corpus using TF-IDF.
///
/// Each document is a separate entry; TF-IDF finds terms that are
/// important *about* a document relative to the whole corpus.
pub fn extract_tfidf(
    documents: &[String],
    stop_words: &[String],
    max_keywords: usize,
) -> Vec<ScoredKeyword> {
    if documents.is_empty() {
        return Vec::new();
    }

    let doc_refs: Vec<&str> = documents.iter().map(String::as_str).collect();
    let params = TfIdfParams::UnprocessedDocuments(&doc_refs, stop_words, None);
    let tf_idf = TfIdf::new(params);
    let scored = tf_idf.get_ranked_word_scores(max_keywords);

    if scored.is_empty() {
        return Vec::new();
    }

    // TF-IDF: higher = more relevant. Normalize to 0..1.
    let max_score = scored
        .iter()
        .map(|(_, s)| *s as f64)
        .fold(f64::NEG_INFINITY, f64::max);

    scored
        .into_iter()
        .map(|(term, score)| {
            let normalized = if max_score > 0.0 {
                score as f64 / max_score
            } else {
                0.0
            };
            ScoredKeyword {
                term,
                score: normalized,
            }
        })
        .collect()
}

/// Get English stop words.
pub fn english_stop_words() -> Vec<String> {
    stop_words::get(stop_words::LANGUAGE::English)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_text() -> &'static str {
        "OAuth is an open standard for token-based authorization. \
         OAuth allows users to grant third-party access to their resources \
         without sharing credentials. The OAuth 2.0 framework is widely \
         adopted for API authorization and authentication workflows."
    }

    #[test]
    fn test_yake_extracts_keywords() {
        let stop_words = english_stop_words();
        let keywords = extract_yake(sample_text(), &stop_words, 10);
        assert!(!keywords.is_empty(), "should extract at least one keyword");

        // OAuth should be prominent
        let terms: Vec<&str> = keywords.iter().map(|k| k.term.as_str()).collect();
        let has_oauth = terms.iter().any(|t| t.to_lowercase().contains("oauth"));
        assert!(has_oauth, "should find 'OAuth' — got: {terms:?}");
    }

    #[test]
    fn test_yake_scores_normalized() {
        let stop_words = english_stop_words();
        let keywords = extract_yake(sample_text(), &stop_words, 10);
        for kw in &keywords {
            assert!(
                (0.0..=1.0).contains(&kw.score),
                "score should be 0..1, got {} for '{}'",
                kw.score,
                kw.term
            );
        }
    }

    #[test]
    fn test_yake_empty_text() {
        let stop_words = english_stop_words();
        let keywords = extract_yake("", &stop_words, 10);
        assert!(keywords.is_empty());
    }

    #[test]
    fn test_tfidf_extracts_keywords() {
        let docs = vec![
            "OAuth provides delegated authorization for APIs.".to_string(),
            "Authentication verifies user identity using passwords or tokens.".to_string(),
            "Rate limiting protects APIs from excessive requests.".to_string(),
        ];
        let stop_words = english_stop_words();
        let keywords = extract_tfidf(&docs, &stop_words, 10);
        assert!(!keywords.is_empty(), "should extract keywords from corpus");
    }

    #[test]
    fn test_tfidf_scores_normalized() {
        let docs = vec![
            "OAuth provides delegated authorization.".to_string(),
            "Authentication verifies identity.".to_string(),
        ];
        let stop_words = english_stop_words();
        let keywords = extract_tfidf(&docs, &stop_words, 10);
        for kw in &keywords {
            assert!(
                (0.0..=1.0).contains(&kw.score),
                "score should be 0..1, got {} for '{}'",
                kw.score,
                kw.term
            );
        }
    }

    #[test]
    fn test_tfidf_empty_corpus() {
        let stop_words = english_stop_words();
        let keywords = extract_tfidf(&[], &stop_words, 10);
        assert!(keywords.is_empty());
    }
}
```

**Step 2: Wire into extract/mod.rs**

Add to `crates/colophon-core/src/extract/mod.rs`:

```rust
pub mod keywords;
```

**Step 3: Run tests**

Run: `cargo nextest run -p colophon-core extract::keywords`
Expected: PASS

If `keyword_extraction` panics on edge cases (empty text, very short text), wrap calls with catch or guard. The empty-input guards should prevent most issues.

**Step 4: Commit**

```
feat(core): implement YAKE and TF-IDF keyword extraction
```

---

### Task 6: Implement the extraction pipeline orchestrator

**Files:**
- Modify: `crates/colophon-core/src/extract/mod.rs`

This is the main `run()` function that ties walking, parsing, keyword extraction, and candidate generation together.

**Step 1: Write the failing test**

Add to `extract/mod.rs` below the module declarations:

```rust
use std::collections::HashMap;
use std::path::Path;

use walkdir::WalkDir;

use crate::config::{ExtractConfig, SourceConfig};
use crate::error::ExtractResult;

use self::candidates::{Candidate, CandidateLocation, CandidatesFile};
use self::keywords::{ScoredKeyword, english_stop_words, extract_tfidf, extract_yake};
use self::markdown::{extract_context, extract_prose};

/// A parsed document ready for keyword extraction.
#[derive(Debug)]
struct Document {
    /// Relative path from the source directory.
    relative_path: String,
    /// Extracted prose text (no code, frontmatter, etc).
    prose: String,
}

/// Run the full extraction pipeline.
///
/// 1. Walk the source directory and collect matching files
/// 2. Parse each file and extract prose text
/// 3. Run YAKE on each document for per-doc keywords
/// 4. Run TF-IDF across the corpus for cross-doc keywords
/// 5. Merge and deduplicate candidates
/// 6. Return the candidates file
pub fn run(source: &SourceConfig, extract: &ExtractConfig) -> ExtractResult<CandidatesFile> {
    let source_dir = Path::new(&source.dir);
    tracing::info!(dir = %source.dir, "scanning for documents");

    // Step 1: Collect documents
    let documents = collect_documents(source_dir, &source.extensions, &source.exclude)?;
    if documents.is_empty() {
        return Err(crate::error::ExtractError::NoDocuments(source.dir.clone()));
    }
    tracing::info!(count = documents.len(), "found documents");

    // Step 2: Extract prose
    let documents: Vec<Document> = documents
        .into_iter()
        .map(|(path, content)| Document {
            relative_path: path,
            prose: extract_prose(&content),
        })
        .filter(|doc| !doc.prose.is_empty())
        .collect();

    if documents.is_empty() {
        return Err(crate::error::ExtractError::NoDocuments(source.dir.clone()));
    }

    let stop_words = english_stop_words();

    // Step 3: YAKE per-document
    let mut term_map: HashMap<String, CandidateBuilder> = HashMap::new();

    for doc in &documents {
        let yake_keywords = extract_yake(&doc.prose, &stop_words, extract.max_candidates);
        for kw in yake_keywords {
            if kw.score < extract.min_score {
                continue;
            }
            let key = kw.term.to_lowercase();
            let context = extract_context(&doc.prose, &kw.term, 40)
                .unwrap_or_default();
            let entry = term_map.entry(key).or_insert_with(|| CandidateBuilder {
                display_term: kw.term.clone(),
                best_score: 0.0,
                locations: Vec::new(),
            });
            entry.best_score = entry.best_score.max(kw.score);
            entry.locations.push(CandidateLocation {
                file: doc.relative_path.clone(),
                context,
            });
        }
    }

    // Step 4: TF-IDF across corpus
    let all_prose: Vec<String> = documents.iter().map(|d| d.prose.clone()).collect();
    let tfidf_keywords = extract_tfidf(&all_prose, &stop_words, extract.max_candidates);

    for kw in tfidf_keywords {
        if kw.score < extract.min_score {
            continue;
        }
        let key = kw.term.to_lowercase();
        let entry = term_map.entry(key).or_insert_with(|| {
            // Find which docs contain this term and grab context
            let locations: Vec<CandidateLocation> = documents
                .iter()
                .filter(|doc| doc.prose.to_lowercase().contains(&kw.term.to_lowercase()))
                .map(|doc| CandidateLocation {
                    file: doc.relative_path.clone(),
                    context: extract_context(&doc.prose, &kw.term, 40)
                        .unwrap_or_default(),
                })
                .collect();
            CandidateBuilder {
                display_term: kw.term.clone(),
                best_score: 0.0,
                locations,
            }
        });
        // Boost score if found by both algorithms
        entry.best_score = (entry.best_score + kw.score) / 2.0_f64.max(entry.best_score);
    }

    // Step 5: Build candidates, sort, cap
    let mut candidates: Vec<Candidate> = term_map
        .into_values()
        .map(|b| Candidate {
            term: b.display_term,
            score: b.best_score,
            locations: b.locations,
        })
        .collect();

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    candidates.truncate(extract.max_candidates);

    Ok(CandidatesFile {
        version: 1,
        generated: chrono_now(),
        source_dir: source.dir.clone(),
        document_count: documents.len(),
        candidates,
    })
}

#[derive(Debug)]
struct CandidateBuilder {
    display_term: String,
    best_score: f64,
    locations: Vec<CandidateLocation>,
}

/// Collect all matching files from the source directory.
fn collect_documents(
    dir: &Path,
    extensions: &[String],
    exclude: &[String],
) -> ExtractResult<Vec<(String, String)>> {
    let mut docs = Vec::new();

    for entry in WalkDir::new(dir).follow_links(true) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Check extension
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !extensions.iter().any(|e| e == ext) {
            continue;
        }

        // Check exclusions
        if exclude.iter().any(|e| e == file_name) {
            continue;
        }

        let relative = path
            .strip_prefix(dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::error::ExtractError::ReadFile {
                path: relative.clone(),
                source: e,
            }
        })?;

        docs.push((relative, content));
    }

    // Sort for deterministic output
    docs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(docs)
}

/// Get current time as ISO 8601 string (no chrono dep — use simple format).
fn chrono_now() -> String {
    // Use std time — exact formatting doesn't matter for candidates file
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s-since-epoch", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_test_corpus(dir: &Path) {
        fs::write(
            dir.join("auth.md"),
            "---\ntitle: Auth\n---\n\n# Authentication\n\n\
             OAuth provides delegated authorization. OAuth 2.0 is the \
             current standard for token-based access control.\n\n\
             ## Passwords\n\nPassword hashing uses bcrypt or argon2.\n",
        )
        .unwrap();

        fs::write(
            dir.join("api.md"),
            "# API Design\n\n\
             RESTful APIs use HTTP methods for CRUD operations. \
             Rate limiting protects against abuse. \
             The API requires OAuth tokens for authentication.\n",
        )
        .unwrap();

        fs::write(
            dir.join("security.md"),
            "# Security\n\n\
             Transport Layer Security (TLS) encrypts data in transit. \
             Cross-site scripting (XSS) is a common web vulnerability. \
             Input validation prevents injection attacks.\n",
        )
        .unwrap();
    }

    #[test]
    fn test_pipeline_produces_candidates() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };
        let extract_cfg = ExtractConfig::default();

        let result = run(&source, &extract_cfg);
        assert!(result.is_ok(), "pipeline should succeed: {result:?}");

        let file = result.unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.document_count, 3);
        assert!(!file.candidates.is_empty(), "should produce candidates");
    }

    #[test]
    fn test_pipeline_respects_exclude() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());
        fs::write(tmp.path().join("README.md"), "# Ignore me\n\nThis should be excluded.\n")
            .unwrap();

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: vec!["README.md".to_string()],
        };

        let file = run(&source, &ExtractConfig::default()).unwrap();
        // README.md excluded, so 3 docs not 4
        assert_eq!(file.document_count, 3);
    }

    #[test]
    fn test_pipeline_no_documents_error() {
        let tmp = TempDir::new().unwrap();
        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let result = run(&source, &ExtractConfig::default());
        assert!(result.is_err(), "empty dir should error");
    }

    #[test]
    fn test_pipeline_candidates_sorted_by_score() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let file = run(&source, &ExtractConfig::default()).unwrap();
        for window in file.candidates.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "candidates should be sorted descending: {} ({}) before {} ({})",
                window[0].term,
                window[0].score,
                window[1].term,
                window[1].score,
            );
        }
    }

    #[test]
    fn test_pipeline_output_serializes_to_yaml() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let file = run(&source, &ExtractConfig::default()).unwrap();
        let yaml = file.to_yaml().expect("should serialize to YAML");
        assert!(yaml.contains("version: 1"));
        assert!(yaml.contains("candidates:"));
    }
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p colophon-core extract::tests`
Expected: PASS

**Step 3: Commit**

```
feat(core): implement extraction pipeline orchestrator
```

---

### Task 7: Wire up the `colophon extract` CLI command

**Files:**
- Create: `crates/colophon/src/commands/extract.rs`
- Modify: `crates/colophon/src/commands/mod.rs`
- Modify: `crates/colophon/src/lib.rs`
- Modify: `crates/colophon/src/main.rs`

**Step 1: Create the extract command**

Create `crates/colophon/src/commands/extract.rs`:

```rust
//! Extract command — scan documents and extract keyword candidates.

use anyhow::Context;
use clap::Args;
use colophon_core::config::Config;
use colophon_core::extract;
use tracing::{debug, instrument};

/// Arguments for the `extract` subcommand.
#[derive(Args, Debug, Default)]
pub struct ExtractArgs {
    /// Source directory to scan (overrides config)
    #[arg(short, long)]
    pub dir: Option<String>,

    /// Output file for candidates (default: colophon-candidates.yaml)
    #[arg(short, long, default_value = "colophon-candidates.yaml")]
    pub output: String,
}

/// Run the extraction pipeline.
#[instrument(name = "cmd_extract", skip_all)]
pub fn cmd_extract(args: ExtractArgs, json: bool, config: &Config) -> anyhow::Result<()> {
    debug!("executing extract command");

    let mut source = config.source.clone();
    if let Some(dir) = args.dir {
        source.dir = dir;
    }

    let candidates = extract::run(&source, &config.extract)
        .context("extraction pipeline failed")?;

    if json {
        let json_out = serde_json::to_string_pretty(&candidates)
            .context("failed to serialize candidates as JSON")?;
        println!("{json_out}");
    } else {
        // Write YAML to output file
        let yaml = candidates
            .to_yaml()
            .context("failed to serialize candidates")?;
        std::fs::write(&args.output, &yaml)
            .with_context(|| format!("failed to write {}", args.output))?;

        println!(
            "Extracted {} candidates from {} documents",
            candidates.candidates.len(),
            candidates.document_count
        );
        println!("Written to: {}", args.output);
    }

    Ok(())
}
```

**Step 2: Wire into commands/mod.rs**

Add to `crates/colophon/src/commands/mod.rs`:

```rust
pub mod extract;
```

**Step 3: Add to Commands enum in lib.rs**

Add variant to `Commands` enum in `crates/colophon/src/lib.rs`:

```rust
/// Extract keyword candidates from documents
Extract(commands::extract::ExtractArgs),
```

**Step 4: Add match arm in main.rs**

Add to the `match command` block in `main.rs`:

```rust
Commands::Extract(args) => commands::extract::cmd_extract(args, cli.json, &config),
```

**Step 5: Run all tests**

Run: `cargo nextest run`
Expected: PASS (all crates)

**Step 6: Smoke test the binary**

Run: `cargo run -- extract --help`
Expected: shows extract command help with `--dir` and `--output` options

**Step 7: Commit**

```
feat(cli): wire up colophon extract command
```

---

### Task 8: Integration test for extract command

**Files:**
- Modify: `crates/colophon/tests/cli.rs`

**Step 1: Add integration tests**

Append to `crates/colophon/tests/cli.rs`:

```rust
// =============================================================================
// Extract Command
// =============================================================================

#[test]
fn extract_help_shows_options() {
    cmd()
        .args(["extract", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dir"))
        .stdout(predicate::str::contains("--output"));
}

#[test]
fn extract_on_empty_dir_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "extract", "--dir", "."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no documents found"));
}

#[test]
fn extract_produces_yaml_output() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("test.md"),
        "# OAuth Authentication\n\n\
         OAuth provides delegated authorization for web applications. \
         Token-based authentication is the modern standard.\n",
    )
    .unwrap();

    let output_file = tmp.path().join("candidates.yaml");
    cmd()
        .args([
            "-C",
            tmp.path().to_str().unwrap(),
            "extract",
            "--dir",
            ".",
            "--output",
            output_file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("candidates"))
        .stdout(predicate::str::contains("documents"));

    assert!(output_file.exists(), "should write candidates file");
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert!(content.contains("version: 1"), "should be valid candidates YAML");
}

#[test]
fn extract_json_outputs_valid_json() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("doc.md"),
        "# Testing\n\nThis is a document about keyword extraction and NLP.\n",
    )
    .unwrap();

    let output = cmd()
        .args([
            "-C",
            tmp.path().to_str().unwrap(),
            "extract",
            "--dir",
            ".",
            "--json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json should output valid JSON");
    assert_eq!(json["version"], 1);
}
```

Add `tempfile` to CLI crate's dev-dependencies if not already there (it is).

**Step 2: Run integration tests**

Run: `cargo nextest run -p colophon extract`
Expected: PASS

**Step 3: Commit**

```
test(cli): add integration tests for extract command
```

---

### Task 9: Final verification

**Step 1: Run full check**

Run: `just check`
Expected: fmt + clippy + deny + test + doc-test all pass

Fix any clippy warnings or deny issues that surface. Common things to watch:
- `missing_docs` warnings on public items — add doc comments
- clippy nursery lints on unnecessary clones or allocations
- deny license issues from new transitive deps

**Step 2: Manual smoke test**

If you have markdown files available, run:
```
cargo run -- extract --dir /path/to/your/markdown/files
```
Inspect the `colophon-candidates.yaml` output.

**Step 3: Commit any fixes**

```
chore: fix clippy and deny warnings from phase 1
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Add deps | `colophon-core/Cargo.toml`, `deny.toml` |
| 2 | Config sections | `config.rs` |
| 3 | Candidate types | `extract/mod.rs`, `extract/candidates.rs`, `error.rs`, `lib.rs` |
| 4 | Markdown parser | `extract/markdown.rs` |
| 5 | Keyword extraction | `extract/keywords.rs` |
| 6 | Pipeline orchestrator | `extract/mod.rs` |
| 7 | CLI wiring | `commands/extract.rs`, `commands/mod.rs`, `lib.rs`, `main.rs` |
| 8 | Integration tests | `tests/cli.rs` |
| 9 | Final verification | fix-up pass |
