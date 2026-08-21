//! Shared length-prefixed identity-key encoding plus SHA-256 digest used by
//! the scope and user key types. Each type selects its own fields and namespace
//! prefix. Only the collision-safe framing and digest step are shared here.

use sha2::{Digest, Sha256};

/// Length-prefixes every `(key, value)` pair before concatenating, so e.g.
/// `tenant="a", user="b:c"` can never hash the same as `tenant="a:b", user="c"`
/// — a naive `format!("{tenant}:{user}")` would collide on that boundary.
pub(super) fn encode_parts(parts: &[(&str, String)]) -> String {
    let mut encoded = String::new();
    for (key, value) in parts {
        encoded.push_str(&key.len().to_string());
        encoded.push(':');
        encoded.push_str(key);
        encoded.push('=');
        encoded.push_str(&value.len().to_string());
        encoded.push(':');
        encoded.push_str(value);
        encoded.push(';');
    }
    encoded
}

/// Hex-encoded SHA-256 digest of `raw` (expected to be [`encode_parts`]'s
/// output, though this function itself is encoding-agnostic).
pub(super) fn digest_hex(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefixing_prevents_boundary_collision() {
        let left = encode_parts(&[("tenant", "a".to_string()), ("user", "b:c".to_string())]);
        let right = encode_parts(&[("tenant", "a:b".to_string()), ("user", "c".to_string())]);

        assert_ne!(left, right);
        assert_ne!(digest_hex(&left), digest_hex(&right));
    }
}
