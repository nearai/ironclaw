use std::hash::Hasher;

use ironclaw_host_api::CapabilityId;
use siphasher::sip::SipHasher24;

// stable across Rust releases; do NOT change without bumping CHECKPOINT_SCHEMA_ID
const SIP_HASH_KEY: [u8; 16] = [0u8; 16];

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCallSignature {
    pub name: CapabilityId,
    pub args_hash: ArgsHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ArgsHash(pub u64);

impl CapabilityCallSignature {
    /// Builds a non-cryptographic signature from a capability id and JSON args.
    ///
    /// Collisions are tolerated because this is only a heuristic identity for
    /// no-progress detection; authorization and execution must use the original
    /// typed invocation data.
    pub fn from_call(name: CapabilityId, args: &serde_json::Value) -> Self {
        let mut canonical = String::new();
        canonicalize(args, &mut canonical);
        let mut hasher = SipHasher24::new_with_key(&SIP_HASH_KEY);
        hasher.write(canonical.as_bytes());
        Self {
            name,
            args_hash: ArgsHash(hasher.finish()),
        }
    }
}

fn canonicalize(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {
            out.push_str(&value.to_string())
        }
        serde_json::Value::Number(n) => {
            // Normalize so semantically equal numbers hash identically.
            // f64 Display is stable: 1, 1.0, 1e3 → "1", "1", "1000"; 1.5 → "1.5".
            // Falls back to the raw form for ints outside f64 precision (rare in
            // tool args) so callers never see a panic on oversized integers.
            match n.as_f64() {
                Some(f) => out.push_str(&f.to_string()),
                None => out.push_str(&n.to_string()),
            }
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonicalize(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(object) => {
            out.push('{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::Value::String(key.clone()).to_string());
                out.push(':');
                if let Some(child) = object.get(key) {
                    canonicalize(child, out);
                }
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::CapabilityId;
    use serde_json::json;

    use super::*;

    #[test]
    fn capability_call_signature_hash_is_stable_across_rust_releases() {
        let signature = CapabilityCallSignature::from_call(
            CapabilityId::new("demo.echo").unwrap(),
            &json!({"b": 2, "a": {"d": false, "c": [1, null]}}),
        );

        assert_eq!(signature.args_hash, ArgsHash(13_286_400_333_242_753_100));
    }

    #[test]
    fn capability_call_signature_normalizes_int_and_float_forms() {
        let name = CapabilityId::new("demo.echo").unwrap();
        let int_form = CapabilityCallSignature::from_call(name.clone(), &json!({"x": 1}));
        let float_form = CapabilityCallSignature::from_call(name, &json!({"x": 1.0}));

        assert_eq!(int_form.args_hash, float_form.args_hash);
    }

    #[test]
    fn capability_call_signature_normalizes_scientific_notation() {
        let name = CapabilityId::new("demo.echo").unwrap();
        let int_form = CapabilityCallSignature::from_call(name.clone(), &json!({"x": 1}));
        let exp_form = CapabilityCallSignature::from_call(name, &json!({"x": 1e0}));

        assert_eq!(int_form.args_hash, exp_form.args_hash);
    }
}
