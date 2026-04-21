//! Host state for WASM channel execution.
//!
//! Extends the base tool host state with channel-specific functionality:
//! - Message emission (queueing messages to send to the agent)
//! - Workspace write access (scoped to channel namespace)
//! - Rate limiting for message emission

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::channels::wasm::capabilities::{ChannelCapabilities, EmitRateLimitConfig};
use crate::channels::wasm::error::WasmChannelError;
use crate::tools::wasm::{HostState, LogLevel};

/// Maximum emitted messages per callback execution.
const MAX_EMITS_PER_EXECUTION: usize = 100;

/// Maximum message content size (64 KB).
const MAX_MESSAGE_CONTENT_SIZE: usize = 64 * 1024;

/// A file or media attachment on an incoming message.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Unique identifier within the channel (e.g., Telegram file_id).
    pub id: String,
    /// MIME type (e.g., "image/jpeg", "audio/ogg", "application/pdf").
    pub mime_type: String,
    /// Original filename, if known.
    pub filename: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
    /// URL to download the file from the channel's API.
    pub source_url: Option<String>,
    /// Opaque key for host-side storage (e.g., after download/caching).
    pub storage_key: Option<String>,
    /// Relative path to a project-local copy saved on disk, if persisted.
    pub local_path: Option<String>,
    /// Extracted text content (e.g., OCR result, PDF text, audio transcript).
    pub extracted_text: Option<String>,
    /// Extensible metadata from the channel payload.
    pub extras_json: String,
    /// Raw file bytes (for small files downloaded by the channel).
    pub data: Vec<u8>,
    /// Duration in seconds (for audio/video).
    pub duration_secs: Option<u32>,
}

/// Maximum total attachment size per message (20 MB).
const MAX_ATTACHMENT_TOTAL_SIZE: u64 = 20 * 1024 * 1024;

/// Maximum number of attachments per message.
const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;

/// Allowed MIME type prefixes for attachments.
const ALLOWED_MIME_PREFIXES: &[&str] = &[
    "image/",
    "audio/",
    "video/",
    "application/pdf",
    "application/vnd.",
    "application/msword",
    "application/rtf",
    "text/",
    "application/json",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/octet-stream",
];
/// Truncate a string to at most `max_bytes` without splitting UTF-8 code points.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    let end = crate::util::floor_char_boundary(s, max_bytes);
    &s[..end]
}
/// A message emitted by a WASM channel to be sent to the agent.
#[derive(Debug, Clone)]
pub struct EmittedMessage {
    /// User identifier within the channel.
    pub user_id: String,

    /// Optional user display name.
    pub user_name: Option<String>,

    /// Message content.
    pub content: String,

    /// Optional thread ID for threaded conversations.
    pub thread_id: Option<String>,

    /// Channel-specific metadata as JSON string.
    pub metadata_json: String,

    /// File or media attachments on this message.
    pub attachments: Vec<Attachment>,

    /// Timestamp when the message was emitted.
    pub emitted_at_millis: u64,
}

impl EmittedMessage {
    /// Create a new emitted message.
    pub fn new(user_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: None,
            content: content.into(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: Vec::new(),
            emitted_at_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Set the user name.
    pub fn with_user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    /// Set the thread ID.
    pub fn with_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Set metadata JSON.
    pub fn with_metadata(mut self, metadata_json: impl Into<String>) -> Self {
        self.metadata_json = metadata_json.into();
        self
    }

    /// Set attachments.
    pub fn with_attachments(mut self, attachments: Vec<Attachment>) -> Self {
        self.attachments = attachments;
        self
    }
}

/// A pending workspace write operation.
#[derive(Debug, Clone)]
pub struct PendingWorkspaceWrite {
    /// Full path (already prefixed with channel namespace).
    pub path: String,

    /// Content to write.
    pub content: String,
}

/// Host state for WASM channel callbacks.
///
/// Maintains all side effects during callback execution and enforces limits.
/// This is the channel-specific equivalent of HostState for tools.
pub struct ChannelHostState {
    /// Base tool host state (logging, time, HTTP, etc.).
    base: HostState,

    /// Channel name (for error messages).
    channel_name: String,

    /// Channel capabilities.
    capabilities: ChannelCapabilities,

    /// Emitted messages (queued for delivery).
    emitted_messages: Vec<EmittedMessage>,

    /// Pending workspace writes.
    pending_writes: Vec<PendingWorkspaceWrite>,

    /// Emit count for rate limiting within this execution.
    emit_count: u32,

    /// Whether emit is still allowed (false after rate limit hit).
    emit_enabled: bool,

    /// Count of emits dropped due to rate limiting.
    emits_dropped: usize,

    /// Binary data stored for attachments via `store-attachment-data`.
    /// Keyed by attachment ID, cleared after callback completes.
    attachment_data: HashMap<String, Vec<u8>>,

    /// Total bytes stored in attachment_data (for enforcing limits).
    attachment_data_total: u64,
}

impl std::fmt::Debug for ChannelHostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelHostState")
            .field("channel_name", &self.channel_name)
            .field("emitted_messages_count", &self.emitted_messages.len())
            .field("pending_writes_count", &self.pending_writes.len())
            .field("emit_count", &self.emit_count)
            .field("emit_enabled", &self.emit_enabled)
            .field("emits_dropped", &self.emits_dropped)
            .finish()
    }
}

impl ChannelHostState {
    /// Create a new channel host state.
    pub fn new(channel_name: impl Into<String>, capabilities: ChannelCapabilities) -> Self {
        let base = HostState::new(capabilities.tool_capabilities.clone());

        Self {
            base,
            channel_name: channel_name.into(),
            capabilities,
            emitted_messages: Vec::new(),
            pending_writes: Vec::new(),
            emit_count: 0,
            emit_enabled: true,
            emits_dropped: 0,
            attachment_data: HashMap::new(),
            attachment_data_total: 0,
        }
    }

    /// Get the channel name.
    pub fn channel_name(&self) -> &str {
        &self.channel_name
    }

    /// Get the capabilities.
    pub fn capabilities(&self) -> &ChannelCapabilities {
        &self.capabilities
    }

    /// Get the base host state for tool capabilities.
    pub fn base(&self) -> &HostState {
        &self.base
    }

    /// Get mutable access to the base host state.
    pub fn base_mut(&mut self) -> &mut HostState {
        &mut self.base
    }

    /// Emit a message from the channel.
    ///
    /// Messages are queued and delivered after callback execution completes.
    /// Rate limiting is enforced per-execution and globally.
    /// Attachments are validated for count, total size, and MIME type.
    pub fn emit_message(&mut self, msg: EmittedMessage) -> Result<(), WasmChannelError> {
        // Check per-execution limit
        if !self.emit_enabled {
            self.emits_dropped += 1;
            return Ok(()); // Silently drop, don't fail execution
        }

        if self.emitted_messages.len() >= MAX_EMITS_PER_EXECUTION {
            self.emit_enabled = false;
            self.emits_dropped += 1;
            tracing::warn!(
                channel = %self.channel_name,
                limit = MAX_EMITS_PER_EXECUTION,
                "Channel emit limit reached, further messages dropped"
            );
            return Ok(());
        }

        // Validate attachments
        let msg = self.validate_attachments(msg);

        // Validate message content size
        if msg.content.len() > MAX_MESSAGE_CONTENT_SIZE {
            tracing::warn!(
                channel = %self.channel_name,
                size = msg.content.len(),
                max = MAX_MESSAGE_CONTENT_SIZE,
                "Message content too large, truncating"
            );
            let mut truncated = truncate_utf8(&msg.content, MAX_MESSAGE_CONTENT_SIZE).to_string();
            truncated.push_str("... (truncated)");
            let msg = EmittedMessage {
                content: truncated,
                ..msg
            };
            self.emitted_messages.push(msg);
        } else {
            self.emitted_messages.push(msg);
        }

        self.emit_count += 1;
        Ok(())
    }

    /// Validate and sanitize attachments on an emitted message.
    ///
    /// Enforces count limits, total size limits, and MIME type allowlist.
    /// Invalid attachments are dropped with a warning.
    fn validate_attachments(&self, mut msg: EmittedMessage) -> EmittedMessage {
        if msg.attachments.is_empty() {
            return msg;
        }

        // Enforce attachment count limit
        if msg.attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
            tracing::warn!(
                channel = %self.channel_name,
                count = msg.attachments.len(),
                max = MAX_ATTACHMENTS_PER_MESSAGE,
                "Too many attachments, truncating"
            );
            msg.attachments.truncate(MAX_ATTACHMENTS_PER_MESSAGE);
        }

        // Filter by MIME type and enforce total size limit
        let mut total_size: u64 = 0;
        msg.attachments.retain(|att| {
            let mime_ok = ALLOWED_MIME_PREFIXES
                .iter()
                .any(|prefix| att.mime_type.starts_with(prefix));
            if !mime_ok {
                tracing::warn!(
                    channel = %self.channel_name,
                    mime_type = %att.mime_type,
                    "Attachment MIME type not allowed, dropping"
                );
                return false;
            }

            // Use the larger of reported size_bytes and actual stored data size
            // to prevent WASM channels from under-reporting to bypass limits.
            let stored_size = self
                .attachment_data
                .get(&att.id)
                .map(|d| d.len() as u64)
                .unwrap_or(att.data.len() as u64);
            let size = att
                .size_bytes
                .map(|reported| reported.max(stored_size))
                .unwrap_or(stored_size);
            if size > 0 {
                total_size = total_size.saturating_add(size);
                if total_size > MAX_ATTACHMENT_TOTAL_SIZE {
                    tracing::warn!(
                        channel = %self.channel_name,
                        total_size,
                        max = MAX_ATTACHMENT_TOTAL_SIZE,
                        "Attachment total size exceeded, dropping"
                    );
                    return false;
                }
            }

            true
        });

        msg
    }

    /// Take all emitted messages (clears the queue).
    pub fn take_emitted_messages(&mut self) -> Vec<EmittedMessage> {
        std::mem::take(&mut self.emitted_messages)
    }

    /// Get the number of emitted messages.
    pub fn emitted_count(&self) -> usize {
        self.emitted_messages.len()
    }

    /// Get the number of emits dropped due to rate limiting.
    pub fn emits_dropped(&self) -> usize {
        self.emits_dropped
    }

    /// Store binary data for an attachment.
    ///
    /// Called by WASM channels to associate downloaded bytes with an attachment ID.
    /// The data is retrieved after callback completion and merged into `Attachment::data`.
    pub fn store_attachment_data(
        &mut self,
        attachment_id: &str,
        data: Vec<u8>,
    ) -> Result<(), WasmChannelError> {
        const MAX_PER_ATTACHMENT: u64 = 20 * 1024 * 1024; // 20 MB
        const MAX_TOTAL: u64 = 50 * 1024 * 1024; // 50 MB

        let size = data.len() as u64;
        if size > MAX_PER_ATTACHMENT {
            return Err(WasmChannelError::CallbackFailed {
                name: self.channel_name.clone(),
                reason: format!(
                    "Attachment data too large: {} bytes (max {})",
                    size, MAX_PER_ATTACHMENT
                ),
            });
        }

        // Subtract the old entry size (if overwriting) before adding new size
        let old_size = self
            .attachment_data
            .get(attachment_id)
            .map(|d| d.len() as u64)
            .unwrap_or(0);
        let adjusted_total = self.attachment_data_total.saturating_sub(old_size);
        let new_total = adjusted_total.saturating_add(size);
        if new_total > MAX_TOTAL {
            return Err(WasmChannelError::CallbackFailed {
                name: self.channel_name.clone(),
                reason: format!(
                    "Total attachment data too large: {} bytes (max {})",
                    new_total, MAX_TOTAL
                ),
            });
        }

        self.attachment_data_total = new_total;
        self.attachment_data.insert(attachment_id.to_string(), data);
        Ok(())
    }

    /// Remove stored binary data for a specific attachment ID.
    pub fn remove_attachment_data(&mut self, id: &str) -> Option<Vec<u8>> {
        if let Some(data) = self.attachment_data.remove(id) {
            self.attachment_data_total =
                self.attachment_data_total.saturating_sub(data.len() as u64);
            Some(data)
        } else {
            None
        }
    }

    /// Take all stored attachment data (clears the store).
    pub fn take_attachment_data(&mut self) -> HashMap<String, Vec<u8>> {
        self.attachment_data_total = 0;
        std::mem::take(&mut self.attachment_data)
    }

    /// Write to workspace (scoped to channel namespace).
    ///
    /// Writes are queued and committed after callback execution completes.
    pub fn workspace_write(&mut self, path: &str, content: String) -> Result<(), WasmChannelError> {
        // Validate and prefix path
        let full_path = self
            .capabilities
            .validate_workspace_path(path)
            .map_err(|reason| WasmChannelError::WorkspaceEscape {
                name: self.channel_name.clone(),
                path: reason,
            })?;

        self.pending_writes.push(PendingWorkspaceWrite {
            path: full_path,
            content,
        });

        Ok(())
    }

    /// Take all pending workspace writes (clears the queue).
    pub fn take_pending_writes(&mut self) -> Vec<PendingWorkspaceWrite> {
        std::mem::take(&mut self.pending_writes)
    }

    /// Get the number of pending workspace writes.
    pub fn pending_writes_count(&self) -> usize {
        self.pending_writes.len()
    }

    /// Log a message (delegates to base).
    pub fn log(
        &mut self,
        level: LogLevel,
        message: String,
    ) -> Result<(), crate::tools::wasm::WasmError> {
        self.base.log(level, message)
    }

    /// Get current timestamp in milliseconds (delegates to base).
    pub fn now_millis(&self) -> u64 {
        self.base.now_millis()
    }

    /// Read from workspace (delegates to base).
    pub fn workspace_read(
        &self,
        path: &str,
    ) -> Result<Option<String>, crate::tools::wasm::WasmError> {
        // Prefix the path with channel namespace before reading
        let full_path = self.capabilities.prefix_workspace_path(path);
        self.base.workspace_read(&full_path)
    }

    /// Check if a secret exists (delegates to base).
    pub fn secret_exists(&self, name: &str) -> bool {
        self.base.secret_exists(name)
    }

    /// Check if HTTP is allowed (delegates to base).
    pub fn check_http_allowed(&self, url: &str, method: &str) -> Result<(), String> {
        self.base.check_http_allowed(url, method)
    }

    /// Record an HTTP request (delegates to base).
    pub fn record_http_request(&mut self) -> Result<(), String> {
        self.base.record_http_request()
    }

    /// Take logs (delegates to base).
    pub fn take_logs(&mut self) -> Vec<crate::tools::wasm::LogEntry> {
        self.base.take_logs()
    }
}

/// In-memory workspace store for WASM channels.
///
/// Persists workspace writes across callback invocations within a single
/// channel lifetime. This allows WASM channels to maintain state (e.g.,
/// Telegram polling offsets) between poll ticks without requiring a
/// full database-backed workspace.
///
/// Uses `std::sync::RwLock` (not tokio) because WASM execution runs
/// inside `spawn_blocking`.
pub struct ChannelWorkspaceStore {
    data: std::sync::RwLock<std::collections::HashMap<String, String>>,
}

impl ChannelWorkspaceStore {
    /// Create a new empty workspace store.
    pub fn new() -> Self {
        Self {
            data: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Commit pending writes from a callback execution into the store.
    pub fn commit_writes(&self, writes: &[PendingWorkspaceWrite]) {
        if writes.is_empty() {
            return;
        }
        if let Ok(mut data) = self.data.write() {
            for write in writes {
                tracing::debug!(
                    path = %write.path,
                    content_len = write.content.len(),
                    "Committing workspace write to channel store"
                );
                data.insert(write.path.clone(), write.content.clone());
            }
        }
    }

    /// Restore a previously-persisted subset of workspace values.
    pub fn restore_snapshot(&self, snapshot: &std::collections::HashMap<String, String>) {
        if snapshot.is_empty() {
            return;
        }
        if let Ok(mut data) = self.data.write() {
            for (path, content) in snapshot {
                data.insert(path.clone(), content.clone());
            }
        }
    }

    /// Take a point-in-time snapshot of the current workspace store.
    pub fn snapshot(&self) -> std::collections::HashMap<String, String> {
        self.data
            .read()
            .map(|data| data.clone())
            .unwrap_or_default()
    }

    /// Append a text frame to a JSON queue stored at `path`.
    ///
    /// The queue is stored as a JSON array of strings and bounded to the most
    /// recent `max_items` entries so websocket runtimes cannot grow it without
    /// limit.
    pub fn append_json_text_queue(
        &self,
        path: &str,
        text: &str,
        max_items: usize,
    ) -> Result<(), String> {
        let mut data = self
            .data
            .write()
            .map_err(|_| "workspace store lock poisoned".to_string())?;

        let mut queue: Vec<String> = data
            .get(path)
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();

        queue.push(text.to_string());

        if queue.len() > max_items {
            let overflow = queue.len() - max_items;
            queue.drain(0..overflow);
        }

        let serialized = serde_json::to_string(&queue)
            .map_err(|error| format!("failed to serialize websocket queue: {error}"))?;

        data.insert(path.to_string(), serialized);
        Ok(())
    }

    /// Atomically move a queued JSON array of text frames from `source_path` to `dest_path`.
    pub fn move_json_text_queue(&self, source_path: &str, dest_path: &str) -> Result<bool, String> {
        let mut data = self
            .data
            .write()
            .map_err(|_| "workspace store lock poisoned".to_string())?;

        let Some(raw_queue) = data.remove(source_path) else {
            data.remove(dest_path);
            return Ok(false);
        };

        let queue: Vec<String> = serde_json::from_str(&raw_queue)
            .map_err(|error| format!("failed to deserialize websocket queue: {error}"))?;

        if queue.is_empty() {
            data.remove(dest_path);
            return Ok(false);
        }

        data.insert(dest_path.to_string(), raw_queue);
        Ok(true)
    }
}

impl crate::tools::wasm::WorkspaceReader for ChannelWorkspaceStore {
    fn read(&self, path: &str) -> Option<String> {
        self.data.read().ok()?.get(path).cloned()
    }
}

/// Rate limiter for channel message emission.
///
/// Tracks emission rates across multiple executions.
pub struct ChannelEmitRateLimiter {
    config: EmitRateLimitConfig,
    minute_window: RateWindow,
    hour_window: RateWindow,
}

struct RateWindow {
    count: u32,
    window_start: u64,
    window_duration_ms: u64,
}

impl RateWindow {
    fn new(duration_ms: u64) -> Self {
        Self {
            count: 0,
            window_start: 0,
            window_duration_ms: duration_ms,
        }
    }

    fn check_and_record(&mut self, now_ms: u64, limit: u32) -> bool {
        // Reset window if expired
        if now_ms.saturating_sub(self.window_start) > self.window_duration_ms {
            self.count = 0;
            self.window_start = now_ms;
        }

        if self.count >= limit {
            return false;
        }

        self.count += 1;
        true
    }
}

#[allow(dead_code)]
impl ChannelEmitRateLimiter {
    /// Create a new rate limiter with the given config.
    pub fn new(config: EmitRateLimitConfig) -> Self {
        Self {
            config,
            minute_window: RateWindow::new(60_000), // 1 minute
            hour_window: RateWindow::new(3_600_000), // 1 hour
        }
    }

    /// Check if an emit is allowed and record it if so.
    ///
    /// Returns true if the emit is allowed, false if rate limited.
    pub fn check_and_record(&mut self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Check both windows
        let minute_ok = self
            .minute_window
            .check_and_record(now, self.config.messages_per_minute);
        let hour_ok = self
            .hour_window
            .check_and_record(now, self.config.messages_per_hour);

        minute_ok && hour_ok
    }

    /// Get the current emission count for the minute window.
    pub fn minute_count(&self) -> u32 {
        self.minute_window.count
    }

    /// Get the current emission count for the hour window.
    pub fn hour_count(&self) -> u32 {
        self.hour_window.count
    }
}

#[cfg(test)]
mod tests {
    use crate::channels::wasm::capabilities::{ChannelCapabilities, EmitRateLimitConfig};
    use crate::channels::wasm::host::{
        Attachment, ChannelEmitRateLimiter, ChannelHostState, EmittedMessage,
        MAX_ATTACHMENT_TOTAL_SIZE, MAX_ATTACHMENTS_PER_MESSAGE, MAX_EMITS_PER_EXECUTION,
        MAX_MESSAGE_CONTENT_SIZE,
    };

    #[test]
    fn test_emit_message_basic() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user123", "Hello, world!");
        state.emit_message(msg).unwrap();

        assert_eq!(state.emitted_count(), 1);

        let messages = state.take_emitted_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].user_id, "user123");
        assert_eq!(messages[0].content, "Hello, world!");

        // Queue should be cleared
        assert_eq!(state.emitted_count(), 0);
    }

    #[test]
    fn test_emit_message_with_metadata() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user123", "Hello")
            .with_user_name("John Doe")
            .with_thread_id("thread-1")
            .with_metadata(r#"{"key": "value"}"#);

        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        assert_eq!(messages[0].user_name, Some("John Doe".to_string()));
        assert_eq!(messages[0].thread_id, Some("thread-1".to_string()));
        assert_eq!(messages[0].metadata_json, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_emit_per_execution_limit() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        // Fill up to limit
        for i in 0..MAX_EMITS_PER_EXECUTION {
            let msg = EmittedMessage::new("user", format!("Message {}", i));
            state.emit_message(msg).unwrap();
        }

        // This should be dropped silently
        let msg = EmittedMessage::new("user", "Should be dropped");
        state.emit_message(msg).unwrap();

        assert_eq!(state.emitted_count(), MAX_EMITS_PER_EXECUTION);
        assert_eq!(state.emits_dropped(), 1);
    }

    #[test]
    fn test_emit_message_truncates_utf8_safely() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let prefix = "a".repeat(MAX_MESSAGE_CONTENT_SIZE - 1);
        let content = format!("{}🙂suffix", prefix);
        let msg = EmittedMessage::new("user123", content);

        state.emit_message(msg).unwrap();
        let messages = state.take_emitted_messages();
        assert_eq!(messages.len(), 1);

        let emitted = &messages[0].content;
        assert!(emitted.starts_with(&prefix));
        assert!(emitted.ends_with("... (truncated)"));
        assert!(!emitted.contains("🙂"));
    }

    #[test]
    fn test_workspace_write_prefixing() {
        let caps = ChannelCapabilities::for_channel("slack");
        let mut state = ChannelHostState::new("slack", caps);

        state
            .workspace_write("state.json", "{}".to_string())
            .unwrap();

        let writes = state.take_pending_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "channels/slack/state.json");
    }

    #[test]
    fn test_workspace_write_path_traversal_blocked() {
        let caps = ChannelCapabilities::for_channel("slack");
        let mut state = ChannelHostState::new("slack", caps);

        // Try to escape namespace
        let result = state.workspace_write("../secrets.json", "{}".to_string());
        assert!(result.is_err());

        // Absolute path
        let result = state.workspace_write("/etc/passwd", "{}".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_rate_limiter_basic() {
        let config = EmitRateLimitConfig {
            messages_per_minute: 10,
            messages_per_hour: 100,
        };
        let mut limiter = ChannelEmitRateLimiter::new(config);

        // Should allow 10 messages
        for _ in 0..10 {
            assert!(limiter.check_and_record());
        }

        // 11th should be blocked
        assert!(!limiter.check_and_record());
    }

    #[test]
    fn test_channel_name() {
        let caps = ChannelCapabilities::for_channel("telegram");
        let state = ChannelHostState::new("telegram", caps);

        assert_eq!(state.channel_name(), "telegram");
    }

    #[test]
    fn test_channel_workspace_store_commit_and_read() {
        use crate::channels::wasm::host::{ChannelWorkspaceStore, PendingWorkspaceWrite};
        use crate::tools::wasm::WorkspaceReader;

        let store = ChannelWorkspaceStore::new();

        // Initially empty
        assert!(store.read("channels/telegram/offset").is_none());

        // Commit some writes
        let writes = vec![
            PendingWorkspaceWrite {
                path: "channels/telegram/offset".to_string(),
                content: "103".to_string(),
            },
            PendingWorkspaceWrite {
                path: "channels/telegram/state.json".to_string(),
                content: r#"{"ok":true}"#.to_string(),
            },
        ];
        store.commit_writes(&writes);

        // Should be readable
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("103".to_string())
        );
        assert_eq!(
            store.read("channels/telegram/state.json"),
            Some(r#"{"ok":true}"#.to_string())
        );

        // Overwrite a value
        let writes2 = vec![PendingWorkspaceWrite {
            path: "channels/telegram/offset".to_string(),
            content: "200".to_string(),
        }];
        store.commit_writes(&writes2);
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("200".to_string())
        );

        // Empty writes are a no-op
        store.commit_writes(&[]);
        assert_eq!(
            store.read("channels/telegram/offset"),
            Some("200".to_string())
        );
    }

    #[test]
    fn test_channel_workspace_store_append_json_text_queue_is_bounded() {
        use crate::channels::wasm::host::ChannelWorkspaceStore;
        use crate::tools::wasm::WorkspaceReader;

        let store = ChannelWorkspaceStore::new();
        let path = "channels/discord/state/gateway_event_queue";

        store.append_json_text_queue(path, "frame-1", 2).unwrap();
        store.append_json_text_queue(path, "frame-2", 2).unwrap();
        store.append_json_text_queue(path, "frame-3", 2).unwrap();

        let queue: Vec<String> = serde_json::from_str(&store.read(path).unwrap()).unwrap();
        assert_eq!(queue, vec!["frame-2".to_string(), "frame-3".to_string()]);
    }

    #[test]
    fn test_channel_workspace_store_move_json_text_queue_is_atomic() {
        use crate::channels::wasm::host::ChannelWorkspaceStore;
        use crate::tools::wasm::WorkspaceReader;

        let store = ChannelWorkspaceStore::new();
        let live_path = "channels/discord/state/gateway_event_queue";
        let drain_path = "channels/discord/state/gateway_event_queue_processing";

        store
            .append_json_text_queue(live_path, "frame-1", 4)
            .unwrap();
        store
            .append_json_text_queue(live_path, "frame-2", 4)
            .unwrap();

        assert!(store.move_json_text_queue(live_path, drain_path).unwrap());
        assert_eq!(store.read(live_path), None);

        let drained: Vec<String> = serde_json::from_str(&store.read(drain_path).unwrap()).unwrap();
        assert_eq!(drained, vec!["frame-1".to_string(), "frame-2".to_string()]);

        store
            .append_json_text_queue(live_path, "frame-3", 4)
            .unwrap();
        let live: Vec<String> = serde_json::from_str(&store.read(live_path).unwrap()).unwrap();
        assert_eq!(live, vec!["frame-3".to_string()]);
    }

    // === QA Plan P2 - 2.3: WASM channel lifecycle tests ===

    #[test]
    fn test_workspace_write_then_read_round_trip() {
        // Full lifecycle: write in one "callback", commit, then read in a
        // subsequent "callback" using the same store as the workspace reader.
        use crate::channels::wasm::host::ChannelWorkspaceStore;
        use crate::tools::wasm::{WorkspaceCapability, WorkspaceReader};
        use std::sync::Arc;

        let store = Arc::new(ChannelWorkspaceStore::new());

        // --- Callback 1: write workspace data ---
        let caps = ChannelCapabilities::for_channel("telegram");
        let mut state = ChannelHostState::new("telegram", caps);

        state
            .workspace_write("offset", "12345".to_string())
            .unwrap();
        state
            .workspace_write("state.json", r#"{"ok":true}"#.to_string())
            .unwrap();

        let writes = state.take_pending_writes();
        assert_eq!(writes.len(), 2);
        store.commit_writes(&writes);

        // --- Callback 2: read back the data written in callback 1 ---
        // Build capabilities with the store as the workspace reader.
        let mut caps2 = ChannelCapabilities::for_channel("telegram");
        caps2.tool_capabilities.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: vec![], // empty = all paths allowed
            reader: Some(Arc::clone(&store) as Arc<dyn WorkspaceReader>),
        });
        let state2 = ChannelHostState::new("telegram", caps2);

        // workspace_read prefixes path with "channels/telegram/" before delegating.
        let offset = state2.workspace_read("offset").unwrap();
        assert_eq!(offset, Some("12345".to_string()));

        let json = state2.workspace_read("state.json").unwrap();
        assert_eq!(json, Some(r#"{"ok":true}"#.to_string()));

        // Non-existent key returns None.
        let missing = state2.workspace_read("no_such_key").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_workspace_overwrite_across_callbacks() {
        // Verify that a second write to the same key overwrites the first.
        use crate::channels::wasm::host::ChannelWorkspaceStore;
        use crate::tools::wasm::{WorkspaceCapability, WorkspaceReader};
        use std::sync::Arc;

        let store = Arc::new(ChannelWorkspaceStore::new());

        // Callback 1: write initial value.
        let caps = ChannelCapabilities::for_channel("slack");
        let mut state = ChannelHostState::new("slack", caps);
        state.workspace_write("cursor", "100".to_string()).unwrap();
        let writes = state.take_pending_writes();
        store.commit_writes(&writes);

        // Callback 2: overwrite the same key.
        let caps2 = ChannelCapabilities::for_channel("slack");
        let mut state2 = ChannelHostState::new("slack", caps2);
        state2.workspace_write("cursor", "200".to_string()).unwrap();
        let writes2 = state2.take_pending_writes();
        store.commit_writes(&writes2);

        // Callback 3: read back -- should see the overwritten value.
        let mut caps3 = ChannelCapabilities::for_channel("slack");
        caps3.tool_capabilities.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: vec![],
            reader: Some(Arc::clone(&store) as Arc<dyn WorkspaceReader>),
        });
        let state3 = ChannelHostState::new("slack", caps3);

        let value = state3.workspace_read("cursor").unwrap();
        assert_eq!(value, Some("200".to_string()));
    }

    #[test]
    fn test_emit_and_take_preserves_order_and_content() {
        // Emit multiple messages, take them, verify order and content.
        let caps = ChannelCapabilities::for_channel("discord");
        let mut state = ChannelHostState::new("discord", caps);

        let messages_data = vec![
            ("user-a", "Hello from A"),
            ("user-b", "Hello from B"),
            ("user-a", "Follow-up from A"),
        ];
        for (uid, content) in &messages_data {
            state
                .emit_message(EmittedMessage::new(*uid, *content))
                .unwrap();
        }

        assert_eq!(state.emitted_count(), 3);

        let taken = state.take_emitted_messages();
        assert_eq!(taken.len(), 3);

        // Order preserved.
        for (i, (uid, content)) in messages_data.iter().enumerate() {
            assert_eq!(taken[i].user_id, *uid);
            assert_eq!(taken[i].content, *content);
        }

        // Take empties the queue.
        assert_eq!(state.emitted_count(), 0);
        let taken2 = state.take_emitted_messages();
        assert!(taken2.is_empty());
    }

    #[test]
    fn test_channels_have_isolated_namespaces() {
        // Two channels writing to the same relative path should not collide.
        use crate::channels::wasm::host::ChannelWorkspaceStore;
        use crate::tools::wasm::{WorkspaceCapability, WorkspaceReader};
        use std::sync::Arc;

        let store = Arc::new(ChannelWorkspaceStore::new());

        // Telegram writes "offset" = "100".
        let caps_tg = ChannelCapabilities::for_channel("telegram");
        let mut state_tg = ChannelHostState::new("telegram", caps_tg);
        state_tg
            .workspace_write("offset", "100".to_string())
            .unwrap();
        store.commit_writes(&state_tg.take_pending_writes());

        // Slack writes "offset" = "200".
        let caps_sl = ChannelCapabilities::for_channel("slack");
        let mut state_sl = ChannelHostState::new("slack", caps_sl);
        state_sl
            .workspace_write("offset", "200".to_string())
            .unwrap();
        store.commit_writes(&state_sl.take_pending_writes());

        // Reading back: each channel sees its own value.
        let mut caps_tg_read = ChannelCapabilities::for_channel("telegram");
        caps_tg_read.tool_capabilities.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: vec![],
            reader: Some(Arc::clone(&store) as Arc<dyn WorkspaceReader>),
        });
        let tg_reader = ChannelHostState::new("telegram", caps_tg_read);
        assert_eq!(
            tg_reader.workspace_read("offset").unwrap(),
            Some("100".to_string())
        );

        let mut caps_sl_read = ChannelCapabilities::for_channel("slack");
        caps_sl_read.tool_capabilities.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: vec![],
            reader: Some(Arc::clone(&store) as Arc<dyn WorkspaceReader>),
        });
        let sl_reader = ChannelHostState::new("slack", caps_sl_read);
        assert_eq!(
            sl_reader.workspace_read("offset").unwrap(),
            Some("200".to_string())
        );
    }

    // === Attachment validation tests ===

    fn make_attachment(id: &str, mime: &str, size: Option<u64>) -> Attachment {
        Attachment {
            id: id.to_string(),
            mime_type: mime.to_string(),
            filename: None,
            size_bytes: size,
            source_url: None,
            storage_key: None,
            local_path: None,
            extracted_text: None,
            extras_json: String::new(),
            data: Vec::new(),
            duration_secs: None,
        }
    }

    #[test]
    fn test_emit_message_with_attachments() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user1", "Check this image")
            .with_attachments(vec![make_attachment("file1", "image/jpeg", Some(1024))]);

        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].attachments.len(), 1);
        assert_eq!(messages[0].attachments[0].id, "file1");
        assert_eq!(messages[0].attachments[0].mime_type, "image/jpeg");
        assert_eq!(messages[0].attachments[0].size_bytes, Some(1024));
    }

    #[test]
    fn test_emit_message_no_attachments_backward_compat() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user1", "Just text");
        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].attachments.is_empty());
    }

    #[test]
    fn test_attachment_count_limit() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let attachments: Vec<Attachment> = (0..MAX_ATTACHMENTS_PER_MESSAGE + 5)
            .map(|i| make_attachment(&format!("file{}", i), "image/png", Some(100)))
            .collect();

        let msg = EmittedMessage::new("user1", "Many files").with_attachments(attachments);
        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        assert_eq!(messages[0].attachments.len(), MAX_ATTACHMENTS_PER_MESSAGE);
    }

    #[test]
    fn test_attachment_total_size_limit() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        // Each file is 1/3 of the limit, so 3 fit but 4th does not
        let chunk_size = MAX_ATTACHMENT_TOTAL_SIZE / 3;
        let attachments = vec![
            make_attachment("file1", "image/png", Some(chunk_size)),
            make_attachment("file2", "image/png", Some(chunk_size)),
            make_attachment("file3", "image/png", Some(chunk_size)),
            make_attachment("file4", "image/png", Some(chunk_size)),
        ];

        let msg = EmittedMessage::new("user1", "Big files").with_attachments(attachments);
        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        // Only first 3 fit within the total size limit
        assert_eq!(messages[0].attachments.len(), 3);
    }

    #[test]
    fn test_attachment_mime_type_filtering() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let attachments = vec![
            make_attachment("ok1", "image/jpeg", Some(100)),
            make_attachment("bad1", "application/x-executable", Some(100)),
            make_attachment("ok2", "application/pdf", Some(100)),
            make_attachment("bad2", "application/x-msdos-program", Some(100)),
            make_attachment("ok3", "text/plain", Some(100)),
            make_attachment("ok4", "audio/mpeg", Some(100)),
            make_attachment("ok5", "video/mp4", Some(100)),
        ];

        let msg = EmittedMessage::new("user1", "Mixed files").with_attachments(attachments);
        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        let ids: Vec<&str> = messages[0]
            .attachments
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(ids, vec!["ok1", "ok2", "ok3", "ok4", "ok5"]);
    }

    #[test]
    fn test_attachment_unknown_size_allowed() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let attachments = vec![
            make_attachment("file1", "image/jpeg", None),
            make_attachment("file2", "image/png", None),
        ];

        let msg = EmittedMessage::new("user1", "No sizes").with_attachments(attachments);
        state.emit_message(msg).unwrap();

        let messages = state.take_emitted_messages();
        assert_eq!(messages[0].attachments.len(), 2);
    }
}
