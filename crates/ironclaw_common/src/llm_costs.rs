//! Per-model cost lookup table for multi-provider LLM support.
//!
//! Returns (input_cost_per_token, output_cost_per_token) as Decimal pairs.
//! Ollama and other local models return zero cost.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

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
        "gpt-5.5" | "gpt-5.5-codex" => Some((dec!(0.000002), dec!(0.000008))),
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

        // Family fallbacks: a new GPT-5.x minor release shouldn't need a table
        // edit just to be budgeted. Exact arms above win for known per-model
        // pricing; these only catch unrecognized `gpt-5*` slugs. `*-mini` /
        // `*-nano` bill at the small tier, everything else at the standard tier.
        _ if id.starts_with("gpt-5") && (id.ends_with("-mini") || id.ends_with("-nano")) => {
            Some((dec!(0.0000003), dec!(0.0000012)))
        }
        _ if id.starts_with("gpt-5") => Some((dec!(0.000002), dec!(0.000008))),

        _ => None,
    }
}

/// Default cost for unknown models.
pub fn default_cost() -> (Decimal, Decimal) {
    // Conservative estimate: roughly GPT-4o pricing
    (dec!(0.0000025), dec!(0.00001))
}

/// A per-run USD cost, split by billing category. Priced from cumulative token
/// usage via [`price_usage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageCost {
    /// Fresh (non-cached) input plus cache-creation tokens at the full input rate.
    pub input_cost: Decimal,
    /// Cache-read tokens at the model's discounted rate.
    pub cached_input_cost: Decimal,
    /// Output tokens at the output rate.
    pub output_cost: Decimal,
    /// Sum of the three components above.
    pub total_cost: Decimal,
}

/// Cache-read discount divisor by model family, mirroring the provider defaults
/// documented on `LlmProvider::cache_read_discount` (Anthropic 10× i.e. 90% off,
/// OpenAI 2× i.e. 50% off, others no discount).
pub fn cache_read_discount(model_id: &str) -> Decimal {
    let lower = model_id.to_ascii_lowercase();
    // Strip provider prefixes (e.g. "openai/o1-mini" -> "o1-mini") so the
    // reasoning-tier `starts_with` checks match, mirroring `model_cost`.
    let name = lower.rsplit_once('/').map(|(_, n)| n).unwrap_or(&lower);
    if name.contains("claude") {
        Decimal::from(10)
    } else if name.contains("gpt")
        || name.starts_with("o1")
        || name.starts_with("o3")
        || name.starts_with("o4")
    {
        Decimal::from(2)
    } else {
        Decimal::ONE
    }
}

/// Price a run's cumulative token usage in USD for `model_id`.
///
/// `cache_read_input_tokens` is treated as a subset of `input_tokens` billed at
/// the model's cache-read discount; `cache_creation_input_tokens` is a separate
/// write-side count billed at the full input rate on top. Unknown models fall
/// back to [`default_cost`] (≈GPT-4o), so a new paid model never silently prices
/// at zero. This is the single pricing source shared by every surface that
/// reports per-run cost (OpenAI-compatible API, WebChat v2).
pub fn price_usage(
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: u32,
    cache_creation_input_tokens: u32,
) -> UsageCost {
    let (input_rate, output_rate) = model_cost(model_id).unwrap_or_else(default_cost);
    let discount = cache_read_discount(model_id);
    let billable_input = Decimal::from(
        input_tokens
            .saturating_sub(cache_read_input_tokens)
            .saturating_add(cache_creation_input_tokens),
    );
    let input_cost = billable_input * input_rate;
    let cached_input_cost = if discount > Decimal::ONE {
        Decimal::from(cache_read_input_tokens) * input_rate / discount
    } else {
        Decimal::from(cache_read_input_tokens) * input_rate
    };
    let output_cost = Decimal::from(output_tokens) * output_rate;
    let total_cost = input_cost + cached_input_cost + output_cost;
    UsageCost {
        input_cost,
        cached_input_cost,
        output_cost,
        total_cost,
    }
}

/// Format a USD `Decimal` for the wire: trimmed of trailing zeros, never in
/// scientific notation.
pub fn format_usd(amount: Decimal) -> String {
    amount.normalize().to_string()
}

/// Wire-facing per-run USD cost, split by billing category. Each amount is a
/// [`format_usd`]-formatted string so serialized cost never drifts into
/// scientific notation. Shared by every surface that reports run cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCost {
    pub input_cost_usd: String,
    pub cached_input_cost_usd: String,
    pub output_cost_usd: String,
    pub total_cost_usd: String,
    pub currency: String,
}

impl RunCost {
    /// ISO-4217 currency code for all amounts.
    pub const USD: &'static str = "USD";

    /// Price a run's cumulative token usage for `model_id` and format it for the
    /// wire. See [`price_usage`] for the billing rules.
    pub fn from_usage(
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_input_tokens: u32,
        cache_creation_input_tokens: u32,
    ) -> Self {
        let cost = price_usage(
            model_id,
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        );
        Self {
            input_cost_usd: format_usd(cost.input_cost),
            cached_input_cost_usd: format_usd(cost.cached_input_cost),
            output_cost_usd: format_usd(cost.output_cost),
            total_cost_usd: format_usd(cost.total_cost),
            currency: Self::USD.to_string(),
        }
    }
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

    #[test]
    fn test_cache_read_discount_by_family() {
        // Anthropic: 10× (90% off).
        assert_eq!(cache_read_discount("claude-opus-4-6"), Decimal::from(10));
        // OpenAI GPT + reasoning tiers (o1/o3/o4): 2× (50% off).
        assert_eq!(cache_read_discount("gpt-4o"), Decimal::from(2));
        assert_eq!(cache_read_discount("o1-mini"), Decimal::from(2));
        assert_eq!(cache_read_discount("o3-mini"), Decimal::from(2));
        // o4-mini is priced as an OpenAI reasoning model by `model_cost`, so it
        // must receive the same 2× cache discount (regression: previously fell
        // through to no discount).
        assert_eq!(cache_read_discount("o4-mini"), Decimal::from(2));
        // Provider-prefixed IDs must strip the prefix before matching, matching
        // `model_cost` (regression: "openai/o1-mini" previously got no discount).
        assert_eq!(cache_read_discount("openai/o1-mini"), Decimal::from(2));
        assert_eq!(cache_read_discount("azure/o4-mini"), Decimal::from(2));
        assert_eq!(
            cache_read_discount("anthropic/claude-opus-4-6"),
            Decimal::from(10)
        );
        // Unknown / local models: no discount.
        assert_eq!(cache_read_discount("llama-3.1-70b"), Decimal::ONE);
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
    fn price_usage_bills_input_and_output_at_model_rate() {
        // gpt-4o: input 0.0000025/tok, output 0.00001/tok.
        let cost = price_usage("gpt-4o", 1_000, 500, 0, 0);
        assert_eq!(cost.input_cost, dec!(0.0025));
        assert_eq!(cost.output_cost, dec!(0.005));
        assert_eq!(cost.total_cost, dec!(0.0075));
    }

    #[test]
    fn price_usage_discounts_cache_reads_and_bills_creation_at_full_rate() {
        // 3000 input of which 2000 were cache reads (claude 10× discount),
        // plus 1000 cache-creation tokens billed at the full input rate.
        // claude-opus input rate 0.000015/tok.
        let cost = price_usage("claude-opus-4-6", 3_000, 0, 2_000, 1_000);
        // fresh input = (3000 - 2000) + 1000 = 2000 → 2000 * 0.000015 = 0.03
        assert_eq!(cost.input_cost, dec!(0.03));
        // cached = 2000 * 0.000015 / 10 = 0.003
        assert_eq!(cost.cached_input_cost, dec!(0.003));
        assert_eq!(cost.total_cost, dec!(0.033));
    }

    #[test]
    fn run_cost_from_usage_formats_and_labels_currency() {
        let cost = RunCost::from_usage("gpt-4o", 1_000, 500, 0, 0);
        assert_eq!(cost.input_cost_usd, "0.0025");
        assert_eq!(cost.output_cost_usd, "0.005");
        assert_eq!(cost.total_cost_usd, "0.0075");
        assert_eq!(cost.currency, "USD");
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
