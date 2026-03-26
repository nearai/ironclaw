use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::auth::{
    CONFIG_PATH, CONTEXT_TOKENS_PATH, GET_UPDATES_BUF_PATH, SESSION_EXPIRED_PATH,
    TYPING_TICKETS_PATH,
};
use crate::near::agent::channel_host;
use crate::types::WechatConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TypingTicketEntry {
    pub ticket: String,
    pub fetched_at_ms: u64,
}

pub fn load_config() -> WechatConfig {
    channel_host::workspace_read(CONFIG_PATH)
        .and_then(|raw| serde_json::from_str::<WechatConfig>(&raw).ok())
        .unwrap_or_default()
}

pub fn persist_config(config: &WechatConfig) -> Result<(), String> {
    let serialized =
        serde_json::to_string(config).map_err(|e| format!("Failed to serialize config: {e}"))?;
    channel_host::workspace_write(CONFIG_PATH, &serialized).map_err(|e| e.to_string())
}

pub fn load_get_updates_buf() -> String {
    channel_host::workspace_read(GET_UPDATES_BUF_PATH)
        .and_then(|raw| serde_json::from_str::<String>(&raw).ok())
        .unwrap_or_default()
}

pub fn persist_get_updates_buf(value: &str) -> Result<(), String> {
    let serialized =
        serde_json::to_string(value).map_err(|e| format!("Failed to serialize cursor: {e}"))?;
    channel_host::workspace_write(GET_UPDATES_BUF_PATH, &serialized).map_err(|e| e.to_string())
}

pub fn load_context_tokens() -> HashMap<String, String> {
    channel_host::workspace_read(CONTEXT_TOKENS_PATH)
        .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(&raw).ok())
        .unwrap_or_default()
}

pub fn persist_context_tokens(tokens: &HashMap<String, String>) -> Result<(), String> {
    let serialized =
        serde_json::to_string(tokens).map_err(|e| format!("Failed to serialize tokens: {e}"))?;
    channel_host::workspace_write(CONTEXT_TOKENS_PATH, &serialized).map_err(|e| e.to_string())
}

pub fn load_typing_tickets() -> HashMap<String, TypingTicketEntry> {
    channel_host::workspace_read(TYPING_TICKETS_PATH)
        .and_then(|raw| serde_json::from_str::<HashMap<String, TypingTicketEntry>>(&raw).ok())
        .unwrap_or_default()
}

pub fn persist_typing_tickets(tickets: &HashMap<String, TypingTicketEntry>) -> Result<(), String> {
    let serialized =
        serde_json::to_string(tickets).map_err(|e| format!("Failed to serialize tickets: {e}"))?;
    channel_host::workspace_write(TYPING_TICKETS_PATH, &serialized).map_err(|e| e.to_string())
}

pub fn session_expired() -> bool {
    matches!(
        channel_host::workspace_read(SESSION_EXPIRED_PATH).as_deref(),
        Some("1")
    )
}

pub fn clear_session_expired() {
    let _ = channel_host::workspace_write(SESSION_EXPIRED_PATH, "0");
}

pub fn mark_session_expired() {
    let _ = channel_host::workspace_write(SESSION_EXPIRED_PATH, "1");
}
