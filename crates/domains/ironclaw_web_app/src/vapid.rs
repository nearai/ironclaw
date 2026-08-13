//! VAPID (RFC 8292) key material generation.
//!
//! Generation happens once per deployment (composition seeds it into the
//! channel's credential storage); the *signing* of per-request
//! `Authorization: vapid` headers happens at the host egress credential
//! boundary (`ironclaw_host_runtime`), which parses the same
//! [`VapidCredentialMaterialV1`] schema. This module never signs and never
//! reads stored material back.

use aws_lc_rs::signature::KeyPair as _;
use aws_lc_rs::{rand as aws_rand, signature};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ironclaw_host_api::http::VapidCredentialMaterialV1;

use crate::error::WebAppError;

/// Freshly generated VAPID material: the JSON blob to store as the channel
/// credential, plus the (non-secret) public key the browser needs as
/// `applicationServerKey`.
///
/// `Debug` is hand-written to redact `material_json` — it is the serialized
/// [`VapidCredentialMaterialV1`] and carries the ES256 private key, which the
/// safety boundary forbids from reaching any debug output, log, or panic
/// message. A derived `Debug` would print the key verbatim.
#[derive(Clone)]
pub struct GeneratedVapidKeyMaterial {
    /// Serialized [`VapidCredentialMaterialV1`] — store as the channel
    /// credential under the web-app VAPID handle. Contains the private key.
    pub material_json: String,
    /// Base64url (unpadded) uncompressed P-256 public key (65 bytes).
    pub public_key_b64url: String,
}

impl std::fmt::Debug for GeneratedVapidKeyMaterial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedVapidKeyMaterial")
            .field("material_json", &"<redacted>")
            .field("public_key_b64url", &self.public_key_b64url)
            .finish()
    }
}

/// Generate a fresh P-256 keypair and wrap it in the credential-material
/// schema the host egress VAPID injector consumes.
///
/// `subject` is the RFC 8292 `sub` claim — a `mailto:` or `https:` URI
/// identifying the operator to push services.
pub fn generate_vapid_key_material(
    subject: &str,
) -> Result<GeneratedVapidKeyMaterial, WebAppError> {
    validate_vapid_subject(subject)?;
    let rng = aws_rand::SystemRandom::new();
    let document =
        signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| WebAppError::crypto("VAPID keypair generation failed"))?;
    let key_pair = signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        document.as_ref(),
    )
    .map_err(|_| WebAppError::crypto("generated VAPID keypair failed to parse"))?;
    let public_key = key_pair.public_key().as_ref().to_vec();
    if public_key.len() != 65 || public_key[0] != 0x04 {
        return Err(WebAppError::crypto(
            "VAPID public key is not an uncompressed P-256 point",
        ));
    }
    let public_key_b64url = URL_SAFE_NO_PAD.encode(&public_key);
    let material = VapidCredentialMaterialV1 {
        es256_private_key_pkcs8_b64url: URL_SAFE_NO_PAD.encode(document.as_ref()),
        public_key_b64url: public_key_b64url.clone(),
        subject: subject.to_string(),
    };
    let material_json = serde_json::to_string(&material)
        .map_err(|error| WebAppError::crypto(format!("VAPID material serialization: {error}")))?;
    Ok(GeneratedVapidKeyMaterial {
        material_json,
        public_key_b64url,
    })
}

/// RFC 8292 §2.1: the subject is a contact URI for the application server —
/// `mailto:` or `https:`. Parsed with `url` (a crate dependency already) so a
/// malformed URI is rejected at generation rather than surfacing later as a
/// push-service rejection on every delivery.
pub fn validate_vapid_subject(subject: &str) -> Result<(), WebAppError> {
    if subject.len() > 256 || subject.chars().any(char::is_control) {
        return Err(WebAppError::InvalidScope {
            reason: "VAPID subject must be a short control-free URI".to_string(),
        });
    }
    let parsed = url::Url::parse(subject).map_err(|_| WebAppError::InvalidScope {
        reason: "VAPID subject must be a valid mailto: or https: URI".to_string(),
    })?;
    let valid = match parsed.scheme() {
        "mailto" => !parsed.path().is_empty(),
        "https" => parsed.host_str().is_some(),
        _ => false,
    };
    if !valid {
        return Err(WebAppError::InvalidScope {
            reason: "VAPID subject must be a mailto: address or https: URL".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_material_parses_and_round_trips_public_key() {
        let generated =
            generate_vapid_key_material("mailto:ops@example.com").expect("generation succeeds");
        let material: VapidCredentialMaterialV1 =
            serde_json::from_str(&generated.material_json).expect("material JSON parses");
        assert_eq!(material.public_key_b64url, generated.public_key_b64url);
        assert_eq!(material.subject, "mailto:ops@example.com");

        // The stored private key must re-parse and its public key must match.
        let der = URL_SAFE_NO_PAD
            .decode(&material.es256_private_key_pkcs8_b64url)
            .expect("private key base64");
        let key_pair =
            signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &der)
                .expect("private key parses");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(key_pair.public_key().as_ref()),
            generated.public_key_b64url
        );
    }

    #[test]
    fn distinct_generations_produce_distinct_keys() {
        let first = generate_vapid_key_material("mailto:a@example.com").expect("first");
        let second = generate_vapid_key_material("mailto:a@example.com").expect("second");
        assert_ne!(first.public_key_b64url, second.public_key_b64url);
    }

    #[test]
    fn subjects_are_validated() {
        assert!(validate_vapid_subject("mailto:ops@example.com").is_ok());
        assert!(validate_vapid_subject("https://ironclaw.example").is_ok());
        for bad in ["", "mailto:", "https://", "http://x", "ops@example.com"] {
            assert!(validate_vapid_subject(bad).is_err(), "{bad:?} must fail");
        }
    }
}
