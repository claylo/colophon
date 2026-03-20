//! Incremental curate pipeline — diff, format, and merge.

use std::collections::HashSet;

use crate::curate::terms::{ClaudeDeltaOutput, CuratedTerm, CuratedTermsFile};
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
pub(crate) fn merge_delta(terms: &mut Vec<CuratedTerm>, delta: &ClaudeDeltaOutput) -> MergeLog {
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
        let Some(existing) = terms
            .iter_mut()
            .find(|t| t.term.eq_ignore_ascii_case(&m.term))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curate::terms::{
        ClaudeSuggested, ClaudeTerm, DeltaModification, DeltaRemoval, TermLocation,
    };
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

    fn delta(
        additions: Vec<ClaudeTerm>,
        modifications: Vec<DeltaModification>,
        removals: Vec<DeltaRemoval>,
        suggested: Vec<ClaudeSuggested>,
    ) -> ClaudeDeltaOutput {
        ClaudeDeltaOutput {
            additions,
            modifications,
            removals,
            suggested,
        }
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
        let d = delta(
            vec![],
            vec![],
            vec![DeltaRemoval {
                term: "deprecated".to_string(),
                reason: "gone".to_string(),
            }],
            vec![],
        );
        let log = merge_delta(&mut terms, &d);
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
        let d = delta(
            vec![],
            vec![DeltaModification {
                term: "OAuth".to_string(),
                definition: Some("New definition.".to_string()),
                parent: None,
                aliases: None,
                see_also: None,
                reason: "updated".to_string(),
            }],
            vec![],
            vec![],
        );
        let log = merge_delta(&mut terms, &d);
        assert_eq!(terms[0].definition, "New definition.");
        assert_eq!(terms[0].aliases, vec!["OAuth 2.0"]);
        assert_eq!(log.modified, 1);
    }

    #[test]
    fn merge_modifications_reparent() {
        let mut terms = vec![curated("OAuth", &[], true)];
        let d = delta(
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
        merge_delta(&mut terms, &d);
        assert_eq!(terms[0].parent.as_deref(), Some("security"));
    }

    #[test]
    fn merge_additions() {
        let mut terms = vec![curated("OAuth", &[], true)];
        let d = delta(vec![addition("PKCE")], vec![], vec![], vec![]);
        let log = merge_delta(&mut terms, &d);
        assert_eq!(terms.len(), 2);
        assert!(terms.iter().any(|t| t.term == "PKCE"));
        assert_eq!(log.added, 1);
    }

    #[test]
    fn merge_suggested() {
        let mut terms = vec![curated("OAuth", &[], true)];
        let d = delta(
            vec![],
            vec![],
            vec![],
            vec![ClaudeSuggested {
                term: "bearer token".to_string(),
                definition: "A token type.".to_string(),
                parent: Some("OAuth".to_string()),
            }],
        );
        let log = merge_delta(&mut terms, &d);
        let bt = terms.iter().find(|t| t.term == "bearer token").unwrap();
        assert!(bt.locations.is_empty());
        assert_eq!(bt.parent.as_deref(), Some("OAuth"));
        assert_eq!(log.suggested, 1);
    }

    #[test]
    fn merge_modification_dangling_target_skipped() {
        let mut terms = vec![curated("OAuth", &[], true)];
        let d = delta(
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
        let log = merge_delta(&mut terms, &d);
        assert_eq!(log.modified, 0);
        assert_eq!(terms.len(), 1);
    }
}
