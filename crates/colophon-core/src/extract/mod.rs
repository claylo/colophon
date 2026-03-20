//! Extraction pipeline -- scan markdown files and extract keyword candidates.

pub mod candidates;
pub mod keywords;
pub mod markdown;
pub mod typst;

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use walkdir::WalkDir;

use crate::config::{ExtractConfig, SourceConfig};
use crate::error::{ExtractError, ExtractResult};

use self::candidates::{Candidate, CandidateLocation, CandidatesFile};
use self::keywords::{extract_tfidf, extract_yake, get_stop_words, trim_stopwords};
use self::markdown::extract_context;

/// A parsed document ready for keyword extraction.
struct Document {
    /// Relative path from the source directory.
    relative_path: String,
    /// Extracted prose text.
    prose: String,
}

/// Run the full extraction pipeline: walk files, extract prose, score keywords,
/// and return a [`CandidatesFile`] ready for serialization.
pub fn run(source: &SourceConfig, extract: &ExtractConfig) -> ExtractResult<CandidatesFile> {
    run_with_progress(source, extract, &indicatif::ProgressBar::hidden())
}

/// Run the extraction pipeline with progress indication.
///
/// Same as [`run`] but updates a [`ProgressBar`](indicatif::ProgressBar) during
/// the per-document YAKE extraction phase (the slowest step).
pub fn run_with_progress(
    source: &SourceConfig,
    extract: &ExtractConfig,
    progress: &indicatif::ProgressBar,
) -> ExtractResult<CandidatesFile> {
    // 1. Collect and read source files.
    let raw_docs = collect_documents(&source.dir, &source.extensions, &source.exclude)?;
    if raw_docs.is_empty() {
        return Err(ExtractError::NoDocuments(source.dir.clone()));
    }
    tracing::debug!(
        raw_count = raw_docs.len(),
        dir = %source.dir,
        extensions = ?source.extensions,
        "collected source files"
    );

    // 2. Parse into prose (dispatch by file extension), skip empties.
    let documents: Vec<Document> = raw_docs
        .into_iter()
        .filter_map(|(path, ext, content)| {
            let prose = extract_prose_for(&content, &ext);
            if prose.is_empty() {
                tracing::debug!(file = %path, ext, "skipped file — no prose after parse");
                None
            } else {
                tracing::debug!(file = %path, ext, prose_len = prose.len(), "parsed prose");
                Some(Document {
                    relative_path: path,
                    prose,
                })
            }
        })
        .collect();

    if documents.is_empty() {
        return Err(ExtractError::NoDocuments(source.dir.clone()));
    }

    let doc_count = documents.len();
    tracing::debug!(doc_count, "documents with extractable prose");

    // Set up progress for the per-document extraction loop.
    progress.set_length(doc_count as u64);
    progress.set_position(0);

    // Find the longest filename for fixed-width padding.
    let max_filename_len = documents
        .iter()
        .map(|d| d.relative_path.len())
        .max()
        .unwrap_or(0);

    // Load stop words once from config.
    let stop_words = get_stop_words(&extract.stop_words);
    tracing::debug!(
        source = ?extract.stop_words.source,
        language = %extract.stop_words.language,
        count = stop_words.len(),
        "loaded stop words"
    );

    // Accumulator: lowercase term -> (best_score, original_term, locations).
    let mut term_map: HashMap<String, (f64, String, Vec<CandidateLocation>)> = HashMap::new();

    // 3. Per-document YAKE extraction.
    for doc in &documents {
        progress.set_message(format!(
            "{:<width$}",
            doc.relative_path,
            width = max_filename_len
        ));
        let yake_keywords =
            extract_yake(&doc.prose, extract.ngram_range[1], extract.max_candidates);
        tracing::debug!(
            file = %doc.relative_path,
            yake_count = yake_keywords.len(),
            ngram_max = extract.ngram_range[1],
            "YAKE extraction"
        );

        for kw in yake_keywords {
            if kw.score < extract.min_score {
                continue;
            }
            // Trim stop words from n-gram edges: "covers ZDR covers" → "ZDR"
            let trimmed = trim_stopwords(&kw.term, &stop_words);
            if trimmed.is_empty() {
                continue;
            }
            let key = trimmed.to_lowercase();
            let location = extract_context(&doc.prose, &trimmed, 60)
                .map(|ctx| CandidateLocation {
                    file: doc.relative_path.clone(),
                    context: ctx,
                })
                .unwrap_or_else(|| CandidateLocation {
                    file: doc.relative_path.clone(),
                    context: String::new(),
                });

            let entry = term_map
                .entry(key)
                .or_insert_with(|| (0.0_f64, trimmed.clone(), Vec::new()));
            if kw.score > entry.0 {
                entry.0 = kw.score;
                entry.1 = trimmed.clone();
            }
            entry.2.push(location);
        }
        progress.inc(1);
    }

    // 4. Corpus-wide TF-IDF.
    progress.set_message("Cross-corpus TF-IDF...");
    let prose_texts: Vec<String> = documents.iter().map(|d| d.prose.clone()).collect();
    let tfidf_keywords = extract_tfidf(&prose_texts, &stop_words, extract.max_candidates);
    tracing::debug!(
        tfidf_count = tfidf_keywords.len(),
        "TF-IDF cross-corpus extraction"
    );

    tracing::debug!(
        unique_terms = term_map.len(),
        "unique terms from YAKE (before TF-IDF merge)"
    );

    for kw in tfidf_keywords {
        if kw.score < extract.min_score {
            continue;
        }
        let key = kw.term.to_lowercase();

        // Find locations for this term across documents.
        let locations: Vec<CandidateLocation> = documents
            .iter()
            .filter_map(|doc| {
                extract_context(&doc.prose, &kw.term, 60).map(|ctx| CandidateLocation {
                    file: doc.relative_path.clone(),
                    context: ctx,
                })
            })
            .collect();

        let entry = term_map
            .entry(key)
            .or_insert_with(|| (0.0_f64, kw.term.clone(), Vec::new()));
        if kw.score > entry.0 {
            entry.0 = kw.score;
        }
        // Merge locations, avoiding duplicate file+context pairs.
        for loc in locations {
            if !entry
                .2
                .iter()
                .any(|existing| existing.file == loc.file && existing.context == loc.context)
            {
                entry.2.push(loc);
            }
        }
    }

    tracing::debug!(unique_terms = term_map.len(), "term map after TF-IDF merge");

    // 5. Consolidate known terms — absorb variants and noisy n-grams.
    if !extract.known_terms.is_empty() {
        tracing::debug!(
            known_count = extract.known_terms.len(),
            "starting known_terms consolidation"
        );
        let mut absorbed_keys: Vec<String> = Vec::new();
        for known in &extract.known_terms {
            let canonical_key = known.term.to_lowercase();

            // Collect all keys that match the canonical term or any variant (as substring).
            let matching_keys: Vec<String> = term_map
                .keys()
                .filter(|key| {
                    // Does this candidate contain the known term or any variant?
                    let all_forms: Vec<String> = std::iter::once(known.term.to_lowercase())
                        .chain(known.variants.iter().map(|v| v.to_lowercase()))
                        .collect();
                    all_forms.iter().any(|form| key.contains(form.as_str()))
                })
                .cloned()
                .collect();

            if matching_keys.is_empty() {
                tracing::debug!(term = %known.term, "known term — no matching candidates");
                continue;
            }
            tracing::debug!(
                term = %known.term,
                matches = matching_keys.len(),
                matched = ?matching_keys,
                "known term — absorbing variants"
            );

            // Merge all matching entries into the canonical term.
            let mut best_score = 0.0_f64;
            let mut merged_locations: Vec<CandidateLocation> = Vec::new();

            for key in &matching_keys {
                if let Some((score, _, locations)) = term_map.remove(key) {
                    best_score = best_score.max(score);
                    for loc in locations {
                        if !merged_locations
                            .iter()
                            .any(|existing| existing.file == loc.file)
                        {
                            merged_locations.push(loc);
                        }
                    }
                    if *key != canonical_key {
                        absorbed_keys.push(key.clone());
                    }
                }
            }

            term_map.insert(
                canonical_key,
                (best_score, known.term.clone(), merged_locations),
            );
        }
        tracing::debug!(
            absorbed = absorbed_keys.len(),
            remaining = term_map.len(),
            "known_terms consolidation complete"
        );
    }

    // 5b. Filter terms that appear in too many documents (definitional terms).
    if extract.max_doc_pct < 1.0 {
        let before = term_map.len();
        term_map.retain(|_, (_, term, locations)| {
            let unique_files = locations
                .iter()
                .map(|loc| &loc.file)
                .collect::<std::collections::HashSet<_>>()
                .len();
            let pct = unique_files as f64 / doc_count as f64;
            let dominated = pct > extract.max_doc_pct;
            if dominated {
                tracing::debug!(
                    term = %term,
                    doc_pct = format_args!("{:.0}%", pct * 100.0),
                    files = unique_files,
                    doc_count,
                    "dropped — exceeds max_doc_pct"
                );
            }
            !dominated
        });
        tracing::debug!(
            before,
            after = term_map.len(),
            dropped = before - term_map.len(),
            max_doc_pct = extract.max_doc_pct,
            "max_doc_pct filtering"
        );
    }

    // 6. Build candidate list, filter excluded terms, sort, truncate.
    use crate::config::{CaseSensitivity, MatchMode};

    // Pre-compile regex patterns if in regex mode.
    let compiled_regexes: Vec<regex::Regex> = if extract.exclude_terms_match == MatchMode::Regex {
        extract
            .exclude_terms
            .iter()
            .filter_map(|pattern| {
                let pat = match extract.exclude_terms_case {
                    CaseSensitivity::Insensitive => format!("(?i){pattern}"),
                    CaseSensitivity::Sensitive => pattern.clone(),
                };
                match regex::Regex::new(&pat) {
                    Ok(re) => Some(re),
                    Err(e) => {
                        tracing::warn!(pattern, error = %e, "invalid exclude_terms regex, skipping");
                        None
                    }
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    if !extract.exclude_terms.is_empty() {
        tracing::debug!(
            mode = ?extract.exclude_terms_match,
            case = ?extract.exclude_terms_case,
            patterns = extract.exclude_terms.len(),
            "applying exclude_terms filter"
        );
    }

    let is_excluded = |term: &str| match extract.exclude_terms_match {
        MatchMode::Regex => compiled_regexes.iter().any(|re| re.is_match(term)),
        _ => extract.exclude_terms.iter().any(|excl| {
            let (t, e) = match extract.exclude_terms_case {
                CaseSensitivity::Sensitive => (term.to_string(), excl.clone()),
                CaseSensitivity::Insensitive => (term.to_lowercase(), excl.to_lowercase()),
            };
            match extract.exclude_terms_match {
                MatchMode::Contains => t.contains(&e),
                MatchMode::Exact => t == e,
                MatchMode::Regex => unreachable!(),
            }
        }),
    };

    let pre_filter_count = term_map.len();
    let mut candidates: Vec<Candidate> = term_map
        .into_values()
        .filter(|(_, term, _)| !is_excluded(term))
        .map(|(score, term, locations)| Candidate {
            term,
            score,
            locations,
        })
        .collect();

    tracing::debug!(
        before = pre_filter_count,
        after = candidates.len(),
        excluded = pre_filter_count - candidates.len(),
        "exclude_terms filtering"
    );

    // Sort by score descending to pick top candidates, then truncate.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let pre_truncate = candidates.len();
    candidates.truncate(extract.max_candidates);
    if pre_truncate > candidates.len() {
        tracing::debug!(
            before = pre_truncate,
            after = candidates.len(),
            max = extract.max_candidates,
            "truncated to max_candidates"
        );
    }

    // Final sort: alphabetical by term (case-insensitive) for readability.
    candidates.sort_by(|a, b| a.term.to_lowercase().cmp(&b.term.to_lowercase()));

    progress.finish_and_clear();

    tracing::debug!(
        candidates = candidates.len(),
        documents = doc_count,
        "extraction pipeline complete"
    );

    // 6. Timestamp.
    let generated = format_timestamp(SystemTime::now());

    Ok(CandidatesFile {
        version: 1,
        generated,
        source_dir: source.dir.clone(),
        document_count: doc_count,
        candidates,
    })
}

/// Walk `dir`, filter by extension and exclude list, read contents.
///
/// Returns `(relative_path, extension, content)` tuples sorted by path.
fn collect_documents(
    dir: &str,
    extensions: &[String],
    exclude: &[String],
) -> ExtractResult<Vec<(String, String, String)>> {
    let base = Path::new(dir);
    let mut docs: Vec<(String, String, String)> = Vec::new();

    for entry in WalkDir::new(dir).sort_by_file_name().into_iter() {
        let entry = entry.map_err(ExtractError::WalkDir)?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Check extension.
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if extensions.iter().any(|allowed| allowed == ext) => ext.to_string(),
            _ => continue,
        };

        // Check exclude list (file name only).
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if exclude.iter().any(|ex| ex == file_name) {
            continue;
        }

        // Relative path from base dir.
        let rel = path
            .strip_prefix(base)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(path).map_err(|source| ExtractError::ReadFile {
            path: rel.clone(),
            source,
        })?;

        docs.push((rel, ext, content));
    }

    Ok(docs)
}

/// Extract prose from content based on file extension.
fn extract_prose_for(content: &str, ext: &str) -> String {
    match ext {
        "typ" => typst::extract_prose(content),
        _ => markdown::extract_prose(content),
    }
}

/// Format a [`SystemTime`] as an RFC 3339-ish UTC timestamp without pulling in
/// chrono. Precision is seconds.
pub(crate) fn format_timestamp(time: SystemTime) -> String {
    let dur = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();

    // Constants for calendar math.
    const SECS_PER_MIN: u64 = 60;
    const SECS_PER_HOUR: u64 = 3600;
    const SECS_PER_DAY: u64 = 86400;

    let total_days = secs / SECS_PER_DAY;
    let day_secs = secs % SECS_PER_DAY;
    let hour = day_secs / SECS_PER_HOUR;
    let minute = (day_secs % SECS_PER_HOUR) / SECS_PER_MIN;
    let second = day_secs % SECS_PER_MIN;

    // Days since 1970-01-01 -> (year, month, day) using the civil calendar algorithm.
    let (year, month, day) = civil_from_days(total_days as i64);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert days since 1970-01-01 to (year, month, day).
///
/// Algorithm from Howard Hinnant's `chrono`-compatible date library.
const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
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
    fn pipeline_produces_candidates() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let result = run(&source, &ExtractConfig::default());
        assert!(result.is_ok(), "pipeline should succeed: {result:?}");

        let file = result.unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.document_count, 3);
        assert!(!file.candidates.is_empty());
    }

    #[test]
    fn pipeline_respects_exclude() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());
        fs::write(tmp.path().join("README.md"), "# Ignore me\n").unwrap();

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: vec!["README.md".to_string()],
        };

        let file = run(&source, &ExtractConfig::default()).unwrap();
        assert_eq!(file.document_count, 3);
    }

    #[test]
    fn pipeline_no_documents_error() {
        let tmp = TempDir::new().unwrap();
        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let result = run(&source, &ExtractConfig::default());
        assert!(result.is_err());
    }

    #[test]
    fn pipeline_candidates_sorted_alphabetically() {
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
                window[0].term.to_lowercase() <= window[1].term.to_lowercase(),
                "'{}' should come before '{}'",
                window[0].term,
                window[1].term,
            );
        }
    }

    #[test]
    fn pipeline_output_serializes_to_yaml() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let file = run(&source, &ExtractConfig::default()).unwrap();
        let yaml = file.to_yaml().expect("should serialize");
        assert!(yaml.contains("version: 1"));
        assert!(yaml.contains("candidates:"));
    }

    #[test]
    fn pipeline_excludes_terms() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // First run without exclusions to find a term that exists.
        let baseline = run(&source, &ExtractConfig::default()).unwrap();
        assert!(!baseline.candidates.is_empty());
        let first_term = baseline.candidates[0].term.clone();

        // Now exclude that term.
        let extract_cfg = ExtractConfig {
            exclude_terms: vec![first_term.clone()],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();
        let has_excluded = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase() == first_term.to_lowercase());
        assert!(
            !has_excluded,
            "excluded term '{first_term}' should not appear"
        );
    }

    #[test]
    fn format_timestamp_produces_rfc3339() {
        let ts = format_timestamp(SystemTime::UNIX_EPOCH);
        assert_eq!(ts, "1970-01-01T00:00:00Z");
    }

    // ── known_terms consolidation ──────────────────────────────────

    /// Helper: corpus where "OAuth" and "OAuth 2.0" appear as extractable terms.
    fn write_oauth_corpus(dir: &Path) {
        fs::write(
            dir.join("intro.md"),
            "# OAuth Overview\n\n\
             OAuth is the industry-standard protocol for authorization. \
             OAuth allows third-party applications to obtain limited access. \
             The OAuth framework specifies several grant types.\n",
        )
        .unwrap();

        fs::write(
            dir.join("v2.md"),
            "# OAuth 2.0\n\n\
             OAuth 2.0 is the latest version of the OAuth protocol. \
             OAuth 2.0 focuses on client developer simplicity. \
             The OAuth 2.0 authorization framework was published as RFC 6749.\n",
        )
        .unwrap();

        fs::write(
            dir.join("tokens.md"),
            "# Token Management\n\n\
             Access tokens are credentials used to access protected resources. \
             Refresh tokens allow clients to obtain new access tokens. \
             Bearer tokens are the most common token type in OAuth systems.\n",
        )
        .unwrap();
    }

    #[test]
    fn known_terms_consolidates_variants() {
        let tmp = TempDir::new().unwrap();
        write_oauth_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Disable max_doc_pct — OAuth is in every doc, and we're testing consolidation.
        let no_filter = ExtractConfig {
            max_doc_pct: 1.0,
            ..ExtractConfig::default()
        };

        // Run without known_terms — expect both "OAuth" and "OAuth 2.0" variants.
        let baseline = run(&source, &no_filter).unwrap();
        let baseline_terms: Vec<&str> = baseline
            .candidates
            .iter()
            .map(|c| c.term.as_str())
            .collect();

        // Verify the baseline has oauth-related terms to consolidate.
        let oauth_related = baseline_terms
            .iter()
            .filter(|t| t.to_lowercase().contains("oauth"))
            .count();
        assert!(
            oauth_related >= 1,
            "baseline should have OAuth-related terms, got: {baseline_terms:?}"
        );

        // Now consolidate everything into canonical "OAuth".
        let extract_cfg = ExtractConfig {
            known_terms: vec![crate::config::KnownTerm {
                term: "OAuth".to_string(),
                variants: vec!["OAuth 2.0".to_string()],
            }],
            max_doc_pct: 1.0,
            ..ExtractConfig::default()
        };
        let consolidated = run(&source, &extract_cfg).unwrap();

        // Should have exactly one "OAuth" entry.
        let oauth_count = consolidated
            .candidates
            .iter()
            .filter(|c| c.term == "OAuth")
            .count();
        assert_eq!(
            oauth_count, 1,
            "should have exactly one canonical OAuth entry"
        );

        // No other candidate should contain "oauth" (variants absorbed).
        let stray_oauth: Vec<&str> = consolidated
            .candidates
            .iter()
            .filter(|c| c.term != "OAuth" && c.term.to_lowercase().contains("oauth"))
            .map(|c| c.term.as_str())
            .collect();
        assert!(
            stray_oauth.is_empty(),
            "variants should be absorbed into canonical term, found: {stray_oauth:?}"
        );
    }

    #[test]
    fn known_terms_preserves_canonical_form() {
        let tmp = TempDir::new().unwrap();
        write_oauth_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Use a specific casing for the canonical term.
        let extract_cfg = ExtractConfig {
            known_terms: vec![crate::config::KnownTerm {
                term: "OAuth 2.0".to_string(),
                variants: vec!["OAuth".to_string()],
            }],
            max_doc_pct: 1.0,
            ..ExtractConfig::default()
        };
        let result = run(&source, &extract_cfg).unwrap();

        // The canonical form should be preserved exactly.
        let has_canonical = result.candidates.iter().any(|c| c.term == "OAuth 2.0");
        assert!(
            has_canonical,
            "canonical form 'OAuth 2.0' should appear in output"
        );
    }

    #[test]
    fn known_terms_no_match_passes_through() {
        let tmp = TempDir::new().unwrap();
        write_oauth_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Known term that doesn't match anything in the corpus.
        let no_filter = ExtractConfig {
            max_doc_pct: 1.0,
            ..ExtractConfig::default()
        };
        let extract_cfg = ExtractConfig {
            known_terms: vec![crate::config::KnownTerm {
                term: "GraphQL".to_string(),
                variants: vec!["GQL".to_string()],
            }],
            max_doc_pct: 1.0,
            ..ExtractConfig::default()
        };
        let baseline = run(&source, &no_filter).unwrap();
        let with_known = run(&source, &extract_cfg).unwrap();

        // Should produce the same candidates (known_terms had no effect).
        assert_eq!(baseline.candidates.len(), with_known.candidates.len());
    }

    #[test]
    fn known_terms_merges_locations() {
        let tmp = TempDir::new().unwrap();
        write_oauth_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let extract_cfg = ExtractConfig {
            known_terms: vec![crate::config::KnownTerm {
                term: "OAuth".to_string(),
                variants: vec!["OAuth 2.0".to_string()],
            }],
            max_doc_pct: 1.0,
            ..ExtractConfig::default()
        };
        let result = run(&source, &extract_cfg).unwrap();

        let oauth = result.candidates.iter().find(|c| c.term == "OAuth");
        assert!(oauth.is_some(), "OAuth should exist in candidates");

        // The consolidated term should have locations from multiple files.
        let oauth = oauth.unwrap();
        assert!(
            oauth.locations.len() >= 2,
            "consolidated OAuth should have locations from multiple files, got {}",
            oauth.locations.len()
        );
    }

    // ── regex exclude mode ─────────────────────────────────────────

    #[test]
    fn exclude_regex_removes_matching_terms() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Get baseline terms.
        let baseline = run(&source, &ExtractConfig::default()).unwrap();
        assert!(!baseline.candidates.is_empty());

        // Exclude anything matching a broad regex.
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Regex,
            exclude_terms: vec![r"(?i)oauth".to_string()],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();

        let has_oauth = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase().contains("oauth"));
        assert!(
            !has_oauth,
            "regex exclude should remove all OAuth-related terms"
        );
    }

    #[test]
    fn exclude_regex_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Regex with case-insensitive flag via config.
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Regex,
            exclude_terms_case: crate::config::CaseSensitivity::Insensitive,
            exclude_terms: vec![r"oauth".to_string()],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();

        let has_oauth = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase().contains("oauth"));
        assert!(
            !has_oauth,
            "case-insensitive regex should match OAuth/oauth/OAUTH"
        );
    }

    #[test]
    fn exclude_regex_anchored_pattern() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Anchored regex — only matches terms starting with "OAuth".
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Regex,
            exclude_terms_case: crate::config::CaseSensitivity::Insensitive,
            exclude_terms: vec![r"^oauth$".to_string()],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();

        // "oauth" exactly should be gone, but "OAuth 2.0" or "OAuth tokens" might survive.
        let exact_oauth = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase() == "oauth");
        assert!(!exact_oauth, "anchored regex should exclude exact 'OAuth'");
    }

    #[test]
    fn exclude_regex_invalid_pattern_skipped() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Invalid regex pattern — should be skipped with a warning, not panic.
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Regex,
            exclude_terms: vec![r"[invalid".to_string()],
            ..ExtractConfig::default()
        };
        let result = run(&source, &extract_cfg);
        assert!(
            result.is_ok(),
            "invalid regex should be skipped, not cause pipeline failure"
        );
    }

    #[test]
    fn exclude_regex_multiple_patterns() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let baseline = run(&source, &ExtractConfig::default()).unwrap();

        // Exclude with multiple regex patterns.
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Regex,
            exclude_terms_case: crate::config::CaseSensitivity::Insensitive,
            exclude_terms: vec![r"oauth".to_string(), r"tls".to_string()],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();

        let has_oauth = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase().contains("oauth"));
        let has_tls = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase().contains("tls"));
        assert!(!has_oauth, "regex should exclude OAuth terms");
        assert!(!has_tls, "regex should exclude TLS terms");
        assert!(
            filtered.candidates.len() < baseline.candidates.len(),
            "filtered output should have fewer candidates"
        );
    }

    // ── exact exclude mode ─────────────────────────────────────────

    #[test]
    fn exclude_exact_only_matches_full_term() {
        let tmp = TempDir::new().unwrap();
        write_test_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        let baseline = run(&source, &ExtractConfig::default()).unwrap();
        assert!(!baseline.candidates.is_empty());
        let first_term = baseline.candidates[0].term.clone();

        // Exact mode with a partial string should NOT exclude.
        let partial = if first_term.len() > 2 {
            first_term[..first_term.len() / 2].to_string()
        } else {
            "zzz_no_match".to_string()
        };
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Exact,
            exclude_terms: vec![partial],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();

        // The first term should still be present (partial didn't match in exact mode).
        let still_present = filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase() == first_term.to_lowercase());
        assert!(
            still_present,
            "exact mode should not exclude on partial match"
        );

        // But exact match should exclude.
        let extract_cfg = ExtractConfig {
            exclude_terms_match: crate::config::MatchMode::Exact,
            exclude_terms: vec![first_term.clone()],
            ..ExtractConfig::default()
        };
        let filtered = run(&source, &extract_cfg).unwrap();
        let gone = !filtered
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase() == first_term.to_lowercase());
        assert!(gone, "exact mode should exclude on exact match");
    }

    // ── max_doc_pct filtering ──────────────────────────────────────

    /// Corpus where "OAuth" appears in every document (definitional term).
    fn write_oauth_heavy_corpus(dir: &Path) {
        fs::write(
            dir.join("ch1.md"),
            "OAuth provides delegated authorization. \
             OAuth allows third-party access to resources.\n",
        )
        .unwrap();
        fs::write(
            dir.join("ch2.md"),
            "OAuth tokens must be validated on every request. \
             OAuth scopes limit what a token can do.\n",
        )
        .unwrap();
        fs::write(
            dir.join("ch3.md"),
            "OAuth refresh tokens extend session lifetime. \
             OAuth clients should store tokens securely.\n",
        )
        .unwrap();
        // One doc about something else entirely.
        fs::write(
            dir.join("ch4.md"),
            "Rate limiting protects backend services from abuse. \
             Throttling algorithms include token bucket and leaky bucket.\n",
        )
        .unwrap();
    }

    #[test]
    fn max_doc_pct_drops_ubiquitous_terms() {
        let tmp = TempDir::new().unwrap();
        write_oauth_heavy_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // "OAuth" appears in 3/4 docs = 75%. With max_doc_pct=0.7, it should be dropped.
        let extract_cfg = ExtractConfig {
            max_doc_pct: 0.7,
            ..ExtractConfig::default()
        };
        let result = run(&source, &extract_cfg).unwrap();

        let has_oauth = result
            .candidates
            .iter()
            .any(|c| c.term.to_lowercase() == "oauth");
        assert!(
            !has_oauth,
            "term in 75% of docs should be dropped at max_doc_pct=0.7"
        );
    }

    #[test]
    fn max_doc_pct_keeps_sparse_terms() {
        let tmp = TempDir::new().unwrap();
        write_oauth_heavy_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // "rate limiting" only in 1/4 docs = 25%, should survive at 0.7.
        let extract_cfg = ExtractConfig {
            max_doc_pct: 0.7,
            ..ExtractConfig::default()
        };
        let result = run(&source, &extract_cfg).unwrap();
        assert!(
            !result.candidates.is_empty(),
            "sparse terms should survive max_doc_pct filter"
        );
    }

    #[test]
    fn max_doc_pct_disabled_at_one() {
        let tmp = TempDir::new().unwrap();
        write_oauth_heavy_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Default (0.7) drops ubiquitous terms; 1.0 keeps everything.
        let default_result = run(&source, &ExtractConfig::default()).unwrap();
        let unfiltered = run(
            &source,
            &ExtractConfig {
                max_doc_pct: 1.0,
                ..ExtractConfig::default()
            },
        )
        .unwrap();

        assert!(
            unfiltered.candidates.len() >= default_result.candidates.len(),
            "max_doc_pct=1.0 should keep at least as many candidates as default (0.7)"
        );
    }

    #[test]
    fn max_doc_pct_respects_consolidated_locations() {
        let tmp = TempDir::new().unwrap();
        write_oauth_heavy_corpus(tmp.path());

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string()],
            exclude: Vec::new(),
        };

        // Use known_terms to consolidate OAuth variants, then apply max_doc_pct.
        // After consolidation, the canonical term spans more docs.
        let extract_cfg = ExtractConfig {
            max_doc_pct: 0.7,
            known_terms: vec![crate::config::KnownTerm {
                term: "OAuth".to_string(),
                variants: vec!["OAuth tokens".to_string(), "OAuth scopes".to_string()],
            }],
            ..ExtractConfig::default()
        };
        let result = run(&source, &extract_cfg).unwrap();

        // The consolidated OAuth entry spans 3/4 docs, should be dropped at 0.7.
        let has_oauth = result.candidates.iter().any(|c| c.term == "OAuth");
        assert!(
            !has_oauth,
            "consolidated term exceeding max_doc_pct should be dropped"
        );
    }

    // ── Typst extraction ─────────────────────────────────────────

    #[test]
    fn pipeline_extracts_from_typst() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("auth.typ"),
            "= Authentication\n\n\
             OAuth provides delegated authorization. OAuth 2.0 is the \
             current standard for token-based access control.\n\n\
             == Passwords\n\nPassword hashing uses *bcrypt* or _argon2_.\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("api.typ"),
            "= API Design\n\n\
             RESTful APIs use HTTP methods for CRUD operations. \
             Rate limiting protects against abuse.\n",
        )
        .unwrap();

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["typ".to_string()],
            exclude: Vec::new(),
        };
        let result = run(&source, &ExtractConfig::default());
        assert!(result.is_ok(), "typst pipeline should succeed: {result:?}");

        let file = result.unwrap();
        assert_eq!(file.document_count, 2);
        assert!(!file.candidates.is_empty());
    }

    #[test]
    fn pipeline_mixed_md_and_typst() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("intro.md"),
            "# Introduction\n\nOAuth provides delegated authorization.\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("details.typ"),
            "= Details\n\nTransport Layer Security encrypts data in transit.\n",
        )
        .unwrap();

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["md".to_string(), "typ".to_string()],
            exclude: Vec::new(),
        };
        let result = run(&source, &ExtractConfig::default());
        assert!(result.is_ok(), "mixed pipeline should succeed: {result:?}");

        let file = result.unwrap();
        assert_eq!(file.document_count, 2);
        assert!(!file.candidates.is_empty());
    }

    #[test]
    fn pipeline_typst_skips_code_and_math() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("ch1.typ"),
            "= Chapter 1\n\n\
             OAuth provides delegated authorization.\n\n\
             The formula $x^2 + y^2 = z^2$ is Pythagoras.\n\n\
             ```rust\nfn main() {}\n```\n\n\
             TLS encrypts data in transit.\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("ch2.typ"),
            "= Chapter 2\n\n\
             Rate limiting protects against abuse.\n",
        )
        .unwrap();

        let source = SourceConfig {
            dir: tmp.path().to_string_lossy().to_string(),
            extensions: vec!["typ".to_string()],
            exclude: Vec::new(),
        };
        let file = run(&source, &ExtractConfig::default()).unwrap();

        // Code and math should not appear in candidates.
        for c in &file.candidates {
            assert!(
                !c.term.contains("fn main"),
                "code should not appear in candidates"
            );
            assert!(
                !c.term.contains("x^2"),
                "math should not appear in candidates"
            );
        }
    }
}
