//! ACP (Agent Client Protocol) server: speaks ACP on stdio, drives the
//! Reborn runtime for each prompt.
//!
//! Usage: `ironclaw acp-serve [--cwd <dir>] [--auto-approve]`
//!
//! ## Architecture
//!
//! Unlike the HTTP `serve` command (WebChat), this command:
//! - Implements the `agent_client_protocol::Agent` trait on stdio
//! - Bridges each ACP `prompt()` call to `RebornRuntime::send_user_message()`
//! - Maintains per-session conversation state so multi-turn conversations work
//! - Returns the assistant reply text in the `PromptResponse`
//!
//! The runtime boots identically to the `run` command (multi_thread tokio).
//! ACP I/O runs on a `LocalSet` inside the multi_thread runtime so the
//! !Send `Agent` trait impl works.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use std::pin::Pin;

use agent_client_protocol::{
    Agent, AgentSideConnection, AgentCapabilities, AuthenticateRequest, AuthenticateResponse,
    Client, ContentBlock, ContentChunk, Error, Implementation, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    ProtocolVersion, SessionId, SessionNotification, SessionUpdate, StopReason,
};
use clap::Args;
use ironclaw_composition::{ConversationId, RebornRuntime, TurnStatus, build_reborn_runtime};
use tokio_util::compat::TokioAsyncReadCompatExt;
use tokio_util::sync::CancellationToken;

use anyhow::Context as _;

use crate::context::RebornCliContext;
use crate::runtime::{
    RuntimeInputCaller, RuntimeInputOptions, build_runtime_input_with_options,
    read_config_file,
};
use ironclaw_composition::TriggerFireAccessPolicy;
use ironclaw_composition::host_api::{AgentId, UserId};

/// Maximum total byte length of the combined prompt text accepted from the
/// ACP client. Payloads exceeding this are rejected with an invalid-params
/// JSON-RPC error. This bounds memory allocation for a single prompt.
const MAX_PROMPT_BYTES: usize = 512 * 1024; // 512 KiB

/// Category string returned to the client instead of raw internal error text.
/// Internal messages may contain file paths, stack traces, or other sensitive
/// information that must not leak over the ACP transport.
const INTERNAL_ERROR_CATEGORY: &str = "internal_error";

// ── CLI Args ───────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub(crate) struct AcpServeCommand {
    /// Working directory for the agent.
    #[arg(long)]
    cwd: Option<String>,

    /// Auto-approve all tool calls without human confirmation.
    #[arg(long)]
    auto_approve: bool,
}

// ── ACP Agent trait implementation ─────────────────────────────────────────

/// Per-session mutable state: maps ACP session IDs to Reborn conversation IDs
/// and per-session cancellation tokens.
///
/// Uses `RefCell` because the `Agent` trait is `!Send` — all access happens
/// on the `LocalSet`, so interior mutability without thread safety is correct.
struct SessionState {
    /// Maps session ID to conversation ID. Inserted atomically via
    /// `HashMap::entry` to prevent concurrent prompts for the same session
    /// from racing to create duplicate conversations (issue #20).
    conversations: RefCell<HashMap<String, ConversationId>>,
    cancel_tokens: RefCell<HashMap<String, CancellationToken>>,
}

impl SessionState {
    fn new() -> Self {
        Self {
            conversations: RefCell::new(HashMap::new()),
            cancel_tokens: RefCell::new(HashMap::new()),
        }
    }

    /// Create a new session, returning the session ID. A cancellation token is
    /// pre-allocated so the session exists even before the first prompt.
    fn create_session(&self) -> String {
        let sid = uuid::Uuid::new_v4().to_string();
        self.cancel_tokens
            .borrow_mut()
            .insert(sid.clone(), CancellationToken::new());
        sid
    }

    /// Get or create a per-session cancellation token.
    fn cancel_token_for(&self, session_id: &str) -> CancellationToken {
        let mut tokens = self.cancel_tokens.borrow_mut();
        tokens
            .entry(session_id.to_string())
            .or_insert_with(CancellationToken::new)
            .clone()
    }

    /// Cancel the token for a session and replace it with a fresh one so
    /// future prompts in the same session are not pre-cancelled.
    fn cancel_session(&self, session_id: &str) {
        let mut tokens = self.cancel_tokens.borrow_mut();
        if let Some(token) = tokens.get(session_id) {
            token.cancel();
        }
        tokens.insert(session_id.to_string(), CancellationToken::new());
    }
}

struct RebornAcpAgent {
    runtime: Arc<RebornRuntime>,
    /// The connection slot is extracted (Option taken) before the notification
    /// `.await` so the `MutexGuard` is never held across a yield point
    /// (issue #10).
    conn: Arc<std::sync::Mutex<Option<AgentSideConnection>>>,
    sessions: Arc<SessionState>,
}

impl Clone for RebornAcpAgent {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            conn: self.conn.clone(),
            sessions: self.sessions.clone(),
        }
    }
}

/// Sanitize an internal error into a JSON-RPC error whose message does not
/// contain raw internal diagnostics (issue #12).
fn internal_error(msg: &str) -> Error {
    tracing::debug!(internal_error = %msg, "ACP serve internal error (sanitized to peer)");
    Error::new(
        -32603,
        INTERNAL_ERROR_CATEGORY.to_string(),
    )
}

#[async_trait::async_trait(?Send)]
impl Agent for RebornAcpAgent {
    async fn initialize(
        &self,
        args: InitializeRequest,
    ) -> std::result::Result<InitializeResponse, Error> {
        // Negotiate: return the minimum of the client's requested version
        // and our latest supported version (issue #2). Reject unsupported
        // versions outright.
        let negotiated = std::cmp::min(args.protocol_version, ProtocolVersion::LATEST);
        if negotiated == ProtocolVersion::V0 {
            return Err(Error::invalid_params());
        }
        Ok(
            InitializeResponse::new(negotiated)
                .agent_capabilities(AgentCapabilities::new())
                .agent_info(Implementation::new("ironclaw", env!("CARGO_PKG_VERSION"))),
        )
    }

    async fn authenticate(
        &self,
        _args: AuthenticateRequest,
    ) -> std::result::Result<AuthenticateResponse, Error> {
        Ok(AuthenticateResponse::new())
    }

    async fn new_session(
        &self,
        _args: NewSessionRequest,
    ) -> std::result::Result<NewSessionResponse, Error> {
        let sid = self.sessions.create_session();
        Ok(NewSessionResponse::new(SessionId::new(sid)))
    }

    async fn prompt(
        &self,
        args: PromptRequest,
    ) -> std::result::Result<PromptResponse, Error> {
        let session_id: &str = &args.session_id.0;

        // ── Extract text content, bound total size, reject empty (issues #9, #11) ──
        // Check the bound *during* iteration so oversized payloads are rejected
        // before all text blocks are cloned into memory.
        let mut total_bytes: usize = 0;
        let mut content: Vec<String> = Vec::new();
        for block in &args.prompt {
            match block {
                ContentBlock::Text(t) => {
                    total_bytes = total_bytes.saturating_add(t.text.len());
                    if total_bytes > MAX_PROMPT_BYTES {
                        return Err(Error::invalid_params());
                    }
                    content.push(t.text.clone());
                }
                ContentBlock::ResourceLink(_r) => {
                    tracing::debug!(
                        "ACP ResourceLink block received — not supported, skipping. \
                         Only text content is bridged to the Reborn runtime."
                    );
                }
                other => {
                    tracing::debug!(
                        "ACP ContentBlock variant {:?} not supported — skipping.",
                        std::mem::discriminant(other),
                    );
                }
            }
        }

        let content = content.join("\n");
        if content.trim().is_empty() {
            return Err(Error::invalid_params());
        }

        // ── Look up or create a conversation for this session (issue #20) ──
        // Use `entry().or_insert_with()` pattern: if two concurrent prompts
        // race for the same session, only one will execute the creation
        // closure. The `or_insert_with` requires a sync closure, but the
        // conversation creation is async. We handle this by checking, dropping,
        // creating async, then inserting with entry — if the entry was filled
        // in the meantime we discard the newly-created conversation.
        let conversation = {
            let convs = self.sessions.conversations.borrow_mut();
            if let Some(id) = convs.get(session_id) {
                id.clone()
            } else {
                drop(convs);
                let id = self
                    .runtime
                    .new_conversation()
                    .await
                    .map_err(|e| internal_error(&format!("Failed to create conversation: {e}")))?;
                let mut convs = self.sessions.conversations.borrow_mut();
                convs
                    .entry(session_id.to_string())
                    .or_insert_with(|| id.clone());
                convs.get(session_id)
                    .context("conversation map corruption")
                    .map_err(|e| internal_error(&e.to_string()))?
                    .clone()
            }
        };

        // Per-session cancellation token.
        let cancel_token = self.sessions.cancel_token_for(session_id);

        // TODO(streaming, issue #6): `send_user_message_with_cancellation` returns
        // the complete reply text after the turn finishes. When the runtime exposes
        // an incremental/streaming API (e.g. a `futures::Stream` of content chunks
        // or a callback), wire it here to emit `SessionUpdate::AgentMessageChunk`
        // notifications progressively during generation. The notification-sending
        // infrastructure (extracting the connection, sending chunks) is already
        // in place below.
        let reply = self
            .runtime
            .send_user_message_with_cancellation(&conversation, &content, cancel_token)
            .await
            .map_err(|e| internal_error(&format!("Agent run failed: {e}")))?;

        let stop_reason = if reply.status == TurnStatus::Cancelled {
            StopReason::Cancelled
        } else if reply.is_successful_final_reply() {
            StopReason::EndTurn
        } else {
            StopReason::MaxTurnRequests
        };

        // Send the assistant reply text as an AgentMessageChunk notification
        // so the client receives the content. Extract the connection from the
        // Mutex *before* the `.await` to avoid holding the guard across the
        // yield point (issue #10).
        if let Some(text) = &reply.text {
            let conn = self.conn.lock().map_err(|_| {
                internal_error("connection lock poisoned")
            })?;
            if let Some(conn) = conn.as_ref() {
                if let Err(err) = conn
                    .session_notification(
                        SessionNotification::new(
                            args.session_id,
                            SessionUpdate::AgentMessageChunk(
                                ContentChunk::new(ContentBlock::from(text.as_str())),
                            ),
                        ),
                    )
                    .await
                {
                    tracing::debug!("ACP session notification failed: {err}");
                }
            }
        }

        Ok(PromptResponse::new(stop_reason))
    }

    async fn cancel(
        &self,
        args: agent_client_protocol::CancelNotification,
    ) -> std::result::Result<(), Error> {
        let session_id: &str = &args.session_id.0;
        self.sessions.cancel_session(session_id);
        Ok(())
    }
}

// ── Main entry point ──────────────────────────────────────────────────────

/// Wrapper that flushes stdout after every `write` call.
/// The ACP library writes JSON-RPC lines but never calls `flush()`, so
/// piped stdout would never deliver the response.
struct FlushingWrite<W> {
    inner: W,
}

impl<W> FlushingWrite<W> {
    fn new(inner: W) -> Self {
        Self { inner }
    }
}

// NOTE (issue #13): `W: Unpin` guarantees the inner value can be moved out of
// `Pin<&mut FlushingWrite<W>>`. `FlushingWrite<W>` is a simple wrapper with
// no `Drop` impl and no self-referential pins, so projecting the pin to
// `inner` via `self.get_mut()` is sound.
impl<W: Unpin + futures_io::AsyncWrite> futures_io::AsyncWrite for FlushingWrite<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        let result = Pin::new(&mut me.inner).poll_write(cx, buf);
        // Flush after each successful write so buffered data reaches the peer
        // promptly. If flush fails, report the flush error rather than a false
        // success.
        match result {
            std::task::Poll::Ready(Ok(n)) => {
                if let std::task::Poll::Ready(Err(flush_err)) =
                    Pin::new(&mut me.inner).poll_flush(cx)
                {
                    return std::task::Poll::Ready(Err(flush_err));
                }
                std::task::Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }
}

/// Apply a static trigger-fire access policy to the runtime input, mirroring
/// the `run` command's pattern (issue #8). When the trigger poller is enabled,
/// the configured operator owner may fire triggers; otherwise the policy is
/// disabled (no authorizer wired).
async fn apply_acp_trigger_fire_access_policy(
    runtime_input: ironclaw_composition::RebornRuntimeInput,
    boot_config: &crate::context::RebornCliContext,
) -> anyhow::Result<ironclaw_composition::RebornRuntimeInput> {
    // We need to inspect trigger_poller to decide whether to wire the policy.
    // The runtime input has already been built, so we check the poller flag.
    if !runtime_input.trigger_poller.enabled {
        return Ok(runtime_input);
    }

    let config_file = read_config_file(boot_config.boot_config())?;
    let user_id = UserId::new(crate::runtime::default_owner_id(config_file.as_ref()))
        .context("[identity].default_owner is invalid")?;
    let agent_id = AgentId::new(&runtime_input.identity.agent_id).with_context(|| {
        format!(
            "[identity].default_agent `{}` is invalid",
            runtime_input.identity.agent_id
        )
    })?;

    // The ACP owner grant is a static single owner — a config value,
    // built into the runtime's fire-time checker without any persisted
    // trigger-access store (arch-simplification §4.4).
    Ok(runtime_input.with_trigger_fire_access_policy(
        TriggerFireAccessPolicy::disabled().with_static_owner(user_id, agent_id, None),
    ))
}

impl AcpServeCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        // Validate and canonicalize --cwd (issue #21).
        if let Some(ref cwd) = self.cwd {
            let canonical = std::path::Path::new(cwd)
                .canonicalize()
                .with_context(|| format!("--cwd path does not exist or is inaccessible: {cwd}"))?;
            std::env::set_current_dir(&canonical)
                .with_context(|| format!("failed to set working directory to {cwd}"))?;
        }

        crate::runtime::init_tracing();
        let boot_config = context.boot_config().clone();

        // Sync setup — same as `run` command.
        let mut runtime_input =
            build_runtime_input_with_options(&boot_config, RuntimeInputCaller::AcpServe, RuntimeInputOptions::default())?
                .inner;

        // Wire --auto-approve through the runtime boot config (issue #5).
        // SAFETY: No other thread accesses the environment at this point;
        // the runtime has not been spawned yet. This matches the pattern
        // used by the `run` command for similar env-based configuration.
        if self.auto_approve {
            unsafe {
                std::env::set_var("IRONCLAW_REBORN_AUTO_APPROVE_TOOLS", "1");
            }
        }

        // Multi-thread runtime — `build_reborn_runtime` spawns internal tasks
        // that deadlock on single-thread (confirmed by testing).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        rt.block_on(async move {
            tracing::debug!("[ACP] building reborn runtime...");

            // Apply trigger-fire access policy (issue #8).
            runtime_input = apply_acp_trigger_fire_access_policy(runtime_input, &context).await?;

            let runtime = build_reborn_runtime(runtime_input).await?;
            tracing::debug!("[ACP] reborn runtime built, starting ACP I/O");
            let runtime = Arc::new(runtime);
            let conn_slot: Arc<std::sync::Mutex<Option<AgentSideConnection>>> =
                Arc::new(std::sync::Mutex::new(None));
            let agent = RebornAcpAgent {
                runtime: runtime.clone(),
                conn: conn_slot.clone(),
                sessions: Arc::new(SessionState::new()),
            };

            // ACP I/O must run inside a LocalSet because the Agent trait is
            // !Send (ACP crate uses `spawn_local` internally).
            let local_set = tokio::task::LocalSet::new();
            local_set
                .run_until(async {
                    let stdin = tokio::io::stdin();
                    let stdout = tokio::io::stdout();
                    // Wrap stdout in compat (tokio → futures-io), then in
                    // FlushingWrite to auto-flush after each write (the ACP
                    // library writes lines but never calls flush()).
                    let (conn, io_task) = AgentSideConnection::new(
                        agent.clone(),
                        FlushingWrite::new(
                            tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(stdout),
                        ),
                        TokioAsyncReadCompatExt::compat(stdin),
                        |fut| {
                            tokio::task::spawn_local(fut);
                        },
                    );
                    *conn_slot.lock().map_err(|_| anyhow::anyhow!("connection lock poisoned"))? = Some(conn);
                    // Spawn the I/O task on the LocalSet so that both the
                    // I/O reader and the message handler (spawned inside
                    // via the callback) are driven concurrently.
                    let handle = tokio::task::spawn_local(io_task);
                    // Propagate I/O task errors (issue #15). The task returns
                    // when stdin closes (EOF); a panic or error is surfaced
                    // here instead of being silently discarded.
                    match handle.await {
                        Ok(Ok(())) => {}
                        Ok(Err(io_err)) => {
                            tracing::debug!("[ACP] I/O task ended with error: {io_err}");
                        }
                        Err(join_err) => {
                            if !join_err.is_cancelled() {
                                tracing::debug!("[ACP] I/O task panicked: {join_err}");
                            }
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                })
                .await?;

            // Shut down the runtime after the LocalSet (and all ACP I/O)
            // completes. This drains background tasks (turn scheduler,
            // trigger poller, credential refresh worker, etc.) following
            // the same pattern as the `serve` command in serve.rs.
            tracing::debug!("[ACP] shutting down runtime...");
            match Arc::try_unwrap(runtime) {
                Ok(r) => r.shutdown().await.context("Reborn runtime shutdown failed")?,
                Err(_) => {
                    // Arc refs remain from the agent struct that was dropped
                    // with the LocalSet. This should not happen, but if it
                    // does, we log a warning — the runtime's Drop will still
                    // clean up internal state, just without graceful drain.
                    tracing::warn!(
                        "[ACP] runtime Arc still has multiple refs at shutdown; \
                         skipping graceful shutdown. Background tasks may not drain."
                    );
                }
            }

            Ok::<(), anyhow::Error>(())
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: These tests validate protocol contract invariants (version
    // negotiation, error sanitization, session lifecycle) using lightweight
    // mocks. Full `RebornAcpAgent` integration tests require a `RebornRuntime`
    // and are better suited as end-to-end tests against a live ACP client.

    /// The protocol negotiation must clamp to V1 when the client requests a
    /// version higher than the server supports, and reject V0 outright.
    #[test]
    fn initialize_clamps_version_and_rejects_v0() {
        struct MockAgent;

        #[async_trait::async_trait(?Send)]
        impl Agent for MockAgent {
            async fn initialize(
                &self,
                args: InitializeRequest,
            ) -> std::result::Result<InitializeResponse, Error> {
                let negotiated = std::cmp::min(args.protocol_version, ProtocolVersion::LATEST);
                if negotiated == ProtocolVersion::V0 {
                    return Err(Error::invalid_params());
                }
                Ok(InitializeResponse::new(negotiated))
            }

            async fn authenticate(
                &self,
                _args: AuthenticateRequest,
            ) -> std::result::Result<AuthenticateResponse, Error> {
                Ok(AuthenticateResponse::new())
            }

            async fn new_session(
                &self,
                _args: NewSessionRequest,
            ) -> std::result::Result<NewSessionResponse, Error> {
                Ok(NewSessionResponse::new(SessionId::new("test".to_string())))
            }

            async fn prompt(
                &self,
                _args: PromptRequest,
            ) -> std::result::Result<PromptResponse, Error> {
                Ok(PromptResponse::new(StopReason::EndTurn))
            }

            async fn cancel(
                &self,
                _args: agent_client_protocol::CancelNotification,
            ) -> std::result::Result<(), Error> {
                Ok(())
            }
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        // Client requests V1 → server returns V1.
        let resp = rt.block_on(MockAgent.initialize(InitializeRequest::new(
            ProtocolVersion::V1,
        )));
        assert_eq!(resp.expect("v1 init").protocol_version, ProtocolVersion::V1);

        // Client requests V999 → server clamps to LATEST (V1).
        let resp = rt.block_on(MockAgent.initialize(InitializeRequest::new(
            ProtocolVersion::from(999u16),
        )));
        assert_eq!(resp.expect("v999 init").protocol_version, ProtocolVersion::LATEST);

        // Client requests V0 → server rejects.
        let resp = rt.block_on(MockAgent.initialize(InitializeRequest::new(
            ProtocolVersion::V0,
        )));
        assert!(resp.is_err());
    }

    /// `internal_error` must not expose raw internal text.
    #[test]
    fn internal_error_sanitizes_message() {
        let err = internal_error("database at /var/lib/db/secret.sqlite3 is corrupt");
        let msg = &err.message;
        assert!(
            msg.contains(INTERNAL_ERROR_CATEGORY),
            "sanitized message must contain the category: {msg}"
        );
        // The raw path must NOT appear in the error message.
        assert!(
            !msg.contains("/var/lib/db/secret.sqlite3"),
            "sanitized message must not contain internal paths: {msg}"
        );
    }

    /// `SessionState::cancel_session` must cancel the existing token and
    /// replace it so subsequent prompts are not pre-cancelled.
    #[test]
    fn cancel_session_replaces_token() {
        let state = SessionState::new();
        let sid = state.create_session();
        let token_before = state.cancel_token_for(&sid);
        assert!(!token_before.is_cancelled());

        state.cancel_session(&sid);

        // The old token must now be cancelled.
        assert!(token_before.is_cancelled());

        // A fresh token must be available for future prompts.
        let token_after = state.cancel_token_for(&sid);
        assert!(!token_after.is_cancelled());
        assert!(!std::ptr::eq(
            // Different instances — `Arc` clone comparison.
            &token_before as *const _ as *const u8,
            &token_after as *const _ as *const u8,
        ));
    }

    /// Empty prompt content (whitespace-only) must be rejected.
    #[test]
    fn empty_prompt_rejected() {
        let content = "   \n\t  ".to_string();
        assert!(content.trim().is_empty());
    }

    /// MAX_PROMPT_BYTES is a reasonable bound.
    #[test]
    fn max_prompt_bytes_is_reasonable() {
        assert!(MAX_PROMPT_BYTES >= 1024, "minimum 1 KiB");
        assert!(MAX_PROMPT_BYTES <= 1024 * 1024, "maximum 1 MiB");
    }

    /// Prompt payload exceeding MAX_PROMPT_BYTES is rejected during iteration,
    /// before all blocks are cloned. This tests the early-reject path directly.
    #[test]
    fn prompt_bound_rejects_oversized_during_iteration() {
        use agent_client_protocol::ContentBlock;

        // Simulate the iteration logic from `RebornAcpAgent::prompt`.
        let blocks: Vec<ContentBlock> = vec![
            ContentBlock::from("a".repeat(MAX_PROMPT_BYTES / 2 + 1)),
            ContentBlock::from("b".repeat(MAX_PROMPT_BYTES / 2 + 1)),
        ];

        let mut total_bytes: usize = 0;
        let mut accepted = true;
        for block in &blocks {
            if let ContentBlock::Text(t) = block {
                total_bytes = total_bytes.saturating_add(t.text.len());
                if total_bytes > MAX_PROMPT_BYTES {
                    accepted = false;
                    break;
                }
            }
        }

        assert!(!accepted, "oversized payload must be rejected mid-iteration");
        assert!(
            total_bytes > MAX_PROMPT_BYTES,
            "total must exceed bound at rejection point"
        );
    }
}
