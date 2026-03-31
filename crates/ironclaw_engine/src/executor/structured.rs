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
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    error: format!("no lease for action '{}'", call.action_name),
                    params_summary: None,
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
                        params_summary: None,
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
            Ok(mut action_result) => {
                // EffectExecutor doesn't receive call_id; stamp it from the
                // original ActionCall so downstream messages carry the correct ID.
                action_result.call_id = call.id.clone();
                events.push(EventKind::ActionExecuted {
                    step_id: context.step_id,
                    action_name: call.action_name.clone(),
                    call_id: call.id.clone(),
                    duration_ms: action_result.duration.as_millis() as u64,
                    params_summary: None,
                });
                results.push(action_result);
            }
            Err(crate::types::error::EngineError::NeedAuthentication {
                credential_name,
                action_name,
                call_id,
                parameters,
            }) => {
                // Interrupt the batch — thread should pause for authentication.
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: action_name.clone(),
                    call_id: call_id.clone(),
                    error: format!("authentication required for credential '{credential_name}'"),
                    params_summary: None,
                });
                return Ok(ActionBatchResult {
                    results,
                    events,
                    need_approval: Some(ThreadOutcome::NeedAuthentication {
                        credential_name,
                        action_name,
                        call_id,
                        parameters,
                    }),
                });
            }
            Err(crate::types::error::EngineError::GatePaused {
                gate_name,
                action_name,
                call_id,
                parameters,
                resume_kind,
            }) => {
                // Unified gate pause — interrupt the batch.
                events.push(EventKind::ApprovalRequested {
                    action_name: action_name.clone(),
                    call_id: call_id.clone(),
                });
                return Ok(ActionBatchResult {
                    results,
                    events,
                    need_approval: Some(ThreadOutcome::GatePaused {
                        gate_name,
                        action_name,
                        call_id,
                        parameters: *parameters,
                        resume_kind: *resume_kind,
                    }),
                });
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
                    params_summary: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::effect::ThreadExecutionContext;
    use crate::types::capability::{ActionDef, CapabilityLease, EffectType};
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

        leases.grant(thread.id, "search", vec![], None, None).await;

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

        leases.grant(thread.id, "exec", vec![], None, None).await;

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

        leases.grant(thread.id, "cap", vec![], None, None).await;

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

    // ── NeedAuthentication tests ─────────────────────────────

    #[tokio::test]
    async fn need_authentication_interrupts_batch() {
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        );
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("http")],
            vec![Err(EngineError::NeedAuthentication {
                credential_name: "github_token".into(),
                action_name: "http".into(),
                call_id: "call_auth_1".into(),
                parameters: serde_json::json!({"url": "https://api.github.com/repos"}),
            })],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases.grant(thread.id, "tools", vec![], None, None).await;

        let calls = vec![ActionCall {
            id: "call_auth_1".into(),
            action_name: "http".into(),
            parameters: serde_json::json!({"url": "https://api.github.com/repos"}),
        }];

        let result = execute_action_calls(&calls, &thread, &effects, &leases, &policy, &ctx, &[])
            .await
            .unwrap();

        // Batch should be interrupted with NeedAuthentication outcome
        assert!(
            result.need_approval.is_some(),
            "NeedAuthentication should interrupt the batch"
        );
        match result.need_approval.unwrap() {
            ThreadOutcome::NeedAuthentication {
                credential_name,
                action_name,
                ..
            } => {
                assert_eq!(credential_name, "github_token");
                assert_eq!(action_name, "http");
            }
            other => panic!("expected NeedAuthentication, got {:?}", other),
        }

        // ActionFailed event should be emitted
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, EventKind::ActionFailed { .. })),
            "should emit ActionFailed event"
        );
    }

    #[tokio::test]
    async fn need_authentication_stops_before_subsequent_calls() {
        // Two calls: first needs auth, second should never execute
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
                Err(EngineError::NeedAuthentication {
                    credential_name: "api_key".into(),
                    action_name: "http".into(),
                    call_id: "call_1".into(),
                    parameters: serde_json::json!({}),
                }),
                // This should never be called
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "echo".into(),
                    output: serde_json::json!("should not appear"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
            ],
        ));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());
        let ctx = make_exec_context(&thread);

        leases.grant(thread.id, "tools", vec![], None, None).await;

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

        // Second call should NOT have executed
        assert!(
            result.results.is_empty(),
            "no results should be returned before the interrupted call"
        );
        assert!(result.need_approval.is_some());
    }

    /// Regular EngineError::Effect (not NeedAuthentication) should NOT interrupt —
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

        leases.grant(thread.id, "tools", vec![], None, None).await;

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

        leases.grant(thread.id, "cap", vec![], None, None).await;

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

        leases.grant(thread.id, "cap", vec![], None, None).await;

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
