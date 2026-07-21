use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ProviderToolName};
use ironclaw_llm::{
    CompletionRequest, CompletionResponse, CompletionStreamSink, FinishReason, LlmError,
    LlmProvider, ToolCompletionRequest, ToolCompletionResponse,
};
use ironclaw_loop_host::{
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessage,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelStreamSink,
};
use ironclaw_runner::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind,
    CapabilitySurfaceVersion, LoopCapabilityPort, ModelProfileId, ProviderToolCall,
    ProviderToolCallCapabilityIds, ProviderToolDefinition, RegisterProviderToolCallRequest,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use ironclaw_turns::{LoopMessageRef, TurnId, TurnRunId};
use rust_decimal::Decimal;

const MODEL_PROFILE: &str = "interactive_model";

#[derive(Default)]
struct RecordingStreamSink {
    updates: Mutex<Vec<String>>,
}

#[async_trait]
impl HostManagedModelStreamSink for RecordingStreamSink {
    async fn safe_text_update(&self, safe_text: String) {
        self.updates.lock().unwrap().push(safe_text);
    }
}

struct BarrierProvider {
    deltas: Vec<String>,
    fail_stream: bool,
    finish_reason: FinishReason,
}

impl BarrierProvider {
    fn streaming_error(deltas: Vec<String>) -> Self {
        Self {
            deltas,
            fail_stream: true,
            finish_reason: FinishReason::Stop,
        }
    }

    fn invalid_terminal(deltas: Vec<String>) -> Self {
        Self {
            deltas,
            fail_stream: false,
            finish_reason: FinishReason::Unknown,
        }
    }

    async fn emit_deltas(&self, sink: Arc<dyn CompletionStreamSink>) {
        for delta in &self.deltas {
            sink.text_delta(delta.clone()).await;
        }
    }
}

#[async_trait]
impl LlmProvider for BarrierProvider {
    fn model_name(&self) -> &str {
        "barrier-test-provider"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(unexpected_non_streaming_error())
    }

    async fn complete_streaming(
        &self,
        _request: CompletionRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<CompletionResponse, LlmError> {
        self.emit_deltas(sink).await;
        if self.fail_stream {
            return Err(LlmError::RateLimited {
                provider: self.model_name().to_string(),
                retry_after: None,
            });
        }
        Ok(CompletionResponse {
            content: "visible response".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: self.finish_reason,
            reasoning: None,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(unexpected_non_streaming_error())
    }

    async fn complete_with_tools_streaming(
        &self,
        _request: ToolCompletionRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.emit_deltas(sink).await;
        Ok(ToolCompletionResponse {
            content: Some("visible response".to_string()),
            tool_calls: Vec::new(),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: self.finish_reason,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: None,
            reasoning_details: None,
        })
    }
}

fn unexpected_non_streaming_error() -> LlmError {
    LlmError::RequestFailed {
        provider: "barrier-test-provider".to_string(),
        reason: "non-streaming completion is not expected".to_string(),
    }
}

struct ToolSurface {
    registrations: AtomicUsize,
}

impl ToolSurface {
    fn new() -> Self {
        Self {
            registrations: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LoopCapabilityPort for ToolSurface {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(vec![ProviderToolDefinition {
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            name: ProviderToolName::new("demo__echo").unwrap(),
            description: "Echo input".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }])
    }

    fn provider_tool_call_capability_ids(
        &self,
        _tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        Err(unexpected_capability_error())
    }

    fn validate_provider_tool_call(
        &self,
        _tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        Err(unexpected_capability_error())
    }

    async fn register_provider_tool_call(
        &self,
        _request: RegisterProviderToolCallRequest,
    ) -> Result<ironclaw_turns::run_profile::CapabilityCallCandidate, AgentLoopHostError> {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        Err(unexpected_capability_error())
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            callable_capability_ids: None,
            version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
            descriptors: Vec::new(),
        })
    }

    async fn invoke_capability(
        &self,
        _request: ironclaw_turns::run_profile::CapabilityInvocation,
    ) -> Result<ironclaw_host_api::Resolution, AgentLoopHostError> {
        Err(unexpected_capability_error())
    }

    async fn invoke_capability_batch(
        &self,
        _request: ironclaw_turns::run_profile::CapabilityBatchInvocation,
    ) -> Result<ironclaw_host_api::ResolutionBatch, AgentLoopHostError> {
        Err(unexpected_capability_error())
    }
}

fn unexpected_capability_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Internal,
        "capability interaction is not expected",
    )
}

#[tokio::test]
async fn stream_error_without_text_remains_retry_classifiable() {
    let sink = Arc::new(RecordingStreamSink::default());
    let error = gateway(BarrierProvider::streaming_error(Vec::new()))
        .stream_model_with_progress(model_request(), sink.clone())
        .await
        .unwrap_err();

    assert_eq!(error.reason_kind, None);
    assert!(sink.updates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn text_post_stream_validation_failure_is_not_retryable() {
    let sink = Arc::new(RecordingStreamSink::default());
    let error = gateway(BarrierProvider::invalid_terminal(vec![
        "visible response".to_string(),
    ]))
    .stream_model_with_progress(model_request(), sink.clone())
    .await
    .unwrap_err();

    assert_partial_output_error(&error);
    assert_eq!(
        sink.updates.lock().unwrap().as_slice(),
        ["visible response"]
    );
}

#[tokio::test]
async fn tool_post_stream_validation_failure_cannot_register_a_call() {
    let sink = Arc::new(RecordingStreamSink::default());
    let capabilities = Arc::new(ToolSurface::new());
    let error = gateway(BarrierProvider::invalid_terminal(vec![
        "visible response".to_string(),
    ]))
    .stream_model_with_capabilities_and_progress(
        model_request(),
        capabilities.clone(),
        sink.clone(),
    )
    .await
    .unwrap_err();

    assert_partial_output_error(&error);
    assert_eq!(
        sink.updates.lock().unwrap().as_slice(),
        ["visible response"]
    );
    assert_eq!(capabilities.registrations.load(Ordering::SeqCst), 0);
}

fn assert_partial_output_error(error: &ironclaw_loop_host::HostManagedModelError) {
    assert_eq!(error.kind, HostManagedModelErrorKind::Unavailable);
    assert_eq!(
        error.reason_kind,
        Some(AgentLoopHostErrorReasonKind::ModelPartialOutputVisible)
    );
}

fn gateway(provider: BarrierProvider) -> LlmProviderModelGateway<BarrierProvider> {
    LlmProviderModelGateway::with_provider_identity(
        "barrier-test-provider",
        Arc::new(provider),
        LlmModelProfilePolicy::new().allow_model_profile(
            ModelProfileId::new(MODEL_PROFILE).unwrap(),
            Some("barrier-test-model".to_string()),
        ),
    )
}

fn model_request() -> HostManagedModelRequest {
    HostManagedModelRequest {
        model_profile_id: ModelProfileId::new(MODEL_PROFILE).unwrap(),
        messages: vec![HostManagedModelMessage {
            role: HostManagedModelMessageRole::User,
            content: "hello model".to_string(),
            content_ref: LoopMessageRef::new("msg:22222222-2222-2222-2222-222222222222").unwrap(),
            tool_result_provider_call: None,
            tool_result_content: None,
            image_parts: Vec::new(),
        }],
        surface_version: None,
        resolved_model_route: None,
        run_id: TurnRunId::new(),
        turn_id: TurnId::new(),
    }
}
