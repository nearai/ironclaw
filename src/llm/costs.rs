//! Per-model cost lookup table for multi-provider LLM support.
//!
//! Returns (input_cost_per_token, output_cost_per_token) as Decimal pairs.
//! Ollama and other local models return zero cost.
//!
//! Also hosts the shared per-call cost computation used by both v1
//! (`CostGuard::record_llm_call`) and v2 (`LlmBridgeAdapter::cost_usd_from`)
//! so both engines agree on the number billed for a single completion.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;

/// Look up known per-token costs for a model by its identifier.
///
/// Returns `Some((input_cost, output_cost))` for known models, `None` otherwise.
pub fn model_cost(model_id: &str) -> Option<(Decimal, Decimal)> {
    // OpenRouter free-tier models: `:free` suffix or the `openrouter/free` router
    // should always report zero cost (see #463).
    if model_id.ends_with(":free") || model_id == "openrouter/free" || model_id == "free" {
        return Some((Decimal::ZERO, Decimal::ZERO));
    }

    // Normalize: strip provider prefixes (e.g., "openai/gpt-4o" -> "gpt-4o")
    let id = model_id
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(model_id);

    match id {
        // OpenAI — GPT-5.x / Codex
        "gpt-5.3-codex" | "gpt-5.3-codex-spark" => Some((dec!(0.000002), dec!(0.000008))),
        "gpt-5.2-codex" | "gpt-5.2-pro" | "gpt-5.2" => Some((dec!(0.000002), dec!(0.000008))),
        "gpt-5.1-codex" | "gpt-5.1-codex-max" | "gpt-5.1" => Some((dec!(0.000002), dec!(0.000008))),
        "gpt-5.1-codex-mini" => Some((dec!(0.0000003), dec!(0.0000012))),
        "gpt-5-codex" | "gpt-5-pro" | "gpt-5" => Some((dec!(0.000002), dec!(0.000008))),
        "gpt-5-mini" | "gpt-5-nano" => Some((dec!(0.0000003), dec!(0.0000012))),
        // OpenAI — GPT-4.x
        "gpt-4.1" => Some((dec!(0.000002), dec!(0.000008))),
        "gpt-4.1-mini" => Some((dec!(0.0000004), dec!(0.0000016))),
        "gpt-4.1-nano" => Some((dec!(0.0000001), dec!(0.0000004))),
        "gpt-4o" | "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" => {
            Some((dec!(0.0000025), dec!(0.00001)))
        }
        "gpt-4o-mini" | "gpt-4o-mini-2024-07-18" => Some((dec!(0.00000015), dec!(0.0000006))),
        "gpt-4-turbo" | "gpt-4-turbo-2024-04-09" => Some((dec!(0.00001), dec!(0.00003))),
        "gpt-4" | "gpt-4-0613" => Some((dec!(0.00003), dec!(0.00006))),
        "gpt-3.5-turbo" | "gpt-3.5-turbo-0125" => Some((dec!(0.0000005), dec!(0.0000015))),
        // OpenAI — reasoning
        "o3" => Some((dec!(0.000002), dec!(0.000008))),
        "o3-mini" | "o3-mini-2025-01-31" => Some((dec!(0.0000011), dec!(0.0000044))),
        "o4-mini" => Some((dec!(0.0000011), dec!(0.0000044))),
        "o1" | "o1-2024-12-17" => Some((dec!(0.000015), dec!(0.00006))),
        "o1-mini" | "o1-mini-2024-09-12" => Some((dec!(0.000003), dec!(0.000012))),

        // Anthropic
        "claude-opus-4-6"
        | "claude-opus-4-5"
        | "claude-opus-4-5-20251101"
        | "claude-opus-4-1"
        | "claude-opus-4-1-20250805"
        | "claude-opus-4-0"
        | "claude-opus-4-20250514"
        | "claude-3-opus-20240229"
        | "claude-3-opus-latest" => Some((dec!(0.000015), dec!(0.000075))),
        "claude-sonnet-4-6"
        | "claude-sonnet-4-5"
        | "claude-sonnet-4-5-20250929"
        | "claude-sonnet-4-0"
        | "claude-sonnet-4-20250514"
        | "claude-3-7-sonnet-20250219"
        | "claude-3-7-sonnet-latest"
        | "claude-3-5-sonnet-20241022"
        | "claude-3-5-sonnet-latest" => Some((dec!(0.000003), dec!(0.000015))),
        "claude-haiku-4-5"
        | "claude-haiku-4-5-20251001"
        | "claude-3-5-haiku-20241022"
        | "claude-3-5-haiku-latest" => Some((dec!(0.0000008), dec!(0.000004))),
        "claude-3-haiku-20240307" => Some((dec!(0.00000025), dec!(0.00000125))),

        // Ollama / local models -- free
        _ if is_local_model(id) => Some((Decimal::ZERO, Decimal::ZERO)),

        _ => None,
    }
}

/// Default cost for unknown models.
pub fn default_cost() -> (Decimal, Decimal) {
    // Conservative estimate: roughly GPT-4o pricing
    (dec!(0.0000025), dec!(0.00001))
}

/// Compute the USD cost of a single completion response as a `Decimal`,
/// honoring prompt-caching pricing.
///
/// Shared by v1 (`CostGuard::record_llm_call`) and v2 (via
/// `compute_call_cost_usd`). Both engines must call this so
/// `Thread::total_cost_usd` and v1's daily budget tracker agree.
///
/// Pricing rules:
/// - uncached input tokens priced at `input_rate`
/// - cache-read tokens discounted by `cache_read_discount` (e.g. 10 for
///   Anthropic, 2 for OpenAI). Zero is treated as "no discount".
/// - cache-write tokens multiplied by `cache_write_multiplier` (e.g.
///   1.25× for Anthropic 5-minute TTL, 2× for 1-hour TTL)
/// - output tokens priced at `output_rate`
///
/// `input_tokens` is the provider-reported total; cache tokens are already
/// counted inside that total, so uncached input = input_tokens - cache_read
/// - cache_creation.
#[allow(clippy::too_many_arguments)]
pub fn compute_call_cost_decimal(
    input_rate: Decimal,
    output_rate: Decimal,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: u32,
    cache_creation_input_tokens: u32,
    cache_read_discount: Decimal,
    cache_write_multiplier: Decimal,
) -> Decimal {
    let cached_total = cache_read_input_tokens.saturating_add(cache_creation_input_tokens);
    let uncached_input = input_tokens.saturating_sub(cached_total);
    let effective_discount = if cache_read_discount.is_zero() {
        Decimal::ONE
    } else {
        cache_read_discount
    };
    let cache_read_cost =
        input_rate * Decimal::from(cache_read_input_tokens) / effective_discount;
    let cache_write_cost =
        input_rate * Decimal::from(cache_creation_input_tokens) * cache_write_multiplier;
    input_rate * Decimal::from(uncached_input)
        + cache_read_cost
        + cache_write_cost
        + output_rate * Decimal::from(output_tokens)
}

/// Convenience wrapper returning the cost as `f64`, for engine v2's
/// `TokenUsage::cost_usd` field. Lossy at extreme precision but adequate
/// for display / budget comparisons. Returns 0.0 on NaN or conversion
/// failure (e.g. extremely large Decimals).
#[allow(clippy::too_many_arguments)]
pub fn compute_call_cost_usd(
    input_rate: Decimal,
    output_rate: Decimal,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: u32,
    cache_creation_input_tokens: u32,
    cache_read_discount: Decimal,
    cache_write_multiplier: Decimal,
) -> f64 {
    compute_call_cost_decimal(
        input_rate,
        output_rate,
        input_tokens,
        output_tokens,
        cache_read_input_tokens,
        cache_creation_input_tokens,
        cache_read_discount,
        cache_write_multiplier,
    )
    .to_f64()
    .unwrap_or(0.0)
}

/// Heuristic to detect local/self-hosted models (Ollama, llama.cpp, etc.).
fn is_local_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    lower.starts_with("llama")
        || lower.starts_with("mistral")
        || lower.starts_with("mixtral")
        || lower.starts_with("phi")
        || lower.starts_with("gemma")
        || lower.starts_with("qwen")
        || lower.starts_with("codellama")
        || lower.starts_with("deepseek")
        || lower.starts_with("starcoder")
        || lower.starts_with("vicuna")
        || lower.starts_with("yi")
        || lower.contains(":latest")
        || lower.contains(":instruct")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_model_costs() {
        let (input, output) = model_cost("gpt-4o").unwrap();
        assert!(input > Decimal::ZERO);
        assert!(output > input);
    }

    #[test]
    fn test_claude_costs() {
        let (input, output) = model_cost("claude-3-5-sonnet-20241022").unwrap();
        assert!(input > Decimal::ZERO);
        assert!(output > input);
    }

    #[test]
    fn test_local_model_free() {
        let (input, output) = model_cost("llama3").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_ollama_tagged_model_free() {
        let (input, output) = model_cost("mistral:latest").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_unknown_model_returns_none() {
        assert!(model_cost("some-totally-unknown-model-xyz").is_none());
    }

    #[test]
    fn test_default_cost_nonzero() {
        let (input, output) = default_cost();
        assert!(input > Decimal::ZERO);
        assert!(output > Decimal::ZERO);
    }

    #[test]
    fn test_provider_prefix_stripped() {
        // "openai/gpt-4o" should resolve to same as "gpt-4o"
        assert_eq!(model_cost("openai/gpt-4o"), model_cost("gpt-4o"));
    }

    /// Regression for #2800 PR-A: `compute_call_cost_decimal` must produce
    /// the same number v1's `CostGuard::record_llm_call` previously computed
    /// inline. Guards against anyone "simplifying" the formula on either side.
    #[test]
    fn compute_call_cost_decimal_matches_legacy_inline_formula() {
        // Representative Anthropic-with-caching call.
        let input_rate = dec!(0.000003);
        let output_rate = dec!(0.000015);
        let input_tokens: u32 = 10_000;
        let output_tokens: u32 = 500;
        let cache_read: u32 = 3_000;
        let cache_write: u32 = 1_000;
        let discount = dec!(10);
        let multiplier = dec!(1.25);

        // Re-derive inline the same way v1 used to do it.
        let cached_total = cache_read.saturating_add(cache_write);
        let uncached_input = input_tokens.saturating_sub(cached_total);
        let effective_discount = if discount.is_zero() {
            Decimal::ONE
        } else {
            discount
        };
        let expected = input_rate * Decimal::from(uncached_input)
            + input_rate * Decimal::from(cache_read) / effective_discount
            + input_rate * Decimal::from(cache_write) * multiplier
            + output_rate * Decimal::from(output_tokens);

        let actual = compute_call_cost_decimal(
            input_rate,
            output_rate,
            input_tokens,
            output_tokens,
            cache_read,
            cache_write,
            discount,
            multiplier,
        );
        assert_eq!(actual, expected);
    }

    /// Zero discount must be treated as "no discount" (not divide-by-zero).
    #[test]
    fn compute_call_cost_zero_discount_does_not_panic() {
        let cost = compute_call_cost_decimal(
            dec!(0.000003),
            dec!(0.000015),
            1_000,
            100,
            500,
            0,
            Decimal::ZERO, // zero discount: treat as no discount
            Decimal::ONE,
        );
        // With uncached=500, cache_read=500, output=100: 500*rate + 500*rate + 100*out_rate
        assert!(cost > Decimal::ZERO);
    }

    /// Providers that report zero cost per token (subscription billing)
    /// must return zero USD cost regardless of token counts.
    #[test]
    fn compute_call_cost_zero_rate_returns_zero() {
        let cost_f64 = compute_call_cost_usd(
            Decimal::ZERO,
            Decimal::ZERO,
            100_000,
            10_000,
            0,
            0,
            Decimal::ONE,
            Decimal::ONE,
        );
        assert_eq!(cost_f64, 0.0);
    }

    /// The f64 wrapper is a thin convenience over the Decimal path; its
    /// output must match `.to_f64()` of the Decimal result, so engine v2
    /// (f64 consumer) and v1 (Decimal consumer) agree to the last bit f64
    /// can represent.
    #[test]
    fn compute_call_cost_usd_tracks_decimal_path() {
        let input_rate = dec!(0.000003);
        let output_rate = dec!(0.000015);
        let dec_val = compute_call_cost_decimal(
            input_rate,
            output_rate,
            1_234,
            567,
            200,
            100,
            dec!(10),
            dec!(1.25),
        );
        let f64_val = compute_call_cost_usd(
            input_rate,
            output_rate,
            1_234,
            567,
            200,
            100,
            dec!(10),
            dec!(1.25),
        );
        assert_eq!(dec_val.to_f64().unwrap(), f64_val);
    }

    #[test]
    fn test_openrouter_free_suffix_zero_cost() {
        // Models with `:free` suffix should report zero cost (#463)
        let (input, output) = model_cost("stepfun/step-3.5-flash:free").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_openrouter_free_router_zero_cost() {
        // The "openrouter/free" router model should report zero cost (#463)
        let (input, output) = model_cost("openrouter/free").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_bare_free_zero_cost() {
        // Edge case: bare "free" after prefix stripping
        let (input, output) = model_cost("free").unwrap();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_free_suffix_various_providers() {
        // Various provider-prefixed free models
        for model in &[
            "google/gemma-3-27b-it:free",
            "meta-llama/llama-4-maverick:free",
            "microsoft/phi-4:free",
            "nousresearch/deephermes-3-llama-3-8b-preview:free",
        ] {
            let (input, output) =
                model_cost(model).unwrap_or_else(|| panic!("{model} should return Some"));
            assert_eq!(input, Decimal::ZERO, "{model} input cost should be zero");
            assert_eq!(output, Decimal::ZERO, "{model} output cost should be zero");
        }
    }
}
