//! `AgentLoopDriverHost` and `LoopXxxPort` host-boundary contracts plus the
//! neutral DTOs passed over those ports.
//!
//! Split by contract cluster into focused submodules; every public name is
//! re-exported here so the crate's `run_profile::X` public paths are unchanged.

mod capability;
mod checkpoint;
mod context;
mod error;
mod input;
mod model;
mod progress;
mod refs;
mod run_context;
mod transcript;
mod validate;

pub use capability::{
    AuthResumeApprovalIdentity, CapabilityApprovalResume, CapabilityAuthResume,
    CapabilityBatchInvocation, CapabilityDeniedReasonKind, CapabilityDeniedReasonKindValue,
    CapabilityDescriptorView, CapabilityFailure, CapabilityFailureKind, CapabilityFailureKindValue,
    CapabilityInvocation, CapabilityProgress, CapabilityResultMessage, ConcurrencyHint,
    LoopCapabilityPort, ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolCallReference,
    ProviderToolCallReplay, ProviderToolDefinition, RegisterProviderToolCallRequest,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
pub use checkpoint::{
    LoadCheckpointPayloadRequest, LoadedCheckpointPayload, LoopCheckpointKind, LoopCheckpointPort,
    LoopCheckpointRequest, StageCheckpointPayloadRequest,
};
pub use context::{
    LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES, LOOP_CONTEXT_TOTAL_MODEL_CONTENT_MAX_BYTES,
    LoopContextBundle, LoopContextCompactionKind, LoopContextCompactionMetadata,
    LoopContextMessage, LoopContextPort, LoopContextRequest, LoopContextSnippet,
    LoopContextSnippetMetadata, LoopInputCursor,
};
pub use error::{AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind};
pub use input::{
    LoopCancelReasonKind, LoopInput, LoopInputAck, LoopInputBatch, LoopInputPort, LoopInterruptKind,
};
pub use model::{
    AssistantReply, CapabilityCallCandidate, LoopInlineMessage, LoopInlineMessageRole,
    LoopModelCapabilityView, LoopModelMessage, LoopModelPort, LoopModelRequest, LoopModelResponse,
    LoopModelUsage, LoopPromptBundle, LoopPromptBundleAuthority, LoopPromptBundleGrant,
    LoopPromptBundleRequest, LoopPromptPort, ModelStreamChunk, ParentLoopOutput, PromptMode,
};
pub use progress::{
    AgentLoopDriverHost, BatchPolicyKind, LoopCancellationPort, LoopCancellationSignal,
    LoopDriverNoteKind, LoopGateKind, LoopProgressEvent, LoopProgressPort,
};
pub use refs::{
    CapabilityInputRef, CapabilityResumeToken, CapabilitySurfaceVersion, LoopCheckpointStateRef,
    LoopInlineMessageBody, LoopInputAckToken, LoopInputCursorToken, LoopProcessRef,
    LoopPromptBundleRef, LoopSafeSummary,
};
pub use run_context::{LoopModelRouteSnapshot, LoopRunContext, LoopRunInfoPort};
pub use transcript::{
    AppendCapabilityResultRef, BeginAssistantDraft, FinalizeAssistantMessage, LoopTranscriptPort,
    UpdateAssistantDraft,
};
pub use validate::{sanitize_model_visible_text, validate_model_route_component_value};
