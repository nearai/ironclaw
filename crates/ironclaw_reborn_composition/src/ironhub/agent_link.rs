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

    pub(super) fn as_str(&self) -> &str {
        &self.0
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

pub(super) fn install_payload(request: &IronhubInstallDeliveryRequest) -> String {
    format!(
        "install:{}:{}:{}:{}:{}:{}:{}",
        request.slug,
        request.version,
        request.uid,
        request.aid,
        request.ts,
        request.nonce,
        request.artifact_digest
    )
}

pub(super) fn verify_signature(shared_key: &str, payload: &str, sig_hex: &str) -> bool {
    let Ok(expected) = hex::decode(sig_hex) else {
        // silent-ok: a non-hex signature is an invalid signature; reject it.
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(shared_key.as_bytes()) else {
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
    const INSTALL_SIG: &str = "f00a213648f926e263d25b02b5fcd2731d371355a5728c3c07a3c7eb817be2ea";

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
            kind: None,
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
            "install:my-skill:1.0.0:user-1:aid-1:1700000000:nonce-abc:sha256:deadbeef"
        );
    }

    #[test]
    fn verifies_hub_register_signature() {
        assert!(verify_signature(
            SHARED_KEY,
            &register_payload(&register_request()),
            REGISTER_SIG
        ));
    }

    #[test]
    fn verifies_hub_install_signature() {
        assert!(verify_signature(
            SHARED_KEY,
            &install_payload(&install_request()),
            INSTALL_SIG
        ));
    }

    #[test]
    fn rejects_tampered_signature() {
        let tampered = format!("00{}", &REGISTER_SIG[2..]);
        assert!(!verify_signature(
            SHARED_KEY,
            &register_payload(&register_request()),
            &tampered
        ));
    }

    #[test]
    fn rejects_wrong_shared_key() {
        assert!(!verify_signature(
            "ihub_sk_wrong",
            &register_payload(&register_request()),
            REGISTER_SIG
        ));
    }

    #[test]
    fn rejects_non_hex_signature() {
        assert!(!verify_signature(
            SHARED_KEY,
            &register_payload(&register_request()),
            "zzzz"
        ));
    }
}
