//! Progress-detection strategy.
//!
//! Loop-detection and diminishing-returns are kept as a separate sealed
//! strategy because they need a different signal (recent step deltas)
//! than budget enforcement. A run can be killed for being stuck even
//! when it has plenty of budget left; conversely a productive run is
//! never killed for iteration count.
//!
//! The default impl tracks two signals:
//!
//! 1. **Diminishing returns** — running average of recent assistant-output
//!    token deltas. When the last `window` steps all sit below
//!    `min_delta_tokens`, the strategy returns `Warning`; after
//!    `noprogress_consecutive_window` consecutive warnings the loop
//!    exits as `StuckNoProgress`.
//! 2. **Repeated tool call** — a sliding window of recent
//!    `(capability_id, [ParamHash])` pairs. When the same pair appears
//!    `repeat_threshold` times in a row, the strategy returns
//!    `RepeatedToolCall { Stuck }` and the loop exits as `StuckLoop`.
//!
//! Param-hash normalization strips ISO-8601 timestamps, UUID-shaped
//! substrings, and well-known correlation_id-style keys before hashing
//! so that "same-call-twice" detection isn't fooled by random
//! request-id noise.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) use ironclaw_turns::run_profile::normalize_for_hash;

/// Stable, normalized hash of a tool/capability call's arguments.
///
/// Two calls whose arguments differ only by timestamps, UUIDs, or
/// well-known correlation/request id keys hash to the same `ParamHash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParamHash([u8; 32]);

impl ParamHash {
    /// Compute a normalized hash. See module docs for the normalization
    /// rules.
    pub fn from_value(value: &Value) -> Self {
        let normalized = normalize_for_hash(value);
        let canonical = match serde_jcs::to_string(&normalized) {
            Ok(canonical) => canonical,
            Err(_) => serde_json::to_string(&normalized).unwrap_or_default(),
        };
        Self::from_bytes(canonical.as_bytes())
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let digest = blake3::hash(bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(digest.as_bytes());
        Self(out)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use std::fmt::Write;
            let _ = write!(out, "{:02x}", byte);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_values_hash_identically() {
        let a = json!({"path": "/foo", "count": 3});
        let b = json!({"count": 3, "path": "/foo"}); // key order varies
        assert_eq!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }

    #[test]
    fn timestamps_inside_values_normalize_to_same_hash() {
        let a = json!({"trace": "log 2026-05-21T10:00:00Z entry"});
        let b = json!({"trace": "log 2026-05-21T10:30:00Z entry"});
        assert_eq!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }

    #[test]
    fn uuids_inside_values_normalize_to_same_hash() {
        let a = json!({"id": "a1b2c3d4-e5f6-7890-abcd-ef0123456789"});
        let b = json!({"id": "00000000-0000-0000-0000-000000000000"});
        assert_eq!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }

    #[test]
    fn correlation_keys_are_dropped_before_hashing() {
        let a = json!({"path": "/foo", "request_id": "abc"});
        let b = json!({"path": "/foo", "request_id": "xyz"});
        assert_eq!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }

    #[test]
    fn different_payloads_hash_differently() {
        let a = json!({"path": "/foo"});
        let b = json!({"path": "/bar"});
        assert_ne!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }

    #[test]
    fn nested_arrays_normalize_recursively() {
        let a = json!([
            {"id": "a1b2c3d4-e5f6-7890-abcd-ef0123456789", "kind": "x"},
            {"id": "ffffffff-ffff-ffff-ffff-ffffffffffff", "kind": "x"},
        ]);
        let b = json!([
            {"id": "00000000-0000-0000-0000-000000000000", "kind": "x"},
            {"id": "11111111-2222-3333-4444-555555555555", "kind": "x"},
        ]);
        assert_eq!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }

    #[test]
    fn hex_string_long_enough_to_resemble_uuid_but_no_dashes_does_not_collapse() {
        let a = json!({"blob": "deadbeefdeadbeefdeadbeefdeadbeef"});
        let b = json!({"blob": "cafebabecafebabecafebabecafebabe"});
        assert_ne!(ParamHash::from_value(&a), ParamHash::from_value(&b));
    }
}
