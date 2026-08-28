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

// ── TriggerSettlementObserver tests ────────────────────────────────────

mod trigger_settlement_observer {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::super::PostSubmitDeliveryHook;
    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_host_api::{
        ids::{AgentId, InvocationId, TenantId, ThreadId, UserId},
        resource::ResourceScope,
    };
    use ironclaw_telemetry_contracts::{
        observation::{AutomationKind, RunOutcome, TelemetryObservation},
        recorder::{NoopTelemetryRecorder, RecordOutcome, TelemetryRecorder},
    };
    use ironclaw_triggers::{
        TriggerAcceptedFireSettlement, TriggerAutomationKind, TriggerFailedFireSettlement,
        TriggerFire, TriggerFireIdentity, TriggerFireSettlementObserver, TriggerId,
        TriggerPollerFailureReason, TriggerRunTerminalSettlement, TriggerTerminalOutcome,
    };
    use ironclaw_turns::{TurnRunId, TurnScope};
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::super::{POST_SUBMIT_HOOK_PENDING_CAPACITY, TriggerSettlementObserver};

    fn test_observer(
        hook_slot: Arc<std::sync::OnceLock<Arc<dyn PostSubmitDeliveryHook>>>,
        drain_cancel: CancellationToken,
    ) -> TriggerSettlementObserver {
        TriggerSettlementObserver::with_telemetry_recorder(
            hook_slot,
            drain_cancel,
            Arc::new(NoopTelemetryRecorder),
        )
    }

    struct RecordingTelemetryRecorder {
        calls: Mutex<Vec<(ResourceScope, TelemetryObservation)>>,
        outcome: RecordOutcome,
    }

    impl TelemetryRecorder for RecordingTelemetryRecorder {
        fn try_record(
            &self,
            scope: ResourceScope,
            observation: TelemetryObservation,
        ) -> RecordOutcome {
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((scope, observation));
            self.outcome
        }
    }

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

        async fn wait_for_failed_calls(&self, expected: usize) -> Vec<TriggerFailedFireSettlement> {
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

    fn terminal_settlement_event(run_id: TurnRunId) -> TriggerRunTerminalSettlement {
        TriggerRunTerminalSettlement {
            scope: ResourceScope {
                tenant_id: observer_tenant(),
                user_id: UserId::new("terminal-observer-user").expect("user"),
                agent_id: Some(AgentId::new("terminal-observer-agent").expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            trigger_id: TriggerId::new(),
            fire_slot: Utc::now(),
            run_id,
            automation_kind: TriggerAutomationKind::Once,
            outcome: TriggerTerminalOutcome::RecoveryRequired,
        }
    }

    #[tokio::test]
    async fn terminal_settlement_records_all_event_fields_once() {
        let recorder = Arc::new(RecordingTelemetryRecorder {
            calls: Mutex::new(Vec::new()),
            outcome: RecordOutcome::Accepted,
        });
        let observer = TriggerSettlementObserver::with_telemetry_recorder(
            Arc::new(std::sync::OnceLock::new()),
            CancellationToken::new(),
            recorder.clone(),
        );
        let event = terminal_settlement_event(TurnRunId::new());

        observer.on_run_terminal_settled(event.clone()).await;

        let calls = recorder.calls.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, event.scope);
        let TelemetryObservation::AutomationSettled(observation) = &calls[0].1 else {
            panic!("expected automation settled observation");
        };
        assert_eq!(observation.occurred_at(), event.fire_slot);
        assert_eq!(
            observation.automation_id().as_str(),
            &event.trigger_id.to_string()
        );
        assert_eq!(observation.automation_kind(), AutomationKind::Once);
        assert_eq!(observation.outcome(), RunOutcome::RecoveryRequired);
    }

    #[tokio::test]
    async fn terminal_settlement_recorder_loss_is_observational_only() {
        let recorder = Arc::new(RecordingTelemetryRecorder {
            calls: Mutex::new(Vec::new()),
            outcome: RecordOutcome::DroppedClosed,
        });
        let observer = TriggerSettlementObserver::with_telemetry_recorder(
            Arc::new(std::sync::OnceLock::new()),
            CancellationToken::new(),
            recorder.clone(),
        );

        observer
            .on_run_terminal_settled(terminal_settlement_event(TurnRunId::new()))
            .await;

        assert_eq!(
            recorder
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .len(),
            1,
            "recorder loss must not cause retries or duplicate settlement work"
        );
    }

    #[tokio::test]
    async fn uninstalled_hook_buffers_until_hook_is_installed() {
        let run_id = TurnRunId::new();
        let hook_slot = Arc::new(std::sync::OnceLock::new());
        let observer = test_observer(Arc::clone(&hook_slot), CancellationToken::new());
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
        let observer = test_observer(Arc::clone(&hook_slot), CancellationToken::new());
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
        let observer = test_observer(hook_slot, CancellationToken::new());

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
        let observer = test_observer(hook_slot, CancellationToken::new());
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
        let observer = test_observer(hook_slot, CancellationToken::new());

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
    async fn uninstalled_hook_drain_task_exits_when_cancelled() {
        let hook_slot = Arc::new(std::sync::OnceLock::new());
        let cancel = CancellationToken::new();
        let observer = test_observer(Arc::clone(&hook_slot), cancel.clone());

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
