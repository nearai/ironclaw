//! The signed intent: "this agent crafted exactly this transaction for exactly
//! this approver" (attested-signing Phase B).
//!
//! ## What the intent is, and what it is NOT
//!
//! The intent adds **attribution, integrity, and addressing** on top of the
//! existing gate machinery. It is NOT an authorization: nothing downstream may
//! treat an intent signature as permission to sign or to advance a turn.
//! Authorization to sign remains the human + device ceremony; authorization to
//! advance the turn remains the sealed one-shot grant CAS.
//!
//! The agent key lives on the same backend infrastructure that stores it, so
//! the signature detects tampering **across the untrusted chat-channel hop**
//! and pins which agent crafted what. It is deliberately not an independent
//! trust root.
//!
//! ## The bridge to the gate machinery
//!
//! An intent embeds the SAME [`ApprovedTxHash`] the gate binds
//! ([`crate::approved_tx_hash_for`]), computed exactly once at raise time.
//! There is no parallel hash scheme: the intent points at the existing binding
//! rather than re-deriving one, so an intent can never attest to different
//! bytes than the ones the device will clear-sign and the resume path will
//! verify against.
//!
//! ## Key material never enters this crate
//!
//! `ironclaw_attestation` is forbidden from depending on `ironclaw_secrets`
//! (pinned by `attested_signing_boundaries`). So this module owns the intent
//! type, its deterministic signing pre-image, and **verification** (a public
//! key is not secret) — while *signing* is an injected [`IntentSigner`] port
//! whose production implementation lives in the layer that owns the sealed
//! keystore. Raw private-key bytes have no path into this crate.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use ironclaw_signing_provider::{ApprovedTxHash, ChainId, TenantId, UserId};

use crate::decoded_tx::{DecodedTransaction, RenderingSchemaVersion};

/// Domain separator for the intent signing pre-image. Distinct from the
/// canonical-bytes and approved-tx-hash domains so the three pre-images can
/// never be confused for one another.
const INTENT_DOMAIN: &[u8] = b"ironclaw.attestation.intent.v1";

/// Length of an ed25519 public key, in bytes.
pub const AGENT_PUBLIC_KEY_LEN: usize = 32;

/// Length of an ed25519 signature, in bytes.
pub const INTENT_SIGNATURE_LEN: usize = 64;

/// Opaque identifier for one intent. ULID: lexicographically sortable by
/// creation time, which keeps durable index scans in creation order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntentId(String);

impl IntentId {
    /// Mint a fresh intent id.
    pub fn new() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    /// Wrap an existing id string (durable read-back).
    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for IntentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for IntentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies WHICH agent key signed an intent: `(tenant, agent, generation)`.
///
/// The generation is what makes rotation expressible — a new generation becomes
/// active while the previous one is still accepted for verification during a
/// bounded overlap window, so intents already in flight do not fail closed the
/// instant a key rotates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKeyId {
    /// Tenant boundary that owns the key.
    pub tenant: TenantId,
    /// The agent the key attributes to.
    pub agent: String,
    /// Monotonic key generation within `(tenant, agent)`.
    pub generation: u32,
}

impl AgentKeyId {
    /// Construct an agent key id.
    pub fn new(tenant: TenantId, agent: impl Into<String>, generation: u32) -> Self {
        Self {
            tenant,
            agent: agent.into(),
            generation,
        }
    }
}

/// Why an intent was rejected. Sanitized categories: no key material, no chain
/// detail, nothing an attacker could use to distinguish probe outcomes beyond
/// the category itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IntentError {
    /// `expires_at_ms` was not strictly after `created_at_ms` — the intent
    /// would be born already expired.
    #[error("intent expiry must be after its creation timestamp")]
    InvalidExpiry,

    /// A timestamp was negative (pre-epoch).
    #[error("intent timestamps must be non-negative")]
    InvalidTimestamp,

    /// The verifying key was not a well-formed ed25519 public key.
    #[error("agent verifying key is malformed")]
    MalformedKey,

    /// The signature did not verify against the pre-image under the supplied
    /// key. Covers both a tampered field and a wrong/forged signature — the two
    /// are deliberately indistinguishable to the caller.
    #[error("intent signature did not verify")]
    BadSignature,

    /// The intent was presented at or after `expires_at_ms`.
    #[error("intent has expired")]
    Expired,

    /// The intent's tenant did not match the tenant it was presented under.
    #[error("intent tenant mismatch")]
    TenantMismatch,

    /// The carried `decoded_tx` does not hash to the intent's
    /// `approved_tx_hash` (or could not be canonicalized at all). The intent
    /// attests to a different transaction than the one it carries.
    #[error("intent does not bind the transaction it carries")]
    TransactionMismatch,
}

/// An intent before it carries a signature.
///
/// Build one at raise time, take [`Self::signing_preimage`], hand those bytes to
/// the [`IntentSigner`] port, then seal it with [`Self::into_signed`]. Keeping
/// the unsigned form a distinct type makes "signed something other than what we
/// built" unrepresentable: the pre-image and the sealed record are derived from
/// the same value.
///
/// The serde impls exist so a durable backend can persist the intent body as a
/// single JSON column, alongside its signature, and reconstruct it with
/// [`SignedIntent::from_parts`]. Deserializing is *transport*, not trust: it
/// asserts nothing about the signature, and a record read back from storage is
/// still subject to [`SignedIntent::verify`] and
/// [`SignedIntent::verify_binds_transaction`] before anything acts on it.
/// `deny_unknown_fields` keeps a future field from being silently dropped on
/// the way through a round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsignedIntent {
    /// Identity of this intent.
    pub intent_id: IntentId,
    /// Tenant boundary.
    pub tenant: TenantId,
    /// Which agent key is expected to sign.
    pub agent_key_id: AgentKeyId,
    /// The human bound at craft time. Phase C's review link authorizes against
    /// exactly this user; the token never carries it.
    pub approver: UserId,
    /// Exact target chain id.
    pub chain_id: ChainId,
    /// The binding hash the gate machinery already uses — computed once by
    /// [`crate::approved_tx_hash_for`] and embedded here, never re-derived.
    pub approved_tx_hash: ApprovedTxHash,
    /// The server-decoded transaction this intent attests to.
    pub decoded_tx: DecodedTransaction,
    /// Creation time (unix millis).
    pub created_at_ms: i64,
    /// Expiry (unix millis), strictly after `created_at_ms`.
    pub expires_at_ms: i64,
    /// Rendering schema the hash was sealed under.
    pub schema_version: RenderingSchemaVersion,
}

impl UnsignedIntent {
    /// Validate the timestamp invariants. Fallible construction rather than a
    /// silently-accepted already-expired intent.
    pub fn validate(&self) -> Result<(), IntentError> {
        if self.created_at_ms < 0 || self.expires_at_ms < 0 {
            return Err(IntentError::InvalidTimestamp);
        }
        if self.expires_at_ms <= self.created_at_ms {
            return Err(IntentError::InvalidExpiry);
        }
        Ok(())
    }

    /// The deterministic, domain-separated signing pre-image.
    ///
    /// Same hand-rolled length-prefixed encoding as [`crate::canonical`] (see
    /// that module for why there is no CBOR dependency): every component is
    /// length-prefixed, which makes the encoding injective — no two distinct
    /// intents can produce the same bytes, so tampering with ANY field
    /// invalidates the signature.
    ///
    /// The transaction is bound via `approved_tx_hash`, which already commits
    /// to the render, the canonical bytes, the signer account, the chain, the
    /// tx type, and the schema version. Re-encoding the decoded transaction
    /// here would add a second, independently-maintained binding of the same
    /// facts — exactly the parallel-scheme drift this design avoids.
    pub fn signing_preimage(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(INTENT_DOMAIN);
        push_lp(&mut out, self.intent_id.as_str().as_bytes());
        push_lp(&mut out, self.tenant.as_str().as_bytes());
        push_lp(&mut out, self.agent_key_id.tenant.as_str().as_bytes());
        push_lp(&mut out, self.agent_key_id.agent.as_bytes());
        push_lp(&mut out, &self.agent_key_id.generation.to_be_bytes());
        push_lp(&mut out, self.approver.as_str().as_bytes());
        push_lp(&mut out, self.chain_id.as_str().as_bytes());
        push_lp(&mut out, self.approved_tx_hash.as_bytes());
        push_lp(&mut out, &self.created_at_ms.to_be_bytes());
        push_lp(&mut out, &self.expires_at_ms.to_be_bytes());
        push_lp(&mut out, &self.schema_version.get().to_be_bytes());
        out
    }

    /// Seal the intent with a signature produced over [`Self::signing_preimage`].
    pub fn into_signed(self, signature: [u8; INTENT_SIGNATURE_LEN]) -> SignedIntent {
        SignedIntent {
            intent: self,
            signature,
        }
    }
}

/// A sealed, verifiable intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIntent {
    intent: UnsignedIntent,
    signature: [u8; INTENT_SIGNATURE_LEN],
}

impl SignedIntent {
    /// The attested intent body.
    pub fn intent(&self) -> &UnsignedIntent {
        &self.intent
    }

    /// The raw signature bytes (durable persistence / audit).
    pub fn signature(&self) -> &[u8; INTENT_SIGNATURE_LEN] {
        &self.signature
    }

    /// Reconstruct a sealed intent read back from durable storage.
    pub fn from_parts(intent: UnsignedIntent, signature: [u8; INTENT_SIGNATURE_LEN]) -> Self {
        Self { intent, signature }
    }

    /// Verify that the transaction this intent CARRIES is the one its
    /// `approved_tx_hash` commits to, by recomputing the hash the same way the
    /// gate and the resume path do.
    ///
    /// The signing pre-image binds `approved_tx_hash`, not a second encoding of
    /// the transaction — so this recompute is what makes the carried
    /// `decoded_tx` trustworthy. Ingest MUST call it: without it, a tampered
    /// `decoded_tx` alongside an authentic hash would render one transaction in
    /// the review UI while the device signed another.
    ///
    /// `signer_account` is the gate-bound signer (`SigningContext`'s
    /// `key_or_account_id`) — the same input `recompute_approved_hash` uses, so
    /// this can never diverge from the sign-time re-check.
    pub fn verify_binds_transaction(&self, signer_account: &str) -> Result<(), IntentError> {
        let recomputed = crate::approved_tx_hash_for(
            &self.intent.decoded_tx,
            signer_account,
            self.intent.schema_version,
        )
        .map_err(|_| IntentError::TransactionMismatch)?;
        if recomputed != self.intent.approved_tx_hash {
            return Err(IntentError::TransactionMismatch);
        }
        Ok(())
    }

    /// Verify the intent fail-closed: signature over the pre-image under
    /// `verifying_key`, the presented tenant, and the expiry window.
    ///
    /// `now_ms` is supplied by the caller — this crate never reads the wall
    /// clock, so verification stays deterministic and expiry is testable.
    /// Expiry is inclusive at the boundary (`now >= expires_at` is expired).
    pub fn verify(
        &self,
        verifying_key: &[u8; AGENT_PUBLIC_KEY_LEN],
        presented_tenant: &TenantId,
        now_ms: i64,
    ) -> Result<(), IntentError> {
        // Cheap, non-cryptographic checks first — but note they leak nothing an
        // attacker could not already determine from the values they submitted.
        if self.intent.tenant.as_str() != presented_tenant.as_str() {
            return Err(IntentError::TenantMismatch);
        }

        let key = VerifyingKey::from_bytes(verifying_key).map_err(|_| IntentError::MalformedKey)?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.intent.signing_preimage(), &signature)
            .map_err(|_| IntentError::BadSignature)?;

        // Expiry AFTER signature verification: an unsigned/forged intent must
        // not be able to learn anything from the timing of the expiry check,
        // and an expired-but-authentic intent is a different category from a
        // forged one.
        if now_ms >= self.intent.expires_at_ms {
            return Err(IntentError::Expired);
        }
        Ok(())
    }
}

/// Append `len(bytes) ∥ bytes`, mirroring [`crate::canonical`]'s encoder.
fn push_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Injected signer for intent pre-images.
///
/// The implementation lives outside this crate, in the layer that owns the
/// sealed agent keystore (`ironclaw_secrets` is a forbidden dependency here).
/// This crate hands it opaque pre-image bytes and receives a signature; no
/// private key material ever crosses into the attestation core.
#[async_trait::async_trait]
pub trait IntentSigner: Send + Sync {
    /// The key `(tenant, agent)` should sign with right now, provisioning one
    /// on first use (§B4: keys are minted lazily, per agent).
    ///
    /// Returning the id rather than taking one keeps generation selection —
    /// and therefore rotation — entirely inside the implementation: a caller
    /// cannot ask to sign under a retired or revoked generation because it
    /// never names one.
    async fn active_key_id(
        &self,
        tenant: &TenantId,
        agent: &str,
        now_ms: i64,
    ) -> Result<AgentKeyId, IntentSignerError>;

    /// Sign `preimage` with the agent key identified by `key_id`.
    ///
    /// Implementations MUST fail closed when the key is unknown, revoked, or
    /// unusable — never by signing with a substitute key.
    fn sign_intent(
        &self,
        key_id: &AgentKeyId,
        preimage: &[u8],
    ) -> Result<[u8; INTENT_SIGNATURE_LEN], IntentSignerError>;
}

/// Why an intent could not be signed. Sanitized: carries no key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IntentSignerError {
    /// No active key exists for the requested `(tenant, agent, generation)`.
    #[error("no usable agent signing key")]
    UnknownKey,

    /// The key exists but is revoked — signing with it is refused.
    #[error("agent signing key is revoked")]
    RevokedKey,

    /// The keystore backend failed.
    #[error("agent keystore unavailable: {reason}")]
    Backend {
        /// Sanitized backend description.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    const NOW: i64 = 1_000;
    const EXPIRES: i64 = 61_000; // NOW + 60s

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn decoded() -> DecodedTransaction {
        DecodedTransaction::Evm(crate::decoded_tx::EvmTransaction {
            chain_id: 11155111,
            nonce: 7,
            tx_type: 2,
            to: Some(crate::decoded_tx::EvmAddress([0x11; 20])),
            value: vec![0x0d, 0xe0],
            data: vec![],
            gas_limit: 21_000,
            gas_price: None,
            max_fee_per_gas: Some(vec![0x09, 0x18]),
            max_priority_fee_per_gas: Some(vec![0x3b, 0x9a]),
            access_list: vec![],
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
        })
    }

    const SIGNER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// An intent whose `approved_tx_hash` genuinely binds its carried tx.
    fn bound_unsigned() -> UnsignedIntent {
        let mut intent = unsigned();
        intent.approved_tx_hash =
            crate::approved_tx_hash_for(&intent.decoded_tx, SIGNER, intent.schema_version)
                .expect("hashable sample tx");
        intent
    }

    fn unsigned() -> UnsignedIntent {
        UnsignedIntent {
            intent_id: IntentId::from_string("01J000000000000000000000AA"),
            tenant: TenantId::new("tenant-a"),
            agent_key_id: AgentKeyId::new(TenantId::new("tenant-a"), "agent-1", 1),
            approver: UserId::new("alice"),
            chain_id: ChainId::new("eip155:11155111"),
            approved_tx_hash: ApprovedTxHash::from_bytes([0xab; 32]),
            decoded_tx: decoded(),
            created_at_ms: NOW,
            expires_at_ms: EXPIRES,
            schema_version: RenderingSchemaVersion::CURRENT,
        }
    }

    fn sealed(intent: UnsignedIntent, key: &SigningKey) -> SignedIntent {
        let signature = key.sign(&intent.signing_preimage()).to_bytes();
        intent.into_signed(signature)
    }

    #[test]
    fn a_correctly_signed_intent_verifies() {
        let key = signing_key();
        let signed = sealed(unsigned(), &key);
        assert_eq!(
            signed.verify(
                &key.verifying_key().to_bytes(),
                &TenantId::new("tenant-a"),
                NOW
            ),
            Ok(())
        );
    }

    #[test]
    fn the_preimage_is_deterministic() {
        let intent = unsigned();
        assert_eq!(intent.signing_preimage(), intent.signing_preimage());
    }

    /// The injectivity property that makes the signature meaningful: mutating
    /// ANY signed field must change the pre-image, so the existing signature no
    /// longer verifies. A field that could be changed without invalidating the
    /// signature would be a field the agent never actually attested to.
    #[test]
    fn tampering_with_any_signed_field_breaks_verification() {
        let key = signing_key();
        let base = unsigned();
        let signature = key.sign(&base.signing_preimage()).to_bytes();

        // Each case carries the tenant to PRESENT at verification. For the
        // tenant mutation we present the tampered tenant, so the cheap tenant
        // check passes and the assertion isolates the property under test: the
        // SIGNATURE covers this field. (Presenting the original tenant would
        // pass too, via `TenantMismatch` — a weaker result that would not prove
        // the field is signed.)
        type Mutation = (&'static str, &'static str, Box<dyn Fn(&mut UnsignedIntent)>);
        let mutations: Vec<Mutation> = vec![
            (
                "intent_id",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| {
                    i.intent_id = IntentId::from_string("01J000000000000000000000BB")
                }),
            ),
            (
                "tenant",
                "tenant-b",
                Box::new(|i: &mut UnsignedIntent| i.tenant = TenantId::new("tenant-b")),
            ),
            (
                "agent_key_id.agent",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| i.agent_key_id.agent = "agent-2".to_string()),
            ),
            (
                "agent_key_id.generation",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| i.agent_key_id.generation = 2),
            ),
            (
                "approver",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| i.approver = UserId::new("mallory")),
            ),
            (
                "chain_id",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| i.chain_id = ChainId::new("eip155:1")),
            ),
            (
                "approved_tx_hash",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| {
                    i.approved_tx_hash = ApprovedTxHash::from_bytes([0xcd; 32])
                }),
            ),
            (
                "created_at_ms",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| i.created_at_ms = NOW + 1),
            ),
            (
                "expires_at_ms",
                "tenant-a",
                Box::new(|i: &mut UnsignedIntent| i.expires_at_ms = EXPIRES + 1),
            ),
        ];

        for (label, presented_tenant, mutate) in mutations {
            let mut tampered = base.clone();
            mutate(&mut tampered);
            assert_ne!(
                tampered.signing_preimage(),
                base.signing_preimage(),
                "{label} must be part of the signed pre-image"
            );
            let resealed = tampered.into_signed(signature);
            assert_eq!(
                resealed.verify(
                    &key.verifying_key().to_bytes(),
                    &TenantId::new(presented_tenant),
                    NOW
                ),
                Err(IntentError::BadSignature),
                "tampering with {label} must fail signature verification"
            );
        }
    }

    #[test]
    fn a_signature_from_another_key_is_rejected() {
        let attacker = SigningKey::from_bytes(&[9u8; 32]);
        let signed = sealed(unsigned(), &attacker);
        assert_eq!(
            signed.verify(
                &signing_key().verifying_key().to_bytes(),
                &TenantId::new("tenant-a"),
                NOW
            ),
            Err(IntentError::BadSignature)
        );
    }

    #[test]
    fn an_expired_intent_is_rejected_at_and_after_the_boundary() {
        let key = signing_key();
        let signed = sealed(unsigned(), &key);
        let vk = key.verifying_key().to_bytes();
        let tenant = TenantId::new("tenant-a");
        // One millisecond before expiry the intent is still live...
        assert_eq!(signed.verify(&vk, &tenant, EXPIRES - 1), Ok(()));
        // ...and expiry is inclusive: at the boundary it is already expired.
        assert_eq!(
            signed.verify(&vk, &tenant, EXPIRES),
            Err(IntentError::Expired)
        );
        assert_eq!(
            signed.verify(&vk, &tenant, EXPIRES + 10_000),
            Err(IntentError::Expired)
        );
    }

    #[test]
    fn an_intent_presented_under_another_tenant_is_rejected() {
        let key = signing_key();
        let signed = sealed(unsigned(), &key);
        assert_eq!(
            signed.verify(
                &key.verifying_key().to_bytes(),
                &TenantId::new("tenant-b"),
                NOW
            ),
            Err(IntentError::TenantMismatch)
        );
    }

    #[test]
    fn a_malformed_verifying_key_fails_closed() {
        let key = signing_key();
        let signed = sealed(unsigned(), &key);
        // All-zero is not a valid ed25519 point encoding.
        let bogus = [0u8; AGENT_PUBLIC_KEY_LEN];
        assert!(matches!(
            signed.verify(&bogus, &TenantId::new("tenant-a"), NOW),
            Err(IntentError::MalformedKey) | Err(IntentError::BadSignature)
        ));
    }

    #[test]
    fn timestamp_invariants_are_validated() {
        let mut born_expired = unsigned();
        born_expired.expires_at_ms = born_expired.created_at_ms;
        assert_eq!(born_expired.validate(), Err(IntentError::InvalidExpiry));

        let mut negative = unsigned();
        negative.created_at_ms = -1;
        assert_eq!(negative.validate(), Err(IntentError::InvalidTimestamp));

        assert_eq!(unsigned().validate(), Ok(()));
    }

    #[test]
    fn a_matching_transaction_binding_verifies() {
        let signed = sealed(bound_unsigned(), &signing_key());
        assert_eq!(signed.verify_binds_transaction(SIGNER), Ok(()));
    }

    /// The check that makes the carried `decoded_tx` trustworthy: swapping the
    /// transaction while keeping an authentic signature + hash must fail. This
    /// is the "render one tx, sign another" attack the review UI depends on
    /// being impossible.
    #[test]
    fn a_swapped_transaction_fails_the_binding_check() {
        let mut tampered = bound_unsigned();
        tampered.decoded_tx = DecodedTransaction::Evm(crate::decoded_tx::EvmTransaction {
            // Same shape, different recipient: the attacker's address.
            to: Some(crate::decoded_tx::EvmAddress([0x66; 20])),
            ..match tampered.decoded_tx.clone() {
                DecodedTransaction::Evm(tx) => tx,
                other => panic!("fixture is EVM, got {other:?}"),
            }
        });
        let signed = sealed(tampered, &signing_key());
        assert_eq!(
            signed.verify_binds_transaction(SIGNER),
            Err(IntentError::TransactionMismatch)
        );
    }

    /// The signer is folded into the hash, so an intent bound for one signer
    /// cannot be re-presented against another.
    #[test]
    fn a_different_signer_fails_the_binding_check() {
        let signed = sealed(bound_unsigned(), &signing_key());
        assert_eq!(
            signed.verify_binds_transaction("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Err(IntentError::TransactionMismatch)
        );
    }
}
