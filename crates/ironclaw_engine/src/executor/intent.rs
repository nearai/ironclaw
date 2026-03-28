//! Tool intent nudge detection.
//!
//! Detects when the LLM expresses intent to use a tool without actually
//! producing action calls (e.g. "Let me search..." or "I'll fetch...").
//! Mirrors the logic in `src/agent/agentic_loop.rs` `llm_signals_tool_intent`.

/// Check if a text response signals tool intent without actual action calls.
///
/// Returns `true` if the text contains phrases like "Let me search...",
/// "I'll fetch...", etc. that indicate the LLM wanted to call a tool.
pub fn signals_tool_intent(response: &str) -> bool {
    let lower = response.to_lowercase();

    // Skip false positives
    let false_positive_phrases = [
        "let me explain",
        "let me think",
        "let me know",
        "let me summarize",
        "let me clarify",
    ];
    for phrase in &false_positive_phrases {
        if lower.contains(phrase) {
            return false;
        }
    }

    let intent_prefixes = ["let me ", "i'll ", "i will ", "i'm going to "];
    let action_verbs = [
        "search", "look up", "check", "fetch", "find", "query", "read", "run", "execute", "call",
        "use", "invoke",
    ];

    for prefix in &intent_prefixes {
        if let Some(after) = lower.strip_prefix(prefix) {
            for verb in &action_verbs {
                if after.starts_with(verb) {
                    return true;
                }
            }
        }
        // Also check if the prefix appears mid-sentence (after period or newline)
        for sep in [". ", ".\n", "\n"] {
            for part in lower.split(sep) {
                let trimmed = part.trim();
                if let Some(after) = trimmed.strip_prefix(prefix) {
                    for verb in &action_verbs {
                        if after.starts_with(verb) {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

/// The nudge message injected into context when tool intent is detected.
pub const TOOL_INTENT_NUDGE: &str = "You expressed intent to use a tool but didn't make an action call. \
     Please go ahead and call the appropriate action.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_let_me_search() {
        assert!(signals_tool_intent("Let me search for that"));
    }

    #[test]
    fn detects_ill_fetch() {
        assert!(signals_tool_intent("I'll fetch the latest data"));
    }

    #[test]
    fn ignores_let_me_explain() {
        assert!(!signals_tool_intent("Let me explain how this works"));
    }

    #[test]
    fn ignores_let_me_know() {
        assert!(!signals_tool_intent("Let me know if you need more"));
    }

    #[test]
    fn ignores_plain_text() {
        assert!(!signals_tool_intent("The answer is 42."));
    }

    #[test]
    fn detects_after_period() {
        assert!(signals_tool_intent("Sure. Let me search for that."));
    }
}
