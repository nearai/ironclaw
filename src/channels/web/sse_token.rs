//! Short-lived SSE token generation and validation.
//!
//! The browser `EventSource` API does not support custom HTTP headers, forcing
//! the gateway auth token into URL query parameters where it leaks into proxy
//! access logs, browser history, and HTTP `Referer` headers.
//!
//! This module issues short-lived HMAC-SHA256 tokens derived from the gateway
//! token. The main token never appears in an SSE URL; instead clients POST to
//! `/api/auth/sse-token` (with the bearer header) to obtain an ephemeral token
//! that is valid for ~120 seconds.
//!
//! ## Token format
//!
//! ```text
//! HMAC-SHA256(gateway_token, "sse:" || floor(unix_secs / 120))
//! ```
//!
//! Validation accepts both the current and previous time windows to handle
//! clock-boundary races gracefully.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// SSE token validity window in seconds.
pub const SSE_TOKEN_LIFETIME_SECS: u64 = 120;

type HmacSha256 = Hmac<Sha256>;

/// Generate a short-lived SSE token for the current time window.
///
/// The token is the hex-encoded HMAC-SHA256 of `"sse:<window>"` keyed on the
/// gateway auth token.
pub fn generate_sse_token(gateway_token: &str) -> String {
    let now = current_epoch_secs();
    let window = now / SSE_TOKEN_LIFETIME_SECS;
    compute_hmac_hex(gateway_token, window)
}

/// Validate an SSE token against the gateway auth token.
///
/// Returns `true` if the token matches either the current or immediately
/// preceding time window (to handle requests made right at a window boundary).
pub fn validate_sse_token(gateway_token: &str, candidate: &str) -> bool {
    let now = current_epoch_secs();
    let current_window = now / SSE_TOKEN_LIFETIME_SECS;

    // Check current window
    let expected_current = compute_hmac_hex(gateway_token, current_window);
    if bool::from(candidate.as_bytes().ct_eq(expected_current.as_bytes())) {
        return true;
    }

    // Check previous window (clock boundary grace)
    if current_window > 0 {
        let expected_prev = compute_hmac_hex(gateway_token, current_window - 1);
        if bool::from(candidate.as_bytes().ct_eq(expected_prev.as_bytes())) {
            return true;
        }
    }

    false
}

/// Compute `hex(HMAC-SHA256(key, "sse:<window>"))`.
///
/// Returns an empty string if the HMAC cannot be initialised (should never
/// happen for HMAC-SHA256 which accepts any key length).
fn compute_hmac_hex(key: &str, window: u64) -> String {
    let message = format!("sse:{}", window);
    let Ok(mut mac) = HmacSha256::new_from_slice(key.as_bytes()) else {
        // HMAC-SHA256 accepts any key length, so this branch is unreachable.
        return String::new();
    };
    mac.update(message.as_bytes());
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

fn current_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_returns_hex_string() {
        let token = generate_sse_token("test-gateway-token");
        // HMAC-SHA256 produces 32 bytes = 64 hex chars
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_validate_accepts_current_token() {
        let gateway_token = "my-secret-gateway-token";
        let sse_token = generate_sse_token(gateway_token);
        assert!(validate_sse_token(gateway_token, &sse_token));
    }

    #[test]
    fn test_validate_rejects_wrong_token() {
        let gateway_token = "my-secret-gateway-token";
        assert!(!validate_sse_token(gateway_token, "deadbeef"));
    }

    #[test]
    fn test_validate_rejects_different_gateway_token() {
        let sse_token = generate_sse_token("token-a");
        assert!(!validate_sse_token("token-b", &sse_token));
    }

    #[test]
    fn test_validate_rejects_empty_candidate() {
        assert!(!validate_sse_token("any-token", ""));
    }

    #[test]
    fn test_compute_hmac_hex_deterministic() {
        let a = compute_hmac_hex("key", 42);
        let b = compute_hmac_hex("key", 42);
        assert_eq!(a, b);
    }

    #[test]
    fn test_compute_hmac_hex_different_windows() {
        let a = compute_hmac_hex("key", 1);
        let b = compute_hmac_hex("key", 2);
        assert_ne!(a, b);
    }

    #[test]
    fn test_sse_token_lifetime_is_120() {
        assert_eq!(SSE_TOKEN_LIFETIME_SECS, 120);
    }
}
