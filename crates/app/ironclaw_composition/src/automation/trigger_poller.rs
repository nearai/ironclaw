use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_triggers::{
    ScheduleTriggerSourceProvider, TriggerActiveRunLookup, TriggerError, TriggerPollerWorker,
    TriggerPollerWorkerDeps, TriggerPromptMaterializer, TriggerRepository,
    TrustedTriggerFireSubmitter,
};
use ironclaw_triggers::{
    TriggerAcceptedFireSettlement, TriggerFailedFireSettlement, TriggerFireSettlementObserver,
    TriggerRunFailureSettlement,
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
    let fire_settlement_observer: Arc<dyn TriggerFireSettlementObserver> = Arc::new(
        PostSubmitHookObserver::new(deps.post_submit_hook_slot, cancel.clone()),
    );
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
/// channel delivery hook. Delivery is detached from the poller tick only after
/// the worker has persisted either the accepted run/thread mapping or the
/// permanent pre-submit failure.
pub(crate) struct PostSubmitHookObserver {
    pub(crate) hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
    install_cancel: CancellationToken,
}

impl PostSubmitHookObserver {
    fn new(
        hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
        install_cancel: CancellationToken,
    ) -> Self {
        Self {
            hook_slot,
            install_cancel,
        }
    }

    async fn wait_for_installed_hook(&self) -> Option<Arc<dyn PostSubmitDeliveryHook>> {
        loop {
            if let Some(hook) = self.hook_slot.get().cloned() {
                return Some(hook);
            }
            tokio::select! {
                _ = self.install_cancel.cancelled() => return None,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    }
}

#[async_trait]
impl TriggerFireSettlementObserver for PostSubmitHookObserver {
    async fn on_accepted_fire_settled(&self, event: TriggerAcceptedFireSettlement) {
        let hook = if let Some(hook) = self.hook_slot.get().cloned() {
            hook
        } else {
            tracing::debug!(
                target: "ironclaw::reborn::trigger_poller",
                "post-submit hook not installed; waiting before releasing trigger settlement"
            );
            let Some(hook) = self.wait_for_installed_hook().await else {
                return;
            };
            hook
        };
        spawn_post_submit_delivery(hook, TriggerFireSettlement::Accepted(event));
    }

    async fn on_failed_fire_settled(&self, event: TriggerFailedFireSettlement) {
        let hook = if let Some(hook) = self.hook_slot.get().cloned() {
            hook
        } else {
            tracing::debug!(
                target: "ironclaw::reborn::trigger_poller",
                "post-submit hook not installed; waiting before releasing failed trigger settlement"
            );
            let Some(hook) = self.wait_for_installed_hook().await else {
                return;
            };
            hook
        };
        spawn_post_submit_delivery(hook, TriggerFireSettlement::Failed(event));
    }

    async fn on_run_failure_settled(&self, event: TriggerRunFailureSettlement) {
        // The accepted fire's run reached a terminal failure state. The
        // canonical delivery watcher owns the creator-facing notice; this
        // observer only emits structured automation-health telemetry and
        // must never mint a replacement run or bypass the delivery path.
        tracing::warn!(
            target: "ironclaw::reborn::trigger_poller",
            tenant_id = %event.tenant_id,
            trigger_id = %event.trigger_id,
            fire_slot = %event.fire_slot,
            run_id = %event.run_id,
            "accepted trigger fire settled with a failed run"
        );
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
mod tests {
    use super::*;
    use ironclaw_triggers::TriggerPollerWorkerConfig;

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

    // ── PostSubmitHookObserver tests ────────────────────────────────────────

    mod post_submit_observer {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use super::super::PostSubmitDeliveryHook;
        use async_trait::async_trait;
        use chrono::Utc;
        use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId, UserId};
        use ironclaw_triggers::{
            TriggerAcceptedFireSettlement, TriggerFailedFireSettlement, TriggerFire,
            TriggerFireIdentity, TriggerFireSettlementObserver, TriggerId,
            TriggerPollerFailureReason, TriggerRunFailureSettlement,
        };
        use ironclaw_turns::{TurnRunId, TurnScope};
        use tokio::sync::Notify;
        use tokio_util::sync::CancellationToken;
        use tracing_test::traced_test;

        use super::super::PostSubmitHookObserver;

        #[derive(Default)]
        struct RecordingHook {
            calls: Mutex<Vec<(TriggerFire, TurnRunId, TurnScope)>>,
            failed_calls: Mutex<Vec<TriggerFailedFireSettlement>>,
            notify: Notify,
        }

        impl RecordingHook {
            fn calls(&self) -> Vec<(TriggerFire, TurnRunId, TurnScope)> {
                self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
            }

            async fn wait_for_calls(
                &self,
                expected: usize,
            ) -> Vec<(TriggerFire, TurnRunId, TurnScope)> {
                loop {
                    let calls = self.calls();
                    if calls.len() >= expected {
                        return calls;
                    }
                    self.notify.notified().await;
                }
            }

            async fn wait_for_failed_calls(
                &self,
                expected: usize,
            ) -> Vec<TriggerFailedFireSettlement> {
                loop {
                    let calls = self
                        .failed_calls
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    if calls.len() >= expected {
                        return calls;
                    }
                    self.notify.notified().await;
                }
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
                self.notify.notify_one();
            }

            async fn on_trigger_failed_before_submit(&self, event: TriggerFailedFireSettlement) {
                self.failed_calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(event);
                self.notify.notify_one();
            }
        }

        struct BlockingHook {
            entered: Arc<Notify>,
            release: Arc<Notify>,
            completed: Arc<AtomicBool>,
        }

        #[async_trait]
        impl PostSubmitDeliveryHook for BlockingHook {
            async fn on_trigger_submitted(
                &self,
                _fire: TriggerFire,
                _run_id: TurnRunId,
                _scope: TurnScope,
            ) {
                self.entered.notify_one();
                self.release.notified().await;
                self.completed.store(true, Ordering::SeqCst);
            }
        }

        fn observer_tenant() -> TenantId {
            TenantId::new("post-submit-observer-tenant").expect("tenant")
        }

        fn observer_thread_id(run_id: TurnRunId) -> ThreadId {
            ThreadId::new(format!("post-submit-observer-thread-{run_id}")).expect("thread id")
        }

        fn settlement_event(run_id: TurnRunId) -> TriggerAcceptedFireSettlement {
            let trigger_id = TriggerId::new();
            let fire = TriggerFire {
                identity: TriggerFireIdentity::new(observer_tenant(), trigger_id, Utc::now()),
                creator_user_id: UserId::new("hook-wrapper-user").expect("user"),
                agent_id: Some(AgentId::new("hook-wrapper-agent").expect("agent")),
                project_id: None,
                prompt: "hook wrapper test prompt".to_string(),
                execution_policy: None,
            };
            let scope = TurnScope::new_with_owner(
                observer_tenant(),
                fire.agent_id.clone(),
                None,
                observer_thread_id(run_id),
                Some(fire.creator_user_id.clone()),
            );
            TriggerAcceptedFireSettlement {
                fire,
                run_id,
                turn_scope: scope,
            }
        }

        fn failed_settlement_event() -> TriggerFailedFireSettlement {
            let accepted = settlement_event(TurnRunId::new());
            TriggerFailedFireSettlement {
                fire: accepted.fire,
                reason: TriggerPollerFailureReason::InvalidMaterialization,
            }
        }

        fn run_failure_settlement_event(run_id: TurnRunId) -> TriggerRunFailureSettlement {
            TriggerRunFailureSettlement {
                tenant_id: observer_tenant(),
                trigger_id: TriggerId::new(),
                fire_slot: Utc::now(),
                run_id,
            }
        }

        #[tokio::test]
        #[traced_test]
        async fn run_failure_settlement_emits_health_warning_without_delivery() {
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let recording = Arc::new(RecordingHook::default());
            let observer = PostSubmitHookObserver::new(hook_slot.clone(), CancellationToken::new());
            hook_slot.set(recording.clone()).ok().expect("hook install");

            observer
                .on_run_failure_settled(run_failure_settlement_event(TurnRunId::new()))
                .await;

            assert!(
                logs_contain("accepted trigger fire settled with a failed run"),
                "production observer must emit the automation-health signal; buffer: {:?}",
                String::from_utf8(
                    tracing_test::internal::global_buf()
                        .lock()
                        .expect("buf")
                        .to_vec()
                )
                .expect("utf8")
            );
            assert!(
                recording.calls().is_empty()
                    && recording
                        .failed_calls
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .is_empty(),
                "run-failure observation must not bypass the canonical delivery watcher"
            );
        }

        #[tokio::test]
        async fn uninstalled_hook_waits_until_hook_is_installed() {
            let run_id = TurnRunId::new();
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let observer = Arc::new(PostSubmitHookObserver::new(
                Arc::clone(&hook_slot),
                CancellationToken::new(),
            ));
            let recording = Arc::new(RecordingHook::default());

            let observer_task = {
                let observer = Arc::clone(&observer);
                tokio::spawn(async move {
                    observer
                        .on_accepted_fire_settled(settlement_event(run_id))
                        .await;
                })
            };

            assert!(
                tokio::time::timeout(Duration::from_millis(50), recording.wait_for_calls(1))
                    .await
                    .is_err(),
                "settlement must be buffered while hook is not installed"
            );
            hook_slot
                .set(Arc::clone(&recording) as Arc<dyn PostSubmitDeliveryHook>)
                .ok()
                .expect("first hook install must succeed");
            tokio::time::timeout(Duration::from_secs(1), observer_task)
                .await
                .expect("settlement caller should resume after hook install")
                .expect("settlement caller task should not panic");

            let calls = tokio::time::timeout(Duration::from_secs(1), recording.wait_for_calls(1))
                .await
                .expect("buffered settlement should be delivered after hook install");
            assert_eq!(calls[0].1, run_id);
        }

        #[tokio::test]
        async fn uninstalled_hook_backpressures_instead_of_dropping_settlement() {
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let observer = Arc::new(PostSubmitHookObserver::new(
                Arc::clone(&hook_slot),
                CancellationToken::new(),
            ));
            let recording = Arc::new(RecordingHook::default());
            let run_id = TurnRunId::new();
            let observer_task = {
                let observer = Arc::clone(&observer);
                tokio::spawn(async move {
                    observer
                        .on_accepted_fire_settled(settlement_event(run_id))
                        .await;
                })
            };

            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(
                !observer_task.is_finished(),
                "an uninstalled hook must backpressure the settlement caller instead of buffering it"
            );

            hook_slot
                .set(Arc::clone(&recording) as Arc<dyn PostSubmitDeliveryHook>)
                .ok()
                .expect("first hook install must succeed");

            tokio::time::timeout(Duration::from_secs(1), observer_task)
                .await
                .expect("settlement caller should resume after hook installation")
                .expect("settlement caller task should not panic");
            let calls = tokio::time::timeout(Duration::from_secs(1), recording.wait_for_calls(1))
                .await
                .expect("settlement should be delivered after hook installation");
            assert_eq!(calls[0].1, run_id);
        }

        #[tokio::test]
        async fn filled_slot_settlement_invokes_hook_with_run_id_and_scope() {
            let run_id = TurnRunId::new();
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let recording = Arc::new(RecordingHook::default());
            hook_slot
                .set(Arc::clone(&recording) as Arc<dyn PostSubmitDeliveryHook>)
                .ok()
                .expect("hook install");
            let observer = PostSubmitHookObserver::new(hook_slot, CancellationToken::new());

            observer
                .on_accepted_fire_settled(settlement_event(run_id))
                .await;

            let calls = tokio::time::timeout(Duration::from_secs(1), recording.wait_for_calls(1))
                .await
                .expect("hook should be invoked asynchronously");
            assert_eq!(calls.len(), 1, "hook must fire exactly once");

            let (recorded_fire, called_run_id, called_scope) = &calls[0];
            assert_eq!(
                *called_run_id, run_id,
                "hook must receive the accepted run_id"
            );
            let expected_thread_id = observer_thread_id(run_id);
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

        #[tokio::test]
        async fn filled_slot_failed_settlement_invokes_no_run_hook() {
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let recording = Arc::new(RecordingHook::default());
            hook_slot
                .set(Arc::clone(&recording) as Arc<dyn PostSubmitDeliveryHook>)
                .ok()
                .expect("hook install");
            let observer = PostSubmitHookObserver::new(hook_slot, CancellationToken::new());
            let event = failed_settlement_event();

            observer.on_failed_fire_settled(event.clone()).await;

            let calls =
                tokio::time::timeout(Duration::from_secs(1), recording.wait_for_failed_calls(1))
                    .await
                    .expect("failed settlement hook should be invoked asynchronously");
            assert_eq!(calls, vec![event]);
        }

        #[tokio::test]
        async fn filled_slot_slow_hook_does_not_block_observer() {
            let run_id = TurnRunId::new();
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let entered = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let completed = Arc::new(AtomicBool::new(false));
            hook_slot
                .set(Arc::new(BlockingHook {
                    entered: Arc::clone(&entered),
                    release: Arc::clone(&release),
                    completed: Arc::clone(&completed),
                }) as Arc<dyn PostSubmitDeliveryHook>)
                .ok()
                .expect("hook install");
            let observer = PostSubmitHookObserver::new(hook_slot, CancellationToken::new());

            observer
                .on_accepted_fire_settled(settlement_event(run_id))
                .await;

            tokio::time::timeout(Duration::from_secs(1), entered.notified())
                .await
                .expect("hook task should have started");
            assert!(
                !completed.load(Ordering::SeqCst),
                "hook must still be blocked until the test releases it"
            );

            release.notify_one();
            tokio::time::timeout(Duration::from_secs(1), async {
                while !completed.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("hook task should complete after release");
        }

        #[tokio::test]
        async fn uninstalled_hook_wait_exits_when_cancelled() {
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let cancel = CancellationToken::new();
            let observer = Arc::new(PostSubmitHookObserver::new(
                Arc::clone(&hook_slot),
                cancel.clone(),
            ));

            let observer_task = {
                let observer = Arc::clone(&observer);
                tokio::spawn(async move {
                    observer
                        .on_accepted_fire_settled(settlement_event(TurnRunId::new()))
                        .await;
                })
            };
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(
                !observer_task.is_finished(),
                "settlement caller should wait while the hook is uninstalled"
            );

            cancel.cancel();
            tokio::time::timeout(Duration::from_secs(1), observer_task)
                .await
                .expect("settlement caller should observe runtime cancellation")
                .expect("settlement caller task should not panic");
        }
    }
}
