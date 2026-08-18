//! Assistant-draft and capability-result transcript DTOs plus the
//! [`LoopTranscriptPort`] host boundary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::model_observation::ModelVisibleToolObservation;
use ironclaw_host_api::turn::{LoopMessageRef, LoopResultRef};

use super::capability::ProviderToolCallReference;
use super::error::{AgentLoopHostError, unsupported_host_method};
use super::model::AssistantReply;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeginAssistantDraft {
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAssistantDraft {
    pub message_ref: LoopMessageRef,
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizeAssistantMessage {
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendCapabilityResultRef {
    pub result_ref: LoopResultRef,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_call: Option<ProviderToolCallReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_observation: Option<ModelVisibleToolObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intrinsic_outcome: Option<CapabilityResultIntrinsicOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityResultIntrinsicOutcome {
    NothingToReport,
}

#[async_trait]
pub trait LoopTranscriptPort: Send + Sync {
    async fn begin_assistant_draft(
        &self,
        _request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported_host_method("begin_assistant_draft"))
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        Err(unsupported_host_method("update_assistant_draft"))
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError>;

    async fn append_capability_result_ref(
        &self,
        _request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported_host_method("append_capability_result_ref"))
    }
}
