//! Delivered-gate route records for cross-thread approval routing.
//!
//! When a triggered run is blocked on approval and the approval prompt is
//! delivered to the creator's personal target (e.g., a Slack DM), the DM
//! thread is different from the run's original thread. When the user replies
//! with `approve <gate_ref>` in their DM, the inbound path resolves the scope
//! from the DM conversation rather than the run's thread — causing a
//! `MissingGate` error in the approval service.
//!
//! This module stores a lightweight routing record that maps
//! `(tenant_id, user_id, gate_ref)` → `(run_id, scope)`. The composition
//! layer records this mapping when an approval prompt is delivered, and a
//! routing wrapper around the `ApprovalInteractionService` rewrites the
//! incoming resolve request to use the stored scope before forwarding to the
//! inner service.
//!
//! ## Record lifetime
//!
//! Route records expire after [`DELIVERED_GATE_ROUTE_TTL`]. Expired records
//! are ignored on load (treated as a miss) and removed lazily by the routing
//! wrapper. An opportunistic sweep runs when a new route is recorded (on
//! approval-prompt delivery) via [`DeliveredGateRouteStore::sweep_expired_delivered_gate_routes`].
//!
//! Design constraints:
//! - Channel-neutral: no Slack, WebUI, or other channel-specific words.
//! - Best-effort writes: callers must swallow store errors and never abort
//!   delivery on write failure.
//! - Security: the lookup key binds tenant + user; the wrapper also verifies
//!   that the requesting actor matches the record owner before rewriting.
//! - Personal scope only: route records are only written for personal-scope
//!   triggers (the driver already fails closed to personal-only).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_turns::{TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

/// How long a delivered-gate route record may live before it is considered
/// expired. Bounds how long an approval reply in a personal DM can rewrite
/// the request to the run's original thread. 48 hours far exceeds any gate's
/// pending lifetime and the idempotent-replay window; after expiry the record
/// is ignored on load and removed lazily (or swept opportunistically).
pub const DELIVERED_GATE_ROUTE_TTL: Duration = Duration::hours(48);

/// A route record mapping a delivered gate prompt back to the run and scope
/// it was delivered for.
///
/// Persisted per `(tenant_id, user_id, gate_ref)`. The gate_ref is unique per
/// run; a user cannot hold two concurrent pending approvals with the same
/// gate_ref. The routing wrapper removes the record (best-effort) once the
/// gate resolves; routes for gates that never resolve linger, which is
/// accepted — records are tiny and keys never collide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredGateRouteRecord {
    /// Tenant the gate belongs to.
    pub tenant_id: TenantId,
    /// Owner user who received the approval prompt.
    pub user_id: UserId,
    /// Gate reference string as delivered in the approval prompt.
    pub gate_ref: String,
    /// Run that is blocked on this gate.
    pub run_id: TurnRunId,
    /// Scope the run lives under (the triggered run's thread scope, not the
    /// DM thread scope where the reply arrives).
    pub scope: TurnScope,
    /// When this record was written.
    pub recorded_at: DateTime<Utc>,
    /// Conversation fingerprints where this gate prompt was delivered.
    #[serde(default)]
    pub delivered_conversation_fingerprints: Vec<String>,
}

impl DeliveredGateRouteRecord {
    /// Returns `true` when this record is older than [`DELIVERED_GATE_ROUTE_TTL`]
    /// relative to `now`. Expired records should be treated as a store miss and
    /// removed lazily.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.recorded_at) > DELIVERED_GATE_ROUTE_TTL
    }
}

/// Lookup key used by the routing wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RouteKey {
    tenant_id: TenantId,
    user_id: UserId,
    gate_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConversationIndexKey {
    tenant_id: TenantId,
    conversation_fingerprint: String,
}

impl ConversationIndexKey {
    fn new(tenant_id: TenantId, conversation_fingerprint: String) -> Self {
        Self {
            tenant_id,
            conversation_fingerprint,
        }
    }
}

impl RouteKey {
    fn new(tenant_id: TenantId, user_id: UserId, gate_ref: String) -> Self {
        Self {
            tenant_id,
            user_id,
            gate_ref,
        }
    }
}

/// Store for [`DeliveredGateRouteRecord`]s.
///
/// Writes are best-effort: production callers must not propagate store errors
/// to the delivery path. Reads are used by the routing wrapper before
/// forwarding an approval-resolve request to the inner service.
#[async_trait::async_trait]
pub trait DeliveredGateRouteStore: Send + Sync {
    /// Record a delivered gate route. Best-effort: errors are returned as
    /// `String` so callers can log them without depending on a specific error
    /// type.
    async fn record_delivered_gate_route(
        &self,
        record: DeliveredGateRouteRecord,
    ) -> Result<(), String>;

    /// Load the route record for `(tenant_id, user_id, gate_ref)`. Returns
    /// `None` if no record exists (miss → forward the request unchanged).
    async fn load_delivered_gate_route(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        gate_ref: &str,
    ) -> Result<Option<DeliveredGateRouteRecord>, String>;

    /// Load the route record delivered into an external conversation, when
    /// bare approval/auth replies omit an explicit gate reference.
    async fn load_delivered_gate_route_by_conversation_fingerprint(
        &self,
        tenant_id: &TenantId,
        conversation_fingerprint: &str,
    ) -> Result<Option<DeliveredGateRouteRecord>, String>;

    /// Remove the route record for `(tenant_id, user_id, gate_ref)`.
    /// Best-effort cleanup after the gate is resolved; removing a missing
    /// record is not an error. Routes for gates that are never resolved
    /// linger — accepted for now, since records are tiny and gate refs are
    /// unique per run.
    async fn remove_delivered_gate_route(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        gate_ref: &str,
    ) -> Result<(), String>;

    /// Remove all route records that are expired as of `now`. Returns the
    /// number of records removed.
    ///
    /// Best-effort: implementations must not abort the sweep on a single
    /// bad record. Callers must swallow errors and must not let sweep failure
    /// affect the delivery path.
    async fn sweep_expired_delivered_gate_routes(
        &self,
        now: DateTime<Utc>,
    ) -> Result<usize, String>;
}

/// In-memory [`DeliveredGateRouteStore`].
#[derive(Default)]
pub struct InMemoryDeliveredGateRouteStore {
    records: Mutex<HashMap<RouteKey, DeliveredGateRouteRecord>>,
    conversation_index: Mutex<HashMap<ConversationIndexKey, RouteKey>>,
    reverse_conv_index: Mutex<HashMap<RouteKey, Vec<ConversationIndexKey>>>,
}

#[async_trait::async_trait]
impl DeliveredGateRouteStore for InMemoryDeliveredGateRouteStore {
    async fn record_delivered_gate_route(
        &self,
        record: DeliveredGateRouteRecord,
    ) -> Result<(), String> {
        let key = RouteKey::new(
            record.tenant_id.clone(),
            record.user_id.clone(),
            record.gate_ref.clone(),
        );
        let conversation_keys = conversation_keys_for_record(&record);
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut conversation_index = self
            .conversation_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut reverse_conv_index = self
            .reverse_conv_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(old_keys) = reverse_conv_index.remove(&key) {
            for old_key in old_keys {
                if conversation_index.get(&old_key) == Some(&key) {
                    conversation_index.remove(&old_key);
                }
            }
        }

        records.insert(key.clone(), record);
        for conversation_key in conversation_keys {
            conversation_index.insert(conversation_key.clone(), key.clone());
            reverse_conv_index
                .entry(key.clone())
                .or_default()
                .push(conversation_key);
        }
        Ok(())
    }

    async fn load_delivered_gate_route(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        gate_ref: &str,
    ) -> Result<Option<DeliveredGateRouteRecord>, String> {
        let key = RouteKey::new(tenant_id.clone(), user_id.clone(), gate_ref.to_string());
        Ok(self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .cloned())
    }

    async fn load_delivered_gate_route_by_conversation_fingerprint(
        &self,
        tenant_id: &TenantId,
        conversation_fingerprint: &str,
    ) -> Result<Option<DeliveredGateRouteRecord>, String> {
        let conversation_key =
            ConversationIndexKey::new(tenant_id.clone(), conversation_fingerprint.to_string());
        let records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let conversation_index = self
            .conversation_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(conversation_index
            .get(&conversation_key)
            .and_then(|route_key| records.get(route_key))
            .cloned())
    }

    async fn remove_delivered_gate_route(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        gate_ref: &str,
    ) -> Result<(), String> {
        let key = RouteKey::new(tenant_id.clone(), user_id.clone(), gate_ref.to_string());
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut conversation_index = self
            .conversation_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut reverse_conv_index = self
            .reverse_conv_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        records.remove(&key);
        if let Some(old_keys) = reverse_conv_index.remove(&key) {
            for old_key in old_keys {
                if conversation_index.get(&old_key) == Some(&key) {
                    conversation_index.remove(&old_key);
                }
            }
        }
        Ok(())
    }

    async fn sweep_expired_delivered_gate_routes(
        &self,
        now: DateTime<Utc>,
    ) -> Result<usize, String> {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut conversation_index = self
            .conversation_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut reverse_conv_index = self
            .reverse_conv_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = records.len();
        let expired_keys: Vec<RouteKey> = records
            .iter()
            .filter_map(|(key, record)| record.is_expired(now).then_some(key.clone()))
            .collect();
        for key in &expired_keys {
            records.remove(key);
            if let Some(old_keys) = reverse_conv_index.remove(key) {
                for old_key in old_keys {
                    if conversation_index.get(&old_key) == Some(key) {
                        conversation_index.remove(&old_key);
                    }
                }
            }
        }
        Ok(before - records.len())
    }
}

fn conversation_keys_for_record(record: &DeliveredGateRouteRecord) -> Vec<ConversationIndexKey> {
    let mut keys = Vec::new();
    for conversation_fingerprint in &record.delivered_conversation_fingerprints {
        let key =
            ConversationIndexKey::new(record.tenant_id.clone(), conversation_fingerprint.clone());
        if !keys.iter().any(|existing| existing == &key) {
            keys.push(key);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ThreadId};

    use super::*;

    fn tenant() -> TenantId {
        TenantId::new("tenant-gate-route-test").unwrap()
    }

    fn user() -> UserId {
        UserId::new("user-gate-route-test").unwrap()
    }

    fn scope() -> TurnScope {
        let agent = AgentId::new("agent-gate-route-test").unwrap();
        let thread = ThreadId::new("thread-gate-route-test").unwrap();
        let owner = UserId::new("user-gate-route-test").unwrap();
        TurnScope::new_with_owner(tenant(), Some(agent), None, thread, Some(owner))
    }

    fn record(gate_ref: &str) -> DeliveredGateRouteRecord {
        DeliveredGateRouteRecord {
            tenant_id: tenant(),
            user_id: user(),
            gate_ref: gate_ref.to_string(),
            run_id: TurnRunId::new(),
            scope: scope(),
            recorded_at: Utc::now(),
            delivered_conversation_fingerprints: Vec::new(),
        }
    }

    fn conversation_fingerprint(thread_id: &str) -> String {
        format!("fingerprint:{thread_id}")
    }

    #[tokio::test]
    async fn in_memory_store_round_trips_route_record() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let rec = record("gate:round-trip-001");

        store
            .record_delivered_gate_route(rec.clone())
            .await
            .expect("write succeeds");

        let loaded = store
            .load_delivered_gate_route(&tenant(), &user(), "gate:round-trip-001")
            .await
            .expect("read succeeds");

        assert_eq!(loaded, Some(rec));
    }

    #[tokio::test]
    async fn in_memory_store_returns_none_for_missing_key() {
        let store = InMemoryDeliveredGateRouteStore::default();

        let loaded = store
            .load_delivered_gate_route(&tenant(), &user(), "gate:does-not-exist")
            .await
            .expect("read succeeds even on miss");

        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn in_memory_store_key_includes_all_three_dimensions() {
        let store = InMemoryDeliveredGateRouteStore::default();

        // Same gate_ref, different user.
        let rec_a = DeliveredGateRouteRecord {
            user_id: UserId::new("user-a").unwrap(),
            ..record("gate:multi-key-001")
        };
        let rec_b = DeliveredGateRouteRecord {
            user_id: UserId::new("user-b").unwrap(),
            ..record("gate:multi-key-001")
        };

        store
            .record_delivered_gate_route(rec_a.clone())
            .await
            .unwrap();
        store
            .record_delivered_gate_route(rec_b.clone())
            .await
            .unwrap();

        let loaded_a = store
            .load_delivered_gate_route(
                &tenant(),
                &UserId::new("user-a").unwrap(),
                "gate:multi-key-001",
            )
            .await
            .unwrap();
        let loaded_b = store
            .load_delivered_gate_route(
                &tenant(),
                &UserId::new("user-b").unwrap(),
                "gate:multi-key-001",
            )
            .await
            .unwrap();

        assert_eq!(loaded_a, Some(rec_a));
        assert_eq!(loaded_b, Some(rec_b));
    }

    #[test]
    fn is_expired_returns_false_before_ttl() {
        let now = Utc::now();
        let rec = DeliveredGateRouteRecord {
            recorded_at: now - DELIVERED_GATE_ROUTE_TTL + Duration::seconds(1),
            ..record("gate:ttl-before")
        };
        assert!(!rec.is_expired(now));
    }

    #[test]
    fn is_expired_returns_false_exactly_at_ttl_boundary() {
        let now = Utc::now();
        // exactly at TTL: duration == TTL → NOT expired (> is the check)
        let rec = DeliveredGateRouteRecord {
            recorded_at: now - DELIVERED_GATE_ROUTE_TTL,
            ..record("gate:ttl-boundary")
        };
        assert!(!rec.is_expired(now));
    }

    #[test]
    fn is_expired_returns_true_after_ttl() {
        let now = Utc::now();
        let rec = DeliveredGateRouteRecord {
            recorded_at: now - DELIVERED_GATE_ROUTE_TTL - Duration::seconds(1),
            ..record("gate:ttl-after")
        };
        assert!(rec.is_expired(now));
    }

    #[tokio::test]
    async fn in_memory_sweep_removes_only_expired_records() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let now = Utc::now();

        // Fresh record: recorded just now.
        let fresh = DeliveredGateRouteRecord {
            recorded_at: now,
            ..record("gate:sweep-fresh")
        };
        // Expired record: recorded 49 hours ago.
        let expired = DeliveredGateRouteRecord {
            recorded_at: now - Duration::hours(49),
            ..record("gate:sweep-expired")
        };

        store
            .record_delivered_gate_route(fresh.clone())
            .await
            .unwrap();
        store
            .record_delivered_gate_route(expired.clone())
            .await
            .unwrap();

        let removed = store
            .sweep_expired_delivered_gate_routes(now)
            .await
            .expect("sweep succeeds");
        assert_eq!(removed, 1, "exactly one expired record removed");

        // Fresh record still loadable.
        let still_present = store
            .load_delivered_gate_route(&tenant(), &user(), "gate:sweep-fresh")
            .await
            .expect("load succeeds")
            .expect("fresh record must still be present");
        assert_eq!(still_present.gate_ref, "gate:sweep-fresh");

        // Expired record is gone.
        let gone = store
            .load_delivered_gate_route(&tenant(), &user(), "gate:sweep-expired")
            .await
            .expect("load succeeds");
        assert!(gone.is_none(), "expired record must be absent after sweep");
    }

    #[tokio::test]
    async fn in_memory_sweep_empty_store_returns_zero() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let removed = store
            .sweep_expired_delivered_gate_routes(Utc::now())
            .await
            .expect("sweep on empty store succeeds");
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn in_memory_store_overwrites_on_second_write() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let first = record("gate:overwrite-001");
        let second = DeliveredGateRouteRecord {
            run_id: TurnRunId::new(),
            ..record("gate:overwrite-001")
        };
        assert_ne!(first.run_id, second.run_id);

        store.record_delivered_gate_route(first).await.unwrap();
        store
            .record_delivered_gate_route(second.clone())
            .await
            .unwrap();

        let loaded = store
            .load_delivered_gate_route(&tenant(), &user(), "gate:overwrite-001")
            .await
            .unwrap();
        assert_eq!(loaded.map(|r| r.run_id), Some(second.run_id));
    }

    #[tokio::test]
    async fn in_memory_conversation_lookup_round_trips() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let conv_a = conversation_fingerprint("thread-conv-a");
        let conv_b = conversation_fingerprint("thread-conv-b");
        let rec = DeliveredGateRouteRecord {
            delivered_conversation_fingerprints: vec![conv_a.clone(), conv_b.clone()],
            ..record("gate:conversation-round-trip")
        };

        store
            .record_delivered_gate_route(rec.clone())
            .await
            .expect("write succeeds");

        let loaded_a = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &conv_a)
            .await
            .expect("conversation lookup succeeds");
        let loaded_b = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &conv_b)
            .await
            .expect("conversation lookup succeeds");

        assert_eq!(loaded_a, Some(rec.clone()));
        assert_eq!(loaded_b, Some(rec));
    }

    #[tokio::test]
    async fn in_memory_conversation_lookup_returns_none_for_unknown() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let unknown = conversation_fingerprint("thread-unknown");

        let loaded = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &unknown)
            .await
            .expect("conversation lookup succeeds");

        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn in_memory_ttl_expiry_removes_conversation_index() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let now = Utc::now();
        let conv = conversation_fingerprint("thread-expired-index");
        let rec = DeliveredGateRouteRecord {
            recorded_at: now - Duration::hours(49),
            delivered_conversation_fingerprints: vec![conv.clone()],
            ..record("gate:conversation-expired")
        };

        store.record_delivered_gate_route(rec).await.unwrap();

        let removed = store
            .sweep_expired_delivered_gate_routes(now)
            .await
            .expect("sweep succeeds");
        assert_eq!(removed, 1);

        let loaded = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &conv)
            .await
            .expect("conversation lookup succeeds");
        assert!(loaded.is_none());
    }

    #[test]
    fn in_memory_old_record_rehydration_without_conversation_field() {
        let rec = record("gate:old-json");
        let json = serde_json::json!({
            "tenant_id": rec.tenant_id,
            "user_id": rec.user_id,
            "gate_ref": rec.gate_ref,
            "run_id": rec.run_id,
            "scope": rec.scope,
            "recorded_at": rec.recorded_at,
        });

        let rehydrated: DeliveredGateRouteRecord =
            serde_json::from_value(json).expect("old JSON rehydrates");

        assert!(rehydrated.delivered_conversation_fingerprints.is_empty());
    }

    #[tokio::test]
    async fn in_memory_overwrite_updates_conversation_index() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let conv_a = conversation_fingerprint("thread-overwrite-a");
        let conv_b = conversation_fingerprint("thread-overwrite-b");
        let first = DeliveredGateRouteRecord {
            delivered_conversation_fingerprints: vec![conv_a.clone()],
            ..record("gate:conversation-overwrite")
        };
        let second = DeliveredGateRouteRecord {
            delivered_conversation_fingerprints: vec![conv_b.clone()],
            run_id: TurnRunId::new(),
            ..record("gate:conversation-overwrite")
        };

        store.record_delivered_gate_route(first).await.unwrap();
        store
            .record_delivered_gate_route(second.clone())
            .await
            .unwrap();

        let loaded_a = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &conv_a)
            .await
            .expect("conversation lookup succeeds");
        let loaded_b = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &conv_b)
            .await
            .expect("conversation lookup succeeds");

        assert!(loaded_a.is_none());
        assert_eq!(loaded_b, Some(second));
    }

    #[tokio::test]
    async fn in_memory_removing_old_route_preserves_reused_conversation_index() {
        let store = InMemoryDeliveredGateRouteStore::default();
        let shared = conversation_fingerprint("thread-reused-index");
        let old = DeliveredGateRouteRecord {
            delivered_conversation_fingerprints: vec![shared.clone()],
            ..record("gate:conversation-old")
        };
        let new = DeliveredGateRouteRecord {
            delivered_conversation_fingerprints: vec![shared.clone()],
            ..record("gate:conversation-new")
        };

        store.record_delivered_gate_route(old).await.unwrap();
        store
            .record_delivered_gate_route(new.clone())
            .await
            .unwrap();
        store
            .remove_delivered_gate_route(&tenant(), &user(), "gate:conversation-old")
            .await
            .unwrap();

        let loaded = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant(), &shared)
            .await
            .expect("conversation lookup succeeds");
        assert_eq!(loaded, Some(new));
    }
}
