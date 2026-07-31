//! Host-layer turn coordination contracts for IronClaw Reborn.
//!
//! `ironclaw_turns` sits above the Reborn kernel service. Product adapters use
//! the adapter-safe [`TurnCoordinator`] API with canonical refs resolved by the
//! binding/session layer. Trusted workers use [`runner`] explicitly; runner
//! transition APIs are intentionally not re-exported from this crate prelude.
#![warn(unreachable_pub)]

mod admission;
mod agent_turn_runtime;
mod checkpoint_state;
mod coordinator;
pub mod events;
mod external_tool_catalog;
mod ids;
pub mod loop_exit;
mod origin;
pub mod process_projection;
pub mod product_adapter;
pub mod product_context;
mod request;
mod response;
pub mod run_profile;
pub mod runner;
pub mod scope;
mod status;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use admission::{
    AllowAllTurnAdmissionLimitProvider, StaticTurnAdmissionLimitProvider, TurnAdmissionAxisKind,
    TurnAdmissionBucket, TurnAdmissionBucketKind, TurnAdmissionBucketScope,
    TurnAdmissionCapacityDenial, TurnAdmissionClass, TurnAdmissionLimit,
    TurnAdmissionLimitProvider, TurnAdmissionLimitUnavailable,
};
pub use agent_turn_runtime::{
    AgentTurnRuntimePort, AgentTurnSpawnTreeRuntimePort, SpawnTreeReservation, TurnRunRecord,
    active_run_ref_state,
};
pub use checkpoint_state::{
    GetLoopCheckpointRequest, LoopCheckpointRecord, LoopCheckpointStore,
    MAX_CHECKPOINT_STATE_PAYLOAD_BYTES, PutLoopCheckpointRequest, RedactedCheckpointPayload,
};
pub use coordinator::{
    AllowAllTurnAdmissionPolicy, DefaultTurnCoordinator, NoopTurnRunWakeNotifier,
    TurnAdmissionPolicy, TurnCoordinator, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    TurnSpawnTreePort,
};
pub use events::{
    EventCursor, InMemoryTurnEventSink, MAX_TURN_EVENT_PROJECTION_LIMIT, TurnBlockedGateKind,
    TurnBlockedGateMetadata, TurnCommittedEventObserver, TurnEventKind, TurnEventPage,
    TurnEventProjectionCursor, TurnEventProjectionError, TurnEventProjectionRequest,
    TurnEventProjectionService, TurnEventProjectionSnapshot, TurnEventProjectionSource,
    TurnEventReducerService, TurnEventReducerSnapshot, TurnEventSink, TurnLifecycleEvent,
    TurnLifecycleProjectionEntry,
};
pub use external_tool_catalog::{
    ExternalToolCatalog, ExternalToolCatalogError, ExternalToolSpec, ExternalToolSpecError,
    InMemoryExternalToolCatalog, PendingExternalCall,
};
pub use ids::{
    AcceptedMessageRef, CapabilityActivityId, GateRef, IdempotencyKey, LoopExitId, LoopGateRef,
    LoopMessageRef, LoopResultRef, ReplyTargetBindingRef, RunProfileId, RunProfileRequest,
    RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunnerId,
};
pub use ironclaw_host_api::turn::{
    ModelInvalidOutputDetailReason, SanitizedCancelReason, SanitizedFailure, TurnOwner,
};
pub use loop_exit::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    FinalCheckpointEvidenceRequest, LoopBlocked, LoopBlockedKind, LoopCancelled,
    LoopCancelledReasonKind, LoopCompleted, LoopCompletionKind, LoopExit, LoopExitApplier,
    LoopExitEvidencePort, LoopExitMapping, LoopExitValidationDecision, LoopExitViolation,
    LoopExitViolationKind, LoopFailed, LoopFailureKind,
};
pub use origin::{ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnSurfaceType};
pub use process_projection::{
    AGENT_TURN_PROCESS_KIND, AgentTurnProcessCommitObserver, AgentTurnProcessMetadata,
    AgentTurnProcessRuntime, AgentTurnProcessStateMetadata, ProcessJournalStoreTurnAdapter,
    ProcessLoopCheckpointStore, TurnEventProjectionFromProcessJournal,
    claimed_turn_run_from_process_claim, turn_run_state_from_process_snapshot,
};
pub use request::{
    CancelRunRequest, GateResumeDisposition, GetRunStateRequest, ResumeTurnPrecondition,
    ResumeTurnRequest, RetryTurnRequest, SubmitChildRunRequest, SubmitTurnRequest, TurnTimestamp,
};
pub use response::{
    CancelRunResponse, ResumeTurnResponse, RetryTurnResponse, SubmitTurnResponse, ThreadBusy,
};
pub use run_profile::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverResumeRequest,
    AgentLoopDriverRunRequest, CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
    CheckpointSchemaId, CommunicationRuntimeContext, ConcurrencyClass, ConnectedChannelSummary,
    ConnectedChannelsState, ContextProfileId, DeliveryTargetState, DeliveryTargetSummary,
    EmptyMemoryPromptContextService, InMemoryRunProfileRegistry, InMemoryRunProfileResolver,
    LoopCheckpointKind, LoopCheckpointStateRef, LoopDriverId, MemoryPromptContextRequest,
    MemoryPromptContextService, ModelProfileId, PrivilegedRunProfileDimension,
    RedactedRunProfileProvenance, RedactedRunProfileSource, ResolvedRunProfile,
    ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
    RunProfileRegistryError, RunProfileRequestAuthority, RunProfileResolutionError,
    RunProfileResolutionRequest, RunProfileResolver, RunProfileSourceLayer, RunProfileSourceRef,
    RunnerPoolId, RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
};
pub use scope::{TurnActor, TurnScope};
pub use status::{
    AdmissionRejection, AdmissionRejectionReason, BlockedReason, GateKind, TurnActiveRunRefState,
    TurnCapacityResource, TurnError, TurnErrorCategory, TurnRunProfile, TurnRunState, TurnStatus,
    is_recoverability_critical,
};
