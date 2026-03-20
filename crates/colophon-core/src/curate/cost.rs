//! Token counting and cost estimation for the curate pipeline.
//!
//! Uses `ah-ah-ah` for offline Claude token counting. Estimates are
//! conservative (~4% overcount) so budget enforcement is safe.

use ah_ah_ah::{count_tokens, Backend};

/// Per-million-token pricing for a Claude model.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Input tokens ($/MTok).
    pub input: f64,
    /// Output tokens ($/MTok).
    pub output: f64,
    /// Cache write tokens ($/MTok).
    pub cache_write: f64,
    /// Cache read tokens ($/MTok).
    pub cache_read: f64,
}

impl ModelPricing {
    /// Look up pricing by model name.
    ///
    /// Matches on prefix so "opus", "claude-opus-4-6", etc. all resolve.
    /// Returns `None` for unrecognized models.
    pub fn for_model(model: &str) -> Option<Self> {
        let m = model.to_lowercase();
        if m.contains("opus") {
            Some(Self::OPUS)
        } else if m.contains("sonnet") {
            Some(Self::SONNET)
        } else if m.contains("haiku") {
            Some(Self::HAIKU)
        } else {
            None
        }
    }

    /// Opus pricing (as of 2026-03).
    const OPUS: Self = Self {
        input: 15.0,
        output: 75.0,
        cache_write: 18.75,
        cache_read: 1.50,
    };

    /// Sonnet pricing (as of 2026-03).
    const SONNET: Self = Self {
        input: 3.0,
        output: 15.0,
        cache_write: 3.75,
        cache_read: 0.30,
    };

    /// Haiku pricing (as of 2026-03).
    const HAIKU: Self = Self {
        input: 0.80,
        output: 4.0,
        cache_write: 1.0,
        cache_read: 0.08,
    };
}

/// Pre-flight cost estimate (before invoking Claude).
#[derive(Debug, Clone)]
pub struct CostEstimate {
    /// Estimated input tokens (system prompt + payload + schema).
    pub input_tokens: usize,
    /// Estimated max output tokens (from config).
    pub max_output_tokens: u32,
    /// Estimated cost in USD (first call, no cache).
    pub estimated_usd: f64,
    /// Estimated cost with full cache hit on turn 2+.
    pub estimated_cached_usd: f64,
    /// Model used for pricing.
    pub model: String,
    /// Whether pricing was found for this model.
    pub pricing_known: bool,
}

impl std::fmt::Display for CostEstimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "~{} input tokens, {} max output tokens",
            self.input_tokens, self.max_output_tokens
        )?;
        if self.pricing_known {
            write!(
                f,
                " → ${:.2} estimated (${:.2} with cache)",
                self.estimated_usd, self.estimated_cached_usd
            )?;
        } else {
            write!(f, " (pricing unknown for {})", self.model)?;
        }
        Ok(())
    }
}

/// Actual token usage from a completed Claude invocation.
///
/// Accumulated from `message_start` and `message_delta` stream events.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Input tokens billed.
    pub input_tokens: usize,
    /// Output tokens billed.
    pub output_tokens: usize,
    /// Tokens written to cache (first turn).
    pub cache_creation_input_tokens: usize,
    /// Tokens read from cache (turn 2+).
    pub cache_read_input_tokens: usize,
}

impl TokenUsage {
    /// Calculate actual cost in USD.
    pub fn actual_cost(&self, pricing: &ModelPricing) -> f64 {
        let cost_per = |tokens: usize, rate: f64| tokens as f64 * rate / 1_000_000.0;
        cost_per(self.input_tokens, pricing.input)
            + cost_per(self.output_tokens, pricing.output)
            + cost_per(self.cache_creation_input_tokens, pricing.cache_write)
            + cost_per(self.cache_read_input_tokens, pricing.cache_read)
    }
}

/// Count tokens in the full prompt payload and estimate cost.
///
/// Counts: system prompt + stdin payload + schema JSON.
/// Uses `Backend::Claude` (conservative, ~4% overcount).
pub fn estimate(
    system_prompt: &str,
    stdin_payload: &str,
    schema_json: &str,
    max_output_tokens: u32,
    model: &str,
) -> CostEstimate {
    // Count each component separately for better tracing.
    let system_tokens = count_tokens(system_prompt, None, Backend::Claude, None).count;
    let payload_tokens = count_tokens(stdin_payload, None, Backend::Claude, None).count;
    let schema_tokens = count_tokens(schema_json, None, Backend::Claude, None).count;

    // Claude CLI adds framing overhead (~200-500 tokens for conversation wrapper).
    let overhead = 300;
    let input_tokens = system_tokens + payload_tokens + schema_tokens + overhead;

    tracing::debug!(
        system_tokens,
        payload_tokens,
        schema_tokens,
        overhead,
        total = input_tokens,
        "token count breakdown"
    );

    let pricing = ModelPricing::for_model(model);
    let (estimated_usd, estimated_cached_usd) = pricing
        .map(|p| {
            let mtok = 1_000_000.0;
            let output_cost = (max_output_tokens as f64 / mtok) * p.output;
            // First call: all input is cache_write + output.
            let first_call = (input_tokens as f64 / mtok).mul_add(p.cache_write, output_cost);
            // Cached call: all input is cache_read + output.
            let cached = (input_tokens as f64 / mtok).mul_add(p.cache_read, output_cost);
            (first_call, cached)
        })
        .unwrap_or((0.0, 0.0));

    CostEstimate {
        input_tokens,
        max_output_tokens,
        estimated_usd,
        estimated_cached_usd,
        model: model.to_string(),
        pricing_known: pricing.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_lookup_by_name() {
        assert!(ModelPricing::for_model("opus").is_some());
        assert!(ModelPricing::for_model("claude-opus-4-6").is_some());
        assert!(ModelPricing::for_model("sonnet").is_some());
        assert!(ModelPricing::for_model("claude-sonnet-4-6").is_some());
        assert!(ModelPricing::for_model("haiku").is_some());
        assert!(ModelPricing::for_model("unknown-model").is_none());
    }

    #[test]
    fn pricing_case_insensitive() {
        assert!(ModelPricing::for_model("OPUS").is_some());
        assert!(ModelPricing::for_model("Sonnet").is_some());
    }

    #[test]
    fn estimate_produces_nonzero() {
        let est = estimate("You are a bot.", "Some input text.", "{}", 64_000, "opus");
        assert!(est.input_tokens > 0);
        assert!(est.estimated_usd > 0.0);
        assert!(est.estimated_cached_usd > 0.0);
        assert!(est.pricing_known);
    }

    #[test]
    fn estimate_unknown_model() {
        let est = estimate("prompt", "input", "{}", 64_000, "gpt-5");
        assert!(est.input_tokens > 0);
        assert_eq!(est.estimated_usd, 0.0);
        assert!(!est.pricing_known);
    }

    #[test]
    fn cached_cheaper_than_uncached() {
        let est = estimate("You are a bot.", "Some input text.", "{}", 64_000, "opus");
        assert!(
            est.estimated_cached_usd < est.estimated_usd,
            "cached should be cheaper: {} vs {}",
            est.estimated_cached_usd,
            est.estimated_usd
        );
    }

    #[test]
    fn opus_more_expensive_than_sonnet() {
        let opus = estimate("prompt", "input", "{}", 64_000, "opus");
        let sonnet = estimate("prompt", "input", "{}", 64_000, "sonnet");
        assert!(
            opus.estimated_usd > sonnet.estimated_usd,
            "opus should cost more: {} vs {}",
            opus.estimated_usd,
            sonnet.estimated_usd
        );
    }

    #[test]
    fn token_usage_cost_calculation() {
        let usage = TokenUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            cache_creation_input_tokens: 100_000,
            cache_read_input_tokens: 0,
        };
        let pricing = ModelPricing::for_model("opus").unwrap();
        let cost = usage.actual_cost(&pricing);
        assert!(cost > 0.0);
    }

    #[test]
    fn display_known_model() {
        let est = estimate("prompt", "input", "{}", 64_000, "opus");
        let display = est.to_string();
        assert!(display.contains("input tokens"));
        assert!(display.contains("$"));
    }

    #[test]
    fn display_unknown_model() {
        let est = estimate("prompt", "input", "{}", 64_000, "custom");
        let display = est.to_string();
        assert!(display.contains("pricing unknown"));
    }
}
