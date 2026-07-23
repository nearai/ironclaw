//! Composition-layer implementation of the crypto-free
//! [`AttestedGateContinuationPort`] (attested-signing PR11).
//!
//! This is the bridge between the crypto-free WebUI facade
//! ([`ironclaw_product_workflow`]) and the attested-signing signer-continuation
//! driver assembled in [`crate::attested`] over [`ironclaw_attested_runtime`].
//!
//! Atomic verify-before-resume (PR11 item B): this port runs the heavyweight
//! cryptographic half in two phases, straddling the turn transition.
//!
//! 1. [`AttestedGateContinuationPort::verify_and_claim`] (BEFORE the turn
//!    transitions): decode the opaque [`AttestedProofClaim`] into the concrete
//!    [`ironclaw_signing_provider::SigningProof`] for its proof family (mirrors
//!    the legacy monolith decode in
//!    `src/channels/web/features/chat/attested.rs`), then call
//!    [`AttestedSignerContinuationDriver::verify_and_sign`], which reads the
//!    authoritative binding, claims the sealed one-shot grant, and verifies the
//!    proof through the bound provider. On any failure the turn is left
//!    `BlockedAttested` (the facade never resumes). On success it returns an
//!    opaque verified handle.
//! 2. [`AttestedGateContinuationPort::broadcast_resolved`] (AFTER `resume_turn`
//!    drove the turn to `AttestedResolved`): consume the verified handle and
//!    call [`AttestedSignerContinuationDriver::broadcast_signed_continuation`]
//!    to perform the ledger-guarded broadcast. No re-verification, no re-claim.
//!
//! All verification (signer/hash binding, sealed-grant CAS, ledger idempotency)
//! lives in `ironclaw_attested_runtime` / the providers — this module is decode
//! + dispatch only.

use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_attested_runtime::{BindingOwner, ContinuationError, VerifiedContinuation};
use ironclaw_product_workflow::{
    AttestedContinuationOutcome, AttestedContinuationRejection, AttestedGateContinuationPort,
    AttestedProofClaim, AttestedProofKind, VerifiedAttestedContinuation,
};
use ironclaw_signing_provider::{
    ApprovedTxHash, GateRef as SigningGateRef, SigningProof, SigningProviderError,
};
use ironclaw_turns::{GateRef, TurnActor, TurnRunId, TurnScope};
use ironclaw_wallet_external::{
    InjectedProofPayload, InjectedScheme, NearAccessKeyScope, NearRedirectProofPayload,
    WalletConnectProofPayload, encode_injected_proof, encode_near_redirect_proof,
    encode_walletconnect_proof,
};
use serde::Deserialize;

use crate::attested::{LocalDevContinuationDriver, RebornAttestedComposition};

/// Composition-layer [`AttestedGateContinuationPort`].
///
/// Holds the assembled signer-continuation driver shared with the reborn
/// runtime (the same driver + binding store + ledger the resume port reads).
pub struct RebornAttestedContinuation {
    driver: Arc<LocalDevContinuationDriver>,
}

impl RebornAttestedContinuation {
    /// Build the port over the runtime's attested-signing composition.
    pub fn new(composition: &RebornAttestedComposition) -> Self {
        Self {
            driver: Arc::clone(composition.driver()),
        }
    }
}

#[async_trait]
impl AttestedGateContinuationPort for RebornAttestedContinuation {
    async fn verify_and_claim(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        _run_id: TurnRunId,
        gate_ref: &GateRef,
        claim: &AttestedProofClaim,
    ) -> Result<VerifiedAttestedContinuation, AttestedContinuationRejection> {
        let signing_gate_ref = SigningGateRef::new(gate_ref.as_str());

        // IDOR DEFENSE (threat #2): assert the calling identity owns the
        // authoritative binding BEFORE any decode / provider verify / custodial
        // sign / grant claim. The driver reconstructs and signs the custodial
        // path from the authoritative binding regardless of who presents the
        // `gate_ref`, so without this check a second tenant member who learns
        // another user's `gate_ref` could drive that user's signing
        // continuation. The thread-ownership probe upstream only proves the
        // caller owns *their own* thread, not the gate. Fail closed
        // indistinguishably from a missing binding (no existence oracle).
        self.driver
            .assert_binding_owner(
                &signing_gate_ref,
                BindingOwner {
                    tenant_id: scope.tenant_id.as_str(),
                    user_id: actor.user_id.as_str(),
                },
            )
            .await
            .map_err(map_continuation_error)?;

        // FULL verification + one-shot grant claim, run BEFORE the facade
        // transitions the turn. A malformed proof fails closed here at decode; a
        // forged signature / signer mismatch / already-claimed grant fails closed
        // inside the driver's `verify_and_sign` (provider `verify_resume` + the
        // sealed-grant CAS) — all before any `AttestedResolved` transition. The
        // driver reads the authoritative binding itself and re-checks the bound
        // hash against the proof, so the caller can only attest to the bound hash
        // (threat #3), never redefine it.
        let proof = decode_proof(claim)?;

        // External-wallet path only: the wallet already signed, so no custodial
        // EVM transaction is supplied. The custodial path is selected purely by
        // the authoritative binding's `provider_id` (never by the caller).
        let verified = self
            .driver
            .verify_and_sign(&signing_gate_ref, &proof)
            .await
            .map_err(map_continuation_error)?;

        Ok(VerifiedAttestedContinuation::new(verified))
    }

    async fn broadcast_resolved(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
        _gate_ref: &GateRef,
        verified: VerifiedAttestedContinuation,
    ) -> Result<AttestedContinuationOutcome, AttestedContinuationRejection> {
        // Recover the concrete verified continuation produced by
        // `verify_and_claim`. A type mismatch (only possible if a different port
        // implementation produced the handle) fails closed rather than panicking.
        let verified = *verified
            .downcast::<VerifiedContinuation>()
            .map_err(|_| AttestedContinuationRejection::ProofRejected)?;

        // Broadcast only — the proof is already verified and the grant already
        // claimed. No re-verification, no re-claim.
        let outcome = self
            .driver
            .broadcast_signed_continuation(verified)
            .await
            .map_err(map_continuation_error)?;

        Ok(AttestedContinuationOutcome {
            signer: outcome.signer,
        })
    }
}

/// Upper bound on the serialized size of a single attested-proof blob. The
/// `proof_json` arrives as an opaque `serde_json::Value` from the browser and
/// is NOT subject to the `USER_MESSAGE_TEXT_MAX_BYTES` message limit, so an
/// explicit ceiling keeps a syntactically-valid but pathologically large proof
/// (and the `parse_input` clone it forces) bounded. Every real proof family
/// (injected / NEAR-redirect / WalletConnect) is a small fixed struct of
/// hex/string fields; 16 KiB is generous headroom.
const ATTESTED_PROOF_MAX_BYTES: usize = 16 * 1024;

/// Upper bound on a WalletConnect `session_topic`. WalletConnect topic ids are
/// 32-byte hex (64 chars); 256 is generous headroom while bounding an
/// untrusted, persisted browser-supplied string (finding #5).
const WALLETCONNECT_SESSION_TOPIC_MAX_LEN: usize = 256;

/// Decode the opaque WebUI proof claim into the concrete provider proof for its
/// family. Mirrors the legacy monolith wire contract
/// (`src/channels/web/features/chat/attested.rs`): every byte field arrives as
/// lowercase-hex (optionally `0x`-prefixed) and the hash as hex, so we parse the
/// JSON via explicit input structs rather than the payload types directly (the
/// payload's `ApprovedTxHash` serde is a raw byte array, not the hex wire form).
/// A malformed payload fails closed as `MalformedProof`.
fn decode_proof(claim: &AttestedProofClaim) -> Result<SigningProof, AttestedContinuationRejection> {
    // Bound the untrusted proof blob before any clone/parse work.
    let serialized_len = serde_json::to_vec(&claim.proof_json)
        .map(|v| v.len())
        .map_err(|_| AttestedContinuationRejection::MalformedProof)?;
    if serialized_len > ATTESTED_PROOF_MAX_BYTES {
        return Err(AttestedContinuationRejection::MalformedProof);
    }
    match claim.kind {
        AttestedProofKind::InjectedWallet => {
            let input: InjectedWalletProofInput = parse_input(&claim.proof_json)?;
            let scheme = match input.scheme.as_str() {
                "evm" => InjectedScheme::Evm,
                "solana" => InjectedScheme::Solana,
                _ => return Err(AttestedContinuationRejection::MalformedProof),
            };
            let payload = InjectedProofPayload {
                scheme,
                approved_tx_hash: parse_hash(&input.approved_tx_hash)?,
                claimed_signer: input.claimed_signer,
                signature: parse_hex(&input.signature)?,
                public_key: input.public_key.as_deref().map(parse_hex).transpose()?,
            };
            Ok(SigningProof::InjectedProof(
                encode_injected_proof(&payload)
                    .map_err(|_| AttestedContinuationRejection::MalformedProof)?,
            ))
        }
        AttestedProofKind::NearRedirect => {
            let input: NearRedirectProofInput = parse_input(&claim.proof_json)?;
            let access_key_scope = match input.access_key_scope {
                NearAccessKeyScopeInput::FullAccess => NearAccessKeyScope::FullAccess,
                NearAccessKeyScopeInput::FunctionCall {
                    receiver_id,
                    method_names,
                } => NearAccessKeyScope::FunctionCall {
                    receiver_id,
                    method_names,
                },
            };
            let payload = NearRedirectProofPayload {
                approved_tx_hash: parse_hash(&input.approved_tx_hash)?,
                account_id: input.account_id,
                public_key: parse_hex(&input.public_key)?,
                signature: parse_hex(&input.signature)?,
                access_key_scope,
                state: input.state,
            };
            Ok(SigningProof::NearRedirectProof(
                encode_near_redirect_proof(&payload)
                    .map_err(|_| AttestedContinuationRejection::MalformedProof)?,
            ))
        }
        AttestedProofKind::WalletConnect => {
            let input: WalletConnectProofInput = parse_input(&claim.proof_json)?;
            if input.session_topic.len() > WALLETCONNECT_SESSION_TOPIC_MAX_LEN {
                return Err(AttestedContinuationRejection::MalformedProof);
            }
            let payload = WalletConnectProofPayload {
                session_topic: input.session_topic,
                approved_tx_hash: parse_hash(&input.approved_tx_hash)?,
                claimed_signer: input.claimed_signer,
                nonce: parse_hex(&input.nonce)?,
                signed_payload: parse_hex(&input.signed_payload)?,
                signature: parse_hex(&input.signature)?,
                public_key: input.public_key.as_deref().map(parse_hex).transpose()?,
            };
            Ok(SigningProof::WalletConnectProof(
                encode_walletconnect_proof(&payload)
                    .map_err(|_| AttestedContinuationRejection::MalformedProof)?,
            ))
        }
    }
}

fn parse_input<T: for<'de> Deserialize<'de>>(
    value: &serde_json::Value,
) -> Result<T, AttestedContinuationRejection> {
    serde_json::from_value(value.clone()).map_err(|_| AttestedContinuationRejection::MalformedProof)
}

/// Parse a 32-byte hex (optionally `0x`-prefixed) approved-tx hash.
fn parse_hash(s: &str) -> Result<ApprovedTxHash, AttestedContinuationRejection> {
    let bytes = parse_hex(s)?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| AttestedContinuationRejection::MalformedProof)?;
    Ok(ApprovedTxHash::from_bytes(arr))
}

/// Decode a hex string (optionally `0x`-prefixed) to bytes.
///
/// Operates over raw bytes after validating the input is pure ASCII-hex, so a
/// multibyte-Unicode JSON value can never trigger a non-char-boundary slice
/// panic — a malformed (non-ASCII-hex or odd-length) input fails closed as
/// [`AttestedContinuationRejection::MalformedProof`].
fn parse_hex(s: &str) -> Result<Vec<u8>, AttestedContinuationRejection> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(AttestedContinuationRejection::MalformedProof);
    }
    bytes
        .chunks_exact(2)
        .map(|pair| {
            let hi = hex_nibble(pair[0])?;
            let lo = hex_nibble(pair[1])?;
            Ok((hi << 4) | lo)
        })
        .collect()
}

/// Decode a single ASCII-hex digit to its nibble value, fail-closed.
fn hex_nibble(byte: u8) -> Result<u8, AttestedContinuationRejection> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(AttestedContinuationRejection::MalformedProof),
    }
}

/// Wire input for an injected-wallet proof (lowercase-hex fields). Mirrors the
/// legacy `InjectedWalletProofInput`.
#[derive(Debug, Deserialize)]
struct InjectedWalletProofInput {
    scheme: String,
    claimed_signer: String,
    signature: String,
    approved_tx_hash: String,
    #[serde(default)]
    public_key: Option<String>,
}

/// Wire input for a NEAR redirect proof. Mirrors the legacy
/// `NearRedirectProofInput`.
#[derive(Debug, Deserialize)]
struct NearRedirectProofInput {
    account_id: String,
    public_key: String,
    signature: String,
    approved_tx_hash: String,
    access_key_scope: NearAccessKeyScopeInput,
    state: String,
}

/// Wire form of the NEAR access-key scope. Mirrors the legacy
/// `NearAccessKeyScopeInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum NearAccessKeyScopeInput {
    FullAccess,
    FunctionCall {
        receiver_id: String,
        #[serde(default)]
        method_names: Vec<String>,
    },
}

/// Wire input for a WalletConnect v2 proof.
#[derive(Debug, Deserialize)]
struct WalletConnectProofInput {
    session_topic: String,
    claimed_signer: String,
    nonce: String,
    signature: String,
    approved_tx_hash: String,
    /// The exact bytes the wallet's chain signature covers (the EVM sighash /
    /// Solana message), as lowercase hex. Bound to the recorded expectation by
    /// the provider before any signature work (WYSIWYS, #1).
    signed_payload: String,
    #[serde(default)]
    public_key: Option<String>,
}

/// Map the driver's [`ContinuationError`] to the sanitized facade rejection.
/// Categories only — no chain, signer, or ledger internals cross this boundary.
fn map_continuation_error(error: ContinuationError) -> AttestedContinuationRejection {
    match error {
        // A cross-user/cross-tenant gate_ref is surfaced identically to a
        // non-existent binding (404) so it is not an existence oracle (IDOR
        // defense, threat #2).
        ContinuationError::MissingBinding | ContinuationError::OwnerMismatch => {
            AttestedContinuationRejection::MissingBinding
        }
        ContinuationError::ProviderMismatch { .. } => {
            AttestedContinuationRejection::ProviderMismatch
        }
        ContinuationError::ProofRejected(SigningProviderError::GrantClaimFailed) => {
            // A replayed proof for an already-claimed grant is an idempotency
            // guard outcome, surfaced as a conflict to the client.
            AttestedContinuationRejection::LedgerGuard
        }
        // A tampered/inconsistent authoritative binding (sign-time hash re-check
        // mismatch, the binding's chain not matching its own decoded tx, or a
        // decoded tx that cannot be rebuilt into a signable) all fail closed
        // BEFORE any signing. None are retryable as-is; surface them as a proof
        // rejection rather than a recoverable infra failure.
        ContinuationError::ProofRejected(_)
        | ContinuationError::ApprovedHashMismatch
        | ContinuationError::BindingChainMismatch
        | ContinuationError::Rebuild(_) => AttestedContinuationRejection::ProofRejected,
        ContinuationError::Ledger(_) | ContinuationError::LedgerRowExists { .. } => {
            AttestedContinuationRejection::LedgerGuard
        }
        ContinuationError::ChainSigning(_) => AttestedContinuationRejection::ProofRejected,
        // A broadcast failure is a POST-verification, server-side (recoverable)
        // infrastructure failure: the proof was already verified and the grant
        // claimed. Surfacing it as ProofRejected (400) would wrongly imply the
        // client's proof was bad; map it to Unavailable (503) so the client can
        // retry the broadcast tail instead.
        ContinuationError::Broadcast { .. } => AttestedContinuationRejection::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_rejects_multibyte_unicode_without_panicking() {
        // A multibyte-Unicode string whose byte length is even: the old
        // byte-offset `&s[i..i+2]` slice would panic on a non-char-boundary.
        // It must fail closed as MalformedProof instead.
        let result = parse_hex("déadbeef");
        assert!(matches!(
            result,
            Err(AttestedContinuationRejection::MalformedProof)
        ));

        // Other Unicode shapes (odd byte length, emoji) must also fail closed.
        assert!(matches!(
            parse_hex("é"),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
        assert!(matches!(
            parse_hex("🦀🦀"),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
    }

    #[test]
    fn parse_hex_accepts_valid_hex_with_optional_prefix() {
        assert_eq!(parse_hex("00ff").unwrap(), vec![0x00, 0xff]);
        assert_eq!(parse_hex("0xDEAD").unwrap(), vec![0xde, 0xad]);
        assert_eq!(parse_hex("").unwrap(), Vec::<u8>::new());
    }

    fn hash_hex_64() -> String {
        "11".repeat(32)
    }

    #[test]
    fn decode_injected_wallet_proof() {
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::InjectedWallet,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({
                "scheme": "evm",
                "claimed_signer": "0xabc",
                "signature": "deadbeef",
                "approved_tx_hash": hash_hex_64(),
            }),
        };
        assert!(matches!(
            decode_proof(&claim),
            Ok(SigningProof::InjectedProof(_))
        ));
    }

    #[test]
    fn decode_near_redirect_proof() {
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::NearRedirect,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({
                "account_id": "alice.near",
                "public_key": "aa",
                "signature": "bbcc",
                "approved_tx_hash": hash_hex_64(),
                "access_key_scope": { "kind": "full_access" },
                "state": "opaque-state",
            }),
        };
        assert!(matches!(
            decode_proof(&claim),
            Ok(SigningProof::NearRedirectProof(_))
        ));

        // FunctionCall scope variant also decodes.
        let claim_fc = AttestedProofClaim {
            kind: AttestedProofKind::NearRedirect,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({
                "account_id": "alice.near",
                "public_key": "aa",
                "signature": "bbcc",
                "approved_tx_hash": hash_hex_64(),
                "access_key_scope": {
                    "kind": "function_call",
                    "receiver_id": "contract.near",
                    "method_names": ["do_thing"],
                },
                "state": "opaque-state",
            }),
        };
        assert!(matches!(
            decode_proof(&claim_fc),
            Ok(SigningProof::NearRedirectProof(_))
        ));
    }

    #[test]
    fn decode_walletconnect_proof() {
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::WalletConnect,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({
                "session_topic": "topic-123",
                "claimed_signer": "0xabc",
                "nonce": "0011",
                "signed_payload": "cafe",
                "signature": "deadbeef",
                "approved_tx_hash": hash_hex_64(),
            }),
        };
        assert!(matches!(
            decode_proof(&claim),
            Ok(SigningProof::WalletConnectProof(_))
        ));
    }

    #[test]
    fn decode_proof_rejects_malformed_payload() {
        // Missing required fields for the family fails closed.
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::WalletConnect,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({ "session_topic": "only-this" }),
        };
        assert!(matches!(
            decode_proof(&claim),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
    }

    #[test]
    fn broadcast_failure_maps_to_unavailable_not_proof_rejected() {
        // A broadcast / RPC failure is a post-verification, server-side
        // (recoverable) infrastructure failure — it must NOT be surfaced as
        // ProofRejected (which implies the proof was bad and maps to 400). It
        // maps to Unavailable (503) so clients can retry.
        let rejection = map_continuation_error(ContinuationError::Broadcast {
            reason: "rpc timeout".to_string(),
        });
        assert!(matches!(
            rejection,
            AttestedContinuationRejection::Unavailable
        ));
    }

    #[test]
    fn custodial_signing_failure_still_maps_to_proof_rejected() {
        // A custodial signer failure happens during verification/signing (before
        // broadcast) and remains a client-facing rejection.
        let rejection = map_continuation_error(ContinuationError::ChainSigning(
            ironclaw_chain_signing::ChainSigningError::SignerMismatch,
        ));
        assert!(matches!(
            rejection,
            AttestedContinuationRejection::ProofRejected
        ));
    }

    #[test]
    fn decode_proof_rejects_oversized_blob() {
        // A syntactically valid but pathologically large proof blob must be
        // rejected before any clone/parse work (finding #3).
        let big = "a".repeat(ATTESTED_PROOF_MAX_BYTES + 1);
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::InjectedWallet,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({
                "scheme": "evm",
                "claimed_signer": "0xabc",
                "signature": "deadbeef",
                "approved_tx_hash": hash_hex_64(),
                "public_key": big,
            }),
        };
        assert!(matches!(
            decode_proof(&claim),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
    }

    #[test]
    fn decode_walletconnect_rejects_oversized_session_topic() {
        // An over-long session_topic must fail closed (finding #5) while a
        // bounded one still decodes.
        let long_topic = "t".repeat(WALLETCONNECT_SESSION_TOPIC_MAX_LEN + 1);
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::WalletConnect,
            approved_tx_hash_hex: hash_hex_64(),
            proof_json: serde_json::json!({
                "session_topic": long_topic,
                "claimed_signer": "0xabc",
                "nonce": "0011",
                "signed_payload": "cafe",
                "signature": "deadbeef",
                "approved_tx_hash": hash_hex_64(),
            }),
        };
        assert!(matches!(
            decode_proof(&claim),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
    }

    #[test]
    fn parse_hash_rejects_unicode_and_wrong_length() {
        assert!(matches!(
            parse_hash("déadbeef"),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
        // Valid hex but not 32 bytes.
        assert!(matches!(
            parse_hash("00ff"),
            Err(AttestedContinuationRejection::MalformedProof)
        ));
    }
}
