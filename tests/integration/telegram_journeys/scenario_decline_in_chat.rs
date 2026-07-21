use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use serde_json::json;
use std::time::Duration;

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
    // model call), so the two trailing entries serve the post-deny turns.
    // Both carry the same text so the assertion is deterministic regardless
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
    stack
        .wait_for_dm_send(|text| text.contains("accounts.google.com"))
        .await
        .expect("the gated install DMs the authorization link first");

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
