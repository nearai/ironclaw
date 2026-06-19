//! Shared helpers for opaque, model-visible snippet references.
//!
//! These hashes are deterministic display identifiers only. They are unkeyed,
//! not collision-resistant, and not a secrecy boundary; callers must never use
//! them for authorization, tenancy checks, or backend lookup.

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001B3;
const FIELD_SEPARATOR: u8 = 0xFF;

/// Build a stable opaque memory-snippet display reference over ordered string fields.
///
/// This intentionally uses FNV-1a for short deterministic model-facing refs,
/// not security. Field separators prevent simple concatenation drift between
/// independent call sites. It preserves the legacy memory-ref layout, which
/// appends a separator after every field.
pub fn memory_snippet_display_ref<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    let hash = stable_memory_snippet_display_hash(fields);
    format!("memory-snippet:{hash:016x}")
}

fn stable_memory_snippet_display_hash<'a>(fields: impl IntoIterator<Item = &'a str>) -> u64 {
    stable_snippet_display_hash_with_layout(fields, SeparatorLayout::Trailing)
}

/// Compute legacy skill-snippet display hash semantics.
///
/// Skill refs existed before the shared helper. Their field separator appeared
/// only between fields, so centralization must preserve those model-visible
/// refs instead of rotating them. Keep crate-private so external callers cannot
/// select the wrong legacy layout for new refs.
pub(crate) fn stable_skill_snippet_display_hash<'a>(
    fields: impl IntoIterator<Item = &'a str>,
) -> u64 {
    stable_snippet_display_hash_with_layout(fields, SeparatorLayout::BetweenFields)
}

#[derive(Clone, Copy)]
enum SeparatorLayout {
    Trailing,
    BetweenFields,
}

fn stable_snippet_display_hash_with_layout<'a>(
    fields: impl IntoIterator<Item = &'a str>,
    layout: SeparatorLayout,
) -> u64 {
    let fields: Vec<&str> = fields.into_iter().collect();
    let mut hash = FNV_OFFSET;
    for (index, field) in fields.iter().enumerate() {
        feed_hash(&mut hash, field.as_bytes());
        let should_append_separator = match layout {
            SeparatorLayout::Trailing => true,
            SeparatorLayout::BetweenFields => index + 1 < fields.len(),
        };
        if should_append_separator {
            feed_hash(&mut hash, &[FIELD_SEPARATOR]);
        }
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
        let first = stable_memory_snippet_display_hash(["skill:alpha", "summary", "0"]);
        let second = stable_memory_snippet_display_hash(["skill:alpha", "summary", "0"]);
        let different = stable_memory_snippet_display_hash(["skill:alpha", "0", "summary"]);

        assert_eq!(first, second);
        assert_ne!(first, different);
    }

    #[test]
    fn display_hash_separates_fields() {
        assert_ne!(
            stable_memory_snippet_display_hash(["ab", "c"]),
            stable_memory_snippet_display_hash(["a", "bc"]),
        );
    }

    #[test]
    fn skill_display_hash_preserves_existing_model_visible_refs() {
        assert_eq!(
            stable_skill_snippet_display_hash(["skill:alpha", "summary", "0"]),
            0x6e54cb74d742607c
        );
    }

    #[test]
    fn memory_display_ref_preserves_trailing_separator_layout() {
        assert_eq!(
            memory_snippet_display_ref(["skill:alpha", "summary", "0"]),
            "memory-snippet:bc763a89c5c9fe99"
        );
    }
}
