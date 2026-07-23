//! Pure-Rust COSE public key parsing + leaf assertion-signature verification.
//!
//! This is the openssl-free replacement for the small slice of functionality
//! we previously borrowed from `webauthn-rs-core`: parsing a registered
//! credential's COSE_Key and verifying the assertion signature over
//! `authenticatorData ∥ SHA-256(clientDataJSON)`. Everything else — the full
//! Relying-Party policy validation — lives in [`super::verify`].
//!
//! ## Dependencies (all pure-Rust, no openssl / native C)
//!
//! - [`coset`] (Apache-2.0, Google) decodes the COSE_Key CBOR map into a typed
//!   `CoseKey` (`kty` / `alg` / `params`). We then project out exactly the
//!   fields we support.
//! - `p256` (RustCrypto) verifies ES256 (ECDSA P-256 over SHA-256). WebAuthn
//!   authenticators emit DER-encoded ECDSA signatures, which `p256` parses via
//!   `Signature::from_der`.
//! - `ed25519-dalek` verifies EdDSA (Ed25519), the raw 64-byte signature form.
//!
//! ## Supported algorithms (fail-closed otherwise)
//!
//! Only ES256 (EC2 / P-256) and EdDSA (OKP / Ed25519) are supported — the two
//! algorithms real passkey authenticators use. Any other `kty` / `alg` / `crv`
//! combination is rejected with [`CoseError::UnsupportedAlgorithm`] rather than
//! silently accepted. This is the conservative choice for COSE-parsing
//! ambiguity: an unknown algorithm is a verification failure, never a pass.

use coset::iana::{Algorithm, Ec2KeyParameter, EllipticCurve, KeyType, OkpKeyParameter};
use coset::{CborSerializable, CoseKey, Label, RegisteredLabel, RegisteredLabelWithPrivate};

/// Errors from COSE key parsing or signature verification. All are fail-closed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoseError {
    /// The COSE_Key CBOR could not be decoded.
    #[error("malformed COSE_Key CBOR: {reason}")]
    MalformedKey {
        /// Non-secret parse-error description.
        reason: String,
    },

    /// The key type / algorithm / curve combination is not one we support
    /// (only ES256 over P-256 and EdDSA over Ed25519).
    #[error("unsupported COSE key algorithm/curve")]
    UnsupportedAlgorithm,

    /// A required key parameter (x / y coordinate) was missing or malformed.
    #[error("missing or malformed key parameter: {reason}")]
    InvalidKeyParameter {
        /// Non-secret description.
        reason: String,
    },

    /// The signature bytes could not be parsed for the key's algorithm.
    #[error("malformed signature for {alg}")]
    MalformedSignature {
        /// Algorithm label (non-secret).
        alg: &'static str,
    },
}

/// A parsed, supported COSE public key. Holds only the raw public-key material
/// we need to verify a signature; construction validates the algorithm/curve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CosePublicKey {
    /// ES256: ECDSA over NIST P-256 with SHA-256. `sec1` is the SEC1
    /// uncompressed encoding (`0x04 ∥ X ∥ Y`).
    Es256 {
        /// SEC1-uncompressed public point.
        sec1: Vec<u8>,
    },
    /// EdDSA over Ed25519. `public` is the 32-byte compressed public key.
    Ed25519 {
        /// 32-byte Ed25519 public key.
        public: [u8; 32],
    },
}

/// Find a parameter value by its integer label in a COSE_Key `params` list.
fn param(
    params: &[(Label, coset::cbor::value::Value)],
    label: i64,
) -> Option<&coset::cbor::value::Value> {
    params
        .iter()
        .find(|(k, _)| *k == Label::Int(label))
        .map(|(_, v)| v)
}

/// Extract the byte string at `label`, fail-closed.
fn param_bytes(
    params: &[(Label, coset::cbor::value::Value)],
    label: i64,
    name: &str,
) -> Result<Vec<u8>, CoseError> {
    match param(params, label) {
        Some(coset::cbor::value::Value::Bytes(b)) => Ok(b.clone()),
        Some(_) => Err(CoseError::InvalidKeyParameter {
            reason: format!("{name} is not a byte string"),
        }),
        None => Err(CoseError::InvalidKeyParameter {
            reason: format!("{name} missing"),
        }),
    }
}

/// Read the curve (`crv`) parameter as an i64, fail-closed.
fn param_crv(params: &[(Label, coset::cbor::value::Value)], crv_label: i64) -> Option<i64> {
    match param(params, crv_label)? {
        coset::cbor::value::Value::Integer(i) => i128::from(*i).try_into().ok(),
        _ => None,
    }
}

impl CosePublicKey {
    /// Parse a COSE_Key from its CBOR encoding, accepting only ES256/P-256 and
    /// EdDSA/Ed25519. Any other combination is [`CoseError::UnsupportedAlgorithm`].
    pub fn from_cose_key_bytes(bytes: &[u8]) -> Result<Self, CoseError> {
        let key = CoseKey::from_slice(bytes).map_err(|e| CoseError::MalformedKey {
            reason: format!("{e:?}"),
        })?;
        Self::from_cose_key(&key)
    }

    /// Project a parsed [`CoseKey`] into a supported [`CosePublicKey`].
    ///
    /// The `(kty, crv, alg)` triple is matched *together* to defeat
    /// algorithm-confusion: an EC2/P-256 key may only carry `alg` absent or
    /// `ES256`, and an OKP/Ed25519 key may only carry `alg` absent or `EdDSA`.
    /// A cross combination (e.g. EC2/P-256 tagged `alg=EdDSA`, or OKP/Ed25519
    /// tagged `alg=ES256`) is rejected fail-closed rather than silently coerced
    /// to whatever the `kty`/`crv` implies. `alg` is OPTIONAL in COSE_Key, so
    /// absence is permitted and the concrete verifier is derived from
    /// `kty`+`crv`; but a *present* `alg` MUST be consistent with the curve.
    pub fn from_cose_key(key: &CoseKey) -> Result<Self, CoseError> {
        /// Whether a present `alg` matches the expected assigned algorithm.
        /// `None` (absent) is always permitted; any other assigned/text/private
        /// label is a mismatch.
        fn alg_ok(
            alg: &Option<RegisteredLabelWithPrivate<Algorithm>>,
            expected: Algorithm,
        ) -> bool {
            match alg {
                None => true,
                Some(RegisteredLabelWithPrivate::Assigned(a)) => *a == expected,
                Some(_) => false,
            }
        }

        match key.kty {
            RegisteredLabel::Assigned(KeyType::EC2) => {
                let crv = param_crv(&key.params, Ec2KeyParameter::Crv as i64)
                    .ok_or(CoseError::UnsupportedAlgorithm)?;
                if crv != EllipticCurve::P_256 as i64 {
                    return Err(CoseError::UnsupportedAlgorithm);
                }
                // EC2/P-256 binds to ES256: reject a present, non-ES256 alg
                // (incl. EdDSA) before deriving the ES256 verifier.
                if !alg_ok(&key.alg, Algorithm::ES256) {
                    return Err(CoseError::UnsupportedAlgorithm);
                }
                let x = param_bytes(&key.params, Ec2KeyParameter::X as i64, "EC2 x")?;
                let y = param_bytes(&key.params, Ec2KeyParameter::Y as i64, "EC2 y")?;
                if x.len() != 32 || y.len() != 32 {
                    return Err(CoseError::InvalidKeyParameter {
                        reason: "P-256 coordinate is not 32 bytes".to_string(),
                    });
                }
                let mut sec1 = Vec::with_capacity(65);
                sec1.push(0x04); // SEC1 uncompressed point prefix.
                sec1.extend_from_slice(&x);
                sec1.extend_from_slice(&y);
                Ok(CosePublicKey::Es256 { sec1 })
            }
            RegisteredLabel::Assigned(KeyType::OKP) => {
                let crv = param_crv(&key.params, OkpKeyParameter::Crv as i64)
                    .ok_or(CoseError::UnsupportedAlgorithm)?;
                if crv != EllipticCurve::Ed25519 as i64 {
                    return Err(CoseError::UnsupportedAlgorithm);
                }
                // OKP/Ed25519 binds to EdDSA: reject a present, non-EdDSA alg
                // (incl. ES256) before deriving the Ed25519 verifier.
                if !alg_ok(&key.alg, Algorithm::EdDSA) {
                    return Err(CoseError::UnsupportedAlgorithm);
                }
                let x = param_bytes(&key.params, OkpKeyParameter::X as i64, "OKP x")?;
                let public: [u8; 32] =
                    x.as_slice()
                        .try_into()
                        .map_err(|_| CoseError::InvalidKeyParameter {
                            reason: "Ed25519 public key is not 32 bytes".to_string(),
                        })?;
                Ok(CosePublicKey::Ed25519 { public })
            }
            _ => Err(CoseError::UnsupportedAlgorithm),
        }
    }

    /// Verify `signature` over `message`, returning `Ok(())` only on a valid
    /// signature. A wrong signature is [`CoseError`]-free: it returns
    /// `Ok(false)` analog via `Err`? No — we return a `bool` so the caller can
    /// distinguish "verification ran, signature invalid" from "could not run".
    ///
    /// `Ok(true)`  = signature valid.
    /// `Ok(false)` = verification ran, signature INVALID (fail-closed at caller).
    /// `Err(_)`    = the signature could not even be parsed (also fail-closed).
    pub fn verify(&self, signature: &[u8], message: &[u8]) -> Result<bool, CoseError> {
        match self {
            CosePublicKey::Es256 { sec1 } => {
                use p256::ecdsa::signature::Verifier;
                use p256::ecdsa::{Signature, VerifyingKey};
                let vk = VerifyingKey::from_sec1_bytes(sec1).map_err(|_| {
                    CoseError::InvalidKeyParameter {
                        reason: "invalid P-256 public point".to_string(),
                    }
                })?;
                // WebAuthn ES256 signatures are ASN.1 DER-encoded.
                let sig = Signature::from_der(signature)
                    .map_err(|_| CoseError::MalformedSignature { alg: "ES256" })?;
                Ok(vk.verify(message, &sig).is_ok())
            }
            CosePublicKey::Ed25519 { public } => {
                use ed25519_dalek::{Signature, Verifier, VerifyingKey};
                let vk = VerifyingKey::from_bytes(public).map_err(|_| {
                    CoseError::InvalidKeyParameter {
                        reason: "invalid Ed25519 public key".to_string(),
                    }
                })?;
                let sig_bytes: [u8; 64] = signature
                    .try_into()
                    .map_err(|_| CoseError::MalformedSignature { alg: "EdDSA" })?;
                let sig = Signature::from_bytes(&sig_bytes);
                Ok(vk.verify(message, &sig).is_ok())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::iana;
    use coset::{CoseKeyBuilder, KeyType as CosetKeyType};

    #[test]
    fn unsupported_kty_rejected() {
        // A symmetric key (kty=Symmetric) is unsupported.
        let key = CoseKey {
            kty: RegisteredLabel::Assigned(KeyType::Symmetric),
            ..Default::default()
        };
        assert_eq!(
            CosePublicKey::from_cose_key(&key),
            Err(CoseError::UnsupportedAlgorithm)
        );
    }

    #[test]
    fn unsupported_alg_rejected() {
        // EC2 key but alg explicitly ES384 (unsupported).
        let key = CoseKey {
            kty: RegisteredLabel::Assigned(KeyType::EC2),
            alg: Some(RegisteredLabelWithPrivate::Assigned(Algorithm::ES384)),
            ..Default::default()
        };
        assert_eq!(
            CosePublicKey::from_cose_key(&key),
            Err(CoseError::UnsupportedAlgorithm)
        );
    }

    #[test]
    fn ec2_wrong_curve_rejected() {
        let key = CoseKeyBuilder::new_ec2_pub_key(
            iana::EllipticCurve::P_384, // wrong curve for ES256
            vec![1u8; 48],
            vec![2u8; 48],
        )
        .build();
        assert_eq!(
            CosePublicKey::from_cose_key(&key),
            Err(CoseError::UnsupportedAlgorithm)
        );
    }

    #[test]
    fn es256_roundtrip_parse() {
        let key = CoseKeyBuilder::new_ec2_pub_key(
            iana::EllipticCurve::P_256,
            vec![7u8; 32],
            vec![9u8; 32],
        )
        .build();
        let parsed = CosePublicKey::from_cose_key(&key).expect("parse");
        match parsed {
            CosePublicKey::Es256 { sec1 } => {
                assert_eq!(sec1.len(), 65);
                assert_eq!(sec1[0], 0x04);
            }
            _ => panic!("expected ES256"),
        }
    }

    #[test]
    fn ec2_p256_with_eddsa_alg_rejected() {
        // Algorithm confusion: an EC2/P-256 key explicitly tagged alg=EdDSA
        // must NOT be coerced to ES256. Reject fail-closed.
        let key = CoseKeyBuilder::new_ec2_pub_key(
            iana::EllipticCurve::P_256,
            vec![7u8; 32],
            vec![9u8; 32],
        )
        .algorithm(iana::Algorithm::EdDSA)
        .build();
        assert_eq!(
            CosePublicKey::from_cose_key(&key),
            Err(CoseError::UnsupportedAlgorithm)
        );
    }

    #[test]
    fn ec2_p256_with_es256_alg_accepted() {
        // The matching combination still parses.
        let key = CoseKeyBuilder::new_ec2_pub_key(
            iana::EllipticCurve::P_256,
            vec![7u8; 32],
            vec![9u8; 32],
        )
        .algorithm(iana::Algorithm::ES256)
        .build();
        assert!(matches!(
            CosePublicKey::from_cose_key(&key),
            Ok(CosePublicKey::Es256 { .. })
        ));
    }

    #[test]
    fn okp_ed25519_with_es256_alg_rejected() {
        // Algorithm confusion in the other direction: an OKP/Ed25519 key tagged
        // alg=ES256 must NOT be coerced to Ed25519.
        let key = CoseKeyBuilder::new_okp_key()
            .param(
                iana::OkpKeyParameter::Crv as i64,
                coset::cbor::value::Value::from(iana::EllipticCurve::Ed25519 as i64),
            )
            .param(
                iana::OkpKeyParameter::X as i64,
                coset::cbor::value::Value::Bytes(vec![3u8; 32]),
            )
            .algorithm(iana::Algorithm::ES256)
            .build();
        assert_eq!(
            CosePublicKey::from_cose_key(&key),
            Err(CoseError::UnsupportedAlgorithm)
        );
    }

    #[test]
    fn okp_ed25519_with_eddsa_alg_accepted() {
        let key = CoseKeyBuilder::new_okp_key()
            .param(
                iana::OkpKeyParameter::Crv as i64,
                coset::cbor::value::Value::from(iana::EllipticCurve::Ed25519 as i64),
            )
            .param(
                iana::OkpKeyParameter::X as i64,
                coset::cbor::value::Value::Bytes(vec![3u8; 32]),
            )
            .algorithm(iana::Algorithm::EdDSA)
            .build();
        assert!(matches!(
            CosePublicKey::from_cose_key(&key),
            Ok(CosePublicKey::Ed25519 { .. })
        ));
    }

    #[test]
    fn keytype_import_is_used() {
        // Touch the imported alias so the test module compiles cleanly under
        // -D warnings even as helpers evolve.
        let _ = CosetKeyType::Assigned(KeyType::EC2);
    }

    #[test]
    fn from_cose_key_bytes_parses_valid_cbor_and_rejects_invalid() {
        // Valid COSE_Key CBOR (ES256/P-256) round-trips through the byte entry
        // point and yields the same parsed key as the typed path.
        let key = CoseKeyBuilder::new_ec2_pub_key(
            iana::EllipticCurve::P_256,
            vec![7u8; 32],
            vec![9u8; 32],
        )
        .algorithm(iana::Algorithm::ES256)
        .build();
        let bytes = key.clone().to_vec().expect("encode COSE_Key");
        let from_bytes = CosePublicKey::from_cose_key_bytes(&bytes).expect("parse valid CBOR");
        assert_eq!(
            from_bytes,
            CosePublicKey::from_cose_key(&key).expect("typed")
        );
        assert!(matches!(from_bytes, CosePublicKey::Es256 { .. }));

        // Garbage / non-COSE bytes fail closed with MalformedKey rather than
        // panicking or being silently accepted.
        let err = CosePublicKey::from_cose_key_bytes(&[0xff, 0x00, 0x13, 0x37])
            .expect_err("invalid CBOR must be rejected");
        assert!(matches!(err, CoseError::MalformedKey { .. }));

        // Empty input is also malformed, not a default key.
        assert!(matches!(
            CosePublicKey::from_cose_key_bytes(&[]),
            Err(CoseError::MalformedKey { .. })
        ));
    }
}
