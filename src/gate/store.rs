//! Pending gate store — atomic, channel-verified, persistent.
//!
//! Uses a single [`Mutex`] (not `RwLock`) because every meaningful read
//! is followed by a mutation. This eliminates TOCTOU races by design.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::pending::{PendingGate, PendingGateKey, PendingGateView};

// ── Trusted channels ────────────────────────────────────────

/// Channels that may resolve gates created by any source channel.
/// The web gateway is trusted because it authenticates users server-side.
pub const TRUSTED_GATE_CHANNELS: &[&str] = &["web", "gateway"];

/// Channel names reserved by the system. WASM extensions cannot register
/// these names, preventing impersonation attacks.
pub const RESERVED_CHANNEL_NAMES: &[&str] = &[
    "web",
    "gateway",
    "telegram",
    "signal",
    "slack",
    "discord",
    "repl",
    "cli",
    "http",
    "__bootstrap__",
];

// ── Error type ──────────────────────────────────────────────

/// Errors from the pending gate store.
#[derive(Debug, thiserror::Error)]
pub enum GateStoreError {
    #[error("no pending gate for this thread")]
    NotFound,

    #[error("request ID mismatch (stale approval)")]
    RequestIdMismatch,

    #[error("channel '{actual}' cannot resolve gates from channel '{expected}'")]
    ChannelMismatch { expected: String, actual: String },

    #[error("pending gate has expired")]
    Expired,

    #[error("a gate is already pending for this thread")]
    AlreadyExists,

    #[error("persistence error: {reason}")]
    Persistence { reason: String },
}

// ── Persistence trait ───────────────────────────────────────

/// Backend for persisting pending gates across restarts.
#[async_trait]
pub trait GatePersistence: Send + Sync {
    async fn save(&self, gate: &PendingGate) -> Result<(), GateStoreError>;
    async fn remove(&self, key: &PendingGateKey) -> Result<(), GateStoreError>;
    async fn load_all(&self) -> Result<Vec<PendingGate>, GateStoreError>;
}

// ── Store ───────────────────────────────────────────────────

struct StoreInner {
    by_key: HashMap<PendingGateKey, PendingGate>,
    by_request_id: HashMap<Uuid, PendingGateKey>,
}

/// Thread-safe store for pending execution gates.
///
/// All mutations happen under a single [`Mutex`] lock to prevent TOCTOU
/// races. The `take_verified` method is the **only** way to retrieve a
/// pending gate for resolution — it atomically verifies request ID,
/// channel authorization, and expiry before removing the gate.
pub struct PendingGateStore {
    inner: Mutex<StoreInner>,
    persistence: Option<Arc<dyn GatePersistence>>,
}

impl PendingGateStore {
    /// Create a new store with optional persistence backend.
    pub fn new(persistence: Option<Arc<dyn GatePersistence>>) -> Self {
        Self {
            inner: Mutex::new(StoreInner {
                by_key: HashMap::new(),
                by_request_id: HashMap::new(),
            }),
            persistence,
        }
    }

    /// Create a store without persistence (in-memory only).
    pub fn in_memory() -> Self {
        Self::new(None)
    }

    /// Insert a pending gate. Fails if one already exists for (user, thread).
    pub async fn insert(&self, gate: PendingGate) -> Result<(), GateStoreError> {
        let key = gate.key();
        {
            let mut inner = self.inner.lock().await;
            if inner.by_key.contains_key(&key) {
                return Err(GateStoreError::AlreadyExists);
            }
            inner.by_request_id.insert(gate.request_id, key.clone());
            inner.by_key.insert(key, gate.clone());
        }
        // Persist after lock is released (async I/O outside lock)
        if let Some(ref persistence) = self.persistence {
            persistence.save(&gate).await?;
        }
        Ok(())
    }

    /// Atomically take a pending gate after verifying all invariants.
    ///
    /// This is the **only** way to retrieve a gate for resolution. Under
    /// a single lock acquisition it:
    /// 1. Checks the gate exists for `(user_id, thread_id)`
    /// 2. Verifies `request_id` matches (prevents stale approvals)
    /// 3. Verifies channel authorization
    /// 4. Checks expiry
    /// 5. Removes from both indices
    pub async fn take_verified(
        &self,
        key: &PendingGateKey,
        request_id: Uuid,
        responding_channel: &str,
    ) -> Result<PendingGate, GateStoreError> {
        let gate = {
            let mut inner = self.inner.lock().await;

            let gate = inner.by_key.get(key).ok_or(GateStoreError::NotFound)?;

            // Verify request ID
            if gate.request_id != request_id {
                return Err(GateStoreError::RequestIdMismatch);
            }

            // Verify channel authorization
            let channel_authorized = gate.source_channel == responding_channel
                || TRUSTED_GATE_CHANNELS.contains(&responding_channel);
            if !channel_authorized {
                return Err(GateStoreError::ChannelMismatch {
                    expected: gate.source_channel.clone(),
                    actual: responding_channel.to_string(),
                });
            }

            // Check expiry
            if gate.is_expired() {
                // Clean up expired gate while we hold the lock
                let gate = inner.by_key.remove(key);
                if let Some(ref g) = gate {
                    inner.by_request_id.remove(&g.request_id);
                }
                return Err(GateStoreError::Expired);
            }

            // Atomically remove — no TOCTOU gap
            let gate = inner.by_key.remove(key).ok_or(GateStoreError::NotFound)?;
            inner.by_request_id.remove(&gate.request_id);
            gate
        };

        // Persist removal after lock is released
        if let Some(ref persistence) = self.persistence
            && let Err(e) = persistence.remove(key).await
        {
            tracing::debug!(error = %e, "gate persistence removal failed (gate already taken from memory)");
        }
        Ok(gate)
    }

    /// Read-only peek at a pending gate (for history/reconnect responses).
    ///
    /// Does NOT remove the gate. Returns `None` if no gate exists or it
    /// has expired.
    pub async fn peek(&self, key: &PendingGateKey) -> Option<PendingGateView> {
        let inner = self.inner.lock().await;
        inner
            .by_key
            .get(key)
            .filter(|g| !g.is_expired())
            .map(PendingGateView::from)
    }

    /// List all non-expired gates for a user.
    pub async fn list_for_user(&self, user_id: &str) -> Vec<PendingGate> {
        let inner = self.inner.lock().await;
        inner
            .by_key
            .values()
            .filter(|gate| gate.user_id == user_id && !gate.is_expired())
            .cloned()
            .collect()
    }

    /// List all non-expired gates.
    pub async fn list_all(&self) -> Vec<PendingGate> {
        let inner = self.inner.lock().await;
        inner
            .by_key
            .values()
            .filter(|gate| !gate.is_expired())
            .cloned()
            .collect()
    }

    /// Remove a gate by key without verification.
    ///
    /// Used for cleanup paths like conversation clears or explicit cancel flows.
    pub async fn discard(&self, key: &PendingGateKey) -> Result<(), GateStoreError> {
        let removed = {
            let mut inner = self.inner.lock().await;
            let gate = inner.by_key.remove(key).ok_or(GateStoreError::NotFound)?;
            inner.by_request_id.remove(&gate.request_id);
            gate
        };
        if let Some(ref persistence) = self.persistence {
            persistence.remove(key).await?;
        }
        let _ = removed;
        Ok(())
    }

    /// Restore pending gates from persistent storage on startup.
    /// Returns the number of non-expired gates restored.
    pub async fn restore_from_persistence(&self) -> Result<usize, GateStoreError> {
        let Some(ref persistence) = self.persistence else {
            return Ok(0);
        };
        let gates = persistence.load_all().await?;
        let mut count = 0;
        let mut inner = self.inner.lock().await;
        for gate in gates {
            if gate.is_expired() {
                continue;
            }
            let key = gate.key();
            inner.by_request_id.insert(gate.request_id, key.clone());
            inner.by_key.insert(key, gate);
            count += 1;
        }
        Ok(count)
    }

    /// Remove all expired gates. Returns the number removed.
    pub async fn expire_stale(&self) -> usize {
        let expired_keys = {
            let mut inner = self.inner.lock().await;
            let expired_keys: Vec<PendingGateKey> = inner
                .by_key
                .iter()
                .filter(|(_, g)| g.is_expired())
                .map(|(k, _)| k.clone())
                .collect();
            for key in &expired_keys {
                if let Some(gate) = inner.by_key.remove(key) {
                    inner.by_request_id.remove(&gate.request_id);
                }
            }
            expired_keys
        };
        // Persist removals outside the lock
        let count = expired_keys.len();
        if let Some(ref persistence) = self.persistence {
            for key in &expired_keys {
                if let Err(e) = persistence.remove(key).await {
                    tracing::debug!(error = %e, "failed to remove expired gate from persistence");
                }
            }
        }
        count
    }

    /// Check whether a channel name is reserved (WASM cannot register it).
    pub fn is_channel_reserved(name: &str) -> bool {
        RESERVED_CHANNEL_NAMES.contains(&name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use ironclaw_engine::{ConversationId, ResumeKind, ThreadId};

    fn sample_gate_with(
        user_id: &str,
        thread_id: ThreadId,
        channel: &str,
        expires_in_secs: i64,
    ) -> PendingGate {
        PendingGate {
            request_id: Uuid::new_v4(),
            gate_name: "approval".into(),
            user_id: user_id.into(),
            thread_id,
            scope_thread_id: None,
            conversation_id: ConversationId::new(),
            source_channel: channel.into(),
            action_name: "shell".into(),
            call_id: "call_1".into(),
            parameters: serde_json::json!({"command": "ls"}),
            display_parameters: None,
            description: "Run shell command".into(),
            resume_kind: ResumeKind::Approval { allow_always: true },
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::seconds(expires_in_secs),
            original_message: None,
            resume_output: None,
            approval_already_granted: false,
        }
    }

    fn sample_gate(channel: &str) -> PendingGate {
        sample_gate_with("user1", ThreadId::new(), channel, 300)
    }

    // ── Basic operations ─────────────────────────────────────

    #[tokio::test]
    async fn test_insert_and_take_verified_roundtrip() {
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("telegram");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        let taken = store
            .take_verified(&key, request_id, "telegram")
            .await
            .unwrap();
        assert_eq!(taken.action_name, "shell");
    }

    #[tokio::test]
    async fn test_insert_duplicate_key_fails() {
        let store = PendingGateStore::in_memory();
        let tid = ThreadId::new();
        let g1 = sample_gate_with("user1", tid, "web", 300);
        let g2 = sample_gate_with("user1", tid, "web", 300);
        store.insert(g1).await.unwrap();
        assert!(matches!(
            store.insert(g2).await,
            Err(GateStoreError::AlreadyExists)
        ));
    }

    // ── Request ID verification ──────────────────────────────

    #[tokio::test]
    async fn test_take_verified_request_id_mismatch() {
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("telegram");
        let key = gate.key();
        store.insert(gate).await.unwrap();

        let wrong_id = Uuid::new_v4();
        assert!(matches!(
            store.take_verified(&key, wrong_id, "telegram").await,
            Err(GateStoreError::RequestIdMismatch)
        ));
    }

    #[tokio::test]
    async fn test_request_id_mismatch_never_drops_pending_gate() {
        // Regression: 74cbe5c2 — wrong request_id must NOT consume the gate
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("telegram");
        let key = gate.key();
        let correct_id = gate.request_id;
        store.insert(gate).await.unwrap();

        // Wrong ID → error
        let _ = store.take_verified(&key, Uuid::new_v4(), "telegram").await;

        // Correct ID → still works (gate was not consumed)
        let taken = store
            .take_verified(&key, correct_id, "telegram")
            .await
            .unwrap();
        assert_eq!(taken.action_name, "shell");
    }

    #[tokio::test]
    async fn test_list_for_user_filters_expired_and_other_users() {
        let store = PendingGateStore::in_memory();
        let live = sample_gate_with("alice", ThreadId::new(), "web", 300);
        let expired = sample_gate_with("alice", ThreadId::new(), "web", -1);
        let other = sample_gate_with("bob", ThreadId::new(), "web", 300);

        store.insert(live.clone()).await.unwrap();
        store.insert(expired).await.unwrap();
        store.insert(other).await.unwrap();

        let listed = store.list_for_user("alice").await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].request_id, live.request_id);
    }

    #[tokio::test]
    async fn test_list_all_filters_expired() {
        let store = PendingGateStore::in_memory();
        store
            .insert(sample_gate_with("alice", ThreadId::new(), "web", 300))
            .await
            .unwrap();
        store
            .insert(sample_gate_with("alice", ThreadId::new(), "web", -1))
            .await
            .unwrap();

        assert_eq!(store.list_all().await.len(), 1);
    }

    // ── Channel verification ─────────────────────────────────

    #[tokio::test]
    async fn test_take_verified_channel_mismatch() {
        // Regression: 5d1d504e — cross-channel hijacking
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("telegram");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        assert!(matches!(
            store.take_verified(&key, request_id, "slack").await,
            Err(GateStoreError::ChannelMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn test_http_channel_cannot_approve_telegram_thread() {
        // Regression: 5d1d504e
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("telegram");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        assert!(matches!(
            store.take_verified(&key, request_id, "http").await,
            Err(GateStoreError::ChannelMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn test_take_verified_trusted_channel_bypasses() {
        // Regression: 427f908e — web gateway is trusted
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("telegram");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        // "gateway" is in TRUSTED_GATE_CHANNELS
        let taken = store
            .take_verified(&key, request_id, "gateway")
            .await
            .unwrap();
        assert_eq!(taken.source_channel, "telegram");
    }

    #[tokio::test]
    async fn test_web_trusted_channel_bypasses() {
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("signal");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        let taken = store.take_verified(&key, request_id, "web").await.unwrap();
        assert_eq!(taken.source_channel, "signal");
    }

    // ── Expiry ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_take_verified_expired_gate() {
        let store = PendingGateStore::in_memory();
        let gate = sample_gate_with("user1", ThreadId::new(), "web", -1); // already expired
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        assert!(matches!(
            store.take_verified(&key, request_id, "web").await,
            Err(GateStoreError::Expired)
        ));
    }

    #[tokio::test]
    async fn test_expire_stale_removes_expired() {
        let store = PendingGateStore::in_memory();
        let tid1 = ThreadId::new();
        let tid2 = ThreadId::new();
        let expired = sample_gate_with("user1", tid1, "web", -1);
        let valid = sample_gate_with("user1", tid2, "web", 300);

        store.insert(expired).await.unwrap();
        store.insert(valid).await.unwrap();

        let removed = store.expire_stale().await;
        assert_eq!(removed, 1);

        // Valid gate still accessible via peek
        let key = PendingGateKey {
            user_id: "user1".into(),
            thread_id: tid2,
        };
        assert!(store.peek(&key).await.is_some());
    }

    // ── Concurrency ──────────────────────────────────────────

    #[tokio::test]
    async fn test_concurrent_take_only_one_succeeds() {
        // Regression: 52d935d7 — TOCTOU race in approval resolution
        let store = Arc::new(PendingGateStore::in_memory());
        let gate = sample_gate("web");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        let s1 = Arc::clone(&store);
        let s2 = Arc::clone(&store);
        let k1 = key.clone();
        let k2 = key;

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { s1.take_verified(&k1, request_id, "web").await }),
            tokio::spawn(async move { s2.take_verified(&k2, request_id, "web").await }),
        );

        let results = [r1.unwrap(), r2.unwrap()];
        let successes = results.iter().filter(|r| r.is_ok()).count();
        let failures = results.iter().filter(|r| r.is_err()).count();

        assert_eq!(successes, 1, "Exactly one concurrent take must succeed");
        assert_eq!(failures, 1, "Exactly one concurrent take must fail");
    }

    // ── Peek ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_peek_does_not_remove() {
        let store = PendingGateStore::in_memory();
        let gate = sample_gate("web");
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        // Peek returns view
        let view = store.peek(&key).await;
        assert!(view.is_some());
        assert_eq!(view.unwrap().tool_name, "shell");

        // Gate still accessible for take
        let taken = store.take_verified(&key, request_id, "web").await;
        assert!(taken.is_ok());
    }

    #[tokio::test]
    async fn test_peek_returns_none_for_expired() {
        let store = PendingGateStore::in_memory();
        let gate = sample_gate_with("user1", ThreadId::new(), "web", -1);
        let key = gate.key();
        store.insert(gate).await.unwrap();

        assert!(store.peek(&key).await.is_none());
    }

    // ── Thread scoping ───────────────────────────────────────

    #[tokio::test]
    async fn test_pending_gate_scoped_to_thread_not_leaked() {
        // Regression: e3b66f69 — cross-thread approval leakage
        let store = PendingGateStore::in_memory();
        let tid_a = ThreadId::new();
        let tid_b = ThreadId::new();

        let gate_a = sample_gate_with("user1", tid_a, "web", 300);
        store.insert(gate_a).await.unwrap();

        // Query for thread B returns None
        let key_b = PendingGateKey {
            user_id: "user1".into(),
            thread_id: tid_b,
        };
        assert!(store.peek(&key_b).await.is_none());

        // Query for thread A returns the gate
        let key_a = PendingGateKey {
            user_id: "user1".into(),
            thread_id: tid_a,
        };
        assert!(store.peek(&key_a).await.is_some());
    }

    // ── Persistence ──────────────────────────────────────────

    #[tokio::test]
    async fn test_restore_from_persistence_skips_expired() {
        use std::sync::Mutex as StdMutex;

        struct FakePersistence {
            gates: StdMutex<Vec<PendingGate>>,
        }

        #[async_trait]
        impl GatePersistence for FakePersistence {
            async fn save(&self, gate: &PendingGate) -> Result<(), GateStoreError> {
                self.gates.lock().unwrap().push(gate.clone());
                Ok(())
            }
            async fn remove(&self, _key: &PendingGateKey) -> Result<(), GateStoreError> {
                Ok(())
            }
            async fn load_all(&self) -> Result<Vec<PendingGate>, GateStoreError> {
                Ok(self.gates.lock().unwrap().clone())
            }
        }

        let tid_valid = ThreadId::new();
        let tid_expired = ThreadId::new();
        let valid = sample_gate_with("user1", tid_valid, "web", 300);
        let expired = sample_gate_with("user1", tid_expired, "web", -10);

        let persistence = Arc::new(FakePersistence {
            gates: StdMutex::new(vec![valid, expired]),
        });

        let store = PendingGateStore::new(Some(persistence));
        let restored = store.restore_from_persistence().await.unwrap();
        assert_eq!(restored, 1);

        // Valid gate present
        let key = PendingGateKey {
            user_id: "user1".into(),
            thread_id: tid_valid,
        };
        assert!(store.peek(&key).await.is_some());

        // Expired gate NOT present
        let key_expired = PendingGateKey {
            user_id: "user1".into(),
            thread_id: tid_expired,
        };
        assert!(store.peek(&key_expired).await.is_none());
    }

    // ── Reserved channel names ───────────────────────────────

    #[test]
    fn test_wasm_channel_cannot_claim_telegram_name() {
        // Regression: 92138b8c
        assert!(PendingGateStore::is_channel_reserved("telegram"));
    }

    #[test]
    fn test_wasm_channel_cannot_register_as_bootstrap() {
        // Regression: aa151d9f
        assert!(PendingGateStore::is_channel_reserved("__bootstrap__"));
    }

    #[test]
    fn test_custom_channel_not_reserved() {
        assert!(!PendingGateStore::is_channel_reserved("my-custom-channel"));
    }
}
