//! Host state for WASM channel execution.
//!
//! Extends the base tool host state with channel-specific functionality:
//! - Message emission (queueing messages to send to the agent)
//! - Workspace write access (scoped to channel namespace)
//! - Rate limiting for message emission

use std::time::{SystemTime, UNIX_EPOCH};

use crate::channels::wasm::capabilities::{ChannelCapabilities, EmitRateLimitConfig};
use crate::channels::wasm::error::WasmChannelError;
use crate::tools::wasm::{HostState, LogLevel};

/// Maximum emitted messages per callback execution.
const MAX_EMITS_PER_EXECUTION: usize = 100;

/// Maximum message content size (64 KB).
const MAX_MESSAGE_CONTENT_SIZE: usize = 64 * 1024;

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

        // Validate message content size
        if msg.content.len() > MAX_MESSAGE_CONTENT_SIZE {
            tracing::warn!(
                channel = %self.channel_name,
                size = msg.content.len(),
                max = MAX_MESSAGE_CONTENT_SIZE,
                "Message content too large, truncating"
            );
            let mut truncated = msg.content[..MAX_MESSAGE_CONTENT_SIZE].to_string();
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
        ChannelEmitRateLimiter, ChannelHostState, EmittedMessage, MAX_EMITS_PER_EXECUTION,
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
}
