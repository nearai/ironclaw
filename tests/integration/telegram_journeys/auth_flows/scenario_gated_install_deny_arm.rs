//! Telegram auth-flow journey: unconfigured-provider recovery feedback.

use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use ironclaw_turns::TurnStatus;
use serde_json::json;

/// Ben's regression (2026-07-16): a paired user DMed the bot "can you
/// install slack" and the conversation HUNG. Current provider-readiness
/// behavior rejects activation before creating an OAuth gate when the operator
/// has not configured Slack, so the user must receive actionable setup guidance
/// and the turn must finish instead of parking forever. This pins that contract:
///
/// 1. a paired DM asking for a slack install runs the REAL
///    `builtin.extension_install` + `builtin.extension_activate` capabilities;
/// 2. activation's model-visible result names the missing operator settings;
/// 3. the model explains that configuration is required and the run completes;
/// 4. the same Telegram thread accepts a normal follow-up DM.
///
/// Covers (docs/qa/telegram-coverage-map.md): the cross-extension in-DM
/// install/gate rows of qa-telegram Conversations + the deferred-busy
/// feedback rows of Telegram Failure and Recovery.
#[tokio::test]
async fn telegram_dm_unconfigured_slack_returns_setup_guidance_and_frees_thread() {
    let stack = build_journey_stack([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "slack"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "slack"}),
        ),
        RebornScriptedReply::text(
            "Slack is not configured on this IronClaw instance yet. An operator must enable Slack, configure the OAuth redirect URI, restart the service, and then you can ask me again here.",
        ),
        RebornScriptedReply::text("still here"),
    ])
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;
    // The regression trigger: a paired DM asking for a slack install.
    let status = stack
        .webhook_dm(&secret, 2, "can you install slack for me?")
        .await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");

    // The working message posts first ("Ironclaw is thinking...") — the
    // observer's placeholder, riding the wired status path. Its silent
    // failure was half of the "hung" experience.
    stack
        .wait_for_dm_send(|text| text.contains("Ironclaw is thinking"))
        .await
        .expect("the DM turn must post the working message, not silence");

    // Missing instance-level Slack configuration is surfaced to the model at
    // the real activation boundary. Pin the exact actionable ingredients so
    // the scripted final answer cannot hide an empty or generic tool failure.
    let captured_requests = serde_json::to_string(&stack.model_trace.captured_requests())
        .expect("captured model requests serialize");
    for expected in [
        "config set slack.enabled",
        "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI",
        "service restart",
    ] {
        assert!(
            captured_requests.contains(expected),
            "activation result must tell the model how to recover; missing {expected:?}: {captured_requests}"
        );
    }

    let notice = stack
        .wait_for_dm_send(|text| text.contains("not configured on this IronClaw instance"))
        .await
        .expect("the unconfigured Slack install must produce setup guidance, not silence");
    assert_eq!(notice["chat_id"], TG_CHAT_ID);
    assert!(
        notice["text"]
            .as_str()
            .is_some_and(|text| text.contains("ask me again here")),
        "the response tells the user how to recover: {notice}"
    );

    let (turn_scope, run_id) = stack
        .run_for_dm_message("can you install slack for me?")
        .await;
    wait_for_run_status(
        &stack.runtime.webui_turn_coordinator_for_test(),
        &turn_scope,
        run_id,
        TurnStatus::Completed,
    )
    .await
    .expect("unconfigured provider failure completes instead of parking an impossible auth gate");

    // Not wedged: a follow-up DM starts a normal turn on the same thread.
    let before_follow_up = stack.network.request_bodies_for("/sendMessage").len();
    let status = stack.webhook_dm(&secret, 3, "are you still there?").await;
    assert_eq!(status, StatusCode::OK, "follow-up DM webhook acks 200");
    let follow_up = stack
        .wait_for_dm_send_after(before_follow_up, |text| text.contains("still here"))
        .await
        .expect("the same Telegram thread accepts a normal follow-up turn");
    assert!(
        follow_up["text"]
            .as_str()
            .is_some_and(|text| text.contains("still here")),
        "follow-up reply is attributable to the new message: {follow_up}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
