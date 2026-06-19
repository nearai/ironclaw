use async_trait::async_trait;
use ironclaw_turns::{
    LoopBlocked, LoopExit,
    run_profile::{
        CapabilityApprovalResume, CapabilityCallCandidate, CapabilityResultMessage,
        LoopProgressEvent,
    },
};

use crate::{
    state::{
        CheckpointKind, LoopExecutionState, PendingApprovalResume, PendingAuthResume,
        capability_activity_id_from_resume_token,
    },
    strategies::{GateKind, GateOutcome},
};

use super::{
    AgentLoopExecutorError, BatchStep, CancelCheck, CheckpointStage, ExecutorStage, StageContext,
    append_capability_result_ref, append_capability_safe_summary_ref, blocked_kind,
    clear_matching_pending_auth_resume, exit_id, failed_exit, gate_tool_result_summary,
    loop_gate_kind, push_completed_result,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GateStage;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AwaitDependentRunGateStage;

pub(super) struct GateInput {
    pub(super) state: LoopExecutionState,
    pub(super) call: CapabilityCallCandidate,
    pub(super) kind: GateKind,
    pub(super) gate_ref: ironclaw_turns::LoopGateRef,
    pub(super) credential_requirements: Vec<ironclaw_host_api::RuntimeCredentialAuthRequirement>,
    pub(super) approval_resume: Option<CapabilityApprovalResume>,
    pub(super) auth_resume: Option<ironclaw_turns::run_profile::CapabilityAuthResume>,
}

pub(super) struct AwaitDependentRunGateInput {
    pub(super) state: LoopExecutionState,
    pub(super) call: CapabilityCallCandidate,
    pub(super) gate_ref: ironclaw_turns::LoopGateRef,
    pub(super) resolved_result: CapabilityResultMessage,
}

#[async_trait]
impl ExecutorStage<GateInput> for GateStage {
    type Output = BatchStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: GateInput,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let call = input.call;
        let kind = input.kind;
        let gate_ref = input.gate_ref;
        let summary = crate::strategies::GateSummary {
            kind,
            gate_ref: gate_ref.clone(),
        };
        match ctx.planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { gate } => {
                state.gate_state = gate;
                state.last_gate = Some(gate_ref.clone());
                let auth_resume = input.auth_resume.as_ref();
                let auth_resume_token = auth_resume.map(|r| r.resume_token.clone());
                let auth_activity_id = auth_resume_token
                    .as_ref()
                    .and_then(capability_activity_id_from_resume_token);
                let auth_replay = auth_resume.and_then(|r| r.replay.clone());
                let auth_prior_approval = auth_resume.and_then(|r| r.prior_approval.clone());
                if matches!(kind, GateKind::Approval) {
                    let approval_resume = input.approval_resume;
                    state.pending_approval_resume = approval_resume.map(|resume| {
                        let activity_id =
                            capability_activity_id_from_resume_token(&resume.resume_token);
                        PendingApprovalResume {
                            gate_ref: gate_ref.clone(),
                            capability_id: call.capability_id.clone(),
                            approval_request_id: resume.approval_request_id,
                            resume_token: resume.resume_token,
                            activity_id,
                            correlation_id: resume.correlation_id,
                            surface_version: call.surface_version.clone(),
                            input_ref: resume.input_ref,
                            effective_capability_ids: call.effective_capability_ids.clone(),
                            provider_replay: call.provider_replay.clone(),
                            input: resume.input,
                            estimate: resume.estimate,
                            // Disposition is stamped by PlannedDriver at resume time;
                            // GateStage writes the initial (blocking) checkpoint where
                            // no denial has occurred yet.
                            disposition: None,
                        }
                    });
                } else if matches!(kind, GateKind::Auth) {
                    // Auth gates fold any prior approval identity into
                    // pending_auth_resume.prior_approval below. Keeping a
                    // pending approval slot for the same gate makes resume
                    // disposition stamping ambiguous and can re-dispatch the
                    // approval path before the auth denial is consumed.
                    state.pending_approval_resume = None;
                }
                if matches!(kind, GateKind::Auth) {
                    // CapabilityStage shapes auth-resume metadata; GateStage
                    // only persists it at the blocking checkpoint.
                    state.pending_auth_resume = Some(PendingAuthResume {
                        gate_ref: gate_ref.clone(),
                        capability_id: call.capability_id.clone(),
                        surface_version: call.surface_version.clone(),
                        input_ref: call.input_ref.clone(),
                        effective_capability_ids: call.effective_capability_ids.clone(),
                        provider_replay: call.provider_replay.clone(),
                        resume_token: auth_resume_token,
                        activity_id: auth_activity_id,
                        prior_approval: auth_prior_approval,
                        replay: auth_replay,
                        disposition: None,
                    });
                }
                // Non-auth blocks do not invalidate a pending auth resume: a resource or
                // approval gate can fire mid-re-dispatch, and clearing here would erase the
                // record before it is consumed. Clearing on completion happens in the
                // capability stage.
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::GateBlocked {
                            iteration: state.iteration,
                            gate_kind: loop_gate_kind(kind),
                        },
                    )
                    .await;
                let checked = CheckpointStage
                    .write_before_block(ctx, state, &gate_ref)
                    .await?;
                Ok(BatchStep::Exit(LoopExit::Blocked(LoopBlocked {
                    kind: blocked_kind(kind),
                    gate_ref,
                    credential_requirements: input.credential_requirements,
                    checkpoint_id: checked.checkpoint_id,
                    state_ref: checked.state_ref,
                    exit_id: exit_id(ctx.host, "blocked")?,
                })))
            }
            GateOutcome::SkipAndContinue { gate } => {
                state.gate_state = gate;
                // A skipped gate bypasses all capability-outcome clear sites, so a
                // pending_auth_resume for this call would survive and trigger an
                // infinite re-dispatch loop on the next prompt iteration.
                clear_matching_pending_auth_resume(&mut state, &call);
                append_capability_safe_summary_ref(
                    ctx.host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "skipped"),
                )
                .await?;
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                Ok(BatchStep::Continue(Box::new(state)))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                // Clear any pending auth resume so a stale record does not persist
                // into the Final checkpoint for an aborted capability.
                clear_matching_pending_auth_resume(&mut state, &call);
                append_capability_safe_summary_ref(
                    ctx.host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "aborted"),
                )
                .await?;
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                Ok(BatchStep::Exit(failed_exit(
                    ctx.host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )?))
            }
        }
    }
}

#[async_trait]
impl ExecutorStage<AwaitDependentRunGateInput> for AwaitDependentRunGateStage {
    type Output = BatchStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: AwaitDependentRunGateInput,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let call = input.call;
        let gate_ref = input.gate_ref;
        let summary = crate::strategies::GateSummary {
            kind: GateKind::AwaitDependentRun,
            gate_ref: gate_ref.clone(),
        };
        match ctx.planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { gate } => {
                state.gate_state = gate;
                state.last_gate = Some(gate_ref.clone());
                append_capability_result_ref(ctx.host, &call, &input.resolved_result).await?;
                push_completed_result(&mut state, &call.capability_id, input.resolved_result);
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::GateBlocked {
                            iteration: state.iteration,
                            gate_kind: loop_gate_kind(GateKind::AwaitDependentRun),
                        },
                    )
                    .await;
                let checked = CheckpointStage
                    .write_before_block(ctx, state, &gate_ref)
                    .await?;
                Ok(BatchStep::Exit(LoopExit::Blocked(LoopBlocked {
                    kind: blocked_kind(GateKind::AwaitDependentRun),
                    gate_ref,
                    credential_requirements: Vec::new(),
                    checkpoint_id: checked.checkpoint_id,
                    state_ref: checked.state_ref,
                    exit_id: exit_id(ctx.host, "blocked")?,
                })))
            }
            GateOutcome::SkipAndContinue { gate } => {
                state.gate_state = gate;
                append_capability_result_ref(ctx.host, &call, &input.resolved_result).await?;
                push_completed_result(&mut state, &call.capability_id, input.resolved_result);
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                Ok(BatchStep::Continue(Box::new(state)))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                append_capability_safe_summary_ref(
                    ctx.host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(GateKind::AwaitDependentRun, "aborted"),
                )
                .await?;
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                Ok(BatchStep::Exit(failed_exit(
                    ctx.host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )?))
            }
        }
    }
}
