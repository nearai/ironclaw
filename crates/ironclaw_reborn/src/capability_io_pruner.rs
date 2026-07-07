//! Terminal-run capability I/O pruning.
//!
//! Staged capability inputs are retained (not consumed) for the life of a run
//! so the executor's bounded capability retry can re-resolve the same
//! `input_ref`. They draw from a budget shared across every run on the writer,
//! so the composition must release them once the run can no longer read them.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::LoopCapabilityResultWriter;
use ironclaw_turns::{TurnCommittedEventObserver, TurnError, TurnLifecycleEvent, TurnRunState};

/// Calls [`LoopCapabilityResultWriter::prune_run`] once a run's committed
/// status reaches [`ironclaw_turns::TurnStatus::is_terminal`].
/// `RecoveryRequired` is included alongside `Completed`/`Failed`/`Cancelled`:
/// it is a compat status for legacy persisted records and is never re-entered
/// from a live run, so no run this observer prunes can resume under the same
/// run id afterward.
pub(crate) struct CapabilityIoTerminalPruner {
    writer: Arc<dyn LoopCapabilityResultWriter>,
}

impl CapabilityIoTerminalPruner {
    pub(crate) fn new(writer: Arc<dyn LoopCapabilityResultWriter>) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl TurnCommittedEventObserver for CapabilityIoTerminalPruner {
    fn observes_state(&self, state: &TurnRunState) -> bool {
        state.status.is_terminal()
    }

    fn observes_event(&self, event: &TurnLifecycleEvent) -> bool {
        event.status.is_terminal()
    }

    async fn observe_committed_state(&self, state: TurnRunState) -> Result<(), TurnError> {
        // Re-checked here, not just in `observes_state`: a run's staged io
        // must never be released while it can still retry (`Running`), even
        // if a bus dispatches without honoring the observe filter.
        if state.status.is_terminal() {
            self.writer.prune_run(&state.run_id.to_string());
        }
        Ok(())
    }

    async fn observe_committed_event(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if event.status.is_terminal() {
            self.writer.prune_run(&event.run_id.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, TenantId, ThreadId};
    use ironclaw_loop_support::{
        CapabilityResultWrite, CapabilityWriteResult, LoopCapabilityResultWriter,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, EventCursor, ReplyTargetBindingRef, RunProfileId, RunProfileVersion,
        SourceBindingRef, TurnCommittedEventObserver, TurnEventKind, TurnId, TurnLifecycleEvent,
        TurnRunId, TurnRunState, TurnScope, TurnStatus, run_profile::AgentLoopHostError,
    };

    use super::CapabilityIoTerminalPruner;

    fn test_run_state(run_id: TurnRunId, status: TurnStatus) -> TurnRunState {
        TurnRunState {
            scope: TurnScope::new(
                TenantId::new("tenant-pruner-test").unwrap(),
                Some(AgentId::new("agent-pruner-test").unwrap()),
                None,
                ThreadId::new("thread-pruner-test").unwrap(),
            ),
            actor: None,
            turn_id: TurnId::new(),
            run_id,
            status,
            accepted_message_ref: AcceptedMessageRef::new("message-pruner-test").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-pruner-test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-pruner-test").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            product_context: None,
            resume_disposition: None,
        }
    }

    struct SpyCapabilityResultWriter {
        pruned: Mutex<Vec<String>>,
    }

    impl SpyCapabilityResultWriter {
        fn new() -> Self {
            Self {
                pruned: Mutex::new(Vec::new()),
            }
        }

        fn pruned_run_ids(&self) -> Vec<String> {
            self.pruned.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for SpyCapabilityResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            unimplemented!("not exercised by CapabilityIoTerminalPruner tests")
        }

        fn prune_run(&self, run_id: &str) {
            self.pruned.lock().unwrap().push(run_id.to_string());
        }
    }

    /// Pins the exact reviewer-flagged gap: staged capability inputs must be
    /// released once a run reaches a terminal status, and never before.
    /// Mutation check: deleting `writer.prune_run(...)` from either
    /// `observe_committed_state`/`observe_committed_event` turns this red.
    #[tokio::test]
    async fn prunes_only_terminal_runs() {
        let spy = Arc::new(SpyCapabilityResultWriter::new());
        let pruner = CapabilityIoTerminalPruner::new(
            Arc::clone(&spy) as Arc<dyn LoopCapabilityResultWriter>
        );

        let running_run_id = TurnRunId::new();
        let running_state = test_run_state(running_run_id, TurnStatus::Running);
        assert!(
            !pruner.observes_state(&running_state),
            "a still-running state must not be observed for pruning"
        );
        let running_event = TurnLifecycleEvent::from_run_state(
            &running_state,
            TurnEventKind::RunnerHeartbeat,
            None,
        );
        assert!(
            !pruner.observes_event(&running_event),
            "a still-running event must not be observed for pruning"
        );
        pruner
            .observe_committed_event(running_event)
            .await
            .expect("observing a non-terminal event must not fail the run");
        assert!(
            spy.pruned_run_ids().is_empty(),
            "a running run's staged capability io must not be pruned"
        );

        let completed_run_id = TurnRunId::new();
        let completed_state = test_run_state(completed_run_id, TurnStatus::Completed);
        assert!(
            pruner.observes_state(&completed_state),
            "a completed state must be observed for pruning"
        );
        pruner
            .observe_committed_state(completed_state.clone())
            .await
            .expect("pruning a completed run must not fail");
        assert_eq!(
            spy.pruned_run_ids(),
            vec![completed_run_id.to_string()],
            "the completed run's staged capability io must be pruned exactly once"
        );

        let completed_event =
            TurnLifecycleEvent::from_run_state(&completed_state, TurnEventKind::Completed, None);
        assert!(pruner.observes_event(&completed_event));
        pruner
            .observe_committed_event(completed_event)
            .await
            .expect("pruning via the event path must not fail");
        assert_eq!(
            spy.pruned_run_ids(),
            vec![completed_run_id.to_string(), completed_run_id.to_string()],
            "both the state and event dispatch paths must prune the terminal run"
        );
    }
}
