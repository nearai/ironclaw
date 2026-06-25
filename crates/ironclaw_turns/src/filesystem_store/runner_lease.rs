use std::{future::Future, sync::Arc, time::Duration};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordKind, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};

use crate::{
    EventCursor, TurnError, TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnRunState,
    TurnStatus,
    filesystem_store::io::{
        FILESYSTEM_CAS_RETRIES, PutError, cas_retry_backoff, fs_error, put_with_cas,
    },
    runner::HeartbeatRequest,
};

const TURNS_RUNNER_LEASE_PREFIX: &str = "/turns/runner-leases";
const TURNS_RUNNER_LEASE_KIND: &str = "turn_runner_lease";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

pub(super) struct RunnerLeaseSidecar<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    runner_lease_ttl: chrono::Duration,
    apply_timeout: Duration,
}

impl<F> RunnerLeaseSidecar<F>
where
    F: RootFilesystem,
{
    pub(super) fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        runner_lease_ttl: chrono::Duration,
        apply_timeout: Duration,
    ) -> Self {
        Self {
            filesystem,
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
        for run in snapshot
            .runs
            .iter_mut()
            .filter(|record| run_can_use_external_lease(record))
        {
            let Some((lease, _version)) = self.read(run.run_id).await? else {
                continue;
            };
            apply_runner_lease_overlay(run, &lease);
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
        let Some((lease, _version)) = self.read(run.run_id).await? else {
            return Ok((snapshot, version));
        };
        apply_runner_lease_overlay(run, &lease);
        Ok((snapshot, version))
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

    async fn seed_from_snapshot_if_missing_inner(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        if self.read(run_id).await?.is_some() {
            return Ok(());
        }
        self.seed_from_snapshot_inner(snapshot, run_id).await
    }

    async fn heartbeat_inner(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        for attempt in 0..FILESYSTEM_CAS_RETRIES {
            let now = chrono::Utc::now();
            let Some((existing, version)) = self.read(request.run_id).await? else {
                return Err(TurnError::ScopeNotFound);
            };
            ensure_active_runner_lease(&existing, request.runner_id, request.lease_token, now)?;
            if existing.status != TurnStatus::Running {
                return Err(TurnError::InvalidTransition {
                    from: existing.status,
                    to: TurnStatus::Running,
                });
            }
            let mut next = existing.clone();
            next.lease_expires_at = next_lease_expiry(self.runner_lease_ttl, now);
            next.last_heartbeat_at = now;
            match self.write(&next, CasExpectation::Version(version)).await {
                Ok(_) => return Ok(existing.event_cursor),
                Err(PutError::VersionMismatch) => cas_retry_backoff(attempt).await,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(TurnError::Unavailable {
            reason: "turn runner lease CAS retries exhausted".to_string(),
        })
    }

    async fn restore_if_current_status_inner(
        &self,
        previous: RunnerLeaseRecord,
        current_status: TurnStatus,
    ) -> Result<(), TurnError> {
        for attempt in 0..FILESYSTEM_CAS_RETRIES {
            let Some((current, version)) = self.read(previous.run_id).await? else {
                return Ok(());
            };
            if current.runner_id != previous.runner_id
                || current.lease_token != previous.lease_token
                || current.status != current_status
            {
                return Ok(());
            }
            match self
                .write(&previous, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(PutError::VersionMismatch) => cas_retry_backoff(attempt).await,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(TurnError::Unavailable {
            reason: "turn runner lease CAS retries exhausted".to_string(),
        })
    }

    async fn cleanup_after_state_inner(&self, result: &Result<TurnRunState, TurnError>) {
        if let Ok(state) = result
            && state.status.is_terminal()
        {
            self.delete_best_effort_inner(state.run_id).await;
        }
    }

    async fn delete_best_effort_inner(&self, run_id: TurnRunId) {
        match self.delete(run_id).await {
            Ok(()) | Err(TurnError::ScopeNotFound) => {}
            Err(error) => {
                tracing::debug!(
                    run_id = %run_id,
                    error = %error,
                    "failed to delete runner lease sidecar after run left runner-owned state"
                );
            }
        }
    }

    async fn upsert(&self, record: RunnerLeaseRecord) -> Result<(), TurnError> {
        for attempt in 0..FILESYSTEM_CAS_RETRIES {
            let cas = match self.read(record.run_id).await? {
                Some((_existing, version)) => CasExpectation::Version(version),
                None => CasExpectation::Absent,
            };
            match self.write(&record, cas).await {
                Ok(_) => return Ok(()),
                Err(PutError::VersionMismatch) => cas_retry_backoff(attempt).await,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(TurnError::Unavailable {
            reason: "turn runner lease CAS retries exhausted".to_string(),
        })
    }

    async fn write_status_from_snapshot(
        &self,
        snapshot: &TurnPersistenceSnapshot,
        run_id: TurnRunId,
        expected_runner: Option<(crate::TurnRunnerId, crate::TurnLeaseToken)>,
        status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let fallback = runner_lease_from_snapshot(snapshot, run_id)?;
        for attempt in 0..FILESYSTEM_CAS_RETRIES {
            let (existing, cas) = match self.read(run_id).await? {
                Some((existing, version)) => (existing, CasExpectation::Version(version)),
                None => (fallback.clone(), CasExpectation::Absent),
            };
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
            match self.write(&next, cas).await {
                Ok(_) => return Ok(Some(existing)),
                Err(PutError::VersionMismatch) => cas_retry_backoff(attempt).await,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(TurnError::Unavailable {
            reason: "turn runner lease CAS retries exhausted".to_string(),
        })
    }

    async fn read(
        &self,
        run_id: TurnRunId,
    ) -> Result<Option<(RunnerLeaseRecord, RecordVersion)>, TurnError> {
        let path = runner_lease_path(run_id)?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => {
                let record = deserialize_runner_lease(&versioned.entry.body)?;
                Ok(Some((record, versioned.version)))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn write(
        &self,
        record: &RunnerLeaseRecord,
        cas: CasExpectation,
    ) -> Result<RecordVersion, PutError> {
        let path = runner_lease_path(record.run_id).map_err(PutError::Other)?;
        put_with_cas(
            self.filesystem.as_ref(),
            &path,
            runner_lease_entry(record).map_err(PutError::Other)?,
            cas,
        )
        .await
    }

    async fn delete(&self, run_id: TurnRunId) -> Result<(), TurnError> {
        let path = runner_lease_path(run_id)?;
        match self
            .filesystem
            .delete(&ResourceScope::system(), &path)
            .await
        {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(fs_error(error)),
        }
    }
}

fn runner_lease_path(run_id: TurnRunId) -> Result<ScopedPath, TurnError> {
    ScopedPath::new(format!("{TURNS_RUNNER_LEASE_PREFIX}/{run_id}.json")).map_err(|error| {
        TurnError::Unavailable {
            reason: format!("invalid turn runner lease path: {error}"),
        }
    })
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

fn runner_lease_entry(record: &RunnerLeaseRecord) -> Result<Entry, TurnError> {
    let body = serde_json::to_vec(record).map_err(|error| TurnError::Unavailable {
        reason: format!("turn runner lease serialization failed: {error}"),
    })?;
    let kind =
        RecordKind::new(TURNS_RUNNER_LEASE_KIND).map_err(|error| TurnError::Unavailable {
            reason: format!("invalid turn runner lease record kind: {error}"),
        })?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    Ok(entry)
}

fn deserialize_runner_lease(bytes: &[u8]) -> Result<RunnerLeaseRecord, TurnError> {
    serde_json::from_slice(bytes).map_err(|error| TurnError::Unavailable {
        reason: format!("turn runner lease deserialization failed: {error}"),
    })
}
