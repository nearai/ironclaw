#![allow(dead_code)]

//! WeeChat Relay WASM channel for IronClaw.
//!
//! Connects to WeeChat's API relay protocol (v2) via HTTP polling or WebSocket.
//! This channel bridges IronClaw agents to any IRC network supported by WeeChat.
//!
//! # Architecture
//!
//!   IronClaw host → (on_poll) → this WASM → HTTP GET /api/buffers/*/lines → WeeChat
//!   IronClaw host → (on_respond) → this WASM → HTTP POST /api/input → WeeChat
//!
//! # Connection Modes
//!
//! - **auto** (default): Use WebSocket adapter if running, else fall back to HTTP polling
//! - **websocket**: Always route through ws_adapter.py (error if adapter not running)
//! - **http**: Direct HTTP polling of WeeChat relay every 3-5 seconds
//!
//! The WebSocket adapter (`ws_adapter.py`) runs as a separate process, holds a
//! persistent WebSocket connection to WeeChat, and exposes a local HTTP API that
//! mirrors WeeChat's REST format. The WASM polls the adapter instead of WeeChat
//! directly, getting real-time message delivery at WebSocket latency.
//!
//! # Features
//!
//! - Multi-network IRC support (libera, OFTC, ergo, darkirc, etc.)
//! - DM and group channel support
//! - Message chunking for IRC line length limits
//! - Per-buffer watermarking to avoid replaying history
//! - Network filtering (allowlist/denylist)
//!
//! # Security
//!
//! - Relay password injected by host via config
//! - HTTP requests restricted to configured relay endpoint
//! - All IronClaw security layers apply (prompt injection defense, rate limiting)

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// WeeChat API Types
// ============================================================================

/// Response from GET /api/version
#[derive(Debug, Deserialize)]
struct VersionResponse {
    weechat_version: Option<String>,
    relay_api_version: Option<String>,
}

/// Response from GET /api/buffers
#[derive(Debug, Deserialize)]
struct BuffersResponse {
    buffers: Option<Vec<BufferInfo>>,
}

/// Single buffer info
#[derive(Debug, Deserialize, Serialize, Clone)]
struct BufferInfo {
    id: Option<i64>,
    #[serde(alias = "name")]
    full_name: Option<String>,
    short_name: Option<String>,
}

/// Response from GET /api/buffers/<name>/lines
#[derive(Debug, Deserialize)]
struct LinesResponse {
    lines: Option<Vec<LineInfo>>,
}

/// Single IRC line
#[derive(Debug, Deserialize)]
struct LineInfo {
    id: Option<i64>,
    date: Option<String>,
    date_printed: Option<String>,
    tags: Option<Vec<String>>,
    prefix: Option<String>,
    message: Option<String>,
}

/// Request body for POST /api/input
#[derive(Debug, Serialize)]
struct InputRequest {
    buffer_name: String,
    command: String,
}

// ============================================================================
// Channel Configuration
// ============================================================================

/// Configuration from weechat.capabilities.json, injected by host via on_start.
#[derive(Debug, Deserialize)]
struct WeechatConfig {
    /// HTTP URL of WeeChat relay (e.g., http://127.0.0.1:9001)
    #[serde(default = "default_relay_url")]
    relay_url: String,

    /// Relay password (plain text, injected by host)
    #[serde(default, alias = "weechat_relay_password")]
    relay_password: String,

    /// Connection mode: "auto" (default), "websocket", or "http"
    /// - auto: use ws_adapter if reachable, else fall back to HTTP polling
    /// - websocket: always use ws_adapter (fails if not running)
    /// - http: direct HTTP polling of WeeChat relay
    #[serde(default = "default_connection_mode")]
    connection_mode: String,

    /// URL of the ws_adapter.py process (default: http://127.0.0.1:6681)
    /// Used when connection_mode is "websocket" or "auto".
    #[serde(default = "default_ws_adapter_url")]
    ws_adapter_url: String,

    /// Networks to monitor (empty = all networks)
    #[serde(default)]
    networks: Vec<String>,

    /// Networks to exclude
    #[serde(default)]
    exclude_networks: Vec<String>,

    /// Regex filter for buffer names (applied to full_name)
    #[serde(default)]
    buffer_filter: Option<String>,

    /// DM policy: "open", "allowlist", or "pairing" (default "pairing")
    #[serde(default = "default_dm_policy")]
    dm_policy: String,

    /// Group policy: "open", "allowlist", or "deny" (default "allowlist")
    #[serde(default = "default_group_policy")]
    group_policy: String,

    /// Allowlisted sender IDs (nick or nick!user@host)
    #[serde(default)]
    allow_from: Vec<String>,

    /// Max characters per IRC message chunk
    #[serde(default = "default_max_chunk_length")]
    max_chunk_length: usize,

    /// Poll interval in seconds (minimum 3)
    #[serde(default = "default_poll_interval")]
    poll_interval_seconds: u32,

    /// Log the reason every time a message is silently dropped.
    #[serde(default)]
    verbose_drops: bool,
}

fn default_relay_url() -> String {
    "http://127.0.0.1:9001".to_string()
}

fn default_connection_mode() -> String {
    "auto".to_string()
}

fn default_ws_adapter_url() -> String {
    "http://127.0.0.1:6681".to_string()
}

fn default_dm_policy() -> String {
    "open".to_string()
}

fn default_group_policy() -> String {
    "allowlist".to_string()
}

fn default_max_chunk_length() -> usize {
    420
}

fn default_poll_interval() -> u32 {
    3
}

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct WeechatMessageMetadata {
    /// Full buffer name (e.g., "irc.libera.#openclaw")
    buffer: String,
    /// Network name (e.g., "libera")
    network: String,
    /// Target (channel or DM nick)
    target: String,
    /// Sender nick
    nick: String,
    /// Is this a DM or group channel?
    is_dm: bool,
}

// ============================================================================
// Workspace Paths
// ============================================================================

const CHANNEL_NAME: &str = "weechat";
const RELAY_URL_PATH: &str = "state/relay_url";
const RELAY_PASSWORD_PATH: &str = "state/relay_password";
const CONNECTION_MODE_PATH: &str = "state/connection_mode";
const NETWORKS_PATH: &str = "state/networks";
const EXCLUDE_NETWORKS_PATH: &str = "state/exclude_networks";
const BUFFER_FILTER_PATH: &str = "state/buffer_filter";
const DM_POLICY_PATH: &str = "state/dm_policy";
const GROUP_POLICY_PATH: &str = "state/group_policy";
const ALLOW_FROM_PATH: &str = "state/allow_from";
const MAX_CHUNK_LENGTH_PATH: &str = "state/max_chunk_length";
const LAST_SEEN_DATES_PATH: &str = "state/last_seen_dates";  // JSON: {buffer: timestamp_ms} (unused, kept for migration)
const LAST_SEEN_IDS_PATH: &str = "state/last_seen_ids";      // JSON: {buffer: last_line_id}
const BUFFER_LIST_PATH: &str = "state/buffer_list";  // JSON: [BufferInfo]
const WS_ADAPTER_URL_PATH: &str = "state/ws_adapter_url";
const VERBOSE_DROPS_PATH: &str = "state/verbose_drops";

// ============================================================================
// Channel Implementation
// ============================================================================

struct WeechatRelayChannel;

impl Guest for WeechatRelayChannel {
    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
     Ok(())
    } 
    /// Initialize the channel. Persist config to workspace and verify connectivity.
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("WeeChat Relay channel config: {}", config_json),
        );

        let config: WeechatConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("WeeChat Relay channel starting, relay at {}", config.relay_url),
        );

        // Normalize relay URL (strip trailing slashes, /api suffix)
        let relay_url = normalize_relay_url(&config.relay_url);

        // Persist config for subsequent callbacks.
        // IronClaw passes {} as config_json so all values here are serde defaults.
        // The actual allow_from/dm_policy/group_policy will be loaded from the
        // ws_adapter's /api/config endpoint on the first poll.
        let _ = channel_host::workspace_write(RELAY_URL_PATH, &relay_url);
        let _ = channel_host::workspace_write(RELAY_PASSWORD_PATH, &config.relay_password);
        let _ = channel_host::workspace_write(CONNECTION_MODE_PATH, &config.connection_mode);
        let _ = channel_host::workspace_write(WS_ADAPTER_URL_PATH, &config.ws_adapter_url);
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &config.dm_policy);
        let _ = channel_host::workspace_write(GROUP_POLICY_PATH, &config.group_policy);
        let _ = channel_host::workspace_write(MAX_CHUNK_LENGTH_PATH, &config.max_chunk_length.to_string());
        let _ = channel_host::workspace_write(VERBOSE_DROPS_PATH, if config.verbose_drops { "true" } else { "false" });

        let networks_json = serde_json::to_string(&config.networks).unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(NETWORKS_PATH, &networks_json);

        let exclude_json = serde_json::to_string(&config.exclude_networks).unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(EXCLUDE_NETWORKS_PATH, &exclude_json);

        let allow_from_json = serde_json::to_string(&config.allow_from).unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        if let Some(filter) = &config.buffer_filter {
            let _ = channel_host::workspace_write(BUFFER_FILTER_PATH, filter);
        }

        // Validate relay connectivity
        match check_relay_health(&relay_url, &config.relay_password) {
            Ok(version_info) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Connected to WeeChat {} (API v{})",
                        version_info.0, version_info.1),
                );
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Relay not reachable (will retry on poll): {}", e),
                );
            }
        }

        // Determine poll URL — use adapter if running so seed_watermarks respects ?limit=1.
        // WeeChat direct ignores limit and returns all lines (potentially large).
        let poll_url = resolve_poll_url(
            &config.connection_mode,
            &relay_url,
            &config.ws_adapter_url,
            &config.relay_password,
        );

        // Initialize buffer list and watermarks
        if let Ok(buffers) = fetch_buffer_list(&poll_url, &config.relay_password) {
            let irc_buffers = filter_irc_buffers(&buffers);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Found {} IRC buffers", irc_buffers.len()),
            );

            // Save buffer list
            if let Ok(json) = serde_json::to_string(&irc_buffers) {
                let _ = channel_host::workspace_write(BUFFER_LIST_PATH, &json);
            }

            // Seed watermarks via poll_url so ?limit=1 is respected
            seed_watermarks(&poll_url, &config.relay_password, &irc_buffers);
        }

        // In websocket/auto mode the adapter buffers messages, so we poll it
        // frequently to drain the queue. HTTP mode polls WeeChat directly.
        // All modes use the same interval — the adapter just responds faster.
        let interval_ms = (config.poll_interval_seconds.max(3) * 1000).max(3000);

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Connection mode: {} (ws_adapter: {}, poll interval: {}ms)",
                config.connection_mode, config.ws_adapter_url, interval_ms
            ),
        );

        Ok(ChannelConfig {
            display_name: "WeeChat Relay".to_string(),
            http_endpoints: vec![],  // No inbound webhooks needed
            poll: Some(PollConfig {
                interval_ms,
                enabled: true,
            }),
        })
    }

    /// No-op: WeeChat doesn't receive inbound webhooks.
    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        json_response(
            404,
            serde_json::json!({"error": "WeeChat channel does not accept webhooks"}),
        )
    }

    /// Poll for new IRC messages and emit them to the agent.
    ///
    /// In "auto" mode: tries the WebSocket adapter first, falls back to direct
    /// HTTP polling if the adapter is not reachable.
    /// In "websocket" mode: always uses the adapter (logs warning if unavailable).
    /// In "http" mode: polls WeeChat relay directly (classic behavior).
    fn on_poll() {
        let relay_url = channel_host::workspace_read(RELAY_URL_PATH)
            .unwrap_or_else(default_relay_url);
        let relay_password = channel_host::workspace_read(RELAY_PASSWORD_PATH)
            .unwrap_or_default();
        let connection_mode = channel_host::workspace_read(CONNECTION_MODE_PATH)
            .unwrap_or_else(default_connection_mode);
        let adapter_url = channel_host::workspace_read(WS_ADAPTER_URL_PATH)
            .unwrap_or_else(default_ws_adapter_url);

        let poll_url = resolve_poll_url(&connection_mode, &relay_url, &adapter_url, &relay_password);
        do_poll(&poll_url, &relay_url, &relay_password);
    }

    /// Deliver the agent's response back to IRC via WeeChat relay.
    fn on_respond(response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("on_respond metadata_json={}", response.metadata_json),
        );
        let metadata: WeechatMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        let relay_url = channel_host::workspace_read(RELAY_URL_PATH)
            .unwrap_or_else(default_relay_url);
        let relay_password = channel_host::workspace_read(RELAY_PASSWORD_PATH)
            .unwrap_or_default();

        let max_chunk = channel_host::workspace_read(MAX_CHUNK_LENGTH_PATH)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(420);

        // Split response into IRC-friendly chunks
        let chunks = split_message(&response.content, max_chunk);

        let mut successful_chunks = 0;
        let mut last_error = None;

        for chunk in &chunks {
            let result = if metadata.is_dm {
                send_dm(&relay_url, &relay_password, &metadata.buffer, &metadata.network, &metadata.target, chunk)
            } else {
                send_input(&relay_url, &relay_password, &metadata.buffer, chunk)
            };

            match result {
                Ok(()) => {
                    successful_chunks += 1;
                }
                Err(e) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to send chunk {} to '{}': {}",
                            successful_chunks + 1, metadata.buffer, e),
                    );
                    last_error = Some(e);
                }
            }

            // Small delay between chunks to avoid flooding
            if chunks.len() > 1 {
                // Note: WASM can't sleep, but WeeChat handles flood protection
            }
        }

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Sent {} of {} chunk(s) to '{}' ({} chars total)",
                successful_chunks,
                chunks.len(),
                metadata.buffer,
                response.content.len(),
            ),
        );

        if successful_chunks > 0 {
            Ok(())
        } else {
            Err(last_error.unwrap_or_else(|| "Failed to send any chunks".to_string()))
        }
    }

    /// Forward actionable status updates to IRC.
    fn on_status(update: StatusUpdate) {
        match update.status {
            StatusType::ApprovalNeeded
            | StatusType::AuthRequired
            | StatusType::AuthCompleted
            | StatusType::JobStarted => {
                let message = update.message.trim();
                if message.is_empty() {
                    return;
                }

                let metadata: WeechatMessageMetadata = match serde_json::from_str(&update.metadata_json) {
                    Ok(m) => m,
                    Err(_) => return,
                };

                let relay_url = channel_host::workspace_read(RELAY_URL_PATH)
                    .unwrap_or_else(default_relay_url);
                let relay_password = channel_host::workspace_read(RELAY_PASSWORD_PATH)
                    .unwrap_or_default();

                let truncated = if message.len() > 400 {
                    format!("{}...", &message[..397])
                } else {
                    message.to_string()
                };

                let status_text = format!("[status] {}", truncated);

                let send_result = if metadata.is_dm {
                    send_dm(&relay_url, &relay_password, &metadata.buffer, &metadata.network, &metadata.target, &status_text)
                } else {
                    send_input(&relay_url, &relay_password, &metadata.buffer, &status_text)
                };
                if let Err(e) = send_result {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Failed to send status to '{}': {}", metadata.buffer, e),
                    );
                }
            }
            _ => {}
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "WeeChat Relay channel shutting down",
        );
    }
}

// ============================================================================
// Drop Logging
// ============================================================================

fn drop_log(_verbose: bool, reason: &str) {
    channel_host::log(channel_host::LogLevel::Warn, &format!("[drop] {}", reason));
}

// ============================================================================
// Poll URL Resolution
// ============================================================================

/// Determine which URL to use for polling based on connection mode.
///
/// - "http": always relay_url (direct WeeChat HTTP polling)
/// - "websocket": always adapter_url (adapter must be running)
/// - "auto": probe adapter; use it if healthy, else fall back to relay_url
fn resolve_poll_url(mode: &str, relay_url: &str, adapter_url: &str, password: &str) -> String {
    match mode {
        "http" => {
            relay_url.to_string()
        }
        "websocket" => {
            if adapter_url.is_empty() {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    "connection_mode=websocket but ws_adapter_url is empty; falling back to HTTP polling",
                );
                relay_url.to_string()
            } else {
                if !is_adapter_healthy(adapter_url, password) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("WebSocket adapter at {} is not reachable (mode=websocket, no fallback)", adapter_url),
                    );
                }
                adapter_url.to_string()
            }
        }
        _ => {
            // "auto": try adapter, fall back silently to HTTP polling
            if !adapter_url.is_empty() && is_adapter_healthy(adapter_url, password) {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("auto mode: adapter healthy at {}, using it", adapter_url),
                );
                adapter_url.to_string()
            } else {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("auto mode: adapter health check failed for {}, using relay_url directly", adapter_url),
                );
                relay_url.to_string()
            }
        }
    }
}

/// Quick health check against the adapter's /api/version endpoint.
fn is_adapter_healthy(adapter_url: &str, password: &str) -> bool {
    let url = format!("{}/api/version", adapter_url);
    http_get(&url, password, 2_000)
        .map(|r| r.status == 200)
        .unwrap_or(false)
}

// ============================================================================
// Polling Implementation
// ============================================================================

/// Poll poll_url for new lines across all IRC buffers and emit them to the agent.
///
/// poll_url is either the WeeChat relay URL (HTTP mode) or the ws_adapter URL
/// (websocket/auto mode). In both cases the HTTP API shape is identical.
/// relay_url is always used for sending responses (POST /api/input).
fn do_poll(poll_url: &str, relay_url: &str, relay_password: &str) {

    // Always fetch config from ws_adapter's /api/config so changes to
    // weechat_local_config.json are picked up without removing the channel.
    {
        let adapter_url = channel_host::workspace_read(WS_ADAPTER_URL_PATH)
            .unwrap_or_else(default_ws_adapter_url);
        let cfg_url = format!("{}/api/config", normalize_relay_url(&adapter_url));
        if let Ok(resp) = http_get(&cfg_url, "", 3_000) {
            if resp.status == 200 {
                if let Ok(cfg) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
                    if let Some(v) = cfg["dm_policy"].as_str() {
                        let _ = channel_host::workspace_write(DM_POLICY_PATH, v);
                    }
                    if let Some(v) = cfg["group_policy"].as_str() {
                        let _ = channel_host::workspace_write(GROUP_POLICY_PATH, v);
                    }
                    if let Some(arr) = cfg["allow_from"].as_array() {
                        if let Ok(json) = serde_json::to_string(arr) {
                            let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &json);
                        }
                    }
                    if let Some(arr) = cfg["networks"].as_array() {
                        if let Ok(json) = serde_json::to_string(arr) {
                            let _ = channel_host::workspace_write(NETWORKS_PATH, &json);
                        }
                    }
                    channel_host::log(channel_host::LogLevel::Info,
                        &format!("Loaded config from adapter: dm_policy={:?} group_policy={:?} allow_from={:?}",
                            cfg["dm_policy"], cfg["group_policy"], cfg["allow_from"]));
                }
            }
        }
    }

    // Load buffer list
    let buffers: Vec<BufferInfo> = channel_host::workspace_read(BUFFER_LIST_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if buffers.is_empty() {
        channel_host::log(
            channel_host::LogLevel::Warn,
            "poll: buffer list empty, refreshing",
        );
        // Try to refresh buffer list
        match fetch_buffer_list(poll_url, relay_password) {
            Ok(new_buffers) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Fetched {} total buffers: {:?}",
                        new_buffers.len(),
                        new_buffers.iter().filter_map(|b| b.full_name.as_deref()).collect::<Vec<_>>()),
                );
                let irc_buffers = filter_irc_buffers(&new_buffers);
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Filtered to {} IRC buffers", irc_buffers.len()),
                );
                if !irc_buffers.is_empty() {
                    if let Ok(json) = serde_json::to_string(&irc_buffers) {
                        let _ = channel_host::workspace_write(BUFFER_LIST_PATH, &json);
                    }
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to fetch buffer list: {}", e),
                );
            }
        }
        return;
    }

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!("Polling {} buffers via {}", buffers.len(), poll_url),
    );

    // Load watermarks (line-ID based)
    let mut last_seen_ids: HashMap<String, i64> = channel_host::workspace_read(LAST_SEEN_IDS_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut updated = false;

    // Poll each buffer
    for buffer in &buffers {
        if let Some(full_name) = &buffer.full_name {
            // Track whether this buffer has been seen before.
            // On first poll (watermark = -1), seed the watermark without emitting.
            let first_time = !last_seen_ids.contains_key(full_name);

            match poll_buffer(poll_url, relay_password, full_name, &last_seen_ids) {
                Ok(new_lines) => {
                    if !new_lines.is_empty() {
                        channel_host::log(
                            channel_host::LogLevel::Info,
                            &format!("Buffer {}: {} new lines{}", full_name, new_lines.len(),
                                if first_time { " (seeding watermark, not emitting)" } else { "" }),
                        );
                    }
                    for (line, line_id) in new_lines {
                        // Update watermark always
                        if line_id > *last_seen_ids.get(full_name).unwrap_or(&-1) {
                            last_seen_ids.insert(full_name.clone(), line_id);
                            updated = true;
                        }
                        // Only emit on subsequent polls, not the first time
                        if !first_time {
                            handle_inbound_line(full_name, &line);
                        }
                    }
                }
                Err(e) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to poll {}: {}", full_name, e),
                    );
                }
            }
        }
    }

    // Save updated watermarks
    if updated {
        if let Ok(json) = serde_json::to_string(&last_seen_ids) {
            let _ = channel_host::workspace_write(LAST_SEEN_IDS_PATH, &json);
        }
    }

    // Periodically refresh buffer list (every 30 polls = ~90s at 3s interval)
    let poll_count = channel_host::workspace_read("state/poll_count")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        .wrapping_add(1);
    let _ = channel_host::workspace_write("state/poll_count", &poll_count.to_string());
    if poll_count % 30 == 0 {
        if let Ok(new_buffers) = fetch_buffer_list(poll_url, relay_password) {
            let irc_buffers = filter_irc_buffers(&new_buffers);
            if let Ok(json) = serde_json::to_string(&irc_buffers) {
                let _ = channel_host::workspace_write(BUFFER_LIST_PATH, &json);
            }
        }
    }

    // Suppress unused variable warning — relay_url is used by on_respond, not here
    let _ = relay_url;
}

/// Poll a single buffer for new lines.
fn poll_buffer(
    relay_url: &str,
    relay_password: &str,
    buffer_name: &str,
    last_seen_ids: &HashMap<String, i64>,
) -> Result<Vec<(LineInfo, i64)>, String> {
    let encoded_name = encode_buffer_name(buffer_name);
    let url = format!("{}/api/buffers/{}/lines?limit=10", relay_url, encoded_name);

    let response = http_get(&url, relay_password, 5_000)?;

    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }

    // Response is a bare JSON array of line objects
    let line_values: Vec<serde_json::Value> = serde_json::from_slice(&response.body)
        .unwrap_or_default();
    let lines: Vec<LineInfo> = line_values.iter().map(|v| LineInfo {
        id: v["id"].as_i64(),
        date: v["date"].as_str().map(String::from),
        date_printed: v["date_printed"].as_str().map(String::from),
        tags: v["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()),
        prefix: v["prefix"].as_str().map(String::from),
        message: v["message"].as_str().map(String::from),
    }).collect();

    if lines.is_empty() {
        return Ok(vec![]);
    }

    let verbose = channel_host::workspace_read(VERBOSE_DROPS_PATH)
        .map(|s| s == "true")
        .unwrap_or(false);

    let last_seen_id = last_seen_ids.get(buffer_name).copied().unwrap_or(-1);
    let mut new_lines = Vec::new();

    // Process chronologically (reverse API order which is newest-first)
    for line in lines.into_iter().rev() {
        let line_id = line.id.unwrap_or(-1);

        // Skip already-seen lines by ID
        if line_id <= last_seen_id {
            drop_log(verbose, &format!("line skipped (id watermark): id {} <= last_seen {} in {}", line_id, last_seen_id, buffer_name));
            continue;
        }

        // Filter for PRIVMSG only
        if let Some(tags) = &line.tags {
            if !tags.iter().any(|t| t == "irc_privmsg") {
                drop_log(verbose, &format!("line skipped (not irc_privmsg): tags={:?} in {}", tags, buffer_name));
                continue;
            }
            if tags.iter().any(|t| t == "self_msg" || t == "no_log") {
                drop_log(verbose, &format!("line skipped (self_msg or no_log) in {}", buffer_name));
                continue;
            }
        }

        new_lines.push((line, line_id));
    }

    Ok(new_lines)
}


// ============================================================================
// Inbound Message Handling
// ============================================================================

/// Process a single inbound IRC line and emit to agent if policy allows.
fn handle_inbound_line(buffer_name: &str, line: &LineInfo) {
    let verbose = channel_host::workspace_read(VERBOSE_DROPS_PATH)
        .map(|s| s == "true")
        .unwrap_or(false);

    // Parse buffer name: irc.<network>.<target>
    let parts: Vec<&str> = buffer_name.split('.').collect();
    if parts.len() < 3 || parts[0] != "irc" {
        drop_log(verbose, &format!("line dropped (invalid buffer name format): {}", buffer_name));
        return;
    }

    let network = parts[1];
    let target = parts[2..].join(".");

    // Check network filters
    let networks: Vec<String> = channel_host::workspace_read(NETWORKS_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if !networks.is_empty() && !networks.iter().any(|n| n == network) {
        drop_log(verbose, &format!("line dropped (network not in allowlist): network={}, allowed={:?}", network, networks));
        return;
    }

    let exclude_networks: Vec<String> = channel_host::workspace_read(EXCLUDE_NETWORKS_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if exclude_networks.iter().any(|n| n == network) {
        drop_log(verbose, &format!("line dropped (network excluded): network={}", network));
        return;
    }

    // Extract message details
    let tags = line.tags.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);

    let nick = extract_nick_from_tags(tags)
        .or_else(|| line.prefix.as_ref().map(|s| s.as_str()))
        .unwrap_or("unknown");

    let host = extract_host_from_tags(tags).unwrap_or("");
    let hostmask = if !host.is_empty() {
        format!("{}!{}", nick, host)
    } else {
        nick.to_string()
    };

    let message = line.message.as_ref().map(|s| s.as_str()).unwrap_or("");
    let text = strip_irc_formatting(message);

    if text.trim().is_empty() {
        drop_log(verbose, &format!("line dropped (empty message after formatting strip): {}", buffer_name));
        return;
    }

    let is_dm = !target.starts_with('#') && !target.starts_with('&') && !target.starts_with('!');

    // Apply DM/group policy
    if is_dm {
        let dm_policy = channel_host::workspace_read(DM_POLICY_PATH)
            .unwrap_or_else(|| "open".to_string());

        if !check_sender_allowed(nick, &hostmask, &dm_policy) {
            drop_log(verbose, &format!("line held (sender not allowed, triggering pairing): nick={}", nick));
            handle_pairing_request(buffer_name, nick);
            return;
        }
    } else {
        let group_policy = channel_host::workspace_read(GROUP_POLICY_PATH)
            .unwrap_or_else(|| "allowlist".to_string());

        if group_policy == "deny" {
            drop_log(verbose, &format!("line dropped (group policy=deny): {}", buffer_name));
            return;
        }

        let allow_from: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        if group_policy == "allowlist" && !check_sender_allowed(nick, &hostmask, "allowlist") {
            drop_log(verbose, &format!("line dropped (group allowlist): nick={} not in {:?}", nick, allow_from));
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Dropping group message from '{}' in {} (not in allowlist)", nick, buffer_name),
            );
            return;
        }
    }

    // Emit to agent
    let metadata = WeechatMessageMetadata {
        buffer: buffer_name.to_string(),
        network: network.to_string(),
        target: target.clone(),
        nick: nick.to_string(),
        is_dm,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    let user_id = format!("id:{}", hostmask);
    let thread_id = if is_dm {
        format!("weechat:dm:{}:{}", network, nick)
    } else {
        format!("weechat:group:{}:{}", network, target)
    };

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: Some(nick.to_string()),
        content: text,
        thread_id: Some(thread_id),
        metadata_json,
        attachments: vec![],
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!("Emitted message from '{}' in {} ({} chars)", nick, buffer_name, message.len()),
    );
}

/// Check if sender is allowed based on policy.
fn check_sender_allowed(nick: &str, hostmask: &str, policy: &str) -> bool {
    if policy == "open" {
        return true;
    }

    let allow_from: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Also check pairing store
    let pairing_allowed = channel_host::pairing_read_allow_from(CHANNEL_NAME)
        .unwrap_or_default();

    let in_allow_from = allow_from.iter().any(|a| {
        if a == "*" {
            true
        } else if a.contains('!') {
            // Full hostmask entry — match against nick!user@host
            a.eq_ignore_ascii_case(hostmask)
        } else {
            // Nick-only entry
            a.eq_ignore_ascii_case(nick)
        }
    });

    in_allow_from || pairing_allowed.iter().any(|a| a.eq_ignore_ascii_case(nick))
}

/// Handle pairing request for unknown sender.
fn handle_pairing_request(buffer_name: &str, nick: &str) {
    let meta = serde_json::json!({
        "buffer": buffer_name,
        "nick": nick,
    }).to_string();

    match channel_host::pairing_upsert_request(CHANNEL_NAME, nick, &meta) {
        Ok(result) => {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Pairing request for '{}': code {}", nick, result.code),
            );

            if result.created {
                let relay_url = channel_host::workspace_read(RELAY_URL_PATH)
                    .unwrap_or_else(default_relay_url);
                let relay_password = channel_host::workspace_read(RELAY_PASSWORD_PATH)
                    .unwrap_or_default();

                let reply = format!(
                    "To pair with this agent, run: ironclaw pairing approve {} {}",
                    CHANNEL_NAME, result.code
                );

                // Extract network from buffer name (irc.<network>.<nick>)
                let network = buffer_name.split('.').nth(1).unwrap_or("");
                let send_result = if !network.is_empty() {
                    send_dm(&relay_url, &relay_password, buffer_name, network, nick, &reply)
                } else {
                    send_input(&relay_url, &relay_password, buffer_name, &reply)
                };
                if let Err(e) = send_result {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to send pairing reply: {}", e),
                    );
                }
            }
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Pairing upsert failed: {}", e),
            );
        }
    }
}

// ============================================================================
// WeeChat HTTP API Helpers
// ============================================================================

/// Check WeeChat relay health and version.
fn check_relay_health(relay_url: &str, relay_password: &str) -> Result<(String, String), String> {
    let url = format!("{}/api/version", relay_url);
    let response = http_get(&url, relay_password, 3_000)?;

    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }

    let version: VersionResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("parse error: {}", e))?;

    let weechat_version = version.weechat_version.unwrap_or_else(|| "unknown".to_string());
    let api_version = version.relay_api_version.unwrap_or_else(|| "unknown".to_string());

    Ok((weechat_version, api_version))
}

/// Fetch list of all buffers from WeeChat.
fn fetch_buffer_list(relay_url: &str, relay_password: &str) -> Result<Vec<BufferInfo>, String> {
    let url = format!("{}/api/buffers", relay_url);
    let response = http_get(&url, relay_password, 5_000)?;

    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }

    // WeeChat API v2 returns a bare array; parse via serde_json::Value for resilience
    let values: Vec<serde_json::Value> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("parse error: {}", e))?;
    let buffers = values.into_iter().filter_map(|v| {
        let id = v["id"].as_i64();
        let full_name = v["name"].as_str()
            .or_else(|| v["full_name"].as_str())
            .map(String::from);
        let short_name = v["short_name"].as_str().map(String::from);
        Some(BufferInfo { id, full_name, short_name })
    }).collect();
    Ok(buffers)
}

/// Send a message to a DM nick, falling back to core.weechat + /msg -server if the DM buffer is missing.
fn send_dm(
    relay_url: &str,
    relay_password: &str,
    buffer_name: &str,
    network: &str,
    nick: &str,
    text: &str,
) -> Result<(), String> {
    match send_input(relay_url, relay_password, buffer_name, text) {
        Err(ref e) if e.contains("404") || e.contains("not found") || e.contains("Not Found") => {
            // Use the server buffer with /msg <nick> <text> — IRC commands must run
            // in the context of a connected server buffer, not core.weechat.
            let server_buffer = format!("irc.server.{}", network);
            let msg_cmd = format!("/msg {} {}", nick, text);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("DM buffer '{}' not found, routing via '{}'", buffer_name, server_buffer),
            );
            send_input(relay_url, relay_password, &server_buffer, &msg_cmd)
        }
        other => other,
    }
}

/// Send input (message) to a WeeChat buffer.
fn send_input(
    relay_url: &str,
    relay_password: &str,
    buffer_name: &str,
    text: &str,
) -> Result<(), String> {
    let url = format!("{}/api/input", relay_url);

    let payload = serde_json::to_vec(&InputRequest {
        buffer_name: buffer_name.to_string(),
        command: text.to_string(),
    }).map_err(|e| format!("serialize error: {}", e))?;

    let response = http_post(&url, relay_password, &payload, 5_000)?;

    // WeeChat returns 204 No Content on success; also accept 200
    if response.status != 200 && response.status != 204 {
        let body_str = String::from_utf8_lossy(&response.body);
        return Err(format!("HTTP {}: {}", response.status, body_str));
    }

    Ok(())
}

/// Seed watermarks for all IRC buffers to avoid replaying history.
fn seed_watermarks(relay_url: &str, relay_password: &str, buffers: &[BufferInfo]) {
    let mut watermarks = HashMap::new();

    for buffer in buffers {
        if let Some(full_name) = &buffer.full_name {
            let encoded_name = encode_buffer_name(full_name);
            let url = format!("{}/api/buffers/{}/lines?limit=1", relay_url, encoded_name);

            if let Ok(response) = http_get(&url, relay_password, 3_000) {
                if response.status == 200 {
                    // Response is a bare JSON array
                    if let Ok(lines) = serde_json::from_slice::<Vec<serde_json::Value>>(&response.body) {
                        if let Some(line) = lines.first() {
                            let line_id = line["id"].as_i64().unwrap_or(-1);
                            watermarks.insert(full_name.clone(), line_id);
                        }
                    }
                }
            }
        }
    }

    if !watermarks.is_empty() {
        if let Ok(json) = serde_json::to_string(&watermarks) {
            let _ = channel_host::workspace_write(LAST_SEEN_IDS_PATH, &json);
        }

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Seeded ID watermarks for {} buffers", watermarks.len()),
        );
    }
}

/// Perform HTTP GET request.
fn http_get(url: &str, _password: &str, timeout_ms: u32) -> Result<channel_host::HttpResponse, String> {
    let headers_json = serde_json::json!({}).to_string();

    channel_host::http_request(
        "GET",
        url,
        &headers_json,
        None,
        Some(timeout_ms),
    )
}

/// Perform HTTP POST request.
fn http_post(url: &str, _password: &str, body: &[u8], timeout_ms: u32) -> Result<channel_host::HttpResponse, String> {
    let headers_json = serde_json::json!({
        "Content-Type": "application/json"
    }).to_string();

    channel_host::http_request(
        "POST",
        url,
        &headers_json,
        Some(body),
        Some(timeout_ms),
    )
}

// ============================================================================
// Utilities
// ============================================================================

/// Normalize WeeChat relay URL.
fn normalize_relay_url(url: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches("/api")
        .replace("ws://", "http://")
        .replace("wss://", "https://")
}

/// Filter buffer list to IRC buffers only.
fn filter_irc_buffers(buffers: &[BufferInfo]) -> Vec<BufferInfo> {
    buffers.iter()
        .filter(|b| {
            if let Some(name) = &b.full_name {
                let parts: Vec<&str> = name.split('.').collect();
                // Must be irc.<network>.<target>, exclude irc.server.*
                parts.len() >= 3 && parts[0] == "irc" && parts[1] != "server"
            } else {
                false
            }
        })
        .cloned()
        .collect()
}

/// Encode buffer name for URL (escape #).
fn encode_buffer_name(name: &str) -> String {
    name.replace('#', "%23")
}

/// Extract nick from IRC tags.
fn extract_nick_from_tags(tags: &[String]) -> Option<&str> {
    tags.iter()
        .find(|t| t.starts_with("nick_"))
        .map(|t| &t[5..])
}

/// Extract host from IRC tags.
fn extract_host_from_tags(tags: &[String]) -> Option<&str> {
    tags.iter()
        .find(|t| t.starts_with("host_"))
        .map(|t| &t[5..])
}

/// Strip IRC formatting codes.
fn strip_irc_formatting(text: &str) -> String {
    // IRC format codes: bold (\x02), italic (\x1d), underline (\x1f),
    // reverse (\x16), reset (\x0f), color (\x03...)
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\x02' | '\x1d' | '\x1f' | '\x16' | '\x0f' => {
                // Skip formatting char
            }
            '\x03' => {
                // Color code - skip color numbers
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() || next == ',' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            '\x04' => {
                // Hex color - skip 6 or 12 hex digits
                for _ in 0..6 {
                    if let Some(&next) = chars.peek() {
                        if next.is_ascii_hexdigit() {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
            }
            _ => result.push(ch),
        }
    }

    result
}

/// Split message into chunks at word boundaries.
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find char boundary
        let mut end = max_len;
        while end > 0 && !remaining.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            let first_char_len = remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            chunks.push(remaining[..first_char_len].to_string());
            remaining = &remaining[first_char_len..];
            continue;
        }

        // Try to break at newline or space
        let chunk = &remaining[..end];
        let break_at = chunk.rfind('\n')
            .or_else(|| chunk.rfind(' '))
            .unwrap_or(end);

        let break_at = if break_at == 0 { end } else { break_at };

        chunks.push(remaining[..break_at].to_string());
        remaining = remaining[break_at..].trim_start_matches('\n').trim_start();
    }

    chunks
}

/// Parse ISO8601 timestamp to milliseconds.
fn parse_iso8601_to_ms(date_str: &str) -> i64 {
    // Simple parser for WeeChat's ISO8601 format: "2026-03-15T12:34:56Z"
    // For production, use chrono crate, but keeping dependencies minimal for WASM

    if date_str.is_empty() {
        return 0;
    }

    // Basic extraction (not RFC3339 compliant, but works for WeeChat format)
    // This is a simplified version - for production use a proper parser

    // For now, use a simple heuristic: treat as seconds since epoch
    // WeeChat API should provide timestamps in a more parseable format

    // TODO: Implement proper ISO8601 parsing or add chrono dependency
    0
}

/// Simple pseudo-random check (returns true with given probability).
fn rand_check(probability: f64) -> bool {
    // Without std::rand, use a simple heuristic based on current state
    // This is deterministic but varies across calls due to workspace state
    // For production, consider adding a lightweight PRNG

    // Simple approach: hash some changing state and check threshold
    // For now, just return false (disable random features)
    false
}

/// Base64 encode (simple implementation for Basic auth).
fn base64_encode(input: &str) -> String {
    // Simple base64 implementation for WASM (no std::base64)
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let bytes = input.as_bytes();
    let mut result = String::new();

    let mut i = 0;
    while i + 2 < bytes.len() {
        let b1 = bytes[i];
        let b2 = bytes[i + 1];
        let b3 = bytes[i + 2];

        result.push(CHARS[(b1 >> 2) as usize] as char);
        result.push(CHARS[(((b1 & 0x03) << 4) | (b2 >> 4)) as usize] as char);
        result.push(CHARS[(((b2 & 0x0f) << 2) | (b3 >> 6)) as usize] as char);
        result.push(CHARS[(b3 & 0x3f) as usize] as char);

        i += 3;
    }

    // Handle remaining bytes
    match bytes.len() - i {
        1 => {
            let b1 = bytes[i];
            result.push(CHARS[(b1 >> 2) as usize] as char);
            result.push(CHARS[((b1 & 0x03) << 4) as usize] as char);
            result.push('=');
            result.push('=');
        }
        2 => {
            let b1 = bytes[i];
            let b2 = bytes[i + 1];
            result.push(CHARS[(b1 >> 2) as usize] as char);
            result.push(CHARS[(((b1 & 0x03) << 4) | (b2 >> 4)) as usize] as char);
            result.push(CHARS[((b2 & 0x0f) << 2) as usize] as char);
            result.push('=');
        }
        _ => {}
    }

    result
}

/// Create JSON HTTP response.
fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(WeechatRelayChannel);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("hello", 420);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_message_at_space() {
        let text = "hello world this is a test message that is quite long";
        let chunks = split_message(text, 20);
        assert!(chunks[0].len() <= 20);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_normalize_relay_url() {
        assert_eq!(normalize_relay_url("http://127.0.0.1:9001/"), "http://127.0.0.1:9001");
        assert_eq!(normalize_relay_url("http://127.0.0.1:9001/api"), "http://127.0.0.1:9001");
        assert_eq!(normalize_relay_url("ws://localhost:9001"), "http://localhost:9001");
    }

    #[test]
    fn test_encode_buffer_name() {
        assert_eq!(encode_buffer_name("irc.libera.#openclaw"), "irc.libera.%23openclaw");
    }

    #[test]
    fn test_strip_irc_formatting() {
        assert_eq!(strip_irc_formatting("\x02bold\x02 normal"), "bold normal");
        assert_eq!(strip_irc_formatting("\x0312blue\x03 normal"), "blue normal");
    }

    #[test]
    fn test_filter_irc_buffers() {
        let buffers = vec![
            BufferInfo {
                id: Some(1),
                full_name: Some("irc.libera.#openclaw".to_string()),
                short_name: None,
                name: None,
            },
            BufferInfo {
                id: Some(2),
                full_name: Some("irc.server.libera".to_string()),
                short_name: None,
                name: None,
            },
            BufferInfo {
                id: Some(3),
                full_name: Some("core.weechat".to_string()),
                short_name: None,
                name: None,
            },
        ];

        let filtered = filter_irc_buffers(&buffers);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].full_name.as_ref().unwrap(), "irc.libera.#openclaw");
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode("hello"), "aGVsbG8=");
        assert_eq!(base64_encode("plain:password"), "cGxhaW46cGFzc3dvcmQ=");
    }
}
