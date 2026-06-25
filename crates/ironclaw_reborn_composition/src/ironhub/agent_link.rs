use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

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
}

impl InstallDelivery<'_> {
    pub(super) fn payload(&self) -> String {
        format!(
            "install:{}:{}:{}:{}:{}:{}:{}",
            self.slug, self.version, self.uid, self.aid, self.ts, self.nonce, self.artifact_digest
        )
    }
}

pub(super) fn verify_signature(shared_key: &str, payload: &str, sig_hex: &str) -> bool {
    let Ok(expected) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(shared_key.as_bytes()) else {
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
        }
    }

    #[test]
    fn register_payload_matches_hub_format() {
        assert_eq!(
            register().payload(),
            "register:user-1:aid-1:1700000000:nonce-abc"
        );
    }

    #[test]
    fn install_payload_matches_hub_format() {
        assert_eq!(
            install().payload(),
            "install:my-skill:1.0.0:user-1:aid-1:1700000000:nonce-abc:sha256:deadbeef"
        );
    }

    #[test]
    fn verifies_hub_register_signature() {
        assert!(verify_signature(
            SHARED_KEY,
            &register().payload(),
            REGISTER_SIG
        ));
    }

    #[test]
    fn verifies_hub_install_signature() {
        assert!(verify_signature(
            SHARED_KEY,
            &install().payload(),
            INSTALL_SIG
        ));
    }

    #[test]
    fn rejects_tampered_signature() {
        let tampered = format!("00{}", &REGISTER_SIG[2..]);
        assert!(!verify_signature(
            SHARED_KEY,
            &register().payload(),
            &tampered
        ));
    }

    #[test]
    fn rejects_wrong_shared_key() {
        assert!(!verify_signature(
            "ihub_sk_wrong",
            &register().payload(),
            REGISTER_SIG
        ));
    }

    #[test]
    fn rejects_non_hex_signature() {
        assert!(!verify_signature(SHARED_KEY, &register().payload(), "zzzz"));
    }
}
