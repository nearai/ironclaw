//! User-facing error message sanitization.
//!
//! The engine's `ThreadOutcome::Failed { error }` carries a deeply nested
//! error string that reaches the user verbatim via the web/chat UI. In
//! practice the raw string exposes internals:
//!
//! - Rust-level wrapping (`Orchestrator error: effect execution error: ...`)
//! - Python tracebacks from the Monty-hosted orchestrator script
//!   (`Traceback ... File "orchestrator.py", line 907, in ...`)
//! - Raw upstream failures (`HTTP 502 Bad Gateway`, JSON payloads)
//!
//! Issue #2546 tracked a case where a 502 from the LLM provider surfaced
//! the full traceback to a user on staging. The fix is two-sided:
//!
//! 1. Keep the raw error in the server-side logs (callers of this module
//!    are expected to `tracing::warn!` with the full string before
//!    rendering the sanitized text).
//! 2. Return a short, user-friendly message derived from the raw error
//!    whenever a known pattern matches. Unknown errors fall back to a
//!    generic "something went wrong" message rather than exposing the
//!    internal chain.
//!
//! This lives in `bridge::` because the adapter layer (not the engine
//! itself) owns the contract between engine outcomes and channel
//! responses — the engine is intentionally free to surface raw diagnostic
//! text, and the bridge is responsible for the presentation.

/// Categorization of a failure, used to pick the user-facing message and
/// to let tests assert on intent rather than matching on the full string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureCategory {
    /// The upstream LLM provider returned a transient server-side error
    /// (HTTP 502/503/504, explicit "bad gateway"/"service unavailable",
    /// provider-level timeouts). Users should retry.
    LlmUnavailable,
    /// The upstream LLM provider rate-limited the request (HTTP 429).
    LlmRateLimited,
    /// The request/context was too large for the provider (HTTP 413 or
    /// "context length exceeded"). Users can shorten the request.
    ContextTooLarge,
    /// Authentication with the provider failed (HTTP 401/403 or an
    /// `AuthFailed`/"session expired" message).
    AuthFailure,
    /// The agent stopped because it hit the iteration/step limit for a
    /// single turn. Already surfaced separately by `ThreadOutcome::MaxIterations`
    /// but can also appear inside a failed outcome in some paths.
    IterationLimit,
    /// Something else. Render a generic message and log the raw text.
    Unknown,
}

/// Convert a raw `ThreadOutcome::Failed { error }` string into a short,
/// user-friendly message. The returned text is safe to show verbatim in
/// chat — it never includes Python tracebacks, file paths, JSON payloads,
/// or internal wrapping like "effect execution error".
///
/// Callers should log the raw `error` string themselves before calling
/// this function so that full diagnostic detail is retained server-side.
pub(crate) fn user_facing_thread_failure(error: &str) -> String {
    match classify_failure(error) {
        FailureCategory::LlmUnavailable => {
            "The AI model is temporarily unavailable. Please try again in a few moments.".into()
        }
        FailureCategory::LlmRateLimited => {
            "The AI model is currently rate-limited. Please try again shortly.".into()
        }
        FailureCategory::ContextTooLarge => {
            "The request was too large for the AI model. Please shorten the conversation or attachments and try again."
                .into()
        }
        FailureCategory::AuthFailure => {
            "The AI model could not authenticate. Please re-authenticate the provider and try again."
                .into()
        }
        FailureCategory::IterationLimit => {
            "The agent reached its step limit before finishing. Please try again.".into()
        }
        FailureCategory::Unknown => {
            "Something went wrong while processing your message. Please try again.".into()
        }
    }
}

/// Convert a raw orchestrator-rollback reason into a short,
/// operator-facing classification that is safe to broadcast over SSE.
///
/// The rollback event's source string is
/// `format!("execution failed: {e}")` where `e` is an `EngineError` —
/// variants like `Store { reason }` and `Llm { reason }` can carry DB
/// connection strings, file paths, and raw upstream HTTP bodies. That
/// raw text must not reach every authenticated SSE consumer. Callers
/// should log the raw `reason` at `debug!` before calling this so
/// operators retain the diagnostic detail server-side.
pub(crate) fn user_facing_rollback_reason(reason: &str) -> &'static str {
    match classify_failure(reason) {
        FailureCategory::LlmUnavailable => "LLM provider unavailable",
        FailureCategory::LlmRateLimited => "LLM provider rate-limited",
        FailureCategory::ContextTooLarge => "context too large for provider",
        FailureCategory::AuthFailure => "LLM provider authentication failed",
        FailureCategory::IterationLimit => "execution step limit reached",
        FailureCategory::Unknown => "execution failed",
    }
}

/// Classify a raw failure string. Public(crate) so tests can assert on the
/// category independently of the user-facing wording.
pub(crate) fn classify_failure(error: &str) -> FailureCategory {
    // Case-insensitive substring matching. The raw error is wrapped
    // through multiple layers (Rust `Display`, Python traceback, upstream
    // HTTP body) so we intentionally do not try to parse it — we scan
    // for stable keywords.
    let lower = error.to_ascii_lowercase();

    // Rate limiting: check before the generic 5xx branch because 429s
    // sometimes get surfaced alongside "provider request failed".
    if lower.contains("http 429")
        || lower.contains("rate limited")
        || lower.contains("rate-limited")
        || lower.contains("ratelimited")
        || lower.contains("too many requests")
    {
        return FailureCategory::LlmRateLimited;
    }

    // Context length: HTTP 413 or explicit context-length error strings.
    // Note: we deliberately do NOT match on the generic "tokens used"
    // phrase — it's overly broad and can appear in informational text.
    // The explicit `context length exceeded` / `context_length_exceeded`
    // markers cover the real failure modes (including issue #2408).
    if lower.contains("http 413")
        || lower.contains("payload too large")
        || lower.contains("context length exceeded")
        || lower.contains("context_length_exceeded")
    {
        return FailureCategory::ContextTooLarge;
    }

    // Authentication failures. Match on specific HTTP status lines and
    // explicit auth-failure markers. We deliberately do NOT match on
    // bare "unauthorized" because that word also appears in
    // tool-level / resource-access errors that are not LLM auth issues.
    if lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("401 unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("authentication failed")
        || lower.contains("session expired")
        || lower.contains("session renewal failed")
    {
        return FailureCategory::AuthFailure;
    }

    // Provider unavailability. Matches the exact shape of the issue
    // #2546 traceback (`HTTP 502 Bad Gateway`) and also covers 503/504
    // and common upstream-timeout phrasings. We deliberately do NOT
    // match on the very generic `"request failed"` or `"provider nearai"`
    // phrases — they would misclassify non-5xx provider failures
    // (e.g. `Provider openai_codex request failed: HTTP 400`) as
    // transient unavailability.
    if lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
        || lower.contains("gateway timeout")
        || lower.contains("upstream connect error")
        || lower.contains("upstream")
        || lower.contains("provider temporarily unavailable")
        || lower.contains("llm call failed")
    {
        return FailureCategory::LlmUnavailable;
    }

    if lower.contains("max iterations") || lower.contains("maximum iterations") {
        return FailureCategory::IterationLimit;
    }

    FailureCategory::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact error string from issue #2546. This is the canonical
    /// regression fixture — the raw Python traceback must never reach
    /// the user verbatim.
    const ISSUE_2546_RAW: &str = "Orchestrator error: effect execution error: Orchestrator error after resume: \
         Traceback (most recent call last): \
         File \"orchestrator.py\", line 907, in  \
         File \"orchestrator.py\", line 548, in run_loop \
         RuntimeError: LLM call failed: Provider nearai_chat request failed: HTTP 502 Bad Gateway";

    #[test]
    fn issue_2546_502_bad_gateway_is_sanitized() {
        let msg = user_facing_thread_failure(ISSUE_2546_RAW);
        assert_eq!(
            msg,
            "The AI model is temporarily unavailable. Please try again in a few moments."
        );
    }

    #[test]
    fn issue_2546_is_classified_as_llm_unavailable() {
        assert_eq!(
            classify_failure(ISSUE_2546_RAW),
            FailureCategory::LlmUnavailable
        );
    }

    #[test]
    fn sanitized_message_never_leaks_python_traceback() {
        let msg = user_facing_thread_failure(ISSUE_2546_RAW);
        assert!(!msg.contains("Traceback"), "msg leaked traceback: {msg}");
        assert!(
            !msg.contains("orchestrator.py"),
            "msg leaked file path: {msg}"
        );
        assert!(
            !msg.contains("RuntimeError"),
            "msg leaked Python exc: {msg}"
        );
        assert!(!msg.contains("run_loop"), "msg leaked internal fn: {msg}");
        assert!(
            !msg.contains("effect execution error"),
            "msg leaked rust wrap: {msg}"
        );
        assert!(!msg.contains("nearai"), "msg leaked provider name: {msg}");
    }

    #[test]
    fn http_502_bad_gateway_variants() {
        let cases = [
            "HTTP 502 Bad Gateway",
            "http 502",
            "upstream returned Bad Gateway",
            "Provider foo request failed: HTTP 502",
        ];
        for case in cases {
            assert_eq!(
                classify_failure(case),
                FailureCategory::LlmUnavailable,
                "case: {case}"
            );
        }
    }

    #[test]
    fn http_503_service_unavailable() {
        assert_eq!(
            classify_failure("HTTP 503 Service Unavailable"),
            FailureCategory::LlmUnavailable
        );
    }

    #[test]
    fn http_504_gateway_timeout() {
        assert_eq!(
            classify_failure("HTTP 504 Gateway Timeout"),
            FailureCategory::LlmUnavailable
        );
    }

    #[test]
    fn http_429_is_rate_limited() {
        assert_eq!(
            classify_failure("Provider foo request failed: HTTP 429 Too Many Requests"),
            FailureCategory::LlmRateLimited
        );
        assert_eq!(
            user_facing_thread_failure("HTTP 429 Too Many Requests"),
            "The AI model is currently rate-limited. Please try again shortly."
        );
    }

    #[test]
    fn http_413_is_context_too_large() {
        // Related issue #2276 — 413 Payload Too Large from nearai_chat.
        assert_eq!(
            classify_failure("Provider nearai_chat request failed: HTTP 413 Payload Too Large"),
            FailureCategory::ContextTooLarge
        );
    }

    #[test]
    fn context_length_exceeded_is_context_too_large() {
        // Related issue #2408.
        assert_eq!(
            classify_failure("Context length exceeded: 200000 tokens used, 128000 allowed"),
            FailureCategory::ContextTooLarge
        );
    }

    #[test]
    fn http_401_is_auth_failure() {
        assert_eq!(
            classify_failure("HTTP 401 Unauthorized"),
            FailureCategory::AuthFailure
        );
    }

    #[test]
    fn authentication_failed_text_is_auth_failure() {
        assert_eq!(
            classify_failure("Authentication failed for provider 'nearai'."),
            FailureCategory::AuthFailure
        );
    }

    #[test]
    fn session_expired_is_auth_failure() {
        assert_eq!(
            classify_failure("Session expired for provider nearai"),
            FailureCategory::AuthFailure
        );
    }

    #[test]
    fn unknown_errors_get_generic_message() {
        let msg = user_facing_thread_failure("something totally unexpected");
        assert_eq!(
            msg,
            "Something went wrong while processing your message. Please try again."
        );
        assert!(!msg.contains("something totally unexpected"));
    }

    #[test]
    fn empty_error_string_does_not_panic() {
        let msg = user_facing_thread_failure("");
        assert_eq!(
            msg,
            "Something went wrong while processing your message. Please try again."
        );
    }

    #[test]
    fn case_insensitive_matching() {
        // Real errors come through a chain of `Display` impls, so the
        // exact casing is not guaranteed across versions.
        assert_eq!(
            classify_failure("HTTP 502 BAD GATEWAY"),
            FailureCategory::LlmUnavailable
        );
        assert_eq!(
            classify_failure("http 502 bad gateway"),
            FailureCategory::LlmUnavailable
        );
    }

    #[test]
    fn rate_limit_takes_precedence_over_unavailable() {
        // If a response somehow surfaces both keywords (e.g. a 429
        // response body that mentions "bad gateway upstream"), we
        // prefer the rate-limit message because it's more actionable.
        assert_eq!(
            classify_failure("HTTP 429 Too Many Requests (upstream: bad gateway)"),
            FailureCategory::LlmRateLimited
        );
    }

    #[test]
    fn iteration_limit_is_classified() {
        assert_eq!(
            classify_failure("Reached maximum iterations"),
            FailureCategory::IterationLimit
        );
    }

    #[test]
    fn non_5xx_provider_failure_is_not_llm_unavailable() {
        // Regression for PR #2747 review: the old classifier matched
        // on the very generic `"request failed"` phrase, which caused
        // non-5xx provider failures (like a 400 Bad Request) to be
        // misclassified as transient unavailability. Those should now
        // fall through to `Unknown` so the user gets the generic
        // "something went wrong" message instead of an incorrect
        // "try again in a few moments" nudge.
        let raw = "Provider openai_codex request failed: HTTP 400 Bad Request";
        assert_ne!(
            classify_failure(raw),
            FailureCategory::LlmUnavailable,
            "non-5xx provider failure must not classify as LlmUnavailable"
        );
        assert_eq!(classify_failure(raw), FailureCategory::Unknown);
    }

    #[test]
    fn bare_unauthorized_is_not_auth_failure() {
        // Regression for PR #2747 review: the old classifier matched
        // on bare `"unauthorized"`, which caught tool-level resource
        // permission errors and mislabeled them as LLM provider auth
        // failures. Only explicit LLM-auth markers should match.
        let raw = "Tool failed: unauthorized to access /etc/shadow";
        assert_ne!(
            classify_failure(raw),
            FailureCategory::AuthFailure,
            "bare 'unauthorized' must not classify as AuthFailure"
        );
    }

    #[test]
    fn invalid_api_key_is_auth_failure() {
        assert_eq!(
            classify_failure("Provider returned: Invalid API key"),
            FailureCategory::AuthFailure
        );
        assert_eq!(
            classify_failure("{\"error\":{\"code\":\"invalid_api_key\"}}"),
            FailureCategory::AuthFailure
        );
    }

    #[test]
    fn http_401_with_unauthorized_word_still_matches() {
        // The common wire shape `HTTP 401 Unauthorized` continues to
        // classify as AuthFailure via the explicit `"http 401"` marker.
        assert_eq!(
            classify_failure("401 Unauthorized: invalid token"),
            FailureCategory::AuthFailure
        );
    }

    #[test]
    fn auth_failure_message_is_channel_agnostic() {
        // Regression for PR #2747 review: the old copy said "Please
        // reconnect the provider", which is web-UI-specific. The
        // router is used across channels (web/telegram/CLI), so the
        // message must avoid channel-specific verbs.
        let msg = user_facing_thread_failure("HTTP 401 Unauthorized");
        assert!(
            !msg.contains("reconnect"),
            "auth failure copy must not use 'reconnect' (web-only verb): {msg}"
        );
        assert!(
            msg.contains("re-authenticate"),
            "auth failure copy should guide users to re-authenticate: {msg}"
        );
    }

    #[test]
    fn rollback_reason_drops_engine_error_detail() {
        // Regression for PR #2844: `EventKind::OrchestratorRollback.reason`
        // originates from `format!("execution failed: {e}")` in the engine,
        // where `e: EngineError` can render DB connection strings, file
        // paths, tokens, and upstream HTTP bodies. The sanitizer must
        // produce an output that is fully independent of the raw engine
        // detail — two unrelated leaky strings must collapse to the same
        // classified message. `assert_eq!(msg_a, msg_b)` is the load-
        // bearing check; the `contains` probes below are sentinel sniffs
        // for the specific leak shapes the fix is meant to prevent.
        let raw_a = "execution failed: store error: connection \
            'postgres://bob:hunter2@db:5432/x' refused: \
            File \"/home/runner/.ironclaw/state.db\" not found";
        // Deliberately avoid any substring that the classifier recognises
        // (no `upstream`, no `http 5xx`, no `rate limited`, etc.) so both
        // inputs fall into the `Unknown` category — the test is about
        // sanitisation, not classification.
        let raw_b = "execution failed: store error: leaked \
            token sk_live_123 and request id req_abc123 at /etc/secrets/keyring";

        let msg_a = user_facing_rollback_reason(raw_a);
        let msg_b = user_facing_rollback_reason(raw_b);

        assert_eq!(msg_a, "execution failed");
        assert_eq!(msg_b, "execution failed");
        assert_eq!(
            msg_a, msg_b,
            "sanitized rollback reason must be independent of raw engine detail"
        );
        assert!(!msg_a.contains("postgres://"));
        assert!(!msg_a.contains("hunter2"));
        assert!(!msg_a.contains("/home/runner"));
        assert!(!msg_b.contains("sk_live_123"));
        assert!(!msg_b.contains("req_abc123"));
    }

    #[test]
    fn rollback_reason_classifies_known_upstream_failures() {
        assert_eq!(
            user_facing_rollback_reason("execution failed: LLM error: HTTP 502 Bad Gateway"),
            "LLM provider unavailable"
        );
        assert_eq!(
            user_facing_rollback_reason("execution failed: HTTP 429 Too Many Requests"),
            "LLM provider rate-limited"
        );
        assert_eq!(
            user_facing_rollback_reason("execution failed: HTTP 413 Payload Too Large"),
            "context too large for provider"
        );
        assert_eq!(
            user_facing_rollback_reason("execution failed: HTTP 401 Unauthorized"),
            "LLM provider authentication failed"
        );
        assert_eq!(
            user_facing_rollback_reason("execution failed: max iterations reached"),
            "execution step limit reached"
        );
    }

    #[test]
    fn all_messages_end_with_period() {
        // Tiny presentation invariant — every sanitized message is a
        // complete sentence. Keeps the UI consistent.
        for cat in [
            FailureCategory::LlmUnavailable,
            FailureCategory::LlmRateLimited,
            FailureCategory::ContextTooLarge,
            FailureCategory::AuthFailure,
            FailureCategory::IterationLimit,
            FailureCategory::Unknown,
        ] {
            let raw = match cat {
                FailureCategory::LlmUnavailable => "HTTP 502 Bad Gateway",
                FailureCategory::LlmRateLimited => "HTTP 429",
                FailureCategory::ContextTooLarge => "HTTP 413",
                FailureCategory::AuthFailure => "HTTP 401",
                FailureCategory::IterationLimit => "max iterations reached",
                FailureCategory::Unknown => "???",
            };
            let msg = user_facing_thread_failure(raw);
            assert!(
                msg.ends_with('.'),
                "category {cat:?} msg does not end with period: {msg}"
            );
        }
    }
}
