use hmac::{Hmac, Mac};
use ironclaw_product_workflow::{IronhubInstallDeliveryRequest, IronhubRegisterRequest};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const MIN_SHARED_KEY_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum IronhubSharedKeyError {
    #[error("ironhub shared key must be at least {min} bytes")]
    TooShort { min: usize },
}

#[derive(Clone)]
pub struct IronhubSharedKey(String);

impl IronhubSharedKey {
    pub fn new(value: impl Into<String>) -> Result<Self, IronhubSharedKeyError> {
        let value = value.into();
        if value.len() < MIN_SHARED_KEY_LEN {
            return Err(IronhubSharedKeyError::TooShort {
                min: MIN_SHARED_KEY_LEN,
            });
        }
        Ok(Self(value))
    }
}

impl std::fmt::Debug for IronhubSharedKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("IronhubSharedKey(redacted)")
    }
}

pub(super) fn register_payload(request: &IronhubRegisterRequest) -> String {
    format!(
        "register:{}:{}:{}:{}",
        request.uid, request.aid, request.ts, request.nonce
    )
}

// Length-prefix every field so the encoding stays injective regardless of
// field content; the artifact digest and manifest URL both contain colons.
pub(super) fn install_payload(request: &IronhubInstallDeliveryRequest) -> String {
    let ts = request.ts.to_string();
    let fields = [
        request.slug.as_str(),
        request.version.as_str(),
        request.uid.as_str(),
        request.aid.as_str(),
        ts.as_str(),
        request.nonce.as_str(),
        request.artifact_digest.as_str(),
        request.private_manifest_url.as_deref().unwrap_or(""),
    ];
    let mut payload = String::from("install");
    for field in fields {
        payload.push(':');
        payload.push_str(&field.len().to_string());
        payload.push(':');
        payload.push_str(field);
    }
    payload
}

pub(super) fn verify_signature(
    shared_key: &IronhubSharedKey,
    payload: &str,
    sig_hex: &str,
) -> bool {
    let Ok(expected) = hex::decode(sig_hex) else {
        // silent-ok: a non-hex signature is an invalid signature; reject it.
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(shared_key.0.as_bytes()) else {
        // silent-ok: HMAC-SHA256 accepts any key length, so this arm never fires.
        return false;
    };
    mac.update(payload.as_bytes());
    mac.verify_slice(&expected).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARED_KEY: &str = "ihub_sk_E2ETestSharedKey0000000000000000000000000";
    const REGISTER_SIG: &str = "7e69b8cd66138589a2ae1320d6ba894870462efeb295acf80336ddd00e0953b5";
    const INSTALL_SIG: &str = "d1b7519d96c098b84554ac8c5be9838ccd979249ea892378105ab0febe9b0472";

    fn shared_key() -> IronhubSharedKey {
        IronhubSharedKey::new(SHARED_KEY).expect("shared key")
    }

    fn register_request() -> IronhubRegisterRequest {
        IronhubRegisterRequest {
            uid: "user-1".to_string(),
            aid: "aid-1".to_string(),
            ts: 1_700_000_000,
            nonce: "nonce-abc".to_string(),
            sig: String::new(),
        }
    }

    fn install_request() -> IronhubInstallDeliveryRequest {
        IronhubInstallDeliveryRequest {
            slug: "my-skill".to_string(),
            version: "1.0.0".to_string(),
            uid: "user-1".to_string(),
            aid: "aid-1".to_string(),
            ts: 1_700_000_000,
            nonce: "nonce-abc".to_string(),
            artifact_digest: "sha256:deadbeef".to_string(),
            sig: String::new(),
            private_manifest_url: None,
        }
    }

    #[test]
    fn register_payload_matches_hub_format() {
        assert_eq!(
            register_payload(&register_request()),
            "register:user-1:aid-1:1700000000:nonce-abc"
        );
    }

    #[test]
    fn install_payload_matches_hub_format() {
        assert_eq!(
            install_payload(&install_request()),
            "install:8:my-skill:5:1.0.0:6:user-1:5:aid-1:10:1700000000:9:nonce-abc:15:sha256:deadbeef:0:"
        );
    }

    #[test]
    fn install_payload_covers_private_manifest_url() {
        let mut request = install_request();
        request.private_manifest_url =
            Some("https://hub.example/api/private-artifacts/manifest/tok".to_string());
        assert_eq!(
            install_payload(&request),
            "install:8:my-skill:5:1.0.0:6:user-1:5:aid-1:10:1700000000:9:nonce-abc:15:sha256:deadbeef:54:https://hub.example/api/private-artifacts/manifest/tok"
        );
    }

    #[test]
    fn verifies_hub_register_signature() {
        assert!(verify_signature(
            &shared_key(),
            &register_payload(&register_request()),
            REGISTER_SIG
        ));
    }

    #[test]
    fn verifies_hub_install_signature() {
        assert!(verify_signature(
            &shared_key(),
            &install_payload(&install_request()),
            INSTALL_SIG
        ));
    }

    #[test]
    fn install_signature_breaks_when_private_manifest_url_is_tampered() {
        let mut request = install_request();
        request.private_manifest_url = Some("https://evil.example/manifest".to_string());
        assert!(!verify_signature(
            &shared_key(),
            &install_payload(&request),
            INSTALL_SIG
        ));
    }

    #[test]
    fn rejects_tampered_signature() {
        let tampered = format!("00{}", &REGISTER_SIG[2..]);
        assert!(!verify_signature(
            &shared_key(),
            &register_payload(&register_request()),
            &tampered
        ));
    }

    #[test]
    fn rejects_wrong_shared_key() {
        let wrong = IronhubSharedKey::new("ihub_sk_wrong0000000000000000000").expect("shared key");
        assert!(!verify_signature(
            &wrong,
            &register_payload(&register_request()),
            REGISTER_SIG
        ));
    }

    #[test]
    fn rejects_non_hex_signature() {
        assert!(!verify_signature(
            &shared_key(),
            &register_payload(&register_request()),
            "zzzz"
        ));
    }
}
