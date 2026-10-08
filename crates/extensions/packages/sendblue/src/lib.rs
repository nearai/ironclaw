//! Direct text messaging through Sendblue. The host owns secrets, admission,
//! identity pairing, replay suppression, delivery state, and network policy.
mod preference_targets;
mod reply;
pub use preference_targets::SendbluePreferenceTargetCodec;

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::auth_prompt::render_channel_auth_prompt;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelDelivery, ChannelError, ChannelIngress, ChannelSurfaces, DeliveryReport, InboundOutcome,
    NormalizedInboundMessage, OutboundEnvelope, OutboundPart, PartDeliveryOutcome,
    ProductTriggerReason, VerifiedInbound,
};
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
};
use ironclaw_host_api::{action::NetworkMethod, ids::SecretHandle};
use serde::Deserialize;

#[derive(Debug, Default)]
pub struct SendblueChannelAdapter;

/// The package-owned surfaces shared by native activation and host assembly.
pub fn sendblue_surfaces() -> ChannelSurfaces {
    let adapter = Arc::new(SendblueChannelAdapter);
    ChannelSurfaces::default()
        .with_ingress(adapter.clone())
        .with_reply(adapter.clone())
        .with_delivery(adapter)
}

#[derive(Deserialize)]
struct Webhook {
    #[serde(default)]
    is_outbound: Option<bool>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    from_number: Option<String>,
    #[serde(default)]
    to_number: Option<String>,
    #[serde(default)]
    message_handle: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    media_url: Option<String>,
}

fn phone(value: &str) -> bool {
    let bytes = value.as_bytes();
    (9..=16).contains(&bytes.len())
        && bytes.first() == Some(&b'+')
        && bytes.get(1).is_some_and(|b| matches!(b, b'1'..=b'9'))
        && bytes.iter().skip(1).all(u8::is_ascii_digit)
}

#[async_trait]
impl ChannelIngress for SendblueChannelAdapter {
    async fn receive(
        &self,
        request: VerifiedInbound<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<InboundOutcome, ChannelError> {
        let config = |name: &str| {
            request
                .config
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let line = config("sendblue_from_number")
            .filter(|n| phone(n))
            .ok_or_else(|| ChannelError::Configuration {
                reason: "Configure an assigned E.164 Sendblue line".into(),
            })?;
        let allow = config("sendblue_allow_from").unwrap_or("");
        if request.body.len() > 65536 {
            return Err(ChannelError::Parse {
                reason: "Sendblue webhook exceeds 64 KiB".into(),
            });
        }
        let body: Webhook =
            serde_json::from_slice(request.body).map_err(|error| ChannelError::Parse {
                reason: format!("Invalid Sendblue webhook: {error}"),
            })?;
        if body.is_outbound != Some(false)
            || body.status.as_deref() != Some("RECEIVED")
            || body.group_id.as_ref().is_some_and(|g| !g.is_empty())
            || body.to_number.as_deref() != Some(line)
        {
            return Ok(InboundOutcome::Ignore);
        }
        let sender = body
            .from_number
            .filter(|n| phone(n))
            .ok_or_else(|| ChannelError::Parse {
                reason: "Missing E.164 sender".into(),
            })?;
        if !allow
            .split(',')
            .map(str::trim)
            .any(|n| n == sender || n == "*")
        {
            return Ok(InboundOutcome::Ignore);
        }
        let handle = body
            .message_handle
            .filter(|h| !h.is_empty() && h.len() <= 256 && !h.chars().any(char::is_control))
            .ok_or_else(|| ChannelError::Parse {
                reason: "Missing bounded message handle".into(),
            })?;
        let mut text = body.content.unwrap_or_default();
        if body.media_url.is_some_and(|url| !url.is_empty()) {
            text.push_str("\n[Attachment received; this channel supports text only. Please resend its contents as text.]");
        }
        if text.trim().is_empty() {
            return Ok(InboundOutcome::Ignore);
        }
        let parse = |error| ChannelError::Parse {
            reason: format!("Invalid Sendblue reference: {error}"),
        };
        Ok(InboundOutcome::Messages(vec![NormalizedInboundMessage {
            actor: ExternalActorRef::new("sendblue_user", &sender, Some(sender.clone()))
                .map_err(parse)?,
            conversation: ExternalConversationRef::new(Some(line), &sender, None, Some(&handle))
                .map_err(parse)?,
            event_id: ExternalEventId::new(format!("{}:{handle}", request.installation_id))
                .map_err(parse)?,
            text,
            trigger: ProductTriggerReason::DirectChat,
            attachments: Vec::new(),
            conversation_context: None,
            reply_context: None,
        }]))
    }
}

#[async_trait]
impl ChannelDelivery for SendblueChannelAdapter {
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        let recipient = envelope.target.conversation.conversation_id();
        let line = envelope.target.conversation.space_id().unwrap_or("");
        if !phone(line)
            || !phone(recipient)
            || envelope.target.conversation.topic_id().is_some()
            || envelope.target.thread_anchor.is_some()
        {
            return Err(ChannelError::Render {
                reason: "Sendblue requires a direct phone conversation with its assigned line"
                    .into(),
            });
        }
        let mut results = Vec::new();
        for part in envelope.parts {
            let text = match part {
                OutboundPart::Text(text) => text,
                OutboundPart::AuthPrompt {
                    view,
                    direct_message,
                } => render_channel_auth_prompt(&view, direct_message),
                _ => {
                    results.push(PartDeliveryOutcome::Permanent {
                        reason: "Sendblue supports text only".into(),
                    });
                    break;
                }
            };
            let characters: Vec<char> = text.chars().collect();
            for chunk in characters.chunks(2000) {
                let outcome =
                    send_text(line, recipient, &chunk.iter().collect::<String>(), egress).await?;
                let stop = !matches!(outcome, PartDeliveryOutcome::Sent { .. });
                results.push(outcome);
                if stop {
                    return Ok(DeliveryReport::from_parts(results));
                }
            }
        }
        Ok(DeliveryReport::from_parts(results))
    }
    fn supports_private_delivery(&self) -> bool {
        true
    }
}

async fn send_text(
    line: &str,
    recipient: &str,
    text: &str,
    egress: &dyn RestrictedEgress,
) -> Result<PartDeliveryOutcome, ChannelError> {
    let credential =
        SecretHandle::new("sendblue_api_secret").map_err(|error| ChannelError::Configuration {
            reason: error.to_string(),
        })?;
    let body = serde_json::to_vec(
        &serde_json::json!({"number": recipient, "from_number": line, "content": text}),
    )
    .map_err(|error| ChannelError::Render {
        reason: error.to_string(),
    })?;
    let response = match egress
        .send(RestrictedEgressRequest {
            method: NetworkMethod::Post,
            url: "https://api.sendblue.com/api/send-message".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: Some(body),
            credential: Some(credential),
            body_credentials: Vec::new(),
        })
        .await
    {
        Ok(response) => response,
        Err(RestrictedEgressError::AuthRequired { .. }) => {
            return Ok(PartDeliveryOutcome::Unauthorized {
                reason: "Sendblue credentials are unavailable".into(),
            });
        }
        Err(RestrictedEgressError::Transport { .. } | RestrictedEgressError::ResponseTooLarge) => {
            return Ok(PartDeliveryOutcome::Ambiguous {
                reason:
                    "Sendblue acceptance is unconfirmed; inspect provider history before retrying"
                        .into(),
            });
        }
        Err(error) => {
            return Ok(PartDeliveryOutcome::Permanent {
                reason: format!("Sendblue egress denied: {error}"),
            });
        }
    };
    if response.status == 401 || response.status == 403 {
        return Ok(PartDeliveryOutcome::Unauthorized {
            reason: "Sendblue rejected the credentials".into(),
        });
    }
    if !(200..300).contains(&response.status) {
        return Ok(PartDeliveryOutcome::Ambiguous {
            reason: format!(
                "Sendblue returned HTTP {}; inspect provider history before retrying",
                response.status
            ),
        });
    }
    let Ok(data) = serde_json::from_slice::<serde_json::Value>(&response.body) else {
        return Ok(PartDeliveryOutcome::Ambiguous {
            reason: "Sendblue returned an unreadable acceptance response".into(),
        });
    };
    if matches!(
        data.get("status").and_then(serde_json::Value::as_str),
        Some("ERROR" | "DECLINED")
    ) || data
        .get("error_code")
        .is_some_and(|code| !code.is_null() && code.as_i64() != Some(0))
    {
        return Ok(PartDeliveryOutcome::Permanent {
            reason: "Sendblue rejected the message".into(),
        });
    }
    match (
        data.get("status").and_then(serde_json::Value::as_str),
        data.get("message_handle")
            .and_then(serde_json::Value::as_str),
    ) {
        (Some("SENT" | "DELIVERED" | "READ"), Some(handle))
            if !handle.is_empty()
                && handle.len() <= 256
                && !handle.chars().any(char::is_control) =>
        {
            Ok(PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(handle.into()),
            })
        }
        (Some("QUEUED"), Some(handle))
            if !handle.is_empty()
                && handle.len() <= 256
                && !handle.chars().any(char::is_control) =>
        {
            Ok(PartDeliveryOutcome::Ambiguous {
                reason: "Sendblue accepted and queued the message; handset delivery is unconfirmed"
                    .into(),
            })
        }
        _ => Ok(PartDeliveryOutcome::Ambiguous {
            reason: "Sendblue did not confirm acceptance".into(),
        }),
    }
}
