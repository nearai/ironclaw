//! Host-layer turn coordination contracts for IronClaw Reborn.
//!
//! `ironclaw_turns` sits above the Reborn kernel service. Product adapters use
//! the adapter-safe [`TurnCoordinator`] API with canonical refs resolved by the
//! binding/session layer. Trusted workers use [`runner`] explicitly; runner
//! transition APIs are intentionally not re-exported from this crate prelude.
#![warn(unreachable_pub)]

mod activation_streak;
mod admission;
mod agent_turn_runtime;
mod checkpoint_state;
mod coordinator;
pub mod events;
mod external_tool_catalog;
pub mod host_managed_ports;
pub mod loop_exit;
pub mod process_projection;
pub mod product_context;
mod request;
mod response;
pub mod runner;
mod status;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use activation_streak::{
    SYSTEM_WAKE_STREAK_CAP, SYSTEM_WAKE_WINDOW_OVERFETCH, system_wake_admitted,
};
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
    GetLoopCheckpointRequest, LoopCheckpointRecord, LoopCheckpointStore, PutLoopCheckpointRequest,
};
pub use coordinator::{
    AllowAllTurnAdmissionPolicy, DefaultTurnCoordinator, NoopTurnRunWakeNotifier,
    TurnAdmissionPolicy, TurnCoordinator, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    TurnSpawnTreePort,
};
pub use events::{
    InMemoryTurnEventSink, MAX_TURN_EVENT_PROJECTION_LIMIT, TurnBlockedGateKind,
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
// The turn vocabulary itself is `ironclaw_host_api::turn`'s, not this crate's:
// ids, refs, scope/actor/owner, status and its gate correspondence, the event
// cursor, the run-origin adapter identity, and the sanitized failure/cancel
// shapes. This crate names them because it coordinates turns; it re-exports
// them only so its own consumers keep one import for the kernel API they
// already depend on. A crate that needs *only* vocabulary must depend on
// `ironclaw_host_api` directly — see this crate's CLAUDE.md.
pub use host_managed_ports::{HostManagedLoopModelPort, HostManagedLoopPromptPort};
pub use ironclaw_host_api::turn::{
    AcceptedMessageRef, ActivationProvenance, BlockedReason, CapabilityActivityId, EventCursor,
    GateKind, GateResumeDisposition, IdempotencyKey, LoopExitId, LoopGateRef, LoopMessageRef,
    LoopResultRef, ModelInvalidOutputDetailReason, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileId, RunProfileRequest, RunProfileVersion, SanitizedCancelReason,
    SanitizedFailure, SourceBindingRef, SubmitTurnResponse, TurnActor, TurnCheckpointId,
    TurnExecutionOutcome, TurnGateRef, TurnId, TurnLeaseToken, TurnOriginKind, TurnOwner,
    TurnRunId, TurnRunnerId, TurnScope, TurnStatus, TurnSurfaceType,
};
pub use loop_exit::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    FinalCheckpointEvidenceRequest, LoopExitApplier, LoopExitEvidencePort, LoopExitMapping,
    LoopExitValidationDecision, LoopExitViolation, LoopExitViolationKind,
};
pub use process_projection::{
    AGENT_TURN_PROCESS_KIND, AgentTurnProcessCommitObserver, AgentTurnProcessRuntime,
    AgentTurnProcessStateMetadata, ProcessJournalStoreTurnAdapter, ProcessLoopCheckpointStore,
    TurnEventProjectionFromProcessJournal, claimed_turn_run_from_process_claim,
    turn_run_state_from_process_snapshot,
};
pub use request::{
    ActivateThreadRequest, CancelRunRequest, GetRunStateRequest, ResumeTurnPrecondition,
    ResumeTurnRequest, RetryTurnRequest, SubmitChildRunRequest, SubmitTurnRequest, TurnTimestamp,
};
pub use response::{CancelRunResponse, ResumeTurnResponse, RetryTurnResponse, ThreadBusy};
pub use status::{
    AdmissionRejection, AdmissionRejectionReason, TurnActiveRunRefState, TurnCapacityResource,
    TurnError, TurnErrorCategory, TurnRunProfile, TurnRunState, is_recoverability_critical,
};
