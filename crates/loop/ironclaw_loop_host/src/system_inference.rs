use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::FutureExt;
use ironclaw_loop_contracts::{
    AgentLoopHostErrorKind, LoopModelBudgetAccountant, LoopModelGatewayError, LoopModelPolicyGuard,
    LoopRunContext, LoopSafeSummary, ModelWorkOutcome, ModelWorkRequest, ParentLoopOutput,
    SystemInferenceContextRole, SystemInferenceError, SystemInferencePort, SystemInferenceRequest,
    SystemInferenceResponse,
};

use crate::{
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessage,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelStreamSink,
    token_estimator::estimate_tokens_from_chars,
};
use ironclaw_host_api::output::OutputContract;
use ironclaw_llm::{CompletionResponseFormat, JsonSchemaResponseFormat};

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

/// Releases a successful pre-model-work reservation if the caller cancels
/// before post-model accounting completes.
struct SystemInferenceReservationReleaseGuard<'a> {
    accountant: &'a dyn LoopModelBudgetAccountant,
    context: &'a LoopRunContext,
    armed: bool,
}

impl<'a> SystemInferenceReservationReleaseGuard<'a> {
    fn new(accountant: &'a dyn LoopModelBudgetAccountant, context: &'a LoopRunContext) -> Self {
        Self {
            accountant,
            context,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SystemInferenceReservationReleaseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.accountant.release_in_flight(self.context);
        }
    }
}

struct DiscardSystemInferenceProgress;

#[async_trait]
impl HostManagedModelStreamSink for DiscardSystemInferenceProgress {
    fn accepts_safe_text_updates(&self) -> bool {
        false
    }

    async fn safe_text_update(&self, _safe_text: String) {}
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

        let mut release_guard = SystemInferenceReservationReleaseGuard::new(
            self.accountant.as_ref(),
            &self.run_context,
        );
        let result = AssertUnwindSafe(self.inner.call_system_inference(request))
            .catch_unwind()
            .await
            .map_err(|panic| map_system_inference_panic(panic, "model inference"))?;
        let outcome = ModelWorkOutcome::from_system_inference_result(&result);
        let post_model_work = AssertUnwindSafe(self.accountant.post_model_work(
            &self.run_context,
            &work_request,
            outcome,
        ))
        .catch_unwind()
        .await
        .map_err(|panic| map_system_inference_panic(panic, "post-model accounting"))?;
        if let Err(error) = post_model_work {
            return Err(map_gateway_error(error));
        }
        release_guard.disarm();
        result
    }
}

fn map_system_inference_panic(
    panic: Box<dyn std::any::Any + Send>,
    phase: &'static str,
) -> SystemInferenceError {
    let (panic_payload_kind, panic_payload_length) =
        if let Some(message) = panic.downcast_ref::<&str>() {
            ("static-string", message.len())
        } else if let Some(message) = panic.downcast_ref::<String>() {
            ("owned-string", message.len())
        } else {
            ("non-string", 0)
        };
    tracing::debug!(
        panic_payload_kind,
        panic_payload_length,
        phase,
        "system inference worker panicked"
    );
    SystemInferenceError::Failed {
        safe_summary: safe("system inference task failed"),
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
        let context_tokens = request
            .context_messages
            .iter()
            .map(|message| estimate_tokens_from_chars(&message.content).as_u64())
            .sum::<u64>();
        let input_tokens = context_tokens
            .saturating_add(estimate_tokens_from_chars(&request.identity.system_prompt).as_u64())
            .saturating_add(estimate_tokens_from_chars(&request.input_text).as_u64());
        if input_tokens > request.max_input_tokens {
            return Err(SystemInferenceError::InputTooLarge);
        }

        let started = Instant::now();
        let use_streaming_transport = matches!(
            request.identity.task_kind,
            ironclaw_loop_contracts::SystemTaskKind::StructuredOutputFinalization
        );
        let response_format = match (request.identity.task_kind, request.output_contract.as_ref()) {
            (
                ironclaw_loop_contracts::SystemTaskKind::StructuredOutputFinalization,
                Some(OutputContract::JsonSchema { name, schema }),
            ) => Some(CompletionResponseFormat::JsonSchema(
                JsonSchemaResponseFormat::strict(name.clone(), schema.clone()),
            )),
            (
                ironclaw_loop_contracts::SystemTaskKind::StructuredOutputFinalization,
                Some(OutputContract::JsonObject),
            ) => Some(CompletionResponseFormat::JsonObject),
            (
                ironclaw_loop_contracts::SystemTaskKind::StructuredOutputFinalization,
                Some(OutputContract::AssistantMessage) | None,
            ) => {
                return Err(SystemInferenceError::Failed {
                    safe_summary: safe("structured finalization requires a structured output"),
                });
            }
            (_, None) => None,
            (_, Some(_)) => {
                return Err(SystemInferenceError::Failed {
                    safe_summary: safe("output contract is only valid for structured finalization"),
                });
            }
        };
        let system_ref = system_inference_ref(request.task_id.as_uuid(), "system-prompt")?;
        let input_ref = system_inference_ref(request.task_id.as_uuid(), "input")?;
        let mut messages = vec![HostManagedModelMessage {
            role: HostManagedModelMessageRole::System,
            content: request.identity.system_prompt.clone(),
            content_ref: system_ref,
            tool_result_provider_call: None,
            tool_result_content: None,
            image_parts: Vec::new(),
        }];
        for (index, message) in request.context_messages.iter().enumerate() {
            let (role, content) = match message.role {
                SystemInferenceContextRole::System => {
                    (HostManagedModelMessageRole::System, message.content.clone())
                }
                SystemInferenceContextRole::User => {
                    (HostManagedModelMessageRole::User, message.content.clone())
                }
                SystemInferenceContextRole::Assistant => (
                    HostManagedModelMessageRole::Assistant,
                    message.content.clone(),
                ),
                // A system inference deliberately has no provider tool-call
                // round to pair this historical result with. Keep the
                // canonical tool observation as untrusted, role-labelled
                // user context instead of emitting an invalid provider
                // ToolResult message without a call id.
                SystemInferenceContextRole::Tool => (
                    HostManagedModelMessageRole::User,
                    format!("[Untrusted tool result context]\n{}", message.content),
                ),
            };
            messages.push(HostManagedModelMessage {
                role,
                content,
                content_ref: system_inference_ref(
                    request.task_id.as_uuid(),
                    &format!("context-{index}"),
                )?,
                tool_result_provider_call: None,
                tool_result_content: None,
                image_parts: Vec::new(),
            });
        }
        if !request.input_text.is_empty() {
            messages.push(HostManagedModelMessage {
                role: HostManagedModelMessageRole::User,
                content: request.input_text.clone(),
                content_ref: input_ref,
                tool_result_provider_call: None,
                tool_result_content: None,
                image_parts: Vec::new(),
            });
        }
        let model_request = HostManagedModelRequest {
            model_profile_id: self
                .run_context
                .resolved_run_profile
                .model_profile_id
                .clone(),
            messages,
            surface_version: None,
            fallback_index: 0,
            resolved_model_route: self.run_context.resolved_model_route.clone(),
            run_id: self.run_context.run_id,
            turn_id: self.run_context.turn_id,
            tool_choice: None,
            response_format,
        };
        let requested_fallback_index = model_request.fallback_index;

        let model_call = if use_streaming_transport {
            self.gateway
                .stream_model_with_progress(model_request, Arc::new(DiscardSystemInferenceProgress))
                .boxed()
        } else {
            self.gateway.stream_model(model_request).boxed()
        };
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(request.deadline_ms),
            model_call,
        )
        .await
        .map_err(|_| SystemInferenceError::Timeout)?
        .map_err(|error| map_model_error(error.kind))?;

        if response.effective_fallback_index != Some(requested_fallback_index) {
            return Err(SystemInferenceError::Failed {
                safe_summary: safe("system inference model route evidence is invalid"),
            });
        }

        let usage = response.usage;
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
            usage,
        })
    }
}

fn map_model_error(kind: HostManagedModelErrorKind) -> SystemInferenceError {
    let safe_summary = match kind {
        HostManagedModelErrorKind::Cancelled => return SystemInferenceError::Cancelled,
        HostManagedModelErrorKind::BudgetExceeded
        | HostManagedModelErrorKind::SpendBudgetExceeded => "system inference budget exceeded",
        HostManagedModelErrorKind::ContextOverflow => "system inference context exceeded",
        HostManagedModelErrorKind::OutputTruncated => "system inference output truncated",
        HostManagedModelErrorKind::BudgetAccountingFailed => {
            "system inference resource accounting unavailable"
        }
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
        AgentLoopHostErrorKind::BudgetExceeded
        | AgentLoopHostErrorKind::SpendBudgetExceeded
        | AgentLoopHostErrorKind::BudgetApprovalRequired => SystemInferenceError::Failed {
            safe_summary: safe("system inference budget exceeded"),
        },
        AgentLoopHostErrorKind::ContextOverflow => SystemInferenceError::Failed {
            safe_summary: safe("system inference context exceeded"),
        },
        AgentLoopHostErrorKind::OutputTruncated => SystemInferenceError::Failed {
            safe_summary: safe("system inference output truncated"),
        },
        AgentLoopHostErrorKind::BudgetAccountingFailed => SystemInferenceError::Failed {
            safe_summary: safe("system inference resource accounting unavailable"),
        },
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
    label: &str,
) -> Result<ironclaw_turns::LoopMessageRef, SystemInferenceError> {
    ironclaw_turns::LoopMessageRef::new(format!("msg:system-inference.{label}.{task_id}")).map_err(
        |_| SystemInferenceError::Failed {
            safe_summary: safe("system inference ref invalid"),
        },
    )
}

#[cfg(test)]
#[path = "system_inference/tests.rs"]
mod tests;
