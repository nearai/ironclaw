use std::sync::Mutex;

use ironclaw_host_api::{RestrictedEgressError, RestrictedEgressResponse};
use ironclaw_product_adapters::ProductTriggerReason;

use super::*;

struct RecordingEgress {
    requests: Mutex<Vec<RestrictedEgressRequest>>,
    status: u16,
}

impl RecordingEgress {
    fn ok() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            status: 200,
        }
    }

    fn failing() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            status: 500,
        }
    }
}

#[async_trait]
impl RestrictedEgress for RecordingEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        self.requests.lock().expect("requests lock").push(request);
        Ok(RestrictedEgressResponse {
            status: self.status,
            body: b"{\"ok\":true}".to_vec(),
        })
    }
}

fn context<'a>(config: &'a [(String, String)]) -> ChannelContext<'a> {
    ChannelContext {
        extension_id: "telegram",
        installation_id: "install_alpha",
        config,
    }
}

fn inbound(body: &[u8]) -> Result<InboundOutcome, ChannelError> {
    TelegramChannelAdapter::default().inbound(VerifiedInbound {
        extension_id: "telegram",
        installation_id: "install_alpha",
        body,
        headers: &[],
    })
}

#[tokio::test]
async fn activate_names_the_webhook_secret_as_a_declared_body_credential() {
    let egress = RecordingEgress::ok();
    let config = vec![(
        TELEGRAM_WEBHOOK_URL_CONFIG.to_string(),
        "https://host.example/webhooks/extensions/telegram/updates".to_string(),
    )];
    TelegramChannelAdapter::default()
        .activate(&context(&config), &egress)
        .await
        .expect("activate succeeds");
    let requests = egress.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(request.url.ends_with("/setWebhook"));
    assert!(request.url.starts_with("https://api.telegram.org/"));
    assert_eq!(
        request.credential.as_ref().map(SecretHandle::as_str),
        Some(TELEGRAM_BOT_TOKEN_HANDLE),
        "the bot token is a host-injected handle, never bytes"
    );
    assert_eq!(
        request
            .body_credentials
            .iter()
            .map(SecretHandle::as_str)
            .collect::<Vec<_>>(),
        vec![TELEGRAM_WEBHOOK_SECRET_HANDLE],
        "the webhook secret rides as a declared body-credential handle; \
             the host inserts its VALUE at the manifest's /secret_token pointer"
    );
    let body: serde_json::Value =
        serde_json::from_slice(request.body.as_deref().unwrap_or_default()).expect("json");
    assert_eq!(
        body["url"],
        "https://host.example/webhooks/extensions/telegram/updates"
    );
    assert!(
        body.get("secret_token").is_none(),
        "the adapter must not fabricate the secret field; insertion is host-side"
    );
    assert!(
        body.get("secret_token_handle").is_none(),
        "the handle name must never be sent to the vendor"
    );
}

#[tokio::test]
async fn activate_fails_without_a_webhook_url_and_on_vendor_error() {
    let egress = RecordingEgress::ok();
    let error = TelegramChannelAdapter::default()
        .activate(&context(&[]), &egress)
        .await
        .expect_err("missing webhook url must fail activation");
    assert!(matches!(error, ChannelError::VendorWiring { .. }));

    let failing = RecordingEgress::failing();
    let config = vec![(
        TELEGRAM_WEBHOOK_URL_CONFIG.to_string(),
        "https://host.example/hooks".to_string(),
    )];
    let error = TelegramChannelAdapter::default()
        .activate(&context(&config), &failing)
        .await
        .expect_err("vendor failure must fail activation");
    assert!(matches!(error, ChannelError::VendorWiring { .. }));
}

#[tokio::test]
async fn cleanup_unregisters_the_webhook() {
    let egress = RecordingEgress::ok();
    TelegramChannelAdapter::default()
        .cleanup(&context(&[]), &egress)
        .await
        .expect("cleanup succeeds");
    let requests = egress.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/deleteWebhook"));
}

#[test]
fn private_chat_update_normalizes_to_one_message() {
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

#[test]
fn ambient_group_chatter_and_non_message_updates_are_ignored() {
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
        ),
        Ok(InboundOutcome::Ignore)
    ));
    // Non-message update kinds.
    assert!(matches!(
            inbound(br#"{"update_id": 44, "edited_message": {"message_id": 9, "date": 1, "chat": {"id": 1, "type": "private"}}}"#),
            Ok(InboundOutcome::Ignore)
        ));
}

#[test]
fn malformed_updates_are_typed_parse_errors() {
    assert!(matches!(
        inbound(br#"{"update_id":"#),
        Err(ChannelError::Parse { .. })
    ));
    assert!(matches!(
        inbound(
            br#"{"message": {"message_id": 1, "date": 1, "chat": {"id": 1, "type": "private"}}}"#
        ),
        Err(ChannelError::Parse { .. })
    ));
}

#[test]
fn attachment_only_private_message_is_forwarded_with_an_empty_text_body() {
    let outcome = inbound(
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
    )
    .expect("attachment-only update parses");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("expected Messages");
    };
    assert_eq!(messages.len(), 1);
    assert!(messages[0].text.is_empty());
    assert_eq!(messages[0].attachments.len(), 1);
    assert_eq!(messages[0].attachments[0].vendor_ref, "file-opaque-1");
}
