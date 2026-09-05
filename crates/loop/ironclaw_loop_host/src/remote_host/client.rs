use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::resolution::{Resolution, ResolutionBatch};
use ironclaw_loop_contracts::*;
use ironclaw_turns::LoopMessageRef;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::remote_host::protocol::*;
use crate::remote_host::server::wire_error_to_host_error;

struct StdioRpcClient<R, W> {
    reader: tokio::sync::Mutex<R>,
    writer: tokio::sync::Mutex<W>,
    exchange: tokio::sync::Mutex<()>,
    next_id: std::sync::atomic::AtomicU64,
    cancellation: Mutex<Option<LoopCancellationSignal>>,
    tool_definitions: Vec<ProviderToolDefinition>,
    visible: Mutex<Option<VisibleCapabilitySurface>>,
}

impl<R, W> StdioRpcClient<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    fn new(
        reader: R,
        writer: W,
        tool_definitions: Vec<ProviderToolDefinition>,
        visible: Option<VisibleCapabilitySurface>,
    ) -> Self {
        Self {
            reader: tokio::sync::Mutex::new(reader),
            writer: tokio::sync::Mutex::new(writer),
            exchange: tokio::sync::Mutex::new(()),
            next_id: std::sync::atomic::AtomicU64::new(1),
            cancellation: Mutex::new(None),
            tool_definitions,
            visible: Mutex::new(visible),
        }
    }

    async fn call_raw(&self, call: HostCall) -> Result<serde_json::Value, WireError> {
        // One reader carries responses for every request. Serialize the full
        // exchange so concurrent canonical batch calls cannot consume each
        // other's response frame.
        let _exchange = self.exchange.lock().await;
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let request = encode(&WorkerFrame::HostRequest(Box::new(HostRequestFrame {
            id,
            call,
        })))
        .map_err(WireError::Host)?;
        {
            let mut writer = self.writer.lock().await;
            write_framed(&mut *writer, &request)
                .await
                .map_err(WireError::Host)?;
        }

        loop {
            let bytes = {
                let mut reader = self.reader.lock().await;
                read_framed(&mut *reader).await.map_err(WireError::Host)?
            }
            .ok_or_else(|| {
                WireError::Host(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "loop worker host pipe closed",
                ))
            })?;
            match decode::<HostFrame>(&bytes).map_err(WireError::Host)? {
                HostFrame::Cancel(signal) => {
                    if let Ok(mut cancellation) = self.cancellation.lock() {
                        *cancellation = Some(signal);
                    }
                }
                HostFrame::HostResponse(response) if response.id == id => {
                    return response.result;
                }
                HostFrame::HostResponse(_) | HostFrame::Bootstrap(_) | HostFrame::OutcomeAck => {
                    return Err(WireError::Host(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "loop worker received an unexpected host frame",
                    )));
                }
            }
        }
    }

    async fn call<T: DeserializeOwned>(&self, call: HostCall) -> Result<T, AgentLoopHostError> {
        let value = self.call_raw(call).await.map_err(|error| match error {
            WireError::Host(error) => error,
            WireError::Compaction(error) => {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
            }
            WireError::Protocol(detail) => {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, detail)
            }
        })?;
        serde_json::from_value(value).map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("host response shape is invalid: {error}"),
            )
        })
    }
}

/// Worker-side implementation of the canonical loop's one host membrane.
pub struct RemoteAgentLoopDriverHost<R, W> {
    run_context: LoopRunContext,
    rpc: Arc<StdioRpcClient<R, W>>,
}

impl<R, W> RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    fn new(run_context: LoopRunContext, rpc: StdioRpcClient<R, W>) -> Self {
        Self {
            run_context,
            rpc: Arc::new(rpc),
        }
    }
    pub async fn write_outcome(
        &self,
        outcome: LoopWorkerOutcome,
    ) -> Result<(), AgentLoopHostError> {
        let frame = encode(&WorkerFrame::Outcome(outcome))?;
        {
            let mut writer = self.rpc.writer.lock().await;
            write_framed(&mut *writer, &frame).await?;
        }
        let bytes = {
            let mut reader = self.rpc.reader.lock().await;
            read_framed(&mut *reader).await?
        }
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop worker host pipe closed before outcome acknowledgement",
            )
        })?;
        match decode::<HostFrame>(&bytes)? {
            HostFrame::OutcomeAck => Ok(()),
            _ => Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "loop worker expected outcome acknowledgement",
            )),
        }
    }
}

impl<R, W> RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    /// Worker-side `HostCall::ResolveMessages`. Only meaningful for a worker
    /// bootstrapped `WorkerContentVisibility::Resolved`; a blind host denies
    /// the call with `PolicyDenied`.
    pub async fn resolve_messages(
        &self,
        messages: Vec<LoopModelMessage>,
    ) -> Result<Vec<WireResolvedModelMessage>, AgentLoopHostError> {
        self.rpc
            .call(HostCall::ResolveMessages(ResolveMessagesRequest {
                messages,
            }))
            .await
    }
}

impl<R, W> LoopRunInfoPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl<R, W> LoopContextPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        let bundle: WireLoopContextBundle = self.rpc.call(HostCall::LoadContext(request)).await?;
        Ok(bundle.into())
    }
}

#[async_trait]
impl<R, W> LoopPromptPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.rpc.call(HostCall::BuildPrompt(request)).await
    }
}

#[async_trait]
impl<R, W> LoopInputPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.rpc.call(HostCall::PollInputs { after, limit }).await
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.rpc.call(HostCall::AckInputs(tokens)).await
    }
}

#[async_trait]
impl<R, W> LoopModelPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.rpc.call(HostCall::StreamModel(request)).await
    }
}

#[async_trait]
impl<R, W> LoopCapabilityPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(self.rpc.tool_definitions.clone())
    }

    fn current_visible_capabilities(
        &self,
    ) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        self.rpc
            .visible
            .lock()
            .map(|surface| surface.clone())
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "remote capability surface cache is unavailable",
                )
            })
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.rpc
            .call(HostCall::RegisterProviderToolCall(request))
            .await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let value = self
            .rpc
            .call_raw(HostCall::VisibleCapabilities(request))
            .await
            .map_err(wire_error_to_host_error)?;
        let wire: WireVisibleCapabilitySurface =
            serde_json::from_value(value).map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("visible capability response shape is invalid: {error}"),
                )
            })?;
        let surface: VisibleCapabilitySurface = wire.into();
        if let Ok(mut visible) = self.rpc.visible.lock() {
            *visible = Some(surface.clone());
        }
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.rpc.call(HostCall::InvokeCapability(request)).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        self.rpc
            .call(HostCall::InvokeCapabilityBatch(request))
            .await
    }
}

#[async_trait]
impl<R, W> LoopTranscriptPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn begin_assistant_draft(
        &self,
        request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        self.rpc.call(HostCall::BeginAssistantDraft(request)).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        self.rpc.call(HostCall::UpdateAssistantDraft(request)).await
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        self.rpc
            .call(HostCall::FinalizeAssistantMessage(request))
            .await
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        self.rpc
            .call(HostCall::AppendCapabilityResultRef(Box::new(request)))
            .await
    }
}

#[async_trait]
impl<R, W> LoopCheckpointPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<ironclaw_host_api::turn::TurnCheckpointId, AgentLoopHostError> {
        self.rpc.call(HostCall::Checkpoint(request)).await
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.rpc
            .call(HostCall::StageCheckpointPayload(request))
            .await
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        let payload: WireLoadedCheckpointPayload = self
            .rpc
            .call(HostCall::LoadCheckpointPayload(request))
            .await?;
        payload.try_into()
    }
}

#[async_trait]
impl<R, W> LoopProgressPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        self.rpc.call(HostCall::EmitProgress(event)).await
    }
}

#[async_trait]
impl<R, W> LoopCompactionPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    async fn compact_loop_context(
        &self,
        request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        match self.rpc.call_raw(HostCall::Compact(request)).await {
            Ok(value) => {
                serde_json::from_value(value).map_err(|_| LoopCompactionError::InferenceFailed {
                    safe_summary: LoopSafeSummary::tool_failure_details_redacted(),
                })
            }
            Err(WireError::Compaction(error)) => Err(error),
            Err(WireError::Host(error)) => Err(LoopCompactionError::InferenceFailed {
                safe_summary: LoopSafeSummary::new(error.safe_summary)
                    .unwrap_or_else(|_| LoopSafeSummary::tool_failure_details_redacted()),
            }),
            Err(WireError::Protocol(_)) => Err(LoopCompactionError::InferenceFailed {
                safe_summary: LoopSafeSummary::tool_failure_details_redacted(),
            }),
        }
    }
}

#[async_trait]
impl<R, W> LoopCancellationPort for RemoteAgentLoopDriverHost<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        self.rpc
            .cancellation
            .lock()
            .ok()
            .and_then(|signal| signal.clone())
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        loop {
            if let Some(signal) = self.observe_cancellation() {
                return signal;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

pub async fn read_worker_bootstrap<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<LoopWorkerBootstrap, AgentLoopHostError> {
    let bytes = read_framed(reader).await?.ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "loop worker bootstrap pipe closed",
        )
    })?;
    match decode::<HostFrame>(&bytes)? {
        HostFrame::Bootstrap(bootstrap) if bootstrap.wire_version == LOOP_WORKER_WIRE_VERSION => {
            Ok(*bootstrap)
        }
        HostFrame::Bootstrap(_) => Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "loop worker wire version mismatch",
        )),
        _ => Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "loop worker expected bootstrap as its first frame",
        )),
    }
}

pub fn remote_host_from_stdio(
    bootstrap: &LoopWorkerBootstrap,
) -> Result<RemoteAgentLoopDriverHost<tokio::io::Stdin, tokio::io::Stdout>, AgentLoopHostError> {
    let visible = bootstrap
        .current_visible_capabilities
        .clone()
        .map(serde_json::from_value::<WireVisibleCapabilitySurface>)
        .transpose()
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                format!("visible capability bootstrap shape is invalid: {error}"),
            )
        })?
        .map(VisibleCapabilitySurface::from);
    Ok(RemoteAgentLoopDriverHost::new(
        bootstrap.run_context.clone(),
        StdioRpcClient::new(
            tokio::io::stdin(),
            tokio::io::stdout(),
            bootstrap.tool_definitions.clone(),
            visible,
        ),
    ))
}

async fn write_framed<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
) -> Result<(), AgentLoopHostError> {
    let length = u32::try_from(bytes.len()).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "loop worker frame length cannot be represented",
        )
    })?;
    writer.write_u32(length).await.map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("loop worker pipe write failed: {error}"),
        )
    })?;
    writer.write_all(bytes).await.map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("loop worker pipe write failed: {error}"),
        )
    })?;
    writer.flush().await.map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("loop worker pipe flush failed: {error}"),
        )
    })
}

async fn read_framed<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, AgentLoopHostError> {
    let length = match reader.read_u32().await {
        Ok(length) => usize::try_from(length).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "loop worker frame length is invalid",
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("loop worker pipe read failed: {error}"),
            ));
        }
    };
    if length > LOOP_WORKER_MAX_FRAME_BYTES {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "loop worker frame exceeds the configured byte limit",
        ));
    }
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes).await.map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("loop worker frame read failed: {error}"),
        )
    })?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concurrent_calls_cannot_consume_each_others_responses() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client_io);
        let (mut server_read, mut server_write) = tokio::io::split(server_io);
        let rpc = Arc::new(StdioRpcClient::new(
            client_read,
            client_write,
            Vec::new(),
            None,
        ));
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let bytes = read_framed(&mut server_read)
                    .await
                    .expect("request frame reads")
                    .expect("request frame exists");
                let WorkerFrame::HostRequest(request) =
                    decode::<WorkerFrame>(&bytes).expect("request decodes")
                else {
                    panic!("expected host request");
                };
                let response = encode(&HostFrame::HostResponse(HostResponseFrame {
                    id: request.id,
                    result: Ok(serde_json::json!(request.id)),
                }))
                .expect("response encodes");
                write_framed(&mut server_write, &response)
                    .await
                    .expect("response writes");
            }
        });

        let (first, second) = tokio::join!(
            rpc.call_raw(HostCall::AckInputs(Vec::new())),
            rpc.call_raw(HostCall::AckInputs(Vec::new())),
        );
        server.await.expect("server task");
        let mut ids = vec![
            first.expect("first response").as_u64().expect("first id"),
            second
                .expect("second response")
                .as_u64()
                .expect("second id"),
        ];
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);
    }
}
