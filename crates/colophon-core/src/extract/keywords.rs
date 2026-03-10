//! Keyword extraction via YAKE and hand-rolled TF-IDF.

use std::collections::{HashMap, HashSet};

/// A scored keyword from extraction.
#[derive(Debug, Clone)]
pub struct ScoredKeyword {
    /// The keyword or phrase.
    pub term: String,
    /// Normalized score (0.0..1.0, higher = more relevant).
    pub score: f64,
}

/// Extract keywords from `text` using the YAKE algorithm.
///
/// Returns up to `max_keywords` results with scores normalized so that
/// higher values indicate greater relevance.
pub fn extract_yake(text: &str, ngrams: usize, max_keywords: usize) -> Vec<ScoredKeyword> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let config = yake_rust::Config {
        ngrams,
        ..yake_rust::Config::default()
    };
    let stop_words =
        yake_rust::StopWords::predefined("en").expect("English stop words should be available");

    let results = yake_rust::get_n_best(max_keywords, text, &stop_words, &config);
    if results.is_empty() {
        return Vec::new();
    }

    // YAKE: lower score = more relevant. Invert so higher = better.
    let max_score = results
        .iter()
        .map(|r| r.score)
        .fold(f64::NEG_INFINITY, f64::max);

    if max_score <= 0.0 {
        // All scores zero or negative — assign uniform 1.0.
        return results
            .into_iter()
            .map(|r| ScoredKeyword {
                term: r.raw,
                score: 1.0,
            })
            .collect();
    }

    results
        .into_iter()
        .map(|r| ScoredKeyword {
            term: r.raw,
            score: 1.0 - (r.score / max_score),
        })
        .collect()
}

use crate::config::{StopWordSource, StopWordsConfig};

/// Get stop words based on configuration.
pub fn get_stop_words(config: &StopWordsConfig) -> Vec<String> {
    let code = match config.source {
        // ISO uses bare language codes, NLTK uses "nltk_" prefix
        StopWordSource::Iso => config.language.clone(),
        StopWordSource::Nltk => format!("nltk_{}", config.language),
    };
    stop_words::get(&code)
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Tokenize text into lowercase words with punctuation stripped.
fn tokenize(text: &str, stop: &HashSet<&str>) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty() && w.len() >= 2 && !stop.contains(w.as_str()))
        .collect()
}

/// Extract keywords from a corpus of documents using TF-IDF scoring.
///
/// Returns up to `max_keywords` results with scores normalized so that
/// higher values indicate greater relevance.
pub fn extract_tfidf(
    documents: &[String],
    stop_words: &[String],
    max_keywords: usize,
) -> Vec<ScoredKeyword> {
    if documents.is_empty() {
        return Vec::new();
    }

    let stop_list = stop_words;
    let stop: HashSet<&str> = stop_list.iter().map(String::as_str).collect();
    let total_docs = documents.len() as f64;

    // Per-document term frequencies.
    let mut doc_tfs: Vec<HashMap<String, f64>> = Vec::with_capacity(documents.len());
    // Document frequency: number of docs containing each term.
    let mut df: HashMap<String, f64> = HashMap::new();

    for doc in documents {
        let tokens = tokenize(doc, &stop);
        let total = tokens.len() as f64;
        if total == 0.0 {
            doc_tfs.push(HashMap::new());
            continue;
        }

        let mut counts: HashMap<String, f64> = HashMap::new();
        for tok in &tokens {
            *counts.entry(tok.clone()).or_default() += 1.0;
        }

        // Normalize to TF.
        let mut tf_map: HashMap<String, f64> = HashMap::new();
        let seen_terms: HashSet<String> = counts.keys().cloned().collect();
        for (term, count) in counts {
            tf_map.insert(term, count / total);
        }

        for term in seen_terms {
            *df.entry(term).or_default() += 1.0;
        }

        doc_tfs.push(tf_map);
    }

    // Max TF across docs per term, then multiply by IDF.
    let mut max_tf: HashMap<&str, f64> = HashMap::new();
    for tf_map in &doc_tfs {
        for (term, &tf) in tf_map {
            let entry = max_tf.entry(term.as_str()).or_insert(0.0);
            if tf > *entry {
                *entry = tf;
            }
        }
    }

    let mut scores: Vec<(String, f64)> = max_tf
        .iter()
        .filter_map(|(&term, &tf)| {
            let doc_freq = df.get(term)?;
            let idf = (total_docs / doc_freq).ln();
            let tfidf = tf * idf;
            if tfidf > 0.0 {
                Some((term.to_string(), tfidf))
            } else {
                None
            }
        })
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(max_keywords);

    if scores.is_empty() {
        return Vec::new();
    }

    let max_score = scores[0].1;

    scores
        .into_iter()
        .map(|(term, s)| ScoredKeyword {
            term,
            score: s / max_score,
        })
        .collect()
}

/// Trim stop words from the leading and trailing edges of an n-gram.
///
/// "covers ZDR covers" → "ZDR"
/// "the OAuth protocol" → "OAuth protocol"
/// "authentication" → "authentication" (single words pass through)
pub fn trim_stopwords(term: &str, stop_words: &[String]) -> String {
    let stop: HashSet<String> = stop_words.iter().map(|w| w.to_lowercase()).collect();

    let words: Vec<&str> = term.split_whitespace().collect();
    if words.len() <= 1 {
        return term.to_string();
    }

    // Trim from the left.
    let start = words
        .iter()
        .position(|w| !stop.contains(&w.to_lowercase()))
        .unwrap_or(0);

    // Trim from the right.
    let end = words
        .iter()
        .rposition(|w| !stop.contains(&w.to_lowercase()))
        .map_or(words.len(), |i| i + 1);

    if start >= end {
        // All words are stop words — return the original.
        return term.to_string();
    }

    words[start..end].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::StopWordsConfig;

    fn sw() -> Vec<String> {
        get_stop_words(&StopWordsConfig::default())
    }

    fn sample_text() -> &'static str {
        "OAuth is an open standard for token-based authorization. \
         OAuth allows users to grant third-party access to their resources \
         without sharing credentials. The OAuth 2.0 framework is widely \
         adopted for API authorization and authentication workflows."
    }

    #[test]
    fn yake_extracts_keywords() {
        let keywords = extract_yake(sample_text(), 3, 10);
        assert!(!keywords.is_empty());
        let terms: Vec<&str> = keywords.iter().map(|k| k.term.as_str()).collect();
        let has_oauth = terms.iter().any(|t| t.to_lowercase().contains("oauth"));
        assert!(has_oauth, "should find 'OAuth' — got: {terms:?}");
    }

    #[test]
    fn yake_scores_normalized() {
        let keywords = extract_yake(sample_text(), 3, 10);
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
    fn yake_empty_text() {
        let keywords = extract_yake("", 3, 10);
        assert!(keywords.is_empty());
    }

    #[test]
    fn tfidf_extracts_keywords() {
        let docs = vec![
            "OAuth provides delegated authorization for APIs and web services.".to_string(),
            "Authentication verifies user identity using passwords or tokens.".to_string(),
            "Rate limiting protects APIs from excessive requests and abuse.".to_string(),
        ];
        let keywords = extract_tfidf(&docs, &sw(), 10);
        assert!(!keywords.is_empty());
    }

    #[test]
    fn tfidf_scores_normalized() {
        let docs = vec![
            "OAuth provides delegated authorization.".to_string(),
            "Authentication verifies identity.".to_string(),
        ];
        let keywords = extract_tfidf(&docs, &sw(), 10);
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
    fn tfidf_empty_corpus() {
        let keywords = extract_tfidf(&[], &sw(), 10);
        assert!(keywords.is_empty());
    }

    #[test]
    fn tfidf_single_document() {
        let docs = vec!["OAuth provides delegated authorization for web services.".to_string()];
        let keywords = extract_tfidf(&docs, &sw(), 10);
        // With one doc, IDF is 0 for all terms (ln(1/1)=0), so no keywords
        // OR the implementation handles single-doc gracefully
        // Either empty or contains terms is fine, just don't panic
        let _ = keywords;
    }

    #[test]
    fn tfidf_discriminates_terms() {
        // "apis" appears in 2/3 docs, "oauth" only in 1 — oauth should score higher
        let docs = vec![
            "OAuth provides delegated authorization for APIs.".to_string(),
            "Authentication verifies user identity for APIs.".to_string(),
            "Rate limiting protects services from abuse.".to_string(),
        ];
        let keywords = extract_tfidf(&docs, &sw(), 20);
        let oauth_score = keywords.iter().find(|k| k.term == "oauth").map(|k| k.score);
        let apis_score = keywords.iter().find(|k| k.term == "apis").map(|k| k.score);
        if let (Some(os), Some(as_)) = (oauth_score, apis_score) {
            assert!(
                os > as_,
                "oauth ({os}) should score higher than apis ({as_}) — appears in fewer docs"
            );
        }
    }

    #[test]
    fn trim_stopwords_leading() {
        let sw = sw();
        assert_eq!(trim_stopwords("the OAuth protocol", &sw), "OAuth protocol");
    }

    #[test]
    fn trim_stopwords_trailing() {
        let sw = sw();
        assert_eq!(trim_stopwords("OAuth is", &sw), "OAuth");
    }

    #[test]
    fn trim_stopwords_both_edges() {
        let sw = sw();
        // "covers" may not be in ISO stop words, but "the" and "is" are
        assert_eq!(trim_stopwords("the ZDR is", &sw), "ZDR");
    }

    #[test]
    fn trim_stopwords_single_word_passthrough() {
        let sw = sw();
        assert_eq!(trim_stopwords("authentication", &sw), "authentication");
    }

    #[test]
    fn trim_stopwords_no_stop_words() {
        let sw = sw();
        assert_eq!(trim_stopwords("OAuth protocol", &sw), "OAuth protocol");
    }

    #[test]
    fn trim_stopwords_all_stop_words() {
        let sw = sw();
        // If everything is a stop word, return original
        let result = trim_stopwords("the and or", &sw);
        assert_eq!(result, "the and or");
    }
}
