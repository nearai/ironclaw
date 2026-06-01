//! Reasoning/thinking model detection utilities.
//!
//! ## Default: assume native thinking
//!
//! Most modern LLMs either have built-in thinking (Qwen3, DeepSeek-R1, GLM-5)
//! or work fine without `<think>/<final>` prompt injection (GPT-4o, Claude,
//! Llama). Injecting `<think>/<final>` tags into a native-thinking model's
//! system prompt causes broken responses: the model puts reasoning in its
//! native `reasoning` field and only `<think>` tags in `content`, which the
//! response cleaner strips to empty.
//!
//! We therefore **default to NOT injecting** `<think>/<final>` tags. Only
//! models explicitly listed in `REQUIRES_THINK_FINAL_PATTERNS` get the strict
//! tag format. This is the safe default because:
//!
//! - Skipping injection for a model that could use it = slightly less
//!   structured but working responses
//! - Injecting into a native-thinking model = broken/empty responses
//!
//! This also handles model aliases like NEAR AI's `"auto"` which resolve
//! server-side to models like `Qwen/Qwen3.5-122B-A10B`. Since `"auto"`
//! doesn't match any pattern, it falls through to the safe default.

/// Models that explicitly require `<think>/<final>` prompt injection.
///
/// These are models proven to benefit from structured thinking tags AND
/// that do NOT have native thinking support. The list is intentionally
/// empty/minimal — the safe default is to skip injection.
const REQUIRES_THINK_FINAL_PATTERNS: &[&str] = &[
    // Currently empty: no models have been identified that require
    // <think>/<final> injection to function correctly. Add patterns
    // here only when a specific model is proven to need them.
];

/// Check if a model requires explicit `<think>/<final>` prompt injection.
///
/// Returns `true` only for models in the allowlist that are known to need
/// structured thinking tags. All other models — including unknown names,
/// aliases like `"auto"`, and native-thinking models — return `false` and
/// get the direct-answer prompt format.
pub fn requires_think_final_tags(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    REQUIRES_THINK_FINAL_PATTERNS
        .iter()
        .any(|p| lower.contains(p))
}

/// Legacy helper — returns `true` for known native-thinking models.
///
/// Retained for call sites that need to know whether a model has native
/// thinking (e.g. for response parsing heuristics), but no longer used
/// for prompt injection decisions. Use [`requires_think_final_tags`] for
/// that instead.
pub fn has_native_thinking(model: &str) -> bool {
    const NATIVE_THINKING_PATTERNS: &[&str] = &[
        "qwen3",
        "qwq",
        "deepseek-r1",
        "deepseek-reasoner",
        "glm-z1",
        "glm-4-plus",
        "glm-5",
        "nanbeige",
        "step-3.5",
        "minimax-m2",
    ];
    let lower = model.to_ascii_lowercase();
    NATIVE_THINKING_PATTERNS.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── requires_think_final_tags tests ──

    #[test]
    fn unknown_models_do_not_require_tags() {
        assert!(!requires_think_final_tags("gpt-4o"));
        assert!(!requires_think_final_tags("claude-3-5-sonnet"));
        assert!(!requires_think_final_tags("llama-3.1-70b"));
        assert!(!requires_think_final_tags("mistral-7b"));
        assert!(!requires_think_final_tags("gemini-2.0-flash"));
    }

    #[test]
    fn auto_alias_does_not_require_tags() {
        assert!(!requires_think_final_tags("auto"));
    }

    #[test]
    fn resolved_qwen_does_not_require_tags() {
        assert!(!requires_think_final_tags("Qwen/Qwen3.5-122B-A10B"));
        assert!(!requires_think_final_tags("qwen3-8b"));
        assert!(!requires_think_final_tags("Qwen3.5-35B"));
    }

    #[test]
    fn native_thinking_models_do_not_require_tags() {
        assert!(!requires_think_final_tags("deepseek-r1-distill-qwen-32b"));
        assert!(!requires_think_final_tags("deepseek-reasoner"));
        assert!(!requires_think_final_tags("glm-z1-airx"));
        assert!(!requires_think_final_tags("GLM-5"));
        assert!(!requires_think_final_tags("qwq-32b"));
    }

    #[test]
    fn empty_and_unusual_names_do_not_require_tags() {
        assert!(!requires_think_final_tags(""));
        assert!(!requires_think_final_tags("some-custom-model-v2"));
    }

    // ── has_native_thinking legacy tests ──

    #[test]
    fn detects_qwen3_models() {
        assert!(has_native_thinking("qwen3-coder-next-80b"));
        assert!(has_native_thinking("Qwen3.5-35B"));
        assert!(has_native_thinking("qwen3-0.6b"));
        assert!(has_native_thinking("qwen3:8b"));
        assert!(has_native_thinking("qwen3-30b-a3b"));
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
        assert!(!has_native_thinking("qwen2.5:7b"));
        assert!(!has_native_thinking("qwen2.5-instruct"));
        assert!(!has_native_thinking("glm-4-flash"));
        assert!(!has_native_thinking("glm-4-air"));
        assert!(!has_native_thinking("glm-4v"));
        assert!(!has_native_thinking("step-3-mini"));
    }
}
