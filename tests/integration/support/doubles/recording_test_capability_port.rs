#![allow(dead_code)] // Carried from harness.rs's blanket allow: shared across bins with differing usage.

/// Test double substituting the whole production capability-port dispatch
/// pipeline (`HostRuntimeLoopCapabilityPortFactory` +
/// `RefreshingLoopCapabilityPortFactory`) with a lightweight in-memory Echo backend.
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_host_api::resolution::{Resolution, ResolutionBatch};
use ironclaw_host_api::{
    ids::{CapabilityId, ExtensionId, ProviderToolName},
    runtime::RuntimeKind,
};
use ironclaw_host_runtime::READ_FILE_CAPABILITY_ID;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityCallCandidate, CapabilityDescriptorView,
    CapabilityInputRef, CapabilitySurfaceVersion, ContentDigest, LoopCapabilityPort, LoopRequest,
    LoopRequestBatch, ProviderToolCallReplay, ProviderToolDefinition, VisibleCapabilityRequest,
    VisibleCapabilitySurface, resolution,
};
use ironclaw_loop_host::{
    DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, build_spawn_subagent_parameters_schema,
};
use ironclaw_turns::LoopGateRef;
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
    invocations: Arc<Mutex<Vec<LoopRequest>>>,
    next_result: Arc<AtomicUsize>,
    approval_calls: Arc<AtomicUsize>,
    /// Scripted call arguments keyed by the `input_ref` minted for them in
    /// `register_provider_tool_call`. Only `CapabilityMode::NoProgress`
    /// reads this (in `completed_result`): the no-progress loop-stop check
    /// (`ironclaw_agent_loop::strategies::stop`) keys off (signature,
    /// output_digest) pairs, so a test double driving that check must report
    /// a digest that varies with the scripted arguments the same way a real
    /// capability's output would. The changing-output negative control uses a
    /// fixed marker argument and instead varies both the returned text and its
    /// digest from the result sequence.
    arguments_by_input_ref: Arc<Mutex<HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Copy)]
enum CapabilityMode {
    Echo,
    NoProgress,
    ApprovalThenEcho,
    SpawnAuthThenApprovalThenEcho,
    InvocationError,
    RecoverablePortError,
    InvalidInputThenEcho,
}

impl RecordingTestCapabilityPort {
    pub fn echo() -> Self {
        Self::new(CapabilityMode::Echo, false, false)
    }

    pub fn no_progress() -> Self {
        Self::new(CapabilityMode::NoProgress, false, false)
    }

    /// Every capability invocation returns a scripted **caller-shaped** port
    /// error (`InvalidInvocation`). Before #6284's capability-stage fix, any
    /// non-`Cancelled` port error ended the run; now caller-shaped kinds
    /// surface model-visibly and the run continues. Pairs with
    /// [`Self::invocation_error`], which uses a kind that is still terminal.
    pub fn recoverable_port_error() -> Self {
        Self::new(CapabilityMode::RecoverablePortError, false, false)
    }

    /// Every capability invocation fails with a scripted TERMINAL host fault
    /// (`Unavailable` — fault-matrix P4: non-model capability-stage failure).
    /// Deliberately a kind in the executor's terminal set
    /// (`capability_port_error_is_terminal`): caller-shaped kinds such as
    /// `InvalidInvocation` now surface model-visibly and the run recovers
    /// in-loop, which would defeat the run-failed → user-retry journeys this
    /// double exists to drive.
    pub fn invocation_error() -> Self {
        Self::new(CapabilityMode::InvocationError, false, false)
    }

    /// First invocation is a model-actionable invalid input; a changed second
    /// invocation succeeds. Used to prove the whole-turn correction loop
    /// independently of any one capability producer.
    pub fn invalid_input_then_echo() -> Self {
        Self::new(CapabilityMode::InvalidInputThenEcho, false, false)
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
            arguments_by_input_ref: Arc::new(Mutex::new(HashMap::new())),
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

    pub(crate) fn invocations(&self) -> Vec<LoopRequest> {
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

    fn completed_result(&self, input_ref: &CapabilityInputRef) -> Resolution {
        let ordinal = self.next_result.fetch_add(1, Ordering::SeqCst);
        let progress = if matches!(self.mode, CapabilityMode::NoProgress) {
            ironclaw_loop_contracts::CapabilityProgress::NoChange
        } else {
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress
        };
        let arguments = self
            .arguments_by_input_ref
            .lock()
            .unwrap()
            .get(input_ref.as_str())
            .cloned();
        // The changing-output integration negative control keeps the call
        // arguments fixed, so its output must vary from the result sequence,
        // not be derived from those arguments. Other no-progress cases retain
        // the argument-derived digest that makes their scripted output repeat.
        let changing_output = matches!(
            arguments.as_ref().and_then(|arguments| arguments.get("message")),
            Some(serde_json::Value::String(message)) if message == "changing-output"
        );
        let output = if matches!(self.mode, CapabilityMode::NoProgress) && changing_output {
            format!("echo: changing-output-{ordinal}")
        } else {
            "echo: hi".to_string()
        };
        // Only the no-progress mode needs a corroborating output digest — see
        // the doc comment on `arguments_by_input_ref`. For the changing-output
        // probe, hash the returned output itself so the digest varies
        // independently of the fixed call signature.
        let output_digest: Option<ContentDigest> =
            if matches!(self.mode, CapabilityMode::NoProgress) {
                if changing_output {
                    ContentDigest::from_json_value(&serde_json::Value::String(output.clone())).ok()
                } else {
                    arguments
                        .as_ref()
                        .and_then(|arguments| ContentDigest::from_json_value(arguments).ok())
                }
            } else {
                None
            };
        resolution::completed(
            ironclaw_turns::LoopResultRef::new(format!("result:test-echo-{ordinal}"))
                .expect("valid result ref"),
            output,
            progress,
            false,
            0,
            output_digest,
            None,
        )
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingTestCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = vec![ProviderToolDefinition {
            capability_id: self.primary_capability_id(),
            name: ProviderToolName::new(self.primary_tool_name()).expect("provider tool name"),
            description: "Echo a test payload".to_string(),
            description_trust: Default::default(),
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
                description_trust: Default::default(),
                parameters: build_spawn_subagent_parameters_schema(&[]),
            });
        }
        Ok(definitions)
    }

    async fn register_provider_tool_call(
        &self,
        request: ironclaw_loop_contracts::RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let call = request.tool_call;
        let capability_id = self.capability_id_for_provider_tool(&call.name)?;
        let input_ref =
            CapabilityInputRef::new(format!("input:{}", call.id)).expect("valid input ref");
        self.arguments_by_input_ref
            .lock()
            .unwrap()
            .insert(input_ref.as_str().to_string(), call.arguments.clone());
        Ok(CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
            input_ref,
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
            description_trust: Default::default(),
            parameters_schema: json!({"type": "object"}),
        }];
        if self.expose_spawn_subagent {
            descriptors.push(CapabilityDescriptorView {
                capability_id: Self::spawn_subagent_capability_id(),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string(),
                safe_description: "Spawn a child subagent run and wait for its result".to_string(),
                description_trust: Default::default(),
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
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        let input_ref = request.input_ref.clone();
        self.invocations.lock().unwrap().push(request);
        if matches!(self.mode, CapabilityMode::InvocationError) {
            // Terminal host fault: `Unavailable` stays in the executor's
            // terminal set, so the run fails with a retryable checkpoint
            // instead of recovering in-loop (see `invocation_error()`).
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "scripted capability invocation failure",
            ));
        }
        if matches!(self.mode, CapabilityMode::RecoverablePortError) {
            // Caller-shaped host fault: the model can act on it, so the
            // executor surfaces it as a tool error and the run continues.
            //
            // `InvalidInvocation` (not `Unauthorized`) on purpose: the summary
            // prefix for `Authorization` is "capability failed with
            // authorization: ", and "authorization:" is a banned marker in the
            // loop-safe validator, so that kind fail-softs to the redacted
            // fallback and would hide the very kind this test asserts on.
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "scripted caller-shaped capability port failure",
            ));
        }
        if matches!(self.mode, CapabilityMode::InvalidInputThenEcho)
            && self.approval_calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Ok(resolution::failed(
                ironclaw_host_api::result_meta::FailureKind::InputEncode,
                "capability input failed validation".to_string(),
                ironclaw_loop_contracts::CapabilityFailureDetail::Diagnostic {
                    text: "capability input failed validation".to_string(),
                },
            ));
        }
        if matches!(self.mode, CapabilityMode::ApprovalThenEcho)
            && self.approval_calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Ok(resolution::approval_required(
                LoopGateRef::new("gate:test-approval").expect("valid gate ref"),
                "test approval required".to_string(),
                None,
            )
            .resolution);
        }
        if matches!(self.mode, CapabilityMode::SpawnAuthThenApprovalThenEcho) {
            match self.approval_calls.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    return Ok(self.completed_result(&input_ref));
                }
                1 => {
                    return Ok(resolution::approval_required(
                        LoopGateRef::new("gate:test-approval").expect("valid gate ref"),
                        "test approval required".to_string(),
                        None,
                    )
                    .resolution);
                }
                _ => {}
            }
        }
        Ok(self.completed_result(&input_ref))
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let stop_on_first_suspension = request.stop_on_first_suspension;
        let mut resolutions = Vec::new();
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let resolution = self.invoke_capability(invocation).await?;
            // `parks()`, not `is_suspension()` (H1): a re-entrant gate stops the batch too.
            let parks = resolution.parks();
            resolutions.push(resolution);
            if parks && stop_on_first_suspension {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension,
        })
    }
}
