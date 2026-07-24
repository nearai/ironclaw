use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ProviderToolName};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityCallCandidate, CapabilityInputRef, CapabilitySurfaceVersion,
    LoopCapabilityPort, ParentLoopOutput, ProviderToolCall, ProviderToolCallReplay,
    ProviderToolDefinition, RegisterProviderToolCallRequest, VisibleCapabilityRequest,
};
use thiserror::Error;

use crate::support::trace_llm::{
    ExpectedToolResult, LlmTrace, TraceResponse, TraceStep, TraceToolCall,
};

const TRACE_REPLAY_SURFACE_VERSION: &str = "trace_replay_v1";

#[derive(Debug, Error)]
pub enum RebornTraceReplayError {
    #[error("trace response variant cannot be replayed by the Reborn model gateway")]
    UnsupportedResponse,
    #[error("invalid trace capability surface version: {0}")]
    InvalidSurfaceVersion(String),
    #[error("invalid trace capability id for {name}: {reason}")]
    InvalidCapabilityId { name: String, reason: String },
    #[error("invalid trace provider tool name for {name}: {reason}")]
    InvalidProviderToolName { name: String, reason: String },
    #[error("invalid trace capability input ref for {id}: {reason}")]
    InvalidInputRef { id: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct RebornTraceReplayModelGateway {
    inner: Arc<Mutex<ReplayState>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum RebornModelReplayStep {
    Response {
        response: HostManagedModelResponse,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    AssertProviderToolsThenResponse {
        capability_ids: Vec<CapabilityId>,
        response: HostManagedModelResponse,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    AssertProviderToolsThenProviderToolCalls {
        capability_ids: Vec<CapabilityId>,
        calls: Vec<RebornScriptedProviderToolCall>,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    ResponseForRequest {
        request_contains: String,
        response: HostManagedModelResponse,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    DelayedResponse {
        response: HostManagedModelResponse,
        delay: Duration,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    DelayedResponseForRequest {
        request_contains: String,
        response: HostManagedModelResponse,
        delay: Duration,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    ProviderToolCalls {
        calls: Vec<RebornScriptedProviderToolCall>,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    ProviderToolCallsForRequest {
        request_contains: String,
        calls: Vec<RebornScriptedProviderToolCall>,
        expected_tool_results: Vec<ExpectedToolResult>,
    },
    /// Returns a model gateway error, simulating a provider failure (5xx,
    /// unavailable, rejected request, …). Used to exercise the no-borking
    /// failure path: the loop must surface a sanitized, retryable run failure
    /// rather than a borking executor error.
    ModelError {
        kind: HostManagedModelErrorKind,
        message: String,
    },
    /// Same as [`Self::ModelError`] but only matches requests whose messages
    /// contain `request_contains`, so a scripted run can fail one specific
    /// model call and answer others normally.
    ModelErrorForRequest {
        request_contains: String,
        kind: HostManagedModelErrorKind,
        message: String,
    },
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RebornScriptedProviderToolCall {
    pub capability_id: CapabilityId,
    pub call_id: String,
    pub arguments: serde_json::Value,
}

impl RebornScriptedProviderToolCall {
    #[allow(dead_code)]
    pub fn new(
        capability_id: CapabilityId,
        call_id: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            capability_id,
            call_id: call_id.into(),
            arguments,
        }
    }
}

#[derive(Debug)]
struct ReplayState {
    steps: VecDeque<ReplayStep>,
    requests: Vec<HostManagedModelRequest>,
}

#[derive(Debug, Clone)]
struct ReplayStep {
    output: ReplayOutput,
    request_contains: Option<String>,
    expected_tool_results: Vec<ExpectedToolResult>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum ReplayOutput {
    Response(HostManagedModelResponse),
    AssertProviderToolsThenResponse {
        capability_ids: Vec<CapabilityId>,
        response: HostManagedModelResponse,
    },
    AssertProviderToolsThenProviderToolCalls {
        capability_ids: Vec<CapabilityId>,
        calls: Vec<RebornScriptedProviderToolCall>,
    },
    DelayedResponse {
        response: HostManagedModelResponse,
        delay: Duration,
    },
    ProviderToolCalls(Vec<RebornScriptedProviderToolCall>),
    ModelError {
        kind: HostManagedModelErrorKind,
        message: String,
    },
}

impl RebornTraceReplayModelGateway {
    pub fn from_trace(trace: LlmTrace) -> Result<Self, RebornTraceReplayError> {
        let mut steps = VecDeque::new();
        for turn in trace.turns {
            for step in turn.steps {
                steps.push_back(replay_step(step)?);
            }
        }
        Ok(Self::from_steps(steps))
    }

    pub fn with_responses(responses: impl IntoIterator<Item = HostManagedModelResponse>) -> Self {
        Self::from_steps(
            responses
                .into_iter()
                .map(|response| ReplayStep {
                    output: ReplayOutput::Response(response),
                    request_contains: None,
                    expected_tool_results: Vec::new(),
                })
                .collect(),
        )
    }

    #[allow(dead_code)]
    pub fn with_scripted_steps(steps: impl IntoIterator<Item = RebornModelReplayStep>) -> Self {
        Self::from_steps(
            steps
                .into_iter()
                .map(|step| match step {
                    RebornModelReplayStep::Response {
                        response,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::Response(response),
                        request_contains: None,
                        expected_tool_results,
                    },
                    RebornModelReplayStep::ResponseForRequest {
                        request_contains,
                        response,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::Response(response),
                        request_contains: Some(request_contains),
                        expected_tool_results,
                    },
                    RebornModelReplayStep::AssertProviderToolsThenResponse {
                        capability_ids,
                        response,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::AssertProviderToolsThenResponse {
                            capability_ids,
                            response,
                        },
                        request_contains: None,
                        expected_tool_results,
                    },
                    RebornModelReplayStep::AssertProviderToolsThenProviderToolCalls {
                        capability_ids,
                        calls,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::AssertProviderToolsThenProviderToolCalls {
                            capability_ids,
                            calls,
                        },
                        request_contains: None,
                        expected_tool_results,
                    },
                    RebornModelReplayStep::DelayedResponse {
                        response,
                        delay,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::DelayedResponse { response, delay },
                        request_contains: None,
                        expected_tool_results,
                    },
                    RebornModelReplayStep::DelayedResponseForRequest {
                        request_contains,
                        response,
                        delay,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::DelayedResponse { response, delay },
                        request_contains: Some(request_contains),
                        expected_tool_results,
                    },
                    RebornModelReplayStep::ProviderToolCalls {
                        calls,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::ProviderToolCalls(calls),
                        request_contains: None,
                        expected_tool_results,
                    },
                    RebornModelReplayStep::ProviderToolCallsForRequest {
                        request_contains,
                        calls,
                        expected_tool_results,
                    } => ReplayStep {
                        output: ReplayOutput::ProviderToolCalls(calls),
                        request_contains: Some(request_contains),
                        expected_tool_results,
                    },
                    RebornModelReplayStep::ModelError { kind, message } => ReplayStep {
                        output: ReplayOutput::ModelError { kind, message },
                        request_contains: None,
                        expected_tool_results: Vec::new(),
                    },
                    RebornModelReplayStep::ModelErrorForRequest {
                        request_contains,
                        kind,
                        message,
                    } => ReplayStep {
                        output: ReplayOutput::ModelError { kind, message },
                        request_contains: Some(request_contains),
                        expected_tool_results: Vec::new(),
                    },
                })
                .collect(),
        )
    }

    fn from_steps(steps: VecDeque<ReplayStep>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ReplayState {
                steps,
                requests: Vec::new(),
            })),
        }
    }

    pub fn requests(&self) -> Vec<HostManagedModelRequest> {
        self.inner
            .lock()
            .expect("trace replay lock poisoned")
            .requests
            .clone()
    }

    pub fn remaining_responses(&self) -> usize {
        self.inner
            .lock()
            .expect("trace replay lock poisoned")
            .steps
            .len()
    }

    pub fn assert_exhausted(&self) {
        assert_eq!(self.remaining_responses(), 0, "trace replay not exhausted");
    }
}

#[async_trait]
impl HostManagedModelGateway for RebornTraceReplayModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let step = self.take_step(request)?;
        match step.output {
            ReplayOutput::Response(response) => Ok(response),
            ReplayOutput::AssertProviderToolsThenResponse { .. } => {
                Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    "trace replay provider tool assertions require capability-aware model streaming",
                ))
            }
            ReplayOutput::AssertProviderToolsThenProviderToolCalls { .. } => {
                Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    "trace replay provider tool assertions require capability-aware model streaming",
                ))
            }
            ReplayOutput::DelayedResponse { response, delay } => {
                tokio::time::sleep(delay).await;
                Ok(response)
            }
            ReplayOutput::ProviderToolCalls(_) => Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "trace replay provider tool calls require capability-aware model streaming",
            )),
            ReplayOutput::ModelError { kind, message } => {
                Err(HostManagedModelError::safe(kind, message))
            }
        }
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let step = self.take_step(request.clone())?;
        match step.output {
            ReplayOutput::Response(response) => {
                provider_tool_calls_response_from_replayed_response(
                    &request,
                    capabilities,
                    response,
                )
                .await
            }
            ReplayOutput::AssertProviderToolsThenResponse {
                capability_ids,
                response,
            } => {
                assert_provider_tools(capabilities, &capability_ids).await?;
                Ok(response)
            }
            ReplayOutput::AssertProviderToolsThenProviderToolCalls {
                capability_ids,
                calls,
            } => {
                assert_provider_tools(capabilities.clone(), &capability_ids).await?;
                provider_tool_calls_response(&request, capabilities, calls).await
            }
            ReplayOutput::DelayedResponse { response, delay } => {
                tokio::time::sleep(delay).await;
                Ok(response)
            }
            ReplayOutput::ProviderToolCalls(calls) => {
                provider_tool_calls_response(&request, capabilities, calls).await
            }
            ReplayOutput::ModelError { kind, message } => {
                Err(HostManagedModelError::safe(kind, message))
            }
        }
    }
}

impl RebornTraceReplayModelGateway {
    fn take_step(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<ReplayStep, HostManagedModelError> {
        let mut state = self.inner.lock().map_err(|_| {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "trace replay lock poisoned",
            )
        })?;
        let Some(position) = state
            .steps
            .iter()
            .position(|step| step.matches_request(&request))
        else {
            eprintln!(
                "trace replay has no matching step; request metadata: {}; remaining steps: {}",
                request_metadata_summary(&request),
                state
                    .steps
                    .iter()
                    .map(replay_step_summary)
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                format!(
                    "trace replay has no matching step for request metadata: {}",
                    request_metadata_summary(&request)
                ),
            ));
        };
        let step = state
            .steps
            .get(position)
            .expect("matched replay step position")
            .clone();
        validate_expected_tool_results(&request, &step.expected_tool_results)?;
        state.requests.push(request);
        Ok(state
            .steps
            .remove(position)
            .expect("matched replay step remains"))
    }
}

async fn assert_provider_tools(
    capabilities: Arc<dyn LoopCapabilityPort>,
    capability_ids: &[CapabilityId],
) -> Result<(), HostManagedModelError> {
    // Hosted-MCP packages publish discovered tools asynchronously (bounded
    // discovery retries under #6520), so re-check the surface briefly before
    // failing — the contract is "advertised before the reply", not "at the
    // first instant of the model call". Still fails if a tool never appears.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let definitions = provider_tool_definitions(&capabilities).await?;
        let missing = capability_ids.iter().find(|capability_id| {
            !definitions
                .iter()
                .any(|definition| &definition.capability_id == *capability_id)
        });
        let Some(capability_id) = missing else {
            return Ok(());
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                format!(
                    "expected capability {} was not advertised to the model",
                    capability_id.as_str()
                ),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn provider_tool_definitions(
    capabilities: &Arc<dyn LoopCapabilityPort>,
) -> Result<Vec<ProviderToolDefinition>, HostManagedModelError> {
    capabilities
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .map_err(capability_host_error)?;
    capabilities
        .tool_definitions()
        .map_err(capability_host_error)
}

impl ReplayStep {
    fn matches_request(&self, request: &HostManagedModelRequest) -> bool {
        let Some(needle) = &self.request_contains else {
            return true;
        };
        request
            .messages
            .iter()
            .any(|message| message.content.contains(needle))
    }
}

fn request_metadata_summary(request: &HostManagedModelRequest) -> String {
    let roles = request
        .messages
        .iter()
        .map(|message| format!("{:?}", message.role))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "messages={},roles=[{}],surface_version_present={},resolved_route_present={}",
        request.messages.len(),
        roles,
        request.surface_version.is_some(),
        request.resolved_model_route.is_some()
    )
}

fn replay_step_summary(step: &ReplayStep) -> String {
    let output = match &step.output {
        ReplayOutput::Response(_) => "response",
        ReplayOutput::AssertProviderToolsThenResponse { .. } => {
            "assert_provider_tools_then_response"
        }
        ReplayOutput::AssertProviderToolsThenProviderToolCalls { .. } => {
            "assert_provider_tools_then_provider_tool_calls"
        }
        ReplayOutput::DelayedResponse { .. } => "delayed_response",
        ReplayOutput::ProviderToolCalls(_) => "provider_tool_calls",
        ReplayOutput::ModelError { .. } => "model_error",
    };
    let request_contains = step
        .request_contains
        .as_ref()
        .map(|value| format!("redacted:{} chars", value.chars().count()))
        .unwrap_or_else(|| "any".to_string());
    format!(
        "{output}(request_contains={request_contains}, expected_tool_results={})",
        step.expected_tool_results.len()
    )
}

async fn provider_tool_calls_response(
    request: &HostManagedModelRequest,
    capabilities: Arc<dyn LoopCapabilityPort>,
    calls: Vec<RebornScriptedProviderToolCall>,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    let definitions = provider_tool_definitions(&capabilities).await?;
    let mut candidates = Vec::with_capacity(calls.len());
    for call in calls {
        let definition = definitions
            .iter()
            .find(|definition| definition.capability_id == call.capability_id)
            .ok_or_else(|| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidOutput,
                    format!(
                        "scripted capability {} was not advertised to the model",
                        call.capability_id.as_str()
                    ),
                )
            })?;
        let provider_call = ProviderToolCall {
            provider_id: "trace_replay".to_string(),
            provider_model_id: "trace_replay".to_string(),
            turn_id: Some(format!("trace-turn:{}", request.run_id)),
            id: call.call_id,
            name: definition.name.clone(),
            arguments: call.arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        };
        capabilities
            .validate_provider_tool_call(&provider_call)
            .map_err(capability_host_error)?;
        candidates.push(
            capabilities
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call))
                .await
                .map_err(capability_host_error)?,
        );
    }
    Ok(HostManagedModelResponse::capability_calls(candidates, ""))
}

async fn provider_tool_calls_response_from_replayed_response(
    request: &HostManagedModelRequest,
    capabilities: Arc<dyn LoopCapabilityPort>,
    response: HostManagedModelResponse,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    let ParentLoopOutput::CapabilityCalls(calls) = response.output else {
        return Ok(response);
    };
    let mut scripted_calls = Vec::with_capacity(calls.len());
    for call in calls {
        let replay = call.provider_replay.ok_or_else(|| {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidOutput,
                format!(
                    "trace replay capability {} is missing provider replay metadata",
                    call.capability_id.as_str()
                ),
            )
        })?;
        scripted_calls.push(RebornScriptedProviderToolCall {
            capability_id: call.capability_id,
            call_id: replay.provider_call_id,
            arguments: replay.arguments,
        });
    }
    let mut registered =
        provider_tool_calls_response(request, capabilities, scripted_calls).await?;
    registered.safe_text_deltas = response.safe_text_deltas;
    registered.safe_reasoning_deltas = response.safe_reasoning_deltas;
    registered.usage = response.usage;
    Ok(registered)
}

fn capability_host_error(error: AgentLoopHostError) -> HostManagedModelError {
    HostManagedModelError::safe(
        HostManagedModelErrorKind::InvalidOutput,
        format!("capability trace replay failed: {}", error.safe_summary),
    )
}

fn replay_step(step: TraceStep) -> Result<ReplayStep, RebornTraceReplayError> {
    Ok(ReplayStep {
        output: ReplayOutput::Response(response_from_trace(step.response)?),
        request_contains: None,
        expected_tool_results: step.expected_tool_results,
    })
}

fn response_from_trace(
    response: TraceResponse,
) -> Result<HostManagedModelResponse, RebornTraceReplayError> {
    match response {
        TraceResponse::Text { content, .. } => {
            Ok(HostManagedModelResponse::assistant_reply(content))
        }
        TraceResponse::ToolCalls { tool_calls, .. } => {
            Ok(HostManagedModelResponse::capability_calls(
                tool_calls
                    .into_iter()
                    .map(capability_call_from_trace)
                    .collect::<Result<Vec<_>, _>>()?,
                "",
            ))
        }
        TraceResponse::UserInput { .. } => Err(RebornTraceReplayError::UnsupportedResponse),
    }
}

fn capability_call_from_trace(
    call: TraceToolCall,
) -> Result<CapabilityCallCandidate, RebornTraceReplayError> {
    capability_call_from_trace_with_surface(call, TRACE_REPLAY_SURFACE_VERSION)
}

pub(crate) fn capability_call_from_trace_with_surface(
    call: TraceToolCall,
    surface_version: &str,
) -> Result<CapabilityCallCandidate, RebornTraceReplayError> {
    let surface_version = CapabilitySurfaceVersion::new(surface_version)
        .map_err(RebornTraceReplayError::InvalidSurfaceVersion)?;
    let capability_name = if let Some(builtin) = call.name.strip_prefix("builtin__") {
        format!("builtin.{builtin}")
    } else if call.name.contains('.') {
        call.name.clone()
    } else {
        format!("trace.{}", call.name)
    };
    let capability_id = CapabilityId::new(capability_name.clone()).map_err(|error| {
        RebornTraceReplayError::InvalidCapabilityId {
            name: capability_name.clone(),
            reason: error.to_string(),
        }
    })?;
    let provider_tool_name = trace_provider_tool_name(&call.name)?;
    let input_ref =
        CapabilityInputRef::new(format!("input:trace-{}", call.id)).map_err(|reason| {
            RebornTraceReplayError::InvalidInputRef {
                id: call.id.clone(),
                reason,
            }
        })?;
    Ok(CapabilityCallCandidate {
        activity_id: ironclaw_turns::CapabilityActivityId::new(),
        surface_version,
        effective_capability_ids: vec![capability_id.clone()],
        capability_id,
        input_ref,
        provider_replay: Some(ProviderToolCallReplay {
            provider_id: "trace_replay".to_string(),
            provider_model_id: "trace_replay".to_string(),
            provider_turn_id: "trace-turn".to_string(),
            provider_call_id: call.id,
            provider_tool_name,
            arguments: call.arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }),
    })
}

fn trace_provider_tool_name(name: &str) -> Result<ProviderToolName, RebornTraceReplayError> {
    let provider_name = name.replace('.', "__");
    ProviderToolName::new(provider_name).map_err(|error| {
        RebornTraceReplayError::InvalidProviderToolName {
            name: name.to_string(),
            reason: error.to_string(),
        }
    })
}

fn validate_expected_tool_results(
    request: &HostManagedModelRequest,
    expected: &[ExpectedToolResult],
) -> Result<(), HostManagedModelError> {
    for expected_result in expected {
        let matched = request.messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::ToolResult
                && message.content == expected_result.content
                && message
                    .tool_result_provider_call
                    .as_ref()
                    .is_some_and(|provider_call| {
                        provider_call.provider_call_id == expected_result.tool_call_id
                            && provider_call.provider_tool_name.as_str() == expected_result.name
                    })
        });
        if !matched {
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                format!(
                    "trace replay expected tool result {} for {}",
                    expected_result.tool_call_id, expected_result.name
                ),
            ));
        }
    }
    Ok(())
}
