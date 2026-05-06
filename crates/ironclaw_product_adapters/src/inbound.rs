//! Inbound envelope, payload, and acknowledgement types.

use chrono::{DateTime, Utc};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};

use crate::auth::ProtocolAuthEvidence;
use crate::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
};
use crate::identity::{AdapterInstallationId, ProductAdapterId};

/// Why an adapter is forwarding a group/supergroup/channel message into the
/// canonical pipeline. Group ambient messages must NOT be forwarded; only
/// explicit triggers create envelopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductTriggerReason {
    /// Direct/private chat — the participant always receives messages.
    DirectChat,
    /// Explicit @mention of the bot.
    BotMention,
    /// Reply to a message authored by the bot.
    ReplyToBot,
    /// Recognized bot command (e.g. `/start`).
    BotCommand,
    /// Explicit linked-thread action (e.g. an inline button referencing a
    /// known thread).
    LinkedThreadAction,
}

/// Wrapped user-message inbound payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMessagePayload {
    /// Plain-text content. Adapters MUST strip protocol formatting (HTML,
    /// MarkdownV2, mention prefixes) before populating this field. Length is
    /// bounded by the workflow's content policy; this struct enforces a hard
    /// upper bound to prevent unbounded growth.
    pub text: String,
    pub attachments: Vec<ProductAttachmentDescriptor>,
    pub trigger: ProductTriggerReason,
}

const USER_MESSAGE_TEXT_MAX_BYTES: usize = 64 * 1024;

impl UserMessagePayload {
    pub fn new(
        text: impl Into<String>,
        attachments: Vec<ProductAttachmentDescriptor>,
        trigger: ProductTriggerReason,
    ) -> Result<Self, crate::error::ProductAdapterError> {
        let text = text.into();
        if text.len() > USER_MESSAGE_TEXT_MAX_BYTES {
            return Err(crate::error::ProductAdapterError::MalformedInboundPayload {
                reason: format!(
                    "user message text exceeds {USER_MESSAGE_TEXT_MAX_BYTES}-byte limit"
                ),
            });
        }
        Ok(Self {
            text,
            attachments,
            trigger,
        })
    }
}

/// Wrapped command inbound payload (e.g. `/help`, `/configure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundCommandPayload {
    pub command: String,
    pub arguments: String,
    pub trigger: ProductTriggerReason,
}

/// All supported inbound payload kinds. Approval/auth resolutions and
/// projection subscriptions are tracked here so the adapter contract has
/// space to grow into #3094 without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductInboundPayload {
    UserMessage(UserMessagePayload),
    Command(InboundCommandPayload),
    /// Placeholder for #3094. Contract tests render fake gate payloads;
    /// production resolution flows live in the interaction services.
    ApprovalResolution {
        gate_ref: String,
    },
    /// Placeholder for #3094 (auth flow).
    AuthResolution {
        auth_request_ref: String,
    },
    /// Subscription request for a projection cursor (Web/CLI/API).
    SubscriptionRequest {
        thread_id_hint: Option<String>,
    },
    /// Explicit no-op acknowledgement — for ambient group messages, edited
    /// messages we choose not to act on, etc. Workflow returns
    /// [`ProductInboundAck::NoOp`] in response.
    NoOp,
}

/// Inbound envelope. Constructed by the adapter from a verified protocol
/// event, then handed to the workflow facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductInboundEnvelope {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_event_id: ExternalEventId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub auth_evidence: ProtocolAuthEvidence,
    pub received_at: DateTime<Utc>,
    pub payload: ProductInboundPayload,
}

/// Why a [`ProductInboundAck::Rejected`] outcome was returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductRejectionKind {
    /// External actor is not paired/bound to a canonical user.
    BindingRequired,
    /// Authenticated actor is not allowed to access the resolved thread.
    AccessDenied,
    /// Adapter installation is unknown to the workflow.
    UnknownInstallation,
    /// Workflow-level policy rejected the message (rate limit, content
    /// policy, etc.).
    PolicyDenied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductRejection {
    pub kind: ProductRejectionKind,
    /// User-safe explanation. Must not contain raw secrets, host paths,
    /// raw provider/runtime internals, or backend diagnostics.
    pub reason: String,
}

/// Pipeline outcome of an inbound envelope. Adapters use this to choose the
/// protocol-level response status code:
///
/// * `Accepted` / `DeferredBusy` / `NoOp` / `Duplicate` -> 200
/// * `Rejected { BindingRequired | AccessDenied | UnknownInstallation }` -> 403
/// * `Rejected { PolicyDenied }` -> 403/422 depending on protocol convention
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductInboundAck {
    /// Message accepted and a turn was submitted.
    Accepted {
        accepted_message_ref: String,
        submitted_run_id: Option<TurnRunId>,
    },
    /// Message accepted but submission was deferred because another run is
    /// already active on the same canonical thread.
    DeferredBusy {
        accepted_message_ref: String,
        active_run_id: TurnRunId,
    },
    /// Message was rejected; do not retry.
    Rejected(ProductRejection),
    /// Duplicate external_event_id. Carries the prior outcome so the adapter
    /// can choose an idempotent protocol response.
    Duplicate { prior: Box<ProductInboundAck> },
    /// Successful no-op (ambient group message, ignored edit, etc.).
    NoOp,
}

impl ProductInboundAck {
    pub fn is_durable_outcome(&self) -> bool {
        matches!(
            self,
            Self::Accepted { .. }
                | Self::DeferredBusy { .. }
                | Self::Rejected(_)
                | Self::Duplicate { .. }
                | Self::NoOp
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::mark_shared_secret_header_verified;
    use crate::external::{ExternalActorRef, ExternalConversationRef, ExternalEventId};
    use crate::identity::{AdapterInstallationId, ProductAdapterId};

    fn sample_envelope(payload: ProductInboundPayload) -> ProductInboundEnvelope {
        ProductInboundEnvelope {
            adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
            installation_id: AdapterInstallationId::new("install_alpha").expect("valid"),
            external_event_id: ExternalEventId::new("update:42").expect("valid"),
            external_actor_ref: ExternalActorRef::new("telegram_user", "777", None).expect("valid"),
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "12345",
                Some("topic-7"),
                Some("msg-100"),
            )
            .expect("valid"),
            auth_evidence: mark_shared_secret_header_verified(
                "X-Telegram-Bot-Api-Secret-Token",
                "telegram_install_alpha",
            ),
            received_at: Utc::now(),
            payload,
        }
    }

    #[test]
    fn user_message_text_length_bounded() {
        let oversize = "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES + 1);
        assert!(
            UserMessagePayload::new(oversize, vec![], ProductTriggerReason::DirectChat).is_err()
        );
    }

    #[test]
    fn envelope_round_trips() {
        let envelope = sample_envelope(ProductInboundPayload::NoOp);
        let json = serde_json::to_string(&envelope).expect("serialize");
        let parsed: ProductInboundEnvelope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, envelope);
    }

    #[test]
    fn ack_durable_outcomes_classify_correctly() {
        assert!(
            ProductInboundAck::Accepted {
                accepted_message_ref: "msg".into(),
                submitted_run_id: None,
            }
            .is_durable_outcome()
        );
        assert!(ProductInboundAck::NoOp.is_durable_outcome());
        assert!(
            ProductInboundAck::Duplicate {
                prior: Box::new(ProductInboundAck::NoOp),
            }
            .is_durable_outcome()
        );
    }

    #[test]
    fn user_message_payload_bounds_attachments_implicitly() {
        // The payload itself has no attachment count cap (the workflow
        // applies one via policy). This test pins the contract so any future
        // change is intentional.
        let attachments = (0..16)
            .map(|i| {
                ProductAttachmentDescriptor::new(
                    format!("file_{i}"),
                    "image/jpeg",
                    None,
                    Some(2048),
                    crate::external::ProductAttachmentKind::Image,
                )
                .expect("valid")
            })
            .collect();
        let payload = UserMessagePayload::new("hi", attachments, ProductTriggerReason::DirectChat)
            .expect("valid");
        assert_eq!(payload.attachments.len(), 16);
    }
}
