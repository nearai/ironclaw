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
use ironclaw_extension_contracts::auth_prompt::render_channel_auth_prompt;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelAdapter, ChannelAttachmentRef, ChannelContext, ChannelError, DeliveryReport,
    InboundOutcome, OutboundEnvelope, OutboundPart, PartDeliveryOutcome, ProgressivePreviewPart,
    VerifiedInbound,
};
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgress, RestrictedEgressRequest};
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use ironclaw_host_api::{action::NetworkMethod, attachment::InboundAttachment, ids::SecretHandle};

use crate::{
    GroupTriggerPolicy, TELEGRAM_API_HOST, TelegramInboundEvent, normalize_telegram_update,
};

/// Config field handle (non-secret) carrying the public webhook URL the
/// activation hook registers with the vendor.
pub const TELEGRAM_WEBHOOK_URL_CONFIG: &str = "telegram_webhook_url";
/// Non-secret config handle carrying the receiving bot's public username.
///
/// The adapter enforces Telegram's public username grammar locally (5–32
/// ASCII alphanumeric/underscore characters ending in `bot`,
/// case-insensitively). A syntactically valid but wrong username cannot be
/// detected without vendor I/O; verifying that identity with a mediated
/// `getMe` call is a separate follow-up, not inbound parsing work.
pub const TELEGRAM_BOT_USERNAME_CONFIG: &str = "bot_username";
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

/// The Telegram channel adapter. The constructor policy remains available for
/// compatibility and tests; shipping ingress overlays the receiving bot
/// identity from verified installation configuration on every request.
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

    fn receiving_bot_username(&self, config: &[(String, String)]) -> Result<String, &'static str> {
        let configured_username = config
            .iter()
            .find(|(handle, _)| handle == TELEGRAM_BOT_USERNAME_CONFIG)
            .map(|(_, value)| value.as_str())
            .unwrap_or(self.group_trigger_policy.bot_username.as_str());
        if !(5..=32).contains(&configured_username.len())
            || configured_username.trim() != configured_username
            || !configured_username
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
            || !configured_username
                .get(configured_username.len().saturating_sub(3)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case("bot"))
        {
            return Err("missing or invalid Telegram bot username configuration");
        }
        Ok(configured_username.to_string())
    }

    fn effective_group_trigger_policy(
        &self,
        config: &[(String, String)],
    ) -> Result<GroupTriggerPolicy, ChannelError> {
        let mut policy = self.group_trigger_policy.clone();
        policy.bot_username =
            self.receiving_bot_username(config)
                .map_err(|reason| ChannelError::Configuration {
                    reason: reason.to_string(),
                })?;
        Ok(policy)
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
        self.receiving_bot_username(ctx.config)
            .map_err(|reason| ChannelError::VendorWiring {
                reason: reason.to_string(),
            })?;
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
        let group_trigger_policy = self.effective_group_trigger_policy(request.config)?;
        match normalize_telegram_update(request.body, &installation_id, &group_trigger_policy)
            .map_err(|error| ChannelError::Parse {
                reason: error.to_string(),
            })? {
            TelegramInboundEvent::Ignore => Ok(InboundOutcome::Ignore),
            TelegramInboundEvent::Message(message) => Ok(InboundOutcome::Messages(vec![*message])),
            TelegramInboundEvent::BatchFragment(fragment) => {
                Ok(InboundOutcome::BatchFragment(fragment))
            }
        }
    }

    async fn fetch_attachment(
        &self,
        attachment: &ChannelAttachmentRef,
        egress: &dyn RestrictedEgress,
    ) -> Result<InboundAttachment, ChannelError> {
        crate::attachment_transfer::fetch_attachment(attachment, egress).await
    }

    /// Render one coordinator envelope as Bot API calls: durable text uses
    /// `sendMessage`; disposable private-chat previews use
    /// `sendMessageDraft`. The bot token rides the declared path placeholder
    /// and is injected host-side, never adapter-visible.
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
                        if let Some(reply_to) = reply_to_message_id {
                            body["reply_to_message_id"] = reply_to.into();
                        }
                        let outcome = send_telegram_message(egress, body).await;
                        let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                        parts.push(outcome);
                        if !sent {
                            break 'parts;
                        }
                    }
                }
                OutboundPart::ProgressivePreview(ProgressivePreviewPart::Start(_)) => {
                    let outcome = match private_telegram_chat_id(&chat_id) {
                        Ok(chat_id) => {
                            let draft_id = telegram_draft_id(&envelope.delivery_attempt_id);
                            send_telegram_draft(
                                egress,
                                telegram_draft_body(chat_id, message_thread_id, draft_id, ""),
                                draft_id,
                            )
                            .await
                        }
                        Err(reason) => PartDeliveryOutcome::Permanent { reason },
                    };
                    let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                    parts.push(outcome);
                    if !sent {
                        break 'parts;
                    }
                }
                OutboundPart::ProgressivePreview(ProgressivePreviewPart::Update {
                    vendor_message_ref,
                    accepted_text,
                    current_text,
                }) => {
                    let outcome = match (
                        private_telegram_chat_id(&chat_id),
                        telegram_draft_id_from_ref(vendor_message_ref),
                    ) {
                        (Ok(_), _) if !current_text.starts_with(accepted_text) => {
                            PartDeliveryOutcome::Permanent {
                                reason: "progressive preview no longer extends the accepted text"
                                    .to_string(),
                            }
                        }
                        (Ok(_), _) if current_text.chars().count() > TELEGRAM_TEXT_LIMIT_CHARS => {
                            PartDeliveryOutcome::Permanent {
                                reason: "progressive preview exceeds Telegram's text limit"
                                    .to_string(),
                            }
                        }
                        (Ok(chat_id), Ok(draft_id)) => {
                            send_telegram_draft(
                                egress,
                                telegram_draft_body(
                                    chat_id,
                                    message_thread_id,
                                    draft_id,
                                    current_text,
                                ),
                                draft_id,
                            )
                            .await
                        }
                        (Err(reason), _) | (_, Err(reason)) => {
                            PartDeliveryOutcome::Permanent { reason }
                        }
                    };
                    let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                    parts.push(outcome);
                    if !sent {
                        break 'parts;
                    }
                }
                OutboundPart::ProgressivePreview(ProgressivePreviewPart::Stop {
                    vendor_message_ref,
                }) => {
                    // Telegram exposes no explicit draft-stop operation. A
                    // valid reference means Ironclaw can relinquish the draft;
                    // final delivery clears it, otherwise Telegram expires it.
                    let outcome = match telegram_draft_id_from_ref(vendor_message_ref) {
                        Ok(_) => PartDeliveryOutcome::Sent {
                            vendor_message_ref: Some(vendor_message_ref.clone()),
                        },
                        Err(reason) => PartDeliveryOutcome::Permanent { reason },
                    };
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

fn private_telegram_chat_id(chat_id: &str) -> Result<i64, String> {
    match chat_id.parse::<i64>() {
        Ok(chat_id) if chat_id > 0 => Ok(chat_id),
        _ => Err("sendMessageDraft requires a numeric private chat id".to_string()),
    }
}

fn telegram_draft_id(delivery_attempt_id: &str) -> i32 {
    const FNV_OFFSET_BASIS: u32 = 2_166_136_261;
    const FNV_PRIME: u32 = 16_777_619;

    let hash = delivery_attempt_id
        .bytes()
        .fold(FNV_OFFSET_BASIS, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME)
        });
    (hash & i32::MAX as u32).max(1) as i32
}

fn telegram_draft_id_from_ref(reference: &str) -> Result<i32, String> {
    match reference.parse::<i32>() {
        Ok(draft_id) if draft_id > 0 => Ok(draft_id),
        _ => Err("Telegram draft reference must be a non-zero integer".to_string()),
    }
}

fn telegram_draft_body(
    chat_id: i64,
    message_thread_id: Option<i64>,
    draft_id: i32,
    text: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "draft_id": draft_id,
        "text": text,
    });
    if let Some(thread_id) = message_thread_id {
        body["message_thread_id"] = thread_id.into();
    }
    body
}

async fn send_telegram_draft(
    egress: &dyn RestrictedEgress,
    body: serde_json::Value,
    draft_id: i32,
) -> PartDeliveryOutcome {
    let response = match egress.send(bot_api_request("sendMessageDraft", body)).await {
        Ok(response) => response,
        Err(error) => return telegram_outcome_for_egress_error(&error),
    };
    if !(200..300).contains(&response.status) {
        return telegram_outcome_for_status(
            response.status,
            format!("telegram bot api returned status {}", response.status),
        );
    }
    let parsed: TelegramBooleanResponse = match serde_json::from_slice(&response.body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return PartDeliveryOutcome::Retryable {
                reason: format!("sendMessageDraft response was not valid JSON: {error}"),
            };
        }
    };
    if parsed.ok {
        return match parsed.result {
            Some(true) => PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(draft_id.to_string()),
            },
            Some(false) => PartDeliveryOutcome::Permanent {
                reason: "sendMessageDraft response reported result:false".to_string(),
            },
            None => PartDeliveryOutcome::Retryable {
                reason: "sendMessageDraft response omitted result evidence".to_string(),
            },
        };
    }
    telegram_outcome_for_status(
        parsed.error_code.unwrap_or(400),
        format!(
            "telegram rejected sendMessageDraft ({})",
            parsed
                .description
                .unwrap_or_else(|| "unknown_error".to_string())
        ),
    )
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
struct TelegramBooleanResponse {
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
    let parsed: TelegramBooleanResponse = match serde_json::from_slice(&response.body) {
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
    error: &ironclaw_extension_contracts::tool_adapter::RestrictedEgressError,
) -> PartDeliveryOutcome {
    use ironclaw_extension_contracts::tool_adapter::RestrictedEgressError as EgressError;
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
#[path = "tests/channel.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/channel_fetch.rs"]
mod fetch_tests;

#[cfg(test)]
#[path = "tests/channel_deliver.rs"]
mod deliver_tests;
