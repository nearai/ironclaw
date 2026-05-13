//! Capability dispatch: batching, per-call outcome routing, retry on
//! transient/permanent failures, and surface-filter enforcement.

use std::collections::HashSet;

use ironclaw_turns::{
    LoopFailureKind,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostErrorKind, BatchExecutionPolicy,
        CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityConcurrency,
        CapabilityFailure, CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        VisibleCapabilitySurface,
    },
};

use crate::{
    planner::AgentLoopPlanner,
    state::{CapabilityCallSignature, CheckpointKind, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityCallSummary, CapabilityErrorClass, CapabilityErrorSummary,
        CapabilityFilter, ConcurrencyHint, GateKind, RecoveryOutcome,
    },
};

use super::{
    AgentLoopExecutorError, CancelledKind, CanonicalAgentLoopExecutor, FailureKind, HostStage,
    LoopExit, canonical::Step, drain::process_ref_to_gate_ref, lifecycle::failure_kind_to_exit,
    util::MAX_RETRIES_PER_CALL,
};

impl CanonicalAgentLoopExecutor {
    pub(super) async fn handle_capability_calls(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        surface: &VisibleCapabilitySurface,
        calls: Vec<CapabilityCallCandidate>,
    ) -> Result<Step, AgentLoopExecutorError> {
        let mut state = self
            .checkpoint(host, state, CheckpointKind::BeforeSideEffect)
            .await?;
        let summaries = capability_summaries(surface, &calls);
        let policy = planner.batch().policy(&state, &summaries);
        state.control_state.last_batch_total = summaries.len() as u32;
        state.control_state.terminate_hints_in_last_batch = 0;

        // Enforce executor-side filter (master spec §6): the narrowed
        // surface is built locally, but `VisibleCapabilityRequest` doesn't
        // accept a filter — so the model could cite the unfiltered host
        // surface_version to invoke a hidden capability. The check must
        // preserve the planner's original call order:
        //
        //   - Compute an `(allowed, hidden)` mask in original sequence.
        //   - If ALL calls are allowed, batch-invoke as before (preserves
        //     parallelism for the common case).
        //   - If ANY call is hidden, fall back to ordered per-call
        //     execution: invoke allowed calls one at a time via the
        //     single-call host API, and synthesize a `Denied` outcome at
        //     the hidden call's position. A hidden call that routes
        //     through recovery to `Abort` short-circuits before any
        //     subsequent allowed call's side effect runs.
        let allowed_ids: HashSet<_> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let any_hidden = calls
            .iter()
            .any(|call| !allowed_ids.contains(&call.capability_id));

        if !any_hidden {
            // Fast path: all calls allowed → single batch invocation.
            let host_invocations: Vec<CapabilityInvocation> = calls
                .iter()
                .cloned()
                .map(capability_invocation_from_candidate)
                .collect();
            // Suspension outcomes are dynamic host state
            // (ApprovalRequired/AuthRequired/ResourceBlocked/
            // SpawnedProcess), not something a SafeForParallel descriptor
            // can predict. Always ask the host to stop before executing a
            // later invocation once any call suspends.
            let stop_on_first_suspension = true;
            let batch = host
                .invoke_capability_batch(CapabilityBatchInvocation {
                    invocations: host_invocations,
                    stop_on_first_suspension,
                    policy: batch_policy_to_host(policy),
                })
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Capability,
                })?;
            return self
                .consume_batch_outcomes(planner, host, state, calls, batch, policy)
                .await;
        }

        // Mixed path: process per-call in original order. Hidden calls
        // become synthetic `Denied`; allowed calls invoke single-call.
        let mut seen_signatures = HashSet::new();
        for call in calls.into_iter() {
            let signature = signature_for_call(&call);
            if seen_signatures.insert(signature.clone()) {
                state.recent_call_signatures.push(signature);
            }
            let outcome = if allowed_ids.contains(&call.capability_id) {
                host.invoke_capability(capability_invocation_from_candidate(call.clone()))
                    .await
                    .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                        stage: HostStage::Capability,
                    })?
            } else {
                CapabilityOutcome::Denied(ironclaw_turns::run_profile::CapabilityDenied {
                    reason_kind:
                        ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "capability hidden by executor filter".to_string(),
                })
            };
            match self
                .handle_capability_outcome(planner, host, state, call, outcome)
                .await?
            {
                Step::Continue(next) => state = next,
                Step::Exit(exit_state, exit) => return Ok(Step::Exit(exit_state, exit)),
            }
        }

        Ok(Step::Continue(state))
    }

    /// Consume the outcomes from a full-batch invocation. A `Sequential`
    /// batch may return a short outcome vec when the host stops at the
    /// first suspension; in that case the last outcome must be a suspension
    /// kind and only the executed prefix is processed. A `Parallel` batch
    /// keeps the strict 1:1 count contract.
    pub(super) async fn consume_batch_outcomes(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        calls: Vec<CapabilityCallCandidate>,
        batch: ironclaw_turns::run_profile::CapabilityBatchOutcome,
        _policy: BatchPolicy,
    ) -> Result<Step, AgentLoopExecutorError> {
        let outcomes_len = batch.outcomes.len();
        let calls_len = calls.len();
        if outcomes_len > calls_len {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count exceeded host invocations",
            });
        }
        if outcomes_len < calls_len {
            // Short prefix only valid when the host reports it stopped on
            // suspension, and the tail must be a suspension (per
            // `CapabilityOutcome::is_suspension`). The executor requests
            // stop-on-suspension for every batch, including `Parallel`
            // batches, because dynamic auth/approval/resource/process
            // states can arise even for SafeForParallel descriptors.
            if !batch.stopped_on_suspension {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "capability batch returned a short outcome prefix without stopping on suspension",
                });
            }
            let Some(last) = batch.outcomes.last() else {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "capability batch returned no outcomes after stopping on suspension",
                });
            };
            if !last.is_suspension() {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "capability batch truncated without a suspension tail",
                });
            }
        }
        let mut seen_signatures = HashSet::new();
        let mut outcomes_iter = batch.outcomes.into_iter();
        let mut calls_iter = calls.into_iter();
        while let (Some(outcome), Some(call)) = (outcomes_iter.next(), calls_iter.next()) {
            let signature = signature_for_call(&call);
            if seen_signatures.insert(signature.clone()) {
                state.recent_call_signatures.push(signature);
            }
            match self
                .handle_capability_outcome(planner, host, state, call, outcome)
                .await?
            {
                Step::Continue(next) => state = next,
                Step::Exit(exit_state, exit) => return Ok(Step::Exit(exit_state, exit)),
            }
        }
        Ok(Step::Continue(state))
    }

    pub(super) async fn handle_capability_outcome(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        outcome: CapabilityOutcome,
    ) -> Result<Step, AgentLoopExecutorError> {
        match outcome {
            CapabilityOutcome::Completed(result) => {
                push_completed_result(&mut state, result);
                Ok(Step::Continue(state))
            }
            CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                self.handle_gate(planner, host, state, GateKind::Approval, gate_ref)
                    .await
            }
            CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                self.handle_gate(planner, host, state, GateKind::Auth, gate_ref)
                    .await
            }
            CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
                self.handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                    .await
            }
            CapabilityOutcome::SpawnedProcess(handle) => {
                let gate_ref = process_ref_to_gate_ref(&handle)?;
                self.handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                    .await
            }
            CapabilityOutcome::Denied(denied) => {
                let summary = CapabilityErrorSummary {
                    class: CapabilityErrorClass::PolicyDenied,
                    safe_summary: denied.safe_summary,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
            CapabilityOutcome::Failed(failure) => {
                let summary = capability_failure_summary(failure);
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
        }
    }

    pub(super) async fn handle_capability_error(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        summary: CapabilityErrorSummary,
    ) -> Result<Step, AgentLoopExecutorError> {
        state
            .recent_failure_kinds
            .push(LoopFailureKind::CapabilityProtocolError);

        // Inner retry loop: a still-transient failure on retry must consult
        // recovery again so the per-class budget is consumed (master spec
        // §10). The strategy's own `attempts` counter eventually trips
        // `Abort`; `MAX_RETRIES_PER_CALL` is defense-in-depth against a
        // custom strategy that never gives up.
        let mut current_summary = summary;
        for _ in 0..MAX_RETRIES_PER_CALL {
            let recovery = planner
                .recovery()
                .on_capability_error(&state, &current_summary)
                .await;
            match recovery {
                RecoveryOutcome::Retry { recovery, alter } => {
                    // A `Denied` outcome must NEVER be replayed through the
                    // host. `Denied` is either an executor-side synthetic
                    // denial (the capability was filtered out — replaying
                    // would let the model bypass the filter) or a host-side
                    // policy denial (already authoritative). Treat the
                    // recovery `Retry` as `SkipResult`: consume the budget
                    // bump but do not invoke the host.
                    if matches!(current_summary.class, CapabilityErrorClass::PolicyDenied) {
                        state.recovery_state = recovery;
                        return Ok(Step::Continue(state));
                    }
                    state.recovery_state = recovery;
                    if matches!(
                        alter,
                        Some(crate::strategies::RetryAlteration::AdvanceFallback)
                    ) {
                        return Ok(Step::Exit(
                            state,
                            LoopExit::Failed {
                                kind: FailureKind::Other(LoopFailureKind::DriverBug),
                            },
                        ));
                    }
                    // Honor `Backoff` delay before retry.
                    if let Some(crate::strategies::RetryAlteration::Backoff { delay }) = alter {
                        tokio::time::sleep(delay).await;
                    }
                    let retry_outcome = match host
                        .invoke_capability(capability_invocation_from_candidate(call.clone()))
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(error) if matches!(error.kind, AgentLoopHostErrorKind::Cancelled) => {
                            // Capability-port cancellation surfaces as
                            // `LoopExit::Cancelled`.
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
                            let exit = LoopExit::Cancelled(CancelledKind {
                                interrupted_message_refs: checked.assistant_refs.clone(),
                            });
                            return Ok(Step::Exit(checked, exit));
                        }
                        Err(_) => {
                            return Err(AgentLoopExecutorError::HostUnavailable {
                                stage: HostStage::Capability,
                            });
                        }
                    };
                    match retry_outcome {
                        CapabilityOutcome::Completed(result) => {
                            push_completed_result(&mut state, result);
                            return Ok(Step::Continue(state));
                        }
                        CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                            return self
                                .handle_gate(planner, host, state, GateKind::Approval, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                            return self
                                .handle_gate(planner, host, state, GateKind::Auth, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
                            return self
                                .handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::SpawnedProcess(handle) => {
                            let gate_ref = process_ref_to_gate_ref(&handle)?;
                            return self
                                .handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::Denied(denied) => {
                            // Re-route through recovery as PolicyDenied — the
                            // outer match treats Denied as a non-recoverable
                            // failure for THIS call but lets recovery decide
                            // skip vs abort.
                            current_summary = CapabilityErrorSummary {
                                class: CapabilityErrorClass::PolicyDenied,
                                safe_summary: denied.safe_summary,
                                diagnostic_ref: None,
                            };
                            continue;
                        }
                        CapabilityOutcome::Failed(failure) => {
                            // Same call, still transient (or permanent) —
                            // ask recovery again. Do NOT re-push to
                            // `recent_failure_kinds`: master spec §10 says
                            // failure kind is recorded once per call, not
                            // per retry.
                            current_summary = capability_failure_summary(failure);
                            continue;
                        }
                    }
                }
                RecoveryOutcome::SkipResult { recovery } => {
                    state.recovery_state = recovery;
                    return Ok(Step::Continue(state));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    return Ok(Step::Exit(
                        state,
                        LoopExit::Failed {
                            kind: failure_kind_to_exit(failure_kind),
                        },
                    ));
                }
            }
        }

        // Defense-in-depth: a custom strategy returned `Retry` more than
        // `MAX_RETRIES_PER_CALL` times. Treat as a driver bug.
        Ok(Step::Exit(
            state,
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::DriverBug),
            },
        ))
    }
}

/// Project the model's chosen capability batch into the shape
/// `BatchPolicyStrategy` consumes.
///
/// Each call resolves against the visible-surface descriptor it claims
/// to use, mapping `CapabilityConcurrency::Exclusive` -> `Exclusive` and
/// `CapabilityConcurrency::SafeForParallel` -> `SafeForParallel`. When a
/// descriptor is missing from the visible surface (defensive — the
/// capability-filter strategy should have rejected the call upstream) we
/// fall back to `Exclusive`, which makes the batch run sequentially with
/// `stop_on_first_suspension = true`. Conservative-by-default is the right
/// choice when in doubt.
pub(super) fn capability_summaries(
    surface: &VisibleCapabilitySurface,
    calls: &[CapabilityCallCandidate],
) -> Vec<CapabilityCallSummary> {
    calls
        .iter()
        .map(|call| {
            let concurrency_hint = surface
                .descriptors
                .iter()
                .find(|descriptor| descriptor.capability_id == call.capability_id)
                .map(|descriptor| match descriptor.concurrency {
                    CapabilityConcurrency::SafeForParallel => ConcurrencyHint::SafeForParallel,
                    CapabilityConcurrency::Exclusive => ConcurrencyHint::Exclusive,
                })
                .unwrap_or(ConcurrencyHint::Exclusive);
            CapabilityCallSummary {
                name: call.capability_id.clone(),
                concurrency_hint,
            }
        })
        .collect()
}

/// Map the loop-side `BatchPolicy` (a strategy decision local to the
/// framework) to the host-facing `BatchExecutionPolicy` (carried on the
/// wire as part of `CapabilityBatchInvocation`). Two distinct enums
/// exist so the dependency arrow stays `ironclaw_agent_loop` ->
/// `ironclaw_turns`; this function is the single boundary mapper.
pub(super) fn batch_policy_to_host(policy: BatchPolicy) -> BatchExecutionPolicy {
    match policy {
        BatchPolicy::Sequential => BatchExecutionPolicy::Sequential,
        BatchPolicy::Parallel => BatchExecutionPolicy::Parallel,
    }
}

pub(super) fn capability_invocation_from_candidate(
    call: CapabilityCallCandidate,
) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
    }
}

pub(super) fn signature_for_call(call: &CapabilityCallCandidate) -> CapabilityCallSignature {
    CapabilityCallSignature::from_call(
        call.capability_id.clone(),
        &serde_json::Value::String(call.input_ref.as_str().to_string()),
    )
}

pub(super) fn capability_failure_summary(failure: CapabilityFailure) -> CapabilityErrorSummary {
    CapabilityErrorSummary {
        class: match failure.error_kind.as_str() {
            "transient" => CapabilityErrorClass::Transient,
            "permanent" => CapabilityErrorClass::Permanent,
            "input_invalid" => CapabilityErrorClass::InputInvalid,
            "policy_denied" => CapabilityErrorClass::PolicyDenied,
            "unavailable" => CapabilityErrorClass::Unavailable,
            _ => CapabilityErrorClass::Internal,
        },
        safe_summary: failure.safe_summary,
        diagnostic_ref: None,
    }
}

pub(super) fn push_completed_result(
    state: &mut LoopExecutionState,
    result: CapabilityResultMessage,
) {
    if is_terminate_hint(&result) {
        state.control_state.terminate_hints_in_last_batch = state
            .control_state
            .terminate_hints_in_last_batch
            .saturating_add(1);
    }
    state.result_refs.push(result.result_ref);
}

pub(super) fn is_terminate_hint(result: &CapabilityResultMessage) -> bool {
    matches!(
        result.safe_summary.as_str(),
        "terminate_hint:true" | "terminate:true" | "terminate"
    )
}

/// Narrow the host's visible-capability surface using the planner's filter.
///
/// The host applies its own scope/grant/auth filters first; this strategy
/// filter can only further narrow the result. `CapabilityFilter::All`
/// is a no-op.
pub(super) fn apply_capability_filter(
    surface: VisibleCapabilitySurface,
    filter: &CapabilityFilter,
) -> VisibleCapabilitySurface {
    match filter {
        CapabilityFilter::All => surface,
        CapabilityFilter::AllowOnly(allowed) => {
            let descriptors = surface
                .descriptors
                .into_iter()
                .filter(|descriptor| allowed.contains(&descriptor.capability_id))
                .collect();
            VisibleCapabilitySurface {
                version: surface.version,
                descriptors,
            }
        }
        CapabilityFilter::Deny(denied) => {
            let descriptors = surface
                .descriptors
                .into_iter()
                .filter(|descriptor| !denied.contains(&descriptor.capability_id))
                .collect();
            VisibleCapabilitySurface {
                version: surface.version,
                descriptors,
            }
        }
    }
}
