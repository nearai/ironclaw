//! Process lifecycle contracts for IronClaw Reborn.
//!
//! `ironclaw_processes` stores and manages host-tracked background capability
//! processes. It owns lifecycle mechanics, not capability authorization or
//! runtime dispatch policy.
//!
//! # Module map
//!
//! - [`types`] — public data types, errors, and core traits
//!   ([`ProcessResultStorePort`], [`ProcessExecutor`],
//!   [`ProcessManager`])
//! - [`cancellation`] — cooperative cancellation tokens + per-process registry
//! - [`host`] — read/poll/await/cancel surface ([`ProcessHost`],
//!   [`ProcessSubscription`])
//! - [`result_store`] — externalized process result metadata and output bodies
//! - [`services`] — composition root ([`ProcessServices`]) and the
//!   production [`BackgroundProcessManager`]

mod cancellation;
mod capability_process;
mod host;
mod invocation_state;
mod journal;
mod journal_store;
mod result_store;
mod services;
mod supervisor;
#[cfg(any(test, feature = "test-support"))]
mod test_support;
mod types;

pub use cancellation::{ProcessCancellationRegistry, ProcessCancellationToken};
pub use capability_process::{
    capability_process_record, complete_capability_process, fail_capability_process,
    map_process_journal_error, process_record_from_snapshot, submit_capability_process,
};
pub use host::{ProcessHost, ProcessSubscription};
pub use invocation_state::{
    ProcessInvocationError, ProcessInvocationRecord, ProcessInvocationStart,
    ProcessInvocationStatePort, ProcessInvocationStatus, ProcessInvocationStore,
};
pub use journal::{
    CancelProcessRequest, ClaimProcessesRequest, ClaimedProcess, CloseProcessDependencyRequest,
    FailProcessRequest, GetProcessCheckpointRequest, GetProcessInputRequest,
    GetProcessSnapshotRequest, JournaledProcessSnapshot, KillProcessRequest,
    MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES, MAX_PROCESS_INPUT_PAYLOAD_BYTES,
    OpenProcessDependencyRequest, ProcessCheckpointId, ProcessCheckpointPayload,
    ProcessCheckpointPort, ProcessCheckpointRecord, ProcessCheckpointRef, ProcessConcurrencyClass,
    ProcessConcurrencyLimits, ProcessControlPort, ProcessControlResult, ProcessDependencyPort,
    ProcessDependencyQuery, ProcessDependencyRecord, ProcessDependencyState,
    ProcessDependencySubmission, ProcessGateOwnerMatch, ProcessGateQuery, ProcessGateQuerySource,
    ProcessGateRecord, ProcessGateScopeMatch, ProcessInputPayload, ProcessInputPort,
    ProcessInputRecord, ProcessInputRef, ProcessInputSubmission, ProcessJournalCommit,
    ProcessJournalCommitObserver, ProcessJournalCursor, ProcessJournalEntry, ProcessJournalError,
    ProcessJournalKind, ProcessJournalObserverRegistry, ProcessJournalPage,
    ProcessJournalProjectionCursor, ProcessJournalProjectionRequest,
    ProcessJournalProjectionSnapshot, ProcessJournalSource, ProcessKind, ProcessLeaseRequest,
    ProcessLeaseSnapshot, ProcessLeaseToken, ProcessLifecycleLookupBatchRequest,
    ProcessLifecycleLookupRequest, ProcessLifecycleLookupResult, ProcessLifecycleLookupSource,
    ProcessLifecycleStatus, ProcessOperationId, ProcessOutcome, ProcessSnapshotSource,
    ProcessStateTransitionRequest, ProcessSubmissionPort, ProcessSuspension, ProcessSuspensionKind,
    ProcessTerminalEvidence, ProcessTransitionPort, ProcessTreePort, ProcessTreeReservation,
    ProcessWorkerId, PruneReleasedProcessRequest, RecordProcessCheckpointRequest,
    RecoverExpiredProcessLeasesRequest, RecoverExpiredProcessLeasesResponse,
    ReleaseProcessTreeRequest, ReserveProcessTreeRequest, ResumeProcessRequest,
    SettleProcessDependencyRequest, StopProcessRequest, SubmitProcessRequest,
    SubmitProcessWithCheckpointRequest, SuspendProcessRequest,
};
pub use journal_store::{ProcessJournalStore, ProcessJournalStoreError};
pub use result_store::ProcessResultStore;
pub use services::{
    BackgroundErrorHandler, BackgroundFailure, BackgroundFailureStage, BackgroundProcessManager,
    ProcessServices,
};
pub use supervisor::{
    JournalProcessExecutor, ProcessExecutorFailure, ProcessSupervisor, ProcessSupervisorConfig,
    ProcessSupervisorHandle, ProcessWakeChannel, ProcessWakeError, ProcessWakeNotifier,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{
    ProcessInvocationStateStore, in_memory_backed_process_invocation_state_store,
    in_memory_backed_process_result_store, in_memory_backed_process_services,
    in_memory_backed_process_store, in_memory_backed_processes_filesystem,
};
pub use types::{
    ProcessError, ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessExit, ProcessManager, ProcessRecord, ProcessResultRecord,
    ProcessResultStorePort, ProcessStart, ProcessStatus, ProcessSubmissionLifecycle,
};

/// Complete static-kernel process surface. Consumers should accept narrower
/// ports; composition uses this trait to carry one journal implementation.
pub trait ProcessRuntimePort:
    ProcessSubmissionPort<Error = ProcessJournalStoreError>
    + ProcessTransitionPort<Error = ProcessJournalStoreError>
    + ProcessControlPort<Error = ProcessJournalStoreError>
    + ProcessJournalSource<Error = ProcessJournalStoreError>
    + ProcessSnapshotSource<Error = ProcessJournalStoreError>
    + ProcessLifecycleLookupSource<Error = ProcessJournalStoreError>
    + ProcessGateQuerySource<Error = ProcessJournalStoreError>
    + ProcessTreePort<Error = ProcessJournalStoreError>
    + ProcessDependencyPort<Error = ProcessJournalStoreError>
    + ProcessCheckpointPort<Error = ProcessJournalStoreError>
    + ProcessInputPort<Error = ProcessJournalStoreError>
    + ProcessJournalObserverRegistry
    + Send
    + Sync
{
}
