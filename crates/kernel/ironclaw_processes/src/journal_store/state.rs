use std::collections::{HashMap, VecDeque};

use std::time::Duration;

use ironclaw_host_api::{ids::ProcessId, resource::ResourceScope, turn::SanitizedFailure};
use serde::{Deserialize, Serialize};

use super::{
    ProcessControlAction, ProcessJournalStoreError, StoredCommandOutcome, StoredProcessCommand,
    ensure_lease, ensure_transition, process_claim_within_limits, process_scope_visible,
    same_lineage_scope, validate_tree_root,
};
use crate::{
    ClaimedProcess, JournaledProcessSnapshot, ProcessCheckpointId, ProcessCheckpointKind,
    ProcessCheckpointRecord, ProcessCheckpointRef, ProcessControlResult, ProcessFailureRecovery,
    ProcessInputRecord, ProcessJournalCursor, ProcessJournalEntry, ProcessJournalKind, ProcessKind,
    ProcessLeaseSnapshot, ProcessLeaseToken, ProcessLifecycleStatus, ProcessSubmissionEdge,
    ProcessTreeReservation, RecoverExpiredProcessLeasesResponse, SubmitProcessAtEdgeRequest,
    SubmitProcessRequest, types::same_scope_owner,
};

/// Point a process at a new resume checkpoint.
///
/// Only a `RecordCheckpoint` command knows a checkpoint's kind, so any other
/// mutation that repoints the reference drops the kind rather than carry a stale
/// one forward. A mutation restating the reference the snapshot already holds
/// keeps the kind it was recorded with. An unknown kind is treated as
/// side-effecting by [`ProcessJournalMaterializedState::apply_recover_expired`],
/// so dropping it fails closed.
fn adopt_checkpoint_ref(
    snapshot: &mut JournaledProcessSnapshot,
    checkpoint_ref: Option<ProcessCheckpointRef>,
) {
    if checkpoint_ref.is_none() || checkpoint_ref == snapshot.checkpoint_ref {
        return;
    }
    snapshot.checkpoint_ref = checkpoint_ref;
    snapshot.checkpoint_kind = None;
}

const MAX_IDEMPOTENCY_RECORDS: usize = 4096;
/// Maximum number of crash-recovery claims allowed before a checkpointless
/// process is failed terminally.
pub const MAX_CRASH_RECOVERY_RECLAIMS: u64 = 3;
/// Sanitized failure category recovery writes when a checkpointed process
/// cannot be requeued after its lease expired.
pub const LEASE_EXPIRED_FAILURE_CATEGORY: &str = "lease_expired";
/// Sanitized failure category recovery writes when a checkpointless process
/// has been reclaimed [`MAX_CRASH_RECOVERY_RECLAIMS`] times.
pub const CRASH_RETRY_EXHAUSTED_FAILURE_CATEGORY: &str = "crash_retry_exhausted";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ProcessJournalMaterializedState {
    pub(super) next_cursor: u64,
    pub(super) processes: HashMap<ProcessId, JournaledProcessSnapshot>,
    pub(super) journal: Vec<ProcessJournalEntry>,
    #[serde(default)]
    pub(super) control_idempotency: HashMap<String, ProcessControlResult>,
    #[serde(default)]
    pub(super) control_idempotency_order: VecDeque<String>,
    #[serde(default)]
    pub(super) submission_idempotency: HashMap<String, JournaledProcessSnapshot>,
    #[serde(default)]
    pub(super) submission_idempotency_order: VecDeque<String>,
    /// op-id → process-id binding for edge submissions, kept in memory only.
    ///
    /// Edge submissions deliberately do not persist full-snapshot idempotency
    /// records (see [`Self::apply_submit_at_edge`]); replay validation is
    /// answered from the durable process row instead. This compact binding
    /// retains the same-operation-different-process rejection for the life of
    /// the store without re-serializing a full snapshot per record on every
    /// journal command.
    #[serde(default)]
    pub(super) edge_submission_bindings: HashMap<String, ProcessId>,
    #[serde(default)]
    pub(super) edge_submission_bindings_order: VecDeque<String>,
    #[serde(default)]
    pub(super) tree_reservations: HashMap<ProcessId, ProcessTreeReservation>,
    #[serde(default)]
    pub(super) dependencies: HashMap<(ProcessId, ProcessId), crate::ProcessDependencyRecord>,
    #[serde(default)]
    pub(super) checkpoints: HashMap<ProcessCheckpointId, ProcessCheckpointRecord>,
    #[serde(default)]
    pub(super) inputs: HashMap<ProcessId, ProcessInputRecord>,
    #[serde(default)]
    pub(super) legacy_imported: bool,
}

impl Default for ProcessJournalMaterializedState {
    fn default() -> Self {
        Self {
            next_cursor: 1,
            processes: HashMap::new(),
            journal: Vec::new(),
            control_idempotency: HashMap::new(),
            control_idempotency_order: VecDeque::new(),
            submission_idempotency: HashMap::new(),
            submission_idempotency_order: VecDeque::new(),
            edge_submission_bindings: HashMap::new(),
            edge_submission_bindings_order: VecDeque::new(),
            tree_reservations: HashMap::new(),
            dependencies: HashMap::new(),
            checkpoints: HashMap::new(),
            inputs: HashMap::new(),
            legacy_imported: false,
        }
    }
}

impl ProcessJournalMaterializedState {
    pub(super) fn import_deployed_snapshot(&mut self, mut snapshot: JournaledProcessSnapshot) {
        if self.processes.contains_key(&snapshot.process_id) {
            return;
        }
        let cursor = if snapshot.journal_cursor.0 == 0
            || self
                .journal
                .iter()
                .any(|entry| entry.cursor == snapshot.journal_cursor)
        {
            self.next_cursor()
        } else {
            self.next_cursor = self
                .next_cursor
                .max(snapshot.journal_cursor.0.saturating_add(1));
            snapshot.journal_cursor
        };
        snapshot.journal_cursor = cursor;
        let kind = match snapshot.status {
            ProcessLifecycleStatus::Queued => ProcessJournalKind::Submitted,
            ProcessLifecycleStatus::Running => ProcessJournalKind::Claimed,
            ProcessLifecycleStatus::Suspended => ProcessJournalKind::Suspended,
            ProcessLifecycleStatus::StopRequested => ProcessJournalKind::StopRequested,
            ProcessLifecycleStatus::CancelRequested => ProcessJournalKind::CancelRequested,
            ProcessLifecycleStatus::Stopped => ProcessJournalKind::Stopped,
            ProcessLifecycleStatus::Cancelled => ProcessJournalKind::Cancelled,
            ProcessLifecycleStatus::Completed => ProcessJournalKind::Completed,
            ProcessLifecycleStatus::Failed => ProcessJournalKind::Failed,
            ProcessLifecycleStatus::Killed => ProcessJournalKind::Killed,
            ProcessLifecycleStatus::RecoveryRequired => ProcessJournalKind::RecoveryRequired,
        };
        self.push_entry(ProcessJournalEntry::from_snapshot(&snapshot, cursor, kind));
        self.processes.insert(snapshot.process_id, snapshot);
        self.legacy_imported = true;
    }

    pub(super) fn import_deployed_checkpoint(&mut self, checkpoint: ProcessCheckpointRecord) {
        self.checkpoints
            .entry(checkpoint.checkpoint_id.clone())
            .or_insert(checkpoint);
        self.legacy_imported = true;
    }

    pub(super) fn import_deployed_tree_reservation(&mut self, reservation: ProcessTreeReservation) {
        self.tree_reservations
            .entry(reservation.root_process_id)
            .or_insert(reservation);
        self.legacy_imported = true;
    }

    pub(super) fn import_deployed_submit_idempotency(
        &mut self,
        operation_id: &str,
        snapshot: JournaledProcessSnapshot,
    ) -> Result<(), ProcessJournalStoreError> {
        let scope = &snapshot.scope;
        let scope = serde_json::to_string(&(
            &scope.tenant_id,
            &scope.user_id,
            &scope.agent_id,
            &scope.project_id,
            &scope.mission_id,
            &scope.thread_id,
        ))
        .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))?;
        self.remember_submission_result(
            Some(format!(
                "submit:{scope}:{:?}:{operation_id}",
                snapshot.process_kind
            )),
            snapshot,
        );
        Ok(())
    }

    pub(super) fn import_deployed_control_idempotency(
        &mut self,
        action: &str,
        operation_id: &str,
        snapshot: JournaledProcessSnapshot,
    ) {
        self.remember_control_result(
            Some(format!("{action}:{}:{operation_id}", snapshot.process_id)),
            ProcessControlResult {
                already_terminal: snapshot.status.is_terminal(),
                state: snapshot,
                changed: false,
            },
        );
    }

    pub(super) fn apply_command(
        &mut self,
        command: StoredProcessCommand,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        match command {
            StoredProcessCommand::ImportLegacyState(state) => {
                if !self.legacy_imported && self.processes.is_empty() && self.journal.is_empty() {
                    let mut imported = *state;
                    let max_cursor = imported
                        .journal
                        .last()
                        .map(|entry| entry.cursor.0)
                        .unwrap_or_else(|| imported.next_cursor.saturating_sub(1));
                    imported.next_cursor = max_cursor.saturating_add(1);
                    imported.legacy_imported = true;
                    *self = imported;
                }
                Ok(StoredCommandOutcome::Imported)
            }
            StoredProcessCommand::Submit(request) => self.apply_submit(*request),
            StoredProcessCommand::SubmitAtEdge(request) => self.apply_submit_at_edge(*request),
            StoredProcessCommand::SubmitWithCheckpoint {
                request,
                checkpoint,
            } => {
                let request = *request;
                let checkpoint = *checkpoint;
                if checkpoint.process_id != request.process_id
                    || checkpoint.scope != request.scope
                    || request
                        .checkpoint_ref
                        .as_ref()
                        .map(ProcessCheckpointRef::as_str)
                        != Some(checkpoint.checkpoint_id.as_str())
                {
                    return Err(ProcessJournalStoreError::InvalidRequest(
                        "initial checkpoint must be owned and referenced by the submitted process"
                            .to_string(),
                    ));
                }
                let outcome = self.apply_submit(request)?;
                if matches!(outcome, StoredCommandOutcome::Submitted(_, true)) {
                    self.apply_checkpoint(checkpoint)?;
                }
                Ok(outcome)
            }
            StoredProcessCommand::Claim {
                request,
                now,
                lease_duration_millis,
                lease_nonce,
                limits,
            } => self.apply_claim(
                request,
                now,
                Duration::from_millis(lease_duration_millis),
                lease_nonce,
                limits,
            ),
            StoredProcessCommand::Heartbeat {
                request,
                now,
                lease_duration_millis,
            } => self.apply_heartbeat(request, now, Duration::from_millis(lease_duration_millis)),
            StoredProcessCommand::RecoverExpired {
                request,
                lease_duration_millis,
            } => self.apply_recover_expired(request, Duration::from_millis(lease_duration_millis)),
            StoredProcessCommand::LeasedTransition { request, mutation } => {
                self.apply_leased_transition(request, mutation)
            }
            StoredProcessCommand::Control(mutation) => self.apply_control(mutation),
            StoredProcessCommand::ReserveTree(request) => self.apply_reserve_tree(request),
            StoredProcessCommand::ReleaseTree(request) => self.apply_release_tree(request),
            StoredProcessCommand::PruneTree(request) => self.apply_prune_tree(request),
            StoredProcessCommand::OpenDependency(request) => self.apply_open_dependency(request),
            StoredProcessCommand::SettleDependency(request) => {
                self.apply_settle_dependency(request)
            }
            StoredProcessCommand::TransitionDependency(request) => {
                self.apply_transition_dependency(request)
            }
            StoredProcessCommand::ConsumeDependency(request) => {
                self.apply_close_dependency(request, false)
            }
            StoredProcessCommand::AbandonDependency(request) => {
                self.apply_close_dependency(request, true)
            }
            StoredProcessCommand::RecordCheckpoint(request) => self.apply_checkpoint(request),
        }
    }

    fn apply_submit(
        &mut self,
        request: crate::SubmitProcessRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let replay_key = super::command::submission_replay_key(&request)?;
        if let Some(snapshot) = replay_key
            .as_ref()
            .and_then(|key| self.submission_idempotency.get(key))
        {
            return Ok(StoredCommandOutcome::Submitted(snapshot.clone(), false));
        }
        if self.processes.contains_key(&request.process_id) {
            return Err(ProcessJournalStoreError::ProcessAlreadyExists {
                process_id: request.process_id,
            });
        }
        if request.exclusive_within_scope
            && let Some(active) = self.processes.values().find(|snapshot| {
                snapshot.process_kind == request.process_kind
                    && snapshot.status.keeps_active_lock()
                    && same_scope_owner(&snapshot.scope, &request.scope)
            })
        {
            return Err(ProcessJournalStoreError::ActiveProcessConflict {
                process_id: active.process_id,
                process_kind: active.process_kind.clone(),
                status: active.status,
                suspension: active.suspension.clone().map(Box::new),
                cursor: active.journal_cursor,
            });
        }
        if let Some(dependency) = &request.dependency {
            if request.parent_process_id != Some(dependency.dependent_process_id) {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "submitted dependency does not match parent process".to_string(),
                ));
            }
            if request.root_process_id != Some(dependency.root_process_id) {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "submitted dependency does not match root process".to_string(),
                ));
            }
        }
        let tree_reservation = if let Some(parent_process_id) = request.parent_process_id {
            let parent = self.processes.get(&parent_process_id).ok_or(
                ProcessJournalStoreError::UnknownProcess {
                    process_id: parent_process_id,
                },
            )?;
            if !same_lineage_scope(&parent.scope, &request.scope) {
                return Err(ProcessJournalStoreError::UnauthorizedScope);
            }
            let root_process_id = parent.root_process_id.unwrap_or(parent.process_id);
            if request.root_process_id != Some(root_process_id) {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "child root_process_id does not match parent lineage".to_string(),
                ));
            }
            let cap = request.spawn_tree_descendant_cap.ok_or_else(|| {
                ProcessJournalStoreError::InvalidRequest(
                    "child process submission requires a descendant cap".to_string(),
                )
            })?;
            let current = self
                .tree_reservations
                .get(&root_process_id)
                .map(|reservation| reservation.descendant_count)
                .unwrap_or(0);
            let next = current
                .checked_add(1)
                .ok_or(ProcessJournalStoreError::ProcessTreeCapacityExceeded { cap })?;
            if next > u64::from(cap) {
                return Err(ProcessJournalStoreError::ProcessTreeCapacityExceeded { cap });
            }
            Some((root_process_id, next))
        } else {
            if request.root_process_id.is_some() || request.spawn_tree_descendant_cap.is_some() {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "root lineage requires a parent process".to_string(),
                ));
            }
            None
        };
        let dependency = request.dependency.clone();
        let input = request.input.clone();
        let cursor = self.next_cursor();
        let snapshot = JournaledProcessSnapshot {
            process_id: request.process_id,
            process_kind: request.process_kind,
            scope: request.scope,
            status: ProcessLifecycleStatus::Queued,
            suspension: None,
            checkpoint_ref: request.checkpoint_ref,
            // A submission that carries a checkpoint reference records the
            // checkpoint itself in the same command, which is what sets the kind.
            checkpoint_kind: None,
            input_ref: input.as_ref().map(|input| input.input_ref.clone()),
            failure: None,
            journal_cursor: cursor,
            lease: None,
            crash_reclaim_count: 0,
            created_at: request.created_at,
            owner_user_id: request.owner_user_id,
            concurrency_class: request.concurrency_class,
            parent_process_id: request.parent_process_id,
            root_process_id: request.root_process_id,
            metadata: request.metadata,
        };
        self.push_entry(ProcessJournalEntry::from_snapshot(
            &snapshot,
            cursor,
            ProcessJournalKind::Submitted,
        ));
        self.processes.insert(snapshot.process_id, snapshot.clone());
        if let Some(input) = input {
            self.inputs.insert(
                snapshot.process_id,
                ProcessInputRecord {
                    process_id: snapshot.process_id,
                    scope: snapshot.scope.clone(),
                    input_ref: input.input_ref,
                    payload: input.payload,
                    created_at: snapshot.created_at,
                },
            );
        }
        if let Some((root_process_id, descendant_count)) = tree_reservation {
            let released_processes = self
                .tree_reservations
                .get(&root_process_id)
                .map(|reservation| reservation.released_processes.clone())
                .unwrap_or_default();
            self.tree_reservations.insert(
                root_process_id,
                ProcessTreeReservation {
                    root_process_id,
                    descendant_count,
                    released_processes,
                },
            );
        }
        if let Some(dependency) = dependency {
            self.dependencies.insert(
                (dependency.dependent_process_id, snapshot.process_id),
                crate::ProcessDependencyRecord {
                    dependent_process_id: dependency.dependent_process_id,
                    dependency_process_id: snapshot.process_id,
                    root_process_id: dependency.root_process_id,
                    scope: snapshot.scope.clone(),
                    group_ref: dependency.group_ref,
                    state: crate::ProcessDependencyState::Open,
                    terminal: None,
                    created_at: snapshot.created_at,
                    settled_at: None,
                    consumed_at: None,
                    transitioned_at: None,
                    metadata: dependency.metadata,
                },
            );
        }
        self.remember_submission_result(replay_key, snapshot.clone());
        Ok(StoredCommandOutcome::Submitted(snapshot, true))
    }

    fn edge_replay_matches(
        submission: &SubmitProcessRequest,
        edge: &ProcessSubmissionEdge,
        snapshot: &JournaledProcessSnapshot,
    ) -> bool {
        let edge_matches = match edge {
            ProcessSubmissionEdge::Suspended { suspension } => {
                snapshot.status == ProcessLifecycleStatus::Suspended
                    && snapshot.suspension.as_ref() == Some(suspension)
                    && snapshot.failure.is_none()
            }
            ProcessSubmissionEdge::Completed => {
                snapshot.status == ProcessLifecycleStatus::Completed
                    && snapshot.suspension.is_none()
                    && snapshot.failure.is_none()
            }
            ProcessSubmissionEdge::Failed { failure } => {
                snapshot.status == ProcessLifecycleStatus::Failed
                    && snapshot.suspension.is_none()
                    && snapshot.failure.as_ref() == Some(failure)
            }
        };
        edge_matches
            && snapshot.process_id == submission.process_id
            && snapshot.process_kind == submission.process_kind
            && snapshot.scope == submission.scope
            && snapshot.checkpoint_ref == submission.checkpoint_ref
            && snapshot.owner_user_id == submission.owner_user_id
            && snapshot.concurrency_class == submission.concurrency_class
            && snapshot.metadata == submission.metadata
    }

    fn apply_submit_at_edge(
        &mut self,
        request: SubmitProcessAtEdgeRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let submission = request.submission;
        if submission.process_kind != ProcessKind::CapabilityInvocationState
            || submission.exclusive_within_scope
            || submission.parent_process_id.is_some()
            || submission.root_process_id.is_some()
            || submission.spawn_tree_descendant_cap.is_some()
            || submission.dependency.is_some()
            || submission.input.is_some()
        {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "edge submission only supports standalone bookkeeping processes".to_string(),
            ));
        }
        let replay_key = super::command::submission_replay_key(&submission)?;
        if let Some(snapshot) = replay_key
            .as_ref()
            .and_then(|key| self.submission_idempotency.get(key))
        {
            if !Self::edge_replay_matches(&submission, &request.edge, snapshot) {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "edge submission replay does not match the original request".to_string(),
                ));
            }
            return Ok(StoredCommandOutcome::Submitted(snapshot.clone(), false));
        }
        // Edge submissions keep only a compact in-memory binding, so the
        // durable process row is the replay authority: a retry of the same
        // operation after a restart must return the committed terminal state
        // rather than failing, and a same-operation submission with a
        // different process id must still be rejected.
        if let Some(bound_process_id) = replay_key
            .as_ref()
            .and_then(|key| self.edge_submission_bindings.get(key))
        {
            if *bound_process_id != submission.process_id {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "edge submission replay does not match the original request".to_string(),
                ));
            }
            let existing = self.processes.get(&submission.process_id).ok_or(
                ProcessJournalStoreError::ProcessAlreadyExists {
                    process_id: submission.process_id,
                },
            )?;
            if !Self::edge_replay_matches(&submission, &request.edge, existing) {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "edge submission replay does not match the original request".to_string(),
                ));
            }
            return Ok(StoredCommandOutcome::Submitted(existing.clone(), false));
        }
        if let Some(existing) = self.processes.get(&submission.process_id) {
            // No binding survived (restart), but the process row is durable:
            // accept an identical replay, reject anything else.
            if Self::edge_replay_matches(&submission, &request.edge, existing) {
                return Ok(StoredCommandOutcome::Submitted(existing.clone(), false));
            }
            return Err(ProcessJournalStoreError::ProcessAlreadyExists {
                process_id: submission.process_id,
            });
        }

        let (status, kind, suspension, failure) = match request.edge {
            ProcessSubmissionEdge::Suspended { suspension } => {
                if submission.checkpoint_ref.is_none() {
                    return Err(ProcessJournalStoreError::InvalidRequest(
                        "suspended edge submission requires a checkpoint reference".to_string(),
                    ));
                }
                (
                    ProcessLifecycleStatus::Suspended,
                    ProcessJournalKind::Suspended,
                    Some(suspension),
                    None,
                )
            }
            ProcessSubmissionEdge::Completed => {
                if submission.checkpoint_ref.is_some() {
                    return Err(ProcessJournalStoreError::InvalidRequest(
                        "terminal edge submission cannot carry a checkpoint reference".to_string(),
                    ));
                }
                (
                    ProcessLifecycleStatus::Completed,
                    ProcessJournalKind::Completed,
                    None,
                    None,
                )
            }
            ProcessSubmissionEdge::Failed { failure } => {
                if submission.checkpoint_ref.is_some() {
                    return Err(ProcessJournalStoreError::InvalidRequest(
                        "terminal edge submission cannot carry a checkpoint reference".to_string(),
                    ));
                }
                (
                    ProcessLifecycleStatus::Failed,
                    ProcessJournalKind::Failed,
                    None,
                    Some(failure),
                )
            }
        };
        let cursor = self.next_cursor();
        // `checkpoint_kind` remains unknown intentionally. If a later lease
        // expires, recovery must fail closed rather than redispatching work
        // whose side effect may already have been attempted.
        let snapshot = JournaledProcessSnapshot {
            process_id: submission.process_id,
            process_kind: submission.process_kind,
            scope: submission.scope,
            status,
            suspension,
            checkpoint_ref: submission.checkpoint_ref,
            checkpoint_kind: None,
            input_ref: None,
            failure,
            journal_cursor: cursor,
            lease: None,
            crash_reclaim_count: 0,
            created_at: submission.created_at,
            owner_user_id: submission.owner_user_id,
            concurrency_class: submission.concurrency_class,
            parent_process_id: None,
            root_process_id: None,
            metadata: submission.metadata,
        };
        self.push_entry(ProcessJournalEntry::from_snapshot(&snapshot, cursor, kind));
        self.processes.insert(snapshot.process_id, snapshot.clone());
        if let Some(key) = replay_key {
            self.remember_edge_submission(key, snapshot.process_id);
        }
        Ok(StoredCommandOutcome::Submitted(snapshot, true))
    }

    fn apply_claim(
        &mut self,
        request: crate::ClaimProcessesRequest,
        now: ironclaw_host_api::Timestamp,
        lease_duration: Duration,
        lease_nonce: ProcessId,
        limits: crate::ProcessConcurrencyLimits,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let mut claimed = Vec::new();
        let process_ids = self.claimable_process_ids(
            request.scope_filter.as_ref(),
            request.process_id_filter,
            request.process_kind_filter.as_ref(),
        );
        for process_id in process_ids {
            if claimed.len() >= request.max_processes {
                break;
            }
            if !process_claim_within_limits(self, process_id, &limits) {
                continue;
            }
            let cursor = self.next_cursor();
            let Some(snapshot) = self.processes.get_mut(&process_id) else {
                continue;
            };
            snapshot.status = ProcessLifecycleStatus::Running;
            snapshot.suspension = None;
            snapshot.lease = Some(ProcessLeaseSnapshot {
                worker_id: request.worker_id.clone(),
                lease_token: ProcessLeaseToken::from_trusted(lease_nonce.to_string()),
                lease_expires_at: chrono::Duration::from_std(lease_duration)
                    .ok()
                    .map(|duration| now + duration),
                last_heartbeat_at: Some(now),
                claim_count: snapshot.crash_reclaim_count.saturating_add(1),
            });
            snapshot.journal_cursor = cursor;
            let snapshot = snapshot.clone();
            self.push_entry(ProcessJournalEntry::from_snapshot(
                &snapshot,
                cursor,
                ProcessJournalKind::Claimed,
            ));
            let lease = snapshot
                .lease
                .clone()
                .ok_or(ProcessJournalStoreError::InvalidLease { process_id })?;
            claimed.push(ClaimedProcess {
                state: snapshot,
                worker_id: lease.worker_id,
                lease_token: lease.lease_token,
            });
        }
        Ok(StoredCommandOutcome::Claimed(claimed))
    }

    fn apply_heartbeat(
        &mut self,
        request: crate::ProcessLeaseRequest,
        now: ironclaw_host_api::Timestamp,
        lease_duration: Duration,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let snapshot = self.process_mut(request.process_id)?;
        ensure_transition(snapshot, ProcessLifecycleStatus::Running)?;
        ensure_lease(snapshot, &request.worker_id, &request.lease_token)?;
        if let Some(lease) = &mut snapshot.lease {
            lease.last_heartbeat_at = Some(now);
            lease.lease_expires_at = chrono::Duration::from_std(lease_duration)
                .ok()
                .map(|duration| now + duration);
        }
        Ok(StoredCommandOutcome::Heartbeat(snapshot.clone()))
    }

    fn apply_recover_expired(
        &mut self,
        request: crate::RecoverExpiredProcessLeasesRequest,
        lease_duration: Duration,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let expired = self.expired_process_ids(
            request.scope_filter.as_ref(),
            request.process_kind_filter.as_ref(),
            request.now,
        );
        let mut recovered = Vec::new();
        for process_id in expired {
            if self.requeue_awaits_grace(process_id, request.now, lease_duration)? {
                continue;
            }
            let cursor = self.next_cursor();
            let snapshot = self.process_mut(process_id)?;
            let claim_count = snapshot.lease.as_ref().map_or(0, |lease| lease.claim_count);
            let resumable_checkpoint = snapshot
                .checkpoint_kind
                .is_some_and(|kind| !kind.replays_side_effect());
            let reclaimable = snapshot.checkpoint_ref.is_none() || resumable_checkpoint;
            let (status, kind, failure) =
                if snapshot.status == ProcessLifecycleStatus::CancelRequested {
                    (
                        ProcessLifecycleStatus::Cancelled,
                        ProcessJournalKind::Cancelled,
                        None,
                    )
                } else if reclaimable && claim_count < MAX_CRASH_RECOVERY_RECLAIMS {
                    // Requeuing here does not ask whether the old executor is
                    // alive. Two windows bound how long it could still have been
                    // heartbeating: `budget * (heartbeat interval + heartbeat
                    // timeout) <= lease TTL`, enforced by the turn scheduler
                    // capping both the interval and the budget against the TTL,
                    // plus one further full TTL of grace before this branch runs,
                    // enforced by [`Self::requeue_awaits_grace`].
                    //
                    // That is half the guarantee, and it proves only that the
                    // old worker has stopped *heartbeating* — not that it has
                    // given the run up. Time-based fencing can never be total: a
                    // worker whose heartbeat loop died or was starved while its
                    // main task stayed blocked in a model or capability call
                    // (the Postgres pool-starvation shape) can wake after the
                    // grace window and still try to write.
                    //
                    // The other half, equally necessary, is that such a worker
                    // discovers it lost the run at its next lease-fenced write:
                    // run transitions carry the lease token and `ensure_lease`
                    // refuses them, and transcript finalization asks the journal
                    // for the claimed run's lease before appending
                    // (`ironclaw_loop_host`'s `ThreadBackedLoopTranscriptPort`
                    // fence, wired by the turn runner's loop-driver host).
                    // Neither half alone is sufficient — the write-seam fence is
                    // the missing half of this branch's safety, not optional
                    // hardening. Regression:
                    // `run_parked_before_a_model_call_is_resumed_after_lease_expiry_not_failed`
                    // in `tests/integration/lease_wedge.rs`.
                    //
                    // The journal is the only authority on ownership; do not add
                    // a runtime liveness probe.
                    (
                        ProcessLifecycleStatus::Queued,
                        ProcessJournalKind::Resumed,
                        None,
                    )
                } else {
                    let category = if snapshot.checkpoint_ref.is_some() {
                        LEASE_EXPIRED_FAILURE_CATEGORY
                    } else {
                        CRASH_RETRY_EXHAUSTED_FAILURE_CATEGORY
                    };
                    (
                        ProcessLifecycleStatus::Failed,
                        ProcessJournalKind::Failed,
                        Some(SanitizedFailure::from_trusted_static(category)),
                    )
                };
            snapshot.status = status;
            snapshot.lease = None;
            if status == ProcessLifecycleStatus::Queued {
                snapshot.crash_reclaim_count = claim_count;
            }
            snapshot.failure = failure;
            snapshot.journal_cursor = cursor;
            let snapshot = snapshot.clone();
            self.push_entry(ProcessJournalEntry::from_snapshot(&snapshot, cursor, kind));
            recovered.push(snapshot);
        }
        Ok(StoredCommandOutcome::Recovered(
            RecoverExpiredProcessLeasesResponse { recovered },
        ))
    }

    /// Whether a checkpointed process that recovery would requeue must be left
    /// alone for now because its lease expired too recently.
    ///
    /// Requeuing a checkpointed process re-enters work a worker may still be
    /// executing — the lease expired because heartbeats stopped arriving, which a
    /// starved-but-live worker and a dead worker both look like. Waiting one full
    /// lease TTL past expiry before reclaiming fences the live case: a worker that
    /// is still running would have renewed the lease inside that window, and one
    /// that has not is not coming back. A later sweep picks the process up.
    ///
    /// This applies only to the checkpointed-requeue branch. Cancellation and the
    /// checkpointless requeue keep their immediate timing: neither re-enters
    /// committed work.
    fn requeue_awaits_grace(
        &self,
        process_id: ProcessId,
        now: ironclaw_host_api::Timestamp,
        lease_duration: Duration,
    ) -> Result<bool, ProcessJournalStoreError> {
        let snapshot = self
            .processes
            .get(&process_id)
            .ok_or(ProcessJournalStoreError::UnknownProcess { process_id })?;
        if snapshot.status == ProcessLifecycleStatus::CancelRequested
            || snapshot.checkpoint_ref.is_none()
        {
            return Ok(false);
        }
        if snapshot
            .checkpoint_kind
            .is_none_or(ProcessCheckpointKind::replays_side_effect)
        {
            return Ok(false);
        }
        let Some(expires_at) = snapshot
            .lease
            .as_ref()
            .and_then(|lease| lease.lease_expires_at)
        else {
            return Ok(false);
        };
        let Ok(grace) = chrono::Duration::from_std(lease_duration) else {
            // A lease duration this large cannot bound a grace window, and the
            // same conversion already left such a lease without an expiry, so
            // this process should not have been reported expired. Hold it rather
            // than reclaim on an unbounded window.
            return Ok(true);
        };
        Ok(now < expires_at + grace)
    }

    fn apply_leased_transition(
        &mut self,
        request: crate::ProcessLeaseRequest,
        mut mutation: super::ProcessTransitionMutation,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let cursor = self.next_cursor();
        let snapshot = self.process_mut(request.process_id)?;
        ensure_lease(snapshot, &request.worker_id, &request.lease_token)?;
        // Cancellation wins a concurrent runner failure. The runner claimed the
        // process while it was Running, but a control request may have moved it
        // to CancelRequested before the loop exit was recorded. Converge that
        // race to Cancelled instead of stranding the lease until expiry.
        if snapshot.status == ProcessLifecycleStatus::CancelRequested
            && mutation.status == ProcessLifecycleStatus::Failed
        {
            mutation.status = ProcessLifecycleStatus::Cancelled;
            mutation.kind = ProcessJournalKind::Cancelled;
            mutation.failure = None;
        }
        if snapshot.status == ProcessLifecycleStatus::Running
            && mutation.status == ProcessLifecycleStatus::Failed
            && mutation.failure_recovery == ProcessFailureRecovery::RedriveIfCheckpointless
            && snapshot.checkpoint_ref.is_none()
            && mutation.checkpoint_ref.is_none()
            && snapshot
                .lease
                .as_ref()
                .is_some_and(|lease| lease.claim_count < MAX_CRASH_RECOVERY_RECLAIMS)
        {
            mutation.status = ProcessLifecycleStatus::Queued;
            mutation.kind = ProcessJournalKind::Resumed;
            mutation.failure = None;
            snapshot.crash_reclaim_count = snapshot
                .lease
                .as_ref()
                .map_or(snapshot.crash_reclaim_count, |lease| lease.claim_count);
        }
        ensure_transition(snapshot, mutation.status)?;
        snapshot.status = mutation.status;
        snapshot.suspension = mutation.suspension;
        adopt_checkpoint_ref(snapshot, mutation.checkpoint_ref);
        snapshot.failure = mutation.failure;
        if let Some(metadata) = mutation.metadata {
            snapshot.metadata = metadata;
        }
        if mutation.status != ProcessLifecycleStatus::Running {
            snapshot.lease = None;
        }
        snapshot.journal_cursor = cursor;
        let snapshot = snapshot.clone();
        self.push_entry(ProcessJournalEntry::from_snapshot(
            &snapshot,
            cursor,
            mutation.kind,
        ));
        if snapshot.status.is_terminal() {
            self.inputs.remove(&snapshot.process_id);
        }
        Ok(StoredCommandOutcome::Transitioned(snapshot))
    }

    fn apply_control(
        &mut self,
        mutation: super::ProcessControlMutation,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let replay_key = super::command::control_replay_key(&mutation);
        if let Some(result) = replay_key
            .as_ref()
            .and_then(|key| self.control_idempotency.get(key))
        {
            if !process_scope_visible(&result.state.scope, &mutation.scope) {
                return Err(ProcessJournalStoreError::UnknownProcess {
                    process_id: mutation.process_id,
                });
            }
            return Ok(StoredCommandOutcome::Controlled(result.clone()));
        }
        let snapshot = self.process_mut(mutation.process_id)?;
        if !process_scope_visible(&snapshot.scope, &mutation.scope) {
            return Err(ProcessJournalStoreError::UnknownProcess {
                process_id: mutation.process_id,
            });
        }
        if let Some(expected) = mutation.expected_cursor
            && expected != snapshot.journal_cursor
        {
            return Err(ProcessJournalStoreError::StaleSnapshot {
                process_id: mutation.process_id,
                expected,
                actual: snapshot.journal_cursor,
            });
        }
        let already_terminal = snapshot.status.is_terminal();
        let transition = match mutation.action {
            ProcessControlAction::Resume => {
                ensure_transition(snapshot, ProcessLifecycleStatus::Queued)?;
                Some((ProcessLifecycleStatus::Queued, ProcessJournalKind::Resumed))
            }
            ProcessControlAction::Stop => (!snapshot.status.is_terminal())
                .then_some((ProcessLifecycleStatus::Stopped, ProcessJournalKind::Stopped)),
            ProcessControlAction::Cancel => match snapshot.status {
                status if status.is_terminal() => None,
                ProcessLifecycleStatus::Running | ProcessLifecycleStatus::CancelRequested => {
                    Some((
                        ProcessLifecycleStatus::CancelRequested,
                        ProcessJournalKind::CancelRequested,
                    ))
                }
                _ => Some((
                    ProcessLifecycleStatus::Cancelled,
                    ProcessJournalKind::Cancelled,
                )),
            },
            ProcessControlAction::Kill => (!snapshot.status.is_terminal())
                .then_some((ProcessLifecycleStatus::Killed, ProcessJournalKind::Killed)),
        };
        let Some((status, kind)) = transition else {
            let result = ProcessControlResult {
                state: snapshot.clone(),
                changed: false,
                already_terminal,
            };
            self.remember_control_result(replay_key, result.clone());
            return Ok(StoredCommandOutcome::Controlled(result));
        };
        if status == snapshot.status {
            let result = ProcessControlResult {
                state: snapshot.clone(),
                changed: false,
                already_terminal,
            };
            self.remember_control_result(replay_key, result.clone());
            return Ok(StoredCommandOutcome::Controlled(result));
        }
        let cursor = self.next_cursor();
        let snapshot = self.process_mut(mutation.process_id)?;
        snapshot.status = status;
        snapshot.suspension = None;
        adopt_checkpoint_ref(snapshot, mutation.checkpoint_ref);
        if let Some(metadata) = mutation.metadata {
            snapshot.metadata = metadata;
        }
        snapshot.failure = None;
        if status != ProcessLifecycleStatus::CancelRequested {
            snapshot.lease = None;
        }
        snapshot.journal_cursor = cursor;
        let snapshot = snapshot.clone();
        let mut entry = ProcessJournalEntry::from_snapshot(&snapshot, cursor, kind);
        entry.sanitized_reason = mutation.reason;
        self.push_entry(entry);
        if snapshot.status.is_terminal() {
            self.inputs.remove(&snapshot.process_id);
        }
        let result = ProcessControlResult {
            state: snapshot,
            changed: true,
            already_terminal,
        };
        self.remember_control_result(replay_key, result.clone());
        Ok(StoredCommandOutcome::Controlled(result))
    }

    fn apply_reserve_tree(
        &mut self,
        request: crate::ReserveProcessTreeRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        if request.delta == 0 {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "reservation delta must be greater than zero".to_string(),
            ));
        }
        validate_tree_root(self, &request.scope, request.root_process_id)?;
        let current = self
            .tree_reservations
            .get(&request.root_process_id)
            .map(|reservation| reservation.descendant_count)
            .unwrap_or(0);
        let next = current
            .checked_add(u64::from(request.delta))
            .ok_or(ProcessJournalStoreError::ProcessTreeCapacityExceeded { cap: request.cap })?;
        if next > u64::from(request.cap) {
            return Err(ProcessJournalStoreError::ProcessTreeCapacityExceeded { cap: request.cap });
        }
        let released_processes = self
            .tree_reservations
            .get(&request.root_process_id)
            .map(|reservation| reservation.released_processes.clone())
            .unwrap_or_default();
        let reservation = ProcessTreeReservation {
            root_process_id: request.root_process_id,
            descendant_count: next,
            released_processes,
        };
        self.tree_reservations
            .insert(request.root_process_id, reservation.clone());
        Ok(StoredCommandOutcome::TreeReserved(reservation))
    }

    fn apply_release_tree(
        &mut self,
        request: crate::ReleaseProcessTreeRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        validate_tree_root(self, &request.scope, request.root_process_id)?;
        let Some(reservation) = self.tree_reservations.get_mut(&request.root_process_id) else {
            return Ok(StoredCommandOutcome::TreeReleased);
        };
        if reservation
            .released_processes
            .contains(&request.idempotency_process_id)
        {
            return Ok(StoredCommandOutcome::TreeReleased);
        }
        if reservation.descendant_count < u64::from(request.delta) {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "release delta exceeds current reservation count".to_string(),
            ));
        }
        reservation
            .released_processes
            .insert(request.idempotency_process_id);
        reservation.descendant_count -= u64::from(request.delta);
        if reservation.descendant_count == 0 {
            self.tree_reservations.remove(&request.root_process_id);
        }
        Ok(StoredCommandOutcome::TreeReleased)
    }

    fn apply_prune_tree(
        &mut self,
        request: crate::PruneReleasedProcessRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        if !self.processes.contains_key(&request.root_process_id) {
            return Ok(StoredCommandOutcome::TreePruned);
        }
        validate_tree_root(self, &request.scope, request.root_process_id)?;
        if let Some(reservation) = self.tree_reservations.get_mut(&request.root_process_id) {
            reservation.released_processes.remove(&request.process_id);
        }
        Ok(StoredCommandOutcome::TreePruned)
    }

    fn apply_open_dependency(
        &mut self,
        request: crate::OpenProcessDependencyRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let key = (request.dependent_process_id, request.dependency_process_id);
        if let Some(existing) = self.dependencies.get(&key) {
            return Ok(StoredCommandOutcome::Dependency(Some(existing.clone())));
        }
        let dependent = self.processes.get(&request.dependent_process_id).ok_or(
            ProcessJournalStoreError::UnknownProcess {
                process_id: request.dependent_process_id,
            },
        )?;
        if !same_lineage_scope(&dependent.scope, &request.scope) {
            return Err(ProcessJournalStoreError::UnauthorizedScope);
        }
        let expected_root = dependent.root_process_id.unwrap_or(dependent.process_id);
        if request.root_process_id != expected_root {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "dependency root process does not match dependent lineage".to_string(),
            ));
        }
        let record = crate::ProcessDependencyRecord {
            dependent_process_id: request.dependent_process_id,
            dependency_process_id: request.dependency_process_id,
            root_process_id: request.root_process_id,
            scope: request.scope,
            group_ref: request.group_ref,
            state: crate::ProcessDependencyState::Open,
            terminal: None,
            created_at: request.created_at,
            settled_at: None,
            consumed_at: None,
            transitioned_at: None,
            metadata: request.metadata,
        };
        self.dependencies.insert(key, record.clone());
        Ok(StoredCommandOutcome::Dependency(Some(record)))
    }

    fn apply_settle_dependency(
        &mut self,
        request: crate::SettleProcessDependencyRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        if !request.terminal.status.is_terminal() {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "dependency terminal evidence must carry a terminal process status".to_string(),
            ));
        }
        let key = (request.dependent_process_id, request.dependency_process_id);
        let Some(record) = self.dependencies.get_mut(&key) else {
            return Ok(StoredCommandOutcome::Dependency(None));
        };
        if !same_lineage_scope(&record.scope, &request.scope) {
            return Err(ProcessJournalStoreError::UnauthorizedScope);
        }
        if record.state != crate::ProcessDependencyState::Open {
            return Ok(StoredCommandOutcome::Dependency(Some(record.clone())));
        }
        record.state = crate::ProcessDependencyState::Settled;
        record.terminal = Some(request.terminal);
        record.settled_at = Some(request.settled_at);
        Ok(StoredCommandOutcome::Dependency(Some(record.clone())))
    }

    /// Compare-and-swap over one dependency's state column, gated by
    /// `ProcessDependencyState::legal_predecessors`.
    ///
    /// A transition that already landed replays as a success without
    /// re-applying its payload; a stored state that is not a legal predecessor
    /// of `next` is a typed error rather than a silent no-op, because either
    /// the caller's view of the lifecycle is wrong or it lost a race.
    fn apply_transition_dependency(
        &mut self,
        request: crate::TransitionProcessDependencyRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        // Closing an edge and releasing its descendant reservation are one
        // journal command (crate AGENTS.md). This CAS writes the state column
        // only, so letting it reach a terminal state would be a compensating
        // dual write that leaks the reservation.
        let terminal_door = match request.next {
            crate::ProcessDependencyState::Consumed => Some("consume_process_dependency"),
            crate::ProcessDependencyState::Abandoned => Some("abandon_process_dependency"),
            // Enumerated, not a wildcard: a new terminal variant must fail to
            // compile here, in the one file that owns the paired release.
            crate::ProcessDependencyState::Open
            | crate::ProcessDependencyState::Settled
            | crate::ProcessDependencyState::ResultAppended
            | crate::ProcessDependencyState::AttentionScheduled
            | crate::ProcessDependencyState::AttentionDeferred => None,
        };
        if let Some(door) = terminal_door {
            return Err(ProcessJournalStoreError::InvalidRequest(format!(
                "process dependency transition cannot close an edge: use {door}, which releases \
                 the descendant reservation in the same journal command"
            )));
        }
        // Validated before any mutation: a rejected command must leave the
        // materialized state exactly as it found it.
        let merged_metadata = match request.metadata {
            Some(serde_json::Value::Object(fields)) => Some(fields),
            Some(_) => {
                return Err(ProcessJournalStoreError::InvalidRequest(
                    "process dependency transition metadata must be a JSON object".to_string(),
                ));
            }
            None => None,
        };
        let key = (request.dependent_process_id, request.dependency_process_id);
        let Some(record) = self.dependencies.get_mut(&key) else {
            return Ok(StoredCommandOutcome::Dependency(None));
        };
        if !same_lineage_scope(&record.scope, &request.scope) {
            return Err(ProcessJournalStoreError::UnauthorizedScope);
        }
        // A double-drive of the same step lands here; a racer that arrives
        // late finds a state the relation below refuses.
        if record.state == request.next {
            return Ok(StoredCommandOutcome::Dependency(Some(record.clone())));
        }
        let legal = request.next.legal_predecessors();
        if !legal.contains(&record.state) {
            return Err(ProcessJournalStoreError::InvalidRequest(format!(
                "process dependency transition to {:?} is legal only from {:?}, but the \
                 dependency is {:?}",
                request.next, legal, record.state
            )));
        }
        record.state = request.next;
        record.transitioned_at = Some(request.transitioned_at);
        // Shallow merge: a delivery step records its own facts without
        // discarding the ones the edge already carries.
        if let Some(fields) = merged_metadata {
            match record.metadata.as_object_mut() {
                Some(existing) => existing.extend(fields),
                None => record.metadata = serde_json::Value::Object(fields),
            }
        }
        Ok(StoredCommandOutcome::Dependency(Some(record.clone())))
    }

    fn apply_close_dependency(
        &mut self,
        request: crate::CloseProcessDependencyRequest,
        abandon: bool,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let key = (request.dependent_process_id, request.dependency_process_id);
        let Some(existing) = self.dependencies.get(&key) else {
            return Ok(StoredCommandOutcome::Dependency(None));
        };
        if !same_lineage_scope(&existing.scope, &request.scope) {
            return Err(ProcessJournalStoreError::UnauthorizedScope);
        }
        let target = if abandon {
            crate::ProcessDependencyState::Abandoned
        } else {
            crate::ProcessDependencyState::Consumed
        };
        if existing.state.is_closed() {
            return Ok(StoredCommandOutcome::Dependency(Some(existing.clone())));
        }
        // The same relation the state-column CAS reads: one lifecycle, both
        // closing doors. Abandoning is legal from every in-flight state;
        // consuming is not, because `ResultAppended` has no attention recorded
        // yet and `AttentionDeferred` is parked for a later sweep.
        let legal = target.legal_predecessors();
        if !legal.contains(&existing.state) {
            return Err(ProcessJournalStoreError::InvalidRequest(format!(
                "a process dependency can only enter {:?} from {:?}, but it is {:?}",
                target, legal, existing.state
            )));
        }
        let root_process_id = existing.root_process_id;
        self.release_dependency_reservation(root_process_id, request.dependency_process_id)?;
        let record = self.dependencies.get_mut(&key).ok_or_else(|| {
            ProcessJournalStoreError::InvalidRequest(
                "process dependency disappeared while closing".to_string(),
            )
        })?;
        record.state = target;
        record.consumed_at = Some(request.closed_at);
        Ok(StoredCommandOutcome::Dependency(Some(record.clone())))
    }

    fn release_dependency_reservation(
        &mut self,
        root_process_id: ProcessId,
        dependency_process_id: ProcessId,
    ) -> Result<(), ProcessJournalStoreError> {
        let Some(reservation) = self.tree_reservations.get_mut(&root_process_id) else {
            return Ok(());
        };
        if reservation
            .released_processes
            .contains(&dependency_process_id)
        {
            return Ok(());
        }
        if reservation.descendant_count == 0 {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "dependency release exceeds current reservation count".to_string(),
            ));
        }
        reservation.released_processes.insert(dependency_process_id);
        reservation.descendant_count -= 1;
        if reservation.descendant_count == 0 {
            self.tree_reservations.remove(&root_process_id);
        }
        Ok(())
    }

    fn apply_checkpoint(
        &mut self,
        request: crate::RecordProcessCheckpointRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let link_to_process = request.link_to_process;
        let snapshot = self.processes.get(&request.process_id).ok_or(
            ProcessJournalStoreError::UnknownProcess {
                process_id: request.process_id,
            },
        )?;
        if !process_scope_visible(&snapshot.scope, &request.scope) {
            return Err(ProcessJournalStoreError::UnknownProcess {
                process_id: request.process_id,
            });
        }
        let record = ProcessCheckpointRecord {
            checkpoint_id: request.checkpoint_id.clone(),
            process_id: request.process_id,
            scope: request.scope,
            state_ref: request.state_ref,
            payload: request.payload,
            created_at: request.created_at,
            metadata: request.metadata,
        };
        if let Some(existing) = self.checkpoints.get(&request.checkpoint_id) {
            return if existing == &record {
                Ok(StoredCommandOutcome::Checkpointed(existing.clone()))
            } else {
                Err(ProcessJournalStoreError::InvalidRequest(format!(
                    "process checkpoint {} already exists",
                    request.checkpoint_id.as_str()
                )))
            };
        }
        self.checkpoints
            .insert(request.checkpoint_id.clone(), record.clone());
        if link_to_process {
            let snapshot = self.process_mut(request.process_id)?;
            snapshot.checkpoint_ref = Some(ProcessCheckpointRef::from_trusted(
                request.checkpoint_id.as_str().to_string(),
            ));
            snapshot.checkpoint_kind = request.kind;
        }
        Ok(StoredCommandOutcome::Checkpointed(record))
    }

    pub(super) fn next_cursor(&mut self) -> ProcessJournalCursor {
        let cursor = ProcessJournalCursor(self.next_cursor);
        self.next_cursor = self.next_cursor.saturating_add(1);
        cursor
    }

    pub(super) fn push_entry(&mut self, entry: ProcessJournalEntry) {
        self.journal.push(entry);
    }

    pub(super) fn remember_control_result(
        &mut self,
        key: Option<String>,
        result: ProcessControlResult,
    ) {
        let Some(key) = key else {
            return;
        };
        if let Some(existing) = self.control_idempotency.get_mut(&key) {
            *existing = result;
            return;
        }
        while self.control_idempotency_order.len() >= MAX_IDEMPOTENCY_RECORDS {
            let Some(oldest) = self.control_idempotency_order.pop_front() else {
                self.control_idempotency.clear();
                break;
            };
            self.control_idempotency.remove(&oldest);
        }
        self.control_idempotency_order.push_back(key.clone());
        self.control_idempotency.insert(key, result);
    }

    pub(super) fn remember_submission_result(
        &mut self,
        key: Option<String>,
        snapshot: JournaledProcessSnapshot,
    ) {
        let Some(key) = key else {
            return;
        };
        if let Some(existing) = self.submission_idempotency.get_mut(&key) {
            *existing = snapshot;
            return;
        }
        while self.submission_idempotency_order.len() >= MAX_IDEMPOTENCY_RECORDS {
            let Some(oldest) = self.submission_idempotency_order.pop_front() else {
                self.submission_idempotency.clear();
                break;
            };
            self.submission_idempotency.remove(&oldest);
        }
        self.submission_idempotency_order.push_back(key.clone());
        self.submission_idempotency.insert(key, snapshot);
    }

    /// Remembers which process an edge submission's operation id was applied
    /// to, in memory only.
    ///
    /// Edge submissions do not persist full-snapshot idempotency records: the
    /// durable process row is the replay authority (see
    /// [`Self::apply_submit_at_edge`]). This compact op-id → process-id binding
    /// keeps rejecting same-operation replays with a different process id
    /// without re-serializing a full snapshot per record on every journal
    /// command. It is bounded like the persisted idempotency maps; losing it
    /// across a restart only weakens the cross-process rejection, never replay
    /// safety (the process row still answers identical replays).
    pub(super) fn remember_edge_submission(&mut self, key: String, process_id: ProcessId) {
        while self.edge_submission_bindings_order.len() >= MAX_IDEMPOTENCY_RECORDS {
            let Some(oldest) = self.edge_submission_bindings_order.pop_front() else {
                self.edge_submission_bindings.clear();
                break;
            };
            self.edge_submission_bindings.remove(&oldest);
        }
        self.edge_submission_bindings_order.push_back(key.clone());
        self.edge_submission_bindings.insert(key, process_id);
    }

    pub(super) fn process_mut(
        &mut self,
        process_id: ProcessId,
    ) -> Result<&mut JournaledProcessSnapshot, ProcessJournalStoreError> {
        self.processes
            .get_mut(&process_id)
            .ok_or(ProcessJournalStoreError::UnknownProcess { process_id })
    }

    pub(super) fn claimable_process_ids(
        &self,
        scope_filter: Option<&ResourceScope>,
        process_id_filter: Option<ProcessId>,
        process_kind_filter: Option<&ProcessKind>,
    ) -> Vec<ProcessId> {
        let mut ids = self
            .processes
            .values()
            .filter(|snapshot| snapshot.status == ProcessLifecycleStatus::Queued)
            .filter(|snapshot| {
                process_id_filter.is_none_or(|process_id| snapshot.process_id == process_id)
            })
            .filter(|snapshot| {
                process_kind_filter.is_none_or(|kind| snapshot.process_kind == *kind)
            })
            .filter(|snapshot| {
                scope_filter.is_none_or(|scope| same_scope_owner(&snapshot.scope, scope))
            })
            .map(|snapshot| (snapshot.created_at, snapshot.process_id))
            .collect::<Vec<_>>();
        ids.sort_by_key(|(created_at, process_id)| (*created_at, process_id.as_uuid()));
        ids.into_iter().map(|(_, process_id)| process_id).collect()
    }

    pub(super) fn expired_process_ids(
        &self,
        scope_filter: Option<&ResourceScope>,
        process_kind_filter: Option<&ProcessKind>,
        now: ironclaw_host_api::Timestamp,
    ) -> Vec<ProcessId> {
        self.processes
            .values()
            .filter(|snapshot| {
                matches!(
                    snapshot.status,
                    ProcessLifecycleStatus::Running | ProcessLifecycleStatus::CancelRequested
                )
            })
            .filter(|snapshot| {
                scope_filter.is_none_or(|scope| same_scope_owner(&snapshot.scope, scope))
            })
            .filter(|snapshot| {
                process_kind_filter.is_none_or(|kind| snapshot.process_kind == *kind)
            })
            .filter(|snapshot| {
                snapshot
                    .lease
                    .as_ref()
                    .and_then(|lease| lease.lease_expires_at)
                    .is_some_and(|expires_at| expires_at <= now)
            })
            .map(|snapshot| snapshot.process_id)
            .collect()
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
