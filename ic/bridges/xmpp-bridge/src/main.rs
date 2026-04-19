use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use ironclaw::bootstrap::ironclaw_base_dir;
use ironclaw::channels::{Channel, IncomingMessage, OutgoingResponse, XmppChannel};
use ironclaw::config::XmppConfig;
use openclaw_xmpp_bridge_contract::{
    BridgeMessage, BridgeStatusResponse, ConfigureRequest, ConfigureResponse, MessagesQuery,
    MessagesResponse, OutboundRateLimitRequest, OutboundRateLimitResponse, SendRequest,
};
use secrecy::SecretString;
use tokio::sync::RwLock;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_MAX_MESSAGES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedConfig {
    jid: String,
    password: String,
    dm_policy: String,
    allow_from: Vec<String>,
    allow_rooms: Vec<String>,
    encrypted_rooms: Vec<String>,
    device_id: u32,
    omemo_store_dir: PathBuf,
    allow_plaintext_fallback: bool,
    max_messages_per_hour: u32,
}

#[derive(Debug, Clone)]
struct QueuedMessage {
    cursor: u64,
    payload: BridgeMessage,
}

struct BridgeInner {
    configured: Option<NormalizedConfig>,
    jid: Option<String>,
    channel: Option<Arc<XmppChannel>>,
    current_cursor: u64,
    messages: VecDeque<QueuedMessage>,
}

impl Default for BridgeInner {
    fn default() -> Self {
        Self {
            configured: None,
            jid: None,
            channel: None,
            current_cursor: 0,
            messages: VecDeque::new(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<BridgeInner>>,
    expected_token: Option<String>,
    max_messages: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let bind_addr: SocketAddr = std::env::var("XMPP_BRIDGE_BIND")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string())
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid XMPP_BRIDGE_BIND: {}", e))?;
    let expected_token = std::env::var("XMPP_BRIDGE_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let max_messages = std::env::var("XMPP_BRIDGE_MAX_MESSAGES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_MESSAGES);

    let state = AppState {
        inner: Arc::new(RwLock::new(BridgeInner::default())),
        expected_token,
        max_messages,
    };

    let app = Router::new()
        .route("/v1/configure", post(configure_handler))
        .route("/v1/status", get(status_handler))
        .route("/v1/outbound-rate-limit", post(outbound_rate_limit_handler))
        .route("/v1/messages", get(messages_handler))
        .route("/v1/messages/send", post(send_handler))
        .with_state(state.clone());

    tracing::info!(addr = %bind_addr, "xmpp-bridge listening");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn configure_handler(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ConfigureRequest>,
) -> Result<Json<ConfigureResponse>, BridgeError> {
    authorize(&state, &headers, remote)?;

    let normalized = normalize_config(&request)?;

    {
        let inner = state.inner.read().await;
        if let Some(existing) = inner.configured.as_ref() {
            if existing == &normalized {
                return Ok(Json(ConfigureResponse {
                    configured: true,
                    running: inner.channel.is_some(),
                    jid: inner.jid.clone().unwrap_or_else(|| normalized.jid.clone()),
                }));
            }
            return Err(BridgeError::conflict(
                "xmpp-bridge is already configured; restart it to apply a different config",
            ));
        }
    }

    let native_config = to_native_config(&normalized);
    let channel = Arc::new(XmppChannel::new(native_config).await.map_err(|e| {
        BridgeError::bad_gateway(&format!("failed to initialize XMPP client: {}", e))
    })?);
    let mut stream = channel
        .start()
        .await
        .map_err(|e| BridgeError::bad_gateway(&format!("failed to start XMPP client: {}", e)))?;

    {
        let mut inner = state.inner.write().await;
        inner.configured = Some(normalized.clone());
        inner.jid = Some(normalized.jid.clone());
        inner.channel = Some(Arc::clone(&channel));
    }

    let state_for_task = state.clone();
    tokio::spawn(async move {
        while let Some(message) = stream.next().await {
            enqueue_message(&state_for_task, message).await;
        }
        tracing::warn!("xmpp-bridge input stream ended");
    });

    Ok(Json(ConfigureResponse {
        configured: true,
        running: true,
        jid: normalized.jid,
    }))
}

async fn status_handler(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BridgeStatusResponse>, BridgeError> {
    authorize(&state, &headers, remote)?;

    let (
        configured,
        running,
        current_cursor,
        queued_messages,
        jid,
        configured_rooms,
        configured_limit,
        channel,
    ) = {
        let inner = state.inner.read().await;
        (
            inner.configured.is_some(),
            inner.channel.is_some(),
            inner.current_cursor,
            inner.messages.len(),
            inner.jid.clone(),
            inner
                .configured
                .as_ref()
                .map(|config| config.allow_rooms.clone())
                .unwrap_or_default(),
            inner
                .configured
                .as_ref()
                .map(|config| config.max_messages_per_hour)
                .unwrap_or(0),
            inner.channel.clone(),
        )
    };

    let diagnostics = if let Some(channel) = channel.as_ref() {
        Some(channel.omemo_diagnostics().await)
    } else {
        None
    };
    let presence_diagnostics = if let Some(channel) = channel.as_ref() {
        Some(channel.muc_presence_diagnostics().await)
    } else {
        None
    };
    let room_diagnostics = if let Some(channel) = channel.as_ref() {
        Some(channel.muc_encryption_diagnostics().await)
    } else {
        None
    };
    let outbound_rate_limit = if let Some(channel) = channel.as_ref() {
        Some(channel.outbound_rate_limit_diagnostics().await)
    } else {
        None
    };
    let active_limit = outbound_rate_limit
        .as_ref()
        .map(|value| value.max_messages_per_hour)
        .unwrap_or(configured_limit);
    let outbound_messages_last_hour = outbound_rate_limit
        .as_ref()
        .map(|value| value.messages_in_current_window)
        .unwrap_or(0);

    Ok(Json(BridgeStatusResponse {
        configured,
        running,
        current_cursor,
        queued_messages,
        jid,
        configured_rooms,
        rooms_with_presence: presence_diagnostics
            .map(|value| value.rooms_with_presence)
            .unwrap_or_default(),
        configured_max_messages_per_hour: configured_limit,
        active_max_messages_per_hour: active_limit,
        outbound_messages_last_hour,
        outbound_rate_limit_overridden: active_limit != configured_limit,
        omemo_enabled: diagnostics
            .as_ref()
            .is_some_and(|value| value.omemo_enabled),
        device_id: diagnostics.as_ref().and_then(|value| value.device_id),
        fingerprint: diagnostics
            .as_ref()
            .and_then(|value| value.fingerprint.clone()),
        bundle_published: diagnostics
            .as_ref()
            .is_some_and(|value| value.bundle_published),
        prekeys_available: diagnostics
            .as_ref()
            .map(|value| value.prekeys_available)
            .unwrap_or(0),
        migration_state: diagnostics
            .as_ref()
            .map(|value| value.migration_state.clone()),
        last_omemo_error: diagnostics.and_then(|value| value.last_omemo_error),
        encrypted_rooms_total: room_diagnostics
            .as_ref()
            .map(|value| value.encrypted_rooms_total)
            .unwrap_or(0),
        encrypted_rooms_ready: room_diagnostics
            .as_ref()
            .map(|value| value.encrypted_rooms_ready)
            .unwrap_or(0),
        last_room_error: room_diagnostics.and_then(|value| value.last_room_error),
    }))
}

async fn outbound_rate_limit_handler(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OutboundRateLimitRequest>,
) -> Result<Json<OutboundRateLimitResponse>, BridgeError> {
    authorize(&state, &headers, remote)?;

    let (configured, configured_limit, channel) = {
        let inner = state.inner.read().await;
        (
            inner.configured.is_some(),
            inner
                .configured
                .as_ref()
                .map(|config| config.max_messages_per_hour)
                .unwrap_or(0),
            inner.channel.clone(),
        )
    };

    let channel =
        channel.ok_or_else(|| BridgeError::conflict("xmpp-bridge is not configured yet"))?;
    let current = channel.outbound_rate_limit_diagnostics().await;
    let next_limit = request
        .max_messages_per_hour
        .unwrap_or(current.max_messages_per_hour);
    let updated = channel
        .set_outbound_rate_limit(next_limit, request.reset_counter)
        .await;

    tracing::info!(
        configured_max_messages_per_hour = configured_limit,
        active_max_messages_per_hour = updated.max_messages_per_hour,
        outbound_messages_last_hour = updated.messages_in_current_window,
        reset_counter = request.reset_counter,
        "Updated live XMPP outbound rate limit"
    );

    Ok(Json(OutboundRateLimitResponse {
        configured,
        running: true,
        configured_max_messages_per_hour: configured_limit,
        active_max_messages_per_hour: updated.max_messages_per_hour,
        outbound_messages_last_hour: updated.messages_in_current_window,
        reset_counter_applied: request.reset_counter,
    }))
}

async fn messages_handler(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MessagesQuery>,
) -> Result<Json<MessagesResponse>, BridgeError> {
    authorize(&state, &headers, remote)?;

    let cursor = query.cursor.unwrap_or(0);
    let inner = state.inner.read().await;
    let messages = inner
        .messages
        .iter()
        .filter(|message| message.cursor > cursor)
        .map(|message| message.payload.clone())
        .collect();

    Ok(Json(MessagesResponse {
        cursor: inner.current_cursor,
        messages,
    }))
}

async fn send_handler(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SendRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    authorize(&state, &headers, remote)?;

    let metadata: serde_json::Value = serde_json::from_str(&request.metadata_json)
        .map_err(|e| BridgeError::bad_request(&format!("invalid metadata_json: {}", e)))?;

    let channel = {
        let inner = state.inner.read().await;
        inner.channel.clone()
    }
    .ok_or_else(|| BridgeError::conflict("xmpp-bridge is not configured yet"))?;

    let mut response = OutgoingResponse::text(request.content);
    response.metadata = metadata;

    channel
        .broadcast(&request.target, response)
        .await
        .map_err(|e| BridgeError::bad_gateway(&format!("failed to send XMPP message: {}", e)))?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn enqueue_message(state: &AppState, message: IncomingMessage) {
    let metadata_json =
        serde_json::to_string(&message.metadata).unwrap_or_else(|_| "{}".to_string());
    let payload = BridgeMessage {
        message_id: message.id.to_string(),
        user_id: message.user_id,
        user_name: message.user_name,
        content: message.content,
        thread_id: message.thread_id,
        metadata_json,
    };

    let mut inner = state.inner.write().await;
    inner.current_cursor = inner.current_cursor.saturating_add(1);
    let cursor = inner.current_cursor;
    inner.messages.push_back(QueuedMessage { cursor, payload });
    while inner.messages.len() > state.max_messages {
        inner.messages.pop_front();
    }
}

fn authorize(state: &AppState, headers: &HeaderMap, remote: SocketAddr) -> Result<(), BridgeError> {
    if !remote.ip().is_loopback() {
        return Err(BridgeError::forbidden(
            "xmpp-bridge only accepts loopback clients",
        ));
    }

    let Some(expected_token) = state.expected_token.as_deref() else {
        return Ok(());
    };

    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| BridgeError::unauthorized("missing Authorization header"))?;

    let provided = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .ok_or_else(|| BridgeError::unauthorized("Authorization header must use Bearer auth"))?;

    if provided != expected_token {
        return Err(BridgeError::unauthorized("invalid bridge token"));
    }

    Ok(())
}

fn normalize_config(request: &ConfigureRequest) -> Result<NormalizedConfig, BridgeError> {
    let jid = request.jid.trim();
    if jid.is_empty() {
        return Err(BridgeError::bad_request("jid is required"));
    }
    let password = request.password.trim();
    if password.is_empty() {
        return Err(BridgeError::bad_request("password is required"));
    }

    let resource = request
        .resource
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let jid = if jid.contains('/') || resource.is_none() {
        jid.to_string()
    } else {
        format!("{}/{}", jid, resource.unwrap_or_default())
    };

    let omemo_store_dir = request
        .omemo_store_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| ironclaw_base_dir().join("xmpp"));

    Ok(NormalizedConfig {
        jid,
        password: password.to_string(),
        dm_policy: normalize_policy(&request.dm_policy),
        allow_from: normalize_list(&request.allow_from),
        allow_rooms: {
            let mut rooms = normalize_list(&request.allow_rooms);
            let encrypted_rooms = normalize_list(&request.encrypted_rooms);
            if !rooms.iter().any(|room| room == "*") {
                for room in &encrypted_rooms {
                    if !rooms
                        .iter()
                        .any(|value: &String| value.eq_ignore_ascii_case(room))
                    {
                        rooms.push(room.clone());
                    }
                }
            }
            rooms
        },
        encrypted_rooms: normalize_list(&request.encrypted_rooms),
        device_id: request.device_id,
        omemo_store_dir,
        allow_plaintext_fallback: request.allow_plaintext_fallback,
        max_messages_per_hour: request.max_messages_per_hour,
    })
}

fn to_native_config(config: &NormalizedConfig) -> XmppConfig {
    XmppConfig {
        jid: config.jid.clone(),
        password: SecretString::from(config.password.clone()),
        allow_from: config.allow_from.clone(),
        dm_policy: config.dm_policy.clone(),
        allow_rooms: config.allow_rooms.clone(),
        encrypted_rooms: config.encrypted_rooms.clone(),
        device_id: config.device_id,
        omemo_store_dir: config.omemo_store_dir.clone(),
        allow_plaintext_fallback: config.allow_plaintext_fallback,
        max_messages_per_hour: config.max_messages_per_hour,
    }
}

fn normalize_policy(policy: &str) -> String {
    let normalized = policy.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "allowlist".to_string()
    } else {
        normalized
    }
}

fn normalize_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

struct BridgeError {
    status: StatusCode,
    message: String,
}

impl BridgeError {
    fn bad_request(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    fn unauthorized(message: &str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.to_string(),
        }
    }

    fn forbidden(message: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.to_string(),
        }
    }

    fn conflict(message: &str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.to_string(),
        }
    }

    fn bad_gateway(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.to_string(),
        }
    }
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}
