//! The model-derived prompt context budget, proven through a real turn.
//!
//! What this pins: when the run's model advertises a context window, the loop
//! derives its prompt budget from that window instead of the compiled-in
//! 128k/20k default, and the model is sent a smaller transcript as a result.
//!
//! What this tier uniquely proves is the production wiring no crate test
//! reaches: the scripted provider's `model_metadata().context_length` flows
//! through the real `LlmProviderModelGateway` (route-identity check included)
//! into the turn runner, onto the run context, and into loop behavior.
//!
//! What it deliberately does NOT separate: the budget has two consumers on
//! this path — compaction and outbound request sizing — and they share one
//! threshold (`visible_transcript_tokens`), so compaction always fires first
//! and a run looks the same here whether or not request sizing is wired.
//! Each link of the request-sizing chain is pinned at crate tier instead,
//! with mutation-verified tests:
//! `ironclaw_loop_host` (`thread_resolving_gateway_applies_its_prompt_context_budget_to_message_selection`)
//! and `ironclaw_turn_runner` (`derived_budget_sizes_the_request_the_host_sends_to_the_gateway`).
//!
//! Sizing (both ceilings in characters, because the token figures collide
//! numerically and are easy to mix up):
//!
//! | run | visible budget | at chars/4 |
//! |---|---|---|
//! | advertised 40,000 -> limit 36,000, reserve min(20k, 9k)=9,000 | 27,000 tok | ~108,000 chars |
//! | unadvertised -> limit 128,000, reserve 20,000 | 108,000 tok | ~432,000 chars |
//!
//! The seeded transcript sits between the two, so only the advertised run is
//! forced to shrink what the model sees.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

/// One seeded user message. Six of these put ~240,000 characters of
/// transcript on the thread — comfortably over the advertised run's
/// ~108,000-character ceiling and comfortably under the default's ~432,000.
fn bulky_message(tag: &str) -> String {
    format!("{tag} {}", "context padding. ".repeat(2_500))
}

/// Builds a harness, seeds a six-turn bulky transcript on it, and returns it
/// so the caller can inspect whatever it needs about the last captured
/// model request.
async fn harness_with_seeded_transcript(
    advertised_window: Option<u32>,
) -> RebornIntegrationHarness {
    // One script entry per MODEL CALL, not per turn: the advertised run
    // compacts, and each compaction is an extra summarization call. Script
    // generously so the FIFO is never the thing that ends a run; unconsumed
    // entries are inert.
    let replies: Vec<_> = (1..=24)
        .map(|n| RebornScriptedReply::text(format!("ack {n}")))
        .collect();
    let mut builder = RebornIntegrationHarness::test_default().script(replies);
    if let Some(tokens) = advertised_window {
        builder = builder.advertised_context_window(tokens);
    }
    let harness = builder.build().await.expect("harness builds");

    for turn in 1..=6 {
        harness
            .submit_turn(&bulky_message(&format!("turn-{turn}")))
            .await
            .expect("turn completes");
    }

    harness
}

/// Seeds a transcript, then returns how many messages the model received on
/// its final captured call — the largest transcript, so it is where the two
/// budgets diverge most.
async fn messages_sent_on_last_call(advertised_window: Option<u32>) -> usize {
    harness_with_seeded_transcript(advertised_window)
        .await
        .captured_last_request_message_count()
}

#[tokio::test]
async fn small_advertised_window_shrinks_what_the_model_receives() {
    let narrow = messages_sent_on_last_call(Some(40_000)).await;
    let wide = messages_sent_on_last_call(None).await;

    assert!(
        narrow < wide,
        "a 40k-window model must be sent fewer transcript messages than an \
         unadvertised one; got {narrow} vs {wide}"
    );
}

#[tokio::test]
async fn unadvertised_window_keeps_the_compiled_in_ceiling() {
    // The complement of the test above: with nothing advertised the run must
    // behave exactly as it did before this feature existed, so the whole
    // seeded transcript still fits — prove it by finding every one of the six
    // seeded turn markers still present in the last captured request, not
    // just a message count that a truncation bug could still satisfy.
    let harness = harness_with_seeded_transcript(None).await;
    let contents = harness.captured_last_request_contents();

    for turn in 1..=6 {
        let marker = format!("turn-{turn}");
        assert!(
            contents.iter().any(|content| content.contains(&marker)),
            "an unadvertised run must still carry its full transcript; \
             missing seeded marker {marker:?} from the last captured request: {contents:?}"
        );
    }
}
