//! XMPP bridge-backed channel for IronClaw.
//!
//! This WASM channel talks to a local `xmpp-bridge` process over loopback HTTP.
//! The bridge owns the long-lived XMPP session; the WASM channel handles the
//! standard IronClaw extension lifecycle and message normalization.

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, IncomingHttpRequest, OutgoingHttpResponse, PollConfig,
    StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

const CONFIG_PATH: &str = "config.json";
const CURSOR_PATH: &str = "cursor.txt";
const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:8787";
const DEFAULT_POLL_INTERVAL_MS: u32 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeConfig {
    display_name: String,
    bridge_url: String,
    xmpp_jid: String,
    xmpp_password: String,
    dm_policy: String,
    allow_from: Vec<String>,
    rooms: Vec<String>,
    encrypted_rooms: Vec<String>,
    allow_plaintext_fallback: bool,
    max_messages_per_hour: u32,
    resource: Option<String>,
    device_id: u32,
    omemo_store_dir: Option<String>,
    polling_enabled: bool,
    poll_interval_ms: u32,
}

#[derive(Debug, Serialize)]
struct ConfigureRequest {
    jid: String,
    password: String,
    dm_policy: String,
    allow_from: Vec<String>,
    allow_rooms: Vec<String>,
    encrypted_rooms: Vec<String>,
    device_id: u32,
    omemo_store_dir: Option<String>,
    allow_plaintext_fallback: bool,
    max_messages_per_hour: u32,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PollResponse {
    cursor: u64,
    #[serde(default)]
    messages: Vec<BridgeIncomingMessage>,
}

#[derive(Debug, Deserialize)]
struct BridgeIncomingMessage {
    user_id: String,
    user_name: Option<String>,
    content: String,
    thread_id: Option<String>,
    metadata_json: String,
}

#[derive(Debug, Serialize)]
struct SendRequest {
    target: String,
    content: String,
    metadata_json: String,
}

struct XmppChannel;

impl Guest for XmppChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::workspace_write(CONFIG_PATH, &config_json)
            .map_err(|e| format!("failed to persist channel config: {e}"))?;

        let parsed = parse_runtime_config(&config_json);
        let config = match parsed {
            Ok(config) => {
                if let Err(err) = ensure_bridge_configured(&config) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("XMPP bridge configure failed during on_start: {}", err),
                    );
                }
                config
            }
            Err(err) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("XMPP channel is not fully configured yet: {}", err),
                );
                return Ok(ChannelConfig {
                    display_name: "XMPP".to_string(),
                    http_endpoints: Vec::new(),
                    poll: Some(PollConfig {
                        interval_ms: DEFAULT_POLL_INTERVAL_MS,
                        enabled: false,
                    }),
                });
            }
        };

        Ok(ChannelConfig {
            display_name: config.display_name.clone(),
            http_endpoints: Vec::new(),
            poll: Some(PollConfig {
                interval_ms: config.poll_interval_ms,
                enabled: config.polling_enabled,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        json_response(
            404,
            serde_json::json!({"error": "xmpp channel does not expose webhooks"}),
        )
    }

    fn on_poll() {
        let Ok(config) = load_runtime_config() else {
            return;
        };
        if !config.polling_enabled {
            return;
        }
        if let Err(err) = ensure_bridge_configured(&config) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("XMPP bridge configure failed during poll: {}", err),
            );
            return;
        }

        let cursor = read_cursor();
        let url = format!(
            "{}/v1/messages?cursor={}",
            trim_base_url(&config.bridge_url),
            cursor
        );
        let response: PollResponse = match request_json("GET", &url, None) {
            Ok(value) => match serde_json::from_slice(&value) {
                Ok(value) => value,
                Err(err) => {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to parse XMPP bridge poll response: {}", err),
                    );
                    return;
                }
            },
            Err(err) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("XMPP bridge poll failed: {}", err),
                );
                return;
            }
        };

        for message in &response.messages {
            channel_host::emit_message(&EmittedMessage {
                user_id: message.user_id.clone(),
                user_name: message.user_name.clone(),
                content: message.content.clone(),
                thread_id: message.thread_id.clone(),
                metadata_json: normalize_metadata_json(&message.metadata_json),
                attachments: Vec::new(),
            });
        }

        if let Err(err) = write_cursor(response.cursor) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to persist XMPP bridge cursor: {}", err),
            );
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let config = load_runtime_config()?;
        send_message_via_bridge(&config, response.metadata_json, response.content)
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let config = load_runtime_config()?;
        let metadata_json = if response.metadata_json.trim().is_empty() {
            serde_json::json!({ "xmpp_target": user_id }).to_string()
        } else {
            response.metadata_json
        };
        send_message_via_bridge(&config, metadata_json, response.content)
    }

    fn on_status(update: StatusUpdate) {
        if metadata_is_groupchat(&update.metadata_json) {
            return;
        }

        let Some(message) = render_status_message(&update) else {
            return;
        };

        let Ok(config) = load_runtime_config() else {
            return;
        };
        if let Err(err) = send_message_via_bridge(&config, update.metadata_json.clone(), message) {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Failed to send XMPP status update: {}", err),
            );
        }
    }

    fn on_shutdown() {}
}

fn load_runtime_config() -> Result<RuntimeConfig, String> {
    let config_json = channel_host::workspace_read(CONFIG_PATH)
        .ok_or_else(|| "xmpp runtime config is unavailable".to_string())?;
    parse_runtime_config(&config_json)
}

fn parse_runtime_config(config_json: &str) -> Result<RuntimeConfig, String> {
    let value: serde_json::Value = serde_json::from_str(config_json)
        .map_err(|e| format!("invalid XMPP channel config JSON: {}", e))?;

    let xmpp_jid = read_required_string(&value, "xmpp_jid")
        .ok_or_else(|| "xmpp_jid is required".to_string())?;
    let xmpp_password = read_required_string(&value, "xmpp_password")
        .ok_or_else(|| "xmpp_password is required".to_string())?;

    Ok(RuntimeConfig {
        display_name: read_required_string(&value, "display_name")
            .unwrap_or_else(|| "XMPP".to_string()),
        bridge_url: read_required_string(&value, "bridge_url")
            .unwrap_or_else(|| DEFAULT_BRIDGE_URL.to_string()),
        xmpp_jid,
        xmpp_password,
        dm_policy: read_required_string(&value, "dm_policy")
            .unwrap_or_else(|| "allowlist".to_string()),
        allow_from: read_string_list(&value, "allow_from"),
        rooms: read_string_list(&value, "rooms"),
        encrypted_rooms: read_string_list(&value, "encrypted_rooms"),
        allow_plaintext_fallback: read_bool(&value, "allow_plaintext_fallback").unwrap_or(true),
        max_messages_per_hour: read_u32(&value, "max_messages_per_hour").unwrap_or(0),
        resource: read_optional_string(&value, "resource"),
        device_id: read_u32(&value, "device_id").unwrap_or(0),
        omemo_store_dir: read_optional_string(&value, "omemo_store_dir"),
        polling_enabled: read_bool(&value, "polling_enabled").unwrap_or(true),
        poll_interval_ms: read_u32(&value, "poll_interval_ms")
            .unwrap_or(DEFAULT_POLL_INTERVAL_MS)
            .max(DEFAULT_POLL_INTERVAL_MS),
    })
}

fn ensure_bridge_configured(config: &RuntimeConfig) -> Result<(), String> {
    let url = format!("{}/v1/configure", trim_base_url(&config.bridge_url));
    let request = ConfigureRequest {
        jid: config.xmpp_jid.clone(),
        password: config.xmpp_password.clone(),
        dm_policy: config.dm_policy.clone(),
        allow_from: config.allow_from.clone(),
        allow_rooms: config.rooms.clone(),
        encrypted_rooms: config.encrypted_rooms.clone(),
        device_id: config.device_id,
        omemo_store_dir: config.omemo_store_dir.clone(),
        allow_plaintext_fallback: config.allow_plaintext_fallback,
        max_messages_per_hour: config.max_messages_per_hour,
        resource: config.resource.clone(),
    };

    let payload = serde_json::to_vec(&request)
        .map_err(|e| format!("failed to serialize XMPP bridge configure request: {}", e))?;
    request_json("POST", &url, Some(payload)).map(|_| ())
}

fn send_message_via_bridge(
    config: &RuntimeConfig,
    metadata_json: String,
    content: String,
) -> Result<(), String> {
    ensure_bridge_configured(config)?;
    let target = target_from_metadata_json(&metadata_json)
        .ok_or_else(|| "missing xmpp_target in response metadata".to_string())?;
    let url = format!("{}/v1/messages/send", trim_base_url(&config.bridge_url));
    let request = SendRequest {
        target,
        content,
        metadata_json: normalize_metadata_json(&metadata_json),
    };
    let payload = serde_json::to_vec(&request)
        .map_err(|e| format!("failed to serialize XMPP bridge send request: {}", e))?;
    request_json("POST", &url, Some(payload)).map(|_| ())
}

fn request_json(method: &str, url: &str, body: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let headers = serde_json::json!({ "Content-Type": "application/json" }).to_string();
    let response = channel_host::http_request(method, url, &headers, body.as_deref(), None)
        .map_err(|e| format!("bridge request failed: {}", e))?;

    if response.status / 100 != 2 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "bridge request returned HTTP {}: {}",
            response.status, body_text
        ));
    }

    Ok(response.body)
}

fn read_cursor() -> u64 {
    channel_host::workspace_read(CURSOR_PATH)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn write_cursor(cursor: u64) -> Result<(), String> {
    channel_host::workspace_write(CURSOR_PATH, &cursor.to_string())
}

fn target_from_metadata_json(metadata_json: &str) -> Option<String> {
    let metadata: serde_json::Value = serde_json::from_str(metadata_json).ok()?;
    metadata
        .get("xmpp_target")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            metadata
                .get("xmpp_room")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        })
}

fn metadata_is_groupchat(metadata_json: &str) -> bool {
    let Ok(metadata) = serde_json::from_str::<serde_json::Value>(metadata_json) else {
        return false;
    };
    metadata
        .get("xmpp_type")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == "groupchat")
}

fn render_status_message(update: &StatusUpdate) -> Option<String> {
    match update.status {
        StatusType::ApprovalNeeded
        | StatusType::AuthRequired
        | StatusType::AuthCompleted
        | StatusType::JobStarted => Some(update.message.clone()),
        StatusType::Status => {
            let trimmed = update.message.trim();
            if trimmed.eq_ignore_ascii_case("done")
                || trimmed.eq_ignore_ascii_case("awaiting approval")
                || trimmed.eq_ignore_ascii_case("rejected")
            {
                None
            } else {
                Some(update.message.clone())
            }
        }
        _ => None,
    }
}

fn normalize_metadata_json(metadata_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(metadata_json)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "{}".to_string())
}

fn trim_base_url(base: &str) -> String {
    base.trim_end_matches('/').to_string()
}

fn read_required_string(value: &serde_json::Value, key: &str) -> Option<String> {
    read_optional_string(value, key).filter(|value| !value.trim().is_empty())
}

fn read_optional_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    })
}

fn read_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|value| match value {
        serde_json::Value::Bool(boolean) => Some(*boolean),
        serde_json::Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn read_u32(value: &serde_json::Value, key: &str) -> Option<u32> {
    value.get(key).and_then(|value| match value {
        serde_json::Value::Number(number) => {
            number.as_u64().and_then(|value| u32::try_from(value).ok())
        }
        serde_json::Value::String(text) => text.trim().parse::<u32>().ok(),
        _ => None,
    })
}

fn read_string_list(value: &serde_json::Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(serde_json::Value::String(text)) => text
            .split(',')
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect(),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str().map(|value| value.trim().to_string()))
            .filter(|value| !value.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({ "Content-Type": "application/json" });
    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

export!(XmppChannel);
