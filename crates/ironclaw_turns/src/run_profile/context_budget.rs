/// Shared token budget for transcript context admitted into prompt-shaped model
/// input.
///
/// Storage still scans transcript context by message count. Host adapters use
/// this budget after that scan, and compaction strategies use the same budget
/// shape to decide when the observed prompt is near its context ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PromptContextTokenBudget {
    pub context_limit_tokens: u64,
    pub reserve_tokens: u64,
    pub main_loop_max_output_tokens: u64,
}

impl PromptContextTokenBudget {
    pub const DEFAULT_CONTEXT_LIMIT_TOKENS: u64 = 128_000;
    pub const DEFAULT_RESERVE_TOKENS: u64 = 20_000;
    pub const DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS: u64 = 0;

    /// Env overrides for the compaction context budget. Lets an operator (or the
    /// nearai-bench harness) tighten the visible-transcript ceiling so long,
    /// tool-heavy runs compact sooner and stop re-sending a near-full context
    /// window every turn — the dominant token cost on multi-turn tasks (on top of
    /// progressive tool disclosure). Unset ⇒ the compiled defaults, so this is a
    /// pure no-op for existing deployments.
    pub const CONTEXT_LIMIT_TOKENS_ENV: &'static str = "IRONCLAW_PROMPT_CONTEXT_LIMIT_TOKENS";
    pub const RESERVE_TOKENS_ENV: &'static str = "IRONCLAW_PROMPT_CONTEXT_RESERVE_TOKENS";

    pub const fn new(
        context_limit_tokens: u64,
        reserve_tokens: u64,
        main_loop_max_output_tokens: u64,
    ) -> Self {
        Self {
            context_limit_tokens,
            reserve_tokens,
            main_loop_max_output_tokens,
        }
    }

    pub fn visible_transcript_tokens(self) -> u64 {
        self.context_limit_tokens
            .saturating_sub(self.reserve_tokens.max(self.main_loop_max_output_tokens))
    }
}

impl Default for PromptContextTokenBudget {
    fn default() -> Self {
        Self {
            context_limit_tokens: env_u64_positive(
                Self::CONTEXT_LIMIT_TOKENS_ENV,
                Self::DEFAULT_CONTEXT_LIMIT_TOKENS,
            ),
            reserve_tokens: env_u64_positive(
                Self::RESERVE_TOKENS_ENV,
                Self::DEFAULT_RESERVE_TOKENS,
            ),
            main_loop_max_output_tokens: Self::DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS,
        }
    }
}

/// Parse a positive `u64` from `key`, falling back to `default` when unset,
/// unparseable, or non-positive. A zero/garbage override never silently
/// disables the budget.
fn env_u64_positive(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::PromptContextTokenBudget;

    #[test]
    fn visible_transcript_tokens_reserves_larger_output_buffer() {
        let budget = PromptContextTokenBudget::new(100, 10, 30);

        assert_eq!(budget.visible_transcript_tokens(), 70);
    }

    #[test]
    fn visible_transcript_tokens_saturates_when_reserve_exceeds_limit() {
        let budget = PromptContextTokenBudget::new(10, 20, 0);

        assert_eq!(budget.visible_transcript_tokens(), 0);
    }

    #[test]
    fn visible_transcript_tokens_uses_reserve_when_larger_than_output_budget() {
        let budget = PromptContextTokenBudget::new(100, 30, 10);

        assert_eq!(budget.visible_transcript_tokens(), 70);
    }
}
