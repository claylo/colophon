//! Configuration loading and discovery.
//!
//! Discovery is delegated to [`librebar::config`]; this module supplies
//! colophon's config schema and pins the application name. Sources are:
//! 1. Walking up from the current directory to find project config
//! 2. Loading user config from XDG config directory
//! 3. Reading `COLOPHON_`-prefixed environment variables
//! 4. Merging with sensible defaults
//!
//! # Supported formats
//!
//! The following configuration file formats are supported:
//! - TOML (`.toml`)
//! - YAML (`.yaml`, `.yml`)
//! - JSON (`.json`)
//!
//! # Config file locations (in order of precedence, highest first):
//! - `.config/colophon.<ext>` in current directory or any parent
//! - `.colophon.<ext>` in current directory or any parent
//! - `colophon.<ext>` in current directory or any parent
//! - `~/.config/colophon/config.<ext>` (user config)
//!
//! Where `<ext>` is one of: `toml`, `yaml`, `yml`, `json`
//!
//! # Example
//! ```no_run
//! use camino::Utf8PathBuf;
//! use colophon_core::config::{Config, ConfigLoader};
//!
//! let cwd = std::env::current_dir().unwrap();
//! let cwd = Utf8PathBuf::try_from(cwd).expect("current directory is not valid UTF-8");
//! let config = ConfigLoader::new()
//!     .with_project_search(&cwd)
//!     .load()
//!     .unwrap();
//! ```

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, ConfigResult};

/// Metadata about which configuration sources were loaded.
///
/// Returned alongside [`Config`] from [`ConfigLoader::load()`] so commands
/// can report the actual config files without re-discovering them.
pub use librebar::config::ConfigSources;

/// Configuration for source file discovery.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct SourceConfig {
    /// Root directory to scan for content files.
    pub dir: String,
    /// File extensions to include (without dots).
    pub extensions: Vec<String>,
    /// File names to exclude from processing.
    pub exclude: Vec<String>,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            dir: ".".to_string(),
            extensions: vec!["md".to_string(), "typ".to_string()],
            exclude: Vec::new(),
        }
    }
}

/// Configuration for the keyword extraction pipeline.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ExtractConfig {
    /// N-gram range for keyword extraction [min, max].
    pub ngram_range: [usize; 2],
    /// Minimum relevance score threshold (0.0 to 1.0).
    pub min_score: f64,
    /// Maximum number of candidates to emit.
    pub max_candidates: usize,
    /// Maximum document percentage for a term (0.0 to 1.0).
    ///
    /// If a term appears in more than this fraction of the corpus, it is
    /// considered definitional (the material is *about* that term) and
    /// excluded from the index. Set to `1.0` to disable.
    pub max_doc_pct: f64,
    /// Stop word configuration for n-gram trimming and TF-IDF.
    pub stop_words: StopWordsConfig,
    /// How exclude_terms matches candidates: `contains` or `exact`.
    pub exclude_terms_match: MatchMode,
    /// Case sensitivity for exclude_terms matching.
    pub exclude_terms_case: CaseSensitivity,
    /// Terms to exclude from extraction results.
    pub exclude_terms: Vec<String>,
    /// Known terms with optional variants — consolidates noisy n-grams.
    pub known_terms: Vec<KnownTerm>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            ngram_range: [1, 3],
            min_score: 0.1,
            max_candidates: 500,
            max_doc_pct: 0.7,
            stop_words: StopWordsConfig::default(),
            exclude_terms_match: MatchMode::default(),
            exclude_terms_case: CaseSensitivity::default(),
            exclude_terms: Vec::new(),
            known_terms: Vec::new(),
        }
    }
}

/// A known term with optional variant spellings.
///
/// If a candidate contains a known term or one of its variants,
/// the candidate is replaced with the canonical term.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct KnownTerm {
    /// The canonical display form of the term.
    pub term: String,
    /// Alternative spellings or abbreviations that map to this term.
    #[serde(default)]
    pub variants: Vec<String>,
}

/// Stop word list configuration.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct StopWordsConfig {
    /// Which stop word corpus to use: `iso` or `nltk`.
    pub source: StopWordSource,
    /// Language code (ISO 639-1, e.g. "en", "de", "fr").
    pub language: String,
}

impl Default for StopWordsConfig {
    fn default() -> Self {
        Self {
            source: StopWordSource::default(),
            language: "en".to_string(),
        }
    }
}

/// Stop word corpus source.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StopWordSource {
    /// Stopwords ISO — 60+ languages.
    #[default]
    Iso,
    /// NLTK stop words — 23 languages, different coverage.
    Nltk,
}

/// How exclusion patterns match against candidate terms.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    /// Candidate is excluded if it contains the exclusion phrase as a substring.
    #[default]
    Contains,
    /// Candidate is excluded only if it matches the exclusion phrase exactly.
    Exact,
    /// Each exclude_terms entry is a regex pattern tested against the full candidate term.
    Regex,
}

/// Case sensitivity mode for string matching.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaseSensitivity {
    /// Exact case match.
    #[default]
    Sensitive,
    /// Case-insensitive match.
    Insensitive,
}

/// Configuration for the Claude-powered curation pipeline.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct CurateConfig {
    /// Claude model to use (passed as `claude --model`).
    pub model: String,
    /// Maximum number of terms in the curated output.
    pub max_terms: usize,
    /// Path to the candidates file (input from extract phase).
    pub candidates: String,
    /// Replace the built-in system prompt entirely.
    /// When set, the default structural instructions are not used.
    pub system_prompt: Option<String>,
    /// Additional guidance appended after the candidates payload.
    /// Use for domain-specific steering (e.g., "this is a book about X").
    pub prompt: Option<String>,
    /// Send full YAML with context snippets instead of compact format.
    pub full_candidates: bool,
    /// Effort level for the Claude CLI (`low`, `medium`, `high`, `max`).
    pub effort: String,
    /// Maximum output tokens per turn (set via `CLAUDE_CODE_MAX_OUTPUT_TOKENS`).
    pub max_output_tokens: u32,
    /// Maximum budget in USD. If set, `curate` aborts when the estimated
    /// cost exceeds this value.
    pub max_budget_usd: Option<f64>,
    /// Settings passed through to the Claude CLI via `--settings`.
    /// Written to a temp file as JSON. Use for `alwaysThinkingEnabled`,
    /// `effortLevel`, `fastMode`, etc.
    pub claude_settings: serde_json::Value,
}

impl Default for CurateConfig {
    fn default() -> Self {
        Self {
            model: "sonnet".to_string(),
            max_terms: 200,
            candidates: "colophon-candidates.yaml".to_string(),
            system_prompt: None,
            prompt: None,
            full_candidates: true,
            effort: "high".to_string(),
            max_output_tokens: 64_000,
            max_budget_usd: None,
            claude_settings: serde_json::json!({}),
        }
    }
}

/// Configuration for the render pipeline.
///
/// Note: distinct from [`crate::render::RenderConfig`], which carries the
/// per-invocation run parameters; this struct is the config-file section.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct RenderConfig {
    /// in-dexter package version written into generated imports
    /// (`#import "@preview/in-dexter:<version>": *`).
    ///
    /// Must match the version the consuming Typst template pins (for
    /// tuftelike: see its backmatter module) — bump here, no colophon
    /// release required.
    pub in_dexter_version: String,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            in_dexter_version: "0.7.2".to_string(),
        }
    }
}

/// The configuration for colophon.
///
/// This struct is deserialized from config files found during discovery
/// (TOML, YAML, or JSON).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Log level for the application (e.g., "debug", "info", "warn", "error").
    pub log_level: LogLevel,
    /// Directory for JSONL log files (falls back to platform defaults if unset).
    pub log_dir: Option<Utf8PathBuf>,
    /// Source file discovery settings.
    pub source: SourceConfig,
    /// Keyword extraction pipeline settings.
    pub extract: ExtractConfig,
    /// Claude curation pipeline settings.
    pub curate: CurateConfig,
    /// Render pipeline settings.
    pub render: RenderConfig,
}

/// Log level configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Verbose output for debugging and development.
    Debug,
    /// Standard operational information (default).
    #[default]
    Info,
    /// Warnings about potential issues.
    Warn,
    /// Errors that indicate failures.
    Error,
}

impl LogLevel {
    /// Returns the log level as a lowercase string slice.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Application name for XDG directory lookup and config file names.
const APP_NAME: &str = "colophon";

/// Translate a librebar loader error into colophon's config error.
fn map_load_error(error: librebar::Error) -> ConfigError {
    match error {
        librebar::Error::ConfigNotFound => ConfigError::NotFound,
        other => ConfigError::Deserialize(Box::new(other)),
    }
}

/// Builder for loading configuration from multiple sources.
///
/// Thin wrapper over [`librebar::config::ConfigLoader`] that pins the
/// application name to `colophon` and the config type to [`Config`].
#[derive(Debug)]
pub struct ConfigLoader {
    inner: librebar::config::ConfigLoader,
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigLoader {
    /// Create a new config loader with default settings.
    pub fn new() -> Self {
        Self {
            inner: librebar::config::ConfigLoader::new(APP_NAME),
        }
    }

    /// Set the starting directory for project config search.
    ///
    /// The loader will walk up from this directory looking for config files.
    pub fn with_project_search<P: AsRef<Utf8Path>>(self, path: P) -> Self {
        Self {
            inner: self.inner.with_project_search(path),
        }
    }

    /// Set whether to include user config from `~/.config/colophon/`.
    pub fn with_user_config(self, include: bool) -> Self {
        Self {
            inner: self.inner.with_user_config(include),
        }
    }

    /// Set a boundary marker to stop directory traversal.
    ///
    /// When walking up directories, stop if we find a directory containing
    /// this file or directory name. Default is `.git`.
    pub fn with_boundary_marker<S: Into<String>>(self, marker: S) -> Self {
        Self {
            inner: self.inner.with_boundary_marker(marker),
        }
    }

    /// Disable boundary marker (search all the way to filesystem root).
    pub fn without_boundary_marker(self) -> Self {
        Self {
            inner: self.inner.without_boundary_marker(),
        }
    }

    /// Add an explicit config file to load.
    ///
    /// Files are loaded in order, with later files taking precedence.
    /// Explicit files are loaded after discovered files.
    pub fn with_file<P: AsRef<Utf8Path>>(self, path: P) -> Self {
        Self {
            inner: self.inner.with_file(path),
        }
    }

    /// Load configuration, merging all discovered sources.
    ///
    /// Returns the merged config alongside metadata about which files
    /// were loaded — pass the [`ConfigSources`] to commands instead of
    /// having them re-discover config files.
    ///
    /// Precedence (highest to lowest):
    /// 1. Explicit files (in order added via `with_file`)
    /// 2. `COLOPHON_`-prefixed environment variables
    /// 3. Project config (closest to search root)
    /// 4. User config (`~/.config/colophon/config.<ext>`)
    /// 5. Default values
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Deserialize`] if a config file cannot be read
    /// or parsed, or if the merged result does not match [`Config`].
    pub fn load(self) -> ConfigResult<(Config, ConfigSources)> {
        self.inner.load::<Config>().map_err(map_load_error)
    }

    /// Load configuration, returning an error if no config source is found.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::NotFound`] if no config file, environment
    /// variable, or override supplied any value.
    pub fn load_or_error(self) -> ConfigResult<(Config, ConfigSources)> {
        self.inner.load_or_error::<Config>().map_err(map_load_error)
    }
}

/// Get the user config directory path.
///
/// Returns `~/.config/colophon/` on Linux, `~/Library/Application Support/colophon/`
/// on macOS, and equivalent on other platforms.
pub fn user_config_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_config_dir(APP_NAME)
}

/// Get the user cache directory path.
///
/// Returns `~/.cache/colophon/` on Linux, `~/Library/Caches/colophon/`
/// on macOS, and equivalent on other platforms.
pub fn user_cache_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_cache_dir(APP_NAME)
}

/// Get the user data directory path.
///
/// Returns `~/.local/share/colophon/` on Linux, `~/Library/Application Support/colophon/`
/// on macOS, and equivalent on other platforms.
pub fn user_data_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_data_dir(APP_NAME)
}

/// Get the local data directory path (machine-specific, not synced).
///
/// Returns `~/.local/share/colophon/` on Linux, `~/Library/Application Support/colophon/`
/// on macOS, and equivalent on other platforms.
pub fn user_data_local_dir() -> Option<Utf8PathBuf> {
    librebar::config::user_data_local_dir(APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(config.log_dir.is_none());
        assert_eq!(config.render.in_dexter_version, "0.7.2");
    }

    #[test]
    fn test_render_in_dexter_version_from_yaml() {
        let config: Config = serde_yaml::from_str("render:\n  in_dexter_version: \"0.8.0\"\n")
            .expect("yaml render section should deserialize");
        assert_eq!(config.render.in_dexter_version, "0.8.0");
        // untouched sections keep defaults
        assert_eq!(config.curate.model, "sonnet");
    }

    #[test]
    fn test_loader_builds_with_defaults() {
        let loader = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker();

        // Should succeed with defaults even if no files found
        let (config, sources) = loader.load().unwrap();
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(sources.primary_file().is_none());
    }

    #[test]
    fn test_single_file_overrides_default() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"log_level = "debug"
log_dir = "/tmp/colophon"
"#,
        )
        .unwrap();

        // Convert to Utf8PathBuf for API call
        let config_path = Utf8PathBuf::try_from(config_path).unwrap();

        let (config, _sources) = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Debug);
        assert_eq!(
            config.log_dir.as_ref().map(|dir| dir.as_str()),
            Some("/tmp/colophon")
        );
    }

    #[test]
    fn test_later_file_overrides_earlier() {
        let tmp = TempDir::new().unwrap();

        let base_config = tmp.path().join("base.toml");
        fs::write(&base_config, r#"log_level = "warn""#).unwrap();

        let override_config = tmp.path().join("override.toml");
        fs::write(&override_config, r#"log_level = "error""#).unwrap();

        // Convert to Utf8PathBuf for API calls
        let base_config = Utf8PathBuf::try_from(base_config).unwrap();
        let override_config = Utf8PathBuf::try_from(override_config).unwrap();

        let (config, _sources) = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&base_config)
            .with_file(&override_config)
            .load()
            .unwrap();

        // Later file wins
        assert_eq!(config.log_level, LogLevel::Error);
    }

    #[test]
    fn test_project_config_discovery() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("project");
        let sub_dir = project_dir.join("src").join("deep");
        fs::create_dir_all(&sub_dir).unwrap();

        // Create config in project root
        let config_path = project_dir.join(".colophon.toml");
        fs::write(&config_path, r#"log_level = "debug""#).unwrap();

        // Convert to Utf8PathBuf for API call
        let sub_dir = Utf8PathBuf::try_from(sub_dir).unwrap();

        // Search from deep subdirectory
        let (config, sources) = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .with_project_search(&sub_dir)
            .load()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Debug);
        assert!(sources.project_file.is_some());
    }

    #[test]
    fn test_dotconfig_directory_takes_precedence() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("project");
        let dotconfig_dir = project_dir.join(".config");
        fs::create_dir_all(&dotconfig_dir).unwrap();

        // Create both .config/project.toml and .project.toml
        fs::write(
            dotconfig_dir.join("colophon.toml"),
            r#"log_level = "debug""#,
        )
        .unwrap();
        fs::write(project_dir.join(".colophon.toml"), r#"log_level = "warn""#).unwrap();

        let project_dir = Utf8PathBuf::try_from(project_dir).unwrap();

        let (config, sources) = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .with_project_search(&project_dir)
            .load()
            .unwrap();

        // .config/ should win over dotfile
        assert_eq!(config.log_level, LogLevel::Debug);
        let found = sources.project_file.unwrap();
        assert!(found.as_str().contains(".config/"));
    }

    #[test]
    fn test_boundary_marker_stops_search() {
        let tmp = TempDir::new().unwrap();

        // Create structure: /parent/config.toml, /parent/child/.git/, /parent/child/work/
        let parent = tmp.path().join("parent");
        let child = parent.join("child");
        let work = child.join("work");
        fs::create_dir_all(&work).unwrap();

        // Config in parent (should NOT be found due to .git boundary)
        fs::write(parent.join(".colophon.toml"), r#"log_level = "warn""#).unwrap();

        // .git marker in child
        fs::create_dir(child.join(".git")).unwrap();

        // Convert to Utf8PathBuf for API call
        let work = Utf8PathBuf::try_from(work).unwrap();

        // Search from work directory - should not find parent config
        let (config, sources) = ConfigLoader::new()
            .with_user_config(false)
            .with_boundary_marker(".git")
            .with_project_search(&work)
            .load()
            .unwrap();

        // Should get default since config is beyond boundary
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(sources.project_file.is_none());
    }

    #[test]
    fn test_explicit_file_overrides_project_config() {
        let tmp = TempDir::new().unwrap();

        // Project config
        let project_config = tmp.path().join(".colophon.toml");
        fs::write(&project_config, r#"log_level = "warn""#).unwrap();

        // Explicit override
        let override_config = tmp.path().join("override.toml");
        fs::write(&override_config, r#"log_level = "error""#).unwrap();

        // Convert to Utf8PathBuf for API calls
        let tmp_path = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        let override_config = Utf8PathBuf::try_from(override_config).unwrap();

        let (config, sources) = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .with_project_search(&tmp_path)
            .with_file(&override_config)
            .load()
            .unwrap();

        // Explicit file wins over project config
        assert_eq!(config.log_level, LogLevel::Error);
        assert!(sources.project_file.is_some());
        assert_eq!(sources.explicit_files.len(), 1);
    }

    #[test]
    fn test_load_or_error_fails_when_no_config() {
        let result = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .load_or_error();

        assert!(matches!(result, Err(ConfigError::NotFound)));
    }

    #[test]
    fn test_load_or_error_succeeds_with_explicit_file() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(&config_path, r#"log_level = "debug""#).unwrap();

        // Convert to Utf8PathBuf for API call
        let config_path = Utf8PathBuf::try_from(config_path).unwrap();

        let (config, _sources) = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load_or_error()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Debug);
    }

    #[test]
    fn test_source_and_extract_config_from_toml() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
log_level = "info"

[source]
dir = "docs/"
extensions = ["md", "mdx"]
exclude = ["README.md"]

[extract]
ngram_range = [1, 4]
min_score = 0.05
max_candidates = 300
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let (config, _) = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        assert_eq!(config.source.dir, "docs/");
        assert_eq!(config.source.extensions, vec!["md", "mdx"]);
        assert_eq!(config.source.exclude, vec!["README.md"]);
        assert_eq!(config.extract.ngram_range, [1, 4]);
        assert_eq!(config.extract.min_score, 0.05);
        assert_eq!(config.extract.max_candidates, 300);
    }

    #[test]
    fn test_source_and_extract_defaults() {
        let config = Config::default();
        assert_eq!(config.source.dir, ".");
        assert_eq!(config.source.extensions, vec!["md", "typ"]);
        assert!(config.source.exclude.is_empty());
        assert_eq!(config.extract.ngram_range, [1, 3]);
        assert_eq!(config.extract.min_score, 0.1);
        assert_eq!(config.extract.max_candidates, 500);
    }

    #[test]
    fn test_user_config_dir() {
        // Should return Some on most systems
        let dir = user_config_dir();
        if let Some(path) = dir {
            assert!(path.as_str().contains("colophon"));
        }
    }
}
