//! Signal channel via signal-cli daemon HTTP/JSON-RPC.
//!
//! Connects to a running `signal-cli daemon --http <host:port>`.
//! Listens for messages via SSE at `/api/v1/events` and sends via
//! JSON-RPC at `/api/v1/rpc`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use lru::LruCache;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::SignalConfig;
use crate::error::ChannelError;
use crate::pairing::PairingStore;

const GROUP_TARGET_PREFIX: &str = "group:";
const SIGNAL_HEALTH_ENDPOINT: &str = "/api/v1/check";

const MAX_SSE_BUFFER_SIZE: usize = 1024 * 1024;
const MAX_SSE_EVENT_SIZE: usize = 256 * 1024;
const MAX_HTTP_RESPONSE_SIZE: usize = 10 * 1024 * 1024;
const MAX_REPLY_TARGETS: usize = 10000;
const MAX_ERROR_LOG_BODY: usize = 1024;

const REPLY_TARGETS_CAP: NonZeroUsize = NonZeroUsize::new(MAX_REPLY_TARGETS).unwrap();

/// Recipient classification for outbound messages.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecipientTarget {
    Direct(String),
    Group(String),
}

// ── signal-cli SSE event JSON shapes ────────────────────────────

#[derive(Debug, Deserialize)]
struct SseEnvelope {
    #[serde(default)]
    envelope: Option<Envelope>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    source: Option<String>,
    #[serde(rename = "sourceNumber", default)]
    source_number: Option<String>,
    #[serde(rename = "sourceName", default)]
    source_name: Option<String>,
    #[serde(rename = "sourceUuid", default)]
    source_uuid: Option<String>,
    #[serde(rename = "dataMessage", default)]
    data_message: Option<DataMessage>,
    #[serde(rename = "storyMessage", default)]
    story_message: Option<serde_json::Value>,
    #[serde(default)]
    timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DataMessage {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    timestamp: Option<u64>,
    #[serde(rename = "groupInfo", default)]
    group_info: Option<GroupInfo>,
    #[serde(default)]
    attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct GroupInfo {
    #[serde(rename = "groupId", default)]
    group_id: Option<String>,
}

/// Signal channel using signal-cli daemon's native JSON-RPC + SSE API.
pub struct SignalChannel {
    config: SignalConfig,
    client: Client,
    /// LRU cache of reply targets per incoming message, used by `respond()`.
    /// Bounded to `MAX_REPLY_TARGETS` entries; least-recently-used entries
    /// are evicted automatically when the cache is full.
    reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
}

impl SignalChannel {
    /// Create a new Signal channel with normalized config and fresh client/cache.
    pub fn new(config: SignalConfig) -> Result<Self, ChannelError> {
        let mut config = config;
        config.http_url = config.http_url.trim_end_matches('/').to_string();

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        let cap = REPLY_TARGETS_CAP;
        let reply_targets = Arc::new(RwLock::new(LruCache::new(cap)));

        Ok(Self::from_parts(config, client, reply_targets))
    }

    /// Construct a SignalChannel from pre-validated parts.
    ///
    /// Used by [`new()`][Self::new] after normalization and by [`sse_listener`]
    /// to ensure both code paths use the same constructor.
    fn from_parts(
        config: SignalConfig,
        client: Client,
        reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
    ) -> Self {
        Self {
            config,
            client,
            reply_targets,
        }
    }

    /// Effective sender: prefer `sourceNumber` (E.164), fall back to `source`
    /// (UUID for privacy-enabled users).
    fn sender(envelope: &Envelope) -> Option<String> {
        envelope
            .source_number
            .as_deref()
            .or(envelope.source.as_deref())
            .map(String::from)
    }

    /// Normalize an allowlist entry to the bare identifier.
    ///
    /// Strips the `uuid:` prefix if present, so `uuid:<id>` and `<id>` both
    /// match against a bare UUID sender.
    fn normalize_allow_entry(entry: &str) -> &str {
        entry.strip_prefix("uuid:").unwrap_or(entry)
    }

    /// Check whether a sender is in the allowed users list.
    fn is_sender_allowed(&self, sender: &str) -> bool {
        if self.config.allow_from.is_empty() {
            return false;
        }
        self.config.allow_from.iter().any(|entry| {
            entry == "*"
                || Self::normalize_allow_entry(entry) == Self::normalize_allow_entry(sender)
        })
    }

    /// Check if sender is allowed via config allow_from OR pairing store.
    fn is_sender_allowed_with_pairing(&self, sender: &str) -> bool {
        if self.is_sender_allowed(sender) {
            return true;
        }
        let store = PairingStore::new();
        if let Ok(allowed) = store.read_allow_from("signal") {
            return allowed.iter().any(|entry| entry == "*" || entry == sender);
        }
        false
    }

    /// Handle pairing request for unapproved sender.
    /// Returns Ok(true) if message should be allowed (was already paired),
    /// Ok(false) if message was blocked but pairing request was processed.
    fn handle_pairing_request(&self, sender: &str, source_name: Option<&str>) -> Result<bool, ()> {
        let store = PairingStore::new();
        let meta = serde_json::json!({
            "sender": sender,
            "name": source_name,
        });

        match store.upsert_request("signal", sender, Some(meta)) {
            Ok(result) => {
                tracing::info!(
                    sender = %sender,
                    code = %result.code,
                    "Signal: pairing request upserted"
                );
                if result.created {
                    let message = format!(
                        "To pair with this bot, run: `ironclaw pairing approve signal {}`",
                        result.code
                    );
                    let http_url = self.config.http_url.clone();
                    let account = self.config.account.clone();
                    let sender_owned = sender.to_string();
                    let message_owned = message.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::send_pairing_reply_async(
                            &http_url,
                            &account,
                            &sender_owned,
                            &message_owned,
                        )
                        .await
                        {
                            tracing::error!(sender = %sender_owned, error = %e, "Signal: failed to send pairing reply");
                        }
                    });
                }
                Ok(false)
            }
            Err(e) => {
                tracing::error!(sender = %sender, error = %e, "Signal: pairing upsert failed");
                Err(())
            }
        }
    }

    /// Send a pairing reply message to the sender (async helper for spawned task).
    async fn send_pairing_reply_async(
        http_url: &str,
        account: &str,
        recipient: &str,
        message: &str,
    ) -> Result<(), ChannelError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        let target = Self::parse_recipient_target(recipient);
        let params = Self::build_rpc_params_static(http_url, account, &target, Some(message));

        let url = format!("{}/api/v1/rpc", http_url);
        let id = Uuid::new_v4().to_string();

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "send",
            "params": params,
            "id": id,
        });

        let resp = client
            .post(&url)
            .timeout(Duration::from_secs(30))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("RPC request failed to {}: {e}", Self::redact_url(&url)),
            })?;

        let status = resp.status();
        let is_success = status.is_success();

        if status.as_u16() == 201 {
            return Ok(());
        }

        if !is_success {
            let bytes = resp.bytes().await.unwrap_or_default();
            let truncated_len = bytes.len().min(MAX_ERROR_LOG_BODY);
            let truncated_body = String::from_utf8_lossy(&bytes[..truncated_len]);
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("HTTP error {}: {}", status.as_u16(), truncated_body),
            });
        }

        Ok(())
    }

    /// Get effective group allow_from list (inherits from allow_from if empty).
    fn effective_group_allow_from(&self) -> &[String] {
        if self.config.group_allow_from.is_empty() {
            &self.config.allow_from
        } else {
            &self.config.group_allow_from
        }
    }

    /// Check whether a group is in the allowed groups list.
    ///
    /// - Empty list — deny all groups (DMs only, secure by default).
    /// - `*` — allow all groups.
    /// - Specific IDs — allow only those groups.
    fn is_group_allowed(&self, group_id: &str) -> bool {
        if self.config.allow_from_groups.is_empty() {
            return false;
        }
        self.config
            .allow_from_groups
            .iter()
            .any(|entry| entry == "*" || entry == group_id)
    }

    /// Check whether a sender is allowed for group messages.
    fn is_group_sender_allowed(&self, sender: &str) -> bool {
        let effective_list = self.effective_group_allow_from();
        if effective_list.is_empty() {
            return false;
        }
        effective_list.iter().any(|entry| {
            entry == "*"
                || Self::normalize_allow_entry(entry) == Self::normalize_allow_entry(sender)
        })
    }

    /// Redact credentials from a URL for safe logging.
    ///
    /// Replaces any embedded username/password with `**REDACTED**` and returns
    /// the sanitised string. Returns `"<invalid-url>"` when parsing fails.
    pub fn redact_url(url: &str) -> String {
        reqwest::Url::parse(url)
            .map(|mut u| {
                if u.password().is_some() || !u.username().is_empty() {
                    let _ = u.set_username("**REDACTED**");
                    let _ = u.set_password(None);
                }
                u.to_string()
            })
            .unwrap_or_else(|_| "<invalid-url>".to_string())
    }

    fn is_e164(recipient: &str) -> bool {
        let Some(number) = recipient.strip_prefix('+') else {
            return false;
        };
        (7..=15).contains(&number.len()) && number.chars().all(|c| c.is_ascii_digit())
    }

    /// Check whether a string is a valid UUID (signal-cli uses these for
    /// privacy-enabled users who have opted out of sharing their phone number).
    fn is_uuid(s: &str) -> bool {
        Uuid::parse_str(s).is_ok()
    }

    /// Generate a deterministic UUID from an identifier (phone number or group ID).
    ///
    /// This ensures that the same phone number or group always produces the same UUID,
    /// allowing conversation history to persist across gateway restarts.
    fn thread_id_from_identifier(identifier: &str) -> String {
        // Use a stable, deterministic UUID v5 derived from the identifier.
        // This avoids relying on `DefaultHasher` implementation details and
        // provides a full 128 bits of entropy.
        Uuid::new_v5(&Uuid::NAMESPACE_URL, identifier.as_bytes()).to_string()
    }

    fn parse_recipient_target(recipient: &str) -> RecipientTarget {
        if let Some(group_id) = recipient.strip_prefix(GROUP_TARGET_PREFIX) {
            return RecipientTarget::Group(group_id.to_string());
        }

        if Self::is_e164(recipient) || Self::is_uuid(recipient) {
            RecipientTarget::Direct(recipient.to_string())
        } else {
            RecipientTarget::Group(recipient.to_string())
        }
    }

    /// Determine the reply target: group id (prefixed) or the sender's identifier.
    fn reply_target(data_msg: &DataMessage, sender: &str) -> String {
        if let Some(group_id) = data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref())
        {
            format!("{GROUP_TARGET_PREFIX}{group_id}")
        } else {
            sender.to_string()
        }
    }

    /// Send a JSON-RPC request to signal-cli daemon.
    async fn rpc_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, ChannelError> {
        let url = format!("{}/api/v1/rpc", self.config.http_url);
        let id = Uuid::new_v4().to_string();

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(30))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("RPC request failed to {}: {e}", Self::redact_url(&url)),
            })?;

        // 201 = success with no body (e.g. typing indicators).
        if resp.status().as_u16() == 201 {
            return Ok(None);
        }

        // Reject obviously oversized responses before buffering.
        if let Some(len) = resp.content_length()
            && len as usize > MAX_HTTP_RESPONSE_SIZE
        {
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!(
                    "RPC response Content-Length too large: {} bytes (max {})",
                    len, MAX_HTTP_RESPONSE_SIZE
                ),
            });
        }

        let status = resp.status();
        let mut stream = resp.bytes_stream();
        let mut total_bytes = 0usize;
        let mut body = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Failed to read RPC response: {e}"),
            })?;
            let chunk_len = chunk.len();
            total_bytes += chunk_len;

            if total_bytes > MAX_HTTP_RESPONSE_SIZE {
                return Err(ChannelError::SendFailed {
                    name: "signal".to_string(),
                    reason: format!(
                        "RPC response too large: {} bytes (max {})",
                        total_bytes, MAX_HTTP_RESPONSE_SIZE
                    ),
                });
            }

            body.extend_from_slice(&chunk);
        }

        let bytes = body;

        if bytes.is_empty() {
            return Ok(None);
        }

        // Check for non-success HTTP status codes before parsing as JSON.
        if !status.is_success() {
            let truncated_len = std::cmp::min(bytes.len(), 512);
            let truncated_body = String::from_utf8_lossy(&bytes[..truncated_len]);
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("HTTP error {}: {}", status.as_u16(), truncated_body),
            });
        }

        let parsed: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Invalid RPC response JSON: {e}"),
            })?;

        if let Some(err) = parsed.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Signal RPC error {code}: {msg}"),
            });
        }

        Ok(parsed.get("result").cloned())
    }

    /// Build JSON-RPC params for a send/typing call.
    fn build_rpc_params(
        &self,
        target: &RecipientTarget,
        message: Option<&str>,
    ) -> serde_json::Value {
        match target {
            RecipientTarget::Direct(id) => {
                let mut params = serde_json::json!({
                    "recipient": [id],
                    "account": &self.config.account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
            RecipientTarget::Group(group_id) => {
                let mut params = serde_json::json!({
                    "groupId": group_id,
                    "account": &self.config.account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
        }
    }

    /// Build JSON-RPC params for a send/typing call (static version).
    fn build_rpc_params_static(
        _http_url: &str,
        account: &str,
        target: &RecipientTarget,
        message: Option<&str>,
    ) -> serde_json::Value {
        match target {
            RecipientTarget::Direct(id) => {
                let mut params = serde_json::json!({
                    "recipient": [id],
                    "account": account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
            RecipientTarget::Group(group_id) => {
                let mut params = serde_json::json!({
                    "groupId": group_id,
                    "account": account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
        }
    }

    /// Process a single SSE envelope, returning an `IncomingMessage` if valid.
    fn process_envelope(&self, envelope: &Envelope) -> Option<(IncomingMessage, String)> {
        // Skip story messages when configured.
        if self.config.ignore_stories && envelope.story_message.is_some() {
            return None;
        }

        let data_msg = envelope.data_message.as_ref()?;

        // Skip attachment-only messages when configured.
        let has_attachments = data_msg.attachments.as_ref().is_some_and(|a| !a.is_empty());
        let has_message_text = data_msg.message.as_ref().is_some_and(|m| !m.is_empty());
        if self.config.ignore_attachments && has_attachments && !has_message_text {
            return None;
        }

        // Use message text, or fall back to "[Attachment]" for attachment-only messages
        // when ignore_attachments is false. This ensures attachment-only messages are
        // still processed when the user wants them (rather than always being dropped).
        let text = data_msg
            .message
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(String::from)
            .or_else(|| {
                if has_attachments {
                    Some("[Attachment]".to_string())
                } else {
                    None
                }
            })?;
        let sender = Self::sender(envelope)?;

        // Log sender info including UUID if available
        tracing::debug!(
            sender = %sender,
            uuid = ?envelope.source_uuid,
            "Signal: received message"
        );

        // Check if this is a group message
        let is_group = data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref())
            .is_some();

        // Apply group policy first (before DM policy for group messages)
        if is_group {
            match self.config.group_policy.as_str() {
                "disabled" => {
                    tracing::debug!("Signal: group messages disabled, dropping");
                    return None;
                }
                "open" => {
                    // For "open" policy, check group allowlist but not sender allowlist
                    if let Some(group_id) = data_msg
                        .group_info
                        .as_ref()
                        .and_then(|g| g.group_id.as_deref())
                        && !self.is_group_allowed(group_id)
                    {
                        tracing::debug!(
                            group_id = %group_id,
                            "Signal: group not in allow_from_groups, dropping"
                        );
                        return None;
                    }
                }
                "allowlist" => {
                    // Default to allowlist - check group AND sender
                    if let Some(group_id) = data_msg
                        .group_info
                        .as_ref()
                        .and_then(|g| g.group_id.as_deref())
                    {
                        if !self.is_group_allowed(group_id) {
                            tracing::debug!(
                                group_id = %group_id,
                                "Signal: group not in allow_from_groups, dropping"
                            );
                            return None;
                        }
                        // Also check sender is allowed for group
                        if !self.is_group_sender_allowed(&sender) {
                            tracing::debug!(
                                sender = %sender,
                                group_id = %group_id,
                                "Signal: sender not in group_allow_from, dropping"
                            );
                            return None;
                        }
                    }
                }
                _ => {}
            }
        } else {
            // DM message - apply DM policy
            match self.config.dm_policy.as_str() {
                "open" => {}
                "pairing" => {
                    // Pairing policy: check allow_from + pairing store
                    if !self.is_sender_allowed_with_pairing(&sender) {
                        // Handle pairing request - this will create a request and send reply if new
                        match self.handle_pairing_request(&sender, envelope.source_name.as_deref())
                        {
                            Ok(_) => {
                                // Pairing request processed (new or existing), drop the message
                                return None;
                            }
                            Err(()) => {
                                // Error processing pairing, drop message
                                return None;
                            }
                        }
                    }
                }
                "allowlist" => {
                    // Default: check allow_from list
                    if !self.is_sender_allowed(&sender) {
                        tracing::debug!(sender = %sender, "Signal: sender not in allow_from, dropping");
                        return None;
                    }
                }
                _ => {}
            }
        }

        let target = Self::reply_target(data_msg, &sender);

        let timestamp = data_msg
            .timestamp
            .or(envelope.timestamp)
            .unwrap_or_else(|| {
                u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                )
                .unwrap_or(u64::MAX)
            });

        // Build metadata with signal-specific routing info.
        let metadata = serde_json::json!({
            "signal_sender": &sender,
            "signal_target": &target,
            "signal_timestamp": timestamp,
        });

        let mut msg = IncomingMessage::new("signal", &sender, text).with_metadata(metadata);

        // Use sourceName as display name if available.
        if let Some(ref name) = envelope.source_name
            && !name.is_empty()
        {
            msg = msg.with_user_name(name);
        }

        // Use a deterministic UUID as thread_id for all conversations.
        // This ensures DMs and groups continue the same thread AND work with
        // maybe_hydrate_thread, enabling conversation history persistence.
        // Priority: source_uuid > generated UUID from phone/group
        if data_msg.group_info.is_some() {
            // For groups, use the group ID to generate a deterministic UUID
            msg = msg.with_thread(Self::thread_id_from_identifier(&target));
        } else if let Some(ref uuid) = envelope.source_uuid {
            // Privacy mode users already have a UUID
            msg = msg.with_thread(uuid.clone());
        } else {
            // For regular DMs, generate a deterministic UUID from the phone number
            msg = msg.with_thread(Self::thread_id_from_identifier(&sender));
        }

        Some((msg, target))
    }
}

#[async_trait]
impl Channel for SignalChannel {
    fn name(&self) -> &str {
        "signal"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        let config = self.config.clone();
        let client = self.client.clone();
        let reply_targets = Arc::clone(&self.reply_targets);

        tokio::spawn(async move {
            if let Err(e) = sse_listener(config, client, tx, reply_targets).await {
                tracing::error!("Signal SSE listener exited with error: {e}");
            }
        });

        // Log the URL with credentials redacted (if any).
        let safe_url = Self::redact_url(&self.config.http_url);
        tracing::info!(
            url = %safe_url,
            "Signal channel started"
        );

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Resolve reply target from stored metadata.
        let target_str = {
            let targets = self.reply_targets.read().await;
            targets.peek(&msg.id).cloned()
        }
        .or_else(|| {
            // Fall back to metadata if not in the map.
            msg.metadata
                .get("signal_target")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| msg.user_id.clone());

        let target = Self::parse_recipient_target(&target_str);
        let params = self.build_rpc_params(&target, Some(&response.content));
        self.rpc_request("send", params).await?;

        // Clean up stored target.
        self.reply_targets.write().await.pop(&msg.id);

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Send typing indicator for thinking status.
        if matches!(status, StatusUpdate::Thinking(_))
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let target = Self::parse_recipient_target(target_str);
            let params = self.build_rpc_params(&target, None);
            let _ = self.rpc_request("sendTyping", params).await;
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let target = Self::parse_recipient_target(user_id);
        let params = self.build_rpc_params(&target, Some(&response.content));
        self.rpc_request("send", params).await?;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let url = format!("{}{}", self.config.http_url, SIGNAL_HEALTH_ENDPOINT);
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::HealthCheckFailed {
                name: format!("signal ({}): {e}", Self::redact_url(&url)),
            })?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: format!("signal: HTTP {}", resp.status()),
            })
        }
    }
}

/// Long-running SSE listener that reconnects with exponential backoff.
async fn sse_listener(
    config: SignalConfig,
    client: Client,
    tx: tokio::sync::mpsc::Sender<IncomingMessage>,
    reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
) -> Result<(), ChannelError> {
    let channel = SignalChannel::from_parts(config, client, Arc::clone(&reply_targets));

    let mut url = reqwest::Url::parse(&format!("{}/api/v1/events", channel.config.http_url))
        .map_err(|e| ChannelError::StartupFailed {
            name: "signal".to_string(),
            reason: format!("Invalid SSE URL: {e}"),
        })?;
    url.query_pairs_mut()
        .append_pair("account", &channel.config.account);

    let mut retry_delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(60);

    loop {
        let resp = channel
            .client
            .get(url.clone())
            .header("Accept", "text/event-stream")
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let status = r.status();
                let mut stream = r.bytes_stream();
                let mut bytes = Vec::new();
                let mut collected = 0usize;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.unwrap_or_default();
                    let remaining = MAX_ERROR_LOG_BODY.saturating_sub(collected);
                    if remaining == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                    collected = bytes.len();
                    if collected >= MAX_ERROR_LOG_BODY {
                        break;
                    }
                }
                let body = String::from_utf8_lossy(&bytes);
                tracing::warn!("Signal SSE returned {status}: {body}");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(max_delay);
                continue;
            }
            Err(e) => {
                let safe_url = SignalChannel::redact_url(url.as_str());
                tracing::warn!("Signal SSE connect error to {safe_url}: {e}, retrying...");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(max_delay);
                continue;
            }
        };

        // Connection succeeded — reset backoff.
        retry_delay = Duration::from_secs(2);
        tracing::info!("Signal SSE connected");

        let mut bytes_stream = resp.bytes_stream();
        let mut buffer = String::with_capacity(8192);
        let mut current_data = String::with_capacity(4096);
        // Holds trailing bytes from the previous chunk that form an incomplete
        // multi-byte UTF-8 sequence. At most 3 bytes (the longest incomplete
        // leading sequence for a 4-byte character).
        let mut utf8_carry: Vec<u8> = Vec::with_capacity(4);

        while let Some(chunk) = bytes_stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("Signal SSE chunk error, reconnecting: {e}");
                    break;
                }
            };

            // Prepend any leftover bytes from the previous chunk.
            let decode_buf = if utf8_carry.is_empty() {
                chunk.to_vec()
            } else {
                let mut combined = std::mem::take(&mut utf8_carry);
                combined.extend_from_slice(&chunk);
                combined
            };

            // Decode as much valid UTF-8 as possible, carrying over any
            // incomplete trailing sequence to the next iteration.
            let (valid_len, carry_start) = match std::str::from_utf8(&decode_buf) {
                Ok(_) => (decode_buf.len(), decode_buf.len()),
                Err(e) => {
                    let valid_up_to = e.valid_up_to();
                    match e.error_len() {
                        Some(bad_len) => {
                            // Genuinely invalid byte sequence (not just incomplete).
                            // Skip the bad byte(s) and keep going with what we have.
                            tracing::debug!(
                                "Signal SSE invalid UTF-8 byte at offset {valid_up_to}, \
                                 skipping"
                            );
                            // Advance past the bad byte(s); remaining data (if any)
                            // will be carried over to the next chunk.
                            (valid_up_to, valid_up_to + bad_len)
                        }
                        None => {
                            // Incomplete multi-byte sequence at the end – carry it over.
                            (valid_up_to, valid_up_to)
                        }
                    }
                }
            };

            use std::borrow::Cow;

            debug_assert!(
                std::str::from_utf8(&decode_buf[..valid_len]).is_ok(),
                "valid_len {} should be a valid UTF-8 boundary (buffer len: {})",
                valid_len,
                decode_buf.len()
            );

            let text: Cow<str> = match std::str::from_utf8(&decode_buf[..valid_len]) {
                Ok(s) => Cow::Borrowed(s),
                Err(_) => {
                    tracing::warn!(
                        "Signal SSE: unexpected invalid UTF-8 boundary at valid_len {}, \
                         falling back to lossy conversion",
                        valid_len
                    );
                    Cow::Owned(String::from_utf8_lossy(&decode_buf[..valid_len]).into_owned())
                }
            };

            if buffer.len() + text.len() > MAX_SSE_BUFFER_SIZE {
                tracing::warn!(
                    "Signal SSE buffer overflow, resetting: buffer_len={} text_len={} max={}",
                    buffer.len(),
                    text.len(),
                    MAX_SSE_BUFFER_SIZE
                );
                buffer.clear();
                utf8_carry.clear();
                current_data.clear();
                continue;
            }
            buffer.push_str(&text);

            // Preserve any trailing incomplete bytes for the next chunk.
            if carry_start < decode_buf.len() {
                utf8_carry.extend_from_slice(&decode_buf[carry_start..]);
            }

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                buffer.drain(..=newline_pos);

                // Skip SSE comments (keepalive).
                if line.starts_with(':') {
                    continue;
                }

                if line.is_empty() {
                    // Empty line = event boundary, dispatch accumulated data.
                    if !current_data.is_empty() {
                        match serde_json::from_str::<SseEnvelope>(&current_data) {
                            Ok(sse) => {
                                if let Some(ref envelope) = sse.envelope
                                    && let Some((msg, target)) = channel.process_envelope(envelope)
                                {
                                    // Store reply target for respond().
                                    // LruCache automatically evicts the
                                    // least-recently-used entry when full.
                                    {
                                        let mut targets = reply_targets.write().await;
                                        targets.put(msg.id, target);
                                    }
                                    if tx.send(msg).await.is_err() {
                                        tracing::debug!("Signal SSE: receiver dropped, exiting");
                                        return Ok(());
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::debug!("Signal SSE parse skip: {e}");
                            }
                        }
                        current_data.clear();
                    }
                } else if let Some(data) = line.strip_prefix("data:") {
                    if current_data.len() + data.len() > MAX_SSE_EVENT_SIZE {
                        tracing::warn!("Signal SSE event too large, dropping");
                        current_data.clear();
                        continue;
                    }
                    if !current_data.is_empty() {
                        current_data.push('\n');
                    }
                    current_data.push_str(data.trim_start());
                }
                // Ignore "event:", "id:", "retry:" lines.
            }
        }

        // Process any trailing data before reconnect.
        if !current_data.is_empty()
            && let Ok(sse) = serde_json::from_str::<SseEnvelope>(&current_data)
            && let Some(ref envelope) = sse.envelope
            && let Some((msg, target)) = channel.process_envelope(envelope)
        {
            reply_targets.write().await.put(msg.id, target);
            let _ = tx.send(msg).await;
        }

        tracing::debug!("Signal SSE stream ended, reconnecting with backoff...");
        tokio::time::sleep(retry_delay).await;
        retry_delay = std::cmp::min(retry_delay * 2, max_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> SignalConfig {
        SignalConfig {
            http_url: "http://127.0.0.1:8686".to_string(),
            account: "+1234567890".to_string(),
            allow_from: vec!["+1111111111".to_string()],
            allow_from_groups: vec![],
            dm_policy: "allowlist".to_string(),
            group_policy: "disabled".to_string(),
            group_allow_from: vec![],
            ignore_attachments: false,
            ignore_stories: false,
        }
    }

    /// Create a config that allows a specific group (and all senders).
    fn make_config_with_allowed_group(group_id: &str) -> SignalConfig {
        SignalConfig {
            http_url: "http://127.0.0.1:8686".to_string(),
            account: "+1234567890".to_string(),
            allow_from: vec!["*".to_string()],
            allow_from_groups: vec![group_id.to_string()],
            dm_policy: "allowlist".to_string(),
            group_policy: "allowlist".to_string(),
            group_allow_from: vec![],
            ignore_attachments: true,
            ignore_stories: true,
        }
    }

    fn make_channel() -> Result<SignalChannel, ChannelError> {
        SignalChannel::new(make_config())
    }

    fn make_channel_with_allowed_group(group_id: &str) -> Result<SignalChannel, ChannelError> {
        SignalChannel::new(make_config_with_allowed_group(group_id))
    }

    fn make_envelope(source_number: Option<&str>, message: Option<&str>) -> Envelope {
        Envelope {
            source: source_number.map(String::from),
            source_number: source_number.map(String::from),
            source_name: None,
            source_uuid: None,
            data_message: message.map(|m| DataMessage {
                message: Some(m.to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        }
    }

    #[test]
    fn creates_with_correct_fields() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
        assert_eq!(ch.config.account, "+1234567890");
        assert_eq!(ch.config.allow_from.len(), 1);
        assert!(ch.config.allow_from_groups.is_empty());
        assert!(!ch.config.ignore_attachments);
        assert!(!ch.config.ignore_stories);
        Ok(())
    }

    #[test]
    fn strips_trailing_slash() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.http_url = "http://127.0.0.1:8686/".to_string();
        let ch = SignalChannel::new(config)?;
        assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
        Ok(())
    }

    #[test]
    fn wildcard_allows_anyone() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_sender_allowed("+9999999999"));
        Ok(())
    }

    #[test]
    fn specific_sender_allowed() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        assert!(ch.is_sender_allowed("+1111111111"));
        Ok(())
    }

    #[test]
    fn unknown_sender_denied() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        assert!(!ch.is_sender_allowed("+9999999999"));
        Ok(())
    }

    #[test]
    fn empty_allowlist_denies_all() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec![];
        let ch = SignalChannel::new(config)?;
        assert!(!ch.is_sender_allowed("+1111111111"));
        Ok(())
    }

    #[test]
    fn uuid_prefix_in_allowlist() -> Result<(), ChannelError> {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let mut config = make_config();
        config.allow_from = vec![format!("uuid:{uuid}")];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_sender_allowed(uuid));
        // Should not match phone numbers.
        assert!(!ch.is_sender_allowed("+1111111111"));
        Ok(())
    }

    #[test]
    fn bare_uuid_in_allowlist() -> Result<(), ChannelError> {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let mut config = make_config();
        config.allow_from = vec![uuid.to_string()];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_sender_allowed(uuid));
        Ok(())
    }

    #[test]
    fn group_allowlist_filtering() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.allow_from_groups = vec!["group123".to_string()];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_group_allowed("group123"));
        assert!(!ch.is_group_allowed("other_group"));
        Ok(())
    }

    #[test]
    fn group_allowlist_wildcard() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from_groups = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_group_allowed("any_group"));
        Ok(())
    }

    #[test]
    fn group_allowlist_empty_denies_all() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from_groups = vec![];
        let ch = SignalChannel::new(config)?;
        assert!(!ch.is_group_allowed("any_group"));
        Ok(())
    }

    #[test]
    fn name_returns_signal() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        assert_eq!(ch.name(), "signal");
        Ok(())
    }

    #[test]
    fn process_envelope_dm_accepted_with_empty_allow_from_groups() -> Result<(), ChannelError> {
        // Empty allow_from_groups = DMs only. DMs should be accepted.
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), Some("Hello!"));
        assert!(ch.process_envelope(&env).is_some());
        Ok(())
    }

    #[test]
    fn process_envelope_group_denied_with_empty_allow_from_groups() -> Result<(), ChannelError> {
        // Empty allow_from_groups = DMs only. Group messages should be denied.
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("hi".to_string()),
                timestamp: Some(1000),
                group_info: Some(GroupInfo {
                    group_id: Some("group123".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1000),
        };
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn process_envelope_group_accepted_when_in_allow_from_groups() -> Result<(), ChannelError> {
        let ch = make_channel_with_allowed_group("group123")?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("hi".to_string()),
                timestamp: Some(1000),
                group_info: Some(GroupInfo {
                    group_id: Some("group123".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1000),
        };
        assert!(ch.process_envelope(&env).is_some());

        // Different group should be denied.
        let env2 = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("hi".to_string()),
                timestamp: Some(1000),
                group_info: Some(GroupInfo {
                    group_id: Some("other_group".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1000),
        };
        assert!(ch.process_envelope(&env2).is_none());
        Ok(())
    }

    #[test]
    fn reply_target_dm() {
        let dm = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: None,
            attachments: None,
        };
        assert_eq!(
            SignalChannel::reply_target(&dm, "+1111111111"),
            "+1111111111"
        );
    }

    #[test]
    fn reply_target_group() {
        let group = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        };
        assert_eq!(
            SignalChannel::reply_target(&group, "+1111111111"),
            "group:group123"
        );
    }

    #[test]
    fn parse_recipient_target_e164_is_direct() {
        assert_eq!(
            SignalChannel::parse_recipient_target("+1234567890"),
            RecipientTarget::Direct("+1234567890".to_string())
        );
    }

    #[test]
    fn parse_recipient_target_prefixed_group_is_group() {
        assert_eq!(
            SignalChannel::parse_recipient_target("group:abc123"),
            RecipientTarget::Group("abc123".to_string())
        );
    }

    #[test]
    fn parse_recipient_target_uuid_is_direct() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        assert_eq!(
            SignalChannel::parse_recipient_target(uuid),
            RecipientTarget::Direct(uuid.to_string())
        );
    }

    #[test]
    fn parse_recipient_target_non_e164_plus_is_group() {
        assert_eq!(
            SignalChannel::parse_recipient_target("+abc123"),
            RecipientTarget::Group("+abc123".to_string())
        );
    }

    #[test]
    fn is_uuid_valid() {
        assert!(SignalChannel::is_uuid(
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
        ));
        assert!(SignalChannel::is_uuid(
            "00000000-0000-0000-0000-000000000000"
        ));
    }

    #[test]
    fn is_uuid_invalid() {
        assert!(!SignalChannel::is_uuid("+1234567890"));
        assert!(!SignalChannel::is_uuid("not-a-uuid"));
        assert!(!SignalChannel::is_uuid("group:abc123"));
        assert!(!SignalChannel::is_uuid(""));
    }

    #[test]
    fn thread_id_from_identifier_is_deterministic() {
        let id1 = SignalChannel::thread_id_from_identifier("+1234567890");
        let id2 = SignalChannel::thread_id_from_identifier("+1234567890");
        assert_eq!(id1, id2, "same input should produce same UUID");
    }

    #[test]
    fn thread_id_from_identifier_is_valid_uuid() {
        let id = SignalChannel::thread_id_from_identifier("+1234567890");
        assert!(Uuid::parse_str(&id).is_ok(), "should be a valid UUID");
    }

    #[test]
    fn thread_id_from_identifier_different_inputs() {
        let id1 = SignalChannel::thread_id_from_identifier("+1234567890");
        let id2 = SignalChannel::thread_id_from_identifier("+9876543210");
        assert_ne!(id1, id2, "different inputs should produce different UUIDs");
    }

    #[test]
    fn sender_prefers_source_number() {
        let env = Envelope {
            source: Some("uuid-123".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: None,
            story_message: None,
            timestamp: Some(1000),
        };
        assert_eq!(SignalChannel::sender(&env), Some("+1111111111".to_string()));
    }

    #[test]
    fn sender_falls_back_to_source() {
        let env = Envelope {
            source: Some("a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string()),
            source_number: None,
            source_name: None,
            source_uuid: None,
            data_message: None,
            story_message: None,
            timestamp: Some(1000),
        };
        assert_eq!(
            SignalChannel::sender(&env),
            Some("a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string())
        );
    }

    #[test]
    fn sender_none_when_both_missing() {
        let env = Envelope {
            source: None,
            source_number: None,
            source_name: None,
            source_uuid: None,
            data_message: None,
            story_message: None,
            timestamp: None,
        };
        assert_eq!(SignalChannel::sender(&env), None);
    }

    #[test]
    fn process_envelope_valid_dm() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), Some("Hello!"));
        let (msg, target) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.content, "Hello!");
        assert_eq!(msg.user_id, "+1111111111");
        assert_eq!(msg.channel, "signal");
        assert_eq!(target, "+1111111111");
        Ok(())
    }

    #[test]
    fn process_envelope_denied_sender() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+9999999999"), Some("Hello!"));
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn process_envelope_empty_message() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), Some(""));
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn process_envelope_no_data_message() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), None);
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn process_envelope_skips_stories() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.ignore_stories = true;
        let ch = SignalChannel::new(config)?;
        let mut env = make_envelope(Some("+1111111111"), Some("story text"));
        env.story_message = Some(serde_json::json!({}));
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn process_envelope_skips_attachment_only() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.ignore_attachments = true;
        let ch = SignalChannel::new(config)?;
        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: None,
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: Some(vec![serde_json::json!({"contentType": "image/png"})]),
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn process_envelope_uuid_sender_dm() -> Result<(), ChannelError> {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some(uuid.to_string()),
            source_number: None,
            source_name: Some("Privacy User".to_string()),
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Hello from privacy user".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let (msg, target) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.user_id, uuid);
        assert_eq!(msg.user_name.as_deref(), Some("Privacy User"));
        assert_eq!(msg.content, "Hello from privacy user");
        assert_eq!(target, uuid);

        // Verify reply routing: UUID sender in DM should route as Direct.
        let parsed = SignalChannel::parse_recipient_target(&target);
        assert_eq!(parsed, RecipientTarget::Direct(uuid.to_string()));
        Ok(())
    }

    #[test]
    fn process_envelope_uuid_sender_in_group() -> Result<(), ChannelError> {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let mut config = make_config_with_allowed_group("testgroup");
        config.ignore_attachments = false;
        config.ignore_stories = false;
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some(uuid.to_string()),
            source_number: None,
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Group msg from privacy user".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: Some(GroupInfo {
                    group_id: Some("testgroup".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let (msg, target) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.user_id, uuid);
        assert_eq!(target, "group:testgroup");
        // Groups now use deterministic UUID derived from group ID
        let expected_thread_id = SignalChannel::thread_id_from_identifier("group:testgroup");
        assert_eq!(msg.thread_id, Some(expected_thread_id));

        // Verify reply routing: group message should still route as Group.
        let parsed = SignalChannel::parse_recipient_target(&target);
        assert_eq!(parsed, RecipientTarget::Group("testgroup".to_string()));
        Ok(())
    }

    #[test]
    fn process_envelope_group_not_in_allow_from_groups() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.allow_from_groups = vec!["allowed_group".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Hi".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: Some(GroupInfo {
                    group_id: Some("other_group".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        assert!(ch.process_envelope(&env).is_none());
        Ok(())
    }

    #[test]
    fn sse_envelope_deserializes() {
        let json = r#"{
            "envelope": {
                "source": "+1111111111",
                "sourceNumber": "+1111111111",
                "sourceName": "Test User",
                "timestamp": 1700000000000,
                "dataMessage": {
                    "message": "Hello Signal!",
                    "timestamp": 1700000000000
                }
            }
        }"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        let env = sse.envelope.unwrap();
        assert_eq!(env.source_number.as_deref(), Some("+1111111111"));
        assert_eq!(env.source_name.as_deref(), Some("Test User"));
        let dm = env.data_message.unwrap();
        assert_eq!(dm.message.as_deref(), Some("Hello Signal!"));
    }

    #[test]
    fn sse_envelope_deserializes_group() {
        let json = r#"{
            "envelope": {
                "sourceNumber": "+2222222222",
                "dataMessage": {
                    "message": "Group msg",
                    "groupInfo": {
                        "groupId": "abc123"
                    }
                }
            }
        }"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        let env = sse.envelope.unwrap();
        let dm = env.data_message.unwrap();
        assert_eq!(
            dm.group_info.as_ref().unwrap().group_id.as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn envelope_defaults() {
        let json = r#"{}"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert!(env.source.is_none());
        assert!(env.source_number.is_none());
        assert!(env.source_name.is_none());
        assert!(env.data_message.is_none());
        assert!(env.story_message.is_none());
        assert!(env.timestamp.is_none());
    }

    #[test]
    fn normalize_allow_entry_strips_uuid_prefix() {
        assert_eq!(
            SignalChannel::normalize_allow_entry("uuid:abc-123"),
            "abc-123"
        );
        assert_eq!(
            SignalChannel::normalize_allow_entry("+1234567890"),
            "+1234567890"
        );
        assert_eq!(SignalChannel::normalize_allow_entry("*"), "*");
    }

    // ── build_rpc_params tests ──────────────────────────────────────

    #[test]
    fn build_rpc_params_direct_with_message() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let target = RecipientTarget::Direct("+5555555555".to_string());
        let params = ch.build_rpc_params(&target, Some("Hello!"));
        assert_eq!(params["recipient"], serde_json::json!(["+5555555555"]));
        assert_eq!(params["account"], "+1234567890");
        assert_eq!(params["message"], "Hello!");
        // Direct targets must NOT include groupId.
        assert!(params.get("groupId").is_none());
        Ok(())
    }

    #[test]
    fn build_rpc_params_direct_without_message() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let target = RecipientTarget::Direct("+5555555555".to_string());
        let params = ch.build_rpc_params(&target, None);
        assert_eq!(params["recipient"], serde_json::json!(["+5555555555"]));
        assert_eq!(params["account"], "+1234567890");
        // No message key should be present for typing indicators.
        assert!(params.get("message").is_none());
        Ok(())
    }

    #[test]
    fn build_rpc_params_group_with_message() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let target = RecipientTarget::Group("abc123".to_string());
        let params = ch.build_rpc_params(&target, Some("Group msg"));
        assert_eq!(params["groupId"], "abc123");
        assert_eq!(params["account"], "+1234567890");
        assert_eq!(params["message"], "Group msg");
        // Group targets must NOT include recipient.
        assert!(params.get("recipient").is_none());
        Ok(())
    }

    #[test]
    fn build_rpc_params_group_without_message() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let target = RecipientTarget::Group("abc123".to_string());
        let params = ch.build_rpc_params(&target, None);
        assert_eq!(params["groupId"], "abc123");
        assert_eq!(params["account"], "+1234567890");
        assert!(params.get("message").is_none());
        Ok(())
    }

    #[test]
    fn build_rpc_params_uuid_direct_target() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let target = RecipientTarget::Direct(uuid.to_string());
        let params = ch.build_rpc_params(&target, Some("hi"));
        assert_eq!(params["recipient"], serde_json::json!([uuid]));
        Ok(())
    }

    // ── metadata assertion tests ────────────────────────────────────

    #[test]
    fn process_envelope_metadata_has_signal_fields() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), Some("Hello!"));
        let (msg, _) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.metadata["signal_sender"], "+1111111111");
        assert_eq!(msg.metadata["signal_target"], "+1111111111");
        assert_eq!(msg.metadata["signal_timestamp"], 1_700_000_000_000_u64);
        Ok(())
    }

    #[test]
    fn process_envelope_metadata_group_target() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.allow_from_groups = vec!["*".to_string()];
        config.group_policy = "allowlist".to_string();
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+2222222222".to_string()),
            source_number: Some("+2222222222".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("In the group".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: Some(GroupInfo {
                    group_id: Some("mygroup".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.metadata["signal_target"], "group:mygroup");
        assert_eq!(msg.metadata["signal_sender"], "+2222222222");
        Ok(())
    }

    // ── attachment-with-text tests ──────────────────────────────────

    #[test]
    fn process_envelope_attachment_with_text_not_skipped() -> Result<(), ChannelError> {
        // Even with ignore_attachments=true, messages that have BOTH text
        // and attachments should be processed (only attachment-only are skipped).
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.ignore_attachments = true;
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Check this out".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: Some(vec![serde_json::json!({"contentType": "image/png"})]),
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let result = ch.process_envelope(&env);
        assert!(
            result.is_some(),
            "Message with text + attachment should not be skipped"
        );
        let (msg, _) = result.unwrap();
        assert_eq!(msg.content, "Check this out");
        Ok(())
    }

    #[test]
    fn process_envelope_attachment_only_not_skipped_when_ignore_disabled()
    -> Result<(), ChannelError> {
        // With ignore_attachments=false, attachment-only messages should be
        // processed with the "[Attachment]" placeholder text.
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.ignore_attachments = false;
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: None,
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: Some(vec![serde_json::json!({"contentType": "image/png"})]),
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        // With ignore_attachments=false, attachment-only messages are now
        // processed with a placeholder "[Attachment]" text.
        let result = ch.process_envelope(&env);
        assert!(
            result.is_some(),
            "Attachment-only should be processed when ignore_attachments=false"
        );
        let (msg, _) = result.unwrap();
        assert_eq!(msg.content, "[Attachment]");
        Ok(())
    }

    // ── source_name / display name tests ────────────────────────────

    #[test]
    fn process_envelope_source_name_sets_user_name() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+3333333333".to_string()),
            source_number: Some("+3333333333".to_string()),
            source_name: Some("Alice".to_string()),
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Hey".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.user_name.as_deref(), Some("Alice"));
        Ok(())
    }

    #[test]
    fn process_envelope_empty_source_name_not_set() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+3333333333".to_string()),
            source_number: Some("+3333333333".to_string()),
            source_name: Some("".to_string()),
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Hey".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        assert!(
            msg.user_name.is_none(),
            "Empty source_name should not set user_name"
        );
        Ok(())
    }

    #[test]
    fn process_envelope_no_source_name_not_set() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), Some("hi"));
        let (msg, _) = ch.process_envelope(&env).unwrap();
        assert!(msg.user_name.is_none());
        Ok(())
    }

    // ── thread_id tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn process_envelope_dm_sets_thread_id_to_uuid() -> Result<(), ChannelError> {
        let ch = make_channel()?;
        let env = make_envelope(Some("+1111111111"), Some("DM"));
        let (msg, _) = ch.process_envelope(&env).unwrap();
        // DMs now set thread_id to a deterministic UUID derived from phone number
        let expected_thread_id = SignalChannel::thread_id_from_identifier("+1111111111");
        assert_eq!(
            msg.thread_id,
            Some(expected_thread_id),
            "DMs should set thread_id to UUID"
        );
        Ok(())
    }

    #[test]
    fn process_envelope_group_sets_thread_id_to_uuid() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.allow_from_groups = vec!["*".to_string()];
        config.group_policy = "allowlist".to_string();
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("Group msg".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: Some(GroupInfo {
                    group_id: Some("grp999".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        // Groups now set thread_id to a deterministic UUID derived from group ID
        let expected_thread_id = SignalChannel::thread_id_from_identifier("group:grp999");
        assert_eq!(
            msg.thread_id,
            Some(expected_thread_id),
            "Groups should set thread_id to UUID"
        );
        Ok(())
    }

    // ── timestamp edge cases ────────────────────────────────────────

    #[test]
    fn process_envelope_uses_data_message_timestamp() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("hi".to_string()),
                timestamp: Some(9999),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1111),
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        // data_message timestamp takes priority.
        assert_eq!(msg.metadata["signal_timestamp"], 9999);
        Ok(())
    }

    #[test]
    fn process_envelope_falls_back_to_envelope_timestamp() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("hi".to_string()),
                timestamp: None,
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(7777),
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.metadata["signal_timestamp"], 7777);
        Ok(())
    }

    #[test]
    fn process_envelope_generates_timestamp_when_missing() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("hi".to_string()),
                timestamp: None,
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: None,
        };
        let (msg, _) = ch.process_envelope(&env).unwrap();
        // Should generate a timestamp (current time in millis), just verify it's positive.
        let ts = msg.metadata["signal_timestamp"].as_u64().unwrap();
        assert!(ts > 0, "Generated timestamp should be positive");
        Ok(())
    }

    // ── SSE envelope deserialization edge cases ─────────────────────

    #[test]
    fn sse_envelope_missing_envelope_field() {
        let json = r#"{"account": "+1234567890"}"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        assert!(sse.envelope.is_none());
    }

    #[test]
    fn sse_envelope_with_story_message() {
        let json = r#"{
            "envelope": {
                "sourceNumber": "+1111111111",
                "storyMessage": {"allowsReplies": true},
                "dataMessage": {
                    "message": "story text"
                }
            }
        }"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        let env = sse.envelope.unwrap();
        assert!(env.story_message.is_some());
        assert!(env.data_message.is_some());
    }

    #[test]
    fn sse_envelope_with_attachments() {
        let json = r#"{
            "envelope": {
                "sourceNumber": "+1111111111",
                "dataMessage": {
                    "message": "See attached",
                    "attachments": [
                        {"contentType": "image/jpeg", "filename": "photo.jpg"},
                        {"contentType": "application/pdf"}
                    ]
                }
            }
        }"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        let dm = sse.envelope.unwrap().data_message.unwrap();
        let attachments = dm.attachments.unwrap();
        assert_eq!(attachments.len(), 2);
    }

    // ── is_e164 tests ───────────────────────────────────────────────

    #[test]
    fn is_e164_valid_numbers() {
        assert!(SignalChannel::is_e164("+12345678901"));
        assert!(SignalChannel::is_e164("+1234567")); // min 7 digits after +
        assert!(SignalChannel::is_e164("+123456789012345")); // max 15 digits
    }

    #[test]
    fn is_e164_invalid_numbers() {
        assert!(!SignalChannel::is_e164("12345678901")); // no +
        assert!(!SignalChannel::is_e164("+1")); // too short (1 digit)
        assert!(!SignalChannel::is_e164("+1234567890123456")); // too long (16 digits)
        assert!(!SignalChannel::is_e164("+abc123")); // non-digit
        assert!(!SignalChannel::is_e164("")); // empty
        assert!(!SignalChannel::is_e164("+")); // plus only
    }

    // ── config edge cases ───────────────────────────────────────────

    #[test]
    fn multiple_allow_from() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from = vec![
            "+1111111111".to_string(),
            "+2222222222".to_string(),
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string(),
        ];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_sender_allowed("+1111111111"));
        assert!(ch.is_sender_allowed("+2222222222"));
        assert!(ch.is_sender_allowed("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));
        assert!(!ch.is_sender_allowed("+9999999999"));
        Ok(())
    }

    #[test]
    fn multiple_allow_from_groups() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.allow_from_groups = vec!["group_a".to_string(), "group_b".to_string()];
        let ch = SignalChannel::new(config)?;
        assert!(ch.is_group_allowed("group_a"));
        assert!(ch.is_group_allowed("group_b"));
        assert!(!ch.is_group_allowed("group_c"));
        Ok(())
    }

    #[test]
    fn uuid_prefix_normalization_in_allowlist() -> Result<(), ChannelError> {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let mut config = make_config();
        config.allow_from = vec![format!("uuid:{uuid}"), "+1111111111".to_string()];
        let ch = SignalChannel::new(config)?;
        // uuid:-prefixed entry should match bare UUID sender.
        assert!(ch.is_sender_allowed(uuid));
        // Phone numbers still work alongside UUID entries.
        assert!(ch.is_sender_allowed("+1111111111"));
        // Non-matching should fail.
        assert!(!ch.is_sender_allowed("+9999999999"));
        Ok(())
    }

    // ── stories behavior tests ──────────────────────────────────────

    #[test]
    fn process_envelope_stories_not_skipped_when_disabled() -> Result<(), ChannelError> {
        // With ignore_stories=false, story messages with a data_message
        // should still be processed.
        let mut config = make_config();
        config.allow_from = vec!["*".to_string()];
        config.ignore_stories = false;
        let ch = SignalChannel::new(config)?;

        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            source_name: None,
            source_uuid: None,
            data_message: Some(DataMessage {
                message: Some("story with text".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: Some(serde_json::json!({})),
            timestamp: Some(1_700_000_000_000),
        };
        let result = ch.process_envelope(&env);
        assert!(
            result.is_some(),
            "Stories should not be skipped when ignore_stories=false"
        );
        Ok(())
    }

    // ── trailing slash variations ───────────────────────────────────

    #[test]
    fn strips_multiple_trailing_slashes() -> Result<(), ChannelError> {
        let mut config = make_config();
        config.http_url = "http://127.0.0.1:8686///".to_string();
        let ch = SignalChannel::new(config)?;
        assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
        Ok(())
    }

    #[test]
    fn preserves_url_without_trailing_slash() -> Result<(), ChannelError> {
        let config = make_config();
        let ch = SignalChannel::new(config)?;
        assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
        Ok(())
    }
}
