//! Outbound webhook hooks.
//!
//! Fire-and-forget HTTP POST to external URLs on hook events.
//! Useful for integrating with external monitoring, logging, or alerting
//! systems without blocking the agent pipeline.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::hooks::hook::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};

type HmacSha256 = Hmac<Sha256>;

/// Outbound webhook hook.
///
/// Serializes hook events to JSON and POSTs them to a configured URL.
/// If a signing secret is provided, the request includes an
/// `X-Hook-Signature` header with an HMAC-SHA256 hex digest.
///
/// Webhooks are fire-and-forget: they never block the hook chain and
/// always return `ok()`.
pub struct WebhookHook {
    name: String,
    url: String,
    points: Vec<HookPoint>,
    secret: Option<String>,
    client: Arc<reqwest::Client>,
}

impl WebhookHook {
    /// Create a new webhook hook.
    ///
    /// - `name` — unique hook name (e.g., `"webhook:slack-alerts"`)
    /// - `url` — the endpoint to POST events to (must be `http` or `https`)
    /// - `points` — which lifecycle points trigger this webhook
    /// - `secret` — optional HMAC-SHA256 signing key
    ///
    /// Returns `Err` if the URL scheme is not `http` or `https`, the URL is
    /// invalid, or if the HTTP client cannot be constructed.
    pub fn try_new(
        name: impl Into<String>,
        url: impl Into<String>,
        points: Vec<HookPoint>,
        secret: Option<String>,
    ) -> Result<Self, HookError> {
        let url = url.into();

        // SSRF protection: only allow http/https schemes
        match url::Url::parse(&url) {
            Ok(parsed) => {
                if parsed.scheme() != "http" && parsed.scheme() != "https" {
                    return Err(HookError::ExecutionFailed {
                        reason: format!(
                            "Webhook URL scheme '{}' not allowed; only http/https permitted",
                            parsed.scheme()
                        ),
                    });
                }

                // Warn on private/loopback addresses (informational, not blocking)
                if let Some(host) = parsed.host_str()
                    && (host == "localhost"
                        || host == "127.0.0.1"
                        || host == "::1"
                        || host.starts_with("10.")
                        || host.starts_with("172.")
                        || host.starts_with("192.168.")
                        || host == "169.254.169.254")
                {
                    tracing::warn!(
                        url = %url,
                        "Webhook targets a private/loopback address — ensure this is intentional"
                    );
                }
            }
            Err(e) => {
                return Err(HookError::ExecutionFailed {
                    reason: format!("Invalid webhook URL '{}': {}", url, e),
                });
            }
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| HookError::ExecutionFailed {
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        Ok(Self {
            name: name.into(),
            url,
            points,
            secret,
            client: Arc::new(client),
        })
    }

    /// Compute HMAC-SHA256 signature for the given body.
    fn compute_signature(secret: &str, body: &[u8]) -> Result<String, HookError> {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| {
            HookError::ExecutionFailed {
                reason: format!("Failed to initialize HMAC: {}", e),
            }
        })?;
        mac.update(body);
        let result = mac.finalize();
        Ok(hex::encode(result.into_bytes()))
    }
}

#[async_trait]
impl Hook for WebhookHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn hook_points(&self) -> &[HookPoint] {
        &self.points
    }

    fn failure_mode(&self) -> HookFailureMode {
        // Webhooks are always fail-open — they must never block the agent
        HookFailureMode::FailOpen
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        // Serialize the event
        let body = serde_json::to_vec(event).map_err(|e| HookError::ExecutionFailed {
            reason: format!("Failed to serialize webhook event: {}", e),
        })?;

        // Build the request
        let mut request = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("User-Agent", "ironclaw-webhook/0.1");

        // Add HMAC signature if secret is configured
        if let Some(ref secret) = self.secret {
            let signature = Self::compute_signature(secret, &body)?;
            request = request.header("X-Hook-Signature", format!("sha256={}", signature));
        }

        let request = request.body(body);

        // Fire-and-forget via tokio::spawn — don't block the hook chain
        let url = self.url.clone();
        tokio::spawn(async move {
            match request.send().await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        tracing::warn!(
                            url = %url,
                            status = %resp.status(),
                            "Webhook returned non-success status"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        url = %url,
                        error = %e,
                        "Webhook request failed"
                    );
                }
            }
        });

        // Always return ok — webhooks are informational
        Ok(HookOutcome::ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_hmac_signature() {
        let secret = "my-secret-key";
        let body = b"test payload";

        let sig = WebhookHook::compute_signature(secret, body).unwrap();

        // Verify it's a valid hex string of the right length (SHA-256 = 32 bytes = 64 hex chars)
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));

        // Same input should produce same output (deterministic)
        let sig2 = WebhookHook::compute_signature(secret, body).unwrap();
        assert_eq!(sig, sig2);

        // Different secret should produce different output
        let sig3 = WebhookHook::compute_signature("other-secret", body).unwrap();
        assert_ne!(sig, sig3);
    }

    #[test]
    fn test_webhook_serialization() {
        // Verify that HookEvent serializes to expected JSON format
        let event = HookEvent::ToolCall {
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "ls"}),
            user_id: "user-1".to_string(),
            context: "chat".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Enum serialization wraps in variant name
        assert!(parsed.get("ToolCall").is_some());
        let tc = &parsed["ToolCall"];
        assert_eq!(tc["tool_name"], "shell");
        assert_eq!(tc["user_id"], "user-1");
    }

    #[tokio::test]
    async fn test_webhook_always_returns_ok() {
        // Even with an invalid URL, the webhook should return ok
        // (fire-and-forget, errors logged but not propagated)
        let hook = WebhookHook::try_new(
            "test-webhook",
            "http://localhost:1/nonexistent",
            vec![HookPoint::BeforeInbound],
            None,
        )
        .unwrap();

        let event = HookEvent::Inbound {
            user_id: "user-1".into(),
            channel: "test".into(),
            content: "hello".into(),
            thread_id: None,
        };
        let ctx = HookContext::default();

        let result = hook.execute(&event, &ctx).await.unwrap();
        assert!(matches!(result, HookOutcome::Continue { modified: None }));
    }

    #[test]
    fn test_webhook_hook_points() {
        let hook = WebhookHook::try_new(
            "test",
            "http://example.com",
            vec![HookPoint::BeforeInbound, HookPoint::AfterToolCall],
            Some("secret".to_string()),
        )
        .unwrap();

        assert_eq!(hook.name(), "test");
        assert_eq!(hook.hook_points().len(), 2);
        assert_eq!(hook.failure_mode(), HookFailureMode::FailOpen);
    }

    #[test]
    fn test_webhook_rejects_non_http_scheme() {
        let result = WebhookHook::try_new(
            "bad-scheme",
            "ftp://example.com/hook",
            vec![HookPoint::BeforeInbound],
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_webhook_rejects_invalid_url() {
        let result = WebhookHook::try_new(
            "bad-url",
            "not a url at all",
            vec![HookPoint::BeforeInbound],
            None,
        );
        assert!(result.is_err());
    }
}
