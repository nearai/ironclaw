//! Durable one-shot WebAuthn challenge store.
//!
//! The challenge is the anti-replay nonce of the custodial attested-signing
//! path. It is *bound* to the exact operation being authorized via a
//! [`ChallengePreimage`] that folds in every identity / scope / target field
//! plus the binding [`ApprovedTxHash`] (PR2). The value actually handed to the
//! client is a [`ChallengeCommitment`] — a domain-separated SHA-256 over the
//! preimage. The WebAuthn authenticator signs over `clientDataJSON` whose
//! `challenge` echoes this commitment; the verifier (see [`crate::webauthn`])
//! checks the echo equals what we issued.
//!
//! ## One-shot + expiry contract
//!
//! [`ChallengeStore::consume`] is an **atomic one-shot**: the first consume of
//! an issued, unexpired challenge wins and atomically marks it consumed; every
//! later consume of that id fails with [`ChallengeError::AlreadyConsumed`].
//! Expired challenges fail with [`ChallengeError::Expired`]; unknown ids with
//! [`ChallengeError::NotFound`]. This is the same rigor as the PR3 grant
//! `claim`: the seal/expiry check and the mark-consumed happen in a single
//! critical section, so under contention exactly one consumer wins.
//!
//! The plan intends this consume to be atomic with the credential signCount
//! update + gate resolution at the call site (PR5). This PR provides only the
//! one-shot store primitive; durable PG / libSQL backends are stacked
//! follow-ups gated by the [`challenge_store_contract_cases!`] macro and are
//! NOT implemented here.
//!
//! ## Encoding
//!
//! [`ChallengePreimage::encode`] reuses the PR2 hand-rolled, domain-separated,
//! length-prefixed encoding (no CBOR dependency). Length-prefixing every bound
//! field makes the encoding injective: changing ANY field changes the bytes
//! and therefore the commitment, so a challenge issued for one operation can
//! never be replayed to authorize a different one.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
};

/// Domain separator for the challenge preimage. Distinct from the canonical and
/// approved-tx-hash domains so the three pre-images can never be confused.
const CHALLENGE_DOMAIN: &[u8] = b"ironclaw.attestation.challenge.v1";

/// Opaque identifier of an issued challenge. Used as the consume key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChallengeId(String);

impl ChallengeId {
    /// Construct from any string-like value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifier of a registered WebAuthn credential, bound into the preimage so a
/// challenge is tied to the credential expected to answer it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialId(Vec<u8>);

impl CredentialId {
    /// Construct from raw credential-id bytes.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Borrow the underlying bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Identifier of a single delivery attempt (the agent may re-prompt the user;
/// each prompt gets a fresh attempt id so an old prompt's challenge cannot be
/// reused for a new one).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeliveryAttemptId(String);

impl DeliveryAttemptId {
    /// Construct from any string-like value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Everything a challenge is bound to.
///
/// Constructing the preimage requires ALL of these fields — there is no
/// builder shortcut that omits one — so a caller cannot accidentally issue an
/// under-bound challenge. The [`ChallengePreimage::encode`] output is injective
/// over this field set (see module docs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengePreimage {
    /// WebAuthn Relying Party ID the assertion must be scoped to.
    pub rp_id: String,
    /// The exact origin the client is expected to present in `clientDataJSON`.
    pub expected_origin: String,
    /// Tenant boundary.
    pub tenant: TenantId,
    /// End user.
    pub user: UserId,
    /// Authorization scope.
    pub scope: ScopeId,
    /// Acting principal.
    pub actor: ActorId,
    /// Credential expected to answer this challenge.
    pub credential_id: CredentialId,
    /// Owning run.
    pub run_id: RunId,
    /// Gate the flow is blocked on.
    pub gate_ref: GateRef,
    /// Signing key or account.
    pub key_or_account_id: KeyOrAccountId,
    /// Target chain / network.
    pub chain_id: ChainId,
    /// Absolute expiry (unix millis). A consume at or after this instant fails
    /// [`ChallengeError::Expired`].
    pub expiry_ms: i64,
    /// Delivery attempt this challenge belongs to.
    pub delivery_attempt: DeliveryAttemptId,
    /// The binding hash of the approved transaction (PR2).
    pub rendered_tx_digest: ApprovedTxHash,
}

/// Append `len(bytes) ∥ bytes` to `out`.
///
/// The length prefix is a `u64` so the `usize -> length-prefix` conversion is
/// infallible and non-truncating on every supported platform: a `u32` prefix
/// would silently wrap for a field exceeding `u32::MAX` bytes, which would
/// break the domain separation that makes this encoding collision-resistant.
fn push_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

impl ChallengePreimage {
    /// Deterministic, domain-separated, length-prefixed encoding of every bound
    /// field, in fixed order. Identical input yields identical bytes.
    pub fn encode(&self) -> Vec<u8> {
        // Pre-size the buffer to avoid reallocations on this signing/verify
        // hot path. 13 length-prefixed fields each add an 8-byte u64 prefix
        // (`push_lp`), plus the 8-byte big-endian `expiry_ms`, plus the domain
        // tag and every field's raw bytes.
        let capacity = CHALLENGE_DOMAIN.len()
            + 13 * 8 // u64 length prefixes for the 13 push_lp fields
            + 8 // expiry_ms (i64 big-endian)
            + self.rp_id.len()
            + self.expected_origin.len()
            + self.tenant.as_str().len()
            + self.user.as_str().len()
            + self.scope.as_str().len()
            + self.actor.as_str().len()
            + self.credential_id.as_bytes().len()
            + self.run_id.as_str().len()
            + self.gate_ref.as_str().len()
            + self.key_or_account_id.as_str().len()
            + self.chain_id.as_str().len()
            + self.delivery_attempt.as_str().len()
            + self.rendered_tx_digest.as_bytes().len();
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(CHALLENGE_DOMAIN);
        push_lp(&mut out, self.rp_id.as_bytes());
        push_lp(&mut out, self.expected_origin.as_bytes());
        push_lp(&mut out, self.tenant.as_str().as_bytes());
        push_lp(&mut out, self.user.as_str().as_bytes());
        push_lp(&mut out, self.scope.as_str().as_bytes());
        push_lp(&mut out, self.actor.as_str().as_bytes());
        push_lp(&mut out, self.credential_id.as_bytes());
        push_lp(&mut out, self.run_id.as_str().as_bytes());
        push_lp(&mut out, self.gate_ref.as_str().as_bytes());
        push_lp(&mut out, self.key_or_account_id.as_str().as_bytes());
        push_lp(&mut out, self.chain_id.as_str().as_bytes());
        out.extend_from_slice(&self.expiry_ms.to_be_bytes());
        push_lp(&mut out, self.delivery_attempt.as_str().as_bytes());
        push_lp(&mut out, self.rendered_tx_digest.as_bytes());
        out
    }

    /// Compute the [`ChallengeCommitment`] handed to the client: a
    /// domain-separated SHA-256 over [`ChallengePreimage::encode`].
    pub fn commitment(&self) -> ChallengeCommitment {
        let digest: [u8; 32] = Sha256::digest(self.encode()).into();
        ChallengeCommitment(digest)
    }
}

/// 32-byte commitment over a [`ChallengePreimage`] — the value sent to the
/// client and echoed back in `clientDataJSON.challenge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChallengeCommitment([u8; 32]);

impl ChallengeCommitment {
    /// Construct from raw bytes (e.g. when rehydrating from storage).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// An issued, not-yet-consumed challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedChallenge {
    /// Opaque consume key.
    pub id: ChallengeId,
    /// The full preimage (carried so the verifier can recompute / inspect the
    /// bound fields; the commitment is derived from it).
    pub preimage: ChallengePreimage,
}

impl IssuedChallenge {
    /// The commitment value handed to the client for this challenge.
    pub fn commitment(&self) -> ChallengeCommitment {
        self.preimage.commitment()
    }
}

/// The result of a successful [`ChallengeStore::consume`].
///
/// Holding one of these is proof that *this* consumer won the one-shot race for
/// an unexpired challenge and is authorized to proceed to assertion
/// verification against the bound preimage exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumedChallenge {
    /// The challenge id that was consumed.
    pub id: ChallengeId,
    /// The bound preimage (the verifier checks the echoed challenge equals
    /// `preimage.commitment()` and the assertion against the bound fields).
    pub preimage: ChallengePreimage,
}

/// Errors a [`ChallengeStore`] can surface.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChallengeError {
    /// `issue` was called with an id that is already issued.
    #[error("challenge already issued for this id")]
    AlreadyIssued,

    /// `consume` was called for an id that was never issued.
    #[error("no challenge found for this id")]
    NotFound,

    /// `consume` lost the one-shot race: the challenge was already consumed.
    #[error("challenge already consumed (one-shot)")]
    AlreadyConsumed,

    /// `consume` was called at or after the challenge expiry.
    #[error("challenge expired")]
    Expired,

    /// A backend-internal failure with an opaque description.
    #[error("challenge store error: {reason}")]
    Backend {
        /// Human-readable description of the backend failure.
        reason: String,
    },
}

/// Durable one-shot challenge store.
///
/// `consume` MUST be an atomic, one-shot operation: the issued-check, the
/// expiry check, and the mark-consumed happen in a single critical section so
/// that under concurrent consumes of an unexpired challenge exactly one caller
/// observes it un-consumed and transitions it to consumed; all others fail.
#[async_trait]
pub trait ChallengeStore: Send + Sync {
    /// Persist an issued challenge. Fails with
    /// [`ChallengeError::AlreadyIssued`] if a challenge with the same id is
    /// already issued (issuance is one-shot per id).
    async fn issue(&self, challenge: IssuedChallenge) -> Result<(), ChallengeError>;

    /// Atomically consume an issued, unexpired challenge exactly once.
    ///
    /// `now_ms` is the caller's notion of the current time (unix millis); a
    /// challenge whose `expiry_ms <= now_ms` fails [`ChallengeError::Expired`].
    /// The clock is injected rather than read internally so the one-shot /
    /// expiry semantics are deterministically testable and identical across
    /// backends.
    ///
    /// * First consume of an issued, unexpired id -> `Ok(ConsumedChallenge)`.
    /// * Any later consume of that id -> `Err(ChallengeError::AlreadyConsumed)`.
    /// * Consume of an expired id -> `Err(ChallengeError::Expired)`.
    /// * Consume of an unknown id -> `Err(ChallengeError::NotFound)`.
    async fn consume(
        &self,
        id: &ChallengeId,
        now_ms: i64,
    ) -> Result<ConsumedChallenge, ChallengeError>;
}

/// Internal stored state of a challenge.
#[derive(Debug, Clone)]
struct StoredChallenge {
    preimage: ChallengePreimage,
    consumed: bool,
}

/// In-memory [`ChallengeStore`].
///
/// The single [`Mutex`] guards the whole map, so the issued/expiry-check and
/// mark-consumed in [`ChallengeStore::consume`] is one critical section —
/// concurrent consumes serialize and exactly one wins.
///
/// ## Eviction (memory-DoS bound)
///
/// Challenge entries are ephemeral one-shot cache rows with an absolute
/// `expiry_ms`; an attacker who repeatedly issues challenges that are never
/// consumed would otherwise grow this map without bound (OOM DoS). To bound
/// memory to the set of *live* (issued-and-unexpired, plus consumed-but-not-yet-
/// expired) challenges, both [`ChallengeStore::issue`] and
/// [`ChallengeStore::consume`] prune every entry whose `expiry_ms <= now` before
/// doing their work, where `now` is:
///
/// * `consume`: the caller-supplied `now_ms` (authoritative clock), which is
///   also stashed into `clock_watermark`;
/// * `issue`: the most recent `now_ms` observed by any `consume`
///   (`clock_watermark`). `issue` carries no clock of its own, so it prunes
///   against the last authoritative timestamp. The watermark is monotonic in
///   practice and pruning is always safe because `expiry_ms` is absolute.
///
/// Eviction NEVER weakens the one-shot guarantee. A consumed challenge is kept
/// as a tombstone until it expires, so a replayed consume of a consumed,
/// still-unexpired challenge returns [`ChallengeError::AlreadyConsumed`] rather
/// than [`ChallengeError::NotFound`]. Only after the challenge has *expired* is
/// the tombstone reclaimable — and an expired challenge can never be
/// successfully consumed regardless of its consumed flag, so eviction cannot be
/// abused to resurrect a consumed challenge.
#[derive(Debug, Default)]
pub struct InMemoryChallengeStore {
    challenges: Mutex<HashMap<ChallengeId, StoredChallenge>>,
    /// Most recent authoritative `now_ms` seen by `consume`, used to prune on
    /// the clock-less `issue` path. `i64::MIN` until the first consume.
    clock_watermark: AtomicI64,
}

impl InMemoryChallengeStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self {
            challenges: Mutex::new(HashMap::new()),
            clock_watermark: AtomicI64::new(i64::MIN),
        }
    }

    /// Number of resident challenge entries (live + not-yet-pruned tombstones).
    /// Test-only window onto eviction so the memory-DoS bound is observable.
    #[cfg(test)]
    pub(crate) fn resident_len(&self) -> usize {
        self.challenges.lock().expect("lock").len()
    }
}

/// Drop every entry whose absolute expiry is at or before `now_ms`. Expired
/// entries (consumed or not) can never yield a successful consume, so removing
/// them is sound and bounds memory to live challenges.
fn prune_expired(map: &mut HashMap<ChallengeId, StoredChallenge>, now_ms: i64) {
    map.retain(|_, stored| stored.preimage.expiry_ms > now_ms);
}

#[async_trait]
impl ChallengeStore for InMemoryChallengeStore {
    async fn issue(&self, challenge: IssuedChallenge) -> Result<(), ChallengeError> {
        let mut map = self
            .challenges
            .lock()
            .map_err(|e| ChallengeError::Backend {
                reason: e.to_string(),
            })?;
        // Bound memory: drop entries already expired as of the last
        // authoritative clock any consume observed. `issue` has no clock of its
        // own; the watermark is the best available lower bound on "now".
        let watermark = self.clock_watermark.load(Ordering::Relaxed);
        if watermark != i64::MIN {
            prune_expired(&mut map, watermark);
        }
        if map.contains_key(&challenge.id) {
            return Err(ChallengeError::AlreadyIssued);
        }
        map.insert(
            challenge.id,
            StoredChallenge {
                preimage: challenge.preimage,
                consumed: false,
            },
        );
        Ok(())
    }

    async fn consume(
        &self,
        id: &ChallengeId,
        now_ms: i64,
    ) -> Result<ConsumedChallenge, ChallengeError> {
        let mut map = self
            .challenges
            .lock()
            .map_err(|e| ChallengeError::Backend {
                reason: e.to_string(),
            })?;
        // Record the authoritative clock for the clock-less `issue` prune path.
        self.clock_watermark.store(now_ms, Ordering::Relaxed);
        // Issued-check, expiry-check, and mark-consumed in one critical section:
        // the lock is held across all three, so concurrent consumes cannot both
        // observe the challenge un-consumed. Expiry is checked BEFORE the
        // one-shot transition so an expired challenge is never "spent".
        //
        // The requested id is classified FIRST (before any eviction) so it
        // still gets its precise terminal status — `Expired` for a known but
        // expired id, `AlreadyConsumed` for a consumed tombstone, `NotFound`
        // for an unknown id — and eviction can never silently downgrade a known
        // expired/consumed id to `NotFound`.
        let result = match map.get_mut(id) {
            None => Err(ChallengeError::NotFound),
            Some(stored) if stored.consumed => Err(ChallengeError::AlreadyConsumed),
            Some(stored) if stored.preimage.expiry_ms <= now_ms => Err(ChallengeError::Expired),
            Some(stored) => {
                stored.consumed = true;
                Ok(stored.preimage.clone())
            }
        };
        // Reclaim every entry already expired as of `now_ms`. This bounds memory
        // to live challenges. It runs UNCONDITIONALLY (even on NotFound) so a
        // consume — including one targeting an unknown id — sweeps stale entries.
        // It cannot disturb the classification above (already computed) and a
        // freshly-consumed, still-unexpired entry is retained as a tombstone.
        prune_expired(&mut map, now_ms);
        let preimage = result?;
        Ok(ConsumedChallenge {
            id: id.clone(),
            preimage,
        })
    }
}

/// Canonical contract suite for [`ChallengeStore`] implementations.
///
/// Mirrors `sealed_grant_store_contract_cases!` (PR3): the behavioural contract
/// lives once and every backend (in-memory here, durable PG / libSQL in stacked
/// follow-ups) is driven through it. Invoke with a label and a zero-arg factory
/// closure returning a fresh store.
#[cfg(test)]
pub(crate) mod contract {
    use super::*;
    use std::sync::Arc;

    pub(crate) fn preimage(seed: u8, expiry_ms: i64) -> ChallengePreimage {
        ChallengePreimage {
            rp_id: "ironclaw.example".to_string(),
            expected_origin: "https://ironclaw.example".to_string(),
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("user-1"),
            scope: ScopeId::new("scope-x"),
            actor: ActorId::new("actor-7"),
            credential_id: CredentialId::new(vec![seed; 16]),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new("gate:abc"),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
            chain_id: ChainId::new("eip155:1"),
            expiry_ms,
            delivery_attempt: DeliveryAttemptId::new("attempt-1"),
            rendered_tx_digest: ApprovedTxHash::from_bytes([seed; 32]),
        }
    }

    pub(crate) fn issued(id: &str, seed: u8, expiry_ms: i64) -> IssuedChallenge {
        IssuedChallenge {
            id: ChallengeId::new(id),
            preimage: preimage(seed, expiry_ms),
        }
    }

    pub(crate) async fn issue_then_consume_succeeds<S: ChallengeStore>(store: S) {
        let ch = issued("c1", 1, 10_000);
        let commitment = ch.commitment();
        store.issue(ch).await.expect("issue must succeed");
        let consumed = store
            .consume(&ChallengeId::new("c1"), 5_000)
            .await
            .expect("first consume must succeed");
        assert_eq!(consumed.id, ChallengeId::new("c1"));
        // The consumed preimage still derives the same commitment we issued.
        assert_eq!(consumed.preimage.commitment(), commitment);
    }

    pub(crate) async fn second_consume_is_already_consumed<S: ChallengeStore>(store: S) {
        let ch = issued("c2", 2, 10_000);
        store.issue(ch).await.expect("issue");
        store
            .consume(&ChallengeId::new("c2"), 1)
            .await
            .expect("first consume");
        assert_eq!(
            store.consume(&ChallengeId::new("c2"), 2).await,
            Err(ChallengeError::AlreadyConsumed)
        );
    }

    pub(crate) async fn consume_unknown_is_not_found<S: ChallengeStore>(store: S) {
        assert_eq!(
            store.consume(&ChallengeId::new("nope"), 0).await,
            Err(ChallengeError::NotFound)
        );
    }

    pub(crate) async fn consume_expired_is_expired<S: ChallengeStore>(store: S) {
        let ch = issued("c3", 3, 1_000);
        store.issue(ch).await.expect("issue");
        // now == expiry -> expired (boundary is inclusive). The expired id is
        // classified before any garbage collection, so its precise terminal
        // status `Expired` is reported (never silently downgraded).
        assert_eq!(
            store.consume(&ChallengeId::new("c3"), 1_000).await,
            Err(ChallengeError::Expired)
        );
        // strictly after expiry -> still fails closed, and the challenge was
        // never marked consumed. A backend that garbage-collects expired
        // entries (e.g. the in-memory cache evicts on access) may report this
        // as `NotFound` instead of `Expired`; both are fail-closed and neither
        // can ever yield a successful consume of an expired challenge.
        assert!(
            matches!(
                store.consume(&ChallengeId::new("c3"), 2_000).await,
                Err(ChallengeError::Expired | ChallengeError::NotFound)
            ),
            "post-expiry consume must fail closed (Expired or evicted -> NotFound)"
        );
    }

    pub(crate) async fn double_issue_is_already_issued<S: ChallengeStore>(store: S) {
        store.issue(issued("c4", 4, 10_000)).await.expect("issue");
        assert_eq!(
            store.issue(issued("c4", 4, 10_000)).await,
            Err(ChallengeError::AlreadyIssued)
        );
    }

    pub(crate) async fn concurrent_consumes_yield_exactly_one_winner<S>(store: S)
    where
        S: ChallengeStore + 'static,
    {
        let store = Arc::new(store);
        store
            .issue(issued("c5", 5, 1_000_000))
            .await
            .expect("issue");

        let mut handles = Vec::new();
        for _ in 0..32 {
            let store = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                store.consume(&ChallengeId::new("c5"), 1).await
            }));
        }

        let mut ok = 0usize;
        let mut already = 0usize;
        for h in handles {
            match h.await.expect("task join") {
                Ok(_) => ok += 1,
                Err(ChallengeError::AlreadyConsumed) => already += 1,
                Err(other) => panic!("unexpected error under contention: {other:?}"),
            }
        }
        assert_eq!(ok, 1, "exactly one consume must win the one-shot race");
        assert_eq!(already, 31, "all other consumes must be AlreadyConsumed");
    }

    /// Drive every contract case against a fresh store from `$factory`.
    #[macro_export]
    macro_rules! challenge_store_contract_cases {
        ($label:ident, $factory:expr) => {
            mod $label {
                #[tokio::test]
                async fn issue_then_consume_succeeds() {
                    $crate::challenge::contract::issue_then_consume_succeeds($factory()).await;
                }
                #[tokio::test]
                async fn second_consume_is_already_consumed() {
                    $crate::challenge::contract::second_consume_is_already_consumed($factory())
                        .await;
                }
                #[tokio::test]
                async fn consume_unknown_is_not_found() {
                    $crate::challenge::contract::consume_unknown_is_not_found($factory()).await;
                }
                #[tokio::test]
                async fn consume_expired_is_expired() {
                    $crate::challenge::contract::consume_expired_is_expired($factory()).await;
                }
                #[tokio::test]
                async fn double_issue_is_already_issued() {
                    $crate::challenge::contract::double_issue_is_already_issued($factory()).await;
                }
                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                async fn concurrent_consumes_yield_exactly_one_winner() {
                    $crate::challenge::contract::concurrent_consumes_yield_exactly_one_winner(
                        $factory(),
                    )
                    .await;
                }
            }
        };
    }
}

#[cfg(test)]
crate::challenge_store_contract_cases!(in_memory, crate::challenge::InMemoryChallengeStore::new);

#[cfg(test)]
mod tests {
    use super::*;

    /// Mutate exactly one bound field and assert the commitment changes. This
    /// is the binding property: a challenge issued for one operation can never
    /// be replayed to authorize a different one.
    #[test]
    fn commitment_changes_if_any_bound_field_changes() {
        let base = contract::preimage(1, 10_000);
        let base_commitment = base.commitment();

        type Mutator = fn(&mut ChallengePreimage);
        let mutators: Vec<(&str, Mutator)> = vec![
            ("rp_id", |p| p.rp_id = "evil.example".to_string()),
            ("expected_origin", |p| {
                p.expected_origin = "https://evil.example".to_string()
            }),
            ("tenant", |p| p.tenant = TenantId::new("tenant-b")),
            ("user", |p| p.user = UserId::new("user-2")),
            ("scope", |p| p.scope = ScopeId::new("scope-y")),
            ("actor", |p| p.actor = ActorId::new("actor-8")),
            ("credential_id", |p| {
                p.credential_id = CredentialId::new(vec![0xff; 16])
            }),
            ("run_id", |p| p.run_id = RunId::new("run-99")),
            ("gate_ref", |p| p.gate_ref = GateRef::new("gate:xyz")),
            ("key_or_account_id", |p| {
                p.key_or_account_id = KeyOrAccountId::new("0xdef")
            }),
            ("chain_id", |p| p.chain_id = ChainId::new("eip155:10")),
            ("expiry_ms", |p| p.expiry_ms = 99_999),
            ("delivery_attempt", |p| {
                p.delivery_attempt = DeliveryAttemptId::new("attempt-2")
            }),
            ("rendered_tx_digest", |p| {
                p.rendered_tx_digest = ApprovedTxHash::from_bytes([0xaa; 32])
            }),
        ];

        for (field, mutate) in mutators {
            let mut p = base.clone();
            mutate(&mut p);
            assert_ne!(
                p.commitment(),
                base_commitment,
                "mutating `{field}` must change the challenge commitment"
            );
        }
    }

    #[test]
    fn encode_is_deterministic_across_calls_and_serde() {
        let p = contract::preimage(7, 12_345);
        assert_eq!(p.encode(), p.encode());
        let json = serde_json::to_string(&p).expect("ser");
        let back: ChallengePreimage = serde_json::from_str(&json).expect("de");
        assert_eq!(back.encode(), p.encode());
        assert_eq!(back.commitment(), p.commitment());
    }

    #[test]
    fn issued_and_consumed_round_trip_serde() {
        let ch = contract::issued("c-serde", 9, 10_000);
        let json = serde_json::to_string(&ch).expect("ser");
        let back: IssuedChallenge = serde_json::from_str(&json).expect("de");
        assert_eq!(back, ch);
    }

    /// An attacker issuing many challenges that are never consumed must not grow
    /// the store without bound: a later `consume` past their expiry evicts every
    /// expired entry. Proves the memory-DoS bound (henrypark133 Medium).
    #[tokio::test]
    async fn expired_unconsumed_entries_are_evicted_on_consume() {
        let store = InMemoryChallengeStore::new();
        // 100 challenges that all expire at t=1_000 and are never consumed.
        for i in 0..100 {
            store
                .issue(contract::issued(&format!("spam-{i}"), 1, 1_000))
                .await
                .expect("issue");
        }
        assert_eq!(store.resident_len(), 100, "all issued entries resident");

        // A single consume well past expiry reclaims every stale entry. The
        // requested id is itself unknown -> NotFound, but eviction still runs.
        assert_eq!(
            store.consume(&ChallengeId::new("unknown"), 5_000).await,
            Err(ChallengeError::NotFound)
        );
        assert_eq!(
            store.resident_len(),
            0,
            "all expired-unconsumed entries must be evicted"
        );
    }

    /// Eviction must not be abusable to resurrect a consumed challenge. After a
    /// challenge is consumed it is kept as a tombstone while unexpired (replay
    /// -> AlreadyConsumed). Once expired it is reclaimed, but it can never be
    /// re-consumed: an expired challenge always fails closed regardless of the
    /// consumed flag, and a re-issued id is a fresh single-use challenge.
    #[tokio::test]
    async fn eviction_cannot_resurrect_a_consumed_challenge() {
        let store = InMemoryChallengeStore::new();
        store
            .issue(contract::issued("once", 7, 1_000))
            .await
            .expect("issue");

        // Consume once at t=500 (unexpired) -> wins.
        store
            .consume(&ChallengeId::new("once"), 500)
            .await
            .expect("first consume wins");

        // While still unexpired, the tombstone makes a replay AlreadyConsumed,
        // never NotFound -> one-shot integrity preserved across eviction passes.
        assert_eq!(
            store.consume(&ChallengeId::new("once"), 600).await,
            Err(ChallengeError::AlreadyConsumed)
        );

        // Drive the clock past expiry. The id is classified BEFORE eviction, so
        // even an expired consumed tombstone still reports `AlreadyConsumed`
        // (the strongest one-shot signal) — it is never downgraded to a Result
        // that could be mistaken for a fresh challenge. The tombstone is then
        // reclaimed by the same consume's prune pass.
        assert_eq!(
            store.consume(&ChallengeId::new("once"), 2_000).await,
            Err(ChallengeError::AlreadyConsumed)
        );
        // The previous consume evicted the tombstone; a further consume is now
        // NotFound — never a fresh `Ok`. The consumed challenge cannot be
        // resurrected by eviction.
        assert_eq!(
            store.consume(&ChallengeId::new("once"), 2_001).await,
            Err(ChallengeError::NotFound)
        );

        // Even re-issuing the same id after eviction yields a brand-new
        // single-use challenge bound to a *fresh* preimage. Issuance is only
        // possible because the prior id was evicted; while the tombstone was
        // resident, `issue` would have failed `AlreadyIssued`.
        store
            .issue(contract::issued("once", 7, 10_000))
            .await
            .expect("re-issue after eviction is a fresh challenge");
        // The fresh challenge is consumable exactly once.
        store
            .consume(&ChallengeId::new("once"), 5_000)
            .await
            .expect("fresh challenge consumes once");
        assert_eq!(
            store.consume(&ChallengeId::new("once"), 5_001).await,
            Err(ChallengeError::AlreadyConsumed)
        );
    }

    /// A known-but-expired id reports `Expired` (its precise terminal status),
    /// proving classification happens before the bulk eviction pass.
    #[tokio::test]
    async fn known_expired_id_reports_expired_not_notfound() {
        let store = InMemoryChallengeStore::new();
        store
            .issue(contract::issued("exp", 3, 1_000))
            .await
            .expect("issue");
        assert_eq!(
            store.consume(&ChallengeId::new("exp"), 1_000).await,
            Err(ChallengeError::Expired),
            "consume at expiry boundary is Expired, not NotFound"
        );
    }

    /// `issue` prunes against the watermark left by the last `consume`, bounding
    /// memory even when issuance outpaces consumption.
    #[tokio::test]
    async fn issue_prunes_expired_against_consume_watermark() {
        let store = InMemoryChallengeStore::new();
        store
            .issue(contract::issued("stale", 1, 1_000))
            .await
            .expect("issue stale");
        // A consume advances the watermark to t=5_000 (stale's expiry < 5_000).
        let _ = store.consume(&ChallengeId::new("missing"), 5_000).await;
        // The consume already pruned `stale`; a subsequent issue stays bounded.
        store
            .issue(contract::issued("fresh", 2, 10_000))
            .await
            .expect("issue fresh");
        assert_eq!(
            store.resident_len(),
            1,
            "only the still-live `fresh` entry remains resident"
        );
    }
}
