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
use ironclaw_attachments::InboundAttachment;
use ironclaw_host_api::{NetworkMethod, RestrictedEgress, RestrictedEgressRequest, SecretHandle};
use ironclaw_product_adapters::{
    AdapterInstallationId, AttachmentRef, ChannelAdapter, ChannelContext, ChannelError,
    DeliveryReport, InboundOutcome, OutboundEnvelope, OutboundPart, PartDeliveryOutcome,
    VerifiedInbound,
};

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

    async fn fetch_attachment(
        &self,
        attachment: &AttachmentRef,
        egress: &dyn RestrictedEgress,
    ) -> Result<InboundAttachment, ChannelError> {
        crate::attachment_transfer::fetch_attachment(attachment, egress).await
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
        let reply_to_message_id = envelope
            .target
            .conversation
            .reply_target_message_id()
            .map(str::parse::<i64>)
            .transpose()
            .map_err(|_| ChannelError::Render {
                reason: "telegram reply target is not a numeric message id".to_string(),
            })?;

        let mut parts = Vec::new();
        'parts: for part in &envelope.parts {
            match part {
                OutboundPart::Text(text) => {
                    for chunk in telegram_text_chunks(text) {
                        let mut body = serde_json::json!({ "chat_id": chat_id, "text": chunk });
                        if let Some(thread_id) = message_thread_id {
                            body["message_thread_id"] = thread_id.into();
                        }
                        if let Some(reply_to) = reply_to_message_id {
                            body["reply_to_message_id"] = reply_to.into();
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
                OutboundPart::File(file) => {
                    let outcome = crate::attachment_transfer::send_document(
                        egress,
                        &chat_id,
                        message_thread_id,
                        reply_to_message_id,
                        file,
                    )
                    .await;
                    let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                    parts.push(outcome);
                    if !sent {
                        break 'parts;
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
    telegram_message_response_outcome("sendMessage", response.status, &response.body)
}

pub(super) fn telegram_message_response_outcome(
    method: &str,
    status: u16,
    body: &[u8],
) -> PartDeliveryOutcome {
    if !(200..300).contains(&status) {
        return telegram_outcome_for_status(
            status,
            format!("telegram bot api returned status {status}"),
        );
    }
    let parsed: TelegramSendMessageResponse = match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(_) => {
            return PartDeliveryOutcome::Retryable {
                reason: format!("{method} response was not valid JSON"),
            };
        }
    };
    if parsed.ok {
        return match parsed.result {
            Some(message) => PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(message.message_id.to_string()),
            },
            None => PartDeliveryOutcome::Retryable {
                reason: format!("{method} response omitted result.message_id evidence"),
            },
        };
    }
    telegram_outcome_for_status(
        parsed.error_code.unwrap_or(400),
        format!("telegram rejected {method}"),
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

pub(super) fn telegram_outcome_for_egress_error(
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
#[path = "channel_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "channel_fetch_tests.rs"]
mod fetch_tests;

#[cfg(test)]
#[path = "channel_deliver_tests.rs"]
mod deliver_tests;
