use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use std::time::Duration;

/// Delivery idempotency and honest failure through the production stack.
///
/// Covers (docs/qa/telegram-coverage-map.md): qa-telegram:F1 (a retried
/// update produces exactly one turn and one reply), qa-telegram:F3:01 (a
/// blocked-recipient 403 on the reply send does not retry-storm),
/// qa-telegram:F3:02 (the next send succeeds once the failure clears), and
/// the integration leg of qa-telegram:F2 (send outages surface as honest
/// failures — the DeliveryStatus mapping itself is pinned by
/// ironclaw_telegram_v2_adapter's render tests).
#[tokio::test]
async fn telegram_duplicate_updates_and_send_failures_stay_honest() {
    let stack = build_journey_stack([
        RebornScriptedReply::text("first reply"),
        RebornScriptedReply::text("reply during outage"),
        RebornScriptedReply::text("reply after recovery"),
    ])
    .await;
    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    // F1: the same update delivered twice (Telegram redelivery) produces
    // exactly one turn and one reply. Turn-count seam: the model script is a
    // strict FIFO, so a second (deduplication-escaping) turn MUST consume and
    // deliver the NEXT scripted entry — asserting zero later-entry sends is
    // the authoritative "no second turn" proof, and the delivered count
    // (2xx-answered, not merely captured) pins exactly one reply.
    let update = dm_update(2, "count me once");
    assert_eq!(
        stack.webhook_update(&secret, update.clone()).await,
        StatusCode::OK
    );
    assert_eq!(stack.webhook_update(&secret, update).await, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("first reply"))
        .await
        .expect("the update produces its reply");
    // No sleep: a straggling duplicate dispatch cannot escape this scenario —
    // the script is a FIFO, so a second turn would consume "reply during
    // outage" and the outage leg below would then fail its exact-outcome
    // assertion. That downstream fence, plus the entry-absence asserts here,
    // prove exactly-once deterministically.
    assert_eq!(
        stack.network.delivered_sends_containing("first reply"),
        1,
        "a redelivered update must not produce a second delivered reply"
    );
    for later_entry in ["reply during outage", "reply after recovery"] {
        assert_eq!(
            stack.network.delivered_sends_containing(later_entry),
            0,
            "a second turn would have consumed the next scripted entry ({later_entry}); \
             its absence proves exactly one turn ran"
        );
    }

    // F3:01 — the recipient blocks the bot: the reply send gets a 403.
    // Honest failure, no retry storm (the adapter maps 403 to
    // FailedUnauthorized — a terminal, non-retryable delivery status).
    stack
        .network
        .fail_send_containing("reply during outage", 403);
    assert_eq!(
        stack
            .webhook_dm(&secret, 3, "talk to me during the outage")
            .await,
        StatusCode::OK
    );
    // Deterministic positive seam: wait for the outage reply's provider
    // outcome to be recorded (the 403 answer), then assert its exactness.
    // "Never retried" is fenced by the recovery leg below: a retry would
    // consume/emit an extra outcome and break its exact assertions.
    for _ in 0..200 {
        if !stack.network.send_outcomes().iter().any(|(body, _)| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains("reply during outage"))
        }) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        } else {
            break;
        }
    }
    let outage_outcomes: Vec<u16> = stack
        .network
        .send_outcomes()
        .iter()
        .filter(|(body, _)| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains("reply during outage"))
        })
        .map(|(_, status)| *status)
        .collect();
    assert_eq!(
        outage_outcomes,
        vec![403],
        "the outage turn's reply must be attempted exactly once, answered 403, \
         and never retried"
    );
    assert_eq!(
        stack
            .network
            .delivered_sends_containing("reply after recovery"),
        0,
        "the recovery entry must not have been consumed before the recovery turn"
    );

    // F3:02 — the block clears; the next turn's reply DELIVERS (2xx-answered,
    // not merely captured — the 403'd request above is also "captured").
    assert_eq!(
        stack.webhook_dm(&secret, 4, "are we back?").await,
        StatusCode::OK
    );
    let mut recovered_deliveries = 0;
    for _ in 0..200 {
        recovered_deliveries = stack
            .network
            .delivered_sends_containing("reply after recovery");
        if recovered_deliveries > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        recovered_deliveries, 1,
        "exactly one delivered recovery reply after the failure clears"
    );

    // Terminal absence seam: shutdown drains/cancels the production runtime's
    // delivery work. Only after that lifecycle fence can we prove no delayed
    // retry followed the terminal 403.
    stack.runtime.shutdown().await.expect("runtime shuts down");
    let final_outage_outcomes: Vec<u16> = stack
        .network
        .send_outcomes()
        .iter()
        .filter(|(body, _)| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains("reply during outage"))
        })
        .map(|(_, status)| *status)
        .collect();
    assert_eq!(
        final_outage_outcomes,
        vec![403],
        "after the runtime's terminal drain, the unauthorized reply still has exactly one attempt"
    );
}
