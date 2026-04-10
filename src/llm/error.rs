//! LLM provider error types.

use std::time::Duration;

/// LLM provider errors.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Provider {provider} request failed: {reason}")]
    RequestFailed { provider: String, reason: String },

    #[error("Provider {provider} rate limited, retry after {retry_after:?}")]
    RateLimited {
        provider: String,
        retry_after: Option<Duration>,
    },

    #[error("Invalid response from {provider}: {reason}")]
    InvalidResponse { provider: String, reason: String },

    #[error("Empty response from {provider}: no content returned")]
    EmptyResponse { provider: String },

    #[error("Context length exceeded: {used} tokens used, {limit} allowed")]
    ContextLengthExceeded { used: usize, limit: usize },

    #[error("Model {model} not available on provider {provider}")]
    ModelNotAvailable { provider: String, model: String },

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
}
