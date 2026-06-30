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

/// Read a response body, rejecting anything over [`MAX_RESPONSE_BYTES`].
/// Returns the raw bytes on success or a human error string the caller
/// maps to the right [`OAuthError`](super::error::OAuthError) variant.
///
/// An advertised `Content-Length` over the cap fails *before* any body
/// is read. For chunked / length-less responses the body is read one
/// chunk at a time with a running total, so a hostile or misconfigured
/// endpoint cannot force an unbounded allocation regardless of what it
/// advertises — `reqwest` has no built-in body cap, so this loop is the
/// bound (the per-call client timeout only bounds time, not memory).
pub(super) async fn read_capped_body(mut resp: reqwest::Response) -> Result<Vec<u8>, String> {
    let over_limit =
        || format!("OAuth provider response exceeds the {MAX_RESPONSE_BYTES}-byte limit");
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
    {
        return Err(over_limit());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|err| err.to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(over_limit());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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

/// Build a server-log diagnostic string for a `reqwest` transport
/// failure. `reqwest::Error`'s own `Display` is only "error sending
/// request for url (…)"; the actionable cause — TLS handshake failure,
/// DNS resolution error, connection refused, or a per-call timeout —
/// lives in its source chain and in the `is_*` kind flags. This walks
/// both so the OAuth flow log says *why* the exchange could not be sent.
///
/// Server-log only: callers wrap the result in an `OAuthError` that the
/// route layer maps to an opaque client code; it is never echoed to the
/// browser.
pub(super) fn describe_transport_error(err: &reqwest::Error) -> String {
    let mut message = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    let mut kinds = Vec::new();
    if err.is_timeout() {
        kinds.push("timeout");
    }
    if err.is_connect() {
        kinds.push("connect");
    }
    if err.is_request() {
        kinds.push("request");
    }
    if err.is_body() {
        kinds.push("body");
    }
    if !kinds.is_empty() {
        message.push_str(&format!(" [{}]", kinds.join(",")));
    }
    message
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
    fn sanitize_error_code_redacts_codes_with_digits() {
        // The allow-set is lowercase ASCII + `_` only; digits are
        // excluded. This locks that contract so a future maintainer
        // broadening it to permit `error_123` does so deliberately, with
        // a failing test forcing the decision rather than silently
        // widening the log-injection surface.
        assert_eq!(
            sanitize_error_code("bad1code"),
            "<redacted_invalid_error_code>"
        );
        assert_eq!(
            sanitize_error_code("error_123"),
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

    /// A response whose body is cut short mid-stream (the connection
    /// drops before the advertised `Content-Length` is delivered) must
    /// surface the `reqwest::Response::chunk` error, NOT silently return
    /// the partial bytes as `Ok`. Every other test delivers a complete
    /// body, so without this the chunk-read error arm of the streaming
    /// loop is unexercised — a regression swallowing it into
    /// `Ok(partial)` would pass them all.
    #[tokio::test]
    async fn read_capped_body_propagates_chunk_read_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Drain the request so the client's write completes.
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                // Advertise 1000 bytes (well under the cap, so the
                // Content-Length early-bail does NOT fire) but send only
                // 10, then drop the connection. The client reads the
                // first chunk, then errors on the premature EOF.
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nonly-ten!!")
                    .await;
                let _ = sock.flush().await;
                // Socket dropped here without the remaining 990 bytes.
            }
        });

        let resp = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("send");
        let result = read_capped_body(resp).await;
        server.abort();

        assert!(
            result.is_err(),
            "a body cut short mid-stream must surface as an error, not Ok(partial)",
        );
    }
}
