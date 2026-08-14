use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgressError, RestrictedEgressResponse};

use super::*;

struct InertEgress;

#[async_trait]
impl RestrictedEgress for InertEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

struct FixedAttachmentEgress;

#[async_trait]
impl RestrictedEgress for FixedAttachmentEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        if request.url.ends_with("/getFile") {
            return Ok(RestrictedEgressResponse {
                status: 200,
                body:
                    br#"{"ok":true,"result":{"file_size":12,"file_path":"documents/fetched.bin"}}"#
                        .to_vec(),
            });
        }
        Ok(RestrictedEgressResponse {
            status: 200,
            body: b"hello world!".to_vec(),
        })
    }
}

fn bot_username_config() -> (String, String) {
    (
        TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
        "ironclaw_test_bot".to_string(),
    )
}

async fn inbound(body: &[u8]) -> Result<InboundOutcome, ChannelError> {
    inbound_with_egress(body, &InertEgress).await
}

async fn inbound_with_egress(
    body: &[u8],
    egress: &dyn RestrictedEgress,
) -> Result<InboundOutcome, ChannelError> {
    let config = [bot_username_config()];
    TelegramChannelAdapter::default()
        .receive(
            VerifiedInbound {
                extension_id: "telegram",
                installation_id: "install_alpha",
                config: &config,
                body,
                headers: &[],
                can_reply_in_threads: false,
            },
            egress,
        )
        .await
}

// The three `activate`/`cleanup` tests that stood here moved with the
// behavior: vendor-side webhook wiring is now the manifest's
// `[channel.ingress.registration]` / `[channel.ingress.deregistration]`
// recipes, run by the generic host executor. Every assertion they made — the
// bot token travels as a handle and never as bytes, the webhook secret rides
// `body_credentials` so the host inserts its VALUE at the declared pointer,
// the rendered body carries `url` but never `secret_token`, a missing config
// value and a vendor 5xx both fail activation, and deactivation calls
// `deleteWebhook` — is re-pinned in
// `ironclaw_extension_host::lifecycle::tests`, against the executor that now
// owns them.

#[tokio::test]
async fn private_chat_update_normalizes_to_one_message() {
    let outcome = inbound(
        br#"{
                "update_id": 42,
                "message": {
                    "message_id": 7,
                    "date": 1710000000,
                    "text": "hello bot",
                    "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                    "chat": {"id": 555, "type": "private"}
                }
            }"#,
    )
    .await
    .expect("update parses");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("expected Messages");
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "hello bot");
    assert_eq!(messages[0].trigger, ProductTriggerReason::DirectChat);
    assert_eq!(
        messages[0].event_id.as_str(),
        "tg-install_alpha-42",
        "event identity keys the durable dedupe"
    );
    assert_eq!(messages[0].conversation.conversation_id(), "555");
}

#[tokio::test]
async fn ambient_group_chatter_and_non_message_updates_are_ignored() {
    // Group message without any explicit trigger.
    assert!(matches!(
        inbound(
            br#"{
                    "update_id": 43,
                    "message": {
                        "message_id": 8,
                        "date": 1710000000,
                        "text": "ambient chatter",
                        "from": {"id": 1002, "is_bot": false, "first_name": "Bob"},
                        "chat": {"id": -100200, "type": "group"}
                    }
                }"#,
        )
        .await,
        Ok(InboundOutcome::Ignore)
    ));
    // Non-message update kinds.
    assert!(matches!(
            inbound(br#"{"update_id": 44, "edited_message": {"message_id": 9, "date": 1, "chat": {"id": 1, "type": "private"}}}"#).await,
            Ok(InboundOutcome::Ignore)
        ));
}

#[tokio::test]
async fn malformed_updates_are_typed_parse_errors() {
    assert!(matches!(
        inbound(br#"{"update_id":"#).await,
        Err(ChannelError::Parse { .. })
    ));
    assert!(matches!(
        inbound(
            br#"{"message": {"message_id": 1, "date": 1, "chat": {"id": 1, "type": "private"}}}"#
        )
        .await,
        Err(ChannelError::Parse { .. })
    ));
}

#[tokio::test]
async fn attachment_only_private_message_is_forwarded_with_an_empty_text_body() {
    let outcome = inbound_with_egress(
        br#"{
                "update_id": 45,
                "message": {
                    "message_id": 10,
                    "date": 1710000000,
                    "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                    "chat": {"id": 555, "type": "private"},
                    "document": {
                        "file_id": "file-opaque-1",
                        "file_name": "report.pdf",
                        "mime_type": "application/pdf",
                        "file_size": 12
                    }
                }
            }"#,
        &FixedAttachmentEgress,
    )
    .await
    .expect("attachment-only update parses");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("expected Messages");
    };
    assert_eq!(messages.len(), 1);
    assert!(messages[0].text.is_empty());
    assert_eq!(messages[0].attachments.len(), 1);
    assert_eq!(messages[0].attachments[0].id, "file-opaque-1");
    assert!(messages[0].conversation_context.is_none());
}

#[tokio::test]
async fn private_media_group_update_becomes_a_triggered_batch_fragment() {
    let outcome = inbound_with_egress(
        br#"{
            "update_id": 46,
            "message": {
                "message_id": 11,
                "media_group_id": "album-private",
                "date": 1710000000,
                "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 555, "type": "private"},
                "document": {
                    "file_id": "file-opaque-2",
                    "file_name": "notes.txt",
                    "mime_type": "text/plain",
                    "file_size": 12
                }
            }
        }"#,
        &FixedAttachmentEgress,
    )
    .await
    .expect("media-group update parses");
    let InboundOutcome::BatchFragment(fragment) = outcome else {
        panic!("expected BatchFragment");
    };
    assert!(fragment.triggered);
    assert_eq!(
        fragment.batch_key,
        "chat-555-thread-none-group-album-private"
    );
    assert_eq!(
        fragment.message.event_id.as_str(),
        "tg-install_alpha-media-chat-555-thread-none-group-album-private"
    );
    assert_eq!(fragment.message.attachments.len(), 1);
}

#[tokio::test]
async fn uncaptioned_group_media_fragment_is_retained_but_not_triggered() {
    let outcome = inbound_with_egress(
        br#"{
            "update_id": 47,
            "message": {
                "message_id": 12,
                "media_group_id": "album-group",
                "date": 1710000000,
                "from": {"id": 1002, "is_bot": false, "first_name": "Bob"},
                "chat": {"id": -100200, "type": "group"},
                "document": {
                    "file_id": "file-opaque-3",
                    "file_name": "ambient.txt",
                    "mime_type": "text/plain",
                    "file_size": 12
                }
            }
        }"#,
        &FixedAttachmentEgress,
    )
    .await
    .expect("media-group update parses");
    let InboundOutcome::BatchFragment(fragment) = outcome else {
        panic!("expected BatchFragment");
    };
    assert!(!fragment.triggered);
    assert_eq!(fragment.message.attachments.len(), 1);
}
