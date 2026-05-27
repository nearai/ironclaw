//! The browser injected-provider [`SigningProvider`] backend.
//!
//! `window.ethereum` (EVM) and `window.solana` (Solana) wallets render and sign
//! natively. This module accepts the proof they carry back and verifies it
//! fail-closed against the bound [`ApprovedTxHash`], the bound account, and the
//! one-shot grant.

mod evm;
mod solana;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_attestation::{GrantError, GrantKey, SealedGrantStore};
use ironclaw_signing_provider::{
    ApprovedTxHash, DecodedTransaction, InitiationOutcome, ProviderId, RenderedTx, SigningContext,
    SigningProof, SigningProvider, SigningProviderError, TrustModel, VerifiedProof,
};

/// Which injected wallet family produced a proof.
///
/// `#[serde(rename_all = "snake_case")]` pins the wire form: these tags ride in
/// the persisted gate proof, so they must not drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectedScheme {
    /// `window.ethereum`: secp256k1, signer recovered via ecrecover.
    Evm,
    /// `window.solana`: ed25519, signature verified against the connected key.
    Solana,
}

/// The structured payload an injected wallet carries back, serialized into the
/// opaque [`SigningProof::InjectedProof`] byte body.
///
/// The wallet attests to the *bound* [`ApprovedTxHash`] (the WYSIWYS digest
/// IronClaw rendered and the wallet UI mirrors) by signing over its raw 32
/// bytes. The payload echoes that hash and the claimed signer so the verifier
/// can re-check both bindings without trusting any caller-supplied chain bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectedProofPayload {
    /// Which wallet family produced the signature.
    pub scheme: InjectedScheme,
    /// The approved-tx hash the wallet attests to. MUST equal the bound hash;
    /// re-checked in [`InjectedSigningProvider::verify_resume`] (threat #3).
    pub approved_tx_hash: ApprovedTxHash,
    /// The account the wallet claims signed: a `0x`-prefixed lowercase EVM
    /// address (EVM) or a base-free lowercase-hex 32-byte ed25519 public key
    /// (Solana). Re-derived from the signature and compared to the bound
    /// account; the claim itself is never trusted (threat #5).
    pub claimed_signer: String,
    /// The raw signature bytes over the 32-byte approved hash. 65 bytes
    /// (r ∥ s ∥ v) for EVM, 64 bytes for Solana ed25519.
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
    /// For Solana, the 32-byte ed25519 public key bytes (lowercase hex) the
    /// signature verifies against. Unused for EVM (the address is recovered).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "opt_hex_bytes"
    )]
    pub public_key: Option<Vec<u8>>,
}

/// Serialize a [`InjectedProofPayload`] into opaque proof bytes for
/// [`SigningProof::InjectedProof`].
pub fn encode_injected_proof(payload: &InjectedProofPayload) -> Vec<u8> {
    // serde_json keeps the encoding self-describing and stable across the wire;
    // the opaque-bytes contract of `SigningProof::InjectedProof` is satisfied by
    // any deterministic round-trippable encoding.
    serde_json::to_vec(payload).unwrap_or_default()
}

/// Decode opaque [`SigningProof::InjectedProof`] bytes into a structured
/// [`InjectedProofPayload`].
pub fn decode_injected_proof(bytes: &[u8]) -> Result<InjectedProofPayload, SigningProviderError> {
    serde_json::from_slice(bytes).map_err(|e| SigningProviderError::ProofInvalid {
        reason: format!("malformed injected proof payload: {e}"),
    })
}

/// The browser injected-provider signing backend.
///
/// Holds a handle to the sealed-grant store so [`Self::verify_resume`] can claim
/// the one-shot grant atomically. Holds **no key material** — the wallet owns
/// the keys.
pub struct InjectedSigningProvider {
    grants: Arc<dyn SealedGrantStore>,
}

impl InjectedSigningProvider {
    /// Construct over a sealed-grant store.
    pub fn new(grants: Arc<dyn SealedGrantStore>) -> Self {
        Self { grants }
    }
}

#[async_trait]
impl SigningProvider for InjectedSigningProvider {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Injected
    }

    fn trust_model(&self) -> TrustModel {
        TrustModel::ExternalWallet
    }

    async fn initiate(
        &self,
        _context: &SigningContext,
        _decoded: &DecodedTransaction,
        _rendered: &RenderedTx,
        _approved_tx_hash: &ApprovedTxHash,
    ) -> Result<InitiationOutcome, SigningProviderError> {
        // An injected provider prompts in-page: the browser already holds the
        // unsigned transaction the gate rendered, and the wallet renders + signs
        // it natively. There is no server-issued directive to launch, so the
        // ceremony is ready for a proof immediately.
        Ok(InitiationOutcome::ReadyForProof)
    }

    async fn verify_resume(
        &self,
        context: &SigningContext,
        approved_tx_hash: &ApprovedTxHash,
        proof: &SigningProof,
    ) -> Result<VerifiedProof, SigningProviderError> {
        // Only injected proofs are accepted by this provider; anything else is a
        // routing error and fails closed.
        let SigningProof::InjectedProof(bytes) = proof else {
            return Err(SigningProviderError::ProofInvalid {
                reason: "injected provider received a non-injected proof".to_string(),
            });
        };
        let payload = decode_injected_proof(bytes)?;

        // 1. Hash binding (threat #3): the wallet must have attested to the
        //    exact bound hash. Reject a proof that carries a different one
        //    before doing any signature work.
        if &payload.approved_tx_hash != approved_tx_hash {
            return Err(SigningProviderError::ProofInvalid {
                reason: "proof approved-tx hash does not match the bound hash".to_string(),
            });
        }

        let bound_account = context.key_or_account_id.as_str();

        // 2. Signer binding (threat #5): recover / verify the signer over the
        //    bound hash and require it to equal the bound account.
        match payload.scheme {
            InjectedScheme::Evm => {
                evm::verify_signer_over_hash(
                    approved_tx_hash.as_bytes(),
                    &payload.signature,
                    bound_account,
                )?;
            }
            InjectedScheme::Solana => {
                let public_key =
                    payload
                        .public_key
                        .as_deref()
                        .ok_or(SigningProviderError::ProofInvalid {
                            reason: "solana injected proof missing public_key".to_string(),
                        })?;
                solana::verify_signer_over_hash(
                    approved_tx_hash.as_bytes(),
                    &payload.signature,
                    public_key,
                    bound_account,
                )?;
            }
        }

        // 3. One-shot grant (threat #1): claim the sealed grant atomically. A
        //    replay of an already-claimed grant fails closed here.
        let key = GrantKey::from_context(context, *approved_tx_hash);
        self.grants
            .claim(&key, ironclaw_attestation::now_unix_millis())
            .await
            .map_err(map_grant_error)?;

        Ok(VerifiedProof::new(
            ProviderId::Injected,
            *approved_tx_hash,
            proof.clone(),
        ))
    }
}

/// Map a [`GrantError`] onto the provider error taxonomy, fail-closed.
fn map_grant_error(err: GrantError) -> SigningProviderError {
    match err {
        // Replay / missing / lost-CAS / expired all collapse to a single
        // fail-closed grant-claim failure; the distinction is not safe to leak
        // to a caller.
        GrantError::AlreadyClaimed
        | GrantError::NotFound
        | GrantError::AlreadySealed
        | GrantError::Expired { .. } => SigningProviderError::GrantClaimFailed,
        GrantError::Backend { reason } => SigningProviderError::Provider { reason },
    }
}

/// Hex (de)serialization helper for `Vec<u8>` proof fields.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex_encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex_decode(&s).map_err(serde::de::Error::custom)
    }

    pub(super) fn hex_encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
            out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
        }
        out
    }

    pub(super) fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
        // Decode over raw bytes, never by `&str` byte-range indexing: a valid
        // JSON string can carry multi-byte UTF-8, and slicing `&s[i..i + 2]` on
        // a non-char-boundary panics. Working over `&[u8]` is panic-free and any
        // non-ASCII byte is rejected cleanly as a non-hex digit.
        let bytes = s.strip_prefix("0x").unwrap_or(s).as_bytes();
        if !bytes.len().is_multiple_of(2) {
            return Err("odd-length hex".to_string());
        }
        bytes
            .chunks_exact(2)
            .map(|pair| {
                let hi = hex_digit(pair[0])?;
                let lo = hex_digit(pair[1])?;
                Ok((hi << 4) | lo)
            })
            .collect()
    }

    /// Decode a single ASCII hex digit byte to its 0–15 value, rejecting any
    /// non-hex (including non-ASCII) byte without panicking.
    fn hex_digit(b: u8) -> Result<u8, String> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            other => Err(format!("invalid hex digit: {other:#04x}")),
        }
    }
}

/// Hex (de)serialization helper for `Option<Vec<u8>>` proof fields.
mod opt_hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        bytes: &Option<Vec<u8>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(b) => s.serialize_str(&super::hex_bytes::hex_encode(b)),
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let opt = Option::<String>::deserialize(d)?;
        match opt {
            Some(s) => super::hex_bytes::hex_decode(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}
