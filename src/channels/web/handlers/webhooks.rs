//! Public webhook trigger endpoint for routine webhook triggers.
//!
//! `POST /api/webhooks/{path}` — matches the path against routines with
//! `Trigger::Webhook { path, secret }`, validates the request with either
//! `X-Hub-Signature-256` (preferred) or the legacy `X-Webhook-Secret` header,
//! and fires the matching routine through the `RoutineEngine`.

use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use subtle::ConstantTimeEq;

use crate::agent::routine::Trigger;
use crate::channels::web::server::GatewayState;

fn auth_failed() -> (StatusCode, String) {
    (
        StatusCode::UNAUTHORIZED,
        "Webhook authentication failed".to_string(),
    )
}

/// Validate the webhook authentication for a routine.
///
/// Accepts `X-Hub-Signature-256` (preferred) for body HMAC verification and
/// falls back to the legacy `X-Webhook-Secret` header for compatibility.
fn validate_webhook_auth(
    trigger: &Trigger,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), (StatusCode, String)> {
    let expected_secret = match trigger {
        Trigger::Webhook {
            secret: Some(s), ..
        } => s,
        _ => return Err(auth_failed()),
    };

    if let Some(raw_signature) = headers.get("x-hub-signature-256") {
        let signature = raw_signature.to_str().map_err(|_| auth_failed())?;
        if !crate::channels::wasm::signature::verify_hmac_sha256_prefixed(
            expected_secret,
            body,
            signature,
            "sha256=",
        ) {
            return Err(auth_failed());
        }
        return Ok(());
    }

    let provided_secret = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(auth_failed)?;

    if !bool::from(provided_secret.as_bytes().ct_eq(expected_secret.as_bytes())) {
        return Err(auth_failed());
    }

    Ok(())
}

/// Handle incoming webhook POST to `/api/webhooks/{path}`.
///
/// This endpoint is **public** (no gateway auth token required) but protected
/// by either a per-routine HMAC signature (`X-Hub-Signature-256`) or the
/// legacy `X-Webhook-Secret` header.
pub async fn webhook_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Rate limit check
    if !state.webhook_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Targeted query instead of loading all routines
    let routine = store
        .get_webhook_routine_by_path(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(auth_failed)?;

    validate_webhook_auth(&routine.trigger, &headers, &body)?;

    // Fire through the RoutineEngine so guardrails, run tracking,
    // notifications, and FullJob dispatch all work correctly.
    let engine = {
        let guard = state.routine_engine.read().await;
        guard.as_ref().cloned().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Routine engine not available".to_string(),
        ))?
    };

    let run_id = engine.fire_webhook(routine.id, &path).await.map_err(|e| {
        let status = match &e {
            crate::error::RoutineError::NotFound { .. } => StatusCode::NOT_FOUND,
            crate::error::RoutineError::Disabled { .. }
            | crate::error::RoutineError::Cooldown { .. }
            | crate::error::RoutineError::MaxConcurrent { .. } => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, e.to_string())
    })?;

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine.id,
        "routine_name": routine.name,
        "run_id": run_id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn legacy_secret_headers(secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let value =
            HeaderValue::from_str(secret).unwrap_or_else(|_| HeaderValue::from_static("invalid"));
        headers.insert("x-webhook-secret", value);
        headers
    }

    fn hmac_headers(body: &[u8], secret: &str) -> Result<HeaderMap, String> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut headers = HeaderMap::new();
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .map_err(|err| format!("hmac key: {err}"))?;
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        let value = HeaderValue::from_str(&sig).map_err(|err| format!("header: {err}"))?;
        headers.insert("x-hub-signature-256", value);
        Ok(headers)
    }

    /// Routines with `secret: None` must be rejected.
    #[test]
    fn test_validate_rejects_missing_secret() -> Result<(), String> {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: None,
        };
        let result = validate_webhook_auth(&trigger, &HeaderMap::new(), b"{}");
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "Webhook authentication failed");
        Ok(())
    }

    /// Non-webhook triggers must be rejected.
    #[test]
    fn test_validate_rejects_non_webhook_trigger() -> Result<(), String> {
        let trigger = Trigger::Manual;
        let result = validate_webhook_auth(&trigger, &HeaderMap::new(), b"{}");
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        Ok(())
    }

    /// Correct legacy secret passes validation.
    #[test]
    fn test_validate_accepts_correct_legacy_secret() -> Result<(), String> {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("s3cret-token".to_string()),
        };
        assert!(
            validate_webhook_auth(&trigger, &legacy_secret_headers("s3cret-token"), b"{}").is_ok()
        );
        Ok(())
    }

    /// Correct HMAC signature passes validation.
    #[test]
    fn test_validate_accepts_correct_hmac_signature() -> Result<(), String> {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("s3cret-token".to_string()),
        };
        let body = br#"{"event":"push"}"#;
        assert!(
            validate_webhook_auth(&trigger, &hmac_headers(body, "s3cret-token")?, body).is_ok()
        );
        Ok(())
    }

    /// Wrong legacy secret returns 401.
    #[test]
    fn test_validate_rejects_wrong_legacy_secret() -> Result<(), String> {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("correct-secret".to_string()),
        };
        let result = validate_webhook_auth(&trigger, &legacy_secret_headers("wrong-secret"), b"{}");
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "Webhook authentication failed");
        Ok(())
    }

    /// Wrong HMAC signature returns 401.
    #[test]
    fn test_validate_rejects_wrong_hmac_signature() -> Result<(), String> {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("real-secret".to_string()),
        };
        let result = validate_webhook_auth(
            &trigger,
            &hmac_headers(br#"{"event":"push"}"#, "different-secret")?,
            br#"{"event":"push"}"#,
        );
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        Ok(())
    }

    /// Missing auth headers return 401 without revealing whether the route exists.
    #[test]
    fn test_validate_rejects_missing_auth_headers() -> Result<(), String> {
        let trigger = Trigger::Webhook {
            path: None,
            secret: Some("short".to_string()),
        };
        let result = validate_webhook_auth(&trigger, &HeaderMap::new(), b"{}");
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "Webhook authentication failed");
        Ok(())
    }
}
