//! DB-backed pairing store.
//!
//! Replaces the file-based `~/.ironclaw/{channel}-pairing.json` store.
//! Delegates to the `ChannelPairingStore` DB sub-trait. `insert` and `remove`
//! update `OwnershipCache` immediately (write-through); `approve` populates the
//! cache lazily on the next `resolve_identity` call because the channel and
//! external_id are not available at approval time.
//!
//! When no database is available (the `db` field is `None`), the store operates
//! in noop mode: writes silently succeed (returning dummy records where needed)
//! and reads return empty/not-found. This preserves the ability to run WASM
//! channels without a persistent DB.

use std::sync::Arc;

use crate::db::{Database, PairingRequestRecord};
use crate::error::DatabaseError;
use crate::ownership::{Identity, OwnerId, OwnershipCache};

/// Pairing operations: create pending requests, approve them, resolve identities.
///
/// Wraps `ChannelPairingStore` (DB operations) with `OwnershipCache` (warm-path reads).
/// Write-through: `insert` and `remove` update the cache immediately.
/// `approve` populates lazily on next `resolve_identity` call.
/// When `db` is `None`, all operations degrade gracefully (no-ops / empty results).
#[derive(Clone)]
pub struct PairingStore {
    db: Option<Arc<dyn Database>>,
    cache: Arc<OwnershipCache>,
}

impl PairingStore {
    /// Create a DB-backed pairing store with cache write-through.
    pub fn new(db: Arc<dyn Database>, cache: Arc<OwnershipCache>) -> Self {
        Self {
            db: Some(db),
            cache,
        }
    }

    /// Create a no-op pairing store (for environments without a database).
    /// All reads return `None`/empty; all writes are silently discarded.
    pub fn new_noop() -> Self {
        Self {
            db: None,
            cache: Arc::new(OwnershipCache::new()),
        }
    }

    /// Returns the `Identity` for `(channel, external_id)` if the sender is paired.
    /// Cache hit → zero DB reads. Cache miss → one DB read (join channel_identities + users).
    pub async fn resolve_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<Identity>, DatabaseError> {
        if let Some(identity) = self.cache.get(channel, external_id) {
            return Ok(Some(identity));
        }
        let Some(ref db) = self.db else {
            use std::sync::atomic::{AtomicBool, Ordering};
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "PairingStore running in noop mode (no database): pairing-based channel \
                     admission is unavailable. Configure a database or use allow_from in \
                     channel config."
                );
            }
            return Ok(None);
        };
        let identity = db.resolve_channel_identity(channel, external_id).await?;
        if let Some(ref id) = identity {
            self.cache.insert(channel, external_id, id.clone());
        }
        Ok(identity)
    }

    /// Create or refresh a pending pairing request for an unknown sender.
    /// In noop mode, returns a dummy record with a generated code.
    pub async fn upsert_request(
        &self,
        channel: &str,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<PairingRequestRecord, DatabaseError> {
        let Some(ref db) = self.db else {
            return Ok(PairingRequestRecord {
                id: uuid::Uuid::new_v4(),
                channel: channel.to_string(),
                external_id: external_id.to_string(),
                code: crate::db::generate_pairing_code(),
                created: true,
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
            });
        };
        db.upsert_pairing_request(channel, external_id, meta).await
    }

    /// Approve a pairing code, mapping `(channel, external_id)` → `owner_id`.
    /// Updates DB atomically. Cache is populated on next `resolve_identity` call.
    /// In noop mode, silently succeeds.
    pub async fn approve(
        &self,
        channel: &str,
        code: &str,
        owner_id: &OwnerId,
    ) -> Result<(), DatabaseError> {
        let Some(ref db) = self.db else {
            return Ok(());
        };
        db.approve_pairing(channel, code, owner_id.as_str()).await
    }

    /// List pending pairing requests (for CLI and web UI display).
    pub async fn list_pending(
        &self,
        channel: &str,
    ) -> Result<Vec<PairingRequestRecord>, DatabaseError> {
        let Some(ref db) = self.db else {
            return Ok(Vec::new());
        };
        db.list_pending_pairings(channel).await
    }

    /// Remove a channel identity (unlink). Evicts from cache.
    pub async fn remove(&self, channel: &str, external_id: &str) -> Result<(), DatabaseError> {
        let Some(ref db) = self.db else {
            self.cache.evict(channel, external_id);
            return Ok(());
        };
        db.remove_channel_identity(channel, external_id).await?;
        self.cache.evict(channel, external_id);
        Ok(())
    }
}
