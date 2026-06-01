//! NEP-413 signature verification + NEAR access-key validation.
//!
//! Self-contained crypto for the WebChat v2 NEAR wallet login flow.
//! Deliberately does NOT import the v1 gateway's
//! `src/channels/web/oauth/near.rs`: this crate carries no `src/`-tier
//! dependency by contract (see crate CLAUDE.md), so the NEP-413 math
//! is re-implemented here rather than shared. The two copies are
//! covered by their own test suites; the field orderings and tag
//! constant below match the NEP-413 spec and the near-connect/HOT
//! variant exactly so a signature produced for either gateway verifies
//! identically.

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::auth::provider_http::read_capped_body;

/// Maximum accepted lengths for the verify request fields. Bounds the
/// work done decoding hostile input before any crypto runs. NEAR
/// account ids are <=64 chars, `ed25519:`-prefixed base58 keys are
/// ~52 chars, and 64-byte signatures are ~88 chars base64 / ~90 base58.
const MAX_ACCOUNT_ID_LEN: usize = 64;
const MAX_PUBLIC_KEY_LEN: usize = 128;
const MAX_SIGNATURE_LEN: usize = 256;

/// Recipient bound into the NEP-413 payload. The wallet signs over
/// this value, so it is part of the signature contract: changing it
/// invalidates every signature. Matches the v1 gateway's `"ironclaw"`.
pub(super) const NEAR_RECIPIENT: &str = "ironclaw";

/// NEP-413 prefix tag: `2^31 + 413 = 2147484061`. Distinguishes a
/// signed message from a signed transaction so a login signature can
/// never be replayed as an on-chain action.
const NEP413_TAG: u32 = (1 << 31) + 413;

/// Errors raised while verifying a NEAR wallet login. The route
/// handler maps each variant to an HTTP status; the `Display` text is
/// operator-facing (logged) and never echoed verbatim to the client.
#[derive(Debug, Error)]
pub(crate) enum NearVerifyError {
    /// Nonce unknown, expired, replayed, or not valid hex/32 bytes.
    /// Maps to `400`.
    #[error("invalid or expired nonce")]
    InvalidNonce,
    /// A request field was oversized or could not be decoded
    /// (bad `ed25519:` prefix, non-base58 key, signature not 64 bytes).
    /// Maps to `400`.
    #[error("invalid request: {0}")]
    InvalidInput(String),
    /// The Ed25519 signature did not verify against any accepted
    /// NEP-413 payload format. Maps to `401`.
    #[error("signature verification failed")]
    InvalidSignature,
    /// The public key is not an active access key on the claimed
    /// account (also covers wrong-network: a key valid on testnet
    /// fails against a mainnet RPC). Maps to `401`.
    #[error("access key not valid for account: {0}")]
    AccessKeyInvalid(String),
    /// The RPC call itself failed (unreachable, non-success status,
    /// unparseable body). An infrastructure fault, distinct from an
    /// auth miss. Maps to `503`.
    #[error("NEAR RPC backend error: {0}")]
    RpcBackend(String),
}

/// Build the canonical NEAR login message for a hex-encoded nonce.
/// The wallet signs over this exact string; the server reconstructs it
/// from the returned nonce. Matches the v1 gateway verbatim.
pub(super) fn login_message(nonce_hex: &str) -> String {
    format!("Sign in to IronClaw\nNonce: {nonce_hex}")
}

/// NEP-413 v1 (spec) field order: tag → message → nonce → recipient →
/// callback_url(None).
fn build_nep413_v1(message: &str, nonce: &[u8; 32], recipient: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&NEP413_TAG.to_le_bytes());
    buf.extend_from_slice(&(message.len() as u32).to_le_bytes());
    buf.extend_from_slice(message.as_bytes());
    buf.extend_from_slice(nonce);
    buf.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
    buf.extend_from_slice(recipient.as_bytes());
    buf.push(0); // None for callback_url
    buf
}

/// NEP-413 v2 (near-connect / HOT) field order: tag → message →
/// recipient → nonce. Documented at
/// docs.near.org/web3-apps/backend-login.
fn build_nep413_v2(message: &str, nonce: &[u8; 32], recipient: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&NEP413_TAG.to_le_bytes());
    buf.extend_from_slice(&(message.len() as u32).to_le_bytes());
    buf.extend_from_slice(message.as_bytes());
    buf.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
    buf.extend_from_slice(recipient.as_bytes());
    buf.extend_from_slice(nonce);
    buf
}

/// Verify an Ed25519 signature over `message`.
fn verify_ed25519(public_key: &[u8; 32], signature: &[u8; 64], message: &[u8]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let sig = Signature::from_bytes(signature);
    key.verify(message, &sig).is_ok()
}

/// Verify an Ed25519 signature over a NEP-413 structured payload.
///
/// Only structured NEP-413 payloads that include the nonce are
/// accepted — raw message bytes are intentionally rejected because
/// they lack nonce binding and would allow signature replay. Tries
/// both known field orderings (v1 spec, v2 near-connect/HOT) and, for
/// each, both the borsh payload and its SHA-256 (some wallets sign the
/// hash rather than the raw bytes).
pub(super) fn verify_near_signature(
    public_key: &[u8; 32],
    signature: &[u8; 64],
    message: &str,
    nonce: &[u8; 32],
    recipient: &str,
) -> Result<(), NearVerifyError> {
    let payloads = [
        build_nep413_v1(message, nonce, recipient),
        build_nep413_v2(message, nonce, recipient),
    ];
    for payload in &payloads {
        if verify_ed25519(public_key, signature, payload) {
            return Ok(());
        }
        if verify_ed25519(public_key, signature, &Sha256::digest(payload)) {
            return Ok(());
        }
    }
    Err(NearVerifyError::InvalidSignature)
}

/// Decode the hex challenge nonce returned by the client back to the
/// raw 32 bytes the NEP-413 payload binds. A malformed nonce fails as
/// [`NearVerifyError::InvalidNonce`] — it can never have been a value
/// the challenge endpoint issued.
pub(super) fn decode_nonce_bytes(nonce_hex: &str) -> Result<[u8; 32], NearVerifyError> {
    let bytes = hex::decode(nonce_hex).map_err(|_| NearVerifyError::InvalidNonce)?;
    if bytes.len() != 32 {
        return Err(NearVerifyError::InvalidNonce);
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Decode a NEAR public key. NEAR keys always carry the `ed25519:`
/// prefix with a base58 body; enforcing the prefix avoids ambiguity
/// with the base64 signature encoding.
pub(super) fn decode_public_key(key: &str) -> Result<[u8; 32], NearVerifyError> {
    if key.len() > MAX_PUBLIC_KEY_LEN {
        return Err(NearVerifyError::InvalidInput("public key too long".into()));
    }
    let raw = key.strip_prefix("ed25519:").ok_or_else(|| {
        NearVerifyError::InvalidInput("public key missing ed25519: prefix".into())
    })?;
    let bytes = bs58::decode(raw)
        .into_vec()
        .map_err(|_| NearVerifyError::InvalidInput("public key is not valid base58".into()))?;
    if bytes.len() != 32 {
        return Err(NearVerifyError::InvalidInput(format!(
            "public key decoded to {} bytes, expected 32",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Decode a NEAR signature. Wallets return signatures base64-encoded
/// (HOT, Meteor, near-connect) or base58 (MyNearWallet); try standard
/// base64, URL-safe base64, then base58. Each must decode to exactly
/// 64 bytes.
pub(super) fn decode_signature(sig: &str) -> Result<[u8; 64], NearVerifyError> {
    if sig.len() > MAX_SIGNATURE_LEN {
        return Err(NearVerifyError::InvalidInput("signature too long".into()));
    }
    let candidates = [
        base64::engine::general_purpose::STANDARD.decode(sig).ok(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig)
            .ok(),
        bs58::decode(sig).into_vec().ok(),
    ];
    for bytes in candidates.into_iter().flatten() {
        if bytes.len() == 64 {
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            return Ok(arr);
        }
    }
    Err(NearVerifyError::InvalidInput(
        "signature is not 64 bytes of base64/base58".into(),
    ))
}

/// Reject an oversized account id at the boundary before it reaches
/// the RPC call.
pub(super) fn validate_account_id(account_id: &str) -> Result<(), NearVerifyError> {
    if account_id.is_empty() || account_id.len() > MAX_ACCOUNT_ID_LEN {
        return Err(NearVerifyError::InvalidInput("invalid account id".into()));
    }
    Ok(())
}

/// Re-encode the raw 32-byte public key to NEAR's canonical
/// `ed25519:<base58>` form for the RPC `view_access_key` query.
pub(super) fn canonical_public_key(public_key: &[u8; 32]) -> String {
    format!("ed25519:{}", bs58::encode(public_key).into_string())
}

/// Confirm `public_key` is an active access key on `account_id` via a
/// `view_access_key` RPC query against the configured network. A key
/// that exists on a different network fails here, which is how
/// wrong-network logins are rejected.
pub(super) async fn verify_access_key(
    http: &reqwest::Client,
    rpc_url: &str,
    account_id: &str,
    canonical_public_key: &str,
) -> Result<(), NearVerifyError> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "ironclaw",
        "method": "query",
        "params": {
            "request_type": "view_access_key",
            "finality": "final",
            "account_id": account_id,
            "public_key": canonical_public_key,
        }
    });

    let resp = http
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| NearVerifyError::RpcBackend(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Body is untrusted external content — bound it and do not
        // echo it to the client; only the status is operator-facing.
        let _ = read_capped_body(resp).await;
        return Err(NearVerifyError::RpcBackend(format!(
            "RPC returned HTTP {status}"
        )));
    }

    let raw = read_capped_body(resp)
        .await
        .map_err(NearVerifyError::RpcBackend)?;
    let json: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| NearVerifyError::RpcBackend(e.to_string()))?;

    // An RPC-level `error` (key absent, account absent, wrong network)
    // is an auth failure, not a backend fault.
    if let Some(error) = json.get("error") {
        let cause = error
            .get("cause")
            .and_then(|c| c.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");
        return Err(NearVerifyError::AccessKeyInvalid(cause.to_string()));
    }
    if json.get("result").is_none() {
        return Err(NearVerifyError::AccessKeyInvalid(
            "no result for access key query".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::RngCore;
    use rand::rngs::OsRng;

    fn random_signing_key() -> SigningKey {
        let mut b = [0u8; 32];
        OsRng.fill_bytes(&mut b);
        SigningKey::from_bytes(&b)
    }

    #[test]
    fn verifies_nep413_v1_payload() {
        let key = random_signing_key();
        let message = login_message("abcd1234");
        let nonce = [7u8; 32];
        let sig = key.sign(&build_nep413_v1(&message, &nonce, NEAR_RECIPIENT));
        assert!(
            verify_near_signature(
                key.verifying_key().as_bytes(),
                &sig.to_bytes(),
                &message,
                &nonce,
                NEAR_RECIPIENT,
            )
            .is_ok()
        );
    }

    #[test]
    fn verifies_nep413_v2_payload() {
        let key = random_signing_key();
        let message = login_message("ffff");
        let nonce = [42u8; 32];
        let sig = key.sign(&build_nep413_v2(&message, &nonce, NEAR_RECIPIENT));
        assert!(
            verify_near_signature(
                key.verifying_key().as_bytes(),
                &sig.to_bytes(),
                &message,
                &nonce,
                NEAR_RECIPIENT,
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_raw_message_signature_without_nonce_binding() {
        let key = random_signing_key();
        let message = login_message("abcd1234");
        let nonce = [0u8; 32];
        // Signing the bare message (no NEP-413 framing, no nonce) must
        // be rejected — otherwise a captured signature could be replayed.
        let sig = key.sign(message.as_bytes());
        assert!(
            verify_near_signature(
                key.verifying_key().as_bytes(),
                &sig.to_bytes(),
                &message,
                &nonce,
                NEAR_RECIPIENT,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_wrong_key() {
        let signer = random_signing_key();
        let attacker = random_signing_key();
        let message = login_message("abcd1234");
        let nonce = [1u8; 32];
        let sig = signer.sign(&build_nep413_v1(&message, &nonce, NEAR_RECIPIENT));
        assert!(
            verify_near_signature(
                attacker.verifying_key().as_bytes(),
                &sig.to_bytes(),
                &message,
                &nonce,
                NEAR_RECIPIENT,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_signature_bound_to_different_nonce() {
        let key = random_signing_key();
        let message = login_message("abcd1234");
        let signed_nonce = [9u8; 32];
        let sig = key.sign(&build_nep413_v1(&message, &signed_nonce, NEAR_RECIPIENT));
        // Server-side verification uses a DIFFERENT nonce than the one
        // the wallet signed — the signature must not verify.
        let other_nonce = [10u8; 32];
        assert!(
            verify_near_signature(
                key.verifying_key().as_bytes(),
                &sig.to_bytes(),
                &message,
                &other_nonce,
                NEAR_RECIPIENT,
            )
            .is_err()
        );
    }

    #[test]
    fn decode_public_key_requires_prefix_and_32_bytes() {
        let raw = [3u8; 32];
        let encoded = canonical_public_key(&raw);
        assert_eq!(decode_public_key(&encoded).expect("decode"), raw);
        assert!(matches!(
            decode_public_key("no-prefix"),
            Err(NearVerifyError::InvalidInput(_))
        ));
        // base58 of 31 bytes — wrong length after a valid prefix.
        let short = format!("ed25519:{}", bs58::encode([1u8; 31]).into_string());
        assert!(matches!(
            decode_public_key(&short),
            Err(NearVerifyError::InvalidInput(_))
        ));
    }

    #[test]
    fn decode_signature_accepts_base64_and_base58_64_bytes() {
        let sig = [5u8; 64];
        let b64 = base64::engine::general_purpose::STANDARD.encode(sig);
        assert_eq!(decode_signature(&b64).expect("b64"), sig);
        let b58 = bs58::encode(sig).into_string();
        assert_eq!(decode_signature(&b58).expect("b58"), sig);
        assert!(matches!(
            decode_signature("tooshort"),
            Err(NearVerifyError::InvalidInput(_))
        ));
    }

    #[test]
    fn decode_nonce_requires_32_bytes_hex() {
        let nonce = [8u8; 32];
        assert_eq!(decode_nonce_bytes(&hex::encode(nonce)).expect("ok"), nonce);
        assert!(matches!(
            decode_nonce_bytes("zz"),
            Err(NearVerifyError::InvalidNonce)
        ));
        assert!(matches!(
            decode_nonce_bytes(&hex::encode([0u8; 16])),
            Err(NearVerifyError::InvalidNonce)
        ));
    }

    #[test]
    fn validate_account_id_rejects_empty_and_oversized() {
        assert!(validate_account_id("alice.near").is_ok());
        assert!(validate_account_id("").is_err());
        assert!(validate_account_id(&"a".repeat(MAX_ACCOUNT_ID_LEN + 1)).is_err());
    }
}
