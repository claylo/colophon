//! Incremental curate pipeline — diff, format, and merge.

use std::collections::HashSet;

use crate::curate::terms::CuratedTermsFile;
use crate::extract::candidates::{Candidate, CandidatesFile};

/// Diff between fresh extraction candidates and an existing curated term database.
#[derive(Debug)]
pub struct CandidateDiff {
    /// Candidates that do not match any existing curated term or alias.
    pub new_candidates: Vec<Candidate>,
    /// Curated terms (with at least one location) whose name and aliases are
    /// absent from the fresh candidates — likely deleted from the corpus.
    pub stale_terms: Vec<String>,
    /// Total number of candidates in the fresh extraction.
    pub total_candidates: usize,
}

impl CandidateDiff {
    /// Fraction of fresh candidates that are new (0.0 if no candidates).
    pub fn new_ratio(&self) -> f64 {
        if self.total_candidates == 0 {
            return 0.0;
        }
        self.new_candidates.len() as f64 / self.total_candidates as f64
    }
}

/// Diff fresh extraction candidates against an existing curated term database.
///
/// A candidate is **new** if its term (lowercased) does not match any existing
/// curated term name or alias.
///
/// A curated term is **stale** if its name and all aliases have zero matches in
/// the fresh candidates AND it has at least one location (suggested-only terms
/// with empty locations are excluded from stale detection).
pub fn diff_candidates(existing: &CuratedTermsFile, fresh: &CandidatesFile) -> CandidateDiff {
    let mut known_keys: HashSet<String> = HashSet::new();
    for term in &existing.terms {
        known_keys.insert(term.term.to_lowercase());
        for alias in &term.aliases {
            known_keys.insert(alias.to_lowercase());
        }
    }

    let new_candidates: Vec<Candidate> = fresh
        .candidates
        .iter()
        .filter(|c| !known_keys.contains(&c.term.to_lowercase()))
        .cloned()
        .collect();

    let fresh_keys: HashSet<String> = fresh
        .candidates
        .iter()
        .map(|c| c.term.to_lowercase())
        .collect();

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

/// Format the existing curated term database as a compact one-line-per-term
/// index for use as context in incremental Claude prompts.
///
/// Includes only relationship metadata (parent, children, aliases, see_also) —
/// no definitions or locations. Approximately 50 tokens per term.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curate::terms::{CuratedTerm, TermLocation};
    use crate::extract::candidates::CandidateLocation;

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
        let existing = terms_file(vec![
            curated("OAuth", &[], true),
            curated("bearer token", &[], false),
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
        assert!(compact.contains(
            "OAuth | parent: authentication | aliases: OAuth 2.0, OAuth2 | see_also: API key"
        ));
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
}
