//! Shared hashing helpers.
//!
//! Generic SHA-256 utilities used across subsystems (PKCE verifier hashes,
//! opaque-state and authorization-code hashes). No product semantics.

use sha2::{Digest, Sha256};

/// Lowercase hex of `SHA-256(bytes)`. Hashes opaque values for storage/lookup
/// without retaining the raw input.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_hex_never_prefixes_output() {
        assert!(!sha256_hex(b"anything").starts_with("sha256:"));
    }
}
