use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use serde_json::json;

/// Ben's second regression (2026-07-17): the OAuth-CONFIGURED sibling of the
/// scenario above. With a provider client configured, a gated install parks
/// `BlockedAuth` with a link-shaped challenge and the delivery driver takes
/// the link-prompt arm — building an `AuthPrompt` whose authorization URL the
/// adapter must deliver to the DM. The Telegram adapter used to record that
/// prompt `Deferred` and render nothing, so the DM watched "thinking…" get
/// deleted and then SILENCE while the driver waited for an authorization
/// nobody was ever offered. Pins: the auth prompt reaches the chat as a
/// `sendMessage` carrying the authorization URL.
#[tokio::test]
async fn telegram_dm_gated_install_posts_oauth_authorization_link_not_silence() {
    // FIFO: install + activate; activate parks on the google OAuth gate, so
    // the post-resume entry and the follow-up entry may stay unconsumed
    // (nobody completes OAuth inside this test).
    let stack = build_journey_stack_with_google_oauth([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::text("gmail is connected"),
        RebornScriptedReply::text("still here"),
    ])
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    let status = stack.webhook_dm(&secret, 2, "can you set up gmail?").await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");

    stack
        .wait_for_dm_send(|text| text.contains("Ironclaw is thinking"))
        .await
        .expect("the DM turn must post the working message");

    // The actionable prompt: headline/body plus the Google authorization URL
    // rendered by the adapter — the tap-to-authorize path.
    let prompt = stack
        .wait_for_dm_send(|text| text.contains("accounts.google.com"))
        .await
        .expect("the gated install must DM the authorization link, not silence");
    assert_eq!(prompt["chat_id"], TG_CHAT_ID);
    let text = prompt["text"].as_str().expect("prompt text");
    assert!(
        text.contains("Open this link to authorize"),
        "prompt explains the link is the way to continue: {text}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
