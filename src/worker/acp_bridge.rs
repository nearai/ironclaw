//! ACP (Agent Client Protocol) bridge for sandboxed execution.
//!
//! Spawns any ACP-compliant agent (Goose, Codex, Gemini CLI, etc.) as a
//! subprocess inside a Docker container and communicates via the standard
//! ACP protocol (JSON-RPC over stdio). Agent output is translated into
//! IronClaw's `JobEventPayload` stream and posted to the orchestrator.
//!
//! Security model: the Docker container is the primary security boundary
//! (cap-drop ALL, non-root user, memory limits, network isolation).
//! Agent permissions are auto-approved since the container is isolated.
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ Docker Container                              │
//! │                                               │
//! │  ironclaw acp-bridge --job-id <uuid>          │
//! │    └─ spawns ACP agent subprocess             │
//! │    └─ ACP handshake (initialize + session)    │
//! │    └─ sends job description via prompt()      │
//! │    └─ translates ACP events → JobEventPayload │
//! │    └─ POSTs events to orchestrator            │
//! │    └─ polls for follow-up prompts             │
//! └──────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::{self as acp, Agent as _};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uuid::Uuid;

use crate::error::WorkerError;
use crate::worker::api::{CompletionReport, JobEventPayload, WorkerHttpClient};

/// Configuration for the ACP bridge runtime.
pub struct AcpBridgeConfig {
    pub job_id: Uuid,
    pub orchestrator_url: String,
    pub timeout: Duration,
    /// Command to spawn the ACP agent.
    pub agent_command: String,
    /// Arguments for the agent command.
    pub agent_args: Vec<String>,
    /// Extra environment variables for the agent process.
    pub agent_env: HashMap<String, String>,
}

/// The ACP bridge runtime.
pub struct AcpBridgeRuntime {
    config: AcpBridgeConfig,
    client: Arc<WorkerHttpClient>,
}

impl AcpBridgeRuntime {
    /// Create a new bridge runtime.
    ///
    /// Reads `IRONCLAW_WORKER_TOKEN` from the environment for auth.
    pub fn new(config: AcpBridgeConfig) -> Result<Self, WorkerError> {
        let client = Arc::new(WorkerHttpClient::from_env(
            config.orchestrator_url.clone(),
            config.job_id,
        )?);

        Ok(Self { config, client })
    }

    /// Run the bridge: fetch job, spawn ACP agent, stream events, handle follow-ups.
    pub async fn run(&self) -> Result<(), WorkerError> {
        // Fetch the job description from the orchestrator
        let job = self.client.get_job().await?;

        tracing::info!(
            job_id = %self.config.job_id,
            "Starting ACP bridge for: {}",
            truncate(&job.description, 100)
        );

        // Fetch credentials for injection into the spawned Command
        let credentials = self.client.fetch_credentials().await?;
        let mut extra_env = self.config.agent_env.clone();
        for cred in &credentials {
            extra_env.insert(cred.env_var.clone(), cred.value.clone());
        }
        if !credentials.is_empty() {
            tracing::info!(
                job_id = %self.config.job_id,
                "Fetched {} credential(s) for child process injection",
                credentials.len()
            );
        }

        // Report that we're running
        self.client
            .report_status(&crate::worker::api::StatusUpdate {
                state: "running".to_string(),
                message: Some(format!("Spawning ACP agent: {}", self.config.agent_command)),
                iteration: 0,
            })
            .await?;

        // Run the ACP session
        match self.run_acp_session(&job.description, &extra_env).await {
            Ok(()) => {
                self.client
                    .report_complete(&CompletionReport {
                        success: true,
                        message: Some("ACP agent session completed".to_string()),
                        iterations: 1,
                    })
                    .await?;
            }
            Err(e) => {
                tracing::error!(job_id = %self.config.job_id, "ACP session failed: {}", e);
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some(format!("ACP agent failed: {}", e)),
                        iterations: 1,
                    })
                    .await?;
            }
        }

        Ok(())
    }

    /// Spawn the ACP agent and run the protocol lifecycle.
    async fn run_acp_session(
        &self,
        prompt: &str,
        extra_env: &HashMap<String, String>,
    ) -> Result<(), WorkerError> {
        let mut cmd = Command::new(&self.config.agent_command);
        cmd.args(&self.config.agent_args);
        cmd.envs(extra_env);
        cmd.current_dir("/workspace")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| WorkerError::ExecutionFailed {
            reason: format!(
                "failed to spawn ACP agent '{}': {}",
                self.config.agent_command, e
            ),
        })?;

        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture ACP agent stdin".to_string(),
            })?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture ACP agent stdout".to_string(),
            })?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture ACP agent stderr".to_string(),
            })?;

        // Spawn stderr reader that forwards lines as status events
        let client_for_stderr = Arc::clone(&self.client);
        let job_id = self.config.job_id;
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(child_stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(job_id = %job_id, "acp agent stderr: {}", line);
                let payload = JobEventPayload {
                    event_type: "status".to_string(),
                    data: json!({ "message": line }),
                };
                client_for_stderr.post_event(&payload).await;
            }
        });

        // Run the ACP protocol inside a LocalSet (SDK futures are !Send)
        let client_for_acp = Arc::clone(&self.client);
        let prompt_owned = prompt.to_string();
        let job_id = self.config.job_id;
        let timeout = self.config.timeout;

        // Clone client for follow-up loop
        let client_for_followup = Arc::clone(&self.client);

        // Monitor the child process so the follow-up loop can exit if the agent dies.
        // The oneshot is Send, so it crosses the LocalSet boundary cleanly.
        let (child_exit_tx, child_exit_rx) = tokio::sync::oneshot::channel::<Option<i32>>();
        tokio::spawn(async move {
            let exit_code = match child.wait().await {
                Ok(status) => status.code(),
                Err(_) => None,
            };
            let _ = child_exit_tx.send(exit_code);
        });

        let local_set = tokio::task::LocalSet::new();
        let acp_result = local_set
            .run_until(async move {
                let outgoing = child_stdin.compat_write();
                let incoming = child_stdout.compat();

                // Create ACP connection
                let ironclaw_client = IronClawAcpClient::new(Arc::clone(&client_for_acp));

                let (conn, handle_io) =
                    acp::ClientSideConnection::new(ironclaw_client, outgoing, incoming, |fut| {
                        tokio::task::spawn_local(fut);
                    });
                tokio::task::spawn_local(handle_io);

                conn.initialize(ironclaw_init_request())
                    .await
                    .map_err(|e| WorkerError::ExecutionFailed {
                        reason: format!("ACP initialize failed: {}", e),
                    })?;

                tracing::info!(job_id = %job_id, "ACP handshake complete");

                // Create a new session
                let workspace = std::env::current_dir().unwrap_or_else(|_| "/workspace".into());
                let session_response = conn
                    .new_session(acp::NewSessionRequest::new(workspace))
                    .await
                    .map_err(|e| WorkerError::ExecutionFailed {
                        reason: format!("ACP new_session failed: {}", e),
                    })?;

                let session_id = session_response.session_id.clone();
                tracing::info!(job_id = %job_id, session_id = %session_id, "ACP session created");

                // Send the job description as a prompt
                let prompt_result = tokio::time::timeout(timeout, async {
                    conn.prompt(acp::PromptRequest::new(
                        session_id.clone(),
                        vec![prompt_owned.into()],
                    ))
                    .await
                })
                .await;

                let prompt_response = match prompt_result {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(e)) => {
                        return Err(WorkerError::ExecutionFailed {
                            reason: format!("ACP prompt failed: {}", e),
                        });
                    }
                    Err(_) => {
                        return Err(WorkerError::ExecutionFailed {
                            reason: "ACP prompt timed out".to_string(),
                        });
                    }
                };

                // Report prompt result
                let result_payload =
                    stop_reason_to_result(&prompt_response.stop_reason, &session_id.to_string());
                client_for_acp.post_event(&result_payload).await;

                // Follow-up loop: poll for prompts, send additional prompt() calls.
                // Exits when: orchestrator sends done, or agent process exits.
                let prompt_sender = ConnPromptSender { conn: &conn };
                run_follow_up_loop(
                    &client_for_followup,
                    &prompt_sender,
                    &client_for_acp,
                    &session_id,
                    child_exit_rx,
                    job_id,
                )
                .await?;

                Ok::<(), WorkerError>(())
            })
            .await;

        // Wait for stderr reader to finish
        let _ = stderr_handle.await;

        acp_result
    }
}

// ==================== ACP Client trait implementation ====================

/// Sink for ACP events translated from session notifications.
///
/// The bridge posts events to the orchestrator via HTTP; the CLI test
/// command prints them to stdout. Both share the same `IronClawAcpClient`.
pub(crate) trait AcpEventSink: 'static {
    fn emit_event(&self, payload: &JobEventPayload) -> impl std::future::Future<Output = ()>;
}

impl AcpEventSink for Arc<WorkerHttpClient> {
    async fn emit_event(&self, payload: &JobEventPayload) {
        self.post_event(payload).await;
    }
}

/// Source of follow-up prompts from the orchestrator.
pub(crate) trait FollowUpPromptSource {
    fn poll_prompt(
        &self,
    ) -> impl std::future::Future<Output = Result<Option<crate::worker::api::PromptResponse>, WorkerError>>;
}

impl FollowUpPromptSource for Arc<WorkerHttpClient> {
    async fn poll_prompt(&self) -> Result<Option<crate::worker::api::PromptResponse>, WorkerError> {
        WorkerHttpClient::poll_prompt(self).await
    }
}

/// Sends a follow-up prompt to the ACP agent.
trait AcpPromptSender {
    fn send_prompt(
        &self,
        session_id: acp::SessionId,
        content: String,
    ) -> impl std::future::Future<Output = acp::Result<acp::PromptResponse>>;
}

/// IronClaw's implementation of the ACP Client trait.
///
/// Handles callbacks from the agent: session notifications (streaming output)
/// and permission requests (auto-approved). Generic over the event sink so
/// both the container bridge and CLI test command can reuse it.
pub(crate) struct IronClawAcpClient<S: AcpEventSink> {
    sink: S,
}

impl<S: AcpEventSink> IronClawAcpClient<S> {
    pub(crate) fn new(sink: S) -> Self {
        Self { sink }
    }
}

#[async_trait::async_trait(?Send)]
impl<S: AcpEventSink> acp::Client for IronClawAcpClient<S> {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        // Auto-approve by selecting the first option — Docker container is the
        // security boundary, so we trust the agent to operate freely.
        let Some(first_option) = args.options.first() else {
            return Err(acp::Error::invalid_params());
        };
        let option_id = first_option.option_id.clone();
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(option_id)),
        ))
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        if let Some(payload) = session_update_to_payload(&args.update) {
            self.sink.emit_event(&payload).await;
        }
        Ok(())
    }
}

/// Build the standard IronClaw ACP initialization request.
pub(crate) fn ironclaw_init_request() -> acp::InitializeRequest {
    acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_info(
        acp::Implementation::new("ironclaw", env!("CARGO_PKG_VERSION")).title("IronClaw"),
    )
}

// ==================== Follow-up loop ====================

/// Wrapper that implements `AcpPromptSender` for an ACP `ClientSideConnection`.
struct ConnPromptSender<'a, C: acp::Agent> {
    conn: &'a C,
}

impl<C: acp::Agent + 'static> AcpPromptSender for ConnPromptSender<'_, C> {
    async fn send_prompt(
        &self,
        session_id: acp::SessionId,
        content: String,
    ) -> acp::Result<acp::PromptResponse> {
        self.conn
            .prompt(acp::PromptRequest::new(session_id, vec![content.into()]))
            .await
    }
}

/// Run the follow-up prompt loop.
///
/// Polls for follow-up prompts from the orchestrator, sends them to the ACP
/// agent, and translates results into events. Returns `Err` if any follow-up
/// prompt fails, so the caller can report `success: false`.
async fn run_follow_up_loop(
    prompt_source: &impl FollowUpPromptSource,
    agent: &impl AcpPromptSender,
    sink: &impl AcpEventSink,
    session_id: &acp::SessionId,
    mut child_exit_rx: tokio::sync::oneshot::Receiver<Option<i32>>,
    job_id: Uuid,
) -> Result<(), WorkerError> {
    let session_id_str = session_id.to_string();
    loop {
        let backoff = match prompt_source.poll_prompt().await {
            Ok(Some(follow_up)) => {
                if follow_up.done {
                    tracing::debug!(job_id = %job_id, "Orchestrator signaled done");
                    break;
                }
                tracing::debug!(job_id = %job_id, "Got follow-up prompt");

                let follow_result = agent
                    .send_prompt(session_id.clone(), follow_up.content)
                    .await;

                match follow_result {
                    Ok(resp) => {
                        let payload = stop_reason_to_result(&resp.stop_reason, &session_id_str);
                        sink.emit_event(&payload).await;
                    }
                    Err(e) => {
                        let msg = format!("Follow-up prompt failed: {e}");
                        tracing::error!(job_id = %job_id, "{}", msg);
                        sink.emit_event(&JobEventPayload {
                            event_type: "status".to_string(),
                            data: json!({ "message": msg }),
                        })
                        .await;
                        return Err(WorkerError::ExecutionFailed { reason: msg });
                    }
                }
                continue;
            }
            Ok(None) => Duration::from_secs(2),
            Err(e) => {
                tracing::warn!(job_id = %job_id, "Prompt polling error: {}", e);
                Duration::from_secs(5)
            }
        };

        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            exit_code = &mut child_exit_rx => {
                let code = exit_code.ok().flatten();
                tracing::debug!(
                    job_id = %job_id,
                    exit_code = ?code,
                    "ACP agent process exited, ending follow-up loop"
                );
                break;
            }
        }
    }
    Ok(())
}

// ==================== Event translation ====================

/// Convert an ACP `SessionUpdate` into an IronClaw `JobEventPayload`.
fn session_update_to_payload(update: &acp::SessionUpdate) -> Option<JobEventPayload> {
    match update {
        acp::SessionUpdate::AgentMessageChunk(chunk) => text_from_content_block(&chunk.content)
            .map(|text| JobEventPayload {
                event_type: "message".to_string(),
                data: json!({
                    "role": "assistant",
                    "content": text,
                }),
            }),
        acp::SessionUpdate::AgentThoughtChunk(chunk) => text_from_content_block(&chunk.content)
            .map(|text| JobEventPayload {
                event_type: "status".to_string(),
                data: json!({
                    "message": text,
                    "type": "thought",
                }),
            }),
        acp::SessionUpdate::ToolCall(tool_call) => Some(JobEventPayload {
            event_type: "tool_use".to_string(),
            data: json!({
                "tool_name": tool_call.title,
                "tool_use_id": tool_call.tool_call_id.to_string(),
            }),
        }),
        acp::SessionUpdate::ToolCallUpdate(update) => Some(JobEventPayload {
            event_type: "tool_result".to_string(),
            data: json!({
                "tool_use_id": update.tool_call_id.to_string(),
            }),
        }),
        _ => Some(JobEventPayload {
            event_type: "status".to_string(),
            data: json!({ "message": "ACP session update" }),
        }),
    }
}

/// Extract text from a `ContentBlock`, returning `None` for non-text blocks.
fn text_from_content_block(block: &acp::ContentBlock) -> Option<&str> {
    match block {
        acp::ContentBlock::Text(text_content) => Some(&text_content.text),
        _ => None,
    }
}

/// Convert an ACP `StopReason` into a "result" `JobEventPayload`.
fn stop_reason_to_result(reason: &acp::StopReason, session_id: &str) -> JobEventPayload {
    let (status, message) = match reason {
        acp::StopReason::EndTurn => ("completed", "Agent completed successfully"),
        acp::StopReason::MaxTokens => ("error", "Agent reached max tokens"),
        acp::StopReason::MaxTurnRequests => ("error", "Agent reached max turn requests"),
        acp::StopReason::Refusal => ("error", "Agent refused to continue"),
        acp::StopReason::Cancelled => ("cancelled", "Agent was cancelled"),
        _ => ("completed", "Agent finished"),
    };
    JobEventPayload {
        event_type: "result".to_string(),
        data: json!({
            "status": status,
            "session_id": session_id,
            "message": message,
        }),
    }
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_update_agent_message_text() {
        let update = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp::ContentBlock::Text(acp::TextContent::new("Hello world")),
        ));
        let payload = session_update_to_payload(&update).unwrap();
        assert_eq!(payload.event_type, "message");
        assert_eq!(payload.data["role"], "assistant");
        assert_eq!(payload.data["content"], "Hello world");
    }

    #[test]
    fn test_session_update_agent_thought_text() {
        let update = acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
            acp::ContentBlock::Text(acp::TextContent::new("Thinking...")),
        ));
        let payload = session_update_to_payload(&update).unwrap();
        assert_eq!(payload.event_type, "status");
        assert_eq!(payload.data["type"], "thought");
        assert_eq!(payload.data["message"], "Thinking...");
    }

    #[test]
    fn test_session_update_agent_message_image_ignored() {
        let update = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp::ContentBlock::Image(acp::ImageContent::new("base64data", "image/png")),
        ));
        assert!(session_update_to_payload(&update).is_none());
    }

    #[test]
    fn test_stop_reason_end_turn() {
        let payload = stop_reason_to_result(&acp::StopReason::EndTurn, "sid-1");
        assert_eq!(payload.event_type, "result");
        assert_eq!(payload.data["status"], "completed");
    }

    #[test]
    fn test_stop_reason_max_tokens() {
        let payload = stop_reason_to_result(&acp::StopReason::MaxTokens, "sid-1");
        assert_eq!(payload.data["status"], "error");
    }

    #[test]
    fn test_stop_reason_cancelled() {
        let payload = stop_reason_to_result(&acp::StopReason::Cancelled, "sid-1");
        assert_eq!(payload.data["status"], "cancelled");
    }

    #[test]
    fn test_stop_reason_refusal() {
        let payload = stop_reason_to_result(&acp::StopReason::Refusal, "sid-1");
        assert_eq!(payload.data["status"], "error");
        assert_eq!(payload.data["message"], "Agent refused to continue");
    }

    #[test]
    fn test_session_update_tool_call() {
        let update = acp::SessionUpdate::ToolCall(acp::ToolCall::new("tc-1", "Running tests"));
        let payload = session_update_to_payload(&update).unwrap();
        assert_eq!(payload.event_type, "tool_use");
        assert_eq!(payload.data["tool_name"], "Running tests");
        assert_eq!(payload.data["tool_use_id"], "tc-1");
    }

    #[test]
    fn test_session_update_tool_call_update() {
        let fields = acp::ToolCallUpdateFields::new();
        let update = acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new("tc-1", fields));
        let payload = session_update_to_payload(&update).unwrap();
        assert_eq!(payload.event_type, "tool_result");
        assert_eq!(payload.data["tool_use_id"], "tc-1");
    }

    #[test]
    fn test_session_update_thought_image_ignored() {
        let update = acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
            acp::ContentBlock::Image(acp::ImageContent::new("data", "image/png")),
        ));
        assert!(session_update_to_payload(&update).is_none());
    }

    #[test]
    fn test_stop_reason_max_turn_requests() {
        let payload = stop_reason_to_result(&acp::StopReason::MaxTurnRequests, "sid-1");
        assert_eq!(payload.data["status"], "error");
        assert_eq!(payload.data["message"], "Agent reached max turn requests");
    }

    #[test]
    fn test_stop_reason_includes_session_id() {
        let payload = stop_reason_to_result(&acp::StopReason::EndTurn, "my-session-42");
        assert_eq!(payload.data["session_id"], "my-session-42");
    }

    #[test]
    fn test_text_from_content_block_text() {
        let block = acp::ContentBlock::Text(acp::TextContent::new("hello"));
        assert_eq!(text_from_content_block(&block), Some("hello"));
    }

    #[test]
    fn test_text_from_content_block_image_returns_none() {
        let block = acp::ContentBlock::Image(acp::ImageContent::new("data", "image/png"));
        assert!(text_from_content_block(&block).is_none());
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_truncate_multibyte_safe() {
        // 2-byte UTF-8 char: "é" is 0xC3 0xA9
        let s = "café";
        assert_eq!(truncate(s, 3), "caf"); // doesn't split the é
        assert_eq!(truncate(s, 5), "café"); // includes full char
    }

    // ==================== Follow-up loop stubs & tests ====================

    use crate::worker::api::PromptResponse;
    use std::sync::Mutex;

    /// Stub event sink that collects emitted events for assertion.
    struct CollectingSink {
        events: Mutex<Vec<JobEventPayload>>,
    }

    impl CollectingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<JobEventPayload> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AcpEventSink for CollectingSink {
        async fn emit_event(&self, payload: &JobEventPayload) {
            self.events.lock().unwrap().push(payload.clone());
        }
    }

    /// Stub prompt source that yields a sequence of responses then returns None.
    struct StubPromptSource {
        responses: Mutex<Vec<Result<Option<PromptResponse>, WorkerError>>>,
    }

    impl StubPromptSource {
        fn new(mut responses: Vec<Result<Option<PromptResponse>, WorkerError>>) -> Self {
            responses.reverse(); // so we can pop (FIFO)
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    impl FollowUpPromptSource for StubPromptSource {
        async fn poll_prompt(&self) -> Result<Option<PromptResponse>, WorkerError> {
            self.responses.lock().unwrap().pop().unwrap_or(Ok(None))
        }
    }

    /// Stub ACP prompt sender that returns pre-configured results.
    struct StubAcpPromptSender {
        results: Mutex<Vec<acp::Result<acp::PromptResponse>>>,
    }

    impl StubAcpPromptSender {
        fn new(mut results: Vec<acp::Result<acp::PromptResponse>>) -> Self {
            results.reverse(); // so we can pop (FIFO)
            Self {
                results: Mutex::new(results),
            }
        }
    }

    impl AcpPromptSender for StubAcpPromptSender {
        async fn send_prompt(
            &self,
            _session_id: acp::SessionId,
            _content: String,
        ) -> acp::Result<acp::PromptResponse> {
            self.results
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }
    }

    /// Regression test for #1915: follow-up prompt failure must return Err
    /// so that run() reports success: false.
    #[tokio::test]
    async fn follow_up_prompt_failure_returns_error() {
        let sink = CollectingSink::new();

        let prompt_source = StubPromptSource::new(vec![Ok(Some(PromptResponse {
            content: "follow up".to_string(),
            done: false,
        }))]);

        let agent = StubAcpPromptSender::new(vec![Err(acp::Error::internal_error())]);

        let (_tx, rx) = tokio::sync::oneshot::channel();
        let session_id = acp::SessionId::new("test-session");

        let result =
            run_follow_up_loop(&prompt_source, &agent, &sink, &session_id, rx, Uuid::nil()).await;

        assert!(
            result.is_err(),
            "Follow-up prompt failure must propagate as Err"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, WorkerError::ExecutionFailed { .. }),
            "Error must be ExecutionFailed, got: {err}"
        );

        // Status event should still have been posted before the error
        let events = sink.events();
        assert!(
            events.iter().any(|e| e.event_type == "status"
                && e.data["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Follow-up prompt failed")),
            "Should post status event before returning error"
        );
    }

    /// Verify that a successful follow-up followed by orchestrator "done"
    /// signal returns Ok.
    #[tokio::test]
    async fn follow_up_prompt_success_then_done_returns_ok() {
        let sink = CollectingSink::new();

        let prompt_source = StubPromptSource::new(vec![
            Ok(Some(PromptResponse {
                content: "do more work".to_string(),
                done: false,
            })),
            Ok(Some(PromptResponse {
                content: String::new(),
                done: true,
            })),
        ]);

        let agent =
            StubAcpPromptSender::new(vec![Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))]);

        let (_tx, rx) = tokio::sync::oneshot::channel();
        let session_id = acp::SessionId::new("test-session");

        let result =
            run_follow_up_loop(&prompt_source, &agent, &sink, &session_id, rx, Uuid::nil()).await;

        assert!(
            result.is_ok(),
            "Successful follow-up then done should be Ok"
        );

        let events = sink.events();
        assert!(
            events.iter().any(|e| e.event_type == "result"),
            "Should emit result event for successful prompt"
        );
    }

    /// Verify the loop recovers from a transient polling error and
    /// processes the next prompt successfully.
    #[tokio::test(start_paused = true)]
    async fn follow_up_loop_recovers_from_transient_poll_error() {
        let sink = CollectingSink::new();

        // First poll returns an error, second returns a real prompt, third signals done.
        let prompt_source = StubPromptSource::new(vec![
            Err(WorkerError::ConnectionFailed {
                url: "http://test".to_string(),
                reason: "transient".to_string(),
            }),
            Ok(Some(PromptResponse {
                content: "do work".to_string(),
                done: false,
            })),
            Ok(Some(PromptResponse {
                content: String::new(),
                done: true,
            })),
        ]);

        let agent =
            StubAcpPromptSender::new(vec![Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))]);

        let (_tx, rx) = tokio::sync::oneshot::channel();
        let session_id = acp::SessionId::new("test-session");

        let result =
            run_follow_up_loop(&prompt_source, &agent, &sink, &session_id, rx, Uuid::nil()).await;

        assert!(
            result.is_ok(),
            "Loop should recover from transient poll error"
        );

        let events = sink.events();
        assert!(
            events.iter().any(|e| e.event_type == "result"),
            "Should emit result event after recovery"
        );
    }
}
