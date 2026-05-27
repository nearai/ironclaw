//! Agent-loop driver, host-port, prompt-bundle, and run-profile contracts.
//!
//! Prompt bundle APIs are host-managed: drivers request a bounded bundle of
//! context message references from [`LoopPromptPort`] and then pass those refs to
//! the model port. Prompt APIs intentionally move prompt construction out of
//! driver-owned string assembly without exposing raw prompt text in milestones.
//! The initial host-managed implementation supports only [`PromptMode::TextOnly`]
//! and rejects checkpoint-backed prompt state until a durable checkpoint prompt
//! store is introduced.

mod driver;
mod host;
mod instruction_bundle;
mod memory_context;
mod milestones;
mod model;
mod policy;
mod prompt;
mod refs;
mod resolver;
mod skill_context;
mod snapshot;
mod snippet_ref;

pub use crate::CapabilityActivityId;

pub use driver::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverResumeRequest,
    AgentLoopDriverRunRequest,
};
pub use host::{
    AgentLoopDriverHost, AgentLoopHost, AgentLoopHostError, AgentLoopHostErrorKind,
    AppendCapabilityResultRef, AssistantReply, BatchPolicyKind, BeginAssistantDraft,
    CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityDenied,
    CapabilityDeniedReasonKind, CapabilityDeniedReasonKindValue, CapabilityDescriptorView,
    CapabilityFailure, CapabilityFailureKind, CapabilityFailureKindValue, CapabilityInputRef,
    CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, CapabilitySurfaceVersion,
    ConcurrencyHint, FinalizeAssistantMessage, LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES,
    LOOP_CONTEXT_TOTAL_MODEL_CONTENT_MAX_BYTES, LoadCheckpointPayloadRequest,
    LoadedCheckpointPayload, LoopCancelReasonKind, LoopCancellationPort, LoopCancellationSignal,
    LoopCapabilityPort, LoopCheckpointKind, LoopCheckpointPort, LoopCheckpointRequest,
    LoopCheckpointStateRef, LoopContextBundle, LoopContextMessage, LoopContextPort,
    LoopContextRequest, LoopContextSnippet, LoopContextSnippetMetadata, LoopDriverNoteKind,
    LoopGateKind, LoopInlineMessage, LoopInlineMessageRole, LoopInput, LoopInputAck,
    LoopInputAckToken, LoopInputBatch, LoopInputCursor, LoopInputCursorToken, LoopInputPort,
    LoopInterruptKind, LoopModelCapabilityView, LoopModelMessage, LoopModelPort, LoopModelRequest,
    LoopModelResponse, LoopModelRouteSnapshot, LoopProcessRef, LoopProgressEvent, LoopProgressPort,
    LoopPromptBundle, LoopPromptBundleAuthority, LoopPromptBundleGrant, LoopPromptBundleRef,
    LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopRunInfoPort, LoopSafeSummary,
    LoopTranscriptPort, ModelStreamChunk, ParentLoopOutput, ProcessHandleSummary, PromptMode,
    ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolCallReference,
    ProviderToolCallReplay, ProviderToolDefinition, StageCheckpointPayloadRequest,
    UpdateAssistantDraft, VisibleCapabilityRequest, VisibleCapabilitySurface,
    sanitize_model_visible_text, validate_model_route_component_value,
};
pub use instruction_bundle::{
    InMemoryInstructionMaterializationStore, InstructionBundle, InstructionBundleBuilder,
    InstructionBundleFingerprint, InstructionBundleMaterializedMessage, InstructionBundleRequest,
    InstructionMaterializationStore, InstructionSafetyContext,
    sort_instruction_snippets_for_prompt,
};
pub use memory_context::{
    EmptyMemoryPromptContextService, MemoryPromptContextRequest, MemoryPromptContextService,
};
pub use milestones::{
    HookDecisionSummary, HookMilestoneSink, InMemoryHookMilestoneSink,
    InMemoryLoopHostMilestoneSink, LoopHostMilestone, LoopHostMilestoneEmitter,
    LoopHostMilestoneKind, LoopHostMilestoneSink, PromptSkillContextMetadata,
    RunScopedHookMilestoneSink,
};
pub use model::{
    HostManagedLoopModelPort, LoopModelBudgetAccountant, LoopModelGateway, LoopModelGatewayError,
    LoopModelGatewayRequest, LoopModelPolicyGuard, ModelCallOutcome, NoOpBudgetAccountant,
    NoOpPolicyGuard,
};
pub use policy::{
    CancellationPolicy, CheckpointPolicy, PersonalContextAuthority, PrivilegedRunProfileDimension,
    RedactedRunProfileProvenance, RedactedRunProfileSource, ResourceBudgetPolicy,
    RunProfileRequestAuthority, RunProfileResolutionError, RuntimeProfileConstraints,
    SteeringPolicy,
};
pub use prompt::HostManagedLoopPromptPort;
pub use refs::{
    CapabilitySurfaceProfileId, CheckpointSchemaId, ConcurrencyClass, ContextProfileId,
    LoopDriverId, ModelProfileId, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
    RunProfileSourceLayer, RunProfileSourceRef, RunnerPoolId, SchedulingClass,
};
pub use resolver::{
    InMemoryRunProfileRegistry, InMemoryRunProfileResolver, RunProfileDefinition,
    RunProfileRegistryError, RunProfileResolutionRequest, RunProfileResolver,
};
pub use skill_context::{
    InstalledSkillSnapshot, NoopSkillContextSource, SkillContextBudget, SkillContextError,
    SkillContextService, SkillContextSnippet, SkillContextSource, SkillRunSnapshot,
    SkillTrustLevel, SkillVisibility, is_skill_snippet_model_message_ref,
    skill_snippet_model_message_ref,
};
pub use snapshot::{PersonalContextPolicy, ResolvedRunProfile};
pub use snippet_ref::memory_snippet_display_ref;
