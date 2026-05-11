//! Shared helpers for opaque, model-visible snippet references.
//!
//! These hashes are deterministic display identifiers only. They are unkeyed and
//! not collision-resistant; callers must never use them for authorization,
//! tenancy checks, or backend lookup.

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001B3;
const FIELD_SEPARATOR: u8 = 0xFF;

/// Compute a stable opaque display hash over ordered string fields.
///
/// This intentionally uses FNV-1a for short deterministic model-facing refs,
/// not security. Field separators prevent simple concatenation drift between
/// independent call sites.
pub fn stable_snippet_display_hash<'a>(fields: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut hash = FNV_OFFSET;
    for field in fields {
        feed_hash(&mut hash, field.as_bytes());
        feed_hash(&mut hash, &[FIELD_SEPARATOR]);
    }
    hash
}

fn feed_hash(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_hash_is_deterministic_and_field_ordered() {
        let first = stable_snippet_display_hash(["skill:alpha", "summary", "0"]);
        let second = stable_snippet_display_hash(["skill:alpha", "summary", "0"]);
        let different = stable_snippet_display_hash(["skill:alpha", "0", "summary"]);

        assert_eq!(first, second);
        assert_ne!(first, different);
    }

    #[test]
    fn display_hash_separates_fields() {
        assert_ne!(
            stable_snippet_display_hash(["ab", "c"]),
            stable_snippet_display_hash(["a", "bc"]),
        );
    }
}
