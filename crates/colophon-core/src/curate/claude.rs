//! Shell out to the `claude` CLI for AI-powered curation.
//!
//! Uses the Claude CLI in headless mode with streaming JSON output.
//! All agent runtime overhead (MCP servers, plugins, tools, slash commands)
//! is disabled via explicit flags so the CLI acts as a thin API passthrough.
//!
//! The response is parsed from JSONL stream events, accumulating
//! structured output across multiple turns if needed (the last valid
//! JSON object wins).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use indicatif::ProgressBar;

use crate::config::CurateConfig;
use crate::error::CurateError;
use crate::extract::candidates::CandidatesFile;

use super::cost::TokenUsage;
use super::terms::ClaudeOutput;

/// The built-in system prompt for curation (base, before input format suffix).
///
/// Replaced entirely if `curate.system_prompt` is set in config.
const DEFAULT_SYSTEM_PROMPT_BASE: &str = r#"You are a professional book indexer and glossary editor. You receive raw keyword candidates extracted from a technical documentation corpus and produce a curated index.

Your job:

1. **Keep** terms that a reader would actually look up in a back-of-book index. These are specific concepts, tools, protocols, configuration options, and named entities.

2. **Kill** noise: sentence fragments ("Code makes Git"), generic words ("time", "set", "create"), and terms that are too vague to be useful index entries. But err on the side of inclusion — post-processing will filter further. If a reader might plausibly look it up, keep it.

3. **Merge** synonyms and variants into a single canonical entry. List the absorbed forms as aliases. Aliases MUST be exact strings from the candidate input — they are used as join keys to map source locations in post-processing.

4. **Hierarchy**: assign a `parent` when a term is clearly a subtype of a broader concept. Example: "OAuth" → parent "authentication". Don't force hierarchy where it doesn't exist.

5. **Definitions**: write a one-sentence glossary definition for each term. Should be understandable without reading the source material. ~20-30 words max.

6. **see_also**: link related terms the reader might also want. Only use terms that exist in your output.

7. **main_files**: for each term, identify which source files contain substantive discussion (not just a passing mention). These become bold page numbers in the printed index.

8. **Suggest** important terms the extraction missed, if any are obvious from the file names and term context. Be comprehensive — suggest every term a reader might look up that isn't already in the candidate list."#;

/// Input format description appended to system prompt for compact mode.
const INPUT_FORMAT_COMPACT: &str = "\n\nInput format: each line is `term | score | file1, file2, ...`\nScore range is 0.0-1.0 (higher = more relevant). Files are where the term was found.";

/// Input format description appended to system prompt for full YAML mode.
const INPUT_FORMAT_FULL: &str = "\n\nInput format: YAML with fields per candidate: term, score (0.0-1.0, higher = more relevant), and locations (file + context snippet showing surrounding prose). Use the context snippets to write better definitions and identify main_files.";

/// JSON Schema for the curate output, embedded at compile time.
///
/// Sourced from `config/curate-schema.yaml` converted to JSON.
/// To regenerate: `yj < config/curate-schema.yaml > config/curate-schema.json`
const SCHEMA_JSON: &str = include_str!("../../../../config/curate-schema.json");

/// JSON Schema for the curate delta output (incremental mode).
// Used by incremental invoke path (Task 6) and tests.
#[allow(dead_code)]
const DELTA_SCHEMA_JSON: &str = include_str!("../../../../config/curate-delta-schema.json");

/// Default instruction appended to stdin payload.
const DEFAULT_INSTRUCTION: &str = "Curate the above candidates into a back-of-book index. Follow the system prompt instructions exactly. Be comprehensive.";

/// Result of a Claude CLI invocation.
pub(super) struct InvokeResult {
    /// Parsed structured output from the last valid turn.
    pub output: ClaudeOutput,
    /// Accumulated thinking output across all turns.
    pub thinking: String,
    /// Editorial summary text (non-tool text blocks).
    pub editorial: String,
    /// Number of API turns used.
    pub turns: usize,
    /// Number of thinking token deltas received.
    pub thinking_tokens: usize,
    /// Actual token usage accumulated from stream events.
    pub usage: TokenUsage,
}

/// Return the system prompt that would be sent to Claude.
///
/// Used by `estimate_cost` to count tokens without invoking the CLI.
pub(super) fn system_prompt_for(config: &CurateConfig) -> String {
    build_system_prompt(config)
}

/// Return the stdin payload that would be sent to Claude.
///
/// Used by `estimate_cost` to count tokens without invoking the CLI.
pub(super) fn stdin_payload_for(
    config: &CurateConfig,
    candidates: &CandidatesFile,
    candidates_yaml: &str,
) -> String {
    build_stdin_payload(config, candidates, candidates_yaml)
}

/// Return the JSON schema string embedded at compile time.
pub(super) const fn schema_json() -> &'static str {
    SCHEMA_JSON
}

/// Build a compact representation of candidates for the prompt.
///
/// Format: `term | score | file1, file2, ...`
///
/// This strips context snippets to keep the token count low.
/// A 500-candidate file goes from ~640KB YAML to ~25KB compact text.
fn build_compact_candidates(candidates: &CandidatesFile) -> String {
    let mut out = String::with_capacity(candidates.candidates.len() * 60);
    out.push_str(&format!(
        "Source: {} ({} documents)\n\n",
        candidates.source_dir, candidates.document_count
    ));
    for c in &candidates.candidates {
        let files: Vec<&str> = c.locations.iter().map(|l| l.file.as_str()).collect();
        out.push_str(&format!(
            "{} | {:.2} | {}\n",
            c.term,
            c.score,
            files.join(", ")
        ));
    }
    out
}

/// Build the system prompt with input format suffix based on mode.
fn build_system_prompt(config: &CurateConfig) -> String {
    let base = config
        .system_prompt
        .as_deref()
        .unwrap_or(DEFAULT_SYSTEM_PROMPT_BASE);

    let format_suffix = if config.full_candidates {
        INPUT_FORMAT_FULL
    } else {
        INPUT_FORMAT_COMPACT
    };

    format!("{base}{format_suffix}")
}

/// Build the stdin payload: candidates + optional user prompt + instruction.
fn build_stdin_payload(
    config: &CurateConfig,
    candidates: &CandidatesFile,
    candidates_yaml: &str,
) -> String {
    let payload = if config.full_candidates {
        candidates_yaml.to_string()
    } else {
        build_compact_candidates(candidates)
    };

    let instruction = config
        .prompt
        .as_deref()
        .map(|p| format!("{p}\n\n{DEFAULT_INSTRUCTION}"))
        .unwrap_or_else(|| DEFAULT_INSTRUCTION.to_string());

    format!("{payload}\n\n{instruction}")
}

/// The incremental system prompt base.
// Used by build_incremental_system_prompt (wired in Task 6) and tests.
#[allow(dead_code)]
const DEFAULT_INCREMENTAL_SYSTEM_PROMPT_BASE: &str = r#"You are a professional book indexer updating an existing back-of-book index with new candidate terms.

EXISTING INDEX (for context — do NOT regenerate these):
"#;

/// Instructions appended after the existing index in incremental mode.
// Used by build_incremental_system_prompt (wired in Task 6) and tests.
#[allow(dead_code)]
const INCREMENTAL_INSTRUCTIONS: &str = r#"
Instructions:
1. For each new candidate: add as new term, merge as alias of existing, or discard
2. For stale terms: recommend keep (suggested/conceptual) or remove
3. If a new term should be a child of an existing term, set parent accordingly
4. If new terms create new see_also relationships with existing terms, include the modification
5. Return ONLY additions, modifications, and removals — not unchanged terms
6. Every modification MUST include a reason"#;

/// Build the incremental system prompt including the compact existing index.
// Called by system_prompt_for_incremental and directly in tests.
#[allow(dead_code)]
pub(super) fn build_incremental_system_prompt(
    config: &CurateConfig,
    compact_index: &str,
) -> String {
    let base = config.system_prompt.as_deref().map_or_else(
        || {
            format!(
                "{DEFAULT_INCREMENTAL_SYSTEM_PROMPT_BASE}{compact_index}{INCREMENTAL_INSTRUCTIONS}"
            )
        },
        |custom| format!("{custom}\n\nEXISTING INDEX:\n{compact_index}{INCREMENTAL_INSTRUCTIONS}"),
    );

    let format_suffix = if config.full_candidates {
        INPUT_FORMAT_FULL
    } else {
        INPUT_FORMAT_COMPACT
    };

    format!("{base}{format_suffix}")
}

/// Build the incremental stdin payload: new candidates + stale terms.
// Called by stdin_payload_for_incremental and directly in tests.
#[allow(dead_code)]
pub(super) fn build_incremental_stdin_payload(
    config: &CurateConfig,
    new_candidates_yaml: &str,
    stale_terms: &[String],
) -> String {
    let mut payload = String::from("NEW CANDIDATES to integrate:\n");
    payload.push_str(new_candidates_yaml);

    if !stale_terms.is_empty() {
        payload.push_str("\n\nPOTENTIALLY STALE terms (no longer found in corpus):\n");
        for term in stale_terms {
            payload.push_str("- ");
            payload.push_str(term);
            payload.push('\n');
        }
    }

    let instruction = config
        .prompt
        .as_deref()
        .map(|p| format!("{p}\n\n{DEFAULT_INSTRUCTION}"))
        .unwrap_or_else(|| DEFAULT_INSTRUCTION.to_string());

    format!("{payload}\n\n{instruction}")
}

/// Return the incremental system prompt for cost estimation.
// Wired in Task 6 (curate/mod.rs incremental path).
#[allow(dead_code)]
pub(super) fn system_prompt_for_incremental(config: &CurateConfig, compact_index: &str) -> String {
    build_incremental_system_prompt(config, compact_index)
}

/// Return the incremental stdin payload for cost estimation.
// Wired in Task 6 (curate/mod.rs incremental path).
#[allow(dead_code)]
pub(super) fn stdin_payload_for_incremental(
    config: &CurateConfig,
    new_candidates_yaml: &str,
    stale_terms: &[String],
) -> String {
    build_incremental_stdin_payload(config, new_candidates_yaml, stale_terms)
}

/// Return the delta JSON schema string.
// Wired in Task 6 (curate/mod.rs incremental invoke path).
#[allow(dead_code)]
pub(super) const fn delta_schema_json() -> &'static str {
    DELTA_SCHEMA_JSON
}

/// Create a no-plugins skeleton directory in temp.
///
/// The Claude CLI needs a valid plugin directory structure to skip
/// plugin discovery. An empty string or nonexistent path doesn't work —
/// it falls back to the default plugin dir. This skeleton has the right
/// structure with zero plugins installed.
fn create_no_plugins_dir() -> Result<tempfile::TempDir, CurateError> {
    let dir = tempfile::TempDir::new()?;

    std::fs::create_dir(dir.path().join("cache"))?;
    std::fs::create_dir(dir.path().join("marketplaces"))?;
    std::fs::write(
        dir.path().join("installed_plugins.json"),
        r#"{"version":2,"plugins":{}}"#,
    )?;
    std::fs::write(dir.path().join("known_marketplaces.json"), "{}")?;

    Ok(dir)
}

/// Write `claude_settings` to a temp file for `--settings`.
fn write_settings_file(
    settings: &serde_json::Value,
) -> Result<tempfile::NamedTempFile, CurateError> {
    let mut file = tempfile::NamedTempFile::new()?;
    let json = serde_json::to_string(settings).map_err(|e| CurateError::ParseResponse {
        detail: format!("failed to serialize claude_settings: {e}"),
    })?;
    file.write_all(json.as_bytes())?;
    file.flush()?;
    Ok(file)
}

/// Invoke the `claude` CLI with streaming output and return parsed results.
///
/// All agent runtime overhead is disabled via headless flags so the CLI
/// acts as a thin API passthrough. Streaming JSONL events are parsed
/// line-by-line, tracking thinking, editorial text, and structured output
/// across multiple turns.
pub(super) fn invoke(
    config: &CurateConfig,
    candidates: &CandidatesFile,
    candidates_yaml: &str,
    extra_args: &[String],
    progress: &ProgressBar,
) -> Result<InvokeResult, CurateError> {
    let claude_path = find_claude()?;

    let system_prompt = build_system_prompt(config);
    let stdin_payload = build_stdin_payload(config, candidates, candidates_yaml);

    // Create temp files that live until this function returns.
    let no_plugins_dir = create_no_plugins_dir()?;
    let settings_file = write_settings_file(&config.claude_settings)?;

    let plugin_dir_str = no_plugins_dir.path().to_string_lossy().to_string();
    let settings_path_str = settings_file.path().to_string_lossy().to_string();

    let mut cmd = Command::new(&claude_path);
    cmd.arg("--print")
        // Schema and model
        .arg("--json-schema")
        .arg(SCHEMA_JSON)
        .arg("--model")
        .arg(&config.model)
        .arg("--system-prompt")
        .arg(&system_prompt)
        // Streaming output (--verbose is REQUIRED with --print + stream-json)
        .arg("--output-format")
        .arg("stream-json")
        .arg("--include-partial-messages")
        .arg("--verbose")
        // Headless: disable agent runtime overhead
        .arg("--tools")
        .arg("")
        .arg("--no-session-persistence")
        .arg("--disable-slash-commands")
        .arg("--effort")
        .arg(&config.effort)
        .arg("--plugin-dir")
        .arg(&plugin_dir_str)
        .arg("--settings")
        .arg(&settings_path_str)
        .arg("--setting-sources")
        .arg("local");

    // Forward any extra CLI args (e.g., --max-budget-usd).
    for arg in extra_args {
        cmd.arg(arg);
    }

    // Environment: disable Claude Code nesting, set max output tokens.
    cmd.env("CLAUDECODE", "")
        .env(
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            config.max_output_tokens.to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    tracing::debug!(
        claude = %claude_path,
        model = %config.model,
        effort = %config.effort,
        full_candidates = config.full_candidates,
        payload_len = stdin_payload.len(),
        max_output_tokens = config.max_output_tokens,
        extra_args = ?extra_args,
        "invoking claude CLI (headless, streaming)"
    );

    let mut child = cmd.spawn().map_err(|e| CurateError::ClaudeFailed {
        exit_code: None,
        stderr: format!("failed to spawn claude: {e}"),
    })?;

    // Write stdin payload and close to signal EOF.
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CurateError::ClaudeFailed {
                exit_code: None,
                stderr: "failed to open claude stdin".to_string(),
            })?;
        stdin
            .write_all(stdin_payload.as_bytes())
            .map_err(|e| CurateError::ClaudeFailed {
                exit_code: None,
                stderr: format!("stdin write failed: {e}"),
            })?;
    }

    // Drain stderr in a background thread so it doesn't block the process.
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| CurateError::ClaudeFailed {
            exit_code: None,
            stderr: "failed to open claude stderr".to_string(),
        })?;
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr_pipe), &mut buf);
        buf
    });

    // Read streaming JSONL from stdout.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CurateError::ClaudeFailed {
            exit_code: None,
            stderr: "failed to open claude stdout".to_string(),
        })?;

    let result = parse_stream(stdout, progress)?;

    // Wait for process to exit.
    let status = child.wait().map_err(|e| CurateError::ClaudeFailed {
        exit_code: None,
        stderr: format!("failed to wait for claude: {e}"),
    })?;

    let captured_stderr = stderr_thread.join().unwrap_or_default();
    if !captured_stderr.is_empty() {
        tracing::debug!(stderr_len = captured_stderr.len(), "claude stderr captured");
    }

    if !status.success() && result.is_none() {
        return Err(CurateError::ClaudeFailed {
            exit_code: status.code(),
            stderr: captured_stderr,
        });
    }

    result.ok_or_else(|| CurateError::ParseResponse {
        detail: "no valid structured output found in stream".to_string(),
    })
}

/// Parse streaming JSONL events from the Claude CLI stdout.
///
/// Tracks `thinking_delta`, `text_delta` (editorial summary), and
/// `input_json_delta` events across multiple turns. Returns the last
/// valid structured output. If a turn hits `max_tokens`, its truncated
/// JSON is discarded and the next turn's output is used instead.
fn parse_stream(
    stdout: impl std::io::Read,
    progress: &ProgressBar,
) -> Result<Option<InvokeResult>, CurateError> {
    let reader = BufReader::new(stdout);

    let mut thinking = String::new();
    let mut editorial = String::new();
    let mut current_turn_json = String::new();
    let mut last_valid_output: Option<ClaudeOutput> = None;
    let mut thinking_deltas: usize = 0;
    let mut json_bytes: usize = 0;
    let mut turn_count: usize = 0;
    let mut usage = TokenUsage::default();

    progress.set_message("Starting...");
    progress.enable_steady_tick(Duration::from_millis(120));

    for line in reader.lines() {
        let line = line.map_err(|e| CurateError::ClaudeFailed {
            exit_code: None,
            stderr: format!("failed to read stream: {e}"),
        })?;

        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if event["type"] != "stream_event" {
            continue;
        }

        let evt = &event["event"];
        match evt["type"].as_str() {
            Some("message_start") => {
                turn_count += 1;
                current_turn_json.clear();

                // Extract token usage from message_start.
                if let Some(msg_usage) = evt["message"]["usage"].as_object() {
                    if let Some(n) = msg_usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        usage.input_tokens += n as usize;
                    }
                    if let Some(n) = msg_usage
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64())
                    {
                        usage.cache_creation_input_tokens += n as usize;
                    }
                    if let Some(n) = msg_usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64())
                    {
                        usage.cache_read_input_tokens += n as usize;
                    }
                    tracing::debug!(
                        turn = turn_count,
                        input = usage.input_tokens,
                        cache_write = usage.cache_creation_input_tokens,
                        cache_read = usage.cache_read_input_tokens,
                        "usage from message_start"
                    );
                }

                tracing::debug!(turn = turn_count, "new turn started");
            }
            Some("content_block_delta") => match evt["delta"]["type"].as_str() {
                Some("thinking_delta") => {
                    if let Some(text) = evt["delta"]["thinking"].as_str() {
                        thinking.push_str(text);
                        thinking_deltas += 1;
                        progress.set_message(format!("Thinking... ({thinking_deltas} tokens)"));
                    }
                }
                Some("text_delta") => {
                    if let Some(text) = evt["delta"]["text"].as_str() {
                        editorial.push_str(text);
                    }
                }
                Some("input_json_delta") => {
                    if let Some(partial) = evt["delta"]["partial_json"].as_str() {
                        current_turn_json.push_str(partial);
                        json_bytes += partial.len();
                        progress.set_message(format!("Generating index... ({json_bytes} bytes)"));
                    }
                }
                _ => {}
            },
            Some("message_delta") => {
                // Output tokens from message_delta.usage.
                if let Some(n) = evt["usage"]["output_tokens"].as_u64() {
                    usage.output_tokens += n as usize;
                }

                if let Some(stop) = evt["delta"]["stop_reason"].as_str() {
                    tracing::debug!(turn = turn_count, stop_reason = stop, "turn ending");
                    if stop == "max_tokens" {
                        tracing::warn!(
                            turn = turn_count,
                            "turn hit max_tokens — structured output may be truncated"
                        );
                    }
                }
            }
            Some("message_stop") => {
                // Try to parse the accumulated JSON for this turn.
                if !current_turn_json.is_empty() {
                    match serde_json::from_str::<ClaudeOutput>(&current_turn_json) {
                        Ok(output) => {
                            tracing::debug!(
                                turn = turn_count,
                                terms = output.terms.len(),
                                suggested = output.suggested.len(),
                                "valid structured output from turn"
                            );
                            last_valid_output = Some(output);
                        }
                        Err(e) => {
                            tracing::warn!(
                                turn = turn_count,
                                error = %e,
                                json_len = current_turn_json.len(),
                                "failed to parse turn JSON (likely truncated)"
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(ref output) = last_valid_output {
        progress.finish_with_message(format!(
            "Curated {} terms + {} suggested ({} turns, {} thinking tokens)",
            output.terms.len(),
            output.suggested.len(),
            turn_count,
            thinking_deltas,
        ));
    } else {
        progress.finish_with_message("No valid output produced");
    }

    Ok(last_valid_output.map(|output| InvokeResult {
        output,
        thinking,
        editorial,
        turns: turn_count,
        thinking_tokens: thinking_deltas,
        usage,
    }))
}

/// Find the `claude` CLI in PATH.
fn find_claude() -> Result<String, CurateError> {
    which::which("claude")
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|_| CurateError::ClaudeNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::candidates::{Candidate, CandidateLocation};

    #[test]
    fn compact_candidates_format() {
        let candidates = CandidatesFile {
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
                            context: "a long context snippet that should not appear".to_string(),
                        },
                        CandidateLocation {
                            file: "api.md".to_string(),
                            context: "another snippet".to_string(),
                        },
                    ],
                },
                Candidate {
                    term: "TLS".to_string(),
                    score: 0.80,
                    locations: vec![CandidateLocation {
                        file: "security.md".to_string(),
                        context: "TLS encrypts data".to_string(),
                    }],
                },
            ],
        };
        let compact = build_compact_candidates(&candidates);
        assert!(compact.contains("OAuth | 0.95 | auth.md, api.md"));
        assert!(compact.contains("TLS | 0.80 | security.md"));
        assert!(!compact.contains("long context snippet"));
        assert!(compact.contains("docs/ (3 documents)"));
    }

    #[test]
    fn system_prompt_default_is_full_mode() {
        let config = CurateConfig::default();
        let prompt = build_system_prompt(&config);
        assert!(prompt.contains("indexer"));
        assert!(prompt.contains("YAML with fields per candidate"));
    }

    #[test]
    fn system_prompt_compact_mode() {
        let config = CurateConfig {
            full_candidates: false,
            ..CurateConfig::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(prompt.contains("indexer"));
        assert!(prompt.contains("term | score | file1"));
    }

    #[test]
    fn system_prompt_custom_replaces_default() {
        let config = CurateConfig {
            system_prompt: Some("Custom prompt.".to_string()),
            ..CurateConfig::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(prompt.starts_with("Custom prompt."));
        assert!(!prompt.contains("indexer"));
    }

    #[test]
    fn stdin_payload_compact() {
        let config = CurateConfig {
            full_candidates: false,
            ..CurateConfig::default()
        };
        let candidates = CandidatesFile {
            version: 1,
            generated: "2026-03-10T12:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 1,
            candidates: vec![Candidate {
                term: "test".to_string(),
                score: 0.5,
                locations: vec![CandidateLocation {
                    file: "test.md".to_string(),
                    context: "test context".to_string(),
                }],
            }],
        };
        let payload = build_stdin_payload(&config, &candidates, "raw yaml");
        assert!(payload.contains("test | 0.50 | test.md"));
        assert!(payload.contains(DEFAULT_INSTRUCTION));
        assert!(!payload.contains("raw yaml"));
    }

    #[test]
    fn stdin_payload_full() {
        let config = CurateConfig::default(); // full_candidates=true by default
        let candidates = CandidatesFile {
            version: 1,
            generated: "2026-03-10T12:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 1,
            candidates: Vec::new(),
        };
        let payload = build_stdin_payload(&config, &candidates, "raw yaml content");
        assert!(payload.contains("raw yaml content"));
        assert!(payload.contains(DEFAULT_INSTRUCTION));
    }

    #[test]
    fn stdin_payload_with_user_prompt() {
        let config = CurateConfig {
            prompt: Some("Focus on security terms.".to_string()),
            ..CurateConfig::default()
        };
        let candidates = CandidatesFile {
            version: 1,
            generated: "2026-03-10T12:00:00Z".to_string(),
            source_dir: "docs/".to_string(),
            document_count: 1,
            candidates: Vec::new(),
        };
        let payload = build_stdin_payload(&config, &candidates, "");
        assert!(payload.contains("Focus on security terms."));
        assert!(payload.contains(DEFAULT_INSTRUCTION));
    }

    #[test]
    fn incremental_system_prompt_contains_index_and_instructions() {
        let config = CurateConfig::default();
        let compact_index = "OAuth | parent: auth | aliases: OAuth 2.0\nTLS | (top-level)\n";
        let prompt = build_incremental_system_prompt(&config, compact_index);
        assert!(prompt.contains("updating an existing back-of-book index"));
        assert!(prompt.contains("EXISTING INDEX"));
        assert!(prompt.contains("OAuth | parent: auth"));
        assert!(prompt.contains("For each new candidate"));
    }

    #[test]
    fn incremental_stdin_payload_contains_candidates_and_stale() {
        let config = CurateConfig {
            full_candidates: true,
            ..CurateConfig::default()
        };
        let candidates_yaml = "- term: PKCE\n  score: 0.9\n";
        let stale = vec!["old_term".to_string()];
        let payload = build_incremental_stdin_payload(&config, candidates_yaml, &stale);
        assert!(payload.contains("PKCE"));
        assert!(payload.contains("POTENTIALLY STALE"));
        assert!(payload.contains("old_term"));
    }

    #[test]
    fn incremental_stdin_payload_no_stale_section_when_empty() {
        let config = CurateConfig {
            full_candidates: true,
            ..CurateConfig::default()
        };
        let payload = build_incremental_stdin_payload(&config, "candidates", &[]);
        assert!(!payload.contains("POTENTIALLY STALE"));
    }

    #[test]
    fn delta_schema_json_is_valid() {
        let value: serde_json::Value =
            serde_json::from_str(DELTA_SCHEMA_JSON).expect("delta schema should be valid JSON");
        assert_eq!(value["type"], "object");
        assert!(value["properties"]["additions"].is_object());
        assert!(value["properties"]["modifications"].is_object());
        assert!(value["properties"]["removals"].is_object());
    }

    #[test]
    fn schema_json_is_valid() {
        let value: serde_json::Value =
            serde_json::from_str(SCHEMA_JSON).expect("embedded schema should be valid JSON");
        assert_eq!(value["type"], "object");
        assert!(value["properties"]["terms"].is_object());
    }

    #[test]
    fn default_system_prompt_is_nonempty() {
        assert!(!DEFAULT_SYSTEM_PROMPT_BASE.is_empty());
        assert!(DEFAULT_SYSTEM_PROMPT_BASE.contains("indexer"));
    }

    #[test]
    fn parse_stream_single_turn() {
        let events = [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"test","id":"msg1","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"terms\":[{\"term\":\"OAuth\",\"definition\":\"Auth standard.\"}],\"suggested\":[]}"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":500}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];
        let input = events.join("\n");
        let pb = ProgressBar::hidden();
        let result = parse_stream(input.as_bytes(), &pb).unwrap();
        let result = result.expect("should have valid output");
        assert_eq!(result.output.terms.len(), 1);
        assert_eq!(result.output.terms[0].term, "OAuth");
        assert!(result.thinking.contains("Let me think"));
    }

    #[test]
    fn parse_stream_multi_turn_takes_last_valid() {
        let events = [
            // Turn 1: truncated JSON (max_tokens)
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"test","id":"msg1","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"terms\":[{\"term\":\"partial\""}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":32000}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
            // Turn 2: valid JSON
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"test","id":"msg2","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"terms\":[{\"term\":\"OAuth\",\"definition\":\"Auth.\"}],\"suggested\":[{\"term\":\"TLS\",\"definition\":\"Encryption.\"}]}"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":500}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];
        let input = events.join("\n");
        let pb = ProgressBar::hidden();
        let result = parse_stream(input.as_bytes(), &pb).unwrap();
        let result = result.expect("should have valid output from turn 2");
        assert_eq!(result.output.terms.len(), 1);
        assert_eq!(result.output.terms[0].term, "OAuth");
        assert_eq!(result.output.suggested.len(), 1);
    }

    #[test]
    fn parse_stream_captures_editorial_text() {
        let events = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"test","id":"msg1","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I curated "}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"72 terms."}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"terms\":[{\"term\":\"test\",\"definition\":\"A test.\"}],\"suggested\":[]}"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];
        let input = events.join("\n");
        let pb = ProgressBar::hidden();
        let result = parse_stream(input.as_bytes(), &pb).unwrap();
        let result = result.expect("should have valid output");
        assert_eq!(result.editorial, "I curated 72 terms.");
    }

    #[test]
    fn parse_stream_no_output() {
        let events = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"test","id":"msg1","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"thinking only"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];
        let input = events.join("\n");
        let pb = ProgressBar::hidden();
        let result = parse_stream(input.as_bytes(), &pb).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn no_plugins_dir_structure() {
        let dir = create_no_plugins_dir().unwrap();
        assert!(dir.path().join("cache").is_dir());
        assert!(dir.path().join("marketplaces").is_dir());
        assert!(dir.path().join("installed_plugins.json").is_file());
        assert!(dir.path().join("known_marketplaces.json").is_file());

        let installed: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("installed_plugins.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(installed["version"], 2);
        assert!(installed["plugins"].as_object().unwrap().is_empty());
    }

    #[test]
    fn settings_file_writes_json() {
        let settings = serde_json::json!({
            "alwaysThinkingEnabled": true,
            "effortLevel": "high",
            "fastMode": true
        });
        let file = write_settings_file(&settings).unwrap();
        let contents = std::fs::read_to_string(file.path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["alwaysThinkingEnabled"], true);
        assert_eq!(parsed["effortLevel"], "high");
    }

    #[test]
    fn settings_file_empty_object() {
        let settings = serde_json::json!({});
        let file = write_settings_file(&settings).unwrap();
        let contents = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(contents, "{}");
    }
}
