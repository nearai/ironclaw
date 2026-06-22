use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "slack-v2-host-beta")]
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_triggers::{
    ScheduleTriggerSourceProvider, TriggerActiveRunLookup, TriggerActiveRunState,
    TriggerActiveRunStateRequest, TriggerError, TriggerPollerWorker, TriggerPollerWorkerDeps,
    TriggerPromptMaterializer, TriggerRepository, TriggerRunHistoryStatus,
    TrustedTriggerFireSubmitter,
};
#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_triggers::{TrustedTriggerFireSubmitOutcome, TrustedTriggerSubmitRequest};
use ironclaw_turns::{TurnPersistenceSnapshot, TurnStatus};
use rand::Rng;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::runtime_input::TriggerPollerSettings;
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack_delivery::PostSubmitDeliveryHook;
pub(crate) use crate::trigger_poller_trusted_submit::AccessCheckerTriggerFireAuthorizer;
pub(crate) use crate::trigger_poller_trusted_submit::ConversationContentRefMaterializer;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use crate::trigger_poller_trusted_submit::TenantScopedTrustedTriggerFireAuthorizer;

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
    pub(crate) materializer: Arc<dyn TriggerPromptMaterializer>,
    pub(crate) trusted_submitter: Arc<dyn TrustedTriggerFireSubmitter>,
    pub(crate) active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
    /// Late-binding slot for the post-submit delivery hook. Filled by
    /// `RebornRuntime::set_trigger_post_submit_hook` after the runtime is
    /// built. The poller wrapper checks `slot.get()` at each successful submit
    /// (cheap atomic read), so the hook can be wired after `spawn_trigger_poller`
    /// returns without restarting the poller.
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) post_submit_hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
}

pub(crate) fn spawn_trigger_poller(
    settings: TriggerPollerSettings,
    deps: TriggerPollerCompositionDeps,
) -> Result<Option<TriggerPollerRuntimeHandle>, TriggerError> {
    if !settings.enabled {
        return Ok(None);
    }
    settings.worker.validate()?;
    #[cfg(feature = "slack-v2-host-beta")]
    let submitter: Arc<dyn TrustedTriggerFireSubmitter> =
        Arc::new(PostSubmitHookWrappedSubmitter {
            inner: deps.trusted_submitter,
            hook_slot: deps.post_submit_hook_slot,
        });
    #[cfg(not(feature = "slack-v2-host-beta"))]
    let submitter: Arc<dyn TrustedTriggerFireSubmitter> = deps.trusted_submitter;
    let worker = TriggerPollerWorker::new(
        settings.worker.clone(),
        TriggerPollerWorkerDeps {
            repository: deps.repository,
            source_provider: Arc::new(ScheduleTriggerSourceProvider),
            materializer: deps.materializer,
            trusted_submitter: submitter,
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

/// Wraps a `TrustedTriggerFireSubmitter` to invoke a post-submit hook after
/// each successful fire submission. The hook is stored in a `OnceLock` slot so
/// it can be wired after the poller is spawned (late-binding). If the slot is
/// empty at submit time the hook is simply skipped.
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) struct PostSubmitHookWrappedSubmitter {
    pub(crate) inner: Arc<dyn TrustedTriggerFireSubmitter>,
    pub(crate) hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
}

#[cfg(feature = "slack-v2-host-beta")]
#[async_trait]
impl TrustedTriggerFireSubmitter for PostSubmitHookWrappedSubmitter {
    async fn submit_trusted_trigger_fire(
        &self,
        request: TrustedTriggerSubmitRequest,
    ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
        // Clone the fire before delegating so the hook can receive it.
        let fire = request.fire().clone();
        let outcome = self.inner.submit_trusted_trigger_fire(request).await?;
        if let TrustedTriggerFireSubmitOutcome::Accepted {
            run_id,
            turn_scope: ref scope,
            ..
        } = outcome
        {
            // Cheap atomic read: if the slot is not yet filled the hook simply
            // doesn't fire — the poller is not restarted.
            if let Some(hook) = self.hook_slot.get() {
                hook.on_trigger_submitted(fire, run_id, scope.clone()).await;
            } else {
                tracing::warn!(
                    target = "ironclaw::reborn::trigger_poller",
                    %run_id,
                    "triggered run accepted but post-submit hook slot not yet set (startup window); delivery skipped for this fire"
                );
            }
        }
        Ok(outcome)
    }
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
                TriggerActiveRunState::Terminal {
                    status: terminal_run_history_status(run.status),
                }
            } else if is_human_interaction_gate(run.status) {
                // A scheduled fire runs unattended, so an approval/auth gate
                // will never be answered. Surface it as Blocked so the poller
                // clears the active fire instead of letting one stuck run block
                // every future scheduled run of the trigger (#4986).
                TriggerActiveRunState::Blocked
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

/// A run parked on a gate that needs a human to act (tool-approval or auth).
/// An unattended scheduled fire cannot satisfy these, so the poller treats
/// them as non-advancing and clears the fire. `BlockedResource` and
/// `BlockedDependentRun` are deliberately excluded — those resolve on their
/// own as the resource frees up or the dependent run finishes, so they stay
/// `Nonterminal` and keep their place in the schedule.
fn is_human_interaction_gate(status: TurnStatus) -> bool {
    matches!(
        status,
        TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
    )
}

fn terminal_run_history_status(status: TurnStatus) -> TriggerRunHistoryStatus {
    debug_assert!(
        status.is_terminal(),
        "only terminal turn statuses should be normalized into run-history status"
    );
    match status {
        TurnStatus::Completed => TriggerRunHistoryStatus::Ok,
        TurnStatus::Cancelled | TurnStatus::Failed | TurnStatus::RecoveryRequired => {
            TriggerRunHistoryStatus::Error
        }
        TurnStatus::Queued
        | TurnStatus::Running
        | TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::CancelRequested => TriggerRunHistoryStatus::Error,
    }
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
    use ironclaw_turns::{
        AcceptedMessageRef, AgentLoopDriverDescriptor, CancellationPolicy,
        CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass,
        ContextProfileId, EventCursor, LoopDriverId, ModelProfileId, RedactedRunProfileProvenance,
        ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier, RunClassId,
        RunProfileFingerprint, RunProfileId, RunProfileVersion, RuntimeProfileConstraints,
        SchedulingClass, SourceBindingRef, SteeringPolicy, TurnId, TurnPersistenceSnapshot,
        TurnRunId, TurnRunProfile, TurnRunRecord, TurnScope, TurnStatus,
    };

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

    struct StaticSnapshotSource {
        snapshot: TurnPersistenceSnapshot,
    }

    #[async_trait]
    impl TriggerTurnSnapshotSource for StaticSnapshotSource {
        async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
            Ok(self.snapshot.clone())
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

    #[test]
    fn terminal_turn_statuses_map_to_run_history_statuses() {
        let cases = [
            (TurnStatus::Completed, TriggerRunHistoryStatus::Ok),
            (TurnStatus::Cancelled, TriggerRunHistoryStatus::Error),
            (TurnStatus::Failed, TriggerRunHistoryStatus::Error),
            (TurnStatus::RecoveryRequired, TriggerRunHistoryStatus::Error),
        ];

        for (turn_status, expected) in cases {
            assert_eq!(terminal_run_history_status(turn_status), expected);
        }
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
    async fn active_run_batch_lookup_returns_nonterminal_and_terminal_states_from_snapshot() {
        let tenant_id = TenantId::new("trigger-active-state-tenant").expect("tenant id");
        let nonterminal_run_id = TurnRunId::new();
        let terminal_run_id = TurnRunId::new();
        let missing_run_id = TurnRunId::new();
        let snapshot_source = Arc::new(StaticSnapshotSource {
            snapshot: TurnPersistenceSnapshot {
                runs: vec![
                    turn_run_record(&tenant_id, nonterminal_run_id, TurnStatus::Running),
                    turn_run_record(&tenant_id, terminal_run_id, TurnStatus::Completed),
                ],
                ..TurnPersistenceSnapshot::default()
            },
        });
        let lookup = SnapshotActiveRunLookup::new(snapshot_source);
        let fire_slot = Utc::now();

        let results = lookup
            .active_run_states(vec![
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: nonterminal_run_id,
                },
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: terminal_run_id,
                },
                TriggerActiveRunStateRequest {
                    tenant_id,
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: missing_run_id,
                },
            ])
            .await;

        assert!(matches!(results[0], Ok(TriggerActiveRunState::Nonterminal)));
        assert!(matches!(
            results[1],
            Ok(TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Ok
            })
        ));
        assert!(matches!(results[2], Ok(TriggerActiveRunState::Missing)));
    }

    #[tokio::test]
    async fn human_interaction_gates_map_to_blocked_other_blocks_stay_nonterminal() {
        // #4986: approval/auth gates can never be answered by an unattended
        // fire, so they must surface as Blocked (poller clears the fire).
        // Resource/dependent-run waits resolve on their own and stay
        // Nonterminal so they keep their schedule slot.
        let tenant_id = TenantId::new("trigger-blocked-state-tenant").expect("tenant id");
        let approval_run = TurnRunId::new();
        let auth_run = TurnRunId::new();
        let resource_run = TurnRunId::new();
        let dependent_run = TurnRunId::new();
        let snapshot_source = Arc::new(StaticSnapshotSource {
            snapshot: TurnPersistenceSnapshot {
                runs: vec![
                    turn_run_record(&tenant_id, approval_run, TurnStatus::BlockedApproval),
                    turn_run_record(&tenant_id, auth_run, TurnStatus::BlockedAuth),
                    turn_run_record(&tenant_id, resource_run, TurnStatus::BlockedResource),
                    turn_run_record(&tenant_id, dependent_run, TurnStatus::BlockedDependentRun),
                ],
                ..TurnPersistenceSnapshot::default()
            },
        });
        let lookup = SnapshotActiveRunLookup::new(snapshot_source);
        let fire_slot = Utc::now();
        let request = |run_id| TriggerActiveRunStateRequest {
            tenant_id: tenant_id.clone(),
            trigger_id: TriggerId::new(),
            fire_slot,
            run_id,
        };

        let results = lookup
            .active_run_states(vec![
                request(approval_run),
                request(auth_run),
                request(resource_run),
                request(dependent_run),
            ])
            .await;

        assert!(matches!(results[0], Ok(TriggerActiveRunState::Blocked)));
        assert!(matches!(results[1], Ok(TriggerActiveRunState::Blocked)));
        assert!(matches!(results[2], Ok(TriggerActiveRunState::Nonterminal)));
        assert!(matches!(results[3], Ok(TriggerActiveRunState::Nonterminal)));
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

    fn turn_run_record(
        tenant_id: &TenantId,
        run_id: TurnRunId,
        status: TurnStatus,
    ) -> TurnRunRecord {
        let scope = TurnScope::new(
            tenant_id.clone(),
            None,
            None,
            ironclaw_host_api::ThreadId::new(format!("thread-{run_id}")).expect("thread id"),
        );
        TurnRunRecord {
            run_id,
            turn_id: TurnId::new(),
            scope,
            accepted_message_ref: AcceptedMessageRef::new(format!("message:{run_id}"))
                .expect("message ref"),
            source_binding_ref: SourceBindingRef::new(format!("source:{run_id}"))
                .expect("source binding ref"),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(format!(
                "reply:{run_id}"
            ))
            .expect("reply target binding ref"),
            status,
            profile: TurnRunProfile::from_resolved(resolved_run_profile()),
            resolved_model_route: None,
            checkpoint_id: None,
            gate_ref: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }

    fn resolved_run_profile() -> ResolvedRunProfile {
        let checkpoint_schema_id =
            CheckpointSchemaId::new("trigger_active_checkpoint").expect("checkpoint schema");
        ResolvedRunProfile {
            run_class_id: RunClassId::new("trigger_active").expect("run class"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: AgentLoopDriverDescriptor {
                id: LoopDriverId::new("trigger_active_loop").expect("loop driver"),
                version: RunProfileVersion::new(1),
                checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
                checkpoint_schema_version: Some(RunProfileVersion::new(1)),
            },
            checkpoint_schema_id,
            checkpoint_schema_version: RunProfileVersion::new(1),
            model_profile_id: ModelProfileId::new("trigger_active_model").expect("model profile"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new("trigger_active_caps")
                .expect("capability surface profile"),
            context_profile_id: ContextProfileId::new("trigger_active_context")
                .expect("context profile"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: true,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("trigger_active_budget").expect("budget tier"),
                max_model_calls: 1,
                max_capability_invocations: 1,
            },
            personal_context_policy: Default::default(),
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("trigger_active").expect("scheduling class"),
            concurrency_class: ConcurrencyClass::new("trigger_active").expect("concurrency class"),
            resolution_fingerprint: RunProfileFingerprint::new("trigger-active-profile-v1")
                .expect("run profile fingerprint"),
            provenance: RedactedRunProfileProvenance {
                sources: Vec::new(),
                effective_privileges: Vec::new(),
            },
        }
    }

    // ── PostSubmitHookWrappedSubmitter tests ────────────────────────────────

    #[cfg(feature = "slack-v2-host-beta")]
    mod hook_wrapper {
        use std::sync::{Arc, Mutex, OnceLock};
        use std::time::Duration;

        use async_trait::async_trait;
        use chrono::Utc;
        use ironclaw_host_api::{AgentId, TenantId, ThreadId, Timestamp, UserId};
        use ironclaw_triggers::{
            InMemoryTriggerRepository, TriggerActiveRunLookup, TriggerActiveRunState,
            TriggerActiveRunStateRequest, TriggerError, TriggerFire, TriggerId,
            TriggerInboundContentRef, TriggerMaterializedPrompt, TriggerPollerWorker,
            TriggerPollerWorkerConfig, TriggerPollerWorkerDeps, TriggerPromptMaterializer,
            TriggerRecord, TriggerRepository, TriggerSchedule, TriggerSourceKind, TriggerState,
            TrustedTriggerFireSubmitOutcome, TrustedTriggerFireSubmitter,
            TrustedTriggerSubmitRequest,
        };
        use ironclaw_turns::{TurnRunId, TurnScope};

        use super::super::PostSubmitHookWrappedSubmitter;
        use crate::slack_delivery::PostSubmitDeliveryHook;

        // ── shared fakes ─────────────────────────────────────────────────────

        /// Materializer that always succeeds with a fixed content ref.
        struct FixedMaterializer;

        #[async_trait]
        impl TriggerPromptMaterializer for FixedMaterializer {
            async fn materialize_prompt(
                &self,
                fire: TriggerFire,
            ) -> Result<TriggerMaterializedPrompt, TriggerError> {
                let content_ref = TriggerInboundContentRef::new("content:hook-wrapper-test")
                    .expect("content ref");
                Ok(TriggerMaterializedPrompt::for_fire(&fire, content_ref))
            }
        }

        /// Active-run lookup that always reports `Missing` (no concurrent run).
        struct AlwaysMissingLookup;

        #[async_trait]
        impl TriggerActiveRunLookup for AlwaysMissingLookup {
            async fn active_run_state(
                &self,
                _request: TriggerActiveRunStateRequest,
            ) -> Result<TriggerActiveRunState, TriggerError> {
                Ok(TriggerActiveRunState::Missing)
            }
        }

        /// Inner submitter that always returns `Accepted` with a pre-set run_id
        /// and a scope derived from the request's creator. Used to exercise the
        /// wrapper without going through the real submission pipeline.
        struct FixedAcceptedSubmitter {
            run_id: TurnRunId,
        }

        #[async_trait]
        impl TrustedTriggerFireSubmitter for FixedAcceptedSubmitter {
            async fn submit_trusted_trigger_fire(
                &self,
                request: TrustedTriggerSubmitRequest,
            ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
                let creator = request.fire().creator_user_id.clone();
                // Mirror the post-Task-2 production shape: fabricate the scope
                // with the trigger creator as explicit owner so the fixture
                // matches what the real trusted-submit path now produces.
                let scope = TurnScope::new_with_owner(
                    wrapper_tenant(),
                    Some(AgentId::new("hook-wrapper-agent").expect("agent")),
                    None,
                    hook_wrapper_thread_id(self.run_id),
                    Some(creator),
                );
                Ok(TrustedTriggerFireSubmitOutcome::Accepted {
                    run_id: self.run_id,
                    submitted_at: Utc::now(),
                    turn_scope: scope,
                })
            }
        }

        /// Hook that records every invocation.
        #[derive(Default)]
        struct RecordingHook {
            calls: Mutex<Vec<(TriggerFire, TurnRunId, TurnScope)>>,
        }

        impl RecordingHook {
            fn calls(&self) -> Vec<(TriggerFire, TurnRunId, TurnScope)> {
                self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
            }
        }

        #[async_trait]
        impl PostSubmitDeliveryHook for RecordingHook {
            async fn on_trigger_submitted(
                &self,
                fire: TriggerFire,
                run_id: TurnRunId,
                scope: TurnScope,
            ) {
                self.calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((fire, run_id, scope));
            }
        }

        // ── helpers ───────────────────────────────────────────────────────────

        fn wrapper_tenant() -> TenantId {
            TenantId::new("hook-wrapper-tenant").expect("tenant")
        }

        fn hook_wrapper_thread_id(run_id: TurnRunId) -> ThreadId {
            ThreadId::new(format!("hook-wrapper-thread-{run_id}")).expect("thread id")
        }

        /// Seed one due trigger in `repo` and return the fire slot timestamp.
        async fn seed_due_trigger(
            repo: &InMemoryTriggerRepository,
            fire_slot: Timestamp,
        ) -> TriggerId {
            let trigger_id = TriggerId::new();
            let record = TriggerRecord {
                trigger_id,
                tenant_id: wrapper_tenant(),
                creator_user_id: UserId::new("hook-wrapper-user").expect("user"),
                agent_id: None,
                project_id: None,
                name: "hook-wrapper-trigger".to_string(),
                source: TriggerSourceKind::Schedule,
                schedule: TriggerSchedule::cron("* * * * *").expect("cron"),
                prompt: "hook wrapper test prompt".to_string(),
                state: TriggerState::Scheduled,
                next_run_at: fire_slot,
                last_run_at: None,
                last_fired_slot: None,
                last_status: None,
                active_fire_slot: None,
                active_run_ref: None,
                created_at: fire_slot,
            };
            repo.upsert_trigger(record).await.expect("upsert trigger");
            trigger_id
        }

        /// Build a `TriggerPollerWorker` backed by the supplied repo, with the
        /// given `trusted_submitter`. The caller must seed triggers into `repo`
        /// before calling `tick_once`.
        fn build_worker_with_repo(
            repo: Arc<InMemoryTriggerRepository>,
            trusted_submitter: Arc<dyn TrustedTriggerFireSubmitter>,
        ) -> TriggerPollerWorker {
            TriggerPollerWorker::new(
                TriggerPollerWorkerConfig {
                    poll_interval: Duration::from_millis(50),
                    fires_per_tick: 1,
                    max_concurrent_fires_per_trigger: 1,
                },
                TriggerPollerWorkerDeps {
                    repository: repo,
                    source_provider: Arc::new(ironclaw_triggers::ScheduleTriggerSourceProvider),
                    materializer: Arc::new(FixedMaterializer),
                    trusted_submitter,
                    active_run_lookup: Arc::new(AlwaysMissingLookup),
                },
            )
            .expect("valid worker")
        }

        // ── tests ─────────────────────────────────────────────────────────────

        /// Empty hook slot: poller fires the trigger, inner submitter accepts,
        /// but the hook is never invoked.
        #[tokio::test]
        async fn empty_slot_submit_succeeds_hook_does_not_fire() {
            let repo = Arc::new(InMemoryTriggerRepository::default());
            let fire_slot = Utc::now() - chrono::Duration::seconds(1);
            seed_due_trigger(&repo, fire_slot).await;

            let run_id = TurnRunId::new();
            let inner = Arc::new(FixedAcceptedSubmitter { run_id });
            let hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>> =
                Arc::new(OnceLock::new());

            // Wrap the inner submitter; hook slot is empty.
            let wrapper = Arc::new(PostSubmitHookWrappedSubmitter {
                inner: inner as Arc<dyn TrustedTriggerFireSubmitter>,
                hook_slot: Arc::clone(&hook_slot),
            });

            let worker =
                build_worker_with_repo(repo, wrapper as Arc<dyn TrustedTriggerFireSubmitter>);
            let report = worker
                .tick_once(Utc::now())
                .await
                .expect("tick_once succeeds");

            // The trigger was processed.
            assert_eq!(
                report.due_records, 1,
                "one due trigger should have been processed"
            );
            // Hook slot is still empty — nothing wired it up.
            assert!(
                hook_slot.get().is_none(),
                "hook slot must remain empty when no hook was set"
            );
        }

        /// Filled hook slot: poller fires the trigger, inner submitter accepts,
        /// hook receives the accepted run_id and scope.
        #[tokio::test]
        async fn filled_slot_accepted_submit_invokes_hook_with_run_id_and_scope() {
            let repo = Arc::new(InMemoryTriggerRepository::default());
            let fire_slot = Utc::now() - chrono::Duration::seconds(1);
            seed_due_trigger(&repo, fire_slot).await;

            let run_id = TurnRunId::new();
            let inner = Arc::new(FixedAcceptedSubmitter { run_id });
            let hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>> =
                Arc::new(OnceLock::new());

            // Pre-fill the slot with a recording hook.
            let recording = Arc::new(RecordingHook::default());
            hook_slot
                .set(Arc::clone(&recording) as Arc<dyn PostSubmitDeliveryHook>)
                .unwrap_or_else(|_| panic!("slot set should succeed on first call"));

            let wrapper = Arc::new(PostSubmitHookWrappedSubmitter {
                inner: inner as Arc<dyn TrustedTriggerFireSubmitter>,
                hook_slot: Arc::clone(&hook_slot),
            });

            let worker =
                build_worker_with_repo(repo, wrapper as Arc<dyn TrustedTriggerFireSubmitter>);
            let report = worker
                .tick_once(Utc::now())
                .await
                .expect("tick_once succeeds");

            assert_eq!(report.due_records, 1, "one due trigger must be processed");

            // Hook was invoked exactly once.
            let calls = recording.calls();
            assert_eq!(calls.len(), 1, "hook must fire exactly once");

            let (recorded_fire, called_run_id, called_scope) = &calls[0];
            assert_eq!(
                *called_run_id, run_id,
                "hook must receive the accepted run_id"
            );
            let expected_thread_id = hook_wrapper_thread_id(run_id);
            assert_eq!(
                called_scope.thread_id, expected_thread_id,
                "hook must receive the accepted turn_scope thread_id"
            );
            assert_eq!(
                called_scope.explicit_owner_user_id(),
                Some(&recorded_fire.creator_user_id),
                "post-submit hook must receive a TurnScope owned by the trigger creator"
            );
        }
    }
}
