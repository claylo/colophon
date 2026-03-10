//! Extraction pipeline -- scan markdown files and extract keyword candidates.

pub mod candidates;
pub mod keywords;
pub mod markdown;

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use walkdir::WalkDir;

use crate::config::{ExtractConfig, SourceConfig};
use crate::error::{ExtractError, ExtractResult};

use self::candidates::{Candidate, CandidateLocation, CandidatesFile};
use self::keywords::{extract_tfidf, extract_yake, get_stop_words, trim_stopwords};
use self::markdown::{extract_context, extract_prose};

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
    // 1. Collect and read source files.
    let raw_docs = collect_documents(&source.dir, &source.extensions, &source.exclude)?;
    if raw_docs.is_empty() {
        return Err(ExtractError::NoDocuments(source.dir.clone()));
    }

    // 2. Parse markdown into prose, skip empties.
    let documents: Vec<Document> = raw_docs
        .into_iter()
        .filter_map(|(path, content)| {
            let prose = extract_prose(&content);
            if prose.is_empty() {
                None
            } else {
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

    // Load stop words once from config.
    let stop_words = get_stop_words(&extract.stop_words);

    // Accumulator: lowercase term -> (best_score, original_term, locations).
    let mut term_map: HashMap<String, (f64, String, Vec<CandidateLocation>)> = HashMap::new();

    // 3. Per-document YAKE extraction.
    for doc in &documents {
        let yake_keywords =
            extract_yake(&doc.prose, extract.ngram_range[1], extract.max_candidates);

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
    }

    // 4. Corpus-wide TF-IDF.
    let prose_texts: Vec<String> = documents.iter().map(|d| d.prose.clone()).collect();
    let tfidf_keywords = extract_tfidf(&prose_texts, &stop_words, extract.max_candidates);

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

    // 5. Build candidate list, filter excluded terms, sort, truncate.
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

    let mut candidates: Vec<Candidate> = term_map
        .into_values()
        .filter(|(_, term, _)| !is_excluded(term))
        .map(|(score, term, locations)| Candidate {
            term,
            score,
            locations,
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(extract.max_candidates);

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
/// Returns `(relative_path, content)` pairs sorted by path.
fn collect_documents(
    dir: &str,
    extensions: &[String],
    exclude: &[String],
) -> ExtractResult<Vec<(String, String)>> {
    let base = Path::new(dir);
    let mut docs: Vec<(String, String)> = Vec::new();

    for entry in WalkDir::new(dir).sort_by_file_name().into_iter() {
        let entry = entry.map_err(ExtractError::WalkDir)?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Check extension.
        let ext_match = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| extensions.iter().any(|allowed| allowed == ext));
        if !ext_match {
            continue;
        }

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

        docs.push((rel, content));
    }

    Ok(docs)
}

/// Format a [`SystemTime`] as an RFC 3339-ish UTC timestamp without pulling in
/// chrono. Precision is seconds.
fn format_timestamp(time: SystemTime) -> String {
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
    fn pipeline_candidates_sorted_by_score() {
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
                "{} ({}) should be >= {} ({})",
                window[0].term,
                window[0].score,
                window[1].term,
                window[1].score,
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
}
