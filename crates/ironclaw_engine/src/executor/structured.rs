//! Tier 0 executor: structured tool calls.
//!
//! Executes action calls by delegating to the `EffectExecutor` trait,
//! checking leases and policies for each call.
//!
//! Uses a two-phase approach: sequential preflight (lease/policy checks)
//! followed by parallel execution of all approved actions via `JoinSet`.

use std::sync::Arc;

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
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
) -> Result<ActionBatchResult, EngineError> {
    let mut preflight_results: Vec<PreflightOutcome> = Vec::with_capacity(calls.len());
    let mut early_events = Vec::new();
    let mut early_results = Vec::new();

    // ── Phase 1: Preflight (sequential) ─────────────────────────
    // Check leases and policies for every call. RequireApproval interrupts
    // the entire batch immediately. Denied/no-lease calls become error results.

    for (idx, call) in calls.iter().enumerate() {
        // 1. Find the lease for this action (read-only lookup for policy check)
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
                    params_summary: None,
                };
                preflight_results.push(PreflightOutcome::Error {
                    index: idx,
                    result: error_result,
                    event,
                });
                continue;
            }
        };

        // 2. Find the action definition and check policy
        let action_def = effects
            .available_actions(std::slice::from_ref(&lease))
            .await?
            .into_iter()
            .find(|a| a.name == call.action_name);

        if let Some(ref action_def) = action_def {
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
                        params_summary: None,
                    };
                    preflight_results.push(PreflightOutcome::Error {
                        index: idx,
                        result: error_result,
                        event,
                    });
                    continue;
                }
                PolicyDecision::RequireApproval { .. } => {
                    // Collect error results from earlier preflight failures
                    for pf in preflight_results {
                        if let PreflightOutcome::Error { result, event, .. } = pf {
                            early_results.push(result);
                            early_events.push(event);
                        }
                    }
                    early_events.push(EventKind::ApprovalRequested {
                        action_name: call.action_name.clone(),
                        call_id: call.id.clone(),
                        parameters: Some(call.parameters.clone()),
                        description: None,
                        allow_always: None,
                        gate_name: None,
                        params_summary: crate::types::event::summarize_params(
                            &call.action_name,
                            &call.parameters,
                        ),
                    });
                    return Ok(ActionBatchResult {
                        results: early_results,
                        events: early_events,
                        need_approval: Some(ThreadOutcome::GatePaused {
                            gate_name: "approval".into(),
                            action_name: call.action_name.clone(),
                            call_id: call.id.clone(),
                            parameters: call.parameters.clone(),
                            resume_kind: crate::gate::ResumeKind::Approval { allow_always: true },
                            resume_output: None,
                        }),
                    });
                }
                PolicyDecision::Allow => {}
            }
        }

        // 3. Atomically find + consume a lease use under a single write lock.
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

    // Separate runnable from preflight errors
    let mut slot_results: Vec<Option<(ActionResult, EventKind)>> = vec![None; calls.len()];
    let mut runnable_indices = Vec::new();

    for pf in preflight_results {
        match pf {
            PreflightOutcome::Error {
                index,
                result,
                event,
                ..
            } => {
                slot_results[index] = Some((result, event));
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
        let mut exec_ctx = context.clone();
        exec_ctx.current_call_id = Some(call.id.clone());
        let exec_result = effects
            .execute_action(
                &call.action_name,
                call.parameters.clone(),
                &lease,
                &exec_ctx,
            )
            .await;
        if interrupted_call_needs_refund(&exec_result) {
            let _ = leases.refund_use(lease.id).await;
        }
        slot_results[idx] = Some(classify_exec_result(exec_result, call, &exec_ctx));
    } else if runnable_indices.len() > 1 {
        // Multiple calls: execute in parallel via JoinSet
        let mut join_set = tokio::task::JoinSet::new();
        let effects = effects.clone();

        for (idx, lease) in runnable_indices {
            let call = calls[idx].clone();
            let mut ctx = context.clone();
            ctx.current_call_id = Some(call.id.clone());
            let effects = effects.clone();
            let lease = lease.clone();

            join_set.spawn(async move {
                let result = effects
                    .execute_action(&call.action_name, call.parameters.clone(), &lease, &ctx)
                    .await;
                (idx, lease.id, result, call, ctx)
            });
        }

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, lease_id, result, call, ctx)) => {
                    if interrupted_call_needs_refund(&result) {
                        let _ = leases.refund_use(lease_id).await;
                    }
                    slot_results[idx] = Some(classify_exec_result(result, &call, &ctx));
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
    let mut events = Vec::new();
    let mut first_interrupt: Option<ThreadOutcome> = None;

    for (idx, slot) in slot_results.into_iter().enumerate() {
        if let Some((result, event)) = slot {
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
                });
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

/// Classify an execution result into an `(ActionResult, EventKind)` pair.
///
/// Used by both the single-call fast path and the parallel JoinSet path
/// to produce uniform output.
fn classify_exec_result(
    result: Result<ActionResult, EngineError>,
    call: &ActionCall,
    context: &ThreadExecutionContext,
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
                EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: error_msg,
                    params_summary: None,
                }
            } else {
                EventKind::ActionExecuted {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    duration_ms: action_result.duration.as_millis() as u64,
                    params_summary: None,
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
                params_summary: None,
            };
            (error_result, event)
        }
    }
}

fn interrupted_call_needs_refund(result: &Result<ActionResult, EngineError>) -> bool {
    matches!(result, Err(EngineError::GatePaused { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::effect::ThreadExecutionContext;
    use crate::types::capability::{ActionDef, CapabilityLease, EffectType, GrantedActions};
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
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.clone())
        }
    }

    fn test_action(name: &str) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: "Test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
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
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
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
                    credential_name: "github_token".into(),
                    instructions: "Provide your github_token token".into(),
                    auth_url: None,
                }),
                resume_output: None,
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
                        credential_name: "api_key".into(),
                        instructions: "Provide your api_key token".into(),
                        auth_url: None,
                    }),
                    resume_output: None,
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
}
