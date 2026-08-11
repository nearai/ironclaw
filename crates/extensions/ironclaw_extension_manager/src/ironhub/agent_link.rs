use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const MIN_SHARED_KEY_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IronhubSharedKeyError {
    #[error("IronHub shared key must be at least {min} bytes")]
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

pub(super) struct RegisterChallenge<'a> {
    pub uid: &'a str,
    pub aid: &'a str,
    pub ts: u64,
    pub nonce: &'a str,
}

impl RegisterChallenge<'_> {
    pub(super) fn payload(&self) -> String {
        format!(
            "register:{}:{}:{}:{}",
            self.uid, self.aid, self.ts, self.nonce
        )
    }
}

pub(super) struct InstallDelivery<'a> {
    pub slug: &'a str,
    pub version: &'a str,
    pub uid: &'a str,
    pub aid: &'a str,
    pub ts: u64,
    pub nonce: &'a str,
    pub artifact_digest: &'a str,
    pub private_manifest_url: Option<&'a str>,
}

impl InstallDelivery<'_> {
    pub(super) fn payload(&self) -> String {
        let ts = self.ts.to_string();
        let fields = [
            self.slug,
            self.version,
            self.uid,
            self.aid,
            ts.as_str(),
            self.nonce,
            self.artifact_digest,
            self.private_manifest_url.unwrap_or(""),
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
}

pub(super) fn verify_signature(
    shared_key: &IronhubSharedKey,
    payload: &str,
    signature_hex: &str,
) -> bool {
    let Ok(signature) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(shared_key.0.as_bytes()) else {
        return false;
    };
    mac.update(payload.as_bytes());
    mac.verify_slice(&signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARED_KEY: &str = "ihub_sk_E2ETestSharedKey0000000000000000000000000";
    const REGISTER_SIG: &str = "7e69b8cd66138589a2ae1320d6ba894870462efeb295acf80336ddd00e0953b5";
    const INSTALL_SIG: &str = "d1b7519d96c098b84554ac8c5be9838ccd979249ea892378105ab0febe9b0472";

    fn shared_key() -> IronhubSharedKey {
        IronhubSharedKey::new(SHARED_KEY).expect("test shared key")
    }

    fn register() -> RegisterChallenge<'static> {
        RegisterChallenge {
            uid: "user-1",
            aid: "aid-1",
            ts: 1_700_000_000,
            nonce: "nonce-abc",
        }
    }

    fn install() -> InstallDelivery<'static> {
        InstallDelivery {
            slug: "my-skill",
            version: "1.0.0",
            uid: "user-1",
            aid: "aid-1",
            ts: 1_700_000_000,
            nonce: "nonce-abc",
            artifact_digest: "sha256:deadbeef",
            private_manifest_url: None,
        }
    }

    #[test]
    fn shared_key_rejects_short_values_and_redacts_debug() {
        assert!(matches!(
            IronhubSharedKey::new("x".repeat(MIN_SHARED_KEY_LEN - 1)),
            Err(IronhubSharedKeyError::TooShort { min: 32 })
        ));
        assert!(IronhubSharedKey::new("x".repeat(MIN_SHARED_KEY_LEN)).is_ok());
        assert_eq!(format!("{:?}", shared_key()), "IronhubSharedKey(redacted)");
    }

    #[test]
    fn register_payload_matches_hub_format() {
        assert_eq!(
            register().payload(),
            "register:user-1:aid-1:1700000000:nonce-abc"
        );
    }

    #[test]
    fn install_payload_is_injective_and_covers_private_manifest_url() {
        assert_eq!(
            install().payload(),
            "install:8:my-skill:5:1.0.0:6:user-1:5:aid-1:10:1700000000:9:nonce-abc:15:sha256:deadbeef:0:"
        );
        let mut private = install();
        private.private_manifest_url =
            Some("https://hub.example/api/private-artifacts/manifest/tok");
        assert_ne!(private.payload(), install().payload());
    }

    #[test]
    fn verifies_known_hub_signatures() {
        assert!(verify_signature(
            &shared_key(),
            &register().payload(),
            REGISTER_SIG
        ));
        assert!(verify_signature(
            &shared_key(),
            &install().payload(),
            INSTALL_SIG
        ));
    }

    #[test]
    fn rejects_bad_signature_without_string_comparison() {
        assert!(!verify_signature(
            &shared_key(),
            &register().payload(),
            "00"
        ));
        assert!(!verify_signature(
            &shared_key(),
            &register().payload(),
            "not-hex"
        ));
    }
}
