//! Canonical hashing of predicate bucket identities into fixed-width 32-byte
//! digests, shared by every durable predicate-state backend (Postgres, libSQL).
//!
//! These digests are the durable **DB identity contract**: `scope_hash` is the
//! tenant trust boundary / per-tenant LRU-quota grain, and `key_hash` is the
//! dedup + count/sum grain (and table primary key). Defining the derivation
//! once here — rather than copying it into each backend crate — is deliberate:
//! a per-backend copy is drift-prone, and historically the two copies *did*
//! diverge (Postgres used a 4-byte big-endian `u32` length prefix while libSQL
//! used an 8-byte little-endian `u64`, producing different digests for the same
//! logical key). Both backends now delegate here, so the identity contract has
//! exactly one source of truth.
//!
//! - `scope_hash` = blake3 over the length-prefixed `tenant_id`.
//! - `key_hash` = blake3 over a one-byte map discriminant followed by the
//!   length-prefixed bucket identity, so an invocation key and a value key that
//!   share `(hook, tenant, capability)` never collide.
//!
//! Every field is length-prefixed (8-byte big-endian `u64` ++ bytes), which
//! makes the serialization injective: `("ab", "c")` and `("a", "bc")` hash
//! apart, closing the classic concatenation-collision hole. `u64` makes the
//! `usize → prefix` conversion infallible on every supported platform — no
//! lossy `u32::try_from(len).unwrap_or(u32::MAX)` saturation that could alias
//! two pathologically large fields.

use crate::predicate_state::{InvocationKey, ValueKey};

/// Map discriminants folded into `key_hash` so the invocation and value maps
/// can share a table without cross-contaminating dedup.
const KIND_INVOCATION: u8 = b'i';
const KIND_VALUE: u8 = b'v';

/// 32-byte blake3 digest, stored as `BYTEA` (Postgres) / `BLOB` (libSQL).
pub type Digest = [u8; 32];

/// Feed a length-prefixed field into the hasher. The 8-byte big-endian `u64`
/// prefix is infallible from `usize` on all supported platforms and makes the
/// field boundary unambiguous.
fn feed(hasher: &mut blake3::Hasher, field: &[u8]) {
    hasher.update(&(field.len() as u64).to_be_bytes());
    hasher.update(field);
}

/// `scope_hash` for a tenant — the trust boundary and per-tenant LRU-quota
/// grain. blake3 of the length-prefixed `tenant_id`.
pub fn scope_hash(tenant_id: &str) -> Digest {
    let mut hasher = blake3::Hasher::new();
    feed(&mut hasher, tenant_id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// `key_hash` for an invocation-counter bucket. The map discriminant keeps it
/// disjoint from the value table's key space for a shared
/// `(hook, tenant, capability)`.
pub fn invocation_key_hash(key: &InvocationKey) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[KIND_INVOCATION]);
    feed(&mut hasher, key.hook_id.as_bytes());
    feed(&mut hasher, key.tenant_id.as_str().as_bytes());
    feed(&mut hasher, key.capability.as_bytes());
    *hasher.finalize().as_bytes()
}

/// `key_hash` for a numeric-value-sum bucket (invocation components + field).
pub fn value_key_hash(key: &ValueKey) -> Digest {
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
    use crate::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
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
        // because of the one-byte map discriminant.
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
