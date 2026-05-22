use super::*;

impl CanonicalAgentLoopExecutor {
    pub(super) async fn handle_gate(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        kind: GateKind,
        gate_ref: ironclaw_turns::LoopGateRef,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let summary = crate::strategies::GateSummary {
            kind,
            gate_ref: gate_ref.clone(),
        };
        match planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { gate } => {
                state.gate_state = gate;
                state.last_gate = Some(gate_ref.clone());
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                self.emit_progress(
                    host,
                    LoopProgressEvent::GateBlocked {
                        iteration: state.iteration,
                        gate_kind: loop_gate_kind(kind),
                    },
                )
                .await;
                let checked = self
                    .checkpoint(host, state, CheckpointKind::BeforeBlock)
                    .await?;
                Ok(BatchStep::Exit(LoopExit::Blocked(LoopBlocked {
                    kind: blocked_kind(kind),
                    gate_ref,
                    checkpoint_id: checked.checkpoint_id,
                    state_ref: checked.state_ref,
                    exit_id: exit_id(host, "blocked")?,
                })))
            }
            GateOutcome::SkipAndContinue { gate } => {
                state.gate_state = gate;
                append_capability_safe_summary_ref(
                    host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "skipped"),
                )
                .await?;
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                Ok(BatchStep::Continue(Box::new(state)))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                append_capability_safe_summary_ref(
                    host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "aborted"),
                )
                .await?;
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok(BatchStep::Exit(failed_exit(
                    host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )?))
            }
        }
    }
}
