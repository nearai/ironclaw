use ironclaw_host_api::process::MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES;
use ironclaw_loop_contracts::*;
use ironclaw_turns::LoopMessageRef;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const LOOP_WORKER_WIRE_VERSION: u16 = 2;
pub const LOOP_WORKER_MAX_FRAME_BYTES: usize = MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopWorkerInvocation {
    Run(AgentLoopDriverRunRequest),
    Resume(AgentLoopDriverResumeRequest),
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LoopWorkerSettings {
    pub default_iteration_limit: Option<u32>,
    pub model_availability_attempts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopWorkerBootstrap {
    pub wire_version: u16,
    pub run_context: LoopRunContext,
    pub invocation: LoopWorkerInvocation,
    pub settings: LoopWorkerSettings,
    pub tool_definitions: Vec<ProviderToolDefinition>,
    pub current_visible_capabilities: Option<serde_json::Value>,
    /// Whether the worker is allowed to see resolved transcript content.
    /// An omitted field is blind; workers still reject unsupported wire versions.
    #[serde(default)]
    pub content_visibility: WorkerContentVisibility,
}

/// Whether the worker was bootstrapped allowed to request resolved
/// model-visible content for the message refs the host already issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerContentVisibility {
    /// The worker sees refs and safe summaries only (canonical Rust worker).
    #[default]
    Blind,
    /// The worker may call `HostCall::ResolveMessages` for issued refs.
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopWorkerFailure {
    pub kind: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopWorkerOutcome {
    Exit(LoopExit),
    Failed(LoopWorkerFailure),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRequestFrame {
    pub id: u64,
    pub call: HostCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostResponseFrame {
    pub id: u64,
    pub result: Result<serde_json::Value, WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerFrame {
    HostRequest(Box<HostRequestFrame>),
    Outcome(LoopWorkerOutcome),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostFrame {
    Bootstrap(Box<LoopWorkerBootstrap>),
    HostResponse(HostResponseFrame),
    Cancel(LoopCancellationSignal),
    OutcomeAck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireError {
    Host(AgentLoopHostError),
    Compaction(LoopCompactionError),
    Protocol(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostCall {
    ResolveMessages(ResolveMessagesRequest),
    LoadContext(LoopContextRequest),
    BuildPrompt(LoopPromptBundleRequest),
    PollInputs {
        after: LoopInputCursor,
        limit: usize,
    },
    AckInputs(Vec<LoopInputAckToken>),
    StreamModel(LoopModelRequest),
    RegisterProviderToolCall(RegisterProviderToolCallRequest),
    VisibleCapabilities(VisibleCapabilityRequest),
    InvokeCapability(LoopRequest),
    InvokeCapabilityBatch(LoopRequestBatch),
    BeginAssistantDraft(BeginAssistantDraft),
    UpdateAssistantDraft(UpdateAssistantDraft),
    FinalizeAssistantMessage(FinalizeAssistantMessage),
    AppendCapabilityResultRef(Box<AppendCapabilityResultRef>),
    Checkpoint(LoopCheckpointRequest),
    StageCheckpointPayload(StageCheckpointPayloadRequest),
    LoadCheckpointPayload(LoadCheckpointPayloadRequest),
    EmitProgress(LoopProgressEvent),
    Compact(LoopCompactionRequest),
}

/// Request for host-resolved model-visible content of message refs the run
/// already holds. Only honored for a worker bootstrapped `Resolved`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveMessagesRequest {
    pub messages: Vec<LoopModelMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireResolvedToolResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_call_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireResolvedModelMessage {
    pub role: String,
    pub content_ref: LoopMessageRef,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<WireResolvedToolResult>,
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, AgentLoopHostError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("loop worker frame serialization failed: {error}"),
        )
    })?;
    if bytes.len() > LOOP_WORKER_MAX_FRAME_BYTES {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "loop worker frame exceeds the configured byte limit",
        ));
    }
    Ok(bytes)
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, AgentLoopHostError> {
    if bytes.len() > LOOP_WORKER_MAX_FRAME_BYTES {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "loop worker frame exceeds the configured byte limit",
        ));
    }
    serde_json::from_slice(bytes).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("loop worker frame is malformed: {error}"),
        )
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireLoopContextBundle {
    identity_messages: Vec<WireLoopContextMessage>,
    messages: Vec<WireLoopContextMessage>,
    compaction_message_index: Vec<LoopContextCompactionMetadata>,
    recent_window_truncation: Option<LoopContextWindowTruncation>,
    instruction_snippets: Vec<WireLoopContextSnippet>,
    memory_snippets: Vec<WireLoopContextSnippet>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireLoadedCheckpointPayload {
    kind: LoopCheckpointKind,
    schema_id: CheckpointSchemaId,
    schema_version: ironclaw_host_api::turn::RunProfileVersion,
    payload: Vec<u8>,
}

impl From<LoadedCheckpointPayload> for WireLoadedCheckpointPayload {
    fn from(payload: LoadedCheckpointPayload) -> Self {
        Self {
            kind: payload.kind,
            schema_id: payload.schema_id,
            schema_version: payload.schema_version,
            payload: payload.payload.into_payload_bytes(),
        }
    }
}

impl TryFrom<WireLoadedCheckpointPayload> for LoadedCheckpointPayload {
    type Error = AgentLoopHostError;

    fn try_from(payload: WireLoadedCheckpointPayload) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: payload.kind,
            schema_id: payload.schema_id,
            schema_version: payload.schema_version,
            payload: RedactedCheckpointPayload::new(payload.payload).map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    format!("remote checkpoint payload is invalid: {error}"),
                )
            })?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireVisibleCapabilitySurface {
    version: CapabilitySurfaceVersion,
    descriptors: Vec<WireCapabilityDescriptorView>,
    callable_capability_ids: Option<Vec<ironclaw_host_api::ids::CapabilityId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireCapabilityDescriptorView {
    capability_id: ironclaw_host_api::ids::CapabilityId,
    provider: Option<ironclaw_host_api::ids::ExtensionId>,
    #[serde(deserialize_with = "ironclaw_host_api::runtime::deserialize_trusted_runtime_kind")]
    runtime: ironclaw_host_api::runtime::RuntimeKind,
    safe_name: String,
    safe_description: String,
    description_trust: ironclaw_host_api::capability::CapabilityDescriptionTrust,
    parameters_schema: serde_json::Value,
}

impl From<VisibleCapabilitySurface> for WireVisibleCapabilitySurface {
    fn from(surface: VisibleCapabilitySurface) -> Self {
        Self {
            version: surface.version,
            descriptors: surface
                .descriptors
                .into_iter()
                .map(WireCapabilityDescriptorView::from)
                .collect(),
            callable_capability_ids: surface.callable_capability_ids,
        }
    }
}

impl From<WireVisibleCapabilitySurface> for VisibleCapabilitySurface {
    fn from(surface: WireVisibleCapabilitySurface) -> Self {
        Self {
            version: surface.version,
            descriptors: surface
                .descriptors
                .into_iter()
                .map(CapabilityDescriptorView::from)
                .collect(),
            callable_capability_ids: surface.callable_capability_ids,
        }
    }
}

impl From<CapabilityDescriptorView> for WireCapabilityDescriptorView {
    fn from(descriptor: CapabilityDescriptorView) -> Self {
        Self {
            capability_id: descriptor.capability_id,
            provider: descriptor.provider,
            runtime: descriptor.runtime,
            safe_name: descriptor.safe_name,
            safe_description: descriptor.safe_description,
            description_trust: descriptor.description_trust,
            parameters_schema: descriptor.parameters_schema,
        }
    }
}

impl From<WireCapabilityDescriptorView> for CapabilityDescriptorView {
    fn from(descriptor: WireCapabilityDescriptorView) -> Self {
        Self {
            capability_id: descriptor.capability_id,
            provider: descriptor.provider,
            runtime: descriptor.runtime,
            safe_name: descriptor.safe_name,
            safe_description: descriptor.safe_description,
            description_trust: descriptor.description_trust,
            parameters_schema: descriptor.parameters_schema,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireLoopContextMessage {
    message_ref: Option<ironclaw_host_api::turn::LoopMessageRef>,
    role: String,
    safe_summary: String,
    compaction: Option<LoopContextCompactionMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireLoopContextSnippet {
    snippet_ref: String,
    model_content: String,
    safe_summary: String,
    metadata: Option<WireLoopContextSnippetMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireLoopContextSnippetMetadata {
    source_name: String,
    trust_level: SkillTrustLevel,
}

impl From<LoopContextBundle> for WireLoopContextBundle {
    fn from(bundle: LoopContextBundle) -> Self {
        Self {
            identity_messages: bundle
                .identity_messages
                .into_iter()
                .map(WireLoopContextMessage::from)
                .collect(),
            messages: bundle
                .messages
                .into_iter()
                .map(WireLoopContextMessage::from)
                .collect(),
            compaction_message_index: bundle.compaction_message_index,
            recent_window_truncation: bundle.recent_window_truncation,
            instruction_snippets: bundle
                .instruction_snippets
                .into_iter()
                .map(WireLoopContextSnippet::from)
                .collect(),
            memory_snippets: bundle
                .memory_snippets
                .into_iter()
                .map(WireLoopContextSnippet::from)
                .collect(),
        }
    }
}

impl From<WireLoopContextBundle> for LoopContextBundle {
    fn from(bundle: WireLoopContextBundle) -> Self {
        Self {
            identity_messages: bundle
                .identity_messages
                .into_iter()
                .map(LoopContextMessage::from)
                .collect(),
            messages: bundle
                .messages
                .into_iter()
                .map(LoopContextMessage::from)
                .collect(),
            compaction_message_index: bundle.compaction_message_index,
            recent_window_truncation: bundle.recent_window_truncation,
            instruction_snippets: bundle
                .instruction_snippets
                .into_iter()
                .map(LoopContextSnippet::from)
                .collect(),
            memory_snippets: bundle
                .memory_snippets
                .into_iter()
                .map(LoopContextSnippet::from)
                .collect(),
        }
    }
}

impl From<LoopContextMessage> for WireLoopContextMessage {
    fn from(message: LoopContextMessage) -> Self {
        Self {
            message_ref: message.message_ref,
            role: message.role,
            safe_summary: message.safe_summary,
            compaction: message.compaction,
        }
    }
}

impl From<WireLoopContextMessage> for LoopContextMessage {
    fn from(message: WireLoopContextMessage) -> Self {
        Self {
            message_ref: message.message_ref,
            role: message.role,
            safe_summary: message.safe_summary,
            compaction: message.compaction,
        }
    }
}

impl From<LoopContextSnippet> for WireLoopContextSnippet {
    fn from(snippet: LoopContextSnippet) -> Self {
        Self {
            snippet_ref: snippet.snippet_ref,
            model_content: snippet.model_content,
            safe_summary: snippet.safe_summary,
            metadata: snippet.metadata.map(WireLoopContextSnippetMetadata::from),
        }
    }
}

impl From<WireLoopContextSnippet> for LoopContextSnippet {
    fn from(snippet: WireLoopContextSnippet) -> Self {
        Self {
            snippet_ref: snippet.snippet_ref,
            model_content: snippet.model_content,
            safe_summary: snippet.safe_summary,
            metadata: snippet.metadata.map(LoopContextSnippetMetadata::from),
        }
    }
}

impl From<LoopContextSnippetMetadata> for WireLoopContextSnippetMetadata {
    fn from(metadata: LoopContextSnippetMetadata) -> Self {
        Self {
            source_name: metadata.source_name,
            trust_level: metadata.trust_level,
        }
    }
}

impl From<WireLoopContextSnippetMetadata> for LoopContextSnippetMetadata {
    fn from(metadata: WireLoopContextSnippetMetadata) -> Self {
        Self {
            source_name: metadata.source_name,
            trust_level: metadata.trust_level,
        }
    }
}
