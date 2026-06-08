use std::sync::Arc;

use async_trait::async_trait;

use crate::strategies::{ByteCapStrategy, CompactionForceStrategy};

use super::{AgentLoopExecutorError, ExecutorStage, StageContext, TurnCompletedStep};

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
    compaction_force: Arc<dyn CompactionForceStrategy>,
}

impl PostCapabilityStage {
    pub(crate) fn new(compaction_force: Arc<dyn CompactionForceStrategy>) -> Self {
        Self { compaction_force }
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
        Self::new(Arc::new(ByteCapStrategy::default()))
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
        _ctx: StageContext<'_>,
        input: TurnCompletedStep,
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        // Exit terminates the loop — state is discarded, no future reuse possible.
        // R2 drain and R1 policy check both apply only to the Continue path which
        // carries state forward.
        let TurnCompletedStep::Continue { mut state, summary } = input else {
            return Ok(input);
        };

        // R2: drain settled background children (no-op until producers exist).
        let _drained = self.drain_settled();

        // R1: proactive compaction policy check.
        // Only consult policy if any capability bytes accumulated this turn.
        // AssistantReply turns reach here with an empty map and gain nothing
        // from the policy scan + Arc<dyn> virtual dispatch.
        if !state
            .post_capability_state
            .pending_capability_bytes
            .is_empty()
            && let Some(initiator) = self.compaction_force.should_force_compact(&state)
        {
            state.compaction_state.force_compact_on_next_iteration = true;
            state.compaction_state.force_compact_initiator = Some(initiator);
            state.post_capability_state.skip_model_this_iteration = true;
            // CompactionStarted is emitted by PromptCompactionStep on the next
            // iteration when it actually runs the compactor. Threading the
            // initiator through state.compaction_state.force_compact_initiator
            // ensures the event reports CapabilityResultOverflow (or whichever
            // policy variant tripped) instead of falling back to Auto.
        }

        // Always clear the per-turn byte accumulator regardless of whether the
        // policy tripped. ByteCapStrategy doc states "during the current turn" —
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
    use crate::strategies::CompactionForceStrategy;
    use crate::strategies::TurnSummary;
    use crate::test_support::{MockAgentLoopDriverHost, test_run_context};

    use super::super::{ExecutorStage, StageContext, TurnCompletedStep};
    use super::PostCapabilityStage;

    /// Minimal stub strategy that always returns the same outcome.
    struct StubPolicy(Option<CompactionInitiator>);

    impl CompactionForceStrategy for StubPolicy {
        fn should_force_compact(&self, _state: &LoopExecutionState) -> Option<CompactionInitiator> {
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
        assert!(
            out.post_capability_state
                .pending_capability_bytes
                .is_empty()
        );
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
        assert_eq!(
            out.compaction_state.force_compact_initiator,
            Some(CompactionInitiator::CapabilityResultOverflow),
            "D-A: PostCapabilityStage must thread the initiator through \
             compaction_state.force_compact_initiator instead of emitting CompactionStarted"
        );
        assert!(out.post_capability_state.skip_model_this_iteration);
        assert!(
            out.post_capability_state
                .pending_capability_bytes
                .is_empty()
        );
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

    /// BUG-N1 regression: policy returns None but pending bytes are non-empty.
    /// Before the fix, only the trip-arm cleared the map, so a non-tripping
    /// capability turn would carry bytes over to the next iteration (causing
    /// cross-turn accumulation). After the fix, the unconditional clear at the
    /// end of process() must drain the map even when the policy returns None,
    /// while leaving both compaction flags false.
    #[tokio::test]
    async fn policy_none_with_nonempty_map_clears_map_without_setting_flags() {
        let stage = PostCapabilityStage::new(Arc::new(StubPolicy(None)));
        let ctx_data = test_run_context("post-cap-none-nonempty");
        let mut state = LoopExecutionState::initial_for_run(&ctx_data);

        // Pre-seed the byte accumulator with a non-empty entry to simulate a
        // capability turn that accumulated bytes but did not exceed the cap.
        let cap_id = CapabilityId::new("builtin.http").expect("valid");
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(cap_id, 100);

        assert!(
            !state
                .post_capability_state
                .pending_capability_bytes
                .is_empty(),
            "pre-condition: map must be non-empty before process()"
        );

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
        // Policy returned None → flags must stay false.
        assert!(
            !out.compaction_state.force_compact_on_next_iteration,
            "force_compact_on_next_iteration must stay false when policy returns None"
        );
        assert!(
            !out.post_capability_state.skip_model_this_iteration,
            "skip_model_this_iteration must stay false when policy returns None"
        );
        // BUG-N1 fix: map must be cleared unconditionally even on the non-trip path.
        assert!(
            out.post_capability_state
                .pending_capability_bytes
                .is_empty(),
            "pending_capability_bytes must be cleared even when the policy does not trip \
             (BUG-N1: non-trip turns were carrying bytes over to the next iteration)"
        );
    }
}
