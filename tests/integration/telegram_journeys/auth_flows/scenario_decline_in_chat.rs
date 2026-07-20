//! Telegram auth-flow journey: explicit in-chat abort.

use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use ironclaw_auth::{AuthFlowOutcome, AuthFlowState, OAuthCallbackState, OAuthCallbackStateKind};
use ironclaw_turns::TurnStatus;
use serde_json::json;
use std::time::Duration;
use url::Url;

/// Ben's third regression (2026-07-17): the busy-on-auth hint tells the user
/// to reply `auth deny gate:<ref>` in this chat, but that reply used to parse
/// as a plain user message and bounce off the busy thread with the same hint
/// — an advertised affordance with no implementation (the phantom loop). This
/// pins the whole decline journey at user seams only: park on auth → busy
/// hint advertises the command → the user sends exactly what the hint said →
/// the gate dies and the thread takes new messages again.
#[tokio::test]
async fn telegram_dm_auth_deny_command_cancels_gate_and_frees_the_thread() {
    // FIFO: install + activate park the run; the deny CANCELS (no resume
    // model call), so the trailing entries serve the post-deny turns.
    // They carry the same text so the assertion is deterministic regardless
    // of which entry the next turn consumes.
    let stack = build_journey_stack_with_google_oauth([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::text("thread is free again"),
        RebornScriptedReply::text("thread is free again"),
        RebornScriptedReply::text("thread is free again"),
    ])
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    let status = stack.webhook_dm(&secret, 2, "can you set up gmail?").await;
    assert_eq!(status, StatusCode::OK);
    let auth_prompt = stack
        .wait_for_dm_send(|text| text.contains("accounts.google.com"))
        .await
        .expect("the gated install DMs the authorization link first");
    let authorization_url = auth_prompt["text"]
        .as_str()
        .and_then(|text| {
            text.split_whitespace()
                .find(|part| part.starts_with("https://accounts.google.com/"))
        })
        .and_then(|part| Url::parse(part).ok())
        .expect("Google authorization URL parses");
    let state = authorization_url
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("Google authorization URL carries opaque state");
    let callback_state = OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, &state)
        .expect("Google callback state decodes");
    let auth_flow_id = callback_state.flow_id();
    let auth_flow_scope = callback_state.scope().clone();

    // Resolve the exact run from the durable thread transcript before acting
    // on the gate. This lets the denial assertion wait for an authoritative
    // terminal state instead of relying on a timing window.
    let (turn_scope, run_id) = stack.run_for_dm_message("can you set up gmail?").await;

    // A message while the run is parked draws the busy hint that advertises
    // the in-chat decline command, gate ref included.
    let status = stack.webhook_dm(&secret, 3, "hello?").await;
    assert_eq!(status, StatusCode::OK);
    let hint = stack
        .wait_for_dm_send(|text| text.contains("auth deny gate:"))
        .await
        .expect("the busy thread advertises the decline command");
    let hint_text = hint["text"].as_str().expect("hint text");
    let command_start = hint_text.find("auth deny gate:").expect("command in hint");
    let command: String = hint_text[command_start..]
        .chars()
        .take_while(|c| !c.is_whitespace() || *c == ' ')
        .collect::<String>()
        .split('`')
        .next()
        .expect("command before closing backtick")
        .trim()
        .to_string();
    assert!(
        command.starts_with("auth deny gate:") && command.len() > "auth deny gate:".len(),
        "extracted a concrete command from the hint: {command:?}"
    );

    // The user does exactly what the hint said, and the decline is
    // acknowledged in the chat.
    let status = stack.webhook_dm(&secret, 4, &command).await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("Authentication canceled"))
        .await
        .expect("the in-chat decline must be acknowledged, not silent");

    wait_for_run_status(
        &stack.runtime.webui_turn_coordinator_for_test(),
        &turn_scope,
        run_id,
        TurnStatus::Cancelled,
    )
    .await
    .expect("explicit denial terminally cancels the exact gated run");
    let product_auth = stack
        .webui
        .product_auth
        .as_ref()
        .expect("journey runtime exposes product auth");
    let mut aborted_flow = None;
    for _ in 0..400 {
        let flow = product_auth
            .flow_manager()
            .get_flow(&auth_flow_scope, auth_flow_id)
            .await
            .expect("read explicitly denied flow")
            .expect("explicitly denied flow remains durable");
        if flow.state == AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
            && flow.resolution_delivered_at.is_some()
        {
            aborted_flow = Some(flow);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        aborted_flow.is_some(),
        "in-chat auth deny durably resolves as UserAborted and delivers the exact resolution"
    );
    // Once the run is terminally cancelled, no late continuation can consume
    // another model step. The install and activate calls are the only calls
    // before the next user turn.
    assert_eq!(
        stack.model_trace.calls(),
        2,
        "explicit auth denial must not resume the canceled model run"
    );

    // The thread frees. Cancellation is asynchronous after the in-chat ack,
    // so follow-ups sent while it settles draw the documented "please
    // resend" bounce. The seam contract asserted here: EVERY attempt gets a
    // visible outcome — the real reply or that explicit bounce, never
    // silence — and the reply arrives within a bounded number of resends.
    let sends_before = stack.network.request_bodies_for("/sendMessage").len();
    let mut reply = None;
    // Bound calibrated to the observed post-cancel settle window (~25-40s:
    // the cancelled run releases the thread after its delivery loop drains);
    // each attempt still asserts a visible outcome, so no iteration is
    // silent.
    for attempt in 0..8 {
        // Each attempt's outcome is read from sends captured AFTER that
        // attempt (the capture log is cumulative; matching stale bounces
        // from earlier attempts would break the every-attempt-visible seam).
        let attempt_baseline = stack.network.request_bodies_for("/sendMessage").len();
        let status = stack
            .webhook_dm(&secret, 5 + attempt, "are you still there?")
            .await;
        assert_eq!(status, StatusCode::OK);
        let mut outcome = None;
        for _ in 0..200 {
            if let Some(send) = stack.network.request_bodies_for("/sendMessage")[attempt_baseline..]
                .iter()
                .find(|body| {
                    body["text"].as_str().is_some_and(|text| {
                        text.contains("thread is free again") || text.contains("please resend")
                    })
                })
                .cloned()
            {
                outcome = Some(send);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let outcome = outcome.expect(
            "every post-decline DM draws a reply or the explicit resend notice — never silence",
        );
        if outcome["text"]
            .as_str()
            .is_some_and(|text| text.contains("thread is free again"))
        {
            reply = Some(outcome);
            break;
        }
    }
    let reply =
        reply.expect("the decline settles within the bounded resends and the thread replies");
    assert_eq!(reply["chat_id"], TG_CHAT_ID);
    let post_deny_sends: Vec<String> = stack.network.request_bodies_for("/sendMessage")
        [sends_before..]
        .iter()
        .filter_map(|body| body["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        !post_deny_sends
            .iter()
            .any(|text| text.contains("waiting on authentication")),
        "no auth-busy-hint loop after the decline: {post_deny_sends:?}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
