//! Outbound envelope, projection-derived payloads, and projection cursor.

use chrono::{DateTime, Utc};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::error::ProductAdapterError;
use crate::external::{ExternalActorRef, ExternalConversationRef};
use crate::identity::{AdapterInstallationId, ProductAdapterId};

const PROJECTION_CURSOR_MAX_BYTES: usize = 1024;
const PROJECTION_THREAD_ID_MAX_BYTES: usize = 512;
const PROJECTION_ITEM_ID_MAX_BYTES: usize = 512;
const PROJECTION_TEXT_MAX_BYTES: usize = 128 * 1024;

fn invalid(kind: &'static str, reason: impl Into<String>) -> ProductAdapterError {
    ProductAdapterError::InvalidIdentifier {
        kind,
        reason: reason.into(),
    }
}

fn validate_bounded_text(
    kind: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ProductAdapterError> {
    if value.is_empty() {
        return Err(invalid(kind, "must not be empty"));
    }
    if value.len() > max {
        return Err(invalid(kind, format!("must be at most {max} bytes")));
    }
    if value
        .chars()
        .any(|c| c == '\0' || c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid(
            kind,
            "must not contain unsupported control characters",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectionCursor(String);

impl ProjectionCursor {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        validate_bounded_text("projection_cursor", &value, PROJECTION_CURSOR_MAX_BYTES)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProjectionCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalReplyView {
    pub turn_run_id: TurnRunId,
    pub text: String,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressUpdateView {
    pub turn_run_id: TurnRunId,
    pub kind: ProgressKind,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    Typing,
    ToolRunning,
    Reflecting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatePromptView {
    pub turn_run_id: TurnRunId,
    pub gate_ref: String,
    pub headline: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPromptView {
    pub turn_run_id: TurnRunId,
    pub auth_request_ref: String,
    pub headline: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductProjectionItem {
    Text { id: String, body: String },
    RunStatus { run_id: TurnRunId, status: String },
    Gate { gate_ref: String, headline: String },
}

impl ProductProjectionItem {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        match self {
            Self::Text { id, body } => {
                validate_bounded_text("projection_item_id", id, PROJECTION_ITEM_ID_MAX_BYTES)?;
                validate_bounded_text("projection_text", body, PROJECTION_TEXT_MAX_BYTES)
            }
            Self::RunStatus { status, .. } => validate_bounded_text(
                "projection_run_status",
                status,
                PROJECTION_ITEM_ID_MAX_BYTES,
            ),
            Self::Gate { gate_ref, headline } => {
                validate_bounded_text(
                    "projection_gate_ref",
                    gate_ref,
                    PROJECTION_ITEM_ID_MAX_BYTES,
                )?;
                validate_bounded_text(
                    "projection_gate_headline",
                    headline,
                    PROJECTION_TEXT_MAX_BYTES,
                )
            }
        }
    }
}

impl<'de> Deserialize<'de> for ProductProjectionItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Wire {
            Text { id: String, body: String },
            RunStatus { run_id: TurnRunId, status: String },
            Gate { gate_ref: String, headline: String },
        }
        let value = match Wire::deserialize(deserializer)? {
            Wire::Text { id, body } => ProductProjectionItem::Text { id, body },
            Wire::RunStatus { run_id, status } => {
                ProductProjectionItem::RunStatus { run_id, status }
            }
            Wire::Gate { gate_ref, headline } => ProductProjectionItem::Gate { gate_ref, headline },
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductProjectionState {
    pub thread_id: String,
    pub items: Vec<ProductProjectionItem>,
}

impl ProductProjectionState {
    pub fn new(
        thread_id: impl Into<String>,
        items: Vec<ProductProjectionItem>,
    ) -> Result<Self, ProductAdapterError> {
        let thread_id = thread_id.into();
        validate_bounded_text(
            "projection_thread_id",
            &thread_id,
            PROJECTION_THREAD_ID_MAX_BYTES,
        )?;
        if items.is_empty() {
            return Err(invalid("projection_items", "must include renderable state"));
        }
        for item in &items {
            item.validate()?;
        }
        Ok(Self { thread_id, items })
    }
}

impl<'de> Deserialize<'de> for ProductProjectionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            thread_id: String,
            items: Vec<ProductProjectionItem>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.thread_id, wire.items).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductOutboundPayload {
    FinalReply(FinalReplyView),
    Progress(ProgressUpdateView),
    GatePrompt(GatePromptView),
    AuthPrompt(AuthPromptView),
    ProjectionSnapshot { state: ProductProjectionState },
    ProjectionUpdate { state: ProductProjectionState },
    KeepAlive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOutboundTarget {
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_actor_ref: Option<ExternalActorRef>,
}

impl ProductOutboundTarget {
    pub fn new(
        reply_target_binding_ref: ReplyTargetBindingRef,
        external_conversation_ref: ExternalConversationRef,
        external_actor_ref: Option<ExternalActorRef>,
    ) -> Self {
        Self {
            reply_target_binding_ref,
            external_conversation_ref,
            external_actor_ref,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductSynchronousResponse {
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductRenderOutcome {
    DeliveryRecorded,
    SynchronousResponse(ProductSynchronousResponse),
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOutboundEnvelope {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub target: ProductOutboundTarget,
    pub projection_cursor: ProjectionCursor,
    pub payload: ProductOutboundPayload,
    pub delivery_attempt_id: Uuid,
}

impl ProductOutboundEnvelope {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        target: ProductOutboundTarget,
        projection_cursor: ProjectionCursor,
        payload: ProductOutboundPayload,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            target,
            projection_cursor,
            payload,
            delivery_attempt_id: Uuid::new_v4(),
        }
    }

    pub fn projection_cursor(&self) -> &ProjectionCursor {
        &self.projection_cursor
    }

    pub fn payload(&self) -> &ProductOutboundPayload {
        &self.payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() {
        let cursor = ProjectionCursor::new("thread:42#cursor:7").expect("valid");
        let json = serde_json::to_string(&cursor).expect("serialize");
        let parsed: ProjectionCursor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cursor, parsed);
    }

    #[test]
    fn cursor_rejects_oversize() {
        assert!(ProjectionCursor::new("a".repeat(PROJECTION_CURSOR_MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn projection_state_requires_renderable_items() {
        assert!(ProductProjectionState::new("thread-1", vec![]).is_err());
    }

    #[test]
    fn final_reply_serializes_with_plaintext() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello world".into(),
            generated_at: Utc::now(),
        };
        let json = serde_json::to_value(&view).expect("serialize");
        assert_eq!(json["text"], "hello world");
    }
}
