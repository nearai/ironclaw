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

use ironclaw_attested_runtime::{ContinuationError, VerifiedContinuation};
use ironclaw_product_workflow::{
    AttestedContinuationOutcome, AttestedContinuationRejection, AttestedGateContinuationPort,
    AttestedProofClaim, AttestedProofKind, VerifiedAttestedContinuation,
};
use ironclaw_signing_provider::{
    ApprovedTxHash, GateRef as SigningGateRef, SigningProof, SigningProviderError,
};
use ironclaw_turns::{GateRef, TurnRunId, TurnScope};
use ironclaw_wallet_external::{
    InjectedProofPayload, InjectedScheme, NearAccessKeyScope, NearRedirectProofPayload,
    WalletConnectProofPayload, encode_injected_proof, encode_near_redirect_proof,
    encode_walletconnect_proof,
};
use serde::Deserialize;

use crate::attested::{LocalDevAttestedComposition, LocalDevContinuationDriver};

/// Composition-layer [`AttestedGateContinuationPort`].
///
/// Holds the assembled signer-continuation driver shared with the reborn
/// runtime (the same driver + binding store + ledger the resume port reads).
///
/// Production-driver wiring (deferred): this port is constructed over the
/// concrete [`LocalDevContinuationDriver`] monomorphization because
/// [`crate::RebornRuntime`] itself holds a concrete
/// [`crate::attested::LocalDevAttestedComposition`] field — there is currently
/// no type through which `build_webui_services` could hand it the durable
/// (`Postgres*`/`LibSql*`) driver produced by
/// [`crate::attested_durable::assemble_postgres`] / `assemble_libsql`. The
/// runtime is also gated to local-dev (`build_reborn_runtime` rejects every
/// other profile), so this cannot silently take the in-memory path in a
/// production deployment: a production deployment cannot construct a
/// `RebornRuntime` at all today. Erasing `RebornRuntime.attested_signing`
/// behind a trait/enum so it can hold a durable monomorphization — and wiring
/// this port over it — is the dedicated follow-up slice.
pub struct RebornAttestedContinuation {
    driver: Arc<LocalDevContinuationDriver>,
}

impl RebornAttestedContinuation {
    /// Build the port over the runtime's attested-signing composition.
    pub fn new(composition: &LocalDevAttestedComposition) -> Self {
        Self {
            driver: Arc::clone(composition.driver()),
        }
    }
}

#[async_trait]
impl AttestedGateContinuationPort for RebornAttestedContinuation {
    async fn verify_and_claim(
        &self,
        // TODO(tenant-audit): `scope`/`run_id` are not propagated to the driver
        // yet. The driver derives all authority from the `gate_ref`-keyed
        // persisted binding, so they are not needed for verification today; they
        // are reserved for threading a per-tenant audit trail (scope/run id) into
        // the driver once the audit-sink wiring lands. Until then they are
        // intentionally unused.
        _scope: &TurnScope,
        _run_id: TurnRunId,
        gate_ref: &GateRef,
        claim: &AttestedProofClaim,
    ) -> Result<VerifiedAttestedContinuation, AttestedContinuationRejection> {
        // FULL verification + one-shot grant claim, run BEFORE the facade
        // transitions the turn. A malformed proof fails closed here at decode; a
        // forged signature / signer mismatch / already-claimed grant fails closed
        // inside the driver's `verify_and_sign` (provider `verify_resume` + the
        // sealed-grant CAS) — all before any `AttestedResolved` transition. The
        // driver reads the authoritative binding itself and re-checks the bound
        // hash against the proof, so the caller can only attest to the bound hash
        // (threat #3), never redefine it.
        let proof = decode_proof(claim)?;
        let signing_gate_ref = SigningGateRef::new(gate_ref.as_str());

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
        // `verify_and_claim`. A type mismatch is only reachable if a *different*
        // port implementation produced the handle — i.e. an internal composition
        // wiring bug, not a bad client proof. Surface it as a backend-health
        // failure (503), never `ProofRejected` (400), so a wiring regression
        // cannot masquerade as a client proof rejection. Still fails closed (no
        // broadcast) rather than panicking.
        let verified = *verified
            .downcast::<VerifiedContinuation>()
            .map_err(|_| AttestedContinuationRejection::BackendUnavailable)?;

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

/// Decode the opaque WebUI proof claim into the concrete provider proof for its
/// family. Mirrors the legacy monolith wire contract
/// (`src/channels/web/features/chat/attested.rs`): every byte field arrives as
/// lowercase-hex (optionally `0x`-prefixed) and the hash as hex, so we parse the
/// JSON via explicit input structs rather than the payload types directly (the
/// payload's `ApprovedTxHash` serde is a raw byte array, not the hex wire form).
/// A malformed payload fails closed as `MalformedProof`.
fn decode_proof(claim: &AttestedProofClaim) -> Result<SigningProof, AttestedContinuationRejection> {
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
            Ok(SigningProof::InjectedProof(encode_injected_proof(&payload)))
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
    // Empty input (`""` or a bare `"0x"`) is a malformed proof field, not a
    // valid zero-length byte string: a signature / public key / nonce always
    // carries bytes. Fail closed rather than decode to an empty `Vec`. The
    // `is_ascii` guard keeps the per-nibble decode over raw bytes safe for any
    // (even multibyte-Unicode) caller-supplied value.
    if s.is_empty() || !s.is_ascii() || !bytes.len().is_multiple_of(2) {
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
    // The established browser wire contract (see `src/channels/web/types.rs`
    // and the legacy `attested.rs` resolve path) names this field `signer`;
    // accept it as an alias so the durable v2 ingress decodes the same payload
    // the browser already produces, while keeping the unambiguous
    // `claimed_signer` as the canonical name.
    #[serde(alias = "signer")]
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
        ContinuationError::MissingBinding => AttestedContinuationRejection::MissingBinding,
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
        // Chain-signing backend errors and broadcast/RPC failures are
        // infrastructure/runtime failures, not client input failures. Surface
        // them as a service-health (503, retryable) signal rather than a 400
        // proof rejection, which would both mislead the client and suppress the
        // backend-health signal during an RPC outage / backend misconfiguration.
        ContinuationError::ChainSigning(_) | ContinuationError::Broadcast { .. } => {
            AttestedContinuationRejection::BackendUnavailable
        }
        // A startup/assembly misconfiguration should never reach this runtime
        // mapping (it is raised while building the durable composition, long
        // before any gate resolves). If one ever does, surface it as a backend
        // health failure rather than a client proof rejection.
        ContinuationError::Config { .. } => AttestedContinuationRejection::BackendUnavailable,
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
    }

    #[test]
    fn parse_hex_rejects_empty_proof_field() {
        // An empty proof field (`""` or a bare `"0x"`) is malformed: a
        // signature / public key / nonce always carries bytes. Fail closed
        // rather than decode to an empty `Vec`.
        assert_eq!(
            parse_hex(""),
            Err(AttestedContinuationRejection::MalformedProof)
        );
        assert_eq!(
            parse_hex("0x"),
            Err(AttestedContinuationRejection::MalformedProof)
        );
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
    fn broadcast_failure_maps_to_backend_unavailable_not_proof_rejected() {
        // A broadcast / RPC failure is a post-verification, server-side
        // (recoverable) infrastructure failure — it must NOT be surfaced as
        // ProofRejected (which implies the proof was bad and maps to 400). It
        // maps to BackendUnavailable (503, retryable) so clients can retry and
        // the backend-health signal is preserved during an RPC outage.
        let rejection = map_continuation_error(ContinuationError::Broadcast {
            reason: "rpc timeout".to_string(),
        });
        assert!(matches!(
            rejection,
            AttestedContinuationRejection::BackendUnavailable
        ));
    }

    #[test]
    fn custodial_signing_failure_maps_to_backend_unavailable() {
        // A custodial signer / chain-signing backend failure is an
        // infrastructure/runtime failure, not a client input failure: surface it
        // as a service-health (503, retryable) signal rather than a 400 proof
        // rejection, which would mislead the client and mask the backend-health
        // signal during a backend misconfiguration / outage.
        let rejection = map_continuation_error(ContinuationError::ChainSigning(
            ironclaw_chain_signing::ChainSigningError::SignerMismatch,
        ));
        assert!(matches!(
            rejection,
            AttestedContinuationRejection::BackendUnavailable
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

#[cfg(test)]
mod decoder_tests {
    use super::*;
    use ironclaw_attested_runtime::ContinuationError;
    use ironclaw_wallet_external::decode_injected_proof;

    fn injected_claim(proof_json: serde_json::Value) -> AttestedProofClaim {
        AttestedProofClaim {
            kind: AttestedProofKind::InjectedWallet,
            approved_tx_hash_hex: "ab".repeat(32),
            proof_json,
        }
    }

    #[test]
    fn injected_decoder_accepts_legacy_signer_alias() {
        // The established browser wire contract names the field `signer`; the
        // durable v2 ingress must decode the same payload the browser produces.
        let claim = injected_claim(serde_json::json!({
            "scheme": "evm",
            "signer": "0x00000000000000000000000000000000000000aa",
            "signature": "00".repeat(65),
            "approved_tx_hash": "ab".repeat(32),
        }));
        let proof = decode_proof(&claim).expect("legacy `signer` key must decode");
        let SigningProof::InjectedProof(bytes) = proof else {
            panic!("expected injected proof");
        };
        let payload = decode_injected_proof(&bytes).expect("payload");
        assert_eq!(
            payload.claimed_signer,
            "0x00000000000000000000000000000000000000aa"
        );
    }

    #[test]
    fn injected_decoder_accepts_canonical_claimed_signer() {
        let claim = injected_claim(serde_json::json!({
            "scheme": "evm",
            "claimed_signer": "0x00000000000000000000000000000000000000aa",
            "signature": "00".repeat(65),
            "approved_tx_hash": "ab".repeat(32),
        }));
        decode_proof(&claim).expect("canonical `claimed_signer` key must decode");
    }

    #[test]
    fn parse_hex_rejects_non_ascii_without_panicking() {
        // A multi-byte UTF-8 value must fail closed, not panic on a non-char
        // boundary slice.
        assert_eq!(
            parse_hex("00é0"),
            Err(AttestedContinuationRejection::MalformedProof)
        );
    }

    #[test]
    fn parse_hex_rejects_odd_length() {
        assert_eq!(
            parse_hex("abc"),
            Err(AttestedContinuationRejection::MalformedProof)
        );
    }

    #[test]
    fn backend_failures_map_to_backend_unavailable_not_proof_rejected() {
        assert_eq!(
            map_continuation_error(ContinuationError::Broadcast {
                reason: "rpc timeout".to_string(),
            }),
            AttestedContinuationRejection::BackendUnavailable
        );
    }

    #[test]
    fn parse_hex_rejects_empty_and_bare_prefix() {
        // Empty / `0x`-only is a malformed field, not a valid empty byte string.
        assert_eq!(
            parse_hex(""),
            Err(AttestedContinuationRejection::MalformedProof)
        );
        assert_eq!(
            parse_hex("0x"),
            Err(AttestedContinuationRejection::MalformedProof)
        );
    }

    #[test]
    fn parse_hex_accepts_prefixed_and_unprefixed() {
        assert_eq!(parse_hex("0xab").unwrap(), vec![0xab]);
        assert_eq!(parse_hex("ab").unwrap(), vec![0xab]);
    }

    #[test]
    fn map_continuation_error_covers_all_variants() {
        use ironclaw_attestation::LedgerError;
        use ironclaw_chain_signing::ChainSigningError;
        use ironclaw_signing_provider::{ProviderId, SigningProviderError};

        // MissingBinding -> MissingBinding
        assert_eq!(
            map_continuation_error(ContinuationError::MissingBinding),
            AttestedContinuationRejection::MissingBinding
        );
        // ProviderMismatch -> ProviderMismatch
        assert_eq!(
            map_continuation_error(ContinuationError::ProviderMismatch {
                bound: ProviderId::Injected,
            }),
            AttestedContinuationRejection::ProviderMismatch
        );
        // ProofRejected(GrantClaimFailed) -> LedgerGuard (idempotency replay)
        assert_eq!(
            map_continuation_error(ContinuationError::ProofRejected(
                SigningProviderError::GrantClaimFailed
            )),
            AttestedContinuationRejection::LedgerGuard
        );
        // ProofRejected(other) -> ProofRejected
        assert_eq!(
            map_continuation_error(ContinuationError::ProofRejected(
                SigningProviderError::SignerMismatch
            )),
            AttestedContinuationRejection::ProofRejected
        );
        // ApprovedHashMismatch -> ProofRejected
        assert_eq!(
            map_continuation_error(ContinuationError::ApprovedHashMismatch),
            AttestedContinuationRejection::ProofRejected
        );
        // Ledger -> LedgerGuard
        assert_eq!(
            map_continuation_error(ContinuationError::Ledger(LedgerError::NotFound)),
            AttestedContinuationRejection::LedgerGuard
        );
        // ChainSigning -> BackendUnavailable
        assert_eq!(
            map_continuation_error(ContinuationError::ChainSigning(
                ChainSigningError::ApprovedHashMismatch
            )),
            AttestedContinuationRejection::BackendUnavailable
        );
        // Broadcast -> BackendUnavailable
        assert_eq!(
            map_continuation_error(ContinuationError::Broadcast {
                reason: "rpc 503".to_string(),
            }),
            AttestedContinuationRejection::BackendUnavailable
        );
        // Config (startup misconfig) -> BackendUnavailable (should never reach
        // this runtime mapping in practice, but must stay exhaustive + safe).
        assert_eq!(
            map_continuation_error(ContinuationError::Config {
                reason: "evm RPC URL is not a valid URL".to_string(),
            }),
            AttestedContinuationRejection::BackendUnavailable
        );
    }

    #[test]
    fn near_redirect_proof_decode_covers_full_access_and_function_call() {
        use ironclaw_wallet_external::{NearAccessKeyScope, decode_near_redirect_proof};

        // FullAccess variant.
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::NearRedirect,
            approved_tx_hash_hex: "cd".repeat(32),
            proof_json: serde_json::json!({
                "account_id": "alice.near",
                "public_key": "11".repeat(32),
                "signature": "22".repeat(64),
                "approved_tx_hash": "cd".repeat(32),
                "access_key_scope": { "kind": "full_access" },
                "state": "state-token",
            }),
        };
        let SigningProof::NearRedirectProof(bytes) =
            decode_proof(&claim).expect("full_access near proof decodes")
        else {
            panic!("expected near redirect proof");
        };
        let payload = decode_near_redirect_proof(&bytes).expect("payload");
        assert_eq!(payload.account_id, "alice.near");
        assert!(matches!(
            payload.access_key_scope,
            NearAccessKeyScope::FullAccess
        ));

        // FunctionCall variant with method names.
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::NearRedirect,
            approved_tx_hash_hex: "cd".repeat(32),
            proof_json: serde_json::json!({
                "account_id": "alice.near",
                "public_key": "11".repeat(32),
                "signature": "22".repeat(64),
                "approved_tx_hash": "cd".repeat(32),
                "access_key_scope": {
                    "kind": "function_call",
                    "receiver_id": "contract.near",
                    "method_names": ["ft_transfer", "nft_transfer"],
                },
                "state": "state-token",
            }),
        };
        let SigningProof::NearRedirectProof(bytes) =
            decode_proof(&claim).expect("function_call near proof decodes")
        else {
            panic!("expected near redirect proof");
        };
        let payload = decode_near_redirect_proof(&bytes).expect("payload");
        match payload.access_key_scope {
            NearAccessKeyScope::FunctionCall {
                receiver_id,
                method_names,
            } => {
                assert_eq!(receiver_id, "contract.near");
                assert_eq!(method_names, vec!["ft_transfer", "nft_transfer"]);
            }
            NearAccessKeyScope::FullAccess => panic!("expected function_call scope"),
        }
    }

    #[test]
    fn walletconnect_proof_decode_covers_with_and_without_public_key() {
        use ironclaw_wallet_external::decode_walletconnect_proof;

        // Without optional public_key.
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::WalletConnect,
            approved_tx_hash_hex: "ef".repeat(32),
            proof_json: serde_json::json!({
                "session_topic": "topic-abc",
                "claimed_signer": "0x00000000000000000000000000000000000000bb",
                "nonce": "33".repeat(16),
                "signed_payload": "cafe",
                "signature": "44".repeat(65),
                "approved_tx_hash": "ef".repeat(32),
            }),
        };
        let SigningProof::WalletConnectProof(bytes) =
            decode_proof(&claim).expect("wc proof without public_key decodes")
        else {
            panic!("expected walletconnect proof");
        };
        let payload = decode_walletconnect_proof(&bytes).expect("payload");
        assert_eq!(payload.session_topic, "topic-abc");
        assert!(payload.public_key.is_none());

        // With optional public_key.
        let claim = AttestedProofClaim {
            kind: AttestedProofKind::WalletConnect,
            approved_tx_hash_hex: "ef".repeat(32),
            proof_json: serde_json::json!({
                "session_topic": "topic-abc",
                "claimed_signer": "0x00000000000000000000000000000000000000bb",
                "nonce": "33".repeat(16),
                "signed_payload": "cafe",
                "signature": "44".repeat(65),
                "approved_tx_hash": "ef".repeat(32),
                "public_key": "55".repeat(33),
            }),
        };
        let SigningProof::WalletConnectProof(bytes) =
            decode_proof(&claim).expect("wc proof with public_key decodes")
        else {
            panic!("expected walletconnect proof");
        };
        let payload = decode_walletconnect_proof(&bytes).expect("payload");
        assert!(payload.public_key.is_some());
    }

    #[test]
    fn decode_proof_rejects_empty_signature_field() {
        // Regression for the parse_hex empty guard reaching a real field.
        let claim = injected_claim(serde_json::json!({
            "scheme": "evm",
            "claimed_signer": "0x00000000000000000000000000000000000000aa",
            "signature": "",
            "approved_tx_hash": "ab".repeat(32),
        }));
        assert_eq!(
            decode_proof(&claim),
            Err(AttestedContinuationRejection::MalformedProof)
        );
    }
}
