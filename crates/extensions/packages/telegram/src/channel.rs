//! The Telegram channel halves (generic ingress, extension-runtime P4).
//!
//! `receive` parses one HOST-VERIFIED Bot API webhook update (the manifest's
//! `shared_secret_header` recipe — Telegram's `X-Telegram-Bot-Api-Secret-Token`
//! — runs in the host's generic verifier; this adapter never sees the
//! secret).
//!
//! **Vendor-side webhook wiring is not here any more.** `setWebhook` /
//! `deleteWebhook` were the only `activate`/`cleanup` implementations in the
//! workspace, and every input they needed was already host-known: the host
//! owns the webhook route, so it owns the URL. They are now the manifest's
//! `[channel.ingress.registration]` / `[channel.ingress.deregistration]`
//! recipes, run by the generic host executor through the same restricted
//! egress with the same host-side credential injection. Two method bodies
//! became zero lines, and a manifest field can no longer drift from an
//! implementation because there is no implementation.

use async_trait::async_trait;
use ironclaw_extension_contracts::auth_prompt::render_channel_auth_prompt;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelDelivery, ChannelError, ChannelIngress, ChannelReply, DeliveryReport, InboundOutcome,
    NormalizedInboundMessage, OutboundEnvelope, OutboundPart, PartDeliveryOutcome, ReactionAction,
    RunReaction, VerifiedInbound,
};
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgress, RestrictedEgressRequest};
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use ironclaw_host_api::{action::NetworkMethod, ids::SecretHandle};

use crate::attachment_transfer::{ParsedTelegramBatchFragment, ParsedTelegramInboundMessage};
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
impl ChannelIngress for TelegramChannelAdapter {
    async fn receive(
        &self,
        request: VerifiedInbound<'_>,
        egress: &dyn RestrictedEgress,
    ) -> Result<InboundOutcome, ChannelError> {
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
            TelegramInboundEvent::Message(parsed) => {
                let ParsedTelegramInboundMessage {
                    message,
                    pending_attachments,
                } = *parsed;
                let message = complete_message(message, pending_attachments, egress).await?;
                if message.text.trim().is_empty() && message.attachments.is_empty() {
                    // Nothing usable survived (e.g. a sticker-only update whose
                    // transfer failed permanently, or an update type we carry
                    // no content for): acknowledge instead of starting an
                    // empty turn.
                    return Ok(InboundOutcome::Ignore);
                }
                Ok(InboundOutcome::Messages(vec![message]))
            }
            TelegramInboundEvent::BatchFragment(parsed) => {
                let ParsedTelegramBatchFragment {
                    mut fragment,
                    pending_attachments,
                } = *parsed;
                fragment.message =
                    complete_message(fragment.message, pending_attachments, egress).await?;
                // A degraded-to-empty fragment still ships: the batch key and
                // order slot must survive so sibling fragments settle into one
                // coherent message.
                Ok(InboundOutcome::BatchFragment(Box::new(fragment)))
            }
        }
    }
}

async fn complete_message(
    mut message: NormalizedInboundMessage,
    pending_attachments: Vec<ironclaw_extension_contracts::channel_adapter::ChannelAttachmentRef>,
    egress: &dyn RestrictedEgress,
) -> Result<NormalizedInboundMessage, ChannelError> {
    for pending in pending_attachments {
        // A retryable transfer failure keeps failing the whole request so
        // ingress answers 503 and vendor redelivery can succeed later with the
        // full content. Every deterministic failure degrades to "message
        // without this attachment" instead: failing the update would make
        // Telegram redeliver a payload that can never improve, wedging the
        // chat's in-order queue behind it.
        let external_file_id = pending.descriptor.external_file_id.clone();
        match crate::attachment_transfer::fetch_attachment(&pending, egress).await {
            Ok(fetched) => match pending.complete(fetched) {
                Ok(attachment) => message.attachments.push(attachment),
                Err(error) => {
                    tracing::debug!(
                        %external_file_id,
                        %error,
                        "dropping telegram attachment whose fetched bytes failed completion"
                    );
                }
            },
            Err(
                error @ ChannelError::AttachmentTransfer {
                    retryable: true, ..
                },
            ) => return Err(error),
            Err(error) => {
                tracing::debug!(
                    %external_file_id,
                    %error,
                    "dropping telegram attachment after a non-retryable transfer failure"
                );
            }
        }
    }
    Ok(message)
}

/// One vendor mechanism serves both output axes here, as it does for every
/// conversational vendor — which is exactly why the reply/delivery
/// distinction stayed invisible until a streaming channel existed. The halves
/// stay separate because the coordinator picks one by route; the sharing is
/// an implementation fact, recorded here rather than folded into the contract.
impl TelegramChannelAdapter {
    /// Render one coordinator envelope as Bot API `sendMessage` calls: plain
    /// text split at the vendor's 4096-char limit, `chat_id` from the
    /// conversation ref, forum-topic threading when the anchor is numeric.
    /// The bot token rides the declared path placeholder — injected
    /// host-side, never adapter-visible.
    async fn send(
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
                OutboundPart::React {
                    vendor_message_ref,
                    reaction,
                    action,
                } => {
                    let outcome = match vendor_message_ref.parse::<i64>() {
                        Ok(message_id) => {
                            // `setMessageReaction` REPLACES the bot's reactions
                            // on the message, so an add sets the single emoji and
                            // a remove sets the empty set.
                            let reaction_field = match action {
                                ReactionAction::Add => serde_json::json!([{
                                    "type": "emoji",
                                    "emoji": telegram_reaction_emoji(*reaction),
                                }]),
                                ReactionAction::Remove => serde_json::json!([]),
                            };
                            set_telegram_reaction(
                                egress,
                                serde_json::json!({
                                    "chat_id": chat_id,
                                    "message_id": message_id,
                                    "reaction": reaction_field,
                                }),
                            )
                            .await
                        }
                        Err(_) => PartDeliveryOutcome::Permanent {
                            reason: format!(
                                "reaction target `{vendor_message_ref}` is not a telegram message id"
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
        Ok(DeliveryReport::from_parts(parts))
    }
}

#[async_trait]
impl ChannelReply for TelegramChannelAdapter {
    async fn send_reply(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        self.send(envelope, egress).await
    }
}

#[async_trait]
impl ChannelDelivery for TelegramChannelAdapter {
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        self.send(envelope, egress).await
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

/// Telegram allowed-reaction emoji for a neutral run reaction. `setMessageReaction`
/// accepts only a fixed allowlist, so the Slack ✅/⚠️/❌ map to the nearest
/// allowed Telegram reactions (👌 / 🤔 / 👎).
fn telegram_reaction_emoji(reaction: RunReaction) -> &'static str {
    match reaction {
        RunReaction::Working => "👀",
        RunReaction::Done => "👌",
        RunReaction::NeedsInput => "🤔",
        RunReaction::Failed => "👎",
    }
}

/// Set (or clear) the bot's reaction on a message. Best-effort: a failed
/// reaction never fails the run.
async fn set_telegram_reaction(
    egress: &dyn RestrictedEgress,
    body: serde_json::Value,
) -> PartDeliveryOutcome {
    let response = match egress
        .send(bot_api_request("setMessageReaction", body))
        .await
    {
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
            return PartDeliveryOutcome::Ambiguous {
                reason: format!("setMessageReaction response was not valid JSON: {error}"),
            };
        }
    };
    if parsed.ok {
        return PartDeliveryOutcome::Sent {
            vendor_message_ref: None,
        };
    }
    let description = parsed
        .description
        .unwrap_or_else(|| "unknown_error".to_string());
    telegram_outcome_for_status(
        parsed.error_code.unwrap_or(400),
        format!("telegram rejected setMessageReaction ({description})"),
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
            return PartDeliveryOutcome::Ambiguous {
                reason: format!("{method} response was not valid JSON"),
            };
        }
    };
    if parsed.ok {
        return match parsed.result {
            Some(message) => PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(message.message_id.to_string()),
            },
            None => PartDeliveryOutcome::Ambiguous {
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
            return PartDeliveryOutcome::Ambiguous {
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
            None => PartDeliveryOutcome::Ambiguous {
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
        EgressError::Transport { .. } => PartDeliveryOutcome::Ambiguous {
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

/// Split text at Telegram's 4096-UTF-16-unit limit without breaking scalar
/// values. The protocol engine owns the one authoritative splitter.
fn telegram_text_chunks(text: &str) -> Vec<String> {
    crate::render::chunk_text_utf16(text, crate::render::TELEGRAM_MESSAGE_MAX_UTF16_UNITS)
        .into_iter()
        .map(str::to_string)
        .collect()
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
