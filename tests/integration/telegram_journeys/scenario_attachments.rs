use super::harness::*;
use super::reborn_support::reply::RebornScriptedReply;
use axum::http::StatusCode;
use ironclaw_host_api::NetworkMethod;
use ironclaw_product_workflow::RebornGetRunStateRequest;
use ironclaw_threads::{MessageKind, MessageStatus};
use ironclaw_turns::TurnStatus;
use serde_json::json;

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

    let (_timeline, accepted_message) = stack
        .wait_for_timeline_message("Please review the attached report")
        .await
        .expect("attachment-bearing message is durable");
    assert_eq!(accepted_message.kind, MessageKind::User);
    assert_eq!(accepted_message.status, MessageStatus::Submitted);
    assert_eq!(accepted_message.attachments.len(), 1);
    let attachment = &accepted_message.attachments[0];
    let storage_key = attachment
        .storage_key
        .as_deref()
        .expect("accepted attachment carries its durable workspace ref");
    assert!(storage_key.starts_with("/workspace/attachments/"));
    assert_eq!(attachment.size_bytes, Some(24));
    assert_eq!(
        stack
            .read_project_file_bytes(&accepted_message.thread_id, storage_key)
            .await
            .expect("landed Telegram bytes read through product filesystem"),
        b"journey attachment bytes"
    );

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

/// A transient Bot API download failure must be visible before the webhook is
/// acknowledged so Telegram redelivers the same update. The second delivery
/// then reuses the production idempotency/release path and lands the file once.
#[tokio::test]
async fn telegram_transient_attachment_download_retries_before_webhook_ack() {
    let stack =
        build_journey_stack([RebornScriptedReply::text("I received the retried report")]).await;
    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    stack.network.set_get_file_failure(Some(503));
    assert_eq!(
        stack.webhook_update(&secret, dm_document_update(2)).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        !stack
            .network
            .requests()
            .iter()
            .any(|request| request.url.contains("/file/bot")),
        "a failed metadata lookup must not attempt the byte download"
    );

    stack.network.set_get_file_failure(None);
    assert_eq!(
        stack.webhook_update(&secret, dm_document_update(2)).await,
        StatusCode::OK
    );
    stack
        .wait_for_dm_send(|text| text.contains("received the retried report"))
        .await
        .expect("provider redelivery produces the attachment-bearing reply");

    let (timeline, accepted_message) = stack
        .wait_for_timeline_message("Please review the attached report")
        .await
        .expect("retried attachment-bearing message is durable");
    let matching_messages = timeline
        .messages
        .iter()
        .filter(|message| message.content.as_deref() == Some("Please review the attached report"))
        .collect::<Vec<_>>();
    assert_eq!(matching_messages.len(), 1, "one accepted provider message");
    assert_eq!(accepted_message.attachments.len(), 1);
    let storage_key = accepted_message.attachments[0]
        .storage_key
        .as_deref()
        .expect("retried attachment carries its durable workspace ref");
    assert!(storage_key.starts_with("/workspace/attachments/"));
    assert_eq!(
        stack
            .read_project_file_bytes(&accepted_message.thread_id, storage_key)
            .await
            .expect("retried Telegram bytes read through product filesystem"),
        b"journey attachment bytes"
    );
    let run_id = accepted_message
        .turn_run_id
        .as_deref()
        .expect("accepted provider message carries one run id");
    let run = stack
        .webui
        .api
        .get_run_state(
            stack.caller.clone(),
            RebornGetRunStateRequest {
                thread_id: accepted_message.thread_id.as_str().to_string(),
                run_id: run_id.to_string(),
            },
        )
        .await
        .expect("read retried Telegram run");
    assert_eq!(run.status, TurnStatus::Completed);
    assert_eq!(
        timeline
            .messages
            .iter()
            .filter(|message| {
                message.kind == MessageKind::Assistant
                    && message.status == MessageStatus::Finalized
                    && message.content.as_deref() == Some("I received the retried report")
            })
            .count(),
        1,
        "one finalized assistant reply"
    );

    let requests = stack.network.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.contains("/getFile?"))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.contains("/file/bot"))
            .count(),
        1
    );
    assert_eq!(
        stack
            .network
            .delivered_sends_containing("I received the retried report"),
        1,
        "provider redelivery must produce one delivered final reply"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}

/// A channel-originated agent turn writes a real workspace file through the
/// first-party capability, then the final-reply delivery observer resolves the
/// canonical workspace path and renders one native Telegram document upload.
#[tokio::test]
async fn telegram_workspace_file_reply_sends_native_document() {
    let stack = build_journey_stack([
        RebornScriptedReply::text("ready to create the report"),
        RebornScriptedReply::tool_call(
            "builtin.write_file",
            json!({"path": "/workspace/report.txt", "content": "telegram report bytes"}),
        ),
        RebornScriptedReply::text("Here is /workspace/report.txt"),
    ])
    .await;
    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    assert_eq!(
        stack.webhook_dm(&secret, 2, "prepare a report").await,
        StatusCode::OK
    );
    stack
        .wait_for_dm_send(|text| text == "ready to create the report")
        .await
        .expect("first channel turn creates the conversation");
    let (_timeline, first_message) = stack
        .wait_for_timeline_message("prepare a report")
        .await
        .expect("first channel turn is durable");
    stack
        .enable_auto_approve_for_thread(first_message.thread_id.clone())
        .await;

    assert_eq!(
        stack.webhook_dm(&secret, 3, "write and send it").await,
        StatusCode::OK
    );
    stack
        .wait_for_dm_send(|text| text == "Here is /workspace/report.txt")
        .await
        .expect("workspace-bearing final reply reaches Telegram");

    let document_requests = stack
        .network
        .requests()
        .into_iter()
        .filter(|request| request.url.ends_with("/sendDocument"))
        .collect::<Vec<_>>();
    assert_eq!(document_requests.len(), 1, "one native document upload");
    assert_eq!(
        stack.network.send_document_outcomes(),
        vec![200],
        "the one native document upload is provider-accepted"
    );
    let document = &document_requests[0];
    assert_eq!(document.method, NetworkMethod::Post);
    let multipart = String::from_utf8_lossy(&document.body);
    assert!(multipart.contains("name=\"chat_id\"\r\n\r\n555\r\n"));
    assert!(multipart.contains("filename=\"report.txt\""));
    assert!(multipart.contains("telegram report bytes"));
    assert_eq!(
        stack
            .network
            .delivered_sends_containing("Here is /workspace/report.txt"),
        1,
        "one native text part accompanies the document"
    );

    let (_timeline, final_reply) = stack
        .wait_for_timeline_message("Here is /workspace/report.txt")
        .await
        .expect("workspace-bearing final reply is durable");
    assert_eq!(final_reply.kind, MessageKind::Assistant);
    assert_eq!(final_reply.status, MessageStatus::Finalized);
    assert_eq!(
        stack
            .read_project_file_bytes(&final_reply.thread_id, "/workspace/report.txt")
            .await
            .expect("capability-created workspace file is durable"),
        b"telegram report bytes"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
