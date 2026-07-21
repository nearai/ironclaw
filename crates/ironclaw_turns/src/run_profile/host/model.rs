//! Model request/response DTOs, the host-managed prompt bundle contracts and
//! authority, and the [`LoopModelPort`]/[`LoopPromptPort`] traits.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use serde::{Deserialize, Serialize};

use crate::run_profile::instruction_bundle::InstructionBundleFingerprint;
use crate::run_profile::refs::ModelProfileId;
use crate::{CapabilityActivityId, LoopMessageRef};

use super::capability::ProviderToolCallReplay;
use super::context::{LoopContextCompactionMetadata, LoopInputCursor};
use super::error::{AgentLoopHostError, AgentLoopHostErrorKind};
use super::refs::{
    CapabilityInputRef, CapabilitySurfaceVersion, LoopCheckpointStateRef, LoopInlineMessageBody,
    LoopPromptBundleRef,
};
use super::run_context::LoopRunContext;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelCapabilityView {
    /// Final capability IDs visible to this model call after the loop driver has
    /// applied its strategy to the host-owned capability surface.
    pub visible_capability_ids: Vec<CapabilityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelRequest {
    pub messages: Vec<LoopModelMessage>,
    #[serde(default)]
    pub inline_messages: Vec<LoopInlineMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub model_preference: Option<ModelProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<LoopModelCapabilityView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelMessage {
    pub role: String,
    pub content_ref: LoopMessageRef,
}

/// Prompt construction mode requested by an agent-loop driver.
///
/// `TextOnly` builds a prompt from transcript/context message refs and is the
/// only mode supported by [`crate::run_profile::HostManagedLoopPromptPort`]
/// today. `CodeAct` is reserved for a future checkpoint/tool-aware prompt
/// bundle flow and is rejected by the text-only host port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    TextOnly,
    #[serde(rename = "codeact")]
    CodeAct,
}

impl PromptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextOnly => "text_only",
            Self::CodeAct => "codeact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInlineMessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInlineMessage {
    pub role: LoopInlineMessageRole,
    pub safe_body: LoopInlineMessageBody,
}

/// Request for a host-managed prompt bundle.
///
/// The optional cursor and checkpoint refs are run-scoped and are validated by
/// host ports before context is loaded. `max_messages` is a host budget hint;
/// zero is accepted only for inline-only context-free prompts, and oversized
/// values may be clamped by the implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPromptBundleRequest {
    pub mode: PromptMode,
    pub context_cursor: Option<LoopInputCursor>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<LoopModelCapabilityView>,
    pub checkpoint_state_ref: Option<LoopCheckpointStateRef>,
    pub max_messages: Option<u32>,
    #[serde(default)]
    pub inline_messages: Vec<LoopInlineMessage>,
}

/// Prompt bundle returned to a driver.
///
/// The bundle carries model-message references rather than raw prompt text.
/// Drivers pass these refs to [`LoopModelPort`], allowing the host to resolve
/// content under the same run scope and policy checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPromptBundle {
    pub bundle_ref: LoopPromptBundleRef,
    pub messages: Vec<LoopModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compaction_message_index: Vec<LoopContextCompactionMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_fingerprint: Option<InstructionBundleFingerprint>,
    #[serde(default)]
    pub identity_message_count: u32,
    #[serde(default)]
    pub instruction_snippet_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopPromptBundleGrant {
    pub bundle_ref: LoopPromptBundleRef,
    pub messages: Vec<LoopModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub instruction_fingerprint: Option<InstructionBundleFingerprint>,
}

#[derive(Clone, Default)]
pub struct LoopPromptBundleAuthority {
    inner: Arc<Mutex<LoopPromptBundleAuthorityState>>,
}

#[derive(Default)]
struct LoopPromptBundleAuthorityState {
    latest_by_run: HashMap<String, LoopPromptBundleGrant>,
}

impl LoopPromptBundleAuthority {
    pub fn shared() -> Self {
        static AUTHORITY: OnceLock<LoopPromptBundleAuthority> = OnceLock::new();
        AUTHORITY.get_or_init(Self::default).clone()
    }

    pub fn issue_bundle(
        &self,
        context: &LoopRunContext,
        bundle: &LoopPromptBundle,
    ) -> Result<(), AgentLoopHostError> {
        if !bundle.bundle_ref.is_for_run(context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "prompt bundle ref is not scoped to this loop run",
            ));
        }

        self.lock_state()?.latest_by_run.insert(
            context.run_id.to_string(),
            LoopPromptBundleGrant {
                bundle_ref: bundle.bundle_ref.clone(),
                messages: bundle.messages.clone(),
                surface_version: bundle.surface_version.clone(),
                instruction_fingerprint: bundle.instruction_fingerprint.clone(),
            },
        );
        Ok(())
    }

    pub fn authorize_latest_model_request(
        &self,
        context: &LoopRunContext,
        messages: &[LoopModelMessage],
        surface_version: &Option<CapabilitySurfaceVersion>,
    ) -> Result<LoopPromptBundleGrant, AgentLoopHostError> {
        let grant = self
            .lock_state()?
            .latest_by_run
            .remove(&context.run_id.to_string())
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "model request has no host-built prompt bundle",
                )
            })?;

        if !grant.bundle_ref.is_for_run(context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "prompt bundle ref is not scoped to this loop run",
            ));
        }
        if grant.messages != messages {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "model request messages do not match the host-built prompt bundle",
            ));
        }
        if &grant.surface_version != surface_version {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "model request surface version does not match the host-built prompt bundle",
            ));
        }

        Ok(grant)
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, LoopPromptBundleAuthorityState>, AgentLoopHostError> {
        self.inner.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "prompt bundle authority is unavailable",
            )
        })
    }
}

/// Host boundary for building prompt bundles before model invocation.
///
/// Implementations own context loading, scoping, prompt-shape policy, and
/// milestone emission. Drivers should not assemble raw prompt strings when a
/// prompt port is available.
#[async_trait]
pub trait LoopPromptPort: Send + Sync {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelResponse {
    pub chunks: Vec<ModelStreamChunk>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_reasoning_deltas: Vec<String>,
    pub output: ParentLoopOutput,
    pub effective_model_profile_id: ModelProfileId,
    /// Provider-reported token usage for this call. `None` when the gateway
    /// could not surface real numbers (replay test stubs, providers without
    /// a usage object); downstream budget accounting falls back to the
    /// reservation estimate in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
}

/// Token usage reported by a provider for a single model call. The accountant
/// uses this to record actual USD spend instead of the conservative
/// reservation estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens read from the provider's server-side prompt cache (e.g. Anthropic
    /// cache reads). A subset of `input_tokens`, billed at a discount. Zero when
    /// caching is unsupported or on a cache miss.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cache_read_input_tokens: u32,
    /// Tokens written to the provider's server-side prompt cache. Zero when
    /// caching is unsupported or no new prefix was cached.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub cache_creation_input_tokens: u32,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

impl LoopModelUsage {
    /// Accumulate another call's usage into this running per-run total.
    pub fn add_assign(&mut self, other: &LoopModelUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_input_tokens = self
            .cache_read_input_tokens
            .saturating_add(other.cache_read_input_tokens);
        self.cache_creation_input_tokens = self
            .cache_creation_input_tokens
            .saturating_add(other.cache_creation_input_tokens);
    }

    /// Total billable tokens (input + output). Cache tokens are already counted
    /// within `input_tokens` by every provider that reports them, so they are
    /// not added again here.
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStreamChunk {
    pub safe_text_delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentLoopOutput {
    AssistantReply(AssistantReply),
    CapabilityCalls(Vec<CapabilityCallCandidate>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantReply {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCallCandidate {
    /// Stable activity identity assigned before capability dispatch. Hosts use
    /// this as the runtime invocation identity, and tokenless gate checkpoints
    /// persist it so terminal events can close the same activity.
    pub activity_id: CapabilityActivityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub capability_id: CapabilityId,
    pub input_ref: CapabilityInputRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effective_capability_ids: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderToolCallReplay>,
}

#[async_trait]
pub trait LoopModelPort: Send + Sync {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError>;
}
