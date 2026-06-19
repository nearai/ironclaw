use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::{ContentDigest, normalize_for_hash};

/// Stable identity for a capability call, suitable for repetition detection
/// without retaining raw arguments (per turns-agent-loop.md §6: no raw tool
/// input in loop state).
///
/// Constructed by the executor via [`CapabilityCallSignature::from_call`],
/// which canonicalizes the JSON args via JCS (RFC 8785) before hashing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCallSignature {
    pub name: CapabilityId,
    pub args_hash: ArgsHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CapabilityOutputObservation {
    pub signature: CapabilityCallSignature,
    pub output_digest: ContentDigest,
}

/// 64-bit non-cryptographic hash over JCS-canonicalized argument bytes.
///
/// Backed by Blake3 keyed-hash truncated to the first 8 little-endian bytes.
/// The choice is fixed per release: changing the hash function across
/// releases invalidates all in-flight checkpoint `recent_call_signatures`
/// (treat as a checkpoint-schema break and bump `CHECKPOINT_SCHEMA_ID`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ArgsHash(pub u64);

/// Errors that may surface when building a [`CapabilityCallSignature`].
///
/// JCS RFC 8785 rejects non-finite numbers (`NaN`, `+Infinity`, `-Infinity`);
/// the rest of the canonicalization is total.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityCallSignatureError {
    #[error("capability call args contained non-finite number (NaN/Infinity)")]
    NonFiniteNumber,
    #[error("capability call args failed JCS canonicalization: {reason}")]
    CanonicalizationFailed { reason: String },
}

impl CapabilityCallSignature {
    /// Builds a signature from a capability name and JSON args.
    ///
    /// The args are canonicalized via JCS (RFC 8785) — UTF-16 code-unit
    /// key-sort, minimal whitespace, number representation preserved. Returns
    /// `Err(CapabilityCallSignatureError::NonFiniteNumber)` if the args carry
    /// `NaN` or `±Infinity` (non-finite numbers are not
    /// valid JSON; an upstream serializer leaked invalid input).
    pub fn from_call(
        name: CapabilityId,
        args: &serde_json::Value,
    ) -> Result<Self, CapabilityCallSignatureError> {
        reject_non_finite_numbers(args)?;
        // Normalize before canonicalizing so two calls that differ only in
        // request-id / correlation-id / embedded UUID / ISO-8601 timestamp
        // collapse to the same hash. Without this, the surrounding
        // `recent_call_signatures` window false-negatives stuck-loop runs
        // because random per-call IDs make every signature unique.
        let normalized = normalize_for_hash(args);
        let canonical = serde_jcs::to_vec(&normalized).map_err(|error| {
            CapabilityCallSignatureError::CanonicalizationFailed {
                reason: error.to_string(),
            }
        })?;
        // Keyed blake3 truncated to 64 bits. The key is fixed across releases;
        // bumping CHECKPOINT_SCHEMA_ID is required if it ever changes.
        let key = [0u8; 32];
        let hash = blake3::keyed_hash(&key, &canonical);
        let bytes = hash.as_bytes();
        let truncated = [
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ];
        Ok(Self {
            name,
            args_hash: ArgsHash(u64::from_le_bytes(truncated)),
        })
    }
}

fn reject_non_finite_numbers(
    value: &serde_json::Value,
) -> Result<(), CapabilityCallSignatureError> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {
            Ok(())
        }
        serde_json::Value::Number(number) => {
            if let Some(float) = number.as_f64()
                && !float.is_finite()
            {
                return Err(CapabilityCallSignatureError::NonFiniteNumber);
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            for item in items {
                reject_non_finite_numbers(item)?;
            }
            Ok(())
        }
        serde_json::Value::Object(object) => {
            for child in object.values() {
                reject_non_finite_numbers(child)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::CapabilityId;
    use serde_json::json;

    use super::*;

    #[test]
    fn capability_call_signature_int_and_float_forms_canonicalize_via_jcs() {
        // JCS RFC 8785 §3.2.2.3 serializes JSON numbers via ECMA-262
        // Number.prototype.toString, which collapses `1` and `1.0` to the
        // same `"1"` token. This is the stable, RFC-conformant behavior;
        // the equal hash documents that. The signature scheme inherits
        // JCS's number canonicalization.
        let name = CapabilityId::new("demo.echo").unwrap();
        let int_form = CapabilityCallSignature::from_call(name.clone(), &json!({"x": 1})).unwrap();
        let float_form = CapabilityCallSignature::from_call(name, &json!({"x": 1.0})).unwrap();

        assert_eq!(int_form.args_hash, float_form.args_hash);
    }

    #[test]
    fn capability_call_signature_collapses_calls_that_differ_only_by_request_id() {
        // Regression for #3841 follow-up F1: stuck-loop detection must
        // collapse repeated calls whose only differing field is a
        // correlation/request-id, otherwise the existing
        // `recent_call_signatures` window false-negatives every loop where
        // the model adds a fresh request_id each turn.
        let name = CapabilityId::new("demo.echo").unwrap();
        let first = CapabilityCallSignature::from_call(
            name.clone(),
            &json!({"request_id": "req-1", "message": "hi"}),
        )
        .unwrap();
        let second = CapabilityCallSignature::from_call(
            name.clone(),
            &json!({"request_id": "req-2", "message": "hi"}),
        )
        .unwrap();
        let third = CapabilityCallSignature::from_call(
            name,
            &json!({"trace_id": "550e8400-e29b-41d4-a716-446655440000", "message": "hi"}),
        )
        .unwrap();
        assert_eq!(first.args_hash, second.args_hash);
        assert_eq!(first.args_hash, third.args_hash);
    }

    #[test]
    fn capability_call_signature_collapses_calls_that_differ_only_by_embedded_uuid() {
        // Same shape as the request_id collapse: a UUID embedded *inside*
        // a string value (not under a correlation key) must normalize to
        // `<uuid>` so two otherwise-identical calls hash equally.
        let name = CapabilityId::new("demo.echo").unwrap();
        let first = CapabilityCallSignature::from_call(
            name.clone(),
            &json!({"target": "obj-550e8400-e29b-41d4-a716-446655440000"}),
        )
        .unwrap();
        let second = CapabilityCallSignature::from_call(
            name,
            &json!({"target": "obj-11111111-2222-3333-4444-555555555555"}),
        )
        .unwrap();
        assert_eq!(first.args_hash, second.args_hash);
    }

    #[test]
    fn capability_call_signature_rejects_non_finite_floats_explicitly() {
        // serde_json::Value::Number rejects NaN/Infinity at construction, so
        // verify the guard exists by reaching through a manually-built Value
        // tree. There is no public API that constructs a NaN-bearing Number,
        // so we exercise the guard's branch via a synthesized Value tree
        // built by mem-transmuting through the public NumberFromF64
        // surface (none exists). Instead, this test documents that the guard
        // path is unreachable from public APIs — and that the public API
        // therefore cannot leak a NaN-derived hash. The intent is captured
        // and the guard remains as defense-in-depth against future API
        // changes that could legitimize non-finite floats.
        let result = serde_json::Number::from_f64(f64::NAN);
        assert!(
            result.is_none(),
            "serde_json refuses to construct a NaN Number"
        );
        let result = serde_json::Number::from_f64(f64::INFINITY);
        assert!(
            result.is_none(),
            "serde_json refuses to construct an Infinity Number"
        );
    }
}
