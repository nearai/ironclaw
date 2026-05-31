//! HTTP helpers shared across OAuth provider implementations.
//!
//! These are deliberately provider-agnostic (they take a bare
//! `reqwest::Response` / `&str`) so GitHub today — and Google / NEAR /
//! future providers — read response bodies and log provider error
//! codes through one hardened path instead of each re-implementing the
//! size cap and the log-injection guard.

/// Defensive cap on any single OAuth provider JSON response body. Real
/// responses are a few KB; anything past this is treated as a hostile
/// or misconfigured endpoint (a non-HTTPS / overridden `*_endpoint`
/// pointing at an attacker) and rejected before serde allocates the
/// parsed structure.
pub(super) const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Read a response body, rejecting anything over [`MAX_RESPONSE_BYTES`]
/// before it is handed to serde. Returns the raw bytes on success or a
/// human error string the caller maps to the right
/// [`OAuthError`](super::error::OAuthError) variant.
///
/// An advertised `Content-Length` over the cap fails *before* the body
/// is buffered; the post-read length check then covers chunked /
/// length-less responses (`reqwest` has no built-in body cap, and the
/// per-call client timeout is the only other bound on a hostile stream).
pub(super) async fn read_capped_body(resp: reqwest::Response) -> Result<Vec<u8>, String> {
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "OAuth provider response exceeds the {MAX_RESPONSE_BYTES}-byte limit"
        ));
    }
    let bytes = resp.bytes().await.map_err(|err| err.to_string())?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(format!(
            "OAuth provider response exceeds the {MAX_RESPONSE_BYTES}-byte limit"
        ));
    }
    Ok(bytes.to_vec())
}

/// OAuth error codes returned in a provider's response body follow the
/// RFC 6749 §5.2 `error` grammar — lowercase ASCII + underscore
/// (`bad_verification_code`, `redirect_uri_mismatch`, …). The value is
/// attacker-influenceable (a hostile token endpoint could return
/// arbitrary bytes), so anything off that grammar — or implausibly
/// long — is redacted before it reaches a log line or error string,
/// preventing newline / ANSI log injection.
pub(super) fn sanitize_error_code(error: &str) -> &str {
    if !error.is_empty()
        && error.len() <= 64
        && error.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    {
        error
    } else {
        "<redacted_invalid_error_code>"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_error_code_allows_well_formed_codes() {
        assert_eq!(
            sanitize_error_code("bad_verification_code"),
            "bad_verification_code"
        );
        assert_eq!(
            sanitize_error_code("redirect_uri_mismatch"),
            "redirect_uri_mismatch"
        );
    }

    #[test]
    fn sanitize_error_code_redacts_newline_injection() {
        assert_eq!(
            sanitize_error_code("code\nX-Injected: hdr"),
            "<redacted_invalid_error_code>"
        );
    }

    #[test]
    fn sanitize_error_code_redacts_uppercase_and_punctuation() {
        assert_eq!(
            sanitize_error_code("Bad_Code"),
            "<redacted_invalid_error_code>"
        );
        assert_eq!(
            sanitize_error_code("bad-code"),
            "<redacted_invalid_error_code>"
        );
    }

    #[test]
    fn sanitize_error_code_redacts_oversized() {
        let oversized = "a".repeat(65);
        assert_eq!(
            sanitize_error_code(&oversized),
            "<redacted_invalid_error_code>"
        );
    }

    #[test]
    fn sanitize_error_code_redacts_empty() {
        assert_eq!(sanitize_error_code(""), "<redacted_invalid_error_code>");
    }
}
