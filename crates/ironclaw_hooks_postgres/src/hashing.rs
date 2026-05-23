//! Canonical hashing of predicate bucket identities into fixed-width
//! BYTEA index keys.
//!
//! Two digests are derived per bucket:
//!
//! - `scope_hash` = blake3 over the length-prefixed `tenant_id`. This is
//!   the trust boundary (one tenant's counters never affect another's)
//!   and the grain at which the distinct-key LRU quota is enforced.
//! - `key_hash` = blake3 over a length-prefixed canonical serialization
//!   of the *whole* bucket identity, including a one-byte map
//!   discriminant so an invocation key and a value key that share
//!   `(hook, tenant, capability)` never collide.
//!
//! Length-prefixing every field (4-byte big-endian length ++ bytes)
//! makes the serialization injective: `("ab", "c")` and `("a", "bc")`
//! produce distinct digests, closing the classic concatenation-collision
//! hole that a naive `a ++ b` would leave open.

use ironclaw_hooks::predicate_state::{InvocationKey, ValueKey};

/// Map discriminants folded into `key_hash` so the invocation and value
/// maps share a table without cross-contaminating dedup.
const KIND_INVOCATION: u8 = b'i';
const KIND_VALUE: u8 = b'v';

/// 32-byte blake3 digest stored as `BYTEA`.
pub(crate) type Digest = [u8; 32];

fn feed(hasher: &mut blake3::Hasher, field: &[u8]) {
    // 4-byte length prefix makes the field boundary unambiguous; u32 is
    // ample for any tenant/capability/field string we will ever see.
    let len = u32::try_from(field.len()).unwrap_or(u32::MAX);
    hasher.update(&len.to_be_bytes());
    hasher.update(field);
}

/// `scope_hash` for a tenant — the LRU-quota and trust grain.
pub(crate) fn scope_hash(tenant_id: &str) -> Digest {
    let mut hasher = blake3::Hasher::new();
    feed(&mut hasher, tenant_id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// `key_hash` for an invocation-counter bucket.
pub(crate) fn invocation_key_hash(key: &InvocationKey) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[KIND_INVOCATION]);
    feed(&mut hasher, key.hook_id.as_bytes());
    feed(&mut hasher, key.tenant_id.as_str().as_bytes());
    feed(&mut hasher, key.capability.as_bytes());
    *hasher.finalize().as_bytes()
}

/// `key_hash` for a numeric-value-sum bucket.
pub(crate) fn value_key_hash(key: &ValueKey) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[KIND_VALUE]);
    feed(&mut hasher, key.hook_id.as_bytes());
    feed(&mut hasher, key.tenant_id.as_str().as_bytes());
    feed(&mut hasher, key.capability.as_bytes());
    feed(&mut hasher, key.field.as_bytes());
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
    use ironclaw_host_api::TenantId;

    fn hook() -> HookId {
        HookId::derive(
            &ExtensionId::new("ext").unwrap(),
            "1.0",
            &HookLocalId::new("h").unwrap(),
            HookVersion::ONE,
        )
    }

    fn inv(tenant: &str, capability: &str) -> InvocationKey {
        InvocationKey {
            hook_id: hook(),
            tenant_id: TenantId::new(tenant).unwrap(),
            capability: capability.to_string(),
        }
    }

    fn val(tenant: &str, capability: &str, field: &str) -> ValueKey {
        ValueKey {
            hook_id: hook(),
            tenant_id: TenantId::new(tenant).unwrap(),
            capability: capability.to_string(),
            field: field.to_string(),
        }
    }

    #[test]
    fn distinct_tenants_have_distinct_scope_hashes() {
        assert_ne!(scope_hash("alpha"), scope_hash("beta"));
    }

    #[test]
    fn invocation_and_value_keys_never_collide() {
        // Same hook/tenant/capability across the two maps must hash apart
        // because of the map discriminant.
        let i = invocation_key_hash(&inv("t", "cap.x"));
        let v = value_key_hash(&val("t", "cap.x", "cap.x"));
        assert_ne!(i, v);
    }

    #[test]
    fn field_boundary_is_injective() {
        // Length-prefixing prevents ("ab","c") and ("a","bc") aliasing.
        let a = value_key_hash(&val("t", "ab", "c"));
        let b = value_key_hash(&val("t", "a", "bc"));
        assert_ne!(a, b);
    }

    #[test]
    fn capability_boundary_is_injective_for_invocations() {
        let a = invocation_key_hash(&inv("t", "abc"));
        let b = invocation_key_hash(&inv("tabc", ""));
        assert_ne!(a, b);
    }
}
