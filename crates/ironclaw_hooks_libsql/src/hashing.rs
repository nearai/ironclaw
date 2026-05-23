//! Scope-hash derivation for predicate-state keys.
//!
//! A "scope" is the durable identity of a counter / value-sum bucket. The
//! components are blake3-hashed into a fixed 32-byte `scope_hash` BLOB that
//! becomes part of the table PRIMARY KEY. Components are length-prefixed so
//! distinct splits can't alias (`("a","bc")` vs `("ab","c")`).

use chrono::{DateTime, Utc};
use ironclaw_hooks::predicate_state::{InvocationKey, ValueKey};

/// blake3 of the invocation scope components. Length-prefixed so distinct
/// component splits can't alias.
pub(crate) fn invocation_scope_hash(key: &InvocationKey) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, key.hook_id.as_bytes());
    hash_component(&mut hasher, key.tenant_id.as_str().as_bytes());
    hash_component(&mut hasher, key.capability.as_bytes());
    hasher.finalize().as_bytes().to_vec()
}

/// blake3 of the value scope components (invocation scope + numeric field).
pub(crate) fn value_scope_hash(key: &ValueKey) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
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
