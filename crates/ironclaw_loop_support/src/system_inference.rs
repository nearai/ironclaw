use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    LoopRunContext, LoopSafeSummary, ParentLoopOutput, SystemInferenceError, SystemInferencePort,
    SystemInferenceRequest, SystemInferenceResponse,
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
                },
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::User,
                    content: request.input_text.clone(),
                    content_ref: input_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
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
            InMemoryRunProfileResolver, SystemInferenceIdentity, SystemInferenceTaskId,
            SystemPromptSource, SystemTaskKind,
        },
    };
    use std::sync::Mutex;

    struct RecordingGateway {
        request: Mutex<Option<HostManagedModelRequest>>,
        response: crate::HostManagedModelResponse,
    }

    impl RecordingGateway {
        fn new(response: crate::HostManagedModelResponse) -> Self {
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
    }

    #[async_trait]
    impl HostManagedModelGateway for RecordingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<crate::HostManagedModelResponse, crate::HostManagedModelError> {
            *self.request.lock().expect("lock") = Some(request);
            Ok(self.response.clone())
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
                        prompt_id: "test".to_string(),
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
                        prompt_id: "test".to_string(),
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
                        prompt_id: "test".to_string(),
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
