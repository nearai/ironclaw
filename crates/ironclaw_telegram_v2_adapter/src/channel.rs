//! The Telegram [`ChannelAdapter`] (generic ingress, extension-runtime P4).
//!
//! `inbound` parses one HOST-VERIFIED Bot API webhook update (the manifest's
//! `shared_secret_header` recipe — Telegram's `X-Telegram-Bot-Api-Secret-Token`
//! — runs in the host's generic verifier; this adapter never sees the
//! secret). `activate`/`cleanup` own the vendor-side webhook wiring
//! (`setWebhook` with the secret token, `deleteWebhook`) through restricted
//! egress: the bot token is a declared credential handle the HOST injects —
//! never token bytes in adapter scope.

use async_trait::async_trait;
use ironclaw_host_api::{NetworkMethod, RestrictedEgress, RestrictedEgressRequest, SecretHandle};
use ironclaw_product_adapters::{
    AdapterInstallationId, AttachmentRef, ChannelAdapter, ChannelContext, ChannelError,
    DeliveryReport, InboundOutcome, NormalizedInboundMessage, OutboundEnvelope, VerifiedInbound,
};

use crate::payload::{
    GroupTriggerPolicy, TELEGRAM_API_HOST, TelegramInboundEvent, normalize_telegram_update,
};

/// Config field handle (non-secret) carrying the public webhook URL the
/// activation hook registers with the vendor.
pub const TELEGRAM_WEBHOOK_URL_CONFIG: &str = "telegram_webhook_url";
/// Secret handle for the webhook shared secret (the same handle the
/// manifest's `shared_secret_header` recipe verifies with).
pub const TELEGRAM_WEBHOOK_SECRET_HANDLE: &str = "telegram_webhook_secret";
/// Secret handle for the bot token the host injects on Bot API egress.
pub const TELEGRAM_BOT_TOKEN_HANDLE: &str = "telegram_bot_token";

/// The Telegram channel adapter. Group-forwarding triggers are non-secret
/// installation config, supplied at construction (bind-time).
#[derive(Debug, Default)]
pub struct TelegramChannelAdapter {
    group_trigger_policy: GroupTriggerPolicy,
}

impl TelegramChannelAdapter {
    pub fn new(group_trigger_policy: GroupTriggerPolicy) -> Self {
        Self {
            group_trigger_policy,
        }
    }
}

#[async_trait]
impl ChannelAdapter for TelegramChannelAdapter {
    /// Register the webhook with the vendor: `setWebhook` carrying the public
    /// webhook URL and the shared secret token. Idempotent (Telegram
    /// overwrites the previous webhook). The bot token is injected host-side
    /// via the declared credential handle.
    async fn activate(
        &self,
        ctx: &ChannelContext<'_>,
        egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        let webhook_url = ctx
            .config
            .iter()
            .find(|(handle, _)| handle == TELEGRAM_WEBHOOK_URL_CONFIG)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| ChannelError::VendorWiring {
                reason: format!("missing {TELEGRAM_WEBHOOK_URL_CONFIG} config value"),
            })?;
        let body = serde_json::json!({
            "url": webhook_url,
            // The vendor echoes this on every webhook delivery; the host's
            // shared_secret_header recipe verifies it. The VALUE is resolved
            // host-side at delivery-verification time; registration sends the
            // handle's stored value via host substitution when real channel
            // egress lands — the adapter itself only names the handle.
            "secret_token_handle": TELEGRAM_WEBHOOK_SECRET_HANDLE,
        });
        let response = egress
            .send(bot_api_request("setWebhook", body))
            .await
            .map_err(|error| ChannelError::VendorWiring {
                reason: format!("setWebhook egress failed: {error}"),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(ChannelError::VendorWiring {
                reason: format!("setWebhook returned status {}", response.status),
            });
        }
        Ok(())
    }

    /// Unregister the webhook (`deleteWebhook`). Idempotent and best-effort:
    /// the host records failures as `RemovalPending` and retries.
    async fn cleanup(
        &self,
        _ctx: &ChannelContext<'_>,
        egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        let response = egress
            .send(bot_api_request("deleteWebhook", serde_json::json!({})))
            .await
            .map_err(|error| ChannelError::VendorWiring {
                reason: format!("deleteWebhook egress failed: {error}"),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(ChannelError::VendorWiring {
                reason: format!("deleteWebhook returned status {}", response.status),
            });
        }
        Ok(())
    }

    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        let installation_id =
            AdapterInstallationId::new(request.installation_id).map_err(|error| {
                ChannelError::Parse {
                    reason: format!("invalid installation id: {error}"),
                }
            })?;
        match normalize_telegram_update(request.body, &installation_id, &self.group_trigger_policy)
            .map_err(|error| ChannelError::Parse {
                reason: error.to_string(),
            })? {
            TelegramInboundEvent::Ignore => Ok(InboundOutcome::Ignore),
            TelegramInboundEvent::Message(message) => {
                let attachments = message
                    .attachments
                    .into_iter()
                    .map(|descriptor| AttachmentRef {
                        vendor_ref: descriptor.external_file_id.clone(),
                        mime_hint: Some(descriptor.mime_type.clone()),
                        descriptor,
                    })
                    .collect();
                Ok(InboundOutcome::Messages(vec![NormalizedInboundMessage {
                    actor: message.actor,
                    conversation: message.conversation,
                    event_id: message.event_id,
                    text: message.text,
                    trigger: message.trigger,
                    attachments,
                    reply_context: None,
                }]))
            }
        }
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        // Outbound cutover is extension-runtime P5 (delivery coordinator).
        Err(ChannelError::Unsupported)
    }
}

/// A Bot API request against the declared vendor host, naming the bot-token
/// credential handle for host-side injection. Token bytes never enter
/// adapter scope.
fn bot_api_request(method: &str, body: serde_json::Value) -> RestrictedEgressRequest {
    RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: format!("https://{TELEGRAM_API_HOST}/bot/{method}"),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: Some(body.to_string().into_bytes()),
        credential: SecretHandle::new(TELEGRAM_BOT_TOKEN_HANDLE).ok(),
    }
}

#[cfg(test)]
mod tests {
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
    async fn activate_registers_the_webhook_with_the_secret_token_handle() {
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
        let body: serde_json::Value =
            serde_json::from_slice(request.body.as_deref().unwrap_or_default()).expect("json");
        assert_eq!(
            body["url"],
            "https://host.example/webhooks/extensions/telegram/updates"
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
            inbound(br#"{"message": {"message_id": 1, "date": 1, "chat": {"id": 1, "type": "private"}}}"#),
            Err(ChannelError::Parse { .. })
        ));
    }
}
