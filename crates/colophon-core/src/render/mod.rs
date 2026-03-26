//! Render pipeline — annotate source files with index markers.

pub mod typst;

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

use crate::curate::terms::{CuratedTerm, CuratedTermsFile};
use crate::error::RenderResult;

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

/// A term that could not be found in its expected source file.
#[derive(Debug, Clone, Serialize)]
pub struct TermNotFound {
    /// The canonical term text.
    pub term: String,
    /// The source file where the term was expected.
    pub file: String,
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
    /// Details of terms not found, for reporting.
    pub not_found_details: Vec<TermNotFound>,
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
    ///
    /// `spacing` is an optional Typst length value (e.g., `"12pt"`, `"1.5em"`)
    /// controlling the gap between glossary entries.
    fn glossary(&self, terms: &CuratedTermsFile, spacing: Option<&str>) -> String;
}

/// Build the parent chain for a term, from root to immediate parent.
///
/// Walks parent pointers, detects cycles via a visited set.
/// Dangling parents (not in the term list) are included in the chain.
pub(crate) fn build_parent_chain(term_name: &str, terms: &[CuratedTerm]) -> Vec<String> {
    let by_name: std::collections::HashMap<&str, &CuratedTerm> =
        terms.iter().map(|t| (t.term.as_str(), t)).collect();

    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(term_name.to_string());

    // Start from the given term's parent
    let mut current = by_name
        .get(term_name)
        .and_then(|t| t.parent.as_deref())
        .map(String::from);

    while let Some(ref parent_name) = current {
        if !visited.insert(parent_name.clone()) {
            tracing::warn!(term = term_name, cycle_at = %parent_name, "cycle detected in parent chain");
            break;
        }
        chain.push(parent_name.clone());
        current = by_name
            .get(parent_name.as_str())
            .and_then(|t| t.parent.as_deref())
            .map(String::from);
    }

    // Reverse so chain reads root -> immediate parent
    chain.reverse();
    chain
}

/// Find the byte offset immediately after the first occurrence of `term` in `text`.
///
/// Case-insensitive search. Returns `None` if not found.
pub(crate) fn find_term_offset(text: &str, term: &str) -> Option<usize> {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();
    lower_text.find(&lower_term).and_then(|pos| {
        let offset = pos + term.len();
        // Guard: offset from lowered buffer may not be a char boundary in the
        // original text when Unicode case folding changes byte lengths.
        if offset <= text.len() && text.is_char_boundary(offset) {
            Some(offset)
        } else {
            None
        }
    })
}

/// Render format selection.
pub enum RenderFormat {
    /// Typst with in-dexter index markers.
    Typst,
}

/// Configuration for a render run.
pub struct RenderConfig<'a> {
    /// Directory containing source files to annotate.
    pub source_dir: &'a str,
    /// File extensions to process (e.g. `["typ"]`).
    pub extensions: &'a [String],
    /// Directory to write annotated output files.
    pub output_dir: &'a str,
    /// Whether to emit a standalone glossary document.
    pub glossary: bool,
    /// Only insert markers for main (substantive) locations.
    pub main_only: bool,
    /// Optional spacing between glossary entries (e.g. `"12pt"`).
    pub glossary_spacing: Option<&'a str>,
    /// Output format.
    pub format: RenderFormat,
}

/// Run the render pipeline.
pub fn run(terms: &CuratedTermsFile, config: RenderConfig<'_>) -> RenderResult<RenderOutput> {
    match config.format {
        RenderFormat::Typst => {
            let renderer = typst::TypstRenderer;
            run_with_renderer(terms, &config, &renderer)
        }
    }
}

/// Orchestrate annotation of source files and optional glossary generation.
///
/// Walks `source_dir` for files matching `extensions`, finds terms in each file,
/// inserts index markers via the provided [`Renderer`], and writes annotated files
/// to `output_dir` preserving relative paths.
///
/// When `main_only` is true, only locations marked as substantive (`main: true`)
/// receive index markers — producing a much sparser, more navigable index.
pub(crate) fn run_with_renderer(
    terms: &CuratedTermsFile,
    config: &RenderConfig<'_>,
    renderer: &dyn Renderer,
) -> RenderResult<RenderOutput> {
    let source_path = Path::new(config.source_dir);
    let output_path = Path::new(config.output_dir);
    std::fs::create_dir_all(output_path)?;

    let ext_set: HashSet<&str> = config.extensions.iter().map(String::as_str).collect();
    let mut stats = RenderOutput::default();

    for entry in WalkDir::new(source_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext_set.contains(ext) {
            continue;
        }

        let rel_path = path
            .strip_prefix(source_path)
            .unwrap_or(path)
            .to_string_lossy();

        let source = std::fs::read_to_string(path)?;

        // For Typst files, compute prose-safe byte ranges from the AST.
        // This prevents markers from landing inside labels, links, code, etc.
        let prose_ranges = if ext == "typ" {
            Some(typst::collect_prose_ranges(&source))
        } else {
            None
        };

        // Collect terms whose locations reference this file.
        let mut annotations = Vec::new();
        for term in &terms.terms {
            for loc in &term.locations {
                if loc.file != rel_path {
                    continue;
                }

                // In main-only mode, skip non-substantive locations.
                if config.main_only && !loc.main {
                    continue;
                }

                // Try canonical term first, then aliases.
                // For Typst: AST-aware search (skips labels, links, etc.)
                // For other formats: simple text search.
                let offset = prose_ranges.as_ref().map_or_else(
                    || {
                        find_term_offset(&source, &term.term).or_else(|| {
                            term.aliases
                                .iter()
                                .find_map(|alias| find_term_offset(&source, alias))
                        })
                    },
                    |ranges| {
                        typst::find_term_offset_in_prose(&source, &term.term, ranges).or_else(
                            || {
                                term.aliases.iter().find_map(|alias| {
                                    typst::find_term_offset_in_prose(&source, alias, ranges)
                                })
                            },
                        )
                    },
                );

                match offset {
                    Some(byte_offset) => {
                        let parent_chain = build_parent_chain(&term.term, &terms.terms);
                        annotations.push(Annotation {
                            term: term.term.clone(),
                            parent_chain,
                            main: loc.main,
                            byte_offset,
                        });
                    }
                    None => {
                        tracing::warn!(
                            term = %term.term,
                            file = %rel_path,
                            "term not found in source file"
                        );
                        stats.not_found_details.push(TermNotFound {
                            term: term.term.clone(),
                            file: rel_path.to_string(),
                        });
                        stats.terms_not_found += 1;
                    }
                }

                // One marker per term per file — stop after first location match.
                break;
            }
        }

        if annotations.is_empty() {
            // Still copy the file through even if no annotations.
            let dest = output_path.join(rel_path.as_ref());
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &source)?;
            continue;
        }

        // Sort descending by byte_offset so insertions don't shift later offsets.
        annotations.sort_by(|a, b| b.byte_offset.cmp(&a.byte_offset));

        let annotated = renderer.annotate(&source, &annotations);

        let dest = output_path.join(rel_path.as_ref());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &annotated)?;

        stats.files_annotated += 1;
        for ann in &annotations {
            stats.markers_inserted += 1;
            if ann.main {
                stats.markers_main += 1;
            }
        }
    }

    if config.glossary {
        let glossary_content = renderer.glossary(terms, config.glossary_spacing);
        let glossary_path = output_path.join("glossary.typ");
        std::fs::write(&glossary_path, &glossary_content)?;
        stats.glossary_terms = terms.terms.len();
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curate::terms::TermLocation;

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
        let terms = vec![
            term("OAuth", Some("authentication")),
            term("authentication", None),
        ];
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
        let terms = vec![term("A", Some("B")), term("B", Some("A"))];
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
        let text = "Use \u{2192} OAuth for auth.";
        let offset = find_term_offset(text, "OAuth");
        assert!(offset.is_some());
        let o = offset.unwrap();
        // Verify the bytes before the offset spell "OAuth"
        assert_eq!(&text[o - "OAuth".len()..o], "OAuth");
    }

    #[test]
    fn run_annotates_source_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("auth.typ"),
            "OAuth provides delegated authorization.\n",
        )
        .unwrap();

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
        let exts = vec!["typ".to_string()];
        let config = RenderConfig {
            source_dir: source_dir.to_str().unwrap(),
            extensions: &exts,
            output_dir: output_dir.to_str().unwrap(),
            glossary: false,
            main_only: false,
            glossary_spacing: None,
            format: RenderFormat::Typst,
        };
        let result = run_with_renderer(&terms, &config, &renderer);
        assert!(result.is_ok(), "run should succeed: {result:?}");
        let output = result.unwrap();
        assert_eq!(output.files_annotated, 1);
        assert_eq!(output.markers_inserted, 1);
        assert_eq!(output.markers_main, 1);

        let annotated = std::fs::read_to_string(output_dir.join("auth.typ")).unwrap();
        assert!(annotated.contains("#index-main[OAuth]"));
        assert!(annotated.contains("in-dexter"));
    }

    #[test]
    fn run_alias_generates_canonical_annotation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("api.typ"),
            "The OAuth 2.0 protocol is widely used.\n",
        )
        .unwrap();

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
        let exts = vec!["typ".to_string()];
        let config = RenderConfig {
            source_dir: source_dir.to_str().unwrap(),
            extensions: &exts,
            output_dir: output_dir.to_str().unwrap(),
            glossary: false,
            main_only: false,
            glossary_spacing: None,
            format: RenderFormat::Typst,
        };
        let result = run_with_renderer(&terms, &config, &renderer).unwrap();
        assert_eq!(result.markers_inserted, 1);

        let annotated = std::fs::read_to_string(output_dir.join("api.typ")).unwrap();
        assert!(
            annotated.contains("#index[OAuth]"),
            "marker should use canonical term, got: {annotated}"
        );
    }

    #[test]
    fn run_one_marker_per_term_per_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("auth.typ"),
            "OAuth is great. OAuth is flexible. OAuth is everywhere.\n",
        )
        .unwrap();

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
        let exts = vec!["typ".to_string()];
        let config = RenderConfig {
            source_dir: source_dir.to_str().unwrap(),
            extensions: &exts,
            output_dir: output_dir.to_str().unwrap(),
            glossary: false,
            main_only: false,
            glossary_spacing: None,
            format: RenderFormat::Typst,
        };
        let result = run_with_renderer(&terms, &config, &renderer).unwrap();
        assert_eq!(result.markers_inserted, 1);

        let annotated = std::fs::read_to_string(output_dir.join("auth.typ")).unwrap();
        assert_eq!(annotated.matches("#index[OAuth]").count(), 1);
    }

    #[test]
    fn run_terms_not_found_counted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source_dir = tmp.path().join("src");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(source_dir.join("empty.typ"), "Nothing relevant here.\n").unwrap();

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
        let exts = vec!["typ".to_string()];
        let config = RenderConfig {
            source_dir: source_dir.to_str().unwrap(),
            extensions: &exts,
            output_dir: output_dir.to_str().unwrap(),
            glossary: false,
            main_only: false,
            glossary_spacing: None,
            format: RenderFormat::Typst,
        };
        let result = run_with_renderer(&terms, &config, &renderer).unwrap();
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
        let exts = vec!["typ".to_string()];
        let config = RenderConfig {
            source_dir: source_dir.to_str().unwrap(),
            extensions: &exts,
            output_dir: output_dir.to_str().unwrap(),
            glossary: true,
            main_only: false,
            glossary_spacing: None,
            format: RenderFormat::Typst,
        };
        let result = run_with_renderer(&terms, &config, &renderer).unwrap();
        assert_eq!(result.glossary_terms, 1);

        let glossary_path = output_dir.join("glossary.typ");
        assert!(glossary_path.exists(), "glossary.typ should be written");
        let glossary = std::fs::read_to_string(glossary_path).unwrap();
        assert!(glossary.contains("<term-oauth>"));
    }
}
