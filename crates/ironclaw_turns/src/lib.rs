//! Host-layer turn coordination contracts for IronClaw Reborn.
//!
//! `ironclaw_turns` sits above the Reborn kernel facade. Product adapters use
//! the adapter-safe [`TurnCoordinator`] API with canonical refs resolved by the
//! binding/session layer. Trusted workers use [`runner`] explicitly; runner
//! transition APIs are intentionally not re-exported from this crate prelude.

pub mod coordinator;
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub mod db;
pub mod events;
pub mod ids;
pub mod loop_exit;
pub mod memory;
pub mod request;
pub mod response;
pub mod runner;
pub mod scope;
pub mod status;
pub mod store;

pub use coordinator::{
    AllowAllTurnAdmissionPolicy, DefaultTurnCoordinator, NoopTurnRunWakeNotifier,
    TurnAdmissionPolicy, TurnCoordinator, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
};
#[cfg(feature = "libsql")]
pub use db::LibSqlTurnStateStore;
#[cfg(feature = "postgres")]
pub use db::PostgresTurnStateStore;
pub use events::{InMemoryTurnEventSink, TurnEventKind, TurnEventSink, TurnLifecycleEvent};
pub use ids::{
    AcceptedMessageRef, GateRef, IdempotencyKey, LoopDiagnosticRef, LoopExitId, LoopGateRef,
    LoopMessageRef, LoopResultRef, LoopUsageSummaryRef, ReplyTargetBindingRef, RunProfileId,
    RunProfileRequest, RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunnerId,
};
pub use loop_exit::{
    LoopBlocked, LoopBlockedKind, LoopCancelled, LoopCancelledReasonKind, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitInvalidHandling, LoopExitMapping,
    LoopExitValidationDecision, LoopExitValidationPolicy, LoopExitViolation, LoopExitViolationKind,
    LoopFailed, LoopFailureKind,
};
pub use memory::{InMemoryTurnStateStore, InMemoryTurnStateStoreLimits};
pub use request::{
    CancelRunRequest, GetRunStateRequest, ResumeTurnRequest, SubmitTurnRequest, TurnTimestamp,
};
pub use response::{CancelRunResponse, ResumeTurnResponse, SubmitTurnResponse, ThreadBusy};
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
