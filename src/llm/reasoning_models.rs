//! Reasoning/thinking model detection utilities.
//!
//! Models with native thinking support produce structured chain-of-thought
//! via `reasoning_content` fields or built-in `<think>` tags. Injecting
//! IronClaw's own `<think>/<final>` format instructions into the system
//! prompt collides with these models' native behavior, causing:
//! - Thinking-only responses with no visible content
//! - Double-wrapped thinking tags that confuse response cleaning
//!
//! When a model has native thinking, we skip the `<think>/<final>` prompt
//! injection and let the model use its own format. The response cleaning
//! pipeline already handles stripping all known thinking tag variants.
//!
//! ## Design note: why match broadly (e.g. all Qwen3)?
//!
//! Some families (Qwen3) have ALL variants trained with native `<think>` tags,
//! even tiny models like 0.6B. Thinking can be disabled at inference time via
//! `enable_thinking=false`, but we can't detect that from the model name alone.
//! We err on the safe side: skip injection for all variants because:
//! - False negative (inject when model thinks natively) = broken responses
//! - False positive (skip injection for non-thinking model) = less structured
//!   but working responses
//!
//! For families where only SOME variants reason (GLM-4), we match specific
//! sub-families (glm-z1, glm-4-plus) to avoid false positives.

/// Known model families with native thinking/reasoning support.
///
/// These models produce chain-of-thought reasoning either via a dedicated
/// `reasoning_content` response field or via built-in `<think>` tags that
/// the model was trained to emit without prompt injection.
const NATIVE_THINKING_PATTERNS: &[&str] = &[
    // Qwen3 family — ALL variants (0.6B through 235B) emit native <think> tags
    // by default. Thinking can be toggled via `enable_thinking` parameter or
    // `/think` `/no_think` soft switches, but the default is ON and we can't
    // detect the runtime setting from the model name.
    "qwen3",
    // QwQ is Qwen's dedicated reasoning model (based on Qwen2.5-32B + RL).
    // Always thinks, no disable toggle.
    "qwq",
    // DeepSeek reasoning models — native reasoning_content field
    "deepseek-r1",
    "deepseek-reasoner",
    // GLM reasoning variants only (glm-4-flash, glm-4-air, glm-4v do NOT reason)
    "glm-z1",
    "glm-4-plus",
    "glm-5",
    // Nanbeige reasoning models
    "nanbeige",
    // Step reasoning models (3.5+ have native thinking; step-3 base does not)
    "step-3.5",
    // MiniMax reasoning models
    "minimax-m2",
];

/// Check if a model name indicates native thinking/reasoning support.
///
/// Models that return `true` should NOT have IronClaw's `<think>/<final>`
/// format instructions injected into their system prompt, as this collides
/// with their built-in reasoning behavior.
///
/// Note: this is a best-effort heuristic based on model name. Some models
/// support toggling thinking at runtime (e.g. Qwen3's `enable_thinking`),
/// which we cannot detect here. We default to assuming thinking is ON for
/// models that have it, since that's the default behavior.
pub fn has_native_thinking(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    NATIVE_THINKING_PATTERNS.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_qwen3_models() {
        // All Qwen3 variants have native thinking (even small ones)
        assert!(has_native_thinking("qwen3-coder-next-80b"));
        assert!(has_native_thinking("Qwen3.5-35B"));
        assert!(has_native_thinking("qwen3-0.6b"));
        assert!(has_native_thinking("qwen3:8b"));
        assert!(has_native_thinking("qwen3-30b-a3b"));
        // Ollama-style tag format
        assert!(has_native_thinking("qwen3-coder:latest"));
    }

    #[test]
    fn detects_qwq() {
        assert!(has_native_thinking("qwq-32b"));
        assert!(has_native_thinking("QwQ-32B-Preview"));
    }

    #[test]
    fn detects_deepseek_reasoning() {
        assert!(has_native_thinking("deepseek-r1-distill-qwen-32b"));
        assert!(has_native_thinking("deepseek-reasoner"));
    }

    #[test]
    fn detects_glm_reasoning_variants() {
        assert!(has_native_thinking("glm-z1-airx"));
        assert!(has_native_thinking("glm-4-plus"));
        assert!(has_native_thinking("GLM-5"));
    }

    #[test]
    fn detects_other_reasoning_models() {
        assert!(has_native_thinking("nanbeige-4.1-3b"));
        assert!(has_native_thinking("step-3.5-flash-197b"));
        assert!(has_native_thinking("minimax-m2.5-139b"));
        assert!(has_native_thinking("MiniMax-M2.7"));
        assert!(has_native_thinking("MiniMax-M2.7-highspeed"));
    }

    #[test]
    fn rejects_non_reasoning_models() {
        assert!(!has_native_thinking("gpt-4o"));
        assert!(!has_native_thinking("claude-3-5-sonnet"));
        assert!(!has_native_thinking("llama-3.1-70b"));
        assert!(!has_native_thinking("mistral-7b"));
        assert!(!has_native_thinking("gemini-2.0-flash"));
    }

    #[test]
    fn rejects_non_reasoning_variants_in_same_family() {
        // Qwen2.5 does NOT have native thinking (only Qwen3/QwQ do)
        assert!(!has_native_thinking("qwen2.5:7b"));
        assert!(!has_native_thinking("qwen2.5-instruct"));
        // GLM-4 base variants do NOT have reasoning_content
        assert!(!has_native_thinking("glm-4-flash"));
        assert!(!has_native_thinking("glm-4-air"));
        assert!(!has_native_thinking("glm-4v"));
        // step-3 base does not reason (only 3.5+)
        assert!(!has_native_thinking("step-3-mini"));
    }
}
