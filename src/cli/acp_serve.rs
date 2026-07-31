//! Standalone ACP server: speaks ACP protocol on stdio, runs the full IronClaw agent.
//!
//! Usage: ironclaw acp-serve [--config <path>] [--no-db] [--auto-approve] [--cwd <dir>]
//!
//! ## Architecture
//!
//! Agent loop runs on a tokio task. ACP RPC (stdio) lives on a LocalSet.
//! The agent sends session notifications through a channel; a notify_writer
//! task on the LocalSet calls `conn.session_notification()` to write them.
//! `prompt()` blocks until the agent signals done, preventing the race where
//! the RPC response arrives before notifications.

use std::collections::HashMap;
use std::sync::Arc;

use agent_client_protocol::{
    Agent, AgentSideConnection, AgentCapabilities, AuthenticateRequest, AuthenticateResponse,
    Client, ContentBlock, Error, Implementation, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, ProtocolVersion,
    SessionId, SessionNotification, SessionUpdate, StopReason, TextContent,
};
use clap::Parser;
use tokio::sync::{mpsc, RwLock};

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

// ── CLI Args ───────────────────────────────────────────────────────────────

#[derive(Parser, Debug, Clone)]
pub struct AcpServeArgs {
    /// Path to TOML config file
    #[arg(long)]
    config: Option<String>,

    /// Run without database
    #[arg(long)]
    no_db: bool,

    /// Auto-approve tool execution
    #[arg(long)]
    auto_approve: bool,

    /// Working directory for the agent
    #[arg(long)]
    cwd: Option<String>,
}

// ── Internal bridge types ──────────────────────────────────────────────────

/// Notification request: agent task → LocalSet notify_writer.
struct NotifyRequest {
    session_id: String,
    update_type: NotifyType,
    text: String,
    done: tokio::sync::oneshot::Sender<()>,
}

#[derive(Debug)]
enum NotifyType {
    Message,
    Thought,
}

/// Shared state between ACP Agent trait and the Channel impl.
struct AcpState {
    notify_tx: mpsc::UnboundedSender<NotifyRequest>,
    msg_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    done_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
    active_session: RwLock<Option<String>>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn send_notify(state: &Arc<AcpState>, session_id: &str, kind: NotifyType, text: &str) {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    if state
        .notify_tx
        .send(NotifyRequest {
            session_id: session_id.to_string(),
            update_type: kind,
            text: text.to_string(),
            done: tx,
        })
        .is_ok()
    {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), rx).await;
    }
}

fn active_session_id(state: &Arc<AcpState>) -> String {
    state
        .active_session
        .try_read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

// ── Channel implementation ────────────────────────────────────────────────

struct AcpChannel {
    state: Arc<AcpState>,
}

#[async_trait::async_trait]
impl Channel for AcpChannel {
    fn name(&self) -> &str {
        "acp"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self.state.msg_rx.lock().await.take().ok_or_else(|| {
            ChannelError::StartupFailed {
                name: "acp".into(),
                reason: "Message receiver already taken".into(),
            }
        })?;
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let sid = active_session_id(&self.state);
        eprintln!("[ACP] respond() called: len={} session={}", response.content.len(), sid);
        send_notify(&self.state, &sid, NotifyType::Message, &response.content).await;
        if let Some(tx) = self.state.done_tx.write().await.take() {
            eprintln!("[ACP] respond() signaling done");
            let _ = tx.send(());
        }
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let sid = active_session_id(&self.state);
        let (kind, text) = match &status {
            StatusUpdate::StreamChunk(t) => (NotifyType::Message, t.as_str()),
            StatusUpdate::Thinking(t) => (NotifyType::Thought, t.as_str()),
            _ => return Ok(()),
        };
        send_notify(&self.state, &sid, kind, text).await;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    fn conversation_context(
        &self,
        _metadata: &serde_json::Value,
    ) -> HashMap<String, String> {
        HashMap::new()
    }
}

// ── ACP Agent trait implementation ─────────────────────────────────────────

struct AcpAgent {
    state: Arc<AcpState>,
    msg_tx: mpsc::Sender<IncomingMessage>,
}

#[async_trait::async_trait(?Send)]
impl Agent for AcpAgent {
    async fn initialize(
        &self,
        _args: InitializeRequest,
    ) -> std::result::Result<InitializeResponse, Error> {
        Ok(InitializeResponse::new(ProtocolVersion::from(2u16))
            .agent_capabilities(AgentCapabilities::new())
            .agent_info(Implementation::new("ironclaw", env!("CARGO_PKG_VERSION"))))
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
        let sid = uuid::Uuid::new_v4().to_string();
        Ok(NewSessionResponse::new(SessionId::new(sid)))
    }

    async fn prompt(
        &self,
        args: PromptRequest,
    ) -> std::result::Result<PromptResponse, Error> {
        let content: String = args
            .prompt
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if content.is_empty() {
            return Err(Error::invalid_params());
        }

        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        *self.state.done_tx.write().await = Some(done_tx);

        let session_id = args.session_id.to_string();
        *self.state.active_session.write().await = Some(session_id.clone());

        let msg = IncomingMessage::new("acp", "user", content).with_thread(session_id);

        if self.msg_tx.send(msg).await.is_err() {
            return Err(Error::new(
                -32603,
                "Agent loop not available".to_string(),
            ));
        }

        // Block until the agent has finished sending all notifications
        // and signals done via respond().
        let _ = tokio::time::timeout(std::time::Duration::from_secs(300), done_rx).await;

        Ok(PromptResponse::new(StopReason::EndTurn))
    }

    async fn cancel(
        &self,
        _args: agent_client_protocol::CancelNotification,
    ) -> std::result::Result<(), Error> {
        if let Some(tx) = self.state.done_tx.write().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

// ── Main entry point ──────────────────────────────────────────────────────

pub async fn run_acp_serve(args: AcpServeArgs) -> anyhow::Result<()> {
    if args.auto_approve {
        crate::config::set_runtime_env("AGENT_AUTO_APPROVE_TOOLS", "true");
    }

    if let Some(ref cwd) = args.cwd {
        std::env::set_current_dir(cwd)?;
    }

    tracing::info!("ACP: loading config");
    let toml_path = args.config.as_deref().map(std::path::Path::new);
    let config = crate::config::Config::from_env_with_toml(toml_path)
        .await
        .map_err(|e| anyhow::anyhow!("Config error: {e}"))?;

    tracing::info!("ACP: creating LLM session manager");
    let session = crate::llm::create_session_manager(config.llm.session.clone()).await;

    tracing::info!("ACP: building app components");
    let flags = crate::app::AppBuilderFlags { no_db: args.no_db };
    let log_broadcaster = Arc::new(crate::channels::web::log_layer::LogBroadcaster::new());
    let components = crate::app::AppBuilder::new(
        config,
        flags,
        toml_path.map(std::path::PathBuf::from),
        session.clone(),
        log_broadcaster,
    )
    .build_all()
    .await?;

    let config = components.config;

    // Notification channel: agent task → LocalSet writer
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<NotifyRequest>();

    let (msg_tx, msg_rx) = mpsc::channel(64);
    let state = Arc::new(AcpState {
        notify_tx,
        msg_rx: tokio::sync::Mutex::new(Some(msg_rx)),
        done_tx: RwLock::new(None),
        active_session: RwLock::new(None),
    });

    let acp_channel = AcpChannel { state: state.clone() };
    let channels = Arc::new(crate::channels::ChannelManager::new());
    channels.add(Box::new(acp_channel)).await;

    // Register message tools so the agent can send messages (buzz messages send etc.)
    components
        .tools
        .register_message_tools(Arc::clone(&channels), components.extension_manager.clone())
        .await;

    // Register buzz_send tool for direct Buzz channel publishing
    components.tools.register_buzz_send_tool().await;

    let deps = crate::agent::AgentDeps {
        owner_id: config.owner_id.clone(),
        settings_store: components.settings_store.clone(),
        store: components.db,
        llm: components.llm,
        cheap_llm: components.cheap_llm,
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager: components.extension_manager,
        skill_registry: components.skill_registry,
        skill_catalog: components.skill_catalog,
        skills_config: config.skills.clone(),
        hooks: components.hooks,
        auth_manager: None,
        cost_guard: components.cost_guard,
        sse_tx: None,
        http_interceptor: components.http_interceptor,
        transcription: config.transcription.create_provider().map(|p| {
            Arc::new(crate::llm::transcription::TranscriptionMiddleware::new(p))
        }),
        document_extraction: Some(Arc::new(
            crate::document_extraction::DocumentExtractionMiddleware::new(),
        )),
        sandbox_readiness: crate::agent::routine_engine::SandboxReadiness::DisabledByConfig,
        builder: components.builder,
        llm_backend: config.llm.backend.clone(),
        tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(
            config.agent.max_llm_concurrent_per_user.unwrap_or(4),
            config.agent.max_jobs_concurrent_per_user.unwrap_or(3),
        )),
    };

    let mut agent = crate::agent::Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(components.agent_session_manager),
    );

    tracing::info!("ACP: spawning agent.run()");
    let agent_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            tracing::error!("Agent run loop exited with error: {e}");
        }
    });

    let acp_agent = AcpAgent {
        state: state.clone(),
        msg_tx,
    };

    let local_set = tokio::task::LocalSet::new();
    tracing::info!("ACP: starting LocalSet for protocol I/O");
    let result = local_set
        .run_until(async {
            let stdin = tokio::io::stdin();
            let stdout = tokio::io::stdout();
            let (conn, io_task) = AgentSideConnection::new(
                acp_agent,
                tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(stdout),
                tokio_util::compat::TokioAsyncReadCompatExt::compat(stdin),
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );

            // Notify writer runs on the LocalSet alongside io_task.
            // Uses conn.session_notification() (the typed ACP crate method)
            // so there's only ONE stdout writer — no interleaving.
            let notify_writer = tokio::task::spawn_local(async move {
                while let Some(req) = notify_rx.recv().await {
                    eprintln!("[ACP] notify: type={:?} len={} session={}", req.update_type, req.text.len(), req.session_id);
                    let chunk = agent_client_protocol::ContentChunk::new(
                        agent_client_protocol::ContentBlock::Text(
                            agent_client_protocol::TextContent::new(&req.text),
                        ),
                    );
                    let update = match req.update_type {
                        NotifyType::Message => SessionUpdate::AgentMessageChunk(chunk),
                        NotifyType::Thought => SessionUpdate::AgentThoughtChunk(chunk),
                    };
                    let notification =
                        SessionNotification::new(SessionId::new(req.session_id), update);
                    // Log exact JSON for debugging
                    if let Ok(json) = serde_json::to_string(&notification) {
                        eprintln!("[ACP] JSON: {json}");
                    }
                    if let Err(e) = conn.session_notification(notification).await {
                        eprintln!("[ACP] notify FAILED: {e}");
                    } else {
                        eprintln!("[ACP] notify sent OK");
                    }
                    let _ = req.done.send(());
                }
            });

            io_task.await.map_err(|e| anyhow::anyhow!("ACP I/O: {e}"))?;
            notify_writer.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await;

    agent_handle.abort();

    result?;
    Ok(())
}
