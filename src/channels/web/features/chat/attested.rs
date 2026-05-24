//! Attested-signing gate resolution for the injected-wallet (`window.ethereum`
//! / `window.solana`) proof path (attested-signing PR7).
//!
//! This is the web ingress for resolving a `BlockedAttested` signing gate with
//! an external-wallet proof. It deserializes the wire payload into a
//! [`SigningProof::InjectedProof`], runs
//! [`InjectedSigningProvider::verify_resume`] (signer recovery + hash binding +
//! one-shot grant claim), and — on success — reaches the verified-proof
//! boundary.
//!
//! ## PR7 scope boundary
//!
//! Two composition seams are deferred to PR10 and are NOT built here:
//!
//! 1. **Authoritative gate binding.** The persisted `BlockedAttested` gate's
//!    bound [`ApprovedTxHash`] + [`SigningContext`] (tenant/user/run/gate/chain/
//!    account) are owned by the reborn/composition layer's gate store, which is
//!    wired in PR10. Until that store carries the attested binding, this handler
//!    has nothing authoritative to verify the proof *against*, so it fails
//!    closed (`503`) rather than trusting caller-supplied context.
//! 2. **Broadcast + deterministic continuation.** Broadcasting the
//!    wallet-signed transaction (through `ironclaw_chain_signing`) and building
//!    the `ResumeTurnRequest { attestation: Some(..) }` that drives
//!    `resume_turn_once` through the existing `AttestedResumePort` boundary is
//!    PR10. This handler stops at the verified-proof boundary and marks the
//!    handoff with a `// PR10:` note.
//!
//! The reusable, fully-tested core is [`verify_injected_proof`]: it is
//! crypto-real today and is the function PR10 will call once the gate store
//! supplies the authoritative binding.

use std::sync::Arc;

use axum::http::StatusCode;
use uuid::Uuid;

use ironclaw_attestation::SealedGrantStore;
use ironclaw_signing_provider::{
    ApprovedTxHash, SigningContext, SigningProof, SigningProvider, SigningProviderError,
};
use ironclaw_wallet_external::{
    InjectedProofPayload, InjectedScheme, InjectedSigningProvider, encode_injected_proof,
};

use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::ActionResponse;

/// The browser-supplied injected-wallet proof fields, as carried on the
/// `/api/chat/gate/resolve` wire payload.
#[derive(Debug, Clone)]
pub(crate) struct InjectedWalletProofInput {
    /// Wallet family: `evm` or `solana`.
    pub scheme: String,
    /// Claimed signer (re-derived from the signature; never trusted).
    pub signer: String,
    /// Hex signature bytes over the bound hash.
    pub signature: String,
    /// Hex of the approved-tx hash the wallet attested to.
    pub approved_tx_hash: String,
    /// Solana only: hex of the 32-byte ed25519 public key.
    pub public_key: Option<String>,
}

/// Parse the wire input into the structured [`SigningProof::InjectedProof`].
///
/// Fails closed (`BAD_REQUEST`) on any malformed field. This is pure
/// deserialization — no trust is conferred; verification happens in
/// [`verify_injected_proof`].
fn proof_from_input(
    input: &InjectedWalletProofInput,
) -> Result<SigningProof, (StatusCode, String)> {
    let scheme = match input.scheme.as_str() {
        "evm" => InjectedScheme::Evm,
        "solana" => InjectedScheme::Solana,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unknown injected wallet scheme: {other}"),
            ));
        }
    };
    let approved_tx_hash = parse_hash(&input.approved_tx_hash)?;
    let signature = parse_hex(&input.signature).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid signature hex: {e}"),
        )
    })?;
    let public_key = match &input.public_key {
        Some(pk) => Some(parse_hex(pk).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("invalid public_key hex: {e}"),
            )
        })?),
        None => None,
    };

    let payload = InjectedProofPayload {
        scheme,
        approved_tx_hash,
        claimed_signer: input.signer.clone(),
        signature,
        public_key,
    };
    Ok(SigningProof::InjectedProof(encode_injected_proof(&payload)))
}

/// Verify an injected-wallet proof against the authoritative bound context and
/// hash, via [`InjectedSigningProvider::verify_resume`].
///
/// This is the reusable security core: signer recovery (EVM ecrecover / Solana
/// ed25519) + hash binding + one-shot grant claim, all fail-closed. PR10 calls
/// this once its gate store supplies the authoritative `context` +
/// `approved_tx_hash`.
// PR10: the production caller is the gate-store-backed resume path wired in
// PR10; in PR7 only the tests drive it (the handler fails closed before
// reaching it because the authoritative binding is not yet persisted).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn verify_injected_proof(
    grants: Arc<dyn SealedGrantStore>,
    context: &SigningContext,
    approved_tx_hash: &ApprovedTxHash,
    proof: &SigningProof,
) -> Result<(), SigningProviderError> {
    let provider = InjectedSigningProvider::new(grants);
    provider
        .verify_resume(context, approved_tx_hash, proof)
        .await
        .map(|_verified| ())
}

/// Resolve a `BlockedAttested` gate with an injected-wallet proof.
///
/// Deserializes the proof, then — at the verified-proof boundary — defers the
/// authoritative gate binding + broadcast handoff to PR10 (see module docs).
pub(crate) async fn resolve_injected_wallet_proof(
    state: &Arc<GatewayState>,
    _user_id: &str,
    _gate_request_id: Uuid,
    _thread_id: Option<String>,
    input: InjectedWalletProofInput,
) -> Result<axum::Json<ActionResponse>, (StatusCode, String)> {
    // Always validate the wire shape first so malformed proofs reject uniformly
    // regardless of whether the composition layer is wired.
    let _proof = proof_from_input(&input)?;

    let Some(_grants) = state.attested_grant_store.clone() else {
        // PR10: the reborn/composition layer wires the sealed-grant store and
        // persists the authoritative `BlockedAttested` binding (ApprovedTxHash +
        // SigningContext) into the gate store. Until then there is nothing
        // authoritative to verify the proof against, so fail closed rather than
        // trust caller-supplied context. `verify_injected_proof` above is the
        // crypto-real core PR10 will drive once that binding is available; the
        // broadcast of the wallet-signed tx (ironclaw_chain_signing) and the
        // `ResumeTurnRequest { attestation: Some(..) }` continuation through the
        // existing `AttestedResumePort` gate-resolve path also land in PR10.
        tracing::debug!(
            "[gate_resolve] injected-wallet proof received but attested signing composition is \
             not wired (PR10); failing closed"
        );
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Attested external-wallet signing is not enabled on this deployment.".to_string(),
        ));
    };

    // PR10: with `_grants` present, look up the persisted `BlockedAttested` gate
    // for `_gate_request_id`, recover its bound `SigningContext` +
    // `ApprovedTxHash`, call `verify_injected_proof(_grants, &ctx, &hash,
    // &_proof)`, and on success build `ResumeTurnRequest { attestation:
    // Some(..) }` + dispatch the broadcast through the existing gate-resolve
    // engine submission path. Not built in PR7.
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "Attested external-wallet signing resume is not yet wired (pending PR10).".to_string(),
    ))
}

/// Parse a 32-byte hex (optionally `0x`-prefixed) approved-tx hash.
fn parse_hash(s: &str) -> Result<ApprovedTxHash, (StatusCode, String)> {
    let bytes = parse_hex(s).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid approved_tx_hash hex: {e}"),
        )
    })?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "approved_tx_hash must be 32 bytes".to_string(),
        )
    })?;
    Ok(ApprovedTxHash::from_bytes(arr))
}

/// Decode a hex string (optionally `0x`-prefixed) to bytes.
fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_attestation::{AttestedSigningGrant, GrantKey, InMemorySealedGrantStore};
    use ironclaw_signing_provider::{
        ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
    };

    use ed25519_dalek::{Signer as _, SigningKey as EdSigningKey};

    fn lower_hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
            out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
        }
        out
    }

    fn ctx(account: &str) -> SigningContext {
        SigningContext {
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("user-1"),
            scope: ScopeId::new("scope-x"),
            actor: ActorId::new("actor-7"),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new("gate:abc"),
            chain_id: ChainId::new("solana:mainnet"),
            key_or_account_id: KeyOrAccountId::new(account),
        }
    }

    #[test]
    fn proof_from_input_rejects_unknown_scheme() {
        let err = proof_from_input(&InjectedWalletProofInput {
            scheme: "bitcoin".into(),
            signer: "x".into(),
            signature: "00".into(),
            approved_tx_hash: "00".repeat(32),
            public_key: None,
        })
        .expect_err("unknown scheme must reject");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn proof_from_input_rejects_bad_hash_length() {
        let err = proof_from_input(&InjectedWalletProofInput {
            scheme: "evm".into(),
            signer: "0x00".into(),
            signature: "00".repeat(65),
            approved_tx_hash: "00".repeat(16),
            public_key: None,
        })
        .expect_err("short hash must reject");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn verify_injected_proof_solana_success_then_replay_fails() {
        let store = Arc::new(InMemorySealedGrantStore::new());
        let key = EdSigningKey::from_bytes(&[0x22u8; 32]);
        let pubkey = key.verifying_key().to_bytes();
        let account = lower_hex(&pubkey);
        let context = ctx(&account);
        let hash = ApprovedTxHash::from_bytes([5u8; 32]);

        // Seal the grant (PR10's gate store would do this when raising the gate).
        let gk = GrantKey::from_context(&context, hash);
        store
            .seal(AttestedSigningGrant::seal(gk, 1_000, None))
            .await
            .expect("seal");

        let sig = key.sign(hash.as_bytes());
        let proof = proof_from_input(&InjectedWalletProofInput {
            scheme: "solana".into(),
            signer: account.clone(),
            signature: lower_hex(&sig.to_bytes()),
            approved_tx_hash: lower_hex(hash.as_bytes()),
            public_key: Some(lower_hex(&pubkey)),
        })
        .expect("proof parses");

        verify_injected_proof(store.clone(), &context, &hash, &proof)
            .await
            .expect("valid proof verifies through the web helper");

        // Replay must fail closed (one-shot grant).
        let err = verify_injected_proof(store, &context, &hash, &proof)
            .await
            .expect_err("replay fails closed");
        assert!(matches!(err, SigningProviderError::GrantClaimFailed));
    }

    #[tokio::test]
    async fn verify_injected_proof_rejects_wrong_signer() {
        let store = Arc::new(InMemorySealedGrantStore::new());
        let key = EdSigningKey::from_bytes(&[0x22u8; 32]);
        let pubkey = key.verifying_key().to_bytes();
        // Bind a different account than the proof's key.
        let bound = lower_hex(&[0x33u8; 32]);
        let context = ctx(&bound);
        let hash = ApprovedTxHash::from_bytes([5u8; 32]);
        let gk = GrantKey::from_context(&context, hash);
        store
            .seal(AttestedSigningGrant::seal(gk, 1_000, None))
            .await
            .expect("seal");

        let sig = key.sign(hash.as_bytes());
        let proof = proof_from_input(&InjectedWalletProofInput {
            scheme: "solana".into(),
            signer: bound.clone(),
            signature: lower_hex(&sig.to_bytes()),
            approved_tx_hash: lower_hex(hash.as_bytes()),
            public_key: Some(lower_hex(&pubkey)),
        })
        .expect("proof parses");

        let err = verify_injected_proof(store, &context, &hash, &proof)
            .await
            .expect_err("wrong signer must reject");
        assert!(matches!(err, SigningProviderError::SignerMismatch));
    }
}
