//! Curated term database types and serialization.

use serde::{Deserialize, Serialize};

/// A single curated term for the index/glossary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CuratedTerm {
    /// Canonical display form of the term (properly cased).
    pub term: String,
    /// One-sentence glossary definition.
    pub definition: String,
    /// Broader concept this term belongs under (creates hierarchy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Other surface forms that refer to the same concept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Related terms the reader might also want to look up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub see_also: Vec<String>,
    /// Child terms (populated by inverting `parent` refs in post-processing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    /// Where this term appears in the source documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<TermLocation>,
}

/// Where a curated term appears in the source material.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TermLocation {
    /// Relative path to the source file.
    pub file: String,
    /// Whether this is a substantive discussion (bold page number in index).
    #[serde(default)]
    pub main: bool,
    /// A short snippet of surrounding context.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context: String,
}

/// The full curated term database output file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CuratedTermsFile {
    /// Schema version.
    pub version: u32,
    /// When this file was generated.
    pub generated: String,
    /// Source directory that was scanned.
    pub source_dir: String,
    /// Number of documents in the original corpus.
    pub document_count: usize,
    /// Curated terms, sorted alphabetically.
    pub terms: Vec<CuratedTerm>,
}

impl CuratedTermsFile {
    /// Serialize to YAML string.
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Deserialize from YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}

/// Raw term output from Claude (before post-processing).
///
/// This matches the JSON Schema in `config/curate-schema.yaml`.
/// Locations are NOT included — they are mapped from the candidates
/// file during post-processing.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClaudeOutput {
    /// Curated terms.
    pub terms: Vec<ClaudeTerm>,
    /// Terms Claude spotted that YAKE missed.
    #[serde(default)]
    pub suggested: Vec<ClaudeSuggested>,
}

/// A single term as returned by Claude.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClaudeTerm {
    /// Canonical display form.
    pub term: String,
    /// One-sentence glossary definition.
    pub definition: String,
    /// Parent term for hierarchy.
    pub parent: Option<String>,
    /// Candidate terms that map to this entry.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Related terms.
    #[serde(default)]
    pub see_also: Vec<String>,
    /// Files where this term is substantively discussed.
    #[serde(default)]
    pub main_files: Vec<String>,
}

/// A suggested term that YAKE missed.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClaudeSuggested {
    /// Canonical display form.
    pub term: String,
    /// One-sentence glossary definition.
    pub definition: String,
    /// Parent term for hierarchy.
    pub parent: Option<String>,
}

/// Delta response from Claude in incremental mode.
///
/// Matches the JSON Schema in `config/curate-delta-schema.yaml`.
/// Contains only changes — existing unchanged terms are not included.
#[allow(dead_code)] // used by merge logic in Task 4
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClaudeDeltaOutput {
    /// New terms to add.
    pub additions: Vec<ClaudeTerm>,
    /// Modifications to existing terms (sparse updates).
    pub modifications: Vec<DeltaModification>,
    /// Terms to remove.
    pub removals: Vec<DeltaRemoval>,
    /// Terms Claude spotted that extraction missed.
    #[serde(default)]
    pub suggested: Vec<ClaudeSuggested>,
}

/// A modification to an existing term. Only changed fields are present.
#[allow(dead_code)] // used by merge logic in Task 4
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeltaModification {
    /// Exact name of the existing term being modified.
    pub term: String,
    /// Updated definition (only if changed).
    pub definition: Option<String>,
    /// New parent (only if reparented).
    pub parent: Option<String>,
    /// Updated alias list (replaces existing if present).
    pub aliases: Option<Vec<String>>,
    /// Updated see_also list (replaces existing if present).
    pub see_also: Option<Vec<String>>,
    /// Justification for the change.
    pub reason: String,
}

/// A term to remove from the index.
#[allow(dead_code)] // used by merge logic in Task 4
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeltaRemoval {
    /// Exact name of the term to remove.
    pub term: String,
    /// Why it should be removed.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_terms_file() -> CuratedTermsFile {
        CuratedTermsFile {
            version: 1,
            generated: "2026-03-10T12:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 61,
            terms: vec![
                CuratedTerm {
                    term: "OAuth".to_string(),
                    definition: "An open standard for token-based authorization.".to_string(),
                    parent: Some("authentication".to_string()),
                    aliases: vec!["OAuth 2.0".to_string()],
                    see_also: vec!["API key".to_string()],
                    children: Vec::new(),
                    locations: vec![
                        TermLocation {
                            file: "auth.md".to_string(),
                            main: true,
                            context: "OAuth provides delegated authorization".to_string(),
                        },
                        TermLocation {
                            file: "api.md".to_string(),
                            main: false,
                            context: "requires OAuth tokens".to_string(),
                        },
                    ],
                },
                CuratedTerm {
                    term: "authentication".to_string(),
                    definition: "The process of verifying user or system identity.".to_string(),
                    parent: None,
                    aliases: Vec::new(),
                    see_also: Vec::new(),
                    children: vec!["OAuth".to_string()],
                    locations: vec![TermLocation {
                        file: "auth.md".to_string(),
                        main: true,
                        context: "Authentication verifies identity".to_string(),
                    }],
                },
            ],
        }
    }

    #[test]
    fn terms_file_roundtrip_yaml() {
        let file = sample_terms_file();
        let yaml = file.to_yaml().expect("serialization should succeed");
        let parsed = CuratedTermsFile::from_yaml(&yaml).expect("deserialization should succeed");
        assert_eq!(file, parsed);
    }

    #[test]
    fn terms_file_yaml_structure() {
        let file = sample_terms_file();
        let yaml = file.to_yaml().unwrap();
        assert!(yaml.contains("version: 1"));
        assert!(yaml.contains("term: OAuth"));
        assert!(yaml.contains("parent: authentication"));
        assert!(yaml.contains("main: true"));
        assert!(yaml.contains("aliases:"));
    }

    #[test]
    fn empty_terms_file() {
        let file = CuratedTermsFile {
            version: 1,
            generated: "2026-03-10T12:00:00Z".to_string(),
            source_dir: ".".to_string(),
            document_count: 0,
            terms: Vec::new(),
        };
        let yaml = file.to_yaml().unwrap();
        assert!(yaml.contains("terms: []"));
    }

    #[test]
    fn optional_fields_omitted_in_yaml() {
        let term = CuratedTerm {
            term: "test".to_string(),
            definition: "A test term.".to_string(),
            parent: None,
            aliases: Vec::new(),
            see_also: Vec::new(),
            children: Vec::new(),
            locations: Vec::new(),
        };
        let yaml = serde_yaml::to_string(&term).unwrap();
        assert!(!yaml.contains("parent:"));
        assert!(!yaml.contains("aliases:"));
        assert!(!yaml.contains("see_also:"));
        assert!(!yaml.contains("children:"));
        assert!(!yaml.contains("locations:"));
    }

    #[test]
    fn claude_output_deserializes() {
        let json = r#"{
            "terms": [
                {
                    "term": "OAuth",
                    "definition": "An authorization standard.",
                    "parent": "authentication",
                    "aliases": ["OAuth 2.0"],
                    "see_also": ["API key"],
                    "main_files": ["auth.md"]
                }
            ],
            "suggested": [
                {
                    "term": "bearer token",
                    "definition": "A token type for API access."
                }
            ]
        }"#;
        let output: ClaudeOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.terms.len(), 1);
        assert_eq!(output.terms[0].term, "OAuth");
        assert_eq!(output.terms[0].aliases, vec!["OAuth 2.0"]);
        assert_eq!(output.terms[0].main_files, vec!["auth.md"]);
        assert_eq!(output.suggested.len(), 1);
        assert_eq!(output.suggested[0].term, "bearer token");
    }

    #[test]
    fn claude_output_minimal() {
        let json = r#"{
            "terms": [
                {
                    "term": "test",
                    "definition": "A test."
                }
            ]
        }"#;
        let output: ClaudeOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.terms.len(), 1);
        assert!(output.terms[0].parent.is_none());
        assert!(output.terms[0].aliases.is_empty());
        assert!(output.suggested.is_empty());
    }

    #[test]
    fn delta_output_deserializes() {
        let json = r#"{
            "additions": [
                {
                    "term": "PKCE",
                    "definition": "Proof Key for Code Exchange.",
                    "parent": "OAuth",
                    "aliases": ["Proof Key for Code Exchange"],
                    "see_also": ["OAuth"],
                    "main_files": ["auth.md"]
                }
            ],
            "modifications": [
                {
                    "term": "OAuth",
                    "definition": "Updated definition.",
                    "reason": "PKCE changes the OAuth landscape"
                }
            ],
            "removals": [
                {
                    "term": "deprecated_term",
                    "reason": "No longer in corpus"
                }
            ],
            "suggested": [
                {
                    "term": "bearer token",
                    "definition": "A type of access token."
                }
            ]
        }"#;
        let output: ClaudeDeltaOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.additions.len(), 1);
        assert_eq!(output.additions[0].term, "PKCE");
        assert_eq!(
            output.additions[0].aliases,
            vec!["Proof Key for Code Exchange"]
        );
        assert_eq!(output.modifications.len(), 1);
        assert_eq!(output.modifications[0].term, "OAuth");
        assert!(output.modifications[0].definition.is_some());
        assert_eq!(output.removals.len(), 1);
        assert_eq!(output.removals[0].term, "deprecated_term");
        assert_eq!(output.suggested.len(), 1);
    }

    #[test]
    fn delta_output_empty_arrays() {
        let json = r#"{
            "additions": [],
            "modifications": [],
            "removals": [],
            "suggested": []
        }"#;
        let output: ClaudeDeltaOutput = serde_json::from_str(json).unwrap();
        assert!(output.additions.is_empty());
        assert!(output.modifications.is_empty());
        assert!(output.removals.is_empty());
        assert!(output.suggested.is_empty());
    }

    #[test]
    fn delta_modification_minimal() {
        let json = r#"{
            "additions": [],
            "modifications": [{"term": "OAuth", "reason": "just testing"}],
            "removals": [],
            "suggested": []
        }"#;
        let output: ClaudeDeltaOutput = serde_json::from_str(json).unwrap();
        let m = &output.modifications[0];
        assert!(m.definition.is_none());
        assert!(m.parent.is_none());
        assert!(m.aliases.is_none());
        assert!(m.see_also.is_none());
    }
}
