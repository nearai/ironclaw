use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Metadata stored alongside a pending async tool result.
#[derive(Debug, Clone)]
pub struct CallbackMetadata {
    pub tool_name: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    pub channel: String,
}

/// Internal entry with timestamp for TTL expiry.
#[derive(Debug)]
struct PendingEntry {
    #[allow(dead_code)]
    metadata: CallbackMetadata,
    #[allow(dead_code)]
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
}
