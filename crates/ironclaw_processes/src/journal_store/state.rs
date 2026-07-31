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
    ClaimedProcess, JournaledProcessSnapshot, ProcessCheckpointId, ProcessCheckpointRecord,
    ProcessCheckpointRef, ProcessControlResult, ProcessFailureRecovery, ProcessInputRecord,
    ProcessJournalCursor, ProcessJournalEntry, ProcessJournalKind, ProcessKind,
    ProcessLeaseSnapshot, ProcessLeaseToken, ProcessLifecycleStatus, ProcessTreeReservation,
    RecoverExpiredProcessLeasesResponse, types::same_scope_owner,
};

const MAX_IDEMPOTENCY_RECORDS: usize = 4096;
/// Maximum number of crash-recovery claims allowed before a checkpointless
/// process is failed terminally.
pub const MAX_CRASH_RECOVERY_RECLAIMS: u64 = 3;

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
            StoredProcessCommand::RecoverExpired(request) => self.apply_recover_expired(request),
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
                    metadata: dependency.metadata,
                },
            );
        }
        self.remember_submission_result(replay_key, snapshot.clone());
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
        let cursor = self.next_cursor();
        let snapshot = self.process_mut(request.process_id)?;
        ensure_transition(snapshot, ProcessLifecycleStatus::Running)?;
        ensure_lease(snapshot, &request.worker_id, &request.lease_token)?;
        if let Some(lease) = &mut snapshot.lease {
            lease.last_heartbeat_at = Some(now);
            lease.lease_expires_at = chrono::Duration::from_std(lease_duration)
                .ok()
                .map(|duration| now + duration);
        }
        snapshot.journal_cursor = cursor;
        let snapshot = snapshot.clone();
        self.push_entry(ProcessJournalEntry::from_snapshot(
            &snapshot,
            cursor,
            ProcessJournalKind::Heartbeat,
        ));
        Ok(StoredCommandOutcome::Heartbeat(snapshot))
    }

    fn apply_recover_expired(
        &mut self,
        request: crate::RecoverExpiredProcessLeasesRequest,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        let expired = self.expired_process_ids(
            request.scope_filter.as_ref(),
            request.process_kind_filter.as_ref(),
            request.now,
        );
        let mut recovered = Vec::new();
        for process_id in expired {
            let cursor = self.next_cursor();
            let snapshot = self.process_mut(process_id)?;
            let claim_count = snapshot.lease.as_ref().map_or(0, |lease| lease.claim_count);
            let (status, kind, failure) = if snapshot.status
                == ProcessLifecycleStatus::CancelRequested
            {
                (
                    ProcessLifecycleStatus::Cancelled,
                    ProcessJournalKind::Cancelled,
                    None,
                )
            } else if snapshot.checkpoint_ref.is_none() && claim_count < MAX_CRASH_RECOVERY_RECLAIMS
            {
                (
                    ProcessLifecycleStatus::Queued,
                    ProcessJournalKind::Resumed,
                    None,
                )
            } else {
                let category = if snapshot.checkpoint_ref.is_some() {
                    "lease_expired"
                } else {
                    "crash_retry_exhausted"
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
        if mutation.checkpoint_ref.is_some() {
            snapshot.checkpoint_ref = mutation.checkpoint_ref;
        }
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
            return Ok(StoredCommandOutcome::Controlled(result.clone(), None));
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
            return Ok(StoredCommandOutcome::Controlled(result, None));
        };
        if status == snapshot.status {
            let result = ProcessControlResult {
                state: snapshot.clone(),
                changed: false,
                already_terminal,
            };
            self.remember_control_result(replay_key, result.clone());
            return Ok(StoredCommandOutcome::Controlled(result, None));
        }
        let cursor = self.next_cursor();
        let snapshot = self.process_mut(mutation.process_id)?;
        snapshot.status = status;
        snapshot.suspension = None;
        if mutation.checkpoint_ref.is_some() {
            snapshot.checkpoint_ref = mutation.checkpoint_ref;
        }
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
        Ok(StoredCommandOutcome::Controlled(result, Some(kind)))
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
        if matches!(
            existing.state,
            crate::ProcessDependencyState::Consumed | crate::ProcessDependencyState::Abandoned
        ) {
            return Ok(StoredCommandOutcome::Dependency(Some(existing.clone())));
        }
        if !abandon && existing.state != crate::ProcessDependencyState::Settled {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "only a settled process dependency can be consumed".to_string(),
            ));
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
            self.process_mut(request.process_id)?.checkpoint_ref = Some(
                ProcessCheckpointRef::from_trusted(request.checkpoint_id.as_str().to_string()),
            );
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
