use std::sync::Arc;
#[cfg(feature = "slack-v2-host-beta")]
use std::sync::OnceLock;
use std::time::Duration;

#[cfg(feature = "slack-v2-host-beta")]
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_triggers::{
    ScheduleTriggerSourceProvider, TriggerActiveRunLookup, TriggerError, TriggerPollerWorker,
    TriggerPollerWorkerDeps, TriggerPromptMaterializer, TriggerRepository,
    TrustedTriggerFireSubmitter,
};
#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_triggers::{TrustedTriggerFireSubmitOutcome, TrustedTriggerSubmitRequest};
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

mod active_run_lookup;
pub(crate) use active_run_lookup::{
    LocalTriggerTurnSnapshotSource, SnapshotActiveRunLookup, TriggerTurnSnapshotSource,
};

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

/// Wraps a `TrustedTriggerFireSubmitter` to start a post-submit hook after each
/// successful fire submission. The hook is detached from the poller tick so
/// delivery latency cannot delay fire settlement. The hook is stored in a
/// `OnceLock` slot so it can be wired after the poller is spawned
/// (late-binding). If the slot is empty at submit time the hook is simply
/// skipped.
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
            if let Some(hook) = self.hook_slot.get().cloned() {
                let scope = scope.clone();
                tokio::spawn(async move {
                    hook.on_trigger_submitted(fire, run_id, scope).await;
                });
            } else {
                tracing::debug!(
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

    // ── PostSubmitHookWrappedSubmitter tests ────────────────────────────────

    #[cfg(feature = "slack-v2-host-beta")]
    mod hook_wrapper {
        use std::sync::atomic::{AtomicBool, Ordering};
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
        use tokio::sync::Notify;

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
            ) {
                self.calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((fire, run_id, scope));
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
                    ..TriggerPollerWorkerConfig::default()
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
            let calls = tokio::time::timeout(Duration::from_secs(1), recording.wait_for_calls(1))
                .await
                .expect("hook should be invoked asynchronously");
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

        /// A slow hook must not delay the poller from persisting the accepted
        /// run id; otherwise the trigger remains claim-only active and blocks
        /// later slots until the delivery task finishes.
        #[tokio::test]
        async fn filled_slot_slow_hook_does_not_block_trigger_settlement() {
            let repo = Arc::new(InMemoryTriggerRepository::default());
            let fire_slot = Utc::now() - chrono::Duration::seconds(1);
            let trigger_id = seed_due_trigger(&repo, fire_slot).await;

            let run_id = TurnRunId::new();
            let inner = Arc::new(FixedAcceptedSubmitter { run_id });
            let hook_slot: Arc<OnceLock<Arc<dyn PostSubmitDeliveryHook>>> =
                Arc::new(OnceLock::new());

            let entered = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let completed = Arc::new(AtomicBool::new(false));
            hook_slot
                .set(Arc::new(BlockingHook {
                    entered: Arc::clone(&entered),
                    release: Arc::clone(&release),
                    completed: Arc::clone(&completed),
                }) as Arc<dyn PostSubmitDeliveryHook>)
                .unwrap_or_else(|_| panic!("slot set should succeed on first call"));

            let wrapper = Arc::new(PostSubmitHookWrappedSubmitter {
                inner: inner as Arc<dyn TrustedTriggerFireSubmitter>,
                hook_slot: Arc::clone(&hook_slot),
            });

            let worker = build_worker_with_repo(
                Arc::clone(&repo),
                wrapper as Arc<dyn TrustedTriggerFireSubmitter>,
            );
            let report = tokio::time::timeout(Duration::from_secs(1), worker.tick_once(Utc::now()))
                .await
                .expect("slow post-submit hook must not block tick_once")
                .expect("tick_once succeeds");
            assert_eq!(report.due_records, 1, "one due trigger must be processed");

            tokio::time::timeout(Duration::from_secs(1), entered.notified())
                .await
                .expect("hook task should have started");
            assert!(
                !completed.load(Ordering::SeqCst),
                "hook must still be blocked until the test releases it"
            );

            let persisted = repo
                .get_trigger(wrapper_tenant(), trigger_id)
                .await
                .expect("load trigger")
                .expect("trigger present");
            assert_eq!(
                persisted.active_run_ref,
                Some(run_id),
                "accepted run id must be persisted before delivery hook completes"
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
    }
}
