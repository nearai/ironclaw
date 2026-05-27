use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopMessageRef,
    run_profile::{
        AgentLoopHostErrorKind, InstructionBundleMaterializedMessage,
        InstructionMaterializationStore, LoopModelMessage, LoopModelPort, LoopModelRequest,
        LoopProgressEvent, LoopProgressPort, LoopRunContext, LoopSafeSummary, ParentLoopOutput,
        SystemInferenceError, SystemInferencePort, SystemInferenceRequest, SystemInferenceResponse,
    },
};

use crate::token_estimator::estimate_tokens_from_chars;

#[derive(Clone)]
pub struct ModelGatewayBackedSystemInferencePort {
    model: Arc<dyn LoopModelPort>,
    progress: Arc<dyn LoopProgressPort>,
    run_context: LoopRunContext,
    materialization_store: Arc<dyn InstructionMaterializationStore>,
}

impl ModelGatewayBackedSystemInferencePort {
    pub fn new(
        model: Arc<dyn LoopModelPort>,
        progress: Arc<dyn LoopProgressPort>,
        run_context: LoopRunContext,
        materialization_store: Arc<dyn InstructionMaterializationStore>,
    ) -> Self {
        Self {
            model,
            progress,
            run_context,
            materialization_store,
        }
    }
}

#[async_trait]
impl SystemInferencePort for ModelGatewayBackedSystemInferencePort {
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        if estimate_tokens_from_chars(&request.input_text).as_u64() > request.max_input_tokens {
            return Err(SystemInferenceError::InputTooLarge);
        }

        if let Err(error) = self
            .progress
            .emit_loop_progress(LoopProgressEvent::CompactionStarted {
                task_id: request.task_id,
                initiator: ironclaw_turns::run_profile::CompactionInitiator::Auto,
            })
            .await
        {
            tracing::debug!(safe_error = %error, "system inference progress start failed");
        }

        let started = Instant::now();
        let system_ref = system_inference_ref(request.task_id.as_uuid(), "system-prompt")?;
        let input_ref = system_inference_ref(request.task_id.as_uuid(), "input")?;
        self.materialization_store
            .put_materialized_messages(
                &self.run_context,
                vec![
                    InstructionBundleMaterializedMessage {
                        role: "system".to_string(),
                        content_ref: system_ref.clone(),
                        safe_content: request.identity.system_prompt.clone(),
                    },
                    InstructionBundleMaterializedMessage {
                        role: "user".to_string(),
                        content_ref: input_ref.clone(),
                        safe_content: request.input_text.clone(),
                    },
                ],
            )
            .map_err(|_| SystemInferenceError::Failed {
                safe_summary: safe("system inference materialization failed"),
            })?;
        let model_request = LoopModelRequest {
            messages: vec![
                LoopModelMessage {
                    role: "system".to_string(),
                    content_ref: system_ref,
                },
                LoopModelMessage {
                    role: "user".to_string(),
                    content_ref: input_ref,
                },
            ],
            surface_version: None,
            model_preference: None,
            capability_view: Some(ironclaw_turns::run_profile::LoopModelCapabilityView {
                visible_capability_ids: Vec::new(),
            }),
        };

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(request.deadline_ms),
            self.model.stream_model(model_request),
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

fn map_model_error(kind: AgentLoopHostErrorKind) -> SystemInferenceError {
    let safe_summary = match kind {
        AgentLoopHostErrorKind::Cancelled => return SystemInferenceError::Cancelled,
        AgentLoopHostErrorKind::BudgetExceeded => "system inference budget exceeded",
        AgentLoopHostErrorKind::Unavailable => "system inference unavailable",
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
) -> Result<LoopMessageRef, SystemInferenceError> {
    LoopMessageRef::new(format!("msg:system-inference.{label}.{task_id}")).map_err(|_| {
        SystemInferenceError::Failed {
            safe_summary: safe("system inference ref invalid"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{
            InMemoryInstructionMaterializationStore, InMemoryRunProfileResolver,
            SystemInferenceIdentity, SystemInferenceTaskId, SystemPromptSource, SystemTaskKind,
        },
    };

    struct PanicModel;

    #[async_trait]
    impl LoopModelPort for PanicModel {
        async fn stream_model(
            &self,
            _request: LoopModelRequest,
        ) -> Result<
            ironclaw_turns::run_profile::LoopModelResponse,
            ironclaw_turns::run_profile::AgentLoopHostError,
        > {
            panic!("oversized inference input must fail before model dispatch")
        }
    }

    struct PanicProgress;

    #[async_trait]
    impl LoopProgressPort for PanicProgress {
        async fn emit_loop_progress(
            &self,
            _event: LoopProgressEvent,
        ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
            panic!("oversized inference input must fail before progress emission")
        }
    }

    #[tokio::test]
    async fn oversized_input_fails_before_materialization_or_model_call() {
        let context = test_run_context("system-inference-oversized").await;
        let store = Arc::new(InMemoryInstructionMaterializationStore::default());
        let port = ModelGatewayBackedSystemInferencePort::new(
            Arc::new(PanicModel),
            Arc::new(PanicProgress),
            context,
            store,
        );

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
