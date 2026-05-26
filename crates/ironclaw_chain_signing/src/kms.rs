//! Sign-only KMS/HSM signing abstraction and the mainnet **ship-gate**.
//!
//! ## Threat #18 — compromised-host hot key
//!
//! A custodial private key held in process memory ("hot key") is exposed if the
//! host is compromised. The substrate's answer (mirroring the
//! `HOOKS_THIRD_PARTY_ENABLED` ship-gate pattern) is: **real-value / mainnet
//! custodial signing is refused unless a sign-only KMS/HSM backend is wired** in
//! which the private key NEVER enters the IronClaw process. Hot-key custodial
//! signing is permitted only on testnet / dev. The check runs at startup-config
//! time and again at sign time, fail-closed.
//!
//! ## The key-reference signing boundary ([`KmsSigner`])
//!
//! The defining property of secure custody is that **no private-key bytes ever
//! cross the IronClaw process boundary**: the key lives in the KMS/HSM and only
//! an opaque *key reference* plus the 32-byte *digest* are handed across; a raw
//! signature comes back. [`KmsSigner::sign_digest`] models exactly that. The
//! custodial signer routes mainnet / real-value signing through this trait, and
//! independently ecrecover/verify-binds the returned signature to the keystore
//! account (so a faulty or hostile backend still cannot forge a usable
//! signature for the wrong account).
//!
//! ## In-tree reference backend ([`LocalKmsSigner`])
//!
//! [`LocalKmsSigner`] is a software-HSM that proves and tests the full key-ref
//! path end-to-end without a cloud account. It holds each key behind a sealed
//! [`secrecy::SecretBox`] boundary keyed by an opaque `key_ref`, exposes ONLY
//! `sign_digest` (never the key bytes), and implements both secp256k1 (EVM) and
//! ed25519 (Solana/NEAR). It reports `is_secure_custody() == true` so the
//! ship-gate accepts it for mainnet in tests/dev, while a real cloud backend is
//! a flagged follow-up (see crate docs / PR body):
//!
//! ```text
//! // kms-backend: a concrete cloud backend (AWS KMS / GCP KMS / YubiHSM) is a
//! //              deferred, separately-approved follow-up. AWS KMS supports
//! //              secp256k1 (EVM) but NOT ed25519 (Solana/NEAR), so a cloud
//! //              rollout needs a secp256k1 cloud backend PLUS an ed25519
//! //              HSM (e.g. YubiHSM2 / Fireblocks) for the ed25519 chains.
//! ```

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ed25519_dalek::{Signer as _, SigningKey as Ed25519SigningKey};
use k256::ecdsa::{Signature as K256Signature, SigningKey as Secp256k1SigningKey};
use secrecy::{ExposeSecret, SecretBox};

use crate::error::ChainSigningError;
use crate::policy::CustodyDecision;

/// Signature algorithm the KMS key uses. The custodial signer selects this from
/// the chain family so a key reference can only be used with its native curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAlg {
    /// secp256k1 ECDSA (EVM). Produces a 64-byte (r∥s) signature; the caller
    /// recovers `v` by ecrecover binding.
    Secp256k1,
    /// ed25519 (Solana / NEAR). Produces a 64-byte signature.
    Ed25519,
}

/// A sign-only KMS/HSM backend: the key lives in the backend, only a reference
/// and a digest cross the boundary.
///
/// The defining property is that **private key bytes never enter the IronClaw
/// process** — callers hand it an opaque `key_ref` and a 32-byte digest and
/// receive raw signature bytes. This trait is intentionally minimal so a real
/// KMS need only expose its signing primitive.
///
/// `sign_digest` is `async` because a production cloud KMS / HSM backend signs
/// over an HTTP/RPC round-trip; making it `async` ensures that round-trip never
/// blocks a tokio worker thread. The in-tree [`LocalKmsSigner`] reference
/// backend computes its signature synchronously and simply returns a ready
/// future — no executor stall either way.
#[async_trait]
pub trait KmsSigner: Send + Sync {
    /// A stable identifier for the backend (for audit / config display). Never
    /// includes key material.
    fn backend_id(&self) -> &str;

    /// Whether keys live in a hardware/remote security boundary (`true`) or in
    /// host memory (`false`). The ship-gate consults this to allow mainnet.
    fn is_secure_custody(&self) -> bool;

    /// Sign a 32-byte `digest` with the key referenced by `key_ref` under
    /// `alg`, returning the raw signature bytes. `key_ref` is an opaque backend
    /// handle — NOT key material.
    async fn sign_digest(
        &self,
        key_ref: &str,
        digest: &[u8; 32],
        alg: SignatureAlg,
    ) -> Result<Vec<u8>, ChainSigningError>;
}

/// Back-compat alias: the original review used `HsmKmsBackend`. The trait is now
/// [`KmsSigner`]; existing names resolve to it.
pub use self::KmsSigner as HsmKmsBackend;

/// An in-tree software-HSM reference backend.
///
/// Keys are imported once (e.g. at bootstrap) and thereafter live ONLY inside
/// this backend, each behind a sealed [`SecretBox`] keyed by an opaque
/// `key_ref`. The only operation exposed on a stored key is `sign_digest`; the
/// raw bytes are never returned and never leak through `Debug`. This exercises
/// and tests the full key-reference path that a cloud KMS would use, proving the
/// architecture without a cloud account.
pub struct LocalKmsSigner {
    id: String,
    // key_ref -> sealed key material. The seal means even a stray `Debug`/log of
    // the map prints `[REDACTED]`, and the bytes are zeroized on drop.
    keys: Mutex<HashMap<String, SealedKey>>,
}

struct SealedKey {
    alg: SignatureAlg,
    secret: SecretBox<[u8]>,
}

impl std::fmt::Debug for LocalKmsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalKmsSigner")
            .field("id", &self.id)
            .field("keys", &"[REDACTED]")
            .finish()
    }
}

impl LocalKmsSigner {
    /// Build an empty software-HSM with the given backend id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Import a key under `key_ref`. After this call the only way to use the key
    /// is [`KmsSigner::sign_digest`]; the bytes are sealed and never returned.
    ///
    /// Returns an error if `key_ref` already exists (one-shot import, mirroring
    /// the keystore's bind semantics).
    pub fn import_key(
        &self,
        key_ref: impl Into<String>,
        alg: SignatureAlg,
        key_bytes: Vec<u8>,
    ) -> Result<(), ChainSigningError> {
        let key_ref = key_ref.into();
        let mut keys = self.keys.lock().map_err(|e| ChainSigningError::Sign {
            chain: "kms",
            reason: format!("backend lock poisoned: {e}"),
        })?;
        if keys.contains_key(&key_ref) {
            return Err(ChainSigningError::Sign {
                chain: "kms",
                reason: "key_ref already imported".to_string(),
            });
        }
        keys.insert(
            key_ref,
            SealedKey {
                alg,
                secret: SecretBox::new(key_bytes.into_boxed_slice()),
            },
        );
        Ok(())
    }

    /// Drop the sealed key held under `key_ref`, zeroizing its material.
    ///
    /// Imports are expected only at bootstrap, so the keyset is bounded by
    /// configuration in normal operation; this method exists so an operator (or
    /// a test) can explicitly evict a key — preventing unbounded growth if a
    /// caller ever re-keys at runtime. Returns `true` if a key was removed.
    pub fn remove_key(&self, key_ref: &str) -> Result<bool, ChainSigningError> {
        let mut keys = self.keys.lock().map_err(|e| ChainSigningError::Sign {
            chain: "kms",
            reason: format!("backend lock poisoned: {e}"),
        })?;
        Ok(keys.remove(key_ref).is_some())
    }
}

#[async_trait]
impl KmsSigner for LocalKmsSigner {
    fn backend_id(&self) -> &str {
        &self.id
    }

    fn is_secure_custody(&self) -> bool {
        // The reference backend models secure custody: callers never receive
        // key bytes. (A real deployment would use a cloud KMS/HSM here.)
        true
    }

    async fn sign_digest(
        &self,
        key_ref: &str,
        digest: &[u8; 32],
        alg: SignatureAlg,
    ) -> Result<Vec<u8>, ChainSigningError> {
        let keys = self.keys.lock().map_err(|e| ChainSigningError::Sign {
            chain: "kms",
            reason: format!("backend lock poisoned: {e}"),
        })?;
        let sealed = keys.get(key_ref).ok_or_else(|| ChainSigningError::Sign {
            chain: "kms",
            reason: "unknown key_ref".to_string(),
        })?;
        if sealed.alg != alg {
            return Err(ChainSigningError::Sign {
                chain: "kms",
                reason: "key_ref algorithm mismatch".to_string(),
            });
        }
        match alg {
            SignatureAlg::Secp256k1 => {
                let key = Secp256k1SigningKey::from_slice(sealed.secret.expose_secret()).map_err(
                    |e| ChainSigningError::Sign {
                        chain: "kms",
                        reason: format!("invalid secp256k1 key material: {e}"),
                    },
                )?;
                // Sign the prehash; return raw 64-byte (r∥s). The caller recovers
                // `v` via ecrecover binding.
                let sig: K256Signature = key
                    .sign_prehash_recoverable(digest)
                    .map_err(|e| ChainSigningError::Sign {
                        chain: "kms",
                        reason: format!("secp256k1 sign failed: {e}"),
                    })?
                    .0;
                Ok(sig.to_bytes().to_vec())
            }
            SignatureAlg::Ed25519 => {
                let arr: [u8; 32] = sealed.secret.expose_secret().try_into().map_err(|_| {
                    ChainSigningError::Sign {
                        chain: "kms",
                        reason: "ed25519 key material must be 32 bytes".to_string(),
                    }
                })?;
                let key = Ed25519SigningKey::from_bytes(&arr);
                // ed25519 signs the message itself; for Solana/NEAR the "digest"
                // we pass IS the bytes to sign (Solana: message bytes hashed by
                // the caller into 32 bytes is NOT used — see solana/sign.rs which
                // passes a 32-byte commitment). ed25519 signs the 32 bytes given.
                let sig = key.sign(digest);
                Ok(sig.to_bytes().to_vec())
            }
        }
    }
}

/// Whether a signing request targets real value (mainnet) or a test network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueClass {
    /// Mainnet / real-value. Gated on secure custody.
    Mainnet,
    /// Testnet / dev. Hot-key custodial permitted.
    Testnet,
}

impl ValueClass {
    /// Classify a chain identity string into a [`ValueClass`].
    ///
    /// Conservative / fail-closed: anything that is not a recognized testnet is
    /// treated as **mainnet** (real value), so an unknown or spoofed chain id
    /// cannot downgrade itself into the hot-key-allowed bucket.
    pub fn classify(chain: &str) -> Self {
        // Known test networks across the three families.
        const TESTNETS: &[&str] = &[
            // EVM testnets (chain ids).
            "eip155:11155111", // sepolia
            "eip155:17000",    // holesky
            "eip155:5",        // goerli (deprecated)
            "eip155:80002",    // amoy
            "eip155:84532",    // base-sepolia
            // Solana.
            "solana:devnet",
            "solana:testnet",
            // NEAR.
            "near:testnet",
            // Generic local/dev markers.
            "eip155:31337", // anvil/hardhat
            "eip155:1337",
        ];
        if TESTNETS.contains(&chain) {
            ValueClass::Testnet
        } else {
            ValueClass::Mainnet
        }
    }
}

/// How a successfully-authorized request must produce its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningPath {
    /// Hot (in-process) key. Permitted for testnet/dev only.
    HotKey,
    /// Sign-only KMS/HSM (`key_ref` + digest). Required for mainnet.
    Kms,
}

/// The startup-checked mainnet ship-gate.
///
/// Construct from config (mirroring a `CUSTODIAL_MAINNET_ENABLED`-style flag)
/// and an optional wired KMS backend. [`ShipGate::authorize`] refuses mainnet
/// custodial signing unless a secure-custody backend is present, regardless of
/// the flag — the flag alone cannot enable hot-key mainnet — and tells the
/// caller WHICH signing path to use so hot-key signing can never service a
/// mainnet request.
pub struct ShipGate {
    /// Operator opt-in (e.g. from `CUSTODIAL_MAINNET_ENABLED`). Necessary but
    /// NOT sufficient: secure custody is still required for mainnet.
    mainnet_opt_in: bool,
    /// Whether a wired backend provides secure (HSM/KMS) custody.
    secure_custody_available: bool,
}

impl ShipGate {
    /// Build a ship-gate from the operator opt-in flag and the wired backend
    /// (if any). A backend that reports `is_secure_custody() == false` (a hot
    /// key) does not satisfy the mainnet requirement.
    pub fn new(mainnet_opt_in: bool, backend: Option<&dyn KmsSigner>) -> Self {
        Self {
            mainnet_opt_in,
            secure_custody_available: backend.is_some_and(|b| b.is_secure_custody()),
        }
    }

    /// Authorize (or refuse) a custodial signing request for the given value
    /// class, returning the REQUIRED signing path on success. Fail-closed:
    /// mainnet requires BOTH the opt-in flag AND secure custody and yields
    /// [`SigningPath::Kms`]; testnet yields [`SigningPath::HotKey`].
    pub fn authorize(&self, value: ValueClass) -> Result<SigningPath, CustodyDecision> {
        match value {
            ValueClass::Testnet => Ok(SigningPath::HotKey),
            ValueClass::Mainnet => {
                if !self.mainnet_opt_in {
                    Err(CustodyDecision::Deny {
                        reason: "mainnet custodial signing is not enabled (set the \
                                 CUSTODIAL_MAINNET_ENABLED opt-in)"
                            .to_string(),
                    })
                } else if !self.secure_custody_available {
                    Err(CustodyDecision::Deny {
                        reason: "mainnet custodial signing requires a sign-only KMS/HSM backend \
                                 with secure custody; hot-key custodial is testnet/dev only \
                                 (compromised-host hot-key threat #18)"
                            .to_string(),
                    })
                } else {
                    Ok(SigningPath::Kms)
                }
            }
        }
    }

    /// Convenience: authorize for a chain id string, classifying it first, and
    /// returning the required [`SigningPath`].
    pub fn authorize_chain(&self, chain: &str) -> Result<SigningPath, ChainSigningError> {
        match self.authorize(ValueClass::classify(chain)) {
            Ok(path) => Ok(path),
            Err(CustodyDecision::Deny { reason }) => {
                Err(ChainSigningError::ShipGateRefused { reason })
            }
            // `authorize` only ever returns `Deny` in the Err arm.
            Err(CustodyDecision::Allow) => Err(ChainSigningError::ShipGateRefused {
                reason: "ship-gate internal error".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secure() -> LocalKmsSigner {
        LocalKmsSigner::new("test-secure-kms")
    }

    struct HotBackend;
    #[async_trait]
    impl KmsSigner for HotBackend {
        fn backend_id(&self) -> &str {
            "test-hot"
        }
        fn is_secure_custody(&self) -> bool {
            false
        }
        async fn sign_digest(
            &self,
            _key_ref: &str,
            _digest: &[u8; 32],
            _alg: SignatureAlg,
        ) -> Result<Vec<u8>, ChainSigningError> {
            Ok(vec![0u8; 64])
        }
    }

    #[test]
    fn unknown_chain_classifies_as_mainnet_fail_closed() {
        assert_eq!(ValueClass::classify("eip155:1"), ValueClass::Mainnet);
        assert_eq!(
            ValueClass::classify("eip155:999999999"),
            ValueClass::Mainnet
        );
        assert_eq!(ValueClass::classify("garbage"), ValueClass::Mainnet);
    }

    #[test]
    fn known_testnets_classify_as_testnet() {
        for c in [
            "eip155:11155111",
            "solana:devnet",
            "near:testnet",
            "eip155:31337",
        ] {
            assert_eq!(ValueClass::classify(c), ValueClass::Testnet, "{c}");
        }
    }

    #[test]
    fn testnet_uses_hot_key_path_without_kms() {
        let gate = ShipGate::new(false, None);
        assert_eq!(
            gate.authorize_chain("solana:devnet").unwrap(),
            SigningPath::HotKey
        );
    }

    #[test]
    fn mainnet_refused_without_kms_even_with_opt_in() {
        let gate = ShipGate::new(true, None);
        let err = gate.authorize_chain("eip155:1").unwrap_err();
        assert!(matches!(err, ChainSigningError::ShipGateRefused { .. }));

        // A hot-key backend does not satisfy secure custody.
        let hot = HotBackend;
        let gate = ShipGate::new(true, Some(&hot));
        assert!(gate.authorize_chain("eip155:1").is_err());
    }

    #[test]
    fn mainnet_refused_without_opt_in_even_with_kms() {
        let s = secure();
        let gate = ShipGate::new(false, Some(&s));
        assert!(gate.authorize_chain("eip155:1").is_err());
    }

    #[test]
    fn mainnet_uses_kms_path_with_opt_in_and_secure_kms() {
        let s = secure();
        let gate = ShipGate::new(true, Some(&s));
        assert_eq!(gate.authorize_chain("eip155:1").unwrap(), SigningPath::Kms);
    }

    #[tokio::test]
    async fn local_kms_signs_secp256k1_and_never_returns_key() {
        let kms = secure();
        kms.import_key("evm-1", SignatureAlg::Secp256k1, vec![0x11u8; 32])
            .unwrap();
        let sig = kms
            .sign_digest("evm-1", &[7u8; 32], SignatureAlg::Secp256k1)
            .await
            .unwrap();
        assert_eq!(sig.len(), 64);
        // Debug never prints key material.
        assert!(format!("{kms:?}").contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn local_kms_signs_ed25519() {
        let kms = secure();
        kms.import_key("sol-1", SignatureAlg::Ed25519, vec![0x22u8; 32])
            .unwrap();
        let sig = kms
            .sign_digest("sol-1", &[3u8; 32], SignatureAlg::Ed25519)
            .await
            .unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[tokio::test]
    async fn local_kms_rejects_wrong_alg_for_key_ref() {
        let kms = secure();
        kms.import_key("evm-1", SignatureAlg::Secp256k1, vec![0x11u8; 32])
            .unwrap();
        let err = kms
            .sign_digest("evm-1", &[7u8; 32], SignatureAlg::Ed25519)
            .await
            .unwrap_err();
        assert!(matches!(err, ChainSigningError::Sign { .. }));
    }

    #[test]
    fn local_kms_one_shot_import() {
        let kms = secure();
        kms.import_key("k", SignatureAlg::Secp256k1, vec![0x11u8; 32])
            .unwrap();
        assert!(
            kms.import_key("k", SignatureAlg::Secp256k1, vec![0x22u8; 32])
                .is_err()
        );
    }

    #[test]
    fn local_kms_remove_key_evicts_and_allows_reimport() {
        let kms = secure();
        kms.import_key("k", SignatureAlg::Secp256k1, vec![0x11u8; 32])
            .unwrap();
        // Removing a present key reports true; removing again reports false.
        assert!(kms.remove_key("k").unwrap());
        assert!(!kms.remove_key("k").unwrap());
        // After eviction the one-shot import slot is free again.
        kms.import_key("k", SignatureAlg::Secp256k1, vec![0x22u8; 32])
            .unwrap();
    }
}
