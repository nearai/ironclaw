use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::Method;
use axum::http::StatusCode;
use serde_json::json;

/// Ben's fourth regression (2026-07-17): unpair → re-pair resurrected the OLD
/// thread — including its parked BlockedAuth run, whose busy hint greeted the
/// freshly re-paired user. Telegram's `unpair` cleaned pairing codes,
/// identity bindings, and DM targets but never the conversation-actor
/// pairing (Slack's disconnect cleans all four), so the chat re-attached to
/// the stale thread. Pins the fresh-slate journey: park a run → self-service
/// disconnect → re-pair → the next DM starts a FRESH conversation (real
/// reply), not the old run's busy hint.
#[tokio::test]
async fn telegram_unpair_then_repair_starts_fresh_thread_not_the_old_blocked_one() {
    let stack = build_journey_stack_with_google_oauth([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::text("fresh conversation reply"),
        RebornScriptedReply::text("fresh conversation reply"),
    ])
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    // Park a run on the auth gate in the paired conversation.
    let status = stack.webhook_dm(&secret, 2, "can you set up gmail?").await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("accounts.google.com"))
        .await
        .expect("the gated install parks with a visible auth prompt");

    // Self-service disconnect through the production route.
    let (status, body) = call_route(
        stack.mounts.protected_routes().router,
        Method::DELETE,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(stack.caller.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "disconnect response: {body}"
    );

    // Ben's fifth regression (2026-07-17): in the disconnected-but-installed
    // state ("setup needed"), a DM produced NOTHING — not even the static
    // pairing hint the unpaired contract promises. Reproduce his exact
    // sequence: paired turns happened above, a fresh code is pending (the
    // setup panel minted one), and the disconnected user DMs plain text.
    let (status, body) = call_route(
        stack.mounts.protected_routes().router,
        Method::POST,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(stack.caller.clone()),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "fresh code mints: {body}");
    let status = stack.webhook_dm(&secret, 3, "anyone there?").await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("Pair your account"))
        .await
        .expect(
            "a disconnected (installed, unpaired) DM must get the static pairing hint, not silence",
        );

    // Re-pair with a fresh code.
    pair_via_webhook(&stack, &secret, 4).await;

    // The re-paired user's first message must start a fresh conversation —
    // a real reply, never the resurrected run's auth-busy hint.
    let sends_before = stack.network.request_bodies_for("/sendMessage").len();
    let status = stack.webhook_dm(&secret, 5, "hello again!").await;
    assert_eq!(status, StatusCode::OK);
    let reply = stack
        .wait_for_dm_send(|text| text.contains("fresh conversation reply"))
        .await
        .expect("a re-paired user's DM must start fresh, not hit the old blocked thread");
    assert_eq!(reply["chat_id"], TG_CHAT_ID);
    let post_repair_sends: Vec<String> = stack.network.request_bodies_for("/sendMessage")
        [sends_before..]
        .iter()
        .filter_map(|body| body["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        !post_repair_sends
            .iter()
            .any(|text| text.contains("waiting on authentication")),
        "the old parked run must not greet the re-paired user: {post_repair_sends:?}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
