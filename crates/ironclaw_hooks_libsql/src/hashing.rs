//! Hash derivation for predicate-state keys.
//!
//! Two digests are derived per bucket, matching the canonical cross-backend
//! schema (see `crate::schema`):
//!
//! - `scope_hash` = blake3 of the length-prefixed `tenant_id`. This is the
//!   tenant trust boundary and the grain at which the per-tenant LRU quota is
//!   enforced (`COUNT(DISTINCT key_hash) WHERE scope_hash = ?`). It replaces
//!   the earlier raw `tenant_id` TEXT column.
//! - `key_hash` = blake3 of the length-prefixed *whole* bucket identity,
//!   including a one-byte map discriminant so an invocation key and a value
//!   key that share `(hook, tenant, capability)` never collide. This is the
//!   dedup + count/sum grain and the table PRIMARY KEY (with `event_id`).
//!
//! Components are length-prefixed so distinct splits can't alias
//! (`("a","bc")` vs `("ab","c")`).

use chrono::{DateTime, Utc};
use ironclaw_hooks::predicate_state::{InvocationKey, ValueKey};

/// Map discriminants folded into `key_hash` so the invocation and value tables
/// never produce the same `key_hash` for a shared `(hook, tenant, capability)`.
const KIND_INVOCATION: u8 = b'i';
const KIND_VALUE: u8 = b'v';

/// `scope_hash` for a tenant — the trust boundary and per-tenant LRU-quota
/// grain. blake3 of the length-prefixed `tenant_id`.
pub(crate) fn tenant_scope_hash(tenant_id: &str) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, tenant_id.as_bytes());
    hasher.finalize().as_bytes().to_vec()
}

/// `key_hash` for an invocation-counter bucket. Length-prefixed so distinct
/// component splits can't alias; map discriminant keeps it disjoint from the
/// value table's key space.
pub(crate) fn invocation_key_hash(key: &InvocationKey) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[KIND_INVOCATION]);
    hash_component(&mut hasher, key.hook_id.as_bytes());
    hash_component(&mut hasher, key.tenant_id.as_str().as_bytes());
    hash_component(&mut hasher, key.capability.as_bytes());
    hasher.finalize().as_bytes().to_vec()
}

/// `key_hash` for a numeric-value-sum bucket (invocation components + field).
pub(crate) fn value_key_hash(key: &ValueKey) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[KIND_VALUE]);
    hash_component(&mut hasher, key.hook_id.as_bytes());
    hash_component(&mut hasher, key.tenant_id.as_str().as_bytes());
    hash_component(&mut hasher, key.capability.as_bytes());
    hash_component(&mut hasher, key.field.as_bytes());
    hasher.finalize().as_bytes().to_vec()
}

fn hash_component(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Epoch milliseconds for the canonical host clock. `timestamp_millis`
/// returns `i64`; the column is INTEGER so this is exact.
pub(crate) fn to_epoch_millis(now: DateTime<Utc>) -> i64 {
    now.timestamp_millis()
}

/// Window cutoff in epoch milliseconds. `window` is a non-negative
/// [`std::time::Duration`]; we saturate on overflow so a pathological
/// multi-million-year window trims nothing (conservative for a rate/value
/// cap), matching the in-memory backend's `window_cutoff` saturation behavior.
/// The trim comparison is `occurred_at < cutoff` (strictly older), so an entry
/// exactly at the cutoff is retained — the `< cutoff, not <=` contract.
pub(crate) fn window_cutoff_millis(now: DateTime<Utc>, window: std::time::Duration) -> i64 {
    let now_ms = now.timestamp_millis();
    let window_ms = i64::try_from(window.as_millis()).unwrap_or(i64::MAX);
    now_ms.saturating_sub(window_ms)
}
