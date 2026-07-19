#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use std::collections::VecDeque;
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use std::future::Future;
use std::sync::Arc;
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_triggers::{
    ScheduleTriggerSourceProvider, TriggerActiveRunLookup, TriggerError, TriggerPollerWorker,
    TriggerPollerWorkerDeps, TriggerPromptMaterializer, TriggerRepository,
    TrustedTriggerFireSubmitter,
};
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use ironclaw_triggers::{TriggerAcceptedFireSettlement, TriggerFireSettlementObserver};
use rand::RngExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(crate) use crate::automation::trigger_poller_trusted_submit::AccessCheckerTriggerFireAuthorizer;
pub(crate) use crate::automation::trigger_poller_trusted_submit::ConversationContentRefMaterializer;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use crate::automation::trigger_poller_trusted_submit::TenantScopedTrustedTriggerFireAuthorizer;
use crate::runtime_input::TriggerPollerSettings;
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use ironclaw_channel_delivery::PostSubmitDeliveryHook;

mod active_run_lookup;
pub(crate) use active_run_lookup::SnapshotActiveRunLookup;

pub(crate) const TRIGGER_POLLER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct TriggerPollerRuntimeHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    post_submit_tasks: Arc<PostSubmitDeliveryTaskOwner>,
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
        #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
        self.post_submit_tasks.drain(timeout).await;
    }
}

#[derive(Clone)]
pub(crate) struct TriggerPollerCompositionDeps {
    pub(crate) repository: Arc<dyn TriggerRepository>,
    pub(crate) materializer: Arc<dyn TriggerPromptMaterializer>,
    pub(crate) trusted_submitter: Arc<dyn TrustedTriggerFireSubmitter>,
    pub(crate) active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
    /// Late-binding slot for the post-submit delivery hook.
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
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
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    let post_submit_observer = Arc::new(PostSubmitHookObserver::new(
        deps.post_submit_hook_slot,
        cancel.clone(),
    ));
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    let post_submit_tasks = Arc::clone(&post_submit_observer.task_owner);
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    let fire_settlement_observer: Arc<dyn TriggerFireSettlementObserver> = post_submit_observer;
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    let trusted_submitter = deps.trusted_submitter;
    #[cfg(not(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta")))]
    let fire_settlement_observer: Arc<dyn ironclaw_triggers::TriggerFireSettlementObserver> =
        Arc::new(ironclaw_triggers::NoopTriggerFireSettlementObserver);
    #[cfg(not(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta")))]
    let trusted_submitter = deps.trusted_submitter;
    let worker = TriggerPollerWorker::new(
        settings.worker.clone(),
        TriggerPollerWorkerDeps {
            repository: deps.repository,
            source_provider: Arc::new(ScheduleTriggerSourceProvider),
            materializer: deps.materializer,
            trusted_submitter,
            active_run_lookup: deps.active_run_lookup,
            fire_settlement_observer,
        },
    )?;
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_trigger_poller(worker, settings, task_cancel).await;
    });
    Ok(Some(TriggerPollerRuntimeHandle {
        cancel,
        handle,
        #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
        post_submit_tasks,
    }))
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
const POST_SUBMIT_HOOK_PENDING_CAPACITY: usize = 256;

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
#[derive(Default)]
struct PostSubmitDeliveryTaskOwner {
    tasks: tokio::sync::Mutex<tokio::task::JoinSet<()>>,
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
impl PostSubmitDeliveryTaskOwner {
    async fn spawn(
        &self,
        hook: Arc<dyn PostSubmitDeliveryHook>,
        event: TriggerAcceptedFireSettlement,
    ) {
        self.spawn_task(run_post_submit_hook(hook, event)).await;
    }

    async fn spawn_task(&self, task: impl Future<Output = ()> + Send + 'static) {
        let mut tasks = self.tasks.lock().await;
        while let Some(joined) = tasks.try_join_next() {
            if let Err(error) = joined {
                tracing::debug!(%error, "post-submit delivery task join failed");
            }
        }
        if tasks.len() >= POST_SUBMIT_HOOK_PENDING_CAPACITY
            && let Some(joined) = tasks.join_next().await
            && let Err(error) = joined
        {
            tracing::debug!(%error, "post-submit delivery task join failed");
        }
        tasks.spawn(task);
    }

    async fn drain(&self, timeout: Duration) {
        let drain = async {
            let mut tasks = self.tasks.lock().await;
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined {
                    tracing::debug!(%error, "post-submit delivery task join failed during drain");
                }
            }
        };
        if tokio::time::timeout(timeout, drain).await.is_err() {
            tracing::warn!(
                ?timeout,
                "post-submit delivery tasks did not drain before shutdown timeout; aborting"
            );
            let mut tasks = self.tasks.lock().await;
            tasks.abort_all();
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined
                    && error.is_panic()
                {
                    tracing::debug!(%error, "aborted post-submit delivery task panicked");
                }
            }
        }
    }
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
async fn run_post_submit_hook(
    hook: Arc<dyn PostSubmitDeliveryHook>,
    event: TriggerAcceptedFireSettlement,
) {
    let run_id = event.run_id;
    if let Err(error) = hook
        .on_trigger_submitted(event.fire, run_id, event.turn_scope)
        .await
    {
        tracing::warn!(
            target = "ironclaw::reborn::trigger_poller",
            %run_id,
            %error,
            "managed post-submit delivery did not reach an authoritative terminal state"
        );
    }
}

/// Bridges trigger-domain settlement notifications to the composition-owned
/// Slack delivery hook. Delivery is detached from the poller tick only after the
/// worker has persisted the accepted run/thread mapping.
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
pub(crate) struct PostSubmitHookObserver {
    pub(crate) hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
    pending: Arc<Mutex<VecDeque<TriggerAcceptedFireSettlement>>>,
    drain_scheduled: Arc<AtomicBool>,
    drain_cancel: CancellationToken,
    task_owner: Arc<PostSubmitDeliveryTaskOwner>,
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
impl PostSubmitHookObserver {
    fn new(
        hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
        drain_cancel: CancellationToken,
    ) -> Self {
        Self {
            hook_slot,
            pending: Arc::new(Mutex::new(VecDeque::new())),
            drain_scheduled: Arc::new(AtomicBool::new(false)),
            drain_cancel,
            task_owner: Arc::new(PostSubmitDeliveryTaskOwner::default()),
        }
    }

    async fn buffer_until_hook_installed(&self, event: TriggerAcceptedFireSettlement) {
        {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.len() >= POST_SUBMIT_HOOK_PENDING_CAPACITY {
                pending.pop_front();
                tracing::debug!(
                    target = "ironclaw::reborn::trigger_poller",
                    pending_capacity = POST_SUBMIT_HOOK_PENDING_CAPACITY,
                    "post-submit hook startup buffer full; dropped oldest pending trigger settlement"
                );
            }
            pending.push_back(event);
        }
        self.ensure_drain_task().await;
    }

    async fn ensure_drain_task(&self) {
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
        let owner = Arc::clone(&self.task_owner);
        owner
            .spawn_task(async move {
                loop {
                    if let Some(hook) = hook_slot.get().cloned() {
                        let buffered = {
                            let mut pending = pending
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            pending.drain(..).collect::<Vec<_>>()
                        };
                        // The startup buffer is bounded. A local joined set
                        // preserves concurrency while the owner retains one
                        // shutdown handle for the complete drain operation.
                        let mut deliveries = tokio::task::JoinSet::new();
                        for event in buffered {
                            deliveries.spawn(run_post_submit_hook(Arc::clone(&hook), event));
                        }
                        while let Some(joined) = deliveries.join_next().await {
                            if let Err(error) = joined {
                                tracing::debug!(%error, "buffered post-submit delivery task join failed");
                            }
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
            })
            .await;
    }
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
#[async_trait]
impl TriggerFireSettlementObserver for PostSubmitHookObserver {
    async fn on_accepted_fire_settled(&self, event: TriggerAcceptedFireSettlement) {
        let Some(hook) = self.hook_slot.get().cloned() else {
            tracing::debug!(
                target = "ironclaw::reborn::trigger_poller",
                "post-submit hook not installed; buffering trigger settlement"
            );
            self.buffer_until_hook_installed(event).await;
            return;
        };
        self.task_owner.spawn(hook, event).await;
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
        let runtime_handle = TriggerPollerRuntimeHandle {
            cancel,
            handle,
            #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
            post_submit_tasks: Arc::new(PostSubmitDeliveryTaskOwner::default()),
        };

        runtime_handle.shutdown(Duration::from_millis(1)).await;
    }

    // ── PostSubmitHookObserver tests ────────────────────────────────────────

    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    mod post_submit_observer {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use async_trait::async_trait;
        use chrono::Utc;
        use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
        use ironclaw_triggers::{
            TriggerAcceptedFireSettlement, TriggerFire, TriggerFireIdentity,
            TriggerFireSettlementObserver, TriggerId,
        };
        use ironclaw_turns::{TurnRunId, TurnScope};
        use tokio::sync::Notify;
        use tokio_util::sync::CancellationToken;

        use super::super::{POST_SUBMIT_HOOK_PENDING_CAPACITY, PostSubmitHookObserver};
        use ironclaw_channel_delivery::PostSubmitDeliveryHook;

        #[derive(Default)]
        struct RecordingHook {
            calls: Mutex<Vec<(TriggerFire, TurnRunId, TurnScope)>>,
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
        }

        #[async_trait]
        impl PostSubmitDeliveryHook for RecordingHook {
            async fn on_trigger_submitted(
                &self,
                fire: TriggerFire,
                run_id: TurnRunId,
                scope: TurnScope,
            ) -> Result<(), ironclaw_channel_delivery::PostSubmitDeliveryError> {
                self.calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((fire, run_id, scope));
                self.notify.notify_one();
                Ok(())
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
            ) -> Result<(), ironclaw_channel_delivery::PostSubmitDeliveryError> {
                self.entered.notify_one();
                self.release.notified().await;
                self.completed.store(true, Ordering::SeqCst);
                Ok(())
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
                delivery_target: None,
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

        #[tokio::test]
        async fn uninstalled_hook_buffers_until_hook_is_installed() {
            let run_id = TurnRunId::new();
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let observer =
                PostSubmitHookObserver::new(Arc::clone(&hook_slot), CancellationToken::new());
            let recording = Arc::new(RecordingHook::default());

            observer
                .on_accepted_fire_settled(settlement_event(run_id))
                .await;

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

            let calls = tokio::time::timeout(Duration::from_secs(1), recording.wait_for_calls(1))
                .await
                .expect("buffered settlement should be delivered after hook install");
            assert_eq!(calls[0].1, run_id);
        }

        #[tokio::test]
        async fn uninstalled_hook_buffer_drops_oldest_when_full() {
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let observer =
                PostSubmitHookObserver::new(Arc::clone(&hook_slot), CancellationToken::new());
            let recording = Arc::new(RecordingHook::default());
            let run_ids: Vec<_> = (0..=POST_SUBMIT_HOOK_PENDING_CAPACITY)
                .map(|_| TurnRunId::new())
                .collect();

            for run_id in run_ids.iter().copied() {
                observer
                    .on_accepted_fire_settled(settlement_event(run_id))
                    .await;
            }

            hook_slot
                .set(Arc::clone(&recording) as Arc<dyn PostSubmitDeliveryHook>)
                .ok()
                .expect("first hook install must succeed");

            let calls = tokio::time::timeout(
                Duration::from_secs(1),
                recording.wait_for_calls(POST_SUBMIT_HOOK_PENDING_CAPACITY),
            )
            .await
            .expect("capped buffered settlements should be delivered after hook install");
            let delivered_run_ids: Vec<_> = calls
                .iter()
                .map(|(_, delivered_run_id, _)| *delivered_run_id)
                .collect();
            assert_eq!(
                delivered_run_ids.len(),
                POST_SUBMIT_HOOK_PENDING_CAPACITY,
                "startup buffer must deliver only the capped number of settlements"
            );
            assert!(
                !delivered_run_ids.contains(&run_ids[0]),
                "oldest settlement must be dropped on overflow"
            );
            assert!(
                delivered_run_ids.contains(run_ids.last().expect("run ids")),
                "newest settlement must be retained on overflow"
            );
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

            let task_owner = Arc::clone(&observer.task_owner);
            let mut drain = tokio::spawn(async move {
                task_owner.drain(Duration::from_secs(1)).await;
            });
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut drain)
                    .await
                    .is_err(),
                "the lifecycle owner must retain and await the in-flight hook"
            );
            release.notify_one();
            tokio::time::timeout(Duration::from_secs(1), drain)
                .await
                .expect("hook task should drain after release")
                .expect("drain task joins");
            assert!(completed.load(Ordering::SeqCst));
        }

        #[tokio::test]
        async fn uninstalled_hook_drain_task_exits_when_cancelled() {
            let hook_slot = Arc::new(std::sync::OnceLock::new());
            let cancel = CancellationToken::new();
            let observer = PostSubmitHookObserver::new(Arc::clone(&hook_slot), cancel.clone());

            observer
                .on_accepted_fire_settled(settlement_event(TurnRunId::new()))
                .await;
            assert!(
                observer.drain_scheduled.load(Ordering::SeqCst),
                "buffered settlement should schedule a drain task"
            );

            cancel.cancel();
            tokio::time::timeout(Duration::from_secs(1), async {
                while observer.drain_scheduled.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("drain task should observe runtime cancellation");
        }
    }
}
