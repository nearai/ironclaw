use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;

/// Metadata stored alongside a pending async tool result.
#[derive(Debug, Clone)]
pub struct CallbackMetadata {
    pub tool_name: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    pub channel: String,
}

/// Error type for callback resolution.
#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error("unknown correlation ID: {0}")]
    UnknownCorrelationId(String),

    #[error("failed to inject message: {0}")]
    InjectionFailed(String),
}

/// Internal entry with timestamp for TTL expiry.
#[derive(Debug)]
struct PendingEntry {
    metadata: CallbackMetadata,
    registered_at: Instant,
}

/// Registry for async tool results. Tools register a correlation ID when
/// returning a pending result; external backends call resolve() when the
/// result arrives, which injects an IncomingMessage into the channel system.
pub struct ToolCallbackRegistry {
    pending: tokio::sync::RwLock<HashMap<String, PendingEntry>>,
    ttl: Duration,
}

impl ToolCallbackRegistry {
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending: tokio::sync::RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Register a pending async tool result.
    pub async fn register(&self, correlation_id: String, metadata: CallbackMetadata) {
        let entry = PendingEntry {
            metadata,
            registered_at: Instant::now(),
        };
        self.pending.write().await.insert(correlation_id, entry);
    }

    /// Check if a correlation ID is pending.
    pub async fn is_pending(&self, correlation_id: &str) -> bool {
        self.pending.read().await.contains_key(correlation_id)
    }

    /// Cancel a pending result (cleanup).
    pub async fn cancel(&self, correlation_id: &str) {
        self.pending.write().await.remove(correlation_id);
    }

    /// Returns the configured TTL.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Resolve a pending result, injecting an `IncomingMessage` into the channel system.
    /// Removes the entry from the pending map on success.
    pub async fn resolve(
        &self,
        correlation_id: &str,
        result: String,
        inject_tx: &mpsc::Sender<IncomingMessage>,
    ) -> Result<(), CallbackError> {
        let entry = self
            .pending
            .write()
            .await
            .remove(correlation_id)
            .ok_or_else(|| CallbackError::UnknownCorrelationId(correlation_id.to_string()))?;

        let mut message =
            IncomingMessage::new(entry.metadata.channel, entry.metadata.user_id, result)
                .into_internal();

        if let Some(tid) = entry.metadata.thread_id {
            message = message.with_thread(tid);
        }

        inject_tx
            .send(message)
            .await
            .map_err(|e: mpsc::error::SendError<IncomingMessage>| {
                CallbackError::InjectionFailed(e.to_string())
            })
    }

    /// Remove expired entries and inject timeout messages.
    /// Returns the number of expired entries swept.
    pub async fn sweep_expired(&self, inject_tx: &mpsc::Sender<IncomingMessage>) -> usize {
        let expired: Vec<(String, PendingEntry)> = {
            let mut pending = self.pending.write().await;
            let now = Instant::now();
            let expired_ids: Vec<String> = pending
                .iter()
                .filter(|(_, entry)| now.duration_since(entry.registered_at) >= self.ttl)
                .map(|(id, _)| id.clone())
                .collect();

            expired_ids
                .iter()
                .filter_map(|id| pending.remove(id).map(|entry| (id.clone(), entry)))
                .collect()
        };

        let count = expired.len();
        for (correlation_id, entry) in expired {
            let content = format!(
                "Transaction {}: timed out waiting for approval (tool: {})",
                correlation_id, entry.metadata.tool_name
            );
            let mut message =
                IncomingMessage::new(entry.metadata.channel, entry.metadata.user_id, content)
                    .into_internal();

            if let Some(tid) = entry.metadata.thread_id {
                message = message.with_thread(tid);
            }

            if let Err(e) = inject_tx.send(message).await {
                tracing::warn!(correlation_id, "failed to inject timeout message: {}", e);
            }
        }
        count
    }

    /// Spawn a background task that periodically sweeps expired entries.
    /// Returns a JoinHandle that can be used to abort the task.
    pub fn start_sweep_task(
        self: &std::sync::Arc<Self>,
        inject_tx: mpsc::Sender<IncomingMessage>,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let registry = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let count = registry.sweep_expired(&inject_tx).await;
                if count > 0 {
                    tracing::info!(count, "swept expired callback entries");
                }
            }
        })
    }
}
