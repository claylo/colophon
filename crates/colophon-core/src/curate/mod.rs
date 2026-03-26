//! Curation pipeline — Claude-powered term curation via the `claude` CLI.

mod claude;
pub mod cost;
pub mod incremental;
pub mod terms;

use std::collections::HashMap;

use indicatif::ProgressBar;

use crate::config::CurateConfig;
use crate::error::{CurateError, CurateResult};
use crate::extract::candidates::CandidatesFile;

use self::cost::{CostEstimate, TokenUsage};
use self::terms::{ClaudeOutput, CuratedTerm, CuratedTermsFile, TermLocation};

/// Result of the full curation pipeline.
pub struct CurationOutput {
    /// The curated terms file (post-processed with locations and hierarchy).
    pub terms_file: CuratedTermsFile,
    /// Accumulated thinking output from Claude (editorial audit trail).
    pub thinking: String,
    /// Editorial summary text from Claude (non-tool text blocks).
    pub editorial: String,
    /// Number of API turns used.
    pub turns: usize,
    /// Number of thinking token deltas received.
    pub thinking_tokens: usize,
    /// Actual token usage from the API (for cost reporting).
    pub usage: TokenUsage,
}

/// Estimate cost without invoking Claude.
///
/// Counts tokens in the full prompt payload (system prompt + candidates +
/// schema + instruction) and calculates estimated USD cost by model.
pub fn estimate_cost(
    candidates: &CandidatesFile,
    candidates_yaml: &str,
    config: &CurateConfig,
) -> CostEstimate {
    let system_prompt = claude::system_prompt_for(config);
    let stdin_payload = claude::stdin_payload_for(config, candidates, candidates_yaml);
    let schema_json = claude::schema_json();

    cost::estimate(
        &system_prompt,
        &stdin_payload,
        schema_json,
        config.max_output_tokens,
        &config.model,
    )
}

/// Run the full curation pipeline.
///
/// Reads candidates, invokes the Claude CLI with streaming output,
/// post-processes the response (maps locations, builds hierarchy),
/// and returns the curated term database along with thinking/editorial text.
pub fn run(
    candidates: &CandidatesFile,
    candidates_yaml: &str,
    config: &CurateConfig,
    known_terms: &[crate::config::KnownTerm],
    extra_args: &[String],
    progress: &ProgressBar,
) -> CurateResult<CurationOutput> {
    if candidates.candidates.is_empty() {
        return Err(CurateError::NoCandidates(candidates.source_dir.clone()));
    }

    tracing::debug!(
        candidates = candidates.candidates.len(),
        source_dir = %candidates.source_dir,
        model = %config.model,
        full_candidates = config.full_candidates,
        max_terms = config.max_terms,
        "starting curation pipeline"
    );

    // Invoke Claude CLI with streaming output.
    let invoke_result = claude::invoke(config, candidates, candidates_yaml, extra_args, progress)?;

    // Post-process into the final term database.
    let terms = post_process(
        &invoke_result.output,
        candidates,
        config.max_terms,
        known_terms,
    );

    tracing::debug!(terms = terms.len(), "curation pipeline complete");

    Ok(CurationOutput {
        terms_file: CuratedTermsFile {
            version: 1,
            generated: crate::extract::format_timestamp(std::time::SystemTime::now()),
            source_dir: candidates.source_dir.clone(),
            document_count: candidates.document_count,
            terms,
        },
        thinking: invoke_result.thinking,
        editorial: invoke_result.editorial,
        turns: invoke_result.turns,
        thinking_tokens: invoke_result.thinking_tokens,
        usage: invoke_result.usage,
    })
}

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

/// Estimate cost for incremental curate.
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
pub fn run_incremental(
    existing: &CuratedTermsFile,
    candidates: &CandidatesFile,
    _candidates_yaml: &str,
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

    let ratio = diff.new_ratio();
    if ratio >= 0.7 {
        tracing::warn!(
            "{}% new candidates — strongly recommend --full-rebuild for better cross-term relationships",
            (ratio * 100.0) as u32
        );
    } else if ratio >= 0.4 {
        tracing::warn!(
            "{}% new candidates — consider --full-rebuild for better results",
            (ratio * 100.0) as u32
        );
    }

    let mut terms = existing.terms.clone();

    let (merge_log, invoke_result) = if diff.new_candidates.is_empty() {
        progress.set_message("No new terms — refreshing locations only");
        (incremental::MergeLog::default(), None)
    } else {
        let compact_index = format_compact_index(existing);

        let new_candidates_file = CandidatesFile {
            version: candidates.version,
            generated: candidates.generated.clone(),
            source_dir: candidates.source_dir.clone(),
            document_count: candidates.document_count,
            candidates: diff.new_candidates,
        };
        let new_yaml = serde_yaml::to_string(&new_candidates_file)?;

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

    remap_locations(&mut terms, candidates);
    rebuild_children(&mut terms);
    validate_parents(&mut terms);

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

/// Re-map locations for all terms using fresh candidate data.
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
        term.children = parent_map.get(&term.term).cloned().unwrap_or_default();
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

/// Post-process Claude's output into the final curated term list.
///
/// 1. Map locations from candidates using term + aliases as join keys
/// 2. Flag main locations using Claude's `main_files`
/// 3. Invert parent→children hierarchy
/// 4. Merge in suggested terms (without locations for now)
/// 5. Sort alphabetically, truncate to max_terms
fn post_process(
    output: &ClaudeOutput,
    candidates: &CandidatesFile,
    max_terms: usize,
    known_terms: &[crate::config::KnownTerm],
) -> Vec<CuratedTerm> {
    // Build a lookup: lowercase term → candidate locations.
    let candidate_map: HashMap<String, &crate::extract::candidates::Candidate> = candidates
        .candidates
        .iter()
        .map(|c| (c.term.to_lowercase(), c))
        .collect();

    let mut terms: Vec<CuratedTerm> = output
        .terms
        .iter()
        .map(|ct| {
            // Collect locations from the canonical term + all aliases.
            let lookup_keys: Vec<String> = std::iter::once(ct.term.to_lowercase())
                .chain(ct.aliases.iter().map(|a| a.to_lowercase()))
                .collect();

            let mut locations: Vec<TermLocation> = Vec::new();
            let mut seen_files = std::collections::HashSet::new();

            for key in &lookup_keys {
                if let Some(candidate) = candidate_map.get(key) {
                    for loc in &candidate.locations {
                        if seen_files.insert(loc.file.clone()) {
                            let is_main = ct
                                .main_files
                                .iter()
                                .any(|mf| std::path::Path::new(&loc.file).ends_with(mf.as_str()));
                            locations.push(TermLocation {
                                file: loc.file.clone(),
                                main: is_main,
                                context: loc.context.clone(),
                            });
                        }
                    }
                }
            }

            let unmatched_aliases: Vec<&str> = lookup_keys
                .iter()
                .filter(|k| !candidate_map.contains_key(k.as_str()))
                .map(String::as_str)
                .collect();
            if !unmatched_aliases.is_empty() {
                tracing::debug!(
                    term = %ct.term,
                    unmatched = ?unmatched_aliases,
                    "aliases not found in candidates"
                );
            }

            // Auto-alias: ensure render can find terms in source files.
            //
            // Two cases where the curated term won't match source text:
            // 1. Claude renames a candidate (e.g., "Bedrock" → "Amazon Bedrock")
            // 2. Extract consolidated known_terms variants that appear in source
            //    (e.g., config says Amazon Bedrock has variant "Bedrock")
            let mut aliases = ct.aliases.clone();
            let lower_curated = ct.term.to_lowercase();

            // Case 1: candidate name differs from curated name.
            for key in &lookup_keys {
                if let Some(candidate) = candidate_map.get(key) {
                    let lower_candidate = candidate.term.to_lowercase();
                    if lower_candidate != lower_curated
                        && !aliases.iter().any(|a| a.to_lowercase() == lower_candidate)
                    {
                        tracing::debug!(
                            term = %ct.term,
                            candidate = %candidate.term,
                            "auto-adding candidate as alias (renamed during curation)"
                        );
                        aliases.push(candidate.term.clone());
                    }
                }
            }

            // Case 2: known_terms variants absorbed during extract.
            // Match by curated term name OR by any alias (handles Claude renames).
            let alias_lower: Vec<String> = aliases.iter().map(|a| a.to_lowercase()).collect();
            for known in known_terms {
                let known_lower = known.term.to_lowercase();
                if known_lower == lower_curated || alias_lower.contains(&known_lower) {
                    for variant in &known.variants {
                        if !aliases
                            .iter()
                            .any(|a| a.to_lowercase() == variant.to_lowercase())
                        {
                            tracing::debug!(
                                term = %ct.term,
                                variant = %variant,
                                known_term = %known.term,
                                "auto-adding known_terms variant as alias"
                            );
                            aliases.push(variant.clone());
                        }
                    }
                }
            }

            CuratedTerm {
                term: ct.term.clone(),
                definition: ct.definition.clone(),
                parent: ct.parent.clone(),
                aliases,
                see_also: ct.see_also.clone(),
                children: Vec::new(),
                locations,
            }
        })
        .collect();

    // Add suggested terms (no locations — future: search source files).
    for suggested in &output.suggested {
        tracing::debug!(term = %suggested.term, "adding suggested term (no locations)");
        terms.push(CuratedTerm {
            term: suggested.term.clone(),
            definition: suggested.definition.clone(),
            parent: suggested.parent.clone(),
            aliases: Vec::new(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: Vec::new(),
        });
    }

    // Invert parent→children.
    let parent_map: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for t in &terms {
            if let Some(ref parent) = t.parent {
                map.entry(parent.clone()).or_default().push(t.term.clone());
            }
        }
        map
    };
    for term in &mut terms {
        if let Some(children) = parent_map.get(&term.term) {
            term.children = children.clone();
            term.children.sort_unstable();
        }
    }

    // Warn on dangling parent refs.
    let term_set: std::collections::HashSet<String> =
        terms.iter().map(|t| t.term.clone()).collect();
    for t in &terms {
        if let Some(ref parent) = t.parent
            && !term_set.contains(parent)
        {
            tracing::warn!(
                term = %t.term,
                parent = %parent,
                "parent term not found in curated output"
            );
        }
    }

    // Sort alphabetically, truncate.
    terms.sort_by(|a, b| a.term.to_lowercase().cmp(&b.term.to_lowercase()));
    terms.truncate(max_terms);

    terms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::candidates::{Candidate, CandidateLocation, CandidatesFile};

    fn sample_candidates() -> CandidatesFile {
        CandidatesFile {
            version: 1,
            generated: "2026-03-10T12:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 3,
            candidates: vec![
                Candidate {
                    term: "OAuth".to_string(),
                    score: 0.95,
                    locations: vec![
                        CandidateLocation {
                            file: "auth.md".to_string(),
                            context: "OAuth provides delegated authorization".to_string(),
                        },
                        CandidateLocation {
                            file: "api.md".to_string(),
                            context: "requires OAuth tokens".to_string(),
                        },
                    ],
                },
                Candidate {
                    term: "OAuth 2.0".to_string(),
                    score: 0.88,
                    locations: vec![CandidateLocation {
                        file: "auth.md".to_string(),
                        context: "OAuth 2.0 is the current standard".to_string(),
                    }],
                },
                Candidate {
                    term: "authentication".to_string(),
                    score: 0.82,
                    locations: vec![CandidateLocation {
                        file: "auth.md".to_string(),
                        context: "Authentication verifies identity".to_string(),
                    }],
                },
                Candidate {
                    term: "API key".to_string(),
                    score: 0.75,
                    locations: vec![CandidateLocation {
                        file: "api.md".to_string(),
                        context: "API keys for authentication".to_string(),
                    }],
                },
            ],
        }
    }

    fn sample_claude_output() -> ClaudeOutput {
        ClaudeOutput {
            terms: vec![
                terms::ClaudeTerm {
                    term: "OAuth".to_string(),
                    definition: "An open standard for token-based authorization.".to_string(),
                    parent: Some("authentication".to_string()),
                    aliases: vec!["OAuth 2.0".to_string()],
                    see_also: vec!["API key".to_string()],
                    main_files: vec!["auth.md".to_string()],
                },
                terms::ClaudeTerm {
                    term: "authentication".to_string(),
                    definition: "The process of verifying identity.".to_string(),
                    parent: None,
                    aliases: Vec::new(),
                    see_also: Vec::new(),
                    main_files: vec!["auth.md".to_string()],
                },
                terms::ClaudeTerm {
                    term: "API key".to_string(),
                    definition: "A credential for authenticating API requests.".to_string(),
                    parent: Some("authentication".to_string()),
                    aliases: Vec::new(),
                    see_also: vec!["OAuth".to_string()],
                    main_files: vec!["api.md".to_string()],
                },
            ],
            suggested: vec![terms::ClaudeSuggested {
                term: "bearer token".to_string(),
                definition: "A type of access token for API authorization.".to_string(),
                parent: Some("OAuth".to_string()),
            }],
        }
    }

    #[test]
    fn post_process_maps_locations() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 200, &[]);

        let oauth = terms.iter().find(|t| t.term == "OAuth").unwrap();
        // OAuth + alias "OAuth 2.0" should merge locations from both candidates.
        // auth.md appears in both but should be deduplicated.
        assert!(
            !oauth.locations.is_empty(),
            "OAuth should have mapped locations"
        );
        let files: Vec<&str> = oauth.locations.iter().map(|l| l.file.as_str()).collect();
        assert!(files.contains(&"auth.md"));
        assert!(files.contains(&"api.md"));
    }

    #[test]
    fn post_process_flags_main_files() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 200, &[]);

        let oauth = terms.iter().find(|t| t.term == "OAuth").unwrap();
        let auth_loc = oauth
            .locations
            .iter()
            .find(|l| l.file == "auth.md")
            .unwrap();
        assert!(auth_loc.main, "auth.md should be flagged as main for OAuth");

        let api_loc = oauth.locations.iter().find(|l| l.file == "api.md").unwrap();
        assert!(!api_loc.main, "api.md should not be main for OAuth");
    }

    #[test]
    fn post_process_main_files_rejects_substring_overlap() {
        // Regression: "auth.md" in main_files must NOT match "old-auth.md"
        let candidates = CandidatesFile {
            version: 1,
            generated: "2026-03-25T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 3,
            candidates: vec![Candidate {
                term: "OAuth".to_string(),
                score: 0.95,
                locations: vec![
                    CandidateLocation {
                        file: "auth.md".to_string(),
                        context: "OAuth is used here".to_string(),
                    },
                    CandidateLocation {
                        file: "old-auth.md".to_string(),
                        context: "Legacy OAuth reference".to_string(),
                    },
                    CandidateLocation {
                        file: "appendix/auth.md.bak".to_string(),
                        context: "Backup mention".to_string(),
                    },
                ],
            }],
        };
        let output = ClaudeOutput {
            terms: vec![terms::ClaudeTerm {
                term: "OAuth".to_string(),
                definition: "Open authorization standard.".to_string(),
                parent: None,
                aliases: Vec::new(),
                see_also: Vec::new(),
                main_files: vec!["auth.md".to_string()],
            }],
            suggested: Vec::new(),
        };

        let terms = post_process(&output, &candidates, 200, &[]);
        let oauth = terms.iter().find(|t| t.term == "OAuth").unwrap();

        let auth = oauth
            .locations
            .iter()
            .find(|l| l.file == "auth.md")
            .unwrap();
        assert!(auth.main, "auth.md should be main");

        let old_auth = oauth
            .locations
            .iter()
            .find(|l| l.file == "old-auth.md")
            .unwrap();
        assert!(
            !old_auth.main,
            "old-auth.md must NOT be main (substring overlap)"
        );

        let bak = oauth
            .locations
            .iter()
            .find(|l| l.file == "appendix/auth.md.bak")
            .unwrap();
        assert!(!bak.main, "auth.md.bak must NOT be main");
    }

    #[test]
    fn post_process_auto_aliases_known_term_variants() {
        // Extract consolidates "Bedrock" into "Amazon Bedrock" via known_terms,
        // but the variant forms must be preserved as aliases so render can find
        // the term in source files that say "Bedrock".
        let candidates = CandidatesFile {
            version: 1,
            generated: "2026-03-25T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 2,
            candidates: vec![
                // Extract already consolidated — candidate is the canonical form.
                Candidate {
                    term: "Amazon Bedrock".to_string(),
                    score: 0.9,
                    locations: vec![CandidateLocation {
                        file: "infra.md".to_string(),
                        context: "Bedrock powers our models".to_string(),
                    }],
                },
                Candidate {
                    term: "Google Chrome".to_string(),
                    score: 0.85,
                    locations: vec![CandidateLocation {
                        file: "tools.md".to_string(),
                        context: "Debug with Chrome".to_string(),
                    }],
                },
            ],
        };
        let output = ClaudeOutput {
            terms: vec![
                terms::ClaudeTerm {
                    term: "Amazon Bedrock".to_string(),
                    definition: "AWS foundation model platform.".to_string(),
                    parent: None,
                    aliases: Vec::new(), // Claude didn't add any aliases
                    see_also: Vec::new(),
                    main_files: Vec::new(),
                },
                terms::ClaudeTerm {
                    term: "Google Chrome".to_string(),
                    definition: "Browser debugging support.".to_string(),
                    parent: None,
                    // Claude included "Chrome" — variant should not duplicate.
                    aliases: vec!["Chrome".to_string()],
                    see_also: Vec::new(),
                    main_files: Vec::new(),
                },
            ],
            suggested: Vec::new(),
        };

        let known = vec![
            crate::config::KnownTerm {
                term: "Amazon Bedrock".to_string(),
                variants: vec!["Bedrock".to_string(), "AWS Bedrock".to_string()],
            },
            crate::config::KnownTerm {
                term: "Google Chrome".to_string(),
                variants: vec!["Chrome".to_string()],
            },
        ];

        let terms = post_process(&output, &candidates, 200, &known);

        let bedrock = terms.iter().find(|t| t.term == "Amazon Bedrock").unwrap();
        assert!(
            bedrock.aliases.contains(&"Bedrock".to_string()),
            "known_terms variant 'Bedrock' should be auto-added; got {:?}",
            bedrock.aliases
        );
        assert!(
            bedrock.aliases.contains(&"AWS Bedrock".to_string()),
            "known_terms variant 'AWS Bedrock' should be auto-added; got {:?}",
            bedrock.aliases
        );

        let chrome = terms.iter().find(|t| t.term == "Google Chrome").unwrap();
        let chrome_count = chrome
            .aliases
            .iter()
            .filter(|a| a.to_lowercase() == "chrome")
            .count();
        assert_eq!(
            chrome_count, 1,
            "Chrome should appear exactly once (no duplicate); got {:?}",
            chrome.aliases
        );
    }

    #[test]
    fn post_process_auto_aliases_known_terms_via_alias_match() {
        // Claude renames "Google Chrome" to "Chrome integration" but keeps
        // "Google Chrome" as an alias. The known_terms variant "Chrome" should
        // still be auto-added because the alias matches the known_term name.
        let candidates = CandidatesFile {
            version: 1,
            generated: "2026-03-25T00:00:00Z".to_string(),
            source_dir: "src/".to_string(),
            document_count: 1,
            candidates: vec![Candidate {
                term: "Google Chrome".to_string(),
                score: 0.9,
                locations: vec![CandidateLocation {
                    file: "tools.md".to_string(),
                    context: "Debug with Chrome".to_string(),
                }],
            }],
        };
        let output = ClaudeOutput {
            terms: vec![terms::ClaudeTerm {
                term: "Chrome integration".to_string(),
                definition: "Browser debugging.".to_string(),
                parent: None,
                aliases: vec!["Google Chrome".to_string()],
                see_also: Vec::new(),
                main_files: Vec::new(),
            }],
            suggested: Vec::new(),
        };
        let known = vec![crate::config::KnownTerm {
            term: "Google Chrome".to_string(),
            variants: vec!["Chrome".to_string()],
        }];

        let terms = post_process(&output, &candidates, 200, &known);
        let chrome = terms
            .iter()
            .find(|t| t.term == "Chrome integration")
            .unwrap();
        assert!(
            chrome.aliases.contains(&"Chrome".to_string()),
            "known_terms variant 'Chrome' should be added via alias match; got {:?}",
            chrome.aliases
        );
    }

    #[test]
    fn post_process_inverts_hierarchy() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 200, &[]);

        let auth = terms.iter().find(|t| t.term == "authentication").unwrap();
        assert!(
            auth.children.contains(&"OAuth".to_string()),
            "authentication should have OAuth as child"
        );
        assert!(
            auth.children.contains(&"API key".to_string()),
            "authentication should have API key as child"
        );
    }

    #[test]
    fn post_process_includes_suggested() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 200, &[]);

        let bearer = terms.iter().find(|t| t.term == "bearer token");
        assert!(bearer.is_some(), "suggested term should be in output");
        let bearer = bearer.unwrap();
        assert!(
            bearer.locations.is_empty(),
            "suggested term has no locations"
        );
        assert_eq!(bearer.parent.as_deref(), Some("OAuth"));
    }

    #[test]
    fn post_process_sorts_alphabetically() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 200, &[]);

        for window in terms.windows(2) {
            assert!(
                window[0].term.to_lowercase() <= window[1].term.to_lowercase(),
                "'{}' should come before '{}'",
                window[0].term,
                window[1].term,
            );
        }
    }

    #[test]
    fn post_process_truncates_to_max() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 2, &[]);
        assert_eq!(terms.len(), 2, "should truncate to max_terms");
    }

    #[test]
    fn post_process_deduplicates_location_files() {
        let candidates = sample_candidates();
        let output = sample_claude_output();
        let terms = post_process(&output, &candidates, 200, &[]);

        let oauth = terms.iter().find(|t| t.term == "OAuth").unwrap();
        let auth_count = oauth
            .locations
            .iter()
            .filter(|l| l.file == "auth.md")
            .count();
        assert_eq!(
            auth_count, 1,
            "auth.md should appear only once despite being in OAuth and OAuth 2.0 candidates"
        );
    }
}
