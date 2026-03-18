//! Shared relay webhook signature verification helpers.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify a relay callback HMAC signature.
pub fn verify_relay_signature(
    secret: &[u8],
    timestamp: &str,
    body: &[u8],
    signature: &str,
) -> bool {
    verify_signature(secret, timestamp, body, signature)
}

fn verify_signature(secret: &[u8], timestamp: &str, body: &[u8], signature: &str) -> bool {
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_signature(secret: &[u8], timestamp: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn verify_valid_signature() {
        let secret = b"test-secret";
        let body = b"hello";
        let ts = "1234567890";
        let sig = make_signature(secret, ts, body);
        assert!(verify_signature(secret, ts, body, &sig));
    }

    #[test]
    fn verify_wrong_secret_fails() {
        let body = b"hello";
        let ts = "1234567890";
        let sig = make_signature(b"correct", ts, body);
        assert!(!verify_signature(b"wrong", ts, body, &sig));
    }

    #[test]
    fn verify_tampered_body_fails() {
        let secret = b"secret";
        let ts = "1234567890";
        let sig = make_signature(secret, ts, b"original");
        assert!(!verify_signature(secret, ts, b"tampered", &sig));
    }
}
