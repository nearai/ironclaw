use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostErrorKind, LoopModelBudgetAccountant, LoopModelGatewayError, LoopModelPolicyGuard,
    LoopRunContext, LoopSafeSummary, ModelWorkOutcome, ModelWorkRequest, ParentLoopOutput,
    SystemInferenceError, SystemInferencePort, SystemInferenceRequest, SystemInferenceResponse,
};

use crate::{
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessage,
    HostManagedModelMessageRole, HostManagedModelRequest,
    token_estimator::estimate_tokens_from_chars,
};

#[derive(Clone)]
pub struct ModelGatewayBackedSystemInferencePort<G>
where
    G: HostManagedModelGateway + ?Sized,
{
    gateway: Arc<G>,
    run_context: LoopRunContext,
}

impl<G> ModelGatewayBackedSystemInferencePort<G>
where
    G: HostManagedModelGateway + ?Sized,
{
    pub fn new(gateway: Arc<G>, run_context: LoopRunContext) -> Self {
        Self {
            gateway,
            run_context,
        }
    }
}

#[derive(Clone)]
pub struct GuardedSystemInferencePort {
    inner: Arc<dyn SystemInferencePort>,
    run_context: LoopRunContext,
    accountant: Arc<dyn LoopModelBudgetAccountant>,
    policy_guard: Arc<dyn LoopModelPolicyGuard>,
}

impl GuardedSystemInferencePort {
    pub fn new(
        inner: Arc<dyn SystemInferencePort>,
        run_context: LoopRunContext,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
        policy_guard: Arc<dyn LoopModelPolicyGuard>,
    ) -> Self {
        Self {
            inner,
            run_context,
            accountant,
            policy_guard,
        }
    }
}

#[async_trait]
impl SystemInferencePort for GuardedSystemInferencePort {
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        let work_request = ModelWorkRequest::for_system_inference(&self.run_context, &request);
        if let Err(error) = self
            .policy_guard
            .check_model_work_policy(&self.run_context, &work_request)
            .await
        {
            return Err(map_gateway_error(error));
        }

        if let Err(error) = self
            .accountant
            .pre_model_work(&self.run_context, &work_request)
            .await
        {
            return Err(map_gateway_error(error));
        }

        let inner = Arc::clone(&self.inner);
        let accountant = Arc::clone(&self.accountant);
        let run_context = self.run_context.clone();
        let worker_request = work_request.clone();
        tokio::spawn(async move {
            let result = inner.call_system_inference(request).await;
            let outcome = ModelWorkOutcome::from_system_inference_result(&result);
            if let Err(error) = accountant
                .post_model_work(&run_context, &worker_request, outcome)
                .await
            {
                return Err(map_gateway_error(error));
            }
            result
        })
        .await
        .map_err(|error| {
            tracing::debug!(
                error = %error,
                "system inference worker failed before post-model accounting completed"
            );
            SystemInferenceError::Failed {
                safe_summary: safe("system inference task failed"),
            }
        })?
    }
}

#[async_trait]
impl<G> SystemInferencePort for ModelGatewayBackedSystemInferencePort<G>
where
    G: HostManagedModelGateway + ?Sized,
{
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        if estimate_tokens_from_chars(&request.input_text).as_u64() > request.max_input_tokens {
            return Err(SystemInferenceError::InputTooLarge);
        }

        let started = Instant::now();
        let system_ref = system_inference_ref(request.task_id.as_uuid(), "system-prompt")?;
        let input_ref = system_inference_ref(request.task_id.as_uuid(), "input")?;
        let model_request = HostManagedModelRequest {
            model_profile_id: self
                .run_context
                .resolved_run_profile
                .model_profile_id
                .clone(),
            messages: vec![
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::System,
                    content: request.identity.system_prompt.clone(),
                    content_ref: system_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                    image_parts: Vec::new(),
                },
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::User,
                    content: request.input_text.clone(),
                    content_ref: input_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                    image_parts: Vec::new(),
                },
            ],
            surface_version: None,
            resolved_model_route: self.run_context.resolved_model_route.clone(),
            run_id: self.run_context.run_id,
            turn_id: self.run_context.turn_id,
        };

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(request.deadline_ms),
            self.gateway.stream_model(model_request),
        )
        .await
        .map_err(|_| SystemInferenceError::Timeout)?
        .map_err(|error| map_model_error(error.kind))?;

        let output_text = match response.output {
            ParentLoopOutput::AssistantReply(reply) => reply.content,
            ParentLoopOutput::CapabilityCalls(_) => {
                return Err(SystemInferenceError::Failed {
                    safe_summary: safe("system inference returned capability calls"),
                });
            }
        };

        Ok(SystemInferenceResponse {
            task_id: request.task_id,
            output_text,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        })
    }
}

fn map_model_error(kind: HostManagedModelErrorKind) -> SystemInferenceError {
    let safe_summary = match kind {
        HostManagedModelErrorKind::Cancelled => return SystemInferenceError::Cancelled,
        HostManagedModelErrorKind::BudgetExceeded => "system inference budget exceeded",
        HostManagedModelErrorKind::Unavailable => "system inference unavailable",
        HostManagedModelErrorKind::CredentialUnavailable => {
            "system inference credential unavailable"
        }
        HostManagedModelErrorKind::PolicyDenied => "system inference policy denied",
        HostManagedModelErrorKind::ConfigurationError => "system inference configuration error",
        _ => "system inference failed",
    };
    SystemInferenceError::Failed {
        safe_summary: safe(safe_summary),
    }
}

fn map_gateway_error(error: LoopModelGatewayError) -> SystemInferenceError {
    match error.kind {
        AgentLoopHostErrorKind::Cancelled => SystemInferenceError::Cancelled,
        AgentLoopHostErrorKind::BudgetExceeded | AgentLoopHostErrorKind::BudgetApprovalRequired => {
            SystemInferenceError::Failed {
                safe_summary: safe("system inference budget exceeded"),
            }
        }
        AgentLoopHostErrorKind::PolicyDenied => SystemInferenceError::Failed {
            safe_summary: safe("system inference policy denied"),
        },
        AgentLoopHostErrorKind::CredentialUnavailable => SystemInferenceError::Failed {
            safe_summary: safe("system inference credential unavailable"),
        },
        AgentLoopHostErrorKind::Unavailable => SystemInferenceError::Failed {
            safe_summary: safe("system inference unavailable"),
        },
        _ => SystemInferenceError::Failed {
            safe_summary: error.safe_summary,
        },
    }
}

fn safe(value: &'static str) -> LoopSafeSummary {
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}

fn system_inference_ref(
    task_id: uuid::Uuid,
    label: &'static str,
) -> Result<ironclaw_turns::LoopMessageRef, SystemInferenceError> {
    ironclaw_turns::LoopMessageRef::new(format!("msg:system-inference.{label}.{task_id}")).map_err(
        |_| SystemInferenceError::Failed {
            safe_summary: safe("system inference ref invalid"),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostErrorKind, InMemoryRunProfileResolver, LoopModelBudgetAccountant,
            LoopModelGatewayError, LoopModelPolicyGuard, ModelWorkOutcome, ModelWorkRequest,
            NoOpBudgetAccountant, NoOpPolicyGuard, SystemInferenceIdentity, SystemInferenceTaskId,
            SystemPromptSource, SystemTaskKind,
        },
    };
    use std::sync::Mutex;
    use tokio::sync::Notify;

    struct RecordingGateway {
        request: Mutex<Option<HostManagedModelRequest>>,
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
    }

    struct SlowGateway {
        delay: std::time::Duration,
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
    }

    struct PanicGateway;

    #[async_trait]
    impl HostManagedModelGateway for PanicGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
            panic!("oversized inference input must fail before gateway dispatch")
        }
    }

    struct DelayedInferencePort {
        started: Arc<Notify>,
        delay: std::time::Duration,
    }

    #[async_trait]
    impl SystemInferencePort for DelayedInferencePort {
        async fn call_system_inference(
            &self,
            _request: SystemInferenceRequest,
        ) -> Result<SystemInferenceResponse, SystemInferenceError> {
            self.started.notify_one();
            tokio::time::sleep(self.delay).await;
            Err(SystemInferenceError::Timeout)
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
                ironclaw_turns::run_profile::ModelWorkKind::SystemInference { .. }
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
                ironclaw_turns::run_profile::ModelWorkKind::SystemInference { .. }
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
                ironclaw_turns::run_profile::ModelWorkKind::SystemInference { .. }
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
                ironclaw_turns::run_profile::ModelWorkKind::SystemInference { .. }
            ));
            self.post_outcomes.lock().expect("lock").push(outcome);
            Ok(())
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
            max_input_tokens: 100,
            deadline_ms: 100,
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
                max_input_tokens: 100,
                deadline_ms: 100,
            })
            .await
            .expect("system inference succeeds");

        assert_eq!(response.output_text, "summary");
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
    }

    #[tokio::test]
    async fn guarded_system_inference_reconciles_when_outer_future_is_cancelled() {
        let context = test_run_context("system-inference-outer-cancel").await;
        let started = Arc::new(Notify::new());
        let direct: Arc<dyn SystemInferencePort> = Arc::new(DelayedInferencePort {
            started: Arc::clone(&started),
            delay: std::time::Duration::from_millis(25),
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

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !accountant.post_outcomes.lock().expect("lock").is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("worker should reconcile after outer future cancellation");

        let outcomes = accountant.post_outcomes.lock().expect("lock");
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], ModelWorkOutcome::Failure(_)));
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
                max_input_tokens: 100,
                deadline_ms: 100,
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
                max_input_tokens: 1,
                deadline_ms: 100,
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
                max_input_tokens: 100,
                deadline_ms: 1,
            })
            .await
            .expect_err("slow gateway should hit system inference timeout");

        assert_eq!(error, SystemInferenceError::Timeout);
    }

    #[tokio::test]
    async fn cancelled_gateway_error_maps_to_cancelled() {
        let context = test_run_context("system-inference-cancelled").await;
        let gateway = Arc::new(RecordingGateway::with_result(Err(
            crate::HostManagedModelError::new(
                crate::HostManagedModelErrorKind::Cancelled,
                "cancelled",
            ),
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
                max_input_tokens: 100,
                deadline_ms: 100,
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
}
