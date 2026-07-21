use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use serde_json::json;
use std::time::Duration;

/// Ben's regression (2026-07-16): a paired user DMed the bot "can you
/// install slack" and the conversation HUNG — the run parked on slack's
/// OAuth setup gate, and with `TelegramDeliveryProtocol::post_status_message`
/// unwired in v1 every host-authored notice (working message, blocked-run
/// action-needed prompt, busy hints) silently failed, so the DM saw nothing
/// and follow-up messages produced nothing. This pins the fixed contract:
///
/// 1. a paired DM asking for a slack install runs the REAL
///    `builtin.extension_install` + `builtin.extension_activate` capabilities;
///    slack's activation gates on its personal-OAuth credential requirement;
/// 2. the DM RECEIVES host-authored feedback about the gate (via the wired
///    `sendMessage` status path — never silence);
/// 3. the channel is not wedged: a follow-up DM still gets host feedback.
///
/// Covers (docs/qa/telegram-coverage-map.md): the cross-extension in-DM
/// install/gate rows of qa-telegram Conversations + the deferred-busy
/// feedback rows of Telegram Failure and Recovery.
#[tokio::test]
async fn telegram_dm_slack_install_gates_with_action_needed_notice_not_silence() {
    // FIFO: the DM turn calls install then activate; activate parks on the
    // slack OAuth gate, so the post-resume entry and the follow-up-turn entry
    // may stay unconsumed depending on how the gate resolves — both are
    // scripted so neither outcome can starve the trace.
    let stack = build_journey_stack_customized(
        [
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("slack is connected"),
            RebornScriptedReply::text("still here"),
        ],
        |input| {
            // Reach the per-user credential gate while leaving the OAuth
            // challenge provider unwired so the channel takes the deny arm.
            input
                .with_slack_host_beta_enabled(true)
                .with_slack_personal_oauth_redirect_uri_configured(true)
        },
    )
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;
    let sends_after_pairing = stack.network.request_bodies_for("/sendMessage").len();

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

    // Slack's activation parks on its personal-OAuth credential requirement.
    // This stack has no slack OAuth client config, so the challenge is
    // credential-entry shaped and the observer takes the deny arm: it cancels
    // the blocked run and posts the host-authored "set this up in the web
    // app" notice — via the SAME wired status path that used to fail
    // silently. The OAuth-configured link-prompt arm is pinned separately by
    // `telegram_dm_gated_install_posts_oauth_authorization_link_not_silence`.
    let notice = stack
        .wait_for_dm_send(|text| text.contains("credential-based connections can only be set up"))
        .await
        .expect("the gated slack install must produce the action-needed DM notice, not silence");
    assert_eq!(notice["chat_id"], TG_CHAT_ID);
    assert!(
        notice["text"]
            .as_str()
            .is_some_and(|text| text.contains("ask me again here")),
        "the notice tells the user how to recover: {notice}"
    );

    // Not wedged: a follow-up DM still produces host feedback (a busy hint
    // for the parked run, or a fresh reply if the gate auto-resolved).
    let before_follow_up = stack.network.request_bodies_for("/sendMessage").len();
    assert!(
        before_follow_up > sends_after_pairing,
        "the gate notice itself must be a new send"
    );
    let status = stack.webhook_dm(&secret, 3, "are you still there?").await;
    assert_eq!(status, StatusCode::OK, "follow-up DM webhook acks 200");
    for _ in 0..200 {
        if stack.network.request_bodies_for("/sendMessage").len() > before_follow_up {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let follow_up_sends: Vec<String> = stack.network.request_bodies_for("/sendMessage")
        [before_follow_up..]
        .iter()
        .filter_map(|body| body["text"].as_str().map(str::to_string))
        .collect();
    // Two healthy shapes: the deny arm cancelled the parked run, so the
    // follow-up either starts a fresh turn (consuming the next scripted
    // entry) or — if the cancel is still settling — draws a busy hint.
    assert!(
        follow_up_sends.iter().any(|text| {
            text.contains("still working on a previous message")
                || text.contains("waiting on authentication")
                || text.contains("slack is connected")
                || text.contains("still here")
        }),
        "the follow-up DM must draw recognizable host feedback (busy hint or \
         a fresh scripted reply) attributable to it, got: {follow_up_sends:?}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
