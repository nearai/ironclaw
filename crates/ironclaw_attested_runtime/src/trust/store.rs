//! The [`TrustedSignerBinding`] record + the [`TrustStore`] abstraction and its
//! in-memory implementation.
//!
//! A [`TrustedSignerBinding`] is the active "this account/key is a trusted
//! signer for this `(tenant, user, chain, network)`" record. It is the *only*
//! thing the raise side reads from this subsystem: [`TrustStore::lookup_active_binding`]
//! returns it (or `None`, in which case the caller fails closed). The binding
//! says nothing about *which* operation may be signed — it is **enrollment
//! evidence**, strictly separate from per-gate signing authorization (gates
//! still pin the exact payload + one-shot grant + tenant policy at resolve).
//!
//! Storage is in-memory here. The durable PG / libSQL implementation is a
//! gap-D follow-up; it must implement [`TrustStore`] with identical
//! tenant-first keying and dual-backend support.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_signing_provider::{ChainId, TenantId, UserId};

use super::enrollment::TrustEnrollment;

/// Lifecycle status of a [`TrustedSignerBinding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    /// The trusted signer is active and resolvable by the raise side.
    Active,
    /// Explicitly revoked by the user / tenant; never resolves.
    Revoked,
    /// Past its expiry; never resolves.
    Expired,
}

/// The tenant-first key uniquely identifying a trusted-signer slot.
///
/// One active binding per `(tenant, user, chain, network)`. Keyed tenant-first
/// so a binding for tenant A can never resolve for tenant B (tenant isolation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindingKey {
    /// Tenant boundary (first, for isolation).
    pub tenant_id: TenantId,
    /// End user within the tenant.
    pub user_id: UserId,
    /// Target chain.
    pub chain_id: ChainId,
    /// Network within the chain family.
    pub network: String,
}

/// The active trusted-signer record for `(tenant, user, chain, network)`.
///
/// Carries the proven account/key, the binding evidence hash (the challenge
/// digest hex that was signed), its status, and expiry. No secret material —
/// only the public account/key and an evidence hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedSignerBinding {
    /// Tenant boundary.
    pub tenant_id: TenantId,
    /// End user.
    pub user_id: UserId,
    /// Target chain.
    pub chain_id: ChainId,
    /// Network within the chain family.
    pub network: String,
    /// The proven trusted account (EVM address / Solana pubkey / NEAR account).
    pub account_or_key: String,
    /// For NEAR, the specific ed25519 access-key public key proven (hex);
    /// `None` for EVM/Solana where the account *is* the signer identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key: Option<String>,
    /// Hash of the binding evidence (the signed challenge digest, hex). Proves
    /// a ceremony happened without storing any signature/secret material.
    pub evidence_hash: String,
    /// Lifecycle status.
    pub status: BindingStatus,
    /// Creation time (unix millis).
    pub created_at_unix_ms: u64,
    /// Optional expiry (unix millis). `None` = no expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
    /// When the binding was revoked (unix millis); `None` while not revoked.
    /// Set when `status` transitions to [`BindingStatus::Revoked`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at_unix_ms: Option<u64>,
}

impl TrustedSignerBinding {
    /// The tenant-first key this binding is stored under.
    pub fn key(&self) -> BindingKey {
        BindingKey {
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            chain_id: self.chain_id.clone(),
            network: self.network.clone(),
        }
    }

    /// True iff the binding is `Active` and not past its expiry at `now`.
    pub fn is_resolvable(&self, now_unix_ms: u64) -> bool {
        self.status == BindingStatus::Active
            && self.expires_at_unix_ms.is_none_or(|exp| now_unix_ms < exp)
    }
}

/// Persistence abstraction for trust enrollments and the bindings they produce.
///
/// In-memory now ([`InMemoryTrustStore`]); durable PG / libSQL is a gap-D
/// follow-up carrying identical tenant-first keying and dual-backend support.
#[async_trait]
pub trait TrustStore: Send + Sync {
    /// Persist (insert or update) an enrollment, keyed by its stable
    /// idempotency key. Re-initiating with the same key resumes the same
    /// ceremony.
    async fn put_enrollment(&self, enrollment: TrustEnrollment);

    /// Atomically insert `candidate` only if no enrollment yet exists for its
    /// idempotency key; return whatever enrollment now occupies that slot.
    ///
    /// This is the idempotency-race primitive for `initiate_registration`: two
    /// concurrent initiations for the same `(tenant, user, chain, network,
    /// account)` both call this with their own freshly-minted candidate, and
    /// exactly one wins. The loser receives the winner's enrollment (and thus
    /// its challenge) instead of silently overwriting it — so neither client is
    /// handed a challenge that can never complete. The returned `bool` is `true`
    /// iff *this* call inserted `candidate`.
    ///
    /// Durable backends MUST implement this as an `INSERT ... ON CONFLICT DO
    /// NOTHING` + read-back (or equivalent), not a check-then-write.
    async fn put_enrollment_if_absent(&self, candidate: TrustEnrollment)
    -> (TrustEnrollment, bool);

    /// Atomically transition the enrollment identified by `enrollment_id` from
    /// the `expected` state to the `updated` enrollment, but only if its current
    /// state still equals `expected`. Returns `true` iff the swap happened.
    ///
    /// This is the single-use / replay-defense primitive for
    /// `complete_registration`: the completer first claims the `Challenged`
    /// enrollment via this CAS before doing any verification, so two concurrent
    /// completions of the same challenge can never both proceed — the loser sees
    /// the state has already moved and fails closed (`NotChallengeable`).
    ///
    /// Durable backends MUST implement this as a conditional `UPDATE ... WHERE
    /// state = $expected` and report the affected-row count, not a
    /// read-then-write.
    async fn compare_and_swap_enrollment_state(
        &self,
        enrollment_id: &str,
        expected: super::EnrollmentState,
        updated: TrustEnrollment,
    ) -> bool;

    /// Read an enrollment by its idempotency key.
    async fn get_enrollment(&self, idempotency_key: &str) -> Option<TrustEnrollment>;

    /// Read an enrollment by its server-issued enrollment id.
    async fn get_enrollment_by_id(&self, enrollment_id: &str) -> Option<TrustEnrollment>;

    /// Persist (insert or replace) an active binding under its tenant-first key.
    async fn put_binding(&self, binding: TrustedSignerBinding);

    /// Read the raw binding for a key, regardless of status.
    async fn get_binding(&self, key: &BindingKey) -> Option<TrustedSignerBinding>;

    /// The raise-side seam: the *active, unexpired* trusted-signer binding for
    /// `(tenant, user, chain, network)`, or `None`.
    ///
    /// `now_unix_ms` is supplied by the caller so expiry is deterministic and
    /// the store stays clock-free.
    ///
    /// follow-up (gap D / binding-fields): the gate raise will consume this to
    /// pin `expected_signer` / `expected_access_key` / `expected_signing_payload`
    /// onto the `AttestedGateBinding`. A `None` result means the caller MUST
    /// fail closed — never fabricate a signer.
    async fn lookup_active_binding(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        chain_id: &ChainId,
        network: &str,
        now_unix_ms: u64,
    ) -> Option<TrustedSignerBinding>;
}

/// In-memory [`TrustStore`].
#[derive(Default)]
pub struct InMemoryTrustStore {
    enrollments: Mutex<HashMap<String, TrustEnrollment>>,
    bindings: Mutex<HashMap<BindingKey, TrustedSignerBinding>>,
}

impl InMemoryTrustStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TrustStore for InMemoryTrustStore {
    async fn put_enrollment(&self, enrollment: TrustEnrollment) {
        match self.enrollments.lock() {
            Ok(mut map) => {
                map.insert(enrollment.idempotency_key.clone(), enrollment);
            }
            // A poisoned mutex means another task panicked mid-write. Silently
            // dropping the enrollment would let the ceremony proceed as if it
            // were persisted; surface it loudly instead (a system-failure
            // signal, per the repo logging rules).
            Err(_) => tracing::error!(
                enrollment_id = %enrollment.enrollment_id,
                "trust store enrollments mutex poisoned; enrollment write dropped"
            ),
        }
    }

    async fn put_enrollment_if_absent(
        &self,
        candidate: TrustEnrollment,
    ) -> (TrustEnrollment, bool) {
        // The whole get-or-insert is one critical section so concurrent
        // initiations for the same idempotency key can never both miss.
        match self.enrollments.lock() {
            Ok(mut map) => {
                use std::collections::hash_map::Entry;
                match map.entry(candidate.idempotency_key.clone()) {
                    Entry::Occupied(e) => (e.get().clone(), false),
                    Entry::Vacant(e) => (e.insert(candidate).clone(), true),
                }
            }
            // Poisoned lock: do not fabricate a winner — report the candidate as
            // not inserted so the caller treats it as a non-authoritative retry.
            Err(_) => (candidate, false),
        }
    }

    async fn compare_and_swap_enrollment_state(
        &self,
        enrollment_id: &str,
        expected: super::EnrollmentState,
        updated: TrustEnrollment,
    ) -> bool {
        // Read current state, compare, and write the update under one lock so
        // the transition is atomic with respect to other completers.
        match self.enrollments.lock() {
            Ok(mut map) => {
                let current = map.values().find(|e| e.enrollment_id == enrollment_id);
                match current {
                    Some(e) if e.state == expected => {
                        map.insert(updated.idempotency_key.clone(), updated);
                        true
                    }
                    _ => false,
                }
            }
            Err(_) => false,
        }
    }

    async fn get_enrollment(&self, idempotency_key: &str) -> Option<TrustEnrollment> {
        // Fail closed on a poisoned mutex, but surface the system fault loudly
        // like the write paths rather than collapsing it silently to `None`.
        match self.enrollments.lock() {
            Ok(map) => map.get(idempotency_key).cloned(),
            Err(_) => {
                tracing::error!(
                    idempotency_key,
                    "trust store enrollments mutex poisoned; enrollment read returning None"
                );
                None
            }
        }
    }

    async fn get_enrollment_by_id(&self, enrollment_id: &str) -> Option<TrustEnrollment> {
        match self.enrollments.lock() {
            Ok(map) => map
                .values()
                .find(|e| e.enrollment_id == enrollment_id)
                .cloned(),
            Err(_) => {
                tracing::error!(
                    enrollment_id,
                    "trust store enrollments mutex poisoned; enrollment-by-id read returning None"
                );
                None
            }
        }
    }

    async fn put_binding(&self, binding: TrustedSignerBinding) {
        match self.bindings.lock() {
            Ok(mut map) => {
                map.insert(binding.key(), binding);
            }
            // See `put_enrollment`: never silently drop a binding write on a
            // poisoned mutex — a lost binding would fail-close every later
            // raise-side lookup with no trace of why.
            Err(_) => tracing::error!(
                tenant_id = %binding.tenant_id.as_str(),
                "trust store bindings mutex poisoned; binding write dropped"
            ),
        }
    }

    async fn get_binding(&self, key: &BindingKey) -> Option<TrustedSignerBinding> {
        match self.bindings.lock() {
            Ok(map) => map.get(key).cloned(),
            Err(_) => {
                tracing::error!(
                    tenant_id = %key.tenant_id.as_str(),
                    "trust store bindings mutex poisoned; binding read returning None"
                );
                None
            }
        }
    }

    async fn lookup_active_binding(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        chain_id: &ChainId,
        network: &str,
        now_unix_ms: u64,
    ) -> Option<TrustedSignerBinding> {
        let key = BindingKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            chain_id: chain_id.clone(),
            network: network.to_string(),
        };
        match self.bindings.lock() {
            Ok(map) => map
                .get(&key)
                .cloned()
                .filter(|b| b.is_resolvable(now_unix_ms)),
            Err(_) => {
                tracing::error!(
                    tenant_id = %tenant_id.as_str(),
                    "trust store bindings mutex poisoned; active-binding lookup returning None"
                );
                None
            }
        }
    }
}
