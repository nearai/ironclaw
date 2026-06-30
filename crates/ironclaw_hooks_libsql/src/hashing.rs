//! Predicate bucket-identity hashing for the libSQL backend.
//!
//! The hash derivation lives in the shared crate
//! ([`ironclaw_hooks::predicate_hash`]) so the durable DB identity contract has
//! a single source of truth across backends (it previously diverged: this
//! crate used an 8-byte little-endian `u64` length prefix while Postgres used
//! 4-byte big-endian `u32`). The wrappers below adapt the canonical 32-byte
//! [`Digest`] to the `Vec<u8>` the `BLOB` columns and libSQL params take, and
//! keep the libSQL-specific epoch/window-cutoff projection alongside it.
//!
//! [`Digest`]: ironclaw_hooks::predicate_hash::Digest

use chrono::{DateTime, Utc};
use ironclaw_hooks::predicate_hash;
use ironclaw_hooks::predicate_state::{InvocationKey, ValueKey, window_cutoff};

/// `scope_hash` for a tenant — the trust boundary and per-tenant LRU-quota
/// grain (`COUNT(DISTINCT key_hash) WHERE scope_hash = ?`).
pub(crate) fn tenant_scope_hash(tenant_id: &str) -> Vec<u8> {
    predicate_hash::scope_hash(tenant_id).to_vec()
}

/// `key_hash` for an invocation-counter bucket.
pub(crate) fn invocation_key_hash(key: &InvocationKey) -> Vec<u8> {
    predicate_hash::invocation_key_hash(key).to_vec()
}

/// `key_hash` for a numeric-value-sum bucket.
pub(crate) fn value_key_hash(key: &ValueKey) -> Vec<u8> {
    predicate_hash::value_key_hash(key).to_vec()
}

/// Epoch milliseconds for the canonical host clock. `timestamp_millis`
/// returns `i64`; the column is INTEGER so this is exact.
pub(crate) fn to_epoch_millis(now: DateTime<Utc>) -> i64 {
    now.timestamp_millis()
}

/// Window cutoff in epoch milliseconds. Delegates to the **canonical**
/// [`ironclaw_hooks::predicate_state::window_cutoff`] so the overflow/boundary
/// behaviour is byte-for-byte identical to the in-memory backend (and any other
/// durable backend), then projects the resulting wall-clock instant onto the
/// epoch-millis grain the columns store. We do NOT reimplement the
/// `Duration → cutoff` math here: an independent `saturating_sub` shortcut
/// diverges on a pathological oversized window (it would trim nothing, while
/// the canonical rule trims to `now`). The trim comparison remains
/// `occurred_at < cutoff` (strictly older), so an entry exactly at the cutoff
/// is retained — the `< cutoff, not <=` contract.
pub(crate) fn window_cutoff_millis(now: DateTime<Utc>, window: std::time::Duration) -> i64 {
    window_cutoff(now, window).timestamp_millis()
}
