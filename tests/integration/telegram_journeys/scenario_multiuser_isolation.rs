use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::Method;
use axum::http::StatusCode;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use serde_json::Value;

/// Multi-user identity isolation through the production stack.
///
/// Covers (docs/qa/telegram-coverage-map.md): qa-telegram:D1:01 (threads
/// owned by the bound user), qa-telegram:D1:02 (replies return only to the
/// originating bound user), qa-telegram:D4 (unpairing A leaves B intact),
/// qa-telegram:P6 integration leg (an identity bound to another user is
/// refused a re-bind by code), qa-telegram:R2 (self-service disconnect
/// unpairs only the requester), qa-multitenant-setup:J5:03 (one paired user
/// plus an unbound provider identity coexist without bleed), and
/// qa-telegram:R8 (a DM after removal gets the static hint, no turn).
#[tokio::test]
async fn telegram_two_users_stay_isolated_across_pairing_reply_and_unpair() {
    const USER_B: &str = "tg-journey-user-b";
    const TG_USER_B: i64 = 9002;
    const TG_CHAT_B: i64 = 556;

    // FIFO: A's DM reply, B's DM reply, B's post-unpair reply. A's
    // post-unpair DM must consume NOTHING (hint only, no turn).
    let stack = build_journey_stack([
        RebornScriptedReply::text("isolated reply for user A"),
        RebornScriptedReply::text("isolated reply for user B"),
        RebornScriptedReply::text("user B still works"),
    ])
    .await;
    let caller_a = stack.caller.clone();
    let caller_b = WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).expect("tenant"),
        UserId::new(USER_B).expect("user"),
        Some(AgentId::new(AGENT).expect("agent")),
        None,
    );

    let secret = admin_save(&stack).await;
    pair_user_via_webhook(&stack, &secret, &caller_a, TG_USER_ID, TG_CHAT_ID, 1).await;
    pair_user_via_webhook(&stack, &secret, &caller_b, TG_USER_B, TG_CHAT_B, 2).await;

    // Each DM routes to ITS user's thread and replies to ITS chat only.
    let status = stack.webhook_dm(&secret, 3, "hello from A").await;
    assert_eq!(status, StatusCode::OK);
    let reply_a = stack
        .wait_for_dm_send(|text| text.contains("isolated reply for user A"))
        .await
        .expect("A's DM gets A's reply");
    assert_eq!(reply_a["chat_id"], TG_CHAT_ID);

    let status = stack
        .webhook_update(
            &secret,
            dm_update_from(4, TG_USER_B, TG_CHAT_B, "hello from B"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let reply_b = stack
        .wait_for_send_in(TG_CHAT_B, |text| text.contains("isolated reply for user B"))
        .await
        .expect("B's DM gets B's reply");
    assert_eq!(
        reply_b["chat_id"], TG_CHAT_B,
        "B's reply must land in B's chat only"
    );
    let cross_bleed = stack
        .network
        .request_bodies_for("/sendMessage")
        .iter()
        .any(|body| {
            (body["chat_id"] == TG_CHAT_ID
                && body["text"].as_str().is_some_and(|t| t.contains("user B")))
                || (body["chat_id"] == TG_CHAT_B
                    && body["text"].as_str().is_some_and(|t| t.contains("user A")))
        });
    assert!(!cross_bleed, "replies must never cross user boundaries");

    // P6: a code minted by B cannot re-bind A's telegram identity.
    let code_b = issue_pairing_code(&stack, &caller_b).await;
    let status = stack
        .webhook_update(
            &secret,
            dm_update_from(5, TG_USER_ID, TG_CHAT_ID, &format!("/start {code_b}")),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("already paired"))
        .await
        .expect("the already-bound identity gets an explicit refusal");
    assert!(
        pairing_connected(&stack, &caller_a).await,
        "A's binding survives the re-bind attempt"
    );

    // D4/R2: A disconnects; B is untouched.
    let (status, _body) = call_route(
        stack.mounts.protected_routes().router,
        Method::DELETE,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller_a.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "self-service unpair succeeds"
    );
    assert!(!pairing_connected(&stack, &caller_a).await, "A unpaired");
    assert!(pairing_connected(&stack, &caller_b).await, "B still paired");

    // R8: A's DM now fails closed with the static hint and starts NO turn —
    // proven at the send seam: the only new delivered 555-send is the hint,
    // and no scripted entry was consumed (a turn would have delivered
    // "user B still works", the next FIFO entry, into A's chat).
    let outcomes_before = stack.network.send_outcomes().len();
    let status = stack.webhook_dm(&secret, 6, "hello again from A").await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("Pair your account"))
        .await
        .expect("unpaired A gets the static pairing hint");
    let new_outcomes: Vec<(Value, u16)> = stack.network.send_outcomes().split_off(outcomes_before);
    assert!(
        new_outcomes.iter().all(|(body, _)| {
            body["chat_id"] == TG_CHAT_ID
                && body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Pair your account"))
        }),
        "a post-unpair DM must produce ONLY the static hint (no turn, no reply): {new_outcomes:?}"
    );

    // B keeps working end to end.
    let status = stack
        .webhook_update(
            &secret,
            dm_update_from(7, TG_USER_B, TG_CHAT_B, "still working?"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let reply = stack
        .wait_for_send_in(TG_CHAT_B, |text| text.contains("user B still works"))
        .await
        .expect("B's channel is unaffected by A's unpair");
    assert_eq!(reply["chat_id"], TG_CHAT_B);

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
