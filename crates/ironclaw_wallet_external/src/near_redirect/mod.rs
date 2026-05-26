//! The NEAR browser-wallet redirect [`SigningProvider`] backend.
//!
//! NEAR wallets (NEAR Wallet / Wallet Selector compatible) sign in the user's
//! browser after a redirect: IronClaw sends the user to a wallet URL embedding
//! the unsigned transaction and a `state` parameter cryptographically bound to
//! the gate, the user approves and signs in the wallet, and the wallet
//! redirects back with the signature material. This module builds that redirect
//! ([`NearRedirectSigningProvider::initiate`]) and verifies the proof carried
//! back ([`NearRedirectSigningProvider::verify_resume`]), fail-closed, against
//! the bound [`ApprovedTxHash`], the bound NEAR account / access key, the echoed
//! `state`, and the one-shot grant.
//!
//! NEAR accounts authorize with ed25519 access keys. The wallet attests to the
//! *bound* [`ApprovedTxHash`] (the WYSIWYS digest IronClaw rendered and the
//! wallet UI mirrors) by signing over its raw 32 bytes, exactly as the Solana
//! injected provider does — no `near-primitives` borsh `Transaction` roundtrip
//! is pulled in. The full borsh `Transaction` decode/roundtrip is a deferred
//! follow-up (PR6 made the same deferral); the transaction bytes ride the
//! redirect URL as an opaque base64 payload here.

mod state;
mod verify;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_attestation::{GrantError, GrantKey, SealedGrantStore};
use ironclaw_signing_provider::{
    ApprovedTxHash, DecodedTransaction, InitiationOutcome, ProviderId, RenderedTx, SigningContext,
    SigningProof, SigningProvider, SigningProviderError, TrustModel, VerifiedProof,
};

pub use state::{NearRedirectState, decode_state, derive_state, encode_state, verify_state};

/// The NEAR access-key permission scope a proof claims it signed under.
///
/// `#[serde(rename_all = "snake_case", tag = "kind")]` pins the wire form: these
/// tags ride in the persisted gate proof, so they must not drift. The scope is
/// validated against the bound operation in
/// [`NearRedirectSigningProvider::verify_resume`] (threat #22: NEAR access-key
/// scope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum NearAccessKeyScope {
    /// A full-access key: authorizes any action on the account. Always covers
    /// the bound operation.
    FullAccess,
    /// A function-call access key: restricted to calling specific methods on a
    /// single receiver, optionally with a gas allowance. The bound receiver and
    /// method must fall within these limits.
    FunctionCall {
        /// The single contract account the key may call.
        receiver_id: String,
        /// The method names the key may call. Empty means "any method on
        /// `receiver_id`" (NEAR's convention for an unrestricted method list).
        method_names: Vec<String>,
    },
}

/// The operation a NEAR redirect proof is bound to, used to validate the
/// declared access-key scope (threat #22).
///
/// This is recomputed by the verifier from the bound transaction; it is never
/// taken from caller-supplied proof fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearBoundOperation {
    /// The receiver (contract) account the bound transaction calls, if it is a
    /// function call. `None` for a plain transfer / full-access-only operation.
    pub receiver_id: Option<String>,
    /// The method the bound transaction calls, if it is a function call.
    pub method_name: Option<String>,
}

/// The structured payload a NEAR wallet carries back on the redirect callback,
/// serialized into the opaque [`SigningProof::NearRedirectProof`] byte body.
///
/// The wallet attests to the *bound* [`ApprovedTxHash`] by signing over its raw
/// 32 bytes with the account's ed25519 access key. The payload echoes that hash,
/// the claimed NEAR account, the access-key public key, the declared key scope,
/// and the `state` parameter so the verifier can re-check every binding without
/// trusting any caller-supplied chain bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearRedirectProofPayload {
    /// The approved-tx hash the wallet attests to. MUST equal the bound hash;
    /// re-checked in [`NearRedirectSigningProvider::verify_resume`] (threat #3).
    pub approved_tx_hash: ApprovedTxHash,
    /// The NEAR account id the wallet claims signed (e.g. `alice.near`).
    /// Compared to the bound account; the claim itself is never trusted
    /// (threat #4).
    pub account_id: String,
    /// The ed25519 access-key public key bytes (32 bytes) the signature verifies
    /// against. Lowercase hex on the wire.
    #[serde(with = "hex_bytes")]
    pub public_key: Vec<u8>,
    /// The raw 64-byte ed25519 signature over the 32-byte approved hash.
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
    /// The access-key scope the wallet signed under. Validated against the bound
    /// operation (threat #22).
    pub access_key_scope: NearAccessKeyScope,
    /// The opaque `state` parameter echoed back from the redirect. Re-derived
    /// and compared by the verifier to defeat callback / deep-link interception
    /// (threat #20).
    pub state: String,
}

/// Serialize a [`NearRedirectProofPayload`] into opaque proof bytes for
/// [`SigningProof::NearRedirectProof`].
pub fn encode_near_redirect_proof(
    payload: &NearRedirectProofPayload,
) -> Result<Vec<u8>, SigningProviderError> {
    // serde_json keeps the encoding self-describing and stable across the wire;
    // the opaque-bytes contract of `SigningProof::NearRedirectProof` is satisfied
    // by any deterministic round-trippable encoding. Propagate the error rather
    // than silently returning empty bytes (which would only surface as an opaque
    // decode failure later); `.unwrap()`/`.expect()` is banned in production.
    serde_json::to_vec(payload).map_err(|e| SigningProviderError::ProofInvalid {
        reason: format!("failed to encode near redirect proof payload: {e}"),
    })
}

/// Decode opaque [`SigningProof::NearRedirectProof`] bytes into a structured
/// [`NearRedirectProofPayload`].
pub fn decode_near_redirect_proof(
    bytes: &[u8],
) -> Result<NearRedirectProofPayload, SigningProviderError> {
    // The payload is a small fixed-shape record; a generous 64 KiB ceiling
    // rejects attacker-inflated callback bodies before the serde allocator.
    const MAX_PROOF_BYTES: usize = 64 * 1024;
    if bytes.len() > MAX_PROOF_BYTES {
        return Err(SigningProviderError::ProofInvalid {
            reason: format!(
                "near redirect proof payload too large: {} bytes (max {MAX_PROOF_BYTES})",
                bytes.len()
            ),
        });
    }
    serde_json::from_slice(bytes).map_err(|e| SigningProviderError::ProofInvalid {
        reason: format!("malformed near redirect proof payload: {e}"),
    })
}

/// The NEAR browser-wallet redirect signing backend.
///
/// Holds the wallet base URL (where the user is redirected to sign), the
/// callback URL the wallet redirects back to, a server-side secret used to bind
/// the `state` parameter to the gate, and a handle to the sealed-grant store so
/// [`Self::verify_resume`] can claim the one-shot grant atomically. Holds **no
/// signing key material** — the wallet owns the account's keys.
pub struct NearRedirectSigningProvider {
    wallet_base_url: String,
    callback_url: String,
    state_secret: Vec<u8>,
    grants: Arc<dyn SealedGrantStore>,
}

impl NearRedirectSigningProvider {
    /// Construct over the wallet redirect/callback URLs, a server-side
    /// `state_secret` (used to MAC-bind the `state` parameter to the gate), and
    /// a sealed-grant store.
    pub fn new(
        wallet_base_url: impl Into<String>,
        callback_url: impl Into<String>,
        state_secret: impl Into<Vec<u8>>,
        grants: Arc<dyn SealedGrantStore>,
    ) -> Self {
        Self {
            wallet_base_url: wallet_base_url.into(),
            callback_url: callback_url.into(),
            state_secret: state_secret.into(),
            grants,
        }
    }

    /// Build the NEAR wallet redirect URL for a bound transaction.
    ///
    /// Embeds the opaque (base64) transaction bytes, the callback URL, and the
    /// gate-bound `state` parameter. This is the directive surfaced to the user
    /// in [`InitiationOutcome::AwaitingUserAction`].
    fn build_redirect_url(
        &self,
        context: &SigningContext,
        decoded: &DecodedTransaction,
        approved_tx_hash: &ApprovedTxHash,
    ) -> String {
        use base64::Engine as _;
        let tx_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(decoded.as_opaque());
        let state = derive_state(&self.state_secret, context, approved_tx_hash);
        let sep = if self.wallet_base_url.contains('?') {
            '&'
        } else {
            '?'
        };
        format!(
            "{base}{sep}transactions={tx}&callbackUrl={cb}&state={state}",
            base = self.wallet_base_url,
            tx = url_encode(&tx_b64),
            cb = url_encode(&self.callback_url),
            state = url_encode(&state),
        )
    }
}

#[async_trait]
impl SigningProvider for NearRedirectSigningProvider {
    fn provider_id(&self) -> ProviderId {
        ProviderId::NearRedirect
    }

    fn trust_model(&self) -> TrustModel {
        TrustModel::ExternalWallet
    }

    async fn initiate(
        &self,
        context: &SigningContext,
        decoded: &DecodedTransaction,
        _rendered: &RenderedTx,
        approved_tx_hash: &ApprovedTxHash,
    ) -> Result<InitiationOutcome, SigningProviderError> {
        // The NEAR wallet signs after a browser redirect, so the user must be
        // sent to an external surface. The directive is the redirect URL; the
        // owning channel (web gateway) delivers it outbound to the user.
        let url = self.build_redirect_url(context, decoded, approved_tx_hash);
        Ok(InitiationOutcome::AwaitingUserAction {
            directive: url.into_bytes(),
        })
    }

    async fn verify_resume(
        &self,
        context: &SigningContext,
        approved_tx_hash: &ApprovedTxHash,
        proof: &SigningProof,
    ) -> Result<VerifiedProof, SigningProviderError> {
        // Only NEAR redirect proofs are accepted by this provider; anything else
        // is a routing error and fails closed.
        let SigningProof::NearRedirectProof(bytes) = proof else {
            return Err(SigningProviderError::ProofInvalid {
                reason: "near redirect provider received a non-near-redirect proof".to_string(),
            });
        };
        let payload = decode_near_redirect_proof(bytes)?;

        // 1. Hash binding (threat #3): the wallet must have attested to the
        //    exact bound hash. Reject before any signature work.
        if &payload.approved_tx_hash != approved_tx_hash {
            return Err(SigningProviderError::ProofInvalid {
                reason: "proof approved-tx hash does not match the bound hash".to_string(),
            });
        }

        // 2. State echo (threat #20): re-derive the gate-bound state and require
        //    the callback to echo it. Defeats redirect / deep-link interception:
        //    a callback for a different gate carries a different state.
        if !verify_state(
            &self.state_secret,
            context,
            approved_tx_hash,
            &payload.state,
        ) {
            return Err(SigningProviderError::ProofInvalid {
                reason: "state parameter does not match the gate-bound state".to_string(),
            });
        }

        // 3. Account + key binding (threat #4): parse the gate-bound NEAR
        //    identity into its account id and its expected access-key public key.
        //    BOTH are fixed at gate-raise (from Wallet Selector) and MUST match
        //    the callback — the public key in particular is NEVER trusted from
        //    the callback, exactly as the Solana injected provider binds the
        //    signer pubkey. Binding only the account would let an attacker who
        //    knows the (public) account id supply their own keypair and forge an
        //    approval, since a NEAR account may hold many access keys.
        let bound = BoundNearIdentity::parse(context.key_or_account_id.as_str())?;
        if payload.account_id != bound.account_id {
            return Err(SigningProviderError::SignerMismatch);
        }
        if payload.public_key != bound.public_key {
            return Err(SigningProviderError::SignerMismatch);
        }

        // 4. Signed-bytes binding (threat #2): verify the ed25519 signature over
        //    the bound 32 hash bytes against the GATE-BOUND access-key public key
        //    (`bound.public_key`, NOT the callback-supplied one — they are now
        //    proven equal above). Because the signed message *is* the approved
        //    canonical hash, a valid signature proves the bound wallet signed
        //    exactly the approved bytes.
        verify::verify_signature_over_hash(
            approved_tx_hash.as_bytes(),
            &payload.signature,
            &bound.public_key,
        )?;

        // 5. Access-key scope (threat #22): the declared scope must cover the
        //    bound operation. A function-call key restricted to a different
        //    receiver / method cannot authorize this transaction.
        let bound_op = decode_bound_operation(context, approved_tx_hash);
        validate_access_key_scope(&payload.access_key_scope, &bound_op)?;

        // 6. One-shot grant (threat #1): claim the sealed grant atomically. A
        //    replay of an already-claimed grant fails closed here.
        let key = GrantKey::from_context(context, *approved_tx_hash);
        self.grants.claim(&key).await.map_err(map_grant_error)?;

        Ok(VerifiedProof::new(
            ProviderId::NearRedirect,
            *approved_tx_hash,
            proof.clone(),
        ))
    }
}

/// The gate-bound NEAR identity, parsed from [`SigningContext::key_or_account_id`].
///
/// A NEAR account (`alice.near`) is public and may hold many access keys, so the
/// account id alone cannot authenticate a signature: the gate must also bind the
/// specific access-key public key the user signs with (captured at gate-raise
/// from Wallet Selector). The wire form is `account_id:<64-char lowercase-hex
/// 32-byte ed25519 pubkey>`. The bound key — never the callback-supplied one —
/// is what [`NearRedirectSigningProvider::verify_resume`] verifies against
/// (threat #4: signer/key binding, mirroring the Solana injected provider).
struct BoundNearIdentity {
    account_id: String,
    public_key: Vec<u8>,
}

impl BoundNearIdentity {
    fn parse(bound: &str) -> Result<Self, SigningProviderError> {
        // Split on the LAST ':' so account ids that themselves contain ':' (none
        // do in NEAR, but be defensive) keep the key as the final field.
        let (account_id, key_hex) =
            bound
                .rsplit_once(':')
                .ok_or_else(|| SigningProviderError::ProofInvalid {
                    reason: "bound NEAR identity must be `account_id:<hex ed25519 pubkey>`"
                        .to_string(),
                })?;
        if account_id.is_empty() {
            return Err(SigningProviderError::ProofInvalid {
                reason: "bound NEAR identity has an empty account id".to_string(),
            });
        }
        let public_key =
            hex_bytes::hex_decode(key_hex).map_err(|e| SigningProviderError::ProofInvalid {
                reason: format!("bound NEAR access-key pubkey is not valid hex: {e}"),
            })?;
        if public_key.len() != 32 {
            return Err(SigningProviderError::ProofInvalid {
                reason: format!(
                    "bound NEAR access-key pubkey must be 32 bytes, got {}",
                    public_key.len()
                ),
            });
        }
        Ok(Self {
            account_id: account_id.to_string(),
            public_key,
        })
    }
}

/// Recompute the bound NEAR operation from authoritative context.
///
/// PR6 deferred the full `near-primitives` borsh `Transaction` decode, so the
/// per-action receiver/method are not yet recoverable from the transaction bytes
/// at this layer. Until that decode lands, the bound operation is treated as
/// unconstrained (`None`/`None`): a `FullAccess` key always passes, and a
/// `FunctionCall` key is accepted as long as it is internally well-formed (the
/// scope still cannot be *escalated* beyond what the wallet declares). The
/// receiver/method cross-check against the decoded transaction is wired once the
/// borsh decode follow-up lands.
fn decode_bound_operation(
    _context: &SigningContext,
    _approved_tx_hash: &ApprovedTxHash,
) -> NearBoundOperation {
    // PR-followup(near-borsh): recover receiver_id/method_name from the decoded
    // borsh Transaction once the heavy-SDK-free decode lands, and cross-check
    // them against the FunctionCall scope below.
    NearBoundOperation {
        receiver_id: None,
        method_name: None,
    }
}

/// Validate that the declared access-key scope covers the bound operation
/// (threat #22).
fn validate_access_key_scope(
    scope: &NearAccessKeyScope,
    bound: &NearBoundOperation,
) -> Result<(), SigningProviderError> {
    match scope {
        // A full-access key authorizes any action on the account.
        NearAccessKeyScope::FullAccess => Ok(()),
        NearAccessKeyScope::FunctionCall {
            receiver_id,
            method_names,
        } => {
            if receiver_id.is_empty() {
                return Err(SigningProviderError::ScopeViolation {
                    reason: "function-call access key declares an empty receiver".to_string(),
                });
            }
            // Fail-closed (threat #22): a FunctionCall key is restricted to a
            // single receiver, so we MUST be able to prove the key covers the
            // bound operation's receiver. Until the borsh transaction decode
            // lands, `decode_bound_operation` returns `None`, meaning we cannot
            // recover the tx's actual receiver — so we cannot prove the
            // restricted key authorizes THIS operation. Accepting it would let a
            // key scoped to receiver A authorize a transaction to receiver B
            // (the verifier can't tell them apart yet). Refuse rather than
            // fail open; a FullAccess key (or the post-decode follow-up) is the
            // supported path.
            let Some(bound_receiver) = &bound.receiver_id else {
                return Err(SigningProviderError::ScopeViolation {
                    reason: "function-call access-key scope cannot be verified against the bound \
                             operation (NEAR transaction receiver decode not yet available); \
                             refuse fail-closed"
                        .to_string(),
                });
            };
            if receiver_id != bound_receiver {
                return Err(SigningProviderError::ScopeViolation {
                    reason: format!(
                        "access key receiver `{receiver_id}` does not match bound receiver \
                         `{bound_receiver}`"
                    ),
                });
            }
            if let Some(bound_method) = &bound.method_name
                && !method_names.is_empty()
                && !method_names.iter().any(|m| m == bound_method)
            {
                return Err(SigningProviderError::ScopeViolation {
                    reason: format!(
                        "bound method `{bound_method}` is not in the access key's method list"
                    ),
                });
            }
            Ok(())
        }
    }
}

/// Map a [`GrantError`] onto the provider error taxonomy, fail-closed.
fn map_grant_error(err: GrantError) -> SigningProviderError {
    match err {
        // Replay / missing / lost-CAS all collapse to a single fail-closed
        // grant-claim failure; the distinction is not safe to leak to a caller.
        GrantError::AlreadyClaimed | GrantError::NotFound | GrantError::AlreadySealed => {
            SigningProviderError::GrantClaimFailed
        }
        GrantError::Backend { reason } => SigningProviderError::Provider { reason },
    }
}

/// Percent-encode a URL query component (the small, dependency-free subset we
/// need: keeps unreserved chars, escapes everything else).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(
                    char::from_digit((b >> 4) as u32, 16)
                        .unwrap_or('0')
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((b & 0x0f) as u32, 16)
                        .unwrap_or('0')
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
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
        let s = s.strip_prefix("0x").unwrap_or(s);
        // Decode over BYTES, not `&str` char slices: `&s[i..i+2]` panics when the
        // input contains a multi-byte (non-ASCII) char straddling an odd byte
        // offset, and proof payloads come from external wallets (callback DoS).
        let bytes = s.as_bytes();
        if !bytes.len().is_multiple_of(2) {
            return Err("odd-length hex".to_string());
        }
        bytes
            .chunks_exact(2)
            .map(|pair| {
                let hi = decode_nibble(pair[0])?;
                let lo = decode_nibble(pair[1])?;
                Ok((hi << 4) | lo)
            })
            .collect()
    }

    fn decode_nibble(b: u8) -> Result<u8, String> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            other => Err(format!("invalid hex byte: {other:#x}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(receiver: Option<&str>, method: Option<&str>) -> NearBoundOperation {
        NearBoundOperation {
            receiver_id: receiver.map(str::to_string),
            method_name: method.map(str::to_string),
        }
    }

    #[test]
    fn full_access_scope_always_covers_bound_operation() {
        assert!(
            validate_access_key_scope(&NearAccessKeyScope::FullAccess, &op(None, None)).is_ok()
        );
        assert!(
            validate_access_key_scope(
                &NearAccessKeyScope::FullAccess,
                &op(Some("c.near"), Some("transfer")),
            )
            .is_ok()
        );
    }

    #[test]
    fn function_call_scope_rejects_empty_receiver() {
        let scope = NearAccessKeyScope::FunctionCall {
            receiver_id: String::new(),
            method_names: vec![],
        };
        let err = validate_access_key_scope(&scope, &op(None, None)).expect_err("empty receiver");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn function_call_scope_with_unknown_bound_operation_fails_closed() {
        // Until the borsh tx decode lands, the bound receiver is `None`. A
        // restricted FunctionCall key cannot be proven to cover the bound
        // operation, so it MUST be refused fail-closed (threat #22) rather than
        // accepted on the wallet's word.
        let scope = NearAccessKeyScope::FunctionCall {
            receiver_id: "a.near".to_string(),
            method_names: vec!["foo".to_string()],
        };
        let err = validate_access_key_scope(&scope, &op(None, None))
            .expect_err("unknown bound operation must fail closed for a function-call key");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn function_call_scope_rejects_mismatched_bound_receiver() {
        let scope = NearAccessKeyScope::FunctionCall {
            receiver_id: "a.near".to_string(),
            method_names: vec![],
        };
        let err = validate_access_key_scope(&scope, &op(Some("b.near"), None))
            .expect_err("receiver mismatch");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn function_call_scope_rejects_method_not_in_list() {
        let scope = NearAccessKeyScope::FunctionCall {
            receiver_id: "a.near".to_string(),
            method_names: vec!["foo".to_string()],
        };
        let err = validate_access_key_scope(&scope, &op(Some("a.near"), Some("bar")))
            .expect_err("method mismatch");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn function_call_scope_accepts_matching_receiver_and_method() {
        let scope = NearAccessKeyScope::FunctionCall {
            receiver_id: "a.near".to_string(),
            method_names: vec!["foo".to_string()],
        };
        assert!(validate_access_key_scope(&scope, &op(Some("a.near"), Some("foo"))).is_ok());
    }

    #[test]
    fn url_encode_escapes_reserved_and_keeps_unreserved() {
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(url_encode("a b&c=d"), "a%20b%26c%3Dd");
    }

    #[test]
    fn proof_payload_round_trips_through_opaque_bytes() {
        let payload = NearRedirectProofPayload {
            approved_tx_hash: ApprovedTxHash::from_bytes([4u8; 32]),
            account_id: "alice.near".to_string(),
            public_key: vec![3u8; 32],
            signature: vec![9u8; 64],
            access_key_scope: NearAccessKeyScope::FullAccess,
            state: "abc".to_string(),
        };
        let bytes = encode_near_redirect_proof(&payload).expect("encode");
        let back = decode_near_redirect_proof(&bytes).expect("decode");
        assert_eq!(back, payload);
    }
}
