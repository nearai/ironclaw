use super::*;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_host_api::output::OutputContract;
use ironclaw_loop_contracts::{
    AgentLoopHostErrorKind, InMemoryRunProfileResolver, LoopModelBudgetAccountant,
    LoopModelGatewayError, LoopModelPolicyGuard, ModelWorkOutcome, ModelWorkRequest,
    NoOpBudgetAccountant, NoOpPolicyGuard, RunProfileResolutionRequest, RunProfileResolver,
    SystemInferenceContextMessage, SystemInferenceContextRole, SystemInferenceIdentity,
    SystemInferenceTaskId, SystemPromptSource, SystemTaskKind,
};
use ironclaw_turns::{TurnId, TurnRunId, TurnScope};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

struct RecordingGateway {
    request: Mutex<Option<HostManagedModelRequest>>,
    progress_requests: AtomicUsize,
    response: Result<crate::HostManagedModelResponse, crate::HostManagedModelError>,
}

impl RecordingGateway {
    fn new(response: crate::HostManagedModelResponse) -> Self {
        Self::with_result(Ok(response))
    }

    fn with_result(
        response: Result<crate::HostManagedModelResponse, crate::HostManagedModelError>,
    ) -> Self {
        Self {
            request: Mutex::new(None),
            progress_requests: AtomicUsize::new(0),
            response,
        }
    }

    fn request(&self) -> HostManagedModelRequest {
        self.request
            .lock()
            .expect("lock")
            .clone()
            .expect("request recorded")
    }

    fn request_was_recorded(&self) -> bool {
        self.request.lock().expect("lock").is_some()
    }

    fn progress_requests(&self) -> usize {
        self.progress_requests.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
        *self.request.lock().expect("lock") = Some(request);
        self.response.clone()
    }

    async fn stream_model_with_progress(
        &self,
        request: HostManagedModelRequest,
        _sink: Arc<dyn crate::HostManagedModelStreamSink>,
    ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
        self.progress_requests.fetch_add(1, Ordering::SeqCst);
        *self.request.lock().expect("lock") = Some(request);
        self.response.clone()
    }
}

struct SlowGateway {
    delay: std::time::Duration,
    progress_requests: AtomicUsize,
}

#[async_trait]
impl HostManagedModelGateway for SlowGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
        tokio::time::sleep(self.delay).await;
        Ok(crate::HostManagedModelResponse::assistant_reply("too late"))
    }

    async fn stream_model_with_progress(
        &self,
        _request: HostManagedModelRequest,
        _sink: Arc<dyn crate::HostManagedModelStreamSink>,
    ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
        self.progress_requests.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(crate::HostManagedModelResponse::assistant_reply("too late"))
    }
}

struct PanicGateway;

#[async_trait]
impl HostManagedModelGateway for PanicGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
        panic!("test gateway panic")
    }
}

struct PendingInferencePort {
    started: Arc<Notify>,
}

#[async_trait]
impl SystemInferencePort for PendingInferencePort {
    async fn call_system_inference(
        &self,
        _request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        self.started.notify_one();
        std::future::pending().await
    }
}

struct DenySystemInferencePolicyGuard;

#[async_trait]
impl LoopModelPolicyGuard for DenySystemInferencePolicyGuard {
    async fn check_model_work_policy(
        &self,
        _context: &LoopRunContext,
        request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError> {
        assert!(matches!(
            request.kind,
            ironclaw_loop_contracts::ModelWorkKind::SystemInference { .. }
        ));
        Err(LoopModelGatewayError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            "system inference denied",
        )
        .expect("safe summary is valid"))
    }
}

struct RejectingBudgetAccountant;

#[async_trait]
impl LoopModelBudgetAccountant for RejectingBudgetAccountant {
    async fn pre_model_work(
        &self,
        _context: &LoopRunContext,
        request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError> {
        assert!(matches!(
            request.kind,
            ironclaw_loop_contracts::ModelWorkKind::SystemInference { .. }
        ));
        Err(LoopModelGatewayError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "system inference budget exceeded",
        )
        .expect("safe summary is valid"))
    }

    async fn post_model_work(
        &self,
        _context: &LoopRunContext,
        _request: &ModelWorkRequest,
        _outcome: ModelWorkOutcome,
    ) -> Result<(), LoopModelGatewayError> {
        panic!("post_model_work must not run when pre_model_work rejects")
    }
}

#[derive(Default)]
struct RecordingBudgetAccountant {
    pre_called: Mutex<bool>,
    post_outcomes: Mutex<Vec<ModelWorkOutcome>>,
    release_calls: AtomicUsize,
    fail_post: bool,
    panic_post: bool,
}

#[async_trait]
impl LoopModelBudgetAccountant for RecordingBudgetAccountant {
    async fn pre_model_work(
        &self,
        _context: &LoopRunContext,
        request: &ModelWorkRequest,
    ) -> Result<(), LoopModelGatewayError> {
        assert!(matches!(
            request.kind,
            ironclaw_loop_contracts::ModelWorkKind::SystemInference { .. }
        ));
        *self.pre_called.lock().expect("lock") = true;
        Ok(())
    }

    async fn post_model_work(
        &self,
        _context: &LoopRunContext,
        request: &ModelWorkRequest,
        outcome: ModelWorkOutcome,
    ) -> Result<(), LoopModelGatewayError> {
        assert!(matches!(
            request.kind,
            ironclaw_loop_contracts::ModelWorkKind::SystemInference { .. }
        ));
        if self.panic_post {
            panic!("post_model_work panic");
        }
        if self.fail_post {
            return Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetAccountingFailed,
                "system inference post-accounting failed",
            )
            .expect("safe summary is valid"));
        }
        self.post_outcomes.lock().expect("lock").push(outcome);
        Ok(())
    }

    fn release_in_flight(&self, _context: &LoopRunContext) {
        self.release_calls.fetch_add(1, Ordering::SeqCst);
    }
}

fn system_request(input_text: &str) -> SystemInferenceRequest {
    SystemInferenceRequest {
        task_id: SystemInferenceTaskId::new(),
        identity: SystemInferenceIdentity {
            task_kind: SystemTaskKind::Compaction,
            prompt_source: SystemPromptSource::Static {
                prompt_id: "test".to_string().try_into().unwrap(),
            },
            system_prompt: "summarize".to_string(),
        },
        input_text: input_text.to_string(),
        context_messages: Vec::new(),
        max_input_tokens: 100,
        deadline_ms: 100,
        output_contract: None,
    }
}

#[tokio::test]
async fn dispatches_direct_gateway_request_without_prompt_materialization() {
    let context = test_run_context("system-inference-direct").await;
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::assistant_reply("summary"),
    ));
    let port = ModelGatewayBackedSystemInferencePort::new(gateway.clone(), context.clone());
    let task_id = SystemInferenceTaskId::new();

    let response = port
        .call_system_inference(SystemInferenceRequest {
            task_id,
            identity: SystemInferenceIdentity {
                task_kind: SystemTaskKind::Compaction,
                prompt_source: SystemPromptSource::Static {
                    prompt_id: "test".to_string().try_into().unwrap(),
                },
                system_prompt: "summarize".to_string(),
            },
            input_text: "transcript".to_string(),
            context_messages: Vec::new(),
            max_input_tokens: 100,
            deadline_ms: 100,
            output_contract: None,
        })
        .await
        .expect("system inference succeeds");

    assert_eq!(response.output_text, "summary");
    assert_eq!(gateway.progress_requests(), 0);
    let request = gateway.request();
    assert_eq!(
        request.model_profile_id,
        context.resolved_run_profile.model_profile_id
    );
    assert_eq!(request.resolved_model_route, context.resolved_model_route);
    assert_eq!(request.run_id, context.run_id);
    assert_eq!(request.turn_id, context.turn_id);
    assert_eq!(request.surface_version, None);
    assert_eq!(request.messages.len(), 2);
    assert_eq!(
        request.messages[0].role,
        HostManagedModelMessageRole::System
    );
    assert_eq!(request.messages[0].content, "summarize");
    assert!(
        request.messages[0]
            .content_ref
            .as_str()
            .starts_with("msg:system-inference.system-prompt.")
    );
    assert_eq!(request.messages[1].role, HostManagedModelMessageRole::User);
    assert_eq!(request.messages[1].content, "transcript");
    assert!(
        request.messages[1]
            .content_ref
            .as_str()
            .starts_with("msg:system-inference.input.")
    );
}

#[tokio::test]
async fn structured_finalization_uses_native_schema_without_tools() {
    let context = test_run_context("system-inference-structured").await;
    let usage = ironclaw_loop_contracts::LoopModelUsage {
        input_tokens: 12,
        output_tokens: 4,
        ..Default::default()
    };
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::assistant_reply(r#"{"items":[]}"#).with_usage(usage),
    ));
    let port = ModelGatewayBackedSystemInferencePort::new(gateway.clone(), context);
    let tool_context = serde_json::to_string(
        &ironclaw_threads::ToolResultReferenceEnvelope::new(
            "result:structured-finalization-test",
            ironclaw_threads::ToolResultSafeSummary::new("tool result").unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let response = port
        .call_system_inference(SystemInferenceRequest {
            task_id: SystemInferenceTaskId::new(),
            identity: SystemInferenceIdentity {
                task_kind: SystemTaskKind::StructuredOutputFinalization,
                prompt_source: SystemPromptSource::Static {
                    prompt_id: "test".to_string().try_into().unwrap(),
                },
                system_prompt: "format the candidate".to_string(),
            },
            input_text: String::new(),
            context_messages: vec![
                SystemInferenceContextMessage {
                    role: SystemInferenceContextRole::User,
                    content: "prior user".to_string(),
                },
                SystemInferenceContextMessage {
                    role: SystemInferenceContextRole::Tool,
                    content: tool_context,
                },
                SystemInferenceContextMessage {
                    role: SystemInferenceContextRole::Assistant,
                    content: "prior assistant".to_string(),
                },
                SystemInferenceContextMessage {
                    role: SystemInferenceContextRole::Assistant,
                    content: "candidate".to_string(),
                },
            ],
            max_input_tokens: 100,
            deadline_ms: 100,
            output_contract: Some(OutputContract::JsonSchema {
                name: "response_schema".to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {"items": {"type": "array"}},
                    "required": ["items"]
                }),
            }),
        })
        .await
        .expect("structured finalization succeeds");

    assert_eq!(response.output_text, r#"{"items":[]}"#);
    assert_eq!(response.usage, Some(usage));
    assert_eq!(gateway.progress_requests(), 1);
    let request = gateway.request();
    assert_eq!(request.tool_choice, None);
    assert_eq!(request.messages.len(), 5);
    assert_eq!(request.messages[1].role, HostManagedModelMessageRole::User);
    assert_eq!(request.messages[1].content, "prior user");
    assert_eq!(request.messages[2].role, HostManagedModelMessageRole::User);
    assert!(
        request.messages[2]
            .content
            .contains("Untrusted tool result context")
    );
    assert!(request.messages[2].content.contains("tool result"));
    assert_eq!(
        request.messages[3].role,
        HostManagedModelMessageRole::Assistant
    );
    assert_eq!(request.messages[3].content, "prior assistant");
    assert_eq!(
        request.messages[4].role,
        HostManagedModelMessageRole::Assistant
    );
    assert_eq!(request.messages[4].content, "candidate");
    let format = request.response_format.expect("native response format");
    match format {
        CompletionResponseFormat::JsonSchema(format) => {
            assert_eq!(format.name, "response_schema");
            assert!(format.is_strict());
            assert_eq!(format.schema["required"], serde_json::json!(["items"]));
        }
        CompletionResponseFormat::JsonObject => panic!("expected schema format"),
    }
}

#[tokio::test]
async fn rejects_mismatched_gateway_route_evidence_before_using_output() {
    let context = test_run_context("system-inference-route-mismatch").await;
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::assistant_reply("must not be accepted")
            .with_effective_fallback_index(1),
    ));
    let port = ModelGatewayBackedSystemInferencePort::new(gateway, context);

    let error = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect_err("mismatched route evidence must fail closed");

    assert_eq!(
        error,
        SystemInferenceError::Failed {
            safe_summary: safe("system inference model route evidence is invalid"),
        }
    );
}

#[tokio::test]
async fn guarded_system_inference_policy_denial_skips_gateway_dispatch() {
    let context = test_run_context("system-inference-policy-denied").await;
    let direct: Arc<dyn SystemInferencePort> = Arc::new(
        ModelGatewayBackedSystemInferencePort::new(Arc::new(PanicGateway), context.clone()),
    );
    let port = GuardedSystemInferencePort::new(
        direct,
        context,
        Arc::new(NoOpBudgetAccountant),
        Arc::new(DenySystemInferencePolicyGuard),
    );

    let error = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect_err("policy denial should reject system inference");

    assert!(matches!(error, SystemInferenceError::Failed { .. }));
}

#[tokio::test]
async fn guarded_system_inference_budget_denial_skips_gateway_dispatch() {
    let context = test_run_context("system-inference-budget-denied").await;
    let direct: Arc<dyn SystemInferencePort> = Arc::new(
        ModelGatewayBackedSystemInferencePort::new(Arc::new(PanicGateway), context.clone()),
    );
    let port = GuardedSystemInferencePort::new(
        direct,
        context,
        Arc::new(RejectingBudgetAccountant),
        Arc::new(NoOpPolicyGuard),
    );

    let error = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect_err("budget denial should reject system inference");

    assert!(matches!(error, SystemInferenceError::Failed { .. }));
}

#[tokio::test]
async fn guarded_system_inference_records_budget_around_gateway_dispatch() {
    let context = test_run_context("system-inference-budget-recorded").await;
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::assistant_reply("summary"),
    ));
    let direct: Arc<dyn SystemInferencePort> = Arc::new(
        ModelGatewayBackedSystemInferencePort::new(gateway.clone(), context.clone()),
    );
    let accountant = Arc::new(RecordingBudgetAccountant::default());
    let port = GuardedSystemInferencePort::new(
        direct,
        context,
        accountant.clone(),
        Arc::new(NoOpPolicyGuard),
    );

    let response = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect("system inference succeeds");

    assert_eq!(response.output_text, "summary");
    assert!(gateway.request_was_recorded());
    assert!(*accountant.pre_called.lock().expect("lock"));
    let outcomes = accountant.post_outcomes.lock().expect("lock");
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], ModelWorkOutcome::Success(_)));
    assert_eq!(accountant.release_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn guarded_system_inference_reconciles_when_outer_future_is_cancelled() {
    let context = test_run_context("system-inference-outer-cancel").await;
    let started = Arc::new(Notify::new());
    let direct: Arc<dyn SystemInferencePort> = Arc::new(PendingInferencePort {
        started: Arc::clone(&started),
    });
    let accountant = Arc::new(RecordingBudgetAccountant::default());
    let port = Arc::new(GuardedSystemInferencePort::new(
        direct,
        context,
        accountant.clone(),
        Arc::new(NoOpPolicyGuard),
    ));
    let task = tokio::spawn({
        let port = Arc::clone(&port);
        async move {
            port.call_system_inference(system_request("transcript"))
                .await
        }
    });

    started.notified().await;
    task.abort();

    let task_error = task.await.expect_err("outer task should be cancelled");
    assert!(task_error.is_cancelled());

    let outcomes = accountant.post_outcomes.lock().expect("lock");
    assert!(outcomes.is_empty());
    assert_eq!(accountant.release_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guarded_system_inference_maps_panic_and_releases_reservation() {
    let context = test_run_context("system-inference-panic").await;
    let direct: Arc<dyn SystemInferencePort> = Arc::new(
        ModelGatewayBackedSystemInferencePort::new(Arc::new(PanicGateway), context.clone()),
    );
    let accountant = Arc::new(RecordingBudgetAccountant::default());
    let port = GuardedSystemInferencePort::new(
        direct,
        context,
        accountant.clone(),
        Arc::new(NoOpPolicyGuard),
    );

    let error = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect_err("gateway panic should become a safe inference failure");

    assert!(matches!(error, SystemInferenceError::Failed { .. }));
    assert!(accountant.post_outcomes.lock().expect("lock").is_empty());
    assert_eq!(accountant.release_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guarded_system_inference_releases_reservation_when_post_accounting_fails() {
    let context = test_run_context("system-inference-post-failure").await;
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::assistant_reply("summary"),
    ));
    let direct: Arc<dyn SystemInferencePort> = Arc::new(
        ModelGatewayBackedSystemInferencePort::new(gateway, context.clone()),
    );
    let accountant = Arc::new(RecordingBudgetAccountant {
        fail_post: true,
        ..Default::default()
    });
    let port = GuardedSystemInferencePort::new(
        direct,
        context,
        accountant.clone(),
        Arc::new(NoOpPolicyGuard),
    );

    let error = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect_err("post-accounting failure should fail inference");

    assert!(matches!(error, SystemInferenceError::Failed { .. }));
    assert!(accountant.post_outcomes.lock().expect("lock").is_empty());
    assert_eq!(accountant.release_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guarded_system_inference_maps_post_accounting_panic_and_releases_reservation() {
    let context = test_run_context("system-inference-post-panic").await;
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::assistant_reply("summary"),
    ));
    let direct: Arc<dyn SystemInferencePort> = Arc::new(
        ModelGatewayBackedSystemInferencePort::new(gateway, context.clone()),
    );
    let accountant = Arc::new(RecordingBudgetAccountant {
        panic_post: true,
        ..Default::default()
    });
    let port = GuardedSystemInferencePort::new(
        direct,
        context,
        accountant.clone(),
        Arc::new(NoOpPolicyGuard),
    );

    let error = port
        .call_system_inference(system_request("transcript"))
        .await
        .expect_err("post-accounting panic should become a safe inference failure");

    assert!(matches!(error, SystemInferenceError::Failed { .. }));
    assert!(accountant.post_outcomes.lock().expect("lock").is_empty());
    assert_eq!(accountant.release_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejects_gateway_capability_calls() {
    let context = test_run_context("system-inference-capability-calls").await;
    let gateway = Arc::new(RecordingGateway::new(
        crate::HostManagedModelResponse::capability_calls(Vec::new(), ""),
    ));
    let port = ModelGatewayBackedSystemInferencePort::new(gateway, context);

    let error = port
        .call_system_inference(SystemInferenceRequest {
            task_id: SystemInferenceTaskId::new(),
            identity: SystemInferenceIdentity {
                task_kind: SystemTaskKind::Compaction,
                prompt_source: SystemPromptSource::Static {
                    prompt_id: "test".to_string().try_into().unwrap(),
                },
                system_prompt: "summarize".to_string(),
            },
            input_text: "transcript".to_string(),
            context_messages: Vec::new(),
            max_input_tokens: 100,
            deadline_ms: 100,
            output_contract: None,
        })
        .await
        .expect_err("capability calls are invalid for system inference");

    assert!(matches!(error, SystemInferenceError::Failed { .. }));
}

#[tokio::test]
async fn oversized_input_fails_before_gateway_dispatch() {
    let context = test_run_context("system-inference-oversized").await;
    let port = ModelGatewayBackedSystemInferencePort::new(Arc::new(PanicGateway), context);

    let error = port
        .call_system_inference(SystemInferenceRequest {
            task_id: SystemInferenceTaskId::new(),
            identity: SystemInferenceIdentity {
                task_kind: SystemTaskKind::Compaction,
                prompt_source: SystemPromptSource::Static {
                    prompt_id: "test".to_string().try_into().unwrap(),
                },
                system_prompt: "summarize".to_string(),
            },
            input_text: "abcde".to_string(),
            context_messages: Vec::new(),
            max_input_tokens: 1,
            deadline_ms: 100,
            output_contract: None,
        })
        .await
        .expect_err("input should exceed token preflight");

    assert_eq!(error, SystemInferenceError::InputTooLarge);
}

#[tokio::test]
async fn timeout_returns_timeout_error() {
    let context = test_run_context("system-inference-timeout").await;
    let port = ModelGatewayBackedSystemInferencePort::new(
        Arc::new(SlowGateway {
            delay: std::time::Duration::from_millis(25),
            progress_requests: AtomicUsize::new(0),
        }),
        context,
    );

    let error = port
        .call_system_inference(SystemInferenceRequest {
            task_id: SystemInferenceTaskId::new(),
            identity: SystemInferenceIdentity {
                task_kind: SystemTaskKind::Compaction,
                prompt_source: SystemPromptSource::Static {
                    prompt_id: "test".to_string().try_into().unwrap(),
                },
                system_prompt: "summarize".to_string(),
            },
            input_text: "transcript".to_string(),
            context_messages: Vec::new(),
            max_input_tokens: 100,
            deadline_ms: 1,
            output_contract: None,
        })
        .await
        .expect_err("slow gateway should hit system inference timeout");

    assert_eq!(error, SystemInferenceError::Timeout);
}

#[tokio::test]
async fn structured_finalization_timeout_uses_progress_transport() {
    let context = test_run_context("system-inference-structured-timeout").await;
    let gateway = Arc::new(SlowGateway {
        delay: std::time::Duration::from_millis(25),
        progress_requests: AtomicUsize::new(0),
    });
    let port = ModelGatewayBackedSystemInferencePort::new(gateway.clone(), context);
    let mut request = system_request("transcript");
    request.identity.task_kind = SystemTaskKind::StructuredOutputFinalization;
    request.deadline_ms = 1;
    request.output_contract = Some(OutputContract::JsonObject);

    let error = port
        .call_system_inference(request)
        .await
        .expect_err("structured finalization should hit its deadline");

    assert_eq!(error, SystemInferenceError::Timeout);
    assert_eq!(gateway.progress_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_gateway_error_maps_to_cancelled() {
    let context = test_run_context("system-inference-cancelled").await;
    let gateway = Arc::new(RecordingGateway::with_result(Err(
        crate::HostManagedModelError::new(crate::HostManagedModelErrorKind::Cancelled, "cancelled"),
    )));
    let port = ModelGatewayBackedSystemInferencePort::new(gateway, context);

    let error = port
        .call_system_inference(SystemInferenceRequest {
            task_id: SystemInferenceTaskId::new(),
            identity: SystemInferenceIdentity {
                task_kind: SystemTaskKind::Compaction,
                prompt_source: SystemPromptSource::Static {
                    prompt_id: "test".to_string().try_into().unwrap(),
                },
                system_prompt: "summarize".to_string(),
            },
            input_text: "transcript".to_string(),
            context_messages: Vec::new(),
            max_input_tokens: 100,
            deadline_ms: 100,
            output_contract: None,
        })
        .await
        .expect_err("cancelled gateway error should be preserved");

    assert_eq!(error, SystemInferenceError::Cancelled);
}

async fn test_run_context(label: &str) -> LoopRunContext {
    let tenant_id = TenantId::new(format!("tenant-{label}")).unwrap();
    let agent_id = AgentId::new(format!("agent-{label}")).unwrap();
    let project_id = ProjectId::new(format!("project-{label}")).unwrap();
    let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
    let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
}
