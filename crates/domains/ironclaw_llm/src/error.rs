//! LLM provider error types.

use std::time::Duration;

use futures::StreamExt;

const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Errors that occur while assembling LLM configuration from settings/env.
///
/// Distinct from [`LlmError`] (runtime / request errors): these fire before
/// any provider is constructed, when a per-backend config struct is being
/// built. The binary's `crate::error::ConfigError` carries a
/// `From<LlmConfigError>` impl so callers can `?` through both layers.
#[derive(Debug, thiserror::Error)]
pub enum LlmConfigError {
    #[error("Missing required configuration: {key}. {hint}")]
    MissingRequired { key: String, hint: String },

    #[error("Invalid configuration value for {key}: {message}")]
    InvalidValue { key: String, message: String },
}

/// Provider id used by placeholder providers that exist only because no real
/// LLM has been configured yet. Errors carrying this provider id are a
/// configuration fault, not an availability fault — retrying cannot succeed,
/// so error mapping must fail fast instead of riding an availability backoff.
pub const UNCONFIGURED_PROVIDER_ID: &str = "unconfigured";

/// LLM provider errors.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Provider {provider} request failed: {reason}")]
    RequestFailed { provider: String, reason: String },

    #[error("Provider {provider} rejected the request: {reason}")]
    InvalidRequest { provider: String, reason: String },

    #[error("Provider {provider} rate limited, retry after {retry_after:?}")]
    RateLimited {
        provider: String,
        retry_after: Option<Duration>,
    },

    /// Upstream provider returned any HTTP 5xx (500–599). Covers both
    /// proxy-layer failures (502/503/504) and upstream application errors
    /// (500/501/505…). Response body is intentionally NOT carried on this
    /// variant — upstream 5xx bodies frequently contain Python tracebacks or
    /// other internal detail that must not cross the channel boundary (see
    /// `.claude/rules/error-handling.md`). Operators find the body in
    /// `debug!`-level logs at the source provider.
    #[error("Provider {provider} temporarily unavailable (HTTP {status})")]
    BadGateway {
        provider: String,
        status: u16,
        retry_after: Option<Duration>,
    },

    #[error("Invalid response from {provider}: {reason}")]
    InvalidResponse { provider: String, reason: String },

    #[error("Empty response from {provider}: no content returned")]
    EmptyResponse { provider: String },

    /// A streaming response ended before the provider's terminal frame.
    ///
    /// This is distinct from a completed but structurally invalid or empty
    /// response: the former has connection-level retry evidence, while the
    /// latter must enter bounded invalid-output recovery.
    #[error("Response stream from {provider} was interrupted: {reason}")]
    StreamInterrupted { provider: String, reason: String },

    #[error("Context length exceeded: {used} tokens used, {limit} allowed")]
    ContextLengthExceeded { used: usize, limit: usize },

    #[error("Model {model} not available on provider {provider}")]
    ModelNotAvailable { provider: String, model: String },

    #[error("Provider {provider} quota or billing is exhausted: {reason}")]
    QuotaExceeded { provider: String, reason: String },

    #[error(
        "Authentication failed for provider '{provider}'. {}",
        auth_guidance(provider)
    )]
    AuthFailed { provider: String },

    #[error("Session expired for provider {provider}")]
    SessionExpired { provider: String },

    #[error("Session renewal failed for provider {provider}: {reason}")]
    SessionRenewalFailed { provider: String, reason: String },

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Whether a raw HTTP error carries concrete evidence that retrying may
/// succeed: a connection/timeout/body-stream failure or an availability HTTP
/// status. Decode and request-construction failures are intentionally false.
pub fn is_transient_http_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || error.is_body()
        || error.status().is_some_and(|status| {
            status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
        })
}

/// Whether an opaque I/O error carries connection-level retry evidence.
///
/// In-tree `Io` producers are session-file operations, so filesystem and
/// unknown error kinds fail closed. External providers that surface socket I/O
/// retain retry behavior only for explicitly connection-shaped kinds.
pub fn is_transient_io_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::TimedOut
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "Rig and Bedrock use message/SDK mapping; all variants form the test conformance matrix"
)]
pub(crate) enum ProductionModelAdapter {
    Rig,
    NearAiChat,
    AnthropicOauth,
    GeminiOauth,
    GithubCopilot,
    Bedrock,
    OpenAiCodex,
    CodexChatGpt,
}

impl ProductionModelAdapter {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 8] = [
        Self::Rig,
        Self::NearAiChat,
        Self::AnthropicOauth,
        Self::GeminiOauth,
        Self::GithubCopilot,
        Self::Bedrock,
        Self::OpenAiCodex,
        Self::CodexChatGpt,
    ];

    pub(crate) const fn provider_id(self) -> &'static str {
        match self {
            Self::Rig => "rig",
            Self::NearAiChat => "nearai_chat",
            Self::AnthropicOauth => "anthropic_oauth",
            Self::GeminiOauth => "gemini_oauth",
            Self::GithubCopilot => "github_copilot",
            Self::Bedrock => "bedrock",
            Self::OpenAiCodex => "openai_codex",
            Self::CodexChatGpt => "codex_chatgpt",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProviderHttpError<'a> {
    pub(crate) adapter: ProductionModelAdapter,
    pub(crate) model: &'a str,
    pub(crate) status: u16,
    pub(crate) body: &'a str,
    pub(crate) retry_after: Option<Duration>,
}

/// Read only a bounded preview of an untrusted provider error response.
///
/// The content length only informs a capped initial allocation. Both declared
/// and lengthless responses are streamed only until the cap.
pub(crate) async fn read_bounded_provider_error_body(
    response: reqwest::Response,
) -> Result<Vec<u8>, reqwest::Error> {
    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(MAX_PROVIDER_ERROR_BODY_BYTES);
    let mut body = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let remaining = MAX_PROVIDER_ERROR_BODY_BYTES.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        if chunk.len() >= remaining {
            body.extend_from_slice(&chunk[..remaining]);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) fn map_provider_http_error(error: ProviderHttpError<'_>) -> LlmError {
    let provider = error.adapter.provider_id();
    if let Some(context_error) = context_length_error(error.status, error.body) {
        return context_error;
    }
    if is_quota_or_billing_error(error.status, error.body) {
        return LlmError::QuotaExceeded {
            provider: provider.to_string(),
            reason: bounded_provider_reason(error.status, error.body),
        };
    }
    if matches!(error.status, 401 | 403) {
        return LlmError::AuthFailed {
            provider: provider.to_string(),
        };
    }
    if error.status == 429 {
        return LlmError::RateLimited {
            provider: provider.to_string(),
            retry_after: error.retry_after,
        };
    }
    if matches!(error.status, 400 | 404)
        && is_model_not_available_message(&error.body.to_ascii_lowercase())
    {
        return LlmError::ModelNotAvailable {
            provider: provider.to_string(),
            model: error.model.to_string(),
        };
    }
    if matches!(error.status, 500..=599) {
        tracing::debug!(
            adapter = ?error.adapter,
            status = error.status,
            body = %ironclaw_common::truncate_for_preview(error.body, 512),
            "provider returned an upstream server error"
        );
        return LlmError::BadGateway {
            provider: provider.to_string(),
            status: error.status,
            retry_after: error.retry_after,
        };
    }
    if error.status == 400 {
        return LlmError::InvalidRequest {
            provider: provider.to_string(),
            reason: bounded_provider_reason(error.status, error.body),
        };
    }
    LlmError::RequestFailed {
        provider: provider.to_string(),
        reason: bounded_provider_reason(error.status, error.body),
    }
}

pub(crate) fn map_provider_message_error(
    provider: &str,
    model: &str,
    message: impl Into<String>,
) -> LlmError {
    let message = message.into();
    let lower = message.to_ascii_lowercase();
    if is_context_length_error_message(&lower) {
        let (used, limit) = parse_context_token_counts(&lower);
        return LlmError::ContextLengthExceeded { used, limit };
    }
    if is_quota_or_billing_error(
        first_standalone_http_status(&lower).unwrap_or_default(),
        &lower,
    ) {
        return LlmError::QuotaExceeded {
            provider: provider.to_string(),
            reason: bounded_provider_message_reason(&message),
        };
    }
    if is_auth_error_message(&lower) {
        return LlmError::AuthFailed {
            provider: provider.to_string(),
        };
    }
    if is_rate_limit_message(&lower) {
        return LlmError::RateLimited {
            provider: provider.to_string(),
            retry_after: None,
        };
    }
    if is_model_not_available_message(&lower) {
        return LlmError::ModelNotAvailable {
            provider: provider.to_string(),
            model: model.to_string(),
        };
    }
    if let Some(status) = first_standalone_http_status(&lower)
        && matches!(status, 500..=599)
    {
        return LlmError::BadGateway {
            provider: provider.to_string(),
            status,
            retry_after: None,
        };
    }
    if first_standalone_http_status(&lower) == Some(400) {
        return LlmError::InvalidRequest {
            provider: provider.to_string(),
            reason: bounded_provider_message_reason(&message),
        };
    }
    LlmError::RequestFailed {
        provider: provider.to_string(),
        reason: bounded_provider_message_reason(&message),
    }
}

fn bounded_provider_message_reason(message: &str) -> String {
    bounded_redacted_provider_text(message)
}

fn bounded_provider_reason(status: u16, body: &str) -> String {
    format!("HTTP {status}: {}", bounded_redacted_provider_text(body))
}

fn bounded_redacted_provider_text(text: &str) -> String {
    let bounded = ironclaw_common::truncate_for_preview(text, 512);
    let display_safe = ironclaw_safety::sanitize_display_text(&bounded);
    ironclaw_safety::LeakDetector::default()
        .redact_all_secrets(&display_safe)
        .0
}

pub(crate) fn is_model_not_available_message(lower: &str) -> bool {
    const MODEL_MISSING_PHRASES: &[&str] = &[
        "model not found",
        "model_not_found",
        "unknown model",
        "invalid model",
        "no such model",
        "model is not supported",
        "unsupported model",
    ];
    MODEL_MISSING_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
        || (contains_status_code(lower, "404") && lower.contains("model"))
        || (lower.contains("model") && lower.contains("does not exist"))
}

pub(crate) fn is_auth_error_message(lower: &str) -> bool {
    if ["401", "403"]
        .iter()
        .any(|code| contains_status_code(lower, code))
    {
        return true;
    }
    const AUTH_PATTERNS: &[&str] = &[
        "unauthorized",
        "invalid api key",
        "incorrect api key",
        "invalid_api_key",
        "authentication",
        "permission denied",
        "access denied",
        "missing api key",
        "no api key",
    ];
    AUTH_PATTERNS.iter().any(|pattern| lower.contains(pattern))
}

fn is_quota_or_billing_error(status: u16, message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    status == 402
        || [
            "payment required",
            "insufficient credit",
            "insufficient credits",
            "not enough credit",
            "not enough credits",
            "credits exhausted",
            "out of credits",
            "insufficient_quota",
            "billing hard limit",
            "billing limit",
        ]
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn is_rate_limit_message(lower: &str) -> bool {
    lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("resource_exhausted")
}

pub(crate) fn contains_status_code(lower: &str, code: &str) -> bool {
    let bytes = lower.as_bytes();
    lower.match_indices(code).any(|(start, matched)| {
        let before_is_digit = start
            .checked_sub(1)
            .is_some_and(|i| bytes[i].is_ascii_digit());
        let end = start + matched.len();
        let after_is_digit = bytes.get(end).is_some_and(u8::is_ascii_digit);
        !before_is_digit && !after_is_digit
    })
}

fn first_standalone_http_status(lower: &str) -> Option<u16> {
    (400_u16..=599)
        .filter_map(|status| {
            let code = status.to_string();
            let bytes = lower.as_bytes();
            lower.match_indices(&code).find_map(|(start, matched)| {
                let before_is_digit = start
                    .checked_sub(1)
                    .is_some_and(|index| bytes[index].is_ascii_digit());
                let end = start + matched.len();
                let after_is_digit = bytes.get(end).is_some_and(u8::is_ascii_digit);
                (!before_is_digit && !after_is_digit).then_some((start, status))
            })
        })
        .min_by_key(|(start, _)| *start)
        .map(|(_, status)| status)
}

pub(crate) fn context_length_error(status_code: u16, response_text: &str) -> Option<LlmError> {
    if status_code != 413 && status_code != 400 {
        return None;
    }

    let lower = response_text.to_ascii_lowercase();
    let is_context_overflow = status_code == 413 || is_context_length_error_message(&lower);
    if !is_context_overflow {
        return None;
    }

    let (used, limit) = parse_context_token_counts(&lower);
    Some(LlmError::ContextLengthExceeded { used, limit })
}

pub(crate) fn is_context_length_error_message(lower: &str) -> bool {
    const CONTEXT_PATTERNS: &[&str] = &[
        "context_length_exceeded",
        "maximum context length",
        "too many tokens",
        "payload too large",
        "longer than the model's context length",
    ];

    parse_prompt_too_long_counts(lower).is_some()
        || CONTEXT_PATTERNS
            .iter()
            .any(|pattern| lower.contains(pattern))
}

/// Try to extract token counts from a context-length error message.
///
/// Handles patterns like:
/// - "maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens."
/// - "The input (150000 tokens) is longer than the model's context length (128000 tokens)."
/// - "prompt is too long: 150000 tokens > 128000 maximum"
///
/// Returns `(0, 0)` if parsing fails.
pub(crate) fn parse_context_token_counts(lower: &str) -> (usize, usize) {
    // NEAR Anthropic-compatible proxy pattern:
    // "prompt is too long: {used} tokens > {limit} maximum"
    if let Some((used, limit)) = parse_prompt_too_long_counts(lower) {
        return (used, limit);
    }

    let numbers = token_count_numbers(lower);
    if numbers.len() < 2 {
        return (0, 0);
    }

    // OpenAI pattern: "maximum context length is {limit} tokens. ... resulted in {used} tokens".
    if lower.contains("maximum context length") {
        return (numbers[1], numbers[0]);
    }

    // NEAR/OpenAI-compatible proxy pattern:
    // "The input ({used} tokens) is longer than the model's context length ({limit} tokens)."
    if lower.contains("longer than the model's context length") {
        return (numbers[0], numbers[1]);
    }

    (0, 0)
}

fn parse_prompt_too_long_counts(lower: &str) -> Option<(usize, usize)> {
    let tail = lower.split_once("prompt is too long:")?.1.trim_start();
    let (used, tail) = tail.split_once("tokens")?;
    let used = used.trim().parse().ok().filter(|&n| n > 0)?;
    let tail = tail.trim_start().strip_prefix('>')?.trim_start();
    let (limit, _) = tail.split_once("maximum")?;
    let limit = limit.trim().parse().ok().filter(|&n| n > 0)?;
    Some((used, limit))
}

fn token_count_numbers(lower: &str) -> Vec<usize> {
    lower
        .split("tokens")
        .filter_map(number_immediately_before)
        .filter(|&n| n > 0)
        .collect()
}

fn number_immediately_before(segment: &str) -> Option<usize> {
    let mut skipped_alphabetic = false;
    let digits = segment
        .chars()
        .rev()
        .skip_while(|ch| {
            if ch.is_alphabetic() {
                skipped_alphabetic = true;
            }
            !ch.is_ascii_digit()
        })
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<Vec<_>>();

    if skipped_alphabetic || digits.is_empty() {
        return None;
    }

    let digits = digits.into_iter().rev().collect::<String>();
    digits.parse().ok().filter(|&n| n > 0)
}

/// Return actionable setup guidance for a provider's authentication failure.
///
/// This helps users who see an `AuthFailed` error know exactly what to do
/// without digging through documentation.
fn auth_guidance(provider: &str) -> String {
    let normalized = provider.to_lowercase();
    let (env_hint, extra) = match normalized.as_str() {
        "nearai" | "near_ai" | "near" => (
            "Set NEARAI_API_KEY (from https://cloud.near.ai) or run `ironclaw onboard` to log in",
            "",
        ),
        "openai" => (
            "Set OPENAI_API_KEY (from https://platform.openai.com/api-keys)",
            "",
        ),
        "anthropic" | "claude" => (
            "Set ANTHROPIC_API_KEY (from https://console.anthropic.com/settings/keys)",
            "",
        ),
        "groq" => ("Set GROQ_API_KEY (from https://console.groq.com/keys)", ""),
        "ollama" => (
            "Ensure Ollama is running locally (no API key needed). Set OLLAMA_BASE_URL if not at default http://localhost:11434",
            "",
        ),
        "openai_compatible" => (
            "Set LLM_API_KEY and LLM_BASE_URL for your OpenAI-compatible endpoint",
            "",
        ),
        "tinfoil" => ("Set TINFOIL_API_KEY", ""),
        "bedrock" | "aws_bedrock" | "aws" => (
            "Configure AWS credentials (AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY or AWS_PROFILE)",
            "",
        ),
        "openai_codex" | "codex" => ("Run `ironclaw login --openai-codex` to authenticate", ""),
        "github_copilot" => (
            "Set GITHUB_COPILOT_TOKEN or run `ironclaw onboard --step provider` to log in via device code",
            "",
        ),
        _ => (
            "Check that the required API key environment variable is set for this provider",
            "",
        ),
    };
    if extra.is_empty() {
        format!("{env_hint}. Or run `ironclaw onboard --step provider` to configure interactively.")
    } else {
        format!(
            "{env_hint}. {extra} Or run `ironclaw onboard --step provider` to configure interactively."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ProviderErrorFixture {
        name: &'static str,
        status: u16,
        body: &'static str,
        retry_after: Option<Duration>,
        assert_error: fn(&LlmError, ProductionModelAdapter),
    }

    fn assert_auth(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::AuthFailed { provider }
                    if provider == adapter.provider_id()
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_context(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::ContextLengthExceeded {
                    used: 150_000,
                    limit: 128_000
                }
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_context_without_counts(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(error, LlmError::ContextLengthExceeded { used: 0, limit: 0 }),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_model_missing(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::ModelNotAvailable { provider, model }
                    if provider == adapter.provider_id() && model == "fixture-model"
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_quota(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::QuotaExceeded { provider, reason }
                    if provider == adapter.provider_id()
                        && reason.contains("insufficient_quota")
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_rate_limit(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::RateLimited {
                    provider,
                    retry_after: Some(delay)
                } if provider == adapter.provider_id() && *delay == Duration::from_secs(17)
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_bad_gateway(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::BadGateway {
                    provider,
                    status: 500 | 503 | 504,
                    ..
                } if provider == adapter.provider_id()
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_service_unavailable_retry(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::BadGateway {
                    provider,
                    status: 503,
                    retry_after: Some(delay)
                } if provider == adapter.provider_id() && *delay == Duration::from_secs(17)
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_invalid_request(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::InvalidRequest { provider, reason }
                    if provider == adapter.provider_id()
                        && reason.contains("unsupported request option")
            ),
            "{adapter:?}: {error:?}"
        );
    }

    fn assert_request_failed(error: &LlmError, adapter: ProductionModelAdapter) {
        assert!(
            matches!(
                error,
                LlmError::RequestFailed { provider, reason }
                    if provider == adapter.provider_id()
                        && reason.contains("resource does not exist")
            ),
            "{adapter:?}: {error:?}"
        );
    }

    #[test]
    fn production_adapters_conform_to_provider_error_fixture_matrix() {
        let fixtures = [
            ProviderErrorFixture {
                name: "401 authentication",
                status: 401,
                body: r#"{"error":"unauthorized"}"#,
                retry_after: None,
                assert_error: assert_auth,
            },
            ProviderErrorFixture {
                name: "403 authorization",
                status: 403,
                body: r#"{"error":"permission denied"}"#,
                retry_after: None,
                assert_error: assert_auth,
            },
            ProviderErrorFixture {
                name: "400 context length",
                status: 400,
                body: "maximum context length is 128000 tokens; messages resulted in 150000 tokens",
                retry_after: None,
                assert_error: assert_context,
            },
            ProviderErrorFixture {
                name: "413 Gemini OAuth payload",
                status: 413,
                body: r#"{"error":{"status":"INVALID_ARGUMENT","message":"request too large"}}"#,
                retry_after: None,
                assert_error: assert_context_without_counts,
            },
            ProviderErrorFixture {
                name: "400 model missing",
                status: 400,
                body: r#"{"error":{"code":"model_not_found"}}"#,
                retry_after: None,
                assert_error: assert_model_missing,
            },
            ProviderErrorFixture {
                name: "404 model missing",
                status: 404,
                body: r#"{"error":"model does not exist"}"#,
                retry_after: None,
                assert_error: assert_model_missing,
            },
            ProviderErrorFixture {
                name: "unrelated 404",
                status: 404,
                body: r#"{"error":"resource does not exist"}"#,
                retry_after: None,
                assert_error: assert_request_failed,
            },
            ProviderErrorFixture {
                name: "402 billing",
                status: 402,
                body: r#"{"error":{"code":"insufficient_quota"}}"#,
                retry_after: None,
                assert_error: assert_quota,
            },
            ProviderErrorFixture {
                name: "403 billing exhaustion",
                status: 403,
                body: r#"{"error":{"code":"insufficient_quota"}}"#,
                retry_after: None,
                assert_error: assert_quota,
            },
            ProviderErrorFixture {
                name: "429 retry metadata",
                status: 429,
                body: r#"{"error":"too many requests"}"#,
                retry_after: Some(Duration::from_secs(17)),
                assert_error: assert_rate_limit,
            },
            ProviderErrorFixture {
                name: "500 upstream",
                status: 500,
                body: "sensitive upstream traceback",
                retry_after: None,
                assert_error: assert_bad_gateway,
            },
            ProviderErrorFixture {
                name: "503 retry metadata",
                status: 503,
                body: "temporarily unavailable",
                retry_after: Some(Duration::from_secs(17)),
                assert_error: assert_service_unavailable_retry,
            },
            ProviderErrorFixture {
                name: "504 gateway timeout",
                status: 504,
                body: "gateway timeout",
                retry_after: None,
                assert_error: assert_bad_gateway,
            },
            ProviderErrorFixture {
                name: "generic 400",
                status: 400,
                body: r#"{"error":"unsupported request option"}"#,
                retry_after: None,
                assert_error: assert_invalid_request,
            },
        ];

        for adapter in ProductionModelAdapter::ALL {
            for fixture in &fixtures {
                let error = map_provider_http_error(ProviderHttpError {
                    adapter,
                    model: "fixture-model",
                    status: fixture.status,
                    body: fixture.body,
                    retry_after: fixture.retry_after,
                });
                (fixture.assert_error)(&error, adapter);
                assert!(
                    !error.to_string().contains("sensitive upstream traceback"),
                    "{} leaked a 5xx body for {adapter:?}",
                    fixture.name
                );
            }
        }
    }

    #[test]
    fn message_mapper_uses_first_status_in_provider_text() {
        let error = map_provider_message_error(
            "fixture",
            "fixture-model",
            "upstream returned 503 after an earlier request used option 400",
        );
        assert!(matches!(
            error,
            LlmError::BadGateway {
                provider,
                status: 503,
                retry_after: None
            } if provider == "fixture"
        ));
    }

    #[test]
    fn message_mapper_bounds_payload_bearing_error_reasons() {
        for (prefix, assert_reason) in [
            (
                "insufficient credits: ",
                LlmError::QuotaExceeded {
                    provider: "fixture".to_string(),
                    reason: String::new(),
                },
            ),
            (
                "HTTP 400 invalid request: ",
                LlmError::InvalidRequest {
                    provider: "fixture".to_string(),
                    reason: String::new(),
                },
            ),
            (
                "provider transport failed: ",
                LlmError::RequestFailed {
                    provider: "fixture".to_string(),
                    reason: String::new(),
                },
            ),
        ] {
            let message = format!("{prefix}{}TAIL_MARKER", "x".repeat(2_000));
            let error = map_provider_message_error("fixture", "fixture-model", &message);
            let reason = match (&error, assert_reason) {
                (LlmError::QuotaExceeded { reason, .. }, LlmError::QuotaExceeded { .. })
                | (LlmError::InvalidRequest { reason, .. }, LlmError::InvalidRequest { .. })
                | (LlmError::RequestFailed { reason, .. }, LlmError::RequestFailed { .. }) => {
                    reason
                }
                _ => panic!("unexpected classification for {prefix}: {error:?}"),
            };
            assert!(reason.starts_with(prefix));
            assert!(!reason.contains("TAIL_MARKER"));
            assert!(reason.len() < message.len());
        }
    }

    #[test]
    fn provider_error_reasons_redact_untrusted_tokens_urls_and_paths() {
        let secret = ["gh", "p_012345678901234567890123456789012345"].concat();
        let unsafe_message = format!(
            "provider transport failed token: {secret} at /home/runner/private.json via https://internal.example/v1?access_token={secret}"
        );
        let error = map_provider_message_error("fixture", "fixture-model", &unsafe_message);
        let reason = match error {
            LlmError::RequestFailed { reason, .. } => reason,
            other => panic!("expected request failure, got {other:?}"),
        };
        assert!(reason.contains("[redacted]"));
        assert!(!reason.contains(&secret));
        assert!(!reason.contains("/home/runner/private.json"));
        assert!(!reason.contains("access_token="));

        let error = map_provider_http_error(ProviderHttpError {
            adapter: ProductionModelAdapter::NearAiChat,
            model: "fixture-model",
            status: 400,
            body: &unsafe_message,
            retry_after: None,
        });
        let reason = match error {
            LlmError::InvalidRequest { reason, .. } => reason,
            other => panic!("expected invalid request, got {other:?}"),
        };
        assert!(reason.contains("[redacted]"));
        assert!(!reason.contains(&secret));
        assert!(!reason.contains("/home/runner/private.json"));
        assert!(!reason.contains("access_token="));
    }

    #[tokio::test]
    async fn provider_error_body_reader_stops_at_hard_cap() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener");
        let address = listener.local_addr().expect("loopback address");
        let oversized_body = vec![b'x'; MAX_PROVIDER_ERROR_BODY_BYTES + 4_096];
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let headers = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                oversized_body.len()
            );
            socket
                .write_all(headers.as_bytes())
                .await
                .expect("write headers");
            let _ = socket.write_all(&oversized_body).await;
        });

        let response = reqwest::get(format!("http://{address}"))
            .await
            .expect("loopback response");
        let body = read_bounded_provider_error_body(response)
            .await
            .expect("bounded body");
        server.await.expect("loopback server");

        assert_eq!(body.len(), MAX_PROVIDER_ERROR_BODY_BYTES);
        assert!(body.iter().all(|byte| *byte == b'x'));
    }

    #[test]
    fn auth_failed_error_includes_guidance() {
        let err = LlmError::AuthFailed {
            provider: "openai".to_string(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("OPENAI_API_KEY"),
            "should mention the env var: {msg}"
        );
        assert!(
            msg.contains("ironclaw onboard"),
            "should mention onboard command: {msg}"
        );
    }

    #[test]
    fn auth_failed_error_for_anthropic() {
        let err = LlmError::AuthFailed {
            provider: "anthropic".to_string(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("ANTHROPIC_API_KEY"),
            "should mention ANTHROPIC_API_KEY: {msg}"
        );
    }

    #[test]
    fn auth_failed_error_for_unknown_provider() {
        let err = LlmError::AuthFailed {
            provider: "my_custom_provider".to_string(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("API key environment variable"),
            "should give generic guidance: {msg}"
        );
        assert!(
            msg.contains("ironclaw onboard"),
            "should still mention onboard: {msg}"
        );
    }

    #[test]
    fn auth_guidance_is_provider_specific() {
        assert!(auth_guidance("nearai").contains("NEARAI_API_KEY"));
        assert!(auth_guidance("groq").contains("GROQ_API_KEY"));
        assert!(auth_guidance("ollama").contains("Ollama is running"));
        assert!(auth_guidance("bedrock").contains("AWS"));
    }

    #[test]
    fn request_construction_http_errors_are_not_transient() {
        let error = reqwest::Client::new()
            .get("http://[invalid")
            .build()
            .expect_err("fixture URL must fail request construction");
        assert!(!is_transient_http_error(&error));
    }

    #[tokio::test]
    async fn refused_http_connections_are_transient() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let address = listener.local_addr().expect("loopback address");
        drop(listener);

        let error = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client builds")
            .get(format!("http://{address}"))
            .send()
            .await
            .expect_err("closed loopback port must refuse the connection");
        assert!(
            error.is_connect(),
            "expected connection evidence: {error:?}"
        );
        assert!(is_transient_http_error(&error));
    }

    #[test]
    fn io_retryability_requires_connection_shaped_error_kind() {
        assert!(is_transient_io_error(&std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "socket reset"
        )));
        assert!(!is_transient_io_error(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "session file denied"
        )));
    }

    #[test]
    fn parse_context_token_counts_ignores_non_adjacent_digits_before_tokens() {
        let msg = "provider model 3 has some tokens available. this model's maximum context length is 128000 tokens. however, your messages resulted in 150000 tokens.";
        let (used, limit) = parse_context_token_counts(msg);
        assert_eq!(used, 150000);
        assert_eq!(limit, 128000);
    }

    #[test]
    fn prompt_too_long_requires_near_proxy_token_limit_shape() {
        let malformed = r#"{"error":{"message":"prompt is too long: 234872 tokens"}}"#;
        assert!(!is_context_length_error_message(malformed));
        assert_eq!(parse_context_token_counts(malformed), (0, 0));

        let unrelated = r#"{"error":{"message":"prompt is too long for this schema"}}"#;
        assert!(!is_context_length_error_message(unrelated));
        assert_eq!(parse_context_token_counts(unrelated), (0, 0));
        assert!(context_length_error(400, unrelated).is_none());
    }

    // ------------------------------------------------------------------
    // Snapshot-style coverage for rendered AuthFailed messages.
    //
    // The auth error text is policy-bearing product guidance: it tells
    // users which env var to set and where to get an API key. Treat it
    // as compatibility-sensitive — any change to these strings should
    // be a deliberate, reviewed edit. These tests assert the full
    // rendered `Display` output (the same text users see in the CLI
    // and logs) via `insta::assert_snapshot!` with inline snapshots.
    //
    // We render through `LlmError::AuthFailed { .. }.to_string()`
    // rather than calling `auth_guidance()` directly so that a change
    // to the outer `#[error(..)]` format string is also caught
    // (test-through-the-caller discipline, per CLAUDE.md).
    // ------------------------------------------------------------------

    fn render_auth_failed(provider: &str) -> String {
        LlmError::AuthFailed {
            provider: provider.to_string(),
        }
        .to_string()
    }

    #[test]
    fn snapshot_auth_failed_nearai() {
        insta::assert_snapshot!(
            render_auth_failed("nearai"),
            @"Authentication failed for provider 'nearai'. Set NEARAI_API_KEY (from https://cloud.near.ai) or run `ironclaw onboard` to log in. Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_openai() {
        insta::assert_snapshot!(
            render_auth_failed("openai"),
            @"Authentication failed for provider 'openai'. Set OPENAI_API_KEY (from https://platform.openai.com/api-keys). Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_anthropic() {
        insta::assert_snapshot!(
            render_auth_failed("anthropic"),
            @"Authentication failed for provider 'anthropic'. Set ANTHROPIC_API_KEY (from https://console.anthropic.com/settings/keys). Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_ollama() {
        insta::assert_snapshot!(
            render_auth_failed("ollama"),
            @"Authentication failed for provider 'ollama'. Ensure Ollama is running locally (no API key needed). Set OLLAMA_BASE_URL if not at default http://localhost:11434. Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_openai_compatible() {
        insta::assert_snapshot!(
            render_auth_failed("openai_compatible"),
            @"Authentication failed for provider 'openai_compatible'. Set LLM_API_KEY and LLM_BASE_URL for your OpenAI-compatible endpoint. Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_tinfoil() {
        insta::assert_snapshot!(
            render_auth_failed("tinfoil"),
            @"Authentication failed for provider 'tinfoil'. Set TINFOIL_API_KEY. Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_bedrock() {
        insta::assert_snapshot!(
            render_auth_failed("bedrock"),
            @"Authentication failed for provider 'bedrock'. Configure AWS credentials (AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY or AWS_PROFILE). Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }

    #[test]
    fn snapshot_auth_failed_unknown_provider() {
        // The generic fallback — exercised when a new provider is added
        // but not yet wired into `auth_guidance()`. Snapshotted so that
        // any change to the generic fallback is also deliberate.
        insta::assert_snapshot!(
            render_auth_failed("some_future_provider"),
            @"Authentication failed for provider 'some_future_provider'. Check that the required API key environment variable is set for this provider. Or run `ironclaw onboard --step provider` to configure interactively."
        );
    }
}
