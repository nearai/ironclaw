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

fn normalize_for_hash(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(s) => Value::String(normalize_string(s)),
        Value::Array(items) => Value::Array(items.iter().map(normalize_for_hash).collect()),
        Value::Object(map) => {
            let mut filtered = serde_json::Map::new();
            for (key, val) in map {
                if is_correlation_key(key) {
                    continue;
                }
                filtered.insert(key.clone(), normalize_for_hash(val));
            }
            Value::Object(filtered)
        }
    }
}

fn is_correlation_key(key: &str) -> bool {
    // ASCII case-insensitive comparison against a static slice — zero
    // heap allocation on every hash. Underscore-vs-no-underscore is the
    // only spelling variance we accept; anything else (kebab-case,
    // mixed alnum punctuation) is treated as a distinct param.
    const KEYS: &[&str] = &[
        "request_id",
        "requestid",
        "trace_id",
        "traceid",
        "correlation_id",
        "correlationid",
        "idempotency_key",
        "idempotencykey",
        "x_request_id",
    ];
    KEYS.iter().any(|k| key.eq_ignore_ascii_case(k))
}

fn normalize_string(s: &str) -> String {
    let de_uuid = replace_uuids(s);
    replace_timestamps(&de_uuid)
}

fn replace_uuids(s: &str) -> String {
    // Strict UUID v1-v5: 8-4-4-4-12 hex segments. ASCII-only by construction,
    // so we scan over `&[u8]` and consume one UTF-8 codepoint at a time on
    // the non-match path. Avoids the `Vec<char>` allocation the previous
    // implementation paid on every hash.
    replace_ascii_pattern(s, "<uuid>", match_uuid_bytes)
}

fn match_uuid_bytes(bytes: &[u8]) -> Option<usize> {
    let segments = [8, 4, 4, 4, 12];
    let total_required: usize = segments.iter().sum::<usize>() + segments.len() - 1;
    if bytes.len() < total_required {
        return None;
    }
    let mut idx = 0;
    for (i, len) in segments.iter().enumerate() {
        if i > 0 {
            if bytes[idx] != b'-' {
                return None;
            }
            idx += 1;
        }
        for _ in 0..*len {
            if !bytes[idx].is_ascii_hexdigit() {
                return None;
            }
            idx += 1;
        }
    }
    Some(idx)
}

fn replace_timestamps(s: &str) -> String {
    // Replace ISO-8601-ish timestamps: YYYY-MM-DD[Tt ]HH:MM:SS(.fff)?(Z|±HH:MM)?
    replace_ascii_pattern(s, "<timestamp>", match_iso8601_bytes)
}

fn match_iso8601_bytes(bytes: &[u8]) -> Option<usize> {
    // Minimum: YYYY-MM-DDTHH:MM:SS  = 19 bytes (all ASCII).
    if bytes.len() < 19 {
        return None;
    }
    if !(bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4] == b'-'
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && bytes[7] == b'-'
        && bytes[8].is_ascii_digit()
        && bytes[9].is_ascii_digit()
        && (bytes[10] == b'T' || bytes[10] == b't' || bytes[10] == b' ')
        && bytes[11].is_ascii_digit()
        && bytes[12].is_ascii_digit()
        && bytes[13] == b':'
        && bytes[14].is_ascii_digit()
        && bytes[15].is_ascii_digit()
        && bytes[16] == b':'
        && bytes[17].is_ascii_digit()
        && bytes[18].is_ascii_digit())
    {
        return None;
    }
    let mut consumed = 19;
    // Optional fractional seconds .fff…
    if consumed < bytes.len() && bytes[consumed] == b'.' {
        let mut j = consumed + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > consumed + 1 {
            consumed = j;
        }
    }
    // Optional timezone Z or ±HH:MM
    if consumed < bytes.len() {
        if bytes[consumed] == b'Z' || bytes[consumed] == b'z' {
            consumed += 1;
        } else if (bytes[consumed] == b'+' || bytes[consumed] == b'-')
            && consumed + 5 < bytes.len()
            && bytes[consumed + 1].is_ascii_digit()
            && bytes[consumed + 2].is_ascii_digit()
            && bytes[consumed + 3] == b':'
            && bytes[consumed + 4].is_ascii_digit()
            && bytes[consumed + 5].is_ascii_digit()
        {
            consumed += 6;
        }
    }
    Some(consumed)
}

/// Walk `s` byte-by-byte, replacing each ASCII-pattern match with `replacement`.
/// On the non-match path advance one whole UTF-8 codepoint so we never split a
/// multibyte character (Rust would panic on `&s[i..i+1]` mid-codepoint).
fn replace_ascii_pattern(
    s: &str,
    replacement: &str,
    matcher: fn(&[u8]) -> Option<usize>,
) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(consumed) = matcher(&bytes[i..]) {
            out.push_str(replacement);
            i += consumed;
        } else {
            let mut next = i + 1;
            while next < bytes.len() && !s.is_char_boundary(next) {
                next += 1;
            }
            out.push_str(&s[i..next]);
            i = next;
        }
    }
    out
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
