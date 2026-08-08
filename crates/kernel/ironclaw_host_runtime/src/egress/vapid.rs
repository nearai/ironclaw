//! RFC 8292 `Authorization: vapid` computation for the
//! `RuntimeCredentialTarget::VapidAuthorization` injection kind (Web Push).
//!
//! Runs at the same credential-injection chokepoint as header/query/path
//! injection: the resolved secret material is a serialized
//! [`VapidCredentialMaterialV1`], and the signed token's audience is derived
//! from the request URL actually being sent, so a token can never authorize
//! a different push service than the one this request targets. Adapters
//! never see key bytes — they only name the credential handle.

use aws_lc_rs::rand as aws_rand;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ironclaw_host_api::http::VapidCredentialMaterialV1;

/// RFC 8292 caps token lifetime at 24 hours; half that keeps clock-skew
/// headroom while staying reusable across a retry burst.
pub(super) const VAPID_TOKEN_TTL_SECONDS: u64 = 12 * 60 * 60;

/// Sanitized failure reasons — never carry material or the request URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VapidHeaderError {
    MaterialInvalid,
    AudienceInvalid,
    SigningFailed,
}

impl VapidHeaderError {
    pub(super) fn sanitized_reason(self) -> &'static str {
        match self {
            Self::MaterialInvalid => "VAPID credential material is invalid",
            Self::AudienceInvalid => "VAPID audience could not be derived from the request",
            Self::SigningFailed => "VAPID token signing failed",
        }
    }
}

/// Compute the `Authorization: vapid t=<jwt>, k=<public>` header value for a
/// request to `request_url`, signed with the deployment's stored material.
pub(super) fn vapid_authorization_header(
    material_json: &str,
    request_url: &url::Url,
    now_unix_seconds: u64,
) -> Result<String, VapidHeaderError> {
    let material: VapidCredentialMaterialV1 =
        serde_json::from_str(material_json).map_err(|_| VapidHeaderError::MaterialInvalid)?;
    // Reject a structurally corrupt blob before it can produce a token no push
    // service will accept (base64url shape, P-256 point, subject grammar).
    material
        .validate_shape()
        .map_err(|_| VapidHeaderError::MaterialInvalid)?;

    // Push service audiences are https origins; the outer injection gate has
    // already required TLS (or literal loopback, which never hosts a push
    // service), so anything non-https fails closed here too.
    if request_url.scheme() != "https" {
        return Err(VapidHeaderError::AudienceInvalid);
    }
    let host = request_url
        .host_str()
        .ok_or(VapidHeaderError::AudienceInvalid)?;
    let audience = match request_url.port() {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
    };

    // Sanity-check the public half so a corrupted blob fails before any
    // request carries a token the push service cannot attribute.
    let public_key = URL_SAFE_NO_PAD
        .decode(&material.public_key_b64url)
        .map_err(|_| VapidHeaderError::MaterialInvalid)?;
    if public_key.len() != 65 || public_key[0] != 0x04 {
        return Err(VapidHeaderError::MaterialInvalid);
    }

    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = serde_json::json!({
        "aud": audience,
        "exp": now_unix_seconds + VAPID_TOKEN_TTL_SECONDS,
        "sub": material.subject,
    });
    let payload = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).map_err(|_| VapidHeaderError::SigningFailed)?);
    let signing_input = format!("{header}.{payload}");

    let private_key_der = URL_SAFE_NO_PAD
        .decode(&material.es256_private_key_pkcs8_b64url)
        .map_err(|_| VapidHeaderError::MaterialInvalid)?;
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &private_key_der)
        .map_err(|_| VapidHeaderError::MaterialInvalid)?;
    let rng = aws_rand::SystemRandom::new();
    let signature = key_pair
        .sign(&rng, signing_input.as_bytes())
        .map_err(|_| VapidHeaderError::SigningFailed)?;

    Ok(format!(
        "vapid t={signing_input}.{}, k={}",
        URL_SAFE_NO_PAD.encode(signature.as_ref()),
        material.public_key_b64url
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, KeyPair as _, UnparsedPublicKey};

    fn generated_material(subject: &str) -> (String, Vec<u8>) {
        let rng = aws_rand::SystemRandom::new();
        let document =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).expect("generate");
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref())
                .expect("parse");
        let public_key = key_pair.public_key().as_ref().to_vec();
        let material = VapidCredentialMaterialV1 {
            es256_private_key_pkcs8_b64url: URL_SAFE_NO_PAD.encode(document.as_ref()),
            public_key_b64url: URL_SAFE_NO_PAD.encode(&public_key),
            subject: subject.to_string(),
        };
        (
            serde_json::to_string(&material).expect("material serializes"),
            public_key,
        )
    }

    #[test]
    fn header_carries_a_verifiable_jwt_scoped_to_the_push_origin() {
        let (material_json, public_key) = generated_material("mailto:ops@example.com");
        let request_url =
            url::Url::parse("https://fcm.googleapis.com/fcm/send/token123").expect("url");
        let now = 1_754_000_000u64;

        let header = vapid_authorization_header(&material_json, &request_url, now).expect("header");
        let token_section = header
            .strip_prefix("vapid t=")
            .expect("vapid scheme prefix");
        let (jwt, key_section) = token_section.split_once(", k=").expect("k parameter");
        assert_eq!(key_section, URL_SAFE_NO_PAD.encode(&public_key));

        let mut segments = jwt.split('.');
        let header_b64 = segments.next().expect("jwt header");
        let payload_b64 = segments.next().expect("jwt payload");
        let signature_b64 = segments.next().expect("jwt signature");
        assert!(segments.next().is_none(), "compact JWS has three segments");

        let decoded_header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header_b64).expect("header decodes"))
                .expect("header json");
        assert_eq!(decoded_header["alg"], "ES256");

        let claims: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(payload_b64)
                .expect("payload decodes"),
        )
        .expect("claims json");
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], "mailto:ops@example.com");
        assert_eq!(claims["exp"], now + VAPID_TOKEN_TTL_SECONDS);

        let verifier = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public_key);
        let signing_input = format!("{header_b64}.{payload_b64}");
        verifier
            .verify(
                signing_input.as_bytes(),
                &URL_SAFE_NO_PAD
                    .decode(signature_b64)
                    .expect("signature decodes"),
            )
            .expect("ES256 signature verifies against the advertised public key");
    }

    #[test]
    fn malformed_material_and_non_https_audiences_fail_closed() {
        let (material_json, _) = generated_material("mailto:ops@example.com");
        let https_url = url::Url::parse("https://fcm.googleapis.com/send/x").expect("url");
        assert_eq!(
            vapid_authorization_header("not json", &https_url, 0),
            Err(VapidHeaderError::MaterialInvalid)
        );
        let truncated = {
            let mut material: VapidCredentialMaterialV1 =
                serde_json::from_str(&material_json).expect("parse");
            material.public_key_b64url = URL_SAFE_NO_PAD.encode([0u8; 10]);
            serde_json::to_string(&material).expect("serialize")
        };
        assert_eq!(
            vapid_authorization_header(&truncated, &https_url, 0),
            Err(VapidHeaderError::MaterialInvalid)
        );
        let http_url = url::Url::parse("http://127.0.0.1/send/x").expect("url");
        assert_eq!(
            vapid_authorization_header(&material_json, &http_url, 0),
            Err(VapidHeaderError::AudienceInvalid)
        );
    }
}
