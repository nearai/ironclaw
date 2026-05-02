//! Slack request signature verification.
//!
//! Slack signs every Events / Slash Command / Interactivity request with the
//! app-level signing secret so the receiver can prove the request really
//! came from Slack. The signing secret is app-scoped, not workspace-scoped:
//! one app installed in multiple workspaces shares one signing secret. Bot
//! tokens differ per workspace; the signing secret does not.
//!
//! ## Wire format
//!
//! Slack sends two headers alongside every request:
//!
//!   * `X-Slack-Request-Timestamp: 1531420618` — Unix seconds when Slack
//!     dispatched the request.
//!   * `X-Slack-Signature: v0=a2114d57b48e...` — HMAC-SHA256 of the
//!     concatenated string `v0:<timestamp>:<raw body>`, hex-encoded.
//!
//! IronClaw rejects requests whose timestamp drifts more than five minutes
//! from now (the canonical window in Slack's docs); without it a captured
//! request can be replayed indefinitely.
//!
//! ## Why this lives in `channels::slack` and not in the WASM channel
//!
//! The Slack WASM channel handles inbound events at `/webhook/slack`; slash
//! commands and interactivity hit core IronClaw routes (`/api/channels/slack/slash`,
//! `/api/channels/slack/interactivity`) registered in
//! `src/channels/web/platform/router.rs`. Both surfaces verify against the
//! same signing secret, so the verifier is shared rather than re-implemented
//! per handler.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Maximum allowed clock skew between Slack's server and IronClaw, in
/// seconds. Slack's docs use 5 minutes; we honour that. Below this window
/// we treat the timestamp as live; above, the request is rejected as a
/// possible replay.
pub const MAX_TIMESTAMP_SKEW_SECS: i64 = 5 * 60;

/// Errors the verifier can return. Distinct variants so callers can map to
/// the right HTTP status (`401` for `Mismatch`, `400` for malformed
/// inputs, `408`-ish for `Stale`).
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("missing or empty X-Slack-Request-Timestamp header")]
    MissingTimestamp,
    #[error("X-Slack-Request-Timestamp is not a valid Unix timestamp: {0}")]
    InvalidTimestamp(String),
    #[error(
        "request timestamp is more than {MAX_TIMESTAMP_SKEW_SECS}s away from now (replay window)"
    )]
    Stale,
    #[error("missing or empty X-Slack-Signature header")]
    MissingSignature,
    #[error("X-Slack-Signature missing required `v0=` prefix")]
    BadSignaturePrefix,
    #[error("X-Slack-Signature is not valid hex: {0}")]
    BadSignatureHex(String),
    #[error("signature mismatch — request was not sent by Slack")]
    Mismatch,
}

/// Inputs to a verification call. Borrowed so callers don't have to clone
/// big request bodies.
#[derive(Debug, Clone, Copy)]
pub struct VerifyInputs<'a> {
    /// `X-Slack-Request-Timestamp` header, exactly as Slack sent it.
    pub timestamp_header: &'a str,
    /// `X-Slack-Signature` header, exactly as Slack sent it.
    pub signature_header: &'a str,
    /// Raw request body bytes — never the parsed/decoded form. Slack
    /// signs the bytes that hit the wire; any normalisation invalidates
    /// the HMAC.
    pub body: &'a [u8],
    /// App-level signing secret, fetched from secrets storage.
    pub signing_secret: &'a [u8],
    /// Current Unix time in seconds. Injected so tests can pin the clock.
    pub now_secs: i64,
}

/// Verify a Slack request signature. Returns `Ok(())` on success.
///
/// Constant-time-compares the recomputed HMAC to the header value via
/// `subtle::ConstantTimeEq` so a network-observable timing leak doesn't
/// help an attacker brute-force the signing secret.
pub fn verify(inputs: VerifyInputs<'_>) -> Result<(), SignatureError> {
    if inputs.timestamp_header.is_empty() {
        return Err(SignatureError::MissingTimestamp);
    }
    let ts: i64 = inputs
        .timestamp_header
        .parse()
        .map_err(|e: std::num::ParseIntError| SignatureError::InvalidTimestamp(e.to_string()))?;
    if (inputs.now_secs - ts).abs() > MAX_TIMESTAMP_SKEW_SECS {
        return Err(SignatureError::Stale);
    }

    if inputs.signature_header.is_empty() {
        return Err(SignatureError::MissingSignature);
    }
    let hex_sig = inputs
        .signature_header
        .strip_prefix("v0=")
        .ok_or(SignatureError::BadSignaturePrefix)?;
    let provided =
        hex::decode(hex_sig).map_err(|e| SignatureError::BadSignatureHex(e.to_string()))?;

    // Slack's basestring is exactly `v0:<ts>:<raw body>`.
    // `new_from_slice` only errors for empty keys, which means the operator
    // never configured the signing secret — surface as Mismatch so we don't
    // crash the gateway on misconfiguration.
    let mut mac =
        HmacSha256::new_from_slice(inputs.signing_secret).map_err(|_| SignatureError::Mismatch)?;
    mac.update(b"v0:");
    mac.update(inputs.timestamp_header.as_bytes());
    mac.update(b":");
    mac.update(inputs.body);
    let expected = mac.finalize().into_bytes();

    if expected.ct_eq(&provided).into() {
        Ok(())
    } else {
        Err(SignatureError::Mismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    /// Helper: produce the `v0=<hex>` header Slack would send, given the
    /// same inputs the receiver will recompute over.
    fn sign(secret: &[u8], ts: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(b"v0:");
        mac.update(ts.as_bytes());
        mac.update(b":");
        mac.update(body);
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    const SECRET: &[u8] = b"8f742231b10e8888abcd99yyyzz85a5";
    const TS: &str = "1531420618";
    const BODY: &[u8] = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J";

    #[test]
    fn accepts_valid_signature() {
        let sig = sign(SECRET, TS, BODY);
        let now: i64 = TS.parse().unwrap();
        verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: &sig,
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .expect("valid signature should verify");
    }

    #[test]
    fn rejects_mismatched_signature() {
        let mut sig = sign(SECRET, TS, BODY);
        // Flip the last hex digit to corrupt the HMAC.
        let last = sig.pop().unwrap();
        sig.push(if last == 'a' { 'b' } else { 'a' });
        let now: i64 = TS.parse().unwrap();
        let err = verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: &sig,
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::Mismatch));
    }

    #[test]
    fn rejects_stale_timestamp() {
        let sig = sign(SECRET, TS, BODY);
        let now: i64 = TS.parse::<i64>().unwrap() + MAX_TIMESTAMP_SKEW_SECS + 1;
        let err = verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: &sig,
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::Stale));
    }

    #[test]
    fn rejects_future_timestamp_outside_window() {
        let sig = sign(SECRET, TS, BODY);
        let now: i64 = TS.parse::<i64>().unwrap() - MAX_TIMESTAMP_SKEW_SECS - 1;
        let err = verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: &sig,
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::Stale));
    }

    #[test]
    fn rejects_signature_without_v0_prefix() {
        // Strip the `v0=` prefix so the verifier rejects the header shape.
        let sig = sign(SECRET, TS, BODY);
        let stripped = sig.strip_prefix("v0=").unwrap();
        let now: i64 = TS.parse().unwrap();
        let err = verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: stripped,
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::BadSignaturePrefix));
    }

    #[test]
    fn rejects_empty_timestamp_or_signature() {
        let sig = sign(SECRET, TS, BODY);
        let now: i64 = TS.parse().unwrap();

        let err = verify(VerifyInputs {
            timestamp_header: "",
            signature_header: &sig,
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::MissingTimestamp));

        let err = verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: "",
            body: BODY,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::MissingSignature));
    }

    #[test]
    fn rejects_invalid_timestamp_format() {
        let sig = sign(SECRET, TS, BODY);
        let err = verify(VerifyInputs {
            timestamp_header: "not-a-number",
            signature_header: &sig,
            body: BODY,
            signing_secret: SECRET,
            now_secs: 1_531_420_618,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::InvalidTimestamp(_)));
    }

    #[test]
    fn body_modification_invalidates_signature() {
        // Same secret + timestamp, signature computed over BODY but verified
        // against a tampered body. Any byte change breaks the HMAC.
        let sig = sign(SECRET, TS, BODY);
        let mut tampered = BODY.to_vec();
        tampered[5] ^= 0x20; // flip a bit
        let now: i64 = TS.parse().unwrap();
        let err = verify(VerifyInputs {
            timestamp_header: TS,
            signature_header: &sig,
            body: &tampered,
            signing_secret: SECRET,
            now_secs: now,
        })
        .unwrap_err();
        assert!(matches!(err, SignatureError::Mismatch));
    }
}
