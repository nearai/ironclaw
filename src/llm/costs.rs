//! Per-model cost lookup table for multi-provider LLM support.
//!
//! Returns (input_cost_per_token, output_cost_per_token) as Decimal pairs.
//! Ollama and other local models return zero cost.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Look up known per-token costs for a model by its identifier.
///
/// Returns `Some((input_cost, output_cost))` for known models, `None` otherwise.
pub fn model_cost(model_id: &str) -> Option<(Decimal, Decimal)> {
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
}
