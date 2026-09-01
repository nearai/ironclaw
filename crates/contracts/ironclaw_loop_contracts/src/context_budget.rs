/// Shared token budget for transcript context admitted into prompt-shaped model
/// input.
///
/// Storage still scans transcript context by message count. Host adapters use
/// this budget after that scan, and compaction strategies use the same budget
/// shape to decide when the observed prompt is near its context ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PromptContextTokenBudget {
    pub context_limit_tokens: u64,
    pub reserve_tokens: u64,
    pub main_loop_max_output_tokens: u64,
}

impl PromptContextTokenBudget {
    pub const DEFAULT_CONTEXT_LIMIT_TOKENS: u64 = 128_000;
    pub const DEFAULT_RESERVE_TOKENS: u64 = 20_000;
    pub const DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS: u64 = 0;

    /// Fraction of a provider-advertised window we are willing to fill.
    ///
    /// This margin exists to absorb error in the chars/4 token estimate
    /// (`estimate_tokens_from_chars`), which is the only reason for it. Room
    /// for the model's *response* is a separate axis — `reserve_tokens`.
    pub const DEFAULT_USABLE_FRACTION_PERCENT: u64 = 90;

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

    /// Derive a budget from a provider-advertised total context window.
    ///
    /// `None` (or a nonsense zero) reproduces the compiled-in default
    /// exactly, so a provider that advertises nothing behaves as it always
    /// has. Never guess a window for an unknown model: guessing high
    /// produces the provider rejection this mechanism exists to avoid.
    pub fn from_advertised_window(advertised_tokens: Option<u64>) -> Self {
        let Some(advertised) = advertised_tokens.filter(|tokens| *tokens > 0) else {
            return Self::default();
        };
        let context_limit_tokens =
            advertised.saturating_mul(Self::DEFAULT_USABLE_FRACTION_PERCENT) / 100;
        // A small-window model would otherwise have its whole budget consumed
        // by the flat response reserve, leaving zero visible transcript.
        let reserve_tokens = Self::DEFAULT_RESERVE_TOKENS.min(context_limit_tokens / 4);
        Self {
            context_limit_tokens,
            reserve_tokens,
            main_loop_max_output_tokens: Self::DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS,
        }
    }
}

impl Default for PromptContextTokenBudget {
    fn default() -> Self {
        Self {
            context_limit_tokens: Self::DEFAULT_CONTEXT_LIMIT_TOKENS,
            reserve_tokens: Self::DEFAULT_RESERVE_TOKENS,
            main_loop_max_output_tokens: Self::DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS,
        }
    }
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

    #[test]
    fn advertised_window_of_none_reproduces_the_compiled_in_default() {
        // A provider that reports nothing must behave exactly as it does
        // today. This is the compatibility guarantee of the whole change.
        assert_eq!(
            PromptContextTokenBudget::from_advertised_window(None),
            PromptContextTokenBudget::default()
        );
    }

    #[test]
    fn advertised_window_of_zero_is_treated_as_unknown() {
        assert_eq!(
            PromptContextTokenBudget::from_advertised_window(Some(0)),
            PromptContextTokenBudget::default()
        );
    }

    #[test]
    fn large_advertised_window_keeps_the_flat_response_reserve() {
        let budget = PromptContextTokenBudget::from_advertised_window(Some(2_000_000));

        assert_eq!(budget.context_limit_tokens, 1_800_000);
        assert_eq!(
            budget.reserve_tokens,
            PromptContextTokenBudget::DEFAULT_RESERVE_TOKENS
        );
        assert_eq!(budget.visible_transcript_tokens(), 1_780_000);
    }

    #[test]
    fn small_advertised_window_clamps_the_reserve_and_keeps_budget_usable() {
        // An 8k model would otherwise have its entire budget consumed by the
        // flat 20k response reserve, leaving zero visible transcript and a
        // loop that cannot run at all.
        let budget = PromptContextTokenBudget::from_advertised_window(Some(8_000));

        assert_eq!(budget.context_limit_tokens, 7_200);
        assert_eq!(budget.reserve_tokens, 1_800);
        assert!(
            budget.visible_transcript_tokens() > 0,
            "a small-window model must still have room for transcript"
        );
    }

    #[test]
    fn advertised_window_matching_todays_constant_is_reduced_by_the_margin() {
        // 128k advertised is NOT the same as the 128k fallback: the fallback
        // is a guess, an advertised value gets the estimate-error margin.
        let budget = PromptContextTokenBudget::from_advertised_window(Some(128_000));

        assert_eq!(budget.context_limit_tokens, 115_200);
        assert_eq!(
            budget.reserve_tokens,
            PromptContextTokenBudget::DEFAULT_RESERVE_TOKENS
        );
    }
}
