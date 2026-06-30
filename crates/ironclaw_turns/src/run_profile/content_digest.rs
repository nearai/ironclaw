//! Content-digest contract for output-aware progress detection.
//!
//! Boundary note: `ironclaw_turns` is otherwise a contracts-only crate, but the
//! `ContentDigest` newtype AND its `normalize_for_hash`/JCS/BLAKE3 implementation
//! live here together on purpose — both the host adapter (`ironclaw_loop_support`,
//! which computes the digest from raw output) and the loop mechanics
//! (`ironclaw_agent_loop`, whose `CapabilityCallSignature` reuses the same
//! normalizer) must agree byte-for-byte, and `ironclaw_turns` is the only crate
//! both already depend on. The helpers are pure functions over `serde_json::Value`
//! with no runtime/IO dependencies. Follow-up: if a lower shared crate
//! (`ironclaw_common`) becomes a dependency of `ironclaw_turns`, relocate the
//! implementation there and keep only the newtype contract in this crate.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable digest over normalized, JCS-canonicalized capability output.
///
/// Backed by Blake3 keyed-hash truncated to the first 8 little-endian bytes.
/// The fixed key and truncation mirror agent-loop call-argument signatures so
/// future progress detection can compare outputs without retaining raw content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentDigest(pub u64);

/// Errors that may surface when building a [`ContentDigest`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentDigestError {
    #[error("content contained non-finite number (NaN/Infinity)")]
    NonFiniteNumber,
    #[error("content failed JCS canonicalization: {reason}")]
    CanonicalizationFailed { reason: String },
}

impl ContentDigest {
    pub fn from_json_value(value: &Value) -> Result<Self, ContentDigestError> {
        reject_non_finite_numbers(value)?;
        let normalized = normalize_for_hash(value);
        let canonical = serde_jcs::to_vec(&normalized).map_err(|error| {
            ContentDigestError::CanonicalizationFailed {
                reason: error.to_string(),
            }
        })?;
        let key = [0u8; 32];
        let hash = blake3::keyed_hash(&key, &canonical);
        let bytes = hash.as_bytes();
        let truncated = [
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ];
        Ok(Self(u64::from_le_bytes(truncated)))
    }
}

pub fn normalize_for_hash(value: &Value) -> Value {
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

fn reject_non_finite_numbers(value: &Value) -> Result<(), ContentDigestError> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        Value::Number(number) => {
            if let Some(float) = number.as_f64()
                && !float.is_finite()
            {
                return Err(ContentDigestError::NonFiniteNumber);
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                reject_non_finite_numbers(item)?;
            }
            Ok(())
        }
        Value::Object(object) => {
            for child in object.values() {
                reject_non_finite_numbers(child)?;
            }
            Ok(())
        }
    }
}

fn is_correlation_key(key: &str) -> bool {
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
    replace_ascii_pattern(s, "<timestamp>", match_iso8601_bytes)
}

fn match_iso8601_bytes(bytes: &[u8]) -> Option<usize> {
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
    if consumed < bytes.len() && bytes[consumed] == b'.' {
        let mut j = consumed + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > consumed + 1 {
            consumed = j;
        }
    }
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
    use serde_json::json;

    use super::*;
    use crate::{
        LoopResultRef,
        run_profile::{CapabilityProgress, CapabilityResultMessage},
    };

    #[test]
    fn content_digest_is_deterministic_for_identical_content() {
        let first = json!({"path": "/tmp/demo", "count": 3});
        let second = json!({"count": 3, "path": "/tmp/demo"});

        assert_eq!(
            ContentDigest::from_json_value(&first).expect("digest first"),
            ContentDigest::from_json_value(&second).expect("digest second")
        );
    }

    #[test]
    fn content_digest_normalizes_request_ids_uuids_and_timestamps() {
        let first = json!({
            "request_id": "req-1",
            "trace": "job 550e8400-e29b-41d4-a716-446655440000 at 2026-05-21T10:00:00Z",
            "message": "same"
        });
        let second = json!({
            "request_id": "req-2",
            "trace": "job 11111111-2222-3333-4444-555555555555 at 2026-05-21T11:30:00Z",
            "message": "same"
        });

        assert_eq!(
            ContentDigest::from_json_value(&first).expect("digest first"),
            ContentDigest::from_json_value(&second).expect("digest second")
        );
    }

    #[test]
    fn content_digest_differs_for_genuinely_different_content() {
        let first = json!({"message": "same"});
        let second = json!({"message": "different"});

        assert_ne!(
            ContentDigest::from_json_value(&first).expect("digest first"),
            ContentDigest::from_json_value(&second).expect("digest second")
        );
    }

    #[test]
    fn capability_result_message_without_output_digest_decodes_to_none() {
        let payload = json!({
            "result_ref": "result:legacy",
            "safe_summary": "capability completed",
            "progress": "made_progress",
            "terminate_hint": false,
            "byte_len": 42
        });

        let decoded: CapabilityResultMessage =
            serde_json::from_value(payload).expect("decode legacy result");

        assert_eq!(
            decoded.output_digest, None,
            "legacy result payload without output_digest must decode to None"
        );
        assert_eq!(
            decoded.result_ref,
            LoopResultRef::new("result:legacy").expect("valid result ref")
        );
        assert_eq!(decoded.progress, CapabilityProgress::MadeProgress);
    }
}
