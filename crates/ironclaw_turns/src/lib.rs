//! Host-layer turn coordination contracts for IronClaw Reborn.
//!
//! `ironclaw_turns` sits above the Reborn kernel facade. Product adapters use
//! the adapter-safe [`TurnCoordinator`] API with canonical refs resolved by the
//! binding/session layer. Trusted workers use [`runner`] explicitly; runner
//! transition APIs are intentionally not re-exported from this crate prelude.
#![warn(unreachable_pub)]

mod admission;
mod checkpoint_state;
mod coordinator;
pub mod events;
mod filesystem_store;
mod ids;
pub mod loop_exit;
mod memory;
mod request;
mod response;
pub mod run_profile;
pub mod runner;
pub mod scope;
mod status;
mod store;

pub use admission::{
    AllowAllTurnAdmissionLimitProvider, StaticTurnAdmissionLimitProvider, TurnAdmissionAxisKind,
    TurnAdmissionBucket, TurnAdmissionBucketKind, TurnAdmissionBucketScope,
    TurnAdmissionCapacityDenial, TurnAdmissionClass, TurnAdmissionLimit,
    TurnAdmissionLimitProvider, TurnAdmissionLimitUnavailable, TurnAdmissionReservationRecord,
};
pub use checkpoint_state::{
    CheckpointStateRecord, CheckpointStateStore, GetCheckpointStateRequest,
    GetLoopCheckpointRequest, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    LoopCheckpointRecord, LoopCheckpointStore, MAX_CHECKPOINT_STATE_PAYLOAD_BYTES,
    PutCheckpointStateRequest, PutLoopCheckpointRequest, RedactedCheckpointPayload,
};
pub use coordinator::{
    AllowAllTurnAdmissionPolicy, DefaultTurnCoordinator, NoopTurnRunWakeNotifier,
    TurnAdmissionPolicy, TurnCoordinator, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
};
pub use events::{
    EventCursor, InMemoryTurnEventSink, TurnEventKind, TurnEventPage, TurnEventProjectionCursor,
    TurnEventProjectionError, TurnEventProjectionRequest, TurnEventProjectionService,
    TurnEventProjectionSnapshot, TurnEventProjectionSource, TurnEventSink, TurnLifecycleEvent,
};
pub use filesystem_store::FilesystemTurnStateStore;
pub use ids::{
    AcceptedMessageRef, GateRef, IdempotencyKey, LoopDiagnosticRef, LoopExitId, LoopGateRef,
    LoopMessageRef, LoopResultRef, LoopUsageSummaryRef, ReplyTargetBindingRef, RunProfileId,
    RunProfileRequest, RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunnerId,
};
pub use loop_exit::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    FinalCheckpointEvidenceRequest, LoopBlocked, LoopBlockedKind, LoopCancelled,
    LoopCancelledReasonKind, LoopCompleted, LoopCompletionKind, LoopExit, LoopExitApplier,
    LoopExitEvidencePort, LoopExitInvalidHandling, LoopExitMapping, LoopExitValidationDecision,
    LoopExitViolation, LoopExitViolationKind, LoopFailed, LoopFailureKind,
};
pub use memory::{InMemoryTurnStateStore, InMemoryTurnStateStoreLimits};
pub use request::{
    CancelRunRequest, GetRunStateRequest, ResumeTurnPrecondition, ResumeTurnRequest,
    SubmitTurnRequest, TurnTimestamp,
};
pub use response::{CancelRunResponse, ResumeTurnResponse, SubmitTurnResponse, ThreadBusy};
pub use run_profile::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverResumeRequest,
    AgentLoopDriverRunRequest, CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
    CheckpointSchemaId, ConcurrencyClass, ContextProfileId, EmptyMemoryPromptContextService,
    InMemoryRunProfileRegistry, InMemoryRunProfileResolver, LoopCheckpointKind,
    LoopCheckpointStateRef, LoopDriverId, MemoryPromptContextRequest, MemoryPromptContextService,
    ModelProfileId, PrivilegedRunProfileDimension, RedactedRunProfileProvenance,
    RedactedRunProfileSource, ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier,
    RunClassId, RunProfileFingerprint, RunProfileRegistryError, RunProfileRequestAuthority,
    RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver,
    RunProfileSourceLayer, RunProfileSourceRef, RunnerPoolId, RuntimeProfileConstraints,
    SchedulingClass, SteeringPolicy,
};
pub use scope::{TurnActor, TurnScope};
pub use status::{
    AdmissionRejection, AdmissionRejectionReason, BlockedReason, SanitizedCancelReason,
    SanitizedFailure, TurnError, TurnErrorCategory, TurnRunProfile, TurnRunState, TurnStatus,
};
pub use store::{
    TurnActiveLockKey, TurnActiveLockRecord, TurnCheckpointRecord, TurnIdempotencyErrorReplay,
    TurnIdempotencyOperationKind, TurnIdempotencyOutcomeKind, TurnIdempotencyRecord,
    TurnIdempotencyReplay, TurnLockVersion, TurnPersistenceSnapshot, TurnRecord, TurnRunRecord,
    TurnStateStore,
};
