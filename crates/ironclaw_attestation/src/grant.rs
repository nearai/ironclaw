//! Sealed one-shot signing grant store — the authorization + anti-replay
//! primitive of the attested-signing substrate.
//!
//! An [`ApprovedTxHash`] (PR2) is a *binding*, not an authorization. The thing
//! that actually authorizes a single signing operation is a
//! [`AttestedSigningGrant`] that has been **sealed** and then **claimed
//! exactly once**. The store enforces the one-shot property: the first
//! [`SealedGrantStore::claim`] of a sealed grant wins and atomically marks it
//! claimed; every subsequent claim of that same grant fails with
//! [`GrantError::AlreadyClaimed`]. This is the core anti-replay / anti-double-
//! sign guard — even if an approved hash leaks, it cannot be turned into a
//! second signature.
//!
//! Durable PG / libSQL backends are stacked follow-ups; they must pass the
//! canonical [`sealed_grant_store_contract_cases!`] suite to prove identical
//! semantics (the same split the predicate-state backend used: in-memory here,
//! durable backends behind the contract macro).

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use ironclaw_signing_provider::{
    ApprovedTxHash, ChainId, GateRef, KeyOrAccountId, RunId, SigningContext, TenantId, UserId,
};

/// The composite identity of a signing grant.
///
/// Two requests are "the same grant" iff every component matches. The key
/// deliberately omits `scope`/`actor` from [`SigningContext`]: a grant is
/// keyed by *who* (`tenant`, `user`), *which run* (`run_id`), *which gate*
/// (`gate_ref`), *exactly what* (`approved_tx_hash`), and *with which key on
/// which chain* (`key_or_account_id`, `chain_id`). Any mismatch in these
/// components is a different grant and therefore [`GrantError::NotFound`] on
/// claim.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GrantKey {
    /// Tenant boundary.
    pub tenant: TenantId,
    /// End user.
    pub user: UserId,
    /// Owning run.
    pub run_id: RunId,
    /// Gate the flow is blocked on.
    pub gate_ref: GateRef,
    /// The binding hash of the approved transaction.
    pub approved_tx_hash: ApprovedTxHash,
    /// Signing key or account.
    pub key_or_account_id: KeyOrAccountId,
    /// Target chain / network.
    pub chain_id: ChainId,
}

impl GrantKey {
    /// Derive a grant key from a [`SigningContext`] and the approved-tx hash.
    ///
    /// `scope` and `actor` are intentionally not part of the key (see the type
    /// docs).
    pub fn from_context(ctx: &SigningContext, approved_tx_hash: ApprovedTxHash) -> Self {
        Self {
            tenant: ctx.tenant.clone(),
            user: ctx.user.clone(),
            run_id: ctx.run_id.clone(),
            gate_ref: ctx.gate_ref.clone(),
            approved_tx_hash,
            key_or_account_id: ctx.key_or_account_id.clone(),
            chain_id: ctx.chain_id.clone(),
        }
    }
}

/// Lifecycle status of a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    /// Sealed and awaiting its single claim.
    Sealed,
    /// Already claimed — any further claim must fail one-shot.
    Claimed,
}

/// A sealed authorization for a single signing operation.
///
/// Construct via [`AttestedSigningGrant::new`], hand to
/// [`SealedGrantStore::seal`], and consume exactly once via
/// [`SealedGrantStore::claim`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestedSigningGrant {
    /// Composite identity of the grant.
    pub key: GrantKey,
    /// Current lifecycle status.
    pub status: GrantStatus,
    /// Creation timestamp (unix millis), set by the sealer. Opaque clock at
    /// this layer; the durable backends carry the same value.
    pub created_at_ms: i64,
    /// Optional expiry (unix millis). `None` means no expiry. Enforcement of
    /// expiry on claim is a durable-backend concern noted for later PRs; the
    /// field is carried now so the wire shape is stable.
    pub expiry_ms: Option<i64>,
}

impl AttestedSigningGrant {
    /// Construct a fresh grant in [`GrantStatus::Sealed`].
    ///
    /// Named `new` (not `seal`) so it does not collide with the store-level verb
    /// [`SealedGrantStore::seal`]: `store.seal(AttestedSigningGrant::new(..))`
    /// reads as construct-then-persist rather than nesting two `seal` calls.
    ///
    /// Validates the timestamps fail-closed: `created_at_ms` must be
    /// non-negative (a negative/pre-epoch timestamp is rejected so downstream
    /// TTL math cannot under/overflow), and any `expiry_ms` must be strictly
    /// after `created_at_ms` (an expiry at or before creation is already-expired
    /// and never claimable).
    pub fn new(
        key: GrantKey,
        created_at_ms: i64,
        expiry_ms: Option<i64>,
    ) -> Result<Self, GrantError> {
        if created_at_ms < 0 {
            return Err(GrantError::InvalidTimestamp { created_at_ms });
        }
        if let Some(exp) = expiry_ms
            && exp <= created_at_ms
        {
            return Err(GrantError::InvalidExpiry {
                created_at_ms,
                expiry_ms: exp,
            });
        }
        Ok(Self {
            key,
            status: GrantStatus::Sealed,
            created_at_ms,
            expiry_ms,
        })
    }
}

/// The successfully-claimed grant returned by [`SealedGrantStore::claim`].
///
/// Holding one of these is the proof that *this* caller won the one-shot CAS
/// and is authorized to perform exactly one signing for the bound key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedGrant {
    /// The grant identity that was claimed.
    pub key: GrantKey,
    /// Creation timestamp carried over from the sealed grant.
    pub created_at_ms: i64,
    /// Optional expiry (unix millis) carried over from the sealed grant, so the
    /// downstream signing layer can independently re-check the time window as
    /// defence-in-depth even before durable backends enforce expiry on claim.
    pub expiry_ms: Option<i64>,
}

/// Errors a [`SealedGrantStore`] can surface.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GrantError {
    /// `seal` was called for a key that is already sealed.
    #[error("grant already sealed for this key")]
    AlreadySealed,

    /// `claim` was called for a key that was never sealed (or whose components
    /// do not match any sealed grant).
    #[error("no sealed grant found for this key")]
    NotFound,

    /// `claim` lost the one-shot race: the grant was already claimed.
    #[error("grant already claimed (one-shot)")]
    AlreadyClaimed,

    /// `new` was given a negative (pre-epoch) `created_at_ms`.
    #[error("grant created_at_ms must be non-negative, got {created_at_ms}")]
    InvalidTimestamp {
        /// The offending creation timestamp.
        created_at_ms: i64,
    },

    /// `new` was given an `expiry_ms` at or before `created_at_ms` — the grant
    /// would be born already-expired.
    #[error("grant expiry_ms {expiry_ms} must be after created_at_ms {created_at_ms}")]
    InvalidExpiry {
        /// Creation timestamp.
        created_at_ms: i64,
        /// The offending expiry timestamp.
        expiry_ms: i64,
    },

    /// A backend-internal failure with an opaque description.
    #[error("grant store error: {reason}")]
    Backend {
        /// Human-readable description of the backend failure.
        reason: String,
    },
}

/// Sealed one-shot signing-grant store.
///
/// `claim` MUST be an atomic, one-shot compare-and-set: the seal-check and the
/// mark-claimed happen in a single critical section so that under concurrent
/// claims exactly one caller observes `Sealed` and transitions it to
/// `Claimed`; all others observe `Claimed` and fail.
#[async_trait]
pub trait SealedGrantStore: Send + Sync {
    /// Persist a sealed grant. Fails with [`GrantError::AlreadySealed`] if a
    /// grant with the same key already exists in EITHER lifecycle status
    /// ([`GrantStatus::Sealed`] or [`GrantStatus::Claimed`]) — sealing is
    /// one-shot per key for the lifetime of that key.
    ///
    /// In particular, re-sealing a key that has already been *claimed* also
    /// returns [`GrantError::AlreadySealed`] (NOT [`GrantError::NotFound`]): the
    /// key is permanently spent, and "already sealed" is the canonical "this key
    /// is not available for sealing" signal. Durable backends that archive
    /// claimed rows separately MUST still consult that history so a re-seal of a
    /// claimed key surfaces `AlreadySealed`, matching the in-memory store.
    async fn seal(&self, grant: AttestedSigningGrant) -> Result<(), GrantError>;

    /// Atomically claim a sealed grant exactly once.
    ///
    /// * First claim of a sealed key -> `Ok(ClaimedGrant)`, key now `Claimed`.
    /// * Any later claim of that key -> `Err(GrantError::AlreadyClaimed)`.
    /// * Claim of a never-sealed (or mismatched) key -> `Err(GrantError::NotFound)`.
    async fn claim(&self, key: &GrantKey) -> Result<ClaimedGrant, GrantError>;
}

/// In-memory [`SealedGrantStore`].
///
/// The single [`Mutex`] guards the whole map, so the seal-check-and-mark in
/// [`SealedGrantStore::claim`] is one critical section — concurrent claims
/// serialize and exactly one wins.
#[derive(Debug, Default)]
pub struct InMemorySealedGrantStore {
    grants: Mutex<HashMap<GrantKey, AttestedSigningGrant>>,
}

impl InMemorySealedGrantStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SealedGrantStore for InMemorySealedGrantStore {
    async fn seal(&self, grant: AttestedSigningGrant) -> Result<(), GrantError> {
        let mut grants = self.grants.lock().map_err(|e| GrantError::Backend {
            reason: e.to_string(),
        })?;
        if grants.contains_key(&grant.key) {
            return Err(GrantError::AlreadySealed);
        }
        grants.insert(grant.key.clone(), grant);
        Ok(())
    }

    async fn claim(&self, key: &GrantKey) -> Result<ClaimedGrant, GrantError> {
        let mut grants = self.grants.lock().map_err(|e| GrantError::Backend {
            reason: e.to_string(),
        })?;
        // Seal-check-and-mark in one critical section: the lock is held across
        // the read of `status` and the write back to `Claimed`, so concurrent
        // claims cannot both observe `Sealed`.
        let grant = grants.get_mut(key).ok_or(GrantError::NotFound)?;
        match grant.status {
            GrantStatus::Claimed => Err(GrantError::AlreadyClaimed),
            GrantStatus::Sealed => {
                grant.status = GrantStatus::Claimed;
                Ok(ClaimedGrant {
                    key: grant.key.clone(),
                    created_at_ms: grant.created_at_ms,
                    expiry_ms: grant.expiry_ms,
                })
            }
        }
    }
}

/// Canonical contract suite for [`SealedGrantStore`] implementations.
///
/// Mirrors `predicate_backend_contract_test!` in `ironclaw_hooks`: the
/// behavioural contract lives once and every backend (in-memory here, durable
/// PG / libSQL in stacked follow-ups) is driven through it. Invoke with a label
/// and a zero-arg factory closure returning a fresh store.
#[cfg(any(test, feature = "contract-tests"))]
pub mod contract {
    // The `pub` items below are reachable as `ironclaw_attestation::grant::
    // contract::*` only when the `contract-tests` feature makes the `grant`
    // module public (see lib.rs). Under the crate's own `cargo test` (no
    // feature) the parent module is private, so the crate-level
    // `unreachable_pub` warning would fire on every helper — suppress it for
    // that build; the `pub` is intentional for out-of-crate consumers.
    #![cfg_attr(not(feature = "contract-tests"), allow(unreachable_pub))]
    use super::*;
    use std::sync::Arc;

    pub fn key(seed: u8) -> GrantKey {
        GrantKey {
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("user-1"),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new("gate:abc"),
            approved_tx_hash: ApprovedTxHash::from_bytes([seed; 32]),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
            chain_id: ChainId::new("eip155:1"),
        }
    }

    /// Construct a valid sealed grant for the contract cases, panicking if the
    /// timestamps are rejected (they never are for these fixed inputs).
    pub fn grant_for(
        key: GrantKey,
        created_at_ms: i64,
        expiry_ms: Option<i64>,
    ) -> AttestedSigningGrant {
        AttestedSigningGrant::new(key, created_at_ms, expiry_ms).expect("valid grant")
    }

    pub async fn seal_then_claim_succeeds<S: SealedGrantStore>(store: S) {
        let k = key(1);
        store
            .seal(grant_for(k.clone(), 1_000, None))
            .await
            .expect("seal must succeed");
        let claimed = store.claim(&k).await.expect("first claim must succeed");
        assert_eq!(claimed.key, k);
        assert_eq!(claimed.created_at_ms, 1_000);
    }

    pub async fn second_claim_is_already_claimed<S: SealedGrantStore>(store: S) {
        let k = key(2);
        store
            .seal(grant_for(k.clone(), 0, None))
            .await
            .expect("seal");
        store.claim(&k).await.expect("first claim");
        assert_eq!(store.claim(&k).await, Err(GrantError::AlreadyClaimed));
    }

    pub async fn claim_unsealed_is_not_found<S: SealedGrantStore>(store: S) {
        assert_eq!(store.claim(&key(3)).await, Err(GrantError::NotFound));
    }

    pub async fn claim_mismatched_component_is_not_found<S: SealedGrantStore>(store: S) {
        let sealed = key(4);
        store
            .seal(grant_for(sealed.clone(), 0, None))
            .await
            .expect("seal");
        let mut mismatched = sealed.clone();
        mismatched.user = UserId::new("someone-else");
        assert_eq!(store.claim(&mismatched).await, Err(GrantError::NotFound));
        // Differing only by the approved hash is also a different grant.
        let mut hash_mismatch = sealed;
        hash_mismatch.approved_tx_hash = ApprovedTxHash::from_bytes([99; 32]);
        assert_eq!(store.claim(&hash_mismatch).await, Err(GrantError::NotFound));
    }

    /// Seal one grant, then attempt a claim that differs by EACH individual
    /// [`GrantKey`] component in turn. Every single-component mismatch MUST be
    /// treated as a distinct, never-sealed grant (`NotFound`).
    ///
    /// This pins a backend's unique key to ALL seven components. A durable
    /// backend that omits, say, `chain_id` or `key_or_account_id` from its
    /// primary/unique key would collapse two distinct grants into one and let a
    /// claim for `chain B` consume the grant sealed for `chain A` — a real
    /// cross-chain / cross-key authorization collision. Catching it here means
    /// the durable backends cannot pass with an under-specified key.
    pub async fn each_grant_key_component_is_part_of_identity<S: SealedGrantStore>(store: S) {
        // Distinct, non-overlapping values for every mutator so no two mutated
        // keys accidentally collide with each other or the sealed key.
        let sealed = key(40);
        store
            .seal(grant_for(sealed.clone(), 0, None))
            .await
            .expect("seal");

        // One probe per component: each clones the sealed key and changes
        // exactly that one field to a distinct value.
        let probes = [
            ("tenant", {
                let mut k = sealed.clone();
                k.tenant = TenantId::new("tenant-OTHER");
                k
            }),
            ("user", {
                let mut k = sealed.clone();
                k.user = UserId::new("user-OTHER");
                k
            }),
            ("run_id", {
                let mut k = sealed.clone();
                k.run_id = RunId::new("run-OTHER");
                k
            }),
            ("gate_ref", {
                let mut k = sealed.clone();
                k.gate_ref = GateRef::new("gate:OTHER");
                k
            }),
            ("approved_tx_hash", {
                let mut k = sealed.clone();
                k.approved_tx_hash = ApprovedTxHash::from_bytes([0xAB; 32]);
                k
            }),
            ("key_or_account_id", {
                let mut k = sealed.clone();
                k.key_or_account_id = KeyOrAccountId::new("0xOTHER");
                k
            }),
            ("chain_id", {
                let mut k = sealed.clone();
                k.chain_id = ChainId::new("eip155:999");
                k
            }),
        ];

        for (component, probe) in probes {
            assert_ne!(
                probe, sealed,
                "probe for `{component}` must actually change the key"
            );
            assert_eq!(
                store.claim(&probe).await,
                Err(GrantError::NotFound),
                "claim differing only by `{component}` must be a distinct (unsealed) grant"
            );
        }

        // The original grant is still untouched and claimable exactly once.
        store
            .claim(&sealed)
            .await
            .expect("the sealed grant itself must still be claimable");
    }

    pub async fn double_seal_is_already_sealed<S: SealedGrantStore>(store: S) {
        let k = key(5);
        store
            .seal(grant_for(k.clone(), 0, None))
            .await
            .expect("seal");
        assert_eq!(
            store.seal(grant_for(k, 0, None)).await,
            Err(GrantError::AlreadySealed)
        );
    }

    /// Re-sealing a key that has already been *claimed* returns
    /// [`GrantError::AlreadySealed`] — NOT [`GrantError::NotFound`]. The key is
    /// permanently spent; "already sealed" is the canonical "this key is not
    /// available for sealing" signal regardless of whether the live row is still
    /// `Sealed` or has moved to `Claimed`. Pins the trait-doc behaviour so a
    /// durable backend that archives claimed rows separately cannot diverge by
    /// returning `NotFound` for a re-seal of a claimed key.
    pub async fn seal_after_claimed_is_already_sealed<S: SealedGrantStore>(store: S) {
        let k = key(9);
        store
            .seal(grant_for(k.clone(), 0, None))
            .await
            .expect("seal");
        store.claim(&k).await.expect("claim");
        assert_eq!(
            store.seal(grant_for(k, 0, None)).await,
            Err(GrantError::AlreadySealed),
            "re-sealing a claimed key must be AlreadySealed, not NotFound"
        );
    }

    /// A claimed grant carries `created_at_ms` AND `expiry_ms` through from the
    /// sealed grant, so the downstream signing layer can re-check the time
    /// window as defence-in-depth. A backend that drops `expiry_ms` on claim
    /// (the previous shape) cannot pass.
    pub async fn claim_carries_created_and_expiry_through<S: SealedGrantStore>(store: S) {
        let k = key(10);
        store
            .seal(grant_for(k.clone(), 1_000, Some(9_999)))
            .await
            .expect("seal");
        let claimed = store.claim(&k).await.expect("claim");
        assert_eq!(claimed.created_at_ms, 1_000);
        assert_eq!(
            claimed.expiry_ms,
            Some(9_999),
            "claim must carry expiry_ms through for downstream time-window checks"
        );
    }

    pub async fn concurrent_claims_yield_exactly_one_winner<S>(store: S)
    where
        S: SealedGrantStore + 'static,
    {
        let store = Arc::new(store);
        let k = key(6);
        store
            .seal(grant_for(k.clone(), 0, None))
            .await
            .expect("seal");

        let mut handles = Vec::new();
        for _ in 0..32 {
            let store = Arc::clone(&store);
            let k = k.clone();
            handles.push(tokio::spawn(async move { store.claim(&k).await }));
        }

        let mut ok = 0usize;
        let mut already = 0usize;
        for h in handles {
            match h.await.expect("task join") {
                Ok(_) => ok += 1,
                Err(GrantError::AlreadyClaimed) => already += 1,
                Err(other) => panic!("unexpected error under contention: {other:?}"),
            }
        }
        assert_eq!(ok, 1, "exactly one claim must win the one-shot CAS");
        assert_eq!(already, 31, "all other claims must be AlreadyClaimed");
    }

    /// Drive every contract case against a fresh store from `$factory`.
    #[macro_export]
    macro_rules! sealed_grant_store_contract_cases {
        ($label:ident, $factory:expr) => {
            mod $label {
                #[tokio::test]
                async fn seal_then_claim_succeeds() {
                    $crate::grant::contract::seal_then_claim_succeeds($factory()).await;
                }
                #[tokio::test]
                async fn second_claim_is_already_claimed() {
                    $crate::grant::contract::second_claim_is_already_claimed($factory()).await;
                }
                #[tokio::test]
                async fn claim_unsealed_is_not_found() {
                    $crate::grant::contract::claim_unsealed_is_not_found($factory()).await;
                }
                #[tokio::test]
                async fn claim_mismatched_component_is_not_found() {
                    $crate::grant::contract::claim_mismatched_component_is_not_found($factory())
                        .await;
                }
                #[tokio::test]
                async fn each_grant_key_component_is_part_of_identity() {
                    $crate::grant::contract::each_grant_key_component_is_part_of_identity(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn double_seal_is_already_sealed() {
                    $crate::grant::contract::double_seal_is_already_sealed($factory()).await;
                }
                #[tokio::test]
                async fn seal_after_claimed_is_already_sealed() {
                    $crate::grant::contract::seal_after_claimed_is_already_sealed($factory()).await;
                }
                #[tokio::test]
                async fn claim_carries_created_and_expiry_through() {
                    $crate::grant::contract::claim_carries_created_and_expiry_through($factory())
                        .await;
                }
                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                async fn concurrent_claims_yield_exactly_one_winner() {
                    $crate::grant::contract::concurrent_claims_yield_exactly_one_winner($factory())
                        .await;
                }
            }
        };
    }
}

#[cfg(test)]
crate::sealed_grant_store_contract_cases!(in_memory, crate::grant::InMemorySealedGrantStore::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_key_from_context_omits_scope_and_actor() {
        use ironclaw_signing_provider::{ActorId, ScopeId};
        let ctx = SigningContext {
            tenant: TenantId::new("t"),
            user: UserId::new("u"),
            scope: ScopeId::new("s"),
            actor: ActorId::new("a"),
            run_id: RunId::new("r"),
            gate_ref: GateRef::new("g"),
            chain_id: ChainId::new("c"),
            key_or_account_id: KeyOrAccountId::new("k"),
        };
        let hash = ApprovedTxHash::from_bytes([3; 32]);
        let key = GrantKey::from_context(&ctx, hash);
        assert_eq!(key.tenant, ctx.tenant);
        assert_eq!(key.approved_tx_hash, hash);
        assert_eq!(key.chain_id, ctx.chain_id);
    }

    #[test]
    fn grant_and_status_round_trip_serde() {
        let grant = AttestedSigningGrant::new(super::contract::key(7), 1234, Some(5678))
            .expect("valid grant");
        let json = serde_json::to_string(&grant).expect("ser");
        let back: AttestedSigningGrant = serde_json::from_str(&json).expect("de");
        assert_eq!(back, grant);
        assert_eq!(back.status, GrantStatus::Sealed);
    }

    #[test]
    fn new_rejects_negative_created_at_ms() {
        assert_eq!(
            AttestedSigningGrant::new(super::contract::key(8), -1, None),
            Err(GrantError::InvalidTimestamp { created_at_ms: -1 })
        );
    }

    #[test]
    fn new_rejects_expiry_at_or_before_created_at() {
        assert_eq!(
            AttestedSigningGrant::new(super::contract::key(8), 1_000, Some(1_000)),
            Err(GrantError::InvalidExpiry {
                created_at_ms: 1_000,
                expiry_ms: 1_000
            })
        );
        assert_eq!(
            AttestedSigningGrant::new(super::contract::key(8), 1_000, Some(999)),
            Err(GrantError::InvalidExpiry {
                created_at_ms: 1_000,
                expiry_ms: 999
            })
        );
    }

    // A panic while the `grants` mutex is held poisons it. The next `lock()`
    // must surface as `GrantError::Backend` rather than panicking, so the
    // store degrades to a clean error instead of poisoning the whole process.
    #[tokio::test]
    async fn in_memory_store_returns_backend_error_on_poisoned_lock() {
        let store = std::sync::Arc::new(InMemorySealedGrantStore::new());
        let store_clone = store.clone();
        let _ = std::thread::spawn(move || {
            let _lock = store_clone.grants.lock().expect("lock");
            panic!("poisoning lock");
        })
        .join();

        let key = super::contract::key(1);
        assert!(matches!(
            store.claim(&key).await,
            Err(GrantError::Backend { .. })
        ));
        let grant = AttestedSigningGrant::new(key, 1, None).expect("valid grant");
        assert!(matches!(
            store.seal(grant).await,
            Err(GrantError::Backend { .. })
        ));
    }
}
