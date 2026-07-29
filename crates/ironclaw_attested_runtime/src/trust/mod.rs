//! Connected-wallet **trust registration** ceremony + the
//! `lookup_active_binding` seam (#4052).
//!
//! This subsystem lets a user register an external wallet as a *trusted signer*
//! for their account, independent of any single transaction. It produces a
//! [`TrustedSignerBinding`] — "this account/key is trusted for this
//! `(tenant, user, chain, network)`" — which the gate-raise side will later pin
//! `expected_signer` / `expected_access_key` / `expected_signing_payload` from.
//!
//! ## Scope boundary
//!
//! This is **enrollment evidence only**. It deliberately does NOT touch the
//! gate raise / `AttestedGateBinding`: a binding says an account is trusted for
//! a user, nothing more. Per-gate signing authorization (exact payload pin +
//! one-shot sealed grant + tenant policy) still happens at resolve, unchanged.
//! [`TrustRegistrar::lookup_active_binding`] is the single seam the integrated
//! raise side consumes — see the `follow-up` note on
//! [`TrustStore::lookup_active_binding`].
//!
//! ## Flow
//!
//! 1. [`TrustRegistrar::initiate_registration`] → issues a typed,
//!    domain-separated, single-use, expiring [`TrustChallenge`] → `Challenged`.
//!    Idempotent on `(tenant, user, chain, network, claimed_account)`:
//!    re-initiating an in-flight ceremony returns the same challenge.
//! 2. [`TrustRegistrar::complete_registration`] → verifies control of the
//!    claimed account (EVM ecrecover / Solana ed25519 / NEAR access-key
//!    ed25519, reusing `ironclaw_wallet_external` crypto) → atomically moves
//!    the enrollment `Challenged → Active` and persists an `Active`
//!    [`TrustedSignerBinding`] (no intermediate `Verified` row).
//! 3. [`TrustRegistrar::revoke_binding`] → `Revoked`; expiry → not resolvable.
//!
//! ## Invariants
//!
//! * The challenge is single-use, domain-separated, tenant/user/chain-bound,
//!   and expiring. Nonce / evidence comparisons are constant-time.
//! * No secret material is stored — only public accounts/keys and evidence
//!   hashes.
//! * Tenant isolation: a binding for tenant A never resolves for tenant B
//!   (the store keys tenant-first).

mod challenge;
mod enrollment;
mod store;
mod verifier;

use sha3::{Digest, Keccak256};
use subtle::ConstantTimeEq;

use ironclaw_signing_provider::{ActorId, ChainId, SigningProviderError, TenantId, UserId};

pub use challenge::TrustChallenge;
pub use enrollment::{EnrollmentState, TrustEnrollment, TrustKind};
pub use store::{BindingKey, BindingStatus, InMemoryTrustStore, TrustStore, TrustedSignerBinding};
#[cfg(any(test, feature = "unsafe-always-trust-near"))]
pub use verifier::AlwaysTrustNearAccessKeyVerifier;
pub use verifier::{NearAccessKeyVerifier, VerifiedControl};

/// A source of per-challenge random nonces.
///
/// Injected so the ceremony has no opinion on the entropy source and tests are
/// deterministic. Production wiring supplies a CSPRNG-backed source — use
/// [`CsprngNonceSource`], the shipped OS-entropy implementation.
pub trait NonceSource: Send + Sync {
    /// Return a fresh nonce as lowercase hex. Must be unpredictable in
    /// production (replay/forgery defense).
    ///
    /// Returns [`TrustError::NonceUnavailable`] if the entropy source is
    /// unavailable: the ceremony fails closed rather than minting a weak nonce.
    fn next_nonce_hex(&self) -> Result<String, TrustError>;
}

/// Production [`NonceSource`] backed by the operating system CSPRNG.
///
/// Each call draws 32 fresh bytes from the OS entropy source (`getrandom`,
/// i.e. `getrandom(2)` / `/dev/urandom` / `BCryptGenRandom`) and returns them
/// as lowercase hex. This is the canonical implementation production callers
/// MUST use: a predictable nonce would let a race attacker who observes one
/// challenge pre-compute the digest of the next, eroding the replay/forgery
/// resistance the nonce exists to provide. The deterministic sequential source
/// used by the tests is `#[cfg(test)]`-only and never compiled into a release
/// build.
#[derive(Debug, Default, Clone, Copy)]
pub struct CsprngNonceSource;

impl CsprngNonceSource {
    /// Construct an OS-CSPRNG-backed nonce source.
    pub fn new() -> Self {
        Self
    }
}

impl NonceSource for CsprngNonceSource {
    fn next_nonce_hex(&self) -> Result<String, TrustError> {
        let mut bytes = [0u8; 32];
        // `getrandom` only errors if the OS entropy source is unavailable — a
        // catastrophic platform failure, not a normal runtime condition. We
        // cannot mint an unpredictable challenge without it, so fail closed
        // (propagating to the caller's `Result`) rather than silently emitting
        // a weak/zero nonce or panicking.
        getrandom::getrandom(&mut bytes)
            .map_err(|e| TrustError::NonceUnavailable(e.to_string()))?;
        Ok(hex_encode(&bytes))
    }
}

/// Errors from the trust-registration ceremony.
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    /// No enrollment exists for the supplied id.
    #[error("no enrollment found for id {0}")]
    EnrollmentNotFound(String),
    /// The enrollment is not in a state that accepts completion.
    #[error("enrollment {id} is not awaiting a challenge response (state: {state:?})")]
    NotChallengeable {
        /// The enrollment id.
        id: String,
        /// The current state.
        state: EnrollmentState,
    },
    /// The challenge has expired.
    #[error("challenge expired for enrollment {0}")]
    ChallengeExpired(String),
    /// The supplied signed challenge did not match the issued challenge.
    #[error("signed challenge does not match the issued challenge")]
    ChallengeMismatch,
    /// The chain id could not be mapped to a supported chain family.
    #[error("unsupported chain for trust registration: {0}")]
    UnsupportedChain(String),
    /// The OS CSPRNG was unavailable, so no secure challenge nonce could be
    /// minted. A catastrophic platform failure, surfaced fail-closed.
    #[error("secure challenge nonce unavailable: {0}")]
    NonceUnavailable(String),
    /// Control-of-account verification failed.
    #[error(transparent)]
    Verification(#[from] SigningProviderError),
}

/// The submitted proof completing a registration: the challenge the wallet
/// signed plus the signature (and, for ed25519 chains, the public key).
#[derive(Debug, Clone)]
pub struct SignedChallenge {
    /// The challenge the wallet was issued and signed (echoed back verbatim).
    pub challenge: TrustChallenge,
    /// The raw signature over [`TrustChallenge::digest`] (EIP-191-framed for
    /// EVM): 65 bytes for EVM, 64 bytes for ed25519 chains.
    pub signature: Vec<u8>,
    /// The signer public key, required for Solana/NEAR (ed25519), ignored for
    /// EVM. For Solana this is the account pubkey; for NEAR the access key.
    pub public_key_hex: Option<String>,
}

/// Drives the trust-registration ceremony over a [`TrustStore`].
///
/// Generic over the store and the NEAR access-key verifier so the durable
/// store and RPC-backed NEAR check (gap-D) drop in without touching this logic.
pub struct TrustRegistrar<S: TrustStore, N: NearAccessKeyVerifier> {
    store: S,
    nonce_source: Box<dyn NonceSource>,
    near_verifier: N,
    /// Challenge time-to-live in milliseconds.
    challenge_ttl_ms: u64,
}

impl<S: TrustStore, N: NearAccessKeyVerifier> TrustRegistrar<S, N> {
    /// Construct a registrar.
    pub fn new(
        store: S,
        nonce_source: Box<dyn NonceSource>,
        near_verifier: N,
        challenge_ttl_ms: u64,
    ) -> Self {
        Self {
            store,
            nonce_source,
            near_verifier,
            challenge_ttl_ms,
        }
    }

    /// Borrow the underlying store (e.g. for the raise-side `lookup_active_binding`).
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Begin a registration: issue a single-use, expiring, domain-separated
    /// challenge bound to `(tenant, user, chain, network, claimed_account)` and
    /// move the enrollment to `Challenged`.
    ///
    /// Idempotent: if an in-flight (`Pending`/`Challenged`) enrollment already
    /// exists for the same idempotency key, its existing challenge is returned
    /// rather than minting a new one (retries resume the same ceremony).
    #[allow(clippy::too_many_arguments)]
    pub async fn initiate_registration(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
        chain_id: ChainId,
        network: String,
        claimed_account: String,
        actor: ActorId,
        now_unix_ms: u64,
    ) -> Result<(TrustEnrollment, TrustChallenge), TrustError> {
        // Normalize the claimed account to its canonical per-chain form *at the
        // API boundary*, before it feeds the idempotency key, the stored
        // enrollment, and the challenge digest. Without this, the same physical
        // account submitted in two surface forms (EIP-55 mixed-case vs lowercase
        // EVM; base58 vs hex Solana; mixed-case NEAR id) would hash to two
        // distinct idempotency keys — minting parallel enrollment slots that
        // both collapse onto the single account-less `BindingKey`, leaving the
        // binding's `account_or_key` non-deterministic. Canonicalizing here makes
        // one physical account map to exactly one ceremony and one binding.
        let family = chain_family(&chain_id)
            .ok_or_else(|| TrustError::UnsupportedChain(chain_id.to_string()))?;
        let claimed_account = normalize_claimed_account(family, &claimed_account)?;
        let idempotency_key =
            idempotency_key(&tenant_id, &user_id, &chain_id, &network, &claimed_account);

        // Idempotent resume: an in-flight ceremony with the same key returns its
        // existing challenge (reconstructed from the stored nonce + expiry, so
        // the digest is byte-identical) instead of minting a fresh one.
        if let Some(existing) = self.store.get_enrollment(&idempotency_key).await
            && matches!(
                existing.state,
                EnrollmentState::Pending | EnrollmentState::Challenged
            )
            && existing
                .expires_at_unix_ms
                .is_none_or(|exp| now_unix_ms < exp)
            && let Some(challenge) = challenge_from_enrollment(&existing)
        {
            return Ok((existing, challenge));
        }

        let mut enrollment = TrustEnrollment::pending(
            new_enrollment_id(&idempotency_key, now_unix_ms),
            tenant_id,
            user_id,
            chain_id,
            network,
            claimed_account,
            idempotency_key.clone(),
            actor,
            now_unix_ms,
        );

        let nonce_hex = self.nonce_source.next_nonce_hex()?;
        let expires_at_unix_ms = now_unix_ms + self.challenge_ttl_ms;
        let challenge = build_challenge(&enrollment, nonce_hex.clone(), expires_at_unix_ms);
        let challenge_hash = hex_encode(&challenge.digest());
        enrollment.mark_challenged(challenge_hash, nonce_hex, expires_at_unix_ms, now_unix_ms);

        // Atomic get-or-insert: under a concurrent double-submit for the same
        // idempotency key, exactly one candidate is stored. The loser receives
        // the winner's stored enrollment and reconstructs its (byte-identical)
        // challenge, rather than being handed a challenge whose enrollment was
        // silently overwritten and can therefore never complete.
        let (stored, inserted) = self.store.put_enrollment_if_absent(enrollment).await;
        if inserted {
            return Ok((stored, challenge));
        }
        // We lost the insert race (or a slot already existed): resume the stored
        // ceremony if it is still in-flight and reconstructable.
        if matches!(
            stored.state,
            EnrollmentState::Pending | EnrollmentState::Challenged
        ) && stored
            .expires_at_unix_ms
            .is_none_or(|exp| now_unix_ms < exp)
            && let Some(existing_challenge) = challenge_from_enrollment(&stored)
        {
            return Ok((stored, existing_challenge));
        }
        // Stored slot is terminal/expired: replace it with our fresh ceremony.
        // The `challenge` we already built is bound only to the idempotency-key
        // tuple + nonce + expiry (not the enrollment id), so it is valid for the
        // fresh enrollment we now persist over the dead slot.
        let mut fresh = TrustEnrollment::pending(
            new_enrollment_id(&idempotency_key, now_unix_ms),
            stored.tenant_id.clone(),
            stored.user_id.clone(),
            stored.chain_id.clone(),
            stored.network.clone(),
            stored.claimed_account.clone(),
            idempotency_key,
            stored.actor.clone(),
            now_unix_ms,
        );
        fresh.mark_challenged(
            hex_encode(&challenge.digest()),
            challenge.nonce_hex.clone(),
            expires_at_unix_ms,
            now_unix_ms,
        );
        self.store.put_enrollment(fresh.clone()).await;
        Ok((fresh, challenge))
    }

    /// Complete a registration: verify control of the claimed account from the
    /// signed challenge, then persist an `Active` [`TrustedSignerBinding`].
    ///
    /// Fails closed on a wrong signer, a forged/mismatched challenge, an expired
    /// challenge, or a replayed challenge (the enrollment is no longer
    /// `Challenged` after the first success).
    pub async fn complete_registration(
        &self,
        enrollment_id: &str,
        signed: SignedChallenge,
        now_unix_ms: u64,
    ) -> Result<TrustedSignerBinding, TrustError> {
        let mut enrollment = self
            .store
            .get_enrollment_by_id(enrollment_id)
            .await
            .ok_or_else(|| TrustError::EnrollmentNotFound(enrollment_id.to_string()))?;

        // Replay defense: only a `Challenged` enrollment can be completed. A
        // second completion of an already-`Active` ceremony is rejected here.
        if enrollment.state != EnrollmentState::Challenged {
            return Err(TrustError::NotChallengeable {
                id: enrollment_id.to_string(),
                state: enrollment.state,
            });
        }

        // The submitted challenge must be byte-identical to the one we issued
        // (constant-time compare on the digest hex). This binds the signature to
        // exactly our tenant/user/chain/nonce/expiry commitment.
        let submitted_hash = hex_encode(&signed.challenge.digest());
        let issued_hash = enrollment.challenge_hash.as_deref().unwrap_or_default();
        if !ct_eq(submitted_hash.as_bytes(), issued_hash.as_bytes()) {
            return Err(TrustError::ChallengeMismatch);
        }

        // Expiry (fail-closed). Checked against the issued expiry recorded on the
        // enrollment, not just the echoed challenge.
        if signed.challenge.is_expired(now_unix_ms)
            || enrollment
                .expires_at_unix_ms
                .is_some_and(|exp| now_unix_ms >= exp)
        {
            let mut failed = enrollment.clone();
            failed.mark_failed(now_unix_ms);
            // Only mark failed if still `Challenged`, so a concurrent winner is
            // never clobbered by this caller's expiry verdict.
            self.store
                .compare_and_swap_enrollment_state(
                    enrollment_id,
                    EnrollmentState::Challenged,
                    failed,
                )
                .await;
            return Err(TrustError::ChallengeExpired(enrollment_id.to_string()));
        }

        let family = chain_family(&signed.challenge.chain_id)
            .ok_or_else(|| TrustError::UnsupportedChain(signed.challenge.chain_id.to_string()))?;

        let verified = match self.verify_control(family, &signed) {
            Ok(v) => v,
            Err(e) => {
                let mut failed = enrollment.clone();
                failed.mark_failed(now_unix_ms);
                // Only mark failed if still `Challenged` (don't clobber a winner).
                self.store
                    .compare_and_swap_enrollment_state(
                        enrollment_id,
                        EnrollmentState::Challenged,
                        failed,
                    )
                    .await;
                return Err(e.into());
            }
        };

        let evidence_hash = submitted_hash;
        let access_key = match &verified {
            VerifiedControl::Near { public_key, .. } => Some(public_key.clone()),
            _ => None,
        };

        // Verification above is side-effect-free (ecrecover / ed25519 verify), so
        // concurrent completers may both reach here. The single-use invariant is
        // enforced by an atomic compare-and-swap: only the completer that moves
        // the enrollment `Challenged -> Active` is the winner and goes on to
        // persist the binding. A second (replayed/concurrent) completion finds
        // the state already moved and fails closed with `NotChallengeable`.
        enrollment.mark_active(evidence_hash.clone(), now_unix_ms);
        let won = self
            .store
            .compare_and_swap_enrollment_state(
                enrollment_id,
                EnrollmentState::Challenged,
                enrollment.clone(),
            )
            .await;
        if !won {
            // Lost the race (or the challenge was already consumed): re-read to
            // surface the now-current state in the error.
            let state = self
                .store
                .get_enrollment_by_id(enrollment_id)
                .await
                .map(|e| e.state)
                .unwrap_or(EnrollmentState::Active);
            return Err(TrustError::NotChallengeable {
                id: enrollment_id.to_string(),
                state,
            });
        }

        let binding = TrustedSignerBinding {
            tenant_id: enrollment.tenant_id.clone(),
            user_id: enrollment.user_id.clone(),
            chain_id: enrollment.chain_id.clone(),
            network: enrollment.network.clone(),
            account_or_key: verified.account_or_key(),
            access_key,
            evidence_hash,
            status: BindingStatus::Active,
            created_at_unix_ms: now_unix_ms,
            expires_at_unix_ms: None,
            revoked_at_unix_ms: None,
        };

        self.store.put_binding(binding.clone()).await;
        Ok(binding)
    }

    /// The raise-side seam: the active, unexpired trusted-signer binding for
    /// `(tenant, user, chain, network)`, or `None` (caller fails closed).
    ///
    /// follow-up (gap D / binding-fields): the integrated gate raise consumes
    /// this to pin `expected_signer` / `expected_access_key` /
    /// `expected_signing_payload` onto the `AttestedGateBinding`.
    pub async fn lookup_active_binding(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        chain_id: &ChainId,
        network: &str,
        now_unix_ms: u64,
    ) -> Option<TrustedSignerBinding> {
        self.store
            .lookup_active_binding(tenant_id, user_id, chain_id, network, now_unix_ms)
            .await
    }

    /// Revoke the binding for `(tenant, user, chain, network)`. After this it
    /// never resolves from [`Self::lookup_active_binding`].
    pub async fn revoke_binding(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        chain_id: &ChainId,
        network: &str,
        now_unix_ms: u64,
    ) -> bool {
        let key = BindingKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            chain_id: chain_id.clone(),
            network: network.to_string(),
        };
        if let Some(mut binding) = self.store.get_binding(&key).await {
            binding.status = BindingStatus::Revoked;
            binding.revoked_at_unix_ms = Some(now_unix_ms);
            self.store.put_binding(binding).await;
            true
        } else {
            false
        }
    }

    /// Route to the per-chain control-of-account verifier.
    fn verify_control(
        &self,
        family: TrustChainFamily,
        signed: &SignedChallenge,
    ) -> Result<VerifiedControl, SigningProviderError> {
        match family {
            TrustChainFamily::Evm => verifier::verify_evm(&signed.challenge, &signed.signature),
            TrustChainFamily::Solana => {
                let pk =
                    signed
                        .public_key_hex
                        .as_deref()
                        .ok_or(SigningProviderError::ProofInvalid {
                            reason: "solana trust registration requires a public_key".to_string(),
                        })?;
                let pk_bytes = decode_hex(pk)?;
                verifier::verify_solana(&signed.challenge, &signed.signature, &pk_bytes)
            }
            TrustChainFamily::Near => {
                let pk =
                    signed
                        .public_key_hex
                        .as_deref()
                        .ok_or(SigningProviderError::ProofInvalid {
                            reason: "near trust registration requires an access-key public_key"
                                .to_string(),
                        })?;
                verifier::verify_near(
                    &signed.challenge,
                    &signed.signature,
                    pk,
                    &self.near_verifier,
                )
            }
        }
    }
}

/// Build the canonical challenge for an enrollment from an explicit nonce +
/// expiry. The challenge digest is fully determined by the enrollment's binding
/// fields plus these two values.
fn build_challenge(
    enrollment: &TrustEnrollment,
    nonce_hex: String,
    expires_at_unix_ms: u64,
) -> TrustChallenge {
    TrustChallenge {
        tenant_id: enrollment.tenant_id.clone(),
        user_id: enrollment.user_id.clone(),
        chain_id: enrollment.chain_id.clone(),
        network: enrollment.network.clone(),
        claimed_account: enrollment.claimed_account.clone(),
        nonce_hex,
        expires_at_unix_ms,
    }
}

/// Reconstruct the byte-identical issued challenge from a `Challenged`
/// enrollment's stored nonce + expiry (idempotent resume).
fn challenge_from_enrollment(enrollment: &TrustEnrollment) -> Option<TrustChallenge> {
    let nonce_hex = enrollment.challenge_nonce_hex.clone()?;
    let expires_at_unix_ms = enrollment.expires_at_unix_ms?;
    Some(build_challenge(enrollment, nonce_hex, expires_at_unix_ms))
}

/// The wallet/crypto family a trust registration targets.
///
/// Owned locally rather than reusing `ironclaw_wallet_external::ChainFamily`:
/// the WalletConnect provider's `ChainFamily` deliberately omits NEAR (it
/// fail-closes on the `near` namespace until a real NEAR transaction-signer
/// verifier exists), but trust *registration* proves control of an account via
/// a challenge signature and does support NEAR access keys (see
/// [`verifier::verify_near`]). Keeping a separate enum lets the two subsystems
/// evolve their chain support independently. The crypto kernels themselves are
/// still reused from `ironclaw_wallet_external` (see [`verifier`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrustChainFamily {
    /// EVM chains (`eip155:*`): EIP-191 `personal_sign`, secp256k1 ecrecover.
    Evm,
    /// Solana clusters (`solana:*`): ed25519 `signMessage`.
    Solana,
    /// NEAR (`near:*`): ed25519 access-key signature.
    Near,
}

/// Resolve the wallet/crypto family from a CAIP-2 chain id's namespace.
///
/// `eip155` → EVM, `solana` → Solana, `near` → NEAR; returns `None` for
/// anything else so the caller can fail closed.
fn chain_family(chain_id: &ChainId) -> Option<TrustChainFamily> {
    match chain_id.as_str().split_once(':').map(|(ns, _)| ns)? {
        "eip155" => Some(TrustChainFamily::Evm),
        "solana" => Some(TrustChainFamily::Solana),
        "near" => Some(TrustChainFamily::Near),
        _ => None,
    }
}

/// Canonicalize a `claimed_account` to the per-chain form the rest of the
/// ceremony (idempotency key, stored enrollment, challenge digest, signer
/// match) keys on. Applied once at the API boundary so a single physical
/// account always maps to a single ceremony and a single binding.
///
/// * **EVM** — lowercase, `0x`-prefixed hex address.
/// * **Solana** — lowercase hex of the 32-byte ed25519 pubkey. Real wallets
///   present base58 (Phantom/Solflare `signMessage`), so a base58-decodable
///   32-byte input is accepted and converted; a 64-char hex input is accepted
///   as-is. Anything else fails closed with a clear error.
/// * **NEAR** — lowercase `account_id` (NEAR named/implicit accounts are
///   case-insensitive and conventionally lowercase).
fn normalize_claimed_account(
    family: TrustChainFamily,
    claimed_account: &str,
) -> Result<String, TrustError> {
    match family {
        TrustChainFamily::Evm => Ok(verifier::normalize_evm(claimed_account)),
        TrustChainFamily::Solana => {
            verifier::normalize_solana_pubkey(claimed_account).map_err(TrustError::from)
        }
        TrustChainFamily::Near => Ok(claimed_account.to_ascii_lowercase()),
    }
}

/// Stable idempotency key for a `(tenant, user, chain, network, account)` tuple.
fn idempotency_key(
    tenant: &TenantId,
    user: &UserId,
    chain: &ChainId,
    network: &str,
    account: &str,
) -> String {
    let mut hasher = Keccak256::new();
    for field in [
        tenant.as_str(),
        user.as_str(),
        chain.as_str(),
        network,
        account,
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    let out: [u8; 32] = hasher.finalize().into();
    hex_encode(&out)
}

/// Server-issued enrollment id, stable per idempotency key + creation time.
fn new_enrollment_id(idempotency_key: &str, created_at_unix_ms: u64) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(b"ironclaw/trust/enrollment-id/v1");
    hasher.update(idempotency_key.as_bytes());
    hasher.update(created_at_unix_ms.to_be_bytes());
    let out: [u8; 32] = hasher.finalize().into();
    hex_encode(&out)
}

/// Constant-time byte-slice equality (length-aware).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Lowercase hex of `bytes`. Hand-rolled (no runtime `hex`/`const-hex`
/// dependency) but allocation-free per byte: a single `String` of the exact
/// capacity, pushing nibble chars directly rather than `format!`-ing each byte.
pub(super) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Decode a single ASCII hex digit byte to its 0-15 value, rejecting any
/// non-hex (including non-ASCII) byte without panicking. Shared by the hex
/// decoders so they never index a `&str` by byte (which panics on a non-char
/// boundary for attacker-controlled multi-byte UTF-8); they decode over
/// `.as_bytes()` instead. Mirrors the `hex_digit` in the wallet-external kernel.
pub(super) fn hex_digit(b: u8) -> Result<u8, SigningProviderError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(SigningProviderError::ProofInvalid {
            reason: format!("invalid hex digit: {other:#04x}"),
        }),
    }
}

fn decode_hex(s: &str) -> Result<Vec<u8>, SigningProviderError> {
    // Decode over raw bytes: `s` is attacker-controlled (the uncommitted
    // `public_key_hex`) and may carry even-byte-length multi-byte UTF-8, so
    // `&str` byte-range slicing would panic on a non-char boundary.
    let stripped = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if !stripped.len().is_multiple_of(2) {
        return Err(SigningProviderError::ProofInvalid {
            reason: "odd-length hex".to_string(),
        });
    }
    stripped
        .chunks_exact(2)
        .map(|pair| Ok((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests;
