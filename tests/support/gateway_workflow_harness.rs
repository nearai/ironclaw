#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use secrecy::SecretString;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use ironclaw::agent::routine_engine::RoutineEngine;
use ironclaw::agent::{Agent, AgentDeps, SessionManager as AgentSessionManager};
use ironclaw::app::{AppBuilder, AppBuilderFlags};
use ironclaw::channels::IncomingMessage;
use ironclaw::channels::web::auth::MultiAuthState;
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::channels::web::server::{
    GatewayState, PerUserRateLimiter, RateLimiter, start_server,
};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::ws::WsConnectionTracker;
use ironclaw::config::{Config, RegistryProviderConfig, RoutineConfig};
use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::llm::registry::ProviderProtocol;
use ironclaw::llm::{
    SessionConfig as LlmSessionConfig, SessionManager as LlmSessionManager, create_llm_provider,
};
use ironclaw::secrets::SecretsStore;
use ironclaw::tools::{Tool, ToolError, ToolOutput};

use crate::support::test_channel::{TestChannel, TestChannelHandle};

struct MockGithubWebhookTool;

#[async_trait]
impl Tool for MockGithubWebhookTool {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "Mock GitHub webhook parser for integration harness"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &ironclaw::context::JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let event = params
            .pointer("/webhook/headers/x-github-event")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing x-github-event".to_string()))?;

        let action = params
            .pointer("/webhook/body_json/action")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let mut payload = params
            .pointer("/webhook/body_json")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if payload.get("repository").and_then(|v| v.as_str()).is_none()
            && let Some(full_name) = payload
                .pointer("/repository/full_name")
                .and_then(|v| v.as_str())
        {
            payload["repository"] = serde_json::json!(full_name);
        }
        let event_type = format!(
            "{}.{}",
            if event == "issues" { "issue" } else { event },
            action
        );

        Ok(ToolOutput::success(
            serde_json::json!({
                "emit_events": [{
                    "source": "github",
                    "event_type": event_type,
                    "payload": payload
                }]
            }),
            Duration::from_millis(1),
        ))
    }

    fn webhook_capability(&self) -> Option<ironclaw::tools::wasm::WebhookCapability> {
        Some(ironclaw::tools::wasm::WebhookCapability {
            secret_name: Some("github_webhook_secret".to_string()),
            secret_header: Some("x-webhook-secret".to_string()),
            ..Default::default()
        })
    }
}

pub struct GatewayWorkflowHarness {
    pub addr: SocketAddr,
    pub webhook_addr: SocketAddr,
    pub auth_token: String,
    pub client: reqwest::Client,
    pub user_id: String,
    pub test_channel: Arc<TestChannel>,
    pub db: Arc<dyn Database>,
    gateway_state: Arc<GatewayState>,
    agent_handle: Option<tokio::task::JoinHandle<()>>,
    bridge_handle: Option<tokio::task::JoinHandle<()>>,
    webhook_shutdown_tx: Option<oneshot::Sender<()>>,
    webhook_handle: Option<tokio::task::JoinHandle<()>>,
    _temp_dir: tempfile::TempDir,
}

impl GatewayWorkflowHarness {
    pub async fn start_openai_compatible(base_url: &str, model: &str) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("gateway_workflow_harness.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("failed to create test db");
        backend
            .run_migrations()
            .await
            .expect("failed to run migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        let skills_dir = temp_dir.path().join("skills");
        let installed_skills_dir = temp_dir.path().join("installed_skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        let _ = std::fs::create_dir_all(&installed_skills_dir);
        let mut config = Config::for_testing(db_path, skills_dir, installed_skills_dir);
        config.agent.auto_approve_tools = true;
        config.routines.enabled = true;
        config.routines.max_concurrent_routines = 4;
        config.llm.backend = "openai_compatible".to_string();
        config.llm.provider = Some(RegistryProviderConfig {
            protocol: ProviderProtocol::OpenAiCompletions,
            provider_id: "openai_compatible".to_string(),
            api_key: Some(SecretString::from("dummy".to_string())),
            base_url: base_url.to_string(),
            model: model.to_string(),
            extra_headers: Vec::new(),
            oauth_token: None,
            is_codex_chatgpt: false,
            refresh_token: None,
            auth_path: None,
            cache_retention: Default::default(),
            unsupported_params: Vec::new(),
        });

        let llm_session = Arc::new(LlmSessionManager::new(LlmSessionConfig::default()));
        let llm = create_llm_provider(&config.llm, Arc::clone(&llm_session))
            .await
            .expect("failed to create openai-compatible provider");

        let log_broadcaster = Arc::new(LogBroadcaster::new());
        let mut app_builder = AppBuilder::new(
            config,
            AppBuilderFlags::default(),
            None,
            Arc::clone(&llm_session),
            log_broadcaster,
        );
        app_builder.with_database(Arc::clone(&db));
        app_builder.with_llm(llm);

        let components = app_builder
            .build_all()
            .await
            .expect("failed to build app components");
        components
            .tools
            .register(Arc::new(MockGithubWebhookTool))
            .await;

        components.tools.register_job_tools(
            Arc::clone(&components.context_manager),
            None,
            None,
            components.db.clone(),
            None,
            None,
            None,
            None,
        );

        // Agent::run() creates its own RoutineEngine and populates this slot.
        let routine_slot: Arc<tokio::sync::RwLock<Option<Arc<RoutineEngine>>>> =
            Arc::new(tokio::sync::RwLock::new(None));

        let test_channel = Arc::new(TestChannel::new());
        let handle = TestChannelHandle::with_name(Arc::clone(&test_channel), "gateway");
        let channel_manager = ironclaw::channels::ChannelManager::new();
        channel_manager.add(Box::new(handle)).await;
        let channels = Arc::new(channel_manager);

        let user_id = "gateway-test-user".to_string();
        let (gw_tx, mut gw_rx) = mpsc::channel::<IncomingMessage>(256);
        let forward_channel = Arc::clone(&test_channel);
        let bridge_handle = tokio::spawn(async move {
            while let Some(msg) = gw_rx.recv().await {
                forward_channel.send_incoming(msg).await;
            }
        });

        let scheduler_slot: ironclaw::tools::builtin::SchedulerSlot =
            Arc::new(tokio::sync::RwLock::new(None));
        let agent_session_manager = Arc::new(AgentSessionManager::new());

        let gateway_state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(Some(gw_tx)),
            sse: Arc::new(SseManager::new()),
            workspace: components.workspace.clone(),
            workspace_pool: None,
            session_manager: Some(Arc::clone(&agent_session_manager)),
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: components.extension_manager.clone(),
            tool_registry: Some(Arc::clone(&components.tools)),
            store: components.db.clone(),
            job_manager: None,
            prompt_queue: None,
            scheduler: Some(scheduler_slot.clone()),
            owner_id: user_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: Some(Arc::clone(&components.llm)),
            skill_registry: components.skill_registry.clone(),
            skill_catalog: components.skill_catalog.clone(),
            chat_rate_limiter: PerUserRateLimiter::new(120, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: Some(Arc::clone(&components.cost_guard)),
            routine_engine: Arc::clone(&routine_slot),
            startup_time: Instant::now(),
            active_config: ironclaw::channels::web::server::ActiveConfigSnapshot::default(),
            secrets_store: None,
            db_auth: None,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
            collection_write_tx: None,
            skills_dir: None,
        });

        let mut agent = Agent::new(
            components.config.agent.clone(),
            AgentDeps {
                owner_id: components.config.owner_id.clone(),
                store: components.db,
                llm: components.llm,
                cheap_llm: components.cheap_llm,
                safety: components.safety,
                tools: components.tools,
                workspace: components.workspace,
                extension_manager: components.extension_manager,
                skill_registry: components.skill_registry,
                skill_catalog: components.skill_catalog,
                skills_config: components.config.skills.clone(),
                hooks: components.hooks,
                cost_guard: components.cost_guard,
                sse_tx: None,
                http_interceptor: None,
                transcription: None,
                document_extraction: None,
                sandbox_readiness:
                    ironclaw::agent::routine_engine::SandboxReadiness::DisabledByConfig,
                builder: None,
                llm_backend: "nearai".to_string(),
                tenant_rates: std::sync::Arc::new(ironclaw::tenant::TenantRateRegistry::new(4, 3)),
            },
            channels,
            None,
            None,
            Some(RoutineConfig {
                enabled: true,
                cron_check_interval_secs: 60,
                max_concurrent_routines: 4,
                default_cooldown_secs: 300,
                max_lightweight_tokens: 4096,
                lightweight_tools_enabled: true,
                lightweight_max_iterations: 3,
            }),
            Some(Arc::clone(&components.context_manager)),
            Some(Arc::clone(&agent_session_manager)),
        );
        agent.set_routine_engine_slot(Arc::clone(&routine_slot));
        *scheduler_slot.write().await = Some(agent.scheduler());

        let agent_handle = tokio::spawn(async move {
            let _ = agent.run().await;
        });

        if let Some(rx) = test_channel.take_ready_rx().await {
            let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
        }

        let auth_token = "gateway-test-token".to_string();
        let auth = MultiAuthState::single(auth_token.clone(), user_id.clone());
        let addr = start_server(
            "127.0.0.1:0".parse().expect("valid localhost addr"),
            Arc::clone(&gateway_state),
            auth.into(),
        )
        .await
        .expect("failed to start gateway server");

        let webhook_secrets = Arc::new(ironclaw::secrets::InMemorySecretsStore::new(Arc::new(
            ironclaw::secrets::SecretsCrypto::new(SecretString::from(
                "test-key-at-least-32-chars-long!!".to_string(),
            ))
            .expect("crypto"),
        )));
        webhook_secrets
            .create(
                &user_id,
                ironclaw::secrets::CreateSecretParams::new(
                    "github_webhook_secret",
                    "test-webhook-secret",
                ),
            )
            .await
            .expect("store webhook secret");
        let webhook_state = ironclaw::webhooks::ToolWebhookState {
            tools: Arc::clone(gateway_state.tool_registry.as_ref().expect("tool registry")),
            routine_engine: Arc::clone(&routine_slot),
            user_id: user_id.clone(),
            secrets_store: Some(
                webhook_secrets as Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>,
            ),
        };
        let webhook_app = ironclaw::webhooks::routes(webhook_state);
        let webhook_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind webhook listener");
        let webhook_addr = webhook_listener.local_addr().expect("webhook local addr");
        let (webhook_shutdown_tx, webhook_shutdown_rx) = oneshot::channel();
        let webhook_handle = tokio::spawn(async move {
            let _ = axum::serve(webhook_listener, webhook_app)
                .with_graceful_shutdown(async {
                    let _ = webhook_shutdown_rx.await;
                })
                .await;
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");

        Self {
            addr,
            webhook_addr,
            auth_token,
            client,
            user_id,
            test_channel,
            db,
            gateway_state,
            agent_handle: Some(agent_handle),
            bridge_handle: Some(bridge_handle),
            webhook_shutdown_tx: Some(webhook_shutdown_tx),
            webhook_handle: Some(webhook_handle),
            _temp_dir: temp_dir,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn webhook_base_url(&self) -> String {
        format!("http://{}", self.webhook_addr)
    }

    pub async fn create_thread(&self) -> String {
        let resp = self
            .client
            .post(format!("{}/api/chat/thread/new", self.base_url()))
            .bearer_auth(&self.auth_token)
            .send()
            .await
            .expect("create thread request failed")
            .error_for_status()
            .expect("create thread non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid thread response");
        resp.get("id")
            .and_then(|v| v.as_str())
            .expect("thread id missing")
            .to_string()
    }

    pub async fn send_chat(&self, thread_id: &str, content: &str) {
        let _ = self
            .client
            .post(format!("{}/api/chat/send", self.base_url()))
            .bearer_auth(&self.auth_token)
            .json(&serde_json::json!({"thread_id": thread_id, "content": content}))
            .send()
            .await
            .expect("chat send failed")
            .error_for_status()
            .expect("chat send non-2xx");
    }

    pub async fn history(&self, thread_id: &str) -> serde_json::Value {
        self.client
            .get(format!(
                "{}/api/chat/history?thread_id={thread_id}",
                self.base_url()
            ))
            .bearer_auth(&self.auth_token)
            .send()
            .await
            .expect("history request failed")
            .error_for_status()
            .expect("history non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid history response")
    }

    pub async fn wait_for_turns(
        &self,
        thread_id: &str,
        min_turns: usize,
        timeout: Duration,
    ) -> serde_json::Value {
        let deadline = Instant::now() + timeout;
        loop {
            let history = self.history(thread_id).await;
            let turns = history
                .get("turns")
                .and_then(|v| v.as_array())
                .map(|v| v.len())
                .unwrap_or_default();
            if turns >= min_turns {
                return history;
            }
            assert!(Instant::now() < deadline, "timed out waiting for turns");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn list_routines(&self) -> serde_json::Value {
        self.client
            .get(format!("{}/api/routines", self.base_url()))
            .bearer_auth(&self.auth_token)
            .send()
            .await
            .expect("routines request failed")
            .error_for_status()
            .expect("routines non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid routines response")
    }

    pub async fn routine_by_name(&self, name: &str) -> Option<serde_json::Value> {
        let routines = self.list_routines().await;
        routines
            .get("routines")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|r| r.get("name").and_then(|v| v.as_str()) == Some(name))
                    .cloned()
            })
    }

    pub async fn routine_runs(&self, routine_id: &str) -> serde_json::Value {
        self.client
            .get(format!(
                "{}/api/routines/{routine_id}/runs",
                self.base_url()
            ))
            .bearer_auth(&self.auth_token)
            .send()
            .await
            .expect("routine runs request failed")
            .error_for_status()
            .expect("routine runs non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid routine runs response")
    }

    pub async fn register_tool(&self, tool: Arc<dyn Tool>) {
        let registry = self
            .gateway_state
            .tool_registry
            .as_ref()
            .expect("tool registry should be available");
        registry.register(tool).await;
    }

    pub async fn github_webhook(
        &self,
        event: &str,
        payload: serde_json::Value,
    ) -> serde_json::Value {
        self.client
            .post(format!("{}/webhook/tools/github", self.webhook_base_url()))
            .header("x-github-event", event)
            .header("x-webhook-secret", "test-webhook-secret")
            .json(&payload)
            .send()
            .await
            .expect("webhook request failed")
            .error_for_status()
            .expect("webhook non-2xx")
            .json::<serde_json::Value>()
            .await
            .expect("invalid webhook response")
    }

    pub async fn shutdown(mut self) {
        self.test_channel.signal_shutdown();

        if let Some(tx) = self.gateway_state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        if let Some(tx) = self.webhook_shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.bridge_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.webhook_handle.take() {
            let _ = handle.await;
        }
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for GatewayWorkflowHarness {
    fn drop(&mut self) {
        self.test_channel.signal_shutdown();
        if let Some(handle) = self.bridge_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.webhook_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
    }
}
