//! WebAuthn assertion verifier — the full Relying-Party (RP) validation.
//!
//! [`verify_assertion`] performs the complete RP checklist, fail-closed on any
//! failure. The leaf cryptographic signature check is delegated to the
//! pure-Rust [`crate::webauthn::cose`] layer (`p256` for ES256, `ed25519-dalek`
//! for EdDSA, `coset` for COSE_Key parsing — no openssl); every *policy* check
//! is owned here so we can bind OUR [`crate::ChallengePreimage`] commitment as
//! the expected challenge (see [`crate::webauthn`] module docs for why).
//!
//! ## RP validation checklist (all enforced, all fail-closed)
//!
//! 1. **Credential is registered** for the expected user (`allowCredentials` /
//!    userHandle ownership binding).
//! 2. **`clientDataJSON.type == "webauthn.get"`** (assertion, not creation).
//! 3. **Echoed challenge equals our issued commitment** (anti-replay; the
//!    commitment is over the full [`crate::ChallengePreimage`]).
//! 4. **Origin (and topOrigin) pass the [`OriginPolicy`]**.
//! 5. **`rpIdHash` == SHA-256(rp_id)** from `authenticatorData`.
//! 6. **User Verification (UV) bit set** — not merely User Presence (UP). UP is
//!    also required (UV without UP is malformed).
//! 7. **Backup-eligibility / backup-state (BE/BS) flag policy** holds.
//! 8. **signCount regression handling** via [`SignCountPolicy`]
//!    (cloned-authenticator detection).
//! 9. **Signature verifies** over `authenticatorData ∥ SHA-256(clientDataJSON)`
//!    against the registered COSE public key.
//!
//! Only after ALL pass is a [`VerifiedAssertion`] produced; it carries the new
//! sign count the call site must persist (atomically with challenge-consume +
//! gate resolution in PR5).

use serde::Deserialize;
use sha2::{Digest, Sha256};

use ironclaw_signing_provider::{ApprovedTxHash, GateRef, UserId};

use crate::challenge::{ChallengeCommitment, ConsumedChallenge, CredentialId};
use crate::webauthn::registry::{
    OriginContext, OriginPolicy, RegisteredCredential, SignCountPolicy, WebAuthnCredentialRegistry,
};

/// Authenticator-data flag bits (WebAuthn / CTAP2).
pub(crate) mod flags {
    /// User Present.
    pub(crate) const UP: u8 = 1 << 0;
    /// User Verified.
    pub(crate) const UV: u8 = 1 << 2;
    /// Backup Eligible.
    pub(crate) const BE: u8 = 1 << 3;
    /// Backup State.
    pub(crate) const BS: u8 = 1 << 4;
}

/// Raw assertion material presented by the client, plus the verification
/// context the RP supplies.
pub struct AssertionInput<'a> {
    /// The user the flow expects to be authenticating (from the gated request).
    pub expected_user: &'a UserId,
    /// The userHandle returned by the authenticator (may be `None` for
    /// non-resident keys). When present it MUST match the expected user.
    pub user_handle: Option<&'a [u8]>,
    /// Credential id from the assertion (selected from `allowCredentials`).
    pub credential_id: &'a CredentialId,
    /// Raw `authenticatorData` bytes.
    pub authenticator_data: &'a [u8],
    /// Raw `clientDataJSON` bytes.
    pub client_data_json: &'a [u8],
    /// Raw assertion signature (DER for ES256, raw for EdDSA — as produced by
    /// the authenticator).
    pub signature: &'a [u8],
    /// The RP id this verification is scoped to.
    pub rp_id: &'a str,
    /// The consumed challenge whose commitment must equal the echoed challenge.
    pub consumed_challenge: &'a ConsumedChallenge,
    /// Mapping of the expected user's stable handle bytes, used to validate the
    /// `user_handle` echo when present.
    pub expected_user_handle: &'a [u8],
}

/// A sealed, fully-validated assertion. Existence of this value is proof the
/// entire RP checklist passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedAssertion {
    /// The user the assertion authenticated.
    pub user: UserId,
    /// The credential id that signed.
    pub credential_id: CredentialId,
    /// The new sign count the call site must persist (>= previous).
    pub new_sign_count: u32,
    /// Whether the backup-state bit was set (carried for the call site /
    /// audit).
    pub backup_state: bool,
    /// The gate this assertion authorized, bound from the consumed challenge
    /// preimage. Carried out as proof of THIS gate so the gate-resolution step
    /// (PR5) resolves exactly the gate the user was challenged on — not a
    /// caller-supplied value.
    pub gate_ref: GateRef,
    /// The approved-transaction binding hash from the consumed preimage (PR2).
    /// Carried out as proof of THIS exact transaction so the signing step signs
    /// only the bytes the user approved.
    pub rendered_tx_digest: ApprovedTxHash,
}

/// Fail-closed verification errors. Each maps to a specific RP check.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerificationError {
    /// No credential registered for `(user, credential_id)`.
    #[error("credential not registered for this user")]
    UnknownCredential,

    /// `clientDataJSON` was not valid UTF-8 / JSON.
    #[error("malformed clientDataJSON: {reason}")]
    MalformedClientData {
        /// Non-secret parse-error description.
        reason: String,
    },

    /// `clientDataJSON.type` was not `webauthn.get`.
    #[error("clientDataJSON.type is not webauthn.get")]
    WrongClientDataType,

    /// The echoed challenge did not equal our issued commitment.
    #[error("challenge mismatch")]
    ChallengeMismatch,

    /// Origin / topOrigin failed the origin policy.
    #[error("origin rejected: {reason}")]
    OriginRejected {
        /// Non-secret rejection reason.
        reason: String,
    },

    /// `authenticatorData` was shorter than the fixed 37-byte header.
    #[error("authenticatorData too short")]
    AuthenticatorDataTooShort,

    /// `rpIdHash` did not match `SHA-256(rp_id)`.
    #[error("rpIdHash mismatch")]
    RpIdHashMismatch,

    /// User Presence (UP) bit was not set.
    #[error("user-presence (UP) bit not set")]
    UserPresenceMissing,

    /// User Verification (UV) bit was not set (UP alone is insufficient).
    #[error("user-verification (UV) bit not set")]
    UserVerificationMissing,

    /// BE/BS flag policy violation (e.g. BS set while BE clear is invalid).
    #[error("backup-flag policy violation: {reason}")]
    BackupFlagPolicy {
        /// Non-secret reason.
        reason: String,
    },

    /// signCount regressed (or otherwise failed the sign-count policy):
    /// possible cloned authenticator.
    #[error("signCount policy violation: {reason}")]
    SignCountPolicy {
        /// Non-secret reason.
        reason: String,
    },

    /// The userHandle returned by the authenticator did not match the expected
    /// user (foreign-credential / cross-account binding failure).
    #[error("userHandle does not match expected user")]
    ForeignUserHandle,

    /// A caller-supplied verification input did not match the corresponding
    /// field bound into the consumed challenge preimage. The assertion context
    /// is not the one that was authorized — fail closed.
    #[error("assertion context does not match the consumed challenge preimage: {field}")]
    PreimageContextMismatch {
        /// Which bound field mismatched (non-secret field name).
        field: &'static str,
    },

    /// The signature did not verify against the registered public key.
    #[error("assertion signature verification failed")]
    BadSignature,

    /// An internal error during signature verification.
    #[error("signature verification error: {reason}")]
    VerificationInternal {
        /// Non-secret description.
        reason: String,
    },
}

/// Minimal `clientDataJSON` shape we validate. Extra fields are ignored; the
/// fields we *require* are explicit.
#[derive(Deserialize)]
struct CollectedClientData {
    #[serde(rename = "type")]
    type_: String,
    /// base64url(no pad) of the challenge bytes the client received.
    challenge: String,
    origin: String,
    #[serde(rename = "topOrigin")]
    top_origin: Option<String>,
    /// `crossOrigin` is OPTIONAL in the serialization; absent ⇒ `false`.
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
}

/// Strict base64url (no padding) decode to bytes, fail-closed.
///
/// Rejects:
/// - any non-alphabet byte (incl. `=` padding — this is the no-pad form),
/// - a length ≡ 1 (mod 4), which cannot encode whole bytes,
/// - non-canonical trailing bits (the leftover `nbits` after the last full
///   byte MUST be zero, so each input has exactly one canonical encoding).
fn b64url_decode(s: &str) -> Result<Vec<u8>, VerificationError> {
    fn malformed(reason: &str) -> VerificationError {
        VerificationError::MalformedClientData {
            reason: reason.to_string(),
        }
    }
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    // A base64 group is 4 chars -> 3 bytes; a trailing group of length 1 is
    // impossible (it would carry only 6 bits, less than one byte).
    if bytes.len() % 4 == 1 {
        return Err(malformed("invalid base64url length in challenge"));
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut nbits = 0u32;
    for &c in bytes {
        let v = val(c).ok_or_else(|| malformed("invalid base64url in challenge"))? as u32;
        acc = (acc << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    // Canonical-encoding check: any leftover bits beyond the last whole byte
    // MUST be zero, otherwise this is a non-canonical (malleable) encoding.
    if nbits > 0 && (acc & ((1 << nbits) - 1)) != 0 {
        return Err(malformed("non-canonical trailing bits in challenge"));
    }
    Ok(out)
}

/// Constant-time-ish equality for the challenge commitment comparison. The
/// commitment is public, but we avoid early-return data-dependent timing as a
/// matter of hygiene.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Perform the full WebAuthn RP validation. Returns a [`VerifiedAssertion`]
/// only if every check passes.
pub fn verify_assertion(
    registry: &dyn WebAuthnCredentialRegistry,
    origin_policy: &dyn OriginPolicy,
    sign_count_policy: &dyn SignCountPolicy,
    input: &AssertionInput<'_>,
) -> Result<VerifiedAssertion, VerificationError> {
    // 0. Bind the assertion context to the consumed challenge preimage.
    //
    // The preimage (produced when WE issued the challenge) is the source of
    // truth for WHO is authenticating, WHICH credential answers, the RP id, and
    // the expected origin. The caller-supplied verification inputs are only
    // *claims*; every one must equal the corresponding bound field or we fail
    // closed BEFORE doing any signature work. This prevents a caller from
    // verifying an assertion against a gate/tx other than the one the user was
    // actually challenged on. `gate_ref` and `rendered_tx_digest` are then
    // carried into the success output as proof of THIS gate/tx.
    let preimage = &input.consumed_challenge.preimage;
    if input.expected_user != &preimage.user {
        return Err(VerificationError::PreimageContextMismatch { field: "user" });
    }
    if input.credential_id != &preimage.credential_id {
        return Err(VerificationError::PreimageContextMismatch {
            field: "credential_id",
        });
    }
    if input.rp_id != preimage.rp_id {
        return Err(VerificationError::PreimageContextMismatch { field: "rp_id" });
    }

    // 1. Credential must be registered for the expected user.
    let credential: RegisteredCredential = registry
        .lookup(&preimage.user, &preimage.credential_id)
        .ok_or(VerificationError::UnknownCredential)?;

    // userHandle ownership binding: when the authenticator returns a userHandle
    // it MUST match the expected user's handle. A foreign handle is rejected.
    if let Some(handle) = input.user_handle
        && !ct_eq(handle, input.expected_user_handle)
    {
        return Err(VerificationError::ForeignUserHandle);
    }

    // 2. Parse + validate clientDataJSON.
    let client_data: CollectedClientData =
        serde_json::from_slice(input.client_data_json).map_err(|e| {
            VerificationError::MalformedClientData {
                reason: e.to_string(),
            }
        })?;

    if client_data.type_ != "webauthn.get" {
        return Err(VerificationError::WrongClientDataType);
    }

    // 3. Echoed challenge must equal our issued commitment.
    let echoed = b64url_decode(&client_data.challenge)?;
    let expected: &ChallengeCommitment = &input.consumed_challenge.preimage.commitment();
    if !ct_eq(&echoed, expected.as_bytes()) {
        return Err(VerificationError::ChallengeMismatch);
    }

    // 4. Origin binding + policy. The asserted origin must equal the origin
    //    bound into the preimage (the origin the challenge was issued for); a
    //    different origin is a preimage-context mismatch, failing closed before
    //    the injectable policy even runs. The policy then additionally vets the
    //    cross-origin posture (topOrigin / crossOrigin).
    if client_data.origin != preimage.expected_origin {
        return Err(VerificationError::PreimageContextMismatch { field: "origin" });
    }
    origin_policy
        .evaluate(&OriginContext {
            rp_id: input.rp_id,
            origin: &client_data.origin,
            top_origin: client_data.top_origin.as_deref(),
            cross_origin: client_data.cross_origin,
        })
        .map_err(|reason| VerificationError::OriginRejected { reason })?;

    // 5-7. Parse authenticatorData header and validate rpIdHash + flags.
    if input.authenticator_data.len() < 37 {
        return Err(VerificationError::AuthenticatorDataTooShort);
    }
    let rp_id_hash = &input.authenticator_data[0..32];
    let flag_byte = input.authenticator_data[32];
    let asserted_sign_count = u32::from_be_bytes([
        input.authenticator_data[33],
        input.authenticator_data[34],
        input.authenticator_data[35],
        input.authenticator_data[36],
    ]);

    let expected_rp_hash: [u8; 32] = Sha256::digest(input.rp_id.as_bytes()).into();
    if !ct_eq(rp_id_hash, &expected_rp_hash) {
        return Err(VerificationError::RpIdHashMismatch);
    }

    // 6. UP and UV. UV is REQUIRED (not just UP). A set UV with clear UP is
    //    malformed and rejected.
    if flag_byte & flags::UP == 0 {
        return Err(VerificationError::UserPresenceMissing);
    }
    if flag_byte & flags::UV == 0 {
        return Err(VerificationError::UserVerificationMissing);
    }

    // 7. BE/BS policy: BS set while BE clear is spec-invalid; and a credential
    //    registered as backup-ineligible must not later assert BE.
    let be = flag_byte & flags::BE != 0;
    let bs = flag_byte & flags::BS != 0;
    if bs && !be {
        return Err(VerificationError::BackupFlagPolicy {
            reason: "backup-state set without backup-eligible".to_string(),
        });
    }
    if be && !credential.backup_eligible {
        return Err(VerificationError::BackupFlagPolicy {
            reason: "credential asserted backup-eligible but was registered ineligible".to_string(),
        });
    }

    // 8. signCount regression handling.
    sign_count_policy
        .evaluate(credential.sign_count, asserted_sign_count)
        .map_err(|reason| VerificationError::SignCountPolicy { reason })?;

    // 9. Signature over authenticatorData ∥ SHA-256(clientDataJSON).
    let client_data_hash: [u8; 32] = Sha256::digest(input.client_data_json).into();
    let mut verification_data =
        Vec::with_capacity(input.authenticator_data.len() + client_data_hash.len());
    verification_data.extend_from_slice(input.authenticator_data);
    verification_data.extend_from_slice(&client_data_hash);

    let ok = credential
        .public_key
        .verify(input.signature, &verification_data)
        .map_err(|e| VerificationError::VerificationInternal {
            reason: e.to_string(),
        })?;
    if !ok {
        return Err(VerificationError::BadSignature);
    }

    Ok(VerifiedAssertion {
        user: credential.user,
        credential_id: credential.credential_id,
        new_sign_count: asserted_sign_count,
        backup_state: bs,
        // Carry the bound gate/tx out of the verifier as proof of exactly which
        // gate + transaction THIS assertion authorized.
        gate_ref: preimage.gate_ref.clone(),
        rendered_tx_digest: preimage.rendered_tx_digest,
    })
}

// The test-only software authenticator helper and the adversarial unit-test
// suite live in a sibling file so this module stays focused on the production
// Relying-Party verification logic.
#[cfg(test)]
#[path = "verify_tests.rs"]
mod verify_tests;

// Keep the canonical `crate::webauthn::verify::test_authenticator` path stable
// for sibling test modules (e.g. registry tests) after the extraction.
#[cfg(test)]
pub(crate) use verify_tests::test_authenticator;
