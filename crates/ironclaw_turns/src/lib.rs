//! Host-layer turn coordination contracts for IronClaw Reborn.
//!
//! `ironclaw_turns` sits above the Reborn kernel facade. Product adapters use
//! the adapter-safe [`TurnCoordinator`] API with canonical refs resolved by the
//! binding/session layer. Trusted workers use [`runner`] explicitly; runner
//! transition APIs are intentionally not re-exported from this crate prelude.
#![warn(unreachable_pub)]

mod admission;
mod block_persistence;
mod checkpoint_state;
mod coordinator;
pub mod events;
mod external_tool_catalog;
mod filesystem_store;
mod ids;
mod lifecycle;
pub mod loop_exit;
mod origin;
mod request;
mod response;
pub mod run_profile;
pub mod runner;
pub mod scope;
mod status;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use admission::{
    AllowAllTurnAdmissionLimitProvider, StaticTurnAdmissionLimitProvider, TurnAdmissionAxisKind,
    TurnAdmissionBucket, TurnAdmissionBucketKind, TurnAdmissionBucketScope,
    TurnAdmissionCapacityDenial, TurnAdmissionClass, TurnAdmissionLimit,
    TurnAdmissionLimitProvider, TurnAdmissionLimitUnavailable, TurnAdmissionReservationRecord,
};
pub use block_persistence::TurnStateBlockPersistence;
pub use checkpoint_state::{
    CheckpointStateMatchMetadata, CheckpointStateRecord, CheckpointStateStore,
    GetCheckpointStateRequest, GetLoopCheckpointRequest, LoopCheckpointRecord, LoopCheckpointStore,
    MAX_CHECKPOINT_STATE_PAYLOAD_BYTES, PutCheckpointStateRequest, PutLoopCheckpointRequest,
    RedactedCheckpointPayload, checkpoint_state_metadata_matches_request, new_checkpoint_state_ref,
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
pub use filesystem_store::{
    FilesystemTurnStateBlockPersistence, FilesystemTurnStateRowStore, FilesystemTurnStateStore,
    FilesystemTurnStateStoreKind, TurnStateStoreLimits,
};
pub use ids::{
    AcceptedMessageRef, CapabilityActivityId, GateRef, IdempotencyKey, LoopDiagnosticRef,
    LoopExitId, LoopGateRef, LoopMessageRef, LoopResultRef, ReplyTargetBindingRef, RunProfileId,
    RunProfileRequest, RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunnerId,
};
pub use lifecycle::{
    DefaultTurnLifecycleEventBus, LifecyclePublicationErrorPort, LifecyclePublishingTurnStateStore,
    NoopLifecyclePublicationErrorPort, TurnLifecycleEventBus,
};
pub use loop_exit::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    FinalCheckpointEvidenceRequest, LoopBlocked, LoopBlockedKind, LoopCancelled,
    LoopCancelledReasonKind, LoopCompleted, LoopCompletionKind, LoopExit, LoopExitApplier,
    LoopExitEvidencePort, LoopExitMapping, LoopExitValidationDecision, LoopExitViolation,
    LoopExitViolationKind, LoopFailed, LoopFailureKind,
};
pub use origin::{
    ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnOwner, TurnSurfaceType,
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
    AdmissionRejection, AdmissionRejectionReason, BlockedReason, ModelInvalidOutputDetailReason,
    SanitizedCancelReason, SanitizedFailure, TurnActiveRunRefState, TurnCapacityResource,
    TurnError, TurnErrorCategory, TurnRunProfile, TurnRunState, TurnStatus,
    is_recoverability_critical,
};
pub use store::{
    SpawnTreeReservation, SpawnTreeReservationKey, TurnActiveLockKey, TurnActiveLockRecord,
    TurnCheckpointRecord, TurnIdempotencyErrorReplay, TurnIdempotencyOperationKind,
    TurnIdempotencyOutcomeKind, TurnIdempotencyRecord, TurnIdempotencyReplay, TurnLockVersion,
    TurnPersistenceSnapshot, TurnRecord, TurnRunRecord, TurnSpawnTreeStateStore, TurnStateStore,
    active_run_ref_state,
};
