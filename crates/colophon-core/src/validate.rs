//! Post-curate validation — detect unresolvable locations and suggest aliases.

use std::collections::{HashMap, HashSet};
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

/// File content cache entry.
struct FileEntry {
    /// Raw source text.
    content: String,
    /// Prose ranges for .typ files; `None` for everything else.
    prose_ranges: Option<Vec<(usize, usize)>>,
}

/// Validate that every location in a curated terms file can actually be
/// resolved — i.e., the canonical term or one of its aliases appears in
/// the source file's prose text.
///
/// Returns a [`ValidationReport`] with counts and alias suggestions for
/// any locations that could not be resolved.
pub fn validate_locations(
    terms: &CuratedTermsFile,
    source_dir: &str,
    extensions: &[String],
) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut file_cache: HashMap<String, Option<FileEntry>> = HashMap::new();

    for curated in &terms.terms {
        for loc in &curated.locations {
            // Filter by extension if provided.
            if !extensions.is_empty() {
                let ext = Path::new(&loc.file)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !extensions.iter().any(|e| e == ext) {
                    continue;
                }
            }

            let entry = file_cache
                .entry(loc.file.clone())
                .or_insert_with(|| load_file(source_dir, &loc.file));

            let Some(entry) = entry else {
                // File doesn't exist — unresolved, but no suggestion possible.
                report.unresolved += 1;
                continue;
            };

            let prose_ranges = entry.prose_ranges.as_deref();

            // Try canonical term.
            if try_find(&entry.content, &curated.term, prose_ranges) {
                report.resolved += 1;
                continue;
            }

            // Try each alias.
            let alias_hit = curated
                .aliases
                .iter()
                .any(|alias| try_find(&entry.content, alias, prose_ranges));

            if alias_hit {
                report.resolved += 1;
                continue;
            }

            // Unresolved — attempt suggestion.
            report.unresolved += 1;
            if let Some(suggested) = suggest_alias(&entry.content, &curated.term, prose_ranges) {
                report.suggestions.push(AliasSuggestion {
                    term: curated.term.clone(),
                    file: loc.file.clone(),
                    suggested_alias: suggested,
                });
            }
        }
    }

    // Dedup: keep only the first suggestion per (term, suggested_alias).
    let mut seen = HashSet::new();
    report
        .suggestions
        .retain(|s| seen.insert((s.term.clone(), s.suggested_alias.clone())));

    report
}

/// Load a source file and compute prose ranges for .typ files.
fn load_file(source_dir: &str, relative: &str) -> Option<FileEntry> {
    let path = Path::new(source_dir).join(relative);
    let content = std::fs::read_to_string(&path).ok()?;
    let is_typst = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == "typ");
    let prose_ranges = if is_typst {
        Some(typst_prose::collect_prose_ranges(&content))
    } else {
        None
    };
    Some(FileEntry {
        content,
        prose_ranges,
    })
}

/// Check whether `term` appears in the file content (AST-aware for .typ).
fn try_find(content: &str, term: &str, prose_ranges: Option<&[(usize, usize)]>) -> bool {
    prose_ranges.map_or_else(
        || content.to_lowercase().contains(&term.to_lowercase()),
        |ranges| typst_prose::find_term_offset_in_prose(content, term, ranges).is_some(),
    )
}

/// Heuristic: try sub-phrases and singular/plural toggling to suggest an alias.
fn suggest_alias(
    content: &str,
    term: &str,
    prose_ranges: Option<&[(usize, usize)]>,
) -> Option<String> {
    let words: Vec<&str> = term.split_whitespace().collect();

    if words.len() >= 2 {
        // Try suffixes (drop from front): "Amazon Bedrock" -> "Bedrock"
        for start in 1..words.len() {
            let suffix = words[start..].join(" ");
            if try_find(content, &suffix, prose_ranges) {
                return Some(suffix);
            }
        }

        // Try prefixes (drop from end): "Slack integration" -> "Slack"
        for end in (1..words.len()).rev() {
            let prefix = words[..end].join(" ");
            if try_find(content, &prefix, prose_ranges) {
                return Some(prefix);
            }
        }
    }

    // Singular/plural toggle.
    let toggled = toggle_plural(term);
    if try_find(content, &toggled, prose_ranges) {
        return Some(toggled);
    }

    None
}

/// Toggle trailing 's' — naive but good enough for a suggestion heuristic.
fn toggle_plural(term: &str) -> String {
    term.strip_suffix('s')
        .map_or_else(|| format!("{term}s"), str::to_string)
}

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

    fn term_with_locations(
        name: &str,
        aliases: Vec<&str>,
        locations: Vec<(&str, bool)>,
    ) -> CuratedTerm {
        CuratedTerm {
            term: name.to_string(),
            definition: format!("Definition of {name}."),
            parent: None,
            aliases: aliases.into_iter().map(String::from).collect(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: locations
                .into_iter()
                .map(|(file, main)| TermLocation {
                    file: file.to_string(),
                    main,
                    context: String::new(),
                })
                .collect(),
        }
    }

    // ── Task 2 tests ────────────────────────────────────────────────

    #[test]
    fn all_locations_resolve_no_suggestions() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "This chapter discusses OAuth in depth.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "OAuth",
                vec![],
                vec![("ch01.typ", true)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.resolved, 1);
        assert_eq!(report.unresolved, 0);
        assert!(report.suggestions.is_empty());
    }

    #[test]
    fn unresolved_location_no_matching_text() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "This chapter discusses Kubernetes.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "Zyxelfrob",
                vec![],
                vec![("ch01.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.unresolved, 1);
        assert!(report.suggestions.is_empty());
    }

    #[test]
    fn alias_resolves_location() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "We use Slack for team communication.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "Slack integration",
                vec!["Slack"],
                vec![("ch01.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.resolved, 1);
        assert_eq!(report.unresolved, 0);
    }

    #[test]
    fn markdown_file_uses_simple_search() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("auth.md"),
            "# Authentication\n\nOAuth provides delegated authorization.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "OAuth",
                vec![],
                vec![("auth.md", true)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.resolved, 1);
        assert_eq!(report.unresolved, 0);
    }

    #[test]
    fn missing_source_file_counts_as_unresolved() {
        let dir = TempDir::new().unwrap();
        // Don't create the file — it should be missing.

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "OAuth",
                vec![],
                vec![("nonexistent.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.unresolved, 1);
        assert!(report.suggestions.is_empty());
    }

    // ── Task 3 tests ────────────────────────────────────────────────

    #[test]
    fn suggests_alias_for_compound_term_drop_first_word() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "We evaluated Bedrock as a foundation model platform.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "Amazon Bedrock",
                vec![],
                vec![("ch01.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.unresolved, 1);
        assert_eq!(report.suggestions.len(), 1);
        assert_eq!(report.suggestions[0].suggested_alias, "Bedrock");
    }

    #[test]
    fn suggests_multi_word_suffix() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "Vertex AI provides powerful ML capabilities.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "Google Vertex AI",
                vec![],
                vec![("ch01.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.unresolved, 1);
        assert_eq!(report.suggestions.len(), 1);
        assert_eq!(report.suggestions[0].suggested_alias, "Vertex AI");
    }

    #[test]
    fn suggests_singular_for_plural_term() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "Each plugin extends the build system.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "plugins",
                vec![],
                vec![("ch01.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.unresolved, 1);
        assert_eq!(report.suggestions.len(), 1);
        assert_eq!(report.suggestions[0].suggested_alias, "plugin");
    }

    #[test]
    fn deduplicates_suggestions_across_files() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ch01.typ"),
            "Bedrock powers our inference layer.",
        )
        .unwrap();
        fs::write(
            dir.path().join("ch02.typ"),
            "Bedrock supports multiple model providers.",
        )
        .unwrap();

        let tf = CuratedTermsFile {
            source_dir: dir.path().to_str().unwrap().to_string(),
            ..terms_file(vec![term_with_locations(
                "Amazon Bedrock",
                vec![],
                vec![("ch01.typ", false), ("ch02.typ", false)],
            )])
        };

        let report = validate_locations(&tf, &tf.source_dir, &[]);
        assert_eq!(report.unresolved, 2);
        // Same (term, alias) pair should be deduped to 1.
        assert_eq!(report.suggestions.len(), 1);
        assert_eq!(report.suggestions[0].suggested_alias, "Bedrock");
    }
}
