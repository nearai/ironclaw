use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::run_profile::{
    CapabilityCallCandidate, CapabilityInputRef, CapabilitySurfaceVersion, ProviderToolCallReplay,
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
    #[error("invalid trace capability input ref for {id}: {reason}")]
    InvalidInputRef { id: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct RebornTraceReplayModelGateway {
    inner: Arc<Mutex<ReplayState>>,
}

#[derive(Debug)]
struct ReplayState {
    steps: VecDeque<ReplayStep>,
    requests: Vec<HostManagedModelRequest>,
}

#[derive(Debug, Clone)]
struct ReplayStep {
    response: HostManagedModelResponse,
    expected_tool_results: Vec<ExpectedToolResult>,
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
                    response,
                    expected_tool_results: Vec::new(),
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
        let mut state = self.inner.lock().map_err(|_| {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "trace replay lock poisoned",
            )
        })?;
        let Some(step) = state.steps.front().cloned() else {
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "trace replay exhausted",
            ));
        };
        validate_expected_tool_results(&request, &step.expected_tool_results)?;
        state.requests.push(request);
        state.steps.pop_front();
        Ok(step.response)
    }
}

fn replay_step(step: TraceStep) -> Result<ReplayStep, RebornTraceReplayError> {
    Ok(ReplayStep {
        response: response_from_trace(step.response)?,
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
    let capability_name = if call.name.contains('.') {
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
    let input_ref =
        CapabilityInputRef::new(format!("input:trace-{}", call.id)).map_err(|reason| {
            RebornTraceReplayError::InvalidInputRef {
                id: call.id.clone(),
                reason,
            }
        })?;
    Ok(CapabilityCallCandidate {
        surface_version,
        capability_id,
        input_ref,
        provider_replay: Some(ProviderToolCallReplay {
            provider_id: "trace_replay".to_string(),
            provider_model_id: "trace_replay".to_string(),
            provider_turn_id: "trace-turn".to_string(),
            provider_call_id: call.id,
            provider_tool_name: call.name,
            arguments: call.arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }),
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
                            && provider_call.provider_tool_name == expected_result.name
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
