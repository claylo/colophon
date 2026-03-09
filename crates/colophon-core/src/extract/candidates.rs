//! Candidate types for the extraction pipeline.

use serde::{Deserialize, Serialize};

/// A single keyword candidate extracted from the corpus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    /// The keyword or phrase as it appears in text.
    pub term: String,
    /// Relevance score normalized to 0.0..1.0 where higher is better.
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
    /// When this file was generated.
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
    fn candidates_file_roundtrip_yaml() {
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
    fn empty_candidates_file() {
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
