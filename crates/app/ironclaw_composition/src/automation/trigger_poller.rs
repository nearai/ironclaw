use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_telemetry_contracts::{
    observation::{
        AutomationId, AutomationKind, AutomationSettledObservation, ObservationContext, RunOutcome,
        TelemetryObservation,
    },
    recorder::TelemetryRecorder,
};
use ironclaw_triggers::{
    ScheduleTriggerSourceProvider, TriggerActiveRunLookup, TriggerError, TriggerPollerWorker,
    TriggerPollerWorkerDeps, TriggerPromptMaterializer, TriggerRepository,
    TrustedTriggerFireSubmitter,
};
use ironclaw_triggers::{
    TriggerAcceptedFireSettlement, TriggerAutomationKind, TriggerFailedFireSettlement,
    TriggerFireSettlementObserver, TriggerRunTerminalSettlement, TriggerTerminalOutcome,
};
use rand::RngExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(crate) use crate::automation::trigger_poller_trusted_submit::AccessCheckerTriggerFireAuthorizer;
pub(crate) use crate::automation::trigger_poller_trusted_submit::ConversationContentRefMaterializer;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use crate::automation::trigger_poller_trusted_submit::TenantScopedTrustedTriggerFireAuthorizer;
use crate::runtime_input::TriggerPollerSettings;
pub(crate) use ironclaw_extension_host::channel_triggered_delivery::PostSubmitDeliveryHook;

mod active_run_lookup;
pub(crate) use active_run_lookup::{
    ProcessActiveRunLookup, RebindableProcessLifecycleLookupSource,
};

pub(crate) const TRIGGER_POLLER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct LateBoundTriggerManualFireRunner {
    runner: OnceLock<Arc<dyn ironclaw_triggers::TriggerManualFireRunner>>,
}

impl LateBoundTriggerManualFireRunner {
    pub(crate) fn bind(
        &self,
        runner: Arc<dyn ironclaw_triggers::TriggerManualFireRunner>,
    ) -> Result<(), TriggerError> {
        self.runner.set(runner).map_err(|_| TriggerError::Backend {
            reason: "manual trigger fire runner was already bound".to_string(),
        })
    }
}

#[async_trait]
impl ironclaw_triggers::TriggerManualFireRunner for LateBoundTriggerManualFireRunner {
    async fn run_manual_fire(
        &self,
        tenant_id: ironclaw_host_api::ids::TenantId,
        trigger_id: ironclaw_triggers::TriggerId,
        now: ironclaw_host_api::Timestamp,
    ) -> Result<ironclaw_triggers::TriggerManualFireOutcome, TriggerError> {
        let runner = self.runner.get().ok_or_else(|| TriggerError::Backend {
            reason: "manual trigger fire runner is not available".to_string(),
        })?;
        runner.run_manual_fire(tenant_id, trigger_id, now).await
    }
}

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
    pub(crate) manual_fire_runner: Arc<LateBoundTriggerManualFireRunner>,
    pub(crate) telemetry_recorder: Arc<dyn TelemetryRecorder>,
    /// Late-binding slot for the post-submit delivery hook.
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
    let cancel = CancellationToken::new();
    let fire_settlement_observer: Arc<dyn TriggerFireSettlementObserver> =
        Arc::new(TriggerSettlementObserver::with_telemetry_recorder(
            deps.post_submit_hook_slot,
            cancel.clone(),
            deps.telemetry_recorder,
        ));
    let trusted_submitter = deps.trusted_submitter;
    let worker = Arc::new(TriggerPollerWorker::new(
        settings.worker.clone(),
        TriggerPollerWorkerDeps {
            repository: deps.repository,
            source_provider: Arc::new(ScheduleTriggerSourceProvider),
            materializer: deps.materializer,
            trusted_submitter,
            active_run_lookup: deps.active_run_lookup,
            fire_settlement_observer,
        },
    )?);
    deps.manual_fire_runner.bind(worker.clone())?;
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_trigger_poller(worker, settings, task_cancel).await;
    });
    Ok(Some(TriggerPollerRuntimeHandle { cancel, handle }))
}

const POST_SUBMIT_HOOK_PENDING_CAPACITY: usize = 256;

enum TriggerFireSettlement {
    Accepted(TriggerAcceptedFireSettlement),
    Failed(TriggerFailedFireSettlement),
}

fn spawn_post_submit_delivery(hook: Arc<dyn PostSubmitDeliveryHook>, event: TriggerFireSettlement) {
    tokio::spawn(async move {
        match event {
            TriggerFireSettlement::Accepted(event) => {
                hook.on_trigger_submitted(event.fire, event.run_id, event.turn_scope)
                    .await;
            }
            TriggerFireSettlement::Failed(event) => {
                hook.on_trigger_failed_before_submit(event).await;
            }
        }
    });
}

/// Bridges trigger-domain settlement notifications to the composition-owned
/// channel delivery hook and telemetry recorder. Delivery is detached from the
/// poller tick only after the worker has persisted either the accepted
/// run/thread mapping or the permanent pre-submit failure; terminal trigger
/// outcomes are recorded as best-effort telemetry observations.
pub(crate) struct TriggerSettlementObserver {
    pub(crate) hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
    telemetry_recorder: Arc<dyn TelemetryRecorder>,
    pending: Arc<Mutex<VecDeque<TriggerFireSettlement>>>,
    drain_scheduled: Arc<AtomicBool>,
    drain_cancel: CancellationToken,
}

impl TriggerSettlementObserver {
    fn with_telemetry_recorder(
        hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
        drain_cancel: CancellationToken,
        telemetry_recorder: Arc<dyn TelemetryRecorder>,
    ) -> Self {
        Self {
            hook_slot,
            telemetry_recorder,
            pending: Arc::new(Mutex::new(VecDeque::new())),
            drain_scheduled: Arc::new(AtomicBool::new(false)),
            drain_cancel,
        }
    }

    fn buffer_until_hook_installed(&self, event: TriggerFireSettlement) {
        {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.len() >= POST_SUBMIT_HOOK_PENDING_CAPACITY {
                pending.pop_front();
                tracing::debug!(
                    target: "ironclaw::reborn::trigger_poller",
                    pending_capacity = POST_SUBMIT_HOOK_PENDING_CAPACITY,
                    "post-submit hook startup buffer full; dropped oldest pending trigger settlement"
                );
            }
            pending.push_back(event);
        }
        self.ensure_drain_task();
    }

    fn ensure_drain_task(&self) {
        if self
            .drain_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let hook_slot = Arc::clone(&self.hook_slot);
        let pending = Arc::clone(&self.pending);
        let drain_scheduled = Arc::clone(&self.drain_scheduled);
        let drain_cancel = self.drain_cancel.clone();
        tokio::spawn(async move {
            loop {
                if let Some(hook) = hook_slot.get().cloned() {
                    let buffered = {
                        let mut pending = pending
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        pending.drain(..).collect::<Vec<_>>()
                    };
                    for event in buffered {
                        spawn_post_submit_delivery(Arc::clone(&hook), event);
                    }
                    drain_scheduled.store(false, Ordering::Release);
                    return;
                }
                tokio::select! {
                    _ = drain_cancel.cancelled() => {
                        drain_scheduled.store(false, Ordering::Release);
                        return;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                }
            }
        });
    }
}

#[async_trait]
impl TriggerFireSettlementObserver for TriggerSettlementObserver {
    async fn on_accepted_fire_settled(&self, event: TriggerAcceptedFireSettlement) {
        let Some(hook) = self.hook_slot.get().cloned() else {
            tracing::debug!(
                target: "ironclaw::reborn::trigger_poller",
                "post-submit hook not installed; buffering trigger settlement"
            );
            self.buffer_until_hook_installed(TriggerFireSettlement::Accepted(event));
            return;
        };
        spawn_post_submit_delivery(hook, TriggerFireSettlement::Accepted(event));
    }

    async fn on_failed_fire_settled(&self, event: TriggerFailedFireSettlement) {
        let Some(hook) = self.hook_slot.get().cloned() else {
            tracing::debug!(
                target: "ironclaw::reborn::trigger_poller",
                "post-submit hook not installed; buffering failed trigger settlement"
            );
            self.buffer_until_hook_installed(TriggerFireSettlement::Failed(event));
            return;
        };
        spawn_post_submit_delivery(hook, TriggerFireSettlement::Failed(event));
    }

    async fn on_run_terminal_settled(&self, event: TriggerRunTerminalSettlement) {
        let automation_id = match AutomationId::new(event.trigger_id.to_string()) {
            Ok(automation_id) => automation_id,
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::trigger_poller",
                    ?error,
                    trigger_id = %event.trigger_id,
                    "discarding invalid trigger terminal telemetry identity"
                );
                return;
            }
        };
        let observation = match AutomationSettledObservation::new(
            ObservationContext::new(event.fire_slot),
            automation_id,
            map_automation_kind(event.automation_kind),
            map_run_outcome(event.outcome),
        ) {
            Ok(observation) => observation,
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::trigger_poller",
                    ?error,
                    trigger_id = %event.trigger_id,
                    "discarding invalid trigger terminal telemetry observation"
                );
                return;
            }
        };
        let result = self.telemetry_recorder.try_record(
            event.scope,
            TelemetryObservation::AutomationSettled(observation),
        );
        if result != ironclaw_telemetry_contracts::recorder::RecordOutcome::Accepted {
            tracing::debug!(
                target: "ironclaw::reborn::trigger_poller",
                ?result,
                trigger_id = %event.trigger_id,
                run_id = %event.run_id,
                "trigger terminal telemetry recorder did not accept observation"
            );
        }
    }
}

fn map_automation_kind(kind: TriggerAutomationKind) -> AutomationKind {
    match kind {
        TriggerAutomationKind::Cron => AutomationKind::Cron,
        TriggerAutomationKind::Once => AutomationKind::Once,
        TriggerAutomationKind::Manual => AutomationKind::Manual,
    }
}

fn map_run_outcome(outcome: TriggerTerminalOutcome) -> RunOutcome {
    match outcome {
        TriggerTerminalOutcome::Completed => RunOutcome::Completed,
        TriggerTerminalOutcome::Failed => RunOutcome::Failed,
        TriggerTerminalOutcome::Cancelled => RunOutcome::Cancelled,
        TriggerTerminalOutcome::RecoveryRequired => RunOutcome::RecoveryRequired,
    }
}

async fn run_trigger_poller(
    worker: Arc<TriggerPollerWorker>,
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
    let nanos = rand::rng().random_range(0..=max_nanos);
    let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
    Duration::from_nanos(nanos)
}

#[cfg(test)]
#[path = "trigger_poller/tests.rs"]
mod tests;
