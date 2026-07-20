use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use ironclaw_host_api::NetworkMethod;

/// A Telegram document follows the production webhook -> descriptor ->
/// mediated provider download -> canonical workspace lander -> turn -> native
/// reply path. The successful model reply is the caller-level proof that the
/// attachment-bearing turn was not silently downgraded to text-only.
#[tokio::test]
async fn telegram_document_downloads_into_workspace_before_the_turn_runs() {
    let stack =
        build_journey_stack([RebornScriptedReply::text("I received the attached report")]).await;
    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    assert_eq!(
        stack.webhook_update(&secret, dm_document_update(2)).await,
        StatusCode::OK
    );
    stack
        .wait_for_dm_send(|text| text.contains("received the attached report"))
        .await
        .expect("attachment-bearing turn produces its reply");

    let requests = stack.network.requests();
    assert!(requests.iter().any(|request| {
        request.url.contains("/getFile?") && request.url.contains("telegram-journey-file")
    }));
    assert!(requests.iter().any(|request| {
        request
            .url
            .ends_with("/file/bot777000111:journey-bot-token/documents/journey-notes.txt")
            && request.method == NetworkMethod::Get
    }));

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
