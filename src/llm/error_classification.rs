//! Recovery-oriented classification for LLM provider errors.
//!
//! Hermes-agent treats upstream failures as actionable categories rather than
//! opaque strings. This module brings the same idea to IronClaw so retry,
//! failover, circuit breaking, and context compaction can make consistent
//! decisions from a single classification result.

use std::time::Duration;

use crate::llm::error::LlmError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFailureReason {
    AuthRecoverable,
    AuthPermanent,
    Billing,
    RateLimit,
    Overloaded,
    ServerError,
    Timeout,
    ContextOverflow,
    PayloadTooLarge,
    ModelNotAvailable,
    FormatError,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlmErrorClassification {
    pub reason: LlmFailureReason,
    pub retryable: bool,
    pub should_failover: bool,
    pub should_compact_context: bool,
    pub counts_as_transient: bool,
    pub retry_after: Option<Duration>,
}

impl LlmErrorClassification {
    const fn new(
        reason: LlmFailureReason,
        retryable: bool,
        should_failover: bool,
        should_compact_context: bool,
        counts_as_transient: bool,
        retry_after: Option<Duration>,
    ) -> Self {
        Self {
            reason,
            retryable,
            should_failover,
            should_compact_context,
            counts_as_transient,
            retry_after,
        }
    }
}

pub fn classify_llm_error(err: &LlmError) -> LlmErrorClassification {
    match err {
        LlmError::RateLimited { retry_after, .. } => LlmErrorClassification::new(
            LlmFailureReason::RateLimit,
            true,
            true,
            false,
            true,
            *retry_after,
        ),
        LlmError::ContextLengthExceeded { .. } => context_overflow(),
        LlmError::ModelNotAvailable { .. } => model_not_available(),
        LlmError::AuthFailed { .. } => auth_permanent(),
        LlmError::SessionExpired { .. } => LlmErrorClassification::new(
            LlmFailureReason::AuthRecoverable,
            false,
            true,
            false,
            true,
            None,
        ),
        LlmError::SessionRenewalFailed { reason, .. } => {
            classify_request_like_error(None, reason).unwrap_or_else(|| unknown(true, true, true))
        }
        LlmError::RequestFailed { provider, reason } => {
            classify_request_like_error(Some(provider), reason)
                .unwrap_or_else(|| unknown(true, true, true))
        }
        LlmError::InvalidResponse { provider, reason } => {
            classify_request_like_error(Some(provider), reason).unwrap_or_else(malformed_response)
        }
        LlmError::EmptyResponse { .. } => malformed_response(),
        LlmError::Http(error) => classify_http_transport_error(error),
        LlmError::Json(_) => LlmErrorClassification::new(
            LlmFailureReason::FormatError,
            false,
            false,
            false,
            false,
            None,
        ),
        LlmError::Io(error) => classify_io_transport_error(error),
    }
}

fn classify_request_like_error(
    provider: Option<&str>,
    reason: &str,
) -> Option<LlmErrorClassification> {
    let normalized = normalize(reason);

    if is_github_copilot(provider) && has_auth_pattern(&normalized) {
        return Some(auth_recoverable(true));
    }

    if let Some(status) = extract_http_status(reason) {
        if let Some(classification) = classify_http_status(status, &normalized) {
            return Some(classification);
        }
    }

    if normalized.contains("insufficient_quota")
        || normalized.contains("billing_hard_limit")
        || normalized.contains("billing_not_active")
    {
        return Some(billing());
    }
    if normalized.contains("rate_limit")
        || normalized.contains("overloaded_error")
        || normalized.contains("context_length_exceeded")
        || normalized.contains("model_not_found")
        || normalized.contains("payload_too_large")
    {
        return Some(match () {
            _ if normalized.contains("context_length_exceeded") => context_overflow(),
            _ if normalized.contains("model_not_found") => model_not_available(),
            _ if normalized.contains("payload_too_large") => payload_too_large(),
            _ if normalized.contains("overloaded_error") => overloaded(),
            _ => rate_limit(extract_retry_after(&normalized)),
        });
    }

    if has_billing_pattern(&normalized) {
        return Some(billing());
    }
    if has_rate_limit_pattern(&normalized) {
        return Some(rate_limit(extract_retry_after(&normalized)));
    }
    if has_context_overflow_pattern(&normalized) {
        return Some(context_overflow());
    }
    if has_payload_too_large_pattern(&normalized) {
        return Some(payload_too_large());
    }
    if has_model_not_found_pattern(&normalized) {
        return Some(model_not_available());
    }
    if has_auth_pattern(&normalized) {
        return Some(auth_permanent());
    }
    if has_overloaded_pattern(&normalized) {
        return Some(overloaded());
    }
    if has_server_error_pattern(&normalized) {
        return Some(server_error());
    }

    if normalized.contains("too many messages")
        || normalized.contains("prompt is too long")
        || normalized.contains("request is too large")
        || normalized.contains("message too long")
    {
        return Some(context_overflow());
    }

    if has_timeout_pattern(&normalized) {
        return Some(timeout());
    }
    if has_transport_pattern(&normalized) {
        return Some(unknown(true, true, true));
    }

    None
}

fn classify_http_status(status: u16, normalized: &str) -> Option<LlmErrorClassification> {
    match status {
        400 => {
            if has_context_overflow_pattern(normalized) {
                Some(context_overflow())
            } else if has_payload_too_large_pattern(normalized) {
                Some(payload_too_large())
            } else {
                None
            }
        }
        401 => Some(auth_permanent()),
        402 => Some(billing()),
        403 => {
            if has_billing_pattern(normalized) {
                Some(billing())
            } else {
                Some(auth_permanent())
            }
        }
        404 => {
            if has_model_not_found_pattern(normalized) {
                Some(model_not_available())
            } else {
                None
            }
        }
        408 => Some(timeout()),
        413 => Some(payload_too_large()),
        429 => Some(rate_limit(extract_retry_after(normalized))),
        500..=599 => {
            if has_context_overflow_pattern(normalized) {
                Some(context_overflow())
            } else if has_timeout_pattern(normalized) {
                Some(timeout())
            } else if has_overloaded_pattern(normalized) {
                Some(overloaded())
            } else {
                Some(server_error())
            }
        }
        _ => None,
    }
}

fn classify_http_transport_error(error: &reqwest::Error) -> LlmErrorClassification {
    if error.is_timeout() {
        return timeout();
    }
    if matches!(
        error.status().map(|s| s.as_u16()),
        Some(408 | 429 | 500..=599)
    ) {
        if let Some(status) = error.status().map(|s| s.as_u16()) {
            return classify_http_status(status, "").unwrap_or_else(server_error);
        }
    }
    unknown(true, true, true)
}

fn classify_io_transport_error(error: &std::io::Error) -> LlmErrorClassification {
    match error.kind() {
        std::io::ErrorKind::TimedOut => timeout(),
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::Interrupted
        | std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::BrokenPipe => unknown(true, true, true),
        _ => unknown(true, true, true),
    }
}

fn auth_recoverable(retryable: bool) -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::AuthRecoverable,
        retryable,
        true,
        false,
        false,
        None,
    )
}

fn auth_permanent() -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::AuthPermanent,
        false,
        true,
        false,
        false,
        None,
    )
}

fn billing() -> LlmErrorClassification {
    LlmErrorClassification::new(LlmFailureReason::Billing, false, true, false, false, None)
}

fn rate_limit(retry_after: Option<Duration>) -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::RateLimit,
        true,
        true,
        false,
        true,
        retry_after,
    )
}

fn overloaded() -> LlmErrorClassification {
    LlmErrorClassification::new(LlmFailureReason::Overloaded, true, true, false, true, None)
}

fn server_error() -> LlmErrorClassification {
    LlmErrorClassification::new(LlmFailureReason::ServerError, true, true, false, true, None)
}

fn timeout() -> LlmErrorClassification {
    LlmErrorClassification::new(LlmFailureReason::Timeout, true, true, false, true, None)
}

fn context_overflow() -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::ContextOverflow,
        false,
        false,
        true,
        false,
        None,
    )
}

fn payload_too_large() -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::PayloadTooLarge,
        false,
        true,
        false,
        false,
        None,
    )
}

fn model_not_available() -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::ModelNotAvailable,
        false,
        true,
        false,
        false,
        None,
    )
}

fn malformed_response() -> LlmErrorClassification {
    LlmErrorClassification::new(LlmFailureReason::FormatError, true, true, false, true, None)
}

fn unknown(
    retryable: bool,
    should_failover: bool,
    counts_as_transient: bool,
) -> LlmErrorClassification {
    LlmErrorClassification::new(
        LlmFailureReason::Unknown,
        retryable,
        should_failover,
        false,
        counts_as_transient,
        None,
    )
}

fn is_github_copilot(provider: Option<&str>) -> bool {
    provider
        .map(normalize)
        .is_some_and(|provider| provider.contains("copilot") || provider.contains("github"))
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn extract_http_status(reason: &str) -> Option<u16> {
    let normalized = reason.trim();

    if let Some(rest) = normalized.strip_prefix("HTTP ") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() == 3 {
            return digits.parse().ok();
        }
    }

    for token in normalized.split(|c: char| !(c.is_ascii_alphanumeric())) {
        if token.len() == 3 && token.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(status) = token.parse::<u16>() {
                if (100..=599).contains(&status) {
                    return Some(status);
                }
            }
        }
    }

    None
}

fn extract_retry_after(normalized: &str) -> Option<Duration> {
    let marker = "retry after";
    let start = normalized.find(marker)?;
    let rest = normalized[start + marker.len()..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let seconds: u64 = digits.parse().ok()?;
    Some(Duration::from_secs(seconds))
}

fn contains_any(normalized: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| normalized.contains(pattern))
}

fn has_auth_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "unauthorized",
            "invalid api key",
            "invalid_api_key",
            "authentication failed",
            "authentication error",
            "auth failed",
            "invalid token",
            "token expired",
            "forbidden",
            "permission denied",
            "access denied",
            "invalid x-api-key",
        ],
    )
}

fn has_billing_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "insufficient quota",
            "quota exceeded",
            "insufficient_quota",
            "billing",
            "payment required",
            "credit balance",
            "hard limit",
            "out of credits",
        ],
    )
}

fn has_rate_limit_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "rate limit",
            "too many requests",
            "retry later",
            "requests per minute",
            "tokens per minute",
        ],
    )
}

fn has_context_overflow_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "context length",
            "maximum context",
            "context window",
            "too many tokens",
            "token limit",
            "max context",
            "prompt is too long",
            "messages exceeded",
        ],
    )
}

fn has_payload_too_large_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "payload too large",
            "request entity too large",
            "body too large",
            "content too large",
        ],
    )
}

fn has_model_not_found_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "model not found",
            "model_not_found",
            "does not exist",
            "unknown model",
            "not available on provider",
        ],
    )
}

fn has_overloaded_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "overloaded",
            "temporarily unavailable",
            "try again later",
            "capacity",
            "server is busy",
            "service unavailable",
        ],
    )
}

fn has_server_error_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "internal server error",
            "bad gateway",
            "upstream",
            "server error",
        ],
    )
}

fn has_timeout_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "timed out",
            "timeout",
            "deadline exceeded",
            "operation too slow",
        ],
    )
}

fn has_transport_pattern(normalized: &str) -> bool {
    contains_any(
        normalized,
        &[
            "connection reset",
            "connection refused",
            "connection aborted",
            "broken pipe",
            "unexpected eof",
            "tls handshake",
            "dns",
            "temporarily unavailable",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_hidden_context_overflow_in_request_failed() {
        let err = LlmError::RequestFailed {
            provider: "openai".into(),
            reason: "HTTP 400: maximum context length exceeded".into(),
        };

        let classification = classify_llm_error(&err);
        assert_eq!(classification.reason, LlmFailureReason::ContextOverflow);
        assert!(classification.should_compact_context);
        assert!(!classification.retryable);
    }

    #[test]
    fn classifies_billing_errors_from_message_patterns() {
        let err = LlmError::RequestFailed {
            provider: "openai".into(),
            reason: "insufficient_quota: billing hard limit reached".into(),
        };

        let classification = classify_llm_error(&err);
        assert_eq!(classification.reason, LlmFailureReason::Billing);
        assert!(!classification.retryable);
        assert!(classification.should_failover);
    }

    #[test]
    fn classifies_timeout_transport_errors() {
        let err = LlmError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "operation timed out",
        ));

        let classification = classify_llm_error(&err);
        assert_eq!(classification.reason, LlmFailureReason::Timeout);
        assert!(classification.retryable);
        assert!(classification.counts_as_transient);
    }

    #[test]
    fn keeps_github_copilot_401_recoverable() {
        let err = LlmError::RequestFailed {
            provider: "github_copilot".into(),
            reason: "HTTP 401 Unauthorized".into(),
        };

        let classification = classify_llm_error(&err);
        assert_eq!(classification.reason, LlmFailureReason::AuthRecoverable);
        assert!(classification.retryable);
        assert!(!classification.counts_as_transient);
    }

    #[test]
    fn non_copilot_401_is_permanent_auth_failure() {
        let err = LlmError::RequestFailed {
            provider: "openai".into(),
            reason: "HTTP 401 Unauthorized".into(),
        };

        let classification = classify_llm_error(&err);
        assert_eq!(classification.reason, LlmFailureReason::AuthPermanent);
        assert!(!classification.retryable);
        assert!(classification.should_failover);
    }

    #[test]
    fn extracts_retry_after_from_hidden_429_message() {
        let err = LlmError::RequestFailed {
            provider: "openai".into(),
            reason: "HTTP 429 Too Many Requests, retry after 12 seconds".into(),
        };

        let classification = classify_llm_error(&err);
        assert_eq!(classification.reason, LlmFailureReason::RateLimit);
        assert_eq!(classification.retry_after, Some(Duration::from_secs(12)));
    }
}
