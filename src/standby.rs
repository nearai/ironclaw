use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, Notify, mpsc};
use uuid::Uuid;

use crate::Config;
use crate::channels::web::auth::hash_token;
use crate::channels::web::sse::DEFAULT_MAX_CONNECTIONS;
use crate::config::{
    DEFAULT_GATEWAY_PORT, GatewayConfig, HttpSecurityConfig, HttpSecurityMode, remove_runtime_env,
    set_runtime_env,
};
use crate::llm::ProviderRegistry;
use crate::registry::embedded::load_embedded;
use crate::settings::Settings;
use crate::tools::mcp::config::save_mcp_servers;
use crate::tools::mcp::{McpServerConfig, McpServersFile};
use crate::workspace::{Workspace, layer::MemoryLayer, paths};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigureRequest {
    pub agent_id: Uuid,
    pub llm: TidePoolConfigureLlm,
    #[serde(default)]
    pub mcp_servers: Vec<TidePoolConfigureMcpServer>,
    #[serde(default)]
    pub channels: Vec<TidePoolConfigureChannel>,
    pub http: TidePoolConfigureHttp,
    pub persona: TidePoolConfigurePersona,
    /// Runtime-only env handoff for TidePool fast path.
    #[serde(default)]
    pub runtime_env: HashMap<String, String>,
    /// Extension desired state from LP. Absent or empty means keep current state.
    #[serde(default)]
    pub extensions: Vec<TidePoolExtensionDesiredState>,
}

/// Desired state for a single extension, sent by LP during configure/reconfigure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolExtensionDesiredState {
    pub name: String,
    pub kind: String,
    pub source_url: Option<String>,
    pub install_source: Option<String>,
    pub desired_enabled: bool,
    pub setup_json: Option<serde_json::Value>,
    pub owner_binding_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigureLlm {
    pub backend: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigureMcpServer {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigureChannel {
    pub channel_type: String,
    pub endpoint_url: String,
    #[serde(default)]
    pub credentials: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigureHttp {
    pub security_mode: String,
    pub allow_private_http: bool,
    pub allow_private_ip_literals: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigurePersona {
    pub soul: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub skills: Vec<TidePoolConfigureSkill>,
    /// v2 explicit prompt documents from LP's shared projector.
    /// When present, `write_prompt_documents()` uses these instead of the
    /// legacy `soul + parameters` fields. Both are produced by the same
    /// projector on the LP side.
    #[serde(default)]
    pub prompt_documents: Option<TidePoolPromptDocuments>,
}

/// v2 explicit prompt documents. Each field maps 1:1 to a workspace identity file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolPromptDocuments {
    pub projection_version: u8,
    pub identity_md: String,
    pub soul_md: String,
    pub agents_md: String,
    pub user_md: String,
    pub tools_md: String,
    #[serde(default)]
    pub knowledge_bases: Vec<TidePoolKnowledgeBaseRef>,
}

/// Knowledge base reference for capabilities projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolKnowledgeBaseRef {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidePoolConfigureSkill {
    pub name: String,
    pub content: Option<String>,
    pub description: Option<String>,
    /// `"wasm_tool"`, `"wasm_channel"`, or absent (prompt skill).
    /// WASM skills reference tools already loaded from the filesystem —
    /// they should NOT be written as SKILL.md files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_type: Option<String>,
}

#[derive(Debug)]
pub struct ConfigureCommand {
    pub request: TidePoolConfigureRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandbyPhase {
    Waiting,
    Configuring,
    Configured,
}

pub struct StandbyControl {
    phase: Mutex<StandbyPhase>,
    token_hash: [u8; 32],
    request_tx: mpsc::Sender<ConfigureCommand>,
    startup_state: Mutex<StandbyStartupState>,
    runtime_started_notify: Notify,
}

#[derive(Debug, Clone)]
struct StandbyStartupState {
    last_stage: &'static str,
    runtime_started: bool,
    configure_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandbyStartupSnapshot {
    pub phase: String,
    pub last_stage: String,
    pub runtime_started: bool,
    pub configure_ready: bool,
}

pub struct PrewarmedDatabase {
    pub backend: crate::config::DatabaseBackend,
    pub db: std::sync::Arc<dyn crate::db::Database>,
}

impl StandbyPhase {
    fn as_str(self) -> &'static str {
        match self {
            StandbyPhase::Waiting => "waiting",
            StandbyPhase::Configuring => "configuring",
            StandbyPhase::Configured => "configured",
        }
    }
}

impl StandbyControl {
    pub fn new(auth_token: &str, request_tx: mpsc::Sender<ConfigureCommand>) -> Arc<Self> {
        Arc::new(Self {
            phase: Mutex::new(StandbyPhase::Waiting),
            token_hash: hash_token(auth_token),
            request_tx,
            startup_state: Mutex::new(StandbyStartupState {
                last_stage: "standby.prewarm.pending",
                runtime_started: false,
                configure_ready: false,
            }),
            runtime_started_notify: Notify::new(),
        })
    }

    pub fn authenticate(&self, token: &str) -> bool {
        bool::from(hash_token(token).ct_eq(&self.token_hash))
    }

    pub async fn begin_configure(&self) -> Result<(), &'static str> {
        let mut phase = self.phase.lock().await;
        match *phase {
            StandbyPhase::Waiting => {
                *phase = StandbyPhase::Configuring;
                drop(phase);
                self.mark_startup_stage("configure.accepted").await;
                let mut startup_state = self.startup_state.lock().await;
                startup_state.runtime_started = false;
                Ok(())
            }
            StandbyPhase::Configured => {
                // Reconfigure: allow re-entry from Configured state.
                // Don't reset runtime_started — the agent is already running.
                *phase = StandbyPhase::Configuring;
                drop(phase);
                self.mark_startup_stage("reconfigure.accepted").await;
                Ok(())
            }
            StandbyPhase::Configuring => Err("configuration is already in progress"),
        }
    }

    pub async fn finish_configure(&self, success: bool) {
        let mut phase = self.phase.lock().await;
        if success {
            *phase = StandbyPhase::Configured;
        } else {
            // On failure: if the runtime was already started (reconfigure case),
            // return to Configured so the agent keeps running with old config.
            // Otherwise (initial configure failure), return to Waiting.
            let startup_state = self.startup_state.lock().await;
            *phase = if startup_state.runtime_started {
                StandbyPhase::Configured
            } else {
                StandbyPhase::Waiting
            };
        }
        drop(phase);
        if success {
            self.mark_startup_stage("configure.completed").await;
        } else {
            self.mark_startup_stage("configure.reset").await;
        }
    }

    pub async fn mark_configure_ready(&self, stage: &'static str) {
        let mut startup_state = self.startup_state.lock().await;
        startup_state.last_stage = stage;
        startup_state.configure_ready = true;
        tracing::info!(stage, "standby configure readiness reached");
    }

    pub async fn is_configure_ready(&self) -> bool {
        self.startup_state.lock().await.configure_ready
    }

    pub async fn enqueue(&self, request: TidePoolConfigureRequest) -> Result<(), String> {
        self.request_tx
            .send(ConfigureCommand { request })
            .await
            .map_err(|_| "standby configure receiver is unavailable".to_string())
    }

    pub async fn mark_startup_stage(&self, stage: &'static str) {
        let mut startup_state = self.startup_state.lock().await;
        if startup_state.last_stage != stage {
            startup_state.last_stage = stage;
            tracing::info!(stage, "standby startup stage");
        }
    }

    pub async fn mark_runtime_started(&self, stage: &'static str) {
        let mut startup_state = self.startup_state.lock().await;
        startup_state.last_stage = stage;
        let should_notify = !startup_state.runtime_started;
        startup_state.runtime_started = true;
        drop(startup_state);

        tracing::info!(stage, "standby runtime signaled ready");
        if should_notify {
            self.runtime_started_notify.notify_waiters();
        }
    }

    pub async fn wait_for_runtime_started(&self) {
        loop {
            let notified = self.runtime_started_notify.notified();
            {
                let startup_state = self.startup_state.lock().await;
                if startup_state.runtime_started {
                    return;
                }
            }
            notified.await;
        }
    }

    pub async fn startup_snapshot(&self) -> StandbyStartupSnapshot {
        let phase = *self.phase.lock().await;
        let startup_state = self.startup_state.lock().await;
        StandbyStartupSnapshot {
            phase: phase.as_str().to_string(),
            last_stage: startup_state.last_stage.to_string(),
            runtime_started: startup_state.runtime_started,
            configure_ready: startup_state.configure_ready,
        }
    }
}

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
}

pub fn resolve_standby_gateway_config(
    toml_path: Option<&Path>,
) -> Result<(String, GatewayConfig), String> {
    let settings = match toml_path {
        Some(path) => Settings::load_toml(path)?.unwrap_or_default(),
        None => Settings::default(),
    };

    let owner_id = std::env::var("IRONCLAW_OWNER_ID")
        .ok()
        .or(settings.owner_id.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "default".to_string());

    let host = std::env::var("GATEWAY_HOST")
        .ok()
        .or(settings.channels.gateway_host.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let port = std::env::var("GATEWAY_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .or(settings.channels.gateway_port)
        .unwrap_or(DEFAULT_GATEWAY_PORT);

    let max_connections = std::env::var("GATEWAY_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_CONNECTIONS);

    let workspace_read_scopes = std::env::var("WORKSPACE_READ_SCOPES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    Ok((
        owner_id.clone(),
        GatewayConfig {
            host,
            port,
            auth_token: std::env::var("GATEWAY_AUTH_TOKEN").ok(),
            max_connections,
            workspace_read_scopes,
            memory_layers: MemoryLayer::default_for_user(&owner_id),
            oidc: None,
        },
    ))
}

pub async fn prewarm_runtime_dependencies(
    toml_path: Option<&Path>,
    no_db: bool,
) -> Result<Option<PrewarmedDatabase>, String> {
    if no_db {
        return Ok(None);
    }

    let config = Config::from_env_with_toml(toml_path)
        .await
        .map_err(|error| format!("failed to load config for standby prewarm: {error}"))?;
    let backend = config.database.backend;
    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|error| format!("failed to prewarm database for standby: {error}"))?;
    Ok(Some(PrewarmedDatabase { backend, db }))
}

pub async fn apply_runtime_config(request: &TidePoolConfigureRequest) -> Result<(), String> {
    apply_runtime_env(&request.runtime_env)?;
    apply_llm_env(&request.llm)?;
    apply_channel_env(&request.channels)?;
    apply_http_env(&request.http)?;
    write_mcp_config(&request.mcp_servers).await?;
    Ok(())
}

fn apply_runtime_env(runtime_env: &HashMap<String, String>) -> Result<(), String> {
    for (key, value) in runtime_env {
        if key.trim().is_empty() {
            return Err("runtimeEnv contains an empty key".to_string());
        }

        if key == "GATEWAY_AUTH_TOKEN" {
            tracing::warn!("Ignoring GATEWAY_AUTH_TOKEN override from TidePool configure payload");
            continue;
        }

        set_runtime_env(key, value);
    }

    Ok(())
}

/// Reconcile extension desired state from LP against the running ExtensionManager.
///
/// For each extension in the desired state list:
/// - If not installed: install from source_url or registry
/// - If installed but setup/owner_binding differs: log for future reconciliation
/// - If desired_enabled but not active: activate
///
/// Individual extension failures are logged but do not fail the overall configure.
/// Extensions requiring auth are left in needs_auth state.
pub async fn reconcile_extensions(
    ext_mgr: &crate::extensions::ExtensionManager,
    extensions: &[TidePoolExtensionDesiredState],
    user_id: &str,
) {
    if extensions.is_empty() {
        return;
    }

    for ext in extensions {
        let kind_hint = match ext.kind.as_str() {
            "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
            "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
            "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
            "channel_relay" => Some(crate::extensions::ExtensionKind::ChannelRelay),
            "acp_agent" => Some(crate::extensions::ExtensionKind::AcpAgent),
            _ => {
                tracing::debug!(
                    extension = %ext.name,
                    kind = %ext.kind,
                    "Unknown extension kind in desired state — skipping"
                );
                continue;
            }
        };

        // Check if already installed
        let installed = ext_mgr
            .list(kind_hint, false, user_id)
            .await
            .unwrap_or_default();
        let already_installed = installed
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&ext.name) && e.installed);

        if !already_installed {
            let source = ext.source_url.as_deref();
            match ext_mgr.install(&ext.name, source, kind_hint, user_id).await {
                Ok(result) => {
                    tracing::debug!(
                        extension = %ext.name,
                        kind = %ext.kind,
                        message = ?result.message,
                        "Extension installed via LP desired state"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        extension = %ext.name,
                        kind = %ext.kind,
                        error = %e,
                        "Failed to install extension from LP desired state"
                    );
                    continue;
                }
            }
        }

        // Activate if desired_enabled
        if ext.desired_enabled {
            match ext_mgr.activate(&ext.name, user_id).await {
                Ok(result) => {
                    tracing::debug!(
                        extension = %ext.name,
                        message = ?result.message,
                        "Extension activation result from LP desired state"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        extension = %ext.name,
                        error = %e,
                        "Failed to activate extension from LP desired state (may need auth)"
                    );
                }
            }
        }
    }
}

fn apply_llm_env(llm: &TidePoolConfigureLlm) -> Result<(), String> {
    if llm.backend.trim().is_empty() {
        return Err("llm.backend must not be empty".to_string());
    }
    if llm.model.trim().is_empty() {
        return Err("llm.model must not be empty".to_string());
    }

    let registry = ProviderRegistry::load();
    let provider = registry.find(&llm.backend);

    set_runtime_env("LLM_BACKEND", &llm.backend);
    set_runtime_env("LLM_MODEL", &llm.model);

    let model_env = provider
        .map(|definition| definition.model_env.as_str())
        .unwrap_or("LLM_MODEL");
    set_runtime_env(model_env, &llm.model);

    if let Some(api_key) = llm.api_key.as_deref()
        && !api_key.trim().is_empty()
    {
        let api_key_env = provider
            .and_then(|definition| definition.api_key_env.as_deref())
            .unwrap_or("LLM_API_KEY");
        set_runtime_env(api_key_env, api_key);
        set_runtime_env("LLM_API_KEY", api_key);
    }

    if let Some(base_url) = llm.base_url.as_deref()
        && !base_url.trim().is_empty()
    {
        let base_url_env = provider
            .and_then(|definition| definition.base_url_env.as_deref())
            .unwrap_or("LLM_BASE_URL");
        set_runtime_env(base_url_env, base_url);
        set_runtime_env("LLM_BASE_URL", base_url);
    }

    Ok(())
}

fn apply_channel_env(channels: &[TidePoolConfigureChannel]) -> Result<(), String> {
    let read_string = |value: &serde_json::Value, keys: &[&str]| {
        keys.iter().find_map(|key| {
            value.get(*key).and_then(|field| match field {
                serde_json::Value::String(text) => {
                    let trimmed = text.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                }
                serde_json::Value::Number(number) => Some(number.to_string()),
                serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
                _ => None,
            })
        })
    };
    let read_csv = |value: &serde_json::Value, keys: &[&str]| {
        keys.iter().find_map(|key| {
            value.get(*key).and_then(|field| match field {
                serde_json::Value::String(text) => {
                    let trimmed = text.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                }
                serde_json::Value::Array(items) => {
                    let joined = items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::trim))
                        .filter(|item| !item.is_empty())
                        .collect::<Vec<_>>()
                        .join(",");
                    (!joined.is_empty()).then_some(joined)
                }
                _ => None,
            })
        })
    };
    let set_optional = |key: &str, value: Option<String>| {
        if let Some(value) = value {
            set_runtime_env(key, &value);
        } else {
            remove_runtime_env(key);
        }
    };

    for channel in channels {
        match channel.channel_type.as_str() {
            "dingtalk" => {
                let client_id = channel
                    .credentials
                    .get("clientId")
                    .or_else(|| channel.credentials.get("client_id"))
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "dingtalk credentials.clientId is required".to_string())?;
                let client_secret = channel
                    .credentials
                    .get("clientSecret")
                    .or_else(|| channel.credentials.get("client_secret"))
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "dingtalk credentials.clientSecret is required".to_string())?;

                set_runtime_env("DINGTALK_CLIENT_ID", client_id);
                set_runtime_env("DINGTALK_CLIENT_SECRET", client_secret);
                set_optional(
                    "DINGTALK_ROBOT_CODE",
                    read_string(&channel.credentials, &["robotCode", "robot_code"]),
                );
                set_optional(
                    "DINGTALK_MESSAGE_TYPE",
                    read_string(&channel.credentials, &["messageType", "message_type"]),
                );
                set_optional(
                    "DINGTALK_CARD_TEMPLATE_ID",
                    read_string(
                        &channel.credentials,
                        &["cardTemplateId", "card_template_id"],
                    ),
                );
                set_optional(
                    "DINGTALK_CARD_TEMPLATE_KEY",
                    read_string(
                        &channel.credentials,
                        &["cardTemplateKey", "card_template_key"],
                    ),
                );
                set_optional(
                    "DINGTALK_CARD_STREAMING_MODE",
                    read_string(
                        &channel.credentials,
                        &[
                            "cardStreamingMode",
                            "cardStreamMode",
                            "card_streaming_mode",
                            "card_stream_mode",
                        ],
                    ),
                );
                set_optional(
                    "DINGTALK_CARD_STREAM_INTERVAL",
                    read_string(
                        &channel.credentials,
                        &[
                            "cardStreamInterval",
                            "cardStreamIntervalMs",
                            "card_stream_interval",
                            "card_stream_interval_ms",
                        ],
                    ),
                );
                set_optional(
                    "DINGTALK_ACK_REACTION",
                    read_string(&channel.credentials, &["ackReaction", "ack_reaction"]),
                );
                set_optional(
                    "DINGTALK_REQUIRE_MENTION",
                    read_string(&channel.credentials, &["requireMention", "require_mention"]),
                );
                set_optional(
                    "DINGTALK_DM_POLICY",
                    read_string(&channel.credentials, &["dmPolicy", "dm_policy"]),
                );
                set_optional(
                    "DINGTALK_GROUP_POLICY",
                    read_string(&channel.credentials, &["groupPolicy", "group_policy"]),
                );
                set_optional(
                    "DINGTALK_ALLOW_FROM",
                    read_csv(&channel.credentials, &["allowFrom", "allow_from"]),
                );
                set_optional(
                    "DINGTALK_GROUP_ALLOW_FROM",
                    read_csv(
                        &channel.credentials,
                        &["groupAllowFrom", "group_allow_from"],
                    ),
                );
                set_optional(
                    "DINGTALK_GROUP_SESSION_SCOPE",
                    read_string(
                        &channel.credentials,
                        &["groupSessionScope", "group_session_scope"],
                    ),
                );
                set_optional(
                    "DINGTALK_DISPLAY_NAME_RESOLUTION",
                    read_string(
                        &channel.credentials,
                        &["displayNameResolution", "display_name_resolution"],
                    ),
                );
            }
            "signal" => {
                let account = channel
                    .credentials
                    .get("account")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "signal credentials.account is required".to_string())?;
                set_runtime_env("SIGNAL_ENABLED", "true");
                set_runtime_env("SIGNAL_HTTP_URL", &channel.endpoint_url);
                set_runtime_env("SIGNAL_ACCOUNT", account);
            }
            _ => {
                tracing::warn!(
                    channel_type = %channel.channel_type,
                    "Ignoring unsupported TidePool channel runtime injection"
                );
            }
        }
    }

    Ok(())
}

fn apply_http_env(http: &TidePoolConfigureHttp) -> Result<(), String> {
    let security_mode = match http.security_mode.trim() {
        "strict" => HttpSecurityMode::Strict,
        "infra_trusted" => HttpSecurityMode::InfraTrusted,
        _ => {
            return Err(format!(
                "http.securityMode must be 'strict' or 'infra_trusted', got '{}'",
                http.security_mode
            ));
        }
    };

    HttpSecurityConfig {
        security_mode,
        allow_private_http: http.allow_private_http,
        allow_private_ip_literals: http.allow_private_ip_literals,
    }
    .sync_runtime_env();

    Ok(())
}

async fn write_mcp_config(servers: &[TidePoolConfigureMcpServer]) -> Result<(), String> {
    let mut file = McpServersFile::default();
    for server in servers {
        // Prefer the URL provided by LobsterPool (resolved at reconfigure time).
        // Fall back to the embedded catalog for backward compat with older LP versions.
        let config = if let Some(ref url) = server.url {
            if !url.trim().is_empty() {
                Ok(McpServerConfig::new(&server.name, url))
            } else {
                resolve_mcp_server(&server.name)
            }
        } else {
            resolve_mcp_server(&server.name)
        };
        match config {
            Ok(cfg) => file.upsert(cfg),
            Err(e) => {
                tracing::warn!(
                    server_name = %server.name,
                    error = %e,
                    "Skipping unknown MCP server during configure (not in embedded catalog and no URL provided)"
                );
            }
        }
    }
    save_mcp_servers(&file)
        .await
        .map_err(|error| format!("failed to save MCP configuration: {error}"))?;
    Ok(())
}

fn resolve_mcp_server(name: &str) -> Result<McpServerConfig, String> {
    if name.trim().is_empty() {
        return Err("mcp server name must not be empty".to_string());
    }

    let catalog = load_embedded();
    let key = format!("mcp-servers/{name}");
    let manifest = catalog
        .get(&key)
        .ok_or_else(|| format!("unknown MCP server '{name}'"))?;
    let url = manifest
        .url
        .clone()
        .ok_or_else(|| format!("MCP server '{name}' is missing a URL"))?;

    Ok(McpServerConfig::new(name, url))
}

pub async fn write_persona_files(
    workspace: &Workspace,
    persona: &TidePoolConfigurePersona,
) -> Result<(), String> {
    workspace
        .write(paths::SOUL, &format!("# Soul\n\n{}", persona.soul.trim()))
        .await
        .map_err(|error| format!("failed to write SOUL.md: {error}"))?;

    if let Some(identity) = parameter_string(&persona.parameters, &["identity"]) {
        workspace
            .write(
                paths::IDENTITY,
                &format!("# Identity\n\n{}", identity.trim()),
            )
            .await
            .map_err(|error| format!("failed to write IDENTITY.md: {error}"))?;
    }

    let mut instructions = parameter_string(&persona.parameters, &["instructions"]);
    if instructions.is_none() && !persona.skills.is_empty() {
        let skill_list = persona
            .skills
            .iter()
            .map(|skill| format!("- {}", skill.name))
            .collect::<Vec<_>>()
            .join("\n");
        instructions = Some(format!("Preferred skills:\n{skill_list}"));
    }
    if let Some(instructions) = instructions {
        workspace
            .write(
                paths::AGENTS,
                &format!("# Instructions\n\n{}", instructions.trim()),
            )
            .await
            .map_err(|error| format!("failed to write AGENTS.md: {error}"))?;
    }

    if let Some(user) = parameter_string(&persona.parameters, &["user"]) {
        workspace
            .write(paths::USER, &format!("# User\n\n{}", user.trim()))
            .await
            .map_err(|error| format!("failed to write USER.md: {error}"))?;
    }

    if let Some(tools) = parameter_string(&persona.parameters, &["tools"]) {
        workspace
            .write(paths::TOOLS, &format!("# Tools\n\n{}", tools.trim()))
            .await
            .map_err(|error| format!("failed to write TOOLS.md: {error}"))?;
    }

    Ok(())
}

/// Write explicit v2 prompt documents into the workspace.
///
/// Preferred over `write_persona_files()` when the payload carries
/// `prompt_documents`. Each document maps directly to its workspace identity
/// file with correct semantic content.
pub async fn write_prompt_documents(
    workspace: &Workspace,
    docs: &TidePoolPromptDocuments,
) -> Result<(), String> {
    // IDENTITY.md — "who you are" (injected first in prompt assembly)
    if !docs.identity_md.trim().is_empty() {
        workspace
            .write(
                paths::IDENTITY,
                &format!("# Identity\n\n{}", docs.identity_md.trim()),
            )
            .await
            .map_err(|error| format!("failed to write IDENTITY.md: {error}"))?;
    }

    // SOUL.md — org personality + persona soul core (no knowledge/user context)
    workspace
        .write(paths::SOUL, &format!("# Soul\n\n{}", docs.soul_md.trim()))
        .await
        .map_err(|error| format!("failed to write SOUL.md: {error}"))?;

    // AGENTS.md — agent duties + scenario instructions + memory contracts
    if !docs.agents_md.trim().is_empty() {
        workspace
            .write(
                paths::AGENTS,
                &format!("# Instructions\n\n{}", docs.agents_md.trim()),
            )
            .await
            .map_err(|error| format!("failed to write AGENTS.md: {error}"))?;
    }

    // USER.md — user context only (layer 7)
    if !docs.user_md.trim().is_empty() {
        workspace
            .write(paths::USER, &format!("# User\n\n{}", docs.user_md.trim()))
            .await
            .map_err(|error| format!("failed to write USER.md: {error}"))?;
    }

    // TOOLS.md — tool usage notes
    if !docs.tools_md.trim().is_empty() {
        workspace
            .write(
                paths::TOOLS,
                &format!("# Tools\n\n{}", docs.tools_md.trim()),
            )
            .await
            .map_err(|error| format!("failed to write TOOLS.md: {error}"))?;
    }

    Ok(())
}

/// Write CAPABILITIES.md into the workspace listing MCP servers, skills,
/// and knowledge bases.
///
/// Called by both the initial configure flow (standby path in `main.rs`) and
/// the hot-reload path (`reconfigure_handler`). This makes reconfigure the
/// authoritative owner of the capabilities file once the seed flow is removed.
pub async fn write_capabilities_md(
    workspace: &Workspace,
    mcp_servers: &[TidePoolConfigureMcpServer],
    skills: &[TidePoolConfigureSkill],
) -> Result<(), String> {
    write_capabilities_md_with_kbs(workspace, mcp_servers, skills, &[]).await
}

/// Extended capabilities writer that also includes knowledge base references.
pub async fn write_capabilities_md_with_kbs(
    workspace: &Workspace,
    mcp_servers: &[TidePoolConfigureMcpServer],
    skills: &[TidePoolConfigureSkill],
    knowledge_bases: &[TidePoolKnowledgeBaseRef],
) -> Result<(), String> {
    let mut caps = String::from("## 已绑定的能力\n");

    if !mcp_servers.is_empty() {
        caps.push_str("\n### MCP 服务\n");
        for mcp in mcp_servers {
            caps.push_str(&format!("- **{}**\n", mcp.name));
        }
    }

    // Separate WASM tools (directly callable) from prompt skills (SKILL.md).
    // wasm_channel is a transport channel, NOT a callable tool — keep it out.
    let (wasm_tools, prompt_skills): (Vec<_>, Vec<_>) =
        skills.iter().partition(|s| s.skill_type.as_deref() == Some("wasm_tool"));

    if !wasm_tools.is_empty() {
        caps.push_str("\n### 内置工具（可直接调用）\n");
        for tool in &wasm_tools {
            caps.push_str(&format!("- **{}**", tool.name));
            if let Some(ref desc) = tool.description {
                caps.push_str(&format!(" — {}", desc));
            }
            caps.push('\n');
        }
    }

    if !prompt_skills.is_empty() {
        caps.push_str("\n### 技能\n");
        for skill in &prompt_skills {
            caps.push_str(&format!("- **{}**", skill.name));
            if let Some(ref desc) = skill.description {
                caps.push_str(&format!(" — {}", desc));
            }
            caps.push('\n');
        }
    }

    if !knowledge_bases.is_empty() {
        caps.push_str("\n### 可用知识库\n");
        for kb in knowledge_bases {
            caps.push_str(&format!("- **{}**", kb.name));
            if let Some(ref desc) = kb.description {
                let d = desc.trim();
                if !d.is_empty() {
                    caps.push_str(&format!(" — {}", d));
                }
            }
            caps.push('\n');
        }
    }

    workspace
        .write(paths::CAPABILITIES, &caps)
        .await
        .map_err(|error| format!("failed to write CAPABILITIES.md: {error}"))?;

    Ok(())
}

/// Returns `true` for WASM-based skill types that are loaded as tools at startup
/// and should NOT be written as SKILL.md prompt files.
fn is_wasm_skill_type(skill_type: Option<&str>) -> bool {
    matches!(skill_type, Some("wasm_tool") | Some("wasm_channel"))
}

fn parameter_string(parameters: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| parameters.get(key).and_then(|value| value.as_str()))
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{env_or_override, remove_runtime_env};

    #[test]
    fn configure_request_uses_camel_case() {
        let payload = TidePoolConfigureRequest {
            agent_id: Uuid::nil(),
            llm: TidePoolConfigureLlm {
                backend: "openai".to_string(),
                model: "gpt-4.1".to_string(),
                api_key: Some("secret".to_string()),
                base_url: Some("https://example.com".to_string()),
            },
            mcp_servers: vec![TidePoolConfigureMcpServer {
                name: "notion".to_string(),
                url: None,
            }],
            channels: vec![TidePoolConfigureChannel {
                channel_type: "dingtalk".to_string(),
                endpoint_url: "https://example.com".to_string(),
                credentials: serde_json::json!({"clientId": "id", "clientSecret": "sec"}),
            }],
            http: TidePoolConfigureHttp {
                security_mode: "infra_trusted".to_string(),
                allow_private_http: true,
                allow_private_ip_literals: false,
            },
            runtime_env: HashMap::new(),
            persona: TidePoolConfigurePersona {
                soul: "hello".to_string(),
                parameters: serde_json::json!({"instructions": "be helpful"}),
                skills: vec![TidePoolConfigureSkill {
                    name: "planner".to_string(),
                    content: None,
                    description: None,
                    skill_type: None,
                }],
                prompt_documents: None,
            },
            extensions: vec![],
        };

        let json = serde_json::to_value(payload).expect("serialize configure request");
        assert!(json.get("agentId").is_some());
        assert!(json.get("mcpServers").is_some());
        assert!(json.get("channelType").is_none());
    }

    #[test]
    fn apply_runtime_env_sets_runtime_overrides() {
        let keys = ["DATABASE_BACKEND", "DATABASE_URL", "IRONCLAW_OWNER_ID"];
        for key in &keys {
            remove_runtime_env(key);
        }

        let runtime_env = HashMap::from([
            ("DATABASE_BACKEND".to_string(), "postgres".to_string()),
            (
                "DATABASE_URL".to_string(),
                "postgres://lp:pw@postgres:5432/ironclaw_deadbeef".to_string(),
            ),
            (
                "IRONCLAW_OWNER_ID".to_string(),
                "34003a3d-2d95-4f9b-a145-38496dc5dce7".to_string(),
            ),
        ]);

        apply_runtime_env(&runtime_env).expect("apply runtime env");

        assert_eq!(
            env_or_override("DATABASE_BACKEND").as_deref(),
            Some("postgres")
        );
        assert_eq!(
            env_or_override("DATABASE_URL").as_deref(),
            Some("postgres://lp:pw@postgres:5432/ironclaw_deadbeef")
        );
        assert_eq!(
            env_or_override("IRONCLAW_OWNER_ID").as_deref(),
            Some("34003a3d-2d95-4f9b-a145-38496dc5dce7")
        );

        for key in &keys {
            remove_runtime_env(key);
        }
    }

    #[test]
    fn resolve_mcp_server_from_embedded_catalog() {
        let server = resolve_mcp_server("notion").expect("resolve notion");
        assert_eq!(server.name, "notion");
        assert!(server.url.starts_with("https://"));
    }

    #[tokio::test]
    async fn reconfigure_from_configured_succeeds() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let control = StandbyControl::new("test-token", tx);

        // Initial configure: Waiting → Configuring → Configured
        assert!(control.begin_configure().await.is_ok());
        control.finish_configure(true).await;
        assert_eq!(control.startup_snapshot().await.phase, "configured");

        // Reconfigure: Configured → Configuring → Configured
        assert!(control.begin_configure().await.is_ok());
        control.finish_configure(true).await;
        assert_eq!(control.startup_snapshot().await.phase, "configured");
    }

    #[tokio::test]
    async fn reconfigure_rejects_during_active_configure() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let control = StandbyControl::new("test-token", tx);

        assert!(control.begin_configure().await.is_ok());
        // Second call while still configuring should fail
        let err = control.begin_configure().await.unwrap_err();
        assert_eq!(err, "configuration is already in progress");
    }

    #[tokio::test]
    async fn reconfigure_failure_returns_to_configured_when_runtime_started() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let control = StandbyControl::new("test-token", tx);

        // Initial configure + mark runtime started
        assert!(control.begin_configure().await.is_ok());
        control.mark_runtime_started("test.runtime_ready").await;
        control.finish_configure(true).await;
        assert_eq!(control.startup_snapshot().await.phase, "configured");

        // Reconfigure attempt that fails — should return to Configured, not Waiting
        assert!(control.begin_configure().await.is_ok());
        control.finish_configure(false).await;
        assert_eq!(control.startup_snapshot().await.phase, "configured");
    }

    #[tokio::test]
    async fn initial_configure_failure_returns_to_waiting() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let control = StandbyControl::new("test-token", tx);

        // Initial configure that fails (runtime never started)
        assert!(control.begin_configure().await.is_ok());
        control.finish_configure(false).await;
        assert_eq!(control.startup_snapshot().await.phase, "waiting");
    }

    #[test]
    fn apply_channel_env_updates_dingtalk_runtime_overrides() {
        let keys = [
            "DINGTALK_MESSAGE_TYPE",
            "DINGTALK_CARD_TEMPLATE_ID",
            "DINGTALK_CARD_STREAMING_MODE",
            "DINGTALK_REQUIRE_MENTION",
            "DINGTALK_GROUP_SESSION_SCOPE",
        ];
        for key in &keys {
            remove_runtime_env(key);
        }

        let channels = vec![TidePoolConfigureChannel {
            channel_type: "dingtalk".to_string(),
            endpoint_url: "https://example.com".to_string(),
            credentials: serde_json::json!({
                "clientId": "id",
                "clientSecret": "sec",
                "messageType": "card",
                "cardTemplateId": "tpl-789",
                "cardStreamingMode": "all",
                "requireMention": true,
                "groupSessionScope": "user"
            }),
        }];

        apply_channel_env(&channels).expect("apply dingtalk runtime env");

        assert_eq!(
            env_or_override("DINGTALK_MESSAGE_TYPE").as_deref(),
            Some("card")
        );
        assert_eq!(
            env_or_override("DINGTALK_CARD_TEMPLATE_ID").as_deref(),
            Some("tpl-789")
        );
        assert_eq!(
            env_or_override("DINGTALK_CARD_STREAMING_MODE").as_deref(),
            Some("all")
        );
        assert_eq!(
            env_or_override("DINGTALK_REQUIRE_MENTION").as_deref(),
            Some("true")
        );
        assert_eq!(
            env_or_override("DINGTALK_GROUP_SESSION_SCOPE").as_deref(),
            Some("user")
        );

        for key in &keys {
            remove_runtime_env(key);
        }
    }
}
