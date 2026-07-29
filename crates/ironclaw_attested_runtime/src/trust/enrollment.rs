//! The [`TrustEnrollment`] ceremony state machine.
//!
//! `TrustEnrollment` is the shared shape for *registering a trust anchor* for a
//! user. This PR implements the **connected-wallet** kind ([`TrustKind::ConnectedWallet`]);
//! custodial-key and WebAuthn kinds are future (#4051) and slot in as
//! additional [`TrustKind`] variants with their own verifiers — the state
//! machine here is kind-agnostic.
//!
//! States advance `Pending → Challenged → Active`, with terminal
//! `Revoked` / `Expired` / `Failed`. Kind-specific control-of-account
//! verification ([`super::verifier`]) is kept strictly separate from the
//! transition bookkeeping below. There is no durable `Verified` state: a
//! single atomic compare-and-swap moves `Challenged → Active` once the
//! signature has verified (recording the evidence hash in the same step), so
//! no intermediate row is ever persisted. Splitting it into a separately
//! stored `Verified` state would create a second CAS — and a window in which a
//! verified-but-not-active enrollment could be observed or raced — for no
//! benefit, since the binding is only created on the `Active` transition.
//!
//! Idempotency: an enrollment is keyed by a stable `idempotency_key`
//! `(tenant, user, chain, network, claimed_account)`; re-initiating resumes the
//! same ceremony rather than minting a fresh challenge.

use serde::{Deserialize, Serialize};

use ironclaw_signing_provider::{ActorId, ChainId, TenantId, UserId};

/// The kind of trust anchor being registered.
///
/// Only [`TrustKind::ConnectedWallet`] is implemented here; the variant exists
/// so the state machine and store are shaped for the future custodial-key and
/// WebAuthn kinds (#4051) without reshaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustKind {
    /// An external wallet the user connected and proved control of.
    ConnectedWallet,
}

/// The lifecycle state of a [`TrustEnrollment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentState {
    /// Created, no challenge issued yet.
    Pending,
    /// A single-use challenge has been issued; awaiting a signed response.
    Challenged,
    /// The signed challenge verified control of the claimed account *and* an
    /// active [`super::TrustedSignerBinding`] was persisted from this ceremony.
    /// The `Challenged → Active` transition is a single atomic CAS that also
    /// records the evidence hash — there is no separately persisted `Verified`
    /// state.
    Active,
    /// Explicitly revoked.
    Revoked,
    /// The challenge expired before a valid response.
    Expired,
    /// Verification failed (wrong signer, forged/expired challenge, etc.).
    Failed,
}

/// One trust-registration ceremony.
///
/// Carries no secret material: only the public claim, the challenge hash, and
/// (post-verify) the evidence hash. `challenge_hash` and `evidence_hash` are
/// kept as hex of the 32-byte challenge digest for constant-time comparison and
/// audit without storing the challenge body or any signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEnrollment {
    /// Server-issued unique enrollment id (the `complete_registration` handle).
    pub enrollment_id: String,
    /// Tenant boundary.
    pub tenant_id: TenantId,
    /// End user.
    pub user_id: UserId,
    /// The kind of anchor (connected wallet here).
    pub kind: TrustKind,
    /// Target chain.
    pub chain_id: ChainId,
    /// Network within the chain family.
    pub network: String,
    /// The account/key the user claims to control.
    pub claimed_account: String,
    /// Stable idempotency key; re-initiating with this key resumes the same
    /// ceremony.
    pub idempotency_key: String,
    /// Hex of the issued challenge digest (`None` until `Challenged`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_hash: Option<String>,
    /// The nonce committed in the issued challenge (`None` until `Challenged`).
    /// Stored so an idempotent re-initiate reconstructs the *identical*
    /// challenge (same digest) rather than minting a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_nonce_hex: Option<String>,
    /// Hex of the verified evidence digest (`None` until `Active`). Equals the
    /// challenge digest that was actually signed; recorded atomically with the
    /// `Challenged → Active` transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_hash: Option<String>,
    /// The acting principal that initiated the ceremony.
    pub actor: ActorId,
    /// Current lifecycle state.
    pub state: EnrollmentState,
    /// Creation time (unix millis).
    pub created_at_unix_ms: u64,
    /// Last-update time (unix millis).
    pub updated_at_unix_ms: u64,
    /// Challenge expiry (unix millis); set when `Challenged`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
}

impl TrustEnrollment {
    /// Build a fresh `Pending` enrollment.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pending(
        enrollment_id: String,
        tenant_id: TenantId,
        user_id: UserId,
        chain_id: ChainId,
        network: String,
        claimed_account: String,
        idempotency_key: String,
        actor: ActorId,
        now_unix_ms: u64,
    ) -> Self {
        Self {
            enrollment_id,
            tenant_id,
            user_id,
            kind: TrustKind::ConnectedWallet,
            chain_id,
            network,
            claimed_account,
            idempotency_key,
            challenge_hash: None,
            challenge_nonce_hex: None,
            evidence_hash: None,
            actor,
            state: EnrollmentState::Pending,
            created_at_unix_ms: now_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            expires_at_unix_ms: None,
        }
    }

    /// Record the issued challenge and advance to `Challenged`.
    pub(super) fn mark_challenged(
        &mut self,
        challenge_hash: String,
        challenge_nonce_hex: String,
        expires_at_unix_ms: u64,
        now_unix_ms: u64,
    ) {
        self.challenge_hash = Some(challenge_hash);
        self.challenge_nonce_hex = Some(challenge_nonce_hex);
        self.expires_at_unix_ms = Some(expires_at_unix_ms);
        self.state = EnrollmentState::Challenged;
        self.updated_at_unix_ms = now_unix_ms;
    }

    /// Record the verified evidence hash and advance straight to `Active`.
    ///
    /// This is the single durable post-challenge transition: the signature has
    /// verified, so we record the evidence and mark the ceremony active in one
    /// step. The caller persists this via a `Challenged → Active` CAS, so the
    /// evidence hash and the active state are committed atomically — there is no
    /// intermediate `Verified` row.
    pub(super) fn mark_active(&mut self, evidence_hash: String, now_unix_ms: u64) {
        self.evidence_hash = Some(evidence_hash);
        self.state = EnrollmentState::Active;
        self.updated_at_unix_ms = now_unix_ms;
    }

    /// Advance to a terminal failure state.
    pub(super) fn mark_failed(&mut self, now_unix_ms: u64) {
        self.state = EnrollmentState::Failed;
        self.updated_at_unix_ms = now_unix_ms;
    }
}
