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
use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ChannelAdapter, ChannelContext, ChannelError, DeliveryReport,
    InboundOutcome, OutboundEnvelope, OutboundPart, PartDeliveryOutcome, VerifiedInbound,
    render_channel_auth_prompt,
};
use ironclaw_host_api::{NetworkMethod, RestrictedEgress, RestrictedEgressRequest, SecretHandle};

use ironclaw_telegram_v2_adapter::{
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

/// Path placeholder the manifest's `[[channel.egress]] injection` declares;
/// the host substitutes the token host-side (`/bot{telegram_bot_token}/…`).
pub const TELEGRAM_TOKEN_PLACEHOLDER: &str = "telegram_bot_token";

/// Telegram sendMessage hard limit (characters).
const TELEGRAM_TEXT_LIMIT_CHARS: usize = 4096;

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
        });
        // Telegram's contract wants `secret_token` — the VALUE it will echo
        // back on every webhook delivery, which the host's
        // shared_secret_header recipe then verifies. The adapter only names
        // the handle; the manifest's `[[channel.egress]] body_credentials`
        // binding tells restricted egress to resolve it and insert the value
        // at `/secret_token` host-side. Secret bytes never enter adapter
        // scope.
        let mut request = bot_api_request("setWebhook", body);
        request.body_credentials = vec![
            SecretHandle::new(TELEGRAM_WEBHOOK_SECRET_HANDLE).map_err(|error| {
                ChannelError::VendorWiring {
                    reason: format!("invalid webhook secret handle: {error}"),
                }
            })?,
        ];
        let response = egress
            .send(request)
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
            TelegramInboundEvent::Message(message) => Ok(InboundOutcome::Messages(vec![*message])),
        }
    }

    /// Render one coordinator envelope as Bot API `sendMessage` calls: plain
    /// text split at the vendor's 4096-char limit, `chat_id` from the
    /// conversation ref, forum-topic threading when the anchor is numeric.
    /// The bot token rides the declared path placeholder — injected
    /// host-side, never adapter-visible.
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        if envelope.parts.is_empty() {
            return Err(ChannelError::Render {
                reason: "outbound envelope carries no parts".to_string(),
            });
        }
        let chat_id = envelope.target.conversation.conversation_id().to_string();
        let message_thread_id = envelope
            .target
            .thread_anchor
            .as_deref()
            .or_else(|| envelope.target.conversation.topic_id())
            .and_then(|topic| topic.parse::<i64>().ok());

        let mut parts = Vec::new();
        'parts: for part in &envelope.parts {
            match part {
                OutboundPart::Text(text) => {
                    for chunk in telegram_text_chunks(text) {
                        let mut body = serde_json::json!({ "chat_id": chat_id, "text": chunk });
                        if let Some(thread_id) = message_thread_id {
                            body["message_thread_id"] = thread_id.into();
                        }
                        let outcome = send_telegram_message(egress, body).await;
                        let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                        parts.push(outcome);
                        if !sent {
                            // The report describes what the vendor accepted;
                            // the coordinator owns retry semantics.
                            break 'parts;
                        }
                    }
                }
                OutboundPart::AuthPrompt {
                    view,
                    direct_message,
                } => {
                    let text = render_channel_auth_prompt(view, *direct_message);
                    for chunk in telegram_text_chunks(&text) {
                        let mut body = serde_json::json!({ "chat_id": chat_id, "text": chunk });
                        if let Some(thread_id) = message_thread_id {
                            body["message_thread_id"] = thread_id.into();
                        }
                        let outcome = send_telegram_message(egress, body).await;
                        let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                        parts.push(outcome);
                        if !sent {
                            break 'parts;
                        }
                    }
                }
                OutboundPart::Retract { vendor_message_ref } => {
                    let outcome = match vendor_message_ref.parse::<i64>() {
                        Ok(message_id) => {
                            delete_telegram_message(
                                egress,
                                serde_json::json!({
                                    "chat_id": chat_id,
                                    "message_id": message_id,
                                }),
                            )
                            .await
                        }
                        Err(_) => PartDeliveryOutcome::Permanent {
                            reason: format!(
                                "retract target `{vendor_message_ref}` is not a telegram message id"
                            ),
                        },
                    };
                    let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                    parts.push(outcome);
                    if !sent {
                        break 'parts;
                    }
                }
            }
        }
        Ok(DeliveryReport { parts })
    }
}

#[derive(Debug, serde::Deserialize)]
struct TelegramSendMessageResponse {
    ok: bool,
    error_code: Option<u16>,
    description: Option<String>,
    result: Option<TelegramSentMessage>,
}

#[derive(Debug, serde::Deserialize)]
struct TelegramSentMessage {
    message_id: i64,
}

async fn send_telegram_message(
    egress: &dyn RestrictedEgress,
    body: serde_json::Value,
) -> PartDeliveryOutcome {
    let response = match egress.send(bot_api_request("sendMessage", body)).await {
        Ok(response) => response,
        Err(error) => return telegram_outcome_for_egress_error(&error),
    };
    if !(200..300).contains(&response.status) {
        return telegram_outcome_for_status(
            response.status,
            format!("telegram bot api returned status {}", response.status),
        );
    }
    let parsed: TelegramSendMessageResponse = match serde_json::from_slice(&response.body) {
        Ok(parsed) => parsed,
        // A truncated body from a proxy/LB timeout is transient infra.
        Err(error) => {
            return PartDeliveryOutcome::Retryable {
                reason: format!("sendMessage response was not valid JSON: {error}"),
            };
        }
    };
    if parsed.ok {
        let Some(message) = parsed.result else {
            return PartDeliveryOutcome::Retryable {
                reason: "sendMessage response omitted result.message_id evidence".to_string(),
            };
        };
        return PartDeliveryOutcome::Sent {
            vendor_message_ref: Some(message.message_id.to_string()),
        };
    }
    let description = parsed
        .description
        .unwrap_or_else(|| "unknown_error".to_string());
    telegram_outcome_for_status(
        parsed.error_code.unwrap_or(400),
        format!("telegram rejected sendMessage ({description})"),
    )
}

/// `deleteMessage` responds with `result: true` (a boolean, not a message
/// object), so it gets its own response shape.
#[derive(Debug, serde::Deserialize)]
struct TelegramDeleteMessageResponse {
    ok: bool,
    error_code: Option<u16>,
    description: Option<String>,
    result: Option<bool>,
}

/// Retract an earlier post (`deleteMessage`). The `vendor_message_ref` is
/// the message id a previous `Sent` outcome returned.
async fn delete_telegram_message(
    egress: &dyn RestrictedEgress,
    body: serde_json::Value,
) -> PartDeliveryOutcome {
    let response = match egress.send(bot_api_request("deleteMessage", body)).await {
        Ok(response) => response,
        Err(error) => return telegram_outcome_for_egress_error(&error),
    };
    if !(200..300).contains(&response.status) {
        return telegram_outcome_for_status(
            response.status,
            format!("telegram bot api returned status {}", response.status),
        );
    }
    let parsed: TelegramDeleteMessageResponse = match serde_json::from_slice(&response.body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return PartDeliveryOutcome::Retryable {
                reason: format!("deleteMessage response was not valid JSON: {error}"),
            };
        }
    };
    if parsed.ok {
        return match parsed.result {
            Some(true) => PartDeliveryOutcome::Sent {
                vendor_message_ref: None,
            },
            Some(false) => PartDeliveryOutcome::Permanent {
                reason: "deleteMessage response reported result:false".to_string(),
            },
            None => PartDeliveryOutcome::Retryable {
                reason: "deleteMessage response omitted result evidence".to_string(),
            },
        };
    }
    let description = parsed
        .description
        .unwrap_or_else(|| "unknown_error".to_string());
    telegram_outcome_for_status(
        parsed.error_code.unwrap_or(400),
        format!("telegram rejected deleteMessage ({description})"),
    )
}

fn telegram_outcome_for_status(status: u16, reason: String) -> PartDeliveryOutcome {
    if status >= 500 || status == 429 || status == 408 {
        PartDeliveryOutcome::Retryable { reason }
    } else if status == 401 || status == 403 {
        PartDeliveryOutcome::Unauthorized { reason }
    } else {
        PartDeliveryOutcome::Permanent { reason }
    }
}

fn telegram_outcome_for_egress_error(
    error: &ironclaw_host_api::RestrictedEgressError,
) -> PartDeliveryOutcome {
    use ironclaw_host_api::RestrictedEgressError as EgressError;
    match error {
        EgressError::Transport { .. } => PartDeliveryOutcome::Retryable {
            reason: error.to_string(),
        },
        EgressError::AuthRequired { .. } | EgressError::UndeclaredCredential { .. } => {
            PartDeliveryOutcome::Unauthorized {
                reason: error.to_string(),
            }
        }
        EgressError::UndeclaredHost { .. }
        | EgressError::UndeclaredMethod
        | EgressError::HostOwnedHeader { .. }
        | EgressError::PolicyDenied
        | EgressError::ResponseTooLarge => PartDeliveryOutcome::Permanent {
            reason: error.to_string(),
        },
    }
}

/// Split text at the vendor's 4096-char message limit, preferring newline
/// boundaries within each window.
fn telegram_text_chunks(text: &str) -> Vec<String> {
    if text.chars().count() <= TELEGRAM_TEXT_LIMIT_CHARS {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    for segment in text.split_inclusive('\n') {
        let segment_chars = segment.chars().count();
        if current_chars + segment_chars > TELEGRAM_TEXT_LIMIT_CHARS && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        if segment_chars > TELEGRAM_TEXT_LIMIT_CHARS {
            for ch in segment.chars() {
                if current_chars == TELEGRAM_TEXT_LIMIT_CHARS {
                    chunks.push(std::mem::take(&mut current));
                    current_chars = 0;
                }
                current.push(ch);
                current_chars += 1;
            }
        } else {
            current.push_str(segment);
            current_chars += segment_chars;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// A Bot API request against the declared vendor host, naming the bot-token
/// credential handle for host-side injection. Token bytes never enter
/// adapter scope.
fn bot_api_request(method: &str, body: serde_json::Value) -> RestrictedEgressRequest {
    RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: format!("https://{TELEGRAM_API_HOST}/bot{{{TELEGRAM_TOKEN_PLACEHOLDER}}}/{method}"),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: Some(body.to_string().into_bytes()),
        credential: SecretHandle::new(TELEGRAM_BOT_TOKEN_HANDLE).ok(),
        body_credentials: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::product_adapter::ProductTriggerReason;
    use ironclaw_host_api::{RestrictedEgressError, RestrictedEgressResponse};

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
            inbound(br#"{"message": {"message_id": 1, "date": 1, "chat": {"id": 1, "type": "private"}}}"#),
            Err(ChannelError::Parse { .. })
        ));
    }
}

#[cfg(test)]
mod deliver_tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use ironclaw_host_api::product_adapter::{
        ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget,
        PartDeliveryOutcome,
    };
    use ironclaw_host_api::{RestrictedEgressError, RestrictedEgressResponse};

    use super::*;

    struct ScriptedEgress {
        requests: Mutex<Vec<RestrictedEgressRequest>>,
        responses: Mutex<VecDeque<Result<RestrictedEgressResponse, RestrictedEgressError>>>,
    }

    impl ScriptedEgress {
        fn new(responses: Vec<Result<RestrictedEgressResponse, RestrictedEgressError>>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }

        fn ok(body: &str) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            Ok(RestrictedEgressResponse {
                status: 200,
                body: body.as_bytes().to_vec(),
            })
        }
    }

    #[async_trait]
    impl RestrictedEgress for ScriptedEgress {
        async fn send(
            &self,
            request: RestrictedEgressRequest,
        ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(RestrictedEgressError::PolicyDenied))
        }
    }

    fn envelope(parts: Vec<OutboundPart>, topic: Option<&str>) -> OutboundEnvelope {
        OutboundEnvelope {
            extension_id: "telegram".to_string(),
            installation_id: "install_alpha".to_string(),
            delivery_attempt_id: "attempt-1".to_string(),
            target: OutboundTarget {
                conversation: ExternalConversationRef::new(None, "8675309", topic, None)
                    .expect("conversation"),
                thread_anchor: None,
            },
            parts,
            reply_context: None,
        }
    }

    #[tokio::test]
    async fn deliver_retract_part_deletes_the_referenced_message() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(r#"{"ok":true,"result":true}"#)]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(
                    vec![OutboundPart::Retract {
                        vendor_message_ref: "42".to_string(),
                    }],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert_eq!(report.parts.len(), 1);
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Sent {
                vendor_message_ref: None
            }
        ));
        let requests = egress.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://api.telegram.org/bot{telegram_bot_token}/deleteMessage"
        );
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_deref().unwrap_or_default()).unwrap();
        assert_eq!(body["chat_id"], "8675309");
        assert_eq!(body["message_id"], 42);
    }

    #[tokio::test]
    async fn deliver_retract_requires_true_result_evidence() {
        for body in [r#"{"ok":true,"result":false}"#, r#"{"ok":true}"#] {
            let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(body)]);
            let report = TelegramChannelAdapter::default()
                .deliver(
                    envelope(
                        vec![OutboundPart::Retract {
                            vendor_message_ref: "42".to_string(),
                        }],
                        None,
                    ),
                    &egress,
                )
                .await
                .expect("deliver drives");

            assert!(
                !matches!(&report.parts[0], PartDeliveryOutcome::Sent { .. }),
                "deleteMessage must not report Sent without result:true: {body}"
            );
        }
    }

    #[tokio::test]
    async fn deliver_retract_with_non_numeric_ref_is_permanent_without_egress() {
        let egress = ScriptedEgress::new(Vec::new());
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(
                    vec![OutboundPart::Retract {
                        vendor_message_ref: "not-a-message-id".to_string(),
                    }],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Permanent { .. }
        ));
        assert!(
            egress.requests.lock().unwrap().is_empty(),
            "an unparseable retract target must not reach the vendor"
        );
    }

    #[tokio::test]
    async fn deliver_posts_send_message_through_the_token_path_template() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":true,"result":{"message_id":42}}"#,
        )]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(vec![OutboundPart::Text("hello".to_string())], Some("77")),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert_eq!(report.parts.len(), 1);
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Sent { vendor_message_ref: Some(id) } if id == "42"
        ));
        let requests = egress.requests.lock().unwrap();
        assert_eq!(
            requests[0].url, "https://api.telegram.org/bot{telegram_bot_token}/sendMessage",
            "the token rides the declared path placeholder, substituted host-side"
        );
        assert_eq!(
            requests[0].credential.as_ref().map(SecretHandle::as_str),
            Some(TELEGRAM_BOT_TOKEN_HANDLE)
        );
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_deref().unwrap_or_default()).unwrap();
        assert_eq!(body["chat_id"], "8675309");
        assert_eq!(body["text"], "hello");
        assert_eq!(body["message_thread_id"], 77, "numeric topic threads");
    }

    #[tokio::test]
    async fn deliver_send_requires_message_id_evidence() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(r#"{"ok":true}"#)]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(vec![OutboundPart::Text("hello".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert!(
            !matches!(&report.parts[0], PartDeliveryOutcome::Sent { .. }),
            "sendMessage must not report Sent without result.message_id"
        );
    }

    #[tokio::test]
    async fn deliver_splits_oversized_text_at_the_vendor_limit() {
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":1}}"#),
            ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":2}}"#),
        ]);
        let long_text = "line\n".repeat(1_000); // 5000 chars > 4096
        let report = TelegramChannelAdapter::default()
            .deliver(envelope(vec![OutboundPart::Text(long_text)], None), &egress)
            .await
            .expect("deliver drives");
        assert_eq!(report.parts.len(), 2);
        assert!(
            report
                .parts
                .iter()
                .all(|part| matches!(part, PartDeliveryOutcome::Sent { .. }))
        );
        assert_eq!(egress.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn deliver_classifies_vendor_and_egress_failures_and_stops() {
        // 429 body → Retryable; the second text part is never attempted.
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":false,"error_code":429,"description":"Too Many Requests"}"#,
        )]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(
                    vec![
                        OutboundPart::Text("one".to_string()),
                        OutboundPart::Text("two".to_string()),
                    ],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert_eq!(report.parts.len(), 1);
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Retryable { .. }
        ));
        assert_eq!(egress.requests.lock().unwrap().len(), 1);

        // 403 → Unauthorized (bot kicked / token revoked).
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":false,"error_code":403,"description":"Forbidden"}"#,
        )]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(vec![OutboundPart::Text("x".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Unauthorized { .. }
        ));

        // Missing credential material → Unauthorized without vendor traffic
        // beyond the failed attempt.
        let egress = ScriptedEgress::new(vec![Err(RestrictedEgressError::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        })]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(vec![OutboundPart::Text("x".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Unauthorized { .. }
        ));
    }

    #[tokio::test]
    async fn deliver_rejects_empty_envelopes_and_unsupported_attachments() {
        let egress = ScriptedEgress::new(Vec::new());
        let error = TelegramChannelAdapter::default()
            .deliver(envelope(Vec::new(), None), &egress)
            .await
            .expect_err("empty envelope is a render error");
        assert!(matches!(error, ChannelError::Render { .. }));
        assert!(egress.requests.lock().unwrap().is_empty());
    }
}
