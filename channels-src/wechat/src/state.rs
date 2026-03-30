use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::auth::{
    CONFIG_PATH, CONTEXT_TOKENS_PATH, GET_UPDATES_BUF_PATH, PENDING_INBOUND_PATH,
    PROCESSED_MESSAGE_IDS_PATH, TYPING_TICKETS_PATH,
};
use crate::near::agent::channel_host;
use crate::types::WechatConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TypingTicketEntry {
    pub ticket: String,
    pub fetched_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct StoredInboundAttachment {
    pub id: String,
    pub mime_type: String,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
    pub source_url: Option<String>,
    pub storage_key: Option<String>,
    pub extracted_text: Option<String>,
    pub extras_json: String,
}

impl From<channel_host::InboundAttachment> for StoredInboundAttachment {
    fn from(value: channel_host::InboundAttachment) -> Self {
        Self {
            id: value.id,
            mime_type: value.mime_type,
            filename: value.filename,
            size_bytes: value.size_bytes,
            source_url: value.source_url,
            storage_key: value.storage_key,
            extracted_text: value.extracted_text,
            extras_json: value.extras_json,
        }
    }
}

impl From<StoredInboundAttachment> for channel_host::InboundAttachment {
    fn from(value: StoredInboundAttachment) -> Self {
        Self {
            id: value.id,
            mime_type: value.mime_type,
            filename: value.filename,
            size_bytes: value.size_bytes,
            source_url: value.source_url,
            storage_key: value.storage_key,
            extracted_text: value.extracted_text,
            extras_json: value.extras_json,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PendingInboundBundle {
    pub from_user_id: String,
    pub to_user_id: Option<String>,
    pub session_id: Option<String>,
    pub context_token: Option<String>,
    pub message_id: Option<i64>,
    pub flush_at_ms: u64,
    pub text: String,
    pub attachments: Vec<StoredInboundAttachment>,
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

pub fn load_pending_inbound_bundles() -> Result<HashMap<String, PendingInboundBundle>, String> {
    parse_pending_inbound_bundles(channel_host::workspace_read(PENDING_INBOUND_PATH).as_deref())
}

pub fn persist_pending_inbound_bundles(
    bundles: &HashMap<String, PendingInboundBundle>,
) -> Result<(), String> {
    let serialized =
        serde_json::to_string(bundles).map_err(|e| format!("Failed to serialize bundles: {e}"))?;
    channel_host::workspace_write(PENDING_INBOUND_PATH, &serialized).map_err(|e| e.to_string())
}

fn parse_pending_inbound_bundles(
    raw: Option<&str>,
) -> Result<HashMap<String, PendingInboundBundle>, String> {
    match raw {
        None => Ok(HashMap::new()),
        Some(raw) => serde_json::from_str(raw)
            .map_err(|e| format!("Failed to parse pending inbound bundles: {e}")),
    }
}

pub fn load_processed_message_ids() -> Result<Vec<i64>, String> {
    parse_processed_message_ids(channel_host::workspace_read(PROCESSED_MESSAGE_IDS_PATH).as_deref())
}

pub fn persist_processed_message_ids(message_ids: &[i64]) -> Result<(), String> {
    let serialized = serde_json::to_string(message_ids)
        .map_err(|e| format!("Failed to serialize processed message ids: {e}"))?;
    channel_host::workspace_write(PROCESSED_MESSAGE_IDS_PATH, &serialized).map_err(|e| e.to_string())
}

pub fn has_processed_message_id(processed_message_ids: &[i64], message_id: i64) -> bool {
    processed_message_ids.contains(&message_id)
}

pub fn remember_processed_message_id(
    processed_message_ids: &mut Vec<i64>,
    message_id: i64,
    max_entries: usize,
) -> bool {
    if processed_message_ids.contains(&message_id) {
        return false;
    }
    processed_message_ids.push(message_id);
    if processed_message_ids.len() > max_entries {
        let excess = processed_message_ids.len() - max_entries;
        processed_message_ids.drain(0..excess);
    }
    true
}

fn parse_processed_message_ids(raw: Option<&str>) -> Result<Vec<i64>, String> {
    match raw {
        None => Ok(Vec::new()),
        Some(raw) => serde_json::from_str(raw)
            .map_err(|e| format!("Failed to parse processed message ids: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pending_inbound_bundles_missing_file_returns_empty_map() {
        let bundles = parse_pending_inbound_bundles(None).expect("missing state should be empty");
        assert!(bundles.is_empty());
    }

    #[test]
    fn test_parse_pending_inbound_bundles_invalid_json_returns_error() {
        let error =
            parse_pending_inbound_bundles(Some("{not json")).expect_err("invalid json should err");
        assert!(error.contains("Failed to parse pending inbound bundles"));
    }

    #[test]
    fn test_parse_processed_message_ids_missing_file_returns_empty_vec() {
        let ids = parse_processed_message_ids(None).expect("missing state should be empty");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_remember_processed_message_id_dedups_and_trims() {
        let mut ids = vec![1, 2];
        assert!(!remember_processed_message_id(&mut ids, 2, 3));
        assert_eq!(ids, vec![1, 2]);

        assert!(remember_processed_message_id(&mut ids, 3, 2));
        assert_eq!(ids, vec![2, 3]);
    }
}
