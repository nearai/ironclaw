//! Tier 0 executor: structured tool calls.
//!
//! Executes action calls by delegating to the `EffectExecutor` trait,
//! checking leases and policies for each call.
//!
//! Uses a two-phase approach: sequential preflight (lease/policy checks)
//! followed by parallel execution of all approved actions via `JoinSet`.

use std::sync::Arc;
use std::time::Instant;

use crate::capability::lease::LeaseManager;
use crate::capability::policy::{PolicyDecision, PolicyEngine};
use crate::runtime::messaging::ThreadOutcome;
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::types::capability::CapabilityLease;
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::step::{ActionCall, ActionResult};
use crate::types::thread::Thread;

/// Result of executing a batch of action calls.
pub struct ActionBatchResult {
    /// Results for each action call (in order).
    pub results: Vec<ActionResult>,
    /// Events generated during execution.
    pub events: Vec<EventKind>,
    /// If set, execution was interrupted and the thread needs approval.
    pub need_approval: Option<ThreadOutcome>,
}

/// Outcome of preflight checking a single action call.
enum PreflightOutcome {
    /// Action passed preflight — ready for parallel execution.
    Runnable {
        index: usize,
        lease: CapabilityLease,
    },
    /// Action was denied or had no lease — error result already produced.
    Error {
        index: usize,
        result: ActionResult,
        event: EventKind,
    },
}

/// Execute a batch of action calls using the Tier 0 (structured) approach.
///
/// Two-phase execution:
/// 1. **Preflight** (sequential): For each call, find lease and check policy.
///    Denied calls produce error results immediately. RequireApproval interrupts
///    the entire batch.
/// 2. **Execute** (parallel): All approved calls run concurrently via `JoinSet`.
///    Results are collected and merged in original call order.
pub async fn execute_action_calls(
    calls: &[ActionCall],
    thread: &Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
) -> Result<ActionBatchResult, EngineError> {
    let mut preflight_results: Vec<PreflightOutcome> = Vec::with_capacity(calls.len());
    let mut early_events = Vec::new();
    let active_leases = leases.active_for_thread(thread.id).await;
    let available_inventory = Arc::new(
        effects
            .available_action_inventory(&active_leases, context)
            .await?,
    );
    let available_actions: Arc<[crate::types::capability::ActionDef]> =
        available_inventory.inline.clone().into();

    // ── Phase 1: Preflight (sequential) ─────────────────────────
    // Check leases and policies for every call. RequireApproval interrupts
    // the entire batch immediately. Denied/no-lease calls become error results.

    for (idx, call) in calls.iter().enumerate() {
        // 1. Find the action definition from the callable inventory.
        let action_def = available_actions
            .iter()
            .find(|action| action.matches_name(&call.action_name));

        let Some(action_def) = action_def else {
            let error = format!(
                "action '{}' is not callable in this execution context",
                call.action_name
            );
            let error_result = ActionResult {
                call_id: call.id.clone(),
                action_name: call.action_name.clone(),
                output: serde_json::json!({"error": error}),
                is_error: true,
                duration: std::time::Duration::ZERO,
            };
            let event = EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: call.action_name.clone(),
                call_id: call.id.clone(),
                error: error_result.output["error"]
                    .as_str()
                    .unwrap_or("action is not callable in this execution context")
                    .to_string(),
                duration_ms: 0,
                params_summary: crate::types::event::summarize_params(
                    &call.action_name,
                    &call.parameters,
                ),
            };
            preflight_results.push(PreflightOutcome::Error {
                index: idx,
                result: error_result,
                event,
            });
            continue;
        };

        // 2. Find the lease for this action (read-only lookup for policy check)
        let lease = match leases
            .find_lease_for_action(thread.id, &call.action_name)
            .await
        {
            Some(l) => l,
            None => {
                let error_result = ActionResult {
                    call_id: call.id.clone(),
                    action_name: call.action_name.clone(),
                    output: serde_json::json!({"error": format!(
                        "no active lease covers action '{}'", call.action_name
                    )}),
                    is_error: true,
                    duration: std::time::Duration::ZERO,
                };
                let event = EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: format!("no lease for action '{}'", call.action_name),
                    duration_ms: 0,
                    params_summary: crate::types::event::summarize_params(
                        &call.action_name,
                        &call.parameters,
                    ),
                };
                preflight_results.push(PreflightOutcome::Error {
                    index: idx,
                    result: error_result,
                    event,
                });
                continue;
            }
        };

        // 3. Check policy for the callable action.
        let decision = policy.evaluate(action_def, &lease, capability_policies);
        match decision {
            PolicyDecision::Deny { reason } => {
                let error_result = ActionResult {
                    call_id: call.id.clone(),
                    action_name: call.action_name.clone(),
                    output: serde_json::json!({"error": format!("denied: {reason}")}),
                    is_error: true,
                    duration: std::time::Duration::ZERO,
                };
                let event = EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: reason,
                    duration_ms: 0,
                    params_summary: crate::types::event::summarize_params(
                        &call.action_name,
                        &call.parameters,
                    ),
                };
                preflight_results.push(PreflightOutcome::Error {
                    index: idx,
                    result: error_result,
                    event,
                });
                continue;
            }
            PolicyDecision::RequireApproval { .. } => {
                // Inline gate-await: pause this preflight loop in place
                // until the user resolves the gate. On approval, fall
                // through to lease consumption and queue the call for
                // execution. On denial, mark the call failed and continue
                // preflight for the rest of the batch — same blast radius
                // as a policy-Deny.
                //
                // The controller is required on the context. Code paths
                // that don't pause supply `CancellingGateController`,
                // which surfaces as a typed denial here.
                //
                // Policy doesn't carry the `allow_always` axis; default
                // to the historical value (`true`) so the UI offers it.
                let resume_kind = crate::gate::ResumeKind::Approval { allow_always: true };
                early_events.push(EventKind::ApprovalRequested {
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    parameters: Some(call.parameters.clone()),
                    description: None,
                    allow_always: Some(true),
                    gate_name: Some("approval".into()),
                    params_summary: crate::types::event::summarize_params(
                        &call.action_name,
                        &call.parameters,
                    ),
                });
                let resolution = context
                    .gate_controller
                    .pause(crate::gate::GatePauseRequest {
                        thread_id: thread.id,
                        user_id: thread.user_id.clone(),
                        gate_name: "approval".into(),
                        action_name: call.action_name.clone(),
                        call_id: call.id.clone(),
                        parameters: call.parameters.clone(),
                        resume_kind,
                        conversation_id: context.conversation_id,
                    })
                    .await;

                let denial = crate::executor::scripting::denial_outcome_for_resolution(&resolution);
                if let Some(outcome) = denial {
                    let error_msg = outcome.event_error();
                    let error_result = ActionResult {
                        call_id: call.id.clone(),
                        action_name: call.action_name.clone(),
                        output: serde_json::json!({"error": &error_msg}),
                        is_error: true,
                        duration: std::time::Duration::ZERO,
                    };
                    let event = EventKind::ActionFailed {
                        step_id: context.step_id,
                        action_name: call.action_name.clone(),
                        call_id: call.id.clone(),
                        error: error_msg,
                        duration_ms: 0,
                        params_summary: crate::types::event::summarize_params(
                            &call.action_name,
                            &call.parameters,
                        ),
                    };
                    preflight_results.push(PreflightOutcome::Error {
                        index: idx,
                        result: error_result,
                        event,
                    });
                    continue;
                }
                // Approved: fall through to lease-consume + runnable-queue.
            }
            PolicyDecision::Allow => {}
        }

        // 4. Atomically find + consume a lease use under a single write lock.
        // This avoids the TOCTOU race where a concurrent call could exhaust
        // the lease between our read-only find (step 1) and this consume.
        let lease = leases
            .find_and_consume(thread.id, &call.action_name)
            .await?;

        preflight_results.push(PreflightOutcome::Runnable { index: idx, lease });
    }

    // ── Phase 2: Execute (parallel) ─────────────────────────────
    // All approved calls run concurrently. Results are collected in a
    // HashMap keyed by original index, then merged in order.

    // Separate runnable from preflight errors. Each slot carries the
    // call's terminal `(ActionResult, EventKind)` plus any
    // pre-terminal `ApprovalRequested` events emitted by the inline
    // retry helper. Pre-terminal events are flushed before the
    // terminal event in the merge phase so audit observers see
    // "approval asked → action executed/failed" in order.
    let mut slot_results: Vec<Option<(ActionResult, EventKind, Vec<EventKind>)>> =
        vec![None; calls.len()];
    let mut runnable_indices = Vec::new();

    for pf in preflight_results {
        match pf {
            PreflightOutcome::Error {
                index,
                result,
                event,
                ..
            } => {
                slot_results[index] = Some((result, event, Vec::new()));
            }
            PreflightOutcome::Runnable { index, lease } => {
                runnable_indices.push((index, lease));
            }
        }
    }

    // Short-circuit: single runnable call — execute directly without JoinSet overhead
    if runnable_indices.len() == 1 {
        let (idx, lease) = runnable_indices.into_iter().next().unwrap(); // safety: len()==1 checked above
        let call = &calls[idx];
        let exec_ctx =
            stamp_execution_context(context, &call.id, &available_actions, &available_inventory);
        let execution_start = Instant::now();
        let (exec_result, pre_events) = execute_with_inline_gate_retry(
            effects,
            leases,
            &lease,
            call,
            &exec_ctx,
            thread.id,
            &thread.user_id,
        )
        .await;
        if interrupted_call_needs_refund(&exec_result) {
            let _ = leases.refund_use(lease.id).await;
        }
        let (result, event) = classify_exec_result(
            exec_result,
            call,
            &exec_ctx,
            execution_start.elapsed().as_millis() as u64,
        );
        slot_results[idx] = Some((result, event, pre_events));
    } else if runnable_indices.len() > 1 {
        // Multiple calls: execute in parallel via JoinSet. Each task
        // wraps the call in `execute_with_inline_gate_retry` so a tool
        // raising `Approval` mid-execution pauses inline through the
        // shared bridge controller (which serializes concurrent gates
        // per (user, thread)) and either retries on approval or
        // surfaces a typed denial — same contract as the single-call
        // fast path. Without this wrapper, parallel batches reverted
        // to the legacy `gate_paused` sentinel + thread re-entry path,
        // re-introducing the double-execution bug for any
        // already-completed sibling calls in the same batch.
        let mut join_set = tokio::task::JoinSet::new();

        // Capture thread metadata once outside the spawn loop. Avoids
        // cloning the full `Thread` (with message/event transcripts)
        // per task — the helper only needs the id + user_id.
        let thread_id = thread.id;
        let user_id = thread.user_id.clone();
        for (idx, lease) in runnable_indices {
            let call = calls[idx].clone();
            let ctx = stamp_execution_context(
                context,
                &call.id,
                &available_actions,
                &available_inventory,
            );
            let effects = Arc::clone(effects);
            let leases = Arc::clone(leases);
            let lease = lease.clone();
            let user_id = user_id.clone();

            join_set.spawn(async move {
                let execution_start = Instant::now();
                let (result, pre_events) = execute_with_inline_gate_retry(
                    &effects, &leases, &lease, &call, &ctx, thread_id, &user_id,
                )
                .await;
                (
                    idx,
                    lease.id,
                    result,
                    pre_events,
                    call,
                    ctx,
                    execution_start.elapsed().as_millis() as u64,
                )
            });
        }

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, lease_id, result, pre_events, call, ctx, execution_duration_ms)) => {
                    if interrupted_call_needs_refund(&result) {
                        let _ = leases.refund_use(lease_id).await;
                    }
                    let (action_result, event) =
                        classify_exec_result(result, &call, &ctx, execution_duration_ms);
                    slot_results[idx] = Some((action_result, event, pre_events));
                }
                Err(e) => {
                    // Task panicked — should not happen, but handle gracefully
                    tracing::debug!("parallel tool execution task panicked: {e}");
                }
            }
        }
    }

    // ── Phase 3: Merge results in original call order ───────────

    let mut results = Vec::with_capacity(calls.len());
    // `early_events` carries ApprovalRequested events emitted during
    // preflight; they're emitted *before* any per-call result event so
    // observers see "approval asked → action failed" in the right
    // order.
    let mut events = std::mem::take(&mut early_events);
    let mut first_interrupt: Option<ThreadOutcome> = None;

    for (idx, slot) in slot_results.into_iter().enumerate() {
        if let Some((result, event, pre_events)) = slot {
            // Record the first gate pause as the batch interrupt but still
            // collect all other results.
            if first_interrupt.is_none()
                && let EventKind::ApprovalRequested {
                    ref action_name,
                    ref call_id,
                    ..
                } = event
                && result.output.get("status").and_then(|v| v.as_str()) == Some("gate_paused")
            {
                let gate_name = result
                    .output
                    .get("gate")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let call = &calls[idx];
                first_interrupt = Some(ThreadOutcome::GatePaused {
                    gate_name,
                    action_name: action_name.clone(),
                    call_id: call_id.clone(),
                    parameters: call.parameters.clone(),
                    resume_kind: serde_json::from_value(
                        result.output.get("resume_kind").cloned().unwrap_or_else(
                            || serde_json::json!({"Approval":{"allow_always":false}}),
                        ),
                    )
                    .unwrap_or(crate::gate::ResumeKind::Approval {
                        allow_always: false,
                    }),
                    resume_output: result.output.get("resume_output").cloned(),
                    paused_lease: result
                        .output
                        .get("paused_lease")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok()),
                });
            }
            // Pre-terminal `ApprovalRequested` events from the inline
            // retry helper come first so the audit log reads as
            // "approval asked → action <outcome>" in order.
            for pe in pre_events {
                events.push(pe);
            }
            results.push(result);
            events.push(event);
        }
    }

    Ok(ActionBatchResult {
        results,
        events,
        need_approval: first_interrupt,
    })
}

fn stamp_execution_context(
    context: &ThreadExecutionContext,
    call_id: &str,
    available_actions: &Arc<[crate::types::capability::ActionDef]>,
    available_inventory: &Arc<crate::types::capability::ActionInventory>,
) -> ThreadExecutionContext {
    let mut exec_ctx = context.clone();
    exec_ctx.current_call_id = Some(call_id.to_string());
    exec_ctx.available_actions_snapshot = Some(Arc::clone(available_actions));
    exec_ctx.available_action_inventory_snapshot = Some(Arc::clone(available_inventory));
    exec_ctx
}

/// Classify an execution result into an `(ActionResult, EventKind)` pair.
///
/// Used by both the single-call fast path and the parallel JoinSet path
/// to produce uniform output.
fn classify_exec_result(
    result: Result<ActionResult, EngineError>,
    call: &ActionCall,
    context: &ThreadExecutionContext,
    execution_duration_ms: u64,
) -> (ActionResult, EventKind) {
    match result {
        Ok(mut action_result) => {
            action_result.call_id = call.id.clone();
            // Effect adapters wrap tool errors as `Ok(ActionResult { is_error: true })`
            // — emit ActionFailed in that case so traces and downstream
            // observers see the failure rather than treating it as success.
            let event = if action_result.is_error {
                let error_msg = action_result
                    .output
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| action_result.output.to_string());
                let duration_ms = action_result.duration.as_millis() as u64;
                EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: error_msg,
                    duration_ms: if duration_ms > 0 {
                        duration_ms
                    } else {
                        execution_duration_ms
                    },
                    params_summary: crate::types::event::summarize_params(
                        &call.action_name,
                        &call.parameters,
                    ),
                }
            } else {
                EventKind::ActionExecuted {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    duration_ms: action_result.duration.as_millis() as u64,
                    params_summary: crate::types::event::summarize_params(
                        &call.action_name,
                        &call.parameters,
                    ),
                }
            };
            (action_result, event)
        }
        Err(EngineError::GatePaused {
            gate_name,
            action_name,
            call_id,
            parameters,
            resume_kind,
            resume_output,
            paused_lease,
        }) => {
            let _error_msg = format!("gate paused: {gate_name}");
            let error_result = ActionResult {
                call_id: call.id.clone(),
                action_name: call.action_name.clone(),
                output: serde_json::json!({
                    "status": "gate_paused",
                    "gate": gate_name,
                    "resume_kind": serde_json::to_value(&*resume_kind).unwrap_or_default(),
                    "resume_output": resume_output.as_deref().cloned(),
                    "paused_lease": paused_lease.as_deref().cloned(),
                }),
                is_error: true,
                duration: std::time::Duration::ZERO,
            };
            let event = EventKind::ApprovalRequested {
                action_name,
                call_id,
                parameters: Some((*parameters).clone()),
                description: None,
                allow_always: match *resume_kind {
                    crate::gate::ResumeKind::Approval { allow_always } => Some(allow_always),
                    _ => None,
                },
                gate_name: Some(gate_name.clone()),
                params_summary: crate::types::event::summarize_params(
                    &call.action_name,
                    &parameters,
                ),
            };
            (error_result, event)
        }
        Err(e) => {
            let error_result = ActionResult {
                call_id: call.id.clone(),
                action_name: call.action_name.clone(),
                output: serde_json::json!({"error": e.to_string()}),
                is_error: true,
                duration: std::time::Duration::ZERO,
            };
            let event = EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: call.action_name.clone(),
                call_id: call.id.clone(),
                error: e.to_string(),
                duration_ms: execution_duration_ms,
                params_summary: crate::types::event::summarize_params(
                    &call.action_name,
                    &call.parameters,
                ),
            };
            (error_result, event)
        }
    }
}

fn interrupted_call_needs_refund(result: &Result<ActionResult, EngineError>) -> bool {
    matches!(result, Err(EngineError::GatePaused { .. }))
}

/// Run a single tool action with inline gate-await retry.
///
/// If the executor returns `Err(EngineError::GatePaused { resume_kind: Approval, .. })`,
/// refund the lease, pause for the user via the context's controller,
/// and retry on approval. On denial / cancellation, surface as a
/// deny-style `EngineError::Effect` so the caller produces an
/// `ActionFailed` event rather than a "gate_paused" sentinel.
///
/// Bounded by [`MAX_INLINE_GATE_RETRIES`]: a misbehaving tool that
/// keeps gating after each approval surfaces a clean error rather than
/// pinning a CPU. The bridge installs auto-approve before delivering
/// the resolution, so well-behaved chains converge in 1–2 iterations
/// and the cap is only ever hit on bugs.
///
/// Authentication resume kinds also flow through this loop now —
/// `bridge::resolve_inline_gates_for_credential` (the OAuth-callback
/// hook from #3133 half-2) delivers `GateResolution::Approved` to the
/// parked controller as soon as the credential lands in the secrets
/// store, so the retry sees the credential and the action succeeds.
/// (`bridge::resume_paused_missions_for_credential` is the parallel
/// path for missions whose child threads were paused — separate from
/// the inline-await waiters this loop drives.)
/// External resume kinds still keep the legacy re-entry path: their
/// resolution installs callback-payload state that the suspended call
/// can't see without unwinding.
///
/// Returns `(final_result, events)` where `events` carries the
/// `ApprovalRequested` audit events emitted across retry iterations
/// — one per gate-pause cycle, in the order they fired. Callers
/// MUST emit these events before the per-call outcome event so
/// replay/audit observers see "approval asked → action <outcome>"
/// instead of just the final outcome.
async fn execute_with_inline_gate_retry(
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    lease: &CapabilityLease,
    call: &ActionCall,
    exec_ctx: &ThreadExecutionContext,
    thread_id: crate::types::thread::ThreadId,
    user_id: &str,
) -> (Result<ActionResult, EngineError>, Vec<EventKind>) {
    let mut current_lease = lease.clone();
    // `call_ctx` carries the one-shot approval flag across retries.
    // First iteration: false (the gate hasn't fired yet). After each
    // approval we set true; we reset to false immediately after the
    // call so a re-gating tool doesn't get the flag handed back twice
    // for one approval.
    let mut call_ctx = exec_ctx.clone();
    let mut emitted_events: Vec<EventKind> = Vec::new();
    for _ in 0..crate::executor::scripting::MAX_INLINE_GATE_RETRIES {
        let result = effects
            .execute_action(
                &call.action_name,
                call.parameters.clone(),
                &current_lease,
                &call_ctx,
            )
            .await;
        call_ctx.call_approval_granted = false;

        // Snapshot the original gate (for re-emission on
        // Cancelled+Authentication, see below).
        let original_err = match &result {
            Err(EngineError::GatePaused {
                resume_kind,
                paused_lease,
                resume_output,
                gate_name,
                action_name,
                call_id,
                parameters,
            }) if matches!(
                **resume_kind,
                crate::gate::ResumeKind::Approval { .. }
                    | crate::gate::ResumeKind::Authentication { .. }
            ) =>
            {
                Some(EngineError::GatePaused {
                    gate_name: gate_name.clone(),
                    action_name: action_name.clone(),
                    call_id: call_id.clone(),
                    parameters: parameters.clone(),
                    resume_kind: resume_kind.clone(),
                    paused_lease: paused_lease.clone(),
                    resume_output: resume_output.clone(),
                })
            }
            _ => None,
        };
        let (gate_name, action_name, call_id, parameters, resume_kind) = match result {
            Err(EngineError::GatePaused {
                gate_name,
                action_name,
                call_id,
                parameters,
                resume_kind,
                ..
            }) if matches!(
                *resume_kind,
                crate::gate::ResumeKind::Approval { .. }
                    | crate::gate::ResumeKind::Authentication { .. }
            ) =>
            {
                (gate_name, action_name, call_id, *parameters, *resume_kind)
            }
            other => return (other, emitted_events),
        };

        // Emit the audit event BEFORE awaiting the controller so
        // observers see the request even if the user never resolves.
        // Mirrors the orchestrator (Tier 1) path which records the
        // event before calling `pause()`.
        let allow_always = match resume_kind {
            crate::gate::ResumeKind::Approval { allow_always } => Some(allow_always),
            _ => None,
        };
        emitted_events.push(EventKind::ApprovalRequested {
            action_name: action_name.clone(),
            call_id: call_id.clone(),
            parameters: Some(parameters.clone()),
            description: None,
            allow_always,
            gate_name: Some(gate_name.clone()),
            params_summary: crate::types::event::summarize_params(&call.action_name, &parameters),
        });

        // Refund the lease use this attempt consumed; we'll re-consume
        // on retry if the user approves.
        let _ = leases.refund_use(current_lease.id).await;

        let resolution = exec_ctx
            .gate_controller
            .pause(crate::gate::GatePauseRequest {
                thread_id,
                user_id: user_id.to_string(),
                gate_name: gate_name.clone(),
                action_name: action_name.clone(),
                call_id: call_id.clone(),
                parameters: parameters.clone(),
                resume_kind: resume_kind.clone(),
                conversation_id: exec_ctx.conversation_id,
            })
            .await;

        if let Some(outcome) =
            crate::executor::scripting::denial_outcome_for_resolution(&resolution)
        {
            // Cancelled+Authentication → unwind to legacy
            // `ThreadOutcome::GatePaused` so missions / non-inline-aware
            // controllers can still surface a Paused state. Cancelled
            // here means the controller can't resolve the auth inline
            // (e.g. `CancellingGateController` in tests, or a
            // BridgeGateController without OAuth wiring) — that's
            // semantically "no inline path exists" and the legacy
            // unwind is the right fallback. Denied / explicit
            // Cancelled-by-user remain failures.
            if matches!(resolution, crate::gate::GateResolution::Cancelled)
                && matches!(resume_kind, crate::gate::ResumeKind::Authentication { .. })
                && let Some(err) = original_err
            {
                return (Err(err), emitted_events);
            }
            return (
                Err(EngineError::Effect {
                    reason: outcome.effect_reason(),
                }),
                emitted_events,
            );
        }

        // Approved: re-consume a lease use and mark the next call as
        // pre-approved so the host's `EffectExecutor` skips its
        // approval check.
        match leases.find_and_consume(thread_id, &call.action_name).await {
            Ok(new_lease) => {
                current_lease = new_lease;
                call_ctx.call_approval_granted = true;
                continue;
            }
            Err(e) => {
                return (
                    Err(EngineError::Effect {
                        reason: format!("lease exhausted after approval: {e}"),
                    }),
                    emitted_events,
                );
            }
        }
    }

    // Retry budget exhausted. The last loop iteration ended with a
    // successful `find_and_consume` whose lease was never used —
    // refund it before returning so a misbehaving tool can't slowly
    // drain `max_uses` across approvals. Best-effort; if the lease
    // was already revoked/expired the refund is a no-op.
    let _ = leases.refund_use(current_lease.id).await;
    (
        Err(EngineError::Effect {
            reason: format!(
                "tool '{}' still requires approval after {} retries",
                call.action_name,
                crate::executor::scripting::MAX_INLINE_GATE_RETRIES
            ),
        }),
        emitted_events,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::effect::ThreadExecutionContext;
    use crate::types::capability::{
        ActionDef, CapabilityLease, EffectType, GrantedActions, ModelToolSurface,
    };
    use crate::types::project::ProjectId;
    use crate::types::step::StepId;
    use crate::types::thread::{Thread, ThreadConfig, ThreadType};

    use std::sync::Mutex;
    use std::time::Duration;

    struct MockEffects {
        results: Mutex<Vec<Result<ActionResult, EngineError>>>,
        actions: Vec<ActionDef>,
    }

    impl MockEffects {
        fn new(actions: Vec<ActionDef>, results: Vec<Result<ActionResult, EngineError>>) -> Self {
            Self {
                results: Mutex::new(results),
                actions,
            }
        }
    }

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(
            &self,
            _name: &str,
            _params: serde_json::Value,
            _lease: &CapabilityLease,
            _ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let mut results = self.results.lock().unwrap();
            if results.is_empty() {
                Ok(ActionResult {
                    call_id: String::new(), // EffectExecutor doesn't set call_id
                    action_name: String::new(),
                    output: serde_json::json!({"result": "ok"}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                })
            } else {
                results.remove(0)
            }
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.clone())
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    fn test_action(name: &str) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: "Test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }
    }

    fn make_exec_context(thread: &Thread) -> ThreadExecutionContext {
        ThreadExecutionContext {
            thread_id: thread.id,
            thread_type: thread.thread_type,
            project_id: thread.project_id,
            user_id: "test".into(),
            step_id: StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
            thread_goal: Some(thread.goal.clone()),
            available_actions_snapshot: None,
            available_action_inventory_snapshot: None,
            gate_controller: crate::gate::CancellingGateController::arc(),
            call_approval_granted: false,
            conversation_id: None,
        }
    }

    // ── call_id propagation tests ────────────────────────────

    #[tokio::test]
    async fn call_id_preserved_on_successful_execution() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("web_search")],
            vec![Ok(ActionResult {
                call_id: String::new(), // EffectExecutor returns empty
                action_name: "web_search".into(),
                output: serde_json::json!({"results": []}),
                is_error: false,
                duration: Duration::from_millis(42),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "search", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_r2o5mqBgdNUlH8KzskncUGaX".into(),
            action_name: "web_search".into(),
            parameters: serde_json::json!({"query": "test"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // call_id must be stamped from ActionCall, not the empty EffectExecutor return
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].call_id, "call_r2o5mqBgdNUlH8KzskncUGaX");
        assert_eq!(result.results[0].action_name, "web_search");
        assert!(!result.results[0].is_error);

        // Event should carry the same call_id
        let exec_event = result
            .events
            .iter()
            .find(|e| matches!(e, EventKind::ActionExecuted { .. }));
        assert!(exec_event.is_some());
        if let Some(EventKind::ActionExecuted {
            call_id,
            action_name,
            ..
        }) = exec_event
        {
            assert_eq!(call_id, "call_r2o5mqBgdNUlH8KzskncUGaX");
            assert_eq!(action_name, "web_search");
        }
    }

    #[tokio::test]
    async fn call_id_preserved_on_execution_error() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("shell")],
            vec![Err(EngineError::Effect {
                reason: "permission denied".into(),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "exec", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_abc123def".into(),
            action_name: "shell".into(),
            parameters: serde_json::json!({"cmd": "ls"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].call_id, "call_abc123def");
        assert!(result.results[0].is_error);

        let fail_event = result
            .events
            .iter()
            .find(|e| matches!(e, EventKind::ActionFailed { .. }));
        assert!(fail_event.is_some());
        if let Some(EventKind::ActionFailed { call_id, .. }) = fail_event {
            assert_eq!(call_id, "call_abc123def");
        }
    }

    #[tokio::test]
    async fn call_id_preserved_when_no_lease() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        // Inventory contains `web_search` (so the callable-inventory
        // gate passes), but no lease is granted — the lease lookup is
        // the failure point this test exercises. Without `web_search`
        // in the inventory, preflight short-circuits on
        // "action is not callable in this execution context" before
        // ever reaching the lease check.
        let effects: Arc<dyn EffectExecutor> =
            Arc::new(MockEffects::new(vec![test_action("web_search")], vec![]));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        // No lease granted — action should fail with correct call_id
        let calls = vec![ActionCall {
            id: "call_no_lease_123".into(),
            action_name: "web_search".into(),
            parameters: serde_json::json!({}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].call_id, "call_no_lease_123");
        assert!(result.results[0].is_error);

        if let Some(EventKind::ActionFailed { call_id, error, .. }) = result.events.first() {
            assert_eq!(call_id, "call_no_lease_123");
            assert!(error.contains("no lease"));
        } else {
            panic!("expected ActionFailed event");
        }
    }

    #[tokio::test]
    async fn multiple_calls_each_get_correct_call_id() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("tool_a"), test_action("tool_b")],
            vec![
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "tool_a".into(),
                    output: serde_json::json!("a_result"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "tool_b".into(),
                    output: serde_json::json!("b_result"),
                    is_error: false,
                    duration: Duration::from_millis(2),
                }),
            ],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "cap", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![
            ActionCall {
                id: "id_aaaa".into(),
                action_name: "tool_a".into(),
                parameters: serde_json::json!({}),
            },
            ActionCall {
                id: "id_bbbb".into(),
                action_name: "tool_b".into(),
                parameters: serde_json::json!({}),
            },
        ];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].call_id, "id_aaaa");
        assert_eq!(result.results[1].call_id, "id_bbbb");
    }

    #[tokio::test]
    async fn alias_normalization_stays_consistent_between_preflight_and_consume() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("create-issue")],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "create-issue".into(),
                output: serde_json::json!({"ok": true}),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(
                thread.id,
                "github",
                GrantedActions::Specific(vec!["create_issue".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_alias_consume".into(),
            action_name: "create-issue".into(),
            parameters: serde_json::json!({"title": "test"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].call_id, "call_alias_consume");
        assert_eq!(result.results[0].action_name, "create-issue");
        assert!(!result.results[0].is_error);
    }

    #[tokio::test]
    async fn aliased_action_name_still_triggers_policy_approval() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![ActionDef {
                name: "create_issue".into(),
                description: "Create issue".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: true,
                model_tool_surface: ModelToolSurface::FullSchema,
                discovery: None,
            }],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "create_issue".into(),
                output: serde_json::json!({"ok": true}),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(
                thread.id,
                "github",
                GrantedActions::Specific(vec!["create_issue".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_alias_policy".into(),
            action_name: "create-issue".into(),
            parameters: serde_json::json!({"title": "policy should still apply"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // The default `CancellingGateController` cancels the gate
        // synchronously, so the batch surfaces a denied error result for
        // the aliased call rather than the legacy
        // `need_approval = Some(ThreadOutcome::GatePaused {...})`.
        // What still must hold: the gate was *evaluated* (an
        // ApprovalRequested event was emitted with the aliased name and
        // the original call_id), the action did NOT execute, and the
        // result is_error.
        assert!(
            result.need_approval.is_none(),
            "controller-driven path must not bubble need_approval up; got {:?}",
            result.need_approval
        );
        assert_eq!(result.results.len(), 1);
        assert!(
            result.results[0].is_error,
            "denied gate must surface as an error result"
        );
        assert_eq!(result.results[0].call_id, "call_alias_policy");
        assert_eq!(result.results[0].action_name, "create-issue");
        assert!(
            result.events.iter().any(|event| matches!(
                event,
                EventKind::ApprovalRequested { action_name, call_id, .. }
                    if action_name == "create-issue" && call_id == "call_alias_policy"
            )),
            "approval event should use the aliased action name and original call id"
        );
        assert!(
            result.events.iter().any(|event| matches!(
                event,
                EventKind::ActionFailed { action_name, call_id, .. }
                    if action_name == "create-issue" && call_id == "call_alias_policy"
            )),
            "denied gate should produce an ActionFailed event"
        );
    }

    // ── GatePaused(Authentication) tests ─────────────────────

    #[tokio::test]
    async fn authentication_gate_interrupts_batch() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("http")],
            vec![Err(EngineError::GatePaused {
                gate_name: "authentication".into(),
                action_name: "http".into(),
                call_id: "call_auth_1".into(),
                parameters: Box::new(serde_json::json!({"url": "https://api.github.com/repos"})),
                resume_kind: Box::new(crate::gate::ResumeKind::Authentication {
                    credential_name: ironclaw_common::CredentialName::new("github_token").unwrap(),
                    instructions: "Provide your github_token token".into(),
                    auth_url: None,
                }),
                resume_output: None,
                paused_lease: None,
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_auth_1".into(),
            action_name: "http".into(),
            parameters: serde_json::json!({"url": "https://api.github.com/repos"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // Batch should be interrupted with GatePaused(Authentication)
        assert!(
            result.need_approval.is_some(),
            "GatePaused(Authentication) should interrupt the batch"
        );
        match result.need_approval.unwrap() {
            ThreadOutcome::GatePaused {
                gate_name,
                action_name,
                resume_kind,
                ..
            } => {
                assert_eq!(gate_name, "authentication");
                assert_eq!(action_name, "http");
                match resume_kind {
                    crate::gate::ResumeKind::Authentication {
                        credential_name, ..
                    } => {
                        assert_eq!(credential_name, "github_token");
                    }
                    other => panic!("expected auth resume kind, got {:?}", other),
                }
            }
            other => panic!("expected GatePaused, got {:?}", other),
        }

        // Gate pause event should be emitted
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, EventKind::ApprovalRequested { gate_name: Some(name), .. } if name == "authentication")),
            "should emit gate pause event"
        );
    }

    #[tokio::test]
    async fn authentication_gate_flags_batch_with_parallel_results() {
        // Two calls: first needs auth, second succeeds.
        // With parallel execution, both run concurrently — the batch is flagged
        // with GatePaused(Authentication) but results from all calls are available.
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("http"), test_action("echo")],
            vec![
                Err(EngineError::GatePaused {
                    gate_name: "authentication".into(),
                    action_name: "http".into(),
                    call_id: "call_1".into(),
                    parameters: Box::new(serde_json::json!({})),
                    resume_kind: Box::new(crate::gate::ResumeKind::Authentication {
                        credential_name: ironclaw_common::CredentialName::new("api_key").unwrap(),
                        instructions: "Provide your api_key token".into(),
                        auth_url: None,
                    }),
                    resume_output: None,
                    paused_lease: None,
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "echo".into(),
                    output: serde_json::json!("second ran"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
            ],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![
            ActionCall {
                id: "call_1".into(),
                action_name: "http".into(),
                parameters: serde_json::json!({}),
            },
            ActionCall {
                id: "call_2".into(),
                action_name: "echo".into(),
                parameters: serde_json::json!({}),
            },
        ];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // Both calls executed in parallel — results from both are available
        assert_eq!(result.results.len(), 2);
        // First call should be an auth error
        assert!(result.results[0].is_error);
        assert_eq!(result.results[0].call_id, "call_1");
        // Second call succeeded
        assert_eq!(result.results[1].call_id, "call_2");
        assert!(!result.results[1].is_error);
        // Batch is still flagged with GatePaused(Authentication)
        assert!(result.need_approval.is_some());
        match result.need_approval.unwrap() {
            ThreadOutcome::GatePaused {
                gate_name,
                resume_kind,
                ..
            } => {
                assert_eq!(gate_name, "authentication");
                match resume_kind {
                    crate::gate::ResumeKind::Authentication {
                        credential_name, ..
                    } => {
                        assert_eq!(credential_name, "api_key");
                    }
                    other => panic!("expected auth resume kind, got {:?}", other),
                }
            }
            other => panic!("expected GatePaused, got {:?}", other),
        }
    }

    /// Regular EngineError::Effect (not GatePaused) should NOT interrupt —
    /// it becomes a normal error result and execution continues.
    #[tokio::test]
    async fn regular_effect_error_does_not_interrupt() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("http"), test_action("echo")],
            vec![
                Err(EngineError::Effect {
                    reason: "connection timeout".into(),
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "echo".into(),
                    output: serde_json::json!("second call ran"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
            ],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![
            ActionCall {
                id: "call_1".into(),
                action_name: "http".into(),
                parameters: serde_json::json!({}),
            },
            ActionCall {
                id: "call_2".into(),
                action_name: "echo".into(),
                parameters: serde_json::json!({}),
            },
        ];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // Both calls should have results (error does not interrupt)
        assert_eq!(result.results.len(), 2);
        assert!(result.results[0].is_error);
        assert!(!result.results[1].is_error);
        assert!(
            result.need_approval.is_none(),
            "no interruption for regular errors"
        );
    }

    // ── call_id preservation (OpenAI/Mistral) ─────────────────

    /// Provider-specific: OpenAI rejects empty string call_id. Verify no result
    /// ever has an empty call_id when the ActionCall provided one.
    #[tokio::test]
    async fn openai_empty_call_id_never_produced() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("echo")],
            vec![Ok(ActionResult {
                call_id: String::new(), // EffectExecutor always returns empty
                action_name: String::new(),
                output: serde_json::json!("hello"),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "cap", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "aB3xK9mZq".into(), // Mistral-compatible 9-char ID
            action_name: "echo".into(),
            parameters: serde_json::json!({}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // Must NOT be empty — must be stamped from the ActionCall
        assert!(!result.results[0].call_id.is_empty());
        assert_eq!(result.results[0].call_id, "aB3xK9mZq");
    }

    /// Mistral requires call_id matching [a-zA-Z0-9]{9}.
    /// Verify the ID passes through unmodified (normalization is LLM-layer concern,
    /// but engine must never lose it).
    #[tokio::test]
    async fn mistral_format_call_id_preserved() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("web_search")],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "web_search".into(),
                output: serde_json::json!({}),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "cap", GrantedActions::All, None, None)
            .await
            .unwrap();

        // Mistral format: exactly 9 alphanumeric chars
        let mistral_id = "xK3mR9bZq";
        let calls = vec![ActionCall {
            id: mistral_id.into(),
            action_name: "web_search".into(),
            parameters: serde_json::json!({}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results[0].call_id, mistral_id);

        // Event also preserves the exact format
        if let Some(EventKind::ActionExecuted { call_id, .. }) = result.events.first() {
            assert_eq!(call_id, mistral_id);
        }
    }

    struct SnapshotAwareEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for SnapshotAwareEffects {
        async fn execute_action(
            &self,
            name: &str,
            _params: serde_json::Value,
            _lease: &CapabilityLease,
            ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let canonical_name = ctx
                .available_actions_snapshot
                .as_ref()
                .and_then(|actions| actions.iter().find(|action| action.matches_name(name)))
                .map(|action| action.name.clone())
                .unwrap_or_else(|| format!("missing_snapshot:{name}"));

            Ok(ActionResult {
                call_id: String::new(),
                action_name: canonical_name,
                output: serde_json::json!({"ok": true}),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(vec![test_action("create_issue")])
        }

        async fn available_action_inventory(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<crate::types::capability::ActionInventory, EngineError> {
            Ok(crate::types::capability::ActionInventory {
                inline: vec![test_action("create_issue")],
                discoverable: vec![ActionDef {
                    name: "gmail_send".to_string(),
                    description: "Send an email".to_string(),
                    parameters_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "to": {"type": "string"}
                        },
                        "required": ["to"]
                    }),
                    effects: vec![],
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::CompactToolInfo,
                    discovery: None,
                }],
            })
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn structured_execution_propagates_snapshot_to_executor_context() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(SnapshotAwareEffects);
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(
                thread.id,
                "tools",
                GrantedActions::Specific(vec!["create_issue".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_snapshot_ctx".into(),
            action_name: "create-issue".into(),
            parameters: serde_json::json!({"title": "snapshot propagation"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].action_name, "create_issue");
        assert!(!result.results[0].is_error);
    }

    struct StructuredToolInfoEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for StructuredToolInfoEffects {
        async fn execute_action(
            &self,
            name: &str,
            params: serde_json::Value,
            _lease: &CapabilityLease,
            ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let output = if name == "tool_info" {
                let requested = params
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let action =
                    ctx.available_action_inventory_snapshot
                        .as_ref()
                        .and_then(|inventory| {
                            inventory
                                .inline
                                .iter()
                                .chain(inventory.discoverable.iter())
                                .find(|action| action.matches_name(requested))
                        });
                if params.get("detail").and_then(|value| value.as_str()) == Some("schema") {
                    match action {
                        Some(action) => serde_json::json!({
                            "name": action.name.clone(),
                            "schema": action.parameters_schema.clone()
                        }),
                        None => serde_json::json!({
                            "error": format!("missing_action:{requested}")
                        }),
                    }
                } else {
                    let discovered = action
                        .map(|action| action.name.clone())
                        .unwrap_or_else(|| format!("missing_action:{requested}"));
                    serde_json::json!({ "resolved": discovered })
                }
            } else {
                serde_json::json!({ "ok": true })
            };

            Ok(ActionResult {
                call_id: String::new(),
                action_name: name.replace('-', "_"),
                is_error: output.get("error").is_some(),
                output,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(vec![test_action("tool_info")])
        }

        async fn available_action_inventory(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<crate::types::capability::ActionInventory, EngineError> {
            Ok(crate::types::capability::ActionInventory {
                inline: vec![
                    test_action("tool_info"),
                    ActionDef {
                        name: "mission_create".to_string(),
                        description: "Create a mission".to_string(),
                        parameters_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "goal": {"type": "string"},
                                "cadence": {"type": "string"}
                            },
                            "required": ["name", "goal", "cadence"]
                        }),
                        effects: vec![],
                        requires_approval: false,
                        model_tool_surface: ModelToolSurface::CompactToolInfo,
                        discovery: None,
                    },
                ],
                discoverable: vec![ActionDef {
                    name: "gmail_send".to_string(),
                    description: "Send an email".to_string(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: vec![],
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::CompactToolInfo,
                    discovery: None,
                }],
            })
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn structured_execution_propagates_action_inventory_snapshot_to_executor_context() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(StructuredToolInfoEffects);
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(
                thread.id,
                "tools",
                GrantedActions::Specific(vec!["tool_info".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_tool_info_snapshot".into(),
            action_name: "tool_info".into(),
            parameters: serde_json::json!({"name": "gmail_send", "detail": "summary"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(
            result.results[0].output["resolved"],
            serde_json::json!("gmail_send")
        );
        assert!(!result.results[0].is_error);
    }

    #[tokio::test]
    async fn structured_execution_resolves_tool_info_schema_from_action_inventory() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(StructuredToolInfoEffects);
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(
                thread.id,
                "tools",
                GrantedActions::Specific(vec!["tool_info".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_tool_info_schema".into(),
            action_name: "tool_info".into(),
            parameters: serde_json::json!({"name": "mission-create", "detail": "schema"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert!(!result.results[0].is_error);
        assert_eq!(
            result.results[0].output["name"],
            serde_json::json!("mission_create")
        );
        assert_eq!(
            result.results[0].output["schema"]["required"],
            serde_json::json!(["name", "goal", "cadence"])
        );
    }

    #[tokio::test]
    async fn structured_execution_marks_missing_tool_info_action_as_error() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(StructuredToolInfoEffects);
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(
                thread.id,
                "tools",
                GrantedActions::Specific(vec!["tool_info".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_tool_info_missing_action".into(),
            action_name: "tool_info".into(),
            parameters: serde_json::json!({"name": "missing-action", "detail": "schema"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert!(
            result.results[0].is_error,
            "tool_info missing-action output must be marked as an error: {:?}",
            result.results[0].output
        );
        assert_eq!(
            result.results[0].output["error"],
            serde_json::json!("missing_action:missing-action")
        );
    }

    struct DiscoverableOnlyEffects {
        executed: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl EffectExecutor for DiscoverableOnlyEffects {
        async fn execute_action(
            &self,
            name: &str,
            _params: serde_json::Value,
            _lease: &CapabilityLease,
            _ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            self.executed.lock().unwrap().push(name.to_string());
            Ok(ActionResult {
                call_id: String::new(),
                action_name: name.to_string(),
                output: serde_json::json!({"ok": true}),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(Vec::new())
        }

        async fn available_action_inventory(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<crate::types::capability::ActionInventory, EngineError> {
            Ok(crate::types::capability::ActionInventory {
                inline: Vec::new(),
                discoverable: vec![ActionDef {
                    name: "gmail_send".to_string(),
                    description: "Send an email".to_string(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: vec![],
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::CompactToolInfo,
                    discovery: None,
                }],
            })
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn structured_execution_rejects_discoverable_only_actions() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let executed = Arc::new(Mutex::new(Vec::new()));
        let effects: Arc<dyn EffectExecutor> = Arc::new(DiscoverableOnlyEffects {
            executed: Arc::clone(&executed),
        });
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_discoverable_only".into(),
            action_name: "gmail_send".into(),
            parameters: serde_json::json!({"to": "person@example.com"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert!(result.results[0].is_error);
        assert!(
            result.results[0].output["error"]
                .as_str()
                .unwrap_or_default()
                .contains("not callable in this execution context")
        );
        match result.events.first() {
            Some(EventKind::ActionFailed { params_summary, .. }) => {
                assert_eq!(
                    *params_summary,
                    crate::types::event::summarize_params(
                        "gmail_send",
                        &serde_json::json!({"to": "person@example.com"}),
                    )
                );
            }
            other => panic!("expected ActionFailed event, got {other:?}"),
        }
        assert!(executed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn structured_policy_denial_preserves_params_summary() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> =
            Arc::new(MockEffects::new(vec![test_action("shell")], vec![]));
        let leases = Arc::new(LeaseManager::new());
        let mut policy = PolicyEngine::new();
        policy.deny_effect(EffectType::ReadLocal);
        let policy = Arc::new(policy);
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "exec", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_policy_denied".into(),
            action_name: "shell".into(),
            parameters: serde_json::json!({"cmd": "ls"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(result.results.len(), 1);
        assert!(result.results[0].is_error);
        match result.events.first() {
            Some(EventKind::ActionFailed {
                error,
                params_summary,
                ..
            }) => {
                assert!(error.contains("denied"));
                assert_eq!(
                    *params_summary,
                    crate::types::event::summarize_params(
                        "shell",
                        &serde_json::json!({"cmd": "ls"}),
                    )
                );
            }
            other => panic!("expected ActionFailed event, got {other:?}"),
        }
    }

    // ── Inline-retry ApprovalRequested audit-event tests ────────

    /// Local stub gate controller for the inline-retry tests below.
    /// Approves once on first pause, returns the canned resolution
    /// thereafter; records every pause request for assertion.
    struct StubGateController {
        resolution: Mutex<Option<crate::gate::GateResolution>>,
        pauses: Mutex<Vec<crate::gate::GatePauseRequest>>,
    }

    impl StubGateController {
        fn approving_arc() -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                resolution: Mutex::new(Some(crate::gate::GateResolution::Approved {
                    always: false,
                })),
                pauses: Mutex::new(Vec::new()),
            })
        }

        fn denying_arc() -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                resolution: Mutex::new(Some(crate::gate::GateResolution::Denied {
                    reason: Some("user declined".into()),
                })),
                pauses: Mutex::new(Vec::new()),
            })
        }

        fn pause_count(&self) -> usize {
            self.pauses.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl crate::gate::GateController for StubGateController {
        async fn pause(
            &self,
            request: crate::gate::GatePauseRequest,
        ) -> crate::gate::GateResolution {
            self.pauses.lock().unwrap().push(request);
            self.resolution
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(crate::gate::GateResolution::Cancelled)
        }
    }

    /// Mid-execution `GatePaused(Approval)` followed by user approval
    /// must emit BOTH `ApprovalRequested` and the final
    /// `ActionExecuted` event in order. Regression for the audit drop
    /// noted by serrrfirat on the structured executor.
    #[tokio::test]
    async fn inline_retry_emits_approval_requested_event_before_outcome() {
        let thread = Thread::new(
            "audit-test",
            ThreadType::Foreground,
            ProjectId::new(),
            "audit-user",
            ThreadConfig::default(),
        );

        // Effects:
        //   call 1 → Err(GatePaused) — tool raises mid-execution gate
        //   call 2 → Ok(success)     — after user approval, retry succeeds
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("write_file")],
            vec![
                Err(EngineError::GatePaused {
                    gate_name: "approval".into(),
                    action_name: "write_file".into(),
                    call_id: "call_audit_1".into(),
                    parameters: Box::new(serde_json::json!({"path": "/tmp/x"})),
                    resume_kind: Box::new(crate::gate::ResumeKind::Approval { allow_always: true }),
                    resume_output: None,
                    paused_lease: None,
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "write_file".into(),
                    output: serde_json::json!({"bytes_written": 12}),
                    is_error: false,
                    duration: Duration::from_millis(7),
                }),
            ],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let mut ctx = make_exec_context(&thread);
        let controller = StubGateController::approving_arc();
        ctx.gate_controller = controller.clone();

        leases
            .grant(thread.id, "fs", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_audit_1".into(),
            action_name: "write_file".into(),
            parameters: serde_json::json!({"path": "/tmp/x"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // Controller saw exactly one pause.
        assert_eq!(controller.pause_count(), 1);

        // Events MUST contain ApprovalRequested followed by ActionExecuted.
        let approval_idx = result
            .events
            .iter()
            .position(|e| matches!(e, EventKind::ApprovalRequested { .. }))
            .expect("ApprovalRequested must be emitted");
        let executed_idx = result
            .events
            .iter()
            .position(|e| matches!(e, EventKind::ActionExecuted { .. }))
            .expect("ActionExecuted must be emitted after approval");
        assert!(
            approval_idx < executed_idx,
            "ApprovalRequested must come before ActionExecuted; got events={:?}",
            result.events
        );

        // ApprovalRequested carries the call's identifying metadata.
        match &result.events[approval_idx] {
            EventKind::ApprovalRequested {
                action_name,
                call_id,
                gate_name,
                allow_always,
                ..
            } => {
                assert_eq!(action_name, "write_file");
                assert_eq!(call_id, "call_audit_1");
                assert_eq!(gate_name.as_deref(), Some("approval"));
                assert_eq!(*allow_always, Some(true));
            }
            other => panic!("expected ApprovalRequested, got {other:?}"),
        }

        // The terminal action result is success (not gate_paused).
        assert!(!result.results[0].is_error);
        assert!(result.need_approval.is_none());
    }

    /// Same shape, but the user denies. The `ApprovalRequested` event
    /// is still emitted before the `ActionFailed` event so audit logs
    /// see the full lifecycle.
    #[tokio::test]
    async fn inline_retry_emits_approval_requested_event_before_denial() {
        let thread = Thread::new(
            "audit-test-denied",
            ThreadType::Foreground,
            ProjectId::new(),
            "audit-user",
            ThreadConfig::default(),
        );

        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("write_file")],
            vec![Err(EngineError::GatePaused {
                gate_name: "approval".into(),
                action_name: "write_file".into(),
                call_id: "call_audit_2".into(),
                parameters: Box::new(serde_json::json!({"path": "/tmp/y"})),
                resume_kind: Box::new(crate::gate::ResumeKind::Approval {
                    allow_always: false,
                }),
                resume_output: None,
                paused_lease: None,
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let mut ctx = make_exec_context(&thread);
        let controller = StubGateController::denying_arc();
        ctx.gate_controller = controller.clone();

        leases
            .grant(thread.id, "fs", GrantedActions::All, None, None)
            .await
            .unwrap();

        let calls = vec![ActionCall {
            id: "call_audit_2".into(),
            action_name: "write_file".into(),
            parameters: serde_json::json!({"path": "/tmp/y"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        assert_eq!(controller.pause_count(), 1);

        let approval_idx = result
            .events
            .iter()
            .position(|e| matches!(e, EventKind::ApprovalRequested { .. }))
            .expect("ApprovalRequested must be emitted even on denial");
        let failed_idx = result
            .events
            .iter()
            .position(|e| matches!(e, EventKind::ActionFailed { .. }))
            .expect("ActionFailed must be emitted after denial");
        assert!(
            approval_idx < failed_idx,
            "ApprovalRequested must come before ActionFailed; got events={:?}",
            result.events
        );
    }
}
