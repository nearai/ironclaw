//! Host-mediated Telegram Bot API client for setup-time and pairing-time
//! calls (`getMe`, `setWebhook`, `deleteWebhook`, `sendMessage`).
//!
//! The bot token never appears in URL strings built here: requests carry the
//! literal `{telegram_bot_token}` placeholder and the mediated host egress
//! substitutes the material via [`RuntimeCredentialTarget::PathPlaceholder`].
//! Error values carry stable reasons only — never URLs, bodies, or token
//! material.

use std::sync::Arc;

use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme,
    NetworkTargetPattern, ResourceScope, RuntimeCredentialTarget, RuntimeHttpEgressRequest,
    RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_host_runtime::{
    HostRuntimeCredentialMaterial, HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest,
};
use ironclaw_secrets::SecretMaterial;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use thiserror::Error;

use crate::telegram_account_setup::TELEGRAM_EXTENSION_ID;

pub const TELEGRAM_API_HOST: &str = "api.telegram.org";
const TELEGRAM_BOT_TOKEN_PLACEHOLDER: &str = "telegram_bot_token";
const TELEGRAM_EGRESS_CAPABILITY_ID: &str = "telegram.egress";
const TELEGRAM_EGRESS_TIMEOUT_MS: u32 = 10_000;
const TELEGRAM_EGRESS_RESPONSE_BODY_LIMIT_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramBotIdentity {
    pub id: i64,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramBotApiError {
    /// Transport-level failure reaching api.telegram.org.
    #[error("telegram bot api unavailable: {reason}")]
    Unavailable { reason: String },
    /// Telegram answered `ok: false` (invalid token, rejected webhook URL, …).
    /// Carries a stable category only — the provider-controlled `description`
    /// text never enters this error's `Display` (it flows toward the admin
    /// surface via `TelegramSetupError`); the bounded raw text goes to a
    /// `debug!` diagnostic at the parse site instead.
    #[error("telegram bot api rejected the request: {kind}")]
    Rejected { kind: TelegramBotApiRejection },
    /// Telegram answered 2xx but the body did not match the expected shape.
    #[error("telegram bot api returned an invalid response: {reason}")]
    InvalidResponse { reason: String },
}

/// Stable, sanitized rejection categories derived from the HTTP status —
/// never from Telegram's free-text `description`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramBotApiRejection {
    /// 401/404 — invalid or revoked bot token.
    Unauthorized,
    /// 403 — the bot is blocked or lacks access.
    Forbidden,
    /// 429 — Telegram rate limit.
    RateLimited,
    /// 400 and other request rejections.
    InvalidRequest,
}

impl std::fmt::Display for TelegramBotApiRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            TelegramBotApiRejection::Unauthorized => "unauthorized (check the bot token)",
            TelegramBotApiRejection::Forbidden => "forbidden",
            TelegramBotApiRejection::RateLimited => "rate limited",
            TelegramBotApiRejection::InvalidRequest => "invalid request",
        })
    }
}

fn classify_rejection(status: u16) -> TelegramBotApiRejection {
    match status {
        401 | 404 => TelegramBotApiRejection::Unauthorized,
        403 => TelegramBotApiRejection::Forbidden,
        429 => TelegramBotApiRejection::RateLimited,
        _ => TelegramBotApiRejection::InvalidRequest,
    }
}

/// Production implementation over the shared mediated host HTTP egress port.
pub struct HostEgressTelegramBotApi {
    host_egress: HostRuntimeHttpEgressPort,
    scope_template: ResourceScope,
}

impl std::fmt::Debug for HostEgressTelegramBotApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostEgressTelegramBotApi").finish()
    }
}

impl HostEgressTelegramBotApi {
    pub fn new(host_egress: HostRuntimeHttpEgressPort, scope_template: ResourceScope) -> Self {
        Self {
            host_egress,
            scope_template,
        }
    }

    pub fn arced(
        host_egress: HostRuntimeHttpEgressPort,
        scope_template: ResourceScope,
    ) -> Arc<Self> {
        Arc::new(Self::new(host_egress, scope_template))
    }

    async fn call(
        &self,
        bot_token: &SecretString,
        method: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, TelegramBotApiError> {
        let response = self
            .host_egress
            .execute(bot_api_request(
                &self.scope_template,
                bot_token,
                method,
                body,
            )?)
            .await
            .map_err(|error| TelegramBotApiError::Unavailable {
                reason: error.stable_runtime_reason().to_string(),
            })?;
        parse_bot_api_response(response.status, &response.body)
    }

    pub async fn get_me(
        &self,
        bot_token: &SecretString,
    ) -> Result<TelegramBotIdentity, TelegramBotApiError> {
        let result = self.call(bot_token, "getMe", serde_json::json!({})).await?;
        #[derive(Debug, Deserialize)]
        struct Me {
            id: i64,
            username: Option<String>,
        }
        let me: Me =
            serde_json::from_value(result).map_err(|_| TelegramBotApiError::InvalidResponse {
                reason: "getMe result missing id/username".to_string(),
            })?;
        let username = me
            .username
            .ok_or_else(|| TelegramBotApiError::InvalidResponse {
                reason: "getMe result missing username".to_string(),
            })?;
        Ok(TelegramBotIdentity {
            id: me.id,
            username,
        })
    }

    pub async fn set_webhook(
        &self,
        bot_token: &SecretString,
        url: &str,
        secret_token: &SecretString,
    ) -> Result<(), TelegramBotApiError> {
        self.call(
            bot_token,
            "setWebhook",
            serde_json::json!({
                "url": url,
                "secret_token": secret_token.expose_secret(),
                "allowed_updates": ["message"],
            }),
        )
        .await
        .map(|_| ())
    }

    pub async fn delete_webhook(
        &self,
        bot_token: &SecretString,
    ) -> Result<(), TelegramBotApiError> {
        self.call(bot_token, "deleteWebhook", serde_json::json!({}))
            .await
            .map(|_| ())
    }

    pub async fn send_message(
        &self,
        bot_token: &SecretString,
        chat_id: i64,
        text: &str,
    ) -> Result<(), TelegramBotApiError> {
        self.call(
            bot_token,
            "sendMessage",
            serde_json::json!({ "chat_id": chat_id, "text": text }),
        )
        .await
        .map(|_| ())
    }
}

fn bot_api_request(
    scope_template: &ResourceScope,
    bot_token: &SecretString,
    method: &str,
    body: serde_json::Value,
) -> Result<HostRuntimeHttpEgressRequest, TelegramBotApiError> {
    let capability_id = CapabilityId::new(TELEGRAM_EGRESS_CAPABILITY_ID).map_err(|error| {
        TelegramBotApiError::Unavailable {
            reason: format!("invalid capability id: {error}"),
        }
    })?;
    let extension_id = ExtensionId::new(TELEGRAM_EXTENSION_ID).map_err(|error| {
        TelegramBotApiError::Unavailable {
            reason: format!("invalid extension id: {error}"),
        }
    })?;
    let secret_handle = SecretHandle::new(TELEGRAM_BOT_TOKEN_PLACEHOLDER).map_err(|error| {
        TelegramBotApiError::Unavailable {
            reason: format!("invalid credential handle: {error}"),
        }
    })?;
    let body = serde_json::to_vec(&body).map_err(|error| TelegramBotApiError::Unavailable {
        reason: format!("request serialization failed: {error}"),
    })?;
    let mut scope = scope_template.clone();
    scope.invocation_id = InvocationId::new();
    Ok(HostRuntimeHttpEgressRequest {
        extension_id,
        trust: TrustClass::System,
        request: RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope,
            capability_id,
            method: NetworkMethod::Post,
            url: format!(
                "https://{TELEGRAM_API_HOST}/bot{{{TELEGRAM_BOT_TOKEN_PLACEHOLDER}}}/{method}"
            ),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body,
            network_policy: telegram_network_policy(),
            credential_injections: Vec::new(),
            response_body_limit: Some(TELEGRAM_EGRESS_RESPONSE_BODY_LIMIT_BYTES),
            save_body_to: None,
            timeout_ms: Some(TELEGRAM_EGRESS_TIMEOUT_MS),
        },
        credentials: vec![HostRuntimeCredentialMaterial {
            handle: secret_handle,
            material: SecretMaterial::from(bot_token.expose_secret().to_string()),
            target: RuntimeCredentialTarget::PathPlaceholder {
                placeholder: TELEGRAM_BOT_TOKEN_PLACEHOLDER.to_string(),
            },
            required: true,
        }],
    })
}

fn telegram_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: TELEGRAM_API_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

#[derive(Debug, Deserialize)]
struct BotApiEnvelope {
    ok: bool,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    description: Option<String>,
}

const REJECTION_DIAGNOSTIC_MAX_CHARS: usize = 160;

fn parse_bot_api_response(
    status: u16,
    body: &[u8],
) -> Result<serde_json::Value, TelegramBotApiError> {
    let envelope: BotApiEnvelope =
        serde_json::from_slice(body).map_err(|_| TelegramBotApiError::InvalidResponse {
            reason: format!("non-json bot api response (status {status})"),
        })?;
    if !envelope.ok {
        let kind = classify_rejection(status);
        // The raw provider description stays an internal diagnostic —
        // bounded, char-safe, debug-only. It never rides the error type.
        let description: String = envelope
            .description
            .unwrap_or_default()
            .chars()
            .take(REJECTION_DIAGNOSTIC_MAX_CHARS)
            .collect();
        tracing::debug!(
            status,
            %kind,
            description,
            "telegram bot api rejected the request"
        );
        return Err(TelegramBotApiError::Rejected { kind });
    }
    Ok(envelope.result.unwrap_or(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mediated_request_keeps_token_in_host_credential_material() {
        let request = bot_api_request(
            &ResourceScope::system(),
            &SecretString::from("12345:secret-token".to_string()),
            "setWebhook",
            serde_json::json!({
                "url": "https://ironclaw.example/webhooks/extensions/telegram/updates",
                "secret_token": "webhook-secret",
                "allowed_updates": ["message"],
            }),
        )
        .expect("request builds");

        assert_eq!(request.request.method, NetworkMethod::Post);
        assert_eq!(request.extension_id.as_str(), TELEGRAM_EXTENSION_ID);
        assert_eq!(
            request.request.url,
            "https://api.telegram.org/bot{telegram_bot_token}/setWebhook"
        );
        assert!(!request.request.url.contains("12345:secret-token"));
        assert_eq!(request.credentials.len(), 1);
        assert_eq!(
            request.credentials[0].handle.as_str(),
            TELEGRAM_BOT_TOKEN_PLACEHOLDER
        );
        assert!(matches!(
            &request.credentials[0].target,
            RuntimeCredentialTarget::PathPlaceholder { placeholder }
                if placeholder == TELEGRAM_BOT_TOKEN_PLACEHOLDER
        ));
        assert!(request.credentials[0].required);
        let body: serde_json::Value =
            serde_json::from_slice(&request.request.body).expect("request JSON");
        assert_eq!(
            body["url"],
            "https://ironclaw.example/webhooks/extensions/telegram/updates"
        );
        assert_eq!(body["secret_token"], "webhook-secret");
        assert_eq!(body["allowed_updates"], serde_json::json!(["message"]));
    }

    #[test]
    fn parse_ok_envelope_returns_result() {
        let value = parse_bot_api_response(200, br#"{"ok":true,"result":{"id":7}}"#)
            .expect("ok envelope parses");
        assert_eq!(value["id"], 7);
    }

    #[test]
    fn parse_not_ok_envelope_maps_to_stable_rejection_category() {
        let error = parse_bot_api_response(
            401,
            br#"{"ok":false,"description":"Unauthorized: <a href=\"x\">attacker markup</a>"}"#,
        )
        .expect_err("not-ok envelope rejects");
        assert_eq!(
            error,
            TelegramBotApiError::Rejected {
                kind: TelegramBotApiRejection::Unauthorized
            }
        );
        assert!(
            !error.to_string().contains("attacker"),
            "provider description text must never reach the error Display"
        );

        let bad_request = parse_bot_api_response(
            400,
            br#"{"ok":false,"description":"Bad Request: bad webhook"}"#,
        )
        .expect_err("bad request rejects");
        assert_eq!(
            bad_request,
            TelegramBotApiError::Rejected {
                kind: TelegramBotApiRejection::InvalidRequest
            }
        );
    }

    #[test]
    fn parse_garbage_is_invalid_response_without_body_echo() {
        let error =
            parse_bot_api_response(502, b"<html>bad gateway</html>").expect_err("garbage rejects");
        match error {
            TelegramBotApiError::InvalidResponse { reason } => {
                assert!(!reason.contains("html"), "reason must not echo the body");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
