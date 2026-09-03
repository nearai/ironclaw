use super::*;

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
fn smallest_positive_window_is_treated_as_unknown() {
    // Some(1) survives the `> 0` filter but derives a zero visible
    // transcript, which must fall back to the default exactly like
    // None and Some(0).
    assert_eq!(
        PromptContextTokenBudget::from_advertised_window(Some(1)),
        PromptContextTokenBudget::default()
    );
}

#[test]
fn smallest_usable_window_keeps_a_nonzero_visible_transcript() {
    // Find the smallest advertised window whose derivation does NOT
    // fall back to the default, and prove it still leaves visible
    // transcript room rather than trusting the arithmetic.
    let smallest_non_default = (1..=16)
        .find(|&candidate| {
            PromptContextTokenBudget::from_advertised_window(Some(candidate))
                != PromptContextTokenBudget::default()
        })
        .expect("some small window must derive a non-default budget");

    let budget = PromptContextTokenBudget::from_advertised_window(Some(smallest_non_default));

    assert_ne!(budget, PromptContextTokenBudget::default());
    assert!(
        budget.visible_transcript_tokens() > 0,
        "the smallest non-default derived budget must still leave visible transcript room"
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
