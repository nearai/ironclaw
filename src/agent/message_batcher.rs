//! Message batching (debouncing) for rapid inbound messages.
//!
//! When users send multiple rapid messages (especially on chat channels like
//! WhatsApp), this component collects them into batches before processing.
//! This prevents fragmented agent responses and wasted tokens.
//!
//! # Per-channel Configuration
//!
//! Batching behavior is determined per-channel using `BatchingConfig::for_channel()`:
//! - WhatsApp: 5s window, 5 messages max
//! - Web: 2s window, 5 messages max (shorter for real-time feel)
//! - CLI/REPL: Disabled (instant REPL experience)
//! - Unknown channels: Default (5s, 5 messages)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, trace};

use crate::channels::IncomingMessage;

/// Configuration for message batching per channel.
#[derive(Debug, Clone)]
pub struct BatchingConfig {
    /// Whether batching is enabled.
    pub enabled: bool,
    /// Time window to wait for additional messages (milliseconds).
    pub window_ms: u64,
    /// Maximum messages to batch before forced flush.
    pub max_messages: usize,
}

impl Default for BatchingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_ms: 5000,
            max_messages: 5,
        }
    }
}

impl BatchingConfig {
    /// Configuration optimized for WhatsApp (async, chat-like).
    pub fn whatsapp() -> Self {
        Self {
            enabled: true,
            window_ms: 5000,
            max_messages: 5,
        }
    }

    /// Configuration for web gateway (shorter for real-time feel).
    pub fn web() -> Self {
        Self {
            enabled: true,
            window_ms: 2000,
            max_messages: 5,
        }
    }

    /// Configuration for CLI (no batching, instant REPL).
    pub fn cli() -> Self {
        Self {
            enabled: false,
            window_ms: 0,
            max_messages: 1,
        }
    }

    /// Get config for a given channel name.
    pub fn for_channel(channel: &str) -> Self {
        match channel {
            "whatsapp" => Self::whatsapp(),
            "web" => Self::web(),
            "cli" | "repl" => Self::cli(),
            // Default to WhatsApp-like behavior for unknown chat channels
            _ => Self::default(),
        }
    }
}

/// Pending batch of messages waiting to be processed.
#[derive(Debug)]
struct PendingBatch {
    messages: Vec<IncomingMessage>,
    /// Configuration for this batch (determined by first message's channel).
    config: BatchingConfig,
}

/// Key for identifying a unique conversation batch.
/// Includes thread_id to prevent cross-conversation data leakage.
type BatchKey = (String, String, Option<String>); // (user_id, channel, thread_id)

/// Handle for a pending timer task.
struct TimerHandle {
    /// The task that will flush the batch when the timer expires.
    handle: JoinHandle<()>,
}

/// Batches rapid inbound messages into combined turns.
///
/// Uses a broadcast channel to notify subscribers when batches are ready.
/// Each (user_id, channel, thread_id) tuple has its own independent batch.
/// Batching configuration is determined per-channel using `BatchingConfig::for_channel()`.
pub struct MessageBatcher {
    /// Pending batches keyed by (user_id, channel, thread_id).
    pending: Arc<Mutex<HashMap<BatchKey, PendingBatch>>>,
    /// Timer tasks for each batch.
    timers: Arc<Mutex<HashMap<BatchKey, TimerHandle>>>,
    /// Output channel for flushed batches.
    output_tx: broadcast::Sender<IncomingMessage>,
}

impl MessageBatcher {
    /// Create a new message batcher.
    ///
    /// Batching configuration is determined per-channel based on incoming messages.
    pub fn new() -> Self {
        let (output_tx, _) = broadcast::channel(64);

        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            timers: Arc::new(Mutex::new(HashMap::new())),
            output_tx,
        }
    }

    /// Get a subscriber for flushed batches.
    ///
    /// The subscriber will receive merged messages when batches are flushed
    /// (either by timer or by reaching max_messages).
    pub fn subscribe(&self) -> broadcast::Receiver<IncomingMessage> {
        self.output_tx.subscribe()
    }

    /// Add a message to the batch.
    ///
    /// Batching behavior is determined by the channel using `BatchingConfig::for_channel()`.
    /// If batching is disabled for the channel, the message is immediately sent to subscribers.
    /// Otherwise, it's added to the pending batch for the (user_id, channel, thread_id) tuple.
    pub async fn push(&self, message: IncomingMessage) {
        let config = BatchingConfig::for_channel(&message.channel);

        if !config.enabled {
            // Batching disabled for this channel - pass through immediately
            let _ = self.output_tx.send(message);
            return;
        }

        let key = (
            message.user_id.clone(),
            message.channel.clone(),
            message.thread_id.clone(),
        );

        // Check for existing batch
        let mut pending = self.pending.lock().await;
        if let Some(batch) = pending.get_mut(&key) {
            batch.messages.push(message);

            // Check if batch is full (use batch's config for consistency)
            if batch.messages.len() >= batch.config.max_messages {
                trace!(
                    user_id = %key.0,
                    channel = %key.1,
                    thread_id = ?key.2,
                    count = batch.messages.len(),
                    "Batch full, flushing"
                );
                self.flush_batch_locked(&mut pending, &key).await;
            }
        } else {
            // Start new batch
            trace!(
                user_id = %key.0,
                channel = %key.1,
                thread_id = ?key.2,
                "Starting new batch"
            );
            pending.insert(
                key.clone(),
                PendingBatch {
                    messages: vec![message],
                    config: config.clone(),
                },
            );

            // Drop the lock before spawning the timer task
            drop(pending);

            // Start timer for this batch
            self.start_timer(key, config).await;
        }
    }

    /// Start a timer task for a batch.
    async fn start_timer(&self, key: BatchKey, config: BatchingConfig) {
        let pending = Arc::clone(&self.pending);
        let timers = Arc::clone(&self.timers);
        let output_tx = self.output_tx.clone();
        let window = Duration::from_millis(config.window_ms);
        let key_clone = key.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(window).await;

            // Timer expired - flush and send
            trace!(
                user_id = %key_clone.0,
                channel = %key_clone.1,
                thread_id = ?key_clone.2,
                "Batch timer expired"
            );

            // Lock and flush the batch
            let mut p = pending.lock().await;
            let mut t = timers.lock().await;

            if let Some(batch) = p.remove(&key_clone) {
                // Merge and send
                if let Some(merged) = Self::merge_batch(&batch) {
                    debug!(
                        user_id = %key_clone.0,
                        channel = %key_clone.1,
                        thread_id = ?key_clone.2,
                        count = batch.messages.len(),
                        "Timer expired, sending merged batch"
                    );
                    let _ = output_tx.send(merged);
                }
            }
            t.remove(&key_clone);
        });

        let mut timers = self.timers.lock().await;
        timers.insert(key, TimerHandle { handle });
    }

    /// Flush a specific batch (must hold pending lock).
    async fn flush_batch_locked(
        &self,
        pending: &mut HashMap<BatchKey, PendingBatch>,
        key: &BatchKey,
    ) {
        // Cancel timer if exists
        let mut timers = self.timers.lock().await;
        if let Some(timer) = timers.remove(key) {
            timer.handle.abort();
        }
        drop(timers);

        if let Some(batch) = pending.remove(key)
            && let Some(merged) = Self::merge_batch(&batch)
        {
            debug!(
                user_id = %key.0,
                channel = %key.1,
                thread_id = ?key.2,
                count = batch.messages.len(),
                "Flushing batch"
            );
            let _ = self.output_tx.send(merged);
        }
    }

    /// Flush a batch by key (public interface).
    pub async fn flush(&self, key: &BatchKey) {
        let mut pending = self.pending.lock().await;
        self.flush_batch_locked(&mut pending, key).await;
    }

    /// Flush all pending batches immediately.
    pub async fn flush_all(&self) {
        let mut pending = self.pending.lock().await;
        let keys: Vec<BatchKey> = pending.keys().cloned().collect();

        for key in keys {
            self.flush_batch_locked(&mut pending, &key).await;
        }
    }

    /// Gracefully shutdown the batcher, aborting all pending timers.
    ///
    /// This should be called before the MessageBatcher is dropped to ensure
    /// all timer tasks are properly cleaned up.
    pub async fn shutdown(&self) {
        let mut timers = self.timers.lock().await;
        for (_, timer) in timers.drain() {
            timer.handle.abort();
        }
    }

    /// Merge a batch into a single message.
    fn merge_batch(batch: &PendingBatch) -> Option<IncomingMessage> {
        if batch.messages.is_empty() {
            return None;
        }

        let first = &batch.messages[0];

        // Join message contents with double newlines
        let content = batch
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Create merged message using the first message's metadata
        Some(IncomingMessage {
            id: uuid::Uuid::new_v4(),
            channel: first.channel.clone(),
            user_id: first.user_id.clone(),
            user_name: first.user_name.clone(),
            content,
            thread_id: first.thread_id.clone(),
            received_at: first.received_at,
            metadata: first.metadata.clone(),
        })
    }

    /// Get the number of pending batches.
    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Check if there's a pending batch for a specific key.
    pub async fn has_pending(&self, key: &BatchKey) -> bool {
        self.pending.lock().await.contains_key(key)
    }
}

impl Default for MessageBatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_message(content: &str) -> IncomingMessage {
        IncomingMessage {
            id: uuid::Uuid::new_v4(),
            channel: "test".to_string(),
            user_id: "test_user".to_string(),
            user_name: None,
            content: content.to_string(),
            thread_id: None,
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn test_message_with_thread(content: &str, thread_id: Option<&str>) -> IncomingMessage {
        IncomingMessage {
            id: uuid::Uuid::new_v4(),
            channel: "test".to_string(),
            user_id: "test_user".to_string(),
            user_name: None,
            content: content.to_string(),
            thread_id: thread_id.map(|s| s.to_string()),
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn test_batching_disabled() {
        let batcher = MessageBatcher::new();
        let mut rx = batcher.subscribe();

        // CLI channel has batching disabled
        let mut msg = test_message("hello");
        msg.channel = "cli".to_string();
        batcher.push(msg).await;

        // Should receive immediately
        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;

        assert!(received.is_ok());
    }

    #[tokio::test]
    async fn test_batching_merges_messages() {
        let batcher = MessageBatcher::new();
        let mut rx = batcher.subscribe();

        // Push messages (test channel uses default config with max_messages=5)
        batcher.push(test_message("msg1")).await;
        batcher.push(test_message("msg2")).await;
        batcher.push(test_message("msg3")).await;
        batcher.push(test_message("msg4")).await;
        batcher.push(test_message("msg5")).await;

        // Should flush immediately due to max_messages
        let received = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;

        assert!(received.is_ok());
        let merged = received.unwrap().unwrap();
        assert_eq!(merged.content, "msg1\n\nmsg2\n\nmsg3\n\nmsg4\n\nmsg5");
    }

    #[tokio::test]
    async fn test_timer_flush() {
        let batcher = MessageBatcher::new();
        let mut rx = batcher.subscribe();

        // Push a single message (test channel has 5s window, we'll wait shorter)
        batcher.push(test_message("delayed")).await;

        // Should not receive immediately
        let immediate = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await;
        assert!(immediate.is_err());

        // Flush manually
        batcher.flush_all().await;

        // Should receive after flush
        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;

        assert!(received.is_ok());
    }

    #[tokio::test]
    async fn test_flush_all() {
        let batcher = MessageBatcher::new();
        let mut rx = batcher.subscribe();

        // Push messages for different users
        batcher.push(test_message("user1_msg")).await;

        let mut user2_msg = test_message("user2_msg");
        user2_msg.user_id = "user2".to_string();
        batcher.push(user2_msg).await;

        // Flush all
        batcher.flush_all().await;

        // Should receive both messages
        let mut received_messages = Vec::new();
        for _ in 0..2 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(msg)) => received_messages.push(msg.content),
                _ => break,
            }
        }

        assert_eq!(
            received_messages.len(),
            2,
            "Expected to receive two messages"
        );
        assert!(
            received_messages.contains(&"user1_msg".to_string()),
            "Message from user1 not found"
        );
        assert!(
            received_messages.contains(&"user2_msg".to_string()),
            "Message from user2 not found"
        );
    }

    #[tokio::test]
    async fn test_thread_isolation() {
        let batcher = MessageBatcher::new();
        let mut rx = batcher.subscribe();

        // Push messages for same user/channel but different threads
        batcher
            .push(test_message_with_thread("thread1_msg1", Some("thread1")))
            .await;
        batcher
            .push(test_message_with_thread("thread1_msg2", Some("thread1")))
            .await;
        batcher
            .push(test_message_with_thread("thread2_msg1", Some("thread2")))
            .await;
        batcher
            .push(test_message_with_thread("thread2_msg2", Some("thread2")))
            .await;

        // Flush all
        batcher.flush_all().await;

        // Should receive two separate batches (one per thread)
        let mut received_messages = Vec::new();
        for _ in 0..2 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(msg)) => received_messages.push(msg),
                _ => break,
            }
        }

        assert_eq!(received_messages.len(), 2, "Expected two separate batches");

        // Each batch should contain messages from only one thread
        for msg in &received_messages {
            if msg.content.contains("thread1") {
                assert!(
                    !msg.content.contains("thread2"),
                    "Thread 1 batch should not contain thread 2 messages"
                );
                assert!(
                    msg.content.contains("thread1_msg1"),
                    "Thread 1 batch should contain msg1"
                );
                assert!(
                    msg.content.contains("thread1_msg2"),
                    "Thread 1 batch should contain msg2"
                );
            } else if msg.content.contains("thread2") {
                assert!(
                    !msg.content.contains("thread1"),
                    "Thread 2 batch should not contain thread 1 messages"
                );
                assert!(
                    msg.content.contains("thread2_msg1"),
                    "Thread 2 batch should contain msg1"
                );
                assert!(
                    msg.content.contains("thread2_msg2"),
                    "Thread 2 batch should contain msg2"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_per_channel_config() {
        let batcher = MessageBatcher::new();
        let mut rx = batcher.subscribe();

        // CLI should pass through immediately (no batching)
        let mut cli_msg = test_message("cli_message");
        cli_msg.channel = "cli".to_string();
        batcher.push(cli_msg).await;

        // Should receive immediately
        let received = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(received.is_ok());
        let msg = received.unwrap().unwrap();
        assert_eq!(msg.content, "cli_message");
    }

    #[tokio::test]
    async fn test_shutdown_aborts_timers() {
        let batcher = MessageBatcher::new();

        // Push a message to start a timer
        batcher.push(test_message("test")).await;

        // Shutdown should complete without hanging
        let shutdown_result =
            tokio::time::timeout(Duration::from_millis(100), batcher.shutdown()).await;
        assert!(shutdown_result.is_ok(), "Shutdown should complete quickly");
    }
}
