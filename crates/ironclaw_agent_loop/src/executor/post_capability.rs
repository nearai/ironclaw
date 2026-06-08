use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_turns::run_profile::{LoopProgressEvent, SystemInferenceTaskId};

use crate::strategies::{ByteCapPolicy, CompactionPolicy};

use super::{
    AgentLoopExecutorError, CheckpointStage, ExecutorStage, StageContext, TurnCompletedStep,
};

/// Owns post-capability lifecycle — the seam between `CapabilityStage`
/// and `StopStage.observe()`.
///
/// **R1 (active):** proactive compaction policy evaluation. Reads
/// per-capability byte accumulation on
/// `state.post_capability_state.pending_capability_bytes` (populated by
/// `push_completed_result`) and decides whether the next prompt build
/// should compact-then-skip-the-model.
///
/// **R2 (owner of record, no-op until #4474):** mailbox drain for
/// settled background-mode subagent children. Producer side (durable
/// settlement log + `LoopBackgroundChildPort`) lands in WU-C through
/// WU-E. Until then `drain_settled` returns an empty `Vec` — this stage
/// owns the seam so all post-capability responsibilities live in one
/// file (single-seam thesis per the WU-A design doc).
#[derive(Clone)]
pub(crate) struct PostCapabilityStage {
    policy: Arc<dyn CompactionPolicy>,
}

impl PostCapabilityStage {
    pub(crate) fn new(policy: Arc<dyn CompactionPolicy>) -> Self {
        Self { policy }
    }

    /// R2 — drain settled background-mode subagent results.
    /// Returns an empty `Vec` until durable settlement log +
    /// `LoopBackgroundChildPort` land (#4474).
    fn drain_settled(&self) -> Vec<()> {
        Vec::new()
    }
}

impl Default for PostCapabilityStage {
    fn default() -> Self {
        Self::new(Arc::new(ByteCapPolicy::default()))
    }
}

impl std::fmt::Debug for PostCapabilityStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostCapabilityStage").finish()
    }
}

#[async_trait]
impl ExecutorStage<TurnCompletedStep> for PostCapabilityStage {
    type Output = TurnCompletedStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: TurnCompletedStep,
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        // R2: drain settled background children (no-op until producers exist).
        let _drained = self.drain_settled();

        // Exit propagates untouched.
        let TurnCompletedStep::Continue { mut state, summary } = input else {
            return Ok(input);
        };

        // R1: proactive compaction policy check.
        // Only consult policy if any capability bytes accumulated this turn.
        // AssistantReply turns reach here with an empty map and gain nothing
        // from the policy scan + Arc<dyn> virtual dispatch.
        if !state.post_capability_state.pending_capability_bytes.is_empty() {
            if let Some(initiator) = self.policy.should_force_compact(&state) {
                state.compaction_state.force_compact_on_next_iteration = true;
                state.post_capability_state.skip_model_this_iteration = true;

                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::CompactionStarted {
                            task_id: SystemInferenceTaskId::new(),
                            initiator,
                        },
                    )
                    .await;
            }
        }

        // Always clear the per-turn byte accumulator regardless of whether the
        // policy tripped. ByteCapPolicy doc states "during the current turn" —
        // carrying entries across turns would cause cross-turn accumulation and
        // false-positive trips on subsequent AssistantReply turns. Map is cheap
        // to drop and re-populate per turn.
        state.post_capability_state.pending_capability_bytes.clear();

        Ok(TurnCompletedStep::Continue { state, summary })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::CapabilityId;
    use ironclaw_turns::{LoopExit, LoopExitId, LoopFailureKind, run_profile::CompactionInitiator};

    use crate::state::LoopExecutionState;
    use crate::strategies::CompactionPolicy;
    use crate::strategies::TurnSummary;
    use crate::test_support::{MockAgentLoopDriverHost, test_run_context};

    use super::super::{ExecutorStage, StageContext, TurnCompletedStep};
    use super::PostCapabilityStage;

    /// Minimal stub policy that always returns the same outcome.
    struct StubPolicy(Option<CompactionInitiator>);

    impl CompactionPolicy for StubPolicy {
        fn should_force_compact(
            &self,
            _state: &LoopExecutionState,
        ) -> Option<CompactionInitiator> {
            self.0
        }
    }

    fn make_host() -> MockAgentLoopDriverHost {
        MockAgentLoopDriverHost::builder().build().0
    }

    fn make_family() -> crate::family::LoopFamily {
        crate::families::default()
    }

    /// Policy returns None — stage passes input through unchanged, no flags set.
    #[tokio::test]
    async fn policy_none_passes_through_unchanged() {
        let stage = PostCapabilityStage::new(Arc::new(StubPolicy(None)));
        let ctx_data = test_run_context("post-cap-none");
        let state = LoopExecutionState::initial_for_run(&ctx_data);

        assert!(!state.compaction_state.force_compact_on_next_iteration);
        assert!(!state.post_capability_state.skip_model_this_iteration);

        let summary = TurnSummary::reply_rejected();
        let input = TurnCompletedStep::Continue {
            state: Box::new(state),
            summary,
        };

        let host = make_host();
        let family = make_family();
        let ctx = StageContext {
            planner: family.planner(),
            host: &host,
        };

        let result = stage.process(ctx, input).await.expect("process ok");

        let TurnCompletedStep::Continue { state: out, .. } = result else {
            panic!("expected Continue");
        };
        assert!(!out.compaction_state.force_compact_on_next_iteration);
        assert!(!out.post_capability_state.skip_model_this_iteration);
        assert!(out.post_capability_state.pending_capability_bytes.is_empty());
    }

    /// Policy returns Some(...) — both flags set and byte map cleared.
    #[tokio::test]
    async fn policy_some_sets_flags_and_clears_bytes() {
        let stage = PostCapabilityStage::new(Arc::new(StubPolicy(Some(
            CompactionInitiator::CapabilityResultOverflow,
        ))));
        let ctx_data = test_run_context("post-cap-some");
        let mut state = LoopExecutionState::initial_for_run(&ctx_data);

        // Pre-populate the byte accumulator so we can verify it is cleared.
        let cap_id = CapabilityId::new("builtin.http").expect("valid");
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(cap_id, 99_999);

        let summary = TurnSummary::reply_rejected();
        let input = TurnCompletedStep::Continue {
            state: Box::new(state),
            summary,
        };

        let host = make_host();
        let family = make_family();
        let ctx = StageContext {
            planner: family.planner(),
            host: &host,
        };

        let result = stage.process(ctx, input).await.expect("process ok");

        let TurnCompletedStep::Continue { state: out, .. } = result else {
            panic!("expected Continue");
        };
        assert!(out.compaction_state.force_compact_on_next_iteration);
        assert!(out.post_capability_state.skip_model_this_iteration);
        assert!(out.post_capability_state.pending_capability_bytes.is_empty());
    }

    /// Exit variant passes through untouched — R1 and R2 skipped.
    #[tokio::test]
    async fn exit_passes_through_untouched() {
        // Even with a policy that would trip compaction, an Exit input is
        // returned as-is without mutating any state or emitting events.
        let stage = PostCapabilityStage::new(Arc::new(StubPolicy(Some(
            CompactionInitiator::CapabilityResultOverflow,
        ))));

        let exit_id = LoopExitId::new("exit:test-passthrough").expect("valid");
        let loop_exit = LoopExit::failed(LoopFailureKind::DriverBug, exit_id);
        let input = TurnCompletedStep::Exit(loop_exit);

        let host = make_host();
        let family = make_family();
        let ctx = StageContext {
            planner: family.planner(),
            host: &host,
        };

        let result = stage.process(ctx, input).await.expect("process ok");
        assert!(matches!(result, TurnCompletedStep::Exit(_)));
    }
}
