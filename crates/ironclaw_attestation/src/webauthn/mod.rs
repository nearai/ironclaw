//! WebAuthn registry + verifier — the custodial-path approval layer.
//!
//! This is the security core of the custodial attested-signing path. A
//! registered passkey ([`registry`]) plus a fully-validated assertion
//! ([`verify`]) is what authorizes IronClaw to use a custody key for exactly
//! one signing operation.
//!
//! ## Crypto layering (openssl-free, pure Rust)
//!
//! WebAuthn assertion verification is security-critical and error-prone: COSE
//! key decoding, ECDSA(P-256)/EdDSA signature checks over
//! `authenticatorData ∥ SHA-256(clientDataJSON)`, and DER handling are exactly
//! the places a hand-rolled implementation introduces silent vulnerabilities.
//! This tree is a `ring`/`rustls` tree and does NOT accept the `openssl` native
//! C dependency, so we do not use `webauthn-rs-core` (which pulls in
//! `openssl`/`openssl-sys`). Instead [`cose`] confines the leaf crypto to three
//! pure-Rust crates: [`coset`] (Apache-2.0) decodes the COSE_Key CBOR, `p256`
//! (RustCrypto) verifies ES256, and `ed25519-dalek` verifies EdDSA. Only ES256
//! and EdDSA are supported; any other algorithm is rejected fail-closed. We own
//! all *Relying-Party policy* checks ourselves in [`verify`], and deliberately
//! do NOT adopt any high-level WebAuthn session/state model: our anti-replay
//! nonce is the [`crate::ChallengePreimage`] commitment from
//! [`crate::challenge`], and binding OUR challenge as the expected challenge
//! requires running the RP checks ourselves.
//!
//! ## Fail-closed posture
//!
//! Every check in [`verify::verify_assertion`] is fail-closed: any failure
//! (missing UV, wrong type, challenge mismatch, rpIdHash mismatch, disallowed
//! origin, signCount regression, bad signature, foreign userHandle, unknown
//! credential) returns an `Err` and NO [`verify::VerifiedAssertion`] is
//! produced. A `VerifiedAssertion` can only exist after the full checklist
//! passed.

pub(crate) mod cose;
pub(crate) mod registry;
pub(crate) mod verify;

pub use cose::{CoseError, CosePublicKey};
pub use registry::{
    Aaguid, AttestationPolicy, BackupFlagPolicy, BootstrapPolicy,
    InMemoryWebAuthnCredentialRegistry, OriginContext, OriginPolicy, RegisteredCredential,
    RegistrationError, RegistrationRequest, SignCountPolicy, StandardOriginPolicy,
    WebAuthnCredentialRegistry,
};
pub use verify::{AssertionInput, VerificationError, VerifiedAssertion, verify_assertion};
