//! Tier 0 executor: structured tool calls.
//!
//! Executes action calls by delegating to the `EffectExecutor` trait,
//! checking leases and policies for each call.

use std::sync::Arc;

use crate::capability::lease::LeaseManager;
use crate::capability::policy::{PolicyDecision, PolicyEngine};
use crate::runtime::messaging::ThreadOutcome;
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
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

/// Execute a batch of action calls using the Tier 0 (structured) approach.
///
/// For each action call:
/// 1. Find the lease that grants this action
/// 2. Check policy (deny/allow/approve)
/// 3. Consume a lease use
/// 4. Call `EffectExecutor::execute_action()`
/// 5. Record result and emit event
///
/// Stops at the first action that requires approval.
pub async fn execute_action_calls(
    calls: &[ActionCall],
    thread: &Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
) -> Result<ActionBatchResult, EngineError> {
    let mut results = Vec::with_capacity(calls.len());
    let mut events = Vec::new();

    for call in calls {
        // 1. Find the lease for this action
        let lease = match leases.find_lease_for_action(thread.id, &call.action_name).await {
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
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: format!("no lease for action '{}'", call.action_name),
                });
                results.push(error_result);
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
                    events.push(EventKind::ActionFailed {
                        step_id: context.step_id,
                        action_name: call.action_name.clone(),
                        call_id: call.id.clone(),
                        error: reason,
                    });
                    results.push(error_result);
                    continue;
                }
                PolicyDecision::RequireApproval { .. } => {
                    events.push(EventKind::ApprovalRequested {
                        action_name: call.action_name.clone(),
                        call_id: call.id.clone(),
                    });
                    return Ok(ActionBatchResult {
                        results,
                        events,
                        need_approval: Some(ThreadOutcome::NeedApproval {
                            action_name: call.action_name.clone(),
                            call_id: call.id.clone(),
                            parameters: call.parameters.clone(),
                        }),
                    });
                }
                PolicyDecision::Allow => {}
            }
        }

        // 3. Consume a lease use
        leases.consume_use(lease.id).await?;

        // 4. Execute the action
        let result = effects
            .execute_action(&call.action_name, call.parameters.clone(), &lease, context)
            .await;

        match result {
            Ok(action_result) => {
                events.push(EventKind::ActionExecuted {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    duration_ms: action_result.duration.as_millis() as u64,
                });
                results.push(action_result);
            }
            Err(e) => {
                let error_result = ActionResult {
                    call_id: call.id.clone(),
                    action_name: call.action_name.clone(),
                    output: serde_json::json!({"error": e.to_string()}),
                    is_error: true,
                    duration: std::time::Duration::ZERO,
                };
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: e.to_string(),
                });
                results.push(error_result);
            }
        }
    }

    Ok(ActionBatchResult {
        results,
        events,
        need_approval: None,
    })
}
