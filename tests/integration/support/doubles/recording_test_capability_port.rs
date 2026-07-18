#![allow(dead_code)] // Carried from harness.rs's blanket allow: shared across bins with differing usage.

/// Test double substituting the whole production capability-port dispatch
/// pipeline (`HostRuntimeLoopCapabilityPortFactory` +
/// `RefreshingLoopCapabilityPortFactory`) with a lightweight in-memory Echo backend.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ExtensionId, ProviderToolName, RuntimeKind};
use ironclaw_host_runtime::READ_FILE_CAPABILITY_ID;
use ironclaw_loop_host::{
    DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, build_spawn_subagent_parameters_schema,
};
use ironclaw_turns::{
    LoopGateRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityDescriptorView,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        CapabilitySurfaceVersion, ConcurrencyHint, LoopCapabilityPort, ProviderToolCallReplay,
        ProviderToolDefinition, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};
use serde_json::json;

pub(crate) const TEST_CAPABILITY_ID: &str = "test.echo";
pub(crate) const TEST_CAPABILITY_SURFACE_VERSION: &str = "trace_replay_v1";
const SUBAGENT_ALLOWED_TEST_TOOL_NAME: &str = "test_read_file";
const SPAWN_SUBAGENT_PROVIDER_TOOL_NAME: &str = "builtin__spawn_subagent";

#[derive(Clone)]
pub struct RecordingTestCapabilityPort {
    mode: CapabilityMode,
    expose_spawn_subagent: bool,
    use_subagent_allowed_tool: bool,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    next_result: Arc<AtomicUsize>,
    approval_calls: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy)]
enum CapabilityMode {
    Echo,
    ApprovalThenEcho,
    SpawnAuthThenApprovalThenEcho,
    InvocationError,
}

impl RecordingTestCapabilityPort {
    pub fn echo() -> Self {
        Self::new(CapabilityMode::Echo, false, false)
    }

    /// Every capability invocation fails with a scripted host invocation error
    /// (fault-matrix P4: non-model capability-stage failure).
    pub fn invocation_error() -> Self {
        Self::new(CapabilityMode::InvocationError, false, false)
    }

    pub fn echo_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::Echo, true, false)
    }

    pub fn approval_then_echo() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho, false, false)
    }

    pub fn approval_then_echo_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho, true, false)
    }

    pub fn approval_then_allowed_tool_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho, true, true)
    }

    pub fn spawn_auth_then_approval_then_echo_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::SpawnAuthThenApprovalThenEcho, true, false)
    }

    pub fn spawn_auth_then_approval_then_allowed_tool_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::SpawnAuthThenApprovalThenEcho, true, true)
    }

    fn new(
        mode: CapabilityMode,
        expose_spawn_subagent: bool,
        use_subagent_allowed_tool: bool,
    ) -> Self {
        Self {
            mode,
            expose_spawn_subagent,
            use_subagent_allowed_tool,
            invocations: Arc::new(Mutex::new(Vec::new())),
            next_result: Arc::new(AtomicUsize::new(1)),
            approval_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn primary_capability_id(&self) -> CapabilityId {
        let id = if self.use_subagent_allowed_tool {
            READ_FILE_CAPABILITY_ID
        } else {
            TEST_CAPABILITY_ID
        };
        CapabilityId::new(id).expect("valid capability id")
    }

    fn primary_tool_name(&self) -> &'static str {
        if self.use_subagent_allowed_tool {
            SUBAGENT_ALLOWED_TEST_TOOL_NAME
        } else {
            "test_echo"
        }
    }

    pub(crate) fn exposes_spawn_subagent(&self) -> bool {
        self.expose_spawn_subagent
    }

    fn spawn_subagent_capability_id() -> CapabilityId {
        CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).expect("valid capability id")
    }

    fn capability_id_for_provider_tool(
        &self,
        tool_name: &ProviderToolName,
    ) -> Result<CapabilityId, AgentLoopHostError> {
        if tool_name.as_str() == self.primary_tool_name() {
            return Ok(self.primary_capability_id());
        }
        if self.expose_spawn_subagent && tool_name.as_str() == SPAWN_SUBAGENT_PROVIDER_TOOL_NAME {
            return Ok(Self::spawn_subagent_capability_id());
        }
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("provider tool call {tool_name} is outside the visible capability surface"),
        ))
    }

    pub(crate) fn invocations(&self) -> Vec<CapabilityInvocation> {
        self.invocations.lock().unwrap().clone()
    }

    pub fn invocation_count(&self) -> usize {
        self.invocations.lock().unwrap().len()
    }

    pub(crate) fn capability_allowlist(&self) -> Vec<CapabilityId> {
        let mut allowlist = vec![self.primary_capability_id()];
        if self.expose_spawn_subagent {
            allowlist.push(Self::spawn_subagent_capability_id());
        }
        allowlist
    }

    fn completed_result(&self) -> CapabilityOutcome {
        let ordinal = self.next_result.fetch_add(1, Ordering::SeqCst);
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: ironclaw_turns::LoopResultRef::new(format!("result:test-echo-{ordinal}"))
                .expect("valid result ref"),
            safe_summary: "echo: hi".to_string(),
            progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 0,
            output_digest: None,
            model_observation: None,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingTestCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = vec![ProviderToolDefinition {
            capability_id: self.primary_capability_id(),
            name: ProviderToolName::new(self.primary_tool_name()).expect("provider tool name"),
            description: "Echo a test payload".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            }),
        }];
        if self.expose_spawn_subagent {
            definitions.push(ProviderToolDefinition {
                capability_id: Self::spawn_subagent_capability_id(),
                name: ProviderToolName::new(SPAWN_SUBAGENT_PROVIDER_TOOL_NAME)
                    .expect("provider tool name"),
                description: "Spawn a child subagent run and wait for its result".to_string(),
                parameters: build_spawn_subagent_parameters_schema(&[]),
            });
        }
        Ok(definitions)
    }

    async fn register_provider_tool_call(
        &self,
        request: ironclaw_turns::run_profile::RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let call = request.tool_call;
        let capability_id = self.capability_id_for_provider_tool(&call.name)?;
        Ok(CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
            input_ref: CapabilityInputRef::new(format!("input:{}", call.id))
                .expect("valid input ref"),
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: call.provider_id,
                provider_model_id: call.provider_model_id,
                provider_turn_id: call.turn_id.unwrap_or_else(|| "trace-turn".to_string()),
                provider_call_id: call.id,
                provider_tool_name: call.name,
                arguments: call.arguments,
                response_reasoning: call.response_reasoning,
                reasoning: call.reasoning,
                signature: call.signature,
            }),
        })
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut descriptors = vec![CapabilityDescriptorView {
            capability_id: self.primary_capability_id(),
            provider: Some(ExtensionId::new("test").expect("valid provider")),
            runtime: RuntimeKind::FirstParty,
            safe_name: self.primary_tool_name().to_string(),
            safe_description: "Echo a test payload".to_string(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: json!({"type": "object"}),
        }];
        if self.expose_spawn_subagent {
            descriptors.push(CapabilityDescriptorView {
                capability_id: Self::spawn_subagent_capability_id(),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string(),
                safe_description: "Spawn a child subagent run and wait for its result".to_string(),
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: build_spawn_subagent_parameters_schema(&[]),
            });
        }
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            descriptors,
            callable_capability_ids: None,
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.invocations.lock().unwrap().push(request);
        if matches!(self.mode, CapabilityMode::InvocationError) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "scripted capability invocation failure",
            ));
        }
        if matches!(self.mode, CapabilityMode::ApprovalThenEcho)
            && self.approval_calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Ok(CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:test-approval").expect("valid gate ref"),
                safe_summary: "test approval required".to_string(),
                approval_resume: None,
            });
        }
        if matches!(self.mode, CapabilityMode::SpawnAuthThenApprovalThenEcho) {
            match self.approval_calls.fetch_add(1, Ordering::SeqCst) {
                0 => return Ok(self.completed_result()),
                1 => {
                    return Ok(CapabilityOutcome::ApprovalRequired {
                        gate_ref: LoopGateRef::new("gate:test-approval").expect("valid gate ref"),
                        safe_summary: "test approval required".to_string(),
                        approval_resume: None,
                    });
                }
                _ => {}
            }
        }
        Ok(self.completed_result())
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let stop_on_first_suspension = request.stop_on_first_suspension;
        let mut outcomes = Vec::new();
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let outcome = self.invoke_capability(invocation).await?;
            let is_suspension = outcome.is_suspension();
            outcomes.push(outcome);
            if is_suspension && stop_on_first_suspension {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}
