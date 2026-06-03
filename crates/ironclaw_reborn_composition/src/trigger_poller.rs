use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_triggers::{
    ScheduleTriggerSourceProvider, TriggerActiveRunLookup, TriggerActiveRunState,
    TriggerActiveRunStateRequest, TriggerError, TriggerPollerWorker, TriggerPollerWorkerDeps,
    TriggerRepository, TrustedTriggerFireSubmitter,
};
use ironclaw_turns::TurnPersistenceSnapshot;
use rand::Rng;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::runtime_input::TriggerPollerSettings;
use crate::trigger_poller_trusted_submit::ConversationContentRefMaterializer;
pub(crate) use crate::trigger_poller_trusted_submit::ConversationTrustedTriggerSubmitter;

pub(crate) const TRIGGER_POLLER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct TriggerPollerRuntimeHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl TriggerPollerRuntimeHandle {
    pub(crate) async fn shutdown(self, timeout: Duration) {
        self.cancel.cancel();
        self.join_with_timeout(timeout).await;
    }

    pub(crate) async fn join_with_timeout(self, timeout: Duration) {
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(?error, "trigger poller task join failed");
            }
            Err(_) => {
                tracing::warn!(
                    ?timeout,
                    "trigger poller task did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::warn!(?error, "aborted trigger poller task panicked");
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct TriggerPollerCompositionDeps {
    pub(crate) repository: Arc<dyn TriggerRepository>,
    pub(crate) trusted_submitter: Arc<dyn TrustedTriggerFireSubmitter>,
    pub(crate) active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
}

pub(crate) fn spawn_trigger_poller(
    settings: TriggerPollerSettings,
    deps: TriggerPollerCompositionDeps,
) -> Result<Option<TriggerPollerRuntimeHandle>, TriggerError> {
    if !settings.enabled {
        return Ok(None);
    }
    settings.worker.validate()?;
    let worker = TriggerPollerWorker::new(
        settings.worker.clone(),
        TriggerPollerWorkerDeps {
            repository: deps.repository,
            source_provider: Arc::new(ScheduleTriggerSourceProvider),
            materializer: Arc::new(ConversationContentRefMaterializer),
            trusted_submitter: deps.trusted_submitter,
            active_run_lookup: deps.active_run_lookup,
        },
    )?;
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_trigger_poller(worker, settings, task_cancel).await;
    });
    Ok(Some(TriggerPollerRuntimeHandle { cancel, handle }))
}

async fn run_trigger_poller(
    worker: TriggerPollerWorker,
    settings: TriggerPollerSettings,
    cancel: CancellationToken,
) {
    if !sleep_or_cancel(jitter_delay(settings.startup_jitter_max), &cancel).await {
        return;
    }
    loop {
        let now = Utc::now();
        match worker.tick_once(now).await {
            Ok(report) => {
                tracing::debug!(
                    due_records = report.due_records,
                    active_records = report.active_records,
                    outcomes = report.results.len(),
                    "trigger poller tick completed"
                );
            }
            Err(error) => {
                tracing::warn!(?error, "trigger poller tick failed");
            }
        }
        let delay = settings.worker.poll_interval + jitter_delay(settings.tick_jitter_max);
        if !sleep_or_cancel(delay, &cancel).await {
            return;
        }
    }
}

async fn sleep_or_cancel(delay: Duration, cancel: &CancellationToken) -> bool {
    if delay.is_zero() {
        return !cancel.is_cancelled();
    }
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

fn jitter_delay(max: Duration) -> Duration {
    if max.is_zero() {
        return Duration::ZERO;
    }
    let max_nanos = max.as_nanos().min(u64::MAX as u128);
    let nanos = rand::thread_rng().gen_range(0..=max_nanos);
    let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
    Duration::from_nanos(nanos)
}

pub(crate) struct SnapshotActiveRunLookup {
    snapshot_source: Arc<dyn TriggerTurnSnapshotSource>,
}

impl SnapshotActiveRunLookup {
    pub(crate) fn new(snapshot_source: Arc<dyn TriggerTurnSnapshotSource>) -> Self {
        Self { snapshot_source }
    }
}

#[async_trait]
impl TriggerActiveRunLookup for SnapshotActiveRunLookup {
    async fn active_run_state(
        &self,
        request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError> {
        let snapshot = self.snapshot_source.snapshot().await?;
        let run_index = active_run_index(&snapshot);
        Ok(active_run_state_from_index(&run_index, &request))
    }

    async fn active_run_states(
        &self,
        requests: Vec<TriggerActiveRunStateRequest>,
    ) -> Vec<Result<TriggerActiveRunState, TriggerError>> {
        if requests.is_empty() {
            return Vec::new();
        }
        let snapshot = match self.snapshot_source.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let reason = error.to_string();
                return requests
                    .into_iter()
                    .map(|_| {
                        Err(TriggerError::Backend {
                            reason: reason.clone(),
                        })
                    })
                    .collect();
            }
        };
        let run_index = active_run_index(&snapshot);
        requests
            .iter()
            .map(|request| Ok(active_run_state_from_index(&run_index, request)))
            .collect()
    }
}

fn active_run_index(
    snapshot: &TurnPersistenceSnapshot,
) -> HashMap<(ironclaw_host_api::TenantId, ironclaw_turns::TurnRunId), TriggerActiveRunState> {
    snapshot
        .runs
        .iter()
        .map(|run| {
            let state = if run.status.is_terminal() {
                TriggerActiveRunState::Terminal
            } else {
                TriggerActiveRunState::Nonterminal
            };
            ((run.scope.tenant_id.clone(), run.run_id), state)
        })
        .collect()
}

fn active_run_state_from_index(
    run_index: &HashMap<
        (ironclaw_host_api::TenantId, ironclaw_turns::TurnRunId),
        TriggerActiveRunState,
    >,
    request: &TriggerActiveRunStateRequest,
) -> TriggerActiveRunState {
    run_index
        .get(&(request.tenant_id.clone(), request.run_id))
        .copied()
        .unwrap_or(TriggerActiveRunState::Missing)
}

#[async_trait]
pub(crate) trait TriggerTurnSnapshotSource: Send + Sync {
    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError>;
}

pub(crate) struct LocalTriggerTurnSnapshotSource<S> {
    store: Arc<S>,
}

impl<S> LocalTriggerTurnSnapshotSource<S> {
    pub(crate) fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl<F> TriggerTurnSnapshotSource
    for LocalTriggerTurnSnapshotSource<ironclaw_turns::FilesystemTurnStateStore<F>>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
        self.store
            .persistence_snapshot()
            .await
            .map_err(trigger_backend_error)
    }
}

#[cfg(not(feature = "libsql"))]
#[async_trait]
impl TriggerTurnSnapshotSource
    for LocalTriggerTurnSnapshotSource<ironclaw_turns::InMemoryTurnStateStore>
{
    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
        Ok(self.store.persistence_snapshot())
    }
}

#[cfg(feature = "libsql")]
fn trigger_backend_error(error: impl std::fmt::Display) -> TriggerError {
    TriggerError::Backend {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::TenantId;
    use ironclaw_triggers::{TriggerId, TriggerPollerWorkerConfig};
    use ironclaw_turns::TurnRunId;

    #[derive(Default)]
    struct CountingSnapshotSource {
        calls: std::sync::Mutex<usize>,
    }

    impl CountingSnapshotSource {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("snapshot calls lock")
        }
    }

    #[async_trait]
    impl TriggerTurnSnapshotSource for CountingSnapshotSource {
        async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
            *self.calls.lock().expect("snapshot calls lock") += 1;
            Ok(TurnPersistenceSnapshot::default())
        }
    }

    #[derive(Default)]
    struct FailingSnapshotSource {
        calls: std::sync::Mutex<usize>,
    }

    impl FailingSnapshotSource {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("snapshot calls lock")
        }
    }

    #[async_trait]
    impl TriggerTurnSnapshotSource for FailingSnapshotSource {
        async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
            *self.calls.lock().expect("snapshot calls lock") += 1;
            Err(TriggerError::Backend {
                reason: "snapshot failed".to_string(),
            })
        }
    }

    #[test]
    fn jitter_is_disabled_when_max_is_zero() {
        assert_eq!(jitter_delay(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn jitter_is_bounded_by_max() {
        let max = Duration::from_millis(25);

        assert!(jitter_delay(max) <= max);
    }

    #[test]
    fn trigger_poller_defaults_are_disabled_without_jitter() {
        let settings = TriggerPollerSettings::default();

        assert!(!settings.enabled);
        assert_eq!(settings.startup_jitter_max, Duration::ZERO);
        assert_eq!(settings.tick_jitter_max, Duration::ZERO);
        assert_eq!(settings.worker, TriggerPollerWorkerConfig::default());
    }

    #[test]
    fn trigger_poller_enabled_preserves_default_worker_without_jitter() {
        let settings = TriggerPollerSettings::enabled();

        assert!(settings.enabled);
        assert_eq!(settings.startup_jitter_max, Duration::ZERO);
        assert_eq!(settings.tick_jitter_max, Duration::ZERO);
        assert_eq!(settings.worker, TriggerPollerWorkerConfig::default());
    }

    #[tokio::test]
    async fn trigger_poller_runtime_handle_aborts_when_join_times_out() {
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            task_cancel.cancelled().await;
            std::future::pending::<()>().await;
        });
        let runtime_handle = TriggerPollerRuntimeHandle { cancel, handle };

        runtime_handle.shutdown(Duration::from_millis(1)).await;
    }

    #[tokio::test]
    async fn active_run_batch_lookup_uses_one_snapshot_for_page() {
        let snapshot_source = Arc::new(CountingSnapshotSource::default());
        let lookup = SnapshotActiveRunLookup::new(snapshot_source.clone());
        let tenant_id = TenantId::new("trigger-active-batch-tenant").expect("tenant id");
        let fire_slot = Utc::now();

        let results = lookup
            .active_run_states(vec![
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
                TriggerActiveRunStateRequest {
                    tenant_id,
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
            ])
            .await;

        assert_eq!(snapshot_source.calls(), 1);
        assert_eq!(results.len(), 2);
        assert!(
            results
                .into_iter()
                .all(|result| matches!(result, Ok(TriggerActiveRunState::Missing)))
        );
    }

    #[tokio::test]
    async fn active_run_batch_lookup_returns_empty_without_snapshot() {
        let snapshot_source = Arc::new(CountingSnapshotSource::default());
        let lookup = SnapshotActiveRunLookup::new(snapshot_source.clone());

        let results = lookup.active_run_states(Vec::new()).await;

        assert!(results.is_empty());
        assert_eq!(snapshot_source.calls(), 0);
    }

    #[tokio::test]
    async fn snapshot_source_error_fans_out_to_all_batch_results() {
        let snapshot_source = Arc::new(FailingSnapshotSource::default());
        let lookup = SnapshotActiveRunLookup::new(snapshot_source.clone());
        let tenant_id = TenantId::new("trigger-active-error-tenant").expect("tenant id");
        let fire_slot = Utc::now();

        let results = lookup
            .active_run_states(vec![
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
                TriggerActiveRunStateRequest {
                    tenant_id,
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
            ])
            .await;

        assert_eq!(snapshot_source.calls(), 1);
        assert_eq!(results.len(), 2);
        assert!(results.into_iter().all(|result| matches!(
            result,
            Err(TriggerError::Backend { reason }) if reason.contains("snapshot failed")
        )));
    }
}
