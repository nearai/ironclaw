use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};

use ironclaw_filesystem::RecordVersion;
use tokio::sync::RwLock;

use crate::{
    EventCursor, TurnError, TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnRunState,
    TurnStatus, runner::HeartbeatRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunnerLeaseRecord {
    run_id: TurnRunId,
    runner_id: crate::TurnRunnerId,
    lease_token: crate::TurnLeaseToken,
    lease_expires_at: crate::TurnTimestamp,
    last_heartbeat_at: crate::TurnTimestamp,
    status: TurnStatus,
    event_cursor: EventCursor,
}

#[derive(Clone, Copy)]
pub(super) enum RunnerLeaseOverlay {
    None,
    Run(TurnRunId),
    All,
}

pub(super) type RunnerLeaseMemory = Arc<RwLock<HashMap<TurnRunId, RunnerLeaseRecord>>>;

pub(super) struct RunnerLeaseStore {
    leases: RunnerLeaseMemory,
    runner_lease_ttl: chrono::Duration,
    apply_timeout: Duration,
}

impl RunnerLeaseStore {
    pub(super) fn new(
        leases: RunnerLeaseMemory,
        runner_lease_ttl: chrono::Duration,
        apply_timeout: Duration,
    ) -> Self {
        Self {
            leases,
            runner_lease_ttl,
            apply_timeout,
        }
    }

    pub(super) async fn overlay(
        &self,
        snapshot: (TurnPersistenceSnapshot, Option<RecordVersion>),
        overlay: RunnerLeaseOverlay,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        match overlay {
            RunnerLeaseOverlay::None => Ok(snapshot),
            RunnerLeaseOverlay::Run(run_id) => {
                self.with_timeout(
                    self.overlay_run_inner(snapshot, run_id),
                    "overlay run lease",
                )
                .await
            }
            RunnerLeaseOverlay::All => {
                self.with_timeout(self.overlay_snapshot_inner(snapshot), "overlay leases")
                    .await
            }
        }
    }

    pub(super) async fn overlay_run_record(
        &self,
        run: TurnRunRecord,
    ) -> Result<TurnRunRecord, TurnError> {
        self.with_timeout(
            self.overlay_run_record_inner(run),
            "overlay run record lease",
        )
        .await
    }

    pub(super) async fn seed_from_snapshot(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        self.with_timeout(
            self.seed_from_snapshot_inner(snapshot, run_id),
            "seed runner lease",
        )
        .await
    }

    pub(super) async fn seed_from_run_record(&self, run: TurnRunRecord) -> Result<(), TurnError> {
        self.with_timeout(
            self.seed_from_run_record_inner(run),
            "seed runner lease from run record",
        )
        .await
    }

    pub(super) async fn seed_from_snapshot_if_missing(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        self.with_timeout(
            self.seed_from_snapshot_if_missing_inner(snapshot, run_id),
            "seed missing runner lease",
        )
        .await
    }

    pub(super) async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<EventCursor, TurnError> {
        self.with_timeout(self.heartbeat_inner(request), "heartbeat runner lease")
            .await
    }

    pub(super) async fn mark_cancel_requested_from_snapshot(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        self.with_timeout(
            self.write_status_from_snapshot(snapshot, run_id, None, TurnStatus::CancelRequested),
            "mark runner lease cancel requested",
        )
        .await
    }

    pub(super) async fn retire_runner_lease_from_snapshot(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        self.with_timeout(
            self.write_status_from_snapshot(
                snapshot,
                run_id,
                Some((runner_id, lease_token)),
                retired_status,
            ),
            "retire runner lease",
        )
        .await
    }

    pub(super) async fn retire_runner_lease_from_run_record(
        &self,
        run: TurnRunRecord,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        self.with_timeout(
            self.write_status_from_run_record(run, Some((runner_id, lease_token)), retired_status),
            "retire runner lease",
        )
        .await
    }

    pub(super) async fn restore_if_current_status(
        &self,
        previous: RunnerLeaseRecord,
        current_status: TurnStatus,
    ) {
        self.best_effort_with_timeout(
            self.restore_if_current_status_inner(previous, current_status),
            "restore runner lease",
        )
        .await;
    }

    pub(super) async fn cleanup_after_state(&self, result: &Result<TurnRunState, TurnError>) {
        self.best_effort_unit_with_timeout(
            self.cleanup_after_state_inner(result),
            "cleanup runner lease",
        )
        .await;
    }

    pub(super) async fn delete_best_effort(&self, run_id: TurnRunId) {
        self.best_effort_unit_with_timeout(
            self.delete_best_effort_inner(run_id),
            "delete runner lease",
        )
        .await;
    }

    async fn with_timeout<T, Fut>(
        &self,
        future: Fut,
        operation: &'static str,
    ) -> Result<T, TurnError>
    where
        Fut: Future<Output = Result<T, TurnError>>,
    {
        match tokio::time::timeout(self.apply_timeout, future).await {
            Ok(result) => result,
            Err(_) => Err(TurnError::Unavailable {
                reason: format!("turn runner lease {operation} timed out"),
            }),
        }
    }

    async fn best_effort_with_timeout<Fut>(&self, future: Fut, operation: &'static str)
    where
        Fut: Future<Output = Result<(), TurnError>>,
    {
        match tokio::time::timeout(self.apply_timeout, future).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(%error, operation, "turn runner lease best-effort operation failed");
            }
            Err(_) => {
                tracing::debug!(
                    operation,
                    "turn runner lease best-effort operation timed out"
                );
            }
        }
    }

    async fn best_effort_unit_with_timeout<Fut>(&self, future: Fut, operation: &'static str)
    where
        Fut: Future<Output = ()>,
    {
        match tokio::time::timeout(self.apply_timeout, future).await {
            Ok(()) => {}
            Err(_) => {
                tracing::debug!(
                    operation,
                    "turn runner lease best-effort operation timed out"
                );
            }
        }
    }

    async fn overlay_snapshot_inner(
        &self,
        snapshot: (TurnPersistenceSnapshot, Option<RecordVersion>),
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let (mut snapshot, version) = snapshot;
        let leases = self.leases.read().await;
        for run in snapshot
            .runs
            .iter_mut()
            .filter(|record| run_can_use_external_lease(record))
        {
            let Some(lease) = leases.get(&run.run_id) else {
                continue;
            };
            apply_runner_lease_overlay(run, lease);
        }
        Ok((snapshot, version))
    }

    async fn overlay_run_inner(
        &self,
        snapshot: (TurnPersistenceSnapshot, Option<RecordVersion>),
        run_id: TurnRunId,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let (mut snapshot, version) = snapshot;
        let Some(run) = snapshot
            .runs
            .iter_mut()
            .find(|record| record.run_id == run_id && run_can_use_external_lease(record))
        else {
            return Ok((snapshot, version));
        };
        let leases = self.leases.read().await;
        let Some(lease) = leases.get(&run.run_id) else {
            return Ok((snapshot, version));
        };
        apply_runner_lease_overlay(run, lease);
        Ok((snapshot, version))
    }

    async fn overlay_run_record_inner(
        &self,
        mut run: TurnRunRecord,
    ) -> Result<TurnRunRecord, TurnError> {
        if !run_can_use_external_lease(&run) {
            return Ok(run);
        }
        let leases = self.leases.read().await;
        if let Some(lease) = leases.get(&run.run_id) {
            apply_runner_lease_overlay(&mut run, lease);
        }
        Ok(run)
    }

    async fn seed_from_snapshot_inner(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let Some(run) = snapshot.runs.iter().find(|record| record.run_id == run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        let Some(record) = runner_lease_from_run(run) else {
            return Err(TurnError::InvalidTransition {
                from: run.status,
                to: TurnStatus::Running,
            });
        };
        self.upsert(record).await
    }

    async fn seed_from_run_record_inner(&self, run: TurnRunRecord) -> Result<(), TurnError> {
        let Some(record) = runner_lease_from_run(&run) else {
            return Err(TurnError::InvalidTransition {
                from: run.status,
                to: TurnStatus::Running,
            });
        };
        self.upsert(record).await
    }

    async fn seed_from_snapshot_if_missing_inner(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let mut leases = self.leases.write().await;
        if leases.contains_key(&run_id) {
            return Ok(());
        }
        let record = runner_lease_from_snapshot(snapshot, run_id)?;
        leases.insert(record.run_id, record);
        Ok(())
    }

    async fn heartbeat_inner(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        let now = chrono::Utc::now();
        let mut leases = self.leases.write().await;
        let Some(existing) = leases.get_mut(&request.run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        ensure_active_runner_lease(existing, request.runner_id, request.lease_token, now)?;
        if existing.status != TurnStatus::Running {
            return Err(TurnError::InvalidTransition {
                from: existing.status,
                to: TurnStatus::Running,
            });
        }
        let event_cursor = existing.event_cursor;
        existing.lease_expires_at = next_lease_expiry(self.runner_lease_ttl, now);
        existing.last_heartbeat_at = now;
        Ok(event_cursor)
    }

    async fn restore_if_current_status_inner(
        &self,
        previous: RunnerLeaseRecord,
        current_status: TurnStatus,
    ) -> Result<(), TurnError> {
        let mut leases = self.leases.write().await;
        let Some(current) = leases.get(&previous.run_id) else {
            return Ok(());
        };
        if current.runner_id != previous.runner_id
            || current.lease_token != previous.lease_token
            || current.status != current_status
        {
            return Ok(());
        }
        leases.insert(previous.run_id, previous);
        Ok(())
    }

    async fn cleanup_after_state_inner(&self, result: &Result<TurnRunState, TurnError>) {
        if let Ok(state) = result
            && state.status.is_terminal()
        {
            self.delete_best_effort_inner(state.run_id).await;
        }
    }

    async fn delete_best_effort_inner(&self, run_id: TurnRunId) {
        self.delete(run_id).await;
    }

    async fn upsert(&self, record: RunnerLeaseRecord) -> Result<(), TurnError> {
        self.leases.write().await.insert(record.run_id, record);
        Ok(())
    }

    async fn write_status_from_snapshot(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
        expected_runner: Option<(crate::TurnRunnerId, crate::TurnLeaseToken)>,
        status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let fallback = runner_lease_from_snapshot(snapshot, run_id)?;
        self.write_status_from_fallback(fallback, run_id, expected_runner, status)
            .await
    }

    async fn write_status_from_run_record(
        &self,
        run: TurnRunRecord,
        expected_runner: Option<(crate::TurnRunnerId, crate::TurnLeaseToken)>,
        status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let run_id = run.run_id;
        let from = run.status;
        let fallback = runner_lease_from_run(&run).ok_or(TurnError::InvalidTransition {
            from,
            to: TurnStatus::Running,
        })?;
        self.write_status_from_fallback(fallback, run_id, expected_runner, status)
            .await
    }

    async fn write_status_from_fallback(
        &self,
        fallback: RunnerLeaseRecord,
        run_id: TurnRunId,
        expected_runner: Option<(crate::TurnRunnerId, crate::TurnLeaseToken)>,
        status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let mut leases = self.leases.write().await;
        let existing = leases.get(&run_id).cloned().unwrap_or(fallback);
        if let Some((runner_id, lease_token)) = expected_runner {
            ensure_active_runner_lease(&existing, runner_id, lease_token, chrono::Utc::now())?;
        }
        if existing.status == status {
            return Ok(None);
        }
        if !matches!(
            existing.status,
            TurnStatus::Running | TurnStatus::CancelRequested
        ) {
            return Err(TurnError::InvalidTransition {
                from: existing.status,
                to: status,
            });
        }
        let mut next = existing.clone();
        next.status = status;
        leases.insert(run_id, next);
        Ok(Some(existing))
    }

    async fn delete(&self, run_id: TurnRunId) {
        self.leases.write().await.remove(&run_id);
    }
}

fn next_lease_expiry(
    runner_lease_ttl: chrono::Duration,
    now: crate::TurnTimestamp,
) -> crate::TurnTimestamp {
    now.checked_add_signed(runner_lease_ttl).unwrap_or(now)
}

fn run_can_use_external_lease(record: &TurnRunRecord) -> bool {
    matches!(
        record.status,
        TurnStatus::Running | TurnStatus::CancelRequested
    ) && record.runner_id.is_some()
        && record.lease_token.is_some()
}

fn runner_lease_from_run(record: &TurnRunRecord) -> Option<RunnerLeaseRecord> {
    if !run_can_use_external_lease(record) {
        return None;
    }
    Some(RunnerLeaseRecord {
        run_id: record.run_id,
        runner_id: record.runner_id?,
        lease_token: record.lease_token?,
        lease_expires_at: record.lease_expires_at?,
        last_heartbeat_at: record.last_heartbeat_at?,
        status: record.status,
        event_cursor: record.event_cursor,
    })
}

fn runner_lease_from_snapshot(
    snapshot: &TurnPersistenceSnapshot,
    run_id: TurnRunId,
) -> Result<RunnerLeaseRecord, TurnError> {
    let Some(run) = snapshot.runs.iter().find(|record| record.run_id == run_id) else {
        return Err(TurnError::ScopeNotFound);
    };
    runner_lease_from_run(run).ok_or(TurnError::InvalidTransition {
        from: run.status,
        to: TurnStatus::Running,
    })
}

fn apply_runner_lease_overlay(record: &mut TurnRunRecord, lease: &RunnerLeaseRecord) {
    if record.run_id != lease.run_id
        || record.runner_id != Some(lease.runner_id)
        || record.lease_token != Some(lease.lease_token)
        || !run_can_use_external_lease(record)
    {
        return;
    }
    if record
        .last_heartbeat_at
        .is_some_and(|last_heartbeat_at| lease.last_heartbeat_at < last_heartbeat_at)
    {
        return;
    }
    record.last_heartbeat_at = Some(lease.last_heartbeat_at);
    record.lease_expires_at = Some(lease.lease_expires_at);
}

fn ensure_active_runner_lease(
    record: &RunnerLeaseRecord,
    runner_id: crate::TurnRunnerId,
    lease_token: crate::TurnLeaseToken,
    now: crate::TurnTimestamp,
) -> Result<(), TurnError> {
    if record.runner_id != runner_id || record.lease_token != lease_token {
        return Err(TurnError::LeaseMismatch);
    }
    if record.lease_expires_at <= now {
        return Err(TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AcceptedMessageRef, ReplyTargetBindingRef, SourceBindingRef, TurnId, TurnLeaseToken,
        TurnRunnerId, TurnScope,
    };
    use chrono::Utc;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

    #[tokio::test]
    async fn seed_from_snapshot_if_missing_does_not_overwrite_existing_lease() {
        let run_id = TurnRunId::new();
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        let now = Utc::now();
        let snapshot = TurnPersistenceSnapshot {
            runs: vec![turn_run_record(
                run_id,
                runner_id,
                lease_token,
                TurnStatus::Running,
                now,
                now,
                EventCursor(1),
            )],
            ..TurnPersistenceSnapshot::default()
        };
        let existing = RunnerLeaseRecord {
            run_id,
            runner_id,
            lease_token,
            lease_expires_at: now + chrono::Duration::minutes(2),
            last_heartbeat_at: now + chrono::Duration::seconds(5),
            status: TurnStatus::CancelRequested,
            event_cursor: EventCursor(2),
        };
        let store = RunnerLeaseStore::new(
            Arc::new(RwLock::new(HashMap::new())),
            chrono::Duration::minutes(1),
            Duration::from_secs(1),
        );
        store.upsert(existing.clone()).await.unwrap();

        store
            .seed_from_snapshot_if_missing(&snapshot, run_id)
            .await
            .unwrap();

        let stored = store
            .leases
            .read()
            .await
            .get(&run_id)
            .cloned()
            .expect("existing lease");
        assert_eq!(stored, existing);
    }

    #[tokio::test]
    async fn seed_from_run_record_uses_exact_persisted_lease_metadata() {
        let run_id = TurnRunId::new();
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        let now = Utc::now();
        let lease_expires_at = now + chrono::Duration::minutes(2);
        let last_heartbeat_at = now + chrono::Duration::seconds(7);
        let event_cursor = EventCursor(7);
        let run = turn_run_record(
            run_id,
            runner_id,
            lease_token,
            TurnStatus::Running,
            lease_expires_at,
            last_heartbeat_at,
            event_cursor,
        );
        let store = RunnerLeaseStore::new(
            Arc::new(RwLock::new(HashMap::new())),
            chrono::Duration::minutes(1),
            Duration::from_secs(1),
        );

        store.seed_from_run_record(run).await.unwrap();

        let stored = store
            .leases
            .read()
            .await
            .get(&run_id)
            .cloned()
            .expect("seeded lease");
        assert_eq!(stored.run_id, run_id);
        assert_eq!(stored.runner_id, runner_id);
        assert_eq!(stored.lease_token, lease_token);
        assert_eq!(stored.lease_expires_at, lease_expires_at);
        assert_eq!(stored.last_heartbeat_at, last_heartbeat_at);
        assert_eq!(stored.status, TurnStatus::Running);
        assert_eq!(stored.event_cursor, event_cursor);
    }

    fn turn_run_record(
        run_id: TurnRunId,
        runner_id: TurnRunnerId,
        lease_token: TurnLeaseToken,
        status: TurnStatus,
        lease_expires_at: crate::TurnTimestamp,
        last_heartbeat_at: crate::TurnTimestamp,
        event_cursor: EventCursor,
    ) -> TurnRunRecord {
        let profile: crate::TurnRunProfile = serde_json::from_value(serde_json::json!({
            "id": "default",
            "version": 1,
            "allow_steering": false,
            "auto_queue_followups": false,
        }))
        .expect("profile deserialization");
        TurnRunRecord {
            run_id,
            turn_id: TurnId::new(),
            scope: TurnScope::new(
                TenantId::new("tenant-runner-lease-test").unwrap(),
                Some(AgentId::new("agent-runner-lease-test").unwrap()),
                Some(ProjectId::new("project-runner-lease-test").unwrap()),
                ThreadId::new("thread-runner-lease-test").unwrap(),
            ),
            accepted_message_ref: AcceptedMessageRef::new("accepted-runner-lease-test").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-runner-lease-test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-runner-lease-test")
                .unwrap(),
            status,
            profile,
            resolved_model_route: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: vec![],
            failure: None,
            event_cursor,
            runner_id: Some(runner_id),
            lease_token: Some(lease_token),
            lease_expires_at: Some(lease_expires_at),
            last_heartbeat_at: Some(last_heartbeat_at),
            claim_count: 1,
            received_at: last_heartbeat_at,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }
}
