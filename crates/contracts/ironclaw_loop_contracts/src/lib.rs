//! The loop-tier contract: how any loop, hook, or host adapter talks to the
//! turn kernel without importing it.
//!
//! This crate is vocabulary and ports only — traits and DTOs. It executes
//! nothing, persists nothing, and decides nothing. Every `Loop*Port` declared
//! here is implemented above it, in the loop-hosting tier
//! (`ironclaw_loop_host`, `ironclaw_turn_runner`, `ironclaw_hooks`); the turn
//! kernel (`ironclaw_turns`) implements the kernel side and *validates*
//! against these contracts. The direction never inverts: nothing here may
//! depend on the turn kernel.
//!
//! `ironclaw_agent_loop`'s rule that it depends on contracts-layer crates and
//! nothing else is satisfiable entirely through this crate — that is the point
//! of it existing. See `docs/reborn/target-architecture/PROPOSAL.md` §6.1.4 and
//! `families/contracts.md`.
//!
//! Prompt bundle APIs are host-managed: drivers request a bounded bundle of
//! context message references from [`LoopPromptPort`] and then pass those refs to
//! the model port. Prompt APIs intentionally move prompt construction out of
//! driver-owned string assembly without exposing raw prompt text in milestones.
//! The initial host-managed implementation supports only [`PromptMode::TextOnly`]
//! and rejects checkpoint-backed prompt state until a durable checkpoint prompt
//! store is introduced.
#![warn(unreachable_pub)]

mod checkpoint_payload;
mod compaction;
mod content_digest;
mod context_budget;
mod driver;
mod host;
mod instruction_bundle;
mod loop_exit;
mod memory_context;
mod milestones;
mod model;
mod model_observation;
mod model_work;
mod policy;
mod prompt_text;
mod refs;
pub mod resolution;
mod resolver;
mod runtime_context;
mod skill_context;
mod snapshot;
mod snippet_ref;
mod system_inference;

pub use checkpoint_payload::{MAX_CHECKPOINT_STATE_PAYLOAD_BYTES, RedactedCheckpointPayload};
pub use compaction::{
    CompactionInitiator, LoopCompactionError, LoopCompactionMode, LoopCompactionOutcome,
    LoopCompactionPort, LoopCompactionRequest, LoopCompactionResponse, LoopSummaryArtifactId,
};
pub use content_digest::{ContentDigest, ContentDigestError, normalize_for_hash};
pub use context_budget::PromptContextTokenBudget;
pub use driver::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverResumeRequest,
    AgentLoopDriverRunRequest,
};
pub use host::{
    AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind,
    AppendCapabilityResultRef, AssistantReply, AuthResumeApprovalIdentity, BatchPolicyKind,
    BeginAssistantDraft, CapabilityApprovalResume, CapabilityAuthResume, CapabilityCallCandidate,
    CapabilityDeniedReasonKind, CapabilityDeniedReasonKindValue, CapabilityDescriptionTrust,
    CapabilityDescriptorView, CapabilityFailure, CapabilityInputRef, CapabilityProgress,
    CapabilityResultMessage, CapabilityResumeToken, CapabilitySurfaceVersion, ConcurrencyHint,
    FinalizeAssistantMessage, LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES,
    LOOP_CONTEXT_TOTAL_MODEL_CONTENT_MAX_BYTES, LoadCheckpointPayloadRequest,
    LoadedCheckpointPayload, LoopCancelReasonKind, LoopCancellationPort, LoopCancellationSignal,
    LoopCapabilityPort, LoopCheckpointKind, LoopCheckpointPort, LoopCheckpointRequest,
    LoopCheckpointStateRef, LoopContextBundle, LoopContextCompactionKind,
    LoopContextCompactionMetadata, LoopContextMessage, LoopContextPort, LoopContextRequest,
    LoopContextSnippet, LoopContextSnippetMetadata, LoopDriverNoteKind, LoopGateKind,
    LoopInlineMessage, LoopInlineMessageBody, LoopInlineMessageRole, LoopInput, LoopInputAck,
    LoopInputAckToken, LoopInputBatch, LoopInputCursor, LoopInputCursorToken, LoopInputPort,
    LoopInterruptKind, LoopModelCapabilityView, LoopModelMessage, LoopModelPort, LoopModelRequest,
    LoopModelResponse, LoopModelRouteSnapshot, LoopModelUsage, LoopProcessRef, LoopProgressEvent,
    LoopProgressPort, LoopPromptBundle, LoopPromptBundleAuthority, LoopPromptBundleGrant,
    LoopPromptBundleRef, LoopPromptBundleRequest, LoopPromptDiagnosticMetadata, LoopPromptPort,
    LoopRecoveryClass, LoopRecoveryDisposition, LoopRecoveryStage, LoopRequest, LoopRequestBatch,
    LoopRunContext, LoopRunInfoPort, LoopSafeSummary, LoopTranscriptPort, ModelStreamChunk,
    ParentLoopOutput, PromptMode, ProviderToolCall, ProviderToolCallCapabilityIds,
    ProviderToolCallReference, ProviderToolCallReplay, ProviderToolDefinition,
    RegisterProviderToolCallRequest, StageCheckpointPayloadRequest, UpdateAssistantDraft,
    VisibleCapabilityRequest, VisibleCapabilitySurface, sanitize_model_visible_text,
    validate_model_route_component_value,
};
pub use instruction_bundle::{
    EphemeralInstructionMaterializationStore, InstructionBundle, InstructionBundleBuilder,
    InstructionBundleFingerprint, InstructionBundleMaterializedMessage, InstructionBundleRequest,
    InstructionMaterializationStore, InstructionSafetyContext,
    sort_instruction_snippets_for_prompt,
};
pub use loop_exit::{
    LoopBlocked, LoopBlockedKind, LoopCancelled, LoopCancelledReasonKind, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopFailed, LoopFailureKind,
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
    LoopModelBudgetAccountant, LoopModelGateway, LoopModelGatewayError, LoopModelGatewayRequest,
    LoopModelPolicyGuard, LoopModelProgressSink, ModelCallOutcome, NoOpBudgetAccountant,
    NoOpPolicyGuard,
};
pub use model_observation::{
    CapabilityFailureDetail, CapabilityInputIssue, CapabilityInputRepair,
    MODEL_OBSERVATION_DETAIL_MAX_BYTES, MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
    ModelVisibleArtifact, ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail,
    ToolObservationStatus, ToolRecoveryObservation, validate_model_observation_detail,
};
pub use model_work::{ModelWorkKind, ModelWorkOutcome, ModelWorkRequest, ModelWorkUsage};
pub use policy::{
    CancellationPolicy, CheckpointPolicy, PersonalContextAuthority, PrivilegedRunProfileDimension,
    RedactedRunProfileProvenance, RedactedRunProfileSource, ResourceBudgetPolicy,
    RunProfileRequestAuthority, RunProfileResolutionError, RuntimeProfileConstraints,
    SteeringPolicy,
};
pub use refs::{
    CapabilitySurfaceProfileId, CheckpointSchemaId, ConcurrencyClass, ContextProfileId,
    LoopDriverId, ModelProfileId, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
    RunProfileSourceLayer, RunProfileSourceRef, RunnerPoolId, SchedulingClass,
};
pub use resolution::{DeniedResolution, GatedResolution};
pub use resolver::{
    InMemoryRunProfileRegistry, InMemoryRunProfileResolver, RunProfileDefinition,
    RunProfileRegistryError, RunProfileResolutionRequest, RunProfileResolver,
};
pub use runtime_context::{
    CommunicationContextFetch, CommunicationContextProvider, CommunicationRuntimeContext,
    ConnectedChannelSummary, ConnectedChannelsState, DeliveryTargetState, DeliveryTargetSummary,
    Locale, LocaleError, LoopRuntimeContext, UserProfileContext,
};
pub use skill_context::{
    InstalledSkillSnapshot, NoopSkillContextSource, SkillActivationState, SkillContextBudget,
    SkillContextError, SkillContextService, SkillContextSnippet, SkillContextSource,
    SkillRunSnapshot, SkillTrustLevel, SkillVisibility, is_skill_snippet_model_message_ref,
    skill_snippet_model_message_ref,
};
pub use snapshot::{PersonalContextPolicy, ResolvedRunProfile};
pub use snippet_ref::memory_snippet_display_ref;
pub use system_inference::{
    SystemInferenceError, SystemInferenceIdentity, SystemInferencePort, SystemInferenceRequest,
    SystemInferenceResponse, SystemInferenceTaskId, SystemPromptId, SystemPromptSource,
    SystemTaskKind,
};
